//! Fuzzing, with Bolero: a `nav` message's distance and action are
//! strings from the phone, in whatever locale and app it has. The tests
//! above try the forms the apps send; these try any string, and distances
//! built of any digits, separator, spaces and unit, against a reading of
//! the module's docs done with exact integers.
//!
//! `cargo test` runs each harness over a few hundred random inputs (and
//! any saved in a corpus), so they're regression tests too. To fuzz one
//! properly (nightly, for the sanitizer):
//!
//! ```text
//! cargo +nightly bolero test fuzz::any_distance_reads_as_written --engine libfuzzer -T 10min
//! ```

use super::route::{NEAR_METRES, Route, STALE_AFTER_MS, Step, Turn, metres};

/// The units Gadgetbridge's distances come in, and millimetres in each
/// (a foot is 304.8 mm, a yard 914.4: `metres` rounds them to the mm).
const UNITS: [(&str, u128); 5] = [
    ("m", 1000),
    ("km", 1_000_000),
    ("ft", 305),
    ("yd", 914),
    ("mi", 1_609_344),
];

/// Any string reads the same with its spaces moved about, or its letters
/// in capitals: spaces anywhere are skipped, and units are any case. And
/// it's no distance without a digit.
#[test]
fn any_string_reads_regardless_of_spaces_and_case() {
    bolero::check!().for_each(|bytes: &[u8]| {
        let text = String::from_utf8_lossy(bytes);
        let read = metres(&text);
        assert_eq!(metres(&text.replace(' ', "")), read, "{text:?}");
        assert_eq!(metres(&format!(" {} ", text.replace(' ', "  "))), read);
        assert_eq!(metres(&text.to_ascii_uppercase()), read, "{text:?}");
        if !text.bytes().any(|b| b.is_ascii_digit()) {
            assert_eq!(read, None, "{text:?}");
        }
    });
}

/// A distance written as digits, perhaps a decimal point or comma and
/// more digits, then a unit (any case, spaces anywhere), reads as that
/// many metres: to the thousandth of a unit, rounded down to the metre,
/// and `None` if that's beyond a `u32`.
#[test]
fn any_distance_reads_as_written() {
    bolero::check!()
        .with_type::<(Vec<u8>, Option<(bool, Vec<u8>)>, u8, u16, u8)>()
        .for_each(|(whole, fraction, unit, spaces, case)| {
            let digits = |bytes: &[u8]| -> String {
                bytes.iter().map(|b| char::from(b'0' + b % 10)).collect()
            };
            let whole = digits(whole);
            let fraction = fraction.as_ref().map(|(comma, f)| (*comma, digits(f)));
            let (unit, per_unit) = UNITS[usize::from(*unit) % UNITS.len()];

            let mut text = whole.clone();
            if let Some((comma, fraction)) = &fraction {
                text.push(if *comma { ',' } else { '.' });
                text.push_str(fraction);
            }
            text.push_str(unit);
            // Spaces where `spaces`' bits say, and capitals where `case`'s.
            let text: String = text
                .chars()
                .enumerate()
                .flat_map(|(i, c)| {
                    let c = if case >> (i % 8) & 1 != 0 {
                        c.to_ascii_uppercase()
                    } else {
                        c
                    };
                    let space = (spaces >> (i % 16) & 1 != 0).then_some(' ');
                    space.into_iter().chain([c])
                })
                .collect();

            // Thousandths of the unit, exactly: the whole part (without
            // its leading zeros, to fit), and the fraction's first three
            // digits.
            let fraction = fraction.map(|(_, f)| f).unwrap_or_default();
            let expected = if whole.is_empty() && fraction.is_empty() {
                None
            } else {
                let significant = whole.trim_start_matches('0');
                let thousandths =
                    format!("{significant}{:0<3}", &fraction[..fraction.len().min(3)]);
                let thousandths = thousandths.trim_start_matches('0');
                if thousandths.len() > 30 {
                    None
                } else {
                    let thousandths: u128 = if thousandths.is_empty() {
                        0
                    } else {
                        thousandths.parse().expect("digits")
                    };
                    u32::try_from(thousandths * per_unit / 1_000_000).ok()
                }
            };
            assert_eq!(metres(&text), expected, "{text:?}");
        });
}

/// Gadgetbridge's action names, and the turns they are.
const ACTIONS: [(&str, Turn); 17] = [
    ("continue", Turn::Straight),
    ("left", Turn::Left),
    ("left_slight", Turn::SlightLeft),
    ("left_sharp", Turn::SharpLeft),
    ("right", Turn::Right),
    ("right_slight", Turn::SlightRight),
    ("right_sharp", Turn::SharpRight),
    ("keep_left", Turn::KeepLeft),
    ("keep_right", Turn::KeepRight),
    ("uturn_left", Turn::UturnLeft),
    ("uturn_right", Turn::UturnRight),
    ("offroute", Turn::OffRoute),
    ("roundabout_right", Turn::RoundaboutRight),
    ("roundabout_left", Turn::RoundaboutLeft),
    ("roundabout_straight", Turn::RoundaboutStraight),
    ("roundabout_uturn", Turn::RoundaboutUturn),
    ("finish", Turn::Finish),
];

/// Any action that isn't one of Gadgetbridge's names is
/// [`Turn::Unknown`].
#[test]
fn any_other_action_is_unknown() {
    bolero::check!().for_each(|bytes: &[u8]| {
        let action = String::from_utf8_lossy(bytes);
        let expected = ACTIONS
            .iter()
            .find(|(name, _)| *name == action)
            .map_or(Turn::Unknown, |&(_, turn)| turn);
        assert_eq!(Turn::from_action(Some(&action)), expected);
    });
}

/// Whatever steps arrive, and whenever: a turn buzzes once, when it first
/// comes within [`NEAR_METRES`], unless it's carrying straight on or off
/// route. "A turn" is a run of steps with the same manoeuvre and street,
/// whose distance doesn't jump back up by more than 30 m; an end of
/// navigation, or [`STALE_AFTER_MS`] without a step, ends it too.
#[test]
fn a_turn_buzzes_once_as_it_comes_near() {
    bolero::check!()
        .with_type::<Vec<(u8, u8, u16, u32)>>()
        .for_each(|steps| {
            const STREETS: [&str; 3] = ["Storgata", "Akersgata", ""];
            let mut route: Option<Route> = None;
            let mut now = 0u64;
            // The turn under way: its manoeuvre, street, latest distance,
            // and whether it buzzed.
            let mut turn: Option<(Turn, &str, Option<u32>, bool)> = None;
            for &(what, street, distance, wait) in steps {
                now += u64::from(wait % 400_000);
                if Route::expire(&mut route, now) {
                    turn = None;
                }
                assert_eq!(route.is_none(), turn.is_none());
                // Now and then, the end of navigation.
                if what == 0xFF {
                    assert!(!Route::update(&mut route, None, now));
                    assert!(route.is_none());
                    turn = None;
                    continue;
                }
                let action = ACTIONS[usize::from(what) % ACTIONS.len()];
                let action = (what < 0xF0).then_some(action);
                let kind = action.map_or(Turn::Unknown, |(_, turn)| turn);
                let street = STREETS[usize::from(street) % STREETS.len()];
                // Mostly metres, close by; now and then something else.
                let distance_text = match distance >> 14 {
                    0 | 1 => format!("{} m", distance % 200),
                    2 => format!("{}.{} km", distance % 10, distance % 7),
                    _ => format!("{}", distance % 100),
                };
                let distance = metres(&distance_text);
                let step = Step {
                    turn: Turn::from_action(action.map(|(name, _)| name)),
                    street: street.try_into().expect("streets fit"),
                    distance: distance_text.as_str().try_into().expect("distances fit"),
                };
                let buzzed = Route::update(&mut route, Some(step.clone()), now);
                assert_eq!(route.as_ref().map(|r| &r.step), Some(&step));

                let same = turn.filter(|&(k, s, before, _)| {
                    k == kind
                        && s == street
                        && match (before, distance) {
                            (Some(before), Some(now)) => now <= before + 30,
                            _ => true,
                        }
                });
                let warned = same.is_some_and(|(.., warned)| warned);
                let near = distance.is_some_and(|m| m <= NEAR_METRES);
                let worth = !matches!(kind, Turn::Straight | Turn::OffRoute);
                assert_eq!(buzzed, !warned && near && worth, "{step:?} after {turn:?}");
                turn = Some((kind, street, distance, warned || buzzed));
            }
            // And it all goes stale in the end.
            assert_eq!(
                Route::expire(&mut route, now + STALE_AFTER_MS),
                turn.is_some()
            );
            assert!(route.is_none());
        });
}
