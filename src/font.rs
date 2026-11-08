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

/// Accent marks, two rows each.
const ACUTE: [u8; 2] = [0b001, 0b010];
const GRAVE: [u8; 2] = [0b100, 0b010];
const CIRCUMFLEX: [u8; 2] = [0b010, 0b101];
const TILDE: [u8; 2] = [0b011, 0b110];
/// Raised a row above the letter: directly on top, the dots merged into the
/// stems of U (whose top row is also `101`).
const DIAERESIS: [u8; 2] = [0b101, 0b000];
/// Over A's pointed top, this closes into a little ring.
const RING: [u8; 2] = [0b111, 0b101];

/// The glyph for any character. Names in notifications arrive as Latin-1
/// (Gadgetbridge's encoding), so this covers all of printable ASCII and
/// every Latin-1 letter. Anything beyond Latin-1 arrives escaped (`\u2019`,
/// see [`crate::gb`]); of that, the punctuation phones like to use (curly
/// quotes, dashes, the ellipsis) is drawn as its ASCII look-alike, and the
/// rest as `?`. Lowercase is drawn as uppercase: 3×5 has no room for
/// distinct lowercase forms.
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
    let accented = |base: char, [a, b]: [u8; 2]| {
        let r = small_glyph(base);
        Glyph {
            width: 3,
            rows: [a, b, r[0], r[1], r[2], r[3], r[4]],
        }
    };
    match c {
        // Leaning, as a handwritten one does: upright, it's a grid of dots.
        '#' => return wide([0b0101, 0b1111, 0b1010, 0b1111, 0b1010]),
        // Two loops, the lower one open to the right for the tail.
        '&' => return wide([0b0100, 0b1010, 0b0100, 0b1010, 0b0101]),
        // An `a` (the 2×2 block) in a ring open at the bottom right.
        '@' => return wide([0b0110, 0b1001, 0b1011, 0b1000, 0b0110]),
        _ if c.is_ascii() => return plain(small_glyph(c)),
        _ => {}
    }
    match c.to_uppercase().next().unwrap_or(c) {
        'À' => accented('A', GRAVE),
        'Á' => accented('A', ACUTE),
        'Â' => accented('A', CIRCUMFLEX),
        'Ã' => accented('A', TILDE),
        'Ä' => accented('A', DIAERESIS),
        'Å' => accented('A', RING),
        'Æ' => Glyph {
            width: 5,
            rows: [0, 0, 0b01111, 0b10100, 0b11110, 0b10100, 0b10111],
        },
        'Ç' => plain([0b011, 0b100, 0b100, 0b011, 0b010]),
        'È' => accented('E', GRAVE),
        'É' => accented('E', ACUTE),
        'Ê' => accented('E', CIRCUMFLEX),
        'Ë' => accented('E', DIAERESIS),
        'Ì' => accented('I', GRAVE),
        'Í' => accented('I', ACUTE),
        'Î' => accented('I', CIRCUMFLEX),
        'Ï' => accented('I', DIAERESIS),
        'Ð' => plain([0b110, 0b101, 0b111, 0b101, 0b110]),
        'Ñ' => accented('N', TILDE),
        'Ò' => accented('O', GRAVE),
        'Ó' => accented('O', ACUTE),
        'Ô' => accented('O', CIRCUMFLEX),
        'Õ' => accented('O', TILDE),
        'Ö' => accented('O', DIAERESIS),
        // An O with a slash poking out at both ends.
        'Ø' => plain([0b011, 0b101, 0b111, 0b101, 0b110]),
        'Ù' => accented('U', GRAVE),
        'Ú' => accented('U', ACUTE),
        'Û' => accented('U', CIRCUMFLEX),
        'Ü' => accented('U', DIAERESIS),
        'Ý' => accented('Y', ACUTE),
        'Ÿ' => accented('Y', DIAERESIS),
        'Þ' => plain([0b100, 0b110, 0b101, 0b110, 0b100]),
        // `to_uppercase` turns ß into "SS"; its first character would lose
        // the second, so it gets its own glyph instead.
        'S' if c == 'ß' => plain([0b010, 0b101, 0b110, 0b101, 0b110]),
        '\u{a0}' => plain(small_glyph(' ')),
        // A little ring at the top, for the clock face's temperature (and
        // any notification that mentions one).
        '°' => plain([0b010, 0b101, 0b010, 0b000, 0b000]),
        // A small x in the middle three rows, as in print, so it isn't
        // taken for an X: the clock face's step scale ("×1K"), and any
        // dimensions or "2×" in a notification.
        '×' => plain([0b000, 0b101, 0b010, 0b101, 0b000]),
        // Typographic punctuation, which phones (and keyboards' smart
        // quotes) put in names and titles, as the ASCII it stands in for.
        // The same table as Gadgetbridge's own (`renderUnicodeAsImage`),
        // which it only applies when it renders text as images.
        '\u{2018}' | '\u{2019}' | '\u{201b}' | '\u{2032}' | '\u{2b9}' => plain(small_glyph('\'')),
        '\u{201c}' | '\u{201d}' | '\u{201e}' | '\u{201f}' | '\u{2033}' => plain(small_glyph('"')),
        '\u{201a}' => plain(small_glyph(',')),
        // Hyphens, dashes, and the minus sign.
        '\u{2010}'..='\u{2015}' | '\u{2212}' => plain(small_glyph('-')),
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
// Large text: a short line, or two of about the same width
// ==============================================================================

/// How a short text reads in large type (see [`large`]): on one line, or
/// split at a space into two.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Large<'a> {
    One(&'a str),
    Two(&'a str, &'a str),
}

/// How unequal two large lines may be: the shorter at least 7/10 of the
/// longer, in pixels. In this all but monospaced font, a 5- and a 5-letter
/// word are equal, 4 and 5 make 79%, 5 and 7 just make it at 70%, and 4
/// and 6 (65%) or 3 and 8 (35%) don't. Below about this, the pair reads
/// as a long line with a stub hanging off it rather than as a block.
const BALANCE_TENTHS: i32 = 7;

/// `text` (trimmed) in large type, at `scale`, within `room` pixels: on
/// one line if it fits, or else on two, split at whichever space makes
/// them most equal, if both fit and they're about equal (see
/// [`BALANCE_TENTHS`]): a two-word name, typically. `None` if neither:
/// such text goes in small type, wrapped. Two lines never hyphenate: a
/// broken word in large type is harder to read than the same word whole
/// in small type, and so is a big first line over a one-word second.
pub fn large(text: &str, room: i32, scale: i32) -> Option<Large<'_>> {
    let text = text.trim();
    if small_text_width(text) * scale <= room {
        return Some(Large::One(text));
    }
    // The split whose longer line is shortest is also the most equal one:
    // the two lines' widths add up to the same, whichever space it's at.
    // Ties go to the first such split, its top line the shorter.
    let mut best: Option<(i32, i32, &str, &str)> = None;
    for (i, _) in text.match_indices(' ') {
        let (top, bottom) = (text[..i].trim_end(), text[i + 1..].trim_start());
        let (a, b) = (small_text_width(top), small_text_width(bottom));
        let (shorter, longer) = (a.min(b), a.max(b));
        if longer * scale <= room && best.is_none_or(|(_, best, ..)| longer < best) {
            best = Some((shorter, longer, top, bottom));
        }
    }
    let (shorter, longer, top, bottom) = best?;
    (shorter * 10 >= longer * BALANCE_TENTHS).then_some(Large::Two(top, bottom))
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

/// 5×7 Bluetooth rune.
pub const BLUETOOTH_W: u32 = 5;
pub const BLUETOOTH: [u8; 7] = [
    0b00100, 0b00110, 0b10101, 0b01110, 0b10101, 0b00110, 0b00100,
];

/// 6×7 beamed pair of eighth notes (♫), for the clock face while media
/// plays: a beam two pixels thick, as a printed one is heavier than the
/// stems, two stems, and 2×2 heads, as tall as the Bluetooth rune beside
/// it.
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
