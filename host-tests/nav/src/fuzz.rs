//! Fuzzing, with Bolero: the tests above try lines of a few fixed
//! lengths; these try every length of every line (empty ones too, and
//! lines that shrank under the screen shown), any place, and any swipes,
//! against what the module's docs promise of the screens' layout.
//!
//! `cargo test` runs each harness over a few hundred random inputs (and
//! any saved in a corpus), so they're regression tests too. To fuzz one
//! properly (nightly, for the sanitizer):
//!
//! ```text
//! cargo +nightly bolero test fuzz::any_swipe_goes_somewhere_real_and_back --engine libfuzzer -T 10min
//! ```

use super::nav::{Place, Side, Swipe, home, swipe};

const SWIPES: [Swipe; 4] = [Swipe::Left, Swipe::Right, Swipe::Up, Swipe::Down];

/// The swipe that brings in what's on `side`.
fn towards(side: Side) -> Swipe {
    *SWIPES
        .iter()
        .find(|s| s.towards() == side)
        .expect("a swipe for every side")
}

/// Whatever the lines' lengths, wherever on them (even past a line's end,
/// as when a screen just went away), any swipe goes to a screen that's
/// there, or to the clock, or nowhere: out along the line it's on, or
/// back. The way back is the reverse swipe, and home always leads to the
/// clock.
#[test]
fn any_swipe_goes_somewhere_real_and_back() {
    bolero::check!()
        .with_type::<([u8; 4], Option<(u8, u16)>, Vec<u8>)>()
        .for_each(|(lens, start, swipes)| {
            let len = |side: Side| usize::from(lens[side as usize] % 8);
            let mut place = match start {
                None => Place::Clock,
                Some((side, index)) => Place::Line {
                    side: Side::ALL[usize::from(*side) % 4],
                    index: usize::from(*index),
                },
            };
            for &s in swipes {
                let s = SWIPES[usize::from(s) % 4];
                let went = swipe(place, s, len);
                let sideways = match place {
                    Place::Clock => false,
                    Place::Line { side, .. } => {
                        s.towards() != side && s.towards() != side.opposite()
                    }
                };
                match went {
                    None => {
                        // Nowhere: sideways, past a line's end, or an
                        // empty line from the clock.
                        match place {
                            Place::Clock => assert_eq!(len(s.towards()), 0),
                            Place::Line { side, index } => {
                                assert!(sideways || (s.towards() == side && index + 1 >= len(side)))
                            }
                        }
                    }
                    Some((to, side)) => {
                        assert!(!sideways, "{s:?} from {place:?} to {to:?}");
                        assert_eq!(side, s.towards());
                        // A screen that's there.
                        if let Place::Line { side, index } = to {
                            assert!(index < len(side), "{to:?}");
                        }
                        // The reverse swipe comes back, from a place that
                        // was there.
                        let there = match place {
                            Place::Clock => true,
                            Place::Line { side, index } => index < len(side),
                        };
                        if there {
                            let back = towards(s.towards().opposite());
                            assert_eq!(swipe(to, back, len), Some((place, back.towards())));
                        }
                        place = to;
                    }
                }
            }

            // Home sweeps towards the clock, and swiping that way gets
            // there, a screen at a time.
            let mut steps = 0;
            while let Some(side) = home(place) {
                let (to, went) = swipe(place, towards(side), len).expect("home is a way to go");
                assert_eq!(went, side);
                place = to;
                steps += 1;
                assert!(steps <= 8, "not home after {steps} swipes");
            }
            assert_eq!(place, Place::Clock);
        });
}
