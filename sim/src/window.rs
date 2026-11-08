//! The window: the watch, drawn to scale, with the mouse as a finger, the
//! keyboard for the button and the phone, and the terminal for every other
//! command.
//!
//! The watch is drawn as the PineTime is built (see [`Case`]): a black
//! 37.5 × 40 mm case, the 1.3″ panel behind black glass, and the button on
//! the left edge. Time runs at the wall clock's pace (or `--speed` times
//! it). The panel is shown as the backlight would light it: off is black,
//! and the dimmer levels darker. The watch shakes while the motor runs.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::{Duration as StdDuration, Instant as StdInstant};

use embassy_time::{Duration, Instant};
use minifb::{Key, KeyRepeat, MouseButton, MouseMode, Window, WindowOptions};

use crate::brightness;
use crate::case::Case;
use crate::cli::Options;
use crate::gesture::{Gesture, Touch};
use crate::script::{self, Next};
use crate::ui::PhoneEvent;
use crate::watch::Watch;
use crate::{InputEvent, clock};

/// How often to show a frame.
const FRAME: StdDuration = StdDuration::from_millis(16);

const KEYS: &str = "\
Window: the mouse is a finger (hold still 0.6 s for a long press, drag to
swipe); click the button on the watch's left edge, or press Space, for the
button.
  G glance   N notification   M music play/pause   C call/hang up
  arrows swipe   T tap   L long press (at the middle)
  F a minute on   H an hour on   S screenshot   R record (again: stop)
  P the panel's colours: as the real panel shows them, or as sent to it
  Q quit
The terminal takes commands (`help` lists them).
";

struct Screen {
    window: Window,
    case: Case,
    buffer: Vec<u32>,
    /// The backlight's brightness, 0 (off) to 1, and how far the motor has
    /// shaken the watch, in window pixels, as of the last whole frame: a
    /// transition's frames keep them.
    light: f32,
    shake: f32,
    /// The real panel's colours, or those sent to it (see `crate::panel`).
    mimic: bool,
    shown: StdInstant,
}

impl Screen {
    fn draw(&mut self) {
        self.case
            .draw(&mut self.buffer, self.light, self.shake, self.mimic);
    }

    fn show(&mut self) -> Result<(), String> {
        self.draw();
        self.shown = StdInstant::now();
        self.window
            .update_with_buffer(&self.buffer, self.case.width, self.case.height)
            .map_err(|e| format!("showing the window: {e}"))
    }
}

pub fn run(mut watch: Watch, options: &Options) -> Result<(), String> {
    let case = Case::new(options.zoom);
    let mut window = Window::new("ur", case.width, case.height, WindowOptions::default())
        .map_err(|e| format!("opening the window: {e}"))?;
    // Paced here instead: a transition shows frames from within the watch.
    window.set_target_fps(0);
    let screen = Rc::new(RefCell::new(Screen {
        window,
        buffer: vec![0; case.width * case.height],
        case,
        light: 0.0,
        shake: 0.0,
        mimic: options.mimic,
        shown: StdInstant::now(),
    }));
    eprint!("{KEYS}");

    // A transition's frames: shown as they come, at the pace they'd take
    // on the watch.
    let speed = options.speed;
    watch.on_frame = Some(Box::new({
        let screen = Rc::clone(&screen);
        move || {
            std::thread::sleep(StdDuration::from_secs_f64(0.001 / speed));
            let mut screen = screen.borrow_mut();
            if screen.shown.elapsed() >= FRAME {
                // A frame lost now and then is no loss.
                let _ = screen.show();
            }
        }
    }));

    // The terminal's commands, read on a thread of their own.
    let (commands, typed) = mpsc::channel::<String>();
    std::thread::spawn(move || {
        for line in std::io::stdin().lines() {
            let Ok(line) = line else { break };
            if commands.send(line).is_err() {
                break;
            }
        }
    });

    let mut mouse_down = false;
    // The mouse is holding the button down.
    let mut button_down = false;
    let mut last = StdInstant::now();
    let mut title = String::new();
    let mut notification = 0;
    let mut shots = 0;
    loop {
        // What the wearer did.
        let (open, keys, mouse, pressed) = {
            let screen = screen.borrow();
            let window = &screen.window;
            (
                window.is_open(),
                window.get_keys_pressed(KeyRepeat::No),
                window.get_mouse_pos(MouseMode::Clamp),
                window.get_mouse_down(MouseButton::Left),
            )
        };
        if !open {
            return Ok(());
        }
        if let Some((mx, my)) = mouse {
            let (on_panel, on_button) = {
                let case = &screen.borrow().case;
                (case.on_panel(mx, my), case.on_button(mx, my))
            };
            match (pressed, mouse_down, button_down) {
                // The button, pressed once until the mouse lets go of it.
                (true, false, true) => {}
                (true, false, false) if on_button => {
                    watch.input(InputEvent::Button);
                    button_down = true;
                }
                (true, false, false) => {
                    if let Some((x, y)) = on_panel {
                        watch.finger_at(x, y);
                        mouse_down = true;
                    }
                }
                // Off the panel, the finger stays where it left it.
                (true, true, _) => {
                    if let Some((x, y)) = on_panel {
                        watch.finger_at(x, y);
                    }
                }
                (false, true, _) => {
                    watch.lift();
                    mouse_down = false;
                }
                (false, false, _) => button_down = false,
            }
        }
        for key in keys {
            let gesture = |gesture| {
                InputEvent::Touch(Touch {
                    gesture,
                    x: 40,
                    y: 40,
                    held_ms: if gesture == Gesture::LongPress {
                        800
                    } else {
                        80
                    },
                })
            };
            match key {
                Key::Space | Key::Enter | Key::B => watch.input(InputEvent::Button),
                Key::G => watch.input(InputEvent::Glance),
                Key::T => watch.input(gesture(Gesture::Tap)),
                Key::L => watch.input(gesture(Gesture::LongPress)),
                Key::Up => watch.input(gesture(Gesture::SwipeUp)),
                Key::Down => watch.input(gesture(Gesture::SwipeDown)),
                Key::Left => watch.input(gesture(Gesture::SwipeLeft)),
                Key::Right => watch.input(gesture(Gesture::SwipeRight)),
                Key::N => {
                    command(&mut watch, &format!("notify {notification}"), options);
                    notification = (notification + 1) % 6;
                }
                Key::M => music(&mut watch),
                Key::C => call(&mut watch),
                Key::F => watch.advance(Duration::from_secs(60)),
                Key::H => watch.advance(Duration::from_secs(3600)),
                Key::S => {
                    shots += 1;
                    command(&mut watch, &format!("shot ur-sim-{shots}.png"), options);
                }
                // On the watch's clock, as a script's: the GIF plays at
                // the watch's pace, whatever `--speed`.
                Key::R if crate::record::active() => {
                    command(&mut watch, "stop", options);
                }
                Key::R => {
                    let name = crate::record::timestamped();
                    command(&mut watch, &format!("record {name}"), options);
                }
                Key::P => {
                    let mut screen = screen.borrow_mut();
                    screen.mimic = !screen.mimic;
                    let colours = if screen.mimic {
                        "as the real panel shows them"
                    } else {
                        "as sent to the panel"
                    };
                    eprintln!("colours: {colours}");
                }
                Key::Q | Key::Escape => return Ok(()),
                _ => {}
            }
        }
        while let Ok(line) = typed.try_recv() {
            if let Next::Quit = command(&mut watch, &line, options) {
                return Ok(());
            }
        }

        // Time passes.
        let elapsed = last.elapsed();
        last = StdInstant::now();
        watch.advance(Duration::from_micros(
            (elapsed.as_secs_f64() * options.speed * 1e6) as u64,
        ));

        // And the watch shows it.
        {
            let mut screen = screen.borrow_mut();
            screen.light = crate::case::light(watch.backlight);
            screen.shake = screen.case.shake(watch.motor_pulse(), Instant::now());
            screen.show()?;
            let now = status(&watch);
            if now != title {
                screen.window.set_title(&now);
                title = now;
            }
        }
        std::thread::sleep(FRAME.saturating_sub(last.elapsed()));
    }
}

/// Runs a command, reporting (rather than stopping at) errors.
fn command(watch: &mut Watch, line: &str, options: &Options) -> Next {
    match script::execute(watch, line, &options.output) {
        Ok(next) => next,
        Err(e) => {
            eprintln!("{e}");
            Next::Continue
        }
    }
}

/// Plays or pauses; the first time, the phone starts playing something.
fn music(watch: &mut Watch) {
    use std::sync::atomic::{AtomicBool, Ordering};
    static PLAYING: AtomicBool = AtomicBool::new(false);
    static KNOWN: AtomicBool = AtomicBool::new(false);
    if !KNOWN.swap(true, Ordering::Relaxed) {
        let mut info = (heapless::String::new(), heapless::String::new());
        let _ = info.0.push_str("Ida Moreau");
        let _ = info.1.push_str("Night Signals");
        watch.input(InputEvent::Phone(PhoneEvent::MusicInfo {
            artist: info.0,
            track: info.1,
        }));
    }
    let playing = !PLAYING.fetch_xor(true, Ordering::Relaxed);
    watch.input(InputEvent::Phone(PhoneEvent::MusicState { playing }));
}

/// A call rings, or the one ringing ends.
fn call(watch: &mut Watch) {
    use std::sync::atomic::{AtomicBool, Ordering};
    static RINGING: AtomicBool = AtomicBool::new(false);
    let event = if RINGING.fetch_xor(true, Ordering::Relaxed) {
        PhoneEvent::CallOver
    } else {
        let mut caller = heapless::String::new();
        let _ = caller.push_str("Nora Lindqvist");
        PhoneEvent::IncomingCall { caller }
    };
    watch.input(InputEvent::Phone(event));
}

/// The title bar: the watch's time, and what's on.
fn status(watch: &Watch) -> String {
    let time = clock::now().map_or("--:--".to_owned(), |t| {
        format!("{:02}:{:02}", t.hour, t.minute)
    });
    let screen = match (watch.off, watch.backlight == brightness::Level::OFF) {
        (Some(_), _) => "switched off",
        (None, true) => "screen off",
        (None, false) if watch.app.dim() => "dim",
        (None, false) => "screen on",
    };
    let night = if watch.app.quiet_night() {
        ", quiet night"
    } else {
        ""
    };
    let phone = if watch.connected { "" } else { ", no phone" };
    let recording = if crate::record::active() {
        " · recording"
    } else {
        ""
    };
    format!("ur · {time} · {screen}{night}{phone}{recording}")
}
