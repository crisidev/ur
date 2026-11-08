//! The watch as the PineTime is built, for the window and for screenshots
//! of the whole watch (`shot FILE.png --watch`).

use embassy_time::Instant;

use crate::brightness::{self, BY_CURRENT};
use crate::display::{PANEL, SIZE, rgb888};

/// The watch's outline, and where its panel and button are, in window
/// pixels.
///
/// The dimensions are PINE64's: a 37.5 × 40 mm case (11 mm thick) of zinc
/// alloy and plastic, and a 1.3″ square panel, 23.35 mm a side. The rest is
/// from the product photos
/// (<https://pine64.com/product/pinetime-smartwatch-sealed/>): black all
/// over, round corners, black glass round the panel (twice as much below it
/// as above), the strap's ends just beyond the case, and the one button on
/// the left edge, level with the panel's lower third.
pub struct Case {
    /// Window pixels per millimetre.
    #[cfg_attr(
        not(any(feature = "window", target_arch = "wasm32")),
        allow(dead_code, reason = "for the mouse")
    )]
    scale: f32,
    pub width: usize,
    pub height: usize,
    /// The panel's top left corner, and its side, in window pixels.
    panel: (usize, usize),
    panel_side: usize,
    /// The button, in window pixels: left, top, right, bottom.
    #[cfg_attr(
        not(any(feature = "window", target_arch = "wasm32")),
        allow(dead_code, reason = "for the mouse")
    )]
    button: (f32, f32, f32, f32),
    /// Everything but the panel, drawn once.
    background: Vec<u32>,
}

/// A rounded rectangle, in millimetres about the case's top left corner:
/// left, top, right, bottom, and the corners' radius.
type Shape = (f32, f32, f32, f32, f32);

const CASE: Shape = (0.0, 0.0, 37.5, 40.0, 6.5);
const GLASS: Shape = (1.2, 1.2, 36.3, 38.8, 5.4);
const STRAP: Shape = (7.0, -7.0, 30.5, 47.0, 2.0);
/// The panel's top left corner, and its side (1.3″ on the diagonal).
const PANEL_AT: (f32, f32) = (7.075, 5.8);
const PANEL_SIDE: f32 = 23.35;
/// The button: its left edge, top, how far it stands out, and its length.
const BUTTON: (f32, f32, f32, f32) = (-0.9, 20.0, 1.4, 7.0);
/// Room round the watch, in millimetres.
const MARGIN: f32 = 3.0;

const DESK_COLOR: [f32; 3] = [92.0, 92.0, 96.0];
const STRAP_COLOR: [f32; 3] = [16.0, 16.0, 17.0];
const BUTTON_COLOR: [f32; 3] = [44.0, 44.0, 47.0];
const RIM_COLOR: [f32; 3] = [58.0, 58.0, 62.0];
const CASE_COLOR: [f32; 3] = [24.0, 24.0, 26.0];
const GLASS_COLOR: [f32; 3] = [7.0, 7.0, 8.0];

impl Case {
    /// The watch at `zoom` window pixels a canvas pixel.
    pub fn new(zoom: usize) -> Self {
        let panel_side = zoom * crate::canvas::WIDTH;
        let scale = panel_side as f32 / PANEL_SIDE;
        // Where the case's top left corner is in the window, in millimetres.
        let left = MARGIN - BUTTON.0;
        let top = MARGIN - STRAP.1;
        let width = ((left + CASE.2 + MARGIN) * scale).ceil() as usize;
        let height = ((top + STRAP.3 + MARGIN) * scale).ceil() as usize;
        let px = |mm: f32, origin: f32| (mm + origin) * scale;
        let panel = (
            px(PANEL_AT.0, left).round() as usize,
            px(PANEL_AT.1, top).round() as usize,
        );
        let button = (
            px(BUTTON.0, left),
            px(BUTTON.1, top),
            px(BUTTON.0 + BUTTON.2, left),
            px(BUTTON.1 + BUTTON.3, top),
        );
        let (l, t, r, b, radius) = CASE;
        let layers = [
            (STRAP, STRAP_COLOR),
            (
                (
                    BUTTON.0,
                    BUTTON.1,
                    BUTTON.0 + BUTTON.2 + 1.0,
                    BUTTON.1 + BUTTON.3,
                    0.5,
                ),
                BUTTON_COLOR,
            ),
            (CASE, RIM_COLOR),
            // The rim is lit from the top left: the body sits a little
            // below and to the right of it.
            ((l + 0.35, t + 0.45, r - 0.2, b - 0.2, radius), CASE_COLOR),
            (GLASS, GLASS_COLOR),
        ];
        let mut background = vec![0; width * height];
        for (y, row) in background.chunks_mut(width).enumerate() {
            for (x, pixel) in row.iter_mut().enumerate() {
                // The pixel's middle, in millimetres about the case.
                let (mx, my) = (
                    (x as f32 + 0.5) / scale - left,
                    (y as f32 + 0.5) / scale - top,
                );
                let mut color = DESK_COLOR;
                for (shape, paint) in layers {
                    let cover = coverage(distance(mx, my, shape) * scale);
                    for (c, p) in color.iter_mut().zip(paint) {
                        *c += (p - *c) * cover;
                    }
                }
                *pixel = pack(color);
            }
        }
        Self {
            scale,
            width,
            height,
            panel,
            panel_side,
            button,
            background,
        }
    }

    /// Where the window pixel (`x`, `y`) is on the panel, in panel pixels.
    #[cfg(any(feature = "window", target_arch = "wasm32"))]
    pub fn on_panel(&self, x: f32, y: f32) -> Option<(i32, i32)> {
        let (px, py) = (x - self.panel.0 as f32, y - self.panel.1 as f32);
        let side = self.panel_side as f32;
        ((0.0..side).contains(&px) && (0.0..side).contains(&py)).then(|| {
            let to_panel = SIZE as f32 / side;
            ((px * to_panel) as i32, (py * to_panel) as i32)
        })
    }

    /// Whether the window pixel (`x`, `y`) is on the button, or near
    /// enough: it's small.
    #[cfg(any(feature = "window", target_arch = "wasm32"))]
    pub fn on_button(&self, x: f32, y: f32) -> bool {
        let slack = self.scale;
        let (left, top, right, bottom) = self.button;
        (left - slack..right + slack).contains(&x) && (top..bottom).contains(&y)
    }

    /// Draws the watch into `buffer` (`width` × `height`, 0xRRGGBB): the
    /// panel lit `light` (0 off, to 1), and the whole watch shifted
    /// `shake` pixels sideways (while it buzzes: see [`Case::shake`]), the
    /// desk filling in behind.
    pub fn draw(&self, buffer: &mut [u32], light: f32, shake: f32) {
        // Whole pixels by moving the watch, the rest by blending each pixel
        // with its left neighbour (below).
        let whole = shake.floor();
        let fraction = shake - whole;
        let shake = whole as isize;
        let width = self.width;
        let shift = shake.unsigned_abs().min(width);
        let desk = pack(DESK_COLOR);
        for (row, background) in buffer.chunks_mut(width).zip(self.background.chunks(width)) {
            if shake >= 0 {
                row[shift..].copy_from_slice(&background[..width - shift]);
                row[..shift].fill(desk);
            } else {
                row[..width - shift].copy_from_slice(&background[shift..]);
                row[width - shift..].fill(desk);
            }
        }
        let left = self.panel.0.saturating_add_signed(shake);
        // Each window pixel shows the panel pixel under its middle: canvas
        // pixels are 3 panel pixels, and `zoom` window pixels.
        let side = self.panel_side;
        let panel_pixel = |x: usize| (2 * x + 1) * SIZE / (2 * side);
        PANEL.with_borrow(|panel| {
            for y in 0..side {
                let row = &mut buffer[(self.panel.1 + y) * width..][..width];
                for x in 0..side {
                    let pixel = panel.pixels[panel_pixel(y) * SIZE + panel_pixel(x)];
                    let color = rgb888(pixel, panel.idle);
                    // Off, the panel is as black as the glass round it.
                    let lit: [f32; 3] =
                        std::array::from_fn(|i| (f32::from(color[i]) * light).max(GLASS_COLOR[i]));
                    row[left + x] = pack(lit);
                }
            }
        });
        // A fraction of a pixel further right: each pixel takes that much
        // of the one to its left. A shake of a pixel or two then glides
        // instead of jumping a whole pixel at a time, which strobes. The
        // panel's pixels blur with it, by as much as the case's edges: a
        // canvas pixel is several window pixels, so only its edges soften,
        // as they would in a blur of motion. Unshaken, nothing is blended,
        // so the still watch is as sharp as ever.
        if fraction > 0.0 {
            let weight = (fraction * 256.0) as u32;
            for row in buffer.chunks_mut(width) {
                let mut left = pack(DESK_COLOR);
                for pixel in row.iter_mut() {
                    let this = *pixel;
                    *pixel = mix(this, left, weight);
                    left = this;
                }
            }
        }
    }

    /// How far to shift the watch sideways, in window pixels, with the
    /// motor at `pulse` (`None`: still) at `now`: for [`Case::draw`].
    ///
    /// The watch hums sideways at [`SHAKE_HZ`], as far as [`SHAKE_REACH`]
    /// (a window pixel, up to two at large zooms) at full strength. Each
    /// pulse eases in over [`SPIN_UP_MS`] and out over [`SPIN_DOWN_MS`],
    /// as the motor spins up and down: a short tap only flicks the watch,
    /// less than fully, and a long buzz is a steady hum, so an alarm's
    /// build-up from faint ticks to long buzzes shows in the shake.
    ///
    /// A function of the time, not of the frame before, so the window,
    /// the browser, and recordings, each drawing frames at their own pace,
    /// all shake the watch alike.
    pub fn shake(&self, pulse: Option<Pulse>, now: Instant) -> f32 {
        let Some(Pulse { on_ms, off_ms }) = pulse else {
            return 0.0;
        };
        let strength = smoothstep(on_ms / SPIN_UP_MS)
            * (1.0 - smoothstep(off_ms.unwrap_or(0.0) / SPIN_DOWN_MS));
        if strength <= 0.0 {
            return 0.0;
        }
        let zoom = self.panel_side / crate::canvas::WIDTH;
        let reach = (SHAKE_REACH * zoom as f32).clamp(1.0, 2.0);
        // The time into the second: a whole number of swings a second
        // repeats each second, so this loses no precision however long the
        // simulator has run. `libm`'s sine, as `distance`'s `hypotf`, for
        // the browser's build to match to the bit.
        let second = (now.as_micros() % 1_000_000) as f32 / 1e6;
        let swing = libm::sinf(std::f32::consts::TAU * SHAKE_HZ as f32 * second);
        reach * strength * swing
    }
}

// ==============================================================================
// The motor's shake
// ==============================================================================

/// Where the motor is in its pattern, for [`Case::shake`]: how long it
/// has been on in its current pulse, in milliseconds, or, between pulses
/// (`off_ms`), how long the last pulse was on, and how long ago it ended.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pulse {
    pub on_ms: f32,
    pub off_ms: Option<f32>,
}

/// How fast the watch hums, in swings a second: whole, for
/// [`Case::shake`]'s precision. A real motor's 150 Hz or so would be lost
/// between frames (60 a second in the window, 50 in a recording); at 19,
/// each frame lands somewhere else in the swing, a restless jitter rather
/// than a steady sway (as a slower hum would be) or a strobe (as a frame
/// at each extreme, at 25 or 30, would be).
const SHAKE_HZ: u32 = 19;

/// How far the watch swings at full strength, in window pixels a canvas
/// pixel's width (`zoom`): a pixel at the default zoom of 4, kept to
/// between one and two. Further reads as the watch jumping, not humming;
/// less than a pixel is lost in the blend.
const SHAKE_REACH: f32 = 0.25;

/// How long the motor takes to spin up, in milliseconds: "tens of
/// milliseconds" (`motor.rs`). An alarm's first 30 ms ticks only reach
/// about four fifths of full strength.
const SPIN_UP_MS: f32 = 40.0;

/// How long it takes to wind down once off, in milliseconds: a little
/// longer than it takes to spin up, as it coasts.
const SPIN_DOWN_MS: f32 = 60.0;

/// 0 at `x` ≤ 0 to 1 at `x` ≥ 1, easing in and out.
fn smoothstep(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// `a` with `weight`/256 of `b` (0xRRGGBB each), in whole numbers: the
/// same, to the bit, in the browser's build.
fn mix(a: u32, b: u32, weight: u32) -> u32 {
    let channel = |shift: u32| {
        let (a, b) = ((a >> shift) & 0xFF, (b >> shift) & 0xFF);
        ((a * (256 - weight) + b * weight) >> 8) << shift
    };
    channel(16) | channel(8) | channel(0)
}

/// How bright a backlight level looks: off is black, and the dimmest
/// still readable.
pub fn light(level: brightness::Level) -> f32 {
    match BY_CURRENT.iter().position(|&l| l == level) {
        Some(rank) => 0.5 + 0.5 * rank as f32 / (BY_CURRENT.len() - 1) as f32,
        None => 0.0,
    }
}

/// The distance from (`x`, `y`) to a rounded rectangle's edge, negative
/// inside.
fn distance(x: f32, y: f32, (left, top, right, bottom, radius): Shape) -> f32 {
    let (cx, cy) = ((left + right) / 2.0, (top + bottom) / 2.0);
    let (hx, hy) = ((right - left) / 2.0 - radius, (bottom - top) / 2.0 - radius);
    let (dx, dy) = ((x - cx).abs() - hx, (y - cy).abs() - hy);
    // `libm`'s, not `f32::hypot`, which is the C library's: the browser's
    // build has another, whose last bit can differ, and so a pixel or two
    // of the edges' shading (see `web/check.sh`).
    libm::hypotf(dx.max(0.0), dy.max(0.0)) + dx.max(dy).min(0.0) - radius
}

/// How much of a pixel a shape covers, from the distance in pixels from
/// its middle to the shape's edge: smooth edges.
fn coverage(distance: f32) -> f32 {
    (0.5 - distance).clamp(0.0, 1.0)
}

fn pack([r, g, b]: [f32; 3]) -> u32 {
    (r as u32) << 16 | (g as u32) << 8 | b as u32
}
