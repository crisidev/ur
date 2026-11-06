//! Where the watch is, and when the sun is up there.
//!
//! The location comes from the phone's GPS via Gadgetbridge, but only rarely
//! (see [`GpsPolicy`]), and is saved in flash. Until the first fix, it's
//! home: Oslo. Sunrise and sunset are computed on the watch from the location
//! and date, and the theme switches between dark and light with them.

use core::cell::Cell;

use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_time::{Duration, Instant};
use libm::{acos, asin, ceil, cos, fmod, sin};

use crate::clock::{self, DateTime};

// ==============================================================================
// Location
// ==============================================================================

/// A position, in millionths of a degree (about 10 cm; far finer than
/// anything here needs, but compact and exact to store).
#[derive(Clone, Copy, PartialEq, Eq, defmt::Format)]
pub struct Location {
    pub latitude_micro: i32,
    /// East positive.
    pub longitude_micro: i32,
}

/// Until the phone has ever sent a fix.
pub const HOME: Location = Location {
    latitude_micro: 59_913_900, // Oslo
    longitude_micro: 10_752_200,
};

impl Location {
    pub fn from_degrees(latitude: f64, longitude: f64) -> Option<Self> {
        let valid = (-90.0..=90.0).contains(&latitude) && (-180.0..=180.0).contains(&longitude);
        // A phone with no fix sometimes reports exactly 0,0.
        let null_island = latitude == 0.0 && longitude == 0.0;
        (valid && !null_island).then_some(Self {
            latitude_micro: (latitude * 1e6) as i32,
            longitude_micro: (longitude * 1e6) as i32,
        })
    }

    fn degrees(self) -> (f64, f64) {
        (
            f64::from(self.latitude_micro) / 1e6,
            f64::from(self.longitude_micro) / 1e6,
        )
    }

    pub fn to_bytes(self) -> [u8; 8] {
        let mut out = [0; 8];
        out[..4].copy_from_slice(&self.latitude_micro.to_le_bytes());
        out[4..].copy_from_slice(&self.longitude_micro.to_le_bytes());
        out
    }

    pub fn from_bytes(bytes: [u8; 8]) -> Self {
        let [a, b, c, d, e, f, g, h] = bytes;
        Self {
            latitude_micro: i32::from_le_bytes([a, b, c, d]),
            longitude_micro: i32::from_le_bytes([e, f, g, h]),
        }
    }
}

static LOCATION: Mutex<CriticalSectionRawMutex, Cell<Option<Location>>> =
    Mutex::new(Cell::new(None));

pub fn set_location(location: Location) {
    LOCATION.lock(|cell| cell.set(Some(location)));
}

pub fn location() -> Location {
    LOCATION.lock(Cell::get).unwrap_or(HOME)
}

/// Whether we've ever had a real fix (rather than the home default).
pub fn has_fix() -> bool {
    LOCATION.lock(Cell::get).is_some()
}

// ==============================================================================
// When to ask the phone for a fix
// ==============================================================================
//
// Every fix costs the phone a few seconds of GPS, and the location only
// matters at city scale (for sunrise), so we ask rarely. The interesting
// moments are travel, which the watch can't see directly. It can't see the
// phone's cell signal either (the protocol has no such message), and phones
// often keep Bluetooth on in flight. But it can see:
//
// - the UTC offset changing: phones switch time zone on landing, and
//   Gadgetbridge then re-sends the time with the new offset;
// - a long Bluetooth gap: the phone was off or far away.
//
// Plus a daily refresh, for same-time-zone trips.

const MAX_FIX_AGE: Duration = Duration::from_secs(24 * 3600);
const LONG_ABSENCE: Duration = Duration::from_secs(2 * 3600);
/// Stop asking if no fix arrives within this long (no GPS reception, or
/// GPS forwarding is off in Gadgetbridge); try again at the next trigger.
pub const FIX_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Default)]
pub struct GpsPolicy {
    last_fix: Option<Instant>,
    last_offset: Option<i16>,
    disconnected_at: Option<Instant>,
    /// A trigger fired; ask at the next opportunity.
    wanted: bool,
}

impl GpsPolicy {
    pub fn on_disconnect(&mut self, now: Instant) {
        self.disconnected_at = Some(now);
    }

    pub fn on_connect(&mut self, now: Instant) {
        let long_absence = self
            .disconnected_at
            .is_some_and(|at| now - at >= LONG_ABSENCE);
        let stale = self.last_fix.is_none_or(|at| now - at >= MAX_FIX_AGE);
        self.wanted |= long_absence || stale;
    }

    pub fn on_time_sync(&mut self, offset_minutes: i16) {
        if self.last_offset.is_some_and(|last| last != offset_minutes) {
            defmt::info!("geo: UTC offset changed; will ask for a location");
            self.wanted = true;
        }
        self.last_offset = Some(offset_minutes);
    }

    pub fn on_fix(&mut self, now: Instant) {
        self.last_fix = Some(now);
        self.wanted = false;
    }

    /// Whether to ask the phone for GPS now. Clears the request: if the fix
    /// doesn't come, we wait for the next trigger rather than nagging.
    pub fn take_request(&mut self) -> bool {
        core::mem::take(&mut self.wanted)
    }
}

// ==============================================================================
// Sunrise and sunset
// ==============================================================================

/// Sunrise and sunset, in minutes after local midnight; `sunrise == sunset`
/// is polar night, and `0..1440` polar day.
#[derive(Clone, Copy, PartialEq, Eq, defmt::Format)]
pub struct Sun {
    pub sunrise: u16,
    pub sunset: u16,
}

impl Sun {
    fn contains(&self, minute_of_day: u16) -> bool {
        (self.sunrise..self.sunset).contains(&minute_of_day)
    }
}

/// Today's sun at the current location, or `None` without wall-clock time.
pub fn sun() -> Option<Sun> {
    let now = clock::now()?;
    let offset = clock::utc_offset_minutes()?;
    Some(compute_sun(&now, location(), offset))
}

/// Whether it's between sunrise and sunset. Without wall-clock time, it's
/// treated as night: the dark theme is the safe default when the screen
/// might light up in a dark room.
pub fn is_daytime() -> bool {
    let Some(now) = clock::now() else {
        return false;
    };
    let Some(sun) = sun() else { return false };
    sun.contains(u16::from(now.hour) * 60 + u16::from(now.minute))
}

/// The "sunrise equation" (Wikipedia's complete version), accurate to a
/// minute or two at mid latitudes: checked against published Oslo times for
/// the solstices (03:53/22:43, 09:18/15:12).
///
/// TODO: cache per (date, location, offset) if this ever shows up in
/// profiles; it's a few dozen soft-float operations, once per frame.
fn compute_sun(today: &DateTime, location: Location, offset_minutes: i16) -> Sun {
    let (latitude, longitude) = location.degrees();
    let (sin_deg, cos_deg) = (|d: f64| sin(d.to_radians()), |d: f64| cos(d.to_radians()));

    // Days since the J2000 epoch, at local solar noon of today's date.
    let days_since_1970 = today.days_since_epoch() as f64;
    let julian_noon = days_since_1970 + 2_440_588.0;
    let n = ceil(julian_noon - 2_451_545.0 + 0.0008);
    let mean_solar_time = n - longitude / 360.0;

    // Where the sun is along its (elliptical) path, and the solar transit
    // (local noon) as a Julian date.
    let anomaly = wrap(357.5291 + 0.985_600_28 * mean_solar_time, 360.0);
    let center =
        1.9148 * sin_deg(anomaly) + 0.02 * sin_deg(2.0 * anomaly) + 0.0003 * sin_deg(3.0 * anomaly);
    let ecliptic_longitude = wrap(anomaly + center + 180.0 + 102.9372, 360.0);
    let transit = 2_451_545.0 + mean_solar_time + 0.0053 * sin_deg(anomaly)
        - 0.0069 * sin_deg(2.0 * ecliptic_longitude);

    // The hour angle at which the sun's upper edge crosses the horizon,
    // including atmospheric refraction (the −0.833°).
    let sin_declination = sin_deg(ecliptic_longitude) * sin_deg(23.4397);
    let cos_declination = cos(asin(sin_declination));
    let cos_hour_angle = (sin_deg(-0.833) - sin_deg(latitude) * sin_declination)
        / (cos_deg(latitude) * cos_declination);
    if cos_hour_angle > 1.0 {
        return Sun {
            sunrise: 0,
            sunset: 0,
        };
    }
    if cos_hour_angle < -1.0 {
        return Sun {
            sunrise: 0,
            sunset: 24 * 60,
        };
    }
    let hour_angle = acos(cos_hour_angle).to_degrees();

    let local_minute = |julian: f64| {
        let unix_seconds = (julian - 2_440_587.5) * 86_400.0 + f64::from(offset_minutes) * 60.0;
        (wrap(unix_seconds, 86_400.0) / 60.0) as u16
    };
    Sun {
        sunrise: local_minute(transit - hour_angle / 360.0),
        sunset: local_minute(transit + hour_angle / 360.0),
    }
}

/// `x` modulo `m`, in `0..m` (`f64::rem_euclid` needs `std`).
fn wrap(x: f64, m: f64) -> f64 {
    let r = fmod(x, m);
    if r < 0.0 { r + m } else { r }
}
