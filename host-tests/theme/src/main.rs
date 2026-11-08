//! Host tests for the firmware's `theme` module: that the colours a screen
//! uses to tell things apart stay apart in every theme. Several roles share
//! a colour in some themes by design (a theme has few colours), which is
//! how Last night once drew deep and REM sleep in the same red. `cargo
//! test`.

#[path = "../../../src/theme.rs"]
#[allow(dead_code)]
mod theme;

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
    use crate::theme::{self, Color, Theme};

    // ==============================================================================
    // Telling colours apart
    // ==============================================================================

    // Distances are CIE76 ΔE: straight-line distance in CIELAB, which (unlike
    // RGB) is roughly even to the eye, so one threshold works for dark greens
    // and pale creams alike. About 2 is just noticeable side by side; 10 is
    // plainly different; 50 is red against green. The colours are measured as
    // the panel gets them, RGB565, not as written in theme.rs.

    /// A role's colour in a theme, in CIELAB (L*, a*, b*), from its RGB565.
    fn lab(theme: &Theme, color: Color) -> [f64; 3] {
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

    fn delta_e(theme: &Theme, a: Color, b: Color) -> f64 {
        let (a, b) = (lab(theme, a), lab(theme, b));
        a.iter()
            .zip(b)
            .map(|(a, b)| (a - b).powi(2))
            .sum::<f64>()
            .sqrt()
    }

    /// The themes a screen can be in when it's not dim. The dim look
    /// (`theme::GLANCE`) is black and white by necessity, so everything in it
    /// is white: it's left out on purpose.
    fn themes() -> impl Iterator<Item = &'static Theme> {
        theme::DARK.iter().chain(&theme::LIGHT).copied()
    }

    // ==============================================================================
    // Last night's sleep stages
    // ==============================================================================

    /// The stages, as Last night (`ui::render_last_night`) draws them: the bar
    /// and the legend.
    const STAGES: [(&str, Color); 3] = [
        ("deep", Color::DEEP),
        ("REM", Color::REM),
        ("light", Color::TEXT_DIM),
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
        for theme in themes() {
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
        for theme in themes() {
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
        for theme in themes() {
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

    /// The measure itself: the colours Last night first drew deep and REM in
    /// (accent and heart) are one red in Arcade, and the tests would say so.
    #[test]
    fn the_old_collision_fails() {
        assert!(delta_e(&theme::ARCADE, Color::ACCENT, Color::HEART) < BETWEEN_STAGES);
        // And a pair everyone can tell apart passes easily.
        assert!(delta_e(&theme::ARCADE, Color::TEXT, Color::BACKGROUND) > 80.0);
    }
}
