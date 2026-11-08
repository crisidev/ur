//! The record that carries unsynced activity across System OFF (see
//! [`crate::system_off`]): its format, and which minutes go in it.
//!
//! Hardware-free, so it's tested on the host (`host-tests/snapshot`).
//!
//! The record is a 32-byte header followed by one 6-byte sample per minute
//! (the in-RAM [`crate::activity::Sample`], verbatim) from the first minute
//! the phone may not have to the newest:
//!
//! | Offset | Size | Field |
//! |---:|---:|---|
//! | 0 | 4 | magic, `urH1` |
//! | 4 | 4 | first minute (UTC minutes since 1970) |
//! | 8 | 2 | minutes |
//! | 10 | 2 | reserved (erased) |
//! | 12 | 4 | today's local date (days since 1970, or [`UNKNOWN_DAY`]) |
//! | 16 | 4 | today's steps |
//! | 20 | 4 | checksum (FNV-1a over the samples, then bytes 4..20) |
//! | 24 | 4 | consumed: erased while fresh, 0 once restored and synced |
//! | 28 | 4 | reserved (erased) |
//!
//! The header is written last, so a save cut short leaves no valid record.
//! The consumed word is cleared in place (NOR flash can always clear bits),
//! so retiring a record costs no erase.

/// The header's size, before the samples.
pub const HEADER_BYTES: usize = 32;
/// One minute's sample.
pub const SAMPLE_BYTES: usize = 6;
/// The most minutes a record holds: the whole in-RAM history, a day.
pub const MAX_MINUTES: usize = 24 * 60;
/// The whole record at its largest.
pub const MAX_BYTES: usize = HEADER_BYTES + MAX_MINUTES * SAMPLE_BYTES;

const MAGIC: u32 = u32::from_le_bytes(*b"urH1");
/// Where the consumed word is, from the record's start.
pub const CONSUMED_OFFSET: u32 = 24;
/// What the consumed word becomes once the record has done its job.
pub const CONSUMED: u32 = 0;
/// An erased word: a fresh record's consumed word.
const ERASED: u32 = u32::MAX;

/// Today's date when the clock wasn't known (steps can still have been
/// counted then).
pub const UNKNOWN_DAY: i32 = i32::MIN;

/// FNV-1a's offset basis: where [`checksum`] starts, over the samples.
pub const CHECKSUM_SEED: u32 = 0x811C_9DC5;

/// FNV-1a, continued over `bytes`: enough to tell a record from leftover
/// or half-written flash, which is all it's for.
pub fn checksum(mut hash: u32, bytes: &[u8]) -> u32 {
    for &byte in bytes {
        hash = (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193);
    }
    hash
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    /// The first sample's UTC minute since 1970 (32 bits last until the
    /// year 10136).
    pub first_minute: u32,
    pub minutes: u16,
    /// Local days since 1970, or [`UNKNOWN_DAY`].
    pub day: i32,
    pub steps: u32,
    pub checksum: u32,
}

impl Header {
    /// The record's checksum, given the samples' (see [`checksum`]): that,
    /// continued over the header's own fields. (Samples first, so a writer
    /// can fill in the header after them.)
    pub fn expected_checksum(&self, samples: u32) -> u32 {
        checksum(samples, &self.to_bytes()[4..20])
    }

    pub fn to_bytes(self) -> [u8; HEADER_BYTES] {
        let mut bytes = [0xFF; HEADER_BYTES];
        bytes[0..4].copy_from_slice(&MAGIC.to_le_bytes());
        bytes[4..8].copy_from_slice(&self.first_minute.to_le_bytes());
        bytes[8..10].copy_from_slice(&self.minutes.to_le_bytes());
        bytes[12..16].copy_from_slice(&self.day.to_le_bytes());
        bytes[16..20].copy_from_slice(&self.steps.to_le_bytes());
        bytes[20..24].copy_from_slice(&self.checksum.to_le_bytes());
        bytes
    }

    /// A fresh record's header, or `None` for anything else: erased flash,
    /// a consumed record, or nonsense. The samples' checksum is still the
    /// caller's to check.
    pub fn from_bytes(bytes: &[u8; HEADER_BYTES]) -> Option<Self> {
        let word = |i: usize| u32::from_le_bytes(bytes[i..i + 4].try_into().expect("4 bytes"));
        if word(0) != MAGIC || word(CONSUMED_OFFSET as usize) != ERASED {
            return None;
        }
        let header = Self {
            first_minute: word(4),
            minutes: u16::from_le_bytes([bytes[8], bytes[9]]),
            day: word(12) as i32,
            steps: word(16),
            checksum: word(20),
        };
        (usize::from(header.minutes) <= MAX_MINUTES).then_some(header)
    }
}

/// The first minute worth saving, given the newest minute in the history,
/// and the newest the phone has fetched since boot (if it has).
///
/// The phone's next fetch starts `resend` minutes before what it last got
/// (the window in which sleep and its stages are still being revised), so the record
/// starts there too: those minutes may have changed since. Without a fetch
/// this boot, all of the history is (as far as we know) unsynced.
pub fn unsynced_from(newest: i64, synced_through: Option<i64>, resend: i64) -> i64 {
    let oldest = newest - MAX_MINUTES as i64 + 1;
    synced_through.map_or(oldest, |synced| (synced - resend).max(oldest))
}
