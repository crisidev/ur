//! Fuzzing, with Bolero: the tests above hold two codes to a reference
//! encoder's, module for module; these encode any text, and read every
//! code back with a decoder (`rqrr`, a port of quirc), damaged too, as a
//! phone's camera would.
//!
//! `cargo test` runs each harness over a few hundred random inputs (and
//! any saved in a corpus), so they're regression tests too. To fuzz one
//! properly (nightly, for the sanitizer):
//!
//! ```text
//! cargo +nightly bolero test fuzz::any_code_reads_back_even_damaged --engine libfuzzer -T 10min
//! ```

use super::qr;

/// Text from fuzzed bytes: mostly digits and capitals, which a code can
/// hold, and now and then anything.
fn text_of(bytes: &[u8]) -> Vec<u8> {
    const ALPHABET: &[u8; 36] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ";
    bytes
        .iter()
        .map(|&b| {
            if b < 0xF0 {
                ALPHABET[usize::from(b) % 36]
            } else {
                b
            }
        })
        .collect()
}

/// Whether a module is one of the fixed patterns' (the finders and their
/// separators and format bits, the timing row and column, the alignment
/// pattern), by ISO/IEC 18004's layout of version 2: the data and its
/// error correction are everything else.
fn fixed(x: usize, y: usize) -> bool {
    let (near, far) = (|v: usize| v <= 8, |v: usize| v >= qr::SIZE - 8);
    (near(x) && near(y))
        || (far(x) && near(y))
        || (near(x) && far(y))
        || x == 6
        || y == 6
        || ((16..=20).contains(&x) && (16..=20).contains(&y))
}

/// Any text is encoded if, and only if, it's digits and capitals, at most
/// [`qr::MAX_TEXT`] of them; and the code reads back as that text, as
/// version 2, level M, mask 0. With a few of its data modules flipped (as
/// many as touch at most 7 codewords, of the 8 its 16 error-correction
/// codewords can correct), it still does.
#[test]
fn any_code_reads_back_even_damaged() {
    bolero::check!()
        .with_type::<(Vec<u8>, Vec<(u8, u8)>)>()
        .for_each(|(bytes, damage)| {
            let text = text_of(bytes);
            let encodable = text.len() <= qr::MAX_TEXT
                && text
                    .iter()
                    .all(|c| c.is_ascii_digit() || c.is_ascii_uppercase());
            let Some(matrix) = qr::encode(&text) else {
                assert!(
                    !encodable,
                    "{:?} wasn't encoded",
                    String::from_utf8_lossy(&text)
                );
                return;
            };
            assert!(
                encodable,
                "{:?} was encoded",
                String::from_utf8_lossy(&text)
            );
            assert!(
                matrix.iter().all(|row| row >> qr::SIZE == 0),
                "modules past the edge"
            );

            let dark = |matrix: &qr::Matrix, x: usize, y: usize| matrix[y] >> x & 1 != 0;
            let read = |matrix: &qr::Matrix| {
                let grid = rqrr::SimpleGrid::from_func(qr::SIZE, |x, y| dark(matrix, x, y));
                rqrr::Grid::new(grid).decode()
            };
            let (meta, content) = read(&matrix).expect("a code that reads");
            assert_eq!(content.as_bytes(), &text[..]);
            // Level M is 0b00 in the format bits.
            assert_eq!((meta.version.0, meta.ecc_level, meta.mask), (2, 0, 0));

            // Some damage, in data modules: up to seven, so up to seven
            // codewords.
            let mut damaged = matrix;
            for &(x, y) in damage.iter().take(7) {
                let (x, y) = (usize::from(x) % qr::SIZE, usize::from(y) % qr::SIZE);
                if !fixed(x, y) {
                    damaged[y] ^= 1 << x;
                }
            }
            let (_, content) = read(&damaged).expect("damage the error correction mends");
            assert_eq!(content.as_bytes(), &text[..]);
        });
}
