//! Fuzzing, with Bolero: what the fonts draw comes from the phone
//! (notifications, contacts, street names), in any script. The tests above
//! try names and words of a few kinds; these try any text, in any room,
//! with any number of lines, and any character, against what the module's
//! docs promise: glyphs within their cells, lines at character boundaries
//! (`slice` panics otherwise), each within its room, and together the
//! whole text, or an ellipsis to say it isn't.
//!
//! `cargo test` runs each harness over a few hundred random inputs (and
//! any saved in a corpus), so they're regression tests too. To fuzz one
//! properly (nightly, for the sanitizer):
//!
//! ```text
//! cargo +nightly bolero test fuzz::any_text_lays_out_in_its_lines --engine libfuzzer -T 10min
//! ```

use super::font::{
    Face, Glyph, Line, MediumGlyph, Tail, fill, glyph, medium_glyph, slice, small_glyph,
    small_text_width, wrap,
};

/// Text from fuzzed bytes, mostly of what notifications hold (ASCII,
/// Latin-1, the typographic punctuation phones use, spaces and dashes),
/// each byte picking a character: the fuzzer would rarely find a space or
/// a `-` by itself among a million others.
fn text_of(bytes: &[u8]) -> String {
    const OTHERS: [char; 8] = [
        ' ',
        '-',
        '.',
        '/',
        '\u{2026}',
        '\u{2019}',
        '\u{4e2d}',
        '\u{1F600}',
    ];
    bytes
        .iter()
        .map(|&b| match b {
            0..=7 => OTHERS[usize::from(b)],
            b' '..=b'~' => char::from(b),
            0x80..=0x9F => ' ',
            _ => char::from(b),
        })
        .collect()
}

/// Any character's glyph, in either font, stays within its width, and
/// characters that aren't printable ASCII draw nothing in the small
/// font's 3×5 table but its `?`. Lowercase looks like uppercase.
#[test]
fn any_character_stays_in_its_cell() {
    bolero::check!().with_type::<char>().for_each(|&c| {
        let question = small_glyph('?');
        let rows = small_glyph(c);
        assert!(rows.iter().all(|&row| row < 1 << 3), "{c:?}");
        if !(' '..='~').contains(&c) {
            assert_eq!(rows, question, "{c:?}");
        }
        if c.is_ascii_lowercase() {
            assert_eq!(rows, small_glyph(c.to_ascii_uppercase()));
        }

        let Glyph { width, rows } = glyph(c);
        assert!((3..=5).contains(&width), "{c:?} is {width} wide");
        assert!(
            rows.iter().all(|&row| u32::from(row) >> width == 0),
            "{c:?}"
        );
        let MediumGlyph { width, rows } = medium_glyph(c);
        assert!((2..=8).contains(&width), "{c:?} is {width} wide");
        assert!(
            rows.iter().all(|&row| u32::from(row) >> width == 0),
            "{c:?}"
        );
    });
}

/// Any text's width is its glyphs', a column between each: the widths of
/// two pieces of it, and the column between them.
#[test]
fn any_texts_width_is_its_glyphs() {
    bolero::check!()
        .with_type::<(String, String)>()
        .for_each(|(a, b)| {
            for face in [Face::Small, Face::Medium] {
                let joined = format!("{a}{b}");
                let (wa, wb) = (face.width(a), face.width(b));
                let expected = if a.is_empty() {
                    wb
                } else if b.is_empty() {
                    wa
                } else {
                    wa + 1 + wb
                };
                assert_eq!(face.width(&joined), expected, "{joined:?}");
            }
            assert_eq!(small_text_width(a), Face::Small.width(a));
            let glyphs: i32 = a.chars().map(|c| glyph(c).width as i32 + 1).sum();
            assert_eq!(small_text_width(a), (glyphs - 1).max(0));
        });
}

/// Any text, in any room, into any number of lines: `fill` and `wrap`
/// keep to character boundaries, so `slice` (and so [`Line::of`]) never
/// panics on what they give, and nothing overflows. For text as
/// `phone::Text` keeps it (trimmed, its spaces single) in room for a few
/// glyphs, more: each line is within the room, its tail included; the
/// lines are in order, only spaces between them, and are the whole text,
/// unless it took more lines than there were, when `wrap` ends the last
/// in an ellipsis; and `wrap` balances the lines without taking more of
/// them than `fill` does.
#[test]
fn any_text_lays_out_in_its_lines() {
    bolero::check!()
        .with_type::<(Vec<u8>, bool, i16, u8)>()
        .for_each(|(bytes, medium, room, most)| {
            let face = if *medium { Face::Medium } else { Face::Small };
            let most = usize::from(*most % 8);
            let room = i32::from(*room) % 200;
            let raw = text_of(bytes);
            // As is, then as `phone::Text` keeps it.
            let text = raw
                .split(' ')
                .filter(|w| !w.is_empty())
                .collect::<Vec<_>>()
                .join(" ");
            for text in [raw.as_str(), text.as_str()] {
                let mut filled = vec![Line::default(); most];
                let count = fill(text, face, room, &mut filled);
                let mut wrapped = vec![Line::default(); most];
                let used = wrap(text, face, room, &mut wrapped);
                assert!(used <= most);
                for line in filled[..count.min(most)].iter().chain(&wrapped[..used]) {
                    assert!(line.start <= line.end && line.end <= text.len(), "{line:?}");
                    // At character boundaries, or these panic.
                    let of = line.of(text);
                    assert_eq!(of, slice(text, line.start, line.end));
                    _ = line.width(text, face);
                }
            }

            // The rest is for text as it's kept, with room for a few
            // glyphs and a tail (the widest glyph, …, is 8 px).
            if room < 24 {
                return;
            }
            let mut filled = vec![Line::default(); most];
            let count = fill(&text, face, room, &mut filled);
            let mut lines = vec![Line::default(); most];
            let used = wrap(&text, face, room, &mut lines);
            assert_eq!(used, count.min(most), "{text:?}");
            let lines = &lines[..used];
            for line in lines {
                assert!(
                    line.width(&text, face) <= room,
                    "{text:?}: {line:?} is wider than {room}"
                );
            }
            if count > most {
                // An ellipsis to say there's more, and nothing after it.
                if let Some(last) = lines.last() {
                    assert_eq!(last.tail, Tail::Ellipsis, "{text:?}");
                    assert!(!last.of(&text).ends_with([' ', ',', '.', ':', ';', '-']));
                }
                return;
            }
            // Every character but the spaces between lines, in order.
            let mut at = 0;
            for line in lines {
                assert!(
                    text[at..line.start].bytes().all(|b| b == b' '),
                    "{text:?}: {lines:?}"
                );
                assert_ne!(line.tail, Tail::Ellipsis);
                at = line.end;
            }
            assert_eq!(at, text.len(), "{text:?}: {lines:?}");
            // A line broken with a hyphen is broken in a word.
            for pair in lines.windows(2) {
                if pair[0].tail == Tail::Hyphen {
                    assert_eq!(pair[0].end, pair[1].start, "{text:?}: {lines:?}");
                }
            }
        });
}
