//! How the PineTime's panel shows colours, for a computer's sRGB monitor
//! to show roughly what the watch would: a pure function of a pixel, the
//! backlight, and the light reflected off the glass, with no window in
//! it (so a browser build can use it too).
//!
//! The panel is a 1.3″ transmissive IPS LCD on an ST7789V, lit by two
//! white LEDs. Its only datasheet is the one PINE64's documentation links
//! as the "1.3″ 240x240 IPS LCD Panel Specification for PineTime"
//! (Further information → Datasheets, schematics and certifications,
//! <https://files.pine64.org/doc/datasheet/pinetime/PineTime%20LCD%20Panel.jpg>):
//! a one-page mechanical drawing dated 2019/6/24, page 1/1, with no model
//! number, drawing number or revision filled in. Its table gives the
//! display type (1.33″ TFT, transmissive), viewing direction (full view),
//! driver IC (ST7789V), backlight (two white chip LEDs, in parallel),
//! voltage (2.8 V), temperatures and connector, and **no optical
//! figures at all**: no chromaticity of white or the primaries, no gamut,
//! contrast, luminance or colour temperature. (PINE64's forum asked which
//! module it is, in 2021, with no answer.)
//!
//! Compared with an sRGB monitor, those who've held one next to a
//! monitor say it's "much less saturated", and that the white point
//! differs by batch: cold on early watches, close to neutral or a little
//! warm on later ones (InfiniTime PR 1506's photo,
//! <https://github.com/InfiniTimeOrg/InfiniTime/pull/1506#issuecomment-1370628499>,
//! and issue 745). Nobody has published a colorimeter's measurements of
//! it, so this is a model built from the controller's settings and the
//! closest documented source, a comparable module's datasheet, in three
//! steps, in linear light:
//!
//! 1. **Tone.** The ST7789V's gamma curve after reset is its G2.2
//!    (GAMSET 26h, datasheet v1.6 p. 192), and the firmware writes no
//!    gamma registers: each channel's code `v` (0–1) gives `v^2.2` of
//!    full transmission. The firmware's VDV of −0.4 V (`src/display.rs`,
//!    as InfiniTime's PR 1108) narrows the gamma reference voltages,
//!    which, InfiniTime found, removes a large step from black to the
//!    first grey that the controller's defaults left; so no crushed
//!    shadows are modelled.
//! 2. **Gamut and white.** The panel's primaries and white, from a
//!    comparable 1.3″ 240×240 ST7789 IPS module's datasheet (Allshore
//!    ASI-T-1302A3SPN/N, revision A of 2021-08-31, section 5 "Optical
//!    characteristics", p. 8, typical values,
//!    <https://www.allshore.com/portals/0/PDFs/ASIT1302A3SPNN.pdf>): red
//!    (0.624, 0.329), green (0.288, 0.522), blue (0.136, 0.137), about
//!    50% of NTSC (its sibling's datasheet says 50.1%), against sRGB's
//!    71%; and white (0.302, 0.325), about 7160 K and a little greener
//!    than a black body's. [`GAMUT`] converts the panel's linear RGB to
//!    sRGB's, keeping that white rather than balancing it to D65 (as
//!    this did at first): the PineTime's own datasheet gives no white to
//!    use instead, and an idealised white hides what the panel does.
//!    Green and blue lose saturation, blue, slightly outside sRGB, leans
//!    towards cyan, and white is cold, as on the early watches: sRGB
//!    (237, 251, 255).
//! 3. **Black.** The same table gives a contrast ratio of 640–800:1, so
//!    black lets through [`BLACK`] of white's light, more the brighter
//!    the backlight; and the glass in front reflects the room, over
//!    everything, lit or not.
//!
//! What's left out: the colour shift and contrast loss off-axis (IPS
//! keeps both small; TODO if a photo shows them matter), the backlight's
//! own colour changing with its current, the lowered supply voltages'
//! (PWCTRL1) effect on white, if any, and batch-to-batch variation.
//!
//! **Calibrating it.** Photograph the dev kit and the simulator's window
//! side by side, in a dim room, with the camera's exposure and white
//! balance fixed (daylight), the dev kit showing the theme being checked
//! and the window the same frame (`--zoom` until the screens are the same
//! size). Then adjust: [`GAMMA`] until the greys' steps match, [`WHITE`]
//! until white's tint does, [`PRIMARIES`] until the colours do (and
//! re-derive [`GAMUT`]), and [`BLACK`] until black glows as much. A colour chart photographed under
//! the same light lets the camera's own colour be corrected first.

use std::sync::OnceLock;

/// The controller's gamma curve (step 1).
pub const GAMMA: f32 = 2.2;

/// The panel's primaries, CIE 1931 (x, y): red, green, blue (step 2).
#[cfg_attr(
    not(test),
    allow(dead_code, reason = "GAMUT is derived from it in the tests")
)]
pub const PRIMARIES: [(f64, f64); 3] = [(0.624, 0.329), (0.288, 0.522), (0.136, 0.137)];

/// The panel's white, CIE 1931 (x, y): what its full RGB shows (step 2).
#[cfg_attr(
    not(test),
    allow(dead_code, reason = "GAMUT is derived from it in the tests")
)]
pub const WHITE: (f64, f64) = (0.302, 0.325);

/// The panel's linear RGB to linear sRGB, from [`PRIMARIES`] and
/// [`WHITE`]: the colour the panel makes, as sRGB makes it, with no
/// adaptation to sRGB's D65. Scaled so that the panel's white is as
/// bright as a monitor can show that colour (its bluest channel at
/// full, as the panel's white is the brightest it shows): its rows sum
/// to 0.850, 0.964 and 1, so greys keep white's cold tint. Derived in
/// the test below.
///
/// Watches vary (cold early, neutral or warm later), and the eye adapts
/// to a screen's white, so a watch on the wrist may well look less blue
/// than this; TODO: refit [`WHITE`] once a dev kit's has been
/// photographed (see "Calibrating it" above).
pub const GAMUT: [[f32; 3]; 3] = [
    [0.9643, 0.0396, -0.1536],
    [0.0093, 0.7748, 0.1803],
    [0.0112, 0.1208, 0.8681],
];

/// Black's light, as a fraction of white's (step 3): 1/700, from the
/// middle of the 640–800:1 contrast ratio the comparable module gives.
pub const BLACK: f32 = 1.0 / 700.0;

/// The panel's colour, as an sRGB monitor shows it: `rgb` is what the
/// panel was sent (8 bits a channel, as `display::rgb888` expands it),
/// `light` how bright the backlight is (0 off, to 1; as `case::light`
/// gives, a scale of the encoded values, as the window has always used),
/// and `glass` what the glass reflects (sRGB, as the case's glass is
/// drawn), which adds to everything.
pub fn mimic(rgb: [u8; 3], light: f32, glass: [u8; 3]) -> [u8; 3] {
    let tables = tables();
    // Scaling encoded values by `light` is scaling linear light by
    // `light^2.2`.
    let light = light.powf(GAMMA);
    let panel = rgb.map(|c| tables.panel[usize::from(c)]);
    std::array::from_fn(|i| {
        let row = GAMUT[i];
        let srgb = (row[0] * panel[0] + row[1] * panel[1] + row[2] * panel[2]).max(0.0);
        let emitted = light * (BLACK + (1.0 - BLACK) * srgb);
        tables.encode(emitted + tables.srgb[usize::from(glass[i])])
    })
}

struct Tables {
    /// Code to the panel's linear light, by [`GAMMA`].
    panel: [f32; 256],
    /// An sRGB code to linear light.
    srgb: [f32; 256],
    /// Linear light (0–1, in [`ENCODE`] steps) to an sRGB code.
    encoded: Vec<u8>,
}

/// Steps of linear light for encoding: fine enough that the darkest
/// steps, where sRGB is steepest, are under one code.
const ENCODE: usize = 8192;

impl Tables {
    fn encode(&self, linear: f32) -> u8 {
        let i = (linear.clamp(0.0, 1.0) * (ENCODE - 1) as f32).round() as usize;
        self.encoded[i]
    }
}

fn tables() -> &'static Tables {
    static TABLES: OnceLock<Tables> = OnceLock::new();
    TABLES.get_or_init(|| Tables {
        panel: std::array::from_fn(|c| (c as f32 / 255.0).powf(GAMMA)),
        srgb: std::array::from_fn(|c| srgb_to_linear(c as f32 / 255.0)),
        encoded: (0..ENCODE)
            .map(|i| (linear_to_srgb(i as f32 / (ENCODE - 1) as f32) * 255.0).round() as u8)
            .collect(),
    })
}

/// sRGB's transfer function (IEC 61966-2-1), each way.
fn srgb_to_linear(v: f32) -> f32 {
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(l: f32) -> f32 {
    if l <= 0.003_130_8 {
        l * 12.92
    } else {
        1.055 * l.powf(1.0 / 2.4) - 0.055
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type Matrix = [[f64; 3]; 3];

    fn xyz((x, y): (f64, f64)) -> [f64; 3] {
        [x / y, 1.0, (1.0 - x - y) / y]
    }

    fn invert(m: Matrix) -> Matrix {
        let [[a, b, c], [d, e, f], [g, h, i]] = m;
        let det = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
        [
            [e * i - f * h, c * h - b * i, b * f - c * e],
            [f * g - d * i, a * i - c * g, c * d - a * f],
            [d * h - e * g, b * g - a * h, a * e - b * d],
        ]
        .map(|row| row.map(|v| v / det))
    }

    fn times(m: Matrix, v: [f64; 3]) -> [f64; 3] {
        m.map(|row| row[0] * v[0] + row[1] * v[1] + row[2] * v[2])
    }

    /// RGB to XYZ for these primaries, with RGB 1, 1, 1 at `white` (the
    /// usual derivation, as for sRGB's own matrix).
    fn to_xyz(primaries: [(f64, f64); 3], white: (f64, f64)) -> Matrix {
        let columns = primaries.map(xyz);
        let p: Matrix = std::array::from_fn(|i| std::array::from_fn(|j| columns[j][i]));
        let scale = times(invert(p), xyz(white));
        p.map(|row| std::array::from_fn(|j| row[j] * scale[j]))
    }

    #[test]
    fn the_gamut_matrix_is_derived_from_the_primaries_and_white() {
        const D65: (f64, f64) = (0.3127, 0.3290);
        let srgb = to_xyz([(0.64, 0.33), (0.30, 0.60), (0.15, 0.06)], D65);
        // The panel's own white, not D65: no chromatic adaptation, so
        // the panel's white comes out as that colour in sRGB.
        let panel = to_xyz(PRIMARIES, WHITE);
        let from_xyz = invert(srgb);
        let unscaled: Matrix = std::array::from_fn(|i| {
            std::array::from_fn(|j| (0..3).map(|k| from_xyz[i][k] * panel[k][j]).sum())
        });
        // Then dimmed until white's brightest sRGB channel is just full.
        let brightest = unscaled
            .map(|row| row.iter().sum::<f64>())
            .into_iter()
            .fold(0.0, f64::max);
        for (i, row) in GAMUT.iter().enumerate() {
            for (j, &value) in row.iter().enumerate() {
                let derived = unscaled[i][j] / brightest;
                assert!(
                    (derived - f64::from(value)).abs() < 1e-4,
                    "GAMUT[{i}][{j}] is {value}, derived {derived:.4}"
                );
            }
        }
    }

    #[test]
    fn greys_and_white_take_the_panels_cold_white() {
        assert_eq!(mimic([255; 3], 1.0, [0; 3]), [237, 251, 255]);
        for code in [40u8, 128, 255] {
            let [r, g, b] = mimic([code; 3], 1.0, [0; 3]);
            assert!(r < g && g <= b, "{code}: {r} {g} {b}");
        }
    }

    #[test]
    fn black_glows_with_the_backlight_and_the_glass_shows_when_off() {
        let glass = [7, 7, 8];
        assert_eq!(mimic([0; 3], 0.0, glass), glass);
        let lit = mimic([0; 3], 1.0, glass);
        assert!(lit[0] > glass[0], "{lit:?}");
        assert!(mimic([0; 3], 0.5, glass)[0] < lit[0]);
    }

    #[test]
    fn green_and_blue_are_less_saturated() {
        let [r, g, b] = mimic([0, 255, 0], 1.0, [0; 3]);
        assert!(r > 0 && b > 0 && g < 255, "{r} {g} {b}");
        let [r, g, b] = mimic([0, 0, 255], 1.0, [0; 3]);
        assert!(g > 50 && b < 255, "{r} {g} {b}");
    }
}
