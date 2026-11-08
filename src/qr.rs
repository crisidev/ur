//! A QR code for Home Assistant's key (see [`crate::bthome`]), so that
//! it can be scanned with a phone and pasted into Home Assistant, rather
//! than typed: 32 hex digits read off a watch are easy to get wrong.
//!
//! Only what that takes: alphanumeric mode (digits and capitals, so the
//! key's hex fits in 176 bits; not the mode's nine symbols), version 2 (25×25 modules), error
//! correction level M (15% of the code can be damaged), so a single block
//! of 28 data and 16 error-correction codewords, and one fixed mask
//! (pattern 0; the standard picks the mask that scores best for
//! scanners, but any mask is valid, and decoders read whichever the
//! format says). Checked module for module against a reference encoder
//! (`segno`) in `host-tests/qr`.
//!
//! Built for flash, as it runs once in a while: everything that doesn't
//! depend on the text is worked out by the compiler (the fixed patterns
//! and format bits, as a matrix; the Reed–Solomon generator), and what's
//! left is packing the text, its error correction, and laying it out.
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

/// Encodes `text` (digits and capitals, at most [`MAX_TEXT`]
/// characters). `None` if it can't.
pub fn encode(text: &[u8]) -> Option<Matrix> {
    if text.len() > MAX_TEXT {
        return None;
    }

    // The data codewords: mode (alphanumeric), length, the characters in
    // pairs (11 bits a pair, 6 for one left over), then, after up to four
    // zero bits and the rest of the byte (the buffer's zeros), padding.
    // The error correction follows them, in the same buffer.
    let mut codewords = [0u8; DATA + ECC];
    let mut len = 0;
    let mut push = |value: u32, count: usize| {
        for i in (0..count).rev() {
            codewords[len / 8] |= ((value >> i & 1) as u8) << (7 - len % 8);
            len += 1;
        }
    };
    push(0b0010, 4);
    push(text.len() as u32, 9);
    for (i, &c) in text.iter().enumerate().step_by(2) {
        let first = value(c)?;
        match text.get(i + 1) {
            Some(&second) => push(first * 45 + value(second)?, 11),
            None => push(first, 6),
        }
    }
    let mut byte = (len + 4).min(DATA * 8).div_ceil(8);
    let mut pad = 0xEC;
    while byte < DATA {
        codewords[byte] = pad;
        pad ^= 0xEC ^ 0x11;
        byte += 1;
    }

    // The error correction: the remainder of the data, times x¹⁶,
    // divided by the generator, worked out in the codewords' tail.
    for i in 0..DATA {
        let factor = codewords[i] ^ codewords[DATA];
        codewords.copy_within(DATA + 1.., DATA);
        codewords[DATA + ECC - 1] = 0;
        for (r, &g) in codewords[DATA..].iter_mut().zip(&GENERATOR) {
            *r ^= multiply(g, factor);
        }
    }

    // The bits, in the zigzag: two columns at a time from the bottom
    // right, up then down, round the fixed patterns, masked as they go
    // (pattern 0: flipped where `x + y` is even). Past the codewords come
    // seven remainder bits, light before the mask.
    let mut matrix = FIXED;
    let mut i = 0;
    let mut right = SIZE - 1;
    loop {
        let upward = (right + 1) & 2 == 0;
        for vertical in 0..SIZE {
            let y = if upward {
                SIZE - 1 - vertical
            } else {
                vertical
            };
            for x in (right - 1..=right).rev() {
                if fixed(x, y) {
                    continue;
                }
                let bit = codewords
                    .get(i / 8)
                    .is_some_and(|b| b >> (7 - i % 8) & 1 != 0);
                if bit != ((x + y) % 2 == 0) {
                    matrix[y] |= 1 << x;
                }
                i += 1;
            }
        }
        if right < 2 {
            break;
        }
        // Column 6 is the timing pattern's: the next pair is 4 and 5.
        right -= if right == 8 { 3 } else { 2 };
    }
    Some(matrix)
}

/// A character's alphanumeric value: digits and capitals only (the
/// mode's nine symbols, ` $%*+-./:`, aren't needed for a key).
fn value(c: u8) -> Option<u32> {
    Some(u32::from(match c {
        b'0'..=b'9' => c - b'0',
        b'A'..=b'Z' => c - b'A' + 10,
        _ => return None,
    }))
}

/// Multiplies in GF(2⁸), modulo QR's polynomial x⁸ + x⁴ + x³ + x² + 1, by
/// shifting and adding: no log tables.
const fn multiply(x: u8, y: u8) -> u8 {
    let mut z: u8 = 0;
    let mut i = 8;
    while i > 0 {
        i -= 1;
        z = (z << 1) ^ if z & 0x80 != 0 { 0x1D } else { 0 };
        if y >> i & 1 != 0 {
            z ^= x;
        }
    }
    z
}

/// The generator polynomial, (x - α⁰)(x - α¹)…(x - α¹⁵), without its
/// leading 1, highest power first.
const GENERATOR: [u8; ECC] = {
    let mut divisor = [0u8; ECC];
    divisor[ECC - 1] = 1;
    let mut root = 1;
    let mut n = 0;
    while n < ECC {
        let mut j = 0;
        while j < ECC {
            divisor[j] = multiply(divisor[j], root);
            if j + 1 < ECC {
                divisor[j] ^= divisor[j + 1];
            }
            j += 1;
        }
        root = multiply(root, 0x02);
        n += 1;
    }
    divisor
};

/// Whether a module belongs to the fixed patterns, which the data goes
/// round: the finders with their separators and format bits (each
/// corner's 9×9, or 8 wide or tall on the far sides), the timing row and
/// column, and version 2's one alignment pattern.
fn fixed(x: usize, y: usize) -> bool {
    let near = |v: usize| v < 9;
    let far = |v: usize| v >= SIZE - 8;
    (near(x) && (near(y) || far(y)))
        || (far(x) && near(y))
        || x == 6
        || y == 6
        || (x.abs_diff(18) <= 2 && y.abs_diff(18) <= 2)
}

/// The fixed patterns' dark modules: timing, finders, the alignment
/// pattern, the format bits (level M, mask 0), and the dark module beside
/// the bottom-left finder. Everything else starts light.
const FIXED: Matrix = {
    let mut m = [0u32; SIZE];
    // Timing: alternating along row and column 6, dark on even.
    let mut i = 0;
    while i < SIZE {
        if i % 2 == 0 {
            m[6] |= 1 << i;
            m[i] |= 1 << 6;
        }
        i += 1;
    }
    // Finders, in three corners: a dark ring, a light one, a dark 3×3
    // middle; the separators round them stay light. Clearing the timing
    // pattern's modules where they cross.
    let corners = [(3, 3), (SIZE - 4, 3), (3, SIZE - 4)];
    let mut c = 0;
    while c < 3 {
        let (cx, cy) = corners[c];
        let mut dy: usize = 0;
        while dy < 9 {
            let mut dx: usize = 0;
            while dx < 9 {
                let (x, y) = ((cx + dx) as isize - 4, (cy + dy) as isize - 4);
                if x >= 0 && y >= 0 && (x as usize) < SIZE && (y as usize) < SIZE {
                    let distance = max(dx.abs_diff(4), dy.abs_diff(4));
                    let bit = 1 << x;
                    if distance != 2 && distance != 4 {
                        m[y as usize] |= bit;
                    } else {
                        m[y as usize] &= !bit;
                    }
                }
                dx += 1;
            }
            dy += 1;
        }
        c += 1;
    }
    // The alignment pattern, centred on (18, 18).
    let mut dy: usize = 0;
    while dy < 5 {
        let mut dx: usize = 0;
        while dx < 5 {
            let distance = max(dx.abs_diff(2), dy.abs_diff(2));
            if distance != 1 {
                m[16 + dy] |= 1 << (16 + dx);
            }
            dx += 1;
        }
        dy += 1;
    }
    // The format information: level M (0) and mask 0, its BCH code, and
    // the standard's XOR, along the top-left finder and split between the
    // other two.
    let data = 0u32;
    let mut remainder = data;
    let mut n = 0;
    while n < 10 {
        remainder = (remainder << 1) ^ ((remainder >> 9) * 0x537);
        n += 1;
    }
    let bits = (data << 10 | remainder) ^ 0x5412;
    let mut i = 0;
    while i < 15 {
        if bits >> i & 1 != 0 {
            // Round the top-left finder: down column 8 (skipping the
            // timing row), then left along row 8 (skipping its column).
            let (x, y) = match i {
                0..=5 => (8, i),
                6 => (8, 7),
                7 => (8, 8),
                8 => (7, 8),
                _ => (14 - i, 8),
            };
            m[y] |= 1 << x;
            // And again: along row 8 under the top-right finder, then up
            // column 8 beside the bottom-left one.
            let (x, y) = if i < 8 {
                (SIZE - 1 - i, 8)
            } else {
                (8, SIZE - 15 + i)
            };
            m[y] |= 1 << x;
        }
        i += 1;
    }
    m[SIZE - 8] |= 1 << 8;
    m
};

const fn max(a: usize, b: usize) -> usize {
    if a > b { a } else { b }
}
