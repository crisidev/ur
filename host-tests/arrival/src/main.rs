//! Host tests for the firmware's `arrival` module: when a notification
//! arrived, as the history shows it. `cargo test` runs them.

#[path = "../../../src/arrival.rs"]
mod arrival;

fn main() {
    println!("run `cargo test`");
}

#[cfg(test)]
mod tests {
    use super::arrival::{self, MAX_LEN};

    const DAY: i64 = 24 * 60;
    /// 2026-11-08, a Sunday, in local minutes since 1970.
    const SUNDAY: i64 = 20_765 * DAY;

    fn label(at: i64, now: i64) -> String {
        let mut out = String::new();
        arrival::write(&mut out, at, now).expect("writing to a String");
        assert!(out.len() <= MAX_LEN, "{out:?} is longer than MAX_LEN");
        out
    }

    fn at(day: i64, hour: i64, minute: i64) -> i64 {
        SUNDAY + day * DAY + hour * 60 + minute
    }

    #[test]
    fn today_is_the_time_alone() {
        assert_eq!(label(at(0, 14, 32), at(0, 14, 32)), "14:32");
        assert_eq!(label(at(0, 0, 0), at(0, 23, 59)), "00:00");
        assert_eq!(label(at(0, 9, 5), at(0, 18, 0)), "09:05");
    }

    #[test]
    fn yesterday_is_by_weekday() {
        assert_eq!(label(at(-1, 23, 10), at(0, 0, 1)), "SAT 23:10");
        assert_eq!(label(at(-1, 0, 0), at(0, 23, 59)), "SAT 00:00");
    }

    #[test]
    fn this_week_is_by_weekday() {
        assert_eq!(label(at(-2, 9, 5), at(0, 8, 0)), "FRI 09:05");
        assert_eq!(label(at(-6, 12, 0), at(0, 8, 0)), "MON 12:00");
        // Midnight belongs to the day it starts.
        assert_eq!(label(at(-3, 0, 0), at(0, 0, 0)), "THU 00:00");
    }

    #[test]
    fn older_is_how_long_ago() {
        assert_eq!(label(at(-7, 12, 0), at(0, 8, 0)), "7 DAYS");
        assert_eq!(label(at(-30, 23, 59), at(0, 0, 0)), "30 DAYS");
        // The furthest apart a stored (`u32`, non-zero) arrival and a
        // `u32` now can be still fits.
        assert_eq!(label(1, u32::MAX.into()), "2982616 DAYS");
    }

    #[test]
    fn a_clock_moved_back_shows_the_time() {
        // Arrived just after midnight; the time zone moved the clock back
        // an hour, to before it.
        assert_eq!(label(at(0, 0, 10), at(-1, 23, 30)), "00:10");
    }

    #[test]
    fn weekdays_match_the_calendar() {
        // 1970-01-01 was a Thursday; 2000-01-01 a Saturday.
        assert_eq!(label(0, 3 * DAY), "THU 00:00");
        let y2k = 10_957 * DAY;
        assert_eq!(label(y2k + 61, y2k + 2 * DAY), "SAT 01:01");
    }
}
