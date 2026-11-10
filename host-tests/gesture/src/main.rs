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

/// What `gesture` needs from the canvas: its size, and the panel's
/// pixels to each of its own.
mod canvas {
    pub const WIDTH: usize = 80;
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

    /// A hold keeps the screen on until it's released, but no longer than
    /// [`HOLD_LIMIT`]; without one, the screen's own timeout stands.
    #[test]
    fn a_hold_keeps_the_screen_on_up_to_the_limit() {
        let ten = Duration::from_secs(10);
        assert_eq!(screen_timeout(ten, false), ten);
        assert_eq!(screen_timeout(ten, true), HOLD_LIMIT);
        // Longer timeouts (the music screen's, Home Assistant's key)
        // aren't cut short by a hold.
        let long = HOLD_LIMIT + Duration::from_secs(1);
        assert_eq!(screen_timeout(long, true), long);
        assert_eq!(screen_timeout(Duration::MAX, true), Duration::MAX);
        // Well above any hold a person means, and within what the touch
        // controller's limit (in whole seconds, a byte) can say.
        assert!(HOLD_LIMIT >= Duration::from_secs(30));
        assert!(HOLD_LIMIT.as_secs() <= u64::from(u8::MAX));
        assert_eq!(HOLD_LIMIT.as_ticks() % Duration::from_secs(1).as_ticks(), 0);
    }
}

/// Proofs about telling touches apart, for every position the panel has
/// and every timing, where the tests above play chosen ones. `cargo kani`
/// runs them (one at a time with `--harness`: CBMC takes memory).
///
/// Positions are on the panel, 0–239 each way, which `touch::touch_task`
/// sees to before anything gets here (it drops the controller's misreads);
/// see `positions_off_the_panel_would_overflow` for what that's worth.
///
/// The clock is a number of the proofs' own, [`proofs::now`], stubbed in
/// for `Instant::now` (hence `stubbing` in `Cargo.toml`): embassy finds
/// its time driver, the mock one included, through an `extern "Rust"`
/// function, which Kani can't follow.
#[cfg(kani)]
mod proofs {
    use embassy_time::Instant;

    use super::gesture::*;

    /// `touch::PANEL`.
    const PANEL: i32 = 240;
    /// The longest a proof lets the clock move at once, in ticks: about
    /// 73,000 years, so a few of them can't overflow it.
    const LONGEST: u64 = u64::MAX / 8;

    fn any_position() -> (i32, i32) {
        let (x, y): (i32, i32) = kani::any();
        kani::assume((0..PANEL).contains(&x) && (0..PANEL).contains(&y));
        (x, y)
    }

    /// The time, in ticks (microseconds).
    static mut CLOCK: u64 = 0;

    /// `Instant::now`, by [`CLOCK`]. (Only named in `kani::stub`s, which
    /// the compiler doesn't count as a use.)
    #[allow(dead_code)]
    fn now() -> Instant {
        // SAFETY: a proof is one thread.
        Instant::from_ticks(unsafe { CLOCK })
    }

    /// Any time passes; returns how much, in ticks.
    fn wait_any() -> u64 {
        let ticks: u64 = kani::any();
        kani::assume(ticks <= LONGEST);
        // SAFETY: a proof is one thread.
        unsafe { CLOCK += ticks };
        ticks
    }

    /// Whether a finger from `start`, now at `at`, is past the tap
    /// tolerance: as far as a swipe, either way.
    fn strayed(start: (i32, i32), at: (i32, i32)) -> bool {
        let (dx, dy) = (at.0 - start.0, at.1 - start.1);
        dx <= -SWIPE_DISTANCE
            || dx >= SWIPE_DISTANCE
            || dy <= -SWIPE_DISTANCE
            || dy >= SWIPE_DISTANCE
    }

    /// A finger that goes down anywhere, moves anywhere three times at
    /// any pace, and lifts whenever. Nothing panics or overflows, and
    /// what it's told apart as is what the module docs say:
    ///
    /// - a swipe exactly when it ends as far as [`SWIPE_DISTANCE`] from
    ///   where it started, named for the way it went furthest (so a tap
    ///   or a long press is never also a swipe);
    /// - else a long press exactly when it lasted [`LONG_PRESS`] and never
    ///   strayed that far on the way, not even if it came back;
    /// - else a tap.
    ///
    /// And the touch is where it started, in canvas pixels (on the
    /// canvas, so `buttons::at` and the like get what they expect), with
    /// how long it lasted in whole milliseconds, saturating.
    #[kani::proof]
    #[kani::stub(Instant::now, now)]
    #[kani::unwind(4)]
    fn any_touch_is_told_apart_as_documented() {
        let start = any_position();
        let mut tracking = Tracking::new(start);
        let mut lasted: u64 = 0;
        let mut ever_strayed = false;
        let mut last = start;
        for _ in 0..3 {
            lasted += wait_any();
            last = any_position();
            tracking.moved_to(last);
            ever_strayed |= strayed(start, last);
        }
        lasted += wait_any();
        let touch = tracking.classify();

        let (dx, dy) = (last.0 - start.0, last.1 - start.1);
        let long_enough = lasted >= LONG_PRESS.as_ticks();
        // (In blocks: Kani's `assert!` isn't an expression.)
        match touch.gesture {
            Gesture::SwipeLeft => {
                assert!(dx <= -SWIPE_DISTANCE && -dx > dy && -dx > -dy);
            }
            Gesture::SwipeRight => {
                assert!(dx >= SWIPE_DISTANCE && dx > dy && dx > -dy);
            }
            Gesture::SwipeUp => {
                assert!(dy <= -SWIPE_DISTANCE && -dy >= dx && -dy >= -dx);
            }
            Gesture::SwipeDown => {
                assert!(dy >= SWIPE_DISTANCE && dy >= dx && dy >= -dx);
            }
            Gesture::LongPress => {
                assert!(!ever_strayed && long_enough);
            }
            Gesture::Tap => {
                assert!(!strayed(start, last) && (ever_strayed || !long_enough));
            }
            Gesture::Release => panic!("only `release` says so"),
        }
        kani::cover!(touch.gesture == Gesture::LongPress, "a long press");
        kani::cover!(
            touch.gesture == Gesture::Tap && ever_strayed && long_enough,
            "a tap that went away and came back"
        );

        assert_eq!((touch.x, touch.y), (start.0 / 3, start.1 / 3));
        assert!((0..80).contains(&touch.x) && (0..80).contains(&touch.y));
        // Whole milliseconds, rounded down, or all a `u16` holds.
        let ms = u64::from(touch.held_ms);
        assert!(ms * 1000 <= lasted);
        assert!(lasted < (ms + 1) * 1000 || touch.held_ms == u16::MAX);
    }

    /// When the touch driver's long press timer fires, the touch *is* a
    /// long press. `touch::touch_task` waits for [`Tracking::long_press_at`]
    /// and then sends whatever [`Tracking::classify`] says, as the
    /// touch's one event while it's still down, and its lift as a
    /// release: were that ever a tap or a swipe, it would be acted on
    /// with the finger still down, and followed by a release nothing
    /// expects.
    ///
    /// For a finger that has moved anywhere twice, at any pace: the timer
    /// is only offered while the touch can still be a long press (it
    /// hasn't strayed, nor been reported), it's [`LONG_PRESS`] after the
    /// touch started, and any time from then on the touch classifies as
    /// one.
    #[kani::proof]
    #[kani::stub(Instant::now, now)]
    #[kani::unwind(3)]
    fn the_long_press_timer_fires_on_a_long_press() {
        let start = any_position();
        let mut tracking = Tracking::new(start);
        let since = Instant::now();
        let mut ever_strayed = false;
        for _ in 0..2 {
            wait_any();
            let at = any_position();
            tracking.moved_to(at);
            ever_strayed |= strayed(start, at);
        }
        wait_any();
        let due = tracking.long_press_at();
        assert_eq!(due, (!ever_strayed).then_some(since + LONG_PRESS));
        if let Some(due) = due {
            kani::assume(Instant::now() >= due);
            assert_eq!(tracking.classify().gesture, Gesture::LongPress);
            // Once reported, it's spent: no second timer, and the lift
            // is a release, from where the touch started.
            tracking.held = true;
            assert_eq!(tracking.long_press_at(), None);
            let release = tracking.release();
            assert_eq!(release.gesture, Gesture::Release);
            assert_eq!((release.x, release.y), (start.0 / 3, start.1 / 3));
        }
    }

    /// What `touch::touch_task`'s check that a position is on the panel
    /// is worth: without it, [`Tracking::moved_to`] overflows (a panic
    /// where overflow is checked, as in these host crates; a wrapped
    /// distance, and so a wrong gesture, in the firmware's release
    /// build). Not from anything the controller can report, whose
    /// positions are 12 bits each: this only says the arithmetic isn't
    /// safe for every `i32`, should another caller ever feed it. With
    /// debug assertions on, as here, [`Tracking`] refuses such a position
    /// itself, before the arithmetic gets to it: either way, a panic.
    #[kani::proof]
    #[kani::stub(Instant::now, now)]
    #[kani::should_panic]
    fn positions_off_the_panel_would_overflow() {
        let mut tracking = Tracking::new(kani::any());
        tracking.moved_to(kani::any());
    }

    /// Every long press gets exactly one release, and nothing else does:
    /// what the button grid's holds stand on, as whatever a hold starts
    /// (a light dimming, in Home Assistant) only stops at its release.
    /// (Two firmware bugs were a release that never came: a lift just
    /// before the long press's timer, and a touch cut short by the
    /// controller going to sleep.)
    ///
    /// The touch is driven as `touch::touch_task` drives it, with its
    /// events counted in the order it would send them. Twice over,
    /// after any wait: either a report moves the finger anywhere, or,
    /// if the long press's timer is due, it may fire (the task's
    /// `select3` can as well take a report that's ready at the same
    /// moment, so the timer firing is a choice, not a must), which
    /// reports the long press with the finger down. Then the touch ends
    /// one of the two ways it can: the finger lifts
    /// ([`Tracking::lifted`]), or the controller is put to sleep under
    /// it ([`Tracking::lost`]).
    ///
    /// Whatever happens:
    ///
    /// - a release only ever follows a long press, and at most one of
    ///   each is sent;
    /// - what the timer reports is a long press;
    /// - a touch that lifts sends exactly one event that isn't a
    ///   release, and its release if (and only if) that was a long
    ///   press;
    /// - a touch cut short sends nothing but the release of a long
    ///   press already reported.
    #[kani::proof]
    #[kani::stub(Instant::now, now)]
    #[kani::unwind(3)]
    fn every_long_press_gets_exactly_one_release() {
        let mut tracking = Tracking::new(any_position());
        let (mut long_presses, mut releases, mut others) = (0u8, 0u8, 0u8);
        let mut count = |touch: Touch| match touch.gesture {
            Gesture::LongPress => {
                assert!(releases == 0, "a long press after its release");
                long_presses += 1;
            }
            Gesture::Release => {
                assert!(long_presses == 1, "a release with no long press");
                releases += 1;
            }
            _ => others += 1,
        };
        for _ in 0..2 {
            wait_any();
            if kani::any() {
                tracking.moved_to(any_position());
            } else if tracking
                .long_press_at()
                .is_some_and(|due| Instant::now() >= due)
            {
                // As `touch_task` does when its timer fires.
                tracking.held = true;
                let event = tracking.classify();
                assert_eq!(event.gesture, Gesture::LongPress);
                count(event);
            }
        }
        wait_any();
        let lifted: bool = kani::any();
        if lifted {
            let (event, release) = tracking.lifted();
            count(event);
            if let Some(release) = release {
                count(release);
            }
        } else if let Some(release) = tracking.lost() {
            count(release);
        }
        assert!(long_presses <= 1 && releases <= 1);
        if lifted {
            assert_eq!(long_presses + others, 1);
            assert_eq!(releases, long_presses);
        } else {
            assert_eq!(others, 0);
            assert_eq!(releases, long_presses);
        }
        kani::cover!(
            lifted && long_presses == 1 && !tracking.held,
            "a lift that beat the timer"
        );
        kani::cover!(!lifted && releases == 1, "a hold cut short");
        kani::cover!(lifted && others == 1, "a tap or a swipe");
    }
}
