//! When a notification arrived, as the history above the clock face shows
//! it (see [`crate::ui`]): the time alone if it came today, with the day
//! in front if not. Short, as it shares the page's header with the app's
//! name. Hardware-free, so it runs in host tests
//! (`host-tests/arrival`).
//!
//! Times are local minutes since 1970 (UTC plus the phone's offset, as
//! [`crate::alarm`] uses them), so a day is a local calendar day. A
//! notification keeps the offset it arrived under: after a time zone
//! change, it shows the time it was where it arrived.

use core::fmt::{self, Write};

const DAY: i64 = 24 * 60;

/// The longest label: "2982616 DAYS", at most, from a `u32` minute.
pub const MAX_LEN: usize = 16;

const WEEKDAYS: [&str; 7] = ["MON", "TUE", "WED", "THU", "FRI", "SAT", "SUN"];

/// Writes when local minute `at` was, seen from local minute `now`:
///
/// - Today: "14:32".
/// - Within the last week, yesterday too: "SAT 09:05", the weekday being
///   enough to tell which. ("YESTERDAY" took up half the header.)
/// - Before that: "9 DAYS". The hour hardly matters by then, and the
///   weekday alone would be ambiguous.
pub fn write(out: &mut impl Write, at: i64, now: i64) -> fmt::Result {
    let day = at.div_euclid(DAY);
    let days_ago = now.div_euclid(DAY) - day;
    match days_ago {
        // Today, or a day ahead: the clock moved back across midnight
        // since (a time zone change can do that). The time says enough.
        ..=0 => {}
        1..=6 => {
            // 1970-01-01, day 0, was a Thursday.
            let weekday = (day + 3).rem_euclid(7) as usize;
            write!(out, "{} ", WEEKDAYS[weekday])?;
        }
        _ => return write!(out, "{days_ago} DAYS"),
    }
    let minute = at.rem_euclid(DAY);
    write!(out, "{:02}:{:02}", minute / 60, minute % 60)
}
