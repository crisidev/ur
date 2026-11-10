//! Retro's screens of the watch itself: storage mode, switching off,
//! pairing, and an update arriving.

use embassy_time::Instant;

use super::{BACKGROUND, draw_digit};
use crate::canvas::{Canvas, Color};
use crate::system_off::Reason;
use crate::themes::helpers::centred_lines;

// ==============================================================================
// Switching off
// ==============================================================================

/// The line pitch of these screens' messages: Info's rows' (see
/// `settings`), kept here, so that either file can be replaced alone.
const LINE: i32 = 8;

/// Lines of a message about switching off, centred, from `y` down.
pub(super) fn render_lines(canvas: &mut Canvas, y: i32, lines: &[&str], color: Color) {
    centred_lines(canvas, (40, y), LINE, lines, color);
}

/// Asks whether to switch to storage mode (see [`crate::system_off`]).
pub(super) fn render_storage_prompt(canvas: &mut Canvas) {
    canvas.clear(BACKGROUND);
    canvas.text_centered(40, 6, "STORAGE MODE", Color::ACCENT);
    let explanation = ["OFF UNTIL THE", "BUTTON OR THE", "CHARGER WAKES IT"];
    render_lines(canvas, 20, &explanation, Color::TEXT);
    render_lines(
        canvas,
        54,
        &["TAP: OFF NOW", "ELSE: CANCEL"],
        Color::TEXT_DIM,
    );
}

/// What shows for the few seconds before the watch switches itself off.
pub(super) fn render_off(canvas: &mut Canvas, reason: Reason) {
    canvas.clear(BACKGROUND);
    let (title, color, wake): (_, _, &[&str]) = match reason {
        Reason::BatteryEmpty => ("BATTERY EMPTY", Color::BAD, &["CHARGE TO", "WAKE IT"]),
        Reason::Storage => (
            "STORAGE MODE",
            Color::ACCENT,
            &["BUTTON OR", "CHARGER", "WAKES IT"],
        ),
    };
    canvas.text_centered(40, 14, title, color);
    render_lines(canvas, 30, &["SWITCHING OFF"], Color::TEXT_DIM);
    render_lines(canvas, 46, wake, Color::TEXT);
}

// ==============================================================================
// Pairing
// ==============================================================================

/// The six-digit passkey, big enough to read at a glance while typing it on
/// the phone.
pub(super) fn render_pairing(canvas: &mut Canvas, passkey: u32) {
    canvas.clear(BACKGROUND);
    canvas.text_centered(40, 12, "PAIRING", Color::ACCENT);
    canvas.text_centered(40, 22, "ENTER ON PHONE:", Color::TEXT_DIM);

    // Six 2× digits (10 px) with 2 px gaps: 70 px, centred.
    let mut digits = [0u64; 6];
    let mut rest = u64::from(passkey);
    for digit in digits.iter_mut().rev() {
        *digit = rest % 10;
        rest /= 10;
    }
    for (i, &digit) in digits.iter().enumerate() {
        draw_digit(canvas, 5 + 12 * i as i32, 36, digit, 2, Color::TEXT);
    }
}

// ==============================================================================
// Firmware update
// ==============================================================================

/// A chunky segmented progress bar, as on an old console's loading screen:
/// 16 cells in a frame, filled cells lit with a highlight, and the cell being
/// filled flickering in the transition's dither pattern.
pub(super) fn render_update(canvas: &mut Canvas, percent: u8, now: Instant) {
    const CELLS: i32 = 16;
    const CELL: i32 = 4;
    const LEFT: i32 = 40 - (CELLS * CELL) / 2;
    const TOP: i32 = 38;

    canvas.clear(BACKGROUND);
    canvas.text_centered(40, 14, "UPDATING", Color::ACCENT);
    canvas.text_centered(40, 24, "KEEP CLOSE", Color::TEXT_DIM);

    // Frame, with a drop shadow like the digits'.
    let (width, height) = (CELLS * CELL + 3, CELL + 5);
    canvas.fill_rect(LEFT - 1, TOP - 1, width, height, Color::SHADOW);
    canvas.fill_rect(LEFT - 2, TOP - 2, width, height, Color::TEXT_DIM);
    canvas.fill_rect(LEFT - 1, TOP - 1, width - 2, height - 2, BACKGROUND);

    let filled = i32::from(percent) * CELLS / 100;
    for cell in 0..CELLS {
        let x = LEFT + cell * CELL;
        if cell < filled {
            // The heart's red: the one dash of colour in otherwise
            // single-hue themes (handheld), so progress pops.
            canvas.fill_rect(x, TOP, CELL - 1, CELL, Color::HEART);
            canvas.fill_rect(x, TOP, CELL - 1, 1, Color::HEART_SHINE);
        } else if cell == filled {
            // Flip the checkerboard's phase every frame (one a second).
            let phase = (now.as_secs() % 2) as i32;
            for dy in 0..CELL {
                for dx in 0..CELL - 1 {
                    let color = if (dx + dy + phase) % 2 == 0 {
                        Color::DITHER_A
                    } else {
                        Color::DITHER_B
                    };
                    canvas.pixel(x + dx, TOP + dy, color);
                }
            }
        }
    }

    // The percentage in the clock's 2× digits, right-aligned under the bar.
    let mut x = 40 + 17;
    let mut rest = u64::from(percent);
    loop {
        x -= 12;
        draw_digit(canvas, x, 52, rest % 10, 2, Color::TEXT);
        rest /= 10;
        if rest == 0 {
            break;
        }
    }
    canvas.text(40 + 19, 59, "%", Color::TEXT_DIM);
}
