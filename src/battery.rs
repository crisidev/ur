//! Battery voltage (SAADC on AIN7 / P0.31) and charger status lines; the
//! charge left, as the percentage the watch shows and sends (see
//! [`Battery::percent`]); and when the battery is so nearly empty that the
//! watch should save what it would lose and switch itself off (see
//! [`crate::system_off`]).
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
// This goes by the smoothed voltage itself, not [`Battery::percent`]: the
// percentage deliberately lags (see below), and switching off must not.

/// Below this, on battery, the battery is critically low; see above.
pub const CRITICAL_MV: u32 = 3400;

/// How long the (smoothed) voltage must stay below [`CRITICAL_MV`] before
/// it counts: a load spike (motor, backlight, heart-rate LED) mustn't switch
/// the watch off, and after a boot the smoothing starts from one sample.
pub const CRITICAL_FOR_SECS: u32 = 60;

/// Readings below this aren't a battery we could be running on: a broken
/// measurement, which must not switch the watch off.
const IMPLAUSIBLE_MV: u32 = 2500;

// ==============================================================================
// Charge from voltage
// ==============================================================================
//
// At rest, a LiPo cell's voltage says how full it is, though not linearly:
// most of the charge is on the plateau, where 50 mV is a quarter of the
// cell. The curve is InfiniTime's for the PineTime
// (`src/components/battery/BatteryController.cpp`: 3.5 V 0%, 3.616 V 3%,
// 3.723 V 22%, 3.776 V 48%, 3.979 V 79%, 4.18 V 100%), checked against our
// own logs and changed at both ends:
//
// - Plateau and knee: the dev kit's run down to flat on 2026-11-10 (its
//   clock-face percentage, read off the fuzzer's screenshots, 3.78 V to
//   3.55 V over 7.3 hours at a steady load) falls in a straight line in
//   time on this curve, within ±2 points, and the line reaches 0% within
//   minutes of when the dev kit died.
// - Full: the sealed watch, after each of its three top-ups on 2026-11-09
//   and the dev kit after a full charge on 2026-11-10, settled at
//   4.14–4.15 V within the hour. That's this cell's (and load's) full at
//   rest, so 100% is at 4.15 V rather than InfiniTime's 4.18 V, which would
//   have a fully charged watch read 96%.
// - Empty: 0% is at `CRITICAL_MV`, where the watch saves and switches off,
//   rather than 3.5 V, where hours are left at the watch's usual load:
//   "0%" should mean it's about to.
//
// TODO: a run from full to `CRITICAL_MV` at the sealed watch's own load,
// with the 10-minute lines, would fit the top half (3.8–4.15 V) to this
// cell; it's InfiniTime's there.

/// Open-circuit voltage (mV) against charge (%), for the PineTime's cell at
/// its light load, piecewise linear between the points; see above.
const CHARGE_CURVE: [(u16, u8); 6] = [
    (CRITICAL_MV as u16, 0),
    (3616, 3),
    (3723, 22),
    (3776, 48),
    (3979, 79),
    (4150, 100),
];

/// The charge (%) a cell at rest at `mv` holds, by [`CHARGE_CURVE`].
/// Rounded down, so that it never overstates.
pub fn charge_percent(mv: u32) -> u8 {
    let mut below = CHARGE_CURVE[0];
    if mv <= u32::from(below.0) {
        return below.1;
    }
    for above in CHARGE_CURVE {
        let (mv0, mv1) = (u32::from(below.0), u32::from(above.0));
        if mv <= mv1 {
            let span = u32::from(above.1 - below.1);
            let up = (mv - mv0) * span / (mv1 - mv0);
            return below.1 + up as u8;
        }
        below = above;
    }
    100
}

// ==============================================================================
// What the watch shows
// ==============================================================================
//
// The voltage is only at rest on battery, and even then it sags under load
// (~30 mV under the screen and heart-rate LED: 15 points on the plateau).
// So the percentage the watch shows ([`Battery::percent`]) follows the
// curve only through a few rules, which keep it from overstating and from
// moving when nothing really changed:
//
// - **Charging, it doesn't overstate.** At the charger's constant current
//   the cell reads ~100 mV over its resting voltage (its ~1 Ω internal
//   resistance at ~93 mA): on 2026-11-09 the sealed watch went from
//   3.933 V on battery to 4.047 V and 4.133 V, 5 and 15 minutes into a
//   top-up, while it can only have taken in 8–23 mAh. Then, at constant
//   voltage, it reads 4.2 V whatever its charge. The linear 3.5–4.2 V
//   percentage this replaces read ~95% charging, then ~85% minutes off
//   the charger: the drop the wearer saw. So, charging, the percentage
//   only rises; at most to the curve at the voltage less
//   [`CHARGING_OFFSET_MV`]; at most a point every 70 seconds, as fast as
//   the charger can fill the cell (the schematic's 130 kΩ on the
//   SGM40561's IREF sets 11980 / 130 + 0.6 ≈ 93 mA by its datasheet, a
//   point every 68–70 s of a 175–180 mAh cell, less what the watch itself
//   draws); and once it reads at the charger's voltage (constant voltage,
//   where the current tapers and the voltage says nothing), a point every
//   two minutes, to 99%. 100% is for when the charger says it's done
//   (power, not charging). Each bound errs low (bar a charger weaker than
//   its nominal current): a top-up ended early can show a few points less
//   than it settles at, which the percentage then keeps until the battery
//   has run down to it.
// - **On battery, it never rises,** and it falls by a point only when
//   the curve has read below it for a whole [`FALL_STEP_SECS`] window,
//   at the window's best (least loaded) moment: a load's sag, or the
//   voltage relaxing for half an hour after the charger comes off (the
//   sealed watch: 4.170, 4.158, 4.142 V at 10-minute lines), passes
//   without a jump. At most a point per window, 20 points an hour, is as
//   fast as the heaviest real load drains the cell (the screen and LED at
//   ~35 mA together), so it keeps up with a heavy hour, and catches up
//   after one.
// - **After a boot** on battery, it shows the curve as it reads for the
//   first window, then settles at that window's best. (A boot just off
//   the charger reads a few points high for a while, falling a point a
//   window.) On the charger, it starts from the charging bound at once.
//
// [`Battery::critical`] doesn't go by any of this (see above).

/// How far (mV) the cell reads over its resting voltage while the charger
/// pushes its constant current in; measured from the sealed watch's top-up
/// on 2026-11-09 (see above). Over rather than under, so it understates.
const CHARGING_OFFSET_MV: u32 = 100;

/// At or over this (mV), charging, the charger is at (or nearly at) its
/// constant voltage, 4.2 V (4.152–4.248 V by its datasheet; our two watches
/// read 4.194–4.204 V), where the voltage stops saying how full the cell
/// is.
const TOPPING_UP_MV: u32 = 4150;

/// Charging, the percentage rises at most a point per this many seconds,
/// at constant current (as fast as the charger fills the cell), and when
/// topping up, at constant voltage (where its current tapers off).
const CHARGING_STEP_SECS: u32 = 70;
const TOPPING_UP_STEP_SECS: u32 = 120;

/// On battery, the percentage falls at most a point per this many seconds,
/// and only when the curve read lower for all of them.
const FALL_STEP_SECS: u32 = 180;

/// The steps' clock ticks this often, so that a u16 of it lasts a week
/// before it wraps (and then only delays a step).
const TICK_SECS: u32 = 10;

/// The percentage shown before the first window after a boot.
const NOT_YET: u8 = u8::MAX;

/// A smoothed view of the battery.
///
/// Its fields are small, as it lives in the UI task's future, in static
/// RAM.
pub struct Battery {
    /// Eight times the voltage (mV), exponentially smoothed, because single
    /// samples jump by tens of mV with load (backlight, radio, motor); 0
    /// before the first sample. Eight times, so that the smoothing's
    /// rounding doesn't leave it up to 7 mV low after the voltage rises
    /// (as it did, kept in whole mV), and under 8 × 7.2 V, so it fits.
    eighths: u16,
    /// The charger IC is actively charging.
    pub charging: bool,
    /// USB/dock power is connected (charging or not).
    pub power_present: bool,
    /// The percentage shown (see above), or [`NOT_YET`].
    shown: u8,
    /// On battery, the most the curve has read since `step_tick`.
    best: u8,
    /// The uptime, in [`TICK_SECS`] (wrapping), of the last step of the
    /// percentage, or of the last change of charger state, from which the
    /// next step is due.
    step_tick: u16,
    /// Since when (uptime seconds, at least 1) the voltage has been
    /// critically low, on battery, or 0 if it isn't; see
    /// [`Battery::critical`]. (Not an `Option`: that's 4 bytes more, of
    /// static RAM, in the UI task's future.)
    low_since: u32,
}

impl Battery {
    pub const fn new() -> Self {
        Self {
            eighths: 0,
            charging: false,
            power_present: false,
            shown: NOT_YET,
            best: 0,
            step_tick: 0,
            low_since: 0,
        }
    }

    /// Folds in one 12-bit SAADC sample taken with the internal 0.6 V
    /// reference and 1/6 gain (0–3.6 V full scale at the pin), with the
    /// charger lines, at `uptime_secs`. The board's divider halves the
    /// battery voltage before it reaches the pin.
    pub fn update(
        &mut self,
        raw_sample: i16,
        charging: bool,
        power_present: bool,
        uptime_secs: u32,
    ) {
        // What was shown, and on what power, to check the step below.
        let shown_before = self.shown;
        // Single-ended samples can dip slightly negative from noise.
        let sample_mv = u32::from(raw_sample.max(0).unsigned_abs()) * 3600 * 2 / 4096;
        let old = u32::from(self.eighths);
        // new = old + (sample - old) / 8, in eighths.
        self.eighths = if old == 0 {
            sample_mv * 8
        } else {
            old - old / 8 + sample_mv
        } as u16;

        // A step is due a whole step after the charger last changed: a
        // step up after plugging in, a window after unplugging, so that the
        // first window on battery doesn't include the charging voltage.
        let tick = (uptime_secs / TICK_SECS) as u16;
        if (charging, power_present) != (self.charging, self.power_present) {
            self.charging = charging;
            self.power_present = power_present;
            self.step_tick = tick;
            self.best = 0;
        }
        let since_step = u32::from(tick.wrapping_sub(self.step_tick)) * TICK_SECS;
        if charging || power_present {
            let (cap, step) = self.charging_cap();
            if self.shown == NOT_YET {
                // Boot on the charger: start from the honest bound.
                self.shown = cap.min(self.curve_on_power());
                self.step_tick = tick;
            } else if self.shown < cap && since_step >= step {
                self.shown += 1;
                self.step_tick = tick;
            }
        } else {
            self.best = self.best.max(charge_percent(self.millivolts()));
            if since_step >= FALL_STEP_SECS {
                if self.shown == NOT_YET {
                    self.shown = self.best;
                } else if self.best < self.shown {
                    self.shown -= 1;
                }
                self.best = 0;
                self.step_tick = tick;
            }
        }
        // The rules above, as the wearer sees them: a percentage is one,
        // and once there is one, an update moves it by a point at most,
        // and never the wrong way: down on the charger (the drop this
        // replaced), or up on battery. (`measured` can be told any
        // percentage, for the simulator; from over 100 it only comes down.)
        debug_assert!(
            self.shown == NOT_YET
                || self.shown <= 100
                || (shown_before != NOT_YET && self.shown <= shown_before),
            "{} % after {shown_before} %",
            self.shown
        );
        debug_assert!(
            shown_before == NOT_YET
                || if charging || power_present {
                    self.shown == shown_before || self.shown == shown_before + 1
                } else {
                    self.shown == shown_before || self.shown + 1 == shown_before
                },
            "{} % after {shown_before} %, charging {charging}, power {power_present}",
            self.shown
        );
    }

    /// Charging (or charged), how high the percentage may go, and how many
    /// seconds a point up takes; see above.
    fn charging_cap(&self) -> (u8, u32) {
        if !self.charging {
            (100, CHARGING_STEP_SECS)
        } else if self.millivolts() >= TOPPING_UP_MV {
            (99, TOPPING_UP_STEP_SECS)
        } else {
            (self.curve_on_power().min(99), CHARGING_STEP_SECS)
        }
    }

    /// The curve at the voltage less the charging current's share of it,
    /// on the charger.
    fn curve_on_power(&self) -> u8 {
        let offset = if self.charging { CHARGING_OFFSET_MV } else { 0 };
        charge_percent(self.millivolts().saturating_sub(offset))
    }

    /// A battery as a dev kit measured it, smoothing and all: for the
    /// simulator (`sim/`), which draws frames with a dev kit's readings to
    /// compare them (see `crate::compare`).
    #[cfg(not(target_os = "none"))]
    pub const fn measured(
        millivolts: u32,
        percent: u8,
        charging: bool,
        power_present: bool,
    ) -> Self {
        Self {
            eighths: millivolts as u16 * 8,
            charging,
            power_present,
            shown: percent,
            best: 0,
            step_tick: 0,
            low_since: 0,
        }
    }

    pub fn millivolts(&self) -> u32 {
        u32::from(self.eighths) / 8
    }

    /// The charge left, as the watch shows it and sends it to the phone:
    /// from the voltage by [`CHARGE_CURVE`], kept from overstating while
    /// charging and from jumping after it (see above).
    pub fn percent(&self) -> u32 {
        // (On the charger, `update` sets it at once.)
        u32::from(match self.shown {
            NOT_YET => charge_percent(self.millivolts()),
            shown => shown,
        })
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
