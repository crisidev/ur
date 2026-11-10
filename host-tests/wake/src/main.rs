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

    /// Tap, tap: play or pause, with the screen left off.
    #[test]
    fn a_double_tap_wakes_without_a_phone_and_plays_or_pauses_with() {
        assert_eq!(
            off(Kind::Tap, MID, 20, true, false),
            Action::Wake { first_tap: false }
        );
        assert_eq!(off(Kind::Tap, MID, 20, true, true), Action::PlayPause);
        // Even if the second tap is deliberate on its own.
        assert_eq!(off(Kind::Tap, MID, 90, true, true), Action::PlayPause);
    }

    /// Tap, then tap-and-hold: the next track, with the screen left off.
    /// Without a phone, the hold does nothing, as a long press always did.
    #[test]
    fn a_tap_and_hold_skips_with_a_phone() {
        assert_eq!(off(Kind::LongPress, MID, 600, true, true), Action::Next);
        assert_eq!(off(Kind::LongPress, MID, 600, true, false), Action::Ignore);
    }

    /// The second touch of a pair must come within half a second of the
    /// first tap's lift: a tap's lift, or a hold's start (it's reported
    /// 0.6 s after that, still down).
    #[test]
    fn a_pair_s_second_touch_comes_within_half_a_second() {
        assert_eq!(DOUBLE_TAP_MS, 500);
        assert!(follows(Kind::Tap, 500, 50));
        assert!(!follows(Kind::Tap, 501, 50));
        assert!(follows(Kind::LongPress, 1100, 600));
        assert!(!follows(Kind::LongPress, 1101, 600));
        // At once, too: the touch task can't report a touch before the one
        // before it lifted, but the sums mustn't go wrong if it did.
        assert!(follows(Kind::LongPress, 600, 600));
        assert!(follows(Kind::LongPress, 0, 600));
        // The wearer's double taps from off (0.9.0-m3.1's field log):
        // second taps of 37–57 ms, 126–156 ms after the first.
        assert!(follows(Kind::Tap, 156, 57));
    }

    /// What makes a touch a pair's second, wherever the screen is: a tap
    /// away from the edge, or a hold in the middle.
    #[test]
    fn a_pair_s_second_touch_is_a_tap_inside_or_a_hold_in_the_middle() {
        assert_eq!(second(Kind::Tap, 40, 40), Some(Action::PlayPause));
        assert_eq!(second(Kind::Tap, 15, 70), Some(Action::PlayPause));
        assert_eq!(second(Kind::Tap, 40, 4), None);
        assert_eq!(second(Kind::LongPress, 40, 40), Some(Action::Next));
        assert_eq!(second(Kind::LongPress, 40, 12), None);
        assert_eq!(second(Kind::Swipe, 40, 40), None);
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

    /// A long press on its own does nothing, anywhere, with the screen off
    /// or on a glance: an arm folded over the watch made them.
    #[test]
    fn a_lone_long_press_does_nothing() {
        for x in (0..80).step_by(4) {
            for y in (0..80).step_by(4) {
                for phone in [false, true] {
                    assert_eq!(
                        off(Kind::LongPress, (x, y), 600, false, phone),
                        Action::Ignore
                    );
                }
                assert_eq!(glance(Kind::LongPress, x, y, 600, false), Action::Ignore);
            }
        }
    }

    /// The middle's corners, in canvas pixels: the first and last columns
    /// and rows a long press may start in, with the screen off or on a
    /// glance.
    const LEFT: i32 = LONG_PRESS_MARGIN_X;
    const RIGHT: i32 = 79 - LONG_PRESS_MARGIN_X;
    const TOP: i32 = LONG_PRESS_MARGIN_Y;
    const BOTTOM: i32 = 79 - LONG_PRESS_MARGIN_Y;

    /// Just inside the middle, and just outside it, on each side.
    fn just_inside() -> [(i32, i32); 4] {
        [(LEFT, TOP), (RIGHT, BOTTOM), (LEFT, BOTTOM), (RIGHT, TOP)]
    }
    fn just_outside() -> [(i32, i32); 4] {
        [
            (LEFT - 1, 40),
            (40, TOP - 1),
            (RIGHT + 1, 40),
            (40, BOTTOM + 1),
        ]
    }

    /// A tap-and-hold's hold only in the middle: an arm folded over the
    /// watch rests against its edge. Just inside counts, just outside
    /// doesn't, on each side, even where a tap would wake the screen (or
    /// play or pause, as a double tap's second).
    #[test]
    fn a_tap_and_hold_skips_only_in_the_middle() {
        for at in just_inside() {
            assert_eq!(off(Kind::LongPress, at, 600, true, true), Action::Next);
        }
        for at in just_outside() {
            assert!(inside(at.0, at.1), "{at:?} is away from a tap's edge");
            assert_eq!(off(Kind::LongPress, at, 600, true, true), Action::Ignore);
            assert_eq!(
                off(Kind::Tap, at, 86, false, true),
                Action::Wake { first_tap: true }
            );
            assert_eq!(off(Kind::Tap, at, 20, true, true), Action::PlayPause);
        }
    }

    /// A brush at the edge does nothing, as either touch of a pair, with
    /// the screen off or on a glance, however long, and doesn't count as a
    /// first tap either.
    #[test]
    fn an_edge_brush_does_nothing() {
        for at in [(3, 40), (40, 2), (77, 40), (40, 76)] {
            for recent_tap in [false, true] {
                for kind in [Kind::Tap, Kind::LongPress] {
                    for held_ms in [9, 28, 86, 600] {
                        assert_eq!(
                            off(kind, at, held_ms, recent_tap, true),
                            Action::Ignore,
                            "{kind:?} {at:?} {held_ms} ms"
                        );
                    }
                }
                // A light tap: neither a pair's first nor its second.
                assert_eq!(
                    glance(Kind::Tap, at.0, at.1, 20, recent_tap),
                    Action::Ignore
                );
                assert_eq!(
                    glance(Kind::LongPress, at.0, at.1, 600, recent_tap),
                    Action::Ignore
                );
            }
        }
    }

    /// The middle is 16..=63 across, twice a tap's edge from the sides, and
    /// 20..=59 down, a quarter of the screen from the top and bottom, where
    /// a folded arm lies.
    #[test]
    fn the_middle_leaves_a_fifth_at_the_sides_and_a_quarter_at_the_ends() {
        assert_eq!(LONG_PRESS_MARGIN_X, 2 * TAP_MARGIN);
        assert_eq!((LEFT, RIGHT, TOP, BOTTOM), (16, 63, 20, 59));
        for (x, y) in just_inside() {
            assert!(centred(x, y), "{:?}", (x, y));
        }
        for (x, y) in just_outside() {
            assert!(!centred(x, y), "{:?}", (x, y));
        }
    }

    /// The field logs' long presses by the wearer: a 0.9.0 milestone's
    /// (each followed by the phone's new state), the other mid-screen ones
    /// in older logs, and 0.10.0-rc.1's with the screen off or on a glance
    /// (the last at 59 across, on a glance, pausing what a folded arm had
    /// played 3 s before). All in the middle, so where the wearer holds,
    /// a tap-and-hold skips.
    #[test]
    fn the_field_logs_long_presses_by_the_wearer_would_skip_as_a_hold() {
        let played = [(55, 40), (47, 30), (47, 39), (49, 35), (45, 33), (50, 35)];
        let older = [(39, 51), (45, 52), (48, 44), (57, 45)];
        let rc1 = [(43, 47), (52, 48), (46, 53), (59, 32)];
        for at in played.into_iter().chain(older).chain(rc1) {
            assert!(centred(at.0, at.1), "{at:?}");
            assert_eq!(off(Kind::LongPress, at, 600, true, true), Action::Next);
            assert_eq!(glance(Kind::LongPress, at.0, at.1, 600, true), Action::Next);
        }
    }

    /// 0.10.0-rc.1's field log, a day with the wearer's arms folded now and
    /// then: long presses that played or paused, each undone (on the watch
    /// or the phone) seconds or minutes later, or one of a run at the edge.
    /// Each started near the top or bottom edge, though further in than a
    /// tap's margin, so none does anything now.
    #[test]
    fn the_field_logs_folded_arm_long_presses_do_nothing() {
        // With the screen off: held 9.7 s (the phone paused again 165 s
        // later), and 1.3 s (the wearer paused again 21 s later). Not
        // even after a tap, which neither had.
        for at in [(44, 11), (41, 12)] {
            assert!(inside(at.0, at.1), "{at:?}");
            for recent_tap in [false, true] {
                assert_eq!(
                    off(Kind::LongPress, at, 600, recent_tap, true),
                    Action::Ignore
                );
            }
        }
        // On a glance: 64 down (the phone paused again 128 s later), and
        // at the top edge (the first of three there in 6 s).
        for at in [(46, 64), (35, 3)] {
            assert_eq!(
                glance(Kind::LongPress, at.0, at.1, 600, false),
                Action::Ignore
            );
        }
        // Likewise on a glance, 66 down, 0.6 s after it lit; it paused,
        // and nothing undid that for 24 minutes, so perhaps meant, but
        // lower than any long press the wearer certainly made.
        assert_eq!(glance(Kind::LongPress, 48, 66, 600, false), Action::Ignore);
    }

    /// Two the arm made that no margin turns away: held 2.2 s at 25
    /// across, 29 down, with the screen off (the wearer paused again 3 s
    /// later, on a glance, at (59, 32)); and one dead centre, 3 s after a
    /// 369 ms tap woke the screen, on the clock face, which may well have
    /// been meant. They look like the wearer's. The first now does
    /// nothing, as no tap came just before it (a 417 ms one at the top
    /// edge, 1 s before it landed); the second still plays or pauses, on
    /// the awake clock face.
    #[test]
    fn the_field_logs_mid_screen_long_presses() {
        assert_eq!(off(Kind::Tap, (34, 6), 417, false, true), Action::Ignore);
        assert!(!follows(Kind::LongPress, 1622, 600));
        assert_eq!(
            off(Kind::LongPress, (25, 29), 600, false, true),
            Action::Ignore
        );
        assert!(awake_long_press(47, 48, true));
    }

    /// 0.10.0-rc.1's field log: the folded arm's taps followed by a hold
    /// within half a second, each at the top edge. With the screen off, a
    /// 9 ms tap at (34, 4), then a hold at (31, 4) starting 361 ms after
    /// its lift; on a glance, a 254 ms tap at (29, 3), which woke the
    /// screen, then a hold at (34, 3), 9 ms after. Neither skips: the edge
    /// is out of bounds for both touches.
    #[test]
    fn the_field_logs_folded_arm_tap_and_holds_do_nothing() {
        assert!(follows(Kind::LongPress, 961, 600));
        assert_eq!(off(Kind::Tap, (34, 4), 9, false, true), Action::Ignore);
        assert_eq!(
            off(Kind::LongPress, (31, 4), 600, true, true),
            Action::Ignore
        );
        assert_eq!(
            glance(Kind::Tap, 29, 3, 254, false),
            Action::Wake { first_tap: false }
        );
        assert_eq!(second(Kind::LongPress, 34, 3), None);
    }

    /// What still gets through: in the same log, two light taps the arm
    /// made 9–10 pixels from the bottom edge, 117 ms apart, with the screen
    /// off. Inside a tap's margin, so a double tap: they skipped then, and
    /// would play or pause now. (See the TODO in `wake`'s docs.)
    #[test]
    fn the_field_logs_folded_arm_double_tap_plays_or_pauses() {
        assert_eq!(off(Kind::Tap, (15, 71), 18, false, true), Action::FirstTap);
        assert!(follows(Kind::Tap, 149, 32));
        assert_eq!(off(Kind::Tap, (15, 70), 32, true, true), Action::PlayPause);
    }

    /// On the awake clock face, a tap-and-hold's hold anywhere: something
    /// woke the screen first.
    /// Unless a touch woke it just before, which may have been the folded
    /// arm: then only in the middle, as on a glance.
    #[test]
    fn the_awake_clock_face_takes_a_long_press_anywhere_unless_a_touch_just_woke_it() {
        for at in just_inside().into_iter().chain(just_outside()) {
            assert!(awake_long_press(at.0, at.1, false), "{at:?}");
        }
        assert!(awake_long_press(0, 0, false));
        for at in just_inside() {
            assert!(awake_long_press(at.0, at.1, true), "{at:?}");
        }
        for at in just_outside() {
            assert!(!awake_long_press(at.0, at.1, true), "{at:?}");
        }
    }

    /// Whether a long press `ms` after a touch woke the screen comes while
    /// the clock face still needs it in the middle.
    fn just_woken(ms: u32) -> bool {
        ms < CENTRE_AFTER_TOUCH_WAKE_MS
    }

    /// 0.10.0-rc.1's field log: a 254 ms tap at the top edge of a glance
    /// woke the screen, and two long presses at the top edge, 0.61 s and
    /// 2.75 s later, played and paused on the clock face; both count as
    /// just after the wake, so do nothing now. The wearer's own, 1.28 s
    /// and 4.96 s after a swipe woke the screen from a glance, and 2.90 s
    /// after a tap woke it from off, were in the middle, so count either
    /// way (now as a tap-and-hold's hold, which skips).
    #[test]
    fn the_field_logs_long_presses_just_after_a_touch_woke_the_screen() {
        assert_eq!(
            glance(Kind::Tap, 29, 3, 254, false),
            Action::Wake { first_tap: false }
        );
        for (x, y, ms) in [(34, 3, 610), (31, 3, 2750)] {
            assert!(just_woken(ms), "{ms} ms");
            assert!(!awake_long_press(x, y, just_woken(ms)), "{:?}", (x, y));
        }
        for (x, y, ms) in [(47, 46, 1280), (47, 48, 2900), (48, 48, 4960)] {
            assert!(awake_long_press(x, y, just_woken(ms)), "{:?}", (x, y));
        }
    }

    /// Only for 3 s: soon after a deliberate wake, a long press at the edge
    /// counts again, as the wearer would rather.
    #[test]
    fn the_clock_face_needs_the_middle_only_for_3_s_after_a_touch_wake() {
        assert_eq!(CENTRE_AFTER_TOUCH_WAKE_MS, 3000);
        assert!(!awake_long_press(40, 4, just_woken(2999)));
        assert!(awake_long_press(40, 4, just_woken(3000)));
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
        // 980 ms, 11 canvas pixels from the top: away from a tap's edge,
        // so it would have played or paused before long presses had to be
        // in the middle. (Its next two taps are the last pair in
        // `the_field_logs_phantom_double_taps_at_the_edge_do_nothing`.)
        assert_eq!(
            off(Kind::LongPress, (55, 11), 600, false, true),
            Action::Ignore
        );
        // Twice, 600 ms apart (614 and 1847 ms), at the top edge.
        assert_eq!(
            off(Kind::LongPress, (30, 4), 600, false, true),
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

    fn tap_on_glance(held_ms: u16, recent_tap: bool) -> Action {
        glance(Kind::Tap, MID.0, MID.1, held_ms, recent_tap)
    }

    /// On a glance, as from off: a deliberate tap wakes the screen at once
    /// (and may be the first of a pair finished on the clock face), a light
    /// one waits for a second, and a second tap plays or pauses.
    #[test]
    fn a_glance_wakes_on_a_deliberate_tap_and_plays_or_pauses_on_a_light_double_tap() {
        assert_eq!(tap_on_glance(86, false), Action::Wake { first_tap: true });
        assert_eq!(
            tap_on_glance(DELIBERATE_TAP_MS, false),
            Action::Wake { first_tap: true }
        );
        assert_eq!(
            tap_on_glance(DELIBERATE_TAP_MS - 1, false),
            Action::FirstTap
        );
        assert_eq!(tap_on_glance(20, false), Action::FirstTap);
        // The second tap plays or pauses whatever it's held for.
        assert_eq!(tap_on_glance(20, true), Action::PlayPause);
        assert_eq!(tap_on_glance(86, true), Action::PlayPause);
    }

    /// The wearer is looking, so a deliberate tap at the edge wakes the
    /// screen from a glance; but it can't start a media gesture there, nor
    /// can a light one, which does nothing.
    #[test]
    fn a_glance_wakes_on_a_tap_at_the_edge_but_no_media_gesture_starts_there() {
        assert_eq!(
            glance(Kind::Tap, 2, 40, 86, false),
            Action::Wake { first_tap: false }
        );
        assert_eq!(glance(Kind::Tap, 40, 77, 20, false), Action::Ignore);
        assert_eq!(glance(Kind::Tap, 78, 2, 20, true), Action::Ignore);
        assert_eq!(
            glance(Kind::Tap, 78, 2, 86, true),
            Action::Wake { first_tap: false }
        );
    }

    /// A tap-and-hold skips on a glance, with its hold in the middle, as
    /// with the screen off: folding the arms turns the wrist, which can
    /// light a glance, as the other arm comes to rest against the watch's
    /// edge. Nearer the edge, or without the tap, it does nothing, and the
    /// glance goes on.
    #[test]
    fn a_glance_skips_on_a_tap_and_hold_in_the_middle() {
        assert_eq!(glance(Kind::LongPress, 40, 40, 600, true), Action::Next);
        assert_eq!(glance(Kind::LongPress, 40, 40, 600, false), Action::Ignore);
        for (x, y) in just_inside() {
            assert_eq!(glance(Kind::LongPress, x, y, 600, true), Action::Next);
        }
        for (x, y) in just_outside().into_iter().chain([(0, 0)]) {
            assert_eq!(glance(Kind::LongPress, x, y, 600, true), Action::Ignore);
        }
    }

    /// A swipe wakes the screen properly, wherever it starts, after a
    /// tap too.
    #[test]
    fn a_swipe_on_a_glance_wakes_the_screen() {
        for (x, y) in [MID, (2, 40)] {
            for recent_tap in [false, true] {
                assert_eq!(
                    glance(Kind::Swipe, x, y, 120, recent_tap),
                    Action::Wake { first_tap: false }
                );
            }
        }
    }
}
