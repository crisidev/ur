//! Retro's colour schemes: the [`Retro`] type, its schemes (statics of
//! it, each with a palette of its own), and which side of the day each is
//! for.
//!
//! Pure data, needing only `crate::palette`: `host-tests/theme` includes
//! this file, with the modules beside it, to check the schemes' colours.
//! The drawing is `super`'s.
//!
//! A new scheme is a static of [`Retro`] here, or, for a set of them, in
//! a module of its own beside this file (`tokyo_night.rs`, `mod
//! tokyo_night;` here), listed in [`NIGHT`] or [`DAY`] after the four
//! below: nothing else to change, and no feature, as every one of
//! Retro's schemes is part of `theme-retro`. Its name is what the settings'
//! THEME row shows, on its own, and its ID in flash
//! ([`crate::palette::id`]), so a name says which it is ("TOKYO NIGHT",
//! not "NIGHT"), and is for good. See AUTHORING-THEMES.md.

use crate::palette::{Color, Palette, palette};

/// Retro: today's look of the watch, after old handhelds, arcade
/// cabinets and terminals: few colours a scheme, ramps of a single hue,
/// and dithering (the transition) to fake in-between shades.
///
/// Only a palette: every scheme is drawn alike. `repr(transparent)`, so
/// a scheme's static is its palette, where `scripts/screenshot.py
/// --theme` looks for it.
#[repr(transparent)]
pub struct Retro {
    /// Its colours; every scheme's dim look is [`GLANCE`].
    pub(super) palette: Palette,
}

/// The schemes for darkness, in the order THEME steps through them:
/// ARCADE first, and HANDHELD in [`DAY`], as a watch with nothing saved,
/// one upgraded from 0.10, and one whose saved scheme isn't built in all
/// show them (see `themes::migrate` and `themes::find`).
pub const NIGHT: [&Retro; 2] = [&ARCADE, &TERMINAL];

/// The schemes for daylight, likewise.
pub const DAY: [&Retro; 2] = [&HANDHELD, &PARCHMENT];

// Three schemes survived trying six on the real panel (also tried: amber
// terminal, '80s home-computer blue, and the original navy/teal); Terminal
// was designed afterwards as a second dark one. They are grouped by
// whether they suit darkness or daylight; the UI picks the group from
// sunrise/sunset and remembers a choice within each group.
//
// Statics, not consts: the canvas detects palette changes by address.

/// Black cabinet, white-to-grey text, red shadows and accents.
#[rustfmt::skip]
pub static ARCADE: Retro = Retro { palette: palette("ARCADE", [
    0x101010, 0x5a1010, 0xe8e8e8, 0xa8a8a8, 0x606060, 0x303030, 0xd82828,
    0xd82828, 0x101010, 0xd82828, 0xff9090, 0xffffff, 0xff5050, 0xffffff,
    0xd82828, 0xa8a8a8,
]) };

/// A hacker terminal: phosphor green on near-black, everything one hue at
/// different intensities. The drop shadow is a dim green "glow" rather than
/// a shadow, and only low battery breaks the monochrome (near-white). Sleep
/// stages are intensities too: deep the hottest (almost white), REM full
/// green, light dim.
#[rustfmt::skip]
pub static TERMINAL: Retro = Retro { palette: palette("TERMINAL", [
    0x020c04, 0x0b3d18, 0x4dff7a, 0x2aa84a, 0x146b2c, 0x0a2a12, 0xb8ffc8,
    0x2aa84a, 0x020c04, 0x4dff7a, 0xe0ffe8, 0xb8ffc8, 0xf0fff0, 0xe0ffe8,
    0x4dff7a, 0x2aa84a,
]) };

/// Four shades of olive green, dark on light, like an early handheld LCD,
/// plus one dash of red, like its power light: for the heart, low battery,
/// an update's progress, and REM sleep. Nothing else.
#[rustfmt::skip]
pub static HANDHELD: Retro = Retro { palette: palette("HANDHELD", [
    0xc3cf8c, 0x8a9f3a, 0x1e2a14, 0x3c5428, 0x8a9f3a, 0x8a9f3a, 0x3c5428,
    0x1e2a14, 0x8a9f3a, 0xb8342a, 0xe8836c, 0x3c5428, 0xb8342a, 0x1e2a14,
    0xb8342a, 0x3c5428,
]) };

/// Brown ink on cream, muted red: an old printed game manual.
#[rustfmt::skip]
pub static PARCHMENT: Retro = Retro { palette: palette("PARCHMENT", [
    0xe8dcb8, 0xc0a878, 0x3a2414, 0x6a4a2a, 0xa08860, 0xd0c098, 0xa02820,
    0xa02820, 0xe8dcb8, 0xa02820, 0xe07060, 0x4a6a2a, 0xa02820, 0x3a2414,
    0xa02820, 0x6a4a2a,
]) };

/// The dim look (see `ui`: a glance, a notification on a dark screen, the
/// quiet night), for every scheme: white on black, with red for what
/// comes second. Black pixels are the cheapest on this panel only in that
/// the backlight is dimmed for it; mostly, it's what reads best at a
/// glance.
///
/// Only black, white and red: the panel shows the dim look in idle mode
/// (see `display::Display::set_idle`), which keeps just the top bit of
/// each of red, green, and blue, so greys would come out white or black
/// (faint text black, i.e. invisible), bar the one half-tone below.
/// Pure red is one of the eight colours it can show, and the one that
/// keeps the night's dark adaptation, as astronomers' and cockpits'
/// lights do. So the text is white, and labels, titles, hints and
/// accents (`TEXT_DIM`, `TEXT_FAINT`, `ACCENT`) are red: a
/// notification's app and time, the clock's seconds and colon, the step
/// blocks. That's the one step of hierarchy the dark schemes get from
/// their greys and their accent. The heart and a low battery are red, as
/// in the other schemes; shadows black.
///
/// Last night's stages are red, white and grey: deep white, the heaviest
/// ink, as in every scheme; REM red, the scheme's red elsewhere too; and
/// light sleep, the lightest, grey ([`Color::GREY`]): white half-toned
/// with black, the panel's columns alternately lit (see
/// [`Palette::columns`]), as idle mode has no grey of its own. Light sleep
/// is usually the most of a night, so the bar is mostly a quiet grey,
/// with the deep and REM spells standing out of it.
///
/// The track of bars and gauges (the step blocks not yet filled) is that
/// grey too, white half-toned: black would leave the empty blocks
/// invisible, and red would make one more red thing competing for
/// attention.
/// TODO: idle mode has five more colours (green, blue, and their pairs),
/// if the dim look may ever have more colour than red.
#[rustfmt::skip]
pub static GLANCE: Palette = palette("GLANCE", [
    0x000000, 0x000000, 0xffffff, 0xff0000, 0xff0000, 0xffffff, 0xff0000,
    0x000000, 0x000000, 0xff0000, 0xffffff, 0xffffff, 0xff0000, 0xffffff,
    0xff0000, 0xffffff,
])
.halftone(1 << Color::GREY.0 | 1 << Color::TRACK.0);
