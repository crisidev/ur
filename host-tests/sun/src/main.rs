//! Host tests for the firmware's `sun` module. `cargo test` runs them.

#[path = "../../../src/sun.rs"]
#[allow(dead_code)]
mod sun;

fn main() {
    println!("run `cargo test`");
}

#[cfg(test)]
mod tests {
    use super::sun::*;

    /// Days since 1970-01-01 of a date (Hinnant's `days_from_civil`, as
    /// the firmware's `calendar` has it).
    fn day(year: i64, month: i64, day: i64) -> i64 {
        let y = if month <= 2 { year - 1 } else { year };
        let era = y.div_euclid(400);
        let yoe = y - era * 400;
        let doy = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146_097 + doe - 719_468
    }

    const fn hm(hour: u16, minute: u16) -> u16 {
        hour * 60 + minute
    }

    /// Sunrise and sunset, as times of day (sunset being the first
    /// minute that's night).
    fn times(sun: Sun) -> (u16, u16) {
        (sun.sunrise, (sun.sunrise + sun.daylight) % (24 * 60))
    }

    /// Which minutes of the day are day.
    fn day_minutes(sun: Sun) -> Vec<u16> {
        (0..24 * 60).filter(|&m| sun.contains(m)).collect()
    }

    const OSLO: (f32, f32) = (59.9139, 10.7522);
    const TROMSO: (f32, f32) = (69.6492, 18.9553);
    const REYKJAVIK: (f32, f32) = (64.1466, -21.9426);
    const QUITO: (f32, f32) = (-0.1807, -78.4678);

    fn at((latitude, longitude): (f32, f32), date: i64, offset: i16) -> Sun {
        compute(date, latitude, longitude, offset)
    }

    // ==========================================================================
    // Known places and days
    // ==========================================================================

    /// Published Oslo times for the 2025 solstices (in summer time, then
    /// winter time), which the `f64` equation this replaced also gave.
    #[test]
    fn oslo_at_the_solstices() {
        let summer = at(OSLO, day(2025, 6, 21), 120);
        assert_eq!(times(summer), (hm(3, 53), hm(22, 43)));
        let winter = at(OSLO, day(2025, 12, 21), 60);
        assert_eq!(times(winter), (hm(9, 18), hm(15, 12)));

        // Day from the sunrise's minute, night from the sunset's.
        for (sun, (rise, set)) in [(summer, times(summer)), (winter, times(winter))] {
            assert!(sun.contains(hm(12, 0)));
            assert!(sun.contains(rise) && sun.contains(set - 1));
            assert!(!sun.contains(rise - 1) && !sun.contains(set));
            assert!(!sun.contains(0) && !sun.contains(hm(23, 59)));
            assert_eq!(day_minutes(sun), (rise..set).collect::<Vec<_>>());
        }
    }

    /// On the equator the day is about 12 hours (a few minutes more, for
    /// refraction and the sun's disc) all year.
    #[test]
    fn the_equator_has_twelve_hours_all_year() {
        for i in 0..365 {
            let sun = at(QUITO, day(2025, 1, 1) + i, -300);
            assert!(
                (hm(12, 0)..=hm(12, 12)).contains(&sun.daylight),
                "{i}: {sun:?}"
            );
            assert!(
                (hm(5, 50)..=hm(6, 30)).contains(&sun.sunrise),
                "{i}: {sun:?}"
            );
            assert!(sun.contains(hm(12, 0)) && !sun.contains(0), "{i}");
        }
    }

    /// Where the sun sets after local midnight, the day is the minutes
    /// outside sunset..sunrise. Before `Sun` kept the day's length, these
    /// had no day at all: the dark theme on the brightest days of the year.
    #[test]
    fn the_sun_sets_after_midnight() {
        // Tromsø, the nights before and after the midnight sun.
        let cases = [
            (at(TROMSO, day(2025, 5, 16), 120), (hm(1, 12), hm(0, 9))),
            (at(TROMSO, day(2025, 7, 25), 120), (hm(1, 13), hm(0, 27))),
            (at(TROMSO, day(2025, 7, 26), 120), (hm(1, 32), hm(0, 8))),
            // Reykjavik at midsummer (UTC all year).
            (at(REYKJAVIK, day(2025, 6, 21), 0), (hm(2, 55), hm(0, 4))),
            // Oslo, where the watch last had a fix, with the offset the
            // phone took on landing in San Francisco (summer time): Oslo's
            // sun, on San Francisco's clock.
            (at(OSLO, day(2025, 6, 21), -420), (hm(18, 53), hm(13, 43))),
        ];
        for (sun, (rise, set)) in cases {
            assert_eq!(times(sun), (rise, set), "{sun:?}");
            assert!(set < rise);
            let night: Vec<u16> = (set..rise).collect();
            let day: Vec<u16> = (0..set).chain(rise..24 * 60).collect();
            assert_eq!(day_minutes(sun), day, "{sun:?}");
            assert!(night.iter().all(|&m| !sun.contains(m)));
        }
        // Noon in Tromsø, and the wearer's lunch in San Francisco (21:00
        // in Oslo, before its sunset).
        assert!(cases[0].0.contains(hm(12, 0)));
        assert!(cases[4].0.contains(hm(12, 0)));
        assert!(!cases[4].0.contains(hm(15, 0)));
    }

    #[test]
    fn polar_night_and_polar_day() {
        let winter = at(TROMSO, day(2025, 12, 21), 60);
        assert_eq!(winter, Sun::POLAR_NIGHT);
        assert!(day_minutes(winter).is_empty());
        let summer = at(TROMSO, day(2025, 6, 21), 120);
        assert_eq!(summer, Sun::POLAR_DAY);
        assert_eq!(day_minutes(summer).len(), 24 * 60);
    }

    // ==========================================================================
    // The edges of the representation
    // ==========================================================================

    /// Every sunrise, with lengths from none to all day: exactly
    /// `daylight` minutes are day, one stretch from the sunrise's on
    /// (round midnight if it gets there).
    #[test]
    fn daylight_minutes_from_sunrise() {
        for sunrise in 0..24 * 60 {
            for daylight in [0, 1, 2, 719, 720, 721, 1438, 1439, 1440] {
                let sun = Sun { sunrise, daylight };
                let expected: Vec<u16> = (0..daylight).map(|i| (sunrise + i) % 1440).collect();
                let mut day = day_minutes(sun);
                day.sort_by_key(|&m| (m + 1440 - sunrise) % 1440);
                assert_eq!(day, expected, "{sunrise} {daylight}");
            }
        }
    }

    /// Sunrise and sunset in the same minute: polar night's
    /// representation, but also a day of a minute's fraction (no minute
    /// ends with the sun up) or of 1439.something minutes (no minute ends
    /// in the night, or one does). The length tells them apart.
    #[test]
    fn sunrise_and_sunset_in_one_minute() {
        // A peek of the sun, within minute 100.
        assert_eq!(
            Sun::from_minutes(100.2, 0.5),
            Sun {
                sunrise: 100,
                daylight: 0
            }
        );
        // A peek across the end of minute 100: that minute is day.
        assert_eq!(
            Sun::from_minutes(100.8, 0.5),
            Sun {
                sunrise: 100,
                daylight: 1
            }
        );
        // Up 1439.7 minutes from 100.5: the night, 100.2 to 100.5, has no
        // minute's end in it, so it's all day.
        let sun = Sun::from_minutes(100.5, 1439.7);
        assert_eq!(
            sun,
            Sun {
                sunrise: 100,
                daylight: 1440
            }
        );
        // From 100.2: the night, 99.9 to 100.2, has minute 99's end.
        let sun = Sun::from_minutes(100.2, 1439.7);
        assert_eq!(
            sun,
            Sun {
                sunrise: 100,
                daylight: 1439
            }
        );
        assert_eq!(times(sun), (100, 99));
        assert_eq!(day_minutes(sun).len(), 1439);
        assert!(!sun.contains(99) && sun.contains(100) && sun.contains(98));
        // The same round midnight: the night, 1439.9 to 1440.2, has minute
        // 1439's end, and no other.
        let sun = Sun::from_minutes(0.2, 1439.7);
        assert_eq!(
            sun,
            Sun {
                sunrise: 0,
                daylight: 1439
            }
        );
        assert!(!sun.contains(1439) && sun.contains(0) && sun.contains(1438));
        // A day of exactly 1440 minutes, or a hair more as `f32`s round.
        assert_eq!(Sun::from_minutes(1439.9, 1440.0).daylight, 1440);
        assert_eq!(Sun::from_minutes(1439.9, 1440.001).daylight, 1440);
        // What a location no phone sends can make of it.
        assert_eq!(Sun::from_minutes(f32::NAN, f32::NAN), Sun::POLAR_NIGHT);
    }

    // ==========================================================================
    // Against the hour angle, everywhere
    // ==========================================================================

    /// For every minute, `contains` says what the sun's hour angle says:
    /// a minute is day if, at its end, the sun is less than the day's hour
    /// angle from its transit (either way round the clock). For latitudes
    /// from pole to pole (densely where the day nears 24 hours), a few
    /// longitudes and time zones (some far from the location's), and a
    /// day a fortnight through a year.
    ///
    /// The brute force works in `f64` from `passage`'s transit and hour
    /// angle; `compute` gets there in `f32`, which rounds to within a
    /// hundredth of a second, so minutes ending that close to a sunrise or
    /// sunset are left out.
    #[test]
    fn day_is_when_the_hour_angle_says() {
        let latitudes = (-89..=89)
            .step_by(2)
            .map(|d| d as f32)
            .chain((0..=30).map(|i| 60.0 + i as f32 * 0.5))
            .chain((0..=30).map(|i| -60.0 - i as f32 * 0.5));
        let (mut passages, mut checked, mut wrapped) = (0u64, 0u64, 0u64);
        for latitude in latitudes {
            for longitude in [-21.9, 18.9, 179.9] {
                for offset in [-420, 60, 840] {
                    for fortnight in 0..27 {
                        let date = day(2025, 1, 1) + 14 * fortnight;
                        let sun = compute(date, latitude, longitude, offset);
                        let sky = passage(date, latitude, longitude);
                        let Sky::Passage {
                            transit,
                            hour_angle,
                        } = sky
                        else {
                            let polar = if sky == Sky::PolarDay {
                                Sun::POLAR_DAY
                            } else {
                                Sun::POLAR_NIGHT
                            };
                            assert_eq!(sun, polar);
                            continue;
                        };
                        passages += 1;
                        wrapped += u64::from(sun.sunrise + sun.daylight > 1440);
                        // The transit in local minutes, and the half day's
                        // length, in minutes either side of it.
                        let noon = (0.5 + f64::from(transit)) * 1440.0 + f64::from(offset);
                        let half = 4.0 * f64::from(hour_angle);
                        for minute in 0..24 * 60 {
                            // How long after the transit the minute ends,
                            // within half a day either way.
                            let since =
                                (f64::from(minute + 1) - noon + 720.0).rem_euclid(1440.0) - 720.0;
                            let near = (since - half).abs().min((since + half).abs());
                            if near < 1.0 / 6000.0 {
                                continue;
                            }
                            let up = -half < since && since <= half;
                            assert_eq!(
                                sun.contains(minute),
                                up,
                                "{latitude} {longitude} {offset} {date} {minute}: {sun:?}"
                            );
                            checked += 1;
                        }
                    }
                }
            }
        }
        // Not vacuous: nearly every minute is checked, and many days wrap.
        assert!(
            checked > passages * 1430 && wrapped > passages / 10,
            "{passages} {checked} {wrapped}"
        );
    }

    /// Anywhere, any day, in any time zone: the sun is a sunrise within
    /// the day and a length of at most a day, and computing it ends. That
    /// goes for the locations and offsets nothing sends, too (the
    /// location is read from flash unchecked): `wrap`'s and `sin_deg`'s
    /// assertions about what they're asked hold throughout.
    #[test]
    fn the_sun_rises_within_the_day_everywhere() {
        // Coarse steps, plus the edges: the poles, the polar circles (where
        // the hour angle's cosine passes ±1), the date line, and the
        // integers' ends. In millionths of a degree, as the firmware keeps
        // them.
        let step = |from: i32, to: i32, by: usize| (from..=to).step_by(by);
        let latitudes = step(-90_000_000, 90_000_000, 2_500_000).chain([
            -66_560_000,
            66_560_000,
            89_999_999,
            i32::MIN,
            i32::MAX,
        ]);
        let longitudes: Vec<i32> = step(-180_000_000, 180_000_000, 15_000_000)
            .chain([179_999_999, i32::MIN, i32::MAX])
            .collect();
        let offsets = [-18 * 60, -12 * 60, -60, 0, 345, 12 * 60, 18 * 60]
            .into_iter()
            .chain([i16::MIN, i16::MAX]);
        // Every tenth day of 2025, and the calendar's ends.
        let days: Vec<i64> = (0..36)
            .map(|i| day(2025, 1 + i / 3, 1 + 10 * (i % 3)))
            .chain([
                day(1970, 1, 1),
                day(2020, 2, 29),
                day(2099, 12, 31),
                day(65_535, 12, 31),
            ])
            .collect();
        for latitude_micro in latitudes {
            for &longitude_micro in &longitudes {
                for offset in offsets.clone() {
                    for &date in &days {
                        let (latitude, longitude) =
                            (latitude_micro as f32 / 1e6, longitude_micro as f32 / 1e6);
                        let sun = compute(date, latitude, longitude, offset);
                        assert!(
                            sun.sunrise < 24 * 60 && sun.daylight <= 24 * 60,
                            "{latitude_micro} {longitude_micro} {offset} {date}: {sun:?}",
                        );
                    }
                }
            }
        }
    }

    /// To a few `f32` steps, for every angle the equation takes.
    #[test]
    fn sine_in_degrees() {
        for tenth in -10_800..=10_800 {
            let degrees = tenth as f32 / 10.0;
            let expected = (f64::from(degrees)).to_radians().sin();
            assert!(
                (f64::from(sin_deg(degrees)) - expected).abs() < 3e-7,
                "{degrees}"
            );
        }
    }
}

/// Kani proofs (`cargo kani` here, one at a time with `--harness`).
#[cfg(kani)]
mod proofs {
    use super::sun::*;

    /// For every sunrise and length an `f32` can be (`compute` hands over
    /// a sunrise in `0..1440` minutes and a length in `0..=1440`, or a
    /// hair over), and every minute: the minute is day exactly when the
    /// sun is up at its end, that is, the end falls after the sunrise and
    /// no later than the sunset, either today or, for a day that runs
    /// past midnight, a day on. Exactly, not only in real numbers: the
    /// sunset is the very `f32` sum `from_minutes` floors.
    #[kani::proof]
    fn a_minute_is_day_when_the_sun_is_up_at_its_end() {
        let rise: f32 = kani::any();
        kani::assume((0.0..1440.0).contains(&rise));
        let length: f32 = kani::any();
        kani::assume((0.0..=1440.5).contains(&length));
        let minute: u16 = kani::any();
        kani::assume(minute < 1440);

        let sun = Sun::from_minutes(rise, length);
        assert!(sun.sunrise < 1440 && sun.daylight <= 1440);

        let set = rise + length;
        let up = |at: f32| rise < at && at <= set;
        let end = f32::from(minute + 1);
        assert_eq!(sun.contains(minute), up(end) || up(end + 1440.0));
    }
}
