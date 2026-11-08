//! Nearby Aranet4 air monitors: what their Bluetooth advertisements say,
//! which one is nearest, and when to listen for them.
//!
//! An Aranet4 with "Smart Home integrations" switched on (in the Aranet
//! Home app; firmware 1.2.0 or later) puts its latest measurement in every
//! advertisement it sends, a few seconds apart: CO₂, temperature,
//! humidity, pressure, its battery, and how long ago it measured. So the
//! watch only has to listen (a passive scan, sending nothing), without
//! connecting or pairing. `docs/aranet-design.md` has the byte layout's
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
///
/// Every advertisement the scan hears comes through here, hundreds a
/// second in a busy place, so it does nothing but walk the length-prefixed
/// structures.
pub fn manufacturer_payload(mut ad: &[u8]) -> Option<&[u8]> {
    while let [length, rest @ ..] = ad {
        let length = usize::from(*length);
        // A zero length ends the data early (the rest is padding); one
        // longer than what's left is malformed.
        if length == 0 || length > rest.len() {
            return None;
        }
        let (structure, next) = rest.split_at(length);
        if let [MANUFACTURER_DATA, company @ ..] = structure
            && company.starts_with(&COMPANY)
        {
            return Some(&company[COMPANY.len()..]);
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
    /// for anything else (0 is "error"), from the default thresholds.
    fn from_status(status: u8, co2: Option<u16>) -> Option<Self> {
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

/// One measurement, as an Aranet4 advertises it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Reading {
    /// CO₂ in ppm; `None` while the sensor calibrates.
    pub co2: Option<u16>,
    /// Temperature in twentieths of a degree (the device's own unit);
    /// `None` if the device marks it invalid.
    pub twentieths: Option<u16>,
    /// Pressure in tenths of a hectopascal; `None` if marked invalid.
    pub pressure: Option<u16>,
    /// Relative humidity, %.
    pub humidity: u8,
    /// The device's battery, %.
    pub battery: u8,
    /// The CO₂ band, `None` when there's neither a status nor a CO₂ value.
    pub band: Option<Band>,
    /// Seconds between measurements: 60, 120, 300 (the default), or 600.
    pub interval: u16,
    /// Seconds since this measurement, when advertised.
    pub age: u16,
}

impl Reading {
    /// The temperature in tenths of a degree, rounded half up.
    pub fn tenths(&self) -> Option<i32> {
        self.twentieths.map(|t| (i32::from(t) + 1) / 2)
    }

    /// The temperature in quarter degrees (the die sensor's unit, for
    /// [`crate::temperature`]), rounded to nearest.
    pub fn quarters(&self) -> Option<i32> {
        self.twentieths.map(|t| (i32::from(t) * 4 + 10) / 20)
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
        co2,
        twentieths: valid(u16_at(10), 1 << 14),
        pressure: valid(u16_at(12), 1 << 15),
        humidity: payload[14],
        battery: payload[15],
        band: Band::from_status(payload[16], co2),
        interval: u16_at(17),
        age: u16_at(19),
    })
}

// ==============================================================================
// Which one is nearest
// ==============================================================================

/// The most sensors told apart at once. A home rarely has more than a
/// couple; beyond this, the weakest newcomers are ignored.
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
}

/// When two sensors' signals are this close (dB), the one shown stays:
/// a single report's strength swings by several dB (fading, and the body
/// shadowing the wrist), so a closer comparison would flip between two
/// sensors in the same room.
const HYSTERESIS_DB: i8 = 3;

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
/// enough to cost little (see `docs/aranet-design.md`'s energy section).
pub const EVERY: u64 = 15 * 60 * 1000;
/// How often to scan in the background once nothing has been heard for
/// [`RECENT`]: enough to notice an Aranet4 within the hour of coming home.
pub const SELDOM: u64 = 60 * 60 * 1000;
/// Showing the clock face with a sensor around scans if the last scan is
/// this old: the reading is about to be looked at.
pub const LOOKED_AT: u64 = 10 * 60 * 1000;
/// Opening the Air screen scans if the last scan is this old.
pub const OPENED: u64 = 60 * 1000;

/// How long a scan listens, at most, in ms. An Aranet4 advertises every
/// few seconds (about 4 s by the sources in `docs/aranet-design.md`; not
/// yet measured), so this hears each sensor around at least once, unless
/// a report is lost; most scans stop well before (see
/// [`Nearby::heard_everyone`]).
pub const SCAN_LIMIT: u64 = 5_000;

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
    /// quiet night, off the wrist, and during updates; a look at the clock
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
        if context.quiet_night || context.off_wrist {
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
    }

    /// An Aranet4 was heard, with its signal strength.
    pub fn hear(&mut self, address: Address, rssi: i8, reading: Reading, now: u64) {
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
    }

    /// Whether the scan has heard every sensor heard within [`RECENT`]
    /// (and at least one), so listening longer would only cost power.
    pub fn heard_everyone(&self, now: u64) -> bool {
        !self.heard.is_empty()
            && self
                .known
                .iter()
                .filter(|k| now.saturating_sub(k.heard) < RECENT)
                .all(|k| self.heard.iter().any(|s| s.address == k.address))
    }

    /// The scan is over: picks the sensor to show, the strongest heard,
    /// unless the one shown was heard within [`HYSTERESIS_DB`] of it.
    /// Returns whether anything was heard.
    pub fn end(&mut self, now: u64) -> bool {
        self.scanning = false;
        for sighting in &self.heard {
            match self
                .known
                .iter_mut()
                .find(|k| k.address == sighting.address)
            {
                Some(known) => known.heard = sighting.heard,
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

impl Default for Nearby {
    fn default() -> Self {
        Self::new()
    }
}
