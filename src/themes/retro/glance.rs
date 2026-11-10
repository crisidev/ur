//! Retro's glance: what a look at the wrist shows, the time alone, or the
//! next turn while the phone navigates, and, just after a media gesture,
//! what it did.

use super::{BACKGROUND, action_icon, draw_digit};
use crate::canvas::{Canvas, Color};
use crate::font;
use crate::route::Turn;
use crate::themes::Glance;
use crate::themes::helpers::hhmm;
use crate::ui::Status;

/// A glance: the time alone, or the next turn while the phone navigates.
pub(super) fn render_glance(canvas: &mut Canvas, glance: &Glance) {
    match glance.route {
        Some(step) => render_route_glance(canvas, glance.status, step),
        None => render_time_glance(canvas, glance.status),
    }
    // Just after a media gesture, what it did, in the top right corner,
    // which both layouts leave free.
    if let Some(action) = glance.media {
        let icon = action_icon(action);
        canvas.sprite(65, 3, font::MEDIA_HINT_W, icon, 2, Color::TEXT);
    }
}

/// The glance: the hours above the minutes, as big as fits with room
/// around them, and nothing else.
///
/// The sizes follow from three rules. The digits scale by whole numbers
/// (they're pixel art). The block keeps a margin of about a tenth of the
/// screen (8 px), as one big focal element needs room to read as placed
/// rather than spilling over. And by proximity, the gap between the
/// hours' digits (one glyph cell) is half the gap between hours and
/// minutes (two cells), so they read as two numbers rather than a grid of
/// four digits. 5× (25×35 digits) can't keep the margin; 4× (20×28) can:
/// a 45×65 block, with 7–8 px above and below.
fn render_time_glance(canvas: &mut Canvas, status: &Status) {
    canvas.clear(BACKGROUND);
    let Some(t) = &status.local_time else {
        return;
    };
    const SCALE: i32 = 4;
    const WIDTH: i32 = font::DIGIT_W as i32 * SCALE;
    const HEIGHT: i32 = 7 * SCALE;
    const GAP_X: i32 = SCALE;
    const GAP_Y: i32 = 2 * SCALE;
    // Centred, counting the 1 px shadow.
    const LEFT: i32 = (80 - (2 * WIDTH + GAP_X + 1)) / 2;
    const TOP: i32 = (80 - (2 * HEIGHT + GAP_Y + 1)) / 2;
    for (row, value) in [t.hour, t.minute].into_iter().enumerate() {
        let y = TOP + row as i32 * (HEIGHT + GAP_Y);
        let value = u64::from(value);
        draw_digit(canvas, LEFT, y, value / 10, SCALE, Color::TEXT);
        draw_digit(
            canvas,
            LEFT + WIDTH + GAP_X,
            y,
            value % 10,
            SCALE,
            Color::TEXT,
        );
    }
}

/// The glance while the phone navigates: the next turn takes the middle of
/// the screen, as an arrow with the distance under it, then the street
/// (two lines of the small font at most, as Gadgetbridge gives it). The
/// time stays, smaller, at the top: a glance is still for the time too,
/// but on the way somewhere, the turn is what a look at the wrist is for.
///
/// From the top: the time (2×, 10 px), the arrow (16×16 at 2×), the
/// distance (2×), and the street (1×), with 4 px between them; it just
/// fits, with the street's second line ending a pixel above the bottom.
fn render_route_glance(canvas: &mut Canvas, status: &Status, step: &crate::route::Step) {
    canvas.clear(BACKGROUND);
    const TIME_Y: i32 = 3;
    const ARROW_Y: i32 = 17;
    const DISTANCE_Y: i32 = ARROW_Y + 32 + 4;
    const STREET_Y: i32 = DISTANCE_Y + 10 + 4;
    let centred = |canvas: &mut Canvas, y: i32, text: &str, scale: i32, color: Color| {
        let width = font::small_text_width(text) * scale;
        canvas.text_scaled(40 - width / 2, y, text, color, scale);
    };
    if let Some(t) = &status.local_time {
        let time = hhmm(i64::from(t.hour) * 60 + i64::from(t.minute));
        centred(canvas, TIME_Y, &time, 2, Color::TEXT_DIM);
    }

    let (arrow, mirror) = match step.turn {
        Turn::Straight => (&font::TURN_STRAIGHT, false),
        Turn::Right => (&font::TURN_RIGHT, false),
        Turn::Left => (&font::TURN_RIGHT, true),
        Turn::SlightRight | Turn::KeepRight => (&font::TURN_SLIGHT_RIGHT, false),
        Turn::SlightLeft | Turn::KeepLeft => (&font::TURN_SLIGHT_RIGHT, true),
        Turn::SharpRight => (&font::TURN_SHARP_RIGHT, false),
        Turn::SharpLeft => (&font::TURN_SHARP_RIGHT, true),
        // Gadgetbridge doesn't know which side the traffic keeps to, so a
        // U-turn at a roundabout is drawn as a plain one, going round left.
        Turn::UturnLeft | Turn::RoundaboutUturn => (&font::TURN_UTURN_LEFT, false),
        Turn::UturnRight => (&font::TURN_UTURN_LEFT, true),
        Turn::RoundaboutRight => (&font::TURN_ROUNDABOUT_RIGHT, false),
        Turn::RoundaboutLeft => (&font::TURN_ROUNDABOUT_RIGHT, true),
        Turn::RoundaboutStraight => (&font::TURN_ROUNDABOUT_STRAIGHT, false),
        Turn::Finish => (&font::TURN_FINISH, false),
        Turn::OffRoute => (&font::TURN_OFF_ROUTE, false),
        Turn::Unknown => (&font::TURN_UNKNOWN, false),
    };
    let arrow_x = 40 - font::TURN_W as i32;
    canvas.sprite16(arrow_x, ARROW_Y, arrow, mirror, 2, Color::TEXT);

    // "1,234 km" fits at 2× (62 px); the 1× fallback is for safety.
    let scale = if font::small_text_width(&step.distance) * 2 <= 76 {
        2
    } else {
        1
    };
    centred(canvas, DISTANCE_Y, &step.distance, scale, Color::TEXT);

    let mut lines = [font::Line::default(); 2];
    let street = step.street.trim();
    match font::fill(street, font::Face::Small, 76, &mut lines) {
        count @ 0..=2 => {
            for (i, &line) in lines[..count].iter().enumerate() {
                let width = font::small_text_width(line.of(street))
                    + font::small_text_width(line.tail.as_str())
                    + if line.tail == font::Tail::None { 0 } else { 1 };
                let y = STREET_Y + i as i32 * 7;
                canvas.line(
                    40 - width / 2,
                    y,
                    street,
                    line,
                    font::Face::Small,
                    Color::TEXT_DIM,
                );
            }
        }
        // Too long even for two lines (one very long word): its start.
        _ => canvas.text_cut(2, STREET_Y, &step.street, Color::TEXT_DIM),
    }
}
