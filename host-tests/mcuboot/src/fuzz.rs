//! Fuzzing, with Bolero: an update's bytes come from the phone, so the
//! header, the TLV area and the hash see whatever a sender likes. The
//! tests above try the fixture with one thing wrong at a time; these build
//! images of any shape (any header size, firmware, vector table, TLV
//! entries, a byte flipped anywhere), feed them in pieces of any size, and
//! hold the verdict to what MCUBoot's layout says it should be, with the
//! `sha2` crate's SHA-256 for reference.
//!
//! `cargo test` runs each harness over a few hundred random inputs (and
//! any saved in a corpus), so they're regression tests too. To fuzz one
//! properly (nightly, for the sanitizer):
//!
//! ```text
//! cargo +nightly bolero test fuzz::any_image_gets_its_verdict_in_any_pieces --engine libfuzzer -T 10min
//! ```

use super::mcuboot::{Check, HEADER_LEN, Header, Problem, Sha256, Version};
use sha2::Digest;

/// What `dfu` allows: the secondary slot, less MCUBoot's trailer.
const CAPACITY: u32 = 0x74000 - 0x1B0;

/// Feeds `image` to a [`Check`] in pieces cut at `cuts` (each a length,
/// the rest at the end), as `dfu` would, reading it back from flash.
fn check_in(image: &[u8], cuts: &[u16]) -> Result<Version, Problem> {
    let len = u32::try_from(image.len()).expect("test images are small");
    let header = Header::parse(image, len, CAPACITY)?;
    let mut check = Check::new(header, len);
    let mut rest = image;
    for &cut in cuts {
        let (now, later) = rest.split_at(usize::from(cut).min(rest.len()));
        check.update(now);
        rest = later;
    }
    check.update(rest);
    check.finish()
}

// ==============================================================================
// The header
// ==============================================================================

/// Any header, of an image of any length, for a slot of any size, is read
/// as `struct image_header` says (`bootutil/image.h`): refused without
/// MCUBoot's magic, with a header smaller than one, protected TLVs, or
/// flags, and unless header, firmware (of at least two vectors) and a TLV
/// header fit exactly in what arrived, within the slot; the sums done
/// without wrapping round.
#[test]
fn any_header_reads_as_image_h_says() {
    bolero::check!()
        .with_type::<(Vec<u8>, u8, u32, u32)>()
        .for_each(|(bytes, shape, len, capacity)| {
            let mut bytes = bytes.clone();
            // Mostly a header that gets as far as the sizes: the magic, no
            // protected TLVs or flags, and an image as long as it says
            // (give or take a little), so that the fuzzer needn't find
            // them by itself.
            if shape & 1 != 0 && bytes.len() >= HEADER_LEN {
                bytes[..4].copy_from_slice(&0x96f3_b83du32.to_le_bytes());
            }
            if shape & 2 != 0 && bytes.len() >= HEADER_LEN {
                bytes[10..12].fill(0);
                bytes[16..20].fill(0);
            }
            let field = |at: usize, n: usize| {
                bytes[at..at + n]
                    .iter()
                    .rev()
                    .fold(0u64, |sum, &b| sum << 8 | u64::from(b))
            };
            let len = if shape & 4 != 0 && bytes.len() >= HEADER_LEN {
                let claimed = field(8, 2) + field(12, 4) + 4;
                (claimed as u32)
                    .wrapping_add(u32::from(shape >> 4))
                    .wrapping_sub(8)
            } else {
                *len
            };
            // A slot is some of a flash chip: at most 2 GB, say. (With
            // one of nearly 4 GB, and an image as long, `parse` lets
            // through a header whose sizes come to a byte or so more than
            // there is: its sum saturates rather than overflowing. `Check`
            // refuses the image later, and no slot is that big.)
            let capacity = if shape & 8 != 0 {
                CAPACITY
            } else {
                capacity % (1 << 31)
            };

            let parsed = Header::parse(&bytes, len, capacity);
            let expected = if bytes.len() < HEADER_LEN {
                Err(Problem::WrongSize)
            } else if field(0, 4) != 0x96f3_b83d {
                Err(Problem::NotAnImage)
            } else if field(8, 2) < 32 || field(10, 2) != 0 || field(16, 4) != 0 {
                Err(Problem::Unsupported)
            } else if field(12, 4) < 8
                || field(8, 2) + field(12, 4) + 4 > u64::from(len)
                || len > capacity
            {
                Err(Problem::WrongSize)
            } else {
                Ok(Version {
                    major: bytes[20],
                    minor: bytes[21],
                    revision: field(22, 2) as u16,
                    build: field(24, 4) as u32,
                })
            };
            assert_eq!(parsed.map(|header| header.version), expected);
        });
}

// ==============================================================================
// Whole images
// ==============================================================================

/// What an image is made of, for [`any_image_gets_its_verdict_in_any_pieces`].
#[derive(Debug, bolero::TypeGenerator)]
struct Shape {
    /// Past the 32 bytes of the header proper, padding (the packaging
    /// pads to 0x100).
    padding: u8,
    /// The firmware after its first two vectors.
    firmware: Vec<u8>,
    /// Its initial stack pointer and reset handler: if `None`, ones that
    /// fit (a stack at the top of RAM, a reset handler in the firmware).
    vectors: Option<(u32, u32)>,
    /// TLV entries besides the SHA-256 (a kind, and a value), put after
    /// it, or before it if `sha_last`.
    entries: Vec<(u16, Vec<u8>)>,
    sha_last: bool,
    /// A byte to flip, and where (modulo the image's length), if any.
    flip: Option<(u16, u8)>,
    /// Where to cut the image into pieces.
    cuts: Vec<u16>,
}

/// An image of any shape ([`Shape`]) is refused, or not, as MCUBoot's
/// layout says: when its vector table isn't for firmware in the primary
/// slot of an nRF52832, when its TLV area has a second SHA-256, or when a
/// byte is changed anywhere but in a TLV entry the bootloader ignores;
/// and it gets the same verdict however it's cut up on the way.
#[test]
fn any_image_gets_its_verdict_in_any_pieces() {
    bolero::check!().with_type::<Shape>().for_each(|shape| {
        let header_size = 32 + u16::from(shape.padding);
        let firmware_len = 8 + shape.firmware.len() as u32;
        // Where the firmware will run: after the header, in the primary
        // slot (0x8000..0x7C000).
        let start = 0x8000 + u32::from(header_size);
        let (stack, reset) = shape
            .vectors
            .unwrap_or((0x2001_0000, (start + firmware_len / 2) | 1));

        let mut image = Vec::new();
        image.extend(0x96f3_b83du32.to_le_bytes());
        image.extend(0u32.to_le_bytes());
        image.extend(header_size.to_le_bytes());
        image.extend(0u16.to_le_bytes());
        image.extend(firmware_len.to_le_bytes());
        image.extend(0u32.to_le_bytes());
        image.extend([1, 2]);
        image.extend(3u16.to_le_bytes());
        image.extend(4u32.to_le_bytes());
        image.resize(usize::from(header_size), 0xFF);
        image.extend(stack.to_le_bytes());
        image.extend(reset.to_le_bytes());
        image.extend(&shape.firmware);
        let hashed = image.len();
        let digest = sha2::Sha256::digest(&image);

        // The TLV area: its header (magic, and its length, itself
        // included), then the entries, the SHA-256 among them. Values no
        // longer than a u16 says.
        let mut entries: Vec<(u16, &[u8])> = shape
            .entries
            .iter()
            .map(|(kind, value)| (*kind, &value[..value.len().min(usize::from(u16::MAX))]))
            .collect();
        let sha_at = if shape.sha_last { entries.len() } else { 0 };
        entries.insert(sha_at, (0x10, &digest[..]));
        let total = 4 + entries
            .iter()
            .map(|(_, value)| 4 + value.len())
            .sum::<usize>();
        let Ok(total) = u16::try_from(total) else {
            return;
        };
        image.extend(0x6907u16.to_le_bytes());
        image.extend(total.to_le_bytes());
        // Where each entry is, but the SHA-256's. The bootloader reads
        // nothing else of them, and nothing covers them, so a byte changed
        // in one (its kind, its length, its value) may or may not matter:
        // a length may change the entries after it into others that
        // parse.
        let mut ignored = Vec::new();
        for (kind, value) in &entries {
            let at = image.len();
            image.extend(kind.to_le_bytes());
            image.extend((value.len() as u16).to_le_bytes());
            image.extend(*value);
            if *kind != 0x10 {
                ignored.push(at..image.len());
            }
        }

        // What it should come to, as built.
        let shas = entries.iter().filter(|(kind, _)| *kind == 0x10).count();
        let fits = (0x2000_0000..=0x2001_0000).contains(&stack)
            && stack % 4 == 0
            && reset & 1 == 1
            && (start..start + firmware_len).contains(&(reset & !1));
        let expected = if shas != 1 {
            Err(Problem::BadTlvs)
        } else if !fits {
            Err(Problem::NotForThisWatch)
        } else {
            Ok(Version {
                major: 1,
                minor: 2,
                revision: 3,
                build: 4,
            })
        };

        let flipped = shape.flip.and_then(|(at, bits)| {
            let at = usize::from(at) % image.len();
            (bits != 0).then(|| {
                image[at] ^= bits;
                at
            })
        });

        let verdict = check_in(&image, &shape.cuts);
        // In one piece, or a byte at a time: the same.
        assert_eq!(check_in(&image, &[]), verdict);
        assert_eq!(check_in(&image, &[1; 64]), verdict);

        match flipped {
            None => assert_eq!(verdict, expected, "{} bytes, {hashed} hashed", image.len()),
            // A good image with a byte changed is refused. (A bad one may
            // be put right by it: a second SHA-256's kind changed, say.)
            Some(at) if expected.is_ok() && !ignored.iter().any(|entry| entry.contains(&at)) => {
                assert!(verdict.is_err(), "byte {at:#x} changed, and passed");
            }
            Some(_) => {}
        }
    });
}

/// Any bytes, with MCUBoot's magic or not, in any pieces: nothing panics
/// (or overflows), and the verdict is the same however they're cut.
#[test]
fn any_bytes_get_one_verdict() {
    bolero::check!()
        .with_type::<(Vec<u8>, bool, Vec<u16>)>()
        .for_each(|(bytes, magic, cuts)| {
            let mut image = bytes.clone();
            if *magic && image.len() >= 4 {
                image[..4].copy_from_slice(&0x96f3_b83du32.to_le_bytes());
            }
            assert_eq!(check_in(&image, cuts), check_in(&image, &[]));
        });
}

// ==============================================================================
// SHA-256
// ==============================================================================

/// The SHA-256 of any data, fed in any pieces, is the `sha2` crate's.
#[test]
fn sha256_of_any_pieces_is_sha2s() {
    bolero::check!()
        .with_type::<(Vec<u8>, u8, Vec<u16>)>()
        .for_each(|(data, times, cuts)| {
            // Long data too: the first byte says how many times over.
            let data = data.repeat(1 + usize::from(*times) % 16);
            let mut sha = Sha256::new();
            let mut rest = &data[..];
            for &cut in cuts {
                let (now, later) = rest.split_at(usize::from(cut).min(rest.len()));
                sha.update(now);
                rest = later;
            }
            sha.update(rest);
            assert_eq!(sha.finish()[..], sha2::Sha256::digest(&data)[..]);
        });
}
