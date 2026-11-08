//! The window: the panel in a bezel, the mouse as a finger, the keyboard
//! for the button and the phone, and the terminal for every other command.
//!
//! Time runs at the wall clock's pace (or `--speed` times it). The panel
//! is shown as the backlight would light it: off is black, and the dimmer
//! levels darker. The bezel lights up while the motor runs.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::{Duration as StdDuration, Instant as StdInstant};

use embassy_time::Duration;
use minifb::{Key, KeyRepeat, MouseButton, MouseMode, Scale, Window, WindowOptions};

use crate::brightness::{self, BY_CURRENT};
use crate::cli::Options;
use crate::display::{PANEL, SIZE, rgb888};
use crate::gesture::{Gesture, Touch};
use crate::script::{self, Next};
use crate::ui::PhoneEvent;
use crate::watch::Watch;
use crate::{InputEvent, clock};

/// The bezel around the panel, in panel pixels.
const BEZEL: usize = 16;
/// The side button, on the right of the bezel: its rows.
const BUTTON_ROWS: std::ops::Range<usize> = 100..156;
const WIDTH: usize = SIZE + 2 * BEZEL;
const HEIGHT: usize = SIZE + 2 * BEZEL;
const BEZEL_COLOR: u32 = 0x202020;
const BUZZ_COLOR: u32 = 0xc08020;
const BUTTON_COLOR: u32 = 0x484848;
/// How often to show a frame.
const FRAME: StdDuration = StdDuration::from_millis(16);

const KEYS: &str = "\
Window: the mouse is a finger (hold still 0.6 s for a long press, drag to
swipe); click the bezel's right edge, or Space, for the button.
  G glance   N notification   M music play/pause   C call/hang up
  arrows swipe   T tap   L long press (at the middle)
  F a minute on   H an hour on   S screenshot   Q quit
The terminal takes commands (`help` lists them).
";

struct Screen {
    window: Window,
    buffer: Vec<u32>,
    /// The backlight's brightness, 0 (off) to 1, and the bezel's colour,
    /// as of the last whole frame: a transition's frames keep them.
    light: f32,
    bezel: u32,
    shown: StdInstant,
}

impl Screen {
    fn draw(&mut self) {
        let (light, bezel) = (self.light, self.bezel);
        self.buffer.fill(bezel);
        for y in BUTTON_ROWS {
            for x in WIDTH - BEZEL / 2..WIDTH {
                self.buffer[y * WIDTH + x] = BUTTON_COLOR;
            }
        }
        PANEL.with_borrow(|panel| {
            for y in 0..SIZE {
                for x in 0..SIZE {
                    let [r, g, b] = rgb888(panel.pixels[y * SIZE + x], panel.idle)
                        .map(|c| (f32::from(c) * light) as u32);
                    self.buffer[(y + BEZEL) * WIDTH + x + BEZEL] = r << 16 | g << 8 | b;
                }
            }
        });
    }

    fn show(&mut self) -> Result<(), String> {
        self.draw();
        self.shown = StdInstant::now();
        self.window
            .update_with_buffer(&self.buffer, WIDTH, HEIGHT)
            .map_err(|e| format!("showing the window: {e}"))
    }
}

pub fn run(mut watch: Watch, options: &Options) -> Result<(), String> {
    let mut window = Window::new(
        "ur",
        WIDTH,
        HEIGHT,
        WindowOptions {
            scale: Scale::X2,
            ..WindowOptions::default()
        },
    )
    .map_err(|e| format!("opening the window: {e}"))?;
    // Paced here instead: a transition shows frames from within the watch.
    window.set_target_fps(0);
    let screen = Rc::new(RefCell::new(Screen {
        window,
        buffer: vec![0; WIDTH * HEIGHT],
        light: 0.0,
        bezel: BEZEL_COLOR,
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
            let (x, y) = (mx as i32 - BEZEL as i32, my as i32 - BEZEL as i32);
            let on_panel = (0..SIZE as i32).contains(&x) && (0..SIZE as i32).contains(&y);
            let on_button = mx as usize >= WIDTH - BEZEL && BUTTON_ROWS.contains(&(my as usize));
            match (pressed, mouse_down) {
                (true, false) if on_button => watch.input(InputEvent::Button),
                (true, false) if on_panel => {
                    watch.finger_at(x, y);
                    mouse_down = true;
                }
                (true, true) => watch.finger_at(x, y),
                (false, true) => {
                    watch.lift();
                    mouse_down = false;
                }
                _ => {}
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
            screen.light = light(watch.backlight);
            screen.bezel = if watch.motor_on() {
                BUZZ_COLOR
            } else {
                BEZEL_COLOR
            };
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

/// How bright a backlight level looks: off is black, and the dimmest
/// still readable.
fn light(level: brightness::Level) -> f32 {
    match BY_CURRENT.iter().position(|&l| l == level) {
        Some(rank) => 0.5 + 0.5 * rank as f32 / (BY_CURRENT.len() - 1) as f32,
        None => 0.0,
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
    format!("ur · {time} · {screen}{night}{phone}")
}
