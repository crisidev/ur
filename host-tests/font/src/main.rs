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
    use super::font::{
        Face, Glyph, Line, MediumGlyph, glyph, medium_glyph, small_text_width, wrap,
    };

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

    // ==========================================================================
    // The medium face
    // ==========================================================================

    fn medium(c: char) -> (u32, [u8; 9]) {
        let MediumGlyph { width, rows } = medium_glyph(c);
        (width, rows)
    }

    #[test]
    fn medium_has_printable_ascii_all_different() {
        let unknown = medium('\u{4e2d}');
        let distinct: Vec<char> = printable_ascii()
            .filter(|c| !c.is_ascii_lowercase())
            .collect();
        for &c in &distinct {
            assert!(c == '?' || medium(c) != unknown, "{c:?} is drawn as `?`");
        }
        for (i, &a) in distinct.iter().enumerate() {
            for &b in &distinct[i + 1..] {
                assert_ne!(medium(a), medium(b), "{a:?} and {b:?} look the same");
            }
        }
        for c in 'a'..='z' {
            assert_eq!(medium(c), medium(c.to_ascii_uppercase()), "{c:?}");
        }
    }

    /// Every Latin-1 letter has a glyph of its own, its lowercase the same.
    #[test]
    fn medium_has_every_latin1_letter() {
        let unknown = medium('\u{4e2d}');
        let letters: Vec<char> = ('\u{c0}'..='\u{ff}')
            .filter(|c| c.is_alphabetic() && !matches!(c, '×' | '÷'))
            .collect();
        for &c in &letters {
            assert_ne!(medium(c), unknown, "{c:?}");
            assert_ne!(medium(c), medium('A'), "{c:?}");
        }
        for (i, &a) in letters.iter().enumerate() {
            for &b in &letters[i + 1..] {
                let same_letter = a.to_uppercase().eq(b.to_uppercase());
                if !same_letter && a != 'ß' && b != 'ß' {
                    assert_ne!(medium(a), medium(b), "{a:?} and {b:?} look the same");
                }
            }
        }
        assert_eq!(medium('ø'), medium('Ø'));
        assert_eq!(medium('ÿ').1[2..], medium('Y').1[2..]);
    }

    /// Glyphs stay within their width, letters are bold (no pixel alone
    /// in its row, bar the accents and a few marks), and ASCII leaves the
    /// accent rows empty.
    #[test]
    fn medium_glyphs_stay_in_their_cell() {
        for c in printable_ascii().chain(('\u{c0}'..='\u{ff}').filter(|c| c.is_alphabetic())) {
            let (width, rows) = medium(c);
            assert!((2..=8).contains(&width), "{c:?} is {width} wide");
            for row in rows {
                assert_eq!(u32::from(row) >> width, 0, "{c:?} spills right");
            }
            if c.is_ascii() {
                assert_eq!(rows[..2], [0, 0], "{c:?} rises into the accents");
            }
            if c.is_ascii_alphanumeric() {
                // A lone pixel between two clear ones is a hairline
                // stem: the face is bold.
                for (y, &row) in rows[2..].iter().enumerate() {
                    let row = u32::from(row) << 1;
                    for x in 1..=width {
                        let lone = (row >> x) & 0b111 == 0b010;
                        assert!(
                            !lone || matches!(c.to_ascii_uppercase(), 'M' | 'W' | 'Q'),
                            "{c:?} row {y}"
                        );
                    }
                }
            }
        }
    }

    /// Round letters are a column wider than square ones, so O isn't 0.
    #[test]
    fn medium_widths() {
        assert_eq!(medium('H').0, 5);
        assert_eq!(medium('0').0, 5);
        assert_eq!(medium('O').0, 6);
        assert_ne!(medium('O'), medium('0'));
        assert_eq!(
            Face::Medium.width("NILS HOLM"),
            6 + 4 + 5 + 5 + 2 + 5 + 6 + 5 + 7 + 8
        );
        // Typographic punctuation, as the small font has it.
        assert_eq!(medium('\u{2019}'), medium('\''));
        assert_eq!(medium('\u{2013}'), medium('-'));
    }

    // ==========================================================================
    // Wrapping
    // ==========================================================================

    /// `text` wrapped in the medium face in 72 px, at most `most` lines,
    /// as the lines' text.
    fn wrapped(text: &str, most: usize) -> Vec<String> {
        let mut lines = vec![Line::default(); most];
        let count = wrap(text, Face::Medium, 72, &mut lines);
        lines[..count]
            .iter()
            .map(|l| format!("{}{}", &text[l.start..l.end], l.tail.as_str()))
            .collect()
    }

    #[test]
    fn one_line_if_it_fits() {
        assert_eq!(wrapped("ADA LOVELACE", 3), ["ADA LOVELACE"]);
        assert_eq!(wrapped("", 3), Vec::<String>::new());
    }

    /// Two lines as even as they can be: not a long line and a stub.
    #[test]
    fn balanced() {
        assert_eq!(wrapped("MARGARET HAMILTON", 3), ["MARGARET", "HAMILTON"]);
        assert_eq!(
            wrapped("YOUR PARCEL IS READY FOR PICKUP", 4),
            ["YOUR PARCEL", "IS READY", "FOR PICKUP"]
        );
        assert_eq!(wrapped("3 NEW MESSAGES", 3), ["3 NEW", "MESSAGES"]);
        // Never by breaking a word that fits.
        for line in wrapped("NORSK BIBLIOTEKFORENING MEDLEMSSERVICE", 5) {
            assert!(Face::Medium.width(&line) <= 72, "{line}");
        }
    }

    /// A dash ends a line rather than starting one, and a hyphenated word
    /// breaks after its hyphen.
    #[test]
    fn dashes() {
        assert_eq!(
            wrapped("TANNLEGE - DR. ØSTBY", 3),
            ["TANNLEGE -", "DR. ØSTBY"]
        );
        assert_eq!(wrapped("#FIRMWARE-REVIEW", 3), ["#FIRMWARE-", "REVIEW"]);
    }

    /// A link breaks where its parts meet, never with a hyphen added.
    #[test]
    fn links_keep_their_letters() {
        let lines = wrapped("HTTPS://CI.EXAMPLE.ORG/UR/RUNS/4821", 5);
        assert_eq!(lines.concat(), "HTTPS://CI.EXAMPLE.ORG/UR/RUNS/4821");
        assert_eq!(lines[0], "HTTPS://CI");
        assert!(lines[1].starts_with(".EXAMPLE"));
        // Without anywhere to break, still no hyphen.
        let lines = wrapped("WWW.ABCDEFGHIJKLMNOPQRSTU", 3);
        assert_eq!(lines.concat(), "WWW.ABCDEFGHIJKLMNOPQRSTU");
    }

    /// A word too long for a line, with nothing else to break at, is
    /// hyphenated, with at least three letters either side.
    #[test]
    fn long_words_hyphenate() {
        let lines = wrapped("VELFERDSTEKNOLOGIAVDELINGEN", 4);
        assert!(lines.len() >= 2);
        let mut joined = String::new();
        for line in &lines[..lines.len() - 1] {
            assert!(line.ends_with('-'), "{line}");
            assert!(line.chars().count() >= 4, "{line}");
            joined += line.trim_end_matches('-');
        }
        let last = lines.last().expect("two lines at least");
        assert!(last.chars().count() >= 3, "{last}");
        joined += last;
        assert_eq!(joined, "VELFERDSTEKNOLOGIAVDELINGEN");
    }

    /// What doesn't fit ends in an ellipsis, not cut off silently, and not
    /// after a space or punctuation.
    #[test]
    fn ellipsis_when_out_of_lines() {
        assert_eq!(
            wrapped("NORSK BIBLIOTEKFORENING", 1),
            ["NORSK BIBLI\u{2026}"]
        );
        assert_eq!(
            wrapped("FAMILIEN (12 MESSAGES): ÅSE", 2)[1],
            "MESSAGES)\u{2026}"
        );
        for (text, most) in [
            ("YOUR PARCEL IS READY FOR PICKUP", 2),
            ("ABC DEF GHI JKL MNO PQR", 1),
        ] {
            let lines = wrapped(text, most);
            assert_eq!(lines.len(), most);
            let last = lines.last().expect("a line");
            assert!(last.ends_with('\u{2026}'), "{last}");
            assert!(!last.contains(" \u{2026}"), "{last}");
            for line in &lines {
                assert!(Face::Medium.width(line) <= 72, "{line}");
            }
        }
    }

    /// Every line fits, and the lines together are the text, for anything.
    #[test]
    fn lines_fit_and_keep_everything() {
        for text in [
            "SAMEIET BJØRNSTJERNE BJØRNSONS GATE 12",
            "BURSDAGSGJENGEN 2026",
            "!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!",
            "MMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMM",
            "A - B - C - D - E - F - G - H - I - J - K",
            "- - - - - - - - - - - - - - - - - - - -",
            "ÆØÅÆØÅÆØÅÆØÅÆØÅÆØÅÆØÅ",
            "X",
            "A.B/C@D:E_F-G.H/I@J:K_L-M.N/O@P:Q_R-S",
        ] {
            let lines = wrapped(text, 40);
            let mut joined = String::new();
            for line in &lines {
                assert!(Face::Medium.width(line) <= 72, "{text}: {line}");
                if !joined.is_empty()
                    && !line.starts_with(['.', '/'])
                    && !joined.ends_with(['-', '/', '@', ':', '_'])
                {
                    joined.push(' ');
                }
                joined += line
                    .strip_suffix('-')
                    .filter(|_| line.len() > 1 && !text.contains(line.as_str()))
                    .unwrap_or(line);
            }
            assert_eq!(joined.replace(' ', ""), text.replace(' ', ""), "{lines:?}");
        }
    }
}
