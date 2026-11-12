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
//! Only what the UI needs is implemented: init, sleep/wake, idle mode, and
//! writing a rectangle of pixels.
//!
//! The panel draws ~6 mA while awake (datasheet, at its defaults), as much
//! as the backlight at its usual level, so it's set up to draw less, as
//! InfiniTime does (on every panel revision PINE64 has shipped, including
//! the ST7789P3 of 2026):
//!
//! - **Lower supply voltages** at init: the lowest AVDD/AVCL/VDDS
//!   (PWCTRL1), gate voltages (GCTRL), and booster clocks (PWCTRL2).
//!   InfiniTime's VDV (VDVS) too, which also evens out the step from black
//!   to the darkest greys.
//! - **Idle mode** for the dim white-on-black look ([`Display::set_idle`]):
//!   8 colours, refreshed at ~8 Hz instead of 60.
//!
//! The SPI bus is shared with the SPI flash (see [`crate::spi`]), so the
//! driver locks it per command, and for the whole of each rectangle of pixels
//! (from [`Display::start_pixels`] to [`Display::end_pixels`]). Flash
//! operations interleave between rectangles.

use core::ops::Range;

use embassy_nrf::gpio::Output;
use embassy_time::{Duration, Instant, Timer};

use crate::spi;

// ST7789 commands (datasheet section 9.1).
const SWRESET: u8 = 0x01;
const SLPIN: u8 = 0x10;
const SLPOUT: u8 = 0x11;
const NORON: u8 = 0x13;
const INVON: u8 = 0x21;
const DISPOFF: u8 = 0x28;
const DISPON: u8 = 0x29;
const CASET: u8 = 0x2A;
const RASET: u8 = 0x2B;
const RAMWR: u8 = 0x2C;
const MADCTL: u8 = 0x36;
const IDMOFF: u8 = 0x38;
const IDMON: u8 = 0x39;
const COLMOD: u8 = 0x3A;
// Command table 2 (datasheet section 9.2), which needs CMD2EN.
const PORCTRL: u8 = 0xB2;
const FRCTRL1: u8 = 0xB3;
const GCTRL: u8 = 0xB7;
const VDVS: u8 = 0xC4;
const PWCTRL1: u8 = 0xD0;
const CMD2EN: u8 = 0xDF;
const PWCTRL2: u8 = 0xE8;

/// FRCTRL1 in idle mode: its own frame rate (FRSEN), divided by 4, from
/// RTNB 0x1E and the idle porches below: 10 MHz / ((320 + 108) × (250 +
/// 30 × 16)) / 4 ≈ 8 Hz. InfiniTime's always-on display uses the same.
const FRCTRL1_IDLE: [u8; 3] = [0x12, 0x1E, 0x1E];
/// FRCTRL1 out of idle mode: the defaults. InfiniTime found the divider
/// applies in normal mode too (the datasheet says it shouldn't), so it's
/// switched with idle mode rather than set once.
const FRCTRL1_NORMAL: [u8; 3] = [0x00, 0x0F, 0x0F];

/// The controller requires this long between entering and leaving sleep
/// (either direction) before the next sleep command.
const SLEEP_SETTLE: Duration = Duration::from_millis(120);
/// One panel refresh at the controller's default 60 Hz frame rate, with
/// margin: how long after a change in frame memory (or DISPON) until every
/// row shows it. The panel scans top to bottom, so the bottom lags.
pub const REFRESH: Duration = Duration::from_millis(20);

/// MADCTL's row/column exchange (MV): the controller swaps its column and
/// row address counters, so pixels fill a window column by column rather
/// than row by row (see [`Order`]).
#[cfg(feature = "game")]
const MADCTL_MV: u8 = 0x20;

/// The order the pixels of a window are sent in (see
/// [`Display::set_order`]).
#[cfg(feature = "game")]
#[derive(Clone, Copy, PartialEq, Eq, defmt::Format)]
pub enum Order {
    /// Left to right, then top to bottom: what the canvas sends, and what
    /// every other part of the firmware assumes.
    Rows,
    /// Top to bottom, then left to right: a raycaster's column of pixels
    /// is one run of the stream (see `crate::game`).
    Columns,
}

/// How pixels are sent (COLMOD; see [`Display::set_format`]).
#[cfg(feature = "game")]
#[derive(Clone, Copy, PartialEq, Eq, defmt::Format)]
pub enum Format {
    /// Two bytes a pixel, big-endian: what the canvas sends, and what
    /// every other part of the firmware assumes.
    Rgb565,
    /// Three bytes to two pixels (`RG BR GB`), 4 bits a channel: a
    /// quarter less to send.
    Rgb444,
}

#[cfg(feature = "game")]
impl Format {
    /// The bytes `pixels` pixels take, an even number of them in
    /// [`Format::Rgb444`].
    pub const fn bytes(self, pixels: usize) -> usize {
        match self {
            Self::Rgb565 => pixels * 2,
            Self::Rgb444 => pixels * 3 / 2,
        }
    }
}

pub struct Display {
    /// The bus, while we're using it (see the module docs).
    held: Option<spi::Guard>,
    /// See [`Display::set_order`] and [`Display::set_format`].
    #[cfg(feature = "game")]
    order: Order,
    #[cfg(feature = "game")]
    format: Format,
    cs: Output<'static>,
    /// Low for command bytes, high for parameters and pixel data.
    dc: Output<'static>,
    /// When the last SLPIN/SLPOUT was sent, to honour [`SLEEP_SETTLE`].
    last_sleep_change: Instant,
    /// Whether it's in idle mode (see [`Display::set_idle`]).
    idle: bool,
}

impl Display {
    pub async fn new(cs: Output<'static>, dc: Output<'static>, mut reset: Output<'static>) -> Self {
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
            held: None,
            #[cfg(feature = "game")]
            order: Order::Rows,
            #[cfg(feature = "game")]
            format: Format::Rgb565,
            cs,
            dc,
            last_sleep_change: Instant::now(),
            idle: false,
        };
        display.command(SWRESET, &[]).await;
        Timer::after_millis(150).await;
        // Configure everything while still asleep, and leave sleep last:
        // InfiniTime moved SLPOUT there to fix a rotation problem on the
        // ST7789P3, and it's as good an order on the older controllers.
        //
        // The voltages below (PWCTRL1 onwards) are in command table 2,
        // which needs enabling first.
        display.command(CMD2EN, &[0x5A, 0x69, 0x02, 0x01]).await;
        // 16 bits per pixel, RGB565, sent most significant byte first.
        display.command(COLMOD, &[0x55]).await;
        // Default scan direction and RGB order: row 0 at the top, and the
        // panel shows RAM rows 0–239 of the controller's 320.
        display.command(MADCTL, &[0x00]).await;
        // This panel needs inversion on, or black comes out white.
        display.command(INVON, &[]).await;
        // Porches: the default 12 lines each in normal mode (so its 60 Hz
        // and [`REFRESH`] hold), and InfiniTime's longer ones in idle mode
        // (with PSEN, separate porches per mode), towards its ~8 Hz.
        display
            .command(PORCTRL, &[0x0C, 0x0C, 0x01, 0xED, 0xED])
            .await;
        display.command(FRCTRL1, &FRCTRL1_NORMAL).await;
        display.command(NORON, &[]).await;
        // VDV −0.4 V (default 0 V): InfiniTime's, for the black-to-grey
        // step.
        display.command(VDVS, &[0x10]).await;
        // The lowest AVDD (6.4 V), AVCL (−4.4 V), and VDDS (2.19 V); the
        // defaults are 6.8, −4.8, and 2.3 V.
        display.command(PWCTRL1, &[0xA4, 0x00]).await;
        // The slowest booster clocks (default 0x93).
        display.command(PWCTRL2, &[0xB3]).await;
        // The lowest VGH (12.2 V) and VGL (−7.16 V); the defaults are 13.26
        // and −10.43 V.
        display.command(GCTRL, &[0x00]).await;
        display.command(SLPOUT, &[]).await;
        Timer::after_millis(10).await;
        display.last_sleep_change = Instant::now();
        display.command(DISPON, &[]).await;
        display
    }

    /// Low-power sleep. The controller keeps its frame memory, so what was
    /// on screen is still there after [`Display::wake`].
    ///
    /// The output is blanked first (DISPOFF) and stays blank until
    /// [`Display::show`]. Without that, the panel picked up where it left
    /// off on waking: the backlight came on for a glance while the lower
    /// rows still showed the frame from before sleep (the light theme's
    /// background, or the clock face's weekday and heart rate), until the
    /// panel's first scan reached them.
    pub async fn sleep(&mut self) {
        self.settle().await;
        self.command(DISPOFF, &[]).await;
        self.command(SLPIN, &[]).await;
        self.last_sleep_change = Instant::now();
        Timer::after_millis(5).await;
    }

    /// Leaves sleep, with the output still blank: draw the new frame, then
    /// [`Display::show`] it.
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

    /// Unblanks the output after [`Display::wake`], once the panel's drive
    /// has powered up (the same 120 ms the controller asks between sleep
    /// changes), and returns once a whole refresh has shown the frame
    /// memory, so the backlight can come on over a finished frame.
    pub async fn show(&mut self) {
        self.settle().await;
        self.command(DISPON, &[]).await;
        Timer::after(REFRESH).await;
    }

    /// Into or out of idle mode: 8 colours (each of red, green, and blue
    /// only on or off, from its most significant bit), at a much slower
    /// refresh (see [`FRCTRL1_IDLE`]), for less current. For the dim
    /// white-on-black look, which uses only white and red of them, and a
    /// grey made of white and black columns (see retro's, `crate::themes::retro::GLANCE`).
    ///
    /// Switch only with a finished frame on the panel, or with the
    /// backlight off: at ~8 Hz, a new frame takes ~125 ms to reach the
    /// bottom rows, rather than [`REFRESH`]. So the UI loop leaves idle
    /// mode before drawing a frame to show with the backlight off (waking,
    /// or going into or out of the dim look), shows it at the normal frame
    /// rate, and only then enters idle mode.
    ///
    /// The mode survives sleep. Does nothing if already in that mode.
    pub async fn set_idle(&mut self, idle: bool) {
        if idle == self.idle {
            return;
        }
        if idle {
            self.command(IDMON, &[]).await;
            self.command(FRCTRL1, &FRCTRL1_IDLE).await;
        } else {
            self.command(IDMOFF, &[]).await;
            self.command(FRCTRL1, &FRCTRL1_NORMAL).await;
        }
        self.idle = idle;
        defmt::debug!("display: idle mode {}", idle);
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
        self.held = Some(spi::BUS.get().await.lock().await);
        #[cfg(feature = "compare")]
        crate::compare::window();
        // With the counters exchanged, CASET's addresses run down the
        // panel and RASET's across it.
        #[cfg(feature = "game")]
        let (columns, rows) = match self.order {
            Order::Rows => (columns, rows),
            Order::Columns => (rows, columns),
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
        #[cfg(feature = "compare")]
        crate::compare::pixels(data.len());
    }

    pub fn end_pixels(&mut self) {
        self.cs.set_high();
        self.held = None;
    }

    /// Switches the order windows are filled in (MADCTL's MV bit). The
    /// panel's contents stay as they are: only what's sent from now on
    /// lands differently. Windows keep their meaning (`columns` across
    /// the panel, `rows` down it) either way; only the order of the
    /// pixels within one changes.
    ///
    /// Everything but the game sends [`Order::Rows`], so whatever switches
    /// to [`Order::Columns`] switches back before anything else draws.
    /// The canvas checks (see `Canvas::flush`).
    #[cfg(feature = "game")]
    pub async fn set_order(&mut self, order: Order) {
        if order == self.order {
            return;
        }
        let madctl = match order {
            Order::Rows => 0x00,
            Order::Columns => MADCTL_MV,
        };
        self.command(MADCTL, &[madctl]).await;
        self.order = order;
        defmt::debug!("display: {} order", order);
    }

    #[cfg(feature = "game")]
    pub fn order(&self) -> Order {
        self.order
    }

    /// Switches how pixels are sent (COLMOD). Like [`Display::set_order`],
    /// what switches away from [`Format::Rgb565`] switches back before
    /// anything else draws; a reset (which runs [`Display::new`]) does
    /// too, and sleep keeps it.
    #[cfg(feature = "game")]
    pub async fn set_format(&mut self, format: Format) {
        if format == self.format {
            return;
        }
        let colmod = match format {
            Format::Rgb565 => 0x55,
            Format::Rgb444 => 0x53,
        };
        self.command(COLMOD, &[colmod]).await;
        self.format = format;
        defmt::debug!("display: {}", format);
    }

    #[cfg(feature = "game")]
    pub fn format(&self) -> Format {
        self.format
    }

    /// Sends one column's run of pixels each, in [`Order::Columns`], for
    /// every column in `columns`: `fill` makes column `x`'s, into the
    /// start of the buffer it's given, and says which rows it made (an
    /// even number of them in [`Format::Rgb444`]; none sends nothing).
    /// It makes each while the one before goes out: two buffers,
    /// `buffers`, each big enough for a whole column, take turns.
    ///
    /// Each column is a window of its own, with the bus held only for
    /// it, so the SPI flash, and every other task, gets turns between
    /// them. A window costs its commands (11 bytes, sent blocking), so
    /// only for runs that vary in where they are, as a raycaster's do.
    ///
    /// SPIM's DMA moves at most 255 bytes at a time (embassy-nrf starts
    /// each piece once the last is done, from its interrupt's wake-up),
    /// and `fill` runs without yielding: a column's first piece goes out
    /// while the next is made, and the rest after.
    #[cfg(feature = "game")]
    pub async fn stream_columns(
        &mut self,
        columns: Range<u16>,
        buffers: (&mut [u8], &mut [u8]),
        mut fill: impl FnMut(u16, &mut [u8]) -> Range<u16>,
    ) {
        debug_assert!(self.order == Order::Columns);
        let (mut sending, mut making) = buffers;
        let Some(mut x) = columns.clone().next() else {
            return;
        };
        let mut rows = fill(x, sending);
        loop {
            let bytes = self.format.bytes(rows.len());
            debug_assert!(self.format != Format::Rgb444 || rows.len().is_multiple_of(2));
            let next = x + 1;
            let more = next < columns.end;
            // The next column is made while this one goes out. But not at
            // once: on the nRF52832, embassy-nrf's workaround for anomaly
            // 109 makes a transfer's first poll start a transfer of
            // nothing, and only the poll after its STARTED interrupt
            // starts the real one. Yielding first gets that poll in before
            // `fill` holds the CPU.
            let make = async {
                embassy_futures::yield_now().await;
                more.then(|| fill(next, &mut *making))
            };
            let next_rows = if rows.is_empty() {
                make.await
            } else {
                self.start_pixels(x..next, rows).await;
                let bus = self
                    .held
                    .as_mut()
                    .expect("the bus is locked around every transfer");
                let (_, next_rows) =
                    embassy_futures::join::join(spi::write(bus, &sending[..bytes]), make).await;
                #[cfg(feature = "compare")]
                crate::compare::pixels(bytes);
                self.end_pixels();
                next_rows
            };
            let Some(next_rows) = next_rows else {
                return;
            };
            core::mem::swap(&mut sending, &mut making);
            (x, rows) = (next, next_rows);
        }
    }

    /// Sends a command and its parameters.
    ///
    /// These transfers are 1–4 bytes, so they're sent blocking: a few µs of
    /// busy-waiting is cheaper than an async transfer's interrupt and task
    /// wake-up, and the canvas sends three commands per rectangle (hundreds
    /// per transition).
    async fn command(&mut self, command: u8, parameters: &[u8]) {
        // Inside a rectangle the bus is already ours; otherwise hold it for
        // just this command.
        let lock_here = self.held.is_none();
        if lock_here {
            self.held = Some(spi::BUS.get().await.lock().await);
        }
        self.cs.set_low();
        self.dc.set_low();
        self.write_blocking(&[command]);
        if !parameters.is_empty() {
            self.dc.set_high();
            self.write_blocking(parameters);
        }
        self.cs.set_high();
        if lock_here {
            self.held = None;
        }
    }

    fn write_blocking(&mut self, data: &[u8]) {
        let bus = self
            .held
            .as_mut()
            .expect("the bus is locked around every transfer");
        bus.blocking_write(data).expect(spi::WRITE_CANNOT_FAIL);
    }

    async fn write(&mut self, data: &[u8]) {
        let bus = self
            .held
            .as_mut()
            .expect("the bus is locked around every transfer");
        spi::write(bus, data).await;
    }
}
