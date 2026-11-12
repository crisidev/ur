//! A raycaster game (the `game` feature), on a screen of its own at the
//! end of the line below the clock (see `crate::ui`).
//!
//! The screen shows a title until tapped; then the game plays on the
//! whole panel, until the button pauses it (back to the title; the button
//! again goes to the clock, as everywhere). Whatever else wants the
//! screen (a call, an alarm, "find my watch", pairing, an update, a
//! notification's banner, the screen going off) pauses it too, as does
//! [`IDLE`] without a finger on it.
//!
//! The world, and how the player moves in it, is [`crate::raycast`]'s,
//! and how it looks [`crate::game_view`]'s; this sends it, a column at a time,
//! and steers by where a finger is held ([`Input::from_touch`]): the left
//! and right thirds of the screen turn, the top and bottom ones walk
//! forward and back. Walking into a wall knocks (`Pattern::BUMP`).
//!
//! Frames don't go through the canvas, which would cost a framebuffer:
//! each is sent straight to the panel a column at a time, each column
//! made while the one before goes out, in two buffers borrowed from the
//! canvas ([`Canvas::lend`]). For that the panel takes pixels column by
//! column ([`Order::Columns`]), and the UI loop switches back ([`leave`])
//! before the canvas draws again.
//!
//! In the world are things ([`THINGS`]): gems to find, by walking into
//! them (a knock each, a flourish for the last; once all are found, they
//! come back when the game next starts), and barrels and pillars in the
//! way. Each is a picture standing on the floor, always facing the player
//! ([`Sprites`]), drawn in a column only if it's nearer than the wall
//! there, which the column's own ray says: so no depth buffer.
//!
//! The whole panel is more than the bus can send many times a second
//! (115 KB a frame, 8 MHz: under 9). But the ceiling and floor are the
//! same every frame, so only what's in front of them needs sending: in
//! each column, the rows its wall and things cover now, and those they
//! covered last frame, which go back to ceiling and floor. Which those
//! were is kept in the canvas's other buffer, also lent. Over it all, the
//! HUD ([`Hud`]): the time, the gems found, and, with the `game-stats`
//! feature, the last second's frame rate and times (see [`Stats`]), for
//! a sealed watch without a probe to read the log; their rows are sent
//! when they change.

use embassy_time::{Duration, Instant};

use crate::canvas::{self, Canvas, Color};
use crate::display::{Display, Format, Order};
use crate::font;
use crate::game_view::{self as view, Hud, Readout, Sprites, view_column};
use crate::motor::{self, Pattern};
use crate::raycast::{self, Camera, Input, MAP, Player, THINGS, Taken};

/// The panel's width and height, in pixels: the view's.
const PANEL: usize = canvas::WIDTH * canvas::SCALE;
const _: () = assert!(view::SIZE == PANEL);

/// How pixels go to the panel (see [`view::RGB444`]).
const FORMAT: Format = if view::RGB444 {
    Format::Rgb444
} else {
    Format::Rgb565
};

/// A column's colours, two bytes each as they're made, before they're
/// packed to [`FORMAT`] (see [`view_column`]).
const COLUMN: usize = PANEL * 2;
const _: () = assert!(2 * COLUMN <= canvas::LENT, "two columns in the batch");
/// What each column showed in front of the ceiling and floor: its first
/// row and the one after its last (see [`view_column`]).
const SHOWN: usize = PANEL * 2;
const _: () = assert!(
    SHOWN <= canvas::LENT_FRONT,
    "what's shown in the front buffer"
);

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
    /// When the player last moved: a step covers the time since.
    stepped: Instant,
    /// Whether the player was up against a wall then: a bump only knocks
    /// as it starts, not for as long as it lasts.
    bumped: bool,
    /// Kept while paused: a game picks up where it left off.
    player: Player,
    /// The gems found ([`THINGS`]'), until all are and the game next
    /// starts.
    taken: Taken,
    /// Whether that changed since the HUD last showed it.
    found: bool,
    /// Whether the next frame sends every pixel: the panel shows
    /// something else (the title), and what columns showed means nothing.
    whole: bool,
    /// The time the HUD shows, as hours and minutes, if known.
    shown_time: Option<(u8, u8)>,
}

impl Game {
    pub const fn new() -> Self {
        Self {
            playing: false,
            last_touch: Instant::from_ticks(0),
            frame_due: Instant::from_ticks(0),
            stepped: Instant::from_ticks(0),
            bumped: false,
            player: Player::START,
            taken: 0,
            found: false,
            whole: true,
            shown_time: None,
        }
    }

    pub fn start(&mut self, now: Instant) {
        defmt::info!("game: playing");
        self.playing = true;
        self.last_touch = now;
        self.frame_due = now;
        self.stepped = now;
        self.bumped = false;
        self.whole = true;
        // All found last time: they're back.
        let (found, all) = raycast::gems(&THINGS, self.taken);
        if found == all {
            self.taken = 0;
        }
        cycles::enable();
        cache(true);
    }

    pub fn stop(&mut self) {
        if core::mem::take(&mut self.playing) {
            defmt::info!("game: paused");
            cycles::disable();
            cache(false);
        }
    }

    /// Moves the player, by where a finger is (in panel pixels), if one
    /// is, for the time since the last step.
    pub fn step(&mut self, now: Instant, finger_at: Option<(u8, u8)>) {
        if finger_at.is_some() {
            self.last_touch = now;
        }
        let seconds = (now - self.stepped).as_micros() as f32 / 1e6;
        self.stepped = now;
        let input = Input::from_touch(finger_at, PANEL as u16);
        let bumped = raycast::step(&MAP, &THINGS, &mut self.player, input, seconds);
        if bumped && !self.bumped {
            motor::buzz(Pattern::BUMP);
        }
        self.bumped = bumped;
        if raycast::pick(&THINGS, &mut self.taken, &self.player) {
            self.found = true;
            let (found, all) = raycast::gems(&THINGS, self.taken);
            motor::buzz(if found == all {
                Pattern::ALL_FOUND
            } else {
                Pattern::FOUND
            });
        }
    }

    /// Whether it's gone [`IDLE`] without a touch.
    pub fn idle(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.last_touch) >= IDLE
    }
}

/// Draws the title, and how to play.
pub fn draw(canvas: &mut Canvas) {
    canvas.clear(Color::BACKGROUND);
    let centre = |canvas: &mut Canvas, y, text: &str, color| {
        let x = (canvas::WIDTH as i32 - font::small_text_width(text)) / 2;
        canvas.text(x, y, text, color);
    };
    centre(canvas, 10, "RAYCAST", Color::ACCENT);
    centre(canvas, 26, "TAP TO PLAY", Color::TEXT);
    centre(canvas, 42, "HOLD LEFT, RIGHT: TURN", Color::TEXT_FAINT);
    centre(canvas, 50, "HOLD TOP, BOTTOM: WALK", Color::TEXT_FAINT);
    centre(canvas, 66, "BUTTON: PAUSE, BACK", Color::TEXT_FAINT);
}

/// Readies the panel for frames: column by column, in [`FORMAT`].
pub async fn enter(display: &mut Display, game: &mut Game) {
    display.set_order(Order::Columns).await;
    display.set_format(FORMAT).await;
    game.whole = true;
}

/// Puts the panel back as the canvas expects it.
pub async fn leave(display: &mut Display) {
    display.set_format(Format::Rgb565).await;
    display.set_order(Order::Rows).await;
}

/// Draws a frame, and keeps the time it took in `stats`. The panel must
/// be ready for it ([`enter`]). `lent` is the canvas's (see
/// [`Canvas::lend`]), whose front buffer must be as the last frame left
/// it, unless it's the first since [`enter`] or [`Game::start`].
pub async fn frame(
    display: &mut Display,
    lent: (&mut [u8], &mut [u8]),
    game: &mut Game,
    stats: &mut Stats,
) {
    let started = Instant::now();
    let (batch, front) = lent;
    let (sending, rest) = batch.split_at_mut(COLUMN);
    let making = &mut rest[..COLUMN];
    let shown = front[..SHOWN].as_chunks_mut::<2>().0;
    let whole = core::mem::take(&mut game.whole);
    let time = crate::clock::now().map(|t| (t.hour, t.minute));
    let time_changed = time != core::mem::replace(&mut game.shown_time, time);
    let readout_changed = core::mem::take(&mut stats.readout_changed);
    let hud = Hud {
        time,
        gems: raycast::gems(&THINGS, game.taken),
        readout: cfg!(feature = "game-stats").then_some(&stats.readout),
        time_changed: whole || time_changed,
        gems_changed: whole || core::mem::take(&mut game.found),
        readout_changed: whole || readout_changed,
    };
    let camera = Camera::new(&game.player, PANEL, PANEL);
    let sprites = Sprites::of(&camera, &THINGS, game.taken);
    let mut fill_cycles = 0u32;
    let mut pixels = 0u32;
    let mut sent = 0..0;
    display
        .stream_columns(0..PANEL as u16, (sending, making), |x, column| {
            let start = cycles::now();
            let x = usize::from(x);
            let rows = view_column(
                &camera,
                &sprites,
                x,
                column,
                &mut shown[x],
                &mut sent,
                whole,
                &hud,
            );
            pixels += rows.len() as u32;
            fill_cycles = fill_cycles.wrapping_add(cycles::now().wrapping_sub(start));
            rows.start as u16..rows.end as u16
        })
        .await;
    let ended = Instant::now();
    game.frame_due = ended + FRAME_GAP;
    let bytes = FORMAT.bytes(pixels as usize) as u32;
    stats.add(ended, ended - started, fill_cycles, bytes);
}

/// Frame times, logged once a second (at `debug`: build with
/// `DEFMT_LOG=info,ur::game=debug` to see them), and, with the
/// `game-stats` feature, drawn over the view ([`Readout`]).
#[derive(Default)]
pub struct Stats {
    readout: Readout,
    /// Whether the readout changed since a frame last drew it.
    readout_changed: bool,
    since: Option<Instant>,
    frames: u32,
    /// Sums and maxima, in microseconds; making columns' from CPU cycles.
    frame_us: u32,
    frame_max: u32,
    fill_us: u32,
    fill_max: u32,
    bytes: u32,
}

impl Stats {
    fn add(&mut self, now: Instant, took: Duration, fill_cycles: u32, bytes: u32) {
        let frame = took.as_micros() as u32;
        let fill = fill_cycles / cycles::PER_MICRO;
        let since = *self.since.get_or_insert(now);
        self.frames += 1;
        self.frame_us += frame;
        self.frame_max = self.frame_max.max(frame);
        self.fill_us += fill;
        self.fill_max = self.fill_max.max(fill);
        self.bytes += bytes;
        if now - since >= Duration::from_secs(1) {
            let ms = (now - since).as_millis() as u32;
            defmt::debug!(
                "game: {=u32}.{=u32} fps; frame {=u32} us (max {=u32}), making {=u32} us (max {=u32}), {=u32} B a frame",
                self.frames * 1000 / ms,
                self.frames * 10_000 / ms % 10,
                self.frame_us / self.frames,
                self.frame_max,
                self.fill_us / self.frames,
                self.fill_max,
                self.bytes / self.frames,
            );
            *self = Self {
                readout: Readout::of(
                    self.frames * 10_000 / ms,
                    self.frame_us / self.frames,
                    self.frame_max,
                    self.fill_us / self.frames,
                    self.bytes / self.frames,
                ),
                readout_changed: true,
                since: Some(now),
                ..Self::default()
            };
        }
    }
}

/// Turns the CPU's instruction cache on or off. Off from reset, and
/// nothing else turns it on: without it, every instruction comes from
/// flash, with wait states at 64 MHz, and drawing a column took twice
/// as long as its instructions say. On only while the game plays, so
/// that the rest of the firmware runs as it always has.
fn cache(on: bool) {
    embassy_nrf::pac::NVMC
        .icachecnf()
        .write(|w| w.set_cacheen(on));
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
