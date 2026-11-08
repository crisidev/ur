//! A stand-in for the ST7789 panel (`src/display.rs`): what
//! [`crate::canvas::Canvas`] pushes lands in a 240×240 frame of RGB565
//! pixels, exactly the bytes the real panel would get. Screenshots and the
//! window read that frame, so they show what the panel would, transitions
//! and all, rather than a re-rendering of the canvas.
//!
//! The panel's state lives in a thread-local rather than in [`Display`],
//! because a transition holds the `&mut Display` for its whole duration,
//! and the window still wants to show its frames as they go.
//!
//! Only what the canvas calls is here. The rest of the real driver (sleep,
//! wake, idle mode, `show`) is the UI loop's business, which the simulator
//! mirrors in [`Panel`]'s fields instead.

use core::ops::Range;
use std::cell::RefCell;

pub const SIZE: usize = 240;

/// What the panel shows, and what it's been told.
pub struct Panel {
    /// RGB565, row-major.
    pub pixels: Vec<u16>,
    /// The address window of the pixels being written, and the next one.
    columns: Range<u16>,
    rows: Range<u16>,
    cursor: usize,
    /// A half-pixel left over from the last write: `pixels` writes come in
    /// arbitrary byte counts.
    pending: Option<u8>,
    /// Idle mode, which the UI loop uses for the dim look: only the top
    /// bit of each of red, green, and blue survives (see
    /// `display::Display::set_idle`).
    pub idle: bool,
    /// The backlight, as the UI loop last set it (`OFF`: nothing shows),
    /// mirrored here like `idle` for recordings (see `crate::record`),
    /// which see the screen as the wearer would.
    pub backlight: crate::brightness::Level,
}

thread_local! {
    pub static PANEL: RefCell<Panel> = RefCell::new(Panel {
        // The panel's memory holds noise after power-up; a dark grey
        // stands in for it until the first frame sweeps in.
        pixels: vec![0x2104; SIZE * SIZE],
        columns: 0..0,
        rows: 0..0,
        cursor: 0,
        pending: None,
        idle: false,
        backlight: crate::brightness::Level::OFF,
    });
}

/// The handle the canvas pushes through.
pub struct Display;

impl Display {
    /// Opens a window for the pixels that follow (CASET, RASET, RAMWR).
    pub async fn start_pixels(&mut self, columns: Range<u16>, rows: Range<u16>) {
        PANEL.with_borrow_mut(|panel| {
            panel.columns = columns;
            panel.rows = rows;
            panel.cursor = 0;
            panel.pending = None;
        });
    }

    /// Big-endian RGB565 pixels, filling the window row by row.
    pub async fn pixels(&mut self, data: &[u8]) {
        PANEL.with_borrow_mut(|panel| {
            for &byte in data {
                let Some(high) = panel.pending.take() else {
                    panel.pending = Some(byte);
                    continue;
                };
                let width = panel.columns.len();
                let (dx, dy) = (panel.cursor % width, panel.cursor / width);
                let (x, y) = (
                    usize::from(panel.columns.start) + dx,
                    usize::from(panel.rows.start) + dy,
                );
                if x < SIZE && y < SIZE && dy < panel.rows.len() {
                    panel.pixels[y * SIZE + x] = u16::from_be_bytes([high, byte]);
                }
                panel.cursor += 1;
            }
        });
    }

    pub fn end_pixels(&mut self) {}
}

/// Expands an RGB565 pixel to 8 bits a channel, as `scripts/screenshot.py`
/// does (the top bits repeated into the bottom), so screenshots from either
/// match.
pub fn rgb888(pixel: u16, idle: bool) -> [u8; 3] {
    let (r, g, b) = (pixel >> 11, (pixel >> 5) & 0x3F, pixel & 0x1F);
    if idle {
        let bit = |value: u16, top: u16| if value & top != 0 { 255 } else { 0 };
        return [bit(r, 0x10), bit(g, 0x20), bit(b, 0x10)];
    }
    [
        (r << 3 | r >> 2) as u8,
        (g << 2 | g >> 4) as u8,
        (b << 3 | b >> 2) as u8,
    ]
}
