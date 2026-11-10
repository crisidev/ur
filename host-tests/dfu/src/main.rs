//! Host tests for the firmware's `dfu_slot` module: where a firmware
//! update's erases and programs land in the SPI flash. `cargo test` runs
//! them.

// Used only by the tests (and the proofs).
#[path = "../../../src/dfu_slot.rs"]
#[allow(dead_code)]
mod dfu_slot;

fn main() {
    println!("run `cargo test`");
}

/// `dfu::Session::receive`'s gathering of packets into pages, with the
/// flash (and the protocol) left out: a model of it, kept beside the tests
/// and proofs that use it, since `dfu` itself only builds for the watch.
///
/// It must match `receive`: bytes go into the page buffer until it's full
/// or the image is complete, and then the buffer is programmed at
/// [`dfu_slot::page_address`] and emptied.
#[cfg(any(test, kani))]
#[derive(Clone, Copy, Debug)]
struct Gather {
    image_size: u32,
    received: u32,
    /// How much the page buffer holds.
    page_len: u32,
}

#[cfg(any(test, kani))]
impl Gather {
    fn new(image_size: u32) -> Self {
        Self {
            image_size,
            received: 0,
            page_len: 0,
        }
    }

    /// One pass of `receive`'s loop, over a packet with `left` bytes
    /// still to take: how many it took, and what it programmed (address
    /// and length), if anything.
    fn step(&mut self, left: u32) -> (u32, Option<(u32, u32)>) {
        let now = (dfu_slot::PAGE - self.page_len).min(left);
        self.page_len += now;
        self.received += now;
        let complete = self.received == self.image_size;
        if !(self.page_len == dfu_slot::PAGE || complete) {
            return (now, None);
        }
        let program = (
            dfu_slot::page_address(self.received, self.page_len),
            self.page_len,
        );
        self.page_len = 0;
        (now, Some(program))
    }
}

#[cfg(test)]
mod tests {
    use super::Gather;
    use super::dfu_slot::*;

    /// The catalogue's check value for CRC-16/CCITT-FALSE.
    #[test]
    fn the_crc_is_ccitt_false() {
        let mut crc = Crc16::new();
        crc.update(b"123456789");
        assert_eq!(crc.finish(), 0x29B1);
        assert_eq!(Crc16::new().finish(), 0xFFFF);
    }

    /// As `dfu` feeds it: the slot read back a page at a time.
    #[test]
    fn the_crc_of_pieces_is_the_crc_of_the_whole() {
        let image: Vec<u8> = (0..1000u32).map(|i| (i * 7 + i / 3) as u8).collect();
        let mut whole = Crc16::new();
        whole.update(&image);
        let whole = whole.finish();
        for piece in [1, 7, 20, 256, 999, 1000] {
            let mut crc = Crc16::new();
            image.chunks(piece).for_each(|chunk| crc.update(chunk));
            assert_eq!(crc.finish(), whole, "in pieces of {piece}");
        }
    }

    #[test]
    fn the_slot_is_the_bootloaders() {
        // As in `memory.x`, `spi_flash`'s map and `scripts/package.py`.
        assert_eq!(SLOT_START..SLOT_START + SLOT_SIZE, 0x4_0000..0xB_4000);
        assert_eq!(CAPACITY, 474_704);
        assert_eq!(MAGIC_ADDRESS, 0xB_3FF0);
        assert!(in_slot(SLOT_START..SLOT_START + SLOT_SIZE));
        assert!(in_slot(SLOT_START..SLOT_START));
        assert!(!in_slot(SLOT_START - 1..SLOT_START + 1));
        assert!(!in_slot(SLOT_START..SLOT_START + SLOT_SIZE + 1));
        // A range that wrapped round is no range.
        #[allow(clippy::reversed_empty_ranges)]
        let backwards = SLOT_START + 2..SLOT_START + 1;
        assert!(!in_slot(backwards));
    }

    #[test]
    fn the_erase_is_the_images_sectors_and_the_trailers() {
        let trailer = 0xB_3000..0xB_4000;
        assert_eq!(erase_ranges(1), (0x4_0000..0x4_1000, trailer.clone()));
        assert_eq!(erase_ranges(4096), (0x4_0000..0x4_1000, trailer.clone()));
        assert_eq!(erase_ranges(4097), (0x4_0000..0x4_2000, trailer.clone()));
        // An image that ends in the trailer's sector: that sector once.
        assert_eq!(erase_ranges(CAPACITY), (0x4_0000..0xB_3000, trailer));
    }

    #[test]
    fn progress_is_a_percentage() {
        assert_eq!(percent(0, 400_000), 0);
        assert_eq!(percent(200_000, 400_000), 50);
        assert_eq!(percent(399_999, 400_000), 99);
        assert_eq!(percent(CAPACITY, CAPACITY), 100);
    }

    /// A whole update, in packets of every size a client might use (and
    /// some it wouldn't): each byte of the image is programmed once, in
    /// order, into flash that `erase_ranges` erased, a page at a time.
    #[test]
    fn an_image_is_programmed_once_page_by_page() {
        for image_size in [1, 255, 256, 257, 4096, 100_003, CAPACITY - 1, CAPACITY] {
            for packet in [1, 19, 20, 244, 255, 256, 257, 1000] {
                let (image, trailer) = erase_ranges(image_size);
                let mut gather = Gather::new(image_size);
                let mut programmed = SLOT_START;
                while gather.received < image_size {
                    let mut left = packet.min(image_size - gather.received);
                    while left > 0 {
                        let (took, program) = gather.step(left);
                        left -= took;
                        let Some((address, len)) = program else {
                            continue;
                        };
                        assert_eq!(address, programmed, "in order, with no gap");
                        assert!(address % PAGE + len <= PAGE, "within one page");
                        assert!(
                            image.contains(&address) && address + len <= image.end
                                || trailer.contains(&address) && address + len <= MAGIC_ADDRESS,
                            "into erased flash, short of the swap request"
                        );
                        programmed += len;
                    }
                }
                assert_eq!(programmed, SLOT_START + image_size);
                assert_eq!(gather.page_len, 0, "nothing left in the buffer");
            }
        }
    }

    /// The module's own check: a buffer that doesn't start on a page
    /// boundary is a bug in the caller.
    #[test]
    #[should_panic(expected = "page boundary")]
    fn a_page_off_its_boundary_is_caught() {
        page_address(300, 20);
    }
}

/// Proofs, for `cargo kani`: what the tests above check for some image
/// sizes and packet sizes, for every one. Each is plain arithmetic on
/// 32-bit numbers; the one loop takes two passes at most.
#[cfg(kani)]
mod proofs {
    use super::Gather;
    use super::dfu_slot::*;

    /// Any size `dfu::Session::accept_sizes` lets through.
    fn any_image_size() -> u32 {
        let size: u32 = kani::any();
        kani::assume(0 < size && size <= CAPACITY);
        size
    }

    /// For every image size: the two ranges erased are whole sectors
    /// inside the slot (so `spi_flash::erase` takes them, and nothing
    /// beside the slot is erased), they don't overlap (no sector is
    /// erased twice), and between them they cover the whole image and the
    /// swap request's place.
    #[kani::proof]
    fn the_erase_stays_in_the_slot_and_covers_the_image() {
        let image_size = any_image_size();
        let (image, trailer) = erase_ranges(image_size);
        for range in [&image, &trailer] {
            assert!(in_slot(range.clone()));
            assert!(range.start.is_multiple_of(SECTOR) && range.end.is_multiple_of(SECTOR));
            assert!(range.start < range.end);
        }
        assert_eq!(image.start, SLOT_START);
        assert!(image.end <= trailer.start);
        assert_eq!(trailer.end, SLOT_START + SLOT_SIZE);
        // Every byte of the image is in one or the other.
        let end = SLOT_START + image_size;
        assert!(end <= image.end || (image.end == trailer.start && end <= trailer.end));
        // And so is the swap request, which no image reaches.
        assert!(trailer.contains(&MAGIC_ADDRESS) && MAGIC_ADDRESS + 16 == trailer.end);
        assert!(end <= MAGIC_ADDRESS);
    }

    /// For every image size, every count of bytes received, and every
    /// fill of the page buffer that starts on a page boundary (which
    /// `gathering_keeps_the_buffer_on_a_page_boundary` shows it always
    /// does): the program is within one page (so the chip doesn't wrap it
    /// round), inside what `erase_ranges` erased, and short of the swap
    /// request. So an update can't write outside the slot, over
    /// unerased flash, or ask for a swap by accident.
    #[kani::proof]
    fn a_page_lands_in_one_page_of_what_was_erased() {
        let image_size = any_image_size();
        let (received, page_len): (u32, u32) = kani::any();
        kani::assume(0 < page_len && page_len <= PAGE);
        kani::assume(page_len <= received && received <= image_size);
        kani::assume((received - page_len).is_multiple_of(PAGE));
        let address = page_address(received, page_len);
        let end = address + page_len;

        assert!(address % PAGE + page_len <= PAGE);
        assert!(in_slot(address..end));
        assert_eq!(end, SLOT_START + received);
        let (image, trailer) = erase_ranges(image_size);
        assert!(
            (image.start <= address && end <= image.end)
                || (trailer.start <= address && end <= MAGIC_ADDRESS)
        );
    }

    /// For every image size, any point in an update at which the page
    /// buffer starts on a page boundary and isn't full (as it is at the
    /// start: empty), and any packet that doesn't overrun the image:
    /// every program on the way met `page_address`'s conditions (its
    /// `debug_assert!`s are checked here), and the programs follow one
    /// another without a gap; after the packet, the buffer is empty if
    /// that was the image's end, and otherwise again starts on a page
    /// boundary and isn't full. By induction over the packets, that
    /// holds for a whole update, whatever its packets' sizes.
    ///
    /// Packets are up to 244 bytes (the characteristic's size), and the
    /// loop takes at most two passes over one: what fits the buffer, then
    /// the rest (under a page).
    #[kani::proof]
    #[kani::unwind(3)]
    fn gathering_keeps_the_buffer_on_a_page_boundary() {
        let mut gather = Gather::new(any_image_size());
        gather.received = kani::any();
        gather.page_len = kani::any();
        kani::assume(gather.received < gather.image_size);
        kani::assume(gather.page_len < PAGE && gather.page_len == gather.received % PAGE);

        let packet: u32 = kani::any();
        kani::assume(0 < packet && packet <= 244);
        kani::assume(gather.received + packet <= gather.image_size);

        let mut next = SLOT_START + gather.received - gather.page_len;
        let mut left = packet;
        while left > 0 {
            let (took, program) = gather.step(left);
            assert!(0 < took && took <= left);
            left -= took;
            if let Some((address, len)) = program {
                assert_eq!(address, next);
                next += len;
            }
        }
        assert_eq!(next, SLOT_START + gather.received - gather.page_len);
        if gather.received == gather.image_size {
            // Complete: all of it programmed, and nothing more comes.
            assert_eq!(gather.page_len, 0);
        } else {
            assert!(gather.page_len < PAGE && gather.page_len == gather.received % PAGE);
        }
    }

    /// For every image size and every count received: the percentage
    /// doesn't overflow on the way, is at most 100, and is 100 only when
    /// it's all there.
    #[kani::proof]
    fn progress_is_at_most_a_hundred() {
        let image_size = any_image_size();
        let received: u32 = kani::any();
        kani::assume(received <= image_size);
        let percent = percent(received, image_size);
        assert!(percent <= 100);
        assert_eq!(percent == 100, received == image_size);
    }
}
