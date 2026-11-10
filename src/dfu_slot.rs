//! Where a firmware update goes in the SPI flash: the arithmetic of
//! MCUBoot's secondary slot, apart from [`crate::dfu`]'s protocol and the
//! flash itself.
//!
//! Every address `dfu` erases or programs comes from here. Below the slot
//! are the bootloader's boot logo and recovery firmware, above it our saved
//! activity and field log (see [`crate::spi_flash`] for the map), and at its
//! end MCUBoot's own swap state: a slip in this arithmetic could reach any
//! of them, and what it would break (the recovery firmware, a night's
//! activity, an update that half-installs) only shows much later. So it's
//! hardware-free, with host tests and proofs (`host-tests/dfu`) that it
//! stays where it should for every image size and every packet.

use core::ops::Range;

/// The SPI flash's program page and erase sector, as [`crate::spi_flash`]
/// has them (`dfu` checks they agree): written out here so this module
/// builds without the driver.
pub const PAGE: u32 = 256;
pub const SECTOR: u32 = 4096;

/// Where the bootloader looks for updates in the SPI flash, and how big the
/// slot is (the same size as the primary; see `memory.x`).
pub const SLOT_START: u32 = 0x40000;
pub const SLOT_SIZE: u32 = 0x74000;
/// The end of the slot holds MCUBoot's swap-state trailer.
pub const TRAILER_SIZE: u32 = 0x1B0;
/// The most image the slot takes.
pub const CAPACITY: u32 = SLOT_SIZE - TRAILER_SIZE;
/// MCUBoot's trailer magic (`boot_img_magic` in MCUBoot 1.5), as bytes:
/// written to the last 16 bytes of the secondary slot, it asks for a swap.
/// Its "image OK" flag stays unset, so the swap is a test: the new image
/// must confirm itself or the next reset swaps back.
pub const BOOT_MAGIC: [u32; 4] = [0xf395c277, 0x7fefd260, 0x0f505235, 0x8079b62c];
/// Where [`BOOT_MAGIC`] goes.
pub const MAGIC_ADDRESS: u32 = SLOT_START + SLOT_SIZE - 16;

// The slot starts and ends on sector boundaries (so erasing it touches
// nothing else), and the magic lies within one page of its trailer, past
// anything an image can reach.
const _: () = assert!(SLOT_START.is_multiple_of(SECTOR) && SLOT_SIZE.is_multiple_of(SECTOR));
const _: () = assert!(MAGIC_ADDRESS % PAGE + 16 <= PAGE);
const _: () = assert!(SLOT_START + CAPACITY <= MAGIC_ADDRESS && TRAILER_SIZE <= SECTOR);

/// Whether `range` lies within the secondary slot. Every erase and program
/// checks it first: below the slot are the bootloader's boot logo and
/// recovery firmware, above it our saved activity and field log, and no
/// slip in the arithmetic should reach them.
pub fn in_slot(range: Range<u32>) -> bool {
    SLOT_START <= range.start && range.start <= range.end && range.end <= SLOT_START + SLOT_SIZE
}

/// What to erase before an image of `image_size` bytes (at most
/// [`CAPACITY`]) arrives: the image's sectors, and the trailer's (which
/// may hold state from a past update; MCUBoot must see it erased until we
/// activate). Two ranges that don't overlap: an image that reaches into
/// the trailer's sector has that part erased with the trailer.
pub fn erase_ranges(image_size: u32) -> (Range<u32>, Range<u32>) {
    let image_end = (SLOT_START + image_size).next_multiple_of(SECTOR);
    let trailer = SLOT_START + SLOT_SIZE - SECTOR;
    (
        SLOT_START..image_end.min(trailer),
        trailer..trailer + SECTOR,
    )
}

/// Where the page buffer goes, once `received` bytes of the image have
/// arrived, the last `page_len` of them still in the buffer.
///
/// `dfu` empties the buffer whenever it holds a whole page (or the image's
/// end), so what's in it always starts on a page boundary: `received -
/// page_len` is a multiple of [`PAGE`], and the program stays within one
/// page, as the chip needs (it wraps around within the page otherwise,
/// over what's already written).
pub fn page_address(received: u32, page_len: u32) -> u32 {
    debug_assert!(
        0 < page_len && page_len <= PAGE && page_len <= received,
        "the buffer holds the image's last bytes, a page at most"
    );
    debug_assert!(
        (received - page_len).is_multiple_of(PAGE),
        "the buffer starts on a page boundary"
    );
    SLOT_START + received - page_len
}

/// How far an update has got, in percent, for the progress bar.
pub fn percent(received: u32, image_size: u32) -> u8 {
    // An image is never empty, nor bigger than the slot (so the product
    // fits), and never has more arrived than it holds.
    debug_assert!(0 < image_size && image_size <= CAPACITY && received <= image_size);
    (received * 100 / image_size) as u8
}

/// CRC-16/CCITT-FALSE (polynomial 0x1021, initial 0xFFFF), as Nordic's
/// `crc16_compute` and `scripts/package.py` compute it.
pub struct Crc16(u16);

impl Crc16 {
    pub fn new() -> Self {
        Self(0xFFFF)
    }

    pub fn update(&mut self, data: &[u8]) {
        for &byte in data {
            self.0 ^= u16::from(byte) << 8;
            for _ in 0..8 {
                self.0 = if self.0 & 0x8000 != 0 {
                    (self.0 << 1) ^ 0x1021
                } else {
                    self.0 << 1
                };
            }
        }
    }

    pub fn finish(self) -> u16 {
        self.0
    }
}
