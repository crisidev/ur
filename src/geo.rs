//! Where the watch is, and when the sun is up there.
//!
//! The location comes from the phone's GPS via Gadgetbridge, but only rarely
//! (see [`GpsPolicy`]), and is saved in flash. Until the first fix, it's
//! home: Oslo. Sunrise and sunset are computed on the watch from the location
//! and date (in [`crate::sun`]), and the theme switches between dark and
//! light with them.

use core::cell::Cell;
use core::sync::atomic::{AtomicU8, Ordering};

use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_time::{Duration, Instant};

use crate::clock;
use crate::sun::{self, Sun};

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
        // `asked` started the count over if this is the day's first, and
        // a day's fifth was refused above: so it can't overflow either.
        debug_assert!(self.sensor_requests <= SENSOR_REQUESTS);
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

/// Today's sun at the current location, or `None` without wall-clock time.
pub fn sun() -> Option<Sun> {
    let now = clock::now()?;
    let offset = clock::utc_offset_minutes()?;
    let (latitude, longitude) = location().degrees();
    let today = now.days_since_epoch();
    Some(sun::compute(today, latitude, longitude, offset))
}

/// Whether it's between sunrise and sunset (round past midnight, where the
/// sun sets after it; see [`Sun`]). Without wall-clock time, it's
/// treated as night: the dark theme is the safe default when the screen
/// might light up in a dark room.
pub fn is_daytime() -> bool {
    let Some(now) = clock::now() else {
        return false;
    };
    let Some(sun) = sun() else { return false };
    sun.contains(u16::from(now.hour) * 60 + u16::from(now.minute))
}
