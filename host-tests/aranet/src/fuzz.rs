//! Fuzzing, with Bolero: every advertisement the scan hears, from any
//! device nearby, goes through `find` and one of the two parsers, so they
//! are the watch's most exposed input. The quickcheck tests try a few
//! hundred random byte strings; these build adverts as the formats lay
//! them out (AD structures, an Aranet4's fields, BTHome objects), so the
//! parsers' every branch is reached, and check what they make of them
//! against the formats' documentation, not just that nothing panics.
//!
//! `cargo test` runs each harness over a few hundred random inputs (and
//! any saved in a corpus), so they're regression tests too. To fuzz one
//! properly (nightly, for the sanitizer):
//!
//! ```text
//! cargo +nightly bolero test fuzz::bthome_objects_read_as_the_spec_says --engine libfuzzer -T 10min
//! ```

use crate::aranet::{self, Address, Band, FLOOR_DBM, Kind, Nearby, Place, Reading};
use crate::bthome_sensor::{self, object_length};
use std::collections::HashMap;

// ==============================================================================
// What every reading promises
// ==============================================================================

/// What a reading promises, whichever parser made it, of whatever bytes.
fn check(reading: &Reading) {
    // Rounded half up, to tenths: the hundredths lie in
    // [10 t - 5, 10 t + 5).
    if let (Some(h), Some(t)) = (reading.hundredths, reading.tenths()) {
        let h = i32::from(h);
        assert!((10 * t - 5..10 * t + 5).contains(&h), "{h} to {t} tenths");
    }
    // And to quarters: 4 h in [100 q - 50, 100 q + 50).
    if let (Some(h), Some(q)) = (reading.hundredths, reading.quarters()) {
        let h = 4 * i32::from(h);
        assert!(
            (100 * q - 50..100 * q + 50).contains(&h),
            "{h} to {q} quarters"
        );
    }
    assert_eq!(reading.tenths().is_some(), reading.hundredths.is_some());
    assert_eq!(reading.quarters().is_some(), reading.hundredths.is_some());
    match reading.kind {
        // BTHome says neither, and has no status: the band is the CO₂'s.
        Kind::BTHome { .. } => {
            assert_eq!((reading.interval, reading.age), (0, 0));
            assert!(
                reading.hundredths.is_some(),
                "a BTHome reading without a temperature"
            );
            assert_eq!(reading.band, thresholds(reading.co2));
        }
        // An Aranet4's band is its status, or the CO₂'s without one.
        Kind::Aranet4 => {
            if reading.band.is_none() {
                assert!(reading.co2.is_none());
            }
        }
    }
}

/// The Aranet4's default thresholds: below 1000 ppm good, to 1400 fair.
fn thresholds(co2: Option<u16>) -> Option<Band> {
    co2.map(|ppm| match ppm {
        0..1000 => Band::Good,
        1000..=1400 => Band::Fair,
        _ => Band::Poor,
    })
}

/// No advertisement panics either parser (or overflows: the arithmetic
/// checks are on), and what they read of it keeps [`check`]'s promises.
#[test]
fn any_advertisement_is_safe_to_read() {
    bolero::check!().for_each(|ad: &[u8]| {
        for payload in [aranet::manufacturer_payload(ad), Some(ad)]
            .into_iter()
            .flatten()
        {
            if let Ok(reading) = aranet::parse(payload) {
                assert_eq!(reading.kind, Kind::Aranet4);
                check(&reading);
            }
        }
        for data in [bthome_sensor::service_data(ad), Some(ad)]
            .into_iter()
            .flatten()
        {
            if let Ok(reading) = bthome_sensor::parse(data) {
                assert!(matches!(reading.kind, Kind::BTHome { .. }));
                check(&reading);
            }
        }
    });
}

// ==============================================================================
// The AD structures
// ==============================================================================

/// An advertisement of well-formed AD structures (a length, then a type
/// and its data), then any bytes, finds the first structure of a type
/// whose data starts with a prefix, as the Core Specification lays them
/// out (Vol 3, Part C, 11); and whatever it finds, in any bytes, is the
/// rest of a structure of that type, after the prefix.
#[test]
fn find_returns_the_first_matching_structure() {
    bolero::check!()
        .with_type::<(Vec<(u8, Vec<u8>)>, Vec<u8>, u8, u8)>()
        .for_each(|(structures, tail, pick, prefix_len)| {
            // At most 254 bytes of data each: the length byte counts the
            // type too.
            let structures: Vec<(u8, &[u8])> = structures
                .iter()
                .map(|(kind, data)| (*kind, &data[..data.len().min(254)]))
                .collect();
            let mut ad = Vec::new();
            for (kind, data) in &structures {
                ad.push(1 + data.len() as u8);
                ad.push(*kind);
                ad.extend_from_slice(data);
            }
            let clean = ad.len();
            ad.extend_from_slice(tail);

            // Look for a type, and a prefix, that one of them has (so
            // that there's often something to find), or else anything.
            let (kind, prefix): (u8, &[u8]) = match structures.get(usize::from(*pick)) {
                Some((kind, data)) => {
                    (*kind, &data[..data.len().min(usize::from(*prefix_len) % 4)])
                }
                None => (*pick, &tail[..tail.len().min(usize::from(*prefix_len) % 3)]),
            };
            let found = aranet::find(&ad, kind, prefix);

            // In the bytes it was given, after the type and the prefix.
            if let Some(found) = found {
                let start = found.as_ptr() as usize - ad.as_ptr() as usize;
                assert!(start + found.len() <= ad.len());
                assert!(start > prefix.len());
                assert_eq!(&ad[start - prefix.len()..start], prefix);
                assert_eq!(ad[start - prefix.len() - 1], kind);
            }

            // The first match among the clean structures, if there's one;
            // if not, something in the tail, or nothing.
            let first = structures
                .iter()
                .find(|(k, data)| *k == kind && data.starts_with(prefix))
                .map(|(_, data)| &data[prefix.len()..]);
            match first {
                Some(expected) => assert_eq!(found, Some(expected)),
                None => {
                    if let Some(found) = found {
                        assert!(found.as_ptr() as usize >= ad.as_ptr() as usize + clean);
                    }
                }
            }
        });
}

// ==============================================================================
// An Aranet4's fields
// ==============================================================================

/// An Aranet4's manufacturer data, field by field (see `aranet::parse`'s
/// table), reads as the table says: whole, in an advertisement too.
#[test]
fn aranet4_fields_read_as_the_table_says() {
    bolero::check!()
        .with_type::<([u8; 8], [u16; 3], [u8; 3], [u16; 2], u8)>()
        .for_each(
            |(
                header,
                [co2, twentieths, pressure],
                [humidity, battery, status],
                [interval, age],
                counter,
            )| {
                let mut payload = header.to_vec();
                payload.extend_from_slice(&co2.to_le_bytes());
                payload.extend_from_slice(&twentieths.to_le_bytes());
                payload.extend_from_slice(&pressure.to_le_bytes());
                payload.extend_from_slice(&[*humidity, *battery, *status]);
                payload.extend_from_slice(&interval.to_le_bytes());
                payload.extend_from_slice(&age.to_le_bytes());
                payload.push(*counter);
                assert_eq!(payload.len(), 22);

                // As an advertisement: flags, then the manufacturer data,
                // after SAF Tehnika's company ID (0x0702).
                let mut ad = vec![0x02, 0x01, 0x06, 3 + 22, 0xFF, 0x02, 0x07];
                ad.extend_from_slice(&payload);
                assert_eq!(aranet::manufacturer_payload(&ad), Some(&payload[..]));

                let parsed = aranet::parse(&payload);
                // Bit 5 of the flags: integrations on, or there are no readings.
                if header[0] & 1 << 5 == 0 {
                    assert_eq!(parsed, Err(aranet::Skip::IntegrationsOff));
                    return;
                }
                let reading = parsed.expect("integrations on, and 22 bytes");
                check(&reading);
                assert_eq!(reading.kind, Kind::Aranet4);
                // CO₂ and pressure invalid with bit 15 set.
                let co2 = (co2 & 0x8000 == 0).then_some(*co2);
                assert_eq!(reading.co2, co2);
                assert_eq!(
                    reading.pressure,
                    (pressure & 0x8000 == 0).then_some(*pressure)
                );
                // Twentieths of a degree, invalid with bit 14 set: the same
                // temperature, unless it doesn't fit (above 327.67 °C).
                match reading.hundredths {
                    Some(h) => {
                        assert_eq!(twentieths & 0x4000, 0);
                        assert_eq!(f64::from(h) / 100.0, f64::from(*twentieths) / 20.0);
                    }
                    None => {
                        assert!(twentieths & 0x4000 != 0 || f64::from(*twentieths) / 20.0 > 327.67)
                    }
                }
                assert_eq!(reading.humidity, Some(*humidity));
                assert_eq!(reading.battery, Some(*battery));
                // The device's colour, if it says one, or the default bands.
                let band = match status {
                    1 => Some(Band::Good),
                    2 => Some(Band::Fair),
                    3 => Some(Band::Poor),
                    _ => thresholds(co2),
                };
                assert_eq!(reading.band, band);
                assert_eq!((reading.interval, reading.age), (*interval, *age));
            },
        );
}

// ==============================================================================
// BTHome's objects
// ==============================================================================

/// How the service data a harness builds ends, after its objects.
#[derive(Clone, Copy, Debug)]
enum End {
    /// Right after the last.
    Clean,
    /// An object id the spec doesn't define, then anything: the walk
    /// stops there.
    Unknown,
    /// An object that claims more bytes than are left.
    Truncated,
}

/// BTHome v2 service data, built of objects (each an id, and its value as
/// long as the spec's table says), reads as bthome.io/format says of the
/// objects the watch wants: the first of each kind, in its unit, and the
/// objects after an id it doesn't know, or one cut short, don't count.
///
/// The objects' lengths come from `object_length`, which
/// `the_object_table_is_bthome_bles` checks against Home Assistant's
/// table; the values are read here with floats, rounded as documented.
#[test]
fn bthome_objects_read_as_the_spec_says() {
    bolero::check!()
        .with_type::<(u8, [u8; 6], Vec<(u8, [u8; 3], Vec<u8>)>, u8, Vec<u8>)>()
        .for_each(|(info, mac, objects, end, tail)| {
            // Mostly readable device information (unencrypted, v2), with
            // the MAC and trigger-based bits as they come; now and then,
            // anything.
            let info = if info & 0x80 == 0 {
                0x40 | (info & 0b110)
            } else {
                *info
            };
            let mut data = vec![info];
            if info & 0b10 != 0 {
                data.extend_from_slice(mac);
            }

            // The objects, and what the watch should make of them: the
            // first of each kind it wants. Ids are mostly those it wants,
            // so that several of one kind come up often.
            const WANTED: [u8; 7] = [0x01, 0x02, 0x45, 0x03, 0x2E, 0x04, 0x12];
            let mut temperature: Option<i16> = None;
            let mut humidity: Option<u8> = None;
            let mut pressure: Option<u16> = None;
            let mut co2: Option<u16> = None;
            let mut battery: Option<u8> = None;
            for (id, value, text) in objects {
                let id = if id & 0x80 != 0 {
                    WANTED[usize::from(id & 0x7F) % WANTED.len()]
                } else {
                    *id
                };
                // Text and raw data: a length byte, then that many bytes.
                // A command: an opcode and a length byte (of the
                // arguments, in its low five bits), then the arguments.
                let value: Vec<u8> = match id {
                    0x53 | 0x54 => {
                        let text = &text[..text.len().min(255)];
                        [&[text.len() as u8][..], text].concat()
                    }
                    0x3B => {
                        let arguments = &text[..text.len().min(31)];
                        [&[arguments.len() as u8, value[0]][..], arguments].concat()
                    }
                    _ => match object_length(id, &[]) {
                        Some(n) => value
                            .iter()
                            .copied()
                            .chain(text.iter().copied())
                            .chain([0; 4])
                            .take(n)
                            .collect(),
                        // One it doesn't know: the end, as `End::Unknown`.
                        None => break,
                    },
                };
                data.push(id);
                data.extend_from_slice(&value);
                let le = |n: usize| {
                    value[..n]
                        .iter()
                        .rev()
                        .fold(0u32, |sum, &b| sum << 8 | u32::from(b))
                };
                let signed = || f64::from(i16::from_le_bytes([value[0], value[1]]));
                match id {
                    0x01 => _ = battery.get_or_insert(value[0]),
                    // Hundredths of a degree; or tenths, within what
                    // hundredths hold.
                    0x02 => _ = temperature.get_or_insert(signed() as i16),
                    0x45 => {
                        _ = temperature
                            .get_or_insert((signed() * 10.0).clamp(-32768.0, 32767.0) as i16)
                    }
                    // Hundredths of a percent, to the nearest percent, at
                    // most 100; or whole percent.
                    0x03 => {
                        _ = humidity
                            .get_or_insert((f64::from(le(2)) / 100.0).round().min(100.0) as u8)
                    }
                    0x2E => _ = humidity.get_or_insert(value[0]),
                    // Hundredths of a hectopascal, to the nearest tenth, as
                    // far as a u16 goes.
                    0x04 => {
                        _ = pressure
                            .get_or_insert((f64::from(le(3)) / 10.0).round().min(65535.0) as u16)
                    }
                    0x12 => _ = co2.get_or_insert(le(2) as u16),
                    _ => {}
                }
            }
            match End::from(*end) {
                End::Clean => {}
                End::Unknown => {
                    let unknown = (0..=u8::MAX)
                        .filter(|&id| object_length(id, &[0]).is_none())
                        .nth(usize::from(*end) % 100);
                    data.push(unknown.expect("over 100 ids undefined"));
                    data.extend_from_slice(tail);
                }
                End::Truncated => {
                    // A wanted id, with a byte too few (and a length byte
                    // for more than there is, for text).
                    let id = [0x02, 0x04, 0x12, 0x53][usize::from(*end) % 4];
                    data.push(id);
                    let n = object_length(id, &[0xFF]).expect("a defined id");
                    data.extend_from_slice(&[0xFF].repeat(n - 1));
                }
            }

            let parsed = bthome_sensor::parse(&data);
            // Only v2 can be read, and not encrypted.
            if info & 1 != 0 {
                assert_eq!(parsed, Err(bthome_sensor::Skip::Encrypted));
                return;
            }
            if info >> 5 != 2 {
                assert_eq!(parsed, Err(bthome_sensor::Skip::Version));
                return;
            }
            let Some(hundredths) = temperature else {
                assert_eq!(parsed, Err(bthome_sensor::Skip::NoTemperature));
                return;
            };
            let reading = parsed.expect("a temperature");
            check(&reading);
            assert_eq!(
                reading.kind,
                Kind::BTHome {
                    irregular: info & 0b100 != 0
                }
            );
            assert_eq!(reading.hundredths, Some(hundredths));
            assert_eq!(reading.humidity, humidity);
            assert_eq!(reading.pressure, pressure);
            assert_eq!(reading.co2, co2);
            assert_eq!(reading.battery, battery);

            // In an advertisement, if it fits in one structure: flags,
            // then the service data, under BTHome's UUID (0xFCD2).
            if data.len() <= 252 {
                let mut ad = vec![0x02, 0x01, 0x06, 3 + data.len() as u8, 0x16, 0xD2, 0xFC];
                ad.extend_from_slice(&data);
                assert_eq!(bthome_sensor::service_data(&ad), Some(&data[..]));
            }
        });
}

impl From<u8> for End {
    fn from(byte: u8) -> Self {
        match byte >> 6 {
            0 | 1 => End::Clean,
            2 => End::Unknown,
            _ => End::Truncated,
        }
    }
}

// ==============================================================================
// Choosing the nearest
// ==============================================================================

/// Whatever the scans hear, in any order, at any time: the sensor shown
/// after a scan is one it heard, at or above [`FLOOR_DBM`], within
/// `HYSTERESIS_DB` (3 dB) of the strongest; a scan never waits more than
/// its limits; every eighth scan listens to its limit; and a fix from the
/// phone that forgets the sensors forgets the one shown too.
#[test]
fn the_nearest_is_one_heard() {
    bolero::check!()
        .with_type::<Vec<(u8, u8, i8, u16)>>()
        .for_each(|steps| {
            let mut readings = Vec::new();
            for kind in [
                Kind::Aranet4,
                Kind::BTHome { irregular: false },
                Kind::BTHome { irregular: true },
            ] {
                let mut payload = [0u8; 22];
                payload[0] = 0x22;
                payload[10] = 0xF0; // 12 °C
                let reading = aranet::parse(&payload).expect("a reading");
                readings.push(Reading { kind, ..reading });
            }
            let mut nearby = Nearby::new();
            let mut now = 0u64;
            let mut scans = 0u8;
            let mut scanning = false;
            // What this scan heard: the strongest of each, at or above
            // the floor.
            let mut heard: HashMap<Address, i8> = HashMap::new();
            for &(op, who, rssi, wait) in steps {
                // Up to ~1.8 hours at a time.
                now += u64::from(wait) * 100;
                // Few addresses, so that they come up again; more than
                // `SENSORS`, so that they crowd each other out.
                let address: Address = [who % 6, 0, 0, 0, 0, 0];
                match op % 5 {
                    0 => {
                        nearby.begin(now);
                        scans = scans.wrapping_add(1);
                        scanning = true;
                        heard.clear();
                    }
                    1 | 2 => {
                        let reading = readings[usize::from(who >> 6) % readings.len()];
                        let counts = nearby.hear(address, rssi, reading, now);
                        assert_eq!(counts, rssi >= FLOOR_DBM);
                        if counts {
                            let strongest = heard.entry(address).or_insert(rssi);
                            *strongest = (*strongest).max(rssi);
                        }
                    }
                    3 => {
                        let ended = nearby.end(now);
                        scanning = false;
                        assert_eq!(ended.heard, !heard.is_empty());
                        if ended.heard {
                            let best = nearby.best().expect("heard, so one shown");
                            assert!(best.rssi >= FLOOR_DBM);
                            let strongest =
                                heard.get(&best.address).expect("the one shown was heard");
                            assert!(best.rssi <= *strongest);
                            let loudest = heard.values().max().expect("heard");
                            assert!(i16::from(best.rssi) + 3 >= i16::from(*loudest));
                        }
                    }
                    _ => {
                        let here = Place {
                            latitude_micro: i32::from(rssi) * 1_000 * i32::from(who),
                            longitude_micro: i32::from(wait as i16) * 1_000,
                        };
                        if nearby.located(here, now) {
                            assert!(nearby.best().is_none());
                        }
                    }
                }
                let limit = nearby.limit(now);
                assert!(limit == aranet::SCAN_LIMIT || limit == aranet::SLOW_SCAN_LIMIT);
                if scans.is_multiple_of(aranet::LOOK_EVERY) {
                    assert!(!nearby.heard_everyone(now));
                }
                if scanning {
                    assert_eq!(
                        nearby.due(
                            now,
                            aranet::Context {
                                showing_air: true,
                                ..Default::default()
                            }
                        ),
                        None
                    );
                }
            }
        });
}

/// Distances between any two places, even ones no phone would send, are
/// a distance: not NaN, nor negative, the same both ways, and 0 from a
/// place to itself.
#[test]
fn any_distance_is_a_distance() {
    bolero::check!()
        .with_type::<(i32, i32, i32, i32)>()
        .for_each(|&(a_lat, a_lon, b_lat, b_lon)| {
            let a = Place {
                latitude_micro: a_lat,
                longitude_micro: a_lon,
            };
            let b = Place {
                latitude_micro: b_lat,
                longitude_micro: b_lon,
            };
            let d = aranet::distance_squared(a, b);
            assert!(d >= 0.0, "{d} between {a:?} and {b:?}");
            assert_eq!(d, aranet::distance_squared(b, a));
            assert_eq!(aranet::distance_squared(a, a), 0.0);
        });
}
