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
