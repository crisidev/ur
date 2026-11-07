//! Wall-clock time.
//!
//! The watch has no battery-backed RTC, so after every boot it knows only
//! time-since-boot until the phone tells it the time (Gadgetbridge sends UTC
//! and the UTC offset on every connect and when the phone's time zone
//! changes; see [`crate::gb`]). We then remember "at uptime X, it was UTC
//! time Y, in offset Z" and derive the time from the RTC-driven uptime from
//! then on. Between syncs, accuracy is that of the 32.768 kHz crystal
//! (±20 ppm, i.e. under 2 s/day).
//!
//! The time also survives resets that keep power on (reflashing, watchdog,
//! panics): [`checkpoint`] saves it once a second in RAM that startup doesn't
//! clear, and [`restore`] picks it up at boot, off by at most about a second
//! (plus however long a debugger halted the core) until the phone next syncs.
//! A drained battery still loses it.
//!
//! TODO: daylight-saving changes between syncs keep the old offset until the
//! phone next connects (which it normally does within minutes).

use core::cell::Cell;

use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_time::Instant;

/// A calendar date and time (local, wherever it's used for display).
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
        (self.days_since_epoch() + 3).rem_euclid(7) as u8
    }

    pub fn weekday_name(&self) -> &'static str {
        ["MON", "TUE", "WED", "THU", "FRI", "SAT", "SUN"][usize::from(self.weekday())]
    }

    /// Days since 1970-01-01 of this date.
    pub fn days_since_epoch(&self) -> i64 {
        days_from_civil(self.year.into(), self.month.into(), self.day.into())
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

#[derive(Clone, Copy)]
struct Sync {
    /// Uptime when the time was set.
    at: Instant,
    /// Seconds since 1970-01-01T00:00Z at `at`.
    utc_seconds: i64,
    offset_minutes: i16,
}

static SYNC: Mutex<CriticalSectionRawMutex, Cell<Option<Sync>>> = Mutex::new(Cell::new(None));

pub fn set(utc_seconds: i64, offset_minutes: i16) {
    // How far off we were shows crystal drift between syncs, and how much a
    // reset (e.g. a reflash, which halts the core) set us back.
    if let Some(before) = now_utc_seconds() {
        defmt::info!("clock: was off by {} s", before - utc_seconds);
    }
    let sync = Sync {
        at: Instant::now(),
        utc_seconds,
        offset_minutes,
    };
    SYNC.lock(|cell| cell.set(Some(sync)));
}

/// Seconds since 1970-01-01T00:00Z, if the phone has told us the time.
pub fn now_utc_seconds() -> Option<i64> {
    let sync = SYNC.lock(Cell::get)?;
    Some(sync.utc_seconds + (Instant::now() - sync.at).as_secs() as i64)
}

/// The current local time, if the phone has told us yet.
pub fn now() -> Option<DateTime> {
    let offset = utc_offset_minutes()?;
    Some(DateTime::from_epoch_seconds(
        now_utc_seconds()? + i64::from(offset) * 60,
    ))
}

/// The phone's UTC offset at the last sync.
pub fn utc_offset_minutes() -> Option<i16> {
    SYNC.lock(Cell::get).map(|sync| sync.offset_minutes)
}

// ==============================================================================
// Surviving resets
// ==============================================================================

/// The last known time, in the second half of the RAM block `memory.x` keeps
/// out of the linker's hands. Nothing initialises it at startup, so it
/// survives any reset that keeps power on, and its fixed address means even a
/// different firmware build (after reflashing) finds it.
#[repr(C)]
struct Retained {
    magic: u32,
    offset_minutes: i16,
    utc_seconds: i64,
    /// `utc_seconds` and `offset_minutes` mixed and inverted, so random
    /// power-on RAM contents can't pass for a valid time.
    check: i64,
}

impl Retained {
    fn check(utc_seconds: i64, offset_minutes: i16) -> i64 {
        !(utc_seconds ^ (i64::from(offset_minutes) << 48))
    }
}

/// Changed whenever `Retained`'s layout does, so a new build never misreads
/// an old build's checkpoint.
const RETAINED_MAGIC: u32 = 0x7572_4332; // "urC2"

/// Must match the `RETAINED` region in `memory.x`.
const RETAINED_ADDRESS: usize = 0x2000_E020;
const _: () = assert!(size_of::<Retained>() <= 32);

fn retained() -> *mut Retained {
    RETAINED_ADDRESS as *mut Retained
}

/// Saves the current time for [`restore`]. Call about once a second; the
/// interval bounds how far behind a restored clock can be.
pub fn checkpoint() {
    let Some(sync) = SYNC.lock(Cell::get) else {
        return;
    };
    let utc_seconds = now_utc_seconds().expect("synced, checked above");
    let value = Retained {
        magic: RETAINED_MAGIC,
        offset_minutes: sync.offset_minutes,
        utc_seconds,
        check: Retained::check(utc_seconds, sync.offset_minutes),
    };
    // SAFETY: The address is reserved RAM (see `memory.x`), suitably
    // aligned, and only ever accessed from the UI loop (checkpoint) and once
    // at boot before any task runs (restore); never concurrently. Volatile so
    // the write isn't optimised away as dead.
    unsafe { retained().write_volatile(value) };
}

/// Resumes the clock from the last [`checkpoint`] before a reset, if there
/// was one. Call once, early at boot.
pub fn restore() {
    // SAFETY: See `checkpoint`. Any bit pattern is a valid `Retained`
    // (plain integers), so reading uninitialised-looking RAM is fine; the
    // magic and check fields tell us whether it's real.
    let saved = unsafe { retained().read_volatile() };
    if saved.magic != RETAINED_MAGIC
        || saved.check != Retained::check(saved.utc_seconds, saved.offset_minutes)
    {
        return;
    }
    // The checkpoint was up to a second before the reset, and booting takes
    // a fraction of a second more; splitting the difference keeps the error
    // within about half a second either way.
    set(saved.utc_seconds + 1, saved.offset_minutes);
    defmt::info!("clock: resumed after reset");
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
