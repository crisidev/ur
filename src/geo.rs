//! Where the watch is, and when the sun is up there.
//!
//! The location comes from the phone's GPS via Gadgetbridge, but only rarely
//! (see [`GpsPolicy`]), and is saved in flash. Until the first fix, it's
//! home: Oslo. Sunrise and sunset are computed on the watch from the location
//! and date, and the theme switches between dark and light with them.

use core::cell::Cell;
use core::sync::atomic::{AtomicU8, Ordering};

use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_time::{Duration, Instant};
use libm::{acosf, sqrtf};

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
    /// From millionths of a degree, if in range.
    pub fn from_micro(latitude: i64, longitude: i64) -> Option<Self> {
        let valid = (-90_000_000..=90_000_000).contains(&latitude)
            && (-180_000_000..=180_000_000).contains(&longitude);
        // A phone with no fix sometimes reports exactly 0,0.
        let null_island = latitude == 0 && longitude == 0;
        (valid && !null_island).then_some(Self {
            latitude_micro: latitude as i32,
            longitude_micro: longitude as i32,
        })
    }

    fn degrees(self) -> (f32, f32) {
        (
            self.latitude_micro as f32 / 1e6,
            self.longitude_micro as f32 / 1e6,
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

/// How many precise fixes have come from the phone, wrapping. So the UI
/// loop notices a new one (see `aranet::Nearby::located`) by a byte,
/// rather than keeping a copy of the last.
static SET: AtomicU8 = AtomicU8::new(0);

/// `precise`: a fix from the phone good enough to tell whether the wearer
/// is near the room sensors (`aranet::precise`), which counts in
/// [`times_set`]. A coarse one (from cell towers) does for the sunrise.
pub fn set_location(location: Location, precise: bool) {
    LOCATION.lock(|cell| cell.set(Some(location)));
    if precise {
        SET.fetch_add(1, Ordering::Relaxed);
    }
}

/// See [`SET`].
pub fn times_set() -> u8 {
    SET.load(Ordering::Relaxed)
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
// Plus a daily refresh, for same-time-zone trips, and now and then for
// the room sensors (see [`GpsPolicy::for_sensors`]).
//
// What a request does on the phone (Gadgetbridge's Bangle.js driver,
// `setupGPSUpdateTimer`): with "Use phone GPS data" on, it starts
// Android's GPS and network location providers, and forwards every fix
// until we say stop. It never sends the last known location, so with no
// fix (indoors, with GPS alone) nothing comes. And each fix is dropped
// unless Gadgetbridge's own "Keep location updated" setting is on, and
// Android only delivers fixes to it in the background with location
// allowed "all the time". README FAQ C03 has the steps.
//
// So every request ends with `gps_power` off: at the first fix precise
// enough for the room sensors, after [`FIX_TIMEOUT`] without one, or
// (Gadgetbridge's own doing) when the watch disconnects. A coarse fix
// (`hdop`, which is Android's accuracy in metres, despite the name, over
// `aranet::PRECISE`) doesn't end it: the GPS, or Wi-Fi, may yet do
// better within the timeout.

const MAX_FIX_AGE: Duration = Duration::from_secs(24 * 3600);
const LONG_ABSENCE: Duration = Duration::from_secs(2 * 3600);
/// Stop asking if no fix arrives within this long (no GPS reception, or
/// GPS forwarding is off in Gadgetbridge); try again at the next trigger.
pub const FIX_TIMEOUT: Duration = Duration::from_secs(120);
/// After asking, don't ask again on reconnecting for this long. Without
/// it, a phone that never sends a fix (see above) was asked at every
/// reconnect, each time running its GPS for [`FIX_TIMEOUT`] and showing
/// Gadgetbridge's "GPS" notification; flaky links reconnect every few
/// minutes. A changed time zone still asks at once.
const RETRY_AFTER: Duration = Duration::from_secs(3600);
/// The most requests a day for the room sensors (see
/// [`GpsPolicy::for_sensors`]), on top of the others.
const SENSOR_REQUESTS: u8 = 4;

#[derive(Default)]
pub struct GpsPolicy {
    last_fix: Option<Instant>,
    last_offset: Option<i16>,
    disconnected_at: Option<Instant>,
    /// When we last asked (see [`RETRY_AFTER`]).
    last_request: Option<Instant>,
    /// A trigger fired; ask at the next opportunity.
    wanted: bool,
    /// Requests for the room sensors on `last_request`'s day (since
    /// booting), for [`SENSOR_REQUESTS`]. (In what would be padding.)
    sensor_requests: u8,
}

/// Days since booting: [`SENSOR_REQUESTS`]'s day.
fn day(at: Instant) -> u64 {
    at.as_secs() / (24 * 3600)
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
        let asked_lately = self.last_request.is_some_and(|at| now - at < RETRY_AFTER);
        self.wanted |= (long_absence || stale) && !asked_lately;
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
    pub fn take_request(&mut self, now: Instant) -> bool {
        let wanted = core::mem::take(&mut self.wanted);
        if wanted {
            self.asked(now);
        }
        wanted
    }

    /// A scan for room sensors would like a fix (`aranet::Ask`): whether
    /// to ask the phone now. Not if anything asked within [`RETRY_AFTER`],
    /// or this asked [`SENSOR_REQUESTS`] times today already.
    ///
    /// What that comes to: with sensors at home, a request each time the
    /// scans stop hearing them, i.e. leaving home (or a room far enough
    /// from them, see `aranet::FLOOR_DBM`), so typically 1–3 a day, at
    /// most four, an hour apart; and one after booting, to learn where
    /// they are. Each runs the phone's location for a few seconds (a
    /// Wi-Fi fix comes at once), at worst [`FIX_TIMEOUT`].
    pub fn for_sensors(&mut self, now: Instant) -> bool {
        let today = self.last_request.is_some_and(|at| day(at) == day(now));
        let asked_lately = self
            .last_request
            .is_some_and(|at| now.saturating_duration_since(at) < RETRY_AFTER);
        if asked_lately || (today && self.sensor_requests >= SENSOR_REQUESTS) {
            return false;
        }
        self.asked(now);
        self.sensor_requests += 1;
        true
    }

    /// A request goes out now.
    fn asked(&mut self, now: Instant) {
        if self.last_request.is_none_or(|at| day(at) != day(now)) {
            self.sensor_requests = 0;
        }
        self.last_request = Some(now);
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
/// All in `f32` and integers: the M4F's FPU does single precision only,
/// and `f64`'s soft-float routines (with `libm`'s `sinf` and `cosf`, which
/// compute in `f64` too) cost ~8 KB of flash. A Julian date's 2.4 million
/// days would leave an `f32` a quarter-day resolution, so the whole days
/// stay integers: the mean anomaly's daily turn is taken modulo 360° in
/// integer hundred-millionths of a degree (exact), and they cancel out of
/// the time of day. What's left in `f32` is under a day, or under a few
/// turns of angle: the times come out within ~0.02 s of the `f64`
/// equation's (which itself is off by a minute or two).
///
/// TODO: cache per (date, location, offset) if this ever shows up in
/// profiles; it's a few dozen float operations, once per frame.
fn compute_sun(today: &DateTime, location: Location, offset_minutes: i16) -> Sun {
    let (latitude, longitude) = location.degrees();

    // Days since the J2000 epoch, at local solar noon of today's date:
    // the equation's `ceil(julian_noon - 2_451_545 + 0.0008)`, where
    // `julian_noon` is the date's whole Julian day number,
    // `days_since_epoch + 2_440_588`. Then the mean solar time, `n` days
    // plus the part of one that the longitude takes.
    let n = today.days_since_epoch() - 10_956;
    let part = -longitude / 360.0;

    // Where the sun is along its (elliptical) path: the mean anomaly,
    // 357.5291° + 0.98560028° a day, the whole days' share exactly (in
    // 1e-8 degrees, `n` would have to pass 9e10 to overflow).
    let whole_days = (35_752_910_000 + 98_560_028 * n).rem_euclid(36_000_000_000) as f32 / 1e8;
    // (0.98560028 to an `f32`: the part is under a day, so that rounding
    // is under 1e-8°.)
    let anomaly = wrap(whole_days + 0.985_600_3 * part, 360.0);
    let center =
        1.9148 * sin_deg(anomaly) + 0.02 * sin_deg(2.0 * anomaly) + 0.0003 * sin_deg(3.0 * anomaly);
    let ecliptic_longitude = wrap(anomaly + center + 180.0 + 102.9372, 360.0);
    // The solar transit (local noon), in days after the `n`th's J2000 noon.
    let transit = part + 0.0053 * sin_deg(anomaly) - 0.0069 * sin_deg(2.0 * ecliptic_longitude);

    // The hour angle at which the sun's upper edge crosses the horizon,
    // including atmospheric refraction (the −0.833°).
    let sin_declination = sin_deg(ecliptic_longitude) * sin_deg(23.4397);
    // cos(asin(x)); the declination is within ±23.44°, so its cosine is
    // positive.
    let cos_declination = sqrtf(1.0 - sin_declination * sin_declination);
    let cos_hour_angle = (sin_deg(-0.833) - sin_deg(latitude) * sin_declination)
        / (sin_deg(90.0 - latitude) * cos_declination);
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
    let hour_angle = acosf(cos_hour_angle).to_degrees();

    // A Julian date's days start at noon, and J2000's whole days are
    // whole days after the Unix epoch's (2_451_545 - 2_440_587.5 =
    // 10_957.5), so only the half day and the parts are left of the time
    // of day.
    let local_minute = |days: f32| {
        let seconds = (0.5 + days) * 86_400.0 + f32::from(offset_minutes) * 60.0;
        (wrap(seconds, 86_400.0) / 60.0) as u16
    };
    Sun {
        sunrise: local_minute(transit - hour_angle / 360.0),
        sunset: local_minute(transit + hour_angle / 360.0),
    }
}

/// `x` modulo `m`, in `0..m`, for `x` within a few `m` of it (`fmodf`'s
/// generality costs flash).
fn wrap(mut x: f32, m: f32) -> f32 {
    while x < 0.0 {
        x += m;
    }
    while x >= m {
        x -= m;
    }
    x
}

/// The sine of an angle in degrees, within a few turns of zero, to an
/// `f32`'s precision; [`compute_sun`] says why not `libm`'s.
fn sin_deg(degrees: f32) -> f32 {
    // Into −180°..180°, then −90°..90° by sin(180° − x) = sin(x). Every
    // step is exact: each subtracts floats within a factor of two of each
    // other (Sterbenz's lemma), so an angle of a few turns loses nothing.
    let turns = (degrees / 360.0) as i32 as f32;
    let mut x = degrees - 360.0 * turns;
    if x > 180.0 {
        x -= 360.0;
    } else if x < -180.0 {
        x += 360.0;
    }
    if x > 90.0 {
        x = 180.0 - x;
    } else if x < -90.0 {
        x = -180.0 - x;
    }
    // Taylor's series to x¹¹, whose remainder within ±π/2 is under 6e-8.
    let r = x.to_radians();
    let r2 = r * r;
    // r (1 − r²/(2·3) (1 − r²/(4·5) (… (1 − r²/(10·11))))), inside out.
    let mut factor = 1.0;
    for k in [11.0f32, 9.0, 7.0, 5.0, 3.0] {
        factor = 1.0 - r2 / (k * (k - 1.0)) * factor;
    }
    r * factor
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Published Oslo times for the 2025 solstices (in summer time, then
    /// winter time), which the `f64` equation this replaced also gave.
    #[test]
    fn oslo_at_the_solstices() {
        let date = |month, day| DateTime {
            year: 2025,
            month,
            day,
            hour: 12,
            minute: 0,
            second: 0,
        };
        let summer = compute_sun(&date(6, 21), HOME, 120);
        assert_eq!((summer.sunrise, summer.sunset), (3 * 60 + 53, 22 * 60 + 43));
        let winter = compute_sun(&date(12, 21), HOME, 60);
        assert_eq!((winter.sunrise, winter.sunset), (9 * 60 + 18, 15 * 60 + 12));
    }

    /// To a few `f32` steps, for every angle the equation takes.
    #[test]
    fn sine_in_degrees() {
        for tenth in -10_800..=10_800 {
            let degrees = tenth as f32 / 10.0;
            let expected = (f64::from(degrees)).to_radians().sin();
            assert!(
                (f64::from(sin_deg(degrees)) - expected).abs() < 3e-7,
                "{degrees}"
            );
        }
    }
}
