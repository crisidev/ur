//! Wall-clock time.
//!
//! The watch has no battery-backed RTC, so after every boot it knows only
//! time-since-boot until the phone tells it the time (over Bluetooth's
//! Current Time Service, see [`crate::ble`]). We then remember the pair
//! "at uptime X, local time was Y" and derive the time from the RTC-driven
//! uptime from then on. Between syncs, accuracy is that of the 32.768 kHz
//! crystal (±20 ppm, i.e. under 2 s/day).
//!
//! The time also survives resets that keep power on (reflashing, watchdog,
//! panics): [`checkpoint`] saves it once a second in RAM that startup doesn't
//! clear, and [`restore`] picks it up at boot, off by at most about a second
//! until the phone next syncs. A drained battery still loses it.
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
    // How far off we were shows crystal drift between syncs, and how much a
    // reset (e.g. a reflash, which halts the core) set us back.
    if let Some(before) = self::now() {
        let error = before.to_epoch_seconds() - now.to_epoch_seconds();
        defmt::info!("clock: was off by {} s", error);
    }
    SYNC.lock(|sync| sync.set(Some((Instant::now(), now.to_epoch_seconds()))));
}

/// The current local time, if the phone has told us yet.
pub fn now() -> Option<DateTime> {
    let (synced_at, epoch_seconds) = SYNC.lock(Cell::get)?;
    let elapsed = (Instant::now() - synced_at).as_secs() as i64;
    Some(DateTime::from_epoch_seconds(epoch_seconds + elapsed))
}

// ==============================================================================
// Daylight
// ==============================================================================
//
// The theme follows the sun: dark themes from sunset to sunrise. The watch
// can't know where it is, but Gadgetbridge forwards weather from a weather
// app, and that includes today's sunrise and sunset in local time (see
// `ble`). They're also saved in flash; they drift only minutes per day, so
// old values are fine until the next weather update.

/// Assumed until the phone has ever sent weather.
const DEFAULT_SUN: Sun = Sun {
    sunrise: 7 * 60,
    sunset: 19 * 60,
};

/// Sunrise and sunset, in minutes after local midnight.
#[derive(Clone, Copy, PartialEq, Eq, defmt::Format)]
pub struct Sun {
    pub sunrise: u16,
    pub sunset: u16,
}

impl Sun {
    pub fn to_bytes(self) -> [u8; 4] {
        let [a, b] = self.sunrise.to_le_bytes();
        let [c, d] = self.sunset.to_le_bytes();
        [a, b, c, d]
    }

    /// `None` unless both times are within a day and sunrise comes first.
    /// (Polar day and night don't fit this model; we'd rather keep the
    /// previous values than flip themes on nonsense.)
    pub fn from_bytes([a, b, c, d]: [u8; 4]) -> Option<Self> {
        let sun = Self {
            sunrise: u16::from_le_bytes([a, b]),
            sunset: u16::from_le_bytes([c, d]),
        };
        (sun.sunrise < sun.sunset && sun.sunset < 24 * 60).then_some(sun)
    }
}

static SUN: Mutex<CriticalSectionRawMutex, Cell<Option<Sun>>> = Mutex::new(Cell::new(None));

pub fn set_sun(sun: Sun) {
    SUN.lock(|cell| cell.set(Some(sun)));
}

pub fn sun() -> Sun {
    SUN.lock(Cell::get).unwrap_or(DEFAULT_SUN)
}

/// Whether it's currently between sunrise and sunset. Without wall-clock
/// time, it's treated as night (the dark theme is the safe default when the
/// screen might be lit in a dark room).
pub fn is_daytime() -> bool {
    let Some(now) = now() else { return false };
    let minute = u16::from(now.hour) * 60 + u16::from(now.minute);
    let sun = sun();
    (sun.sunrise..sun.sunset).contains(&minute)
}

// ==============================================================================
// Surviving resets
// ==============================================================================

/// The last known local time, in the last 32 bytes of RAM, which `memory.x`
/// keeps out of the linker's hands. Nothing initialises it at startup, so it
/// survives any reset that keeps power on, and its fixed address means even a
/// different firmware build (after reflashing) finds it.
#[repr(C)]
struct Retained {
    magic: u32,
    epoch_seconds: i64,
    /// `epoch_seconds` inverted, so random power-on RAM contents can't pass
    /// for a valid time.
    check: i64,
}

const RETAINED_MAGIC: u32 = 0x7572_434B; // "urCK"

/// Must match the `RETAINED` region in `memory.x`.
const RETAINED_ADDRESS: usize = 0x2000_FFE0;
const _: () = assert!(size_of::<Retained>() <= 32);

fn retained() -> *mut Retained {
    RETAINED_ADDRESS as *mut Retained
}

/// Saves the current time for [`restore`]. Call about once a second; the
/// interval bounds how far behind a restored clock can be.
pub fn checkpoint() {
    let Some(now) = now() else { return };
    let epoch_seconds = now.to_epoch_seconds();
    let retained_value = Retained {
        magic: RETAINED_MAGIC,
        epoch_seconds,
        check: !epoch_seconds,
    };
    // SAFETY: The address is reserved RAM (see `memory.x`), suitably
    // aligned, and only ever accessed from the UI loop (checkpoint) and once
    // at boot before any task runs (restore); never concurrently. Volatile so
    // the write isn't optimised away as dead.
    unsafe { retained().write_volatile(retained_value) };
}

/// Resumes the clock from the last [`checkpoint`] before a reset, if there
/// was one. Call once, early at boot.
pub fn restore() {
    // SAFETY: See `checkpoint`. Any bit pattern is a valid `Retained`
    // (plain integers), so reading uninitialised-looking RAM is fine; the
    // magic and check fields tell us whether it's real.
    let retained = unsafe { retained().read_volatile() };
    if retained.magic != RETAINED_MAGIC || retained.check != !retained.epoch_seconds {
        return;
    }
    // The checkpoint was up to a second before the reset, and booting takes
    // a fraction of a second more; splitting the difference keeps the error
    // within about half a second either way.
    let time = DateTime::from_epoch_seconds(retained.epoch_seconds + 1);
    defmt::info!("clock: resumed at {} after reset", time);
    set(time);
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
