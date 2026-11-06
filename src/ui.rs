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
//! - Long press: next colour theme (temporary, while choosing one).
//! - Button: return to the clock; on the clock, turn the screen off.
//! - Any button press or touch while the screen is off: wake to the clock.
//!   (That touch is *only* a wake-up; it isn't also acted on.)
//! - No input for [`SCREEN_TIMEOUT`]: turn the screen off.

use core::fmt::Write as _;

use embassy_time::{Duration, Instant};

use crate::InputEvent;
use crate::battery::Battery;
use crate::canvas::{Canvas, Color, Sweep};
use crate::font;
use crate::motor::{self, Pattern};
use crate::theme::{THEMES, Theme};
use crate::touch::Gesture;

const SCREEN_TIMEOUT: Duration = Duration::from_secs(15);

/// Everything screens may display that the UI doesn't own itself.
pub struct Status<'a> {
    pub uptime_secs: u64,
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
    pub fn new(now: Instant) -> Self {
        Self {
            screen: Screen::Clock,
            awake: true,
            last_input: now,
            transition: None,
            theme: 0,
        }
    }

    pub fn theme(&self) -> &'static Theme {
        &THEMES[self.theme]
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
                // TODO: temporary, for picking a theme on the real panel.
                (Gesture::LongPress, _) => self.theme = (self.theme + 1) % THEMES.len(),
                _ => {}
            },
        }
    }

    pub fn on_tick(&mut self, now: Instant) {
        if self.awake && now - self.last_input >= SCREEN_TIMEOUT {
            self.awake = false;
        }
    }

    pub fn render(&self, canvas: &mut Canvas, status: &Status) {
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
//    3..7    status bar: battery % and icon (right)
//   20..41   HH:MM in 5×7 digits at 3× (15×21 each), with a drop shadow
//   48..52   date
//   57..62   heart rate
//   72..73   seconds bar

const BACKGROUND: Color = Color::BACKGROUND;

fn render_clock(canvas: &mut Canvas, status: &Status) {
    canvas.clear(BACKGROUND);
    render_battery(canvas, status.battery);

    // TODO: wall-clock time and date once Bluetooth time sync lands
    // (milestone 2). Until then: time since boot, and say so.
    let secs = status.uptime_secs;
    let (hours, minutes, seconds) = ((secs / 3600) % 24, (secs / 60) % 60, secs % 60);

    // HH:MM. Each digit cell is 15 px wide with 2 px gaps; the colon gets a
    // 3 px column. That's 71 px, leaving 4 px either side (+1 for shadow).
    let digits = [hours / 10, hours % 10, minutes / 10, minutes % 10];
    let digit_x = [4, 21, 43, 60];
    const TIME_Y: i32 = 20;
    const DIGIT_SCALE: i32 = 3;
    for (&digit, &x) in digits.iter().zip(&digit_x) {
        let glyph = &font::DIGITS[digit as usize];
        canvas.sprite(
            x + 1,
            TIME_Y + 1,
            font::DIGIT_W,
            glyph,
            DIGIT_SCALE,
            Color::SHADOW,
        );
        canvas.sprite(x, TIME_Y, font::DIGIT_W, glyph, DIGIT_SCALE, Color::TEXT);
    }
    // The colon blinks: a heartbeat that shows the watch is alive even when
    // the minutes aren't changing. Its dots line up with glyph rows 2 and 4.
    if seconds % 2 == 0 {
        for row in [2, 4] {
            let y = TIME_Y + row * DIGIT_SCALE;
            canvas.fill_rect(39, y + 1, 3, 3, Color::SHADOW);
            canvas.fill_rect(38, y, 3, 3, Color::ACCENT);
        }
    }

    canvas.text_centered(40, 48, "NOT SYNCED", Color::TEXT_FAINT);

    // Heart rate, centred as a group: icon, 2 px gap, value.
    // TODO: real readings once the HRS3300 driver lands (milestone 6).
    let bpm = "--";
    let group_w = font::HEART_W as i32 + 2 + font::small_text_width(bpm);
    let x = 40 - group_w / 2;
    canvas.sprite(x, 57, font::HEART_W, &font::HEART, 1, Color::HEART);
    canvas.sprite(
        x,
        57,
        font::HEART_W,
        &font::HEART_SHINE,
        1,
        Color::HEART_SHINE,
    );
    canvas.text(x + font::HEART_W as i32 + 2, 58, bpm, Color::TEXT);

    // Seconds bar: 60 px, one per second, so it's full on second 59.
    canvas.fill_rect(10, 72, 60, 2, Color::TRACK);
    canvas.fill_rect(10, 72, seconds as i32 + 1, 2, Color::ACCENT);
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
        let y = 16 + 9 * i as i32;
        canvas.text(4, y, label, Color::TEXT_DIM);
        canvas.text_right(77, y, value, Color::TEXT);
    }

    let y = 16 + 9 * labels.len() as i32;
    canvas.text(4, y, "THEME", Color::TEXT_DIM);
    canvas.text_right(77, y, theme.name, Color::TEXT);

    canvas.text(4, 62, "TAP: TEST BUZZ", Color::TEXT_FAINT);
    canvas.text(4, 68, "HOLD: NEXT THEME", Color::TEXT_FAINT);
    canvas.text(4, 74, "SWIPE: CLOCK", Color::TEXT_FAINT);
}
