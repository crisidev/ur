//! Host tests for the firmware's `nav` module: where swipes go around the
//! clock face. `cargo test` runs them.

#[path = "../../../src/nav.rs"]
#[allow(dead_code)]
mod nav;

fn main() {
    println!("run `cargo test`");
}

#[cfg(test)]
mod tests {
    use super::nav::*;

    /// Lines with something in each: two screens left, right, and below,
    /// and three notifications above.
    fn full(side: Side) -> usize {
        match side {
            Side::Left | Side::Right | Side::Below => 2,
            Side::Above => 3,
        }
    }

    fn on(side: Side, index: usize) -> Place {
        Place::Line { side, index }
    }

    const SWIPES: [Swipe; 4] = [Swipe::Left, Swipe::Right, Swipe::Up, Swipe::Down];

    #[test]
    fn the_content_follows_the_finger() {
        // Swiping left brings in the screen to the right, and so on.
        assert_eq!(
            swipe(Place::Clock, Swipe::Left, full),
            Some((on(Side::Right, 0), Side::Right))
        );
        assert_eq!(
            swipe(Place::Clock, Swipe::Right, full),
            Some((on(Side::Left, 0), Side::Left))
        );
        assert_eq!(
            swipe(Place::Clock, Swipe::Down, full),
            Some((on(Side::Above, 0), Side::Above))
        );
        assert_eq!(
            swipe(Place::Clock, Swipe::Up, full),
            Some((on(Side::Below, 0), Side::Below))
        );
    }

    #[test]
    fn the_same_swipe_goes_further_out_the_opposite_one_back() {
        for s in SWIPES {
            let side = s.towards();
            let mut place = Place::Clock;
            let mut out = vec![];
            while let Some((next, towards)) = swipe(place, s, full) {
                assert_eq!(towards, side);
                out.push(next);
                place = next;
            }
            let expected: Vec<_> = (0..full(side)).map(|i| on(side, i)).collect();
            assert_eq!(out, expected, "out along {side:?}");

            // And all the way back, to the clock, which is where it stops.
            let back = SWIPES
                .into_iter()
                .find(|b| b.towards() == side.opposite())
                .expect("every side has an opposite swipe");
            let mut home = vec![];
            while let Some((next, towards)) = swipe(place, back, full) {
                assert_eq!(towards, side.opposite());
                home.push(next);
                place = next;
                if place == Place::Clock {
                    break;
                }
            }
            let mut expected: Vec<_> = (0..full(side) - 1).rev().map(|i| on(side, i)).collect();
            expected.push(Place::Clock);
            assert_eq!(home, expected, "back along {side:?}");
        }
    }

    #[test]
    fn no_wrapping_at_a_lines_end() {
        assert_eq!(swipe(on(Side::Right, 1), Swipe::Left, full), None);
        assert_eq!(swipe(on(Side::Above, 2), Swipe::Down, full), None);
    }

    #[test]
    fn sideways_does_nothing() {
        for side in Side::ALL {
            for index in 0..full(side) {
                for s in SWIPES {
                    // Back always goes somewhere; out, unless at the end.
                    let back = s.towards() == side.opposite();
                    let out = s.towards() == side && index + 1 < full(side);
                    let went = swipe(on(side, index), s, full);
                    assert_eq!(went.is_some(), back || out, "{s:?} from {side:?} {index}");
                }
            }
        }
    }

    #[test]
    fn an_empty_line_goes_nowhere() {
        // No notifications yet.
        let len = |side| if side == Side::Above { 0 } else { full(side) };
        assert_eq!(swipe(Place::Clock, Swipe::Down, len), None);
        assert!(swipe(Place::Clock, Swipe::Up, len).is_some());
        // Nothing below: no grid, no music, no air.
        let len = |side| if side == Side::Below { 0 } else { full(side) };
        assert_eq!(swipe(Place::Clock, Swipe::Up, len), None);
        assert!(swipe(Place::Clock, Swipe::Down, len).is_some());
    }

    #[test]
    fn a_screen_gone_from_the_end_comes_back_to_the_new_end() {
        // On the third notification's page, and the line somehow now has
        // only one: back goes to it, and further out goes nowhere.
        let len = |side| if side == Side::Above { 1 } else { full(side) };
        assert_eq!(
            swipe(on(Side::Above, 2), Swipe::Up, len),
            Some((on(Side::Above, 0), Side::Below))
        );
        assert_eq!(swipe(on(Side::Above, 2), Swipe::Down, len), None);
    }

    #[test]
    fn home_sweeps_towards_the_clock() {
        assert_eq!(home(Place::Clock), None);
        for side in Side::ALL {
            assert_eq!(home(on(side, 1)), Some(side.opposite()));
        }
    }
}

#[cfg(test)]
mod fuzz;

/// Proofs, for every place, swipe and length of every line at once, where
/// the tests try a layout and the fuzzing some thousands. `cargo kani`
/// runs them (see CONTRIBUTING.md).
#[cfg(kani)]
mod proofs {
    use super::nav::*;

    fn any_side() -> Side {
        Side::ALL[usize::from(kani::any::<u8>() % 4)]
    }

    fn any_swipe() -> Swipe {
        [Swipe::Left, Swipe::Right, Swipe::Up, Swipe::Down][usize::from(kani::any::<u8>() % 4)]
    }

    /// Wherever the UI is (even past its line's end, a screen having
    /// just gone), whatever the lines hold, a swipe that goes anywhere
    /// goes to the clock or to a screen that's there: `ui::App::navigate`
    /// indexes the line with what it gets, and an index past the end
    /// would be a panic on the watch. It goes the way the swipe says,
    /// and only along the line it's on.
    ///
    /// The one bound: an index short of `usize::MAX`, as `index + 1`
    /// would overflow there. The UI's are positions in a line of four
    /// screens at most.
    #[kani::proof]
    fn a_swipe_lands_on_a_screen_thats_there() {
        let lens: [usize; 4] = kani::any();
        let len = |side: Side| lens[side as usize];
        let place = if kani::any() {
            Place::Clock
        } else {
            let index: usize = kani::any();
            kani::assume(index < usize::MAX);
            Place::Line {
                side: any_side(),
                index,
            }
        };
        let swipe_ = any_swipe();
        let Some((to, towards)) = swipe(place, swipe_, len) else {
            return;
        };
        assert_eq!(towards, swipe_.towards());
        match (place, to) {
            (_, Place::Line { side, index }) => {
                assert!(index < len(side), "a screen that isn't there");
                match place {
                    // From the clock, the nearest screen on that side.
                    Place::Clock => assert!(side == towards && index == 0),
                    // Along its own line: one further out, or one back
                    // (from past the end: to the end).
                    Place::Line {
                        side: from,
                        index: was,
                    } => {
                        assert_eq!(side, from);
                        if towards == side {
                            assert_eq!(index, was + 1);
                        } else {
                            assert!(index < was);
                            assert!(index + 1 == was || index + 1 == len(side));
                        }
                    }
                }
            }
            // Only back from the nearest screen (or an emptied line)
            // reaches the clock.
            (Place::Line { side, index }, Place::Clock) => {
                assert_eq!(towards, side.opposite());
                assert!(index == 0 || len(side) == 0);
            }
            (Place::Clock, Place::Clock) => panic!("a swipe from the clock to itself"),
        }
    }

    /// From a screen that's there, a swipe that goes anywhere is undone
    /// by the opposite swipe: the way back is always the reverse. (Not
    /// from the clock into a line and back, which the first assertion
    /// covers too: that returns to the clock.)
    #[kani::proof]
    fn the_reverse_swipe_goes_back() {
        let lens: [usize; 4] = kani::any();
        let len = |side: Side| lens[side as usize];
        let place = if kani::any() {
            Place::Clock
        } else {
            let (side, index) = (any_side(), kani::any());
            kani::assume(index < len(side));
            Place::Line { side, index }
        };
        let swipe_ = any_swipe();
        if let Some((to, towards)) = swipe(place, swipe_, len) {
            let reverse = match swipe_ {
                Swipe::Left => Swipe::Right,
                Swipe::Right => Swipe::Left,
                Swipe::Up => Swipe::Down,
                Swipe::Down => Swipe::Up,
            };
            assert_eq!(swipe(to, reverse, len), Some((place, towards.opposite())));
        }
        // And home is the clock's side of wherever it is.
        match place {
            Place::Clock => assert_eq!(home(place), None),
            Place::Line { side, .. } => assert_eq!(home(place), Some(side.opposite())),
        }
    }
}
