//! Host tests for the firmware's `brightness` module. `cargo test` runs
//! them.

#[path = "../../../src/brightness.rs"]
#[allow(dead_code)]
mod brightness;

fn main() {
    println!("run `cargo test`");
}

#[cfg(test)]
mod tests {
    use super::brightness::*;

    const SECOND: u64 = 1000;
    const MINUTE: i64 = 60;
    const DAY: i64 = 24 * 60 * MINUTE;
    /// A day, in local seconds since 1970.
    const TODAY: i64 = 20_400 * DAY;
    const LOW_MID: Level = BY_CURRENT[2];

    /// Minutes after midnight.
    const fn hhmm(hh: i64, mm: i64) -> i64 {
        hh * 60 + mm
    }

    /// A day as `ui::App` and `main` see it: when the sun is up (minutes
    /// after midnight, `start..end`, wrapping past midnight if `end` <
    /// `start`; `None` for polar night), and when the wearer gets up each
    /// day (`None`: never sleeps).
    struct Day {
        sun: Option<(i64, i64)>,
        wake: Option<i64>,
    }

    impl Day {
        fn daytime(&self, minute: i64) -> bool {
            match self.sun {
                None => false,
                Some((start, end)) if start < end => (start..end).contains(&minute),
                Some((start, end)) => minute >= start || minute < end,
            }
        }

        /// The AUTO level at each minute of a day, after a day and a half
        /// of the same (to settle `small_hours`), as the UI loop would set
        /// it: `small_hours` updated once a minute (the boundaries are on
        /// the minute, so that's as good as once a second).
        fn levels(&self) -> Vec<Level> {
            let mut small = false;
            let mut levels = Vec::new();
            let mut t = TODAY - DAY - DAY / 2;
            while t < TODAY + DAY - MINUTE {
                let next = t + MINUTE;
                let minute = next.rem_euclid(DAY) / MINUTE;
                let got_up = self.wake == Some(minute);
                small = small_hours(small, Some((t, next)), got_up);
                if next >= TODAY {
                    let daytime = self.daytime(minute);
                    levels.push(level(Setting::Auto, daytime, small, false, None));
                }
                t = next;
            }
            levels
        }
    }

    /// Checks `levels` (one per minute from midnight) against `expected`:
    /// from each minute, the level until the next entry.
    fn check(levels: &[Level], expected: &[(i64, Level)]) {
        assert_eq!(levels.len() as i64, DAY / MINUTE);
        for (i, &(from, want)) in expected.iter().enumerate() {
            let to = expected.get(i + 1).map_or(DAY / MINUTE, |&(to, _)| to);
            for minute in from..to {
                assert_eq!(
                    levels[minute as usize],
                    want,
                    "at {:02}:{:02}",
                    minute / 60,
                    minute % 60
                );
            }
        }
    }

    #[test]
    fn an_ordinary_day() {
        let day = Day {
            sun: Some((hhmm(7, 30), hhmm(16, 0))),
            wake: Some(hhmm(6, 30)),
        };
        check(
            &day.levels(),
            &[
                (0, Level::LOW),
                (hhmm(6, 30), Level::MID),
                (hhmm(7, 30), Level::ALL),
                (hhmm(16, 0), Level::MID),
            ],
        );
    }

    #[test]
    fn sunset_after_midnight_is_bright_until_sunset_then_low_until_waking() {
        // A high-latitude summer: the sun sets at 00:40, rises at 02:50.
        let day = Day {
            sun: Some((hhmm(2, 50), hhmm(0, 40))),
            wake: Some(hhmm(7, 0)),
        };
        check(
            &day.levels(),
            &[
                (0, Level::ALL),
                (hhmm(0, 40), Level::LOW),
                (hhmm(2, 50), Level::ALL),
            ],
        );
    }

    #[test]
    fn sunset_after_midnight_and_waking_before_sunrise() {
        let day = Day {
            sun: Some((hhmm(5, 0), hhmm(0, 40))),
            wake: Some(hhmm(4, 0)),
        };
        check(
            &day.levels(),
            &[
                (0, Level::ALL),
                (hhmm(0, 40), Level::LOW),
                (hhmm(4, 0), Level::MID),
                (hhmm(5, 0), Level::ALL),
            ],
        );
    }

    #[test]
    fn waking_after_sunrise_is_bright() {
        let day = Day {
            sun: Some((hhmm(4, 30), hhmm(22, 0))),
            wake: Some(hhmm(7, 0)),
        };
        check(
            &day.levels(),
            &[
                (0, Level::LOW),
                (hhmm(4, 30), Level::ALL),
                (hhmm(22, 0), Level::MID),
            ],
        );
    }

    #[test]
    fn polar_night_is_mid_from_waking_until_midnight() {
        let day = Day {
            sun: None,
            wake: Some(hhmm(7, 15)),
        };
        check(&day.levels(), &[(0, Level::LOW), (hhmm(7, 15), Level::MID)]);
    }

    #[test]
    fn never_sleeping_is_low_from_midnight_until_sunrise() {
        let day = Day {
            sun: Some((hhmm(8, 0), hhmm(15, 30))),
            wake: None,
        };
        check(
            &day.levels(),
            &[
                (0, Level::LOW),
                (hhmm(8, 0), Level::ALL),
                // Noon ended the small hours, so the evening is mid again.
                (hhmm(15, 30), Level::MID),
            ],
        );
    }

    #[test]
    fn polar_night_without_sleep_is_low_until_noon() {
        let day = Day {
            sun: None,
            wake: None,
        };
        check(&day.levels(), &[(0, Level::LOW), (hhmm(12, 0), Level::MID)]);
    }

    #[test]
    fn small_hours_follow_midnight_noon_and_getting_up() {
        let at = |hh, mm| TODAY + hhmm(hh, mm) * MINUTE;
        // Midnight starts them.
        assert!(small_hours(false, Some((at(0, 0) - 1, at(0, 0))), false));
        // Getting up ends them, even as midnight passes.
        assert!(!small_hours(true, Some((at(5, 0), at(5, 0) + 1)), true));
        assert!(!small_hours(false, Some((at(0, 0) - 1, at(0, 0))), true));
        // Noon ends them.
        assert!(!small_hours(true, Some((at(12, 0) - 1, at(12, 0))), false));
        // Nothing else changes them.
        for current in [false, true] {
            assert_eq!(
                small_hours(current, Some((at(3, 0), at(3, 0) + 1)), false),
                current
            );
            assert_eq!(small_hours(current, None, false), current);
        }
        assert!(!small_hours(true, None, true));
        // A clock jump past both: the later one counts.
        assert!(small_hours(false, Some((at(11, 0) - DAY, at(1, 0))), false));
        assert!(!small_hours(
            true,
            Some((at(23, 0) - DAY, at(13, 0))),
            false
        ));
    }

    #[test]
    fn soft_is_a_step_down() {
        for small in [false, true] {
            assert_eq!(level(Setting::Soft, true, small, false, None), Level::MID);
            assert_eq!(level(Setting::Soft, false, small, false, None), Level::LOW);
        }
    }

    #[test]
    fn the_dim_look_is_low_whatever_the_setting() {
        for setting in [Setting::Auto, Setting::Soft] {
            for daytime in [false, true] {
                for small in [false, true] {
                    let level = level(setting, daytime, small, true, None);
                    assert_eq!(level, Level::LOW);
                }
            }
        }
    }

    #[test]
    fn levels_are_ordered_by_current() {
        // The wiki's currents for low, mid, high; combinations sum.
        let current = |l: Level| {
            let [low, mid, high] = l.drivers();
            [(low, 1.83), (mid, 5.51), (high, 12.27)]
                .iter()
                .filter(|(on, _)| *on)
                .map(|(_, ma)| ma)
                .sum::<f64>()
        };
        for pair in BY_CURRENT.windows(2) {
            assert!(current(pair[0]) < current(pair[1]), "{pair:?}");
        }
        assert_eq!(BY_CURRENT[0], Level::LOW);
        assert_eq!(BY_CURRENT[6], Level::ALL);
    }

    /// The levels a wake alarm goes through, once a second. (A wake alarm
    /// ends the small hours, so they're always over.)
    fn ramp(setting: Setting, daytime: bool) -> Vec<Level> {
        (0..=2 * RAMP / SECOND)
            .map(|s| level(setting, daytime, false, false, Some(s * SECOND)))
            .collect()
    }

    #[test]
    fn an_alarm_starts_low_and_brightens_over_the_ramp() {
        for (setting, daytime, top) in [
            (Setting::Auto, false, LOW_MID),
            (Setting::Auto, true, Level::ALL),
            (Setting::Soft, false, LOW_MID),
            (Setting::Soft, true, LOW_MID),
        ] {
            let levels = ramp(setting, daytime);
            assert_eq!(levels[0], Level::LOW, "{setting:?} {daytime}");
            // Never dimmer than a moment before.
            let ranks: Vec<_> = levels
                .iter()
                .map(|l| BY_CURRENT.iter().position(|b| b == l).expect("a level"))
                .collect();
            assert!(ranks.windows(2).all(|w| w[0] <= w[1]), "{ranks:?}");
            // At the top by the end of the ramp, and there to stay.
            let at_top = levels
                .iter()
                .position(|&l| l == top)
                .expect("reaches the top");
            assert!(at_top as u64 * SECOND <= RAMP, "{setting:?} {daytime}");
            assert!(at_top as u64 * SECOND >= RAMP / 2, "too fast: {at_top} s");
            assert!(levels[at_top..].iter().all(|&l| l == top));
        }
    }

    #[test]
    fn the_ramp_before_sunrise_is_low_then_mid_then_both() {
        let levels = ramp(Setting::Auto, false);
        assert_eq!(levels[19], Level::LOW);
        assert_eq!(levels[20], Level::MID);
        assert_eq!(levels[39], Level::MID);
        assert_eq!(levels[40], LOW_MID);
    }

    #[test]
    fn a_ringing_alarm_in_the_dim_look_stays_low() {
        let level = level(Setting::Auto, true, false, true, Some(RAMP));
        assert_eq!(level, Level::LOW);
    }

    #[test]
    fn settings_round_trip_and_cycle() {
        for setting in [Setting::Auto, Setting::Soft] {
            assert_eq!(Setting::from_byte(setting.to_byte()), setting);
            assert_ne!(setting.next(), setting);
            assert_eq!(setting.next().next(), setting);
        }
        // Nothing stored reads as 0 in `main`.
        assert_eq!(Setting::from_byte(0), Setting::Auto);
        assert_eq!(Setting::from_byte(200), Setting::Auto);
    }
}
