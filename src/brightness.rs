//! How bright the backlight is.
//!
//! The backlight has three drivers, low, mid, and high, each a transistor
//! with its own resistor. Their currents sum, so any combination of them is
//! a level: seven in all, from the PINE64 wiki's measurements of each
//! alone (low 1.83 mA, mid 5.51 mA, high 12.27 mA) up to all three
//! (~19.6 mA, InfiniTime's "high"). How bright it should be:
//!
//! - **The dim look** (a glance, a notification on a dark screen, the quiet
//!   night): low. White on black reads fine at low, and the eyes looking
//!   are usually dark-adapted.
//! - **[`Setting::Auto`]**, otherwise:
//!   - **While the sun is up**: all three, to read in sunlight.
//!   - **In the small hours** (see [`small_hours`]: from local midnight
//!     until the wearer gets up): low, as for the dim look.
//!   - **Otherwise** (from sunset to midnight, and from getting up to
//!     sunrise): mid.
//!
//!   The sun comes first, so sunset after midnight (a high-latitude
//!   summer) is bright until sunset and low after; getting up after
//!   sunrise is bright. With no sunrise (polar night) it's mid from getting
//!   up until midnight; never sleeping, low from midnight until sunrise.
//! - **[`Setting::Soft`]**, for those who find that too much (or want the
//!   battery): one step down, mid while the sun is up and low otherwise.
//!   All three by day costs ~14 mA more than mid for as long as the screen
//!   is on.
//! - **A ringing wake alarm** starts at low and brightens over [`RAMP`],
//!   so it doesn't blind someone just waking up, to the normal level or
//!   low and mid together, whichever is brighter (before sunrise, a little
//!   brighter than mid, to help wake). The alarm counts as getting up, so
//!   the small hours are over.
//!
//! Like `sleep`, this is free of hardware, so it runs in host tests
//! (`host-tests/brightness`).

/// A combination of the three drivers.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Level(u8);

impl Level {
    pub const OFF: Self = Self(0);
    pub const LOW: Self = Self(0b001);
    pub const MID: Self = Self(0b010);
    pub const HIGH: Self = Self(0b100);
    pub const ALL: Self = Self(0b111);

    const fn and(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Whether each driver is on: low, mid, high.
    pub fn drivers(self) -> [bool; 3] {
        [self.0 & 1 != 0, self.0 & 2 != 0, self.0 & 4 != 0]
    }

    /// Where it falls in [`BY_CURRENT`] (`OFF` counts as the dimmest).
    fn rank(self) -> usize {
        BY_CURRENT.iter().position(|&l| l == self).unwrap_or(0)
    }
}

/// Every level that's on, from the least current to the most (1.8, 5.5,
/// 7.3, 12.3, 14.1, 17.8, 19.6 mA), which is also from dimmest to
/// brightest.
pub const BY_CURRENT: [Level; 7] = [
    Level::LOW,
    Level::MID,
    Level::LOW.and(Level::MID),
    Level::HIGH,
    Level::LOW.and(Level::HIGH),
    Level::MID.and(Level::HIGH),
    Level::ALL,
];

/// The brightness setting, on the Info screen.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum Setting {
    #[default]
    Auto,
    Soft,
}

impl Setting {
    /// As stored (`storage::key::BRIGHTNESS`). Unknown values (none yet)
    /// read as the default.
    pub fn from_byte(byte: u8) -> Self {
        match byte {
            1 => Self::Soft,
            _ => Self::Auto,
        }
    }

    pub fn to_byte(self) -> u8 {
        self as u8
    }

    /// The next choice, round the cycle (a tap on the Info screen).
    pub fn next(self) -> Self {
        match self {
            Self::Auto => Self::Soft,
            Self::Soft => Self::Auto,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Auto => "AUTO",
            Self::Soft => "SOFT",
        }
    }
}

/// How long a ringing wake alarm takes to brighten fully, in milliseconds:
/// about as long as the vibration takes to build up.
pub const RAMP: u64 = 60_000;

/// Whether it's the small hours: local midnight has passed, and the wearer
/// hasn't got up since. Call once a second with the value so far, the
/// local time (seconds since 1970, as in `crate::alarm`) at the last call
/// and now, if known, and whether the wearer `got_up` just now (the quiet
/// night ended, or a wake alarm rang).
///
/// Local noon ends the small hours too, for someone who never slept (or
/// whose night went unnoticed): otherwise the evening after would start
/// low.
///
/// One bool of state, kept by `ui::App`; after a reset it starts out false
/// (mid, not low, until the next midnight).
pub fn small_hours(current: bool, times: Option<(i64, i64)>, got_up: bool) -> bool {
    const DAY: i64 = 24 * 60 * 60;
    if got_up {
        return false;
    }
    let Some((before, now)) = times else {
        return current;
    };
    // The latest midnight and noon at or before `now`; whichever came last
    // since `before` decides (both can pass at once if the clock jumps).
    let midnight = now.div_euclid(DAY) * DAY;
    let noon = (now - DAY / 2).div_euclid(DAY) * DAY + DAY / 2;
    match (before < midnight, before < noon) {
        (true, true) => midnight > noon,
        (true, false) => true,
        (false, true) => false,
        (false, false) => current,
    }
}

/// The level for the screen as it is: `daytime` while the sun is up,
/// `small_hours` as above, `dim` while it shows the dim look, and `waking`
/// with how long (in milliseconds) a wake alarm (not a timer) has been
/// ringing, if one is.
pub fn level(
    setting: Setting,
    daytime: bool,
    small_hours: bool,
    dim: bool,
    waking: Option<u64>,
) -> Level {
    if dim {
        return Level::LOW;
    }
    let normal = match (setting, daytime, small_hours) {
        (Setting::Auto, true, _) => Level::ALL,
        (Setting::Auto, false, true) => Level::LOW,
        (Setting::Auto, false, false) => Level::MID,
        (Setting::Soft, true, _) => Level::MID,
        (Setting::Soft, false, _) => Level::LOW,
    };
    let Some(elapsed) = waking else {
        return normal;
    };
    // Up through every level from low to the target, an equal share of the
    // ramp at each. The steps are coarse (three before sunrise: low, mid,
    // low and mid; all seven after), but even from one to the next the
    // current, and roughly the brightness, at most triples.
    //
    // TODO: a smooth ramp would need PWM on a driver (InfiniTime does it
    // with an RTC and PPI); not worth it unless the steps are a bother.
    let top = normal.rank().max(Level::LOW.and(Level::MID).rank());
    let steps = top as u64 + 1;
    let step = (elapsed.saturating_mul(steps) / RAMP).min(top as u64);
    BY_CURRENT[step as usize]
}
