//! MCUBoot images, as the PineTime's bootloader installs them: checking an
//! update before the bootloader is asked to swap it in.
//!
//! [`crate::dfu`] receives an image into the secondary slot. The stock
//! bootloader can't be replaced, and was built without signature checks:
//! it takes any image with a valid SHA-256 of itself. So what reaches the
//! slot is gated by who may send it (only a client paired with the passkey;
//! see `ble::DfuService`), and checked here, by what it is:
//!
//! - its header: MCUBoot's magic, a header no smaller than itself, nothing
//!   this bootloader can't do (encryption, protected TLVs, other flags),
//!   and sizes that add up to exactly what arrived, within the slot;
//! - its TLV area (the "trailer" after the firmware): well-formed, with
//!   exactly one SHA-256;
//! - that SHA-256, over the header and firmware as read back from flash;
//! - its vector table: a stack in the nRF52832's RAM and a reset handler
//!   inside the image, as it will sit in the primary slot (InfiniTime's
//!   images pass too, with their 32-byte header).
//!
//! The bootloader checks the hash too, but finding out there means a reset,
//! a swap that doesn't happen, and nothing on the screen to say why; and a
//! protocol CRC-16 (all the legacy DFU protocol has) misses one corruption
//! in 65,536. Checking here refuses a bad image while the sender is still
//! connected to hear it, before anything asks for a swap.
//!
//! What this can't do is tell a good image from a malicious one: without
//! signatures, anyone who may send an update may send any firmware they
//! like, which is why sending one takes the passkey. Nor does it refuse an
//! older version: see [`Version`].
//!
//! Hardware-free, with host tests (`host-tests/mcuboot`).

use core::ops::Range;

use defmt::Format;

// ==============================================================================
// Layout
// ==============================================================================

/// MCUBoot's image header: `struct image_header` in `bootutil/image.h`.
pub const HEADER_LEN: usize = 32;
const IMAGE_MAGIC: u32 = 0x96f3_b83d;
/// The (unprotected) TLV area's header magic.
const TLV_INFO_MAGIC: u16 = 0x6907;
const TLV_SHA256: u16 = 0x10;

/// Where the bootloader runs an image from: the primary slot, in internal
/// flash (see `memory.x`).
const PRIMARY_SLOT: Range<u32> = 0x8000..0x7C000;
/// The nRF52832's RAM; an initial stack pointer may sit at its very end.
const RAM: Range<u32> = 0x2000_0000..0x2001_0001;

/// Why an image was refused.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Format)]
pub enum Problem {
    /// No MCUBoot header: not firmware for this bootloader at all.
    NotAnImage,
    /// A header the bootloader here can't act on: smaller than a header,
    /// encrypted, with protected TLVs, or with other flags.
    Unsupported,
    /// What arrived isn't the size the header says (cut short, or with
    /// more after it), or wouldn't fit the slot.
    WrongSize,
    /// The TLV area after the firmware is malformed, or doesn't hold
    /// exactly one SHA-256.
    BadTlvs,
    /// The SHA-256 doesn't match: corrupted on the way, or in flash.
    Corrupt,
    /// Intact, but its vector table isn't one for firmware running from
    /// the primary slot of an nRF52832: built for something else.
    NotForThisWatch,
}

/// An image's version (`struct image_version`), as `scripts/package.py`
/// sets it from Cargo.toml.
///
/// Logged, not enforced: refusing older versions would protect nobody
/// (without signatures, a sender can write any version it likes into an
/// image), and would block a deliberate rollback.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Format)]
pub struct Version {
    pub major: u8,
    pub minor: u8,
    pub revision: u16,
    pub build: u32,
}

/// What an image's header says, checked against how much arrived.
#[derive(Clone, Copy, Debug)]
pub struct Header {
    /// Its own size: the firmware starts here.
    header_size: u32,
    /// Header plus firmware: what the SHA-256 covers, and where the TLV
    /// area starts.
    hashed: u32,
    pub version: Version,
}

impl Header {
    /// Reads the header at the start of an image that's `len` bytes in
    /// all, for a slot that holds at most `capacity` bytes of image.
    ///
    /// Needs only the first [`HEADER_LEN`] bytes, so it can refuse an
    /// update as soon as they arrive; [`Check`] confirms the rest.
    pub fn parse(bytes: &[u8], len: u32, capacity: u32) -> Result<Self, Problem> {
        let bytes: &[u8; HEADER_LEN] = bytes
            .get(..HEADER_LEN)
            .and_then(|b| b.try_into().ok())
            .ok_or(Problem::WrongSize)?;
        let u16_at = |at: usize| u16::from_le_bytes([bytes[at], bytes[at + 1]]);
        let u32_at = |at: usize| {
            u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
        };
        if u32_at(0) != IMAGE_MAGIC {
            return Err(Problem::NotAnImage);
        }
        // The load address (at 4) means nothing to an execute-in-place
        // image, and the bootloader ignores it; so do we.
        let header_size = u32::from(u16_at(8));
        let protected_tlvs = u16_at(10);
        let firmware_size = u32_at(12);
        let flags = u32_at(16);
        if header_size < HEADER_LEN as u32 || protected_tlvs != 0 || flags != 0 {
            return Err(Problem::Unsupported);
        }
        // The firmware needs at least its first two vectors, and the TLV
        // area at least its own 4-byte header.
        let hashed = header_size
            .checked_add(firmware_size)
            .ok_or(Problem::WrongSize)?;
        if firmware_size < 8 || hashed.saturating_add(4) > len || len > capacity {
            return Err(Problem::WrongSize);
        }
        Ok(Self {
            header_size,
            hashed,
            version: Version {
                major: bytes[20],
                minor: bytes[21],
                revision: u16_at(22),
                build: u32_at(24),
            },
        })
    }
}

// ==============================================================================
// Checking a whole image
// ==============================================================================

/// Checks a whole image, fed in order, in pieces of any size: one pass over
/// the slot, with no buffer beyond the hash's.
pub struct Check {
    header: Header,
    len: u32,
    /// How many bytes have been fed.
    at: u32,
    sha: Sha256,
    /// The first two words of the vector table: initial stack pointer and
    /// reset handler.
    vectors: [u8; 8],
    tlvs: Tlvs,
}

impl Check {
    /// For an image of `len` bytes whose header [`Header::parse`] read.
    pub fn new(header: Header, len: u32) -> Self {
        Self {
            header,
            len,
            at: 0,
            sha: Sha256::new(),
            vectors: [0; 8],
            tlvs: Tlvs::default(),
        }
    }

    /// The image's next bytes.
    pub fn update(&mut self, data: &[u8]) {
        let hashed = self.header.hashed;
        // The hash takes its share in one go.
        if self.at < hashed {
            let take = ((hashed - self.at) as usize).min(data.len());
            self.sha.update(&data[..take]);
        }
        for &byte in data {
            let at = self.at;
            if at < hashed {
                if let Some(i) = at.checked_sub(self.header.header_size)
                    && let Some(slot) = self.vectors.get_mut(i as usize)
                {
                    *slot = byte;
                }
            } else {
                self.tlvs.push(byte);
            }
            self.at = at.saturating_add(1);
        }
    }

    /// The verdict, once every byte has been fed: the image's version if
    /// it's fit to install.
    pub fn finish(self) -> Result<Version, Problem> {
        if self.at != self.len {
            return Err(Problem::WrongSize);
        }
        let expected = self.tlvs.finish(self.len - self.header.hashed)?;
        if self.sha.finish() != expected {
            return Err(Problem::Corrupt);
        }
        let word = |at: usize| {
            u32::from_le_bytes([
                self.vectors[at],
                self.vectors[at + 1],
                self.vectors[at + 2],
                self.vectors[at + 3],
            ])
        };
        let (stack, reset) = (word(0), word(4));
        // Where the firmware will run: after the header, in the primary
        // slot. The reset handler is Thumb code, so its address is odd.
        let firmware = PRIMARY_SLOT.start + self.header.header_size
            ..PRIMARY_SLOT.start.saturating_add(self.header.hashed);
        if !RAM.contains(&stack)
            || !stack.is_multiple_of(4)
            || reset & 1 == 0
            || !firmware.contains(&(reset & !1))
            || firmware.end > PRIMARY_SLOT.end
        {
            return Err(Problem::NotForThisWatch);
        }
        Ok(self.header.version)
    }
}

/// Reads the TLV area after the firmware, a byte at a time: a 4-byte
/// header (magic, and the area's length including itself), then entries,
/// each a type, a length (both `u16`), and that many bytes of value.
#[derive(Default)]
struct Tlvs {
    /// The 4-byte header being read (the area's, or an entry's), and how
    /// much of it has arrived.
    field: [u8; 4],
    filled: u8,
    /// The area's length, once its header is in.
    total: Option<u16>,
    /// What's left of the current entry's value, and whether it's the
    /// SHA-256 (being copied into `sha256`).
    left: u16,
    in_sha256: bool,
    sha256: [u8; 32],
    /// How many SHA-256 entries there were.
    sha256s: u8,
    /// Set once something doesn't parse; the rest is ignored.
    broken: bool,
}

impl Tlvs {
    fn push(&mut self, byte: u8) {
        if self.broken {
            return;
        }
        if self.left > 0 {
            if self.in_sha256 {
                self.sha256[32 - usize::from(self.left)] = byte;
            }
            self.left -= 1;
            return;
        }
        self.field[usize::from(self.filled)] = byte;
        self.filled += 1;
        if self.filled < 4 {
            return;
        }
        self.filled = 0;
        let [a, b, c, d] = self.field;
        let (kind, len) = (u16::from_le_bytes([a, b]), u16::from_le_bytes([c, d]));
        if self.total.is_none() {
            self.broken = kind != TLV_INFO_MAGIC;
            self.total = Some(len);
            return;
        }
        self.in_sha256 = kind == TLV_SHA256;
        if self.in_sha256 {
            self.sha256s = self.sha256s.saturating_add(1);
            self.broken = len != 32;
        }
        self.left = len;
    }

    /// The SHA-256 the area holds, given that it was `len` bytes long.
    fn finish(self, len: u32) -> Result<[u8; 32], Problem> {
        let Some(total) = self.total else {
            return Err(Problem::BadTlvs);
        };
        if u32::from(total) != len {
            return Err(Problem::WrongSize);
        }
        // An entry cut short by the area's end, or not one SHA-256.
        if self.broken || self.filled != 0 || self.left != 0 || self.sha256s != 1 {
            return Err(Problem::BadTlvs);
        }
        Ok(self.sha256)
    }
}

// ==============================================================================
// SHA-256
// ==============================================================================

/// SHA-256 (FIPS 180-4), small rather than fast: the nRF52832 has no hash
/// hardware, and a whole image takes a fraction of a second either way,
/// next to reading it out of the SPI flash.
pub struct Sha256 {
    state: [u32; 8],
    block: [u8; 64],
    /// Bytes hashed so far.
    len: u64,
}

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

impl Sha256 {
    pub const fn new() -> Self {
        Self {
            state: [
                0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
                0x5be0cd19,
            ],
            block: [0; 64],
            len: 0,
        }
    }

    pub fn update(&mut self, data: &[u8]) {
        for &byte in data {
            self.block[(self.len % 64) as usize] = byte;
            self.len += 1;
            if self.len.is_multiple_of(64) {
                self.compress();
            }
        }
    }

    pub fn finish(mut self) -> [u8; 32] {
        let bits = self.len.wrapping_mul(8);
        self.update(&[0x80]);
        while self.len % 64 != 56 {
            self.update(&[0]);
        }
        self.update(&bits.to_be_bytes());
        let mut digest = [0; 32];
        for (out, word) in digest.as_chunks_mut::<4>().0.iter_mut().zip(self.state) {
            *out = word.to_be_bytes();
        }
        digest
    }

    fn compress(&mut self) {
        // The message schedule, sixteen words at a time (each word only
        // needs the sixteen before it): a quarter of the stack of all 64.
        let mut w = [0u32; 16];
        for (word, bytes) in w.iter_mut().zip(self.block.as_chunks::<4>().0) {
            *word = u32::from_be_bytes(*bytes);
        }
        let mut v = self.state;
        for (i, k) in K.iter().enumerate() {
            if i >= 16 {
                let (w15, w2) = (w[(i + 1) % 16], w[(i + 14) % 16]);
                let s0 = w15.rotate_right(7) ^ w15.rotate_right(18) ^ (w15 >> 3);
                let s1 = w2.rotate_right(17) ^ w2.rotate_right(19) ^ (w2 >> 10);
                w[i % 16] = w[i % 16]
                    .wrapping_add(s0)
                    .wrapping_add(w[(i + 9) % 16])
                    .wrapping_add(s1);
            }
            let [a, b, c, d, e, f, g, h] = v;
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let choice = (e & f) ^ (!e & g);
            let t1 = h
                .wrapping_add(s1)
                .wrapping_add(choice)
                .wrapping_add(*k)
                .wrapping_add(w[i % 16]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let majority = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(majority);
            v = [t1.wrapping_add(t2), a, b, c, d.wrapping_add(t1), e, f, g];
        }
        for (state, value) in self.state.iter_mut().zip(v) {
            *state = state.wrapping_add(value);
        }
    }
}
