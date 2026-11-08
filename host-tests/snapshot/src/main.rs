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

    /// The sleep classifier's window, as `system_off` passes it.
    const RESEND: i64 = 90;

    fn header() -> Header {
        Header {
            first_minute: 29_500_000,
            minutes: 240,
            day: 20_400,
            steps: 8_123,
            checksum: 0,
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

    #[test]
    fn without_a_sync_the_whole_day_is_saved() {
        let newest = 29_500_000;
        assert_eq!(
            unsynced_from(newest, None, RESEND),
            newest - MAX_MINUTES as i64 + 1
        );
    }

    #[test]
    fn after_a_sync_from_where_the_next_fetch_starts() {
        let newest = 29_500_000;
        // Synced an hour ago: the next fetch sends from the window before
        // that.
        assert_eq!(
            unsynced_from(newest, Some(newest - 60), RESEND),
            newest - 60 - RESEND
        );
        // Synced long ago: never more than the history holds.
        assert_eq!(
            unsynced_from(newest, Some(newest - 5000), RESEND),
            newest - MAX_MINUTES as i64 + 1
        );
    }

    #[test]
    fn a_full_day_fits() {
        assert_eq!(MAX_BYTES, 32 + 1440 * 6);
        let bytes = record(&samples(MAX_MINUTES));
        assert!(read(&bytes).is_some());
    }
}
