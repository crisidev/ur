//! Host tests for the firmware's `snapshot` module: the record that carries
//! unsynced activity across System OFF. `cargo test` runs them.

#[path = "../../../src/snapshot.rs"]
#[allow(dead_code)]
mod snapshot;

fn main() {
    println!("run `cargo test`");
}

#[cfg(test)]
mod tests {
    use super::snapshot::*;

    fn header() -> Header {
        Header {
            first_minute: 29_500_000,
            minutes: 240,
            day: 20_400,
            steps: 8_123,
            checksum: 0,
            synced: true,
        }
    }

    /// A record as `system_off::save` writes it: samples, then the header
    /// sealed with their checksum.
    fn record(samples: &[u8]) -> Vec<u8> {
        let mut header = header();
        header.minutes = (samples.len() / SAMPLE_BYTES) as u16;
        header.checksum = header.expected_checksum(checksum(CHECKSUM_SEED, samples));
        let mut bytes = header.to_bytes().to_vec();
        bytes.extend_from_slice(samples);
        bytes
    }

    /// What `system_off::restore` does with a record: the header and
    /// samples if they check out.
    fn read(bytes: &[u8]) -> Option<(Header, &[u8])> {
        let header = Header::from_bytes(bytes[..HEADER_BYTES].try_into().unwrap())?;
        let samples = &bytes[HEADER_BYTES..][..usize::from(header.minutes) * SAMPLE_BYTES];
        // In chunks, as it's read back.
        let hash = samples.chunks(36).fold(CHECKSUM_SEED, checksum);
        (header.expected_checksum(hash) == header.checksum).then_some((header, samples))
    }

    fn samples(minutes: usize) -> Vec<u8> {
        (0..minutes * SAMPLE_BYTES)
            .map(|i| (i * 7 % 251) as u8)
            .collect()
    }

    #[test]
    fn a_record_reads_back() {
        let samples = samples(240);
        let bytes = record(&samples);
        let (header, read_samples) = read(&bytes).expect("a valid record");
        assert_eq!(header.first_minute, 29_500_000);
        assert_eq!(header.minutes, 240);
        assert_eq!(header.day, 20_400);
        assert_eq!(header.steps, 8_123);
        assert_eq!(read_samples, &samples[..]);
    }

    #[test]
    fn steps_without_history_read_back() {
        let mut header = header();
        header.minutes = 0;
        header.day = UNKNOWN_DAY;
        header.checksum = header.expected_checksum(CHECKSUM_SEED);
        let bytes = header.to_bytes();
        let (read_header, samples) = read(&bytes).expect("a valid record");
        assert_eq!(read_header.day, UNKNOWN_DAY);
        assert!(samples.is_empty());
    }

    #[test]
    fn erased_flash_is_no_record() {
        let bytes = vec![0xFF; MAX_BYTES];
        assert_eq!(read(&bytes), None);
    }

    #[test]
    fn a_consumed_record_is_no_record() {
        // As `system_off::retire` leaves it: the consumed word cleared in
        // place, everything else untouched.
        let mut bytes = record(&samples(10));
        let at = CONSUMED_OFFSET as usize;
        // Programming NOR flash only clears bits.
        for (byte, consumed) in bytes[at..at + 4].iter_mut().zip(CONSUMED.to_le_bytes()) {
            *byte &= consumed;
        }
        assert_eq!(read(&bytes), None);
    }

    #[test]
    fn a_corrupt_record_is_rejected() {
        let mut bytes = record(&samples(100));
        bytes[HEADER_BYTES + 321] ^= 0x10;
        assert_eq!(read(&bytes), None);
        // In the header's fields too.
        let mut bytes = record(&samples(100));
        bytes[16] ^= 0x01; // steps
        assert_eq!(read(&bytes), None);
    }

    #[test]
    fn an_unfinished_save_is_no_record() {
        // The header goes last: cut off before it, the samples are there
        // but the header is still erased.
        let mut bytes = record(&samples(50));
        bytes[..HEADER_BYTES].fill(0xFF);
        assert_eq!(read(&bytes), None);
    }

    #[test]
    fn too_many_minutes_is_nonsense() {
        let mut header = header();
        header.minutes = MAX_MINUTES as u16 + 1;
        assert_eq!(Header::from_bytes(&header.to_bytes()), None);
    }

    /// Whether the phone's copy ends just before the record is its own
    /// word, outside the checksum: a record from an older build, which
    /// leaves it erased, reads as not knowing.
    #[test]
    fn the_synced_word_reads_back() {
        let bytes = record(&samples(10));
        assert_eq!(&bytes[28..32], b"sync");
        assert!(read(&bytes).expect("a valid record").0.synced);
        let mut older = bytes.clone();
        older[28..32].fill(0xFF);
        assert!(!read(&older).expect("still valid").0.synced);
        // Anything but the word is not knowing.
        older[28..32].copy_from_slice(b"synd");
        assert!(!read(&older).expect("still valid").0.synced);
    }

    #[test]
    fn without_a_sync_the_whole_day_is_saved() {
        let newest = 29_500_000;
        assert_eq!(
            unsynced_from(newest, None),
            (newest - MAX_MINUTES as i64 + 1, false)
        );
    }

    #[test]
    fn after_a_sync_from_where_the_next_fetch_starts() {
        let newest = 29_500_000;
        // Synced an hour ago: the next fetch sends from just after that.
        assert_eq!(
            unsynced_from(newest, Some(newest - 60)),
            (newest - 59, true)
        );
        // Synced long ago: never more than the history holds.
        assert_eq!(
            unsynced_from(newest, Some(newest - 5000)),
            (newest - MAX_MINUTES as i64 + 1, true)
        );
    }

    #[test]
    fn a_sync_past_the_newest_minute_saves_the_whole_day() {
        // The phone fetched up to minute 29_500_600 while its clock was
        // ten hours ahead; put right, the history dropped those minutes,
        // and holds new ones the phone hasn't had.
        let newest = 29_500_000;
        assert_eq!(
            unsynced_from(newest, Some(newest + 600)),
            (newest - MAX_MINUTES as i64 + 1, false)
        );
    }

    #[test]
    fn a_full_day_fits() {
        assert_eq!(MAX_BYTES, 32 + 1440 * 6);
        let bytes = record(&samples(MAX_MINUTES));
        assert!(read(&bytes).is_some());
    }
}

#[cfg(test)]
mod fuzz;

/// Proofs, for `cargo kani`: what the fuzzer tries for some headers, for
/// every one. The record is all that carries a day's activity across
/// System OFF, so what's written must read back: a header that didn't
/// would lose the day at the next boot, with nothing to say why.
#[cfg(kani)]
mod proofs {
    use super::snapshot::*;

    /// For every header a record can have (any first minute, date, steps
    /// and checksum, and up to a day of minutes): it reads back from its
    /// 32 bytes exactly as it was written, as a fresh record.
    #[kani::proof]
    fn every_header_reads_back_as_written() {
        let header = Header {
            first_minute: kani::any(),
            minutes: kani::any(),
            day: kani::any(),
            steps: kani::any(),
            checksum: kani::any(),
        };
        kani::assume(usize::from(header.minutes) <= MAX_MINUTES);
        assert_eq!(Header::from_bytes(&header.to_bytes()), Some(header));
    }

    /// For any 32 bytes of flash (erased, a consumed record, half a
    /// write, anything): they read as a header only with the magic and
    /// an erased consumed word, and then one with no more minutes than a
    /// record holds, so `system_off::restore` never reads samples past
    /// the record's sectors; and writing that header gives the same
    /// bytes back, bar the reserved ones.
    #[kani::proof]
    fn only_a_fresh_record_within_bounds_is_read() {
        let bytes: [u8; HEADER_BYTES] = kani::any();
        let fresh = bytes[..4] == *b"urH1" && bytes[24..28] == [0xFF; 4];
        match Header::from_bytes(&bytes) {
            Some(header) => {
                assert!(fresh);
                assert!(usize::from(header.minutes) <= MAX_MINUTES);
                assert!(HEADER_BYTES + usize::from(header.minutes) * SAMPLE_BYTES <= MAX_BYTES);
                let again = header.to_bytes();
                assert_eq!(again[..10], bytes[..10]);
                assert_eq!(again[12..28], bytes[12..28]);
            }
            None => {
                let minutes = u16::from_le_bytes([bytes[8], bytes[9]]);
                assert!(!fresh || usize::from(minutes) > MAX_MINUTES);
            }
        }
    }

    /// For any newest minute (to the year 6000), any minute the phone
    /// has fetched through (or none), and any revision window up to a
    /// day: the record starts within the history (the day up to the
    /// newest minute), so it never holds more minutes than a record can,
    /// which `from_bytes` would refuse at the next boot. It starts where
    /// the phone's next fetch will (the revision window before what it
    /// has) when that's in the history, and holds the whole history when
    /// it's before it, or the phone has nothing, or has "fetched" a
    /// minute the history hasn't reached (the clock went back since).
    #[kani::proof]
    fn the_record_starts_within_the_history() {
        let (newest, resend): (i64, i64) = kani::any();
        kani::assume((0..1 << 31).contains(&newest));
        kani::assume((0..=MAX_MINUTES as i64).contains(&resend));
        let synced: Option<i64> = kani::any();
        if let Some(synced) = synced {
            kani::assume((0..1 << 32).contains(&synced));
        }
        let from = unsynced_from(newest, synced, resend);
        let minutes = newest - from + 1;
        assert!(1 <= minutes && minutes <= MAX_MINUTES as i64);
        let oldest = newest - MAX_MINUTES as i64 + 1;
        match synced {
            Some(synced) if synced <= newest && synced - resend >= oldest => {
                assert_eq!(from, synced - resend)
            }
            _ => assert_eq!(from, oldest),
        }
    }
}
