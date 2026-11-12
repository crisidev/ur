//! When the sun is up: sunrise and the day's length, from a date, a
//! location and the time zone.
//!
//! [`crate::geo`] asks this with the location the phone last sent and the
//! clock's date and offset; whether it's day decides the side of the
//! colour schemes (unless FORCE pins one) and the backlight's level
//! (`brightness`). Hardware-free, so that it runs in host tests
//! (`host-tests/sun`).

use libm::{acosf, sqrtf};

const DAY_MINUTES: u16 = 24 * 60;

/// When the sun is up during a local day: from `sunrise` (a minute of the
/// day) for `daylight` minutes, round past midnight if need be.
///
/// Not sunrise and sunset as two times of day: where the sun sets after
/// local midnight (Tromsø's nights around the midnight sun; anywhere while
/// the phone's time zone is hours from where the watch last had a fix),
/// the sunset's time of day comes before the sunrise's, and a day just
/// short of 24 hours can put both in the same minute, as polar night
/// does. The length tells them apart: 0 is polar
/// night, 1440 polar day, and anything between wraps as it falls.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[cfg_attr(target_os = "none", derive(defmt::Format))]
pub struct Sun {
    /// The minute after local midnight in which the sun rises: `0..1440`.
    pub sunrise: u16,
    /// How many minutes, from `sunrise`'s on, are day: `0..=1440`.
    pub daylight: u16,
}

impl Sun {
    pub const POLAR_NIGHT: Self = Self {
        sunrise: 0,
        daylight: 0,
    };
    pub const POLAR_DAY: Self = Self {
        sunrise: 0,
        daylight: DAY_MINUTES,
    };

    /// From when the sun rises, in minutes after local midnight (`0..1440`),
    /// and how long it stays up, in minutes (`0..=1440`).
    ///
    /// A minute is day if the sun is up at its end: so the minute it rises
    /// in is day, and the one it sets in is night, as a clock showing
    /// 22:43 has the sun set by the time it shows 22:44. That's
    /// `sunrise <= minute < sunset` for the whole minutes the two fall in,
    /// counted on from the sunrise's: `floor(rise + length)` is the
    /// sunset's minute, unwrapped, so its distance from the sunrise's is
    /// the count of day minutes, up to 1440 (a day of 1439.9 minutes can
    /// have no minute's end in its night). `host-tests/sun` proves this
    /// for every `f32` the two can be, not only in real numbers.
    pub fn from_minutes(rise: f32, length: f32) -> Self {
        // `as` saturates (and makes a NaN 0), so even a garbage location
        // can't make a minute out of range.
        let sunrise = (rise as u16).min(DAY_MINUTES - 1);
        let set = (rise + length) as u16;
        Self {
            sunrise,
            daylight: set.saturating_sub(sunrise).min(DAY_MINUTES),
        }
    }

    /// Whether the sun is up in `minute_of_day` (`0..1440`).
    ///
    /// This takes the same day's sunrise and sunset to bound the night
    /// before and the night after alike: right after midnight, with the
    /// sun still up from the day before, it stands in for the day before's
    /// sunset. That's off by as much as the sunset moves in a day: a
    /// minute or two at mid latitudes, a few at Reykjavik in May, but
    /// ~17 minutes at Tromsø in the week the midnight sun starts or ends
    /// (16 May sets at 00:09, 15 May at 23:52, so 00:00–00:09 on 16 May
    /// counts as day).
    ///
    /// TODO: the day before's own sun for the minutes before sunrise, if
    /// those minutes matter to anyone: it's another `compute` a frame, and
    /// a `Sun` that keeps two sunsets.
    pub fn contains(&self, minute_of_day: u16) -> bool {
        debug_assert!(self.sunrise < DAY_MINUTES && self.daylight <= DAY_MINUTES);
        debug_assert!(minute_of_day < DAY_MINUTES);
        // How long after sunrise the minute is, round the clock.
        (minute_of_day + DAY_MINUTES - self.sunrise) % DAY_MINUTES < self.daylight
    }
}

/// The sun's day at a place, apart from the time zone (see [`passage`]).
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Sky {
    /// It doesn't rise all day.
    PolarNight,
    /// It doesn't set.
    PolarDay,
    /// It rises and sets: `transit` (local solar noon) is in days after
    /// the J2000 epoch's noon on the day asked about, and the sun is up
    /// for `hour_angle` degrees (of the sky's 360° a day) either side.
    Passage { transit: f32, hour_angle: f32 },
}

/// Today's sun: `days_since_epoch` is the local date (days since
/// 1970-01-01), latitude and longitude (east positive) are in degrees, and
/// the offset is the local time zone's from UTC.
pub fn compute(days_since_epoch: i64, latitude: f32, longitude: f32, offset_minutes: i16) -> Sun {
    match passage(days_since_epoch, latitude, longitude) {
        Sky::PolarNight => Sun::POLAR_NIGHT,
        Sky::PolarDay => Sun::POLAR_DAY,
        Sky::Passage {
            transit,
            hour_angle,
        } => {
            // A Julian date's days start at noon, and J2000's whole days
            // are whole days after the Unix epoch's (2_451_545 -
            // 2_440_587.5 = 10_957.5), so only the half day and the parts
            // are left of the time of day. In seconds, as an `f32`: a
            // minute's resolution would round a few minutes in `wrap`'s
            // turns.
            let rise = transit - hour_angle / 360.0;
            let seconds = (0.5 + rise) * 86_400.0 + f32::from(offset_minutes) * 60.0;
            // A sky's turn takes a day: 4 minutes a degree, each way.
            Sun::from_minutes(wrap(seconds, 86_400.0) / 60.0, 8.0 * hour_angle)
        }
    }
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
pub fn passage(days_since_epoch: i64, latitude: f32, longitude: f32) -> Sky {
    // Days since the J2000 epoch, at local solar noon of today's date:
    // the equation's `ceil(julian_noon - 2_451_545 + 0.0008)`, where
    // `julian_noon` is the date's whole Julian day number,
    // `days_since_epoch + 2_440_588`. Then the mean solar time, `n` days
    // plus the part of one that the longitude takes.
    let n = days_since_epoch - 10_956;
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
        return Sky::PolarNight;
    }
    if cos_hour_angle < -1.0 {
        return Sky::PolarDay;
    }
    // Within 0°..=180° (or a hair over, as `f32`s round: `from_minutes`
    // caps the day at 1440 minutes), or a NaN for a location no phone
    // sends, which comes out as polar night.
    Sky::Passage {
        transit,
        hour_angle: acosf(cos_hour_angle).to_degrees(),
    }
}

/// `x` modulo `m`, in `0..m`, for `x` within a few `m` of it (`fmodf`'s
/// generality costs flash).
fn wrap(mut x: f32, m: f32) -> f32 {
    // "Within a few": each loop takes a turn per `m`, and a float far
    // enough out doesn't change at all when `m` is added to it, so the
    // loop would never end (and nothing else would run: the watchdog
    // resets the watch, and the next frame does it again). `compute`
    // stays within ~31 `m` even for a location and offset no phone sends
    // (any `i32`s from flash, any `i16`). A NaN ends both loops at once.
    debug_assert!(x.is_nan() || (-64.0 * m..=64.0 * m).contains(&x));
    while x < 0.0 {
        x += m;
    }
    while x >= m {
        x -= m;
    }
    x
}

/// The sine of an angle in degrees, within a few turns of zero, to an
/// `f32`'s precision; [`passage`] says why not `libm`'s.
pub fn sin_deg(degrees: f32) -> f32 {
    // "A few turns": what keeps the steps below exact. `passage` asks for
    // under three, or up to seven of a latitude that isn't one (any `i32`
    // of millionths, from flash).
    debug_assert!(degrees.is_nan() || (-8.0 * 360.0..=8.0 * 360.0).contains(&degrees));
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
