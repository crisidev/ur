//! Fuzzing, with Bolero: the tests above try a week around one Sunday;
//! these try any arrival and any now a watch can hold (`u32` minutes,
//! with a time zone either way), against the three forms the module's
//! docs give, worked out here from a calendar anchor of their own.
//!
//! `cargo test` runs each harness over a few hundred random inputs (and
//! any saved in a corpus), so they're regression tests too. To fuzz one
//! properly (nightly, for the sanitizer):
//!
//! ```text
//! cargo +nightly bolero test fuzz::any_arrival_reads_as_documented --engine libfuzzer -T 10min
//! ```

use super::arrival::{self, MAX_LEN};

const DAY: i64 = 24 * 60;
/// 2026-11-08 was a Sunday: day 20765 since 1970.
const SUNDAY: i64 = 20_765;
const WEEKDAYS: [&str; 7] = ["SUN", "MON", "TUE", "WED", "THU", "FRI", "SAT"];

/// Any arrival, seen from any time (each a `u32` minute, as the history
/// stores them, shifted by a time zone of up to 18 hours either way):
/// the label fits in [`MAX_LEN`], and is the time of day ("14:32") if it
/// came today or seemingly after now, the weekday and the time ("SAT
/// 09:05") within the last six days, and how many days ago ("9 DAYS")
/// before that.
#[test]
fn any_arrival_reads_as_documented() {
    bolero::check!()
        .with_type::<(u32, i16, u32, i16, bool)>()
        .for_each(|&(at, at_zone, now, now_zone, near)| {
            let zone = |offset: i16| i64::from(offset) % (18 * 60 + 1);
            let at = i64::from(at) + zone(at_zone);
            // Now and then, close by, as most arrivals are.
            let now = if near {
                at + i64::from(now % (10 * DAY as u32))
            } else {
                i64::from(now) + zone(now_zone)
            };
            let mut label = String::new();
            arrival::write(&mut label, at, now).expect("writing to a String");
            assert!(label.len() <= MAX_LEN, "{label:?}");

            let days_ago = now.div_euclid(DAY) - at.div_euclid(DAY);
            let time = format!(
                "{:02}:{:02}",
                at.rem_euclid(DAY) / 60,
                at.rem_euclid(DAY) % 60
            );
            let weekday = WEEKDAYS[(at.div_euclid(DAY) - SUNDAY).rem_euclid(7) as usize];
            let expected = match days_ago {
                ..=0 => time,
                1..=6 => format!("{weekday} {time}"),
                _ => format!("{days_ago} DAYS"),
            };
            assert_eq!(label, expected, "{at} from {now}");
        });
}
