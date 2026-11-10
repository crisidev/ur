//! Host tests for Retro's colour schemes (`src/palette.rs`, and
//! `src/themes/retro/schemes/mod.rs`): that the colours its screens use to
//! tell things apart stay apart in every scheme, and that its dim look has
//! only the colours the panel's idle mode can show. Several roles share a
//! colour in some schemes by design (a scheme has few colours), which is
//! how Last night once drew deep and REM sleep in the same red. `cargo
//! test`.
//!
//! Only Retro's: the checks are of what Retro draws with each role, which
//! another theme needn't share (nor have several schemes, nor a palette
//! of roles at all, beyond the 16 the canvas holds). A theme with checks
//! of its own would have a crate of them beside this one.

#[path = "../../../src/palette.rs"]
#[allow(dead_code)]
mod palette;

// Retro's schemes: `schemes/mod.rs`, with any modules of more schemes
// beside it, which come with it, as they're pure data too.
#[path = "../../../src/themes/retro/schemes/mod.rs"]
#[allow(dead_code)]
mod retro;

/// Every scheme of Retro's, both sides.
#[cfg(test)]
fn schemes() -> impl Iterator<Item = &'static retro::Retro> {
    retro::NIGHT.into_iter().chain(retro::DAY)
}

// `Color` derives `defmt::Format`, whose binary log needs the firmware's
// ELF to decode; nothing logs here, but the derive wants a logger to link.
#[defmt::global_logger]
struct Logger;

// SAFETY: Does nothing, so there's nothing to synchronise.
unsafe impl defmt::Logger for Logger {
    fn acquire() {}
    unsafe fn flush() {}
    unsafe fn release() {}
    unsafe fn write(_bytes: &[u8]) {}
}

defmt::timestamp!("");

fn main() {
    println!("run `cargo test`");
}

#[cfg(test)]
mod tests {
    use crate::palette::{Color, Palette};
    use crate::{retro, schemes};

    // ==============================================================================
    // Telling colours apart
    // ==============================================================================

    // Distances are CIE76 ΔE: straight-line distance in CIELAB, which (unlike
    // RGB) is roughly even to the eye, so one threshold works for dark greens
    // and pale creams alike. About 2 is just noticeable side by side; 10 is
    // plainly different; 50 is red against green. The colours are measured as
    // the panel gets them, RGB565, not as written in palette.rs.

    /// A role's colour in a theme, in CIELAB (L*, a*, b*), from its RGB565.
    fn lab(theme: &Palette, color: Color) -> [f64; 3] {
        let rgb565 = u16::from_be_bytes(theme.bytes(color));
        // Back to 8 bits a channel, as screenshot.py does.
        let (r, g, b) = (rgb565 >> 11, (rgb565 >> 5) & 0x3F, rgb565 & 0x1F);
        let channels = [r << 3 | r >> 2, g << 2 | g >> 4, b << 3 | b >> 2];
        // sRGB to linear light, then to XYZ (D65 white).
        let [r, g, b] = channels.map(|c| {
            let c = f64::from(c) / 255.0;
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        });
        let x = (0.4124 * r + 0.3576 * g + 0.1805 * b) / 0.95047;
        let y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
        let z = (0.0193 * r + 0.1192 * g + 0.9505 * b) / 1.08883;
        let f = |t: f64| {
            if t > 0.008856 {
                t.cbrt()
            } else {
                7.787 * t + 16.0 / 116.0
            }
        };
        let (fx, fy, fz) = (f(x), f(y), f(z));
        [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
    }

    fn delta_e(theme: &Palette, a: Color, b: Color) -> f64 {
        let (a, b) = (lab(theme, a), lab(theme, b));
        a.iter()
            .zip(b)
            .map(|(a, b)| (a - b).powi(2))
            .sum::<f64>()
            .sqrt()
    }

    /// The palettes a screen can be in when it's not dim: every scheme's.
    /// The dim look has only idle mode's colours, so distances mean little
    /// there: it has its own tests below.
    fn palettes() -> impl Iterator<Item = &'static Palette> {
        schemes().map(|scheme| &scheme.palette)
    }

    /// The dim look's palettes: Retro has one, for every scheme.
    fn dim_palettes() -> impl Iterator<Item = &'static Palette> {
        [&retro::GLANCE].into_iter()
    }

    // ==============================================================================
    // Last night's sleep stages
    // ==============================================================================

    /// The stages, as Last night (`ui::render_last_night`) draws them: the bar
    /// and the legend.
    const STAGES: [(&str, Color); 3] = [
        ("deep", Color::DEEP),
        ("REM", Color::REM),
        ("light", Color::GREY),
    ];

    /// Side by side in a 4 px bar (and as each one's name in the legend), the
    /// stages must look different at a glance: well past "just noticeable".
    /// Deep and light, both of the theme's ink, are the closest (~20), but
    /// they're only side by side if there was no REM.
    const BETWEEN_STAGES: f64 = 15.0;

    /// Each stage's name is text in its colour, so it must read against the
    /// background like text does (the dimmest text, faint hints, is ~30 off it
    /// in the light themes).
    const FROM_BACKGROUND: f64 = 40.0;

    /// The bar has no track behind it, but a stage shouldn't look like one
    /// (an empty gauge) either.
    const FROM_TRACK: f64 = 25.0;

    #[test]
    fn sleep_stages_differ_from_each_other() {
        for theme in palettes() {
            for (i, &(a_name, a)) in STAGES.iter().enumerate() {
                for &(b_name, b) in &STAGES[i + 1..] {
                    let distance = delta_e(theme, a, b);
                    assert!(
                        distance >= BETWEEN_STAGES,
                        "{}: {a_name} and {b_name} are only {distance:.1} apart",
                        theme.name
                    );
                }
            }
        }
    }

    #[test]
    fn sleep_stages_stand_out_from_the_background_and_track() {
        for theme in palettes() {
            for &(name, stage) in &STAGES {
                let background = delta_e(theme, stage, Color::BACKGROUND);
                assert!(
                    background >= FROM_BACKGROUND,
                    "{}: {name} is only {background:.1} from the background",
                    theme.name
                );
                let track = delta_e(theme, stage, Color::TRACK);
                assert!(
                    track >= FROM_TRACK,
                    "{}: {name} is only {track:.1} from the track",
                    theme.name
                );
            }
        }
    }

    /// Deep sleep is the heaviest ink: the most lightness contrast with the
    /// background, whether that's white on black or dark brown on cream.
    #[test]
    fn deep_sleep_is_the_heaviest() {
        for theme in palettes() {
            let contrast = |color| (lab(theme, color)[0] - lab(theme, Color::BACKGROUND)[0]).abs();
            let deep = contrast(Color::DEEP);
            for &(name, stage) in &STAGES[1..] {
                assert!(
                    deep > contrast(stage),
                    "{}: {name} ({:.1}) is heavier than deep ({deep:.1})",
                    theme.name,
                    contrast(stage)
                );
            }
        }
    }

    /// In the dim look, the stages (in Retro's: red, white and grey) are
    /// told apart as the panel shows them in idle mode (the top bit of
    /// each channel): no two of them the same on both the panel's even
    /// and odd columns.
    #[test]
    fn sleep_stages_differ_in_the_dim_look() {
        for dim in dim_palettes() {
            stages_differ_in(dim);
        }
    }

    fn stages_differ_in(dim: &Palette) {
        let idle = |color| {
            dim.columns(color).map(|bytes| {
                let rgb565 = u16::from_be_bytes(bytes);
                // Grouped as RGB565's channels: 5, 6 and 5 bits.
                #[allow(clippy::unusual_byte_groupings)]
                let top_bits = 0b10000_100000_10000;
                rgb565 & top_bits
            })
        };
        for (i, &(a_name, a)) in STAGES.iter().enumerate() {
            for &(b_name, b) in &STAGES[i + 1..] {
                assert_ne!(
                    idle(a),
                    idle(b),
                    "{}: {a_name} and {b_name} look the same",
                    dim.name
                );
            }
        }
    }

    /// In the dim look, light sleep's grey is half lit, half the
    /// background: white half-toned.
    #[test]
    fn the_dim_grey_is_a_half_tone() {
        let glance = &retro::GLANCE;
        let [lit, unlit] = glance.columns(Color::GREY);
        assert_eq!(lit, glance.bytes(Color::DEEP));
        assert_eq!(unlit, glance.bytes(Color::BACKGROUND));
    }

    /// Every dim look has only the colours the panel's idle mode shows as
    /// they are: each of red, green and blue fully on or off, as idle mode
    /// keeps only their top bits, so anything between would come out as
    /// something else (a grey as white or black: faint text, invisible).
    /// A grey is a half-tone of two of these (see `Palette::columns`).
    #[test]
    fn dim_looks_have_only_idle_modes_colours() {
        for dim in dim_palettes() {
            for role in 0..Color::COUNT as u8 {
                let rgb565 = u16::from_be_bytes(dim.bytes(Color(role)));
                let (red, green, blue) = (rgb565 >> 11, (rgb565 >> 5) & 0x3F, rgb565 & 0x1F);
                assert!(
                    matches!(red, 0 | 0x1F)
                        && matches!(green, 0 | 0x3F)
                        && matches!(blue, 0 | 0x1F),
                    "{}: role {role} is {rgb565:#06x}, which idle mode can't show",
                    dim.name
                );
            }
        }
    }

    /// Every scheme has a name of its own (the simulator's `theme NAME`
    /// finds them by name); `sim`'s tests check every built-in theme's
    /// together.
    #[test]
    fn schemes_have_names_of_their_own() {
        let mut names: Vec<_> = schemes().map(|s| s.palette.name).collect();
        names.sort();
        let count = names.len();
        names.dedup();
        assert_eq!(names.len(), count, "two schemes share a name: {names:?}");
    }

    /// Nor do two share an ID (`palette::id`, a hash of the name), which
    /// is what the watch saves the chosen one as: two that did would each
    /// load as the first of them.
    #[test]
    fn schemes_have_ids_of_their_own() {
        let mut ids: Vec<_> = schemes().map(|s| s.palette.id).collect();
        ids.sort();
        let count = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), count, "two schemes' names hash to one ID");
    }

    /// The measure itself: the colours Last night first drew deep and REM in
    /// (accent and heart) are one red in Arcade, and the tests would say so.
    #[test]
    fn the_old_collision_fails() {
        let arcade = &retro::ARCADE.palette;
        assert!(delta_e(arcade, Color::ACCENT, Color::HEART) < BETWEEN_STAGES);
        // And a pair everyone can tell apart passes easily.
        assert!(delta_e(arcade, Color::TEXT, Color::BACKGROUND) > 80.0);
    }

    // ==============================================================================
    // The button grid
    // ==============================================================================

    /// What a grid button (Retro's `draw_key`) needs to tell apart, in a
    /// dark palette and in a light one (`Palette::is_dark`, which it shades
    /// by): its edge from the background, its cap from
    /// the side it stands on (raised) and the shadow of its well (pressed),
    /// the cap's lit edge from the cap, and a lit (waiting) cap from an
    /// unlit one and from its well. `draw_key` picks these roles; keep the
    /// two in step.
    const KEY_DARK: [(&str, Color, Color); 5] = [
        ("edge", Color::TEXT_FAINT, Color::BACKGROUND),
        ("cap and side", Color::TRACK, Color::SHADOW),
        ("lit edge", Color::TEXT_DIM, Color::TRACK),
        ("lit and unlit", Color::ACCENT, Color::TRACK),
        ("lit and well", Color::ACCENT, Color::SHADOW),
    ];
    const KEY_LIGHT: [(&str, Color, Color); 5] = [
        ("edge", Color::TEXT, Color::BACKGROUND),
        ("cap and side", Color::TRACK, Color::TEXT),
        ("lit edge", Color::BACKGROUND, Color::TRACK),
        ("lit and unlit", Color::ACCENT, Color::TRACK),
        ("lit and well", Color::ACCENT, Color::TEXT),
    ];

    /// Plainly different, if not far apart: these are shading, not text.
    const KEY_PARTS: f64 = 10.0;

    #[test]
    fn grid_buttons_have_visible_edges() {
        for theme in palettes() {
            let pairs = if theme.is_dark() { KEY_DARK } else { KEY_LIGHT };
            for (what, a, b) in pairs {
                let distance = delta_e(theme, a, b);
                assert!(
                    distance >= KEY_PARTS,
                    "{}: a grid button's {what} differ by only {distance:.1}",
                    theme.name
                );
            }
        }
    }
}

/// Kani proofs (`cargo kani` here, one at a time with `--harness`), about
/// every role in every theme, where the tests above compare chosen roles.
/// Looking a role up never panics, and its columns are its colour or the
/// background's; and the dim look has only the colours the panel's idle
/// mode can show.
///
/// A role is any of the `Color::COUNT` the canvas's 4 bits hold. A
/// `Color` made of a higher number (nothing makes one) would index past
/// the palette.
#[cfg(kani)]
mod proofs {
    use crate::palette::{Color, Palette};
    use crate::retro;

    /// Any of Retro's palettes: every scheme in `NIGHT` and `DAY`, so a new
    /// one is proven as soon as it's listed, and the dim look's.
    fn any_theme() -> &'static Palette {
        let (night, day) = (retro::NIGHT.len(), retro::DAY.len());
        let i = usize::from(kani::any::<u8>());
        kani::assume(i <= night + day);
        if i < night {
            &retro::NIGHT[i].palette
        } else if i < night + day {
            &retro::DAY[i - night].palette
        } else {
            &retro::GLANCE
        }
    }

    fn any_role() -> Color {
        let role: u8 = kani::any();
        kani::assume(usize::from(role) < Color::COUNT);
        Color(role)
    }

    /// For every role in every theme: the panel's even columns get the
    /// role's colour, and its odd columns the same, or the background's
    /// if the theme half-tones the role. Only the dim look half-tones
    /// any, and only its grey and its track: every other role, in every
    /// theme, is a solid colour.
    #[kani::proof]
    fn a_role_is_its_colour_or_half_the_background() {
        let (theme, role) = (any_theme(), any_role());
        let [even, odd] = theme.columns(role);
        assert_eq!(even, theme.bytes(role));
        if odd != even {
            assert_eq!(odd, theme.bytes(Color::BACKGROUND));
            assert!(core::ptr::eq(theme, &retro::GLANCE));
            assert!(role == Color::GREY || role == Color::TRACK);
        }
    }

    /// For every role in the dim look: its colour is one the panel's idle
    /// mode shows as it is, each of red, green and blue fully on or off
    /// (idle mode keeps only their top bits, so anything between would
    /// come out as something else); and of those eight, only black,
    /// white and red, as the theme's docs promise.
    #[kani::proof]
    fn the_dim_look_is_black_white_and_red() {
        let rgb565 = u16::from_be_bytes(retro::GLANCE.bytes(any_role()));
        let (red, green, blue) = (rgb565 >> 11, (rgb565 >> 5) & 0x3F, rgb565 & 0x1F);
        assert!(red == 0 || red == 0x1F);
        assert!(green == 0 || green == 0x3F);
        assert!(blue == 0 || blue == 0x1F);
        assert!(matches!(rgb565, 0x0000 | 0xFFFF | 0xF800));
    }
}
