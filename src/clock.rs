//! Wall-clock time.
//!
//! The watch has no battery-backed RTC, so after every boot it knows only
//! time-since-boot until the phone tells it the time (over Bluetooth's
//! Current Time Service, see [`crate::ble`]). We then remember the pair
//! "at uptime X, local time was Y" and derive the time from the RTC-driven
//! uptime from then on. Between syncs, accuracy is that of the 32.768 kHz
//! crystal (±20 ppm, i.e. under 2 s/day).
//!
//! Everything here is *local* time, as sent by the phone; nothing on the
//! watch needs UTC yet.
//!
//! TODO: keep the UTC offset (Local Time Information) once something needs
//! it, e.g. to handle DST changes between syncs.

use core::cell::Cell;

use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_time::Instant;

/// A local calendar date and time.
#[derive(Clone, Copy, PartialEq, Eq, defmt::Format)]
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
    /// 0 = Monday … 6 = Sunday.
    pub fn weekday(&self) -> u8 {
        // 1970-01-01 was a Thursday (3, with Monday = 0).
        (days_from_civil(self.year.into(), self.month.into(), self.day.into()) + 3).rem_euclid(7)
            as u8
    }

    pub fn weekday_name(&self) -> &'static str {
        ["MON", "TUE", "WED", "THU", "FRI", "SAT", "SUN"][usize::from(self.weekday())]
    }

    fn to_epoch_seconds(self) -> i64 {
        let days = days_from_civil(self.year.into(), self.month.into(), self.day.into());
        days * 86_400
            + i64::from(self.hour) * 3600
            + i64::from(self.minute) * 60
            + i64::from(self.second)
    }

    fn from_epoch_seconds(seconds: i64) -> Self {
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

/// When the clock was last set: the uptime then, and the local time (as
/// seconds since 1970-01-01T00:00 local) at that uptime.
static SYNC: Mutex<CriticalSectionRawMutex, Cell<Option<(Instant, i64)>>> =
    Mutex::new(Cell::new(None));

pub fn set(now: DateTime) {
    SYNC.lock(|sync| sync.set(Some((Instant::now(), now.to_epoch_seconds()))));
}

/// The current local time, if the phone has told us yet.
pub fn now() -> Option<DateTime> {
    let (synced_at, epoch_seconds) = SYNC.lock(Cell::get)?;
    let elapsed = (Instant::now() - synced_at).as_secs() as i64;
    Some(DateTime::from_epoch_seconds(epoch_seconds + elapsed))
}

// ==============================================================================
// Calendar arithmetic
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
