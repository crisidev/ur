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
        // Its lift is a release, from where it started, after all the
        // time it was down.
        wait(900);
        tracking.moved_to((150, 150));
        let release = tracking.release();
        assert_eq!(release.gesture, Gesture::Release);
        assert_eq!((release.x, release.y), (40, 40));
        assert_eq!(u64::from(release.held_ms), LONG_PRESS.as_millis() + 900);
    }

    #[test]
    fn a_lift_that_beat_the_timer_is_still_a_long_press() {
        let _clock = clock();
        assert_eq!(
            touch((120, 120), (121, 120), LONG_PRESS.as_millis()).gesture,
            Gesture::LongPress
        );
    }

    /// What a touch's lift reports, by gesture: none but a long press's
    /// has a release.
    fn lifted(tracking: &Tracking) -> (Gesture, Option<Gesture>) {
        let (event, release) = tracking.lifted();
        (event.gesture, release.map(|release| release.gesture))
    }

    #[test]
    fn a_long_press_is_released_whenever_it_was_seen() {
        let _clock = clock();
        // Reported while held: the lift is only its release.
        let mut held = Tracking::new((120, 120));
        wait(LONG_PRESS.as_millis());
        held.held = true;
        wait(900);
        assert_eq!(lifted(&held), (Gesture::Release, None));
        // A lift that beat the timer (the touch task's lift report read
        // just after it was due): the long press, then its release, from
        // where it started, after all the time it was down. This once
        // sent no release, and a button grid hold never ended.
        let beaten = Tracking::new((120, 120));
        wait(LONG_PRESS.as_millis() + 5);
        assert_eq!(
            lifted(&beaten),
            (Gesture::LongPress, Some(Gesture::Release))
        );
        let (_, release) = beaten.lifted();
        let release = release.expect("a long press has one");
        assert_eq!((release.x, release.y), (40, 40));
        assert_eq!(u64::from(release.held_ms), LONG_PRESS.as_millis() + 5);
        // Anything else has none.
        let tap = Tracking::new((120, 120));
        wait(LONG_PRESS.as_millis() - 10);
        assert_eq!(lifted(&tap), (Gesture::Tap, None));
        let mut swipe = Tracking::new((120, 120));
        swipe.moved_to((120, 120 + SWIPE_DISTANCE));
        wait(LONG_PRESS.as_millis() + 100);
        assert_eq!(lifted(&swipe), (Gesture::SwipeDown, None));
    }

    #[test]
    fn a_touch_cut_short_reports_only_a_long_press_release() {
        let _clock = clock();
        // Put to sleep before it was anything: nothing to say.
        let short = Tracking::new((120, 120));
        wait(LONG_PRESS.as_millis() + 5);
        assert!(short.lost().is_none());
        // A long press reported while held (a button grid hold, past the
        // screen's timeout in the quiet night): its release goes now, as
        // its lift never will. This once sent nothing, and the hold ran
        // on.
        let mut held = Tracking::new((120, 120));
        wait(LONG_PRESS.as_millis());
        held.held = true;
        wait(10_000);
        let release = held.lost().expect("a long press is released");
        assert_eq!(release.gesture, Gesture::Release);
        assert_eq!((release.x, release.y), (40, 40));
        assert_eq!(u64::from(release.held_ms), LONG_PRESS.as_millis() + 10_000);
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
