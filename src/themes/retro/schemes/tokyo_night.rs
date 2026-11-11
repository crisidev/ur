//! Tokyo Night's palettes (folke's tokyonight.nvim) as Retro's schemes:
//! Night and Storm for the night, Day for the day. Retro's screens, in
//! Tokyo Night's colours.
//!
//! Each role takes the palette's colour for the same job: the accent its
//! blue, labels fg_dark, and hints the muted dark5 rather than comment,
//! which is too faint for text on the night backgrounds. The heart's
//! highlight (one pixel on the heart) is the yellow, and deep and REM
//! sleep are the cyan and the magenta.

use super::Retro;
use crate::palette::palette;

/// Tokyo Night's night: blue-white on near-black indigo.
#[rustfmt::skip]
pub static TOKYO_NIGHT: Retro = Retro { palette: palette("TOKYO NIGHT", [
    0x1a1b26, 0x16161e, 0xc0caf5, 0xa9b1d6, 0x737aa2, 0x292e42, 0x7aa2f7,
    0x7aa2f7, 0x1a1b26, 0xf7768e, 0xe0af68, 0x9ece6a, 0xf7768e, 0x7dcfff,
    0xbb9af7, 0xa9b1d6,
]) };

/// Tokyo Night's storm: the same, on a lighter, bluer slate.
#[rustfmt::skip]
pub static TOKYO_STORM: Retro = Retro { palette: palette("TOKYO STORM", [
    0x24283b, 0x1f2335, 0xc0caf5, 0xa9b1d6, 0x737aa2, 0x3b4261, 0x7aa2f7,
    0x7aa2f7, 0x24283b, 0xf7768e, 0xe0af68, 0x9ece6a, 0xf7768e, 0x7dcfff,
    0xbb9af7, 0xa9b1d6,
]) };

/// Tokyo Night's day: indigo ink on pale grey-blue.
#[rustfmt::skip]
pub static TOKYO_DAY: Retro = Retro { palette: palette("TOKYO DAY", [
    0xe1e2e7, 0xc4c8da, 0x3760bf, 0x6172b0, 0x848cb5, 0xa1a6c5, 0x2e7de9,
    0x2e7de9, 0xe1e2e7, 0xf52a65, 0xb15c00, 0x587539, 0xf52a65, 0x3760bf,
    0x9854f1, 0x6172b0,
]) };
