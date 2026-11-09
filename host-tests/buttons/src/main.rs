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

        /// The finger stays down until `at`: the UI loop wakes, finger
        /// down, when a hold becomes a long press, if that's by then.
        fn held(&mut self, at: u64) -> &mut Self {
            if let Some(due) = self.grid.long_at().filter(|&d| d <= ms(at)) {
                self.expire(due.as_millis(), true);
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

    use Gesture::{DoubleTap, Hold, LongPress, Release, Tap};

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
                1000 + WAIT_SECOND.as_millis(),
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
    fn a_hold_lifted_before_long_hold_is_no_long_press() {
        let mut run = Run::six();
        let end = 1000 + LONG_HOLD.as_millis() - 1;
        run.hold(5, 1000).held(end).lift(end);
        assert_eq!(run.presses(), [(5, Hold), (5, Release)]);
        assert_eq!(run.grid.long_at(), None, "nothing more due");
    }

    #[test]
    fn a_hold_held_on_is_a_long_press_once_then_its_release() {
        let mut run = Run::six();
        run.hold(5, 1000);
        let due = 1000 + LONG_HOLD.as_millis();
        assert_eq!(run.grid.long_at(), Some(ms(due)), "from the touch's start");
        // Not early, and not once the finger's up (its release decides).
        run.expire(due - 1, true).expire(due, false);
        assert_eq!(run.presses(), [(5, Hold)]);
        run.held(5000);
        assert_eq!(
            run.sent[1],
            (
                due,
                Press {
                    code: 5,
                    gesture: LongPress
                }
            )
        );
        // Once: the loop waking again, finger still down, sends nothing.
        assert_eq!(run.grid.long_at(), None);
        run.expire(6000, true).held(7000).lift(7000);
        assert_eq!(run.presses(), [(5, Hold), (5, LongPress), (5, Release)]);
        // With two layers, a sequence's hold, by its code.
        let mut run = Run::codes();
        run.tap(4, 10_000).hold(7, 10_300).held(13_000).lift(13_000);
        assert_eq!(run.presses(), [(47, Hold), (47, LongPress), (47, Release)]);
        assert_eq!(LongPress.name(), "long_press");
    }

    #[test]
    fn a_long_press_waits_for_no_tap() {
        // A long press doesn't hold up, or get held up by, the waits for
        // taps: there's nothing waiting while a hold is down.
        let mut run = Run::six();
        run.hold(2, 1000);
        assert_eq!(run.grid.deadline(), None);
        assert_eq!(run.grid.waiting(), (None, None));
        run.held(3000);
        assert_eq!(run.grid.deadline(), None);
        // The next touch starts afresh, release or not.
        run.tap(3, 4000).until(5000);
        assert_eq!(run.presses(), [(2, Hold), (2, LongPress), (3, Tap)]);
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
        // The hold starts 0.5 s after the first tap's lift, in time, but is
        // only recognised 0.6 s later, well after the wait. The UI loop
        // woke at the deadline with the finger down, so held off.
        let mut run = Run::codes();
        run.tap(4, 1000);
        run.expire(1000 + WAIT_SECOND.as_millis(), true);
        assert!(run.sent.is_empty(), "a finger is down: it may continue");
        run.touch(9, Touch::Hold, 1500, 2100);
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
    fn six_a_slow_double_tap_still_counts() {
        // The field logs' slowest double taps leave ~160 ms from lift to
        // press (see `WAIT_DOUBLE`); one that leaves 250 ms is still one.
        let mut run = Run::six();
        run.tap(3, 1000).touch(3, Touch::Tap, 1250, 1250 + TAP_MS);
        assert_eq!(run.presses(), [(3, DoubleTap)]);
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
            [Setting::Off, Setting::BtHome, Setting::App, Setting::Both]
        );
        assert!(Setting::BtHome.bthome() && !Setting::BtHome.app());
        assert!(!Setting::App.bthome() && Setting::App.app());
        assert!(Setting::Both.bthome() && Setting::Both.app());
        assert_eq!(Setting::from_byte(200), Setting::Off);
        // As stored by earlier builds, which called the app the phone.
        assert_eq!(Setting::from_byte(2), Setting::App);
    }
}

/// Proofs about the grid for every touch and every time, where the tests
/// above play chosen ones. `cargo kani` runs them (one at a time with
/// `--harness`: CBMC takes memory).
///
/// The sequences are driven as the firmware drives the grid (`ui::App`'s
/// `on_grid`, `on_input` and `on_timer`, and `touch::touch_task`), which
/// these lean on:
///
/// - Buttons are 1–6 or 1–9, as [`at`] gives them (see `at_is_a_button`).
/// - A touch starts no later than it's recognised, and events are handled
///   in order. A touch may start *before* the last event was handled,
///   though: the UI loop can be late, and subtracts how long the finger
///   was down from when it got round to the touch.
/// - After a hold, the finger lifts ([`Grid::lift`]) before another touch
///   can start.
/// - Times stay clear of the end of `Instant`'s range (584,000 years at a
///   tick a microsecond), where adding a wait would overflow.
#[cfg(kani)]
mod proofs {
    use embassy_time::{Duration, Instant};

    use super::buttons::*;

    /// No time is later than this (see the module docs).
    const LATEST: u64 = u64::MAX / 4;
    /// Long after the last event: every wait is over by then.
    const EVENTUALLY: u64 = u64::MAX / 2;
    /// The most touches a sequence here has.
    const TOUCHES: usize = 5;

    fn any_layout() -> Layout {
        if kani::any() {
            Layout::Six
        } else {
            Layout::TwoLayers
        }
    }

    /// The most buttons a layout has.
    fn buttons(layout: Layout) -> u8 {
        match layout {
            Layout::Six => 6,
            Layout::TwoLayers => 9,
        }
    }

    fn any_button(layout: Layout) -> u8 {
        let button: u8 = kani::any();
        kani::assume(1 <= button && button <= buttons(layout));
        button
    }

    /// A hold whose finger is still down, as the wearer knows it.
    #[derive(Clone, Copy)]
    struct Holding {
        code: u8,
        started: u64,
        long: bool,
    }

    /// Someone using the grid, who remembers what they did, and checks
    /// everything the grid says against it.
    struct Wearer {
        layout: Layout,
        grid: Grid,
        /// When the last event was handled, in ticks.
        now: u64,
        /// The buttons touched so far, in order.
        touched: [u8; TOUCHES],
        touches: usize,
        /// How many of those the presses sent so far account for (or
        /// that a cancel dropped): the rest are waiting.
        sent: usize,
        holding: Option<Holding>,
    }

    impl Wearer {
        fn new(layout: Layout) -> Self {
            Self {
                layout,
                grid: Grid::default(),
                now: 0,
                touched: [0; TOUCHES],
                touches: 0,
                sent: 0,
                holding: None,
            }
        }

        /// Time passes: some, none, or a lot.
        fn later(&mut self) -> Instant {
            let now: u64 = kani::any();
            kani::assume(self.now <= now && now <= LATEST);
            self.now = now;
            Instant::from_ticks(now)
        }

        /// One more of the buttons touched is accounted for: it must be
        /// the next one that was waiting.
        fn digit(&mut self, button: u8) {
            assert!(self.sent < self.touches, "a press nothing touched");
            assert_eq!(self.touched[self.sent], button, "not the button touched");
            self.sent += 1;
        }

        /// A tap's, double tap's or hold's press went out: it's one the
        /// layout has, and spells the buttons touched, in order.
        fn press(&mut self, press: Press) {
            let (first, last) = (press.code / 10, press.code % 10);
            assert!(1 <= last && last <= buttons(self.layout));
            assert_eq!(press.last(), last);
            match self.layout {
                Layout::Six => assert_eq!(first, 0, "six buttons have no sequences"),
                Layout::TwoLayers => {
                    assert!(first <= 9);
                    assert!(
                        first != 0 || press.gesture != Gesture::DoubleTap,
                        "two taps on a button are a sequence, with two layers"
                    );
                }
            }
            if first != 0 {
                self.digit(first);
            }
            self.digit(last);
            if press.gesture == Gesture::DoubleTap {
                self.digit(last);
            }
            // The proofs aren't empty: the longest press there is, and
            // each kind of the rest below, do come up.
            kani::cover!(
                first != 0 && press.gesture == Gesture::DoubleTap,
                "a sequence, double-tapped"
            );
            kani::cover!(
                first != 0 && press.gesture == Gesture::Hold,
                "a sequence, held"
            );
        }

        /// A touch on any button, at any time from now on.
        fn touch(&mut self, touch: Touch) {
            // The finger of a hold lifts first (see the module docs).
            kani::assume(self.holding.is_none());
            let button = any_button(self.layout);
            let now = self.later();
            let started: u64 = kani::any();
            kani::assume(started <= self.now);
            self.touched[self.touches] = button;
            self.touches += 1;

            let [waited, completed] = self.grid.touch(
                self.layout,
                button,
                touch,
                Instant::from_ticks(started),
                now,
            );
            if let Some(press) = waited {
                assert_eq!(press.gesture, Gesture::Tap, "only taps wait");
                self.press(press);
            }
            match completed {
                Some(press) if press.gesture == Gesture::Hold => {
                    assert_eq!(touch, Touch::Hold, "a tap was taken for a hold");
                    self.press(press);
                    self.holding = Some(Holding {
                        code: press.code,
                        started,
                        long: false,
                    });
                    // Everything before a hold has gone: nothing follows it.
                    assert_eq!(self.sent, self.touches);
                }
                Some(press) => {
                    assert_eq!(touch, Touch::Tap);
                    assert_eq!(press.gesture, Gesture::DoubleTap);
                    self.press(press);
                    assert_eq!(self.sent, self.touches);
                }
                None => assert_eq!(touch, Touch::Tap, "a hold is sent at once"),
            }
            // What a tap leaves waiting, waits a while yet.
            if let Some(until) = self.grid.deadline() {
                assert!(until > now, "a wait that's over as it starts");
            }
            self.check();
        }

        /// The finger lifts after a long press (anywhere: the grid is told
        /// of every one, see `ui::App::on_input`).
        fn lift(&mut self) {
            let release = self.grid.lift();
            match self.holding.take() {
                Some(hold) => assert_eq!(
                    release,
                    Some(Press {
                        code: hold.code,
                        gesture: Gesture::Release
                    }),
                    "a hold that never ended"
                ),
                None => assert_eq!(release, None, "a release without its hold"),
            }
            self.check();
        }

        /// The UI loop's timer, on the grid's screen: `ui::App::on_timer`.
        /// Returns whether a finger was down.
        fn timer(&mut self) -> bool {
            let now = self.later();
            let finger_down: bool = kani::any();
            let deadline = self.grid.deadline();
            let sent = self.grid.expire(now, finger_down);

            // A hold's long press: once, LONG_HOLD after it started, if
            // the finger's still down.
            let long_due = self.holding.is_some_and(|hold| {
                !hold.long && finger_down && now >= Instant::from_ticks(hold.started) + LONG_HOLD
            });
            // What was waiting: once its wait is over, and no finger is
            // down that might continue it.
            let wait_over = !finger_down && deadline.is_some_and(|until| now >= until);
            match sent {
                Some(press) if press.gesture == Gesture::LongPress => {
                    assert!(long_due, "a long press that wasn't due");
                    let hold = self.holding.as_mut().expect("due only while holding");
                    assert_eq!(press.code, hold.code);
                    hold.long = true;
                    kani::cover!(true, "a long press");
                }
                Some(press) => {
                    assert!(wait_over, "a tap sent before its wait was over");
                    assert_eq!(press.gesture, Gesture::Tap);
                    self.press(press);
                    assert_eq!(self.sent, self.touches, "something still waits");
                }
                None => assert!(!long_due && !wait_over, "nothing sent when due"),
            }
            self.check();
            finger_down
        }

        /// A swipe, or the side button: `ui::App::go_to`.
        fn cancel(&mut self) {
            let cancelled = self.grid.cancel();
            assert_eq!(cancelled, self.sent < self.touches);
            kani::cover!(cancelled, "a waiting tap, cancelled");
            self.sent = self.touches;
            self.check();
        }

        /// What the grid says of itself matches what the wearer did.
        fn check(&self) {
            // What it shows as waiting are the touches not yet sent.
            let waiting = self.touches - self.sent;
            let touched = |i: usize| Some(self.touched[self.sent + i]);
            let shown = match waiting {
                0 => (None, None),
                1 => (touched(0), None),
                2 => (touched(0), touched(1)),
                _ => panic!("more than two taps waiting"),
            };
            assert_eq!(self.grid.waiting(), shown);
            assert_eq!(self.grid.deadline().is_some(), waiting > 0);
            // With six buttons, only one tap ever waits.
            assert!(self.layout == Layout::TwoLayers || waiting <= 1);
            // A long press is due exactly while a hold hasn't had its own.
            match self.holding {
                Some(hold) => {
                    assert_eq!(waiting, 0, "nothing waits behind a hold");
                    let due = Instant::from_ticks(hold.started) + LONG_HOLD;
                    assert_eq!(self.grid.long_at(), (!hold.long).then_some(due));
                }
                None => assert_eq!(self.grid.long_at(), None),
            }
        }

        /// The finger lifts if it's down, and time passes until nothing
        /// can still be waiting: every touch has been accounted for, and
        /// the grid is as it began.
        fn finish(&mut self) {
            if self.holding.is_some() {
                self.lift();
            }
            if let Some(press) = self.grid.expire(Instant::from_ticks(EVENTUALLY), false) {
                assert_eq!(press.gesture, Gesture::Tap);
                self.press(press);
            }
            assert_eq!(self.sent, self.touches, "a touch that came to nothing");
            assert_eq!(self.grid, Grid::default());
        }

        /// Anything the firmware might do next.
        fn anything(&mut self) {
            match kani::any::<u8>() {
                0 => self.touch(Touch::Tap),
                1 => self.touch(Touch::Hold),
                2 => self.lift(),
                3 => self.cancel(),
                _ => {
                    self.timer();
                }
            }
        }
    }

    /// Whatever the wearer does (four things: taps and holds on any
    /// buttons at any times, lifts, swipes away, and the timer firing
    /// whenever, finger down or not), in either layout:
    ///
    /// - nothing panics or overflows;
    /// - every press is one the layout has (no 0 digit, no double-tapped
    ///   single with two layers, no sequence with six buttons);
    /// - the presses spell the buttons touched, in order, each once: none
    ///   dropped, none sent twice, unless cancelled;
    /// - a hold is sent at once, gets its long press once and only when
    ///   due, and always ends in its release; neither comes without it;
    /// - a waiting tap goes exactly when its wait is over with no finger
    ///   down, and a new wait is never over as it starts;
    /// - `waiting`, `deadline` and `long_at` agree with all that;
    /// - and once the finger's up and the waits are over, the grid is
    ///   back as it began.
    #[kani::proof]
    #[kani::unwind(5)]
    fn any_four_events_are_accounted_for() {
        let mut wearer = Wearer::new(any_layout());
        for _ in 0..4 {
            wearer.anything();
        }
        wearer.finish();
    }

    /// The same for the longest thing there is to press, a sequence
    /// double-tapped, which takes three of four events, leaving one to
    /// go before or after it: here, five taps and holds in a row (room
    /// for a touch both before and after), each followed by whatever the
    /// timer and the finger's lift make of it.
    #[kani::proof]
    #[kani::unwind(6)]
    fn any_five_touches_are_accounted_for() {
        let mut wearer = Wearer::new(any_layout());
        for _ in 0..TOUCHES {
            wearer.touch(if kani::any() { Touch::Tap } else { Touch::Hold });
            if kani::any() {
                wearer.timer();
            }
            if wearer.holding.is_some() {
                wearer.lift();
            }
        }
        wearer.finish();
    }

    /// Every press has a code of one or two digits, neither of them 0,
    /// even if the layout changes between touches (the wearer above
    /// keeps to one; the Settings screen can change it, though leaving
    /// the grid's screen for it cancels what was waiting). `ui::draw_key`
    /// writes the code into two characters, and panics if it doesn't
    /// fit; BTHome and the phone get it as a byte.
    ///
    /// For any four events, the lot unchecked: no lift need follow a
    /// hold here.
    #[kani::proof]
    #[kani::unwind(5)]
    fn every_code_fits_two_digits_whatever_the_layout() {
        let mut grid = Grid::default();
        let mut now = 0;
        let fits = |press: Option<Press>| {
            if let Some(press) = press {
                assert!(1 <= press.code && press.code <= 99 && press.last() != 0);
            }
        };
        for _ in 0..4 {
            let later: u64 = kani::any();
            kani::assume(now <= later && later <= LATEST);
            now = later;
            let at = Instant::from_ticks(now);
            match kani::any::<u8>() {
                0 => {
                    let layout = any_layout();
                    let touch = if kani::any() { Touch::Tap } else { Touch::Hold };
                    let started: u64 = kani::any();
                    kani::assume(started <= now);
                    let started = Instant::from_ticks(started);
                    let [waited, completed] =
                        grid.touch(layout, any_button(layout), touch, started, at);
                    fits(waited);
                    fits(completed);
                }
                1 => fits(grid.lift()),
                2 => {
                    grid.cancel();
                }
                _ => fits(grid.expire(at, kani::any())),
            }
        }
    }

    /// The UI loop can't spin: after its timer fires and `on_timer` has
    /// run, whatever it next waits for (`ui::App::deadline`) is still to
    /// come. This mirrors those two functions, since `ui` isn't
    /// hardware-free, for any grid three events can leave, on the grid's
    /// screen or off it.
    ///
    /// It's what went wrong once: with a finger held down off the grid's
    /// screen, `on_timer` only cancels, the hold's `long_at` stayed due,
    /// and the loop ran flat out until the finger lifted. `deadline` now
    /// leaves the hold out when off the grid, which the `filter` below
    /// stands for: without it, this fails.
    #[kani::proof]
    #[kani::unwind(4)]
    fn the_timer_leaves_nothing_due() {
        let mut wearer = Wearer::new(any_layout());
        for _ in 0..3 {
            wearer.anything();
        }
        let on_grid: bool = kani::any();
        let finger_down = if on_grid {
            wearer.timer()
        } else {
            // `on_timer` off the grid: nothing waits.
            wearer.later();
            wearer.grid.cancel();
            kani::any()
        };
        // `App::deadline`, but for the banner's `sent`, which is cleared
        // when due whatever the screen.
        let due = if finger_down {
            wearer.grid.long_at().filter(|_| on_grid)
        } else {
            wearer.grid.deadline()
        };
        if let Some(due) = due {
            assert!(due > Instant::from_ticks(wearer.now), "due already: a spin");
        }
    }

    // Two layers' waits, as the module docs give them, for any buttons
    // and any times.

    /// A tap at `lifted`, and what the grid makes of a second touch
    /// starting `gap` after it, once everything's over: the presses sent,
    /// in order.
    fn two_touches(
        layout: Layout,
        second: Touch,
        gap_within: Duration,
    ) -> (u8, u8, [Option<Press>; 3], bool) {
        let (a, b) = (any_button(layout), any_button(layout));
        let (started, lifted, again, recognised): (u64, u64, u64, u64) = kani::any();
        kani::assume(started <= lifted && lifted <= again && again <= recognised);
        kani::assume(recognised <= LATEST);
        let in_time = Instant::from_ticks(again) <= Instant::from_ticks(lifted) + gap_within;

        let mut grid = Grid::default();
        let at = Instant::from_ticks;
        let first = grid.touch(layout, a, Touch::Tap, at(started), at(lifted));
        assert_eq!(first, [None, None], "a first tap waits");
        // The timer, at any time while the finger is up between the two:
        // before the second touch starts, that is. (On the very tick the
        // wait ends, the timer and a touch starting then would disagree:
        // to `expire` the wait is over, to `touch` it's still in time.
        // Whichever is handled first decides; one tick either way is
        // far finer than a touch's start is known.)
        let between: u64 = kani::any();
        kani::assume(lifted <= between && between < again);
        let expired = grid.expire(at(between), false);
        let [waited, completed] = grid.touch(layout, b, second, at(again), at(recognised));
        // At most one of these two: what expired isn't waiting.
        assert!(expired.is_none() || waited.is_none());
        let _ = grid.lift();
        let last = grid.expire(at(EVENTUALLY), false);
        assert_eq!(grid, Grid::default());
        (a, b, [expired.or(waited), completed, last], in_time)
    }

    fn tap(code: u8) -> Option<Press> {
        Some(Press {
            code,
            gesture: Gesture::Tap,
        })
    }

    /// With two layers, a second tap is a sequence with the first exactly
    /// when it *starts* within [`WAIT_SECOND`] of the first one's lift,
    /// however long it then takes; any later, they're two singles. Never
    /// a mix: a single and a sequence both, or a tap lost.
    #[kani::proof]
    fn two_taps_are_a_sequence_only_in_time() {
        let (a, b, sent, in_time) = two_touches(Layout::TwoLayers, Touch::Tap, WAIT_SECOND);
        if in_time {
            assert_eq!(sent, [None, None, tap(a * 10 + b)]);
        } else {
            assert_eq!(sent, [tap(a), None, tap(b)]);
        }
    }

    /// With two layers, a hold after a tap ends the sequence if it starts
    /// in time, though it's only recognised later (which is why the waits
    /// run from lift to start); any later, it's a tap and a hold.
    #[kani::proof]
    fn a_tap_then_a_hold_is_a_sequence_only_in_time() {
        let (a, b, sent, in_time) = two_touches(Layout::TwoLayers, Touch::Hold, WAIT_SECOND);
        let hold = |code| {
            Some(Press {
                code,
                gesture: Gesture::Hold,
            })
        };
        if in_time {
            assert_eq!(sent, [None, hold(a * 10 + b), None]);
        } else {
            assert_eq!(sent, [tap(a), hold(b), None]);
        }
    }

    /// With six buttons, a second tap makes a double tap exactly when
    /// it's on the same button and starts within [`WAIT_DOUBLE`] of the
    /// first one's lift; otherwise they're two taps.
    #[kani::proof]
    fn six_buttons_double_tap_only_in_time() {
        let (a, b, sent, in_time) = two_touches(Layout::Six, Touch::Tap, WAIT_DOUBLE);
        if in_time && a == b {
            let double = Press {
                code: a,
                gesture: Gesture::DoubleTap,
            };
            assert_eq!(sent, [None, Some(double), None]);
        } else {
            assert_eq!(sent, [tap(a), None, tap(b)]);
        }
    }

    /// With six buttons, a hold never joins the tap before it, in time
    /// or not: a tap, then a hold.
    #[kani::proof]
    fn six_buttons_hold_stands_alone() {
        let (a, b, sent, _) = two_touches(Layout::Six, Touch::Hold, WAIT_DOUBLE);
        let hold = Press {
            code: b,
            gesture: Gesture::Hold,
        };
        assert_eq!(sent, [tap(a), Some(hold), None]);
    }

    /// Any touch the controller could report, and any it couldn't, is on
    /// a button the layout has; and every button can be reached. So the
    /// sequences above, which take buttons from 1 to the layout's most,
    /// leave none out and add none.
    #[kani::proof]
    fn at_is_a_button() {
        let layout = any_layout();
        let (x, y): (i32, i32) = kani::any();
        let button = at(layout, x, y);
        assert!(1 <= button && button <= buttons(layout));
        // On the canvas, it's the column and row the touch is in.
        if (0..80).contains(&x) && (0..80).contains(&y) {
            let rows = i32::from(buttons(layout)) / 3;
            let (column, row) = (i32::from(button - 1) % 3, i32::from(button - 1) / 3);
            assert!(column * 80 <= x * 3 && x * 3 < (column + 1) * 80);
            assert!(row * 80 <= y * rows && y * rows < (row + 1) * 80);
        }
        let wanted = any_button(layout);
        kani::cover!(button == wanted, "every button can be touched");
    }

    /// Any stored byte reads as a layout and a setting (a flash that's
    /// blank, or was written by another build), what's written reads
    /// back, and what reads back is what a stored byte below the count
    /// said.
    #[kani::proof]
    fn any_stored_byte_is_a_layout_and_a_setting() {
        let byte: u8 = kani::any();
        let layout = Layout::from_byte(byte);
        assert_eq!(Layout::from_byte(layout.to_byte()), layout);
        assert!(layout.to_byte() < 2);
        assert!(byte >= 2 || layout.to_byte() == byte);
        assert!(byte < 2 || layout == Layout::default());
        assert_eq!(layout.next().next(), layout);
        assert_ne!(layout.next(), layout);
        assert!(!layout.name().is_empty());

        let setting = Setting::from_byte(byte);
        assert_eq!(Setting::from_byte(setting.to_byte()), setting);
        assert!(setting.to_byte() < 4);
        assert!(byte >= 4 || setting.to_byte() == byte);
        assert!(byte < 4 || setting == Setting::default());
        assert_eq!(setting.next().next().next().next(), setting);
        assert_ne!(setting.next(), setting);
        assert_eq!(setting.on(), setting.bthome() || setting.app());
        assert_eq!(setting.on(), setting != Setting::Off);
        assert!(!setting.name().is_empty());
    }
}
