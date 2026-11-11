//! Living Tokyo's info and settings screens, and Home Assistant's key.

use super::BACKGROUND;
use super::layout::{SETTINGS_ROW, SETTINGS_TOP};
use crate::bthome::Identity;
use crate::canvas::{Canvas, Color};
use crate::font;
use crate::themes::helpers::{self, address, hex, hms, percent, volts};
use crate::themes::{RowKind, SettingsPage};
use crate::ui::{Fits, Status};

/// First value row, and the spacing between rows.
const INFO_TOP: i32 = 12;
const INFO_LINE: i32 = 8;

/// The watch's state, for a look (and for bug reports). The settings that
/// used to be at its foot have a screen of their own, before it.
pub(super) fn render_info(canvas: &mut Canvas, status: &Status) {
    canvas.clear(BACKGROUND);
    // The version without its build metadata (`0.9.0-pr.12`); the commit
    // goes in the BUILD row.
    let (release, build) = crate::version::shown();
    let end = canvas.text(4, 3, "UR ", Color::ACCENT);
    canvas.text(end, 3, release, Color::ACCENT);

    let battery = status.battery;
    let mut charge = percent(battery.percent());
    charge.push(' ').fits();
    charge.push_str(&volts(battery.millivolts())).fits();
    let uptime = hms(status.uptime_secs);
    // "PAIRED" rather than "CONNECTED": the phone only counts as here once
    // it has written over an authenticated link (see `ble`), which takes
    // its bond, so this says both that it's connected and that it's paired
    // (and bonded: ur always stores the keys it pairs with).
    let phone = if status.connected {
        "PAIRED"
    } else if let Some(why) = status.unpaired {
        why.title()
    } else {
        "WAITING"
    };
    // Which commit this build is from, for bug reports: "DEV" for a local
    // `cargo build`, which doesn't know.
    let build = match build {
        "" => "DEV",
        build => build,
    };
    let rows = [
        ("BATTERY", charge.as_str()),
        ("POWER", battery.status()),
        ("UPTIME", &uptime),
        ("PHONE", phone),
        ("BUILD", build),
    ];
    for (i, (label, value)) in rows.into_iter().enumerate() {
        let y = INFO_TOP + INFO_LINE * i as i32;
        canvas.text(4, y, label, Color::TEXT_DIM);
        canvas.text_right(77, y, value, Color::TEXT);
    }

    // Storage mode is a long press here, which nothing else would show.
    // TODO: tapping still buzzes (a motor test); drop that once alarms
    // exercise the motor.
    canvas.text_centered(40, 68, "HOLD: STORAGE MODE", Color::TEXT_FAINT);
}

/// Whether a settings row's value fits right of its label, with a gap.
fn row_fits(label: &str, value: &str) -> bool {
    4 + font::small_text_width(label) + 4 <= 77 - font::small_text_width(value)
}

// The settings: screens of at most three rows, each a tap, between
// hairlines, the rows 20 canvas pixels (~7 mm) tall, a fingertip and
// some.

/// Which row a tap at `y` is on, of `count`: the title's strip above the
/// rows counts as the first's, a forgiving target at the screen's edge.
pub(super) fn row_at(y: i32, count: usize) -> Option<usize> {
    helpers::row_at(y.max(SETTINGS_TOP), SETTINGS_TOP, SETTINGS_ROW, count)
}

/// A settings screen: its title, then each row's label on the left and
/// its value on the right. A value to step through is bright; one that
/// can't change in this build, or a screen to go to, dim.
pub(super) fn render_settings(canvas: &mut Canvas, page: &SettingsPage) {
    canvas.clear(BACKGROUND);
    canvas.text(4, 3, page.title, Color::ACCENT);
    for (i, row) in page.rows.iter().enumerate() {
        let top = SETTINGS_TOP + SETTINGS_ROW * i as i32;
        if i > 0 {
            canvas.fill_rect(4, top, 72, 1, Color::TEXT_FAINT);
        }
        debug_assert!(
            row_fits(row.label, row.value),
            "{} is too long for {}",
            row.value,
            row.label
        );
        let y = top + (SETTINGS_ROW - 5) / 2 + 1;
        canvas.text(4, y, row.label, Color::TEXT_DIM);
        let color = match row.kind {
            RowKind::Choice => Color::TEXT,
            RowKind::Fixed | RowKind::Link => Color::TEXT_DIM,
        };
        canvas.text_right(77, y, row.value, color);
        // A caveat, faint, after the label: an asterisk, as on a
        // footnote, says it's about the value beside it, not one of its
        // own.
        if let Some(note) = row.note {
            canvas.text(
                4 + font::small_text_width(row.label) + 5,
                y,
                note,
                Color::TEXT_FAINT,
            );
        }
    }
}

/// Home Assistant's key as a QR code, filling the screen (see
/// [`Canvas::qr`]), for a phone's camera: what it reads is the key as
/// text (32 hex digits, which Home Assistant takes in capitals too), to
/// paste into its BTHome integration. A tap shows it as text instead.
pub(super) fn render_ha_qr(canvas: &mut Canvas, identity: Option<&Identity>) {
    canvas.clear(BACKGROUND);
    let Some(identity) = identity else {
        canvas.text_centered(40, 36, "NO KEY YET", Color::TEXT_DIM);
        return;
    };
    canvas.qr(&identity.key);
}

/// Home Assistant's key, for typing into its BTHome integration: 32 hex
/// digits, big, in four rows of two groups of four, and the address it'll
/// find the watch at, for picking it out among what it discovers.
pub(super) fn render_ha_key(canvas: &mut Canvas, identity: Option<&Identity>) {
    canvas.clear(BACKGROUND);
    canvas.text_centered(40, 3, "BTHOME KEY", Color::ACCENT);
    let Some(identity) = identity else {
        canvas.text_centered(40, 36, "NO KEY YET", Color::TEXT_DIM);
        return;
    };
    for (row, bytes) in identity.key.chunks(4).enumerate() {
        let mut text = [b' '; 9];
        for (i, &byte) in bytes.iter().enumerate() {
            // A space between the two groups.
            let at = 2 * i + usize::from(i >= 2);
            text[at..at + 2].copy_from_slice(&hex(byte));
        }
        // Hex digits and spaces, so never empty (and no `expect`, whose
        // `Utf8Error` `Debug` would cost flash).
        let text = core::str::from_utf8(&text).unwrap_or_default();
        // 2×: 8 px a character, so 72 px (bar the last one's gap).
        canvas.text_scaled(5, 13 + 13 * row as i32, text, Color::TEXT, 2);
    }
    canvas.text_centered(40, 69, &address(&identity.address), Color::TEXT_DIM);
}
