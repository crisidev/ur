//! Living Tokyo's Air screen: a nearby room sensor's reading.

use core::fmt::Write as _;

use embassy_time::Instant;

use super::{BACKGROUND, draw_digit};
use crate::aranet::{self, Band, Sighting};
use crate::canvas::{Canvas, Color};
use crate::font;
use crate::themes::helpers::tenths;
use crate::ui::Fits;

// Canvas layout (80×80), y ranges inclusive:
//
//    3..7    AIR (left); how old the reading is (right)
//   13..33   CO₂ in ppm, in the time's digits at 3×, centred
//   38..42   "PPM CO2" (left); the band, GOOD/FAIR/POOR (right)
//   46..48   three segments, the band's lit: it reads without colour
//   54..63   temperature and humidity at 2×
//   69..73   pressure (left); the sensor's battery (right)
//
// A sensor without CO₂ (a BTHome thermometer) has the temperature big in
// its place, "°C" for its label, and no band; then the humidity at 2×
// (right), labelled (left).

/// A nearby room sensor's latest reading (see [`crate::aranet`]): from an
/// Aranet4, the CO₂ big, since that's what the watch can't otherwise know
/// and what to act on (open a window), then what else it measures. From a
/// BTHome thermometer, the temperature big (to the tenth, as on the clock
/// face), and whatever else it sends.
///
/// Not on the clock face: CO₂ changes over tens of minutes, and its number
/// would crowd a face that's read at a glance, many times a day. The
/// temperature there comes from the same reading (see
/// `crate::temperature`).
///
/// The readings are as old as the last scan, at most a quarter of an hour
/// with a sensor around (opening this screen scans again if the last is a
/// minute old), so the age is always shown; past [`aranet::STALE`], the
/// numbers go faint.
pub(super) fn render_air(canvas: &mut Canvas, sighting: &Sighting, now: Instant) {
    canvas.clear(BACKGROUND);
    canvas.text(4, 3, "AIR", Color::ACCENT);
    let reading = &sighting.reading;
    let age = sighting.age(now.as_millis());
    let mut text = heapless::String::<12>::new();
    match age {
        0..60 => text.push_str("NOW").fits(),
        60..3600 => write!(text, "{} MIN", age / 60).fits(),
        _ => write!(text, "{} H", age / 3600).fits(),
    }
    let stale = age >= aranet::STALE;
    canvas.text_right(76, 3, &text, Color::TEXT_DIM);
    let value = if stale {
        Color::TEXT_FAINT
    } else {
        Color::TEXT
    };

    // The temperature, to the tenth: "--.-" if invalid. Within ±100 °C
    // (and a sauna's), so that big it still fits the width.
    let mut temperature = match reading.tenths() {
        Some(t) => tenths(t.clamp(-999, 1999)),
        None => {
            let mut text = heapless::String::new();
            text.push_str("--.-").fits();
            text
        }
    };

    // The big number: CO₂ when the sensor measures it (dashes while an
    // Aranet4 calibrates), the temperature from a thermometer.
    const BIG_Y: i32 = 13;
    let mut text = heapless::String::<8>::new();
    let thermometer = reading.co2.is_none() && reading.kind != aranet::Kind::Aranet4;
    if thermometer {
        draw_big_number(canvas, BIG_Y, &temperature, value);
        canvas.text(4, 38, "°C", Color::TEXT_DIM);
    } else {
        match reading.co2 {
            Some(ppm) => {
                write!(text, "{}", ppm.min(9999)).fits();
                draw_big_number(canvas, BIG_Y, &text, value);
            }
            // A digit's width each.
            None => {
                for x in [6, 23, 40, 57] {
                    canvas.fill_rect(x, BIG_Y + 9, 15, 3, value);
                }
            }
        }
        canvas.text(4, 38, "PPM CO2", Color::TEXT_DIM);
    }
    // The band: the word, and which third of the bar is lit, so it reads
    // in every theme, even those of one hue.
    if let Some(band) = reading.band {
        let color = match band {
            Band::Good => Color::GOOD,
            Band::Fair => Color::TEXT,
            Band::Poor => Color::BAD,
        };
        canvas.text_right(76, 38, band.word(), color);
        for (i, segment) in [Band::Good, Band::Fair, Band::Poor].into_iter().enumerate() {
            let lit = if segment == band { color } else { Color::TRACK };
            canvas.fill_rect(4 + 25 * i as i32, 46, 22, 3, lit);
        }
    }

    // Temperature (unless it's big already) and humidity, at 2×.
    if thermometer {
        if reading.humidity.is_some() {
            canvas.text(4, 57, "HUMIDITY", Color::TEXT_DIM);
        }
    } else {
        temperature.push('°').fits();
        canvas.text_scaled(4, 54, &temperature, value, 2);
    }
    if let Some(humidity) = reading.humidity {
        let mut text = heapless::String::<4>::new();
        write!(text, "{}%", humidity.min(100)).fits();
        canvas.text_scaled(76 - 2 * font::small_text_width(&text), 54, &text, value, 2);
    }

    // Pressure, and the sensor's own battery, red when it wants changing:
    // each if the sensor sends it.
    let mut text = heapless::String::<10>::new();
    if let Some(pressure) = reading.pressure {
        // Widened: a BTHome sensor's pressure is clamped to `u16::MAX`
        // tenths (see `bthome_sensor`), and adding 5 to that overflowed.
        write!(text, "{} HPA", (u32::from(pressure) + 5) / 10).fits();
    }
    canvas.text(4, 69, &text, Color::TEXT_DIM);
    if let Some(battery) = reading.battery {
        let mut text = heapless::String::<8>::new();
        write!(text, "BAT {}%", battery.min(100)).fits();
        let color = if battery <= 15 {
            Color::BAD
        } else {
            Color::TEXT_DIM
        };
        canvas.text_right(76, 69, &text, color);
    }
}

/// `text` (digits, '.', '-') in the time's digits at 3×, centred, with
/// its top at `y`: the point is the glyphs' bottom row, a dash their
/// middle one (three of the digits' pixels each).
fn draw_big_number(canvas: &mut Canvas, y: i32, text: &str, color: Color) {
    let advance = |c: char| match c {
        '.' => 5,
        '-' => 11,
        _ => 17,
    };
    let width: i32 = text.chars().map(advance).sum::<i32>() - 2;
    let mut x = 40 - width / 2;
    for c in text.chars() {
        let mark = |canvas: &mut Canvas, dy: i32, w: i32| {
            canvas.fill_rect(x + 1, y + dy + 1, w, 3, Color::SHADOW);
            canvas.fill_rect(x, y + dy, w, 3, color);
        };
        match c {
            '.' => mark(canvas, 18, 3),
            '-' => mark(canvas, 9, 9),
            // Anything else isn't a digit either: drawn as one too big
            // (see `digit_glyph`), rather than underflowing.
            digit => {
                let value = digit.to_digit(10).unwrap_or(u32::MAX);
                draw_digit(canvas, x, y, u64::from(value), 3, color)
            }
        }
        x += advance(c);
    }
}
