//! An 80×80 canvas of colour roles (see [`crate::theme`]), shown on the
//! 240×240 panel at 3×.
//!
//! The low resolution *is* the pixel-art look: every UI element is drawn on
//! this grid, so nothing can accidentally end up finer-grained than a "pixel".
//!
//! The UI redraws the whole canvas from scratch every frame (immediate-mode),
//! which keeps screen code simple. Pushing all of that to the panel would cost
//! ~115 ms per frame at 8 MHz SPI, so we keep a copy of what the panel
//! currently shows (`front`) and [`Canvas::flush`] only sends the rows that
//! differ from it. A typical clock tick touches a handful of rows.

use core::ops::Range;

use embassy_time::{Duration, Instant, Ticker};

use crate::display::Display;
use crate::font;
use crate::theme::{self, Theme};

pub use crate::theme::Color;

pub const WIDTH: usize = 80;
pub const HEIGHT: usize = 80;
/// Panel pixels per canvas pixel, in each direction.
pub const SCALE: usize = 3;

/// Two pixels per byte (4-bit colour roles), low nibble first.
const BYTES: usize = WIDTH * HEIGHT / 2;
/// Two full-width canvas rows of panel pixel bytes.
const BATCH: usize = 2 * WIDTH * SCALE * SCALE * 2;

pub struct Canvas {
    /// The frame being drawn.
    back: [u8; BYTES],
    /// What the panel is currently showing.
    front: [u8; BYTES],
    /// False until the panel has received one full frame (or after a theme
    /// change); until then, `front` says nothing about the panel's contents.
    front_valid: bool,
    /// Resolves colour roles to pixel bytes when pushing.
    theme: &'static Theme,
    /// Pixel bytes waiting to be sent; see [`stream`].
    batch: [u8; BATCH],
}

impl Canvas {
    pub const fn new() -> Self {
        Self {
            back: [0; BYTES],
            front: [0; BYTES],
            front_valid: false,
            theme: &theme::ARCADE,
            batch: [0; BATCH],
        }
    }

    /// Switches theme. The canvas contents (roles) stay as they are, but every
    /// pixel's colour changes, so the next flush repaints the whole panel.
    pub fn set_theme(&mut self, theme: &'static Theme) {
        if !core::ptr::eq(theme, self.theme) {
            self.theme = theme;
            self.front_valid = false;
        }
    }

    // ==========================================================================
    // Drawing
    // ==========================================================================

    pub fn clear(&mut self, color: Color) {
        self.back.fill(color.0 | (color.0 << 4));
    }

    /// Sets one pixel. Out-of-bounds coordinates are silently clipped, so
    /// callers can draw shapes that hang off the edge.
    pub fn pixel(&mut self, x: i32, y: i32, color: Color) {
        if !(0..WIDTH as i32).contains(&x) || !(0..HEIGHT as i32).contains(&y) {
            return;
        }
        set_nibble(&mut self.back, y as usize * WIDTH + x as usize, color.0);
    }

    pub fn fill_rect(&mut self, x: i32, y: i32, w: i32, h: i32, color: Color) {
        for yy in y..y + h {
            for xx in x..x + w {
                self.pixel(xx, yy, color);
            }
        }
    }

    /// Draws a 1-bit sprite: each row is a bitmask whose most significant of
    /// the `width` low bits is the leftmost pixel. Zero bits are transparent.
    pub fn sprite(&mut self, x: i32, y: i32, width: u32, rows: &[u8], scale: i32, color: Color) {
        for (dy, &row) in rows.iter().enumerate() {
            for dx in 0..width {
                if row & (1 << (width - 1 - dx)) != 0 {
                    let px = x + dx as i32 * scale;
                    let py = y + dy as i32 * scale;
                    self.fill_rect(px, py, scale, scale, color);
                }
            }
        }
    }

    /// Draws text in the 3×5 font and returns the x just past its end.
    pub fn text(&mut self, x: i32, y: i32, text: &str, color: Color) -> i32 {
        let mut cursor = x;
        for c in text.chars() {
            self.sprite(cursor, y, font::SMALL_W, &font::small_glyph(c), 1, color);
            cursor += font::SMALL_ADVANCE;
        }
        cursor
    }

    /// Like [`Canvas::text`], but horizontally centred on `center_x`.
    pub fn text_centered(&mut self, center_x: i32, y: i32, text: &str, color: Color) {
        self.text(center_x - font::small_text_width(text) / 2, y, text, color);
    }

    /// Like [`Canvas::text`], but ending at `right_x` (exclusive).
    pub fn text_right(&mut self, right_x: i32, y: i32, text: &str, color: Color) {
        self.text(right_x - font::small_text_width(text), y, text, color);
    }

    // ==========================================================================
    // Pushing to the panel
    // ==========================================================================

    /// Sends every changed region of the canvas to the panel.
    pub async fn flush(&mut self, display: &mut Display) {
        // Consecutive changed rows are merged into one band whose horizontal
        // extent is the union of the rows' changed spans, so a band costs one
        // address-window setup plus its pixels.
        let mut band: Option<(usize, Range<usize>)> = None; // (first row, columns)
        for row in 0..HEIGHT {
            match (self.changed_span(row), band.take()) {
                (Some(span), Some((start, cols))) => {
                    band = Some((start, cols.start.min(span.start)..cols.end.max(span.end)));
                }
                (Some(span), None) => band = Some((row, span)),
                (None, Some((start, cols))) => self.push(display, start..row, cols).await,
                (None, None) => {}
            }
        }
        if let Some((start, cols)) = band {
            self.push(display, start..HEIGHT, cols).await;
        }
        self.front_valid = true;
    }

    /// Replaces the panel contents with the canvas using a retro "block
    /// wipe" that sweeps in `direction`.
    ///
    /// The screen is cut into a 10×10 grid of 8×8-canvas-pixel blocks. A
    /// wavefront of dithered blocks sweeps across; each block shows the dither
    /// for one step, then the new screen. Rows are staggered so the front is a
    /// chevron pointing the way the content is moving (middle rows lead).
    ///
    /// Every block is sent twice (dither, then final), so this costs about two
    /// full frames of SPI time (~230 ms), spread over the steps. That's far
    /// cheaper than a true slide, which would resend the whole screen for
    /// every animation frame.
    pub async fn transition(&mut self, display: &mut Display, direction: Sweep) {
        const BLOCK: usize = 8;
        const BLOCKS: usize = WIDTH / BLOCK;
        /// How many steps behind the leading rows each block row trails:
        /// 2 1 1 0 0 0 0 1 1 2 from top to bottom, i.e. a shallow chevron.
        /// (A deeper one, 4…0…4, read nicely but added steps.)
        const fn row_delay(row: usize) -> usize {
            (2 * row).abs_diff(BLOCKS - 1) / 4
        }
        const LAST_STEP: usize = (BLOCKS - 1) + row_delay(0) + 1;
        /// A floor on each step's duration, so steps that send little data
        /// still read as animation frames rather than a blur.
        const STEP: Duration = Duration::from_millis(10);

        let started = Instant::now();
        let mut ticker = Ticker::every(STEP);
        let column_at = |n: usize| match direction {
            Sweep::LeftToRight => n,
            Sweep::RightToLeft => BLOCKS - 1 - n,
        };
        for step in 0..=LAST_STEP {
            for block_row in 0..BLOCKS {
                // How far the dither front has got in this row, counted in
                // blocks from the side the sweep starts on.
                let Some(reached) = step.checked_sub(row_delay(block_row)) else {
                    continue;
                };
                let rows = block_row * BLOCK..(block_row + 1) * BLOCK;
                if reached < BLOCKS {
                    let x = column_at(reached) * BLOCK;
                    self.push_dither(display, rows.clone(), x..x + BLOCK).await;
                }
                if let Some(revealed) = reached.checked_sub(1)
                    && revealed < BLOCKS
                {
                    let x = column_at(revealed) * BLOCK;
                    self.push(display, rows, x..x + BLOCK).await;
                }
            }
            ticker.next().await;
        }
        defmt::debug!("transition took {}ms", started.elapsed().as_millis());
        self.front_valid = true;
    }

    /// The changed columns in `row`, relative to what the panel shows.
    fn changed_span(&self, row: usize) -> Option<Range<usize>> {
        if !self.front_valid {
            return Some(0..WIDTH);
        }
        let differs =
            |x: &usize| nibble(&self.back, row * WIDTH + x) != nibble(&self.front, row * WIDTH + x);
        let first = (0..WIDTH).find(differs)?;
        let last = (0..WIDTH)
            .rev()
            .find(differs)
            .expect("first exists, so some column differs");
        Some(first..last + 1)
    }

    /// Sends a rectangle of the canvas to the panel, and records that the
    /// panel now shows it.
    async fn push(&mut self, display: &mut Display, rows: Range<usize>, columns: Range<usize>) {
        let Self {
            back,
            front,
            theme,
            batch,
            ..
        } = self;
        stream(display, batch, rows, columns, |row, x| {
            let i = row * WIDTH + x;
            let color = nibble(back, i);
            set_nibble(front, i, color);
            theme.bytes(Color(color))
        })
        .await;
    }

    /// Fills a rectangle with a 1-canvas-pixel checkerboard of the theme's
    /// two dither colours: the classic way to fake an in-between shade on a
    /// limited palette.
    async fn push_dither(
        &mut self,
        display: &mut Display,
        rows: Range<usize>,
        columns: Range<usize>,
    ) {
        let (a, b) = (
            self.theme.bytes(Color::DITHER_A),
            self.theme.bytes(Color::DITHER_B),
        );
        stream(display, &mut self.batch, rows, columns, |row, x| {
            if (x + row).is_multiple_of(2) { a } else { b }
        })
        .await;
    }
}

/// Streams a canvas rectangle to the panel at [`SCALE`]×, with `color(row, x)`
/// giving each canvas pixel's bytes.
///
/// Rows are accumulated in `batch` and sent in as few DMA transfers as
/// possible. Each transfer has a fixed cost (setup, the nRF52832 anomaly-109
/// workaround, an interrupt, a task wake-up), and sending block-sized
/// rectangles a 48-byte line at a time made the transition *slower* than the
/// blocking driver it replaced.
async fn stream(
    display: &mut Display,
    batch: &mut [u8; BATCH],
    rows: Range<usize>,
    columns: Range<usize>,
    mut color: impl FnMut(usize, usize) -> [u8; 2],
) {
    start_pixels(display, rows.clone(), columns.clone()).await;
    let line_len = columns.len() * SCALE * 2;
    let row_len = line_len * SCALE;
    let mut used = 0;
    for row in rows {
        if used + row_len > BATCH {
            display.pixels(&batch[..used]).await;
            used = 0;
        }
        let line = &mut batch[used..used + line_len];
        for x in columns.clone() {
            let bytes = color(row, x);
            let at = (x - columns.start) * SCALE * 2;
            for px in line[at..at + SCALE * 2].as_chunks_mut::<2>().0 {
                *px = bytes;
            }
        }
        // The vertical half of the 3× scale: repeat the line.
        for copy in 1..SCALE {
            batch.copy_within(used..used + line_len, used + copy * line_len);
        }
        used += row_len;
    }
    if used > 0 {
        display.pixels(&batch[..used]).await;
    }
    display.end_pixels();
}

/// Opens a panel window covering the given canvas rectangle.
async fn start_pixels(display: &mut Display, rows: Range<usize>, columns: Range<usize>) {
    let panel = |r: Range<usize>| (r.start * SCALE) as u16..(r.end * SCALE) as u16;
    display.start_pixels(panel(columns), panel(rows)).await;
}

/// Direction a [`Canvas::transition`] sweeps across the panel.
#[derive(Clone, Copy, PartialEq, Eq, defmt::Format)]
pub enum Sweep {
    LeftToRight,
    RightToLeft,
}

/// Pixel `i` of a packed 4-bit buffer.
fn nibble(buffer: &[u8; BYTES], i: usize) -> u8 {
    if i.is_multiple_of(2) {
        buffer[i / 2] & 0x0F
    } else {
        buffer[i / 2] >> 4
    }
}

fn set_nibble(buffer: &mut [u8; BYTES], i: usize, value: u8) {
    let byte = &mut buffer[i / 2];
    *byte = if i.is_multiple_of(2) {
        (*byte & 0xF0) | value
    } else {
        (*byte & 0x0F) | (value << 4)
    };
}
