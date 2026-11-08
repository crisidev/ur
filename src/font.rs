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
/// (Gadgetbridge's encoding; anything beyond it is already `?`), so this
/// covers every Latin-1 letter. Lowercase is drawn as uppercase: 3×5 has no
/// room for distinct lowercase forms.
pub fn glyph(c: char) -> Glyph {
    let plain = |rows: [u8; 5]| Glyph {
        width: 3,
        rows: [0, 0, rows[0], rows[1], rows[2], rows[3], rows[4]],
    };
    let accented = |base: char, [a, b]: [u8; 2]| {
        let r = small_glyph(base);
        Glyph {
            width: 3,
            rows: [a, b, r[0], r[1], r[2], r[3], r[4]],
        }
    };
    if c.is_ascii() {
        return plain(small_glyph(c));
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
        _ => plain(small_glyph('?')),
    }
}

/// Looks up a glyph for an ASCII character (see [`fold`] for the rest).
/// Lowercase is drawn as uppercase (3×5 has no room for distinct lowercase
/// forms), and anything else unknown becomes `?`.
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
        _ => [0b110, 0b001, 0b010, 0b000, 0b010], // '?'
    }
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

/// 5×5 media controls.
pub const PREVIOUS: [u8; 5] = [0b10001, 0b10011, 0b10111, 0b10011, 0b10001];
pub const NEXT: [u8; 5] = [0b10001, 0b11001, 0b11101, 0b11001, 0b10001];
pub const PLAY: [u8; 5] = [0b01000, 0b01100, 0b01110, 0b01100, 0b01000];
pub const PAUSE: [u8; 5] = [0b11011, 0b11011, 0b11011, 0b11011, 0b11011];

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
