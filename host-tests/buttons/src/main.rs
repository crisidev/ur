//! Host tests for the firmware's `buttons` module: telling the button
//! grid's 261 events apart. `cargo test` runs them.
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

    /// A test's grid, and everything it sent.
    #[derive(Default)]
    struct Run {
        grid: Grid,
        sent: Vec<(u64, Press)>,
    }

    impl Run {
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
            for press in self.grid.touch(button, touch, ms(started), ms(now)) {
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

        fn presses(&self) -> Vec<(u8, Gesture)> {
            self.sent.iter().map(|(_, p)| (p.code, p.gesture)).collect()
        }
    }

    use Gesture::{DoubleTap, Hold, Tap};

    #[test]
    fn a_single_tap_goes_once_nothing_follows() {
        let mut run = Run::default();
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
        let mut run = Run::default();
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
    fn two_buttons_are_a_sequence() {
        let mut run = Run::default();
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
        let mut run = Run::default();
        run.tap(1, 1000).tap(1, 1300).until(10_000);
        assert_eq!(run.presses(), [(11, Tap)]);
    }

    #[test]
    fn a_sequence_double_tapped() {
        let mut run = Run::default();
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
        let mut run = Run::default();
        run.tap(1, 1000).tap(1, 1200).tap(1, 1400);
        assert_eq!(run.presses(), [(11, DoubleTap)]);
    }

    #[test]
    fn a_sequence_held() {
        let mut run = Run::default();
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
        let mut run = Run::default();
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
        let mut run = Run::default();
        run.tap(4, 1000);
        run.expire(1000 + WAIT_SECOND.as_millis(), true);
        run.touch(9, Touch::Tap, 1900, 1980);
        assert_eq!(run.presses(), [(4, Tap)]);
        assert_eq!(run.grid.waiting(), (Some(9), None));
        // A late hold: both at once.
        let mut run = Run::default();
        run.tap(4, 1000);
        run.expire(1000 + WAIT_SECOND.as_millis(), true);
        run.touch(9, Touch::Hold, 1900, 2500);
        assert_eq!(run.presses(), [(4, Tap), (9, Hold)]);
    }

    #[test]
    fn a_double_tap_must_start_in_time() {
        let mut run = Run::default();
        run.tap(2, 1000).tap(3, 1500);
        run.expire(1500 + WAIT_DOUBLE.as_millis(), true);
        // The third tap began after the wait: 23, then 3 waits on its own.
        run.touch(3, Touch::Tap, 1950, 2030).until(10_000);
        assert_eq!(run.presses(), [(23, Tap), (3, Tap)]);
    }

    #[test]
    fn another_button_after_a_sequence_starts_the_next() {
        let mut run = Run::default();
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
        let mut run = Run::default();
        run.tap(2, 1000).tap(3, 1500).hold(3, 1600);
        assert_eq!(run.presses(), [(23, Tap), (3, Hold)]);
    }

    #[test]
    fn cancelling_sends_nothing() {
        let mut run = Run::default();
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
        let mut run = Run::default();
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
            let mut run = Run::default();
            let t = next();
            run.tap(a, t).until(t + 5000);
            let t = next();
            run.hold(a, t);
            for b in 1..=9 {
                let t = next();
                run.tap(a, t).tap(b, t + 400).until(t + 5000);
                let t = next();
                run.tap(a, t).tap(b, t + 400).tap(b, t + 600);
                let t = next();
                run.tap(a, t).hold(b, t + 400);
            }
            for (code, gesture) in run.presses() {
                assert!(
                    events.insert((code, gesture.name())),
                    "{code} {gesture:?} twice"
                );
            }
        }
        assert_eq!(events.len(), 9 * 2 + 81 * 3);
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
