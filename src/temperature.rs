//! The temperature on the clock face: the room's, as well as the watch can
//! tell.
//!
//! The PineTime has no ambient temperature sensor, only the one on its
//! chip (the nRF52832's die sensor), which measures the watch itself. So
//! the temperature shown comes from the best of three sources ([`pick`]):
//!
//! 1. **A nearby Aranet4** ([`crate::aranet`]), the strongest heard, if
//!    its reading is recent: a real air temperature, to 0.3 °C.
//!    Underlined, at the degree sign, on the clock face.
//! 2. **On the wrist, an estimate** from the die temperature
//!    ([`Calibration`]). The watch sits between the skin (32–34 °C) and
//!    the air, so the die reads well above the room (27–28 °C on the
//!    wrist indoors, against ~21 °C), and the estimate takes the skin's
//!    share back out. It learns from the Aranet4: whenever one is near
//!    and the watch has been on the wrist a while, each (die, Aranet)
//!    pair refines the fit.
//! 3. **Off the wrist, the die temperature as it is**: with no skin
//!    warming it, it settles to the room's within some minutes (until
//!    then it reads a little warm).
//!
//! How good the estimate is, honestly: with the defaults alone (never
//! near an Aranet4), expect ±3–5 °C, mostly the die sensor's own offset,
//! which Nordic only specifies to within a few degrees and which differs
//! from chip to chip. Once calibrated, about ±2–3 °C sitting indoors, at
//! the temperatures it was calibrated at (an expectation from the model,
//! not yet measured on a wrist). Worse, by several degrees,
//! in the cold (skin cools, and the die with it, in ways a straight line
//! doesn't follow), in sun, during exercise (a warm wrist reads as a warm
//! room), in the first ~20 minutes after putting the watch on or walking
//! into a different room (the watch takes that long to settle), and under
//! a sleeve or a duvet (which isn't the room's air at all). The Aranet4's
//! reading has none of these problems, which is why it wins when near.
//!
//! Whether the watch is on the wrist ([`Wear`]) is itself a guess, from
//! the heart rate sensor and stillness.
//!
//! The die sensor (the TEMP peripheral) belongs to MPSL, Nordic's radio
//! scheduler, so `main` reads it through MPSL rather than directly. A
//! reading takes ~50 µs. Its resolution is 0.25 °C.
//!
//! The watch warms itself a little while it works, but not enough to
//! matter: on the (open-cased) dev kit, idling at 20.5 °C, five minutes
//! with the screen and backlight on raised the reading by a quarter of a
//! degree, gone again a minute after the screen went off. That's within
//! what [`shown`] smooths away, so the sensor is read on a fixed schedule,
//! whether or not the screen is on.
//!
//! Hardware-free, with time as milliseconds since boot, so it runs in host
//! tests (`host-tests/temperature`).
//!
//! TODO: °F for those who'd want it (a setting, or the phone's locale
//! from Gadgetbridge).

/// How often to read the sensor, in seconds. The watch's temperature
/// changes over minutes (its case and the wrist are slow to warm and cool),
/// so once a minute is plenty.
pub const SAMPLE_EVERY: u64 = 60;

const MINUTE: u64 = 60 * 1000;

/// The whole degrees (°C) to show, given what was shown before and a new
/// reading in quarter degrees (the sensor's unit).
///
/// A reading near the boundary between two degrees (wobbling between,
/// say, 31.25 and 31.75 °C) would otherwise flip the shown value back and
/// forth from minute to minute, so it only changes once the reading is a
/// whole degree away from what's shown, and then to the nearest whole
/// degree. What's shown is thus never more than 0.75 °C off.
pub fn shown(previous: Option<i8>, quarters: i32) -> i8 {
    if let Some(previous) = previous
        && (quarters - 4 * i32::from(previous)).abs() < 4
    {
        return previous;
    }
    // Round half up, also for negative readings (`div_euclid` floors).
    let nearest = (quarters + 2).div_euclid(4);
    // The sensor's range is -40 to 85 °C, so this never clamps in
    // practice.
    nearest.clamp(i8::MIN.into(), i8::MAX.into()) as i8
}

// ==============================================================================
// Which source
// ==============================================================================

/// Where the temperature shown comes from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[cfg_attr(target_os = "none", derive(defmt::Format))]
pub enum Source {
    /// A nearby Aranet4.
    Aranet,
    /// Estimated from the die temperature, on the wrist.
    Wrist,
    /// The die temperature, off the wrist.
    Watch,
}

/// The temperature to show, in quarter degrees, and where it comes from:
/// a recent Aranet4 reading if there is one (`aranet`, quarter degrees),
/// otherwise the estimate on the wrist, otherwise the die's own (`die`,
/// quarter degrees).
pub fn pick(aranet: Option<i32>, worn: bool, die: i32, calibration: &Calibration) -> (i32, Source) {
    match aranet {
        Some(quarters) => (quarters, Source::Aranet),
        None if worn => (calibration.estimate(die), Source::Wrist),
        None => (die, Source::Watch),
    }
}

// ==============================================================================
// On the wrist or not
// ==============================================================================

/// After this long perfectly still, the watch is taken to be lying
/// somewhere. A wrist, even at a desk, is never that still while awake:
/// a day's log at a desk had its longest still spell at 18 s.
const STILL_OFF: u64 = 15 * MINUTE;

/// Whether the watch is on a wrist, as far as can be told.
///
/// The heart rate sensor is the only sensor that sees skin, and it only
/// looks now and then (see [`crate::heart`]), and often can't tell: on a
/// busy wrist, or lying on a desk (which reflects its LED like skin),
/// a session ends without a verdict. So a heart reading (a pulse means
/// skin) says "on", while "off" comes from the sensor seeing nothing, or
/// from the watch keeping perfectly still for [`STILL_OFF`]. Each stands
/// until the other says otherwise. It starts off: worn takes a pulse,
/// and until one, the die's own temperature is the safer thing to show (an
/// estimate applied to a watch on a desk would read many degrees too
/// cold).
#[derive(Clone, Copy, Debug)]
pub struct Wear {
    /// The latest sign of being off the wrist, ms since boot.
    off: u64,
    /// Since when the watch has been still, if it is.
    still_since: Option<u64>,
    /// Since when it's been on the wrist, if it is: the first pulse after
    /// `off`.
    worn_since: Option<u64>,
}

impl Wear {
    /// At boot (`now`), not known to be worn.
    pub const fn new(now: u64) -> Self {
        Self {
            off: now,
            still_since: None,
            worn_since: None,
        }
    }

    /// Takes in what the sensors say at `now`: whether the heart rate
    /// sensor's last look found nothing against it (`no_skin`), whether
    /// the watch is still, and when the latest heart rate was measured.
    pub fn update(&mut self, now: u64, no_skin: bool, still: bool, pulse: Option<u64>) {
        self.still_since = if still {
            Some(self.still_since.unwrap_or(now))
        } else {
            None
        };
        let lying = self
            .still_since
            .is_some_and(|since| now.saturating_sub(since) >= STILL_OFF);
        if no_skin || lying {
            self.off = now;
            self.worn_since = None;
        } else if self.worn_since.is_none()
            && let Some(at) = pulse.filter(|&at| at > self.off)
        {
            self.worn_since = Some(at);
        }
    }

    pub fn worn(&self) -> bool {
        self.worn_since.is_some()
    }

    /// How long it's been on the wrist, if it is.
    pub fn worn_for(&self, now: u64) -> Option<u64> {
        self.worn_since.map(|since| now.saturating_sub(since))
    }
}

// ==============================================================================
// The estimate, and learning it
// ==============================================================================
//
// The model: the watch's die sits between the skin and the air, so its
// temperature is a weighted mix of the two, and with the skin roughly
// steady, the air is a straight line in the die:
//
//     air ≈ k · die + c
//
// k is above 1 (each degree the room cools moves the die by less than a
// degree, the skin holding it up), and c takes the skin's warmth, and the
// die sensor's own offset, back out. A line can't follow everything (the
// skin cools in the cold, exercise warms the wrist), which is the accuracy
// the module docs admit to.
//
// The fit is a least-squares line through (die, Aranet) pairs, with a
// prior: it starts at a guess, and moves as far as the pairs say. Pairs
// only come at the temperatures of rooms with an Aranet4, often within a
// few degrees, which pins the line's height (c, where the offset is) well
// but its slope (k) poorly, so the prior holds the slope much harder than
// the height. Older pairs fade (`FORGET`), so seasons and a changed strap
// find their way in. Five running sums are all it keeps: 20 bytes, in
// flash.

/// Where the line is centred, °C: the die and air temperatures the
/// defaults expect together, so that the sums stay small and the slope and
/// height separate cleanly. The sealed watch read 27–28 °C on a wrist
/// indoors (2026-11-08), in a room of about 21–22 °C.
const DIE_CENTRE: f32 = 27.5;
const AIR_CENTRE: f32 = 21.5;
/// The prior slope. From a two-way mix of skin and air, with the skin
/// cooling by ~0.3 °C per degree of air: the die's share of the air is
/// ~0.5 (from the numbers above, with skin at 33 °C), so a degree of air
/// moves the die by 0.5 + 0.5·0.3 ≈ 0.65 °C, and k ≈ 1/0.65 ≈ 1.6. A
/// guess from physics, not data: calibration is what refines it.
const PRIOR_SLOPE: f32 = 1.6;
/// How hard the prior holds, as if it were this many pairs (the height)
/// or this much spread of die temperatures (the slope, in °C²: sixty
/// pairs a degree from the centre, say). Pairs from one room can't tell a
/// steeper line from a lower one; with these weights, they move the
/// height (where a chip's offset shows) and leave the slope nearly alone
/// (ten pairs 1.5 °C off centre, wanting 3 °C less, move it by 0.07). The
/// slope needs pairs over a spread of several degrees, from rooms or
/// seasons apart, to move far.
const HEIGHT_WEIGHT: f32 = 1.0;
const SLOPE_WEIGHT: f32 = 60.0;
/// Each new pair weighs the old ones down by this much: a half-life of
/// ~70 pairs, a few days of wear near an Aranet4.
const FORGET: f32 = 0.99;
/// The slope stays physical whatever the pairs say: at least 1 (the die
/// never moves more than the air), and at most 3 (the die over two
/// thirds skin).
const SLOPE_RANGE: (f32, f32) = (1.0, 3.0);
/// However the line fits, the estimate stays within this of the die, °C:
/// the room is cooler than a warm wrist, but not by more than this, and
/// warmer by little (only above skin temperature).
const CORRECTION_RANGE: (f32, f32) = (-15.0, 5.0);

/// The running fit of the air temperature on the die's, on the wrist.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Calibration {
    /// The pairs' weight, and their sums, centred: x is the die's
    /// temperature less [`DIE_CENTRE`], y the air's less [`AIR_CENTRE`].
    n: f32,
    sx: f32,
    sy: f32,
    sxx: f32,
    sxy: f32,
}

/// The format version of [`Calibration::to_bytes`].
const CALIBRATION_FORMAT: u8 = 1;
pub const CALIBRATION_BYTES: usize = 21;

impl Calibration {
    /// No pairs yet: the prior alone.
    pub const fn new() -> Self {
        Self {
            n: 0.0,
            sx: 0.0,
            sy: 0.0,
            sxx: 0.0,
            sxy: 0.0,
        }
    }

    /// How many pairs it has, by weight (older ones count for less).
    pub fn pairs(&self) -> f32 {
        self.n
    }

    /// The line, (k, c) around the centres: air − AIR_CENTRE = k · (die −
    /// DIE_CENTRE) + c. Ridge regression with the prior's pull: the normal
    /// equations, each with the prior's weight added on its diagonal and
    /// the prior's value on its right-hand side.
    pub fn line(&self) -> (f32, f32) {
        let (a, b, d) = (self.sxx + SLOPE_WEIGHT, self.sx, self.n + HEIGHT_WEIGHT);
        let (e, f) = (self.sxy + SLOPE_WEIGHT * PRIOR_SLOPE, self.sy);
        // Positive: a and d are at least the weights, and by Cauchy–Schwarz
        // sx² ≤ n · sxx.
        let det = a * d - b * b;
        // `max` and `min` rather than `f32::clamp`, whose assertion on its
        // bounds formats floats into its panic message: 12 KB of flash.
        let k = ((e * d - b * f) / det)
            .max(SLOPE_RANGE.0)
            .min(SLOPE_RANGE.1);
        // The height that best fits, given that slope.
        let c = (f - b * k) / d;
        (k, c)
    }

    /// The air temperature for a die temperature, both in quarter degrees.
    pub fn estimate(&self, die: i32) -> i32 {
        let (k, c) = self.line();
        let die_c = die as f32 / 4.0;
        let air = AIR_CENTRE + k * (die_c - DIE_CENTRE) + c;
        let air = air
            .max(die_c + CORRECTION_RANGE.0)
            .min(die_c + CORRECTION_RANGE.1);
        libm::roundf(air * 4.0) as i32
    }

    /// Learns from a pair: the die's and the air's temperatures at the same
    /// time, in quarter degrees.
    pub fn learn(&mut self, die: i32, air: i32) {
        let x = die as f32 / 4.0 - DIE_CENTRE;
        let y = air as f32 / 4.0 - AIR_CENTRE;
        self.n = self.n * FORGET + 1.0;
        self.sx = self.sx * FORGET + x;
        self.sy = self.sy * FORGET + y;
        self.sxx = self.sxx * FORGET + x * x;
        self.sxy = self.sxy * FORGET + x * y;
    }

    pub fn to_bytes(self) -> [u8; CALIBRATION_BYTES] {
        let mut bytes = [0; CALIBRATION_BYTES];
        bytes[0] = CALIBRATION_FORMAT;
        for (i, value) in [self.n, self.sx, self.sy, self.sxx, self.sxy]
            .into_iter()
            .enumerate()
        {
            bytes[1 + 4 * i..5 + 4 * i].copy_from_slice(&value.to_le_bytes());
        }
        bytes
    }

    /// The inverse of [`Self::to_bytes`]; the prior alone for anything
    /// else (another format, or numbers that can't be sums).
    pub fn from_bytes(bytes: [u8; CALIBRATION_BYTES]) -> Self {
        let at = |i: usize| {
            f32::from_le_bytes(
                bytes[1 + 4 * i..5 + 4 * i]
                    .try_into()
                    .expect("a 4-byte slice"),
            )
        };
        let calibration = Self {
            n: at(0),
            sx: at(1),
            sy: at(2),
            sxx: at(3),
            sxy: at(4),
        };
        let sane = [at(0), at(1), at(2), at(3), at(4)]
            .iter()
            .all(|v| v.is_finite())
            && calibration.n >= 0.0
            && calibration.sxx >= 0.0;
        if bytes[0] == CALIBRATION_FORMAT && sane {
            calibration
        } else {
            Self::new()
        }
    }
}

impl Default for Calibration {
    fn default() -> Self {
        Self::new()
    }
}

/// How long the watch must have been on the wrist before its temperature
/// says anything about the room: it takes about this long to warm up to
/// the wrist.
const SETTLED: u64 = 20 * MINUTE;
/// An Aranet4 reading measured longer ago than this (seconds) is too old
/// to pair with the die's temperature now. Its longest measurement
/// interval is 10 minutes.
pub const PAIR_AGE: u64 = 10 * 60;
/// Two pairs in a row this close together (ms) whose temperatures agree
/// within [`STEADY`] show that the wearer is settled in one room.
const PAIR_GAP: u64 = 25 * MINUTE;
/// Quarter degrees: 1 °C.
const STEADY: i32 = 4;

/// Decides which (die, Aranet) pairs are fit to learn from: only on a
/// wrist that has settled ([`SETTLED`]), and only when the pair before,
/// at most [`PAIR_GAP`] earlier, had both temperatures within [`STEADY`]
/// of this one's. That leaves out walking between rooms (the die lags the
/// air by many minutes), and most bursts of exercise.
#[derive(Clone, Copy, Debug, Default)]
pub struct Pairing {
    /// The last pair offered: when, the die's, the air's.
    last: Option<(u64, i32, i32)>,
}

impl Pairing {
    pub const fn new() -> Self {
        Self { last: None }
    }

    /// Offers a pair at `now` (quarter degrees), with how long the watch
    /// has been worn; returns whether to learn from it.
    pub fn offer(&mut self, now: u64, die: i32, air: i32, worn_for: Option<u64>) -> bool {
        let settled = worn_for.is_some_and(|worn| worn >= SETTLED);
        let steady = self.last.is_some_and(|(at, last_die, last_air)| {
            now.saturating_sub(at) <= PAIR_GAP
                && (die - last_die).abs() <= STEADY
                && (air - last_air).abs() <= STEADY
        });
        self.last = settled.then_some((now, die, air));
        settled && steady
    }
}
