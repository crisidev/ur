//! Retro's info and settings screens, and Home Assistant's key.

use core::fmt::Write as _;

use super::BACKGROUND;
use super::layout::{SETTINGS_ROW, SETTINGS_TOP};
use crate::bthome::Identity;
use crate::buttons;
use crate::canvas::{Canvas, Color};
use crate::themes::Settings;
use crate::ui::{Fits, Row, Status};

/// First value row, and the spacing between rows.
const INFO_TOP: i32 = 12;
const INFO_LINE: i32 = 8;

/// The watch's state, for a look (and for bug reports). The settings that
/// used to be at its foot have a screen of their own, before it.
pub(super) fn render_info(canvas: &mut Canvas, status: &Status) {
    canvas.clear(BACKGROUND);
    // The version without its build metadata (`0.9.0-pr.12`); the commit
    // goes in the BUILD row (see `build.rs`).
    canvas.text(4, 3, concat!("UR ", env!("UR_RELEASE")), Color::ACCENT);

    let battery = status.battery;
    let mv = battery.millivolts();
    let secs = status.uptime_secs;
    let mut values: [heapless::String<12>; 3] = Default::default();
    write!(
        values[0],
        "{}% {}.{:02}V",
        battery.percent(),
        mv / 1000,
        (mv % 1000) / 10
    )
    .fits();
    write!(values[1], "{}", battery.status()).fits();
    write!(
        values[2],
        "{:02}:{:02}:{:02}",
        secs / 3600,
        (secs / 60) % 60,
        secs % 60
    )
    .fits();
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
    let build = match env!("UR_BUILD") {
        "" => "DEV",
        build => build,
    };
    let rows = [
        ("BATTERY", values[0].as_str()),
        ("POWER", &values[1]),
        ("UPTIME", &values[2]),
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

/// One setting a row, each a tap for its next value, between hairlines.
pub(super) fn render_settings(canvas: &mut Canvas, settings: &Settings) {
    let Settings {
        rows,
        scheme,
        light,
        buttons,
        layout,
    } = *settings;
    canvas.clear(BACKGROUND);
    canvas.text(4, 3, "SETTINGS", Color::ACCENT);
    for (i, &row) in rows.iter().enumerate() {
        let top = SETTINGS_TOP + SETTINGS_ROW * i as i32;
        if i > 0 {
            canvas.fill_rect(4, top, 72, 1, Color::TEXT_FAINT);
        }
        let (label, value, color) = match row {
            Row::Theme => ("THEME", scheme, Color::TEXT),
            Row::Light => ("LIGHT", light.name(), Color::TEXT),
            Row::Buttons => ("BUTTONS", buttons.name(), Color::TEXT),
            Row::Grid => ("GRID", layout.name(), Color::TEXT),
            // Not a value, but somewhere to go.
            Row::BtHomeKey => ("BTHOME KEY", "SHOW", Color::TEXT_DIM),
        };
        let y = top + (SETTINGS_ROW - 5) / 2 + 1;
        canvas.text(4, y, label, Color::TEXT_DIM);
        canvas.text_right(77, y, value, color);
        // Two layers reach Home Assistant unlike six buttons do (as dimmers
        // turned by the code): FAQ B04 says how, and how to automate them.
        // The asterisk, as on a footnote, says it's a caveat to the value
        // beside it, not a value of its own.
        if row == Row::Grid && buttons.bthome() && layout == buttons::Layout::TwoLayers {
            canvas.text(24, y, "*B04", Color::TEXT_FAINT);
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

const HEX: &[u8; 16] = b"0123456789ABCDEF";

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
    let digit = |byte: u8, high: bool| HEX[usize::from(if high { byte >> 4 } else { byte & 15 })];
    for (row, bytes) in identity.key.chunks(4).enumerate() {
        let mut text = [b' '; 9];
        for (i, &byte) in bytes.iter().enumerate() {
            // A space between the two groups.
            let at = 2 * i + usize::from(i >= 2);
            text[at] = digit(byte, true);
            text[at + 1] = digit(byte, false);
        }
        // Hex digits and spaces, so never empty (and no `expect`, whose
        // `Utf8Error` `Debug` would cost flash).
        let text = core::str::from_utf8(&text).unwrap_or_default();
        // 2×: 8 px a character, so 72 px (bar the last one's gap).
        canvas.text_scaled(5, 13 + 13 * row as i32, text, Color::TEXT, 2);
    }
    // As people write it: most significant byte first.
    let mut address = heapless::String::<17>::new();
    for (i, &byte) in identity.address.iter().rev().enumerate() {
        if i > 0 {
            let _ = address.push(':');
        }
        let _ = address.push(char::from(digit(byte, true)));
        let _ = address.push(char::from(digit(byte, false)));
    }
    canvas.text_centered(40, 69, &address, Color::TEXT_DIM);
}
