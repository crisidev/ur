//! Host tests for the firmware's `wake` module: what a touch does while
//! the screen is off, or glancing. `cargo test` runs them.

#[path = "../../../src/wake.rs"]
mod wake;

fn main() {
    println!("run `cargo test`");
}

#[cfg(test)]
mod tests {
    use super::wake::*;

    /// The middle of the screen, in canvas pixels.
    const MID: (i32, i32) = (40, 40);

    fn off(kind: Kind, (x, y): (i32, i32), held_ms: u16, recent_tap: bool, phone: bool) -> Action {
        screen_off(kind, x, y, held_ms, recent_tap, phone, false)
    }

    /// At once, with or without a phone; with one, it may also be the
    /// first of a double tap on the clock face it woke to.
    #[test]
    fn a_deliberate_tap_wakes_at_once() {
        assert_eq!(
            off(Kind::Tap, MID, 86, false, false),
            Action::Wake { first_tap: false }
        );
        assert_eq!(
            off(Kind::Tap, MID, 86, false, true),
            Action::Wake { first_tap: true }
        );
        // Right at the threshold.
        assert_eq!(
            off(Kind::Tap, MID, DELIBERATE_TAP_MS, false, true),
            Action::Wake { first_tap: true }
        );
    }

    #[test]
    fn a_light_tap_waits_for_a_second() {
        for phone in [false, true] {
            assert_eq!(
                off(Kind::Tap, MID, DELIBERATE_TAP_MS - 1, false, phone),
                Action::FirstTap
            );
        }
    }

    #[test]
    fn a_double_tap_wakes_without_a_phone_and_skips_with() {
        assert_eq!(
            off(Kind::Tap, MID, 20, true, false),
            Action::Wake { first_tap: false }
        );
        assert_eq!(off(Kind::Tap, MID, 20, true, true), Action::Next);
        // Even if the second tap is deliberate on its own.
        assert_eq!(off(Kind::Tap, MID, 90, true, true), Action::Next);
    }

    /// The sealed watch's field log (0.9.0-m3.1): the wearer's double taps
    /// from off, light as they were meant to be, had a deliberate first tap
    /// every time (held 47–67 ms), and second taps of 37–57 ms, 126–156 ms
    /// later. With a phone connected, the first wakes the screen, and the
    /// second (on the clock face, so not decided here) skips. The wearer
    /// would rather that than wait for a single tap's wake; a lighter first
    /// tap skips with the screen off.
    #[test]
    fn the_field_logs_double_taps_wake_with_their_first_tap() {
        for (x, y, held_ms) in [(49, 52, 67), (45, 52, 57), (55, 60, 47)] {
            assert_eq!(
                off(Kind::Tap, (x, y), held_ms, false, true),
                Action::Wake { first_tap: true }
            );
        }
    }

    #[test]
    fn a_long_press_plays_or_pauses_only_with_a_phone() {
        assert_eq!(
            off(Kind::LongPress, MID, 600, false, true),
            Action::PlayPause
        );
        assert_eq!(off(Kind::LongPress, MID, 600, false, false), Action::Ignore);
        // A tap just before doesn't change that.
        assert_eq!(
            off(Kind::LongPress, MID, 600, true, true),
            Action::PlayPause
        );
    }

    #[test]
    fn swipes_do_nothing() {
        for phone in [false, true] {
            for recent_tap in [false, true] {
                assert_eq!(
                    off(Kind::Swipe, MID, 120, recent_tap, phone),
                    Action::Ignore
                );
            }
        }
    }

    #[test]
    fn nothing_in_the_quiet_night() {
        for kind in [Kind::Tap, Kind::LongPress, Kind::Swipe] {
            for (recent_tap, phone) in [(false, false), (true, false), (false, true), (true, true)]
            {
                assert_eq!(
                    screen_off(kind, MID.0, MID.1, 120, recent_tap, phone, true),
                    Action::Ignore
                );
            }
        }
    }

    /// The margin is the outer [`TAP_MARGIN`] canvas pixels on each side.
    #[test]
    fn the_edge_is_the_outer_tenth() {
        assert!(inside(TAP_MARGIN, TAP_MARGIN));
        assert!(inside(79 - TAP_MARGIN, 79 - TAP_MARGIN));
        assert!(!inside(TAP_MARGIN - 1, 40));
        assert!(!inside(40, TAP_MARGIN - 1));
        assert!(!inside(80 - TAP_MARGIN, 40));
        assert!(!inside(40, 80 - TAP_MARGIN));
    }

    /// Touches from a night's field log (Garmin said asleep; these predate
    /// the quiet night's guard, so they show what bedding does), as they'd
    /// land outside the quiet night with a phone connected: canvas positions
    /// where each started.
    #[test]
    fn the_field_logs_phantom_long_presses_at_the_edge_do_nothing() {
        // 9733 ms and 9734 ms, held still.
        assert_eq!(
            off(Kind::LongPress, (66, 6), 600, false, true),
            Action::Ignore
        );
        assert_eq!(
            off(Kind::LongPress, (72, 8), 600, false, true),
            Action::Ignore
        );
    }

    #[test]
    fn the_field_logs_phantom_double_taps_at_the_edge_do_nothing() {
        // Three taps within 50 ms at the top edge (10, 9, and 17 ms).
        assert_eq!(off(Kind::Tap, (41, 3), 10, false, true), Action::Ignore);
        assert_eq!(off(Kind::Tap, (41, 3), 9, false, true), Action::Ignore);
        assert_eq!(off(Kind::Tap, (41, 4), 17, false, true), Action::Ignore);
        // A tap at the right edge, then one at the bottom edge, 470 ms on.
        assert_eq!(off(Kind::Tap, (79, 71), 28, false, true), Action::Ignore);
        assert_eq!(off(Kind::Tap, (71, 73), 106, false, true), Action::Ignore);
        // One at the top edge, then one inside, 240 ms on: the first isn't
        // remembered (`Ignore`, not `FirstTap`), so the second is on its own,
        // and wakes the screen as a deliberate tap would anyway.
        assert_eq!(off(Kind::Tap, (54, 5), 96, false, true), Action::Ignore);
        assert_eq!(
            off(Kind::Tap, (34, 39), 193, false, true),
            Action::Wake { first_tap: true }
        );
    }

    /// On a glance, as from off: a deliberate tap wakes the screen at once
    /// (and may be the first of a double tap on the clock face), a light one
    /// waits for a second, and a second skips.
    #[test]
    fn a_glance_wakes_on_a_deliberate_tap_and_skips_on_a_light_double_tap() {
        assert_eq!(glance(86, false), Action::Wake { first_tap: true });
        assert_eq!(
            glance(DELIBERATE_TAP_MS, false),
            Action::Wake { first_tap: true }
        );
        assert_eq!(glance(DELIBERATE_TAP_MS - 1, false), Action::FirstTap);
        assert_eq!(glance(20, false), Action::FirstTap);
        // The second tap skips whatever it's held for.
        assert_eq!(glance(20, true), Action::Next);
        assert_eq!(glance(86, true), Action::Next);
    }
}
