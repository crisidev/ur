//! Host tests for the firmware's `temperature` module. `cargo test` runs
//! them.

#[path = "../../../src/temperature.rs"]
#[allow(dead_code)]
mod temperature;

fn main() {
    println!("run `cargo test`");
}

#[cfg(test)]
mod tests {
    use super::temperature::*;

    /// Quarter degrees, the sensor's unit.
    fn q(degrees: f32) -> i32 {
        (degrees * 4.0) as i32
    }

    /// Hundredths, a room sensor's unit (and [`pick`]'s).
    fn h(degrees: f32) -> i32 {
        (degrees * 100.0).round() as i32
    }

    /// [`shown`] for the watch's own reading in quarter degrees, in whole
    /// degrees, checking that it is whole.
    fn whole(previous: Option<i8>, quarters: i32) -> i8 {
        let tenths = shown(
            previous.map(|d| 10 * i16::from(d)),
            25 * quarters,
            Source::Watch,
        );
        assert_eq!(tenths % 10, 0, "{tenths} tenths aren't whole degrees");
        (tenths / 10) as i8
    }

    /// [`shown`] for a room sensor's reading, in tenths.
    fn tenths(previous: Option<i16>, degrees: f32) -> i16 {
        shown(previous, h(degrees), Source::Sensor)
    }

    #[test]
    fn first_reading_rounds_to_nearest() {
        assert_eq!(whole(None, q(31.0)), 31);
        assert_eq!(whole(None, q(31.25)), 31);
        assert_eq!(whole(None, q(31.5)), 32);
        assert_eq!(whole(None, q(31.75)), 32);
        assert_eq!(whole(None, 0), 0);
    }

    #[test]
    fn negative_readings_round_like_positive_ones() {
        // Truncating division would make -0.75 °C read as 0°, and -5.25 °C
        // as -5° but -4.75 °C as -4°.
        assert_eq!(whole(None, q(-0.25)), 0);
        assert_eq!(whole(None, q(-0.5)), 0);
        assert_eq!(whole(None, q(-0.75)), -1);
        assert_eq!(whole(None, q(-4.75)), -5);
        assert_eq!(whole(None, q(-5.25)), -5);
        assert_eq!(whole(None, q(-40.0)), -40);
    }

    #[test]
    fn small_changes_keep_what_is_shown() {
        // Hovering around 31.5 °C, as a watch on a steady wrist does, the
        // display stays put rather than flipping each minute.
        let mut degrees = whole(None, q(31.25));
        for reading in [31.5, 31.25, 31.5, 31.75, 31.5, 30.5, 31.0, 30.5] {
            degrees = whole(Some(degrees), q(reading));
            assert_eq!(degrees, 31, "at {reading}");
        }
    }

    #[test]
    fn a_wobble_across_the_boundary_does_not_flip() {
        let mut degrees = whole(None, q(31.75));
        assert_eq!(degrees, 32);
        for reading in [31.25, 31.75, 31.5, 31.25, 31.75] {
            degrees = whole(Some(degrees), q(reading));
            assert_eq!(degrees, 32, "at {reading}");
        }
    }

    #[test]
    fn real_changes_come_through() {
        assert_eq!(whole(Some(31), q(32.0)), 32);
        assert_eq!(whole(Some(31), q(30.0)), 30);
        assert_eq!(whole(Some(31), q(32.5)), 33);
        // Taken off the wrist and left on a desk: straight to the new value.
        assert_eq!(whole(Some(32), q(23.0)), 23);
        assert_eq!(whole(Some(1), q(-0.75)), -1);
    }

    #[test]
    fn a_slow_drift_follows_within_a_degree() {
        // Cooling a quarter degree a minute, from 33 °C to 22 °C: what's
        // shown never lags the reading by a full degree.
        let mut degrees = whole(None, q(33.0));
        for quarters in (q(22.0)..=q(33.0)).rev() {
            degrees = whole(Some(degrees), quarters);
            let lag = 4 * i32::from(degrees) - quarters;
            assert!(lag.abs() < 4, "{degrees}° at {quarters}/4 °C");
        }
        assert_eq!(degrees, 22);
    }

    #[test]
    fn a_room_sensor_shows_tenths() {
        assert_eq!(tenths(None, 21.37), 214);
        assert_eq!(tenths(None, 21.35), 214);
        assert_eq!(tenths(None, 21.34), 213);
        assert_eq!(tenths(None, 20.0), 200);
        // Below zero, half up as above: -4.25 is -4.2, not -4.3.
        assert_eq!(tenths(None, -4.25), -42);
        assert_eq!(tenths(None, -4.26), -43);
        assert_eq!(tenths(None, -0.04), 0);
    }

    #[test]
    fn a_wandering_room_sensor_holds_its_tenth() {
        // An Aranet4's 0.05 °C steps, across a tenth's boundary and a
        // whole step either side of what's shown: no flipping.
        let mut held = tenths(None, 21.40);
        for reading in [21.45, 21.50, 21.40, 21.35, 21.30, 21.45, 21.50] {
            held = tenths(Some(held), reading);
            assert_eq!(held, 214, "at {reading}");
        }
        // A BTHome thermometer's hundredths, likewise.
        for reading in [21.47, 21.33, 21.49, 21.31] {
            held = tenths(Some(held), reading);
            assert_eq!(held, 214, "at {reading}");
        }
    }

    #[test]
    fn a_room_sensor_follows_within_a_tenth() {
        assert_eq!(tenths(Some(214), 21.55), 216);
        assert_eq!(tenths(Some(214), 21.25), 213);
        assert_eq!(tenths(Some(214), 19.0), 190);
        // Warming a hundredth at a time: never more than a tenth behind.
        let mut held = tenths(None, 20.0);
        for hundredths in h(20.0)..=h(22.0) {
            held = shown(Some(held), hundredths, Source::Sensor);
            let lag = hundredths - 10 * i32::from(held);
            assert!(lag.abs() <= 10, "{held}/10 at {hundredths}/100");
        }
        assert_eq!(held, 219);
    }

    // ==========================================================================
    // Which source, on the wrist or not, and the estimate
    // ==========================================================================

    const MIN: u64 = 60 * 1000;

    #[test]
    fn a_nearby_sensor_wins_then_the_wrist_then_the_watch() {
        let calibration = Calibration::new();
        let die = q(27.5);
        assert_eq!(
            pick(Some(h(20.37)), true, die, &calibration),
            (h(20.37), Source::Sensor)
        );
        assert_eq!(
            pick(Some(h(20.37)), false, die, &calibration),
            (h(20.37), Source::Sensor)
        );
        assert_eq!(
            pick(None, true, die, &calibration),
            (h(21.5), Source::Wrist)
        );
        assert_eq!(
            pick(None, false, die, &calibration),
            (h(27.5), Source::Watch)
        );
    }

    #[test]
    fn worn_takes_a_pulse() {
        let mut wear = Wear::new(0);
        assert!(!wear.worn());
        // A session without a verdict (a busy wrist): still not known.
        wear.update(MIN, false, false, None);
        assert!(!wear.worn());
        wear.update(2 * MIN, false, false, Some(90 * 1000));
        assert!(wear.worn());
        assert_eq!(wear.worn_for(2 * MIN + 30 * 1000), Some(MIN));
    }

    #[test]
    fn no_skin_or_lying_still_is_off() {
        let mut wear = Wear::new(0);
        wear.update(MIN, false, false, Some(MIN));
        assert!(wear.worn());
        wear.update(2 * MIN, true, false, Some(MIN));
        assert!(!wear.worn());
        // That old pulse doesn't put it back on; a new one does.
        wear.update(3 * MIN, false, false, Some(MIN));
        assert!(!wear.worn());
        wear.update(4 * MIN, false, false, Some(4 * MIN));
        assert!(wear.worn());
        // Still, but not for long: a wrist at a desk.
        let mut at = 4 * MIN;
        while at < 18 * MIN {
            at += MIN;
            wear.update(at, false, true, Some(4 * MIN));
            assert!(wear.worn(), "at {at}");
        }
        // A moment's movement starts the count again.
        wear.update(at + 1000, false, false, Some(4 * MIN));
        for _ in 0..14 {
            at += MIN;
            wear.update(at, false, true, Some(4 * MIN));
            assert!(wear.worn(), "at {at}");
        }
        // Fifteen minutes still: lying on a table.
        wear.update(at + 2 * MIN, false, true, Some(4 * MIN));
        assert!(!wear.worn());
    }

    #[test]
    fn the_prior_alone_takes_off_the_skin() {
        let calibration = Calibration::new();
        // 27.5 °C on a wrist is a 21.5 °C room; each degree of die is 1.6
        // degrees of air.
        assert_eq!(calibration.estimate(q(27.5)), q(21.5));
        assert_eq!(calibration.estimate(q(30.0)), q(25.5));
        assert_eq!(calibration.estimate(q(25.0)), q(17.5));
        // Never more than 15 °C below the die, nor 5 above.
        assert_eq!(calibration.estimate(q(10.0)), q(-5.0));
    }

    #[test]
    fn learns_this_watch_s_offset_quickly() {
        // A watch whose die reads a degree and a half warmer than the
        // defaults expect, in a 21 °C room.
        let mut calibration = Calibration::new();
        let die = q(29.0);
        // 23.9 °C, to the nearest quarter.
        assert_eq!(calibration.estimate(die), q(24.0));
        for _ in 0..10 {
            calibration.learn(die, q(21.0));
        }
        let error = (calibration.estimate(die) - q(21.0)).abs();
        assert!(error <= 2, "{} quarters off after 10 pairs", error);
        // And the slope held: one room's pairs can't tell it.
        let (k, _) = calibration.line();
        assert!((k - 1.6).abs() < 0.1, "slope {k}");
    }

    #[test]
    fn learns_the_slope_from_a_spread_of_rooms() {
        // A watch whose truth is air = 2.0 · (die − 27.5) + 21.0, seen
        // over a few days in rooms from 17 to 25 °C.
        let truth = |die: f32| 2.0 * (die - 27.5) + 21.0;
        let mut calibration = Calibration::new();
        for i in 0..200 {
            let die = 25.5 + (i % 9) as f32 * 0.5;
            calibration.learn(q(die), q(truth(die)));
        }
        let (k, _) = calibration.line();
        assert!((k - 2.0).abs() < 0.15, "slope {k}");
        for die in [26.0, 27.5, 29.0] {
            let error = (calibration.estimate(q(die)) - q(truth(die))).abs();
            assert!(error <= 2, "{error} quarters off at {die}");
        }
    }

    #[test]
    fn the_slope_stays_physical() {
        // Nonsense pairs (the air moving opposite to the die) can't turn
        // the line around.
        let mut calibration = Calibration::new();
        for i in 0..100 {
            let die = 25.0 + (i % 5) as f32;
            calibration.learn(q(die), q(40.0 - die));
        }
        let (k, _) = calibration.line();
        assert!((1.0..=3.0).contains(&k), "slope {k}");
    }

    #[test]
    fn calibration_survives_flash() {
        let mut calibration = Calibration::new();
        calibration.learn(q(28.0), q(20.0));
        calibration.learn(q(29.0), q(22.0));
        assert_eq!(Calibration::from_bytes(calibration.to_bytes()), calibration);
        // Anything else is the prior.
        assert_eq!(
            Calibration::from_bytes([0; CALIBRATION_BYTES]),
            Calibration::new()
        );
        let mut nan = calibration.to_bytes();
        nan[1..5].copy_from_slice(&f32::NAN.to_le_bytes());
        assert_eq!(Calibration::from_bytes(nan), Calibration::new());
    }

    #[test]
    fn pairs_only_from_a_settled_wrist_in_one_room() {
        let mut pairing = Pairing::new();
        let worn = Some(30 * MIN);
        // The first pair has nothing to compare with.
        assert!(!pairing.offer(0, q(28.0), q(21.0), worn));
        assert!(pairing.offer(15 * MIN, q(28.25), q(21.5), worn));
        // A different room: the air jumped.
        assert!(!pairing.offer(30 * MIN, q(28.25), q(18.0), worn));
        // Then settled there.
        assert!(pairing.offer(45 * MIN, q(27.5), q(18.0), worn));
        // Too long since the last one to know what happened between.
        assert!(!pairing.offer(90 * MIN, q(27.5), q(18.0), worn));
        // Just put on: the watch is still warming up.
        let mut pairing = Pairing::new();
        assert!(!pairing.offer(0, q(25.0), q(21.0), Some(5 * MIN)));
        assert!(!pairing.offer(15 * MIN, q(25.0), q(21.0), Some(20 * MIN - 1)));
        assert!(!pairing.offer(30 * MIN, q(25.0), q(21.0), Some(35 * MIN)));
        assert!(pairing.offer(45 * MIN, q(25.0), q(21.0), Some(50 * MIN)));
        // Off the wrist: never.
        assert!(!pairing.offer(60 * MIN, q(25.0), q(21.0), None));
    }

    #[test]
    fn pairs_only_at_a_room_s_temperature() {
        // Sat by a fridge's thermometer, as steady as can be.
        let mut pairing = Pairing::new();
        let worn = Some(30 * MIN);
        for at in [0, 15 * MIN, 30 * MIN] {
            assert!(!pairing.offer(at, q(27.0), q(4.0), worn));
        }
        // The room's edges.
        let mut pairing = Pairing::new();
        assert!(!pairing.offer(0, q(28.0), q(10.0), worn));
        assert!(pairing.offer(15 * MIN, q(28.0), q(10.0), worn));
        assert!(!pairing.offer(30 * MIN, q(28.0), q(9.75), worn));
        let mut pairing = Pairing::new();
        assert!(!pairing.offer(0, q(31.0), q(35.0), worn));
        assert!(pairing.offer(15 * MIN, q(31.0), q(35.0), worn));
        assert!(!pairing.offer(30 * MIN, q(31.0), q(35.25), worn));
    }
}
