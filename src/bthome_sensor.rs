//! Nearby BTHome thermometers: what their Bluetooth advertisements say.
//!
//! BTHome (<https://bthome.io/format/>) is the open format most DIY and
//! many off-the-shelf sensors advertise their readings in, for Home
//! Assistant: a Xiaomi LYWSD03MMC on pvvx's or ATC's firmware, Shelly's
//! BLU H&T, ESPHome boards. Like an Aranet4's, the readings are in every
//! advertisement, so a passive scan hears them without connecting
//! ([`crate::ble`] listens, and [`crate::aranet::Nearby`] picks the
//! nearest of every room sensor heard, of either kind).
//!
//! **The service data** (AD type 0x16) under UUID 0xFCD2:
//!
//! | Bytes | Field |
//! |---|---|
//! | 1 | device information: bit 0 encrypted, bit 1 MAC follows (see below), bit 2 "trigger based" (advertises irregularly), bits 5–7 the version, 2 |
//! | (6) | the MAC, if bit 1 says so |
//! | … | objects: an id, then its value, little-endian, in a length that the id fixes |
//!
//! Only unencrypted adverts can be read: an encrypted one needs the key
//! its owner typed into Home Assistant. The watch's own button presses
//! (see [`crate::bthome`]) are encrypted, so even if it could hear itself,
//! they would be passed over here; so are other watches'.
//!
//! The objects carry no lengths of their own, so walking them needs the
//! spec's table of every id's length ([`object_length`]): an object the
//! watch doesn't want is skipped by its length, and the walk stops at an
//! id it doesn't know, since nothing after it can be found (as Home
//! Assistant's own parser, `bthome-ble`, does). Readings before it count.
//!
//! The sender ([`crate::bthome`]) shares nothing with this but the UUID,
//! written out again here rather than shared: the sender's module brings
//! AES and the button grid with it, which this one (and its host tests)
//! has no use for.
//!
//! Hardware-free, so it runs in host tests (`host-tests/aranet`).

use crate::aranet::{Band, Kind, Reading};

/// BTHome's 16-bit service UUID, as sent (little-endian).
const UUID: [u8; 2] = [0xD2, 0xFC];
/// The AD type of service data with a 16-bit UUID.
const SERVICE_DATA: u8 = 0x16;

/// BTHome's service data in an advertisement's AD structures, after the
/// UUID; `None` if there's none.
pub fn service_data(ad: &[u8]) -> Option<&[u8]> {
    crate::aranet::find(ad, SERVICE_DATA, &UUID)
}

/// Why BTHome service data had no temperature to read.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[cfg_attr(target_os = "none", derive(defmt::Format))]
pub enum Skip {
    /// Encrypted: it takes the device's key to read.
    Encrypted,
    /// BTHome v1, or a version yet to come (v1 used other UUIDs, so this
    /// is rare).
    Version,
    /// Readable, but without a temperature: another kind of sensor, or a
    /// thermometer's other advert (pvvx's firmware alternates its
    /// temperature with its battery voltage).
    NoTemperature,
    /// Too short for its device information, or its MAC.
    Malformed,
}

/// Bits of the device information byte.
const ENCRYPTED: u8 = 1 << 0;
const MAC_INCLUDED: u8 = 1 << 1;
const IRREGULAR: u8 = 1 << 2;
const VERSION_2: u8 = 2 << 5;
const VERSION_BITS: u8 = 0b111 << 5;

/// Reads BTHome v2 service data (after the UUID): the first temperature
/// (0x02, hundredths, or 0x45, tenths), the first humidity (0x03,
/// hundredths, or 0x2E, whole percent), battery (0x01), pressure (0x04)
/// and CO₂ (0x12), whichever it has. Several of one kind (a thermometer
/// with a probe as well) give their first: the device's own sensor comes
/// first in what pvvx's firmware sends.
pub fn parse(data: &[u8]) -> Result<Reading, Skip> {
    let Some((&info, mut objects)) = data.split_first() else {
        return Err(Skip::Malformed);
    };
    if info & ENCRYPTED != 0 {
        return Err(Skip::Encrypted);
    }
    if info & VERSION_BITS != VERSION_2 {
        return Err(Skip::Version);
    }
    // The spec reserves bit 1, but Home Assistant's parser takes it to
    // mean that the device's MAC (6 bytes) comes first, and so do some
    // devices.
    if info & MAC_INCLUDED != 0 {
        objects = objects.get(6..).ok_or(Skip::Malformed)?;
    }

    let mut reading = Reading {
        kind: Kind::BTHome {
            irregular: info & IRREGULAR != 0,
        },
        co2: None,
        hundredths: None,
        pressure: None,
        humidity: None,
        battery: None,
        band: None,
        interval: 0,
        age: 0,
    };
    while let [id, rest @ ..] = objects {
        // An id it doesn't know, or one that claims more than there is,
        // ends the walk: the rest can't be found.
        let Some(length) = object_length(*id, rest).filter(|&n| n <= rest.len()) else {
            break;
        };
        let (value, next) = rest.split_at(length);
        objects = next;
        let le = |n: usize| {
            value[..n]
                .iter()
                .rev()
                .fold(0u32, |sum, &byte| sum << 8 | u32::from(byte))
        };
        let signed = || i16::from_le_bytes([value[0], value[1]]);
        match id {
            0x01 => _ = reading.battery.get_or_insert(value[0]),
            0x02 => _ = reading.hundredths.get_or_insert(signed()),
            // Tenths, as hundredths, within what fits (±327.67 °C).
            0x45 => {
                let tenths = signed();
                _ = reading.hundredths.get_or_insert(tenths.saturating_mul(10));
            }
            // Hundredths of a percent, to the nearest percent (and no
            // more than 100, which a u16 could exceed).
            0x03 => {
                let percent = ((le(2) + 50) / 100).min(100);
                _ = reading.humidity.get_or_insert(percent as u8);
            }
            0x2E => _ = reading.humidity.get_or_insert(value[0]),
            // Hundredths of a hectopascal, to tenths.
            0x04 => {
                let tenths = ((le(3) + 5) / 10).min(u32::from(u16::MAX));
                _ = reading.pressure.get_or_insert(tenths as u16);
            }
            0x12 => _ = reading.co2.get_or_insert(le(2) as u16),
            _ => {}
        }
    }
    if reading.hundredths.is_none() {
        return Err(Skip::NoTemperature);
    }
    reading.band = Band::from_status(0, reading.co2);
    Ok(reading)
}

/// How many bytes follow object `id`, given what follows it (`rest`):
/// fixed for most, from a length byte for text (0x53), raw data (0x54),
/// and commands (0x3B, whose length byte counts only the arguments after
/// an opcode); `None` for an id the spec doesn't define (as of
/// 2026-11, with `bthome-ble`'s table for reference).
pub fn object_length(id: u8, rest: &[u8]) -> Option<usize> {
    let length = match id {
        // Packet id, battery, counts, binary sensors (0x0F–0x11 and
        // 0x15–0x2D), whole-percent humidity and moisture, button, UV
        // index, 1-byte temperatures, count, channel, light level,
        // settings revision.
        0x00
        | 0x01
        | 0x09
        | 0x0F..=0x11
        | 0x15..=0x2F
        | 0x3A
        | 0x46
        | 0x57..=0x59
        | 0x60
        | 0x64
        | 0x65 => 1,
        0x02
        | 0x03
        | 0x06..=0x08
        | 0x0C..=0x0E
        | 0x12..=0x14
        | 0x3C
        | 0x3D
        | 0x3F..=0x41
        | 0x43..=0x45
        | 0x47..=0x4A
        | 0x51
        | 0x52
        | 0x56
        | 0x5A
        | 0x5D..=0x5F
        | 0x61
        | 0xF0 => 2,
        0x04 | 0x05 | 0x0A | 0x0B | 0x42 | 0x4B | 0xF2 => 3,
        0x3E | 0x4C..=0x50 | 0x55 | 0x5B | 0x5C | 0x62 | 0x63 | 0xF1 => 4,
        0x53 | 0x54 => 1 + usize::from(*rest.first()?),
        0x3B => 2 + usize::from(rest.first()? & 0x1F),
        _ => return None,
    };
    Some(length)
}
