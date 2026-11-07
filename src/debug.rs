//! Hooks for looking at and driving the UI from a computer, over the debug
//! probe (SWD): `scripts/screenshot.py` reads the canvas and turns it into
//! a PNG, and `scripts/screenshot.py --input …` injects taps, swipes, and
//! button presses. Together they let screens be checked without anyone
//! looking at (or touching) the watch.
//!
//! Both work through exported statics at fixed symbol names, which the
//! script finds in the ELF. They're inert without a probe; the cost is
//! [`input_task`]'s poll, ten wake-ups of a few µs a second.

use core::sync::atomic::{AtomicPtr, AtomicU32, Ordering};

use embassy_time::Timer;

use crate::canvas::Canvas;
use crate::touch::{Gesture, Touch};
use crate::{INPUT, InputEvent};

/// Where the canvas is (it's in a `StaticCell`, whose layout isn't ours).
#[unsafe(no_mangle)]
static UR_CANVAS: AtomicPtr<Canvas> = AtomicPtr::new(core::ptr::null_mut());

/// An input to inject: 0 for none; otherwise the low byte is the kind (1
/// button, 2 tap, 3 long press, 4–7 swipe up, down, left, right, 8 a
/// glance, 9 a test notification, 10 hang (spin forever, to test what the
/// watchdog records)) and the next two the canvas x and y. Cleared once
/// taken.
#[unsafe(no_mangle)]
static UR_INPUT: AtomicU32 = AtomicU32::new(0);

pub fn register_canvas(canvas: &mut Canvas) {
    UR_CANVAS.store(canvas, Ordering::Relaxed);
}

/// Turns injected inputs into real ones.
#[embassy_executor::task]
pub async fn input_task() {
    loop {
        Timer::after_millis(100).await;
        let word = UR_INPUT.swap(0, Ordering::Relaxed);
        if word == 0 {
            continue;
        }
        let [kind, x, y, _] = word.to_le_bytes();
        let gesture = match kind {
            1 => {
                INPUT.send(InputEvent::Button).await;
                continue;
            }
            8 => {
                INPUT.send(InputEvent::Glance).await;
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
                    _ => ("Signal", "Jane Smith"),
                };
                let _ = app.push_str(from);
                let _ = who.push_str(sender);
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
        };
        INPUT.send(InputEvent::Touch(touch)).await;
    }
}
