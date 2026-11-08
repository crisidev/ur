//! A QR code for Home Assistant's key (see [`crate::bthome`]), so that
//! it can be scanned with a phone and pasted into Home Assistant, rather
//! than typed: 32 hex digits read off a watch are easy to get wrong.
//!
//! Only what that takes: alphanumeric mode (digits and capitals, so the
//! key's hex fits in 176 bits), version 2 (25×25 modules), error
//! correction level M (15% of the code can be damaged), so a single block
//! of 28 data and 16 error-correction codewords, and one fixed mask
//! ([`MASK`]; the standard picks the mask that scores best for
//! scanners, but any mask is valid, and decoders read whichever the
//! format says). Checked module for module against a reference encoder
//! (`segno`) in `host-tests/qr`.
//!
//! The screen draws it at the panel's own resolution, not the canvas's
//! (see `Canvas::qr`): 7 panel pixels a module, quiet zone and all.
//!
//! Hardware-free, so it runs in host tests (`host-tests/qr`). Follows
//! Project Nayuki's QR code generator's structure, much reduced.

/// Modules along each side.
pub const SIZE: usize = 25;
/// The modules, a row each, bit `x` of row `y` set for a dark module.
pub type Matrix = [u32; SIZE];

/// How many data codewords version 2-M holds, and error-correction
/// codewords it adds.
const DATA: usize = 28;
const ECC: usize = 16;
/// The longest alphanumeric text that fits: 4 bits of mode, 9 of length,
/// 11 for every two characters.
pub const MAX_TEXT: usize = 38;
/// The mask: pattern 0, dark where `x + y` is even.
const MASK: u32 = 0;

/// Encodes `text` (digits, capitals, and ` $%*+-./:`, at most
/// [`MAX_TEXT`] characters). `None` if it can't.
pub fn encode(text: &[u8]) -> Option<Matrix> {
    if text.len() > MAX_TEXT {
        return None;
    }
    let data = codewords(text)?;
    let ecc = reed_solomon(&data);

    let mut qr = Qr {
        dark: [0; SIZE],
        function: [0; SIZE],
    };
    qr.function_patterns();
    // The data, then its error correction, as a stream of bits.
    let bit = |i: usize| {
        let byte = if i / 8 < DATA {
            data[i / 8]
        } else {
            ecc[i / 8 - DATA]
        };
        byte >> (7 - i % 8) & 1 != 0
    };
    qr.place(bit, (DATA + ECC) * 8);
    qr.mask();
    qr.format_bits();
    Some(qr.dark)
}

/// The text as data codewords: mode, length, the characters in pairs,
/// a terminator, and padding.
fn codewords(text: &[u8]) -> Option<[u8; DATA]> {
    let mut bits = Bits {
        bytes: [0; DATA],
        len: 0,
    };
    bits.push(0b0010, 4);
    bits.push(text.len() as u32, 9);
    for pair in text.chunks(2) {
        match *pair {
            [a, b] => bits.push(value(a)? * 45 + value(b)?, 11),
            [a] => bits.push(value(a)?, 6),
            _ => unreachable!("chunks of two"),
        }
    }
    // Up to four zeros, then to a byte boundary (the buffer's zeros).
    let capacity = DATA * 8;
    bits.len = (bits.len + 4).min(capacity).next_multiple_of(8);
    for pad in [0xEC, 0x11].into_iter().cycle() {
        if bits.len >= capacity {
            break;
        }
        bits.push(pad, 8);
    }
    Some(bits.bytes)
}

/// A character's alphanumeric value.
fn value(c: u8) -> Option<u32> {
    const SYMBOLS: &[u8] = b" $%*+-./:";
    Some(match c {
        b'0'..=b'9' => u32::from(c - b'0'),
        b'A'..=b'Z' => u32::from(c - b'A') + 10,
        _ => 36 + SYMBOLS.iter().position(|&s| s == c)? as u32,
    })
}

struct Bits {
    bytes: [u8; DATA],
    len: usize,
}

impl Bits {
    /// Appends the low `count` bits of `value`, most significant first.
    fn push(&mut self, value: u32, count: usize) {
        for i in (0..count).rev() {
            if value >> i & 1 != 0 {
                self.bytes[self.len / 8] |= 0x80 >> (self.len % 8);
            }
            self.len += 1;
        }
    }
}

/// Multiplies in GF(2⁸), modulo QR's polynomial x⁸ + x⁴ + x³ + x² + 1.
fn multiply(x: u8, y: u8) -> u8 {
    let mut z: u8 = 0;
    for i in (0..8).rev() {
        z = (z << 1) ^ if z & 0x80 != 0 { 0x1D } else { 0 };
        if y >> i & 1 != 0 {
            z ^= x;
        }
    }
    z
}

/// The error-correction codewords for `data`.
fn reed_solomon(data: &[u8; DATA]) -> [u8; ECC] {
    // The generator polynomial, (x - α⁰)(x - α¹)…(x - α¹⁵), without its
    // leading 1, highest power first.
    let mut divisor = [0u8; ECC];
    divisor[ECC - 1] = 1;
    let mut root = 1;
    for _ in 0..ECC {
        for j in 0..ECC {
            divisor[j] = multiply(divisor[j], root);
            if j + 1 < ECC {
                divisor[j] ^= divisor[j + 1];
            }
        }
        root = multiply(root, 0x02);
    }
    // The remainder of data × x¹⁶ divided by it.
    let mut remainder = [0u8; ECC];
    for &byte in data {
        let factor = byte ^ remainder[0];
        remainder.copy_within(1.., 0);
        remainder[ECC - 1] = 0;
        for (r, &d) in remainder.iter_mut().zip(&divisor) {
            *r ^= multiply(d, factor);
        }
    }
    remainder
}

/// The matrix being built: which modules are dark, and which belong to
/// the fixed patterns (so the data goes round them, and the mask skips
/// them).
struct Qr {
    dark: Matrix,
    function: Matrix,
}

impl Qr {
    fn set(&mut self, x: usize, y: usize, dark: bool) {
        let bit = 1 << x;
        if dark {
            self.dark[y] |= bit;
        } else {
            self.dark[y] &= !bit;
        }
        self.function[y] |= bit;
    }

    fn function_patterns(&mut self) {
        // Timing: alternating along row and column 6.
        for i in 0..SIZE {
            self.set(6, i, i % 2 == 0);
            self.set(i, 6, i % 2 == 0);
        }
        // Finders, with their light separators, in three corners.
        for (cx, cy) in [(3, 3), (SIZE - 4, 3), (3, SIZE - 4)] {
            for dy in -4i32..=4 {
                for dx in -4i32..=4 {
                    let (x, y) = (cx as i32 + dx, cy as i32 + dy);
                    let distance = dx.abs().max(dy.abs());
                    if (0..SIZE as i32).contains(&x) && (0..SIZE as i32).contains(&y) {
                        self.set(x as usize, y as usize, distance != 2 && distance != 4);
                    }
                }
            }
        }
        // Version 2's one alignment pattern.
        for dy in -2i32..=2 {
            for dx in -2i32..=2 {
                let distance = dx.abs().max(dy.abs());
                self.set((18 + dx) as usize, (18 + dy) as usize, distance != 1);
            }
        }
        // The format information's places, for now (see `format_bits`).
        self.format_bits();
    }

    /// The format information, around the finders: the error-correction
    /// level (M is 0) and mask, with its BCH code. Also the dark module
    /// beside the bottom-left finder.
    fn format_bits(&mut self) {
        let data = MASK;
        let mut remainder = data;
        for _ in 0..10 {
            remainder = (remainder << 1) ^ ((remainder >> 9) * 0x537);
        }
        let bits = (data << 10 | remainder) ^ 0x5412;
        let bit = |i: usize| bits >> i & 1 != 0;
        for i in 0..=5 {
            self.set(8, i, bit(i));
        }
        self.set(8, 7, bit(6));
        self.set(8, 8, bit(7));
        self.set(7, 8, bit(8));
        for i in 9..15 {
            self.set(14 - i, 8, bit(i));
        }
        for i in 0..8 {
            self.set(SIZE - 1 - i, 8, bit(i));
        }
        for i in 8..15 {
            self.set(8, SIZE - 15 + i, bit(i));
        }
        self.set(8, SIZE - 8, true);
    }

    /// Places `count` bits in the zigzag: two columns at a time, from the
    /// bottom right, up then down, round the fixed patterns.
    fn place(&mut self, bit: impl Fn(usize) -> bool, count: usize) {
        let mut i = 0;
        let mut right = SIZE - 1;
        loop {
            // Column 6 is the timing pattern's.
            if right == 6 {
                right = 5;
            }
            let upward = (right + 1) & 2 == 0;
            for vertical in 0..SIZE {
                let y = if upward {
                    SIZE - 1 - vertical
                } else {
                    vertical
                };
                for x in [right, right - 1] {
                    if self.function[y] >> x & 1 == 0 {
                        // Past the data, the remainder bits are light.
                        if i < count && bit(i) {
                            self.dark[y] |= 1 << x;
                        }
                        i += 1;
                    }
                }
            }
            if right < 2 {
                break;
            }
            right -= 2;
        }
    }

    /// Mask pattern 0, over everything but the fixed patterns.
    fn mask(&mut self) {
        for y in 0..SIZE {
            let checkerboard = if y % 2 == 0 { 0x0155_5555 } else { 0x00AA_AAAA };
            self.dark[y] ^= checkerboard & !self.function[y];
        }
    }
}
