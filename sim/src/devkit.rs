//! Checking the simulator against a dev kit, frame for frame:
//! `ur-sim --devkit ELF SCRIPT`.
//!
//! The dev kit runs a build with the `compare` feature, which exports what
//! its UI loop does and renders with (see `src/compare.rs`). The simulator
//! follows it: it starts its UI from the dev kit's boot (the same instant,
//! the same settings from flash), replays every event the dev kit's UI got
//! (ticks and inputs), at the very instants it got
//! them, and for each `compare` in the script renders the frame the dev
//! kit last finished, with the readings it had (time, battery, sensors,
//! today's activity). Same code, same state, same readings: the frames
//! must match, pixel for pixel, and any that don't are reported.
//!
//! The script's inputs go to the dev kit the way `scripts/screenshot.py`
//! injects them, and only those it can inject. The simulator doesn't
//! decide what they do: it replays what the dev kit made of them (a
//! finger's touch, as its touch task recognised it).
//!
//! Each `compare` writes `NAME.png`: the simulator's frame, the dev kit's,
//! and where they differ (magenta), side by side.

use std::collections::VecDeque;
use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration as StdDuration, Instant as StdInstant};

use embassy_time::Instant;

use crate::alarm;
use crate::battery::Battery;
use crate::brightness;
use crate::canvas::{Canvas, HEIGHT, WIDTH};
use crate::clock::{self, DateTime};
use crate::compare::{self, Event, Export, Head, Pass, kind, phone};
use crate::display::{Display, PANEL, SIZE, rgb888};
use crate::gesture::{Gesture, Touch};
use crate::script;
use crate::ui::{App, PhoneEvent, Status, ThemeChoice, Unpaired};
use crate::{InputEvent, activity, debug, time};

const TARGET: [&str; 2] = ["--target", "nRF52832_xxAA"];
/// How long the dev kit may take to take an input (its input task looks
/// every 100 ms) and log it.
const INPUT_TIMEOUT: StdDuration = StdDuration::from_secs(3);
/// While waiting, how often to read the dev kit's events: well within the
/// [`compare::EVENTS`] it keeps (a tick a second).
const POLL: StdDuration = StdDuration::from_secs(2);

pub fn run(elf: &Path, scripts: &[PathBuf], out: &Path) -> Result<(), String> {
    let devkit = DevKit::open(elf)?;
    let mut follower = Follower::start(devkit)?;
    let mut failed = Vec::new();
    for path in scripts {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("reading {}: {e}", path.display()))?;
        for (n, line) in text.lines().enumerate() {
            let at = |e: String| format!("{}:{}: {e}", path.display(), n + 1);
            if let Some(Mismatch { name, pixels }) = follower.execute(line, out).map_err(at)? {
                eprintln!("MISMATCH: {name}: {pixels} pixels differ");
                failed.push(name);
            }
        }
    }
    if failed.is_empty() {
        eprintln!("every frame matched");
        Ok(())
    } else {
        Err(format!("frames differ: {}", failed.join(", ")))
    }
}

struct Mismatch {
    name: String,
    pixels: usize,
}

// ==============================================================================
// The dev kit, over the probe
// ==============================================================================

struct DevKit {
    input: u32,
    /// `UR_CANVAS`, which points to the canvas once the firmware's set it
    /// up, and where it points.
    canvas_pointer: u32,
    canvas: std::cell::Cell<Option<u32>>,
    export: u32,
    /// The last theme's palette, by its address: they don't change.
    palette: std::cell::RefCell<Option<(u32, Vec<u16>)>>,
}

impl DevKit {
    fn open(elf: &Path) -> Result<Self, String> {
        let nm = Command::new("llvm-nm")
            .arg(elf)
            .output()
            .map_err(|e| format!("running llvm-nm: {e}"))?;
        let symbols = String::from_utf8_lossy(&nm.stdout);
        let find = |name: &str| {
            symbols
                .lines()
                .find_map(|line| {
                    let mut parts = line.split_whitespace();
                    let (address, _, symbol) = (parts.next()?, parts.next()?, parts.next()?);
                    (symbol == name).then(|| u32::from_str_radix(address, 16).ok())?
                })
                .ok_or_else(|| {
                    format!(
                        "no {name} in {}{}",
                        elf.display(),
                        if name == "UR_COMPARE" {
                            " (build it with `--features compare`)"
                        } else {
                            ""
                        }
                    )
                })
        };
        Ok(Self {
            input: find("UR_INPUT")?,
            canvas_pointer: find("UR_CANVAS")?,
            canvas: std::cell::Cell::new(None),
            export: find("UR_COMPARE")?,
            palette: std::cell::RefCell::new(None),
        })
    }

    /// Runs an esprobe command, a few times over if the probe's bridge
    /// times out now and then (it does): everything here reads, or writes
    /// a word that's as good written twice.
    fn esprobe(args: &[&str]) -> Result<String, String> {
        let mut tries = 0;
        loop {
            let started = StdInstant::now();
            let output = Command::new("esprobe")
                .args(TARGET)
                .args(args)
                .output()
                .map_err(|e| format!("running esprobe: {e}"))?;
            if started.elapsed() > StdDuration::from_secs(5) {
                eprintln!("  (the probe took {:.0?})", started.elapsed());
            }
            if output.status.success() {
                return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
            }
            tries += 1;
            if tries == 3 {
                return Err(format!(
                    "esprobe {}: {}",
                    args.join(" "),
                    String::from_utf8_lossy(&output.stderr).trim()
                ));
            }
            std::thread::sleep(StdDuration::from_millis(200));
        }
    }

    fn read_words(&self, address: u32, count: usize) -> Result<Vec<u32>, String> {
        let mut words = Vec::with_capacity(count);
        // Each call costs ~0.1 s, however many words (up to a few thousand
        // at least): as few calls as can be, so a frame can be read whole
        // between two of the dev kit's passes.
        while words.len() < count {
            let n = (count - words.len()).min(4096);
            let at = address + 4 * words.len() as u32;
            let out = Self::esprobe(&[
                "core",
                "read",
                &format!("{at:#x}"),
                "--words",
                &n.to_string(),
            ])?;
            let before = words.len();
            for line in out.lines().filter(|l| l.starts_with("0x")) {
                let value = line
                    .split_whitespace()
                    .nth(1)
                    .and_then(|v| u32::from_str_radix(v.trim_start_matches("0x"), 16).ok())
                    .ok_or_else(|| format!("esprobe's output: {line:?}?"))?;
                words.push(value);
            }
            if words.len() - before != n {
                return Err(format!("short read at {at:#x}: {out}"));
            }
        }
        Ok(words)
    }

    fn read_bytes(&self, address: u32, len: usize) -> Result<Vec<u8>, String> {
        let words = self.read_words(address, len.div_ceil(4))?;
        let mut bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
        bytes.truncate(len);
        Ok(bytes)
    }

    fn write_word(&self, address: u32, value: u32) -> Result<(), String> {
        Self::esprobe(&[
            "core",
            "write",
            &format!("{address:#x}"),
            &format!("{value:#x}"),
        ])
        .map(drop)
    }

    /// Everything exported but the events.
    fn head(&self) -> Result<Head, String> {
        let bytes = self.read_bytes(self.export, size_of::<Head>())?;
        // SAFETY: `Head` is `repr(C)` integers and arrays of them, laid out
        // alike on both targets (see the test below), for which any bytes
        // are valid.
        Ok(unsafe { bytes.as_ptr().cast::<Head>().read_unaligned() })
    }

    /// Events `from` up to `to` (counted from boot), which the ring must
    /// still hold.
    fn events(&self, from: u32, to: u32) -> Result<Vec<Event>, String> {
        let ring = self.export + std::mem::offset_of!(Export, events) as u32;
        let size = size_of::<Event>();
        let mut events = Vec::new();
        let mut next = from;
        // In at most two reads: up to the ring's end, then from its start.
        while next != to {
            let slot = next as usize % compare::EVENTS;
            let count = (to.wrapping_sub(next) as usize).min(compare::EVENTS - slot);
            let bytes = self.read_bytes(ring + (slot * size) as u32, count * size)?;
            for chunk in bytes.chunks_exact(size) {
                // SAFETY: As for `head`.
                events.push(unsafe { chunk.as_ptr().cast::<Event>().read_unaligned() });
            }
            next = next.wrapping_add(count as u32);
        }
        Ok(events)
    }

    /// A theme's colours, as the panel gets them.
    fn palette(&self, theme: u32) -> Result<Vec<u16>, String> {
        if let Some((at, palette)) = &*self.palette.borrow()
            && *at == theme
        {
            return Ok(palette.clone());
        }
        // `Theme` is `repr(C)`: the name (`&str`: pointer, length), then
        // a big-endian RGB565 colour per role. (The dev kit's firmware has
        // to be this tree's, for the count to match.)
        let bytes = self.read_bytes(theme + 8, 2 * crate::theme::Color::COUNT)?;
        let palette: Vec<u16> = bytes
            .chunks(2)
            .map(|c| u16::from_be_bytes([c[0], c[1]]))
            .collect();
        *self.palette.borrow_mut() = Some((theme, palette.clone()));
        Ok(palette)
    }

    /// Restarts the dev kit, so its UI starts afresh, as the simulator's
    /// does. The export is cleared first, so its boot isn't mistaken for
    /// the last one's.
    fn reset(&self) -> Result<Head, String> {
        Self::esprobe(&["core", "reset-halt"])?;
        let magic = self.export + std::mem::offset_of!(Head, magic) as u32;
        self.write_word(magic, 0)?;
        Self::esprobe(&["core", "run"])?;
        let started = StdInstant::now();
        loop {
            let head = self.head()?;
            if head.magic == compare::MAGIC {
                return Ok(head);
            }
            if started.elapsed() > StdDuration::from_secs(15) {
                return Err("the dev kit didn't boot (or isn't a `compare` build)".into());
            }
            std::thread::sleep(StdDuration::from_millis(300));
        }
    }

    /// Injects an input, as `scripts/screenshot.py` does (see
    /// `src/debug.rs`), once the last one's been taken.
    fn inject(&self, kind: u8, x: u8, y: u8, extra: u8) -> Result<(), String> {
        self.wait_for_input_taken()?;
        let word = u32::from_le_bytes([kind, x, y, extra]);
        self.write_word(self.input, word)?;
        self.wait_for_input_taken()
    }

    fn wait_for_input_taken(&self) -> Result<(), String> {
        let started = StdInstant::now();
        while self.read_words(self.input, 1)?[0] != 0 {
            if started.elapsed() > INPUT_TIMEOUT {
                return Err("the dev kit didn't take the input".into());
            }
            std::thread::sleep(StdDuration::from_millis(50));
        }
        Ok(())
    }

    /// The first frame the dev kit finishes after every event it has
    /// logged so far (an event's frame comes a moment after it): what it
    /// was drawn with, its colour roles, and the theme's palette (RGB565,
    /// as the panel gets it). `None` if it draws none for a while, as with
    /// the screen off.
    fn frame(&self) -> Result<Option<Frame>, String> {
        let deadline = StdInstant::now() + StdDuration::from_secs(6);
        while StdInstant::now() < deadline {
            let before = self.head()?;
            let newest = match before.written.checked_sub(1) {
                Some(last) => self
                    .events(last, before.written)?
                    .first()
                    .map(|event| event.at.max(event.ticked)),
                None => None,
            };
            // A pass can read the same instant as the event before it: the
            // clock ticks every ~30 µs, and drawing takes milliseconds, so
            // an event can't follow a pass that closely.
            if before.started != before.finished || newest.is_some_and(|at| before.pass.now < at) {
                std::thread::sleep(StdDuration::from_millis(150));
                continue;
            }
            let canvas = match self.canvas.get() {
                Some(canvas) => canvas,
                None => {
                    let canvas = self.read_words(self.canvas_pointer, 1)?[0];
                    self.canvas.set(Some(canvas));
                    canvas
                }
            };
            // `Canvas` is `repr(C)`: back, front, `front_valid`, then (at
            // the next 4-byte boundary) the theme pointer.
            let bytes = WIDTH * HEIGHT / 2;
            let words = self.read_words(canvas + bytes as u32, bytes / 4 + 2)?;
            let theme = words[bytes / 4 + 1];
            let front: Vec<u8> = words[..bytes / 4]
                .iter()
                .flat_map(|w| w.to_le_bytes())
                .collect();
            let after = self.head()?;
            if after.started == before.started && after.finished == before.finished {
                let palette = self.palette(theme)?;
                return Ok(Some(Frame {
                    head: after,
                    front,
                    palette,
                }));
            }
        }
        Ok(None)
    }
}

/// A whole frame from the dev kit: what it was drawn with, its colour
/// roles (two to a byte), and its theme's palette.
struct Frame {
    head: Head,
    front: Vec<u8>,
    palette: Vec<u16>,
}

// ==============================================================================
// The simulator, following
// ==============================================================================

/// What the phone said, for a phone event the dev kit logs without its
/// content: the simulator knows what it injected.
enum Sent {
    Notification {
        app: &'static str,
        who: &'static str,
        backdate_minutes: i64,
    },
}

struct Follower {
    devkit: DevKit,
    app: App,
    canvas: Box<Canvas>,
    display: Display,
    /// Events replayed so far.
    replayed: u32,
    /// As the dev kit's UI loop last told its App (every pass, and awake
    /// passes).
    connected: bool,
    daytime: bool,
    sent: VecDeque<Sent>,
}

impl Follower {
    fn start(devkit: DevKit) -> Result<Self, String> {
        eprintln!("resetting the dev kit");
        let boot = devkit.reset()?.boot;
        time::set(Instant::from_ticks(boot.at));
        let themes = ThemeChoice {
            light: boot.theme_light,
            dark: boot.theme_dark,
        };
        let mut app = App::new(
            Instant::from_ticks(boot.at),
            themes,
            alarm::Settings::from_bytes(boot.alarms),
        );
        app.phone_alarms = alarm::Phone::from_bytes(boot.phone_alarms);
        app.brightness = brightness::Setting::from_byte(boot.brightness);
        eprintln!(
            "following the dev kit from its boot at {} s of uptime",
            boot.at / 32_768
        );
        let mut follower = Self {
            devkit,
            app,
            canvas: Box::new(Canvas::new()),
            display: Display,
            replayed: 0,
            connected: false,
            daytime: false,
            sent: VecDeque::new(),
        };
        follower.catch_up(None)?;
        Ok(follower)
    }

    fn execute(&mut self, line: &str, out: &Path) -> Result<Option<Mismatch>, String> {
        let words = script::split(line)?;
        let Some((command, args)) = words.split_first() else {
            return Ok(None);
        };
        let arg = |i: usize| args.get(i).map(String::as_str);
        let point = |i: usize| arg(i).map_or(Ok((40, 40)), script::point);
        // The dev kit's input kinds (see `src/debug.rs`).
        match command.as_str() {
            "button" => match arg(0) {
                None => self.inject(1, 0, 0, 0)?,
                Some("2") => self.inject(1, 2, 0, 0)?,
                Some(other) => return Err(format!("button {other}?")),
            },
            "glance" => self.inject(8, 0, 0, 0)?,
            "tap" | "long" | "up" | "down" | "left" | "right" => {
                let kind = match command.as_str() {
                    "tap" => 2,
                    "long" => 3,
                    "up" => 4,
                    "down" => 5,
                    "left" => 6,
                    _ => 7,
                };
                let (x, y) = point(0)?;
                let ms = match arg(1) {
                    Some(ms) if kind == 2 => ms
                        .parse::<u8>()
                        .map_err(|_| format!("tap: {ms} ms? (at most 255 on a dev kit)"))?,
                    Some(_) => return Err(format!("{command}: too many arguments")),
                    None => 0,
                };
                self.inject(kind, x as u8, y as u8, ms)?;
            }
            "press" => {
                let (x, y) = script::point(arg(0).ok_or("press: needs X,Y")?)?;
                self.inject(16, x as u8, y as u8, 0)?;
            }
            "lift" => self.inject(17, 0, 0, 0)?,
            "notify" => {
                let n: u8 =
                    arg(0).map_or(Ok(0), |n| n.parse().map_err(|_| format!("notify {n}?")))?;
                let hours: u8 = arg(1).map_or(Ok(0), |h| {
                    h.trim_end_matches('h')
                        .parse()
                        .map_err(|_| format!("notify: {h} hours ago?"))
                })?;
                let (app, who) = script::test_notification(n);
                self.sent.push_back(Sent::Notification {
                    app,
                    who,
                    backdate_minutes: i64::from(hours) * 60,
                });
                self.inject(9, n, hours, 0)?;
            }
            "media" => {
                let state = match arg(0) {
                    None | Some("paused") => 0,
                    Some("playing") => 1,
                    Some("unknown") => 2,
                    Some(other) => {
                        return Err(format!("media {other}? (playing, paused, unknown)"));
                    }
                };
                self.inject(14, state, 0, 0)?;
            }
            "steps" => {
                let steps: u16 = arg(0)
                    .ok_or("steps: needs N")?
                    .parse()
                    .map_err(|_| "steps: up to 65535 on a dev kit")?;
                let [x, y] = steps.to_le_bytes();
                self.inject(15, x, y, 0)?;
            }
            "battery" => {
                let mv: u16 = arg(0)
                    .ok_or("battery: needs MV")?
                    .parse()
                    .map_err(|_| "battery: MV?")?;
                let [x, y] = mv.to_le_bytes();
                self.inject(13, x, y, 0)?;
            }
            "wait" => {
                let wait = script::duration(arg(0).ok_or("wait: needs a duration")?)?;
                let until = StdInstant::now() + StdDuration::from_micros(wait.as_micros());
                while let Some(left) = until.checked_duration_since(StdInstant::now()) {
                    std::thread::sleep(left.min(POLL));
                    self.catch_up(None)?;
                }
            }
            "compare" => {
                let name = arg(0).ok_or("compare: needs a NAME")?;
                return self.compare(name, out).map(|pixels| {
                    (pixels > 0).then(|| Mismatch {
                        name: name.to_owned(),
                        pixels,
                    })
                });
            }
            // What the dev kit's UI is doing, as the simulator's, having
            // replayed the same, says.
            "expect" => {
                let (negated, what) = match arg(0) {
                    Some("not") => (true, arg(1)),
                    what => (false, what),
                };
                self.catch_up(None)?;
                let actual = match what {
                    Some("screen-on") => self.app.awake,
                    Some("dim") => self.app.dim(),
                    Some("quiet-night") => self.app.quiet_night(),
                    other => {
                        return Err(format!(
                            "expect {other:?}? (screen-on, dim, or quiet-night, with a dev kit)"
                        ));
                    }
                };
                if actual == negated {
                    let not = if negated { "not " } else { "" };
                    return Err(format!("expected {not}{}", what.unwrap_or_default()));
                }
            }
            "echo" => eprintln!("{}", args.join(" ")),
            other => {
                return Err(format!(
                    "{other}: not something a dev kit can be made to do (see `ur-sim --help`)"
                ));
            }
        }
        Ok(None)
    }

    fn inject(&mut self, kind: u8, x: u8, y: u8, extra: u8) -> Result<(), String> {
        self.devkit.inject(kind, x, y, extra)?;
        self.catch_up(None)
    }

    /// Replays the dev kit's events since the last time, up to (not
    /// including) `before`, if given.
    fn catch_up(&mut self, before: Option<u64>) -> Result<(), String> {
        let written = self.devkit.head()?.written;
        let behind = written.wrapping_sub(self.replayed) as usize;
        if behind > compare::EVENTS {
            return Err(format!(
                "fell behind the dev kit's events ({behind} since the last look; it keeps {}), \
                 or it restarted (see its log)",
                compare::EVENTS
            ));
        }
        for event in self.devkit.events(self.replayed, written)? {
            // (At the same instant as the pass: before it; see `frame`.)
            if before.is_some_and(|before| event.at > before) {
                break;
            }
            self.replay(&event)?;
            self.replayed = self.replayed.wrapping_add(1);
        }
        Ok(())
    }

    fn replay(&mut self, event: &Event) -> Result<(), String> {
        let at = Instant::from_ticks(event.at);
        time::set(at);
        self.app.set_connected(self.connected);
        self.app.set_daytime(self.daytime);
        let input = match event.kind {
            kind::TICK => {
                let second =
                    (event.local_second != compare::NONE_I64).then_some(event.local_second);
                let flag = |f| event.flags & f != 0;
                self.app.check_alarms(
                    at,
                    second,
                    flag(compare::flag::STIRRING),
                    flag(compare::flag::ASLEEP),
                );
                let ticked = Instant::from_ticks(event.ticked);
                time::set(ticked);
                self.app.on_tick(ticked, flag(compare::flag::WATCHING));
                None
            }
            kind::BUTTON => Some(InputEvent::Button),
            kind::ATTENTION => Some(InputEvent::Attention),
            kind::GLANCE => Some(InputEvent::Glance),
            kind::TOUCH => Some(InputEvent::Touch(Touch {
                gesture: compare::gesture_from_code(event.gesture)
                    .ok_or("the dev kit logged an unknown gesture")?,
                x: i32::from(event.x),
                y: i32::from(event.y),
                held_ms: event.held_ms,
            })),
            kind::PHONE => Some(InputEvent::Phone(self.phone(event.phone)?)),
            other => return Err(format!("the dev kit logged an unknown event ({other})")),
        };
        if let Some(input) = input {
            eprintln!("  dev kit: {} at {:.3} s", describe(&input), seconds(at));
            self.app.on_input(input, at);
        }
        // What the events set off on the dev kit (buzzes, messages for the
        // phone) isn't drawn: drop it here, so it doesn't pile up.
        while crate::motor::take_request().is_some() {}
        while crate::ble::OUTBOX.try_receive().is_ok() {}
        Ok(())
    }

    /// What the phone sent, by the kind the dev kit logged.
    fn phone(&mut self, kind: u8) -> Result<PhoneEvent, String> {
        Ok(match kind {
            phone::NOTIFICATION => match self.sent.pop_front() {
                Some(Sent::Notification {
                    app,
                    who,
                    backdate_minutes,
                }) => {
                    debug::backdate_next(backdate_minutes);
                    PhoneEvent::Notification {
                        app: script::truncated(app),
                        who: script::truncated(who),
                    }
                }
                None => {
                    return Err("the dev kit got a notification this didn't send \
                                (is a phone connected to it?)"
                        .into());
                }
            },
            // The dev kit's `media` input's (see `src/debug.rs`).
            phone::MUSIC_INFO => PhoneEvent::MusicInfo {
                artist: script::truncated("Ida Moreau"),
                track: script::truncated("Night Signals"),
            },
            phone::MUSIC_STATE_PAUSED => PhoneEvent::MusicState { playing: false },
            phone::MUSIC_STATE_PLAYING => PhoneEvent::MusicState { playing: true },
            phone::TEXT_AS_BITMAPS => PhoneEvent::TextAsBitmaps,
            other => {
                return Err(format!(
                    "the dev kit got a message from the phone (kind {other}) this can't \
                     reproduce (is a phone connected to it?)"
                ));
            }
        })
    }

    /// Compares the dev kit's latest whole frame with the simulator's
    /// drawing of it; writes them side by side to `out/NAME.png`. Returns
    /// how many canvas pixels differ.
    fn compare(&mut self, name: &str, out: &Path) -> Result<usize, String> {
        // With the screen off, the panel keeps the last frame it showed,
        // from before what's happened since: nothing to compare. Whether
        // it's off is the simulator's UI's call, having seen the same.
        let off = |follower: &Self| {
            (!follower.app.awake).then(|| {
                eprintln!("compare {name}: skipped: the screen is off");
                0
            })
        };
        self.catch_up(None)?;
        if let Some(skipped) = off(self) {
            return Ok(skipped);
        }
        let Some(Frame {
            head,
            front,
            palette,
        }) = self.devkit.frame()?
        else {
            self.catch_up(None)?;
            return off(self).ok_or_else(|| {
                format!(
                    "compare {name}: the dev kit drew nothing after its last event, \
                     though the simulator's screen is on"
                )
            });
        };
        let pass = head.pass;
        // Everything before the frame, and nothing after.
        self.catch_up(Some(pass.now))?;
        self.render(&pass);
        let devkit: Vec<u16> = (0..WIDTH * HEIGHT)
            .map(|i| {
                let role = if i % 2 == 0 {
                    front[i / 2] & 0x0F
                } else {
                    front[i / 2] >> 4
                };
                palette.get(usize::from(role)).copied().unwrap_or(0xF81F)
            })
            .collect();
        let sim: Vec<u16> = PANEL.with_borrow(|panel| {
            (0..WIDTH * HEIGHT)
                .map(|i| {
                    let (x, y) = (i % WIDTH * 3, i / WIDTH * 3);
                    panel.pixels[(y + 1) * SIZE + x + 1]
                })
                .collect()
        });
        let differ = sim.iter().zip(&devkit).filter(|(a, b)| a != b).count();
        let path = out.join(format!("{name}.png"));
        side_by_side(&path, &sim, &devkit)?;
        let time = if pass.has_time != 0 {
            format!("{:02}:{:02}:{:02}", pass.hour, pass.minute, pass.second)
        } else {
            "no time".into()
        };
        eprintln!(
            "compare {name}: the frame at {:.3} s ({time}): {} -> {}",
            seconds(Instant::from_ticks(pass.now)),
            if differ == 0 {
                "identical".to_owned()
            } else {
                format!("{differ} pixels differ")
            },
            path.display()
        );
        Ok(differ)
    }

    /// Draws the frame the dev kit drew at `pass.now`, with its readings.
    fn render(&mut self, pass: &Pass) {
        let now = Instant::from_ticks(pass.now);
        time::set(now);
        self.connected = pass.connected != 0;
        self.daytime = pass.daytime != 0;
        self.app.set_connected(self.connected);
        self.app.set_daytime(self.daytime);
        if pass.has_offset != 0 && pass.local_second != compare::NONE_I64 {
            let offset = pass.offset_minutes;
            clock::set(pass.local_second - i64::from(offset) * 60, offset);
        }
        activity::show_devkit_day(pass.day());
        let battery =
            Battery::measured(pass.battery_mv, pass.charging != 0, pass.power_present != 0);
        let status = Status {
            uptime_secs: pass.uptime_secs,
            local_time: (pass.has_time != 0).then_some(DateTime {
                year: pass.year,
                month: pass.month,
                day: pass.day,
                hour: pass.hour,
                minute: pass.minute,
                second: pass.second,
            }),
            connected: self.connected,
            unpaired: match pass.unpaired {
                1 => Some(Unpaired::NotPaired),
                2 => Some(Unpaired::PairingLost),
                _ => None,
            },
            passkey: (pass.passkey != compare::NONE_U32).then_some(pass.passkey),
            update_progress: (pass.update_progress != 255).then_some(pass.update_progress),
            steps: (pass.steps != compare::NONE_U32).then_some(pass.steps),
            heart_rate: (pass.heart_rate != 0).then_some(pass.heart_rate),
            temperature: (pass.has_temperature != 0).then_some(pass.temperature),
            sleep: (pass.has_sleep != 0).then_some(activity::Sleep {
                start_utc_minute: pass.sleep_start_utc_minute,
                end_utc_minute: pass.sleep_end_utc_minute,
                asleep_minutes: pass.sleep_asleep_minutes,
                deep_minutes: pass.sleep_deep_minutes,
                rem_minutes: pass.sleep_rem_minutes,
            }),
            local_second: (pass.local_second != compare::NONE_I64).then_some(pass.local_second),
            battery: &battery,
        };
        self.canvas.set_theme(self.app.theme());
        self.app.render(&mut self.canvas, &status, now);
        let Self {
            canvas, display, ..
        } = self;
        crate::watch::run(canvas.flush(display), &mut None);
    }
}

fn seconds(at: Instant) -> f64 {
    at.as_ticks() as f64 / 32_768.0
}

fn describe(event: &InputEvent) -> String {
    match event {
        InputEvent::Button => "button".into(),
        InputEvent::Glance => "glance".into(),
        InputEvent::Attention => "attention".into(),
        InputEvent::Touch(touch) => format!(
            "{} at ({},{}), {} ms",
            match touch.gesture {
                Gesture::Tap => "tap",
                Gesture::LongPress => "long press",
                Gesture::SwipeUp => "swipe up",
                Gesture::SwipeDown => "swipe down",
                Gesture::SwipeLeft => "swipe left",
                Gesture::SwipeRight => "swipe right",
            },
            touch.x,
            touch.y,
            touch.held_ms
        ),
        InputEvent::Phone(_) => "a message from the phone".into(),
    }
}

/// Writes the simulator's frame, the dev kit's, and their differences
/// (magenta where they differ, the dev kit's frame dimmed elsewhere), side
/// by side, at the panel's 3×.
fn side_by_side(path: &Path, sim: &[u16], devkit: &[u16]) -> Result<(), String> {
    const GAP: usize = 8;
    let width = 3 * SIZE + 2 * GAP;
    let mut rgb = vec![0x80u8; width * SIZE * 3];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let i = (y / 3) * WIDTH + x / 3;
            let mut put = |panel: usize, color: [u8; 3]| {
                let at = (y * width + panel * (SIZE + GAP) + x) * 3;
                rgb[at..at + 3].copy_from_slice(&color);
            };
            put(0, rgb888(sim[i], false));
            put(1, rgb888(devkit[i], false));
            put(
                2,
                if sim[i] == devkit[i] {
                    rgb888(devkit[i], false).map(|c| c / 3)
                } else {
                    [255, 0, 255]
                },
            );
        }
    }
    let fail = |e: &dyn std::fmt::Display| format!("writing {}: {e}", path.display());
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| fail(&e))?;
    }
    let file = File::create(path).map_err(|e| fail(&e))?;
    let mut encoder = png::Encoder::new(BufWriter::new(file), width as u32, SIZE as u32);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|e| fail(&e))?;
    writer.write_image_data(&rgb).map_err(|e| fail(&e))?;
    writer.finish().map_err(|e| fail(&e))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The follower reads the dev kit's `Export` bytes straight into its
    /// own: the layout must be the watch's. (On the watch: `llvm-nm -S`
    /// on a `compare` build gives `UR_COMPARE`'s size.)
    #[test]
    fn the_export_is_laid_out_as_on_the_watch() {
        assert_eq!(size_of::<Export>(), 0x950);
        assert_eq!(size_of::<Event>(), 32);
        assert_eq!(
            std::mem::offset_of!(Export, events),
            0x950 - compare::EVENTS * size_of::<Event>()
        );
    }
}
