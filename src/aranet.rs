//! Nearby room sensors: what an Aranet4 air monitor's Bluetooth
//! advertisements say, which sensor is nearest (an Aranet4 or a BTHome
//! thermometer, see [`crate::bthome_sensor`]), and when to listen for them.
//!
//! An Aranet4 with "Smart Home integrations" switched on (in the Aranet
//! Home app; firmware 1.2.0 or later) puts its latest measurement in every
//! advertisement it sends, about twice a second: CO₂, temperature,
//! humidity, pressure, its battery, and how long ago it measured. So the
//! watch only has to listen (a passive scan, sending nothing, for a second
//! or so), without connecting or pairing. `docs/aranet-design.md` has the byte layout's
//! sources and the radio's side; [`crate::ble`] does the listening.
//!
//! Listening costs the radio's receive current (~5.5 mA), so it happens
//! rarely, and briefly (see [`Nearby::due`]): a room's air changes over
//! many minutes, and the watch shows how old a reading is.
//!
//! Hardware-free, with time as milliseconds since boot, so it runs in host
//! tests (`host-tests/aranet`).

// ==============================================================================
// The advertisement
// ==============================================================================

/// SAF Tehnika's Bluetooth company ID (0x0702), as it appears on the air:
/// little-endian, first in the manufacturer-specific data.
const COMPANY: [u8; 2] = [0x02, 0x07];
/// The AD type of manufacturer-specific data.
const MANUFACTURER_DATA: u8 = 0xFF;

/// The manufacturer-specific data of SAF Tehnika (Aranet's maker) in an
/// advertisement's AD structures, after the company ID; `None` if there's
/// none, or the structures are malformed before it.
pub fn manufacturer_payload(ad: &[u8]) -> Option<&[u8]> {
    find(ad, MANUFACTURER_DATA, &COMPANY)
}

/// The first AD structure of type `kind` whose data starts with `prefix`
/// (a company ID, a service UUID), after the prefix; `None` if there's
/// none, or the structures are malformed before it.
///
/// Every advertisement the scan hears comes through here, hundreds a
/// second in a busy place, so it does nothing but walk the length-prefixed
/// structures.
pub fn find<'a>(mut ad: &'a [u8], kind: u8, prefix: &[u8]) -> Option<&'a [u8]> {
    while let [length, rest @ ..] = ad {
        let length = usize::from(*length);
        // A zero length ends the data early (the rest is padding); one
        // longer than what's left is malformed.
        if length == 0 || length > rest.len() {
            return None;
        }
        let (structure, next) = rest.split_at(length);
        if let [first, data @ ..] = structure
            && *first == kind
            && data.starts_with(prefix)
        {
            return Some(&data[prefix.len()..]);
        }
        ad = next;
    }
    None
}

/// The air's quality, as the Aranet4's own display colours it: green,
/// yellow, red. Those follow the thresholds set in the Aranet app (by
/// default 1000 and 1400 ppm), so the watch agrees with the device on the
/// wall.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[cfg_attr(target_os = "none", derive(defmt::Format))]
pub enum Band {
    Good,
    Fair,
    Poor,
}

impl Band {
    /// From the advertisement's status byte (1 green, 2 yellow, 3 red), or
    /// for anything else (0 is "error", and BTHome has none), from the
    /// default thresholds.
    pub fn from_status(status: u8, co2: Option<u16>) -> Option<Self> {
        match (status, co2) {
            (1, _) => Some(Self::Good),
            (2, _) => Some(Self::Fair),
            (3, _) => Some(Self::Poor),
            (_, Some(ppm)) if ppm < 1000 => Some(Self::Good),
            (_, Some(ppm)) if ppm <= 1400 => Some(Self::Fair),
            (_, Some(_)) => Some(Self::Poor),
            (_, None) => None,
        }
    }

    pub fn word(self) -> &'static str {
        match self {
            Self::Good => "GOOD",
            Self::Fair => "FAIR",
            Self::Poor => "POOR",
        }
    }
}

/// Which kind of sensor a reading is from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[cfg_attr(target_os = "none", derive(defmt::Format))]
pub enum Kind {
    /// An Aranet4: CO₂, temperature, humidity and pressure, advertised
    /// about twice a second.
    Aranet4,
    /// A BTHome sensor with a thermometer ([`crate::bthome_sensor`]),
    /// perhaps with humidity, pressure or CO₂ too. `irregular` if it says
    /// it only advertises when something happens (BTHome's "trigger
    /// based" flag), so a scan can't expect to hear it.
    BTHome { irregular: bool },
}

impl Kind {
    /// How long (ms) a scan may listen for a sensor of this kind that it
    /// heard last time, before giving up on it (see [`Nearby::limit`]).
    fn patience(self) -> u16 {
        match self {
            Self::Aranet4 => SCAN_LIMIT as u16,
            Self::BTHome { irregular: false } => SLOW_SCAN_LIMIT as u16,
            Self::BTHome { irregular: true } => 0,
        }
    }
}

/// One measurement, as a room sensor advertises it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Reading {
    pub kind: Kind,
    /// CO₂ in ppm; `None` while an Aranet4 calibrates, or from a sensor
    /// without CO₂.
    pub co2: Option<u16>,
    /// Temperature in hundredths of a degree (BTHome's finest unit; an
    /// Aranet4's twentieths are exact in it), signed, as a BTHome
    /// thermometer may be outdoors; `None` if the device marks it invalid.
    pub hundredths: Option<i16>,
    /// Pressure in tenths of a hectopascal; `None` if marked invalid, or
    /// not measured.
    pub pressure: Option<u16>,
    /// Relative humidity, %; `None` if not measured.
    pub humidity: Option<u8>,
    /// The device's battery, %; `None` if not sent.
    pub battery: Option<u8>,
    /// The CO₂ band, `None` when there's neither a status nor a CO₂ value.
    pub band: Option<Band>,
    /// Seconds between an Aranet4's measurements: 60, 120, 300 (the
    /// default), or 600. 0 for BTHome, which doesn't say.
    pub interval: u16,
    /// Seconds since this measurement, when advertised. 0 for BTHome,
    /// whose adverts carry the latest measurement.
    pub age: u16,
}

impl Reading {
    /// The temperature in tenths of a degree, rounded half up.
    pub fn tenths(&self) -> Option<i32> {
        self.hundredths.map(|h| (i32::from(h) + 5).div_euclid(10))
    }

    /// The temperature in quarter degrees (the die sensor's unit, for
    /// [`crate::temperature`]), rounded to nearest (half up).
    pub fn quarters(&self) -> Option<i32> {
        self.hundredths
            .map(|h| (i32::from(h) * 4 + 50).div_euclid(100))
    }
}

/// Why an advertisement with Aranet's company ID had no Aranet4 reading.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[cfg_attr(target_os = "none", derive(defmt::Format))]
pub enum Skip {
    /// "Smart Home integrations" is off: the 7-byte header only.
    IntegrationsOff,
    /// Firmware before 1.2.0: only the header, even with integrations on.
    OldFirmware,
    /// Another Aranet (Aranet2, Radiation, Radon): they put a model byte
    /// first and are 24 bytes long.
    ///
    /// TODO: the Aranet2's temperature and humidity, at offsets one byte
    /// further on, would serve the temperature as well.
    OtherModel,
    /// Too short, or a length we don't know.
    Malformed,
}

/// Bit 5 of the first byte: "Smart Home integrations" on.
const INTEGRATIONS: u8 = 1 << 5;
/// The Aranet4's payload length with readings (the header alone is 7).
const READING_LEN: usize = 22;

/// Reads an Aranet4's manufacturer data (after the company ID). The
/// layout, little-endian:
///
/// | Offset | Bytes | Field |
/// |---|---|---|
/// | 0 | 1 | flags; bit 5 = integrations on |
/// | 1–3 | 3 | firmware patch, minor, major |
/// | 4–7 | 4 | hardware revision, unknown |
/// | 8 | 2 | CO₂, ppm; invalid with bit 15 set |
/// | 10 | 2 | temperature, 1/20 °C; invalid with bit 14 set |
/// | 12 | 2 | pressure, 1/10 hPa; invalid with bit 15 set |
/// | 14 | 1 | humidity, % |
/// | 15 | 1 | battery, % |
/// | 16 | 1 | status colour: 1 green, 2 yellow, 3 red |
/// | 17 | 2 | measurement interval, s |
/// | 19 | 2 | age of the measurement, s |
/// | 21 | 1 | measurement counter |
pub fn parse(payload: &[u8]) -> Result<Reading, Skip> {
    let Some(&flags) = payload.first() else {
        return Err(Skip::Malformed);
    };
    match payload.len() {
        7 if flags & INTEGRATIONS == 0 => return Err(Skip::IntegrationsOff),
        7 => return Err(Skip::OldFirmware),
        24 => return Err(Skip::OtherModel),
        READING_LEN if flags & INTEGRATIONS == 0 => return Err(Skip::IntegrationsOff),
        READING_LEN => {}
        _ => return Err(Skip::Malformed),
    }
    let u16_at = |at: usize| u16::from_le_bytes([payload[at], payload[at + 1]]);
    let valid = |value: u16, invalid: u16| (value & invalid == 0).then_some(value);
    let co2 = valid(u16_at(8), 1 << 15);
    Ok(Reading {
        kind: Kind::Aranet4,
        co2,
        // Twentieths, five hundredths each. Above 327.67 °C doesn't fit,
        // and an Aranet4 (rated to 50 °C) never says so: that's `None`.
        hundredths: valid(u16_at(10), 1 << 14).and_then(|t| i16::try_from(i32::from(t) * 5).ok()),
        pressure: valid(u16_at(12), 1 << 15),
        humidity: Some(payload[14]),
        battery: Some(payload[15]),
        band: Band::from_status(payload[16], co2),
        interval: u16_at(17),
        age: u16_at(19),
    })
}

// ==============================================================================
// Which one is nearest
// ==============================================================================

/// The most sensors told apart at once, of either kind. A home rarely has
/// more than a few; beyond this, the weakest newcomers are ignored (a
/// block of flats may well have more BTHome thermometers in range, but
/// the strongest are the ones that matter).
pub const SENSORS: usize = 4;

/// A Bluetooth device address, as received (least significant byte
/// first).
pub type Address = [u8; 6];

/// A reading, from whom, how strong, and when it was heard.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Sighting {
    pub address: Address,
    /// The strongest signal heard from it during the scan, dBm.
    pub rssi: i8,
    pub reading: Reading,
    /// When it was heard, ms since boot.
    pub heard: u64,
}

impl Sighting {
    /// Seconds since the measurement was taken, as of `now`: what it was
    /// when advertised, plus the time since.
    pub fn age(&self, now: u64) -> u64 {
        u64::from(self.reading.age) + now.saturating_sub(self.heard) / 1000
    }
}

/// A sensor heard within [`RECENT`] (or that is shown), for deciding when
/// a scan has heard everyone.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Known {
    address: Address,
    heard: u64,
    /// How long (ms) the next scan may wait for it ([`Kind::patience`]):
    /// 0 if the last scan didn't hear it, or it advertises irregularly.
    /// (In what would be padding after the address.)
    patience: u16,
}

/// What the scans have heard: the nearest sensor's latest reading, and
/// when to scan next.
#[derive(Debug)]
pub struct Nearby {
    /// The sensor shown, and its latest reading.
    best: Option<Sighting>,
    /// What the scan under way has heard so far, at most one per sensor,
    /// with its strongest signal.
    heard: heapless::Vec<Sighting, SENSORS>,
    /// Sensors heard recently: a scan that has heard all of them again
    /// can stop early.
    known: heapless::Vec<Known, SENSORS>,
    /// When the last scan began (or was asked for), ms since boot.
    last_scan: Option<u64>,
    /// Whether a scan is under way.
    scanning: bool,
    /// Scans begun, wrapping, for [`LOOK_EVERY`]. (In what would be
    /// padding.)
    scans: u8,
    /// Where the phone was when sensors were last near (see
    /// [`Nearby::located`]).
    place: Option<Place>,
}

/// When two sensors' signals are this close (dB), the one shown stays:
/// a single report's strength swings by several dB (fading, and the body
/// shadowing the wrist), so a closer comparison would flip between two
/// sensors in the same room.
const HYSTERESIS_DB: i8 = 3;

/// The weakest signal (dBm, the strongest of a scan's reports) at which a
/// sensor counts at all: anything weaker is taken for a neighbour's,
/// through a wall or a floor, rather than the room's, and ignored, as if
/// unheard. Otherwise, with nothing nearer heard, a thermometer next door
/// would become the room's temperature, and calibrate the wrist's.
///
/// Indoors, a sensor in the same room is typically heard at −40 to −75 dBm,
/// and through a wall at −75 to −90. On the dev kit, two Aranet4s came in
/// at −63 and −84: one in the room, one past a wall (TODO: replace with
/// the dev kit's spread once measured). −80 keeps the first with 17 dB to
/// spare, room for the wrist's own shadow (5–10 dB), and drops the other.
/// A sensor of one's own across a large room may fall below it, now and
/// then, on the wrist: the watch then shows its own estimate, as it does
/// with no sensor at all, which is the safe way to be wrong.
///
/// No hysteresis in dB, to let a sensor shown stay a little below the
/// floor: one that once peaked above it would then stay at the
/// through-the-wall level, the very thing this keeps out. A sensor at the
/// edge doesn't flicker anyway: its reading stays shown until [`STALE`]
/// (two scans missed), and it's "around" (the Air screen) for [`RECENT`].
/// And a scan waiting for it ([`Nearby::heard_everyone`]) doesn't count a
/// report below the floor, so it listens on, up to its limit, and takes
/// the strongest of several reports, not the first.
pub const FLOOR_DBM: i8 = -80;

/// A sensor heard within this long is still "around": the scans keep
/// their normal pace, and wait for it before stopping early. Also how
/// long the Air screen exists after the last reading.
pub const RECENT: u64 = 60 * 60 * 1000;

/// A reading this old (from its measurement, seconds) or older no longer
/// stands for the room the wearer is in: it's no longer the temperature
/// shown (see [`Nearby::fresh`]), and it's drawn faint. Two missed scans'
/// worth at [`EVERY`].
pub const STALE: u64 = 35 * 60;

/// How often to scan in the background, while a sensor is around: often
/// enough that a reading is at most a quarter of an hour old, and rare
/// enough to cost little (see `docs/aranet-design.md`'s status: a few
/// µA).
pub const EVERY: u64 = 15 * 60 * 1000;
/// How often to scan in the background once nothing has been heard for
/// [`RECENT`]: enough to notice an Aranet4 within the hour of coming home.
pub const SELDOM: u64 = 60 * 60 * 1000;
/// Showing the clock face with a sensor around scans if the last scan is
/// this old: the reading is about to be looked at.
pub const LOOKED_AT: u64 = 10 * 60 * 1000;
/// Opening the Air screen scans if the last scan is this old.
pub const OPENED: u64 = 60 * 1000;

/// How long a scan listens, at most, in ms, unless it waits for a BTHome
/// sensor ([`SLOW_SCAN_LIMIT`]). Two Aranet4s (firmware 2.0.7)
/// advertised every ~0.57 s, and a dev kit, scanning while connected to a
/// phone, heard them at most 2.3 s apart (90% within 1.2 s): this hears
/// every sensor around, with room for a slower one; most scans stop well
/// before (see [`Nearby::heard_everyone`]).
pub const SCAN_LIMIT: u64 = 3_000;

/// How long a scan listens, at most, in ms, when it waits for a BTHome
/// thermometer it heard last time. The commonest, a Xiaomi LYWSD03MMC on
/// pvvx's firmware, advertises every 2.5 s by default, but alternates its
/// temperature with its battery voltage, so the temperature comes every
/// 5 s: this hears it whatever the phase. Scans that wait for none (to
/// find sensors, or with only Aranet4s around) keep to [`SCAN_LIMIT`].
///
/// Sensors that advertise more seldom aren't caught by any scan short
/// enough to be cheap (a Shelly BLU H&T sends once a minute): a scan
/// hears them now and then, and their reading lasts until [`STALE`].
pub const SLOW_SCAN_LIMIT: u64 = 6_000;

/// With no sensor heard recently, a scan listens at least this long (ms)
/// before stopping at the first it hears (see [`Nearby::heard_everyone`]).
pub const LOOK_AROUND: u64 = 1_200;

/// Every this many scans, one listens to its limit ([`Nearby::limit`])
/// whatever it hears, to find sensors it doesn't know yet. The others stop
/// once they've heard the known ones, often within a fraction of a second
/// (an Aranet4 advertises every ~0.57 s), which leaves a new sensor beside
/// them little chance to be heard: on a dev kit, a laptop advertising a
/// BTHome thermometer every second went unheard by scans that stopped at
/// 131 and 214 ms. At the quarter-hour pace, this is every two hours; a
/// new sensor is rare, and finding it sooner isn't worth more: each costs
/// ~2 s of listening more than a scan that stops early, ~1 µA in all.
pub const LOOK_EVERY: u8 = 8;

/// What a scan is for, for the log.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[cfg_attr(target_os = "none", derive(defmt::Format))]
pub enum Why {
    /// The background pace ([`EVERY`], [`SELDOM`]).
    Background,
    /// The clock face is showing ([`LOOKED_AT`]).
    Clock,
    /// The Air screen opened ([`OPENED`]).
    Screen,
}

/// What [`Nearby::due`] needs to know about the watch.
#[derive(Clone, Copy, Debug, Default)]
pub struct Context {
    /// The quiet night: asleep, nobody looking, and a wrist under a duvet
    /// says nothing about the room.
    pub quiet_night: bool,
    /// Known to be off the wrist (see [`crate::temperature::Wear`]): the
    /// watch's own temperature is then the room's, once settled, and a
    /// look at it scans anyway.
    pub off_wrist: bool,
    /// The clock face is on screen.
    pub showing_clock: bool,
    /// The Air screen is on screen.
    pub showing_air: bool,
    /// A firmware update is arriving: nothing may slow it down.
    pub updating: bool,
}

impl Nearby {
    pub const fn new() -> Self {
        Self {
            best: None,
            heard: heapless::Vec::new(),
            known: heapless::Vec::new(),
            last_scan: None,
            scanning: false,
            scans: 0,
            place: None,
        }
    }

    /// The sensor shown, with its latest reading.
    pub fn best(&self) -> Option<Sighting> {
        self.best
    }

    /// The shown sensor's reading if it's recent enough to stand for the
    /// room ([`STALE`]).
    pub fn fresh(&self, now: u64) -> Option<Sighting> {
        self.best.filter(|s| s.age(now) < STALE)
    }

    /// Whether a sensor was heard within [`RECENT`].
    pub fn around(&self, now: u64) -> bool {
        self.known
            .iter()
            .any(|k| now.saturating_sub(k.heard) < RECENT)
    }

    /// Why to scan now, if it's time. The background pace stops in the
    /// quiet night, off the wrist (after a first scan), and during updates; a look at the clock
    /// face with a sensor around, or opening the Air screen, can bring a
    /// scan forward.
    pub fn due(&self, now: u64, context: Context) -> Option<Why> {
        if self.scanning || context.updating {
            return None;
        }
        let since = self.last_scan.map_or(u64::MAX, |at| now.saturating_sub(at));
        if context.showing_air && since >= OPENED {
            return Some(Why::Screen);
        }
        if context.showing_clock && self.around(now) && since >= LOOKED_AT {
            return Some(Why::Clock);
        }
        // Off the wrist, not at all, bar once after booting: that's on a
        // desk (where the watch can't yet tell it isn't worn) as often as
        // on a wrist, and finds out whether there's a sensor around.
        if context.quiet_night || (context.off_wrist && self.last_scan.is_some()) {
            return None;
        }
        let pace = if self.around(now) { EVERY } else { SELDOM };
        (since >= pace).then_some(Why::Background)
    }

    /// A scan begins (or has been asked for): what it hears is collected
    /// afresh.
    pub fn begin(&mut self, now: u64) {
        self.heard.clear();
        self.last_scan = Some(now);
        self.scanning = true;
        self.scans = self.scans.wrapping_add(1);
    }

    /// A sensor was heard, with its signal strength. Returns whether it
    /// counts: not if it's below [`FLOOR_DBM`].
    pub fn hear(&mut self, address: Address, rssi: i8, reading: Reading, now: u64) -> bool {
        if rssi < FLOOR_DBM {
            return false;
        }
        let sighting = Sighting {
            address,
            rssi,
            reading,
            heard: now,
        };
        match self.heard.iter_mut().find(|s| s.address == address) {
            // The strongest of its reports: a single report's strength
            // swings with fading, and the strongest is steadier than the
            // latest.
            Some(earlier) => {
                *earlier = Sighting {
                    rssi: rssi.max(earlier.rssi),
                    ..sighting
                }
            }
            None => {
                if self.heard.push(sighting).is_err() {
                    // Full: a newcomer replaces the weakest, if stronger.
                    let weakest = self
                        .heard
                        .iter_mut()
                        .min_by_key(|s| s.rssi)
                        .expect("full, so not empty");
                    if weakest.rssi < rssi {
                        *weakest = sighting;
                    }
                }
            }
        }
        true
    }

    /// How long the scan under way may listen, at most, in ms:
    /// [`SCAN_LIMIT`], or longer while it waits for a slower sensor (see
    /// [`SLOW_SCAN_LIMIT`]).
    pub fn limit(&self, now: u64) -> u64 {
        self.awaited(now)
            .map(|k| u64::from(k.patience))
            .fold(SCAN_LIMIT, u64::max)
    }

    /// The sensors a scan waits for: those the last scan heard, within
    /// [`RECENT`], that advertise regularly.
    ///
    /// Not every sensor heard within the hour: one the last scan missed
    /// has likely gone out of range (or advertises too seldom to catch),
    /// and waiting for it would hold every scan to its limit, for the
    /// hour, for nothing. If it's there, the scan may hear it anyway.
    fn awaited(&self, now: u64) -> impl Iterator<Item = &Known> {
        self.known
            .iter()
            .filter(move |k| k.patience > 0 && now.saturating_sub(k.heard) < RECENT)
    }

    /// Whether the scan has heard every sensor it waits for ([`Self::awaited`])
    /// (and at least one), so listening longer would only cost power.
    /// Never, in one scan of every [`LOOK_EVERY`] (the eighth, the
    /// sixteenth, …), which looks for new sensors.
    ///
    /// Waiting for none, the first one heard isn't enough: a nearer one
    /// may not have advertised yet, so it listens for at least
    /// [`LOOK_AROUND`], two of an Aranet4's advertising periods.
    pub fn heard_everyone(&self, now: u64) -> bool {
        if self.scans.is_multiple_of(LOOK_EVERY) {
            return false;
        }
        let mut recent = self.awaited(now).peekable();
        let looked_around = recent.peek().is_some()
            || self
                .last_scan
                .is_some_and(|began| now.saturating_sub(began) >= LOOK_AROUND);
        looked_around
            && !self.heard.is_empty()
            && recent.all(|k| self.heard.iter().any(|s| s.address == k.address))
    }

    /// The scan is over: picks the sensor to show, the strongest heard,
    /// unless the one shown was heard within [`HYSTERESIS_DB`] of it.
    /// Returns whether anything was heard.
    pub fn end(&mut self, now: u64) -> bool {
        self.scanning = false;
        // What this scan missed, the next doesn't wait for.
        for known in &mut self.known {
            known.patience = 0;
        }
        for sighting in &self.heard {
            let patience = sighting.reading.kind.patience();
            match self
                .known
                .iter_mut()
                .find(|k| k.address == sighting.address)
            {
                Some(known) => {
                    known.heard = sighting.heard;
                    known.patience = patience;
                }
                None => {
                    // Room for it, or in place of the longest unheard.
                    if self.known.is_full() {
                        let oldest = self
                            .known
                            .iter()
                            .enumerate()
                            .min_by_key(|(_, k)| k.heard)
                            .map(|(i, _)| i)
                            .expect("full, so not empty");
                        self.known.swap_remove(oldest);
                    }
                    let _ = self.known.push(Known {
                        address: sighting.address,
                        heard: sighting.heard,
                        patience,
                    });
                }
            }
        }
        self.known.retain(|k| now.saturating_sub(k.heard) < RECENT);
        let Some(strongest) = self.heard.iter().max_by_key(|s| s.rssi).copied() else {
            return false;
        };
        let shown = self
            .best
            .and_then(|best| self.heard.iter().find(|s| s.address == best.address));
        self.best = Some(match shown {
            Some(&shown) if shown.rssi.saturating_add(HYSTERESIS_DB) >= strongest.rssi => shown,
            _ => strongest,
        });
        true
    }
}

// ==============================================================================
// Where the sensors are
// ==============================================================================
//
// Room sensors stay put. So once the phone says the wearer is well away
// from where they were heard, the sensors known can be forgotten at once,
// rather than looked for every quarter of an hour (and waited for, and
// looked for at every glance at the clock) until an hour has passed
// without them. The hourly scan for sensors goes on, and finds those of
// wherever the wearer is now.
//
// The watch only learns where it is now and then: the phone sends a fix
// when asked (see `geo::GpsPolicy`): after it was away for two hours, at
// a new time zone, and if the last is a day old when it connects. So this
// acts at those moments, and otherwise everything is as it was.

/// A position, in millionths of a degree (as `geo::Location`, which this
/// module can't use: it runs in host tests without the firmware's
/// dependencies).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Place {
    pub latitude_micro: i32,
    /// East positive.
    pub longitude_micro: i32,
}

/// How far from where sensors were near a fix must be (metres) for them
/// to count as away.
///
/// A room sensor is heard across a flat or a house, a few tens of metres
/// (less with [`FLOOR_DBM`]). Only fixes good to [`PRECISE`] count, so
/// at home the place and a later fix are each typically 10–40 m off (a
/// Wi-Fi fix), at most ~75 m (Android's accuracy is a 68% radius): a fix
/// at home rarely lands 200 m from the place (~5% with both at the
/// limit, next to never for typical fixes), and if it does, it costs at
/// most an hour without the room's reading, until the hourly scan hears
/// it again.
///
/// Why not more: a fix soon after leaving should already count. One 0–15
/// min after leaving (a scan's interval) on foot (5 km/h) is 0–1.25 km
/// away: at 500 m, two walks out in five were still within it, and
/// changed nothing; at 200 m, one in six.
pub const AWAY: f32 = 200.0;

/// The most a fix may be off (metres, the accuracy the phone gives with
/// it) to tell whether the wearer is near the sensors ([`AWAY`]).
/// A phone's location is typically good to 5–50 m (GPS, or Wi-Fi), but
/// one from cell towers alone can be hundreds of metres off, and two
/// such fixes at home could look further apart than [`AWAY`]. 75 m keeps
/// Wi-Fi fixes, even in a sparse suburb, and leaves out cell fixes.
/// Coarser fixes still set the sunrise's location.
pub const PRECISE: f32 = 75.0;

/// Whether a fix whose accuracy is `metres` (if it said) is good enough
/// for [`Nearby::located`]: see [`PRECISE`]. Without an accuracy (or with
/// Android's 0, "unknown"), no: there's no telling.
pub fn precise(metres: Option<f32>) -> bool {
    metres.is_some_and(|m| m > 0.0 && m <= PRECISE)
}

/// The square of the distance between two places, in m², near enough
/// for a few kilometres: the earth as flat around them (equirectangular),
/// with a degree of longitude shrunk by the cosine of the latitude, from
/// the start of its series (within 2% to 70° north, and 6% at 80°).
pub fn distance_squared(a: Place, b: Place) -> f32 {
    // Metres in a millionth of a degree of latitude.
    const METRES: f32 = 0.111_195;
    let latitude =
        (a.latitude_micro as f32 + b.latitude_micro as f32) / 2.0 * (core::f32::consts::PI / 180e6);
    let squared = latitude * latitude;
    let cos = 1.0 - squared / 2.0 + squared * squared / 24.0;
    // Differences as integers first: a float can't hold a whole latitude
    // to the micro-degree. Longitudes the short way round, across 180°.
    let north = (i64::from(b.latitude_micro) - i64::from(a.latitude_micro)) as f32;
    let mut east = (i64::from(b.longitude_micro) - i64::from(a.longitude_micro)).abs();
    east = east.min(360_000_000 - east);
    let east = east as f32 * cos;
    (north * north + east * east) * (METRES * METRES)
}

impl Nearby {
    /// The phone sent where it is (`here`, a fix good to [`PRECISE`]). If a sensor was heard within
    /// a background scan's interval, this is where the sensors are. If
    /// not, and it's more than [`AWAY`] from where they were, the sensors
    /// known are gone: forgotten, with the reading shown, so that scans
    /// fall back to the hourly look for sensors at once. Back near it,
    /// with none known, scans at once: the sensors there are likely back
    /// in range. Returns whether it forgot them.
    ///
    /// TODO: the place isn't kept in flash, so after a restart it waits
    /// for a fix near a sensor before this does anything.
    pub fn located(&mut self, here: Place, now: u64) -> bool {
        if self
            .known
            .iter()
            .any(|k| now.saturating_sub(k.heard) <= EVERY)
        {
            self.place = Some(here);
            return false;
        }
        let Some(place) = self.place else {
            return false;
        };
        let away = distance_squared(place, here) > AWAY * AWAY;
        let forgot = away && (!self.known.is_empty() || self.best.is_some());
        if away {
            self.known.clear();
            self.best = None;
        } else if self.known.is_empty() && !self.scanning {
            self.last_scan = None;
        }
        forgot
    }
}

impl Default for Nearby {
    fn default() -> Self {
        Self::new()
    }
}
