//! Hand-drawn bitmap fonts for the 80×80 canvas.
//!
//! Glyph rows are bitmasks with the leftmost pixel in the most significant of
//! the glyph's `width` low bits, e.g. for a 3-wide glyph `0b100` is the left
//! column. They're drawn with [`crate::canvas::Canvas::sprite`].

// ==============================================================================
// 3×5: all general-purpose text
// ==============================================================================

/// A glyph of the small font: `width` columns, and seven rows. Rows 2–6 are
/// the 3×5 capitals; rows 0–1, above them, hold only accents, so text is
/// positioned by its capitals (`y` is the top of row 2) and accents rise into
/// the space above the line.
pub struct Glyph {
    pub width: u32,
    pub rows: [u8; 7],
}

/// Rows above the capitals.
pub const ACCENT_ROWS: i32 = 2;

/// Width in pixels of `text` at 1×, excluding the trailing spacing column.
pub fn small_text_width(text: &str) -> i32 {
    let total: u32 = text.chars().map(|c| glyph(c).width + 1).sum();
    (total as i32 - 1).max(0)
}

// ==============================================================================
// What both fonts draw: ASCII, and Latin-1's letters as a letter and an accent
// ==============================================================================

/// What a character is drawn as, in either font: ASCII as itself
/// (lowercase as uppercase: neither font has room for distinct lowercase
/// forms); Latin-1's accented letters as their base letter and an accent
/// (one of `` ` ' ^ ~ : o ``: grave, acute, circumflex, tilde,
/// diaeresis, ring); the typographic punctuation phones like to use
/// (curly quotes, dashes, the minus sign) as its ASCII look-alike; and
/// anything else as itself, for each font to draw as one of its own few
/// symbols, or as `?`.
///
/// Notification text arrives as Latin-1 (Gadgetbridge's encoding), so
/// both fonts cover all of printable ASCII and every Latin-1 letter.
/// Anything beyond Latin-1 arrives escaped (`’`, see [`crate::gb`]).
fn decompose(c: char) -> (char, u8) {
    // C0–DF, Latin-1's capitals (and ×, ß): the base letter, and the
    // accent, of each accented one; a space for the others.
    const BASES: &[u8; 32] = b"AAAAAA  EEEEIIII NOOOOO  UUUUY  ";
    const MARKS: &[u8; 32] = b"`'^~:o  `'^:`'^: ~`'^~:  `'^:'  ";
    let c = match c {
        // The same table as Gadgetbridge's own (`renderUnicodeAsImage`),
        // which it only applies when it renders text as images.
        '\u{2018}' | '\u{2019}' | '\u{201b}' | '\u{2032}' | '\u{2b9}' => '\'',
        '\u{201c}' | '\u{201d}' | '\u{201e}' | '\u{201f}' | '\u{2033}' => '"',
        '\u{201a}' => ',',
        '\u{2010}'..='\u{2015}' | '\u{2212}' => '-',
        '\u{a0}' => ' ',
        'ÿ' => return ('Y', b':'),
        // Lowercase, bar ÷ (and ÿ, above, whose capital isn't Latin-1).
        'à'..='þ' if c != '÷' => char::from(c as u8 - 0x20),
        c => c.to_ascii_uppercase(),
    };
    match c {
        'À'..='ß' if BASES[c as usize - 0xC0] != b' ' => {
            let i = c as usize - 0xC0;
            (char::from(BASES[i]), MARKS[i])
        }
        c => (c, b' '),
    }
}

/// The small font's accent marks, two rows each, by [`decompose`]'s
/// symbol for them.
fn small_accent(mark: u8) -> [u8; 2] {
    match mark {
        b'\'' => [0b001, 0b010],
        b'`' => [0b100, 0b010],
        b'^' => [0b010, 0b101],
        b'~' => [0b011, 0b110],
        // Raised a row above the letter: directly on top, the dots merged
        // into the stems of U (whose top row is also `101`).
        b':' => [0b101, 0b000],
        // Over A's pointed top, this closes into a little ring: a
        // diamond, as a square one (`111 101`) merged with the A into an
        // 8 at 2×.
        _ => [0b010, 0b101],
    }
}

/// The glyph for any character (see [`decompose`]).
pub fn glyph(c: char) -> Glyph {
    let plain = |rows: [u8; 5]| Glyph {
        width: 3,
        rows: [0, 0, rows[0], rows[1], rows[2], rows[3], rows[4]],
    };
    // A few symbols need a fourth column to read as themselves: at 3 wide,
    // `#` is an H with an extra bar, `&` an 8 with a foot, and `@` an e.
    let wide = |rows: [u8; 5]| Glyph {
        width: 4,
        rows: [0, 0, rows[0], rows[1], rows[2], rows[3], rows[4]],
    };
    let (c, mark) = decompose(c);
    if mark != b' ' {
        let r = small_glyph(c);
        let [a, b] = small_accent(mark);
        return Glyph {
            width: 3,
            rows: [a, b, r[0], r[1], r[2], r[3], r[4]],
        };
    }
    match c {
        // Leaning, as a handwritten one does: upright, it's a grid of dots.
        '#' => wide([0b0101, 0b1111, 0b1010, 0b1111, 0b1010]),
        // Two loops, the lower one open to the right for the tail.
        '&' => wide([0b0100, 0b1010, 0b0100, 0b1010, 0b0101]),
        // An `a` (the 2×2 block) in a ring open at the bottom right.
        '@' => wide([0b0110, 0b1001, 0b1011, 0b1000, 0b0110]),
        _ if c.is_ascii() => plain(small_glyph(c)),
        'Æ' => Glyph {
            width: 5,
            rows: [0, 0, 0b01111, 0b10100, 0b11110, 0b10100, 0b10111],
        },
        'Ç' => plain([0b011, 0b100, 0b100, 0b011, 0b010]),
        'Ð' => plain([0b110, 0b101, 0b111, 0b101, 0b110]),
        // An O with a stroke from corner to corner, a column wider than
        // O: at 3 wide, the stroke filled the counter's middle row, and
        // it read as an 8 (`BJ8RN`), especially at 2×.
        'Ø' => wide([0b0111, 0b1011, 0b1011, 0b1101, 0b1110]),
        'Þ' => plain([0b100, 0b110, 0b101, 0b110, 0b100]),
        'ß' => plain([0b010, 0b101, 0b110, 0b101, 0b110]),
        // A little ring at the top, for the clock face's temperature (and
        // any notification that mentions one).
        '°' => plain([0b010, 0b101, 0b010, 0b000, 0b000]),
        // A small x in the middle three rows, as in print, so it isn't
        // taken for an X: the clock face's step scale ("×1K"), and any
        // dimensions or "2×" in a notification.
        '×' => plain([0b000, 0b101, 0b010, 0b101, 0b000]),
        // Three dots, as tight as the font's own `...` but one glyph.
        // (Gadgetbridge already sends `...` for it; other phones' apps may
        // not.)
        '\u{2026}' => Glyph {
            width: 5,
            rows: [0, 0, 0, 0, 0, 0, 0b10101],
        },
        // TODO: other common symbols beyond ASCII (•, ·, «, », €, …)
        // still show as `?`; add them as notifications turn them up.
        _ => plain(small_glyph('?')),
    }
}

/// Looks up a 3-wide glyph for an ASCII character ([`glyph`] has the wider
/// ones, and the rest of Latin-1). Lowercase is drawn as uppercase (3×5 has
/// no room for distinct lowercase forms), and anything else unknown becomes
/// `?`.
pub fn small_glyph(c: char) -> [u8; 5] {
    let i = (c as u32).wrapping_sub(' ' as u32) as usize;
    let packed = match SMALL_ASCII.get(i) {
        Some(&packed) => packed,
        None => SMALL_ASCII[usize::from(b'?' - b' ')],
    };
    core::array::from_fn(|row| (packed >> (3 * (4 - row))) as u8 & 0b111)
}

/// [`small_art`] for ASCII from the space to DEL, each glyph's five 3-bit
/// rows packed into a `u16`, top row highest: built at compile time, so
/// the art stays readable as a `match` and costs only its bits (192
/// bytes, where the `match` compiled to ~0.9 KB of code).
static SMALL_ASCII: [u16; 96] = {
    let mut table = [0; 96];
    let mut i = 0;
    while i < table.len() {
        let rows = small_art((b' ' + i as u8) as char);
        let mut row = 0;
        while row < rows.len() {
            table[i] = (table[i] << 3) | rows[row] as u16;
            row += 1;
        }
        i += 1;
    }
    table
};

/// The small font's ASCII; see [`small_glyph`].
const fn small_art(c: char) -> [u8; 5] {
    match c.to_ascii_uppercase() {
        ' ' => [0b000, 0b000, 0b000, 0b000, 0b000],
        '0' => [0b111, 0b101, 0b101, 0b101, 0b111],
        '1' => [0b010, 0b110, 0b010, 0b010, 0b111],
        '2' => [0b111, 0b001, 0b111, 0b100, 0b111],
        '3' => [0b111, 0b001, 0b011, 0b001, 0b111],
        '4' => [0b101, 0b101, 0b111, 0b001, 0b001],
        '5' => [0b111, 0b100, 0b111, 0b001, 0b111],
        '6' => [0b111, 0b100, 0b111, 0b101, 0b111],
        '7' => [0b111, 0b001, 0b010, 0b010, 0b010],
        '8' => [0b111, 0b101, 0b111, 0b101, 0b111],
        '9' => [0b111, 0b101, 0b111, 0b001, 0b111],
        'A' => [0b010, 0b101, 0b111, 0b101, 0b101],
        'B' => [0b110, 0b101, 0b110, 0b101, 0b110],
        'C' => [0b011, 0b100, 0b100, 0b100, 0b011],
        'D' => [0b110, 0b101, 0b101, 0b101, 0b110],
        'E' => [0b111, 0b100, 0b110, 0b100, 0b111],
        'F' => [0b111, 0b100, 0b110, 0b100, 0b100],
        'G' => [0b011, 0b100, 0b101, 0b101, 0b011],
        'H' => [0b101, 0b101, 0b111, 0b101, 0b101],
        'I' => [0b111, 0b010, 0b010, 0b010, 0b111],
        'J' => [0b001, 0b001, 0b001, 0b101, 0b010],
        'K' => [0b101, 0b101, 0b110, 0b101, 0b101],
        'L' => [0b100, 0b100, 0b100, 0b100, 0b111],
        'M' => [0b101, 0b111, 0b111, 0b101, 0b101],
        'N' => [0b110, 0b101, 0b101, 0b101, 0b101],
        'O' => [0b010, 0b101, 0b101, 0b101, 0b010],
        'P' => [0b110, 0b101, 0b110, 0b100, 0b100],
        'Q' => [0b010, 0b101, 0b101, 0b110, 0b011],
        'R' => [0b110, 0b101, 0b110, 0b101, 0b101],
        'S' => [0b011, 0b100, 0b010, 0b001, 0b110],
        'T' => [0b111, 0b010, 0b010, 0b010, 0b010],
        'U' => [0b101, 0b101, 0b101, 0b101, 0b111],
        'V' => [0b101, 0b101, 0b101, 0b101, 0b010],
        'W' => [0b101, 0b101, 0b111, 0b111, 0b101],
        'X' => [0b101, 0b101, 0b010, 0b101, 0b101],
        'Y' => [0b101, 0b101, 0b010, 0b010, 0b010],
        'Z' => [0b111, 0b001, 0b010, 0b100, 0b111],
        ':' => [0b000, 0b010, 0b000, 0b010, 0b000],
        '-' => [0b000, 0b000, 0b111, 0b000, 0b000],
        '.' => [0b000, 0b000, 0b000, 0b000, 0b010],
        ',' => [0b000, 0b000, 0b000, 0b010, 0b100],
        '%' => [0b101, 0b001, 0b010, 0b100, 0b101],
        '/' => [0b001, 0b001, 0b010, 0b100, 0b100],
        '!' => [0b010, 0b010, 0b010, 0b000, 0b010],
        '\'' => [0b010, 0b010, 0b000, 0b000, 0b000],
        '+' => [0b000, 0b010, 0b111, 0b010, 0b000],
        '(' => [0b001, 0b010, 0b010, 0b010, 0b001],
        ')' => [0b100, 0b010, 0b010, 0b010, 0b100],
        '<' => [0b001, 0b010, 0b100, 0b010, 0b001],
        '>' => [0b100, 0b010, 0b001, 0b010, 0b100],
        '_' => [0b000, 0b000, 0b000, 0b000, 0b111],
        // Two strokes, like `'`.
        '"' => [0b101, 0b101, 0b000, 0b000, 0b000],
        // An S with a stroke through its middle.
        '$' => [0b011, 0b110, 0b111, 0b011, 0b110],
        // Raised, as in print, so it isn't taken for an x.
        '*' => [0b101, 0b010, 0b101, 0b000, 0b000],
        ';' => [0b000, 0b010, 0b000, 0b010, 0b100],
        '=' => [0b000, 0b111, 0b000, 0b111, 0b000],
        // Brackets and braces share the parentheses' middle column.
        '[' => [0b011, 0b010, 0b010, 0b010, 0b011],
        ']' => [0b110, 0b010, 0b010, 0b010, 0b110],
        '{' => [0b011, 0b010, 0b110, 0b010, 0b011],
        '}' => [0b110, 0b010, 0b011, 0b010, 0b110],
        '\\' => [0b100, 0b100, 0b010, 0b001, 0b001],
        '|' => [0b010, 0b010, 0b010, 0b010, 0b010],
        // The circumflex and grave accents, on the line instead of above it.
        '^' => [0b010, 0b101, 0b000, 0b000, 0b000],
        '`' => [0b100, 0b010, 0b000, 0b000, 0b000],
        // The tilde accent, its lower stroke level with `-`.
        '~' => [0b000, 0b011, 0b110, 0b000, 0b000],
        // `?` itself, and anything unknown.
        _ => [0b110, 0b001, 0b010, 0b000, 0b010],
    }
}

// ==============================================================================
// Medium: a bold 7 px face for the names in notifications
// ==============================================================================
//
// Between the 3×5 at 1× (cap height 5, ~14 arcmin at wrist distance: below
// the 16 that legibility guidelines ask for) and the 3×5 doubled (cap height
// 10, but only 9 letters to a line), for a name too long for the one and too
// important for the other: about 12 letters a line, at 7 px (~20 arcmin).
// Bold, as handheld LCDs drew their text: vertical stems 2 px, horizontals
// 1 px, so its weight matches the doubled font's (2 px stems) rather than
// the hairline 1×, and a name in it reads as the most important thing on
// the screen. Round letters (O, Q, Ø) and the ones with diagonals (M, N, V,
// W, X, Y) are a column wider than square ones, as in any text face, which
// also tells O from 0. Hand-drawn, not doubled pixels: doubling closes
// diagonals (N reads as H) and counters (Ø reads as a blob).
//
// Like the small font, it has no lowercase (drawn as capitals) and keeps
// two rows above the capitals for accents, so `y` is the top of the
// capitals. Characters it lacks are drawn as its `?`.

/// A glyph of the medium face: `width` columns, two accent rows, then the
/// seven of the capitals.
pub struct MediumGlyph {
    pub width: u32,
    pub rows: [u8; 9],
}

/// Height of the medium face's capitals.
pub const MEDIUM_HEIGHT: i32 = 7;

/// A medium glyph without its accent rows, in its 8 bytes of flash.
#[derive(Clone, Copy)]
struct Medium {
    width: u8,
    rows: [u8; 7],
}

/// `art` (rows separated by spaces, `#` set, anything else clear) as a
/// [`Medium`] glyph, at compile time: the art stays readable here, and
/// costs only its bits.
const fn medium(art: &str) -> Medium {
    let art = art.as_bytes();
    let mut rows = [0; 7];
    let (mut row, mut width, mut x, mut i) = (0, 0, 0, 0);
    while i < art.len() {
        if art[i] == b' ' {
            assert!(width == 0 || x == width, "rows are all as wide");
            width = x;
            x = 0;
            row += 1;
        } else {
            rows[row] = (rows[row] << 1) | (art[i] == b'#') as u8;
            x += 1;
        }
        i += 1;
    }
    assert!(row == 6 && (width == 0 || x == width), "seven rows");
    Medium {
        width: x as u8,
        rows,
    }
}

/// ASCII from the space to `_`, in order (lowercase is drawn as uppercase,
/// and the five after it are in [`medium_glyph`]).
static MEDIUM_ASCII: [Medium; 64] = [
    medium(".. .. .. .. .. .. .."), // space: 4 px between words, with the gaps
    medium("## ## ## ## ## .. ##"),
    medium("##.## ##.## ..... ..... ..... ..... ....."),
    medium(".#..#. .#..#. ###### .#..#. ###### .#..#. .#..#."),
    medium("..#.. .#### ##.#. .###. .#.## ####. ..#.."),
    medium("##...# ##..#. ...##. ..##.. .##... .#..## #...##"),
    medium(".###.. ##.##. ##.##. .###.. ##.### ##.##. .###.#"),
    medium("## ## #. .. .. .. .."),
    medium(".## ##. ##. ##. ##. ##. .##"),
    medium("##. .## .## .## .## .## ##."),
    medium("#.#.# .###. #.#.# ..... ..... ..... ....."),
    medium("...... ..##.. ..##.. ###### ..##.. ..##.. ......"),
    medium(".. .. .. .. ## ## #."),
    medium(".... .... .... #### .... .... ...."),
    medium(".. .. .. .. .. ## ##"),
    medium("...## ...## ..##. ..##. .##.. ##... ##..."),
    medium(".###. ##.## ##.## ##.## ##.## ##.## .###."), // 0
    medium(".##. ###. .##. .##. .##. .##. ####"),
    medium(".###. ##.## ...## ..##. .##.. ##... #####"),
    medium("####. ...## ...## .###. ...## ...## ####."),
    medium("##.## ##.## ##.## ##### ...## ...## ...##"),
    medium("##### ##... ####. ...## ...## ##.## .###."),
    medium(".###. ##... ##... ####. ##.## ##.## .###."),
    medium("##### ...## ...## ..##. .##.. .##.. .##.."),
    medium(".###. ##.## ##.## .###. ##.## ##.## .###."),
    medium(".###. ##.## ##.## .#### ...## ...## .###."), // 9
    medium(".. ## ## .. .. ## ##"),
    medium(".. ## ## .. ## ## #."),
    medium("..... ...## .##.. ##... .##.. ...## ....."),
    medium("..... ..... ##### ..... ##### ..... ....."),
    medium("..... ##... ..##. ...## ..##. ##... ....."),
    medium(".###. ##.## ...## ..##. .##.. ..... .##.."),
    medium(".####. ##..## ##.### ##.### ##.##. ##.... .####."),
    medium(".###. ##.## ##.## ##.## ##### ##.## ##.##"), // A
    medium("####. ##.## ##.## ####. ##.## ##.## ####."),
    medium(".###. ##.## ##... ##... ##... ##.## .###."),
    medium("####. ##.## ##.## ##.## ##.## ##.## ####."),
    medium("##### ##... ##... ####. ##... ##... #####"),
    medium("##### ##... ##... ####. ##... ##... ##..."),
    medium(".###. ##.## ##... ##... ##.## ##.## .####"),
    medium("##.## ##.## ##.## ##### ##.## ##.## ##.##"),
    medium("#### .##. .##. .##. .##. .##. ####"),
    medium("...## ...## ...## ...## ...## ##.## .###."),
    medium("##.## ##.## ####. ###.. ####. ##.## ##.##"),
    medium("##... ##... ##... ##... ##... ##... #####"),
    medium("##...## ###.### ####### ##.#.## ##...## ##...## ##...##"),
    medium("##..## ###.## ###.## ##.### ##.### ##..## ##..##"),
    medium(".####. ##..## ##..## ##..## ##..## ##..## .####."),
    medium("####. ##.## ##.## ####. ##... ##... ##..."),
    medium(".####. ##..## ##..## ##..## ##.### ##..## .###.#"),
    medium("####. ##.## ##.## ####. ###.. ##.## ##.##"),
    medium(".#### ##... ##... .###. ...## ...## ####."),
    medium("###### ..##.. ..##.. ..##.. ..##.. ..##.. ..##.."),
    medium("##.## ##.## ##.## ##.## ##.## ##.## .###."),
    medium("##..## ##..## ##..## ##..## .####. .####. ..##.."),
    medium("##...## ##...## ##...## ##.#.## ####### ###.### ##...##"),
    medium("##..## ##..## .####. ..##.. .####. ##..## ##..##"),
    medium("##..## ##..## .####. ..##.. ..##.. ..##.. ..##.."),
    medium("##### ...## ..##. .##.. ##... ##... #####"), // Z
    medium("### ##. ##. ##. ##. ##. ###"),
    medium("##... ##... .##.. ..##. ..##. ...## ...##"),
    medium("### .## .## .## .## .## ###"),
    medium("..#.. .###. ##.## ..... ..... ..... ....."),
    medium("..... ..... ..... ..... ..... ..... #####"),
];

/// The medium face's glyph for any character (see [`decompose`]), and `?`
/// for what it lacks.
pub fn medium_glyph(c: char) -> MediumGlyph {
    let plain = |m: Medium| {
        let r = m.rows;
        MediumGlyph {
            width: u32::from(m.width),
            rows: [0, 0, r[0], r[1], r[2], r[3], r[4], r[5], r[6]],
        }
    };
    let (c, mark) = decompose(c);
    let mut glyph = match c {
        ' '..='_' => plain(MEDIUM_ASCII[c as usize - ' ' as usize]),
        '`' => plain(const { medium("#. .# .. .. .. .. ..") }),
        '{' => plain(const { medium("..## .##. .##. ##.. .##. .##. ..##") }),
        '|' => plain(const { medium("## ## ## ## ## ## ##") }),
        '}' => plain(const { medium("##.. .##. .##. ..## .##. .##. ##..") }),
        '~' => plain(const { medium("..... ..... .##.# ##.## #.##. ..... .....") }),
        'Æ' => plain(const { medium(".##### ##.#.. ##.#.. #####. ##.#.. ##.#.. ##.###") }),
        'Ç' => plain(const { medium(".###. ##.## ##... ##... ##.## .###. ..#..") }),
        'Ð' => plain(const { medium(".####. .##.## .##.## ###### .##.## .##.## .####.") }),
        // The stroke runs corner to corner, through the counter and out of
        // it, so it can't be taken for an O, a 0, or an 8.
        'Ø' => plain(const { medium(".####.# ##..##. ##.###. ##.###. ###.##. ##..##. #.####.") }),
        'Þ' => plain(const { medium("##... ####. ##.## ##.## ####. ##... ##...") }),
        'ß' => plain(const { medium(".###. ##.## ##.## ####. ##.## ##.## ##.#.") }),
        // Three of its full stops, close together: one glyph, as the
        // small font's.
        '\u{2026}' => plain(
            const { medium("........ ........ ........ ........ ........ ##.##.## ##.##.##") },
        ),
        _ => plain(MEDIUM_ASCII[usize::from(b'?' - b' ')]),
    };
    let width = glyph.width;
    // The small font's marks, centred over the letter (leaning right when
    // they can't be exactly), as their 1 px strokes read as accents next
    // to the bold letters, where 2 px ones merged into blobs.
    let (mark_width, rows) = match mark {
        b' ' => return glyph,
        // Over the stems, as wide as they are, and raised a row, so the
        // dots don't merge with them; 1 px over the narrow I.
        b':' => {
            let dot: u8 = if width >= 5 { 0b11 } else { 0b1 };
            glyph.rows[0] = (dot << (width - dot.count_ones())) | dot;
            return glyph;
        }
        // Only Å has a ring: it stands clear of the A, whose top it would
        // otherwise close into an 8; the A is flat-topped under it.
        b'o' => {
            return MediumGlyph {
                width: 5,
                rows: [
                    0b00100, 0b01010, 0b00100, 0b11011, 0b11011, 0b11111, 0b11011, 0b11011, 0b11011,
                ],
            };
        }
        b'~' => (4, [0b0101, 0b1010]),
        mark => (3, small_accent(mark)),
    };
    let shift = (width + 1 - mark_width) / 2;
    glyph.rows[0] = rows[0] << shift;
    glyph.rows[1] = rows[1] << shift;
    glyph
}

// ==============================================================================
// Laying out a line or a few: names in notifications
// ==============================================================================

/// The faces text is laid out in: the small font at 1×, and the medium
/// face. (The small font at 2× is only ever one line: see
/// [`small_text_width`].)
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Face {
    Small,
    Medium,
}

impl Face {
    fn glyph_width(self, c: char) -> i32 {
        (match self {
            Self::Small => glyph(c).width,
            Self::Medium => medium_glyph(c).width,
        }) as i32
    }

    /// Width in pixels of `text`, excluding the trailing spacing column.
    pub fn width(self, text: &str) -> i32 {
        let total: i32 = text.chars().map(|c| self.glyph_width(c) + 1).sum();
        (total - 1).max(0)
    }
}

/// What follows a line's text: nothing, a hyphen (a word broken across
/// lines), or an ellipsis (the text went on, but there was no more room).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Tail {
    #[default]
    None,
    Hyphen,
    Ellipsis,
}

impl Tail {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "",
            Self::Hyphen => "-",
            Self::Ellipsis => "\u{2026}",
        }
    }
}

/// `&text[start..end]`, for offsets at character boundaries, as layout
/// keeps them.
///
/// Not `str` indexing, whose panic message names the character an offset
/// splits with `{:?}`: escaping it (`char::escape_debug`) takes ~5 KB of
/// Unicode tables and code, used nowhere else.
pub fn slice(text: &str, start: usize, end: usize) -> &str {
    text.get(start..end).expect("offsets at char boundaries")
}

/// One line of laid-out text: `text[start..end]`, then its [`Tail`].
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Line {
    pub start: usize,
    pub end: usize,
    pub tail: Tail,
}

impl Line {
    /// Width in pixels of the line of `text` as drawn in `face`, its tail
    /// and the column before it included.
    pub fn width(self, text: &str, face: Face) -> i32 {
        let tail = match self.tail {
            Tail::None => 0,
            tail => 1 + face.width(tail.as_str()),
        };
        face.width(self.of(text)) + tail
    }

    /// The line's characters in `text`, without the tail.
    pub fn of(self, text: &str) -> &str {
        slice(text, self.start, self.end)
    }
}

/// `text` (trimmed, its spaces single, as [`crate::phone::Text`] keeps
/// it) in `face`, in as many `lines` as it takes, each at most `room`
/// pixels wide; returns how many it used. If it doesn't fit in that many,
/// the last ends in an ellipsis.
///
/// Set left-aligned and ragged, as text this narrow reads best, but
/// balanced: the lines are made as even as they can be without taking
/// more of them (CSS's `text-wrap: balance`), so a name over two lines
/// reads as a block rather than a long line and a stub (`NORSK
/// BIBLIOTEKFORENING` / `MEDLEMSSERVICE`, not `NORSK` /
/// `BIBLIOTEKFORENING MEDLEMSSERVICE`, nor the other way round). Lines
/// break as [`fill`] says.
pub fn wrap(text: &str, face: Face, room: i32, lines: &mut [Line]) -> usize {
    let most = lines.len();
    if most == 0 {
        return 0;
    }
    // Greedy first, to learn how many lines it takes.
    let count = fill(text, face, room, lines);
    if count > most {
        // It doesn't fit: as much as does, then an ellipsis, in place of
        // whatever the last line would have broken with.
        let last = &mut lines[most - 1];
        let ellipsis = face.width(Tail::Ellipsis.as_str()) + 1;
        let rest = slice(text, last.start, text.len());
        let mut end = 0;
        for (i, c) in rest.char_indices() {
            let next = i + c.len_utf8();
            if face.width(slice(rest, 0, next)) + ellipsis > room {
                break;
            }
            end = next;
        }
        // Not after a space or a mark that would read as the end of the
        // text itself.
        let kept = slice(rest, 0, end).trim_end_matches([' ', ',', '.', ':', ';', '-']);
        *last = Line {
            start: last.start,
            end: last.start + kept.len(),
            tail: Tail::Ellipsis,
        };
        return most;
    }
    // Then the narrowest room that still takes no more lines: what greedy
    // filling does there is the most even setting. (Fewer lines than that
    // is never possible narrower, so this only ever evens lines out.)
    // Not narrower than the widest word, though, or a word that fitted
    // would be broken to even the lines out. With a word too long for a
    // line, not at all: it breaks wherever it must, and moving that to
    // even the lines out only broke it, and the ones around it, worse
    // (`VELFERDST-` / `EKNOLOGIA-` / `VDELINGEN`).
    // TODO: hyphenate at syllables, if long words turn out to be common;
    // Norwegian compounds are, and break badly without a dictionary.
    let widest = text
        .split(' ')
        .flat_map(|word| word.split_inclusive('-'))
        .map(|word| face.width(word))
        .max()
        .unwrap_or(0);
    if count > 1 && widest < room {
        let mut narrowest = room;
        while narrowest > widest && fill(text, face, narrowest - 1, lines) == count {
            narrowest -= 1;
        }
        fill(text, face, narrowest, lines);
    }
    count
}

/// `text` filled greedily into `lines`, each at most `room` wide: as much
/// on each line as fits, breaking
///
/// - at a space, but not before a dash standing alone: a dash ends a line
///   (`TANNLEGE -` / `DR. ØSTBY`), it doesn't start one;
/// - after a hyphen in a word (`#FIRMWARE-` / `REVIEW`);
/// - in a word too long for a line of its own: after `/`, `@`, `:`, `_`
///   or `-`, or before a `.`, if it has one, so a link, an address or a
///   file name breaks where its parts meet, without a hyphen that would
///   change it (`CI.EXAMPL-E.ORG` did); in a word with none of those, but
///   with a `.` or `/` anywhere, wherever it must, still without one; and
///   otherwise hyphenated, with at least three letters on each side.
///
/// Returns how many lines it took, which may be more than `lines` holds
/// (those past it aren't kept).
pub fn fill(text: &str, face: Face, room: i32, lines: &mut [Line]) -> usize {
    let bytes = text.as_bytes();
    let dash = |i: usize| bytes.get(i) == Some(&b'-');
    let mut count = 0;
    let mut start = 0;
    while start < text.len() {
        // The last place the line could end so far, and where the next
        // would then start.
        let mut fits: Option<(usize, usize)> = None;
        let mut width = -1;
        let mut overflow = None;
        for (i, c) in slice(text, start, text.len()).char_indices() {
            let i = start + i;
            if c == ' ' && !(dash(i + 1) && bytes.get(i + 2).is_none_or(|&b| b == b' ')) {
                fits = Some((i, i + 1));
            }
            width += face.glyph_width(c) + 1;
            if width > room {
                overflow = Some(i);
                break;
            }
            // After a hyphen between letters.
            if c == '-'
                && i > start
                && bytes[i - 1] != b' '
                && bytes.get(i + 1).is_some_and(|&b| b != b' ')
            {
                fits = Some((i + 1, i + 1));
            }
        }
        let (line, next) = match (overflow, fits) {
            (None, _) => (
                Line {
                    start,
                    end: text.len(),
                    tail: Tail::None,
                },
                text.len(),
            ),
            (Some(_), Some((end, next))) => (
                Line {
                    start,
                    end,
                    tail: Tail::None,
                },
                next,
            ),
            (Some(at), None) => break_word(text, face, room, start, at),
        };
        if let Some(slot) = lines.get_mut(count) {
            *slot = line;
        }
        count += 1;
        start = next;
        while bytes.get(start) == Some(&b' ') {
            start += 1;
        }
    }
    count
}

/// A word from `start` too long for a line, the first character that
/// doesn't fit being at `at`: where to break it (see [`fill`]), and where
/// the next line starts.
fn break_word(text: &str, face: Face, room: i32, start: usize, at: usize) -> (Line, usize) {
    let word = slice(text, start, text.len());
    let word = slice(word, 0, word.find(' ').unwrap_or(word.len()));
    let cut = |end: usize, tail: Tail| (Line { start, end, tail }, end);
    // The word fits, but not the dash after it (see `fill`): a dash
    // starts the next line after all, rather than a word being broken.
    if at >= start + word.len() {
        return cut(start + word.len(), Tail::None);
    }
    // Where its parts meet: after `/ @ : _ -`, or before a `.`.
    let mut joint = None;
    let mut prev = None;
    for (i, c) in word.char_indices() {
        let i = start + i;
        if i > at {
            break;
        }
        if i > start && (c == '.' || matches!(prev, Some('/' | '@' | ':' | '_' | '-'))) {
            joint = Some(i);
        }
        prev = Some(c);
    }
    if let Some(end) = joint {
        return cut(end, Tail::None);
    }
    // Something like a link or a file name has no letters to hyphenate.
    let hyphen = face.width("-") + 1;
    if word.contains(['.', '/']) || at == start {
        return cut(
            at.max(start + word.chars().next().map_or(1, char::len_utf8)),
            Tail::None,
        );
    }
    // Hyphenated: back from where it overflowed until the hyphen fits,
    // keeping at least three letters for the next line, and three here.
    let mut end = at;
    loop {
        let here = slice(text, start, end);
        let left = slice(text, end, start + word.len()).chars().count();
        if face.width(here) + hyphen <= room && left >= 3 {
            if here.chars().count() >= 3 {
                return cut(end, Tail::Hyphen);
            }
            break;
        }
        match here.char_indices().last() {
            Some((i, _)) if i > 0 => end = start + i,
            _ => break,
        }
    }
    // A word too short to break by those rules, but too long for the
    // line: as much as fits, unmarked.
    cut(at, Tail::None)
}

// ==============================================================================
// 5×7 digits: the big clock
// ==============================================================================

pub const DIGIT_W: u32 = 5;

pub const DIGITS: [[u8; 7]; 10] = [
    [
        0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110,
    ], // 0
    [
        0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110,
    ], // 1
    [
        0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111,
    ], // 2
    [
        0b11110, 0b00001, 0b00001, 0b01110, 0b00001, 0b00001, 0b11110,
    ], // 3
    [
        0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010,
    ], // 4
    [
        0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110,
    ], // 5
    [
        0b00110, 0b01000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110,
    ], // 6
    [
        0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000,
    ], // 7
    [
        0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110,
    ], // 8
    [
        0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00010, 0b01100,
    ], // 9
];

// ==============================================================================
// Icons
// ==============================================================================

/// 7×6 heart.
pub const HEART_W: u32 = 7;
pub const HEART: [u8; 6] = [
    0b0110110, 0b1111111, 0b1111111, 0b0111110, 0b0011100, 0b0001000,
];
/// Specular highlight drawn over [`HEART`] in a lighter colour.
pub const HEART_SHINE: [u8; 6] = [
    0b0000000, 0b0100000, 0b0000000, 0b0000000, 0b0000000, 0b0000000,
];

/// 5×7 Bluetooth rune, for "no phone" on the clock face, struck through
/// (see retro's clock face, `themes::retro::clock`).
pub const BLUETOOTH_W: u32 = 5;
pub const BLUETOOTH: [u8; 7] = [
    0b00100, 0b00110, 0b10101, 0b01110, 0b10101, 0b00110, 0b00100,
];

/// 6×7 beamed pair of eighth notes (♫), for the clock face while media
/// plays: a beam two pixels thick, as a printed one is heavier than the
/// stems, two stems, and 2×2 heads, as tall as the Bluetooth rune.
pub const NOTE_W: u32 = 6;
pub const NOTE: [u8; 7] = [
    0b011111, 0b011111, 0b010001, 0b010001, 0b010001, 0b110011, 0b110011,
];

/// 5×5 media controls.
pub const PREVIOUS: [u8; 5] = [0b10001, 0b10011, 0b10111, 0b10011, 0b10001];
pub const NEXT: [u8; 5] = [0b10001, 0b11001, 0b11101, 0b11001, 0b10001];
pub const PLAY: [u8; 5] = [0b01000, 0b01100, 0b01110, 0b01100, 0b01000];
pub const PAUSE: [u8; 5] = [0b11011, 0b11011, 0b11011, 0b11011, 0b11011];
/// Play or pause, whichever the phone wasn't doing: a small play arrow,
/// then a pause's two bars (▸‖), for a toggle sent without knowing whether
/// the phone was playing. 6×5, as it needs the gaps to read (at 5 wide it
/// looked like "HI"): media hints are drawn 6 wide (see
/// [`MEDIA_HINT_W`]).
pub const PLAY_PAUSE: [u8; 5] = [0b000101, 0b100101, 0b110101, 0b100101, 0b000101];
/// How wide a media gesture's hint icon is drawn: [`PLAY_PAUSE`]'s width,
/// so the 5×5 controls sit a pixel in from its left.
pub const MEDIA_HINT_W: u32 = 6;

/// 4×7 footprint: toes, sole, and heel.
pub const FOOT_W: u32 = 4;
pub const FOOT: [u8; 7] = [0b0110, 0b1111, 0b1110, 0b1110, 0b0000, 0b0110, 0b0110];

/// 5×7 bell, for alarms.
pub const BELL_W: u32 = 5;
pub const BELL: [u8; 7] = [
    0b00100, 0b01110, 0b01110, 0b01110, 0b11111, 0b00000, 0b00100,
];

/// 5×7 hourglass, for timers (so a countdown isn't read as a time of day).
pub const HOURGLASS_W: u32 = 5;
pub const HOURGLASS: [u8; 7] = [
    0b11111, 0b10001, 0b01010, 0b00100, 0b01010, 0b10001, 0b11111,
];

/// 4×5 lightning bolt, for charging.
pub const BOLT_W: u32 = 4;
pub const BOLT: [u8; 5] = [0b0011, 0b0110, 0b1111, 0b0110, 0b1100];

// ==============================================================================
// Turn arrows: the next turn while the phone navigates (see `crate::route`)
// ==============================================================================

/// The arrows are 16×16, drawn 2× on the glance: big enough to read at arm's
/// length, but with the strokes of the other icons, four pixels wide. Each
/// is drawn for turning right (or straight on); the left ones are their
/// mirror images (see [`crate::canvas::Canvas::sprite16`]).
pub const TURN_W: u32 = 16;

/// Turns the ASCII art below into rows of bits (`#` set), at compile time,
/// so the art stays readable here and costs only its bits in flash.
const fn art(rows: [&str; 16]) -> [u16; 16] {
    let mut out = [0; 16];
    let mut y = 0;
    while y < 16 {
        let row = rows[y].as_bytes();
        assert!(row.len() == 16, "turn arrows are 16 wide");
        let mut x = 0;
        while x < 16 {
            if row[x] == b'#' {
                out[y] |= 1 << (15 - x);
            }
            x += 1;
        }
        y += 1;
    }
    out
}

pub const TURN_STRAIGHT: [u16; 16] = art([
    "................",
    ".......##.......",
    "......####......",
    ".....######.....",
    "....########....",
    "...##########...",
    "..############..",
    ".##############.",
    "......####......",
    "......####......",
    "......####......",
    "......####......",
    "......####......",
    "......####......",
    "......####......",
    "......####......",
]);

pub const TURN_RIGHT: [u16; 16] = art([
    "................",
    ".........#......",
    ".........##.....",
    ".........###....",
    ".........####...",
    ".........#####..",
    "..#############.",
    "..##############",
    "..#############.",
    "..############..",
    "..####...####...",
    "..####...###....",
    "..####...##.....",
    "..####...#......",
    "..####..........",
    "..####..........",
]);

/// Slight right, and keeping right at a fork.
pub const TURN_SLIGHT_RIGHT: [u16; 16] = art([
    "......########..",
    ".......#######..",
    "........######..",
    ".........#####..",
    "........######..",
    ".......#######..",
    "......####..##..",
    ".....####....#..",
    "....####........",
    "...####.........",
    "...####.........",
    "...####.........",
    "...####.........",
    "...####.........",
    "...####.........",
    "...####.........",
]);

pub const TURN_SHARP_RIGHT: [u16; 16] = art([
    "................",
    "..####..........",
    "..#####.........",
    "..######........",
    "..#######.......",
    "..########......",
    "..####.####...#.",
    "..####..####.##.",
    "..####...######.",
    "..####....#####.",
    "..####....#####.",
    "..####...######.",
    "..####..#######.",
    "..####.########.",
    "..####..........",
    "..####..........",
]);

/// A U-turn to the left (the usual one in right-hand traffic; mirrored for
/// the other).
pub const TURN_UTURN_LEFT: [u16; 16] = art([
    "................",
    "....########....",
    "...##########...",
    "..############..",
    "..####....####..",
    "..####....####..",
    "..####....####..",
    "########..####..",
    ".######...####..",
    "..####....####..",
    "...##.....####..",
    "..........####..",
    "..........####..",
    "..........####..",
    "..........####..",
    "..........####..",
]);

/// Into a roundabout from below, and out to the right.
pub const TURN_ROUNDABOUT_RIGHT: [u16; 16] = art([
    "................",
    "................",
    "................",
    "....######......",
    "...########.#...",
    "..###....#####..",
    "..##......#####.",
    "..##......######",
    "..##......######",
    "..##......#####.",
    "..###....#####..",
    "...########.#...",
    "....######......",
    ".....####.......",
    ".....####.......",
    ".....####.......",
]);

pub const TURN_ROUNDABOUT_STRAIGHT: [u16; 16] = art([
    ".......##.......",
    "......####......",
    ".....######.....",
    "....########....",
    ".....######.....",
    "....########....",
    "...###....###...",
    "...##......##...",
    "...##......##...",
    "...##......##...",
    "...##......##...",
    "...###....###...",
    "....########....",
    ".....######.....",
    "......####......",
    "......####......",
]);

/// The destination: a chequered flag.
pub const TURN_FINISH: [u16; 16] = art([
    "................",
    "..####..##..##..",
    "..####..##..##..",
    "..##..##..##..##",
    "..##..##..##..##",
    "..####..##..##..",
    "..####..##..##..",
    "..##..##..##..##",
    "..##..##..##..##",
    "..##............",
    "..##............",
    "..##............",
    "..##............",
    "..##............",
    "..##............",
    "..##............",
]);

/// Off the route: a warning sign.
pub const TURN_OFF_ROUTE: [u16; 16] = art([
    "................",
    ".......##.......",
    "......####......",
    "......####......",
    ".....##..##.....",
    ".....##..##.....",
    "....##.##.##....",
    "....##.##.##....",
    "...##..##..##...",
    "...##..##..##...",
    "..##...##...##..",
    "..##........##..",
    ".##....##....##.",
    ".##....##....##.",
    "################",
    "################",
]);

/// A turn the phone didn't name: a blank diamond, like a road sign.
pub const TURN_UNKNOWN: [u16; 16] = art([
    "................",
    ".......##.......",
    "......####......",
    ".....######.....",
    "....###..###....",
    "...###....###...",
    "..###......###..",
    ".###........###.",
    ".###........###.",
    "..###......###..",
    "...###....###...",
    "....###..###....",
    ".....######.....",
    "......####......",
    ".......##.......",
    "................",
]);
