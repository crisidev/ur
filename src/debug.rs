//! Hooks for looking at and driving the UI from a computer, over the debug
//! probe (SWD): `scripts/screenshot.py` reads the canvas and turns it into
//! a PNG, and `scripts/screenshot.py --input …` injects taps, swipes, and
//! button presses. Together they let screens be checked without anyone
//! looking at (or touching) the watch.
//!
//! Both work through exported statics at fixed symbol names, which the
//! script finds in the ELF. They're inert without a probe; the cost is
//! [`input_task`]'s poll, ten wake-ups of a few µs a second.

use core::sync::atomic::{AtomicPtr, AtomicU8, AtomicU16, AtomicU32, Ordering};

use embassy_time::Timer;

use crate::canvas::Canvas;
use crate::touch::{Gesture, Touch};
use crate::{INPUT, InputEvent};

/// Where the canvas is (it's in a `StaticCell`, whose layout isn't ours).
#[unsafe(no_mangle)]
static UR_CANVAS: AtomicPtr<Canvas> = AtomicPtr::new(core::ptr::null_mut());

/// An input to inject: 0 for none; otherwise the low byte is the kind (1
/// button, pressed twice 0.2 s apart (a double press) if `x` is 2, 2 tap
/// (held for the top byte's milliseconds, if not 0, to try taps too brief
/// to be deliberate; see [`crate::wake::deliberate`]), 3 long press, 4–7 swipe up, down, left, right, 8 a
/// glance, 9 a test notification (`x` picks the sender, `y` how many hours
/// ago it arrived), 10 hang (spin forever, to test what the
/// watchdog records), 11 a `defmt::panic!`, 12 a `core::panic!` as from a
/// SoftDevice Controller fault (to test what a panic records), 13 a battery
/// voltage, `x` + 256 × `y` millivolts, from now on instead of the measured
/// one (0 for the measured one again; to test what a critically low battery
/// does, see [`crate::system_off`]), 14 media from the phone, playing if
/// `x` is 1, paused if 0, and with no word on whether it plays if 2 (as
/// after the watch restarts mid-song; for screenshots of the music screen
/// and the clock's media icon; gone once the phone disconnects), 15 `x` + 256 × `y`
/// steps, as if walked now (to test what keeps them across resets and
/// updates), 16 a simulated finger down at, or moved to, (`x`, `y`), held
/// there until 17 lifts it (see [`input_task`])) and the next two the
/// canvas x and y. Cleared once taken.
#[unsafe(no_mangle)]
static UR_INPUT: AtomicU32 = AtomicU32::new(0);

/// The battery voltage injected by input 13, in millivolts, or 0.
static BATTERY_MV: AtomicU16 = AtomicU16::new(0);

/// How many hours ago the next test notification arrived (input 9's `y`),
/// to see how older ones show in the history; taken by
/// [`take_backdate_minutes`].
static BACKDATE_HOURS: AtomicU8 = AtomicU8::new(0);

/// How many minutes before now a notification arriving now should count
/// as having arrived: 0, bar a backdated test notification.
pub fn take_backdate_minutes() -> i64 {
    i64::from(BACKDATE_HOURS.swap(0, Ordering::Relaxed)) * 60
}

/// The battery voltage to use instead of the measured one, if one was
/// injected, as a raw SAADC sample (see [`crate::battery::Battery::update`]).
pub fn battery_sample() -> Option<i16> {
    match BATTERY_MV.load(Ordering::Relaxed) {
        0 => None,
        mv => Some((u32::from(mv) * 4096 / (3600 * 2)) as i16),
    }
}

pub fn register_canvas(canvas: &mut Canvas) {
    UR_CANVAS.store(canvas, Ordering::Relaxed);
}

/// Turns injected inputs into real ones.
///
/// Most go straight to the UI loop, as the gesture the touch task would
/// have recognised. A simulated finger (inputs 16 and 17) instead goes
/// through the touch task, as the chip's reports, to test what it makes
/// of a touch over time: a long press is recognised while the finger is
/// held, for one. Like the chip, this repeats the finger's position while
/// it's down (each poll, rather than every ~10 ms), or the touch task
/// would take the silence for a missed lift.
#[embassy_executor::task]
pub async fn input_task() {
    // The simulated finger, in panel pixels, while it's down.
    let mut finger: Option<(u8, u8)> = None;
    loop {
        Timer::after_millis(100).await;
        if finger.is_some() {
            crate::touch::SIMULATED.signal(finger);
        }
        let word = UR_INPUT.swap(0, Ordering::Relaxed);
        if word == 0 {
            continue;
        }
        let [kind, x, y, held_ms] = word.to_le_bytes();
        let gesture = match kind {
            1 => {
                INPUT.send(InputEvent::Button).await;
                if x == 2 {
                    Timer::after_millis(200).await;
                    INPUT.send(InputEvent::Button).await;
                }
                continue;
            }
            8 => {
                INPUT.send(InputEvent::Glance).await;
                continue;
            }
            // The way a dependency's `defmt::panic!` or `defmt::unwrap!`
            // fails, for testing the panic breadcrumb's backtrace.
            11 => defmt::panic!("debug: panicking on purpose"),
            // The way a fault in Nordic's SoftDevice Controller fails: a
            // `core::panic!` with a formatted message (the assert's file and
            // line, here `y`), for testing that the breadcrumb keeps it. `x`
            // 1 makes the message too long to keep whole.
            12 => {
                let file = if x == 1 {
                    c"debug: a file name long enough that the message is cut short"
                } else {
                    c"debug"
                };
                // SAFETY: A NUL-terminated string, as the handler needs.
                unsafe { crate::field_log::sdc_fault(file.as_ptr(), u32::from(y)) };
                continue;
            }
            16 | 17 => {
                let scale = crate::canvas::SCALE as u8;
                finger = (kind == 16).then(|| (x.min(79) * scale, y.min(79) * scale));
                defmt::warn!("debug: finger at {}", finger);
                crate::touch::SIMULATED.signal(finger);
                continue;
            }
            15 => {
                let steps = u16::from_le_bytes([x, y]);
                defmt::warn!("debug: {} steps", steps);
                crate::activity::walk(u32::from(steps));
                continue;
            }
            13 => {
                let mv = u16::from_le_bytes([x, y]);
                defmt::warn!("debug: battery at {} mV from now on (0: measured)", mv);
                BATTERY_MV.store(mv, Ordering::Relaxed);
                continue;
            }
            14 => {
                let mut artist = heapless::String::new();
                let mut track = heapless::String::new();
                let _ = artist.push_str("Ida Moreau");
                let _ = track.push_str("Night Signals");
                // Not awaiting room in the queue: holding the two events
                // across an await would grow this task's static future by
                // 200 bytes, all for a debugging aid.
                let _ = INPUT.try_send(InputEvent::Phone(crate::ui::PhoneEvent::MusicInfo {
                    artist,
                    track,
                }));
                if x != 2 {
                    let state = crate::ui::PhoneEvent::MusicState { playing: x == 1 };
                    let _ = INPUT.try_send(InputEvent::Phone(state));
                }
                continue;
            }
            10 => {
                defmt::warn!("debug: hanging on purpose");
                #[allow(clippy::empty_loop)]
                loop {}
            }
            9 => {
                let mut app = heapless::String::new();
                let mut who = heapless::String::new();
                // `x` picks the sender, to check how each length lays out.
                let (from, sender) = match x {
                    1 => ("Podcasts", "Night Signals: Ida Moreau"),
                    // As long as a sender gets (`ui::PhoneEvent`'s 40).
                    2 => ("Calendar", "Quarterly planning: platform team, 4B"),
                    3 => ("Signal", "Nora Lindqvist"),
                    // Accents, and a hyphenated word.
                    4 => ("Signal", "Åse Kristiansen"),
                    // One line, even at 2×.
                    5 => ("Messages", "Ida"),
                    _ => ("Signal", "Jane Smith"),
                };
                let _ = app.push_str(from);
                let _ = who.push_str(sender);
                BACKDATE_HOURS.store(y, Ordering::Relaxed);
                let notification = crate::ui::PhoneEvent::Notification { app, who };
                INPUT.send(InputEvent::Phone(notification)).await;
                continue;
            }
            2 => Gesture::Tap,
            3 => Gesture::LongPress,
            4 => Gesture::SwipeUp,
            5 => Gesture::SwipeDown,
            6 => Gesture::SwipeLeft,
            7 => Gesture::SwipeRight,
            _ => continue,
        };
        let touch = Touch {
            gesture,
            x: i32::from(x),
            y: i32::from(y),
            // A finger's typical tap (and a long press's, for that), or
            // as asked.
            held_ms: match (gesture, held_ms) {
                (Gesture::LongPress, _) => 800,
                (_, 0) => 80,
                (_, held_ms) => u16::from(held_ms),
            },
        };
        INPUT.send(InputEvent::Touch(touch)).await;
    }
}
