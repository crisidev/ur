//! Battery voltage (SAADC on AIN7 / P0.31) and charger status lines.

/// A smoothed view of the battery.
pub struct Battery {
    /// Exponentially smoothed, because single samples jump by tens of mV
    /// with load (backlight, radio, motor).
    millivolts: Option<u32>,
    /// The charger IC is actively charging.
    pub charging: bool,
    /// USB/dock power is connected (charging or not).
    pub power_present: bool,
}

impl Battery {
    pub const fn new() -> Self {
        Self {
            millivolts: None,
            charging: false,
            power_present: false,
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

    pub fn status(&self) -> &'static str {
        match (self.power_present, self.charging) {
            (_, true) => "CHARGING",
            (true, false) => "CHARGED",
            (false, false) => "ON BATTERY",
        }
    }
}
