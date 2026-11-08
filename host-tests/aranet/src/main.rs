//! Host tests for the firmware's `aranet` and `bthome_sensor` modules:
//! Aranet4 advertisements, from the test vectors of Aranet4-Python (the
//! decoder behind Home Assistant's integration), BTHome thermometers'
//! (`bthome.rs`), and choosing the nearest sensor. `cargo test` runs them.

#[path = "../../../src/aranet.rs"]
#[allow(dead_code)]
mod aranet;
#[path = "../../../src/bthome_sensor.rs"]
#[allow(dead_code)]
mod bthome_sensor;

#[cfg(test)]
mod bthome;

fn main() {
    println!("run `cargo test`");
}

#[cfg(test)]
mod tests {
    use super::aranet::*;
    use quickcheck::quickcheck;

    /// Aranet4-Python's `TEST_DATA_ARANET_4["manufacturer_data"]`: v1.3.5,
    /// 1091 ppm, 20.75 °C, 1012.3 hPa, 53 %, battery 12 %, yellow, every
    /// 60 s, 16 s ago.
    const ARANET4: [u8; 22] = [
        0x21, 0x05, 0x03, 0x01, 0x00, 0x05, 0x00, 0x01, 0x43, 0x04, 0x9f, 0x01, 0x8b, 0x27, 0x35,
        0x0c, 0x02, 0x3c, 0x00, 0x10, 0x00, 0x55,
    ];

    /// `ARANET4` as a whole legacy advertisement: flags, then the
    /// manufacturer data with its company ID, 29 of the 31 bytes.
    fn advertisement(payload: &[u8]) -> Vec<u8> {
        let mut ad = vec![
            0x02,
            0x01,
            0x06,
            (payload.len() + 3) as u8,
            0xFF,
            0x02,
            0x07,
        ];
        ad.extend_from_slice(payload);
        ad
    }

    #[test]
    fn reads_the_upstream_vector() {
        let reading = parse(&ARANET4).expect("a full reading");
        assert_eq!(reading.kind, Kind::Aranet4);
        assert_eq!(reading.co2, Some(1091));
        assert_eq!(reading.hundredths, Some(2075));
        // Aranet4-Python rounds 20.75 to 20.8.
        assert_eq!(reading.tenths(), Some(208));
        assert_eq!(reading.quarters(), Some(83));
        assert_eq!(reading.pressure, Some(10123));
        assert_eq!(reading.humidity, Some(53));
        assert_eq!(reading.battery, Some(12));
        assert_eq!(reading.band, Some(Band::Fair));
        assert_eq!(reading.interval, 60);
        assert_eq!(reading.age, 16);
    }

    /// Whole advertisements (`ADV_IND`, from static random addresses) as a
    /// dev kit heard them on 2026-11-08 from two Aranet4s on firmware
    /// v2.0.7, one measuring every 2 minutes, one every 5; each advertised
    /// every ~0.57 s.
    const HEARD: [(&[u8], Reading); 2] = [
        (
            &[
                0x02, 0x01, 0x06, 0x19, 0xff, 0x02, 0x07, 0x21, 0x07, 0x00, 0x02, 0x00, 0x0c, 0x0f,
                0x01, 0xa5, 0x02, 0xe1, 0x01, 0xc6, 0x27, 0x1c, 0x1b, 0x01, 0x78, 0x00, 0x60, 0x00,
                0xaa,
            ],
            Reading {
                kind: Kind::Aranet4,
                co2: Some(677),
                hundredths: Some(2405),
                pressure: Some(10182),
                humidity: Some(28),
                battery: Some(27),
                band: Some(Band::Good),
                interval: 120,
                age: 96,
            },
        ),
        (
            &[
                0x02, 0x01, 0x06, 0x19, 0xff, 0x02, 0x07, 0x21, 0x07, 0x00, 0x02, 0x00, 0x09, 0x0f,
                0x01, 0x89, 0x02, 0xbb, 0x01, 0xb9, 0x27, 0x20, 0x17, 0x01, 0x2c, 0x01, 0x01, 0x00,
                0xb9,
            ],
            Reading {
                kind: Kind::Aranet4,
                co2: Some(649),
                hundredths: Some(2215),
                pressure: Some(10169),
                humidity: Some(32),
                battery: Some(23),
                band: Some(Band::Good),
                interval: 300,
                age: 1,
            },
        ),
    ];

    #[test]
    fn reads_what_a_dev_kit_heard() {
        for (ad, expected) in HEARD {
            let payload = manufacturer_payload(ad).expect("Aranet's data");
            assert_eq!(parse(payload), Ok(expected));
        }
        // 24.05 and 22.15 °C, to the tenth (rounded half up) and quarter.
        assert_eq!(HEARD[0].1.tenths(), Some(241));
        assert_eq!(HEARD[1].1.tenths(), Some(222));
        assert_eq!(HEARD[0].1.quarters(), Some(96));
        assert_eq!(HEARD[1].1.quarters(), Some(89));
    }

    #[test]
    fn finds_the_payload_in_an_advertisement() {
        let ad = advertisement(&ARANET4);
        assert_eq!(ad.len(), 29);
        assert_eq!(manufacturer_payload(&ad), Some(&ARANET4[..]));
        // Padding after it doesn't matter.
        let mut padded = ad.clone();
        padded.extend_from_slice(&[0, 0]);
        assert_eq!(manufacturer_payload(&padded), Some(&ARANET4[..]));
    }

    #[test]
    fn ignores_other_advertisements() {
        // Another company's data (Apple's 0x004C), and a name.
        let other = [
            0x02, 0x01, 0x06, 0x05, 0xFF, 0x4C, 0x00, 0x01, 0x02, 0x04, 0x09, b'u', b'r', b'!',
        ];
        assert_eq!(manufacturer_payload(&other), None);
        // Manufacturer data too short for a company ID.
        assert_eq!(manufacturer_payload(&[0x02, 0xFF, 0x02]), None);
        assert_eq!(manufacturer_payload(&[]), None);
        // A zero length ends the data, even with Aranet's after it.
        let mut early_end = vec![0x00];
        early_end.extend_from_slice(&advertisement(&ARANET4));
        assert_eq!(manufacturer_payload(&early_end), None);
        // Truncated: claims more than there is.
        let ad = advertisement(&ARANET4);
        assert_eq!(manufacturer_payload(&ad[..ad.len() - 1]), None);
    }

    #[test]
    fn says_why_there_is_no_reading() {
        // Upstream's "short", "old_fw", "no_integrations" and "bad" vectors.
        assert_eq!(parse(&ARANET4[..20]), Err(Skip::Malformed));
        assert_eq!(
            parse(&[0x21, 0x0a, 0x04, 0x00, 0x00, 0x00, 0x00]),
            Err(Skip::OldFirmware)
        );
        assert_eq!(
            parse(&[0x01, 0x00, 0x02, 0x01, 0x00, 0x00, 0x00]),
            Err(Skip::IntegrationsOff)
        );
        assert_eq!(parse(&[0x01, 0x00, 0x02]), Err(Skip::Malformed));
        assert_eq!(parse(&[]), Err(Skip::Malformed));
        // A full-length payload with the integrations bit clear.
        let mut off = ARANET4;
        off[0] &= !0x20;
        assert_eq!(parse(&off), Err(Skip::IntegrationsOff));
    }

    #[test]
    fn other_aranets_are_another_model() {
        // Upstream's Aranet2, Radiation, and Radon+ vectors.
        let aranet2 = [
            0x01, 0x21, 0x04, 0x04, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x99, 0x01, 0x00, 0x00,
            0x0a, 0x02, 0x00, 0x3b, 0x09, 0x78, 0x00, 0x52, 0x00, 0x64,
        ];
        let radiation = [
            0x02, 0x21, 0x26, 0x04, 0x01, 0x00, 0xd0, 0x33, 0x00, 0x00, 0x6c, 0x60, 0x06, 0x00,
            0x82, 0x00, 0x00, 0x63, 0x00, 0x2c, 0x01, 0x58, 0x00, 0x72,
        ];
        let radon = [
            0x03, 0x21, 0x04, 0x06, 0x01, 0x00, 0x00, 0x00, 0x07, 0x00, 0xfe, 0x01, 0xc9, 0x27,
            0xce, 0x01, 0x00, 0x64, 0x01, 0x58, 0x02, 0xf6, 0x01, 0x08,
        ];
        for payload in [aranet2, radiation, radon] {
            assert_eq!(parse(&payload), Err(Skip::OtherModel));
        }
    }

    #[test]
    fn invalid_fields_are_none() {
        let mut calibrating = ARANET4;
        calibrating[9] |= 0x80;
        let reading = parse(&calibrating).expect("still a reading");
        assert_eq!(reading.co2, None);
        // The status byte still gives the band.
        assert_eq!(reading.band, Some(Band::Fair));
        let mut no_temperature = ARANET4;
        no_temperature[11] |= 0x40;
        assert_eq!(parse(&no_temperature).expect("a reading").tenths(), None);
        let mut no_pressure = ARANET4;
        no_pressure[13] |= 0x80;
        assert_eq!(parse(&no_pressure).expect("a reading").pressure, None);
    }

    #[test]
    fn without_a_status_the_default_thresholds_band() {
        let band = |ppm: u16| {
            let mut payload = ARANET4;
            payload[8..10].copy_from_slice(&ppm.to_le_bytes());
            payload[16] = 0;
            parse(&payload).expect("a reading").band
        };
        assert_eq!(band(999), Some(Band::Good));
        assert_eq!(band(1000), Some(Band::Fair));
        assert_eq!(band(1400), Some(Band::Fair));
        assert_eq!(band(1401), Some(Band::Poor));
    }

    #[test]
    fn temperatures_convert_to_quarters_and_tenths() {
        let reading = |twentieths: u16| {
            let mut payload = ARANET4;
            payload[10..12].copy_from_slice(&twentieths.to_le_bytes());
            parse(&payload).expect("a reading")
        };
        // 21.0, 21.05, 21.1, 21.15 °C.
        assert_eq!(reading(420).tenths(), Some(210));
        assert_eq!(reading(421).tenths(), Some(211));
        assert_eq!(reading(422).tenths(), Some(211));
        assert_eq!(reading(423).tenths(), Some(212));
        // 21.0, 21.1 (nearest 21.0), 21.15 (nearest 21.25), 21.25 °C.
        assert_eq!(reading(420).quarters(), Some(84));
        assert_eq!(reading(422).quarters(), Some(84));
        assert_eq!(reading(423).quarters(), Some(85));
        assert_eq!(reading(425).quarters(), Some(85));
        // Below zero (a BTHome thermometer outdoors), still half up:
        // -5.25 °C is -5.2, and -5.25 to the quarter.
        let cold = Reading {
            hundredths: Some(-525),
            ..reading(0)
        };
        assert_eq!(cold.tenths(), Some(-52));
        assert_eq!(cold.quarters(), Some(-21));
        // Too hot to fit: no temperature, rather than a wrong one.
        assert_eq!(reading(0x3FFF).hundredths, None);
    }

    quickcheck! {
        /// Whatever arrives over the air, nothing panics.
        fn never_panics(bytes: Vec<u8>) -> bool {
            let _ = manufacturer_payload(&bytes).map(parse);
            let _ = parse(&bytes);
            true
        }
    }

    // ==========================================================================
    // Choosing and scheduling
    // ==========================================================================

    const MIN: u64 = 60 * 1000;
    const KITCHEN: Address = [1, 2, 3, 4, 5, 6];
    const BEDROOM: Address = [9, 8, 7, 6, 5, 4];

    fn reading() -> Reading {
        parse(&ARANET4).expect("a reading")
    }

    /// One scan at `now` hearing each `(address, rssi)` in turn.
    fn scan(nearby: &mut Nearby, now: u64, heard: &[(Address, i8)]) -> bool {
        nearby.begin(now);
        for &(address, rssi) in heard {
            nearby.hear(address, rssi, reading(), now + 500);
        }
        nearby.end(now + 1000)
    }

    #[test]
    fn the_strongest_is_shown() {
        let mut nearby = Nearby::new();
        assert!(scan(&mut nearby, 0, &[(BEDROOM, -80), (KITCHEN, -60)]));
        assert_eq!(nearby.best().map(|s| s.address), Some(KITCHEN));
        // Its strongest report counts, not its latest.
        let mut nearby = Nearby::new();
        scan(
            &mut nearby,
            0,
            &[(KITCHEN, -60), (BEDROOM, -70), (KITCHEN, -90)],
        );
        assert_eq!(nearby.best().map(|s| s.rssi), Some(-60));
    }

    #[test]
    fn a_slightly_stronger_one_does_not_take_over() {
        let mut nearby = Nearby::new();
        scan(&mut nearby, 0, &[(KITCHEN, -60), (BEDROOM, -70)]);
        // Fading: the bedroom's now 2 dB stronger. Still the kitchen.
        scan(&mut nearby, 15 * MIN, &[(KITCHEN, -65), (BEDROOM, -63)]);
        assert_eq!(nearby.best().map(|s| s.address), Some(KITCHEN));
        // Walked into the bedroom.
        scan(&mut nearby, 30 * MIN, &[(KITCHEN, -75), (BEDROOM, -55)]);
        assert_eq!(nearby.best().map(|s| s.address), Some(BEDROOM));
    }

    #[test]
    fn nothing_heard_keeps_the_last_reading_until_stale() {
        let mut nearby = Nearby::new();
        scan(&mut nearby, 0, &[(KITCHEN, -60)]);
        assert!(!scan(&mut nearby, 15 * MIN, &[]));
        assert_eq!(nearby.best().map(|s| s.address), Some(KITCHEN));
        // Heard at 0.5 s, measured 16 s before that.
        assert!(nearby.fresh(30 * MIN).is_some());
        assert!(nearby.fresh(35 * MIN).is_none());
    }

    #[test]
    fn a_scan_stops_once_everyone_recent_is_heard() {
        let mut nearby = Nearby::new();
        // Nothing known yet: the first sensor heard will do, once the scan
        // has had time to hear any nearer one.
        nearby.begin(0);
        assert!(!nearby.heard_everyone(0));
        nearby.hear(KITCHEN, -60, reading(), 100);
        assert!(!nearby.heard_everyone(100));
        nearby.hear(BEDROOM, -70, reading(), 200);
        assert!(nearby.heard_everyone(LOOK_AROUND));
        nearby.end(LOOK_AROUND);
        // Both known now: one isn't enough.
        nearby.begin(15 * MIN);
        nearby.hear(KITCHEN, -60, reading(), 15 * MIN + 100);
        assert!(!nearby.heard_everyone(15 * MIN + 100));
        nearby.hear(BEDROOM, -70, reading(), 15 * MIN + 200);
        assert!(nearby.heard_everyone(15 * MIN + 200));
        nearby.end(15 * MIN + 300);
        // Then only the kitchen, a while later.
        nearby.begin(45 * MIN);
        nearby.hear(KITCHEN, -60, reading(), 45 * MIN);
        nearby.end(45 * MIN + 100);
        // The last scan missed the bedroom: it isn't waited for, though
        // it's still around (and still counts if heard).
        assert!(nearby.around(60 * MIN));
        nearby.begin(60 * MIN);
        nearby.hear(KITCHEN, -60, reading(), 60 * MIN);
        assert!(nearby.heard_everyone(60 * MIN));
        nearby.hear(BEDROOM, -70, reading(), 60 * MIN + 100);
        nearby.end(60 * MIN + 200);
        // Heard again, it's waited for again.
        nearby.begin(75 * MIN);
        nearby.hear(KITCHEN, -60, reading(), 75 * MIN);
        assert!(!nearby.heard_everyone(75 * MIN));
        nearby.end(75 * MIN + SCAN_LIMIT);
        // An hour after the last one heard, nothing is known: it looks
        // around again.
        let much_later = 75 * MIN + RECENT;
        nearby.begin(much_later);
        nearby.hear(KITCHEN, -60, reading(), much_later);
        assert!(!nearby.heard_everyone(much_later));
        assert!(nearby.heard_everyone(much_later + LOOK_AROUND));
    }

    #[test]
    fn a_scan_waits_longer_for_a_bthome_thermometer() {
        let thermometer = |irregular| Reading {
            kind: Kind::BTHome { irregular },
            ..reading()
        };
        let mut nearby = Nearby::new();
        // Finding sensors: the usual limit.
        nearby.begin(0);
        assert_eq!(nearby.limit(0), SCAN_LIMIT);
        nearby.hear(KITCHEN, -60, reading(), 100);
        nearby.hear(BEDROOM, -70, thermometer(false), 2000);
        nearby.end(2000);
        // The thermometer heard last time, it waits for it, longer.
        nearby.begin(15 * MIN);
        assert_eq!(nearby.limit(15 * MIN), SLOW_SCAN_LIMIT);
        nearby.hear(KITCHEN, -60, reading(), 15 * MIN + 100);
        assert!(!nearby.heard_everyone(15 * MIN + 100));
        nearby.end(15 * MIN + SLOW_SCAN_LIMIT);
        // Missed: the next waits for the Aranet4 alone, as long as it
        // would.
        nearby.begin(30 * MIN);
        assert_eq!(nearby.limit(30 * MIN), SCAN_LIMIT);
        nearby.hear(KITCHEN, -60, reading(), 30 * MIN + 100);
        assert!(nearby.heard_everyone(30 * MIN + 100));
        nearby.hear(BEDROOM, -50, thermometer(true), 30 * MIN + 200);
        nearby.end(30 * MIN + 200);
        // Any kind can be shown: the strongest.
        assert_eq!(nearby.best().map(|s| s.address), Some(BEDROOM));
        // One that says it advertises irregularly is never waited for.
        nearby.begin(45 * MIN);
        assert_eq!(nearby.limit(45 * MIN), SCAN_LIMIT);
        nearby.hear(KITCHEN, -60, reading(), 45 * MIN + 100);
        assert!(nearby.heard_everyone(45 * MIN + 100));
    }

    #[test]
    fn more_sensors_than_room_keeps_the_strongest() {
        let mut nearby = Nearby::new();
        nearby.begin(0);
        for i in 0..SENSORS as u8 {
            nearby.hear([i; 6], -90 + i as i8, reading(), 0);
        }
        nearby.hear([99; 6], -40, reading(), 0);
        nearby.hear([98; 6], -95, reading(), 0);
        nearby.end(0);
        assert_eq!(nearby.best().map(|s| s.address), Some([99; 6]));
    }

    fn worn() -> Context {
        Context::default()
    }

    #[test]
    fn scans_every_quarter_hour_with_a_sensor_around() {
        let mut nearby = Nearby::new();
        // At boot, at once.
        assert_eq!(nearby.due(0, worn()), Some(Why::Background));
        scan(&mut nearby, 0, &[(KITCHEN, -60)]);
        assert_eq!(nearby.due(14 * MIN, worn()), None);
        assert_eq!(nearby.due(15 * MIN, worn()), Some(Why::Background));
    }

    #[test]
    fn backs_off_to_hourly_with_nothing_around() {
        let mut nearby = Nearby::new();
        scan(&mut nearby, 0, &[]);
        assert_eq!(nearby.due(15 * MIN, worn()), None);
        assert_eq!(nearby.due(59 * MIN, worn()), None);
        assert_eq!(nearby.due(60 * MIN, worn()), Some(Why::Background));
        // An hour after the last one heard, too.
        let mut nearby = Nearby::new();
        scan(&mut nearby, 0, &[(KITCHEN, -60)]);
        let mut at = 0;
        for _ in 0..4 {
            at += 15 * MIN;
            assert_eq!(nearby.due(at, worn()), Some(Why::Background));
            scan(&mut nearby, at, &[]);
        }
        assert_eq!(nearby.due(at + 15 * MIN, worn()), None);
        assert_eq!(nearby.due(at + 60 * MIN, worn()), Some(Why::Background));
    }

    #[test]
    fn no_background_scans_at_night_off_the_wrist_or_updating() {
        let mut nearby = Nearby::new();
        // Off the wrist, one at boot, to find out what's around.
        let off = Context {
            off_wrist: true,
            ..worn()
        };
        assert_eq!(nearby.due(0, off), Some(Why::Background));
        scan(&mut nearby, 0, &[(KITCHEN, -60)]);
        let later = 2 * SELDOM;
        let quiet = Context {
            quiet_night: true,
            ..worn()
        };
        let updating = Context {
            updating: true,
            showing_air: true,
            ..worn()
        };
        for context in [quiet, off, updating] {
            assert_eq!(nearby.due(later, context), None);
        }
        assert_eq!(nearby.due(later, worn()), Some(Why::Background));
    }

    #[test]
    fn looking_brings_a_scan_forward() {
        let mut nearby = Nearby::new();
        scan(&mut nearby, 0, &[(KITCHEN, -60)]);
        let clock = Context {
            showing_clock: true,
            off_wrist: true,
            ..worn()
        };
        assert_eq!(nearby.due(9 * MIN, clock), None);
        assert_eq!(nearby.due(10 * MIN, clock), Some(Why::Clock));
        // Not without a sensor around: nothing to refresh.
        let mut empty = Nearby::new();
        scan(&mut empty, 0, &[]);
        assert_eq!(empty.due(10 * MIN, clock), None);
        // The Air screen asks for itself, sensor or not, once a minute.
        let air = Context {
            showing_air: true,
            quiet_night: true,
            ..worn()
        };
        assert_eq!(empty.due(MIN - 1, air), None);
        assert_eq!(empty.due(MIN, air), Some(Why::Screen));
    }

    #[test]
    fn a_scan_under_way_is_not_asked_for_again() {
        let mut nearby = Nearby::new();
        nearby.begin(0);
        assert_eq!(nearby.due(2 * SELDOM, worn()), None);
        nearby.end(1000);
        assert_eq!(nearby.due(2 * SELDOM, worn()), Some(Why::Background));
    }

    #[test]
    fn age_counts_from_the_measurement() {
        let mut nearby = Nearby::new();
        scan(&mut nearby, 0, &[(KITCHEN, -60)]);
        let best = nearby.best().expect("heard");
        // Measured 16 s before it was heard at 0.5 s.
        assert_eq!(best.age(500), 16);
        assert_eq!(best.age(60_500), 76);
    }

    // ==========================================================================
    // Where the sensors are
    // ==========================================================================

    fn place(latitude: f64, longitude: f64) -> Place {
        Place {
            latitude_micro: (latitude * 1e6).round() as i32,
            longitude_micro: (longitude * 1e6).round() as i32,
        }
    }

    /// Metres, against the haversine's (on a sphere of 6371 km), within
    /// `percent`.
    fn about(a: Place, b: Place, haversine: f32, percent: f32) {
        let metres = distance_squared(a, b).sqrt();
        assert!(
            (metres - haversine).abs() <= haversine * percent / 100.0,
            "{metres} m, not {haversine}"
        );
    }

    const HOME: (f64, f64) = (59.9139, 10.7522);

    #[test]
    fn distances_are_near_enough() {
        let home = place(HOME.0, HOME.1);
        // 0.009° east in Oslo, 0.01° in Tromsø (where a degree east is
        // shortest of any Norwegian town), the short way across 180°, and
        // Oslo to Bergen: far beyond what it's for, but still near.
        about(home, place(59.9139, 10.7612), 501.7, 1.0);
        about(place(69.6492, 18.9553), place(69.6492, 18.9653), 386.7, 2.0);
        about(place(0.0, 179.999), place(0.0, -179.999), 222.4, 1.0);
        about(home, place(60.3913, 5.3221), 305_067.0, 1.0);
        // North is the same everywhere: 500 m is 0.0045°.
        about(home, place(59.9184, 10.7522), 500.4, 0.5);
        assert_eq!(distance_squared(home, home), 0.0);
    }

    #[test]
    fn far_from_the_sensors_forgets_them() {
        let home = place(HOME.0, HOME.1);
        let across_town = place(59.9439, 10.7522);
        let next_door = place(59.9142, 10.7522);
        let mut nearby = Nearby::new();
        // Heard at home, and the phone there too: the sensors' place.
        scan(&mut nearby, 0, &[(KITCHEN, -60)]);
        nearby.located(home, 5 * MIN);
        // Left, unheard since; the phone, an hour later, is across town.
        scan(&mut nearby, 15 * MIN, &[]);
        assert!(nearby.around(20 * MIN));
        assert!(nearby.located(across_town, 20 * MIN));
        assert!(!nearby.around(20 * MIN));
        // Nothing more to forget.
        assert!(!nearby.located(across_town, 25 * MIN));
        assert!(nearby.best().is_none());
        // So the scans go hourly at once, and wait for nobody.
        assert_eq!(nearby.due(30 * MIN, worn()), None);
        assert_eq!(nearby.due(75 * MIN, worn()), Some(Why::Background));
        nearby.begin(75 * MIN);
        assert_eq!(nearby.limit(75 * MIN), SCAN_LIMIT);
        nearby.end(75 * MIN + SCAN_LIMIT);
        // Back home, a fix there scans at once.
        nearby.located(next_door, 80 * MIN);
        assert_eq!(nearby.due(80 * MIN, worn()), Some(Why::Background));
    }

    #[test]
    fn a_fix_near_them_changes_nothing() {
        let home = place(HOME.0, HOME.1);
        let mut nearby = Nearby::new();
        scan(&mut nearby, 0, &[(KITCHEN, -60)]);
        nearby.located(home, 5 * MIN);
        scan(&mut nearby, 15 * MIN, &[]);
        // Within the threshold (a Wi-Fi fix's error): still around.
        nearby.located(place(59.9175, 10.7522), 20 * MIN);
        assert!(nearby.around(20 * MIN));
        assert!(nearby.best().is_some());
        // And not a scan brought forward, with a sensor known.
        assert_eq!(nearby.due(25 * MIN, worn()), None);
    }

    #[test]
    fn without_a_place_a_fix_changes_nothing() {
        let mut nearby = Nearby::new();
        // Heard, but no fix while it was near.
        scan(&mut nearby, 0, &[(KITCHEN, -60)]);
        nearby.located(place(HOME.0, HOME.1), 40 * MIN);
        nearby.located(place(48.8566, 2.3522), 50 * MIN);
        assert!(nearby.around(50 * MIN));
    }

    #[test]
    fn a_fix_near_a_sensor_heard_moves_the_place() {
        let mut nearby = Nearby::new();
        scan(&mut nearby, 0, &[(KITCHEN, -60)]);
        nearby.located(place(HOME.0, HOME.1), 5 * MIN);
        // At work, with a sensor there, and a fix (a new time zone, say).
        let work = place(59.95, 10.70);
        scan(&mut nearby, 15 * MIN, &[(BEDROOM, -60)]);
        nearby.located(work, 20 * MIN);
        scan(&mut nearby, 30 * MIN, &[]);
        // A fix at work again: the sensors are still around.
        nearby.located(work, 40 * MIN);
        assert!(nearby.around(40 * MIN));
    }
}
