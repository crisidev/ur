//! Retro's clock face, and the battery it shows.
//
// Canvas layout (80×80), y ranges inclusive:
//
//    3..7    status bar: media, or no phone and time-sync state, then the
//            temperature (left); battery % and icon (right)
//   13..34   HH:MM in 5×7 digits at 3× (15×21 each), with a drop shadow
//   38..44   date, YYYY-MM-DD, under the hours (left); seconds, small and
//            in the accent colour, under the minutes (right)
//   56..62   heart rate (left); the next alarm or timer (right)
//   68..74   a foot, then ten blocks for the day's steps (x 10..58),
//            and their scale, ×1K or ×10K (from x = 61)
//
// Everything below the status bar lines up with the time's outer edges
// (x = 4 and 75). The top half is the time; the bottom half is the
// readings, packed into two rows, with the steps drawn as a share of the
// day's goal rather than a number: the exact count is on Today, one
// swipe right.

use core::fmt::Write as _;

use super::{BACKGROUND, action_icon, digit_glyph, draw_digit};
use crate::alarm;
use crate::battery::Battery;
use crate::canvas::{Canvas, Color};
use crate::font;
use crate::themes::helpers::{countdown, hhmm, iso_date, percent as percent_text};
use crate::themes::{Face, Media, STEP_GOAL};
use crate::ui::Fits;

pub(super) fn render_clock(canvas: &mut Canvas, face: &Face) {
    let Face {
        status,
        next_alarm,
        dim,
        ..
    } = *face;
    // The media slot's icon, while there's one (with a phone connected):
    // its width, rows, and top. The slot is there only while an icon is:
    // a fixed width (below) so the temperature doesn't shift as one icon
    // replaces another (a gesture's, then the note), but no empty gap
    // once the media is paused or stopped and the gesture's icon has
    // gone. The temperature moves over once then, which reads as the
    // icon going away.
    let media = face.media.map(|media| match media {
        Media::Did(action) => (font::MEDIA_HINT_W, &action_icon(action)[..], 3),
        Media::Playing => (font::NOTE_W, &font::NOTE[..], 2),
    });
    canvas.clear(BACKGROUND);
    let battery_left = render_battery(canvas, status.battery, status.uptime_secs);

    // Status bar, left: the media icon while a phone is connected (media
    // comes from the phone); without one, a faded Bluetooth rune struck
    // through in red, and "NO TIME" if we don't know the time (in which
    // case the clock shows time since boot). A connected phone is the
    // usual state, so it gets no icon: what's worth a look is its absence.
    //
    // "Connected" is the phone's link (`ble::CONNECTED`): a client that
    // never authenticated (see `Status::unpaired`), or a bonded phone
    // that's out of range, both get the struck-out rune, as neither
    // delivers anything.
    let temperature_x = if status.connected {
        // One width for every media icon (the widest, the note's), each
        // centred in it, so nothing after it moves as the icon changes.
        const MEDIA_SLOT: u32 = font::NOTE_W;
        match media {
            Some((width, rows, y)) => {
                let left = 3 + (MEDIA_SLOT.saturating_sub(width) / 2) as i32;
                canvas.sprite(left, y, width, rows, 1, Color::ACCENT);
                Some(3 + MEDIA_SLOT as i32 + 3)
            }
            None => Some(3),
        }
    } else {
        // The rune in the faintest ink, so it reads as "not there", and a
        // slash over it, a pixel past it at both ends, in the low
        // battery's colour: red, but for Terminal's near-white, its one
        // alarm colour. The slash is a diagonal beside the rune's own
        // diagonals; it only stands apart from them by its colour, so the
        // rune must be a different one. In the dim look, the faint ink is
        // red, like the slash, so the rune is grey there instead
        // (`Color::GREY`, a half-tone the dim look can show).
        // A 3×3 × after the rune said the same in the rune's own colour,
        // and a slash through a rune in full ink merged with it.
        let faded = if dim { Color::GREY } else { Color::TEXT_FAINT };
        canvas.sprite(4, 2, font::BLUETOOTH_W, &font::BLUETOOTH, 1, faded);
        for i in 0..7 {
            canvas.pixel(3 + i, 2 + i, Color::BAD);
        }
        let x = 3 + 7;
        if status.local_time.is_none() {
            // 2 px rather than the usual 3, so "NO TIME" (27 px) ends at
            // x = 38, clear of the battery's widest (charging at 100%: the
            // bolt starts at x = 43).
            let end = canvas.text(x + 2, 3, "NO TIME", Color::TEXT_FAINT);
            // Clear of the battery, as worked out above.
            debug_assert!(end <= battery_left, "NO TIME runs into the battery");
            None
        } else {
            Some(x + 3)
        }
    };

    // Then the temperature, packed against the left: a room sensor's (a
    // nearby Aranet4 or BTHome thermometer; see `crate::temperature`) to
    // the tenth, "21.4°", and the watch's own, or the estimate from it, to
    // the degree, "21°". The precision is the mark: a measurement has
    // tenths worth showing, and an estimate good to a few degrees
    // doesn't. (Tried before: a short line under a measured reading's
    // degree sign, in the labels' colour, and a little house before it,
    // which read as one more status icon.) The decimal point is a single
    // pixel with a column either side, rather than the font's ".", whose
    // glyph is three columns wide: "21.4" then reads as one number, and
    // it's 2 px narrower.
    //
    // The widest a room sensor can send (a BTHome thermometer's i16
    // hundredths, so at most "-327.7°", 25 px) ends at x = 37 after the
    // struck-out rune, and x = 36 after the widest media icon (the note,
    // 6 px); anything a room holds, down to a freezer's "-18.4°" (21 px),
    // ends at x = 33. The watch's own is within -55 °C (the die sensor's
    // -40 °C, less the estimate's largest correction) and 90 °C, so 15 px
    // ("-55°"). All clear of the battery's widest (charging at 100%: the
    // bolt starts at x = 43), so a reading never drops its tenths for
    // room. "NO TIME" and a temperature can't both fit beside that, so
    // until the phone sets the clock (just after a cold boot), "NO TIME"
    // wins.
    // TODO: °F, for those who'd want it.
    if let (Some(tenths), Some(x)) = (status.temperature, temperature_x) {
        // The whole degrees, then (from a room sensor) the point and the
        // tenth, then the degree sign: drawn in pieces, so the point can
        // be narrower than a glyph. `end` is each piece's x past its
        // spacing column. (The watch's own is a multiple of ten tenths:
        // see `temperature::shown`.)
        let magnitude = i32::from(tenths).abs();
        let sign = if tenths < 0 { "-" } else { "" };
        // At most "-3276", an i16's tenths in whole degrees.
        let mut text = heapless::String::<5>::new();
        write!(text, "{sign}{}", magnitude / 10).fits();
        let mut end = canvas.text(x, 3, &text, Color::TEXT);
        if status.temperature_from_sensor {
            // On the baseline, then a column's gap.
            canvas.pixel(end, 7, Color::TEXT);
            let digit = char::from(b'0' + (magnitude % 10) as u8);
            end = canvas.text(end + 2, 3, digit.encode_utf8(&mut [0; 4]), Color::TEXT);
        }
        let end = canvas.text(end, 3, "°", Color::TEXT);
        // Clear of the battery, as worked out above, whatever a sensor
        // sends. And the watch's own reading is in whole degrees, which
        // is why it's drawn without its tenth.
        debug_assert!(end <= battery_left, "the temperature runs into the battery");
        debug_assert!(status.temperature_from_sensor || tenths % 10 == 0);
    }

    let (hours, minutes, seconds) = match &status.local_time {
        Some(t) => (u64::from(t.hour), u64::from(t.minute), u64::from(t.second)),
        None => {
            let secs = status.uptime_secs;
            ((secs / 3600) % 24, (secs / 60) % 60, secs % 60)
        }
    };

    // HH:MM. Each digit cell is 15 px wide with 2 px gaps; the colon gets a
    // 3 px column. That's 71 px, leaving 4 px either side (+1 for shadow).
    const TIME_Y: i32 = 13;
    const TIME_SCALE: i32 = 3;
    let digits = [hours / 10, hours % 10, minutes / 10, minutes % 10];
    for (&digit, x) in digits.iter().zip([4, 21, 43, 60]) {
        draw_digit(canvas, x, TIME_Y, digit, TIME_SCALE, Color::TEXT);
    }
    // The colon blinks: a heartbeat that shows the watch is alive even when
    // the minutes aren't changing. Its dots line up with glyph rows 2 and 4.
    if seconds % 2 == 0 {
        for row in [2, 4] {
            let y = TIME_Y + row * TIME_SCALE;
            canvas.fill_rect(39, y + 1, 3, 3, Color::SHADOW);
            canvas.fill_rect(38, y, 3, 3, Color::ACCENT);
        }
    }

    // Under the time, a footnote line, lined up with its outer edges: the
    // date under the hours, and the seconds under the minutes. The seconds
    // are rarely what a look at the watch is for (the colon's blink already
    // says it's alive), so they're small, the time's own 5×7 glyphs at 1×
    // (without the shadow, which at 1× would only thicken them), and in the
    // accent colour, so they're still easy to find.
    const NOTE_Y: i32 = 39;
    let date = status
        .local_time
        .as_ref()
        .map(|t| iso_date(t.year, t.month, t.day));
    let date = date.as_deref().unwrap_or("----------");
    canvas.text(4, NOTE_Y, date, Color::TEXT_DIM);
    for (digit, x) in [seconds / 10, seconds % 10].into_iter().zip([64, 70]) {
        let glyph = digit_glyph(digit);
        canvas.sprite(x, NOTE_Y - 1, font::DIGIT_W, glyph, 1, Color::ACCENT);
    }

    // The readings, in one row: the heart rate from the left edge, and the
    // next alarm within a day up to the right one, each an icon, 2 px, and
    // its value, the icon a row above the value's capitals.
    const READING_Y: i32 = 57;
    let mut bpm = heapless::String::<3>::new();
    match status.heart_rate {
        Some(rate) => write!(bpm, "{rate}").fits(),
        None => bpm.push_str("--").fits(),
    }
    canvas.sprite(
        4,
        READING_Y - 1,
        font::HEART_W,
        &font::HEART,
        1,
        Color::HEART,
    );
    canvas.sprite(
        4,
        READING_Y - 1,
        font::HEART_W,
        &font::HEART_SHINE,
        1,
        Color::HEART_SHINE,
    );
    let bpm_end = canvas.text(4 + font::HEART_W as i32 + 2, READING_Y, &bpm, Color::TEXT);

    if let (Some((kind, at)), Some(now)) = (next_alarm, status.local_second)
        && at - now <= 24 * 60 * 60
    {
        // A timer counts down; anything else shows when it rings (a smart
        // alarm at its latest). A timer's "42 MIN" is written "42M" here:
        // the full form, after the widest heart rate ("188"), would leave
        // the two groups 3 px apart, reading as one.
        let left = (at - now).max(0);
        let time = match kind {
            alarm::Kind::Timer(_) if (10 * 60..60 * 60).contains(&left) => {
                let mut time = heapless::String::new();
                write!(time, "{}M", (left + 59) / 60).fits();
                time
            }
            alarm::Kind::Timer(_) => countdown(left),
            _ => {
                let mut time = heapless::String::new();
                time.push_str(&hhmm(at.div_euclid(60).rem_euclid(24 * 60)))
                    .fits();
                time
            }
        };
        let x = 76 - font::small_text_width(&time);
        canvas.text(x, READING_Y, &time, Color::TEXT);
        let (icon, width) = match kind {
            alarm::Kind::Timer(_) => (&font::HOURGLASS, font::HOURGLASS_W),
            _ => (&font::BELL, font::BELL_W),
        };
        let icon_x = x - width as i32 - 2;
        // The two readings share the row: the widest of each (a heart
        // rate of three digits; a time or a countdown of five characters)
        // leave a gap between them.
        debug_assert!(icon_x > bpm_end, "the next alarm runs into the heart rate");
        canvas.sprite(icon_x, READING_Y - 1, width, icon, 1, Color::ACCENT);
    }

    // The day's steps, as ten blocks, like an old game's health bar: "six
    // of ten" can be counted at a glance, where a number has to be read.
    // Up to [`STEP_GOAL`], each block is 1,000 steps; from the goal on,
    // the bar starts again at 10,000 a block, so a long day keeps
    // filling it, and the change of scale is what says the goal was
    // reached. The scale stands to the bar's right, "×1K" or "×10K".
    // Each block is a whole multiple (999 steps fill none; 19,999 one
    // block of 10,000), and from 100,000 the bar stays full. Before the
    // time is known there's no day to count, so it's an empty track, at
    // 1,000 a block.
    //
    // Foot, 2 px, the bar, 2 px, and the widest scale ("×10K", 15 px)
    // take exactly the row's 72 px (x = 4..75), with the blocks 4 px wide
    // and 1 px apart: 5 px wide, they'd leave the scale no room. The
    // scale starts where the bar ends, so it reads as the bar's, and the
    // bar never moves as it changes.
    const STEPS_Y: i32 = 69;
    const BLOCKS: u32 = 10;
    const BLOCK_W: i32 = 4;
    const BAR_X: i32 = 4 + font::FOOT_W as i32 + 2;
    const SCALE_X: i32 = BAR_X + BLOCKS as i32 * (BLOCK_W + 1) - 1 + 2;
    const _: () = assert!(STEP_GOAL == 10_000, "the scales say 1K and 10K");
    canvas.sprite(
        4,
        STEPS_Y - 1,
        font::FOOT_W,
        &font::FOOT,
        1,
        Color::TEXT_DIM,
    );
    let steps = status.steps.unwrap_or(0);
    // Past the goal the scale takes the accent, the filled blocks'
    // colour, since the change of scale is what says the goal was reached.
    let (per_block, scale, scale_color) = if steps < STEP_GOAL {
        (STEP_GOAL / BLOCKS, "×1K", Color::TEXT_DIM)
    } else {
        (STEP_GOAL, "×10K", Color::ACCENT)
    };
    let full = (steps / per_block).min(BLOCKS);
    for i in 0..BLOCKS {
        let color = if i < full {
            Color::ACCENT
        } else {
            Color::TRACK
        };
        let x = BAR_X + i as i32 * (BLOCK_W + 1);
        canvas.fill_rect(x, STEPS_Y, BLOCK_W, 5, color);
    }
    canvas.text(SCALE_X, STEPS_Y, scale, scale_color);
}

/// The battery: an icon filled to the charge, and the percentage. While
/// charging, a bolt goes before the percentage and the fill creeps up a
/// pixel a second from the charge to full (an old handheld's charging
/// animation), since the "good" colour alone barely differs from text in
/// some themes.
///
/// Returns its left edge, for the clock face to check that what it draws
/// from the left stops short of it.
fn render_battery(canvas: &mut Canvas, battery: &Battery, uptime_secs: u64) -> i32 {
    let percent = battery.percent();
    let color = if battery.charging {
        Color::GOOD
    } else if percent < 20 {
        Color::BAD
    } else {
        Color::TEXT
    };

    // 10×5 body with a 1×3 nub; 8×3 interior, filled in proportion.
    const X: i32 = 66;
    const Y: i32 = 3;
    canvas.fill_rect(X, Y, 10, 5, color);
    canvas.fill_rect(X + 10, Y + 1, 1, 3, color);
    canvas.fill_rect(X + 1, Y + 1, 8, 3, BACKGROUND);
    // Round up, so any charge at all shows at least one pixel.
    let mut fill = (percent * 8).div_ceil(100) as i32;
    if battery.charging {
        let steps = 8 - fill + 1;
        fill += (uptime_secs % steps.max(1) as u64) as i32;
    }
    canvas.fill_rect(X + 1, Y + 1, fill.min(8), 3, color);

    let text = percent_text(percent);
    let text_x = X - 2 - font::small_text_width(&text);
    canvas.text(text_x, Y, &text, color);
    if battery.charging {
        let x = text_x - 2 - font::BOLT_W as i32;
        canvas.sprite(x, Y, font::BOLT_W, &font::BOLT, 1, Color::HEART);
        return x;
    }
    text_x
}
