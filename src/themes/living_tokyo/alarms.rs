//! Living Tokyo's alarms: the alarms screen, the phone's, the editor, and
//! what rings (an alarm, a timer, "find my watch").

use core::fmt::Write as _;

use embassy_time::Duration;

use super::layout::{ALARMS_TOP, EDITOR_BUTTONS_Y, SMART_TOP, TIMERS_TOP};
use super::{BACKGROUND, draw_digit};
use crate::alarm;
use crate::canvas::{Canvas, Color};
use crate::font;
use crate::themes::Editor;
use crate::themes::helpers::{countdown, duration, hhmm};
use crate::ui::{Editing, Fits, Status};

/// Smart wake across the top (label and state, then both times), then the
/// two alarms and the two timers in pairs of cells, each cell a label, its
/// time, and its state.
pub(super) fn render_alarms(
    canvas: &mut Canvas,
    settings: &alarm::Settings,
    state: &alarm::State,
    status: &Status,
) {
    canvas.clear(BACKGROUND);
    canvas.text(4, 3, "ALARMS", Color::ACCENT);
    let switch = |on: bool| {
        if on {
            ("ON", Color::GOOD)
        } else {
            ("OFF", Color::TEXT_FAINT)
        }
    };

    let (text, color) = switch(settings.smart);
    canvas.text(4, SMART_TOP + 1, "SMART WAKE", Color::TEXT_DIM);
    canvas.text_right(77, SMART_TOP + 1, text, color);
    let mut times = heapless::String::<20>::new();
    write!(
        times,
        "{} / {}",
        hhmm(settings.smart_weekday.into()),
        hhmm(settings.smart_weekend.into())
    )
    .fits();
    canvas.text(4, SMART_TOP + 8, &times, Color::TEXT);

    // Hairlines between the bands, so the pairs read as pairs.
    for y in [ALARMS_TOP - 2, TIMERS_TOP - 2] {
        canvas.fill_rect(4, y, 72, 1, Color::TEXT_FAINT);
    }
    let cell = |canvas: &mut Canvas,
                column: usize,
                top: i32,
                label: &str,
                value: &str,
                status: (&str, Color)| {
        let x = 4 + 40 * column as i32;
        canvas.text(x, top + 1, label, Color::TEXT_DIM);
        canvas.text(x, top + 8, value, Color::TEXT);
        canvas.text(x, top + 15, status.0, status.1);
    };
    for i in 0..alarm::ALARMS {
        let time = hhmm(settings.at_time[i].into());
        cell(
            canvas,
            i,
            ALARMS_TOP,
            ["ALARM 1", "ALARM 2"][i],
            &time,
            switch(settings.at[i]),
        );
    }
    for t in 0..alarm::TIMERS {
        // While running: time left, ticking; otherwise the duration.
        let left = state.remaining(t, status.uptime_secs as i64);
        let (value, running) = match left {
            Some(left) => (countdown(left), ("RUNNING", Color::GOOD)),
            None => (duration(settings.timers[t]), ("", Color::TEXT_FAINT)),
        };
        cell(
            canvas,
            t,
            TIMERS_TOP,
            ["TIMER 1", "TIMER 2"][t],
            &value,
            running,
        );
    }
}

/// The phone's alarms, a row each: the time, the weekdays it rings on (or
/// "ONCE"), and whether it's on. Ten rows fit, as many as Gadgetbridge
/// sends.
pub(super) fn render_phone_alarms(canvas: &mut Canvas, phone: &alarm::Phone) {
    canvas.clear(BACKGROUND);
    canvas.text(4, 3, "PHONE ALARMS", Color::ACCENT);
    for (row, alarm) in phone.alarms().iter().enumerate() {
        let y = 11 + 7 * row as i32;
        let on = alarm.on();
        let text = if on { Color::TEXT } else { Color::TEXT_FAINT };
        canvas.text(4, y, &hhmm(alarm.time.into()), text);
        if alarm.days == 0 {
            canvas.text(26, y, "ONCE", text);
        } else {
            // Its weekdays bright, the others faint, so the row reads as
            // a little calendar.
            for (day, letter) in ["M", "T", "W", "T", "F", "S", "S"].into_iter().enumerate() {
                let color = if on && alarm.days >> day & 1 != 0 {
                    Color::TEXT
                } else {
                    Color::TEXT_FAINT
                };
                canvas.text(26 + 4 * day as i32, y, letter, color);
            }
        }
        let (state, color) = if on {
            ("ON", Color::GOOD)
        } else {
            ("OFF", Color::TEXT_FAINT)
        };
        canvas.text_right(77, y, state, color);
    }
}

/// One setting, big, with how to change it.
pub(super) fn render_editor(canvas: &mut Canvas, editor: &Editor) {
    let Editor {
        editing,
        settings,
        state,
        uptime_secs: uptime,
    } = *editor;
    canvas.clear(BACKGROUND);
    // A running timer shows what's left (as MM:SS under an hour); anything
    // else, the setting being edited (HH:MM).
    let left = match editing {
        Editing::Timer(t) => state.remaining(usize::from(t), uptime),
        _ => None,
    };
    let (title, value, on) = match editing {
        Editing::SmartWeekday => ("SMART: WEEKDAYS", settings.smart_weekday, settings.smart),
        Editing::SmartWeekend => ("SMART: WEEKENDS", settings.smart_weekend, settings.smart),
        Editing::At(i) => (
            ["ALARM 1", "ALARM 2"][usize::from(i)],
            settings.at_time[usize::from(i)],
            settings.at[usize::from(i)],
        ),
        Editing::Timer(t) => (
            ["TIMER 1", "TIMER 2"][usize::from(t)],
            settings.timers[usize::from(t)],
            left.is_some(),
        ),
    };
    canvas.text_centered(40, 4, title, Color::ACCENT);

    // Two pairs of the clock's 2× digits (10 px cells, 2 px gaps), the
    // colon between; for settings, hours on the left half and minutes on
    // the right, matching where to swipe.
    let (pair, labels, color) = match left {
        Some(left) if left < 60 * 60 => ((left / 60, left % 60), ("MIN", "SEC"), Color::ACCENT),
        // Two digits of hours: a timer is at most `alarm::MAX_TIMER`, but
        // should more ever be left, it reads 99:59 rather than overflowing.
        Some(left) => {
            let left = left.min(100 * 3600 - 1);
            (
                (left / 3600, left / 60 % 60),
                ("HOURS", "MINUTES"),
                Color::ACCENT,
            )
        }
        None => (
            (i64::from(value / 60), i64::from(value % 60)),
            ("HOURS", "MINUTES"),
            Color::TEXT,
        ),
    };
    let digits = [pair.0 / 10, pair.0 % 10, pair.1 / 10, pair.1 % 10];
    for (&digit, x) in digits.iter().zip([12, 24, 44, 56]) {
        draw_digit(canvas, x, 22, digit as u64, 2, color);
    }
    for y in [26, 32] {
        canvas.fill_rect(39, y, 2, 2, Color::ACCENT);
    }
    canvas.text_centered(20, 44, labels.0, Color::TEXT_FAINT);
    canvas.text_centered(60, 44, labels.1, Color::TEXT_FAINT);
    let hint = if left.is_some() {
        "LEFT"
    } else {
        "SWIPE UP/DOWN"
    };
    canvas.text_centered(40, 51, hint, Color::TEXT_FAINT);

    let (left, left_color) = match (editing, on) {
        (Editing::Timer(_), true) => ("STOP", Color::BAD),
        (Editing::Timer(_), false) => ("START", Color::GOOD),
        (_, true) => ("ON", Color::GOOD),
        (_, false) => ("OFF", Color::TEXT_FAINT),
    };
    let right = if editing == Editing::SmartWeekday {
        "NEXT"
    } else {
        "DONE"
    };
    canvas.fill_rect(2, EDITOR_BUTTONS_Y, 36, 18, left_color);
    canvas.text_centered(20, EDITOR_BUTTONS_Y + 7, left, BACKGROUND);
    canvas.fill_rect(42, EDITOR_BUTTONS_Y, 36, 18, Color::ACCENT);
    canvas.text_centered(60, EDITOR_BUTTONS_Y + 7, right, BACKGROUND);
}

/// The time, big, and how to stop it.
pub(super) fn render_ringing(canvas: &mut Canvas, kind: alarm::Kind, status: &Status) {
    canvas.clear(BACKGROUND);
    let title = match kind {
        alarm::Kind::Smart => "GOOD MORNING",
        alarm::Kind::At(_) | alarm::Kind::Phone(_) | alarm::Kind::Snooze => "ALARM",
        alarm::Kind::Timer(_) => "TIME'S UP",
    };
    canvas.text_centered(40, 6, title, Color::ACCENT);
    canvas.sprite(37, 15, font::BELL_W, &font::BELL, 1, Color::ACCENT);
    if let Some(second) = status.local_second {
        let time = second.div_euclid(60).rem_euclid(24 * 60);
        let digits = [time / 600, time / 60 % 10, time % 60 / 10, time % 10];
        for (&digit, x) in digits.iter().zip([12, 24, 44, 56]) {
            draw_digit(canvas, x, 28, digit as u64, 2, Color::TEXT);
        }
        for y in [32, 38] {
            canvas.fill_rect(39, y, 2, 2, Color::ACCENT);
        }
    }
    canvas.text_centered(40, 56, "TAP: SNOOZE", Color::TEXT_DIM);
    canvas.text_centered(40, 66, "BUTTON: STOP", Color::TEXT_DIM);
}

/// "Find my watch": HERE, big, with rings round it that spread a ring a
/// second, as a ping does, so the screen catches the eye from across a
/// room (or from under a cushion), and how to stop it. `elapsed` is how
/// long the phone has been looking.
pub(super) fn render_find(canvas: &mut Canvas, elapsed: Duration) {
    canvas.clear(BACKGROUND);
    canvas.text_centered(40, 6, "FIND MY WATCH", Color::ACCENT);
    // "HERE" at 3×: 45×15, at x 17..62, y 28..43.
    let (x, y, w, h) = (17, 28, 45, 15);
    canvas.text_scaled(x, y, "HERE", Color::TEXT, 3);
    // One to three rings, 4 px apart, outlined a pixel thick.
    let rings = elapsed.as_secs() % 3 + 1;
    for ring in 1..=rings as i32 {
        let gap = 4 * ring;
        let (left, top, width, height) = (x - gap, y - gap, w + 2 * gap, h + 2 * gap);
        canvas.fill_rect(left, top, width, 1, Color::ACCENT);
        canvas.fill_rect(left, top + height - 1, width, 1, Color::ACCENT);
        canvas.fill_rect(left, top, 1, height, Color::ACCENT);
        canvas.fill_rect(left + width - 1, top, 1, height, Color::ACCENT);
    }
    canvas.text_centered(40, 66, "TAP: FOUND IT", Color::TEXT_DIM);
}
