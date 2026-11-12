//! A raycaster game (the `game` feature), on a screen of its own at the
//! end of the line below the clock (see `crate::ui`).
//!
//! The screen shows a title until tapped; then the game plays in a
//! 162×120 viewport in the middle of the panel, until the button pauses
//! it (back to the title; the button again goes to the clock, as
//! everywhere). Whatever else wants the screen (a call, an alarm, "find
//! my watch", pairing, an update, a notification's banner, the screen
//! going off) pauses it too, as does [`IDLE`] without a finger on it.
//!
//! The viewport doesn't go through the canvas, which would cost a
//! framebuffer: the canvas draws what's around it once (see [`draw`]),
//! then each frame is sent straight to the panel a column at a time
//! ([`frame`]), each column made while the one before goes out, in two
//! buffers borrowed from the canvas ([`Canvas::lend`]). For that the
//! panel takes pixels column by column ([`Order::Columns`]), so that a
//! column is one 240-byte transfer, under SPIM's 255-byte DMA limit; the
//! UI loop switches back before the canvas draws again.
//!
//! For now, the frame is a test pattern, to check the panel's order and
//! measure what the bus allows, with the last second's frame rate and
//! times drawn over it ([`Stats`]), for a sealed watch without a probe
//! to read the log.
//!
//! [`Order::Columns`]: crate::display::Order::Columns

use core::ops::Range;

use embassy_time::{Duration, Instant};

use crate::canvas::{self, Canvas, Color};
use crate::display::Display;
use crate::font;

/// The viewport, in panel pixels: centred, and on the canvas's 3× grid,
/// so that [`draw`] can frame it exactly.
pub const VIEW_X: Range<u16> = 39..201;
pub const VIEW_Y: Range<u16> = 60..180;
const VIEW_W: usize = (VIEW_X.end - VIEW_X.start) as usize;
const VIEW_H: usize = (VIEW_Y.end - VIEW_Y.start) as usize;
/// One column of the viewport, in bytes.
const COLUMN: usize = VIEW_H * 2;
/// Columns to a strip of [`Display::stream`]. One is a strip per DMA
/// transfer (at most 255 bytes), so that the whole of it goes out while
/// the next is made; more save each transfer's overhead, but stall after
/// the first 255 bytes until the next is made. Two strips must fit in
/// the canvas's batch (see [`Canvas::lend`]): up to 3.
const STRIP_COLUMNS: usize = 1;
const STRIP: usize = COLUMN * STRIP_COLUMNS;
const _: () = assert!(VIEW_W.is_multiple_of(STRIP_COLUMNS));
const _: () = assert!(2 * STRIP <= canvas::LENT, "two strips in the batch");
const _: () = assert!((VIEW_X.start as usize).is_multiple_of(canvas::SCALE));
const _: () = assert!((VIEW_X.end as usize).is_multiple_of(canvas::SCALE));
const _: () = assert!((VIEW_Y.start as usize).is_multiple_of(canvas::SCALE));
const _: () = assert!((VIEW_Y.end as usize).is_multiple_of(canvas::SCALE));

/// The viewport in canvas pixels.
const CELL_X: Range<i32> = VIEW_X.start as i32 / 3..VIEW_X.end as i32 / 3;
const CELL_Y: Range<i32> = VIEW_Y.start as i32 / 3..VIEW_Y.end as i32 / 3;

/// How long without a finger on the screen pauses the game. The screen
/// then goes off as after any input, `ui`'s timeout later.
pub const IDLE: Duration = Duration::from_secs(30);

/// The least time between frames: the UI loop's timer, so that every
/// other task gets a turn between them, beyond the transfers' awaits.
const FRAME_GAP: Duration = Duration::from_millis(1);

/// The game's state, in the UI's [`crate::ui::App`].
pub struct Game {
    /// Whether it's playing, as far as the game knows: `App::playing`
    /// says whether it's on screen.
    pub playing: bool,
    /// When a finger was last on the screen, while playing (see [`IDLE`]).
    last_touch: Instant,
    /// When the next frame is due, while playing.
    pub frame_due: Instant,
    /// Frames drawn, for the test pattern's movement.
    tick: u16,
}

impl Game {
    pub const fn new() -> Self {
        Self {
            playing: false,
            last_touch: Instant::from_ticks(0),
            frame_due: Instant::from_ticks(0),
            tick: 0,
        }
    }

    pub fn start(&mut self, now: Instant) {
        defmt::info!("game: playing");
        self.playing = true;
        self.last_touch = now;
        self.frame_due = now;
        cycles::enable();
    }

    pub fn stop(&mut self) {
        if core::mem::take(&mut self.playing) {
            defmt::info!("game: paused");
            cycles::disable();
        }
    }

    /// A frame's worth of input: whether a finger is down.
    pub fn step(&mut self, now: Instant, finger_down: bool) {
        if finger_down {
            self.last_touch = now;
        }
    }

    /// Whether it's gone [`IDLE`] without a touch.
    pub fn idle(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.last_touch) >= IDLE
    }
}

/// Draws the screen around the viewport, and in it, unless `playing`,
/// the title.
pub fn draw(canvas: &mut Canvas, playing: bool) {
    canvas.clear(Color::BACKGROUND);
    let centre = |canvas: &mut Canvas, y, text: &str, color| {
        let x = (canvas::WIDTH as i32 - font::small_text_width(text)) / 2;
        canvas.text(x, y, text, color);
    };
    centre(canvas, 8, "RAYCAST", Color::ACCENT);
    // A frame one canvas pixel outside the viewport.
    let (x, y) = (CELL_X.start - 1, CELL_Y.start - 1);
    let (w, h) = (CELL_X.len() as i32 + 2, CELL_Y.len() as i32 + 2);
    canvas.fill_rect(x, y, w, h, Color::TRACK);
    canvas.fill_rect(
        CELL_X.start,
        CELL_Y.start,
        CELL_X.len() as i32,
        CELL_Y.len() as i32,
        Color::BACKGROUND,
    );
    if playing {
        centre(canvas, 68, "BUTTON: PAUSE", Color::TEXT_FAINT);
    } else {
        centre(canvas, 37, "TAP TO PLAY", Color::TEXT);
        centre(canvas, 68, "BUTTON: BACK", Color::TEXT_FAINT);
    }
}

/// Draws a frame into the viewport, and keeps the time it took in
/// `stats`. The panel must take pixels in [`Order::Columns`].
///
/// [`Order::Columns`]: crate::display::Order::Columns
pub async fn frame(display: &mut Display, batch: &mut [u8], game: &mut Game, stats: &mut Stats) {
    let started = Instant::now();
    let (sending, rest) = batch.split_at_mut(STRIP);
    let making = &mut rest[..STRIP];
    let tick = game.tick;
    let readout = &stats.readout;
    let mut fill_cycles = 0u32;
    display
        .stream(
            VIEW_X,
            VIEW_Y,
            (sending, making),
            VIEW_W / STRIP_COLUMNS,
            |i, strip| {
                let start = cycles::now();
                for (c, column) in strip.as_chunks_mut::<COLUMN>().0.iter_mut().enumerate() {
                    let x = i * STRIP_COLUMNS + c;
                    test_column(x, tick, column);
                    readout.draw(x, column);
                }
                fill_cycles = fill_cycles.wrapping_add(cycles::now().wrapping_sub(start));
            },
        )
        .await;
    let ended = Instant::now();
    game.tick = game.tick.wrapping_add(1);
    game.frame_due = ended + FRAME_GAP;
    stats.add(ended, ended - started, fill_cycles);
}

/// A test pattern's column `x`, at frame `tick`: a red band along the
/// top and a green one down the left (to see the order's right), a white
/// cross through the middle, and grey diagonal stripes moving right.
fn test_column(x: usize, tick: u16, column: &mut [u8; COLUMN]) {
    const fn rgb(r: u16, g: u16, b: u16) -> [u8; 2] {
        ((r >> 3) << 11 | (g >> 2) << 5 | b >> 3).to_be_bytes()
    }
    const RED: [u8; 2] = rgb(255, 0, 0);
    const GREEN: [u8; 2] = rgb(0, 255, 0);
    const WHITE: [u8; 2] = rgb(255, 255, 255);
    const LIGHT: [u8; 2] = rgb(96, 96, 96);
    const DARK: [u8; 2] = rgb(32, 32, 32);
    for (y, pixel) in column.as_chunks_mut::<2>().0.iter_mut().enumerate() {
        *pixel = if y < 6 {
            RED
        } else if x < 6 {
            GREEN
        } else if x == VIEW_W / 2 || y == VIEW_H / 2 {
            WHITE
        } else if ((x + y + usize::from(tick)) / 8).is_multiple_of(2) {
            LIGHT
        } else {
            DARK
        };
    }
}

/// Frame times, logged once a second (at `debug`: build with
/// `DEFMT_LOG=info,ur::game=debug` to see them), and drawn over the test
/// pattern ([`Readout`]).
#[derive(Default)]
pub struct Stats {
    readout: Readout,
    since: Option<Instant>,
    frames: u32,
    /// Sums and maxima, in microseconds; filling's from CPU cycles.
    frame_us: u32,
    frame_max: u32,
    fill_us: u32,
    fill_max: u32,
}

impl Stats {
    fn add(&mut self, now: Instant, took: Duration, fill_cycles: u32) {
        let frame = took.as_micros() as u32;
        let fill = fill_cycles / cycles::PER_MICRO;
        let since = *self.since.get_or_insert(now);
        self.frames += 1;
        self.frame_us += frame;
        self.frame_max = self.frame_max.max(frame);
        self.fill_us += fill;
        self.fill_max = self.fill_max.max(fill);
        if now - since >= Duration::from_secs(1) {
            let ms = (now - since).as_millis() as u32;
            defmt::debug!(
                "game: {=u32}.{=u32} fps; frame {=u32} us (max {=u32}), filling {=u32} us (max {=u32}), {=u32} B a frame in {=u32} B strips",
                self.frames * 1000 / ms,
                self.frames * 10_000 / ms % 10,
                self.frame_us / self.frames,
                self.frame_max,
                self.fill_us / self.frames,
                self.fill_max,
                (COLUMN * VIEW_W) as u32,
                STRIP as u32,
            );
            self.readout = Readout::of(
                self.frames * 10_000 / ms,
                self.frame_us / self.frames,
                self.frame_max,
                self.fill_us / self.frames,
            );
            *self = Self {
                readout: core::mem::take(&mut self.readout),
                since: Some(now),
                ..Self::default()
            };
        }
    }
}

/// The last second's [`Stats`] as text, drawn into the viewport's top
/// left (in the small font at 2×, white on black) as its columns are
/// made: frames a second, a frame's milliseconds (average and most), and
/// the CPU's part of them (filling the strips).
#[derive(Default)]
struct Readout {
    lines: [[u8; Self::CHARS]; 3],
}

impl Readout {
    const CHARS: usize = 18;
    const SCALE: usize = 2;
    /// A character's advance, in pixels: three, and a gap.
    const ADVANCE: usize = 4 * Self::SCALE;
    const LINE: usize = 6 * Self::SCALE;
    /// Where the text starts, inside the test pattern's bands.
    const X: usize = 8;
    const Y: usize = 8;
    const W: usize = Self::CHARS * Self::ADVANCE;
    const H: usize = 3 * Self::LINE;

    /// From frames a second in tenths, and times in microseconds.
    fn of(fps_x10: u32, frame_us: u32, frame_max: u32, fill_us: u32) -> Self {
        let mut readout = Self::default();
        let tenths = |line: &mut Line, n: u32| {
            line.number(n / 10);
            line.push(b'.');
            line.number(n % 10);
        };
        let mut line = Line::default();
        tenths(&mut line, fps_x10);
        line.text(b" FPS");
        readout.lines[0] = line.done();
        let mut line = Line::default();
        line.text(b"MS ");
        tenths(&mut line, frame_us / 100);
        line.text(b" MAX ");
        tenths(&mut line, frame_max / 100);
        readout.lines[1] = line.done();
        let mut line = Line::default();
        line.text(b"CPU ");
        tenths(&mut line, fill_us / 100);
        line.text(b" MS");
        readout.lines[2] = line.done();
        readout
    }

    /// Draws over column `x` of the viewport.
    fn draw(&self, x: usize, column: &mut [u8; COLUMN]) {
        const WHITE: [u8; 2] = [0xff, 0xff];
        const BLACK: [u8; 2] = [0, 0];
        let Some(dx) = x.checked_sub(Self::X).filter(|&dx| dx < Self::W) else {
            return;
        };
        let (char_at, glyph_x) = (dx / Self::ADVANCE, dx % Self::ADVANCE / Self::SCALE);
        let pixels = column.as_chunks_mut::<2>().0;
        for (l, line) in self.lines.iter().enumerate() {
            let rows = match line[char_at] {
                0 => [0; 5],
                c => crate::font::small_glyph(char::from(c)),
            };
            let top = Self::Y + l * Self::LINE;
            for (dy, pixel) in pixels[top..top + Self::LINE].iter_mut().enumerate() {
                let row = dy / Self::SCALE;
                let lit = glyph_x < 3 && row < 5 && rows[row] >> (2 - glyph_x) & 1 != 0;
                *pixel = if lit { WHITE } else { BLACK };
            }
        }
    }
}

const _: () = assert!(Readout::X + Readout::W <= VIEW_W);
const _: () = assert!(Readout::Y + Readout::H <= VIEW_H);

/// A line of [`Readout`] being written; what doesn't fit is dropped.
#[derive(Default)]
struct Line {
    bytes: [u8; Readout::CHARS],
    len: usize,
}

impl Line {
    fn push(&mut self, b: u8) {
        if let Some(slot) = self.bytes.get_mut(self.len) {
            *slot = b;
            self.len += 1;
        }
    }

    fn text(&mut self, text: &[u8]) {
        text.iter().for_each(|&b| self.push(b));
    }

    fn number(&mut self, mut n: u32) {
        let mut digits = [0u8; 10];
        let mut i = digits.len();
        loop {
            i -= 1;
            digits[i] = b'0' + (n % 10) as u8;
            n /= 10;
            if n == 0 {
                break;
            }
        }
        self.text(&digits[i..]);
    }

    fn done(self) -> [u8; Readout::CHARS] {
        self.bytes
    }
}

/// The CPU's cycle counter (DWT's CYCCNT), for timing what the CPU does:
/// it stops while the CPU sleeps, so it's no good for waits. On only
/// while the game plays.
mod cycles {
    use cortex_m::peripheral::{DCB, DWT};

    /// 64 MHz.
    pub const PER_MICRO: u32 = 64;

    pub fn enable() {
        // SAFETY: only the game uses the DCB and DWT, from the UI loop.
        let mut core = unsafe { cortex_m::Peripherals::steal() };
        core.DCB.enable_trace();
        core.DWT.enable_cycle_counter();
    }

    pub fn disable() {
        // SAFETY: as in `enable`.
        let mut core = unsafe { cortex_m::Peripherals::steal() };
        core.DWT.disable_cycle_counter();
        // Not while a debug probe is attached, which may want the trace
        // block for itself.
        if !DCB::is_debugger_attached() {
            core.DCB.disable_trace();
        }
    }

    pub fn now() -> u32 {
        DWT::cycle_count()
    }
}
