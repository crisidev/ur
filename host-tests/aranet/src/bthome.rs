//! BTHome thermometers' advertisements: the spec's own example, Home
//! Assistant's parser's test vectors (`bthome-ble`, `tests/test_parser_v2.py`
//! and the object table in `src/bthome_ble/const.py`, fetched 2026-11-08),
//! adverts built as pvvx's firmware and Shelly's documentation say they're
//! laid out, the watch's own (encrypted) button press, and garbage.

use crate::aranet::{Band, Kind, find};
use crate::bthome_sensor::*;
use quickcheck::quickcheck;

/// The service data (after the UUID) of `hex`, a whole advertisement.
fn service(ad: &[u8]) -> &[u8] {
    service_data(ad).expect("BTHome service data")
}

/// bthome.io/format's example: flags, the name "DIY-sensor", and
/// 25.00 °C, 50.55 %.
const SPEC_EXAMPLE: [u8; 26] = [
    0x02, 0x01, 0x06, 0x0B, 0x09, 0x44, 0x49, 0x59, 0x2D, 0x73, 0x65, 0x6E, 0x73, 0x6F, 0x72, 0x0A,
    0x16, 0xD2, 0xFC, 0x40, 0x02, 0xC4, 0x09, 0x03, 0xBF, 0x13,
];

#[test]
fn reads_the_spec_example() {
    let reading = parse(service(&SPEC_EXAMPLE)).expect("a temperature");
    assert_eq!(reading.kind, Kind::BTHome { irregular: false });
    assert_eq!(reading.hundredths, Some(2500));
    assert_eq!(reading.tenths(), Some(250));
    assert_eq!(reading.quarters(), Some(100));
    // 50.55 % rounds to 51.
    assert_eq!(reading.humidity, Some(51));
    assert_eq!(reading.battery, None);
    assert_eq!(reading.co2, None);
    assert_eq!(reading.band, None);
    assert_eq!(reading.pressure, None);
    assert_eq!(reading.age, 0);
}

#[test]
fn reads_a_pvvx_thermometer() {
    // bthome-ble's `test_bthome_packet_id_temperature_humidity_battery`,
    // from an "ATC_8D18B2" (a LYWSD03MMC on pvvx's firmware): packet id
    // 9, battery 93 %, 23.97 °C, 63.27 %. The same objects, in the same
    // order, as pvvx's `bthome_beacon.c` (`set_bthome_data1`) sends.
    let reading = parse(&[
        0x40, 0x00, 0x09, 0x01, 0x5D, 0x02, 0x5D, 0x09, 0x03, 0xB7, 0x18,
    ])
    .expect("a temperature");
    assert_eq!(reading.hundredths, Some(2397));
    assert_eq!(reading.tenths(), Some(240));
    assert_eq!(reading.humidity, Some(63));
    assert_eq!(reading.battery, Some(93));

    // pvvx's other advert, alternating with that one: packet id, battery
    // voltage (2.972 V), and its trigger output (`set_bthome_data2`).
    // Nothing to show, and not a sensor heard.
    let flags_and_voltage = [
        0x02, 0x01, 0x06, 0x0B, 0x16, 0xD2, 0xFC, 0x40, 0x00, 0x7C, 0x0C, 0x9C, 0x0B, 0x10, 0x00,
    ];
    assert_eq!(parse(service(&flags_and_voltage)), Err(Skip::NoTemperature));
}

#[test]
fn reads_shelly_thermometers() {
    // bthome-ble's `test_shelly_blu_ht_display_zb`: packet id, battery
    // 100 %, light on, 52 %, 27.8 °C (0x45, in tenths).
    let reading = parse(&[
        0x40, 0x00, 0x2E, 0x01, 0x64, 0x1E, 0x01, 0x2E, 0x34, 0x45, 0x16, 0x01,
    ])
    .expect("a temperature");
    assert_eq!(reading.hundredths, Some(2780));
    assert_eq!(reading.humidity, Some(52));
    assert_eq!(reading.battery, Some(100));

    // A Shelly BLU H&T, as its documentation lists its objects: packet
    // id, battery, humidity (whole percent), button (none pressed), and
    // the temperature in tenths: 21.4 °C, 48 %, 97 %.
    let reading = parse(&[
        0x40, 0x00, 0x51, 0x01, 0x61, 0x2E, 0x30, 0x3A, 0x00, 0x45, 0xD6, 0x00,
    ])
    .expect("a temperature");
    assert_eq!(reading.hundredths, Some(2140));
    assert_eq!(reading.humidity, Some(48));
    assert_eq!(reading.battery, Some(97));

    // bthome-ble's `test_bthome_temperature_2`, a Shelly BLU TRV
    // ("SBTR-001AEU"): 27.3 °C alone.
    let reading = parse(&[0x40, 0x45, 0x11, 0x01]).expect("a temperature");
    assert_eq!(reading.hundredths, Some(2730));
    assert_eq!(reading.humidity, None);
}

#[test]
fn humidity_stays_a_percentage() {
    // 655.35 %, as a u16 of hundredths can say: 100, not wrapped.
    let reading = parse(&[0x40, 0x02, 0xCA, 0x09, 0x03, 0xFF, 0xFF]).expect("a temperature");
    assert_eq!(reading.humidity, Some(100));
}

#[test]
fn reads_below_zero() {
    // -5.25 °C (0x02), and -12.3 °C (0x45).
    let reading = parse(&[0x40, 0x02, 0xF3, 0xFD]).expect("a temperature");
    assert_eq!(reading.hundredths, Some(-525));
    assert_eq!(reading.tenths(), Some(-52));
    let reading = parse(&[0x40, 0x45, 0x85, 0xFF]).expect("a temperature");
    assert_eq!(reading.hundredths, Some(-1230));
    // Tenths beyond what hundredths hold saturate rather than wrap.
    let reading = parse(&[0x40, 0x45, 0xFF, 0x7F]).expect("a temperature");
    assert_eq!(reading.hundredths, Some(i16::MAX));
}

#[test]
fn reads_pressure_and_co2_too() {
    // Packet id, battery 80 %, 21.50 °C, 40.00 %, 1013.25 hPa, 1203 ppm:
    // a CO₂ sensor on BTHome gets the Aranet4's default bands.
    let reading = parse(&[
        0x40, 0x00, 0x01, 0x01, 0x50, 0x02, 0x66, 0x08, 0x03, 0xA0, 0x0F, 0x04, 0xCD, 0x8B, 0x01,
        0x12, 0xB3, 0x04,
    ])
    .expect("a temperature");
    assert_eq!(reading.hundredths, Some(2150));
    assert_eq!(reading.humidity, Some(40));
    assert_eq!(reading.pressure, Some(10133));
    assert_eq!(reading.co2, Some(1203));
    assert_eq!(reading.band, Some(Band::Fair));
}

#[test]
fn the_first_of_several_temperatures_counts() {
    // bthome-ble's `test_bthome_double_temperature`: 25.06 and 25.11 °C.
    let reading = parse(&[0x40, 0x02, 0xCA, 0x09, 0x02, 0xCF, 0x09]).expect("a temperature");
    assert_eq!(reading.hundredths, Some(2506));
    // Tenths after hundredths: still the first.
    let reading = parse(&[0x40, 0x02, 0xCA, 0x09, 0x45, 0x11, 0x01]).expect("a temperature");
    assert_eq!(reading.hundredths, Some(2506));
}

#[test]
fn objects_it_does_not_want_are_skipped_by_length() {
    // bthome-ble's `..._firmware_version_4_...`: 25.06 °C, then a 4-byte
    // firmware version.
    assert!(parse(&[0x40, 0x02, 0xCA, 0x09, 0xF1, 0x00, 0x01, 0x02, 0x04]).is_ok());
    // Before the temperature: a 3-byte illuminance, a 4-byte count, a
    // text ("hi", with its length), raw data (3 bytes), a command
    // (opcode 7 and 2 arguments, its length byte counting the arguments),
    // then 25.06 °C.
    let reading = parse(&[
        0x40, 0x05, 0x01, 0x02, 0x03, 0x3E, 0x01, 0x02, 0x03, 0x04, 0x53, 0x02, b'h', b'i', 0x54,
        0x03, 0xAA, 0xBB, 0xCC, 0x3B, 0x02, 0x07, 0xEE, 0xFF, 0x02, 0xCA, 0x09,
    ])
    .expect("a temperature");
    assert_eq!(reading.hundredths, Some(2506));
}

#[test]
fn an_unknown_object_stops_the_walk() {
    // bthome-ble's `test_bthome_battery_wrong_object_id_humidity`: battery
    // 93 %, an id that doesn't exist (0xFE), then a humidity that can't be
    // found. No temperature.
    assert_eq!(
        parse(&[0x40, 0x01, 0x5D, 0xFE, 0x5D, 0x09, 0x03, 0xB7, 0x18]),
        Err(Skip::NoTemperature)
    );
    // A temperature before it still counts; the humidity after it doesn't.
    let reading = parse(&[0x40, 0x02, 0xCA, 0x09, 0x30, 0x03, 0xB7, 0x18]).expect("a temperature");
    assert_eq!(reading.hundredths, Some(2506));
    assert_eq!(reading.humidity, None);
    // As does a truncated object: the temperature here lacks a byte.
    assert_eq!(
        parse(&[0x40, 0x01, 0x5D, 0x02, 0xCA]),
        Err(Skip::NoTemperature)
    );
    // And text claiming more than there is.
    let reading = parse(&[0x40, 0x02, 0xCA, 0x09, 0x53, 0x09, b'h']).expect("a temperature");
    assert_eq!(reading.hundredths, Some(2506));
}

#[test]
fn the_object_table_is_bthome_bles() {
    // Every fixed-length object in bthome-ble's `MEAS_TYPES`, and its length.
    const TABLE: [(u8, usize); 92] = [
        (0x00, 1),
        (0x01, 1),
        (0x02, 2),
        (0x03, 2),
        (0x04, 3),
        (0x05, 3),
        (0x06, 2),
        (0x07, 2),
        (0x08, 2),
        (0x09, 1),
        (0x0A, 3),
        (0x0B, 3),
        (0x0C, 2),
        (0x0D, 2),
        (0x0E, 2),
        (0x0F, 1),
        (0x10, 1),
        (0x11, 1),
        (0x12, 2),
        (0x13, 2),
        (0x14, 2),
        (0x15, 1),
        (0x16, 1),
        (0x17, 1),
        (0x18, 1),
        (0x19, 1),
        (0x1A, 1),
        (0x1B, 1),
        (0x1C, 1),
        (0x1D, 1),
        (0x1E, 1),
        (0x1F, 1),
        (0x20, 1),
        (0x21, 1),
        (0x22, 1),
        (0x23, 1),
        (0x24, 1),
        (0x25, 1),
        (0x26, 1),
        (0x27, 1),
        (0x28, 1),
        (0x29, 1),
        (0x2A, 1),
        (0x2B, 1),
        (0x2C, 1),
        (0x2D, 1),
        (0x2E, 1),
        (0x2F, 1),
        (0x3A, 1),
        (0x3C, 2),
        (0x3D, 2),
        (0x3E, 4),
        (0x3F, 2),
        (0x40, 2),
        (0x41, 2),
        (0x42, 3),
        (0x43, 2),
        (0x44, 2),
        (0x45, 2),
        (0x46, 1),
        (0x47, 2),
        (0x48, 2),
        (0x49, 2),
        (0x4A, 2),
        (0x4B, 3),
        (0x4C, 4),
        (0x4D, 4),
        (0x4E, 4),
        (0x4F, 4),
        (0x50, 4),
        (0x51, 2),
        (0x52, 2),
        (0x55, 4),
        (0x56, 2),
        (0x57, 1),
        (0x58, 1),
        (0x59, 1),
        (0x5A, 2),
        (0x5B, 4),
        (0x5C, 4),
        (0x5D, 2),
        (0x5E, 2),
        (0x5F, 2),
        (0x60, 1),
        (0x61, 2),
        (0x62, 4),
        (0x63, 4),
        (0x64, 1),
        (0x65, 1),
        (0xF0, 2),
        (0xF1, 4),
        (0xF2, 3),
    ];
    for id in 0..=u8::MAX {
        let expected = match id {
            // The three with a length byte.
            0x53 | 0x54 => Some(1 + 0xE5),
            0x3B => Some(2 + (0xE5 & 0x1F)),
            _ => TABLE.iter().find(|&&(i, _)| i == id).map(|&(_, n)| n),
        };
        assert_eq!(object_length(id, &[0xE5]), expected, "object {id:#04x}");
    }
    // Without the length byte, they can't be walked.
    assert_eq!(object_length(0x53, &[]), None);
}

#[test]
fn a_mac_may_come_first() {
    // bthome-ble's `test_bthome_with_mac`: the MAC, then a pressure
    // (1008.83 hPa) and no temperature.
    let with_mac = [
        0x42, 0xB2, 0x18, 0x8D, 0x38, 0xC1, 0xA4, 0x04, 0x13, 0x8A, 0x01,
    ];
    assert_eq!(parse(&with_mac), Err(Skip::NoTemperature));
    // With a temperature after the MAC.
    let reading = parse(&[0x42, 0xB2, 0x18, 0x8D, 0x38, 0xC1, 0xA4, 0x02, 0xCA, 0x09])
        .expect("a temperature");
    assert_eq!(reading.hundredths, Some(2506));
    // Too short for one.
    assert_eq!(parse(&[0x42, 0xB2, 0x18]), Err(Skip::Malformed));
}

#[test]
fn a_trigger_based_device_says_so() {
    // bthome-ble's `test_sleepy_device`'s flags, with a temperature.
    let reading = parse(&[0x44, 0x02, 0xCA, 0x09]).expect("a temperature");
    assert_eq!(reading.kind, Kind::BTHome { irregular: true });
}

#[test]
fn encrypted_and_other_versions_are_passed_over() {
    // bthome-ble's `test_bindkey_correct`: a Shelly BLU H&T, encrypted
    // (25.06 °C and 50.55 % inside).
    let shelly = [
        0x41, 0xA4, 0x72, 0x66, 0xC9, 0x5F, 0x73, 0x00, 0x11, 0x22, 0x33, 0x78, 0x23, 0x72, 0x14,
    ];
    assert_eq!(parse(&shelly), Err(Skip::Encrypted));
    // BTHome v1's device information (version 1), and a version to come.
    assert_eq!(parse(&[0x20, 0x02, 0xCA, 0x09]), Err(Skip::Version));
    assert_eq!(parse(&[0x60, 0x02, 0xCA, 0x09]), Err(Skip::Version));
    assert_eq!(parse(&[]), Err(Skip::Malformed));
}

#[test]
fn the_watch_passes_over_its_own_presses() {
    // A whole advertisement as `bthome::advert` builds one (a tap of
    // code 23: packet id, battery 82 %, the button's press, and three
    // dimmers, the first turning right by 23; counter 256, bthome.io's
    // example key, address e5:12:34:56:78:9a), computed with Python's
    // `cryptography`. The watch can't hear itself, but another watch's
    // presses (or its own, if it could) are encrypted, so nothing here
    // would take them for a thermometer.
    let press = [
        0x02, 0x01, 0x06, 0x1B, 0x16, 0xD2, 0xFC, 0x45, 0xB3, 0x46, 0x27, 0xEC, 0xA8, 0x95, 0x75,
        0x78, 0xCA, 0xBA, 0x59, 0x39, 0x02, 0x4C, 0xC4, 0x00, 0x01, 0x00, 0x00, 0xA0, 0x7D, 0x23,
        0xB5,
    ];
    assert_eq!(parse(service(&press)), Err(Skip::Encrypted));
    // Even unencrypted, its objects walk through (a 2-byte dimmer after
    // a 1-byte button), and there's no temperature.
    let plain = [
        0x40, 0x00, 0x00, 0x01, 0x52, 0x3A, 0x01, 0x3C, 0x02, 0x23, 0x3C, 0x00, 0x00, 0x3C, 0x00,
        0x00, 0x02, 0xCA, 0x09,
    ];
    assert_eq!(parse(&plain[..16]), Err(Skip::NoTemperature));
    // (With a temperature after them, it would be found: they were
    // walked by their lengths.)
    assert_eq!(parse(&plain).map(|r| r.hundredths), Ok(Some(2506)));
    assert_eq!(crate::aranet::manufacturer_payload(&press), None);
}

#[test]
fn finds_the_service_data() {
    // Other service data (the Eddystone UUID, 0xFEAA) first, then BTHome.
    let ad = [
        0x02, 0x01, 0x06, 0x05, 0x16, 0xAA, 0xFE, 0x10, 0x00, 0x07, 0x16, 0xD2, 0xFC, 0x40, 0x02,
        0xCA, 0x09,
    ];
    assert_eq!(service_data(&ad), Some(&[0x40, 0x02, 0xCA, 0x09][..]));
    // Not in an Aranet4's.
    assert_eq!(service_data(&[0x05, 0xFF, 0x02, 0x07, 0x21, 0x00]), None);
    // The finder takes the AD type into account, not just the bytes:
    // 0xFCD2 as manufacturer data isn't BTHome.
    assert_eq!(service_data(&[0x04, 0xFF, 0xD2, 0xFC, 0x40]), None);
    assert_eq!(
        find(&[0x04, 0xFF, 0xD2, 0xFC, 0x40], 0xFF, &[0xD2, 0xFC]),
        Some(&[0x40][..])
    );
}

quickcheck! {
    /// Whatever arrives over the air, nothing panics.
    fn never_panics(bytes: Vec<u8>) -> bool {
        let _ = service_data(&bytes).map(parse);
        let _ = parse(&bytes);
        true
    }

    /// Valid-looking service data, any objects: nothing panics, and a
    /// temperature, if found, is one that was there.
    fn finds_only_what_is_there(objects: Vec<u8>) -> bool {
        let mut data = vec![0x40];
        data.extend_from_slice(&objects);
        match parse(&data) {
            Ok(reading) => {
                let t = reading.hundredths.expect("Ok means a temperature");
                objects.windows(3).any(|w| {
                    let raw = i16::from_le_bytes([w[1], w[2]]);
                    (w[0] == 0x02 && raw == t) || (w[0] == 0x45 && raw.saturating_mul(10) == t)
                })
            }
            Err(skip) => skip == Skip::NoTemperature,
        }
    }
}
