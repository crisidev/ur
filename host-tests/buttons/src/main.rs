//! Host tests for the firmware's `buttons` module: telling the button
//! grid's events apart, in either layout. `cargo test` runs them.
//!
//! Each test plays touches at given milliseconds; nothing reads the clock,
//! so no time driver is needed.

#[path = "../../../src/buttons.rs"]
#[allow(dead_code)]
mod buttons;

fn main() {
    println!("run `cargo test`");
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use embassy_time::Instant;

    use super::buttons::*;

    fn ms(at: u64) -> Instant {
        Instant::from_millis(at)
    }

    /// The touches a typical tap takes: down for 80 ms.
    const TAP_MS: u64 = 80;

    /// A test's grid, its layout, and everything it sent.
    struct Run {
        layout: Layout,
        grid: Grid,
        sent: Vec<(u64, Press)>,
    }

    impl Run {
        fn new(layout: Layout) -> Self {
            Self {
                layout,
                grid: Grid::default(),
                sent: Vec::new(),
            }
        }

        /// Nine buttons, 90 codes.
        fn codes() -> Self {
            Self::new(Layout::TwoLayers)
        }

        /// Six buttons.
        fn six() -> Self {
            Self::new(Layout::Six)
        }

        /// A tap on `button`, lifting at `at`.
        fn tap(&mut self, button: u8, at: u64) -> &mut Self {
            self.touch(button, Touch::Tap, at - TAP_MS, at)
        }

        /// A hold on `button`, starting at `at`: recognised once held long
        /// enough.
        fn hold(&mut self, button: u8, at: u64) -> &mut Self {
            self.touch(button, Touch::Hold, at, at + 600)
        }

        fn touch(&mut self, button: u8, touch: Touch, started: u64, now: u64) -> &mut Self {
            self.expire(started, false);
            for press in self
                .grid
                .touch(self.layout, button, touch, ms(started), ms(now))
            {
                self.sent.extend(press.map(|p| (now, p)));
            }
            self
        }

        /// The UI loop waking at `at`, as it would at the deadline.
        fn expire(&mut self, at: u64, finger_down: bool) -> &mut Self {
            if let Some(press) = self.grid.expire(ms(at), finger_down) {
                self.sent.push((at, press));
            }
            self
        }

        /// Time passes until `at`: whatever's waiting goes when it's due.
        fn until(&mut self, at: u64) -> &mut Self {
            if let Some(deadline) = self.grid.deadline().filter(|&d| d <= ms(at)) {
                self.expire(deadline.as_millis(), false);
            }
            self
        }

        /// The finger lifting at `at`, after a long press.
        fn lift(&mut self, at: u64) -> &mut Self {
            self.sent.extend(self.grid.lift().map(|p| (at, p)));
            self
        }

        fn presses(&self) -> Vec<(u8, Gesture)> {
            self.sent.iter().map(|(_, p)| (p.code, p.gesture)).collect()
        }
    }

    use Gesture::{DoubleTap, Hold, Release, Tap};

    #[test]
    fn a_single_tap_goes_once_nothing_follows() {
        let mut run = Run::codes();
        run.tap(5, 1000);
        assert!(run.sent.is_empty(), "waits to see whether more follows");
        assert_eq!(run.grid.waiting(), (Some(5), None));
        run.until(10_000);
        assert_eq!(
            run.sent,
            [(
                1800,
                Press {
                    code: 5,
                    gesture: Tap
                }
            )]
        );
        assert_eq!(run.grid.waiting(), (None, None));
    }

    #[test]
    fn a_single_hold_goes_at_once() {
        let mut run = Run::codes();
        run.hold(7, 1000);
        assert_eq!(
            run.sent,
            [(
                1600,
                Press {
                    code: 7,
                    gesture: Hold
                }
            )]
        );
        assert_eq!(run.grid.deadline(), None);
    }

    #[test]
    fn a_hold_ends_when_the_finger_lifts() {
        let mut run = Run::codes();
        run.hold(7, 1000).lift(3000);
        assert_eq!(run.presses(), [(7, Hold), (7, Release)]);
        // Once: a second lift (or one with no hold before it) is nothing.
        run.lift(3100);
        assert_eq!(run.sent.len(), 2);
        // A sequence's hold too, with the sequence's code.
        run.tap(4, 5000).hold(7, 5300).lift(6000);
        assert_eq!(run.presses()[2..], [(47, Hold), (47, Release)]);
    }

    #[test]
    fn a_tap_has_no_release() {
        let mut run = Run::codes();
        run.tap(2, 1000).until(5000).lift(5000);
        assert_eq!(run.presses(), [(2, Tap)]);
    }

    #[test]
    fn cancelling_keeps_a_holds_release() {
        // Swiping away mid-hold can't happen (the hold took the touch),
        // but the side button can: whatever the hold set going must still
        // hear it end.
        let mut run = Run::codes();
        run.hold(5, 1000);
        assert!(!run.grid.cancel(), "nothing was waiting");
        run.lift(2000);
        assert_eq!(run.presses(), [(5, Hold), (5, Release)]);
    }

    #[test]
    fn two_buttons_are_a_sequence() {
        let mut run = Run::codes();
        run.tap(2, 1000).tap(3, 1500);
        assert!(run.sent.is_empty(), "the second may be double-tapped");
        assert_eq!(run.grid.waiting(), (Some(2), Some(3)));
        run.until(10_000);
        assert_eq!(run.presses(), [(23, Tap)]);
        // Sent once the double tap's wait is over.
        assert_eq!(run.sent[0].0, 1500 + WAIT_DOUBLE.as_millis());
    }

    #[test]
    fn the_same_button_twice_is_a_sequence_too() {
        let mut run = Run::codes();
        run.tap(1, 1000).tap(1, 1300).until(10_000);
        assert_eq!(run.presses(), [(11, Tap)]);
    }

    #[test]
    fn a_sequence_double_tapped() {
        let mut run = Run::codes();
        run.tap(2, 1000).tap(3, 1500).tap(3, 1750);
        assert_eq!(
            run.sent,
            [(
                1750,
                Press {
                    code: 23,
                    gesture: DoubleTap
                }
            )]
        );
        assert_eq!(run.grid.deadline(), None, "nothing more to wait for");
        // And 1, 1, 1 is 11 double-tapped.
        let mut run = Run::codes();
        run.tap(1, 1000).tap(1, 1200).tap(1, 1400);
        assert_eq!(run.presses(), [(11, DoubleTap)]);
    }

    #[test]
    fn a_sequence_held() {
        let mut run = Run::codes();
        run.tap(4, 1000).hold(9, 1400);
        assert_eq!(
            run.sent,
            [(
                2000,
                Press {
                    code: 49,
                    gesture: Hold
                }
            )]
        );
    }

    #[test]
    fn a_hold_counts_from_when_it_started() {
        // The hold starts 0.7 s after the first tap's lift, in time, but is
        // only recognised 0.6 s later, well after the wait. The UI loop
        // woke at the deadline with the finger down, so held off.
        let mut run = Run::codes();
        run.tap(4, 1000);
        run.expire(1000 + WAIT_SECOND.as_millis(), true);
        assert!(run.sent.is_empty(), "a finger is down: it may continue");
        run.touch(9, Touch::Hold, 1700, 2300);
        assert_eq!(run.presses(), [(49, Hold)]);
    }

    #[test]
    fn a_touch_too_late_starts_afresh() {
        // The UI loop didn't get to expire the first (a finger was down,
        // but its touch began after the deadline): that touch sends it.
        let mut run = Run::codes();
        run.tap(4, 1000);
        run.expire(1000 + WAIT_SECOND.as_millis(), true);
        run.touch(9, Touch::Tap, 1900, 1980);
        assert_eq!(run.presses(), [(4, Tap)]);
        assert_eq!(run.grid.waiting(), (Some(9), None));
        // A late hold: both at once.
        let mut run = Run::codes();
        run.tap(4, 1000);
        run.expire(1000 + WAIT_SECOND.as_millis(), true);
        run.touch(9, Touch::Hold, 1900, 2500);
        assert_eq!(run.presses(), [(4, Tap), (9, Hold)]);
    }

    #[test]
    fn a_double_tap_must_start_in_time() {
        let mut run = Run::codes();
        run.tap(2, 1000).tap(3, 1500);
        run.expire(1500 + WAIT_DOUBLE.as_millis(), true);
        // The third tap began after the wait: 23, then 3 waits on its own.
        run.touch(3, Touch::Tap, 1950, 2030).until(10_000);
        assert_eq!(run.presses(), [(23, Tap), (3, Tap)]);
    }

    #[test]
    fn another_button_after_a_sequence_starts_the_next() {
        let mut run = Run::codes();
        run.tap(2, 1000).tap(3, 1500).tap(4, 1700);
        assert_eq!(run.presses(), [(23, Tap)]);
        assert_eq!(run.grid.waiting(), (Some(4), None));
        run.tap(5, 2000).until(10_000);
        assert_eq!(run.presses(), [(23, Tap), (45, Tap)]);
    }

    #[test]
    fn a_hold_after_a_sequence_is_a_single_hold() {
        // 2, 3, then holding 3: the tap was the sequence's, and the hold
        // can't extend it.
        let mut run = Run::codes();
        run.tap(2, 1000).tap(3, 1500).hold(3, 1600);
        assert_eq!(run.presses(), [(23, Tap), (3, Hold)]);
    }

    #[test]
    fn cancelling_sends_nothing() {
        let mut run = Run::codes();
        run.tap(2, 1000);
        assert!(run.grid.cancel());
        run.until(10_000);
        assert!(run.sent.is_empty());
        assert!(!run.grid.cancel(), "nothing left to cancel");
        run.tap(2, 20_000).tap(3, 20_300);
        assert!(run.grid.cancel());
        run.until(30_000);
        assert!(run.sent.is_empty());
    }

    #[test]
    fn expiring_early_does_nothing() {
        let mut run = Run::codes();
        run.tap(6, 1000);
        run.expire(1000 + WAIT_SECOND.as_millis() - 1, false);
        assert!(run.sent.is_empty());
    }

    #[test]
    fn every_event_is_distinct() {
        // All 261, through the grid as a wearer would make them.
        let mut events = HashSet::new();
        let mut at = 1000;
        let mut next = || {
            at += 10_000;
            at
        };
        for a in 1..=9 {
            let mut run = Run::codes();
            let t = next();
            run.tap(a, t).until(t + 5000);
            let t = next();
            run.hold(a, t).lift(t + 1000);
            for b in 1..=9 {
                let t = next();
                run.tap(a, t).tap(b, t + 400).until(t + 5000);
                let t = next();
                run.tap(a, t).tap(b, t + 400).tap(b, t + 600);
                let t = next();
                run.tap(a, t).hold(b, t + 400).lift(t + 2000);
            }
            for (code, gesture) in run.presses().into_iter().filter(|&(_, g)| g != Release) {
                assert!(
                    events.insert((code, gesture.name())),
                    "{code} {gesture:?} twice"
                );
            }
        }
        assert_eq!(events.len(), 9 * 2 + 81 * 3);
        let names: HashSet<&str> = events.iter().map(|&(_, name)| name).collect();
        assert_eq!(
            names,
            HashSet::from(["press", "double_press", "hold_press"]),
            "BTHome's names"
        );
        let codes: HashSet<u8> = events.iter().map(|&(code, _)| code).collect();
        assert_eq!(codes.len(), 90);
        assert!(codes.iter().all(|&c| (1..=99).contains(&c) && c % 10 != 0));
    }

    #[test]
    fn a_press_knows_its_last_button() {
        let press = Press {
            code: 23,
            gesture: Tap,
        };
        assert_eq!(press.last(), 3);
        let press = Press {
            code: 7,
            gesture: Hold,
        };
        assert_eq!(press.last(), 7);
    }

    #[test]
    fn the_grid_is_a_keypad() {
        let at = |x, y| at(Layout::TwoLayers, x, y);
        assert_eq!(at(0, 0), 1);
        assert_eq!(at(40, 0), 2);
        assert_eq!(at(79, 0), 3);
        assert_eq!(at(0, 40), 4);
        assert_eq!(at(40, 40), 5);
        assert_eq!(at(79, 79), 9);
        // Thirds of the screen: 0..27, 27..54, 54..80.
        assert_eq!(at(26, 26), 1);
        assert_eq!(at(27, 27), 5);
        assert_eq!(at(53, 53), 5);
        assert_eq!(at(54, 54), 9);
        // Off the edge (a touch's start can't be, but still).
        assert_eq!(at(-5, 200), 7);
    }

    #[test]
    fn six_buttons_are_two_rows_of_three() {
        let at = |x, y| at(Layout::Six, x, y);
        assert_eq!(at(0, 0), 1);
        assert_eq!(at(40, 39), 2);
        assert_eq!(at(79, 0), 3);
        assert_eq!(at(0, 40), 4);
        assert_eq!(at(40, 79), 5);
        assert_eq!(at(79, 79), 6);
        // Off the edge.
        assert_eq!(at(-5, 200), 4);
        assert_eq!(at(200, -5), 3);
    }

    // ==========================================================================
    // Six buttons
    // ==========================================================================

    #[test]
    fn six_a_tap_waits_only_for_a_double_tap() {
        let mut run = Run::six();
        run.tap(5, 1000);
        assert!(run.sent.is_empty(), "it may be double-tapped");
        assert_eq!(run.grid.waiting(), (Some(5), None));
        assert_eq!(run.grid.deadline(), Some(ms(1000) + WAIT_DOUBLE));
        run.until(10_000);
        assert_eq!(
            run.sent,
            [(
                1000 + WAIT_DOUBLE.as_millis(),
                Press {
                    code: 5,
                    gesture: Tap
                }
            )]
        );
    }

    #[test]
    fn six_a_double_tap_is_the_same_button_twice() {
        let mut run = Run::six();
        run.tap(1, 1000).tap(1, 1250);
        assert_eq!(
            run.sent,
            [(
                1250,
                Press {
                    code: 1,
                    gesture: DoubleTap
                }
            )]
        );
        assert_eq!(run.grid.deadline(), None, "nothing more to wait for");
        // A third tap starts afresh, rather than making a triple.
        run.tap(1, 1450).until(10_000);
        assert_eq!(run.presses(), [(1, DoubleTap), (1, Tap)]);
    }

    #[test]
    fn six_another_button_is_two_taps() {
        // No sequences: 2 then 3 is 2 tapped, then 3 tapped, the first
        // sent the moment the second lands.
        let mut run = Run::six();
        run.tap(2, 1000).tap(3, 1200);
        assert_eq!(run.presses(), [(2, Tap)]);
        assert_eq!(run.sent[0].0, 1200);
        assert_eq!(run.grid.waiting(), (Some(3), None));
        run.until(10_000);
        assert_eq!(run.presses(), [(2, Tap), (3, Tap)]);
    }

    #[test]
    fn six_a_double_tap_must_start_in_time() {
        let mut run = Run::six();
        run.tap(4, 1000);
        run.expire(1000 + WAIT_DOUBLE.as_millis(), true);
        assert!(run.sent.is_empty(), "a finger is down: it may continue");
        // The second tap began after the wait: two taps.
        run.touch(4, Touch::Tap, 1000 + WAIT_DOUBLE.as_millis() + 1, 1500)
            .until(10_000);
        assert_eq!(run.presses(), [(4, Tap), (4, Tap)]);
    }

    #[test]
    fn six_a_hold_goes_at_once_and_releases() {
        let mut run = Run::six();
        run.hold(6, 1000);
        assert_eq!(run.presses(), [(6, Hold)]);
        assert_eq!(run.grid.deadline(), None);
        run.lift(2500);
        assert_eq!(run.presses(), [(6, Hold), (6, Release)]);
    }

    #[test]
    fn six_a_hold_after_a_tap_sends_both() {
        // There's no "tap, then hold": the tap goes as it stood, the hold
        // after it, whichever button.
        let mut run = Run::six();
        run.tap(3, 1000).hold(3, 1100);
        assert_eq!(run.presses(), [(3, Tap), (3, Hold)]);
        let mut run = Run::six();
        run.tap(3, 1000).hold(4, 1100);
        assert_eq!(run.presses(), [(3, Tap), (4, Hold)]);
    }

    #[test]
    fn six_cancelling_sends_nothing() {
        let mut run = Run::six();
        run.tap(2, 1000);
        assert!(run.grid.cancel());
        run.until(10_000);
        assert!(run.sent.is_empty());
    }

    #[test]
    fn six_every_event_is_distinct() {
        let mut events = HashSet::new();
        let mut run = Run::six();
        let mut at = 1000;
        for button in 1..=6 {
            at += 10_000;
            run.tap(button, at).until(at + 5000);
            at += 10_000;
            run.tap(button, at).tap(button, at + 200);
            at += 10_000;
            run.hold(button, at).lift(at + 1000);
        }
        for (code, gesture) in run.presses() {
            assert!(
                events.insert((code, gesture.name())),
                "{code} {gesture:?} twice"
            );
        }
        assert_eq!(events.len(), 6 * 4);
        assert!(events.iter().all(|&(code, _)| (1..=6).contains(&code)));
    }

    #[test]
    fn the_layout_round_trips() {
        assert_eq!(Layout::default(), Layout::Six);
        for layout in [Layout::Six, Layout::TwoLayers] {
            assert_eq!(Layout::from_byte(layout.to_byte()), layout);
            assert_ne!(layout.next(), layout);
            assert_eq!(layout.next().next(), layout);
        }
        assert_eq!(Layout::from_byte(200), Layout::Six);
    }

    #[test]
    fn the_setting_round_trips() {
        let mut setting = Setting::default();
        assert_eq!(setting, Setting::Off);
        assert!(!setting.on());
        let mut seen = Vec::new();
        for _ in 0..4 {
            assert_eq!(Setting::from_byte(setting.to_byte()), setting);
            seen.push(setting);
            setting = setting.next();
        }
        assert_eq!(setting, Setting::Off, "round the cycle");
        assert_eq!(
            seen,
            [Setting::Off, Setting::Ha, Setting::Phone, Setting::Both]
        );
        assert!(Setting::Ha.ha() && !Setting::Ha.phone());
        assert!(!Setting::Phone.ha() && Setting::Phone.phone());
        assert!(Setting::Both.ha() && Setting::Both.phone());
        assert_eq!(Setting::from_byte(200), Setting::Off);
    }
}
