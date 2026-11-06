//! Screens, navigation, and screen power.
//!
//! The UI is immediate-mode: [`App::render`] paints the current screen onto
//! the canvas from scratch, and the canvas works out what actually changed.
//! All state that affects what's shown lives in [`App`] or in the [`Status`]
//! snapshot passed to `render`.
//!
//! Navigation:
//!
//! - Swipe left/right: cycle between screens.
//! - Long press: next colour theme.
//! - Button: return to the clock; on the clock, turn the screen off.
//! - Any button press or touch while the screen is off: wake to the clock.
//!   (That touch is *only* a wake-up; it isn't also acted on.)
//! - No input for [`SCREEN_TIMEOUT`]: turn the screen off.

use core::fmt::Write as _;

use embassy_time::{Duration, Instant};

use crate::InputEvent;
use crate::battery::Battery;
use crate::canvas::{Canvas, Color, Sweep};
use crate::clock::DateTime;
use crate::font;
use crate::motor::{self, Pattern};
use crate::theme::{THEMES, Theme};
use crate::touch::Gesture;

const SCREEN_TIMEOUT: Duration = Duration::from_secs(15);

/// Everything screens may display that the UI doesn't own itself.
pub struct Status<'a> {
    pub uptime_secs: u64,
    /// Local wall-clock time, once the phone has set it.
    pub local_time: Option<DateTime>,
    /// Whether a phone is connected over Bluetooth.
    pub connected: bool,
    /// While pairing: the passkey to type on the phone.
    pub passkey: Option<u32>,
    pub battery: &'a Battery,
}

#[derive(Clone, Copy, PartialEq, Eq, defmt::Format)]
enum Screen {
    Clock,
    Info,
}

impl Screen {
    /// Screens in swipe order. Swiping wraps around.
    const ALL: [Self; 2] = [Self::Clock, Self::Info];

    fn neighbour(self, step: isize) -> Self {
        let i = Self::ALL
            .iter()
            .position(|&s| s == self)
            .expect("every screen is in ALL");
        Self::ALL[(i as isize + step).rem_euclid(Self::ALL.len() as isize) as usize]
    }
}

pub struct App {
    screen: Screen,
    /// Whether the panel and backlight should be on.
    pub awake: bool,
    last_input: Instant,
    /// Set when the screen changes; the next frame is shown with this
    /// transition instead of a plain update.
    transition: Option<Sweep>,
    /// Index into [`THEMES`].
    theme: usize,
}

impl App {
    /// `theme` is an index into [`THEMES`], e.g. as saved from a previous
    /// run; out-of-range values (a theme since removed) fall back to the first.
    pub fn new(now: Instant, theme: u8) -> Self {
        Self {
            screen: Screen::Clock,
            awake: true,
            last_input: now,
            transition: None,
            theme: usize::from(theme) % THEMES.len(),
        }
    }

    pub fn theme(&self) -> &'static Theme {
        &THEMES[self.theme]
    }

    /// For persisting the theme choice.
    pub fn theme_index(&self) -> u8 {
        self.theme as u8
    }

    /// The transition to show the next frame with, if any.
    pub fn take_transition(&mut self) -> Option<Sweep> {
        self.transition.take()
    }

    /// Switches screens. The sweep follows the finger: swiping left moves
    /// content left, so the new screen comes in from the right.
    fn go_to(&mut self, screen: Screen, sweep: Sweep) {
        if screen != self.screen {
            self.screen = screen;
            self.transition = Some(sweep);
        }
    }

    pub fn on_input(&mut self, event: InputEvent, now: Instant) {
        self.last_input = now;
        if !self.awake {
            self.awake = true;
            self.screen = Screen::Clock;
            return;
        }

        match event {
            // Only ever a reason to wake up, handled above.
            InputEvent::Attention => {}
            InputEvent::Button => {
                if self.screen == Screen::Clock {
                    self.awake = false;
                } else {
                    // Home is the leftmost screen, so going home sweeps the
                    // same way as swiping back towards it.
                    self.go_to(Screen::Clock, Sweep::LeftToRight);
                }
            }
            InputEvent::Touch(touch) => match (touch.gesture, self.screen) {
                (Gesture::SwipeLeft, _) => self.go_to(self.screen.neighbour(1), Sweep::RightToLeft),
                (Gesture::SwipeRight, _) => {
                    self.go_to(self.screen.neighbour(-1), Sweep::LeftToRight)
                }
                // TODO: remove once something real uses the motor; for now
                // this is how we check it works.
                (Gesture::Tap, Screen::Info) => motor::buzz(Pattern::NOTIFY),
                (Gesture::LongPress, _) => self.theme = (self.theme + 1) % THEMES.len(),
                _ => {}
            },
        }
    }

    /// `keep_awake` holds the screen on regardless of input, e.g. while a
    /// pairing passkey is showing and the user is typing it on the phone.
    pub fn on_tick(&mut self, now: Instant, keep_awake: bool) {
        if keep_awake {
            self.last_input = now;
        }
        if self.awake && now - self.last_input >= SCREEN_TIMEOUT {
            self.awake = false;
        }
    }

    pub fn render(&self, canvas: &mut Canvas, status: &Status) {
        // Pairing takes over the screen until it's done, whatever was showing.
        if let Some(passkey) = status.passkey {
            render_pairing(canvas, passkey);
            return;
        }
        match self.screen {
            Screen::Clock => render_clock(canvas, status),
            Screen::Info => render_info(canvas, status, self.theme()),
        }
    }
}

// ==============================================================================
// Clock face
// ==============================================================================
//
// Canvas layout (80×80), y ranges inclusive:
//
//    3..7    status bar: time-sync state (left), battery % and icon (right)
//   13..34   HH:MM in 5×7 digits at 3× (15×21 each), with a drop shadow
//   39..52   weekday (left), SS in the same digits at 2× (centred), heart
//            rate (right)
//   58..62   date, YYYY-MM-DD
//   66..79   free; meant for the next alarm (milestone 5)

const BACKGROUND: Color = Color::BACKGROUND;

fn render_clock(canvas: &mut Canvas, status: &Status) {
    canvas.clear(BACKGROUND);
    render_battery(canvas, status.battery);

    // Status bar, left: Bluetooth when connected; a note if we don't know
    // the time (in which case the clock shows time since boot).
    if status.connected {
        canvas.sprite(3, 2, font::BLUETOOTH_W, &font::BLUETOOTH, 1, Color::ACCENT);
    } else if status.local_time.is_none() {
        canvas.text(3, 3, "NO TIME", Color::TEXT_FAINT);
    }

    let (hours, minutes, seconds) = match &status.local_time {
        Some(t) => (u64::from(t.hour), u64::from(t.minute), u64::from(t.second)),
        None => {
            let secs = status.uptime_secs;
            ((secs / 3600) % 24, (secs / 60) % 60, secs % 60)
        }
    };

    // HH:MM. Each digit cell is 15 px wide with 2 px gaps; the colon gets a
    // 3 px column. That's 71 px, leaving 4 px either side (+1 for shadow).
    const TIME_Y: i32 = 13;
    const TIME_SCALE: i32 = 3;
    let hhmm = [hours / 10, hours % 10, minutes / 10, minutes % 10];
    for (&digit, x) in hhmm.iter().zip([4, 21, 43, 60]) {
        draw_digit(canvas, x, TIME_Y, digit, TIME_SCALE, Color::TEXT);
    }
    // The colon blinks: a heartbeat that shows the watch is alive even when
    // the minutes aren't changing. Its dots line up with glyph rows 2 and 4.
    if seconds % 2 == 0 {
        for row in [2, 4] {
            let y = TIME_Y + row * TIME_SCALE;
            canvas.fill_rect(39, y + 1, 3, 3, Color::SHADOW);
            canvas.fill_rect(38, y, 3, 3, Color::ACCENT);
        }
    }

    // The seconds row: smaller, dimmer seconds in the middle (two 10 px
    // cells, 2 px gap), flanked by the weekday and heart rate, all
    // vertically centred on the digits.
    const ROW_Y: i32 = 39;
    const TEXT_Y: i32 = ROW_Y + 4;
    for (digit, x) in [seconds / 10, seconds % 10].into_iter().zip([29, 41]) {
        draw_digit(canvas, x, ROW_Y, digit, 2, Color::TEXT_DIM);
    }
    let weekday = status
        .local_time
        .as_ref()
        .map_or("---", DateTime::weekday_name);
    canvas.text(4, TEXT_Y, weekday, Color::TEXT_DIM);

    // Heart rate, right-aligned as a group: icon, 2 px gap, value.
    // TODO: real readings once the HRS3300 driver lands (milestone 6).
    let bpm = "--";
    let x = 77 - (font::HEART_W as i32 + 2 + font::small_text_width(bpm));
    canvas.sprite(x, TEXT_Y - 1, font::HEART_W, &font::HEART, 1, Color::HEART);
    canvas.sprite(
        x,
        TEXT_Y - 1,
        font::HEART_W,
        &font::HEART_SHINE,
        1,
        Color::HEART_SHINE,
    );
    canvas.text(x + font::HEART_W as i32 + 2, TEXT_Y, bpm, Color::TEXT);

    let mut date = heapless::String::<10>::new();
    match &status.local_time {
        Some(t) => {
            write!(date, "{:04}-{:02}-{:02}", t.year, t.month, t.day).expect("exactly 10 chars")
        }
        None => date.push_str("----------").expect("exactly 10 chars"),
    }
    canvas.text_centered(40, 58, &date, Color::TEXT_DIM);
}

/// One big 5×7 digit at `scale`, with a 1 px drop shadow down and right.
fn draw_digit(canvas: &mut Canvas, x: i32, y: i32, digit: u64, scale: i32, color: Color) {
    let glyph = &font::DIGITS[digit as usize];
    canvas.sprite(x + 1, y + 1, font::DIGIT_W, glyph, scale, Color::SHADOW);
    canvas.sprite(x, y, font::DIGIT_W, glyph, scale, color);
}

fn render_battery(canvas: &mut Canvas, battery: &Battery) {
    let percent = battery.percent();
    let color = if battery.charging {
        Color::GOOD
    } else if percent < 20 {
        Color::BAD
    } else {
        Color::TEXT
    };

    // 10×5 body with a 1×3 nub; 8×3 interior, filled in proportion.
    const X: i32 = 66;
    const Y: i32 = 3;
    canvas.fill_rect(X, Y, 10, 5, color);
    canvas.fill_rect(X + 10, Y + 1, 1, 3, color);
    canvas.fill_rect(X + 1, Y + 1, 8, 3, BACKGROUND);
    // Round up, so any charge at all shows at least one pixel.
    let fill = (percent * 8).div_ceil(100) as i32;
    canvas.fill_rect(X + 1, Y + 1, fill, 3, color);

    let mut text = heapless::String::<8>::new();
    write!(text, "{percent}%").expect("at most 4 chars fit in 8");
    canvas.text_right(X - 2, Y, &text, color);
}

// ==============================================================================
// Info screen
// ==============================================================================

/// First value row, and the spacing between rows.
const INFO_TOP: i32 = 13;
const INFO_LINE: i32 = 8;

fn render_info(canvas: &mut Canvas, status: &Status, theme: &Theme) {
    canvas.clear(BACKGROUND);
    canvas.text(
        4,
        3,
        concat!("UR ", env!("CARGO_PKG_VERSION")),
        Color::ACCENT,
    );

    let battery = status.battery;
    let mv = battery.millivolts();
    let secs = status.uptime_secs;
    let mut values: [heapless::String<12>; 4] = Default::default();
    write!(values[0], "{}.{:02}V", mv / 1000, (mv % 1000) / 10).expect("fits in 12");
    write!(values[1], "{}%", battery.percent()).expect("fits in 12");
    write!(values[2], "{}", battery.status()).expect("fits in 12");
    write!(
        values[3],
        "{:02}:{:02}:{:02}",
        secs / 3600,
        (secs / 60) % 60,
        secs % 60
    )
    .expect("fits in 12 until uptime exceeds ~11 years");

    let labels = ["BATTERY", "CHARGE", "POWER", "UPTIME"];
    for (i, (label, value)) in labels.iter().zip(&values).enumerate() {
        let y = INFO_TOP + INFO_LINE * i as i32;
        canvas.text(4, y, label, Color::TEXT_DIM);
        canvas.text_right(77, y, value, Color::TEXT);
    }

    let y = INFO_TOP + INFO_LINE * labels.len() as i32;
    canvas.text(4, y, "THEME", Color::TEXT_DIM);
    canvas.text_right(77, y, theme.name, Color::TEXT);
    let y = y + INFO_LINE;
    canvas.text(4, y, "PHONE", Color::TEXT_DIM);
    let phone = if status.connected {
        "CONNECTED"
    } else {
        "WAITING"
    };
    canvas.text_right(77, y, phone, Color::TEXT);

    canvas.text(4, 62, "TAP: TEST BUZZ", Color::TEXT_FAINT);
    canvas.text(4, 68, "HOLD: NEXT THEME", Color::TEXT_FAINT);
    canvas.text(4, 74, "SWIPE: CLOCK", Color::TEXT_FAINT);
}

// ==============================================================================
// Pairing
// ==============================================================================

/// The six-digit passkey, big enough to read at a glance while typing it on
/// the phone.
fn render_pairing(canvas: &mut Canvas, passkey: u32) {
    canvas.clear(BACKGROUND);
    canvas.text_centered(40, 12, "PAIRING", Color::ACCENT);
    canvas.text_centered(40, 22, "ENTER ON PHONE:", Color::TEXT_DIM);

    // Six 2× digits (10 px) with 2 px gaps: 70 px, centred.
    let mut digits = [0u64; 6];
    let mut rest = u64::from(passkey);
    for digit in digits.iter_mut().rev() {
        *digit = rest % 10;
        rest /= 10;
    }
    for (i, &digit) in digits.iter().enumerate() {
        draw_digit(canvas, 5 + 12 * i as i32, 36, digit, 2, Color::TEXT);
    }
}
