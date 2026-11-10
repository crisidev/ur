//! What any theme is likely to need, whatever it looks like: mechanics
//! that choose no colour, position, size or font (each takes those from
//! the theme), so that a theme carries only what's its own. Every theme
//! may use these, as it may `crate::canvas`, `crate::font`,
//! `crate::palette` and the data a screen shows; never another theme's
//! module (see AUTHORING-THEMES.md).
//!
//! - Formatting: a time, a countdown, a duration, a date, tenths, a
//!   percentage, volts, hex, a Bluetooth address, in the watch's words.
//! - Text: one line cut to fit with an ellipsis, centred lines, and how
//!   far text overflows its room (for `Theme::call_overflow` and the
//!   like; the marquee's own timing is `crate::canvas`'s, shared with
//!   the UI).
//! - Taps: which of a stack of rows a tap is on, given the theme's rows.
//! - The 5×7 digit glyph for a digit.
//!
//! The dim look's rule, that it keeps to the colours the panel's idle
//! mode shows, is `crate::palette::Palette::idle_mode_safe`, a `const
//! fn` for a theme to assert on its dim palette.
//!
//! What stays the theme's: layout, colours, which font where, and what a
//! screen shows.

use core::fmt::Write as _;

use crate::canvas::{Canvas, Color};
use crate::font;
use crate::ui::Fits;

// ==============================================================================
// Formatting
// ==============================================================================

/// Time left, as short as it reads well: "M:SS" under 10 minutes (where
/// seconds matter), "42 MIN" under an hour, "2H05" beyond.
pub fn countdown(seconds: i64) -> heapless::String<8> {
    let mut text = heapless::String::new();
    // Hours stop at 99, as the editor's two digits do.
    let seconds = seconds.clamp(0, 100 * 3600 - 1);
    let result = if seconds < 10 * 60 {
        write!(text, "{}:{:02}", seconds / 60, seconds % 60)
    } else if seconds < 60 * 60 {
        write!(text, "{} MIN", (seconds + 59) / 60)
    } else {
        write!(text, "{}H{:02}", seconds / 3600, seconds / 60 % 60)
    };
    result.fits();
    text
}

/// `minutes` (after midnight, or a duration) as HH:MM.
pub fn hhmm(minutes: i64) -> heapless::String<6> {
    // Two digits of hours, and never negative: a time of day, or a
    // night's sleep. Every caller lays the text out as five characters
    // wide; more would still fit the buffer (to "999:59"), and so pass
    // unnoticed, shifted or cut on the screen.
    debug_assert!((0..100 * 60).contains(&minutes), "not a time: {minutes}");
    let mut text = heapless::String::new();
    write!(text, "{:02}:{:02}", minutes / 60, minutes % 60).fits();
    text
}

/// A timer's duration (not counting down): "3 MIN", "1H30".
pub fn duration(minutes: u16) -> heapless::String<8> {
    let mut text = heapless::String::new();
    let result = if minutes < 60 {
        write!(text, "{minutes} MIN")
    } else {
        write!(text, "{}H{:02}", minutes / 60, minutes % 60)
    };
    result.fits();
    text
}

/// `seconds` as HH:MM:SS, the hours going past 99 as they must (an
/// uptime of weeks).
pub fn hms(seconds: u64) -> heapless::String<12> {
    let mut text = heapless::String::new();
    let (hours, minutes) = (seconds / 3600, (seconds / 60) % 60);
    write!(text, "{hours:02}:{minutes:02}:{:02}", seconds % 60).fits();
    text
}

/// A date as YYYY-MM-DD.
pub fn iso_date(year: u16, month: u8, day: u8) -> heapless::String<10> {
    let mut text = heapless::String::new();
    write!(text, "{year:04}-{month:02}-{day:02}").fits();
    text
}

/// Tenths (of a degree, say) as a number with one decimal: "-4.2", "21.0".
pub fn tenths(tenths: i32) -> heapless::String<12> {
    let mut text = heapless::String::new();
    let sign = if tenths < 0 { "-" } else { "" };
    let magnitude = tenths.unsigned_abs();
    write!(text, "{sign}{}.{}", magnitude / 10, magnitude % 10).fits();
    text
}

/// A percentage: "80%".
pub fn percent(percent: u32) -> heapless::String<12> {
    let mut text = heapless::String::new();
    write!(text, "{percent}%").fits();
    text
}

/// Millivolts as volts to the hundredth: "3.98V".
pub fn volts(millivolts: u32) -> heapless::String<12> {
    let mut text = heapless::String::new();
    write!(
        text,
        "{}.{:02}V",
        millivolts / 1000,
        (millivolts % 1000) / 10
    )
    .fits();
    text
}

/// A byte as two hex digits, capitals, as ASCII.
pub fn hex(byte: u8) -> [u8; 2] {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    [HEX[usize::from(byte >> 4)], HEX[usize::from(byte & 15)]]
}

/// A Bluetooth address as people write it: most significant byte first,
/// "C3:12:…", from the little-endian bytes the radio uses.
pub fn address(address: &[u8; 6]) -> heapless::String<17> {
    let mut text = heapless::String::new();
    for (i, &byte) in address.iter().rev().enumerate() {
        if i > 0 {
            text.push(':').fits();
        }
        for digit in hex(byte) {
            text.push(char::from(digit)).fits();
        }
    }
    text
}

// ==============================================================================
// Text
// ==============================================================================

/// `text` in `face` at (`x`, `y`), in `room` px, ending in an ellipsis if
/// it doesn't fit.
pub fn fitted(
    canvas: &mut Canvas,
    (x, y): (i32, i32),
    text: &str,
    face: font::Face,
    room: i32,
    color: Color,
) {
    let mut line = [font::Line::default()];
    if font::wrap(text, face, room, &mut line) > 0 {
        canvas.line(x, y, text, line[0], face, color);
    }
}

/// `lines` of the small font, each centred on `centre_x`, the first at
/// `y` and each `pitch` below the last.
pub fn centred_lines(
    canvas: &mut Canvas,
    (centre_x, y): (i32, i32),
    pitch: i32,
    lines: &[&str],
    color: Color,
) {
    for (i, line) in lines.iter().enumerate() {
        canvas.text_centered(centre_x, y + pitch * i as i32, line, color);
    }
}

/// How far `text`, in the small font at `scale`, overflows `room` px: what
/// a marquee scrolls (none if it's 0 or less).
pub fn overflow(text: &str, scale: i32, room: i32) -> i32 {
    font::small_text_width(text) * scale - room
}

// ==============================================================================
// Taps
// ==============================================================================

/// Which of `count` rows, each `height` px tall and stacked from `top`,
/// a tap at `y` is on, if any (none above `top`, nor below the last).
pub fn row_at(y: i32, top: i32, height: i32, count: usize) -> Option<usize> {
    if y < top || height <= 0 {
        return None;
    }
    let row = usize::try_from((y - top) / height).ok()?;
    (row < count).then_some(row)
}

// ==============================================================================
// Glyphs
// ==============================================================================

/// The 5×7 glyph for `digit` ([`font::DIGITS`]). A value too big for its
/// cell (a bug upstream) draws as 9 rather than indexing out of range: a
/// wrong digit on the screen beats a panic and a reset.
pub fn digit_glyph(digit: u64) -> &'static [u8; 7] {
    // Where debug assertions are on (the simulator and its fuzzer), that
    // bug is caught here instead: a 9 that isn't one reads as a fine
    // screen. (As was the timer editor's hours, past 99: F1.)
    debug_assert!(digit <= 9, "not a digit: {digit}");
    &font::DIGITS[digit.min(9) as usize]
}
