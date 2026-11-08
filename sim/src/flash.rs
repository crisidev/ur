//! A stand-in for the nRF52832's internal flash, as `src/storage.rs` sees
//! it through MPSL (`nrf_mpsl::Flash`), so the firmware's own storage code
//! (sequential-storage's map, its keys and its encodings) runs here
//! unchanged.
//!
//! It behaves as NOR flash does, and as sequential-storage relies on:
//!
//! - Erasing works on whole 4 KiB pages, and sets every bit (0xFF).
//! - Writing can only clear bits: what's written is ANDed into what's
//!   there, as the nRF52's NVMC does, so a word can be written again (to
//!   clear more bits) without an erase (`MultiwriteNorFlash`).
//! - Writes are whole 32-bit words, at word-aligned addresses; reads are
//!   any bytes.
//! - Addresses are the chip's: 512 KiB of flash, of which `storage`'s 12
//!   KiB at 0x7D000 (`storage::RANGE`, as in `memory.x`) are modelled.
//!   The rest is the bootloader and firmware, which aren't here: touching
//!   it is an error, so a stray access shows up rather than silently
//!   working.
//!
//! The pages live in memory, and with `--flash FILE`, in a file too
//! (written through on every change), so settings survive between runs as
//! they do between boots. The file is the region's 12 KiB exactly as on
//! the chip, so it can be a dump of a watch's (`esprobe … read 0x7D000`),
//! and the simulator then boots with that watch's settings.
//!
//! TODO: the NVMC's limit on writes to a word between erases (and to a
//! 512-byte block, on the nRF52832) isn't enforced; sequential-storage
//! stays well within it.

use std::ops::Range;
#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;
use std::path::PathBuf;

use embedded_storage_async::nor_flash::{
    ErrorType, MultiwriteNorFlash, NorFlash, NorFlashError, NorFlashErrorKind, ReadNorFlash,
};

use crate::storage;

/// The nRF52832's flash (`embassy_nrf::nvmc::FLASH_SIZE`).
const CAPACITY: usize = 512 * 1024;
/// Its page, the unit of erasing (`embassy_nrf::nvmc::PAGE_SIZE`).
pub const PAGE: usize = 4096;
/// The unit of writing (`nrf_mpsl`'s `FLASH_WRITE_SIZE` on the nRF52).
const WORD: usize = 4;

/// What's modelled: the storage region.
const REGION: Range<u32> = storage::RANGE;
pub const REGION_BYTES: usize = (REGION.end - REGION.start) as usize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, defmt::Format)]
pub enum Error {
    /// Past the end of the chip's flash.
    OutOfBounds,
    /// Not on a word (writes) or page (erases) boundary.
    Unaligned,
    /// In the chip's flash, but outside the storage region this models.
    NotModelled,
}

impl NorFlashError for Error {
    fn kind(&self) -> NorFlashErrorKind {
        match self {
            Self::OutOfBounds => NorFlashErrorKind::OutOfBounds,
            Self::Unaligned => NorFlashErrorKind::NotAligned,
            Self::NotModelled => NorFlashErrorKind::Other,
        }
    }
}

pub struct Flash {
    /// The storage region, from its first byte.
    bytes: Vec<u8>,
    /// Where it's kept between runs, if anywhere.
    file: Option<PathBuf>,
}

impl Flash {
    /// Flash as it comes from the factory: erased.
    pub fn erased() -> Self {
        Self {
            bytes: vec![0xFF; REGION_BYTES],
            file: None,
        }
    }

    /// The flash kept in `path`: what's there, or erased flash if there's
    /// no file yet (it's written at the first change).
    ///
    /// Only for the command line's `--flash`: the browser has no files,
    /// and starts erased (see `web.rs`).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn open(path: &Path) -> Result<Self, String> {
        let bytes = match std::fs::read(path) {
            Ok(bytes) if bytes.len() == REGION_BYTES => bytes,
            Ok(bytes) => {
                return Err(format!(
                    "{}: {} bytes, but the storage region is {REGION_BYTES} (0x{:X}..0x{:X})",
                    path.display(),
                    bytes.len(),
                    REGION.start,
                    REGION.end
                ));
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => vec![0xFF; REGION_BYTES],
            Err(e) => return Err(format!("reading {}: {e}", path.display())),
        };
        Ok(Self {
            bytes,
            file: Some(path.to_owned()),
        })
    }

    /// Where in `bytes` the `len` bytes at `offset` are.
    fn span(offset: u32, len: usize) -> Result<Range<usize>, Error> {
        let end = offset as usize + len;
        if end > CAPACITY {
            return Err(Error::OutOfBounds);
        }
        if offset < REGION.start || end > REGION.end as usize {
            return Err(Error::NotModelled);
        }
        let start = (offset - REGION.start) as usize;
        Ok(start..start + len)
    }

    /// Writes the region through to its file, if it has one.
    fn persist(&self) {
        if let Some(path) = &self.file {
            // The watch can't fail to keep what it wrote; a simulator that
            // can't is a broken setup, not something to simulate.
            std::fs::write(path, &self.bytes)
                .unwrap_or_else(|e| panic!("writing the flash to {}: {e}", path.display()));
        }
    }
}

impl Flash {
    /// As `nrf_mpsl::Flash::write`, which `storage::write_word_elsewhere`
    /// calls (not the trait's, which it doesn't import).
    pub async fn write(&mut self, offset: u32, data: &[u8]) -> Result<(), Error> {
        NorFlash::write(self, offset, data).await
    }
}

impl ErrorType for Flash {
    type Error = Error;
}

impl ReadNorFlash for Flash {
    const READ_SIZE: usize = 1;

    async fn read(&mut self, offset: u32, bytes: &mut [u8]) -> Result<(), Error> {
        let span = Self::span(offset, bytes.len())?;
        bytes.copy_from_slice(&self.bytes[span]);
        Ok(())
    }

    fn capacity(&self) -> usize {
        CAPACITY
    }
}

impl NorFlash for Flash {
    const WRITE_SIZE: usize = WORD;
    const ERASE_SIZE: usize = PAGE;

    async fn erase(&mut self, from: u32, to: u32) -> Result<(), Error> {
        if to < from || to as usize > CAPACITY {
            return Err(Error::OutOfBounds);
        }
        if !(from as usize).is_multiple_of(PAGE) || !(to as usize).is_multiple_of(PAGE) {
            return Err(Error::Unaligned);
        }
        let span = Self::span(from, (to - from) as usize)?;
        self.bytes[span].fill(0xFF);
        self.persist();
        Ok(())
    }

    async fn write(&mut self, offset: u32, data: &[u8]) -> Result<(), Error> {
        if offset as usize + data.len() > CAPACITY {
            return Err(Error::OutOfBounds);
        }
        if !(offset as usize).is_multiple_of(WORD) || !data.len().is_multiple_of(WORD) {
            return Err(Error::Unaligned);
        }
        let span = Self::span(offset, data.len())?;
        // Programming only takes bits from 1 to 0.
        for (byte, new) in self.bytes[span].iter_mut().zip(data) {
            *byte &= new;
        }
        self.persist();
        Ok(())
    }
}

impl MultiwriteNorFlash for Flash {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::watch::run;

    const START: u32 = REGION.start;

    fn read(flash: &mut Flash, offset: u32, len: usize) -> Vec<u8> {
        let mut bytes = vec![0; len];
        run(flash.read(offset, &mut bytes), &mut None).expect("in the region");
        bytes
    }

    #[test]
    fn starts_erased() {
        let mut flash = Flash::erased();
        assert_eq!(
            read(&mut flash, START, REGION_BYTES),
            vec![0xFF; REGION_BYTES]
        );
    }

    #[test]
    fn writes_only_clear_bits() {
        let mut flash = Flash::erased();
        run(flash.write(START, &[0x0F, 0xF0, 0x00, 0xFF]), &mut None).expect("aligned");
        // Writing ones over zeros leaves them zeros; zeros clear more.
        run(flash.write(START, &[0xFF, 0x0F, 0xFF, 0x7F]), &mut None).expect("again");
        assert_eq!(read(&mut flash, START, 4), [0x0F, 0x00, 0x00, 0x7F]);
    }

    #[test]
    fn erasing_sets_a_whole_page() {
        let mut flash = Flash::erased();
        let second = START + PAGE as u32;
        run(flash.write(START, &[0; 4]), &mut None).expect("aligned");
        run(flash.write(second, &[0; 4]), &mut None).expect("aligned");
        run(flash.erase(second, second + PAGE as u32), &mut None).expect("a page");
        assert_eq!(read(&mut flash, START, 4), [0; 4]);
        assert_eq!(read(&mut flash, second, 4), [0xFF; 4]);
    }

    #[test]
    fn refuses_what_the_chip_would() {
        let mut flash = Flash::erased();
        let unaligned = run(flash.write(START + 2, &[0; 4]), &mut None);
        assert_eq!(unaligned, Err(Error::Unaligned));
        let short = run(flash.write(START, &[0; 2]), &mut None);
        assert_eq!(short, Err(Error::Unaligned));
        let partial_page = run(flash.erase(START, START + 1024), &mut None);
        assert_eq!(partial_page, Err(Error::Unaligned));
        let past_the_end = run(flash.write(CAPACITY as u32, &[0; 4]), &mut None);
        assert_eq!(past_the_end, Err(Error::OutOfBounds));
        // The firmware's own pages, say the bootloader's flag that
        // `storage::write_word_elsewhere` sets.
        let elsewhere = run(flash.write(0x7BE50, &[0; 4]), &mut None);
        assert_eq!(elsewhere, Err(Error::NotModelled));
    }

    /// The firmware's own storage, over this, through enough saves to fill
    /// the three pages several times over: sequential-storage then erases
    /// pages to make room, keeping only the latest of each key. (The only
    /// test here using `storage`, whose map is a global.)
    #[test]
    fn the_firmware_storage_runs_on_it() {
        storage::init(Flash::erased());
        run(
            async {
                assert_eq!(storage::load::<1>(storage::key::BRIGHTNESS).await, None);
                for i in 0..2000u32 {
                    let alarms = [(i % 251) as u8; crate::alarm::Settings::BYTES];
                    assert!(storage::save(storage::key::ALARMS, &alarms).await);
                    assert!(storage::save(storage::key::BRIGHTNESS, &[i as u8]).await);
                }
                assert_eq!(
                    storage::load(storage::key::ALARMS).await,
                    Some([(1999 % 251) as u8; crate::alarm::Settings::BYTES])
                );
                assert_eq!(
                    storage::load::<1>(storage::key::BRIGHTNESS).await,
                    Some([1999u32 as u8])
                );
                assert!(storage::remove(storage::key::BRIGHTNESS).await);
                assert_eq!(storage::load::<1>(storage::key::BRIGHTNESS).await, None);
            },
            &mut None,
        );
    }
}
