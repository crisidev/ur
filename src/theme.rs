//! Colour themes.
//!
//! Screens never name concrete colours; they paint with *roles*
//! ([`Color::TEXT`], [`Color::ACCENT`], …), and the active [`Theme`] says what
//! each role looks like. The canvas stores role indices and only resolves them
//! to RGB when pushing pixels, so switching theme is a matter of swapping the
//! palette and repainting; nothing has to be re-rendered.
//!
//! The themes lean on classic constraints: few colours per theme, ramps of a
//! single hue, and dithering (the transition) to fake in-between shades.

/// A colour role: an index into the active theme's palette.
#[derive(Clone, Copy, PartialEq, Eq, defmt::Format)]
pub struct Color(pub(crate) u8);

impl Color {
    pub const BACKGROUND: Self = Self(0);
    /// Drop shadows under the big digits.
    pub const SHADOW: Self = Self(1);
    pub const TEXT: Self = Self(2);
    /// Labels.
    pub const TEXT_DIM: Self = Self(3);
    /// Hints and placeholders.
    pub const TEXT_FAINT: Self = Self(4);
    /// The unfilled part of bars and gauges.
    #[allow(dead_code)] // Unused since seconds became digits; timers will use it.
    pub const TRACK: Self = Self(5);
    /// Titles, the blinking colon, filled bars.
    pub const ACCENT: Self = Self(6);
    /// The two colours of the transition's checkerboard.
    pub const DITHER_A: Self = Self(7);
    pub const DITHER_B: Self = Self(8);
    pub const HEART: Self = Self(9);
    /// The heart's specular highlight pixel.
    pub const HEART_SHINE: Self = Self(10);
    /// Battery while charging.
    pub const GOOD: Self = Self(11);
    /// Battery when low.
    pub const BAD: Self = Self(12);

    const COUNT: usize = 13;
}

pub struct Theme {
    pub name: &'static str,
    /// Each role as the two bytes the panel expects: RGB565, big-endian.
    palette: [[u8; 2]; Color::COUNT],
}

impl Theme {
    pub fn bytes(&self, color: Color) -> [u8; 2] {
        self.palette[usize::from(color.0)]
    }
}

/// Builds a theme from `0xRRGGBB` values listed in [`Color`] role order.
const fn theme(name: &'static str, hex: [u32; Color::COUNT]) -> Theme {
    let mut palette = [[0; 2]; Color::COUNT];
    let mut i = 0;
    while i < Color::COUNT {
        // Keep the top 5/6/5 bits of each 8-bit channel.
        let (r, g, b) = (
            (hex[i] >> 19) & 0x1F,
            (hex[i] >> 10) & 0x3F,
            (hex[i] >> 3) & 0x1F,
        );
        let rgb565 = ((r << 11) | (g << 5) | b) as u16;
        palette[i] = rgb565.to_be_bytes();
        i += 1;
    }
    Theme { name, palette }
}

// These three survived trying six on the real panel (also tried: amber
// terminal, '80s home-computer blue, and the original navy/teal).
//
// TODO: persist the selected theme once settings storage exists.
// Kept unformatted so each palette stays a compact two-line block.
#[rustfmt::skip]
pub const THEMES: [Theme; 3] = [
    // Black cabinet, white-to-grey text, red shadows and accents.
    theme("ARCADE", [
        0x101010, 0x5a1010, 0xe8e8e8, 0xa8a8a8, 0x606060, 0x303030, 0xd82828,
        0xd82828, 0x101010, 0xd82828, 0xff9090, 0xffffff, 0xff5050,
    ]),
    // Four shades of olive green, dark on light, like an early handheld LCD.
    theme("HANDHELD", [
        0xc3cf8c, 0x8a9f3a, 0x1e2a14, 0x3c5428, 0x8a9f3a, 0x8a9f3a, 0x3c5428,
        0x1e2a14, 0x8a9f3a, 0x1e2a14, 0x8a9f3a, 0x3c5428, 0x1e2a14,
    ]),
    // Brown ink on cream, muted red: an old printed game manual.
    theme("PARCHMENT", [
        0xe8dcb8, 0xc0a878, 0x3a2414, 0x6a4a2a, 0xa08860, 0xd0c098, 0xa02820,
        0xa02820, 0xe8dcb8, 0xa02820, 0xe07060, 0x4a6a2a, 0xa02820,
    ]),
];
