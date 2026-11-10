//! Host tests for the firmware's `battery` module: when a battery counts as
//! critically low, and the percentage the watch shows, against a model
//! cell and against the sealed watch's field log. `cargo test` runs them.

#[path = "../../../src/battery.rs"]
#[allow(dead_code)]
mod battery;

fn main() {
    println!("run `cargo test`");
}

#[cfg(test)]
mod tests {
    use super::battery::*;

    /// The raw SAADC sample that reads as `mv` at the battery (the inverse
    /// of `Battery::update`'s scaling, as `debug::battery_sample` does).
    fn raw(mv: u32) -> i16 {
        ((mv * 4096 + 3600) / 7200) as i16
    }

    /// A battery that has read `mv` for long enough to have settled.
    fn settled(mv: u32, power_present: bool) -> Battery {
        let mut battery = Battery::new();
        for second in 0..200 {
            battery.update(raw(mv), false, power_present, second);
        }
        battery
    }

    /// Feeds one reading a second for `seconds`, from uptime `start`, and
    /// returns the first second at which the battery was critical.
    fn first_critical(battery: &mut Battery, mv: u32, start: u32, seconds: u32) -> Option<u32> {
        (start..start + seconds).find(|&second| {
            battery.update(raw(mv), false, battery.power_present, second);
            battery.critical(second)
        })
    }

    #[test]
    fn a_healthy_battery_is_never_critical() {
        let mut battery = settled(3700, false);
        assert_eq!(first_critical(&mut battery, 3700, 100, 3600), None);
    }

    #[test]
    fn a_low_battery_is_critical_after_a_minute() {
        let mut battery = settled(3350, false);
        let at = first_critical(&mut battery, 3350, 100, 600).expect("critical");
        assert_eq!(at, 100 + CRITICAL_FOR_SECS);
    }

    #[test]
    fn critical_from_the_first_second_of_uptime() {
        // Second 0 is the field's "not low", so going low then counts from
        // second 1: a second late, but not forever.
        let mut battery = settled(3350, false);
        let at = first_critical(&mut battery, 3350, 0, 600).expect("critical");
        assert_eq!(at, 1 + CRITICAL_FOR_SECS);
    }

    #[test]
    fn the_charger_stops_the_countdown() {
        let mut battery = settled(3350, true);
        assert_eq!(first_critical(&mut battery, 3350, 100, 600), None);
    }

    #[test]
    fn a_brief_dip_doesnt_count() {
        // Low for half a minute, then back up (a load spike passing): the
        // countdown starts over.
        let mut battery = settled(3350, false);
        assert_eq!(first_critical(&mut battery, 3350, 100, 30), None);
        // The smoothed voltage climbs back over the threshold within
        // seconds.
        assert_eq!(first_critical(&mut battery, 3600, 130, 60), None);
        // Low again: a whole minute from when the smoothed voltage drops
        // below, a few seconds from here.
        let at = first_critical(&mut battery, 3350, 190, 600).expect("critical");
        assert!((190 + CRITICAL_FOR_SECS..190 + CRITICAL_FOR_SECS + 30).contains(&at));
    }

    #[test]
    fn a_broken_measurement_is_not_critical() {
        // No battery reading at all, or one far below anything that could
        // run the watch.
        let mut battery = Battery::new();
        assert!(!battery.critical(1000));
        let mut battery = settled(0, false);
        assert_eq!(first_critical(&mut battery, 0, 100, 600), None);
        let mut battery = settled(1800, false);
        assert_eq!(first_critical(&mut battery, 1800, 100, 600), None);
    }

    #[test]
    fn the_threshold_is_zero_percent() {
        // The percentage reaches 0% where the watch switches off, and not
        // before: at 3.5 V an hour or so is left.
        let battery = settled(CRITICAL_MV, false);
        assert_eq!(battery.percent(), 0);
        let battery = settled(3500, false);
        assert!(battery.percent() > 0);
    }

    // ==========================================================================
    // The curve
    // ==========================================================================

    #[test]
    fn the_curve_rises_with_the_voltage() {
        let mut last = 0;
        for mv in 0..5000 {
            let percent = charge_percent(mv);
            assert!(percent >= last, "{mv} mV: {percent}% after {last}%");
            assert!(percent <= 100);
            last = percent;
        }
        assert_eq!(charge_percent(0), 0);
        assert_eq!(charge_percent(CRITICAL_MV), 0);
        assert_eq!(charge_percent(4150), 100);
        assert_eq!(charge_percent(4300), 100);
    }

    #[test]
    fn the_curve_puts_most_of_the_charge_on_the_plateau() {
        // Where the linear 3.5–4.2 V percentage it replaced overstated it.
        assert_eq!(charge_percent(3723), 22);
        assert_eq!(charge_percent(3776), 48);
        assert_eq!(charge_percent(3979), 79);
        assert!(charge_percent(3750) < 40);
    }

    // ==========================================================================
    // A model cell
    // ==========================================================================
    //
    // Open-circuit voltage by the curve (so "the truth" is the curve at it),
    // 0.75 Ω of resistance at once and a 30 mV-per-100 mA polarisation that
    // relaxes over ~15 minutes (as the sealed watch's voltage did after its
    // top-ups): ~90 mV over its resting voltage 15 minutes into a charge,
    // as the sealed watch read. 175 mAh, and the SGM40561's ~93 mA, then
    // 4.2 V.

    struct Cell {
        /// Charge, in mA·s.
        charge: f64,
        /// The relaxing part of the voltage over the resting one, in mV.
        polarisation: f64,
        /// Into the cell (charging) or out of it (a load), in mA.
        current: f64,
        charging: bool,
        power_present: bool,
    }

    const CAPACITY: f64 = 175.0 * 3600.0;

    impl Cell {
        fn at(percent: f64) -> Self {
            Self {
                charge: percent / 100.0 * CAPACITY,
                polarisation: 0.0,
                current: 0.0,
                charging: false,
                power_present: false,
            }
        }

        fn percent(&self) -> f64 {
            self.charge / CAPACITY * 100.0
        }

        /// The resting voltage: the curve, inverted.
        fn rest_mv(&self) -> f64 {
            let percent = self.percent();
            let (mut lo, mut hi) = (CRITICAL_MV as f64, 4150.0);
            if percent >= 100.0 {
                return hi;
            }
            for _ in 0..40 {
                let mid = (lo + hi) / 2.0;
                if f64::from(charge_percent(mid as u32)) <= percent {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            lo
        }

        fn mv(&self) -> f64 {
            self.rest_mv() + self.current * 0.75 + self.polarisation
        }

        /// A second, with `load` mA drawn by the watch: from the battery,
        /// or, charging, from what the charger puts in (it has no power
        /// path).
        fn second(&mut self, load: f64) {
            self.current = if self.charging {
                // Constant current, until that would take it over 4.2 V.
                let headroom = 4200.0 - self.rest_mv() - self.polarisation;
                93.0_f64.min(headroom / 0.75).max(0.0) - load
            } else {
                -load
            };
            self.charge = (self.charge + self.current).min(CAPACITY);
            let target = self.current * 0.3;
            self.polarisation += (target - self.polarisation) / 900.0;
            if self.charging && self.charge >= CAPACITY {
                // (The model's resting voltage stops at the curve's 100%,
                // 4.15 V, so its current doesn't taper to the charger's
                // full-of-charge 3.7 mA: it stops when full.)
                self.charging = false;
            }
        }

        fn plug(&mut self, plugged: bool) {
            self.charging = plugged;
            self.power_present = plugged;
        }
    }

    /// Runs `cell` and `battery` for `seconds` from `start`, with `load`,
    /// and returns the shown percentage each second, with the truth.
    fn run(
        cell: &mut Cell,
        battery: &mut Battery,
        start: u32,
        seconds: u32,
        load: impl Fn(u32) -> f64,
    ) -> Vec<(u32, u32, f64)> {
        (start..start + seconds)
            .map(|second| {
                cell.second(load(second));
                battery.update(
                    raw(cell.mv().round() as u32),
                    cell.charging,
                    cell.power_present,
                    second,
                );
                (second, battery.percent(), cell.percent())
            })
            .collect()
    }

    /// A light load: ~1 mA, the heart-rate LED for 30 s every 5 minutes
    /// (~15 mA), the screen for 15 s every 7 minutes (~25 mA).
    fn worn(second: u32) -> f64 {
        let mut load = 1.0;
        if second % 300 < 30 {
            load += 15.0;
        }
        if second % 420 < 15 {
            load += 25.0;
        }
        load
    }

    /// Asserts the percentage never moved by more than a point at a time,
    /// nor faster than `per_minute` points a minute.
    fn assert_steady(trace: &[(u32, u32, f64)], per_minute: f64) {
        for pair in trace.windows(2) {
            let (_, a, _) = pair[0];
            let (second, b, _) = pair[1];
            assert!(a.abs_diff(b) <= 1, "at {second} s: {a}% to {b}%");
        }
        let (first, last) = (trace[0], trace[trace.len() - 1]);
        let minutes = f64::from(last.0 - first.0) / 60.0;
        let moved = f64::from(first.1.abs_diff(last.1));
        assert!(
            moved <= per_minute * minutes + 1.0,
            "{moved} points in {minutes} min"
        );
    }

    #[test]
    fn charging_never_overstates() {
        // From a few points up the knee to full, unplugged at each point
        // along the way: the percentage never leads the charge by more
        // than a point (the cell can't fill faster than its charger), and
        // never drops by more than a point once off the charger, as it
        // relaxes.
        for unplug_minute in (5..90).step_by(5) {
            let mut cell = Cell::at(30.0);
            let mut battery = Battery::new();
            let before = run(&mut cell, &mut battery, 0, 600, worn);
            assert!(before.last().expect("ran").1 <= 31);
            cell.plug(true);
            let charging = run(&mut cell, &mut battery, 600, unplug_minute * 60, worn);
            for &(second, shown, truth) in &charging {
                assert!(
                    f64::from(shown) <= truth + 1.0,
                    "at {second} s: {shown}% charging, holding {truth:.1}%"
                );
            }
            assert_steady(&charging, 1.0);
            let at_unplug = charging.last().expect("ran").1;
            cell.plug(false);
            let off = run(
                &mut cell,
                &mut battery,
                600 + unplug_minute * 60,
                3600,
                worn,
            );
            // (It does fall as the cell drains: by the hour's ~2 points.)
            for &(second, shown, truth) in &off {
                assert!(
                    shown <= at_unplug,
                    "at {second} s: up to {shown}% from {at_unplug}%"
                );
                let floor = f64::from(at_unplug).min(truth) - 2.0;
                assert!(
                    f64::from(shown) >= floor,
                    "at {second} s: down to {shown}% from {at_unplug}% at {truth:.1}%, unplugged after {unplug_minute} min"
                );
            }
        }
    }

    #[test]
    fn a_full_charge_reads_100_percent() {
        let mut cell = Cell::at(50.0);
        let mut battery = Battery::new();
        run(&mut cell, &mut battery, 0, 600, worn);
        cell.plug(true);
        let charging = run(&mut cell, &mut battery, 600, 3 * 3600, worn);
        assert!(!cell.charging, "the charger finishes");
        assert_eq!(charging.last().expect("ran").1, 100);
        // Unplugged, it follows the cell down, without a jump.
        cell.power_present = false;
        let off = run(&mut cell, &mut battery, 600 + 3 * 3600, 3600, worn);
        for &(second, shown, truth) in &off {
            assert!(
                f64::from(shown) >= truth - 2.0,
                "at {second} s: {shown}% at {truth:.1}%"
            );
        }
        assert_steady(&off, 1.0 / 3.0);
    }

    #[test]
    fn a_boot_on_the_charger_starts_honest() {
        let mut cell = Cell::at(60.0);
        cell.plug(true);
        // Charging for a while before the boot, at its full current.
        run(&mut cell, &mut Battery::new(), 0, 300, |_| 1.0);
        let mut battery = Battery::new();
        let trace = run(&mut cell, &mut battery, 0, 10, |_| 1.0);
        let (_, shown, truth) = trace[9];
        // Under it, if anything: early in a charge the cell reads less than
        // `CHARGING_OFFSET_MV` over its resting voltage.
        assert!(
            f64::from(shown) <= truth + 1.0 && f64::from(shown) >= truth - 6.0,
            "{shown}% at {truth:.1}%"
        );
    }

    #[test]
    fn worn_on_battery_it_follows_the_charge() {
        // A day on the plateau, from 60%, at an ordinary load: the load's
        // sag (tens of mV, many points here) doesn't ratchet it down.
        let mut cell = Cell::at(60.0);
        let mut battery = Battery::new();
        let trace = run(&mut cell, &mut battery, 0, 24 * 3600, worn);
        for &(second, shown, truth) in &trace[600..] {
            let error = f64::from(shown) - truth;
            assert!(
                (-3.0..=1.0).contains(&error),
                "at {second} s: {shown}% at {truth:.1}%"
            );
        }
        assert_steady(&trace[300..], 1.0 / 3.0);
    }

    #[test]
    fn a_heavy_hour_shows() {
        // An hour at 30 mA (the screen on and the LED, say), then worn:
        // the percentage falls with it, if anything a little further.
        let mut cell = Cell::at(70.0);
        let mut battery = Battery::new();
        run(&mut cell, &mut battery, 0, 600, worn);
        let heavy = run(&mut cell, &mut battery, 600, 3600, |_| 30.0);
        let after = run(&mut cell, &mut battery, 4200, 3600, worn);
        let (_, shown, truth) = *after.last().expect("ran");
        assert!(truth < 55.0, "the hour drained it: {truth:.1}%");
        assert!(
            f64::from(shown) <= truth + 1.0 && f64::from(shown) >= truth - 8.0,
            "{shown}% at {truth:.1}%"
        );
        assert_steady(&heavy, 1.0 / 3.0);
    }

    #[test]
    fn a_boot_under_load_settles_at_the_best_reading() {
        // The first window shows the curve as it reads; then the window's
        // least loaded moment, not its first.
        let mut cell = Cell::at(40.0);
        let mut battery = Battery::new();
        let trace = run(&mut cell, &mut battery, 0, 600, |second| {
            if second < 30 { 30.0 } else { 1.0 }
        });
        let (_, shown, truth) = *trace.last().expect("ran");
        assert!(f64::from(shown) >= truth - 1.5, "{shown}% at {truth:.1}%");
    }

    // ==========================================================================
    // The sealed watch's field log
    // ==========================================================================

    /// Its 10-minute lines (local time in minutes, mV, charging) around
    /// the 2026-11-09 top-ups (`tmp/hc-1110/timeline.tsv`), with where
    /// they don't say: plugged in at 09:25 (between 09:20:36 and
    /// 09:30:36) and unplugged at 10:05; and at 18:05 and 18:27.
    const MORNING: &[(u32, u32, bool)] = &[
        (9 * 60, 3934, false),
        (9 * 60 + 10, 3935, false),
        (9 * 60 + 20, 3933, false),
        (9 * 60 + 25, 3933, false),
        (9 * 60 + 25, 4004, true),
        (9 * 60 + 30, 4047, true),
        (9 * 60 + 40, 4133, true),
        (9 * 60 + 50, 4194, true),
        (10 * 60, 4204, true),
        (10 * 60 + 5, 4204, true),
        (10 * 60 + 5, 4185, false),
        (10 * 60 + 10, 4170, false),
        (10 * 60 + 20, 4158, false),
        (10 * 60 + 40, 4152, false),
        (11 * 60, 4144, false),
        (11 * 60 + 30, 4138, false),
        (12 * 60, 4134, false),
    ];
    const EVENING: &[(u32, u32, bool)] = &[
        (17 * 60 + 42, 4121, false),
        (17 * 60 + 52, 4119, false),
        (18 * 60 + 2, 4124, false),
        (18 * 60 + 5, 4124, false),
        (18 * 60 + 5, 4200, true),
        (18 * 60 + 13, 4200, true),
        (18 * 60 + 22, 4202, true),
        (18 * 60 + 27, 4202, true),
        (18 * 60 + 27, 4170, false),
        (18 * 60 + 31, 4163, false),
        (18 * 60 + 41, 4158, false),
        (18 * 60 + 51, 4148, false),
        (19 * 60 + 1, 4150, false),
        (19 * 60 + 11, 4146, false),
        (19 * 60 + 21, 4145, false),
        (19 * 60 + 41, 4140, false),
        (20 * 60 + 1, 4137, false),
    ];

    /// Replays `log` a second at a time (linear between its lines), from a
    /// boot a minute before it starts, and returns the shown percentage,
    /// and whether charging, each second.
    fn replay(log: &[(u32, u32, bool)]) -> Vec<(u32, u32, bool)> {
        let mut battery = Battery::new();
        let start = log[0].0 * 60;
        let mut out = Vec::new();
        for pair in log.windows(2) {
            let ((m0, mv0, _), (m1, mv1, charging)) = (pair[0], pair[1]);
            for second in m0 * 60..m1 * 60 {
                let along = f64::from(second - m0 * 60) / f64::from((m1 - m0) * 60);
                let mv = f64::from(mv0) + (f64::from(mv1) - f64::from(mv0)) * along;
                let uptime = second - start + 60;
                battery.update(raw(mv.round() as u32), charging, charging, uptime);
                out.push((second, battery.percent(), charging));
            }
        }
        out
    }

    /// Asserts that `trace` never drops more than `by` points below where
    /// it was when the charger came off, nor rises after.
    fn assert_no_drop(trace: &[(u32, u32, bool)], by: u32) -> u32 {
        let unplug = trace
            .iter()
            .rposition(|&(_, _, charging)| charging)
            .expect("charged");
        let at_unplug = trace[unplug].1;
        for &(second, shown, _) in &trace[unplug..] {
            assert!(
                shown <= at_unplug,
                "at {}:{:02}: up to {shown}%",
                second / 3600,
                second / 60 % 60
            );
            assert!(
                shown + by >= at_unplug,
                "at {}:{:02}: down to {shown}% from {at_unplug}%",
                second / 3600,
                second / 60 % 60
            );
        }
        at_unplug
    }

    #[test]
    fn the_morning_top_up() {
        let trace = replay(MORNING);
        // 71% before; the linear percentage read 61%.
        let before = trace
            .iter()
            .find(|&&(second, _, _)| second == (9 * 60 + 24) * 60)
            .expect("in the log")
            .1;
        assert_eq!(before, 71);
        // The linear percentage read 90% at 09:40 and 100% at 10:00, and
        // 91% at 10:30, off the charger.
        let at_unplug = assert_no_drop(&trace, 1);
        let settled = u32::from(charge_percent(4138));
        assert!(
            at_unplug <= settled && at_unplug + 4 >= settled,
            "{at_unplug}% charging, {settled}% settled"
        );
    }

    #[test]
    fn the_evening_top_up() {
        let trace = replay(EVENING);
        assert_no_drop(&trace, 1);
    }
}
