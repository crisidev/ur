//! Calendar arithmetic: which date and time of day a count of seconds is,
//! and back. [`crate::clock`] keeps the time as such a count, and turns it
//! into a [`DateTime`] to show.
//!
//! Hardware-free, and apart from the clock (which needs the uptime), so
//! that it runs in host tests (`host-tests/calendar`).

/// A calendar date and time (local, wherever it's used for display).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[cfg_attr(target_os = "none", derive(defmt::Format))]
pub struct DateTime {
    pub year: u16,
    /// 1–12.
    pub month: u8,
    /// 1–31.
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

impl DateTime {
    /// Days since 1970-01-01 of this date.
    pub fn days_since_epoch(&self) -> i64 {
        days_from_civil(self.year.into(), self.month.into(), self.day.into())
    }

    /// The date and time `seconds` after 1970-01-01T00:00.
    pub fn from_epoch_seconds(seconds: i64) -> Self {
        let (days, second_of_day) = (seconds.div_euclid(86_400), seconds.rem_euclid(86_400));
        let (year, month, day) = civil_from_days(days);
        Self {
            year: year as u16,
            month: month as u8,
            day: day as u8,
            hour: (second_of_day / 3600) as u8,
            minute: (second_of_day / 60 % 60) as u8,
            second: (second_of_day % 60) as u8,
        }
    }
}

// ==============================================================================
// Days and dates
// ==============================================================================
//
// Howard Hinnant's days-from-civil algorithms
// (https://howardhinnant.github.io/date_algorithms.html): exact for the
// proleptic Gregorian calendar, no tables, no loops. Day 0 is 1970-01-01.

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    // Treat March as the first month, so the leap day ends the year.
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month_from_march = (month + 9) % 12;
    let day_of_year = (153 * month_from_march + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let days = days + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_from_march = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_from_march + 2) / 5 + 1;
    let month = if month_from_march < 10 {
        month_from_march + 3
    } else {
        month_from_march - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}
