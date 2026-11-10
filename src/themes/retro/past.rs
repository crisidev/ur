//! Retro's screens of the past: today, and last night.

use core::fmt::Write as _;

use super::{BACKGROUND, draw_digit};
use crate::canvas::{Canvas, Color};
use crate::day::Day;
use crate::font;
use crate::themes::hhmm;
use crate::ui::Fits;

/// Today's charts: a column per hour from midnight, [`CHART_BAR`] wide and
/// [`CHART_PITCH`] apart, from [`CHART_LEFT`] (24 of them end at x = 75).
const CHART_LEFT: i32 = 4;
const CHART_PITCH: i32 = 3;
const CHART_BAR: i32 = 2;

/// Today's steps and heart rate: for each, a figure for the day, then a
/// chart hour by hour since midnight, over a faint baseline that shows
/// how much of the day is left.
pub(super) fn render_today(canvas: &mut Canvas, day: Option<&Day>, steps: Option<u32>) {
    canvas.clear(BACKGROUND);
    canvas.text(4, 3, "TODAY", Color::ACCENT);
    // Figures end at x = 74, leaving the right edge to the hint.
    let day = day.copied().unwrap_or_default();
    let column = |hour: usize| CHART_LEFT + CHART_PITCH * hour as i32;

    // Steps: the step counter's total (which, unlike the history, counts
    // steps from before the phone set the clock), then the hours, scaled
    // to the busiest, but to at least 500 steps, so a quiet day looks it.
    const STEPS_BASE: i32 = 30;
    const STEPS_HEIGHT: u32 = 10;
    canvas.sprite(4, 11, font::FOOT_W, &font::FOOT, 1, Color::TEXT_DIM);
    canvas.text(10, 12, "STEPS", Color::TEXT_DIM);
    let mut text = heapless::String::<10>::new();
    match steps {
        Some(steps) => write!(text, "{steps}").fits(),
        None => text.push_str("--").fits(),
    }
    canvas.text_right(75, 12, &text, Color::TEXT);
    let most = u32::from(day.most_steps()).max(500);
    for (i, hour) in day.hours.iter().enumerate() {
        let height = (u32::from(hour.steps) * STEPS_HEIGHT).div_ceil(most) as i32;
        canvas.fill_rect(
            column(i),
            STEPS_BASE - height,
            CHART_BAR,
            height,
            Color::ACCENT,
        );
        canvas.fill_rect(column(i), STEPS_BASE, CHART_BAR, 1, Color::TRACK);
    }

    // The heart rate: resting (see `Day::resting`), then each hour's
    // range, from its lowest reading to its highest, on a scale from the
    // day's lowest to its highest in whole tens, but at least 40 BPM
    // tall, so a calm day doesn't look like a wild one.
    // TODO: the scale's ends as figures, if the shape alone isn't enough.
    const HEART_BASE: i32 = 64;
    const HEART_HEIGHT: i32 = 20;
    canvas.sprite(4, 35, font::HEART_W, &font::HEART, 1, Color::HEART);
    canvas.sprite(
        4,
        35,
        font::HEART_W,
        &font::HEART_SHINE,
        1,
        Color::HEART_SHINE,
    );
    canvas.text(13, 36, "RESTING", Color::TEXT_DIM);
    let mut text = heapless::String::<3>::new();
    match day.resting() {
        Some(bpm) => write!(text, "{bpm}").fits(),
        None => text.push_str("--").fits(),
    }
    canvas.text_right(75, 36, &text, Color::TEXT);
    if let Some((low, high)) = day.heart_range() {
        let floor = i32::from(low) / 10 * 10;
        let ceiling = ((i32::from(high) + 9) / 10 * 10).max(floor + 40);
        // The top of the row `bpm` is drawn in.
        let y = |bpm: u8| {
            HEART_BASE - 1 - (i32::from(bpm) - floor) * (HEART_HEIGHT - 1) / (ceiling - floor)
        };
        for (i, hour) in day.hours.iter().enumerate() {
            if hour.heart_max > 0 {
                let top = y(hour.heart_max);
                let height = y(hour.heart_min) - top + 1;
                canvas.fill_rect(column(i), top, CHART_BAR, height, Color::HEART);
            }
        }
    }
    for i in 0..day.hours.len() {
        canvas.fill_rect(column(i), HEART_BASE, CHART_BAR, 1, Color::TRACK);
    }
    for hour in [0, 6, 12, 18] {
        let mut label = heapless::String::<2>::new();
        write!(label, "{hour}").fits();
        canvas.text(column(hour), HEART_BASE + 4, &label, Color::TEXT_FAINT);
    }
}

/// The last night's (or nap's) sleep: how long, big, from when to when,
/// and in which stages.
pub(super) fn render_last_night(canvas: &mut Canvas, sleep: Option<crate::activity::Sleep>) {
    canvas.clear(BACKGROUND);
    canvas.text(4, 3, "LAST NIGHT", Color::ACCENT);
    let Some(sleep) = sleep else {
        canvas.text_centered(40, 36, "NO SLEEP YET", Color::TEXT_FAINT);
        return;
    };

    // Time asleep (not counting time lying awake) as H:MM, in the clock's
    // 3× digits, laid out as its are (2 px between digits, 7 px for the
    // colon), and centred.
    const TOP: i32 = 14;
    const SCALE: i32 = 3;
    const DIGIT: i32 = font::DIGIT_W as i32 * SCALE;
    let (hours, minutes) = (sleep.asleep_minutes / 60, sleep.asleep_minutes % 60);
    let mut digits = heapless::Vec::<u16, 4>::new();
    if hours >= 10 {
        let _ = digits.push(hours / 10 % 10);
    }
    let _ = digits.push(hours % 10);
    let _ = digits.push(minutes / 10);
    let _ = digits.push(minutes % 10);
    let width = DIGIT * digits.len() as i32 + 2 * (digits.len() as i32 - 2) + 7;
    let mut x = 40 - width / 2;
    let colon_after = digits.len() - 2;
    for (i, &digit) in digits.iter().enumerate() {
        if i == colon_after {
            for row in [2, 4] {
                let y = TOP + row * SCALE;
                canvas.fill_rect(x + 1, y + 1, 3, 3, Color::SHADOW);
                canvas.fill_rect(x, y, 3, 3, Color::ACCENT);
            }
            x += 3 + 2;
        }
        draw_digit(canvas, x, TOP, u64::from(digit), SCALE, Color::TEXT);
        x += DIGIT + 2;
    }
    canvas.text_centered(40, 40, "ASLEEP", Color::TEXT_DIM);

    // From falling asleep to waking, in local time (as the offset is now).
    if let Some(offset) = crate::clock::utc_offset_minutes() {
        let local = |utc_minute: i64| hhmm((utc_minute + i64::from(offset)).rem_euclid(24 * 60));
        let mut window = heapless::String::<16>::new();
        write!(
            window,
            "{} - {}",
            local(sleep.start_utc_minute),
            local(sleep.end_utc_minute)
        )
        .fits();
        canvas.text_centered(40, 50, &window, Color::TEXT);
    }

    // The stages (see `crate::stages`): their shares of the time asleep as
    // one bar, and each one's time below its name, in that stage's colour
    // (roles of their own: the general ones collide in some themes).
    // The stages are parts of the time asleep (`stages::Stages::asleep`
    // is their sum), so the first two's shares of the bar leave the third
    // its own; were they more, the bar would run off the screen.
    debug_assert!(
        u32::from(sleep.deep_minutes) + u32::from(sleep.rem_minutes)
            <= u32::from(sleep.asleep_minutes),
        "more deep and REM sleep than sleep"
    );
    let light = sleep
        .asleep_minutes
        .saturating_sub(sleep.deep_minutes + sleep.rem_minutes);
    let stages = [
        ("DEEP", sleep.deep_minutes, Color::DEEP),
        ("REM", sleep.rem_minutes, Color::REM),
        ("LIGHT", light, Color::GREY),
    ];
    const BAR: i32 = 72;
    let total = i32::from(sleep.asleep_minutes.max(1));
    let mut x = 4;
    for (i, &(name, minutes, color)) in stages.iter().enumerate() {
        // The last takes what's left, so rounding can't leave a gap.
        let width = if i + 1 == stages.len() {
            4 + BAR - x
        } else {
            i32::from(minutes) * BAR / total
        };
        canvas.fill_rect(x, 60, width, 4, color);
        x += width;
        let center = 16 + 24 * i as i32;
        canvas.text_centered(center, 67, name, color);
        canvas.text_centered(center, 74, &hhmm(i64::from(minutes)), Color::TEXT);
    }
}
