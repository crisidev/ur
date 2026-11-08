//! Host tests for the firmware's `gesture` module: telling taps, long
//! presses, and swipes apart. `cargo test` runs them.
//!
//! Touches take time, so these run on embassy's mock clock, moved by hand.
//! It's one clock for the whole process, which the tests take turns with
//! (see `clock`): they run on threads of their own, and one moving it
//! during another's touch would lengthen that touch.

#[path = "../../../src/gesture.rs"]
#[allow(dead_code)]
mod gesture;

/// The one thing `gesture` needs from the canvas.
mod canvas {
    pub const SCALE: usize = 3;
}

fn main() {
    println!("run `cargo test`");
}

#[cfg(test)]
mod tests {
    use std::sync::{Mutex, MutexGuard};

    use embassy_time::{Duration, MockDriver};

    use super::gesture::*;

    /// The clock, to this test alone until the guard drops.
    fn clock() -> MutexGuard<'static, ()> {
        static CLOCK: Mutex<()> = Mutex::new(());
        // A test that failed holding it doesn't spoil the clock.
        CLOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn wait(ms: u64) {
        MockDriver::get().advance(Duration::from_millis(ms));
    }

    /// A finger that goes down at `from`, moves to `to`, and lifts after
    /// `ms`: what it was, as of the lift.
    fn touch(from: (i32, i32), to: (i32, i32), ms: u64) -> Touch {
        let mut tracking = Tracking::new(from);
        wait(ms / 2);
        tracking.moved_to(to);
        wait(ms - ms / 2);
        tracking.classify()
    }

    #[test]
    fn a_short_touch_in_place_is_a_tap() {
        let _clock = clock();
        let tap = touch((120, 120), (120, 120), 80);
        assert_eq!(tap.gesture, Gesture::Tap);
        // In canvas pixels, from where it started.
        assert_eq!((tap.x, tap.y), (40, 40));
        assert_eq!(tap.held_ms, 80);
        // Taps drift a little.
        let drift = SWIPE_DISTANCE - 1;
        assert_eq!(
            touch((120, 120), (120 + drift, 120 - drift), 80).gesture,
            Gesture::Tap
        );
    }

    #[test]
    fn swipes_are_named_for_the_way_the_finger_moved() {
        let _clock = clock();
        let far = SWIPE_DISTANCE;
        assert_eq!(
            touch((120, 120), (120 - far, 120), 150).gesture,
            Gesture::SwipeLeft
        );
        assert_eq!(
            touch((120, 120), (120 + far, 120), 150).gesture,
            Gesture::SwipeRight
        );
        assert_eq!(
            touch((120, 120), (120, 120 - far), 150).gesture,
            Gesture::SwipeUp
        );
        assert_eq!(
            touch((120, 120), (120, 120 + far), 150).gesture,
            Gesture::SwipeDown
        );
        // Mostly sideways is sideways.
        assert_eq!(touch((120, 120), (60, 90), 150).gesture, Gesture::SwipeLeft);
    }

    #[test]
    fn a_long_press_is_due_once_held_still_long_enough() {
        let _clock = clock();
        let mut tracking = Tracking::new((120, 120));
        let due = tracking
            .long_press_at()
            .expect("a still finger can become one");
        assert_eq!(due - tracking.since, LONG_PRESS);
        wait(LONG_PRESS.as_millis());
        // The touch task reports it then, while the finger is down, and
        // marks the touch spent.
        assert_eq!(tracking.classify().gesture, Gesture::LongPress);
        tracking.held = true;
        assert_eq!(tracking.long_press_at(), None);
    }

    #[test]
    fn a_lift_that_beat_the_timer_is_still_a_long_press() {
        let _clock = clock();
        assert_eq!(
            touch((120, 120), (121, 120), LONG_PRESS.as_millis()).gesture,
            Gesture::LongPress
        );
    }

    #[test]
    fn a_finger_that_strayed_is_never_a_long_press() {
        let _clock = clock();
        let mut tracking = Tracking::new((120, 120));
        tracking.moved_to((120 + SWIPE_DISTANCE, 120));
        assert_eq!(tracking.long_press_at(), None);
        // Back where it started: not a swipe, and not a long press either.
        tracking.moved_to((120, 120));
        wait(LONG_PRESS.as_millis() + 100);
        assert_eq!(tracking.long_press_at(), None);
        assert_eq!(tracking.classify().gesture, Gesture::Tap);
    }
}
