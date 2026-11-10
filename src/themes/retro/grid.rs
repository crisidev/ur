//! Retro's button grid.

use core::fmt::Write as _;

use super::BACKGROUND;
use crate::buttons::{self, Press};
use crate::canvas::{Canvas, Color};
use crate::font;
use crate::themes::Grid;
use crate::ui::Fits;

// Nine buttons in a 3×3 grid (two layers), or six in two rows of three,
// unlabelled: each 22 px (~8 mm) square, 5 px apart, either way. Nine
// fill the screen; six are the top two rows of nine, moved down to the
// middle, rather than stretched to fill it: square, they look like the
// arcade buttons they're drawn as, and the same either way. A touch hits
// the screen in thirds across, and thirds or halves down (see
// `buttons::at`), each with a button in its middle (give or take a
// pixel), so the gaps between them, and with six, the space above and
// below, belong to the nearest. They stay clear of the edges' handles.
//
// Drawn as an old arcade cabinet's or console's buttons, in the theme's
// colours: a chunky keycap with a lit edge and a glint at its top left,
// standing on its own dark side, 4 px deep. Pressed, it sinks: the cap
// drops 3 px into its well, the side gone and the well's shadow above it.
//
//    raised             pressed
//    ┌────────┐         ┌────────┐
//    │▓▓▓▓▓▓▓▓│ edge    │████████│ the well's shadow
//    │▓ ▪     │ glint   │█       │
//    │▓  cap  │         │   cap  │
//    │████████│ side    │        │
//    └────────┘         └────────┘

/// How a grid button looks.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Key {
    Up,
    /// Held down: a sequence's first button, while its second waits; or
    /// a hold's, while the finger's still on it.
    Down,
    /// Held down and lit: what's waiting to see whether more follows.
    Lit,
    /// Just sent, with its code: lit in the "good" colour, or the "bad"
    /// one if it went nowhere.
    Sent(Press, bool),
}

/// The grid. `dark` picks the shading for a dark palette (or the dim look),
/// where edges catch light against black; on a light one, the ink is the
/// dark and the paper the light.
pub(super) fn render_buttons(canvas: &mut Canvas, grid: &Grid, dark: bool) {
    let Grid {
        layout, grid, sent, ..
    } = *grid;
    canvas.clear(BACKGROUND);
    let (first, second) = grid.waiting();
    let held = grid.held();
    // How many, and where the first row's top is: two rows (49 px)
    // centred in the 80.
    let (count, top) = match layout {
        buttons::Layout::Six => (6, 15),
        buttons::Layout::TwoLayers => (9, 2),
    };
    for button in 1..=count {
        let x = 2 + 27 * i32::from((button - 1) % 3);
        let y = top + 27 * i32::from((button - 1) / 3);
        let key = match (sent, first, second) {
            (Some((press, ok)), ..) if press.last() == button => Key::Sent(press, ok),
            // A hold, once what it sent has shown: pressed for as long as
            // the finger's on it, up again at the lift.
            _ if held == Some(button) => Key::Down,
            (_, _, Some(second)) if second == button => Key::Lit,
            (_, Some(first), Some(_)) if first == button => Key::Down,
            (_, Some(first), None) if first == button => Key::Lit,
            _ => Key::Up,
        };
        draw_key(canvas, x, y, key, dark);
    }
}

/// One button at (`x`, `y`), 22 px square.
fn draw_key(canvas: &mut Canvas, x: i32, y: i32, key: Key, dark: bool) {
    const SIZE: i32 = 22;
    /// How deep the side is, and so how far a pressed cap sinks (less a
    /// pixel, so it still sits in a well).
    const DEPTH: i32 = 4;
    // The outline, the cap, its lit edge, and the side (and the well's
    // shadow).
    let (outline, cap, light, side) = if dark {
        (
            Color::TEXT_FAINT,
            Color::TRACK,
            Color::TEXT_DIM,
            Color::SHADOW,
        )
    } else {
        (Color::TEXT, Color::TRACK, BACKGROUND, Color::TEXT)
    };
    let (face, down) = match key {
        Key::Up => (cap, false),
        Key::Down => (cap, true),
        Key::Lit => (Color::ACCENT, true),
        Key::Sent(_, true) => (Color::GOOD, true),
        Key::Sent(_, false) => (Color::BAD, true),
    };
    canvas.fill_rect(x, y, SIZE, SIZE, outline);
    // Rounded off, a pixel at each corner.
    for (cx, cy) in [
        (x, y),
        (x + SIZE - 1, y),
        (x, y + SIZE - 1),
        (x + SIZE - 1, y + SIZE - 1),
    ] {
        canvas.pixel(cx, cy, BACKGROUND);
    }
    let (inner, width) = (x + 1, SIZE - 2);
    // Where the cap's top is: sunk into its well when pressed.
    let top = if down { y + DEPTH } else { y + 1 };
    if down {
        canvas.fill_rect(inner, y + 1, width, DEPTH - 1, side);
        canvas.fill_rect(inner, top, width, y + SIZE - 1 - top, face);
        // The well's shadow falls on the cap's left edge too.
        canvas.fill_rect(inner, top, 1, y + SIZE - 1 - top, side);
    } else {
        let cap_height = SIZE - 2 - DEPTH;
        canvas.fill_rect(inner, top, width, cap_height, face);
        // The cap's lower lip, then its side.
        canvas.fill_rect(inner, top + cap_height - 1, width, 1, outline);
        canvas.fill_rect(inner, top + cap_height, width, DEPTH, side);
        // Light from the top left: its edge, and a glint.
        canvas.fill_rect(inner, top, width - 1, 1, light);
        canvas.fill_rect(inner, top + 1, 1, cap_height - 2, light);
        canvas.fill_rect(x + 3, top + 2, 2, 1, light);
        canvas.pixel(x + 3, top + 3, light);
    }
    if let Key::Sent(press, _) = key {
        let mut code = heapless::String::<2>::new();
        write!(code, "{}", press.code).fits();
        let width = font::small_text_width(&code) * 2;
        canvas.text_scaled(x + 1 + (SIZE - width) / 2, top + 1, &code, BACKGROUND, 2);
        let how = match press.gesture {
            buttons::Gesture::Tap => "TAP",
            buttons::Gesture::DoubleTap => "2X",
            // Never shown: a release leaves the hold's on the screen.
            buttons::Gesture::Hold | buttons::Gesture::Release => "HOLD",
            buttons::Gesture::LongPress => "LONG",
        };
        canvas.text_centered(x + 1 + SIZE / 2, top + 12, how, BACKGROUND);
    }
}
