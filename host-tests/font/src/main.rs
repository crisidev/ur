//! Host tests for the firmware's `font` module: which glyph every character
//! gets. `cargo test` runs them.

#[path = "../../../src/font.rs"]
#[allow(dead_code)]
mod font;

fn main() {
    println!("run `cargo test`");
}

#[cfg(test)]
mod tests {
    use super::font::{Glyph, glyph, small_text_width};

    /// A glyph's width and pixels, to compare glyphs by.
    fn look(c: char) -> (u32, [u8; 7]) {
        let Glyph { width, rows } = glyph(c);
        (width, rows)
    }

    /// What a character the font lacks looks like.
    fn unknown() -> (u32, [u8; 7]) {
        look('\u{4e2d}')
    }

    fn printable_ascii() -> impl Iterator<Item = char> {
        ' '..='~'
    }

    /// Every printable ASCII character is drawn as itself, not as the `?`
    /// that stands in for what the font lacks (as `&` once was).
    #[test]
    fn printable_ascii_is_all_there() {
        let missing: String = printable_ascii()
            .filter(|&c| c != '?' && look(c) == unknown())
            .collect();
        assert_eq!(missing, "", "drawn as `?`");
    }

    /// No two characters look the same, bar lowercase, which is drawn as
    /// uppercase.
    #[test]
    fn printable_ascii_is_all_different() {
        let distinct: Vec<char> = printable_ascii()
            .filter(|c| !c.is_ascii_lowercase())
            .collect();
        for (i, &a) in distinct.iter().enumerate() {
            for &b in &distinct[i + 1..] {
                assert_ne!(look(a), look(b), "{a:?} and {b:?} look the same");
            }
        }
        for c in 'a'..='z' {
            assert_eq!(look(c), look(c.to_ascii_uppercase()), "{c:?}");
        }
    }

    /// Glyphs stay within their width, and ASCII leaves the accent rows
    /// above the capitals empty, so lines can sit as close as they do.
    #[test]
    fn glyphs_stay_in_their_cell() {
        for c in printable_ascii().chain(['Å', 'Ø', 'Æ', 'Ü', 'ß', '°', '\u{2026}']) {
            let Glyph { width, rows } = glyph(c);
            assert!((3..=5).contains(&width), "{c:?} is {width} wide");
            for row in rows {
                assert_eq!(u32::from(row) >> width, 0, "{c:?} spills right");
            }
            if c.is_ascii() {
                assert_eq!(rows[..2], [0, 0], "{c:?} rises into the accents");
            }
        }
    }

    /// Phones' typographic punctuation is drawn as the ASCII it stands in
    /// for.
    #[test]
    fn typographic_punctuation_looks_like_ascii() {
        for (fancy, plain) in [
            ('\u{2018}', '\''),
            ('\u{2019}', '\''),
            ('\u{201b}', '\''),
            ('\u{2032}', '\''),
            ('\u{2b9}', '\''),
            ('\u{201c}', '"'),
            ('\u{201d}', '"'),
            ('\u{201e}', '"'),
            ('\u{201f}', '"'),
            ('\u{2033}', '"'),
            ('\u{201a}', ','),
            ('\u{2010}', '-'),
            ('\u{2011}', '-'),
            ('\u{2012}', '-'),
            ('\u{2013}', '-'),
            ('\u{2014}', '-'),
            ('\u{2015}', '-'),
            ('\u{2212}', '-'),
        ] {
            assert_eq!(look(fancy), look(plain), "{fancy:?}");
        }
        // The ellipsis: three dots on the line, a column apart.
        assert_eq!(look('\u{2026}'), (5, [0, 0, 0, 0, 0, 0, 0b10101]));
    }

    /// The Nordic letters and the degree sign keep their own glyphs.
    #[test]
    fn latin1_letters_have_their_own() {
        for c in ['Å', 'å', 'Ø', 'ø', 'Æ', 'æ', 'Ä', 'Ö', 'Ü', 'é', 'ß', '°'] {
            assert_ne!(look(c), unknown(), "{c:?}");
        }
        assert_eq!(look('å'), look('Å'));
    }

    /// Text width counts each glyph's own width, wide ones included, and a
    /// column between glyphs.
    #[test]
    fn text_width() {
        assert_eq!(small_text_width(""), 0);
        assert_eq!(small_text_width("A"), 3);
        assert_eq!(small_text_width("AB"), 7);
        assert_eq!(small_text_width("A&B"), 3 + 1 + 4 + 1 + 3);
        assert_eq!(small_text_width("@#"), 4 + 1 + 4);
        assert_eq!(small_text_width("I\u{2026}"), 3 + 1 + 5);
    }
}
