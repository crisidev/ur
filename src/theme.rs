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
    /// Deep sleep, on Last night: the heaviest ink (the most contrast with
    /// the background). REM is the theme's red, where it has one, and light
    /// sleep is [`Color::GREY`]. Roles of their own, since no three of
    /// the others differ in every theme; `host-tests/theme` checks these do.
    pub const DEEP: Self = Self(13);
    /// REM sleep, on Last night: see [`Color::DEEP`].
    pub const REM: Self = Self(14);
    /// A grey between the text and the background, even in the dim look,
    /// whose panel mode has no greys: there, it's white half-toned with
    /// black (see [`Theme::columns`]). Light sleep on Last night, and the
    /// missing phone's rune in the dim look. Elsewhere it's the labels'
    /// colour ([`Color::TEXT_DIM`]); a role of its own only because
    /// that's red in the dim look.
    pub const GREY: Self = Self(15);

    /// Also `ROLES` in `scripts/screenshot.py`.
    pub(crate) const COUNT: usize = 16;
}

// The canvas stores roles in 4 bits, two pixels to a byte, and that's the
// last of them: one more, and the canvas would have to grow by half.
const _: () = assert!(Color::COUNT <= 16, "the canvas holds 4-bit roles");

/// `repr(C)`, so `scripts/screenshot.py` can find the palette (and the
/// half-tones after it).
#[repr(C)]
pub struct Theme {
    pub name: &'static str,
    /// Each role as the two bytes the panel expects: RGB565, big-endian.
    palette: [[u8; 2]; Color::COUNT],
    /// The roles shown half-toned, a bit each (bit `n` for `Color(n)`):
    /// see [`Theme::columns`].
    halftone: u16,
}

impl Theme {
    pub fn bytes(&self, color: Color) -> [u8; 2] {
        self.palette[usize::from(color.0)]
    }

    /// A role's bytes for the panel's even and odd columns: its colour in
    /// both, unless the theme half-tones it, when the odd columns are the
    /// background. That's 1-pixel vertical stripes at the panel's own
    /// resolution, a third of a canvas pixel, too fine to see at ~0.1 mm:
    /// they read as a grey, where the panel can't show one (see
    /// [`GLANCE`]).
    ///
    /// Stripes rather than a checkerboard, for the panel's dot inversion
    /// (the polarity it drives each subpixel with alternates from one to
    /// the next, and every frame). A checkerboard of lit pixels puts every
    /// lit subpixel's polarity the same way in a frame, the pattern LCD
    /// makers use to show flicker up; at idle mode's ~8 Hz, it would
    /// flicker at 4 Hz. Columns balance within every row. And each canvas
    /// pixel's three panel rows are then the same, so the canvas still
    /// sends a row once and repeats it (see `canvas::stream`).
    pub fn columns(&self, color: Color) -> [[u8; 2]; 2] {
        let bytes = self.bytes(color);
        if self.halftone >> color.0 & 1 != 0 {
            [bytes, self.bytes(Color::BACKGROUND)]
        } else {
            [bytes, bytes]
        }
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
    Theme {
        name,
        palette,
        halftone: 0,
    }
}

// Three themes survived trying six on the real panel (also tried: amber
// terminal, '80s home-computer blue, and the original navy/teal); Terminal
// was designed afterwards as a second dark theme. They are
// grouped by whether they suit darkness or daylight; the UI picks the group
// from sunrise/sunset and remembers a choice within each group.
//
// Statics, not consts: the canvas detects theme changes by address.

/// Black cabinet, white-to-grey text, red shadows and accents.
#[rustfmt::skip]
pub static ARCADE: Theme = theme("ARCADE", [
    0x101010, 0x5a1010, 0xe8e8e8, 0xa8a8a8, 0x606060, 0x303030, 0xd82828,
    0xd82828, 0x101010, 0xd82828, 0xff9090, 0xffffff, 0xff5050, 0xffffff,
    0xd82828, 0xa8a8a8,
]);

/// A hacker terminal: phosphor green on near-black, everything one hue at
/// different intensities. The drop shadow is a dim green "glow" rather than
/// a shadow, and only low battery breaks the monochrome (near-white). Sleep
/// stages are intensities too: deep the hottest (almost white), REM full
/// green, light dim.
#[rustfmt::skip]
pub static TERMINAL: Theme = theme("TERMINAL", [
    0x020c04, 0x0b3d18, 0x4dff7a, 0x2aa84a, 0x146b2c, 0x0a2a12, 0xb8ffc8,
    0x2aa84a, 0x020c04, 0x4dff7a, 0xe0ffe8, 0xb8ffc8, 0xf0fff0, 0xe0ffe8,
    0x4dff7a, 0x2aa84a,
]);

/// Four shades of olive green, dark on light, like an early handheld LCD,
/// plus one dash of red, like its power light: for the heart, low battery,
/// an update's progress, and REM sleep. Nothing else.
#[rustfmt::skip]
pub static HANDHELD: Theme = theme("HANDHELD", [
    0xc3cf8c, 0x8a9f3a, 0x1e2a14, 0x3c5428, 0x8a9f3a, 0x8a9f3a, 0x3c5428,
    0x1e2a14, 0x8a9f3a, 0xb8342a, 0xe8836c, 0x3c5428, 0xb8342a, 0x1e2a14,
    0xb8342a, 0x3c5428,
]);

/// Brown ink on cream, muted red: an old printed game manual.
#[rustfmt::skip]
pub static PARCHMENT: Theme = theme("PARCHMENT", [
    0xe8dcb8, 0xc0a878, 0x3a2414, 0x6a4a2a, 0xa08860, 0xd0c098, 0xa02820,
    0xa02820, 0xe8dcb8, 0xa02820, 0xe07060, 0x4a6a2a, 0xa02820, 0x3a2414,
    0xa02820, 0x6a4a2a,
]);

/// The dim look (see `ui`: a glance, a notification on a dark screen, the
/// quiet night): white on black, whatever the theme, with red for what
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
/// blocks. That's the one step of hierarchy the dark themes get from
/// their greys and their accent. The heart and a low battery are red, as in the
/// other themes; shadows black.
///
/// Last night's stages are red, white and grey: deep white, the heaviest
/// ink, as in every theme; REM red, the theme's red elsewhere too; and
/// light sleep, the lightest, grey ([`Color::GREY`]): white half-toned
/// with black, the panel's columns alternately lit (see
/// [`Theme::columns`]), as idle mode has no grey of its own. Light sleep
/// is usually the most of a night, so the bar is mostly a quiet grey,
/// with the deep and REM spells standing out of it.
/// TODO: idle mode has five more colours (green, blue, and their pairs),
/// if the dim look may ever have more colour than red.
#[rustfmt::skip]
pub static GLANCE: Theme = Theme {
    halftone: 1 << Color::GREY.0,
    ..theme("GLANCE", [
        0x000000, 0x000000, 0xffffff, 0xff0000, 0xff0000, 0x000000, 0xff0000,
        0x000000, 0x000000, 0xff0000, 0xffffff, 0xffffff, 0xff0000, 0xffffff,
        0xff0000, 0xffffff,
    ])
};

pub static DARK: [&Theme; 2] = [&ARCADE, &TERMINAL];

/// Themes for daylight. Long press cycles within this list by day.
pub static LIGHT: [&Theme; 2] = [&HANDHELD, &PARCHMENT];
