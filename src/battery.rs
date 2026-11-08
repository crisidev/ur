//! Battery voltage (SAADC on AIN7 / P0.31) and charger status lines, and
//! when the battery is so nearly empty that the watch should save what it
//! would lose and switch itself off (see [`crate::system_off`]).
//!
//! Hardware-free, so it's tested on the host (`host-tests/battery`).

// ==============================================================================
// Critically low
// ==============================================================================
//
// A LiPo cell's voltage at the PineTime's light load (well under 1 mA,
// C/200 or less) sits on a long plateau, ~3.7–3.9 V, then falls off a knee:
// at 3.5 V perhaps 5% is left, at 3.4 V 2–3%, and below ~3.3 V the voltage
// drops in hours to the cell protector's cut-off (typically 2.4–2.5 V,
// which also wears the cell). What gives out on the way down, in order:
//
// - The 3.3 V regulator stays in regulation to roughly 3.35–3.4 V; below
//   that the rail follows the battery down.
// - The SPI flash (where we save) needs 2.7 V to erase and program.
// - The nRF52832 itself runs down to 1.7 V, so it's not the limit: the
//   watch would keep running, half-working, with a flash it can't write.
//
// So `CRITICAL_MV`, 3.40 V, is where the curve starts to fall but well
// above anything failing: the 2–3% left (~4 mAh) runs System OFF for days
// to weeks, and the save itself takes a few seconds at a few mA. The
// reading is uncertain by tens of millivolts (the 1 MΩ divider's
// tolerance, the SAADC's reference, ~30 mV of sag under the screen and
// heart-rate LED), which this margin absorbs; a reading high by 100 mV
// shuts down at ~5% left, which costs little.
//
// [`Battery::percent`] isn't used for this: it's linear from 3.5 V (0%) to
// 4.2 V (100%), so it reads 0% while ~5% is left, overstates the middle
// of the plateau, and reads high while charging.

/// Below this, on battery, the battery is critically low; see above.
pub const CRITICAL_MV: u32 = 3400;

/// How long the (smoothed) voltage must stay below [`CRITICAL_MV`] before
/// it counts: a load spike (motor, backlight, heart-rate LED) mustn't switch
/// the watch off, and after a boot the smoothing starts from one sample.
pub const CRITICAL_FOR_SECS: u32 = 60;

/// Readings below this aren't a battery we could be running on: a broken
/// measurement, which must not switch the watch off.
const IMPLAUSIBLE_MV: u32 = 2500;

/// A smoothed view of the battery.
pub struct Battery {
    /// Exponentially smoothed, because single samples jump by tens of mV
    /// with load (backlight, radio, motor).
    millivolts: Option<u32>,
    /// The charger IC is actively charging.
    pub charging: bool,
    /// USB/dock power is connected (charging or not).
    pub power_present: bool,
    /// Since when (uptime seconds, at least 1) the voltage has been
    /// critically low, on battery, or 0 if it isn't; see
    /// [`Battery::critical`]. (Not an `Option`: that's 4 bytes more, of
    /// static RAM, in the UI task's future.)
    low_since: u32,
}

impl Battery {
    pub const fn new() -> Self {
        Self {
            millivolts: None,
            charging: false,
            power_present: false,
            low_since: 0,
        }
    }

    /// Folds in one 12-bit SAADC sample taken with the internal 0.6 V
    /// reference and 1/6 gain (0–3.6 V full scale at the pin). The board's
    /// divider halves the battery voltage before it reaches the pin.
    pub fn update(&mut self, raw_sample: i16, charging: bool, power_present: bool) {
        // Single-ended samples can dip slightly negative from noise.
        let sample_mv = u32::from(raw_sample.max(0).unsigned_abs()) * 3600 * 2 / 4096;
        self.millivolts = Some(match self.millivolts {
            None => sample_mv,
            // new = old + (sample - old) / 8, in integers.
            Some(old) => (old * 7 + sample_mv) / 8,
        });
        self.charging = charging;
        self.power_present = power_present;
    }

    /// A battery as a dev kit measured it, smoothing and all: for the
    /// simulator (`sim/`), which draws frames with a dev kit's readings to
    /// compare them (see `crate::compare`).
    #[cfg(not(target_os = "none"))]
    pub const fn measured(millivolts: u32, charging: bool, power_present: bool) -> Self {
        Self {
            millivolts: Some(millivolts),
            charging,
            power_present,
            low_since: 0,
        }
    }

    pub fn millivolts(&self) -> u32 {
        self.millivolts.unwrap_or(0)
    }

    /// A rough state-of-charge estimate.
    ///
    /// TODO: LiPo discharge isn't linear; use a lookup table. Also, the
    /// voltage reads high while charging, so this overstates charge then.
    pub fn percent(&self) -> u32 {
        const EMPTY_MV: u32 = 3500;
        const FULL_MV: u32 = 4200;
        (self.millivolts().clamp(EMPTY_MV, FULL_MV) - EMPTY_MV) * 100 / (FULL_MV - EMPTY_MV)
    }

    /// Whether the battery has been critically low (see [`CRITICAL_MV`]) for
    /// [`CRITICAL_FOR_SECS`], with no charger connected. Call after each
    /// [`Battery::update`], with the uptime in seconds.
    pub fn critical(&mut self, uptime_secs: u32) -> bool {
        let mv = self.millivolts();
        if self.power_present || !(IMPLAUSIBLE_MV..CRITICAL_MV).contains(&mv) {
            self.low_since = 0;
            return false;
        }
        if self.low_since == 0 {
            self.low_since = uptime_secs.max(1);
        }
        uptime_secs.saturating_sub(self.low_since) >= CRITICAL_FOR_SECS
    }

    pub fn status(&self) -> &'static str {
        match (self.power_present, self.charging) {
            (_, true) => "CHARGING",
            (true, false) => "CHARGED",
            (false, false) => "ON BATTERY",
        }
    }
}
