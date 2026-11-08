//! Host tests for the firmware's `wake` module: what a touch does while
//! the screen is off. `cargo test` runs them.

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

    fn off(kind: Kind, (x, y): (i32, i32), held_ms: u16, recent_tap: bool, media: bool) -> Action {
        screen_off(kind, x, y, held_ms, recent_tap, media, false)
    }

    #[test]
    fn a_deliberate_tap_wakes() {
        assert_eq!(
            off(Kind::Tap, MID, 86, false, false),
            Action::Wake { first_tap: false }
        );
        // With media, it may also start a double tap for the next track,
        // on the clock face it woke to.
        assert_eq!(
            off(Kind::Tap, MID, 86, false, true),
            Action::Wake { first_tap: true }
        );
    }

    #[test]
    fn a_brief_tap_waits_for_a_second() {
        assert_eq!(off(Kind::Tap, MID, 20, false, false), Action::FirstTap);
        assert_eq!(off(Kind::Tap, MID, 20, false, true), Action::FirstTap);
    }

    #[test]
    fn a_double_tap_wakes_without_media_and_skips_with() {
        assert_eq!(
            off(Kind::Tap, MID, 20, true, false),
            Action::Wake { first_tap: false }
        );
        assert_eq!(off(Kind::Tap, MID, 20, true, true), Action::Next);
        // Even if the second tap is deliberate on its own.
        assert_eq!(off(Kind::Tap, MID, 90, true, true), Action::Next);
    }

    #[test]
    fn a_long_press_plays_or_pauses_only_with_media() {
        assert_eq!(off(Kind::LongPress, MID, 600, false, true), Action::PlayPause);
        assert_eq!(off(Kind::LongPress, MID, 600, false, false), Action::Ignore);
        // A tap just before doesn't change that.
        assert_eq!(off(Kind::LongPress, MID, 600, true, true), Action::PlayPause);
    }

    #[test]
    fn swipes_do_nothing() {
        for media in [false, true] {
            for recent_tap in [false, true] {
                assert_eq!(
                    off(Kind::Swipe, MID, 120, recent_tap, media),
                    Action::Ignore
                );
            }
        }
    }

    #[test]
    fn nothing_in_the_quiet_night() {
        for kind in [Kind::Tap, Kind::LongPress, Kind::Swipe] {
            for (recent_tap, media) in [(false, false), (true, false), (false, true), (true, true)]
            {
                assert_eq!(
                    screen_off(kind, MID.0, MID.1, 120, recent_tap, media, true),
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
    /// land outside the quiet night with media playing: canvas positions
    /// where each started.
    #[test]
    fn the_field_logs_phantom_long_presses_at_the_edge_do_nothing() {
        // 9733 ms and 9734 ms, held still.
        assert_eq!(off(Kind::LongPress, (66, 6), 600, false, true), Action::Ignore);
        assert_eq!(off(Kind::LongPress, (72, 8), 600, false, true), Action::Ignore);
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
}
