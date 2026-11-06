//! A minimal async driver for the PineTime's ST7789 240×240 LCD.
//!
//! We used `mipidsi` at first, but it is blocking-only and takes pixels as a
//! per-pixel iterator. That meant the CPU converted every pixel *and then*
//! sat idle while SPI sent it, and nothing else (touch, soon Bluetooth) could
//! run during a ~115 ms full-screen push. This driver instead takes ready-made
//! byte slices and sends them with async DMA, so the caller can build each
//! line once and send it repeatedly (our 3× vertical scaling), and the
//! executor keeps running other tasks while bytes go out.
//!
//! Only what the UI needs is implemented: init, sleep/wake, and writing a
//! rectangle of pixels.

use core::ops::Range;

use embassy_nrf::gpio::Output;
use embassy_nrf::spim::Spim;
use embassy_time::{Duration, Instant, Timer};

// ST7789 commands (datasheet section 9.1).
const SWRESET: u8 = 0x01;
const SLPIN: u8 = 0x10;
const SLPOUT: u8 = 0x11;
const NORON: u8 = 0x13;
const INVON: u8 = 0x21;
const DISPON: u8 = 0x29;
const CASET: u8 = 0x2A;
const RASET: u8 = 0x2B;
const RAMWR: u8 = 0x2C;
const MADCTL: u8 = 0x36;
const COLMOD: u8 = 0x3A;

/// The controller requires this long between entering and leaving sleep
/// (either direction) before the next sleep command.
const SLEEP_SETTLE: Duration = Duration::from_millis(120);

pub struct Display {
    spi: Spim<'static>,
    cs: Output<'static>,
    /// Low for command bytes, high for parameters and pixel data.
    dc: Output<'static>,
    /// When the last SLPIN/SLPOUT was sent, to honour [`SLEEP_SETTLE`].
    last_sleep_change: Instant,
}

impl Display {
    pub async fn new(
        spi: Spim<'static>,
        cs: Output<'static>,
        dc: Output<'static>,
        mut reset: Output<'static>,
    ) -> Self {
        // Hardware reset: ≥10 µs low, then up to 120 ms before it accepts
        // commands.
        reset.set_low();
        Timer::after_micros(20).await;
        reset.set_high();
        Timer::after_millis(120).await;
        // The reset pin must stay high from here on; leak it so it isn't
        // released when this function returns.
        core::mem::forget(reset);

        let mut display = Self {
            spi,
            cs,
            dc,
            last_sleep_change: Instant::now(),
        };
        display.command(SWRESET, &[]).await;
        Timer::after_millis(150).await;
        display.command(SLPOUT, &[]).await;
        Timer::after_millis(10).await;
        display.last_sleep_change = Instant::now();
        // 16 bits per pixel, RGB565, sent most significant byte first.
        display.command(COLMOD, &[0x55]).await;
        // Default scan direction and RGB order: row 0 at the top, and the
        // panel shows RAM rows 0–239 of the controller's 320.
        display.command(MADCTL, &[0x00]).await;
        // This panel needs inversion on, or black comes out white.
        display.command(INVON, &[]).await;
        display.command(NORON, &[]).await;
        display.command(DISPON, &[]).await;
        display
    }

    /// Low-power sleep. The controller keeps its frame memory, so what was
    /// on screen is still there after [`Display::wake`].
    pub async fn sleep(&mut self) {
        self.settle().await;
        self.command(SLPIN, &[]).await;
        self.last_sleep_change = Instant::now();
        Timer::after_millis(5).await;
    }

    pub async fn wake(&mut self) {
        self.settle().await;
        self.command(SLPOUT, &[]).await;
        self.last_sleep_change = Instant::now();
        // Commands are accepted again after 5 ms.
        Timer::after_millis(5).await;
    }

    async fn settle(&self) {
        Timer::at(self.last_sleep_change + SLEEP_SETTLE).await;
    }

    /// Starts writing pixels into the given rectangle, left to right, top to
    /// bottom. Follow with [`Display::pixels`] calls totalling exactly the
    /// rectangle's size in bytes (2 per pixel), then [`Display::end_pixels`].
    pub async fn start_pixels(&mut self, columns: Range<u16>, rows: Range<u16>) {
        let range = |r: Range<u16>| {
            let [s0, s1] = r.start.to_be_bytes();
            let [e0, e1] = (r.end - 1).to_be_bytes();
            [s0, s1, e0, e1]
        };
        self.command(CASET, &range(columns)).await;
        self.command(RASET, &range(rows)).await;
        // Leave CS asserted and DC in data mode for the pixel stream.
        self.cs.set_low();
        self.dc.set_low();
        self.write_blocking(&[RAMWR]);
        self.dc.set_high();
    }

    /// Big-endian RGB565 pixel data, continuing the current rectangle.
    pub async fn pixels(&mut self, data: &[u8]) {
        self.write(data).await;
    }

    pub fn end_pixels(&mut self) {
        self.cs.set_high();
    }

    /// Sends a command and its parameters.
    ///
    /// These transfers are 1–4 bytes, so they're sent blocking: a few µs of
    /// busy-waiting is cheaper than an async transfer's interrupt and task
    /// wake-up, and the canvas sends three commands per rectangle (hundreds
    /// per transition).
    async fn command(&mut self, command: u8, parameters: &[u8]) {
        self.cs.set_low();
        self.dc.set_low();
        self.write_blocking(&[command]);
        if !parameters.is_empty() {
            self.dc.set_high();
            self.write_blocking(parameters);
        }
        self.cs.set_high();
    }

    fn write_blocking(&mut self, data: &[u8]) {
        self.spi
            .blocking_write(data)
            .expect("SPIM writes only fail on non-RAM buffers, which embassy copies");
    }

    async fn write(&mut self, data: &[u8]) {
        // The only possible error is a buffer outside RAM, which embassy-nrf
        // handles by copying it first.
        self.spi
            .write(data)
            .await
            .expect("SPIM writes only fail on non-RAM buffers, which embassy copies");
    }
}
