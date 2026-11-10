//! Colour palettes.
//!
//! Screens never name concrete colours; they paint with *roles*
//! ([`Color::TEXT`], [`Color::ACCENT`], …), and the active [`Palette`] says
//! what each role looks like. The canvas stores role indices and only
//! resolves them to RGB when pushing pixels, so switching palette is a
//! matter of swapping it and repainting; nothing has to be re-rendered.
//!
//! Each theme's colour schemes have a palette, and one for the dim look
//! (see `crate::themes`), and the theme draws the screens with these
//! roles. Their names say what retro (`crate::themes::retro`), the first
//! theme, uses each for; another theme may give its 16 slots meanings of
//! its own.
//!
//! Pure data, with nothing of the firmware's but this: `host-tests/theme`
//! includes it, with retro's schemes, to check their colours.

/// A colour role: an index into the active palette.
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
    /// black (see [`Palette::columns`]). Light sleep on Last night, and the
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
pub struct Palette {
    /// What Settings' THEME row shows, for a scheme's palette.
    pub name: &'static str,
    /// Each role as the two bytes the panel expects: RGB565, big-endian.
    colors: [[u8; 2]; Color::COUNT],
    /// The roles shown half-toned, a bit each (bit `n` for `Color(n)`):
    /// see [`Palette::columns`].
    halftone: u16,
}

impl Palette {
    pub fn bytes(&self, color: Color) -> [u8; 2] {
        self.colors[usize::from(color.0)]
    }

    /// A role's bytes for the panel's even and odd columns: its colour in
    /// both, unless the palette half-tones it, when the odd columns are the
    /// background. That's 1-pixel vertical stripes at the panel's own
    /// resolution, a third of a canvas pixel, too fine to see at ~0.1 mm:
    /// they read as a grey, where the panel can't show one (retro's dim
    /// look, `themes::retro::GLANCE`, half-tones its grey so).
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

/// Builds a palette from `0xRRGGBB` values listed in [`Color`] role order.
pub const fn palette(name: &'static str, hex: [u32; Color::COUNT]) -> Palette {
    let mut colors = [[0; 2]; Color::COUNT];
    let mut i = 0;
    while i < Color::COUNT {
        // Keep the top 5/6/5 bits of each 8-bit channel.
        let (r, g, b) = (
            (hex[i] >> 19) & 0x1F,
            (hex[i] >> 10) & 0x3F,
            (hex[i] >> 3) & 0x1F,
        );
        let rgb565 = ((r << 11) | (g << 5) | b) as u16;
        colors[i] = rgb565.to_be_bytes();
        i += 1;
    }
    Palette {
        name,
        colors,
        halftone: 0,
    }
}

impl Palette {
    /// The same palette, with `roles` (a bit each, bit `n` for `Color(n)`)
    /// half-toned with the background: see [`Palette::columns`].
    pub const fn halftone(self, roles: u16) -> Self {
        Self {
            halftone: roles,
            ..self
        }
    }

    /// Whether this is a dark palette: light ink on a darker background,
    /// rather than dark ink on paper. By the background's lightness
    /// against the text's (their RGB565 channels summed, green's six bits
    /// counting as much as the five of red and of blue together), which
    /// is all a drawing needs to pick its shading by (retro's grid
    /// buttons do).
    pub fn is_dark(&self) -> bool {
        let lightness = |color| {
            let rgb565 = u16::from_be_bytes(self.bytes(color));
            let (r, g, b) = (rgb565 >> 11, (rgb565 >> 5) & 0x3F, rgb565 & 0x1F);
            r + g + b
        };
        lightness(Color::BACKGROUND) < lightness(Color::TEXT)
    }
}
