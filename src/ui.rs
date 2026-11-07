//! Screens, navigation, and screen power.
//!
//! The UI is immediate-mode: [`App::render`] paints the current screen onto
//! the canvas from scratch, and the canvas works out what actually changed.
//! All state that affects what's shown lives in [`App`] or in the [`Status`]
//! snapshot passed to `render`.
//!
//! Navigation:
//!
//! - Swipe left/right: cycle between screens (clock, music while the phone
//!   has media, info).
//! - Long press: next colour theme within the current group (dark themes
//!   after sunset, light ones after sunrise; each group remembers its own).
//! - Button: return to the clock; on the clock, turn the screen off. While a
//!   call rings: silence it.
//! - Any button press or touch while the screen is off: wake to the clock.
//!   (That touch is *only* a wake-up; it isn't also acted on.)
//! - No input for [`SCREEN_TIMEOUT`]: turn the screen off.
//!
//! From the phone:
//!
//! - Notifications: a two-line banner (app, then sender) for
//!   [`BANNER_TIME`] and a short buzz. If the screen was off, it wakes just
//!   for the banner and sleeps again after.
//! - Calls: take over the screen, ringing, until the phone says the call is
//!   over (answered, rejected, ended) or the user acts.
//! - Media info: enables the music screen.

use core::fmt::Write as _;

use embassy_time::{Duration, Instant};
use heapless::String;

use crate::InputEvent;
use crate::battery::Battery;
use crate::ble;
use crate::canvas::{Canvas, Color, Sweep, marquee_cycle};
use crate::clock::DateTime;
use crate::font;
use crate::gb::{CallAction, MusicCommand, Outgoing};
use crate::geo::Sun;
use crate::motor::{self, Pattern};
use crate::theme::{self, Theme};
use crate::touch::Gesture;

const SCREEN_TIMEOUT: Duration = Duration::from_secs(15);
/// How long a notification banner shows.
const BANNER_TIME: Duration = Duration::from_secs(4);
/// Stop ringing after this long, in case the phone never says the call is
/// over (e.g. the connection dropped mid-call).
const MAX_RING: Duration = Duration::from_secs(45);

/// Everything screens may display that the UI doesn't own itself.
pub struct Status<'a> {
    pub uptime_secs: u64,
    /// Local wall-clock time, once the phone has set it.
    pub local_time: Option<DateTime>,
    /// Whether a phone is connected over Bluetooth.
    pub connected: bool,
    /// While pairing: the passkey to type on the phone.
    pub passkey: Option<u32>,
    /// While a firmware update is arriving: how far along it is, in percent.
    pub update_progress: Option<u8>,
    /// Today's sunrise/sunset, for the info screen (once the time is known).
    pub sun: Option<Sun>,
    /// Today's steps (once the time is known).
    pub steps: Option<u32>,
    /// The latest heart rate, unless there's none recent enough to show.
    pub heart_rate: Option<u8>,
    pub battery: &'a Battery,
}

/// Things the phone tells the UI, via [`crate::ble`].
pub enum PhoneEvent {
    Notification {
        app: String<24>,
        who: String<40>,
    },
    IncomingCall {
        caller: String<40>,
    },
    /// The call was answered, rejected, or ended, on either side.
    CallOver,
    MusicInfo {
        artist: String<40>,
        track: String<40>,
    },
    MusicState {
        playing: bool,
    },
    /// "Find my watch" from the phone: ring until told to stop.
    Find {
        on: bool,
    },
}

/// Logged without contents: names and titles stay out of the logs.
impl defmt::Format for PhoneEvent {
    fn format(&self, f: defmt::Formatter) {
        let kind = match self {
            Self::Notification { .. } => "Notification",
            Self::IncomingCall { .. } => "IncomingCall",
            Self::CallOver => "CallOver",
            Self::MusicInfo { .. } => "MusicInfo",
            Self::MusicState { .. } => "MusicState",
            Self::Find { .. } => "Find",
        };
        defmt::write!(f, "{=str}", kind)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, defmt::Format)]
enum Screen {
    Clock,
    Music,
    Info,
}

struct Banner {
    app: String<24>,
    who: String<40>,
    /// When it appeared, for scrolling long lines.
    since: Instant,
    until: Instant,
}

struct Call {
    caller: String<40>,
    since: Instant,
    until: Instant,
    silenced: bool,
}

struct Music {
    artist: String<40>,
    track: String<40>,
    playing: bool,
    /// When the track last changed, for scrolling long names.
    since: Instant,
}

impl Music {
    fn new(now: Instant) -> Self {
        Self {
            artist: String::new(),
            track: String::new(),
            playing: false,
            since: now,
        }
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
    /// Whether it's daylight, which picks the light or dark theme group.
    daytime: bool,
    /// The chosen theme within each group: indices into [`theme::LIGHT`] and
    /// [`theme::DARK`]. Long press advances the one currently in use.
    pub themes: ThemeChoice,
    connected: bool,
    banner: Option<Banner>,
    /// The screen woke only to show a notification; sleep again after it.
    brief_wake: bool,
    call: Option<Call>,
    /// What the phone is playing, since it last told us.
    music: Option<Music>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ThemeChoice {
    pub light: u8,
    pub dark: u8,
}

impl App {
    /// `themes` is typically as saved from a previous run; out-of-range
    /// indices (a theme since removed) wrap around.
    pub fn new(now: Instant, themes: ThemeChoice) -> Self {
        Self {
            screen: Screen::Clock,
            awake: true,
            last_input: now,
            transition: None,
            daytime: false,
            themes,
            connected: false,
            banner: None,
            brief_wake: false,
            call: None,
            music: None,
        }
    }

    /// Whether the clock face is on screen for someone to read: awake, and
    /// not just showing a notification or a call. That's when a fresh heart
    /// rate is worth measuring for.
    pub fn showing_clock(&self) -> bool {
        self.awake && self.screen == Screen::Clock && !self.brief_wake && self.call.is_none()
    }

    pub fn set_daytime(&mut self, daytime: bool) {
        self.daytime = daytime;
    }

    /// Media info is only meaningful while connected: forget it on
    /// disconnect (which also removes the music screen).
    pub fn set_connected(&mut self, connected: bool) {
        self.connected = connected;
        if !connected {
            self.music = None;
            if self.screen == Screen::Music {
                self.screen = Screen::Clock;
            }
        }
    }

    pub fn theme(&self) -> &'static Theme {
        let (group, choice): (&[&Theme], u8) = if self.daytime {
            (&theme::LIGHT, self.themes.light)
        } else {
            (&theme::DARK, self.themes.dark)
        };
        group[usize::from(choice) % group.len()]
    }

    /// Long press: the next theme in the group currently in use.
    fn next_theme(&mut self) {
        let (choice, count) = if self.daytime {
            (&mut self.themes.light, theme::LIGHT.len())
        } else {
            (&mut self.themes.dark, theme::DARK.len())
        };
        *choice = ((usize::from(*choice) + 1) % count) as u8;
    }

    /// The transition to show the next frame with, if any.
    pub fn take_transition(&mut self) -> Option<Sweep> {
        self.transition.take()
    }

    /// Screens in swipe order (wrapping around). The music screen only
    /// exists while the phone has told us about media.
    fn screens(&self) -> heapless::Vec<Screen, 3> {
        let mut screens = heapless::Vec::new();
        let _ = screens.push(Screen::Clock);
        if self.music.is_some() {
            let _ = screens.push(Screen::Music);
        }
        let _ = screens.push(Screen::Info);
        screens
    }

    fn neighbour(&self, step: isize) -> Screen {
        let screens = self.screens();
        let i = screens.iter().position(|&s| s == self.screen).unwrap_or(0);
        screens[(i as isize + step).rem_euclid(screens.len() as isize) as usize]
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
        let event = match event {
            InputEvent::Phone(event) => return self.on_phone(event, now),
            other => other,
        };
        // A user input (or a request for attention) from here on.
        self.last_input = now;
        self.brief_wake = false;
        if !self.awake {
            self.awake = true;
            self.screen = Screen::Clock;
            return;
        }

        if let Some(call) = &mut self.call {
            match event {
                InputEvent::Button if !call.silenced => {
                    call.silenced = true;
                    motor::buzz(Pattern::STOP);
                    send(Outgoing::Call(CallAction::Ignore));
                }
                InputEvent::Touch(touch)
                    if touch.gesture == Gesture::Tap && touch.y >= CALL_BUTTONS_Y =>
                {
                    let action = if touch.x < 40 {
                        CallAction::Reject
                    } else {
                        CallAction::Accept
                    };
                    send(Outgoing::Call(action));
                    self.call = None;
                    motor::buzz(Pattern::STOP);
                }
                _ => {}
            }
            return;
        }

        match event {
            InputEvent::Phone(_) => unreachable!("handled above"),
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
                (Gesture::SwipeLeft, _) => self.go_to(self.neighbour(1), Sweep::RightToLeft),
                (Gesture::SwipeRight, _) => self.go_to(self.neighbour(-1), Sweep::LeftToRight),
                (Gesture::Tap, Screen::Music) if touch.y >= MUSIC_CONTROLS_Y => {
                    // Left, middle, and right thirds.
                    let command = match touch.x {
                        ..27 => MusicCommand::Previous,
                        27..53 => MusicCommand::PlayPause,
                        _ => MusicCommand::Next,
                    };
                    send(Outgoing::Music(command));
                    // Show the new state straight away; the phone confirms
                    // (or corrects) it shortly after.
                    if let (MusicCommand::PlayPause, Some(music)) = (command, &mut self.music) {
                        music.playing = !music.playing;
                    }
                }
                // TODO: remove once something real uses the motor; for now
                // this is how we check it works.
                (Gesture::Tap, Screen::Info) => motor::buzz(Pattern::NOTIFY),
                (Gesture::LongPress, _) => self.next_theme(),
                _ => {}
            },
        }
    }

    fn on_phone(&mut self, event: PhoneEvent, now: Instant) {
        match event {
            PhoneEvent::Notification { app, who } => {
                // Long enough to read, and for long lines to scroll through
                // once.
                let overflow = (font::small_text_width(&app).max(font::small_text_width(&who)))
                    - BANNER_TEXT_WIDTH;
                let shown_for = BANNER_TIME.max(marquee_cycle(overflow));
                self.banner = Some(Banner {
                    app,
                    who,
                    since: now,
                    until: now + shown_for,
                });
                motor::buzz(Pattern::NOTIFY);
                if !self.awake {
                    self.awake = true;
                    self.brief_wake = true;
                    self.screen = Screen::Clock;
                }
            }
            PhoneEvent::IncomingCall { caller } => {
                self.call = Some(Call {
                    caller,
                    since: now,
                    until: now + MAX_RING,
                    silenced: false,
                });
                motor::buzz(Pattern::RING);
                self.awake = true;
                self.brief_wake = false;
                self.last_input = now;
            }
            PhoneEvent::CallOver => {
                if self.call.take().is_some() {
                    motor::buzz(Pattern::STOP);
                }
            }
            PhoneEvent::MusicInfo { artist, track } => {
                let music = self.music.get_or_insert_with(|| Music::new(now));
                if music.track != track || music.artist != artist {
                    music.since = now;
                }
                music.artist = artist;
                music.track = track;
            }
            PhoneEvent::MusicState { playing } => {
                self.music.get_or_insert_with(|| Music::new(now)).playing = playing
            }
            PhoneEvent::Find { on } => {
                motor::buzz(if on { Pattern::RING } else { Pattern::STOP });
                if on {
                    self.awake = true;
                    self.last_input = now;
                }
            }
        }
    }

    /// `keep_awake` holds the screen on regardless of input, e.g. while a
    /// pairing passkey is showing and the user is typing it on the phone.
    pub fn on_tick(&mut self, now: Instant, keep_awake: bool) {
        if self.banner.as_ref().is_some_and(|b| now >= b.until) {
            self.banner = None;
            if self.brief_wake {
                self.awake = false;
                self.brief_wake = false;
            }
        }
        if self.call.as_ref().is_some_and(|c| now >= c.until) {
            self.call = None;
            motor::buzz(Pattern::STOP);
        }
        if keep_awake || self.call.is_some() || self.brief_wake {
            self.last_input = now;
        }
        if self.awake && now - self.last_input >= SCREEN_TIMEOUT {
            self.awake = false;
        }
    }

    /// Whether something on screen is scrolling, so frames are needed more
    /// often than once a second (see [`crate::canvas::MARQUEE_FRAME`]).
    pub fn animating(&self) -> bool {
        let overflows =
            |text: &str, scale: i32, room: i32| font::small_text_width(text) * scale > room;
        if !self.awake {
            return false;
        }
        if let Some(call) = &self.call {
            return overflows(&call.caller, 2, CALL_TEXT_WIDTH);
        }
        let banner = self.banner.as_ref().is_some_and(|b| {
            overflows(&b.app, 1, BANNER_TEXT_WIDTH) || overflows(&b.who, 1, BANNER_TEXT_WIDTH)
        });
        let music = self.screen == Screen::Music
            && self.music.as_ref().is_some_and(|m| {
                overflows(&m.artist, 1, MUSIC_TEXT_WIDTH)
                    || overflows(&m.track, 2, MUSIC_TEXT_WIDTH)
            });
        banner || music
    }

    pub fn render(&self, canvas: &mut Canvas, status: &Status, now: Instant) {
        // Updates, pairing, then calls take over the screen until they're
        // done.
        if let Some(percent) = status.update_progress {
            render_update(canvas, percent, now);
            return;
        }
        if let Some(passkey) = status.passkey {
            render_pairing(canvas, passkey);
            return;
        }
        if let Some(call) = &self.call {
            render_call(canvas, call, now);
            return;
        }
        match self.screen {
            Screen::Clock => render_clock(canvas, status),
            Screen::Music => match &self.music {
                Some(music) => render_music(canvas, music, now),
                None => render_clock(canvas, status),
            },
            Screen::Info => render_info(canvas, status, self.theme()),
        }
        if let Some(banner) = &self.banner {
            render_banner(canvas, banner, now);
        }
    }
}

/// Queues a message for the phone (dropped if not connected or the queue is
/// full; none of these are worth retrying later).
fn send(message: Outgoing) {
    let _ = ble::OUTBOX.try_send(message);
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
//   67..73   steps today (left); right half meant for the next alarm

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
    let mut bpm = heapless::String::<3>::new();
    match status.heart_rate {
        Some(rate) => write!(bpm, "{rate}").expect("a u8 is at most 3 digits"),
        None => bpm.push_str("--").expect("2 chars fit in 3"),
    }
    let bpm = bpm.as_str();
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

    // Steps today, with a little footprint.
    if let Some(steps) = status.steps {
        canvas.sprite(3, 67, font::FOOT_W, &font::FOOT, 1, Color::TEXT_DIM);
        let mut text = heapless::String::<8>::new();
        write!(text, "{steps}").expect("u32 fits in 8 digits... almost always");
        canvas.text(9, 68, &text, Color::TEXT);
    }
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
const INFO_TOP: i32 = 12;
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
    canvas.text(4, y, "SUN", Color::TEXT_DIM);
    let mut daylight = heapless::String::<12>::new();
    match status.sun {
        Some(Sun { sunrise, sunset }) => write!(
            daylight,
            "{:02}:{:02}-{:02}:{:02}",
            sunrise / 60,
            sunrise % 60,
            sunset / 60,
            sunset % 60
        )
        .expect("11 chars"),
        None => daylight.push_str("--:-----:--").expect("11 chars"),
    }
    canvas.text_right(77, y, &daylight, Color::TEXT);
    let y = y + INFO_LINE;
    canvas.text(4, y, "PHONE", Color::TEXT_DIM);
    let phone = if status.connected {
        "CONNECTED"
    } else {
        "WAITING"
    };
    canvas.text_right(77, y, phone, Color::TEXT);

    canvas.text(4, 68, "HOLD: NEXT THEME", Color::TEXT_FAINT);
    canvas.text(4, 74, "TAP: TEST BUZZ", Color::TEXT_FAINT);
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

// ==============================================================================
// Firmware update
// ==============================================================================

/// A chunky segmented progress bar, as on an old console's loading screen:
/// 16 cells in a frame, filled cells lit with a highlight, and the cell being
/// filled flickering in the transition's dither pattern.
fn render_update(canvas: &mut Canvas, percent: u8, now: Instant) {
    const CELLS: i32 = 16;
    const CELL: i32 = 4;
    const LEFT: i32 = 40 - (CELLS * CELL) / 2;
    const TOP: i32 = 38;

    canvas.clear(BACKGROUND);
    canvas.text_centered(40, 14, "UPDATING", Color::ACCENT);
    canvas.text_centered(40, 24, "KEEP CLOSE", Color::TEXT_DIM);

    // Frame, with a drop shadow like the digits'.
    let (width, height) = (CELLS * CELL + 3, CELL + 5);
    canvas.fill_rect(LEFT - 1, TOP - 1, width, height, Color::SHADOW);
    canvas.fill_rect(LEFT - 2, TOP - 2, width, height, Color::TEXT_DIM);
    canvas.fill_rect(LEFT - 1, TOP - 1, width - 2, height - 2, BACKGROUND);

    let filled = i32::from(percent) * CELLS / 100;
    for cell in 0..CELLS {
        let x = LEFT + cell * CELL;
        if cell < filled {
            canvas.fill_rect(x, TOP, CELL - 1, CELL, Color::ACCENT);
            canvas.fill_rect(x, TOP, CELL - 1, 1, Color::HEART_SHINE);
        } else if cell == filled {
            // Flip the checkerboard's phase every frame (one a second).
            let phase = (now.as_secs() % 2) as i32;
            for dy in 0..CELL {
                for dx in 0..CELL - 1 {
                    let color = if (dx + dy + phase) % 2 == 0 {
                        Color::DITHER_A
                    } else {
                        Color::DITHER_B
                    };
                    canvas.pixel(x + dx, TOP + dy, color);
                }
            }
        }
    }

    // The percentage in the clock's 2× digits, right-aligned under the bar.
    let mut x = 40 + 17;
    let mut rest = u64::from(percent);
    loop {
        x -= 12;
        draw_digit(canvas, x, 52, rest % 10, 2, Color::TEXT);
        rest /= 10;
        if rest == 0 {
            break;
        }
    }
    canvas.text(40 + 19, 59, "%", Color::TEXT_DIM);
}

// ==============================================================================
// Notifications, calls, and music
// ==============================================================================

/// Text columns of the banner: 3..77.
const BANNER_TEXT_WIDTH: i32 = 74;

/// Across the bottom rows (free on the clock face): app, then sender.
fn render_banner(canvas: &mut Canvas, banner: &Banner, now: Instant) {
    let elapsed = now - banner.since;
    canvas.fill_rect(0, 63, 80, 17, BACKGROUND);
    canvas.fill_rect(0, 63, 80, 1, Color::ACCENT);
    canvas.marquee(
        3..3 + BANNER_TEXT_WIDTH,
        67,
        &banner.app,
        Color::ACCENT,
        1,
        elapsed,
    );
    canvas.marquee(
        3..3 + BANNER_TEXT_WIDTH,
        74,
        &banner.who,
        Color::TEXT,
        1,
        elapsed,
    );
}

/// Where the decline/answer buttons start, for both drawing and tapping.
const CALL_BUTTONS_Y: i32 = 55;
/// The caller's name scrolls within columns 4..76.
const CALL_TEXT_WIDTH: i32 = 72;

fn render_call(canvas: &mut Canvas, call: &Call, now: Instant) {
    canvas.clear(BACKGROUND);
    canvas.text_centered(40, 6, "INCOMING CALL", Color::ACCENT);

    // The caller at 2×, centred if it fits, scrolling if not.
    let width = font::small_text_width(&call.caller) * 2;
    let left = if width <= CALL_TEXT_WIDTH {
        40 - width / 2
    } else {
        4
    };
    canvas.marquee(
        left..4 + CALL_TEXT_WIDTH,
        24,
        &call.caller,
        Color::TEXT,
        2,
        now - call.since,
    );

    let hint = if call.silenced {
        "SILENCED"
    } else {
        "BUTTON: SILENCE"
    };
    canvas.text_centered(40, 44, hint, Color::TEXT_FAINT);

    for (x, label, color) in [(2, "DECLINE", Color::BAD), (42, "ANSWER", Color::GOOD)] {
        canvas.fill_rect(x, CALL_BUTTONS_Y, 36, 18, color);
        canvas.text_centered(x + 18, CALL_BUTTONS_Y + 7, label, BACKGROUND);
    }
}

/// Where the transport controls start, for both drawing and tapping.
const MUSIC_CONTROLS_Y: i32 = 52;
/// Artist and track scroll within columns 3..77.
const MUSIC_TEXT_WIDTH: i32 = 74;

fn render_music(canvas: &mut Canvas, music: &Music, now: Instant) {
    let elapsed = now - music.since;
    canvas.clear(BACKGROUND);
    canvas.text(3, 4, "MUSIC", Color::ACCENT);
    canvas.text_right(
        77,
        4,
        if music.playing { "PLAYING" } else { "PAUSED" },
        Color::TEXT_DIM,
    );

    canvas.marquee(
        3..3 + MUSIC_TEXT_WIDTH,
        17,
        &music.artist,
        Color::TEXT_DIM,
        1,
        elapsed,
    );
    canvas.marquee(
        3..3 + MUSIC_TEXT_WIDTH,
        29,
        &music.track,
        Color::TEXT,
        2,
        elapsed,
    );

    // Previous, play/pause, next: 5×5 icons at 3×, centred in thirds.
    let play_pause = if music.playing {
        &font::PAUSE
    } else {
        &font::PLAY
    };
    for (x, icon) in [(6, &font::PREVIOUS), (33, play_pause), (60, &font::NEXT)] {
        canvas.sprite(x, MUSIC_CONTROLS_Y + 6, 5, icon, 3, Color::TEXT);
    }
}
