//! Fuzzing, with Bolero: the record is read back from flash after System
//! OFF, where it may be anything (erased, half-written, worn, a record of
//! an older layout). The tests above try one of each; these try any
//! header and any samples, with any byte changed, read back in pieces of
//! any size, against the layout in the module's docs.
//!
//! `cargo test` runs each harness over a few hundred random inputs (and
//! any saved in a corpus), so they're regression tests too. To fuzz one
//! properly (nightly, for the sanitizer):
//!
//! ```text
//! cargo +nightly bolero test fuzz::any_change_to_a_record_is_caught --engine libfuzzer -T 10min
//! ```

use super::snapshot::*;

/// FNV-1a over `bytes`, from `hash`, as its authors define it (32-bit:
/// XOR the byte in, then multiply by the prime 16777619, modulo 2³²).
fn fnv1a(hash: u32, bytes: &[u8]) -> u32 {
    bytes.iter().fold(hash, |hash, &byte| {
        ((u64::from(hash ^ u32::from(byte)) * 16_777_619) % (1 << 32)) as u32
    })
}

/// Any header fits in its 32 bytes and reads back as it was, if it holds
/// no more minutes than a record can ([`MAX_MINUTES`]); and any 32 bytes
/// read as a header are those bytes, by the table in the module's docs
/// (bar the reserved, which are ignored): a fresh record's, with the
/// magic `urH1` and the consumed word erased.
#[test]
fn any_header_reads_back_as_written() {
    bolero::check!()
        .with_type::<((u32, u16, i32, u32, u32), [u8; HEADER_BYTES])>()
        .for_each(|&((first_minute, minutes, day, steps, checksum), bytes)| {
            let header = Header {
                first_minute,
                minutes,
                day,
                steps,
                checksum,
            };
            let written = header.to_bytes();
            assert_eq!(&written[..4], b"urH1");
            assert_eq!(&written[24..28], &[0xFF; 4], "the consumed word, erased");
            let expected = (usize::from(minutes) <= MAX_MINUTES).then_some(header);
            assert_eq!(Header::from_bytes(&written), expected);

            // Any bytes: now and then with the magic and an erased
            // consumed word, so that they're read.
            let mut bytes = bytes;
            if bytes[31] & 1 != 0 {
                bytes[..4].copy_from_slice(b"urH1");
                bytes[24..28].fill(0xFF);
                bytes[9] &= 0x07;
            }
            let read = Header::from_bytes(&bytes);
            let fresh = &bytes[..4] == b"urH1" && bytes[24..28] == [0xFF; 4];
            let minutes = u16::from_le_bytes([bytes[8], bytes[9]]);
            assert_eq!(read.is_some(), fresh && usize::from(minutes) <= MAX_MINUTES);
            if let Some(read) = read {
                let again = read.to_bytes();
                assert_eq!(again[..10], bytes[..10]);
                assert_eq!(again[12..28], bytes[12..28]);
            }
        });
}

/// What `system_off::restore` does with a record: the header and the
/// samples, if they check out, the samples' checksum taken over the
/// pieces they're read back in (each `cuts` long, the rest at the end).
fn read<'a>(bytes: &'a [u8], cuts: &[u16]) -> Option<(Header, &'a [u8])> {
    let header = Header::from_bytes(bytes.get(..HEADER_BYTES)?.try_into().expect("32 bytes"))?;
    let samples = bytes
        .get(HEADER_BYTES..)?
        .get(..usize::from(header.minutes) * SAMPLE_BYTES)?;
    let mut hash = CHECKSUM_SEED;
    let mut rest = samples;
    for &cut in cuts {
        let (now, later) = rest.split_at(usize::from(cut).min(rest.len()));
        hash = checksum(hash, now);
        rest = later;
    }
    hash = checksum(hash, rest);
    (header.expected_checksum(hash) == header.checksum).then_some((header, samples))
}

/// A record as `system_off::save` writes it (samples, then the header
/// sealed with their checksum) reads back, in pieces of any size; and with
/// any one byte changed, it's refused, unless the byte is reserved (and
/// ignored). FNV-1a catches every change of a single byte: XOR with a byte,
/// and multiplying by an odd prime modulo 2³², are both one-to-one.
#[test]
fn any_change_to_a_record_is_caught() {
    bolero::check!()
        .with_type::<((u32, i32, u32), Vec<u8>, u8, Option<(u16, u8)>, Vec<u16>)>()
        .for_each(
            |&((first_minute, day, steps), ref samples, times, change, ref cuts)| {
                // Up to a day of samples: the first byte says how many times
                // over.
                let samples = samples.repeat(1 + usize::from(times) % 64);
                let minutes = (samples.len() / SAMPLE_BYTES).min(MAX_MINUTES);
                let samples = &samples[..minutes * SAMPLE_BYTES];
                let mut header = Header {
                    first_minute,
                    minutes: minutes as u16,
                    day,
                    steps,
                    checksum: 0,
                };
                header.checksum = header.expected_checksum(fnv1a(CHECKSUM_SEED, samples));
                let mut record = header.to_bytes().to_vec();
                record.extend_from_slice(samples);

                let Some((at, bits)) = change.filter(|&(_, bits)| bits != 0) else {
                    assert_eq!(read(&record, cuts), Some((header, samples)));
                    return;
                };
                let at = usize::from(at) % record.len();
                record[at] ^= bits;
                let reserved = (10..12).contains(&at) || (28..32).contains(&at);
                assert_eq!(read(&record, cuts).is_some(), reserved, "byte {at} changed");
            },
        );
}

/// FNV-1a, in pieces of any size, is FNV-1a: [`checksum`] continues where
/// the last piece left off.
#[test]
fn checksum_in_any_pieces_is_fnv1a() {
    bolero::check!()
        .with_type::<(u32, Vec<u8>, Vec<u16>)>()
        .for_each(|(seed, bytes, cuts)| {
            let mut hash = *seed;
            let mut rest = &bytes[..];
            for &cut in cuts {
                let (now, later) = rest.split_at(usize::from(cut).min(rest.len()));
                hash = checksum(hash, now);
                rest = later;
            }
            assert_eq!(checksum(hash, rest), fnv1a(*seed, bytes));
        });
}

/// For any newest minute and the phone's last fetch (within a few
/// thousand years of 1970, in minutes), the record starts no earlier than
/// the history does (a day back), and no later than the newest minute; and
/// it holds every minute from where the phone's next fetch starts
/// (`resend` before what it last got), as far as the history goes.
#[test]
fn the_record_starts_where_the_phone_will_fetch() {
    bolero::check!()
        .with_type::<(i32, Option<i32>, u16)>()
        .for_each(|&(newest, synced, resend)| {
            let (newest, synced, resend) =
                (i64::from(newest), synced.map(i64::from), i64::from(resend));
            let from = unsynced_from(newest, synced, resend);
            let oldest = newest - MAX_MINUTES as i64 + 1;
            assert!((oldest..=newest).contains(&from), "{from} for {newest}");
            match synced {
                // The phone has what it fetched, bar the last `resend`.
                Some(synced) if synced <= newest => assert!(from <= (synced - resend).max(oldest)),
                // Without a fetch it knows of, the whole day.
                _ => assert_eq!(from, oldest),
            }
        });
}
