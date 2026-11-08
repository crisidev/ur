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
//! Pushing takes the time it takes on the watch (see [`cost`]): the SPI
//! bus's bytes, and the driver's overhead per command and per transfer,
//! awaited on the simulator's clock as the real driver awaits its DMA.
//!
//! Only what the canvas calls is here. The rest of the real driver (sleep,
//! wake, idle mode, `show`) is the UI loop's business, which the simulator
//! mirrors in [`Panel`]'s fields instead.

use core::ops::Range;
use std::cell::RefCell;

use embassy_time::{TICK_HZ, Timer};

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
    /// Whether pushing takes time (see [`untimed`]).
    timed: bool,
    /// What the pushes so far took beyond the whole ticks already waited,
    /// in units of 1/([`TICK_HZ`] × 10⁹) s, so that rounding each push to
    /// the clock's ~30 µs ticks doesn't add up over a transition's
    /// hundreds of them.
    owed: u64,
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
        timed: true,
        owed: 0,
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
        take(cost::window()).await;
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
        take(cost::pixels(data.len())).await;
    }

    pub fn end_pixels(&mut self) {}
}

// ==============================================================================
// How long pushing takes
// ==============================================================================

/// What the real driver's calls cost, in nanoseconds: the bytes on the
/// SPI bus, and the CPU's work around them, which on the watch holds up
/// the UI loop just the same (the canvas builds each batch of pixels, then
/// waits for it to go out, then builds the next).
///
/// Measured on a dev kit, as `ur-sim --devkit` does (see `src/compare.rs`'s
/// `Push`): the time from each frame's first address window to its being
/// on the panel, for 14 frames of `devkit-compare.txt` (two runs, on
/// 2026-11-08, one with a build that kept the last pass that pushed
/// anything), from a 1-window, 216-byte tick to three 200-block
/// transitions (230 400 bytes, 449 to 499 ms). A least-squares fit of
/// time against windows, transfers, DMA chunks and bytes found that
/// transfers and bytes explain it, every frame within 16%; windows and
/// chunks added nothing measurable, so the model is:
pub mod cost {
    /// `main.rs` runs SPIM2 at 8 MHz (`spim::Frequency::M8`), the
    /// nRF52832's fastest: a byte a microsecond.
    const SPI_HZ: u64 = 8_000_000;
    const SPI_BYTE: u64 = 8 * 1_000_000_000 / SPI_HZ;

    /// `Display::start_pixels`: CASET, RASET (each a command byte and four
    /// parameter bytes) and RAMWR, sent blocking. Only their bytes: the fit
    /// found nothing more per window.
    const WINDOW: u64 = 11 * SPI_BYTE;

    /// Each `Display::pixels` call, an async DMA transfer: 1.03 ms, as
    /// fitted. That's far more than its setup, its interrupts (embassy's
    /// workaround for anomaly 109 adds one) and a task wake-up should
    /// take, and it's why a transition's 200 blocks take ~470 ms rather
    /// than the ~230 ms their bytes do. Where it goes isn't known yet.
    ///
    /// TODO: find out on the watch (a GPIO toggled round a transfer, on a
    /// logic analyser); if it's a stall, transitions could be twice as
    /// fast, and this should then shrink to match.
    const TRANSFER: u64 = 1_030_000;

    /// Each byte of pixels: 1.12 µs, as fitted. The SPI bus's 1 µs, and
    /// the CPU building the rows (`canvas::stream`), which doesn't overlap
    /// with sending them.
    const BYTE: u64 = 1_120;

    /// How long a push takes: `windows` address windows, and `bytes` of
    /// pixels in `transfers` calls.
    pub fn push(windows: u64, transfers: u64, bytes: u64) -> u64 {
        windows * WINDOW + transfers * TRANSFER + bytes * BYTE
    }

    pub fn window() -> u64 {
        push(1, 0, 0)
    }

    pub fn pixels(bytes: usize) -> u64 {
        push(0, 1, bytes as u64)
    }
}

/// Lets `nanoseconds` pass on the simulator's clock, as the watch's UI
/// task waits for its transfers: `watch::run` moves the clock on to when
/// this is done.
async fn take(nanoseconds: u64) {
    let ticks = PANEL.with_borrow_mut(|panel| {
        if !panel.timed {
            return 0;
        }
        panel.owed += nanoseconds * TICK_HZ;
        let ticks = panel.owed / 1_000_000_000;
        panel.owed %= 1_000_000_000;
        ticks
    });
    if ticks > 0 {
        Timer::after_ticks(ticks).await;
    }
}

/// Runs `push` with pushing taking no time: for pushes the watch wouldn't
/// make (a screenshot in another theme's colours, which the dev kit's
/// script renders from the canvas instead), so they don't move the clock.
pub fn untimed<R>(push: impl FnOnce() -> R) -> R {
    PANEL.with_borrow_mut(|panel| panel.timed = false);
    let result = push();
    PANEL.with_borrow_mut(|panel| panel.timed = true);
    result
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
