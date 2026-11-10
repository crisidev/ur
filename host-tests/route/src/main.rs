//! Host tests for the firmware's `route` module: turn-by-turn directions
//! from Gadgetbridge's `nav` messages. `cargo test` runs them.

#[path = "../../../src/route.rs"]
#[allow(dead_code)]
mod route;

fn main() {
    println!("run `cargo test`");
}

#[cfg(test)]
mod tests {
    use super::route::*;

    fn step(turn: Turn, street: &str, distance: &str) -> Option<Step> {
        Some(Step {
            turn,
            street: street.try_into().expect("test streets fit"),
            distance: distance.try_into().expect("test distances fit"),
        })
    }

    /// A `nav` message at `at` ms; whether it buzzed.
    fn nav(route: &mut Option<Route>, at: u64, turn: Turn, street: &str, distance: &str) -> bool {
        Route::update(route, step(turn, street, distance), at)
    }

    #[test]
    fn actions_by_gadgetbridge_name() {
        // Gadgetbridge's `navActions`, in `ACTION_*` order from 1.
        let names = [
            "continue",
            "left",
            "left_slight",
            "left_sharp",
            "right",
            "right_slight",
            "right_sharp",
            "keep_left",
            "keep_right",
            "uturn_left",
            "uturn_right",
            "offroute",
            "roundabout_right",
            "roundabout_left",
            "roundabout_straight",
            "roundabout_uturn",
            "finish",
        ];
        for name in names {
            assert_ne!(Turn::from_action(Some(name)), Turn::Unknown, "{name}");
        }
        assert_eq!(Turn::from_action(Some("left_sharp")), Turn::SharpLeft);
        assert_eq!(
            Turn::from_action(Some("roundabout_uturn")),
            Turn::RoundaboutUturn
        );
        assert_eq!(Turn::from_action(Some("merge")), Turn::Unknown);
        assert_eq!(Turn::from_action(None), Turn::Unknown);
    }

    #[test]
    fn distances_in_any_unit() {
        assert_eq!(metres("150 m"), Some(150));
        assert_eq!(metres("150m"), Some(150)); // OsmAnd
        assert_eq!(metres(" 20 M "), Some(20));
        assert_eq!(metres("1.2 km"), Some(1200));
        assert_eq!(metres("1,2 km"), Some(1200)); // a decimal comma
        assert_eq!(metres("0.3 mi"), Some(482));
        assert_eq!(metres("500 ft"), Some(152));
        assert_eq!(metres("100 yd"), Some(91));
        assert_eq!(metres(".5 km"), Some(500));
        assert_eq!(metres("12.3456 km"), Some(12345));
    }

    #[test]
    fn distances_that_arent() {
        assert_eq!(metres(""), None);
        assert_eq!(metres("km"), None);
        assert_eq!(metres("150"), None);
        assert_eq!(metres("150 parsecs"), None);
        assert_eq!(metres("1.2.3 km"), None);
        assert_eq!(metres("99999999 km"), None);
    }

    #[test]
    fn buzzes_once_as_a_turn_comes_near() {
        let mut route = None;
        assert!(!nav(&mut route, 0, Turn::Left, "Storgata", "200 m"));
        assert!(!nav(&mut route, 1000, Turn::Left, "Storgata", "80 m"));
        assert!(nav(&mut route, 2000, Turn::Left, "Storgata", "50 m"));
        assert!(!nav(&mut route, 3000, Turn::Left, "Storgata", "30 m"));
        // GPS jitter doesn't make it a new turn.
        assert!(!nav(&mut route, 4000, Turn::Left, "Storgata", "45 m"));
        assert_eq!(
            route.as_ref().map(|r| r.step.distance.as_str()),
            Some("45 m")
        );
    }

    #[test]
    fn the_next_turn_buzzes_again() {
        let mut route = None;
        assert!(nav(&mut route, 0, Turn::Left, "Storgata", "40 m"));
        // A different street: the next turn, already close.
        assert!(nav(&mut route, 1000, Turn::Right, "Akersgata", "20 m"));
        // The same way onto the same name, but much further: a new turn.
        assert!(!nav(&mut route, 2000, Turn::Right, "Akersgata", "400 m"));
        assert!(nav(&mut route, 3000, Turn::Right, "Akersgata", "40 m"));
    }

    #[test]
    fn no_buzz_for_carrying_on_or_off_route_or_unknown_distances() {
        let mut route = None;
        assert!(!nav(&mut route, 0, Turn::Straight, "E18", "10 m"));
        assert!(!nav(&mut route, 1000, Turn::OffRoute, "", "10 m"));
        assert!(!nav(&mut route, 2000, Turn::Left, "Storgata", ""));
        // An unknown manoeuvre is still a manoeuvre.
        assert!(nav(&mut route, 3000, Turn::Unknown, "Storgata", "10 m"));
    }

    #[test]
    fn ends_when_the_phone_says_so() {
        let mut route = None;
        nav(&mut route, 0, Turn::Left, "Storgata", "200 m");
        assert!(route.is_some());
        assert!(!Route::update(&mut route, None, 1000));
        assert!(route.is_none());
    }

    #[test]
    fn goes_stale_without_updates() {
        let mut route = None;
        nav(&mut route, 1000, Turn::Left, "Storgata", "200 m");
        assert!(!Route::expire(&mut route, 1000 + STALE_AFTER_MS - 1));
        assert!(route.is_some());
        // An update keeps it going.
        nav(&mut route, 60_000, Turn::Left, "Storgata", "150 m");
        assert!(!Route::expire(&mut route, 1000 + STALE_AFTER_MS));
        assert!(Route::expire(&mut route, 60_000 + STALE_AFTER_MS));
        assert!(route.is_none());
        assert!(!Route::expire(&mut route, u64::MAX));
    }
}

#[cfg(test)]
mod fuzz;

/// Proofs, for every distance the watch can be sent, where the tests and
/// the fuzzing try some. `cargo kani` runs them (see CONTRIBUTING.md).
#[cfg(kani)]
mod proofs {
    use super::route::*;

    /// Any distance a [`Step`] can hold (any [`DISTANCE_LEN`] bytes or
    /// fewer; the phone's, so anything) reads without a panic or an
    /// overflow, and as a distance or not at all. And what it reads as
    /// leaves room for the 30 m of slack `Route::update` adds to it,
    /// unchecked, when it compares one update's distance with the last:
    /// the furthest eight bytes can say is "999999mi", 1.6 million km,
    /// well short of a `u32`'s metres.
    ///
    /// The bytes needn't be UTF-8 for this: `metres` only looks at them
    /// one at a time, and treats any it doesn't know as a unit's letter.
    #[kani::proof]
    #[kani::unwind(9)]
    fn any_distance_a_step_holds_reads_safely() {
        let bytes: [u8; DISTANCE_LEN] = kani::any();
        let len: usize = kani::any();
        kani::assume(len <= DISTANCE_LEN);
        // SAFETY: `metres` reads the bytes, never as characters.
        let distance = unsafe { core::str::from_utf8_unchecked(&bytes[..len]) };
        if let Some(metres) = metres(distance) {
            assert!(u64::from(metres) <= 1_609_344 * 999_999 / 1000);
            assert!(metres.checked_add(30).is_some());
        }
        kani::cover!(metres(distance) == Some(1_609_342_390), "999999mi");
    }
}
