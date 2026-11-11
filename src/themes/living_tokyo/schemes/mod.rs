//! Living Tokyo's colour schemes: the [`LivingTokyo`] type, its schemes
//! (statics of it, each with a palette of its own), and the dim look's
//! palette. All for the night: a city at night is a dark theme. A build
//! of Living Tokyo alone shows them by day too; built beside a theme with
//! day schemes (Retro, with its TOKYO DAY), it shows that theme's by day.
//!
//! Pure data, needing only `crate::palette`: `host-tests/living_tokyo`
//! includes this file to check the schemes' colours. The drawing is
//! `super`'s.
//!
//! Each scheme's name is what the settings' THEME row shows, on its own,
//! beside Retro's (whose TOKYO NIGHT and TOKYO STORM have the same
//! colours, on Retro's screens), and its ID in flash
//! ([`crate::palette::id`]), so a name is for good.

use crate::palette::{Color, Palette, palette};

/// Living Tokyo: a city at night on the clock face, in Tokyo Night's
/// colours (folke's tokyonight.nvim).
///
/// Only a palette: every scheme is drawn alike. `repr(transparent)`, so
/// a scheme's static is its palette, where `scripts/screenshot.py
/// --theme` looks for it.
#[repr(transparent)]
pub struct LivingTokyo {
    /// Its colours; every scheme's dim look is [`GLANCE`].
    pub(super) palette: Palette,
}

/// The schemes for darkness, in the order THEME steps through them:
/// LIVING TOKYO first, which a build of Living Tokyo alone opens on.
pub const NIGHT: [&LivingTokyo; 2] = [&LIVING_TOKYO, &LIVING_STORM];

/// The schemes for daylight: none. (Tokyo Night's Day is Retro's TOKYO
/// DAY instead, Retro's screens in its colours: the city is the night's.)
pub const DAY: [&LivingTokyo; 0] = [];

// The Tokyo Night palettes. Each role takes the palette's colour for the
// same job as on Retro's screens, which Living Tokyo's are copies of: the
// accent its blue, labels fg_dark, and hints the muted dark5 rather than
// comment, which is too faint for text on the night backgrounds. The
// heart's highlight (one pixel on the heart) is the yellow, and deep and
// REM sleep are the cyan and the magenta, so that the clock face's lit
// windows (see `super::clock`) are the palette's yellow, cyan and magenta.
//
// Statics, not consts: the canvas tells palettes apart by address.

/// Tokyo Night's night: blue-white on near-black indigo.
#[rustfmt::skip]
pub static LIVING_TOKYO: LivingTokyo = LivingTokyo { palette: palette("LIVING TOKYO", [
    0x1a1b26, 0x16161e, 0xc0caf5, 0xa9b1d6, 0x737aa2, 0x292e42, 0x7aa2f7,
    0x7aa2f7, 0x1a1b26, 0xf7768e, 0xe0af68, 0x9ece6a, 0xf7768e, 0x7dcfff,
    0xbb9af7, 0xa9b1d6,
]) };

/// Tokyo Night's storm: the same, on a lighter, bluer slate.
#[rustfmt::skip]
pub static LIVING_STORM: LivingTokyo = LivingTokyo { palette: palette("LIVING STORM", [
    0x24283b, 0x1f2335, 0xc0caf5, 0xa9b1d6, 0x737aa2, 0x3b4261, 0x7aa2f7,
    0x7aa2f7, 0x24283b, 0xf7768e, 0xe0af68, 0x9ece6a, 0xf7768e, 0x7dcfff,
    0xbb9af7, 0xa9b1d6,
]) };

/// The dim look, for every scheme: Retro's, black, white and red, with
/// its grey (light sleep, the track) white half-toned with black. Only
/// idle mode's eight colours (see `Theme::dim`), which the assertion
/// below checks as it's built.
#[rustfmt::skip]
pub static GLANCE: Palette = palette("GLANCE", [
    0x000000, 0x000000, 0xffffff, 0xff0000, 0xff0000, 0xffffff, 0xff0000,
    0x000000, 0x000000, 0xff0000, 0xffffff, 0xffffff, 0xff0000, 0xffffff,
    0xff0000, 0xffffff,
])
.halftone(1 << Color::GREY.0 | 1 << Color::TRACK.0);

const _: () = assert!(GLANCE.idle_mode_safe(), "GLANCE: idle mode's colours only");
