//! The browser's front end: the window's job (see `window.rs`), as a
//! WebAssembly module for a web page to drive (`web/index.html`, through
//! `web/ur.js`).
//!
//! The page owns the event loop. Each animation frame, it lets the time
//! that passed go by ([`ur_advance`]) and draws the watch ([`ur_frame`]);
//! between frames, it hands over the pointer ([`ur_pointer`]), keys
//! ([`ur_key`]), and typed commands ([`ur_command`]). Everything the
//! watch does is the same code the command line runs: the same
//! [`Watch`], the same script commands, and the same drawing of the case
//! ([`Case`]) from the same 240×240 panel, so its pixels match the
//! native simulator's.
//!
//! The interface is plain `extern "C"` functions, numbers and pointers
//! into the module's memory, rather than wasm-bindgen's generated glue: it
//! keeps the build to `cargo` alone (no `wasm-bindgen-cli` to pin to the
//! crate's version), and the few strings that cross over are easy to pass
//! by hand. Text goes in through a buffer the page writes into
//! ([`ur_buffer`]), and comes out through the page's `ur_print`.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::path::PathBuf;

use embassy_time::{Duration, Instant};

use crate::InputEvent;
use crate::case::{self, Case};
use crate::display::PANEL;
use crate::script::{self, Next, Output};
use crate::themes::{self, Theme as _};
use crate::watch::Watch;

/// Window pixels a canvas pixel, as the window's default: a 320-pixel
/// screen. The page scales it to fit.
const ZOOM: usize = 4;

/// How often a push's frames are kept for the page to show, in the
/// watch's time that the push runs through.
const FRAME: Duration = Duration::from_millis(16);

/// At most this many of a push's frames wait to be shown: about a second
/// of them, as a frame is a copy of the panel (115 KB). A transition takes
/// ~0.47 s, so ~30; nothing the watch pushes in one go takes longer.
/// Beyond it, the page would hold the last frame kept until the watch's
/// clock is due to move again.
const QUEUED_FRAMES: usize = 64;

/// What the page starts with, so it opens on a watch in use rather than
/// one fresh from a reset (see the file).
const START: &str = include_str!("../web/start.txt");

// ==============================================================================
// The page's side
// ==============================================================================

// What the page gives the module (see `web/ur.js`).
#[link(wasm_import_module = "env")]
unsafe extern "C" {
    /// Shows `len` bytes of UTF-8 text at `text`, of a [`Text`] kind.
    fn ur_print(kind: u32, text: *const u8, len: usize);
    /// Offers `len` bytes at `data` as a download named by `name`.
    fn ur_download(name: *const u8, name_len: usize, data: *const u8, len: usize);
    /// The browser's clock: milliseconds since 1970, UTC.
    safe fn ur_now_ms() -> f64;
    /// The browser's time zone's offset from UTC at that instant, in
    /// minutes (east positive, unlike `Date.getTimezoneOffset`).
    safe fn ur_utc_offset_minutes(ms: f64) -> i32;
}

/// What a text the module prints is.
#[derive(Clone, Copy)]
pub enum Text {
    /// A log line: what happened.
    Log = 0,
    /// An error: a command that didn't work, or a crash.
    Error = 1,
    /// The status line (as the window's title).
    Status = 2,
    /// The colour schemes, one a line: its theme's name, a tab, its own.
    Themes = 3,
}

pub fn print(kind: Text, text: &str) {
    // SAFETY: The page only reads the `len` bytes at `text`, which are
    // valid for the call.
    unsafe { ur_print(kind as u32, text.as_ptr(), text.len()) }
}

pub fn download(name: &str, data: &[u8]) {
    // SAFETY: As in `print`: the page copies the bytes out during the call.
    unsafe { ur_download(name.as_ptr(), name.len(), data.as_ptr(), data.len()) }
}

pub fn now_ms() -> f64 {
    ur_now_ms()
}

pub fn utc_offset_minutes(ms: f64) -> i32 {
    ur_utc_offset_minutes(ms)
}

// ==============================================================================
// The module's state
// ==============================================================================
//
// Kept apart, because they're borrowed at different times: the watch for
// the whole of a call into it, and the screen also from within that call,
// while a transition hands over its frames (see `Screen::push_frame`).

thread_local! {
    static WATCH: RefCell<Option<Watch>> = const { RefCell::new(None) };
    static SCREEN: RefCell<Option<Screen>> = const { RefCell::new(None) };
    static HANDS: RefCell<Hands> = RefCell::new(Hands::default());
    /// Text from the page (see [`ur_buffer`]).
    static INPUT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

// ==============================================================================
// Showing pushes at their pace
// ==============================================================================
//
// The window shows a push's frames as it runs, sleeping through each
// step (see `window.rs`). The page can't: nothing it's given shows until
// the call into the module returns, and a swipe's transition runs to its
// end within the one call (`ur_pointer`, `ur_key`), moving the watch's
// clock ~0.47 s on in no time at all. So the screen keeps the push's
// frames, each with the instant on the watch's clock it shows, and plays
// them back a little after, at their pace: the page has a clock of its
// own on the watch's timeline (`Screen::shown`), which runs with the
// browser's, and shows the latest frame it has reached. While it's
// behind the watch's, the watch waits for it, rather than running on
// while its past is still being shown: `ur_advance` moves the watch only
// once the page has caught up.
//
// Only the latest run of pushes (pushes one after the other, with no
// time passing between them but theirs) is kept: one that starts after
// the clock moved on otherwise (a `wait`) supersedes those before. And
// time a command skips on purpose (`wait 1h`, `F`) isn't played back:
// the page's clock jumps with it, to the start of the run of pushes it
// ended on, if any.

/// What the panel showed at an instant: a frame kept to be shown later.
struct Frame {
    at: Instant,
    /// The panel's pixels and idle mode, as `display::Panel`'s.
    pixels: Vec<u16>,
    idle: bool,
}

impl Frame {
    /// What the panel shows now.
    fn now() -> Self {
        let mut frame = Frame {
            at: Instant::now(),
            pixels: Vec::new(),
            idle: false,
        };
        frame.update();
        frame
    }

    /// Makes this what the panel shows now, in place (as the page draws
    /// it every animation frame).
    fn update(&mut self) {
        PANEL.with_borrow(|panel| {
            self.at = Instant::now();
            self.pixels.clone_from(&panel.pixels);
            self.idle = panel.idle;
        });
    }
}

/// The drawn watch, as the window's `Screen`.
struct Screen {
    case: Case,
    /// The case as drawn, 0xRRGGBB, and as the page wants it, RGBA.
    buffer: Vec<u32>,
    rgba: Vec<u8>,
    /// The backlight's brightness (0 to 1), and how far the motor has
    /// shaken the watch, as of the last frame the page saw live: a push's
    /// frames, played back, keep them, as the window's do.
    light: f32,
    shake: f32,
    /// The real panel's colours, or those sent to it (see `crate::panel`).
    mimic: bool,
    /// What the page shows: the panel now, or a frame of a push.
    showing: Frame,
    /// Where the page has got to on the watch's timeline: behind the
    /// watch's clock while a push's frames play back (see above).
    shown: Instant,
    /// The latest run of pushes' frames not yet shown, oldest first, and
    /// how much of it has run since the last one was kept.
    frames: VecDeque<Frame>,
    since_frame: Duration,
    /// When the latest run of pushes started and ended.
    run_start: Instant,
    run_end: Instant,
}

impl Screen {
    /// `step` of a push has run (see `Watch::on_frame`), up to now.
    fn push_frame(&mut self, step: Duration) {
        let now = Instant::now();
        let start = now - step;
        if start != self.run_end {
            // The clock moved on since the last push (or started again
            // from 0, at a reboot): a new run, which makes those before it
            // past. What the page has reached of them is shown first.
            self.reach_frames();
            self.frames.clear();
            self.since_frame = Duration::from_ticks(0);
            self.run_start = start;
        }
        self.run_end = now;
        self.since_frame += step;
        if self.since_frame < FRAME || self.frames.len() >= QUEUED_FRAMES {
            return;
        }
        self.since_frame = Duration::from_ticks(0);
        self.frames.push_back(Frame::now());
    }

    /// Shows the latest frame waiting that the page's clock has reached.
    fn reach_frames(&mut self) {
        while let Some(frame) = self.frames.pop_front_if(|frame| frame.at <= self.shown) {
            self.showing = frame;
        }
    }

    /// After a call into the module: starts the page's clock again with
    /// the watch's if that did (a reboot), and, if `skips` (a call that may
    /// skip time on purpose, as a command's `wait`), moves it on to what's
    /// left to play back.
    fn settle(&mut self, skips: bool) {
        let now = Instant::now();
        // The clock started again from 0 (a reboot), so what's kept is
        // from the old one: there's nothing to play back.
        if self.run_end > now {
            self.frames.clear();
            self.run_start = now;
            self.run_end = now;
        }
        if self.shown > now {
            self.shown = now;
        }
        if skips {
            self.shown = if self.run_end == now {
                // The call ended on a run of pushes (a swipe's
                // transition, say): play it from its start, unless the
                // page is already past that.
                self.shown.max(self.run_start)
            } else {
                now
            };
        }
    }

    fn draw(&mut self) {
        self.case.draw_pixels(
            &mut self.buffer,
            &self.showing.pixels,
            self.showing.idle,
            self.light,
            self.shake,
            self.mimic,
        );
        for (rgba, pixel) in self
            .rgba
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .zip(&self.buffer)
        {
            let [_, r, g, b] = pixel.to_be_bytes();
            *rgba = [r, g, b, 255];
        }
    }
}

/// What the wearer is doing with the pointer and keys.
#[derive(Default)]
struct Hands {
    /// A finger is on the panel (the pointer went down on it).
    finger: bool,
    /// The next test notification `N` sends, and how many screenshots
    /// `S` has taken and recordings `R` has started: their files' names
    /// count them (the window's name recordings by the computer's clock,
    /// which the browser's build reads only for `time now`).
    notification: u8,
    shots: u32,
    recordings: u32,
    /// The phone is playing music (`M`), has said what (the first `M`),
    /// and is ringing (`C`).
    playing: bool,
    music_known: bool,
    ringing: bool,
}

fn with_watch<R>(f: impl FnOnce(&mut Watch) -> R) -> R {
    WATCH.with_borrow_mut(|watch| f(watch.as_mut().expect("ur_boot comes first")))
}

fn with_screen<R>(f: impl FnOnce(&mut Screen) -> R) -> R {
    SCREEN.with_borrow_mut(|screen| f(screen.as_mut().expect("ur_boot comes first")))
}

fn output() -> Output {
    // `shot FILE` downloads FILE (see `platform::save`): no directory.
    Output {
        dir: PathBuf::new(),
        zoom: ZOOM,
        save: true,
    }
}

/// Runs a command, reporting (rather than stopping at) errors.
fn command(watch: &mut Watch, line: &str) {
    match script::execute(watch, line, &output()) {
        Ok(Next::Continue) => {}
        Ok(Next::Quit) => print(Text::Error, "there's no quitting a web page"),
        Err(e) => print(Text::Error, &e),
    }
}

/// The text the page last put in the buffer.
fn input() -> String {
    INPUT.with_borrow(|input| String::from_utf8_lossy(input).into_owned())
}

// ==============================================================================
// What the page calls
// ==============================================================================

/// Boots the watch, and with `start`, runs `web/start.txt` on it. Without,
/// it's as the command line's, after a reset: for scripts whose frames
/// should match the command line's.
#[unsafe(no_mangle)]
pub extern "C" fn ur_boot(start: bool) {
    // A panic aborts the module (wasm32-unknown-unknown can't unwind), and
    // the page only learns that it trapped: say why first.
    std::panic::set_hook(Box::new(|info| {
        print(Text::Error, &format!("the simulator crashed: {info}"));
    }));
    let case = Case::new(ZOOM);
    let pixels = case.width * case.height;
    SCREEN.set(Some(Screen {
        case,
        buffer: vec![0; pixels],
        rgba: vec![0; pixels * 4],
        light: 0.0,
        shake: 0.0,
        mimic: true,
        showing: Frame::now(),
        shown: Instant::now(),
        frames: VecDeque::new(),
        since_frame: Duration::from_ticks(0),
        run_start: Instant::now(),
        run_end: Instant::now(),
    }));
    // Flash that starts erased, and is forgotten with the page.
    let mut watch = Watch::boot(crate::flash::Flash::erased());
    watch.on_frame = Some(Box::new(|step| with_screen(|s| s.push_frame(step))));
    if start && let Err(e) = script::run(&mut watch, "web/start.txt", START, &output()) {
        print(Text::Error, &e);
    }
    WATCH.set(Some(watch));
    // The page opens on the watch as the script left it, not on its
    // last push played back.
    with_screen(|screen| {
        screen.frames.clear();
        screen.showing = Frame::now();
        screen.shown = Instant::now();
    });
}

/// Room for `len` bytes of text from the page, which it writes there
/// before calling [`ur_command`], [`ur_script`], or [`ur_key`]. Valid
/// until the next call into the module.
#[unsafe(no_mangle)]
pub extern "C" fn ur_buffer(len: usize) -> *mut u8 {
    INPUT.with_borrow_mut(|input| {
        input.clear();
        input.resize(len, 0);
        input.as_mut_ptr()
    })
}

/// Runs the buffer's text as one command, as typed into the terminal.
#[unsafe(no_mangle)]
pub extern "C" fn ur_command() {
    let line = input();
    if !line.trim().is_empty() {
        print(Text::Log, &format!("> {line}"));
    }
    with_watch(|watch| command(watch, &line));
    with_screen(|screen| screen.settle(true));
}

/// Runs the buffer's text as a script (say, one of `sim/`'s), stopping at
/// its first error, as the command line does.
#[unsafe(no_mangle)]
pub extern "C" fn ur_script() {
    let text = input();
    let result = with_watch(|watch| script::run(watch, "script", &text, &output()));
    with_screen(|screen| screen.settle(true));
    match result {
        Ok(Next::Continue) => print(Text::Log, "script done"),
        Ok(Next::Quit) => print(Text::Log, "script done (it quit)"),
        Err(e) => print(Text::Error, &e),
    }
}

/// Lets `ms` milliseconds pass on the page's clock, as the window does
/// each frame, and on the watch's as far as the page's has caught up with
/// it: while a push's frames play back, the watch waits for them (see
/// "Showing pushes at their pace").
#[unsafe(no_mangle)]
pub extern "C" fn ur_advance(ms: f64) {
    let shown = with_screen(|screen| {
        screen.shown += Duration::from_micros((ms.max(0.0) * 1e3) as u64);
        screen.shown
    });
    let now = Instant::now();
    if shown > now {
        with_watch(|watch| watch.advance(shown - now));
    }
    // A push that ran past `shown` (a timer's, say) puts the page behind
    // the watch again, by what's left of it to play back.
    with_screen(|screen| screen.settle(false));
}

/// The page's colours: the real panel's (`true`, the default), or those
/// sent to it, as the window's P switches.
#[unsafe(no_mangle)]
pub extern "C" fn ur_mimic(on: bool) {
    with_screen(|screen| screen.mimic = on);
}

/// The pointer (a mouse, a finger, a pen), at (`x`, `y`) in the drawn
/// watch's pixels: down (0), moved (1), or up or cancelled (2). On the
/// panel, it's a finger, through the touch task's gesture recogniser
/// (hold still for a long press, move for a swipe); on the button, it
/// presses the button.
#[unsafe(no_mangle)]
pub extern "C" fn ur_pointer(kind: u32, x: f32, y: f32) {
    let (on_panel, on_button) = with_screen(|s| (s.case.on_panel(x, y), s.case.on_button(x, y)));
    let finger = HANDS.with_borrow(|hands| hands.finger);
    with_watch(|watch| match kind {
        0 if on_button => watch.input(InputEvent::Button),
        0 => {
            if let Some((x, y)) = on_panel {
                watch.finger_at(x, y);
                HANDS.with_borrow_mut(|hands| hands.finger = true);
            }
        }
        // Off the panel, the finger stays where it left it.
        1 if finger => {
            if let Some((x, y)) = on_panel {
                watch.finger_at(x, y);
            }
        }
        2 if finger => {
            watch.lift();
            HANDS.with_borrow_mut(|hands| hands.finger = false);
        }
        _ => {}
    });
    with_screen(|screen| screen.settle(true));
}

/// A key, by its `KeyboardEvent.key`: the window's keys (see `window.rs`'s
/// `KEYS`), bar quitting. Returns whether it was one.
#[unsafe(no_mangle)]
pub extern "C" fn ur_key() -> bool {
    let key = input().to_ascii_lowercase();
    // The script command each key stands for, worked out first: `HANDS`
    // can't stay borrowed while the watch runs it (see the state above).
    let line = HANDS.with_borrow_mut(|hands| {
        Some(match key.as_str() {
            " " | "enter" | "b" => "button".to_owned(),
            "g" => "glance".to_owned(),
            "t" => "tap".to_owned(),
            "l" => "long".to_owned(),
            "arrowup" => "up".to_owned(),
            "arrowdown" => "down".to_owned(),
            "arrowleft" => "left".to_owned(),
            "arrowright" => "right".to_owned(),
            "f" => "wait 1m".to_owned(),
            "h" => "wait 1h".to_owned(),
            "n" => {
                let n = hands.notification;
                hands.notification = (n + 1) % 6;
                format!("notify {n}")
            }
            // Plays or pauses; the first time, the phone says what.
            "m" => {
                hands.playing = !hands.playing;
                let state = if hands.playing { "playing" } else { "paused" };
                if std::mem::replace(&mut hands.music_known, true) {
                    format!("music {state}")
                } else {
                    format!("music \"Ida Moreau\" \"Night Signals\" {state}")
                }
            }
            // A call rings, or the one ringing ends.
            "c" => {
                hands.ringing = !hands.ringing;
                if hands.ringing {
                    "call \"Nora Lindqvist\"".to_owned()
                } else {
                    "call over".to_owned()
                }
            }
            "s" => {
                hands.shots += 1;
                format!("shot ur-sim-{}.png", hands.shots)
            }
            // A GIF of the screen, as the window's R: on the watch's
            // clock, so it plays at the watch's pace. It downloads when
            // stopped.
            "r" if crate::record::active() => "stop".to_owned(),
            "r" => {
                hands.recordings += 1;
                format!("record ur-sim-{}.gif", hands.recordings)
            }
            _ => return None,
        })
    });
    let Some(line) = line else { return false };
    with_watch(|watch| command(watch, &line));
    with_screen(|screen| screen.settle(true));
    true
}

/// The drawn watch's size, in pixels.
#[unsafe(no_mangle)]
pub extern "C" fn ur_width() -> usize {
    with_screen(|screen| screen.case.width)
}

#[unsafe(no_mangle)]
pub extern "C" fn ur_height() -> usize {
    with_screen(|screen| screen.case.height)
}

/// The watch, drawn: [`ur_width`] × [`ur_height`] RGBA pixels, row by row,
/// valid until the next call into the module. While a push plays back,
/// its latest frame the page has reached; otherwise, the watch as it is
/// now, lit as the backlight is, and shaken while the motor runs.
#[unsafe(no_mangle)]
pub extern "C" fn ur_frame() -> *const u8 {
    let (light, pulse) = with_watch(|watch| (case::light(watch.backlight), watch.motor_pulse()));
    with_screen(|screen| {
        screen.reach_frames();
        // Between the last frame kept and the push's end (less than a
        // frame), the page holds that frame.
        if screen.frames.is_empty() && screen.shown >= Instant::now() {
            screen.showing.update();
            screen.light = light;
            screen.shake = screen.case.shake(pulse, Instant::now());
        }
        screen.draw();
        screen.rgba.as_ptr()
    })
}

/// Prints the status line, as the window's title bar shows it (see
/// [`crate::status`]).
#[unsafe(no_mangle)]
pub extern "C" fn ur_status() {
    let line = with_watch(|watch| {
        let recording = if crate::record::active() {
            " · recording (R to stop)"
        } else {
            ""
        };
        format!("{}{recording}", crate::status::line(watch, 1.0))
    });
    print(Text::Status, &line);
}

/// Prints the colour schemes, for the page's picker (`theme NAME`): a
/// line each, its theme's name, a tab, and its own.
#[unsafe(no_mangle)]
pub extern "C" fn ur_themes() {
    // Each built-in theme's own, so a one-sided build lists each once.
    let lines: Vec<String> = (themes::THEMES.iter())
        .flat_map(|theme| {
            let schemes = theme.night.iter().chain(theme.day);
            schemes.map(|scheme| format!("{}\t{}", theme.display_name(), scheme.palette().name))
        })
        .collect();
    print(Text::Themes, &lines.join("\n"));
}
