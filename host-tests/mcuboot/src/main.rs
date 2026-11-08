//! Host tests for the firmware's `mcuboot` module: checking an update's
//! image before the bootloader is asked to install it. `cargo test` runs
//! them.
//!
//! `fixtures/package-py.img` is an image as `scripts/package.py` makes them
//! (its `mcuboot_image`, from a made-up vector table and filler), so the
//! check is held to what the packaging actually writes. To make it again:
//!
//! ```sh
//! python3 - <<'EOF'   # from the repository's root
//! import importlib.util, struct
//! spec = importlib.util.spec_from_file_location("package", "scripts/package.py")
//! package = importlib.util.module_from_spec(spec); spec.loader.exec_module(package)
//! firmware = struct.pack("<II", 0x20010000, 0x8100 + 0x200 + 1) + bytes((i * 7 + 3) % 256 for i in range(1000))
//! open("host-tests/mcuboot/fixtures/package-py.img", "wb").write(package.mcuboot_image(firmware, (0, 9, 1)))
//! EOF
//! ```

#[path = "../../../src/mcuboot.rs"]
mod mcuboot;

// `mcuboot` derives defmt's `Format`, whose binary log needs the
// firmware's ELF to decode: on the host, logs go nowhere.
#[defmt::global_logger]
struct Logger;

// SAFETY: Does nothing, so there's nothing to synchronise.
unsafe impl defmt::Logger for Logger {
    fn acquire() {}
    unsafe fn flush() {}
    unsafe fn release() {}
    unsafe fn write(_bytes: &[u8]) {}
}

defmt::timestamp!("");

fn main() {
    println!("run `cargo test`");
}

#[cfg(test)]
mod tests {
    use quickcheck::{QuickCheck, TestResult};
    use sha2::Digest;

    use super::mcuboot::{Check, HEADER_LEN, Header, Problem, Sha256, Version};

    /// What `dfu` allows: the secondary slot, less MCUBoot's trailer.
    const CAPACITY: u32 = 0x74000 - 0x1B0;

    const FIXTURE: &[u8] = include_bytes!("../fixtures/package-py.img");

    /// Checks `image` as `dfu` does, handing it over in `chunk`-byte pieces.
    fn check_in(image: &[u8], chunk: usize) -> Result<Version, Problem> {
        let len = u32::try_from(image.len()).expect("test images are small");
        let header = Header::parse(image, len, CAPACITY)?;
        let mut check = Check::new(header, len);
        for piece in image.chunks(chunk.max(1)) {
            check.update(piece);
        }
        check.finish()
    }

    fn check(image: &[u8]) -> Result<Version, Problem> {
        check_in(image, 256)
    }

    /// An image as `package.py` (or imgtool) makes one: header (of
    /// `header_size`, padded with 0xFF), firmware, then the TLV area: its
    /// header, the SHA-256 of header and firmware, and `extra` entries.
    fn image(header_size: u16, firmware: &[u8], extra: &[(u16, &[u8])]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend(0x96f3_b83du32.to_le_bytes());
        out.extend(0u32.to_le_bytes());
        out.extend(header_size.to_le_bytes());
        out.extend(0u16.to_le_bytes());
        out.extend((firmware.len() as u32).to_le_bytes());
        out.extend(0u32.to_le_bytes());
        out.extend([1, 2]);
        out.extend(3u16.to_le_bytes());
        out.extend(4u32.to_le_bytes());
        out.extend(0u32.to_le_bytes());
        out.resize(usize::from(header_size), 0xFF);
        out.extend(firmware);
        let digest = sha2::Sha256::digest(&out);
        let mut tlvs = vec![(0x10u16, &digest[..])];
        tlvs.extend_from_slice(extra);
        let total: usize = 4 + tlvs.iter().map(|(_, v)| 4 + v.len()).sum::<usize>();
        out.extend(0x6907u16.to_le_bytes());
        out.extend((total as u16).to_le_bytes());
        for (kind, value) in tlvs {
            out.extend(kind.to_le_bytes());
            out.extend((value.len() as u16).to_le_bytes());
            out.extend(value);
        }
        out
    }

    /// Firmware linked to run after a `header_size` header in the primary
    /// slot: a vector table, then filler.
    fn firmware(header_size: u16, len: usize) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend(0x2001_0000u32.to_le_bytes());
        out.extend((0x8000 + u32::from(header_size) + 0x41).to_le_bytes());
        out.extend((0..len.saturating_sub(8)).map(|i| (i * 13 + 5) as u8));
        out
    }

    // ==========================================================================
    // Good images
    // ==========================================================================

    #[test]
    fn what_package_py_makes_passes() {
        assert_eq!(
            check(FIXTURE),
            Ok(Version {
                major: 0,
                minor: 9,
                revision: 1,
                build: 0
            })
        );
    }

    #[test]
    fn infinitimes_shape_passes() {
        // InfiniTime's images have the 32-byte header imgtool makes by
        // default, and the firmware right after it.
        let image = image(32, &firmware(32, 4000), &[]);
        assert_eq!(
            check(&image),
            Ok(Version {
                major: 1,
                minor: 2,
                revision: 3,
                build: 4
            })
        );
    }

    #[test]
    fn other_tlvs_are_skipped() {
        // E.g. a key hash and a signature, which this bootloader ignores.
        let image = image(
            0x100,
            &firmware(0x100, 600),
            &[(0x01, &[7; 32]), (0x20, &[9; 72])],
        );
        assert!(check(&image).is_ok());
    }

    #[test]
    fn any_split_gives_the_same_verdict() {
        fn property(chunk: u16, flip: Option<u16>) -> TestResult {
            let mut image = FIXTURE.to_vec();
            if let Some(at) = flip {
                let at = usize::from(at) % image.len();
                image[at] ^= 0x40;
            }
            let whole = check_in(&image, image.len());
            TestResult::from_bool(check_in(&image, usize::from(chunk)) == whole)
        }
        QuickCheck::new()
            .tests(2000)
            .quickcheck(property as fn(u16, Option<u16>) -> TestResult);
    }

    // ==========================================================================
    // Damaged images
    // ==========================================================================

    #[test]
    fn every_flipped_bit_is_refused() {
        for at in 0..FIXTURE.len() {
            for bit in 0..8 {
                let mut image = FIXTURE.to_vec();
                image[at] ^= 1 << bit;
                assert!(check(&image).is_err(), "bit {bit} of byte {at:#x}");
            }
        }
    }

    #[test]
    fn a_flipped_firmware_byte_is_corrupt() {
        let mut image = FIXTURE.to_vec();
        image[0x300] ^= 0xFF;
        assert_eq!(check(&image), Err(Problem::Corrupt));
        let mut image = FIXTURE.to_vec();
        let last = image.len() - 1;
        image[last] ^= 0x01; // in the stored hash
        assert_eq!(check(&image), Err(Problem::Corrupt));
    }

    #[test]
    fn every_truncation_and_extension_is_refused() {
        for len in 0..FIXTURE.len() {
            assert!(check(&FIXTURE[..len]).is_err(), "cut to {len}");
        }
        for extra in [&[0u8][..], &[0xFF; 4], &[0x10, 0x00, 0x20, 0x00]] {
            let mut image = FIXTURE.to_vec();
            image.extend(extra);
            assert_eq!(check(&image), Err(Problem::WrongSize), "{extra:?}");
        }
        // Cut right after the firmware: no room for even the TLV header.
        assert_eq!(check(&FIXTURE[..0x100 + 1008]), Err(Problem::WrongSize));
    }

    #[test]
    fn arbitrary_bytes_are_refused_without_panicking() {
        fn property(bytes: Vec<u8>, magic: bool) -> bool {
            let mut image = bytes;
            if magic && image.len() >= 4 {
                image[..4].copy_from_slice(&0x96f3_b83du32.to_le_bytes());
            }
            for chunk in [1, 7, 256] {
                assert!(check_in(&image, chunk).is_err());
            }
            true
        }
        QuickCheck::new()
            .tests(5000)
            .quickcheck(property as fn(Vec<u8>, bool) -> bool);
    }

    // ==========================================================================
    // Headers
    // ==========================================================================

    /// The fixture with header bytes `at..` replaced by `bytes` (and the
    /// hash left as it was: these must be refused before it matters).
    fn with_header(at: usize, bytes: &[u8]) -> Vec<u8> {
        let mut image = FIXTURE.to_vec();
        image[at..at + bytes.len()].copy_from_slice(bytes);
        image
    }

    #[test]
    fn headers_we_cant_install() {
        let len = FIXTURE.len() as u32;
        let cases: &[(Vec<u8>, Problem)] = &[
            (with_header(0, &[0; 4]), Problem::NotAnImage),
            // InfiniTime's resources zip holds no image at all.
            (
                b"PK\x03\x04 a zip, not an image, with more after it".to_vec(),
                Problem::NotAnImage,
            ),
            (with_header(8, &31u16.to_le_bytes()), Problem::Unsupported),
            (with_header(8, &0u16.to_le_bytes()), Problem::Unsupported),
            // Protected TLVs, and flags: encrypted, RAM-load, non-bootable.
            (with_header(10, &4u16.to_le_bytes()), Problem::Unsupported),
            (
                with_header(16, &0x04u32.to_le_bytes()),
                Problem::Unsupported,
            ),
            (
                with_header(16, &0x20u32.to_le_bytes()),
                Problem::Unsupported,
            ),
            (
                with_header(16, &0x10u32.to_le_bytes()),
                Problem::Unsupported,
            ),
            // Firmware sizes that don't fit what arrived, or wrap round.
            (with_header(12, &len.to_le_bytes()), Problem::WrongSize),
            (with_header(12, &u32::MAX.to_le_bytes()), Problem::WrongSize),
            (
                with_header(12, &(u32::MAX - 0xFF).to_le_bytes()),
                Problem::WrongSize,
            ),
            (
                with_header(12, &(u32::MAX - 0x100 - 2).to_le_bytes()),
                Problem::WrongSize,
            ),
            (with_header(12, &4u32.to_le_bytes()), Problem::WrongSize),
            // A header bigger than the image.
            (with_header(8, &u16::MAX.to_le_bytes()), Problem::WrongSize),
        ];
        for (image, problem) in cases {
            assert_eq!(check(image), Err(*problem), "{:02x?}", &image[..24]);
        }
    }

    #[test]
    fn a_header_alone_says_enough_to_refuse_early() {
        let len = FIXTURE.len() as u32;
        let header = &FIXTURE[..HEADER_LEN];
        assert!(Header::parse(header, len, CAPACITY).is_ok());
        assert!(Header::parse(&header[..31], len, CAPACITY).is_err());
        // Too big for the slot, as announced.
        assert_eq!(
            Header::parse(header, CAPACITY + 1, CAPACITY).err(),
            Some(Problem::WrongSize)
        );
    }

    // ==========================================================================
    // TLVs
    // ==========================================================================

    /// An image with a hand-made TLV area (header magic, the total it
    /// claims, and the raw entries after it).
    fn with_tlvs(magic: u16, total: Option<u16>, entries: &[u8]) -> Vec<u8> {
        let firmware = firmware(0x100, 300);
        let mut image = image(0x100, &firmware, &[]);
        image.truncate(0x100 + firmware.len());
        image.extend(magic.to_le_bytes());
        image.extend(total.unwrap_or(4 + entries.len() as u16).to_le_bytes());
        image.extend(entries);
        image
    }

    fn entry(kind: u16, value: &[u8]) -> Vec<u8> {
        let mut out = kind.to_le_bytes().to_vec();
        out.extend((value.len() as u16).to_le_bytes());
        out.extend(value);
        out
    }

    #[test]
    fn tlv_areas_we_cant_trust() {
        let hash = |image: &[u8]| sha2::Sha256::digest(&image[..0x100 + 300]).to_vec();
        let good = image(0x100, &firmware(0x100, 300), &[]);
        let sha = entry(0x10, &hash(&good));
        let cases: Vec<(Vec<u8>, Problem)> = vec![
            // The protected area's magic, or none at all.
            (with_tlvs(0x6908, None, &sha), Problem::BadTlvs),
            (with_tlvs(0x0000, None, &sha), Problem::BadTlvs),
            // No hash, two hashes, a hash of the wrong length.
            (
                with_tlvs(0x6907, None, &entry(0x01, &[0; 32])),
                Problem::BadTlvs,
            ),
            (
                with_tlvs(0x6907, None, &[sha.clone(), sha.clone()].concat()),
                Problem::BadTlvs,
            ),
            (
                with_tlvs(0x6907, None, &entry(0x10, &[0; 20])),
                Problem::BadTlvs,
            ),
            // An entry running past the end, or a stray byte after one.
            (
                with_tlvs(0x6907, None, &sha[..sha.len() - 1]),
                Problem::BadTlvs,
            ),
            (
                with_tlvs(0x6907, None, &[&sha[..], &[0]].concat()),
                Problem::BadTlvs,
            ),
            // A total that isn't what arrived.
            (with_tlvs(0x6907, Some(40 + 4), &sha), Problem::WrongSize),
            (with_tlvs(0x6907, Some(40 - 4), &sha), Problem::WrongSize),
            (with_tlvs(0x6907, Some(u16::MAX), &sha), Problem::WrongSize),
        ];
        assert!(check(&with_tlvs(0x6907, None, &sha)).is_ok());
        for (image, problem) in cases {
            assert_eq!(
                check(&image),
                Err(problem),
                "{:02x?}",
                &image[0x100 + 300..]
            );
        }
    }

    // ==========================================================================
    // Vector tables
    // ==========================================================================

    #[test]
    fn firmware_for_somewhere_else_is_refused() {
        let with_vectors = |stack: u32, reset: u32| {
            let mut firmware = firmware(0x100, 500);
            firmware[..4].copy_from_slice(&stack.to_le_bytes());
            firmware[4..8].copy_from_slice(&reset.to_le_bytes());
            image(0x100, &firmware, &[])
        };
        let good_reset = 0x8100 + 0x41;
        assert!(check(&with_vectors(0x2001_0000, good_reset)).is_ok());
        assert!(check(&with_vectors(0x2000_8000, 0x8100 + 500 - 1)).is_ok());
        for (stack, reset) in [
            // An nRF52840's RAM (256 KB), and no RAM at all.
            (0x2004_0000, good_reset),
            (0x1FFF_FFF0, good_reset),
            (0x2001_0002, good_reset),
            // Linked for address 0 (no bootloader), for after a 32-byte
            // header when this one is 0x100, past the firmware's end, and
            // not Thumb code.
            (0x2001_0000, 0x0000_0201),
            (0x2001_0000, 0x8020 + 0x41),
            (0x2001_0000, 0x8100 + 500 + 1),
            (0x2001_0000, good_reset & !1),
        ] {
            assert_eq!(
                check(&with_vectors(stack, reset)),
                Err(Problem::NotForThisWatch),
                "{stack:#x} {reset:#x}"
            );
        }
    }

    // ==========================================================================
    // SHA-256
    // ==========================================================================

    fn sha256(data: &[u8]) -> [u8; 32] {
        let mut sha = Sha256::new();
        sha.update(data);
        sha.finish()
    }

    #[test]
    fn sha256_test_vectors() {
        // FIPS 180-4's examples, and the empty string.
        let cases: [(&[u8], &str); 3] = [
            (
                b"",
                "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            ),
            (
                b"abc",
                "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
            ),
            (
                b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq",
                "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1",
            ),
        ];
        for (data, hex) in cases {
            let digest: String = sha256(data).iter().map(|b| format!("{b:02x}")).collect();
            assert_eq!(digest, hex);
        }
    }

    #[test]
    fn sha256_matches_the_sha2_crate() {
        fn property(data: Vec<u8>, split: usize) -> bool {
            let split = split % (data.len() + 1);
            let mut sha = Sha256::new();
            sha.update(&data[..split]);
            sha.update(&data[split..]);
            sha.finish()[..] == sha2::Sha256::digest(&data)[..]
        }
        QuickCheck::new()
            .tests(2000)
            .quickcheck(property as fn(Vec<u8>, usize) -> bool);
        // Every length around the padding's edges.
        for len in 0..200 {
            let data: Vec<u8> = (0..len).map(|i| i as u8).collect();
            assert!(property(data, len / 2), "{len}");
        }
    }
}
