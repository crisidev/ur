//! Host tests for the firmware's `battery` module: when a battery counts as
//! critically low. `cargo test` runs them.

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
        (mv * 4096 / 7200) as i16
    }

    /// A battery that has read `mv` for long enough to have settled.
    fn settled(mv: u32, power_present: bool) -> Battery {
        let mut battery = Battery::new();
        for _ in 0..200 {
            battery.update(raw(mv), false, power_present);
        }
        battery
    }

    /// Feeds one reading a second for `seconds`, from uptime `start`, and
    /// returns the first second at which the battery was critical.
    fn first_critical(battery: &mut Battery, mv: u32, start: u32, seconds: u32) -> Option<u32> {
        (start..start + seconds).find(|&second| {
            battery.update(raw(mv), false, battery.power_present);
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
    fn the_threshold_sits_below_zero_percent() {
        // The percentage reads 0% from 3.5 V, with ~5% left; critical is
        // further down the knee.
        let battery = settled(CRITICAL_MV, false);
        assert_eq!(battery.percent(), 0);
        let battery = settled(3500, false);
        assert_eq!(battery.percent(), 0);
    }
}
