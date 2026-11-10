//! Host tests for the firmware's `calendar` module. `cargo test` runs
//! them.

#[path = "../../../src/calendar.rs"]
#[allow(dead_code)]
mod calendar;

fn main() {
    println!("run `cargo test`");
}

/// How many days `month` of `year` has.
#[cfg(any(test, kani))]
fn days_in(year: u16, month: u8) -> u8 {
    let leap = year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    match month {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

#[cfg(test)]
mod tests {
    use super::calendar::*;
    use super::days_in;

    fn date_time(year: u16, month: u8, day: u8, (hour, minute, second): (u8, u8, u8)) -> DateTime {
        DateTime {
            year,
            month,
            day,
            hour,
            minute,
            second,
        }
    }

    /// Times looked up elsewhere (`date -u -d @N`).
    #[test]
    fn known_times() {
        let known = [
            (0, date_time(1970, 1, 1, (0, 0, 0))),
            (-1, date_time(1969, 12, 31, (23, 59, 59))),
            (951_782_400, date_time(2000, 2, 29, (0, 0, 0))),
            // The first and last second the phone's time is believed for
            // (`gb`'s `SANE_UTC_SECONDS`).
            (1_577_836_800, date_time(2020, 1, 1, (0, 0, 0))),
            (4_102_444_799, date_time(2099, 12, 31, (23, 59, 59))),
            (1_762_794_645, date_time(2025, 11, 10, (17, 10, 45))),
            // 2100 is no leap year.
            (4_107_542_400, date_time(2100, 3, 1, (0, 0, 0))),
        ];
        for (seconds, expected) in known {
            assert_eq!(DateTime::from_epoch_seconds(seconds), expected, "{seconds}");
            assert_eq!(expected.days_since_epoch(), seconds.div_euclid(86_400));
        }
    }

    /// Day by day from 1969 to 2200 (every date the clock can show, and
    /// then some): each day is the day after the one before, by the
    /// calendar's rules, and gives its own number back.
    #[test]
    fn every_day_follows_the_one_before() {
        let first = date_time(1969, 1, 1, (0, 0, 0)).days_since_epoch();
        assert_eq!(first, -365);
        let (mut year, mut month, mut day) = (1969, 1, 1);
        for days in first..=84_000 {
            // At any time of day.
            let second_of_day = (days * 7919).rem_euclid(86_400);
            let date = DateTime::from_epoch_seconds(days * 86_400 + second_of_day);
            assert_eq!((date.year, date.month, date.day), (year, month, day));
            assert_eq!(date.days_since_epoch(), days);
            let seconds = (date.hour, date.minute, date.second);
            let (hour, minute, second) = seconds;
            assert!(hour < 24 && minute < 60 && second < 60, "{seconds:?}");
            let of_day = i64::from(hour) * 3600 + i64::from(minute) * 60 + i64::from(second);
            assert_eq!(of_day, second_of_day);

            day += 1;
            if day > days_in(year, month) {
                (month, day) = (month + 1, 1);
            }
            if month > 12 {
                (year, month) = (year + 1, 1);
            }
        }
        assert_eq!(year, 2199);
    }
}

#[cfg(kani)]
mod proofs {
    use super::calendar::*;
    use super::days_in;

    /// The seconds the proofs cover, as local time: from 1969 (a day
    /// before 1970 is the earliest a clock set to UTC's first day reads,
    /// west of it) to the end of 2199. The phone's time is believed from
    /// 2020 to 2100 (`gb`'s `SANE_UTC_SECONDS`), give or take a time
    /// zone's 18 hours; `host-tests/alarm`'s proofs go to 2200.
    const SECONDS: core::ops::Range<i64> = -365 * 86_400..7_258_118_400;

    /// Any second the clock can show is a real date and time of day: a
    /// month from 1 to 12, a day that month has (in that year: no 29
    /// February but in a leap year), an hour under 24, minutes and
    /// seconds under 60, in a year from 1969 to 2199. Nothing overflows
    /// or is cut short on the way (the year's and the fields' casts).
    ///
    /// What draws the time relies on it: the clock face and the screens
    /// index digits, month and weekday names by these fields (F1 was a
    /// digit out of range), and `geo` and the UI do arithmetic on them
    /// in `u8` and `u16`.
    #[kani::proof]
    fn any_second_is_a_real_date_and_time() {
        let seconds: i64 = kani::any();
        kani::assume(SECONDS.contains(&seconds));
        let date = DateTime::from_epoch_seconds(seconds);

        assert!((1969..2200).contains(&date.year));
        assert!((1..=12).contains(&date.month));
        assert!(1 <= date.day && date.day <= days_in(date.year, date.month));
        assert!(date.hour < 24 && date.minute < 60 && date.second < 60);
    }

    /// A date and time, taken apart from any second the clock can show,
    /// put together again gives that very second: so no two seconds show
    /// as the same date and time, and none is shown as another's (the
    /// two halves of Hinnant's algorithm are each other's inverse here).
    /// `geo`'s sunrise and the simulator count days with
    /// `days_since_epoch`.
    #[kani::proof]
    fn a_date_and_time_is_its_second_and_no_other() {
        let seconds: i64 = kani::any();
        kani::assume(SECONDS.contains(&seconds));
        let date = DateTime::from_epoch_seconds(seconds);

        let of_day =
            i64::from(date.hour) * 3600 + i64::from(date.minute) * 60 + i64::from(date.second);
        assert_eq!(date.days_since_epoch() * 86_400 + of_day, seconds);
    }

    /// The day after any day the clock can show is the next date in the
    /// calendar: the next day of the month, or the first of the next
    /// month (of the next year, after December) once the month's days
    /// are up. With the first day right (`tests::known_times`: day 0 is
    /// 1970-01-01), every day's date is, by induction.
    #[kani::proof]
    fn the_next_day_is_the_next_date() {
        let seconds: i64 = kani::any();
        kani::assume(SECONDS.contains(&seconds) && SECONDS.contains(&(seconds + 86_400)));
        let today = DateTime::from_epoch_seconds(seconds);
        let tomorrow = DateTime::from_epoch_seconds(seconds + 86_400);

        let last_of_month = today.day == days_in(today.year, today.month);
        let expected = match (last_of_month, today.month) {
            (false, _) => (today.year, today.month, today.day + 1),
            (true, 12) => (today.year + 1, 1, 1),
            (true, _) => (today.year, today.month + 1, 1),
        };
        assert_eq!((tomorrow.year, tomorrow.month, tomorrow.day), expected);
    }
}
