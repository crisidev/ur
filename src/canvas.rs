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

pub use self::marquee::{MARQUEE_FRAME, marquee_cycle, marquee_offset};

use crate::display::Display;
use crate::font;
use crate::qr;
use crate::theme::{self, Theme};

pub use crate::theme::Color;

pub const WIDTH: usize = 80;
pub const HEIGHT: usize = 80;
/// Panel pixels per canvas pixel, in each direction.
pub const SCALE: usize = 3;

/// Two pixels per byte (4-bit colour roles), low nibble first.
const BYTES: usize = WIDTH * HEIGHT / 2;
/// One full-width canvas row of panel pixel bytes: the most [`stream`]
/// needs to send any rectangle whole rows at a time, and enough for a
/// transition's 8×8 blocks (1152 bytes) in one transfer. Two rows (as it
/// once was) halved the transfers of a full-width flush, but cost 1.4 KB of
/// RAM to save ~40 transfers' setup on the rare full-frame push.
const BATCH: usize = WIDTH * SCALE * SCALE * 2;

/// `repr(C)`, so `scripts/screenshot.py` can find `front` and `theme` (see
/// [`crate::debug`]).
#[repr(C)]
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
    /// A QR code to show over the canvas this frame (see [`Canvas::qr`]),
    /// if `qr_wanted`; and whether the panel shows it. Before `batch`, at
    /// a fixed place for `scripts/screenshot.py`.
    qr: qr::Matrix,
    qr_wanted: bool,
    qr_shown: bool,
    /// Pixel bytes waiting to be sent; see [`stream`].
    batch: [u8; BATCH],
    /// While set, drawing only touches these columns (for [`Canvas::marquee`]).
    clip: Option<Range<i32>>,
}

impl Canvas {
    pub const fn new() -> Self {
        Self {
            back: [0; BYTES],
            front: [0; BYTES],
            front_valid: false,
            theme: &theme::ARCADE,
            qr: [0; qr::SIZE],
            qr_wanted: false,
            qr_shown: false,
            batch: [0; BATCH],
            clip: None,
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

    /// Clears the frame, starting a new one: every screen starts here.
    pub fn clear(&mut self, color: Color) {
        self.back.fill(color.0 | (color.0 << 4));
        self.qr_wanted = false;
    }

    /// Shows `matrix` over this frame, at the panel's own resolution
    /// rather than the canvas's: a QR code drawn on the canvas would have
    /// modules of 3 or 6 panel pixels, where the panel fits 7, and every
    /// pixel of a module helps a phone's camera. Dark on white, whatever
    /// the theme, as scanners expect, with the standard quiet zone of four
    /// modules: 33 × 7 = 231 pixels, nearly the whole panel.
    ///
    /// It's sent after the canvas, once, while the frames that want it
    /// leave the canvas beneath it unchanged; whatever the canvas does send
    /// meanwhile (a banner, say) it's drawn again over. The first frame
    /// without it repaints the panel.
    pub fn qr(&mut self, matrix: &qr::Matrix) {
        self.qr = *matrix;
        self.qr_wanted = true;
    }

    /// Sets one pixel. Out-of-bounds coordinates are silently clipped, so
    /// callers can draw shapes that hang off the edge.
    pub fn pixel(&mut self, x: i32, y: i32, color: Color) {
        if !(0..WIDTH as i32).contains(&x) || !(0..HEIGHT as i32).contains(&y) {
            return;
        }
        if self.clip.as_ref().is_some_and(|clip| !clip.contains(&x)) {
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

    /// Like [`Canvas::sprite`], for the 16-wide turn arrows (see
    /// [`font::TURN_W`]), optionally mirrored left to right: the left turns
    /// are the right ones' mirror images.
    pub fn sprite16(
        &mut self,
        x: i32,
        y: i32,
        rows: &[u16],
        mirror: bool,
        scale: i32,
        color: Color,
    ) {
        for (dy, &row) in rows.iter().enumerate() {
            let row = if mirror { row.reverse_bits() } else { row };
            for dx in 0..16 {
                if row & (1 << (15 - dx)) != 0 {
                    self.fill_rect(x + dx * scale, y + dy as i32 * scale, scale, scale, color);
                }
            }
        }
    }

    /// Draws text in the 3×5 font and returns the x just past its end.
    pub fn text(&mut self, x: i32, y: i32, text: &str, color: Color) -> i32 {
        self.text_scaled(x, y, text, color, 1)
    }

    /// Text in the small font at `scale`× (each font pixel a `scale`-sided
    /// square); returns the x just past its end. `y` is the top of the
    /// capitals; accents extend above it.
    pub fn text_scaled(&mut self, x: i32, y: i32, text: &str, color: Color, scale: i32) -> i32 {
        let mut cursor = x;
        let top = y - font::ACCENT_ROWS * scale;
        for c in text.chars() {
            let glyph = font::glyph(c);
            self.sprite(cursor, top, glyph.width, &glyph.rows, scale, color);
            cursor += (glyph.width as i32 + 1) * scale;
        }
        cursor
    }

    /// Text confined to `columns`. If it doesn't fit, it scrolls:
    /// a pause at the start, a steady crawl to the end, a pause there, and
    /// round again. `elapsed` is how long the text has been showing.
    pub fn marquee(
        &mut self,
        columns: Range<i32>,
        y: i32,
        text: &str,
        color: Color,
        scale: i32,
        elapsed: Duration,
    ) {
        let overflow = font::small_text_width(text) * scale - columns.len() as i32;
        let offset = if overflow > 0 {
            marquee_offset(overflow, elapsed)
        } else {
            0
        };
        let left = columns.start;
        self.clip = Some(columns);
        self.text_scaled(left - offset, y, text, color, scale);
        self.clip = None;
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
        // Whether anything was sent, which may have covered a QR code, and
        // whether everything was.
        let mut sent = false;
        let whole = !self.front_valid;
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
                (None, Some((start, cols))) => {
                    self.push(display, start..row, cols).await;
                    sent = true;
                }
                (None, None) => {}
            }
        }
        if let Some((start, cols)) = band {
            self.push(display, start..HEIGHT, cols).await;
            sent = true;
        }
        self.front_valid = true;
        self.overlay(display, sent, whole).await;
    }

    /// Draws the QR code over the panel if this frame wants it and the
    /// panel doesn't show it whole (`covered`: the canvas was just sent
    /// over some of it); or, if the frame doesn't want it and the panel
    /// shows it, repaints the canvas over it (unless it just was, `whole`).
    async fn overlay(&mut self, display: &mut Display, covered: bool, whole: bool) {
        const MODULE: usize = 7;
        const MODULES: usize = qr::SIZE + 2 * 4;
        const SIDE: usize = MODULES * MODULE;
        const ORIGIN: usize = (WIDTH * SCALE - SIDE) / 2;
        match (self.qr_wanted, self.qr_shown) {
            (true, shown) if !shown || covered => {
                let span = |at: usize| at as u16..(at + SIDE) as u16;
                display.start_pixels(span(ORIGIN), span(ORIGIN)).await;
                let row_bytes = SIDE * 2;
                let mut used = 0;
                for y in 0..SIDE {
                    if used + row_bytes > BATCH {
                        display.pixels(&self.batch[..used]).await;
                        used = 0;
                    }
                    // Modules from the quiet zone's corner.
                    let my = (y / MODULE).wrapping_sub(4);
                    let line = &mut self.batch[used..used + row_bytes];
                    for (x, px) in line.as_chunks_mut::<2>().0.iter_mut().enumerate() {
                        let mx = (x / MODULE).wrapping_sub(4);
                        let dark = my < qr::SIZE && mx < qr::SIZE && self.qr[my] >> mx & 1 != 0;
                        *px = if dark { [0x00, 0x00] } else { [0xFF, 0xFF] };
                    }
                    used += row_bytes;
                }
                display.pixels(&self.batch[..used]).await;
                display.end_pixels();
                self.qr_shown = true;
            }
            (false, true) => {
                self.qr_shown = false;
                if !whole {
                    self.push(display, 0..HEIGHT, 0..WIDTH).await;
                }
            }
            _ => {}
        }
    }

    /// Replaces the panel contents with the canvas using a retro "block
    /// wipe" that sweeps in `direction`.
    ///
    /// The screen is cut into a 10×10 grid of 8×8-canvas-pixel blocks. A
    /// wavefront of dithered blocks sweeps across (or down, or up); each
    /// block shows the dither for one step, then the new screen. The lanes
    /// the front moves along (rows for a sideways sweep, columns for a
    /// vertical one) are staggered so the front is a chevron pointing the
    /// way the content is moving (middle lanes lead).
    ///
    /// Every block is sent twice (dither, then final), so this costs about two
    /// full frames of SPI time (~230 ms), spread over the steps. That's far
    /// cheaper than a true slide, which would resend the whole screen for
    /// every animation frame.
    pub async fn transition(&mut self, display: &mut Display, direction: Sweep) {
        const BLOCK: usize = 8;
        const BLOCKS: usize = WIDTH / BLOCK;
        /// How many steps behind the leading lanes each lane of blocks
        /// trails: 2 1 1 0 0 0 0 1 1 2 from one edge to the other, i.e. a
        /// shallow chevron. (A deeper one, 4…0…4, read nicely but added
        /// steps.)
        const fn lane_delay(lane: usize) -> usize {
            (2 * lane).abs_diff(BLOCKS - 1) / 4
        }
        const LAST_STEP: usize = (BLOCKS - 1) + lane_delay(0) + 1;
        /// A floor on each step's duration, so steps that send little data
        /// still read as animation frames rather than a blur.
        const STEP: Duration = Duration::from_millis(10);

        let started = Instant::now();
        let mut ticker = Ticker::every(STEP);
        // The canvas rows and columns of the `n`th block along `lane`,
        // counted from the side the sweep starts on.
        let block = |lane: usize, n: usize| {
            let (row, column) = match direction {
                Sweep::LeftToRight => (lane, n),
                Sweep::RightToLeft => (lane, BLOCKS - 1 - n),
                Sweep::TopToBottom => (n, lane),
                Sweep::BottomToTop => (BLOCKS - 1 - n, lane),
            };
            (
                row * BLOCK..(row + 1) * BLOCK,
                column * BLOCK..(column + 1) * BLOCK,
            )
        };
        for step in 0..=LAST_STEP {
            for lane in 0..BLOCKS {
                // How far the dither front has got along this lane.
                let Some(reached) = step.checked_sub(lane_delay(lane)) else {
                    continue;
                };
                if reached < BLOCKS {
                    let (rows, columns) = block(lane, reached);
                    self.push_dither(display, rows, columns).await;
                }
                if let Some(revealed) = reached.checked_sub(1)
                    && revealed < BLOCKS
                {
                    let (rows, columns) = block(lane, revealed);
                    self.push(display, rows, columns).await;
                }
            }
            ticker.next().await;
        }
        defmt::debug!("transition took {}ms", started.elapsed().as_millis());
        self.front_valid = true;
        // Every block was sent.
        self.overlay(display, true, true).await;
    }

    /// The changed columns in `row`, relative to what the panel shows.
    fn changed_span(&self, row: usize) -> Option<Range<usize>> {
        if !self.front_valid {
            return Some(0..WIDTH);
        }
        // Most rows don't change from one frame to the next: compare them
        // whole (a row is whole bytes, as `WIDTH` is even) before looking
        // for where they differ a pixel at a time. Built for size, the
        // per-pixel comparison is a function call per pixel, and scanning
        // every row that way made a flush take a third longer.
        let bytes = row * WIDTH / 2..(row + 1) * WIDTH / 2;
        if self.back[bytes.clone()] == self.front[bytes] {
            return None;
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

/// Direction a [`Canvas::transition`] sweeps across the panel: the way the
/// content moves, so the new screen comes in from where the sweep starts.
#[derive(Clone, Copy, PartialEq, Eq, defmt::Format)]
pub enum Sweep {
    LeftToRight,
    RightToLeft,
    TopToBottom,
    BottomToTop,
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

// ==============================================================================
// Scrolling text
// ==============================================================================

mod marquee {
    use embassy_time::Duration;

    /// Scroll speed, in canvas pixels per second (about 2.5 characters).
    const SPEED: u64 = 10;
    /// Hold still this long at each end, so the start and end can be read.
    const PAUSE: Duration = Duration::from_millis(1200);
    /// How often to redraw while text scrolls: one pixel per frame.
    pub const MARQUEE_FRAME: Duration = Duration::from_millis(1000 / SPEED);

    /// How long one full pause–scroll–pause cycle takes for text that
    /// overflows its space by `overflow` pixels.
    pub fn marquee_cycle(overflow: i32) -> Duration {
        PAUSE * 2 + Duration::from_millis(overflow.max(0) as u64 * 1000 / SPEED)
    }

    /// How far to have scrolled after `elapsed`.
    pub fn marquee_offset(overflow: i32, elapsed: Duration) -> i32 {
        let travel = marquee_cycle(overflow) - PAUSE * 2;
        let t = Duration::from_ticks(elapsed.as_ticks() % marquee_cycle(overflow).as_ticks());
        if t < PAUSE {
            0
        } else if t < PAUSE + travel {
            ((t - PAUSE).as_millis() * SPEED / 1000) as i32
        } else {
            overflow
        }
    }
}
