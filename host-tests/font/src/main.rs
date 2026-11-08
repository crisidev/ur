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
    use super::font::{Glyph, Large, glyph, large, small_text_width};

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

    /// A notification's sender in large type, as the full-screen
    /// notification lays it out: 2×, in the banner's 72 px.
    fn sender(text: &str) -> Option<Large<'_>> {
        large(text, 72, 2)
    }

    /// Nine 3-wide letters (or spaces) fit on a line at 2×, ten don't.
    #[test]
    fn large_on_one_line_if_it_fits() {
        assert_eq!(sender("Ida"), Some(Large::One("Ida")));
        assert_eq!(sender("Bo Ek"), Some(Large::One("Bo Ek")));
        assert_eq!(sender("Nils Holm"), Some(Large::One("Nils Holm")));
        assert_eq!(sender("Christina"), Some(Large::One("Christina")));
        assert_eq!(sender("  Ida  "), Some(Large::One("Ida")));
        assert_ne!(sender("Nils Holme"), Some(Large::One("Nils Holme")));
    }

    /// Two-word names of about the same length go on two lines, down to
    /// 5 and 7 letters (70%).
    #[test]
    fn large_on_two_lines_if_balanced() {
        assert_eq!(sender("Maria Olsen"), Some(Large::Two("Maria", "Olsen")));
        assert_eq!(sender("Jane Smith"), Some(Large::Two("Jane", "Smith")));
        assert_eq!(sender("Nils Holme"), Some(Large::Two("Nils", "Holme")));
        assert_eq!(
            sender("Sofie Lindahl"),
            Some(Large::Two("Sofie", "Lindahl"))
        );
        assert_eq!(
            sender("Henrik Nordahl"),
            Some(Large::Two("Henrik", "Nordahl"))
        );
        // Extra spaces between the words don't end up on either line.
        assert_eq!(sender("Maria  Olsen"), Some(Large::Two("Maria", "Olsen")));
    }

    /// A short word over a long one is small, even if both fit at 2×.
    #[test]
    fn small_if_unbalanced() {
        assert_eq!(sender("Ada Lovelace"), None);
        assert_eq!(sender("Nora Lindqvist"), None);
        // 4 and 6 letters: 65%.
        assert_eq!(sender("Nils Hansen"), None);
        // 6 and 9 letters: 66%.
        assert_eq!(sender("Henrik Johansson"), None);
    }

    /// A word too long for a line is small, never hyphenated.
    #[test]
    fn small_if_a_word_is_too_long() {
        assert_eq!(sender("Bartholomew"), None);
        assert_eq!(sender("Dr. Bartholomew"), None);
        assert_eq!(sender("Åse Kristiansen"), None);
    }

    /// With more words, the split that balances the lines best, which
    /// needn't be where a greedy wrap would break.
    #[test]
    fn several_words_split_where_most_balanced() {
        assert_eq!(
            sender("Hi how are you"),
            Some(Large::Two("Hi how", "are you"))
        );
        // Greedy: "ANA BO EK" over "LUND".
        assert_eq!(
            sender("Ana Bo Ek Lund"),
            Some(Large::Two("Ana Bo", "Ek Lund"))
        );
        // Three lines' worth.
        assert_eq!(sender("Hi how are you doing"), None);
        assert_eq!(sender("Night Signals: Ida Moreau"), None);
        // Every split leaves one line too long, or the two unequal.
        assert_eq!(sender("Bo Ek Lovelace"), None);
    }

    /// Letters beyond ASCII measure as their glyphs do: Æ is wider than
    /// the rest, Å and Ø as wide.
    #[test]
    fn large_with_nordic_letters() {
        assert_eq!(sender("Åse Ørn"), Some(Large::One("Åse Ørn")));
        assert_eq!(
            sender("Bjørn Ødegård"),
            Some(Large::Two("Bjørn", "Ødegård"))
        );
        assert_eq!(sender("Åse Øvrebø"), None);
        // Nine letters fit on a line, but not with Æ's two extra columns.
        assert_eq!(sender("Ærle Holm"), Some(Large::Two("Ærle", "Holm")));
        assert_eq!(sender("Ærle Holme"), Some(Large::Two("Ærle", "Holme")));
        assert_eq!(sender("Æsa Holmsen"), None);
    }

    /// Nothing to show fits on one line.
    #[test]
    fn large_when_empty() {
        assert_eq!(sender(""), Some(Large::One("")));
        assert_eq!(sender("   "), Some(Large::One("")));
    }
}
