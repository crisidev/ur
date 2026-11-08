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

    #[test]
    fn first_reading_rounds_to_nearest() {
        assert_eq!(shown(None, q(31.0)), 31);
        assert_eq!(shown(None, q(31.25)), 31);
        assert_eq!(shown(None, q(31.5)), 32);
        assert_eq!(shown(None, q(31.75)), 32);
        assert_eq!(shown(None, 0), 0);
    }

    #[test]
    fn negative_readings_round_like_positive_ones() {
        // Truncating division would make -0.75 °C read as 0°, and -5.25 °C
        // as -5° but -4.75 °C as -4°.
        assert_eq!(shown(None, q(-0.25)), 0);
        assert_eq!(shown(None, q(-0.5)), 0);
        assert_eq!(shown(None, q(-0.75)), -1);
        assert_eq!(shown(None, q(-4.75)), -5);
        assert_eq!(shown(None, q(-5.25)), -5);
        assert_eq!(shown(None, q(-40.0)), -40);
    }

    #[test]
    fn small_changes_keep_what_is_shown() {
        // Hovering around 31.5 °C, as a watch on a steady wrist does, the
        // display stays put rather than flipping each minute.
        let mut degrees = shown(None, q(31.25));
        for reading in [31.5, 31.25, 31.5, 31.75, 31.5, 30.5, 31.0, 30.5] {
            degrees = shown(Some(degrees), q(reading));
            assert_eq!(degrees, 31, "at {reading}");
        }
    }

    #[test]
    fn a_wobble_across_the_boundary_does_not_flip() {
        let mut degrees = shown(None, q(31.75));
        assert_eq!(degrees, 32);
        for reading in [31.25, 31.75, 31.5, 31.25, 31.75] {
            degrees = shown(Some(degrees), q(reading));
            assert_eq!(degrees, 32, "at {reading}");
        }
    }

    #[test]
    fn real_changes_come_through() {
        assert_eq!(shown(Some(31), q(32.0)), 32);
        assert_eq!(shown(Some(31), q(30.0)), 30);
        assert_eq!(shown(Some(31), q(32.5)), 33);
        // Taken off the wrist and left on a desk: straight to the new value.
        assert_eq!(shown(Some(32), q(23.0)), 23);
        assert_eq!(shown(Some(1), q(-0.75)), -1);
    }

    #[test]
    fn a_slow_drift_follows_within_a_degree() {
        // Cooling a quarter degree a minute, from 33 °C to 22 °C: what's
        // shown never lags the reading by a full degree.
        let mut degrees = shown(None, q(33.0));
        for quarters in (q(22.0)..=q(33.0)).rev() {
            degrees = shown(Some(degrees), quarters);
            let lag = 4 * i32::from(degrees) - quarters;
            assert!(lag.abs() < 4, "{degrees}° at {quarters}/4 °C");
        }
        assert_eq!(degrees, 22);
    }
}
