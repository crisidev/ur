//! Screens, navigation, and screen power.
//!
//! The UI is immediate-mode: [`App::render`] paints the current screen onto
//! the canvas from scratch, and the canvas works out what actually changed.
//! All state that affects what's shown lives in [`App`] or in the [`Status`]
//! snapshot passed to `render`.
//!
//! Navigation:
//!
//! - Swipe left/right: cycle between screens (clock, alarms, music while
//!   the phone has media, info).
//! - Long press: next colour theme within the current group (dark themes
//!   after sunset, light ones after sunrise; each group remembers its own).
//! - Button: return to the clock; on the clock, turn the screen off. While a
//!   call rings: silence it.
//! - Any button press or touch while the screen is off: wake to the clock.
//!   (That touch is *only* a wake-up; it isn't also acted on.)
//! - No input for [`SCREEN_TIMEOUT`]: turn the screen off.
//! - Turning the wrist to look (the accelerometer's gesture) while the
//!   screen is off: a glance, just the time in white on black at a low
//!   backlight, for [`GLANCE_TIME`]. Touching or pressing during it wakes
//!   the screen properly.
//!
//! Alarms (see [`crate::alarm`] for when they ring):
//!
//! - On the alarms screen, tap a row (smart wake, alarm, timer) to edit it;
//!   long-press to switch it on or off (or start/stop the timer).
//! - In the editor: swipe up/down on the left half for hours, the right
//!   half for minutes; tap the bottom left to switch it on/off, the bottom
//!   right for the next page or done. The button also means done.
//! - Ringing takes over the screen: tap to snooze, button to stop.
//!
//! From the phone:
//!
//! - Notifications: a two-line banner (app, then sender) for
//!   [`BANNER_TIME`] and a short buzz. If the screen was off, the
//!   notification gets the whole (dim, white on black) screen instead, like
//!   a glance, and the screen sleeps again after.
//! - Calls: take over the screen, ringing, until the phone says the call is
//!   over (answered, rejected, ended) or the user acts.
//! - Media info: enables the music screen.

use core::fmt::Write as _;

use embassy_time::{Duration, Instant};
use heapless::String;

use crate::InputEvent;
use crate::alarm;
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
/// How long a glance shows the time.
const GLANCE_TIME: Duration = Duration::from_secs(5);
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
    /// The most recent sleep within the last day.
    pub sleep: Option<crate::activity::Sleep>,
    /// Local seconds since 1970 (see [`crate::alarm`]), once the time is
    /// known.
    pub local_second: Option<i64>,
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
    Alarms,
    Music,
    Info,
}

/// An alarm ringing.
struct Ringing {
    kind: alarm::Kind,
    since: Instant,
}

/// Which setting the editor is changing.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Editing {
    SmartWeekday,
    SmartWeekend,
    /// Which of [`alarm::ALARMS`].
    At(u8),
    /// Which of [`alarm::TIMERS`].
    Timer(u8),
}

// The alarms screen: smart wake across the top, then the two alarms side by
// side, then the two timers side by side. These are the tops of those
// bands (also what a tap hits).
const SMART_TOP: i32 = 11;
const ALARMS_TOP: i32 = 30;
const TIMERS_TOP: i32 = 53;
/// Where the editor's buttons start.
const EDITOR_BUTTONS_Y: i32 = 60;

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
    /// Alarm settings, as stored (`main` saves them when they change).
    pub alarms: alarm::Settings,
    alarm_state: alarm::State,
    ringing: Option<Ringing>,
    /// The alarm editor, while open.
    editing: Option<Editing>,
    /// The local minute, as of the last tick (for edits that need "now").
    local_second: Option<i64>,
    /// While glancing: until when.
    glance_until: Option<Instant>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ThemeChoice {
    pub light: u8,
    pub dark: u8,
}

impl App {
    /// `themes` is typically as saved from a previous run; out-of-range
    /// indices (a theme since removed) wrap around.
    pub fn new(now: Instant, themes: ThemeChoice, alarms: alarm::Settings) -> Self {
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
            alarms,
            alarm_state: alarm::State::default(),
            ringing: None,
            editing: None,
            local_second: None,
            glance_until: None,
        }
    }

    /// Whether the screen is only showing a glance (dim, time only).
    pub fn glancing(&self) -> bool {
        self.glance_until.is_some()
    }

    /// Whether a notification has the screen to itself, because it arrived
    /// with the screen off.
    fn quiet_notification(&self) -> bool {
        self.brief_wake && self.banner.is_some()
    }

    /// Whether the screen shows the dim white-on-black look (a glance, or a
    /// notification that arrived with the screen off), for the backlight.
    pub fn dim(&self) -> bool {
        self.glancing() || self.quiet_notification()
    }

    /// Whether the clock face is on screen for someone to read: awake, and
    /// not just showing a notification or a call. That's when a fresh heart
    /// rate is worth measuring for.
    pub fn showing_clock(&self) -> bool {
        self.awake
            && self.screen == Screen::Clock
            && !self.glancing()
            && !self.brief_wake
            && self.call.is_none()
            && self.ringing.is_none()
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
        if self.dim() {
            return &theme::GLANCE;
        }
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
    fn screens(&self) -> heapless::Vec<Screen, 4> {
        let mut screens = heapless::Vec::new();
        let _ = screens.push(Screen::Clock);
        let _ = screens.push(Screen::Alarms);
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
            // Only while the screen is off; it's not a user input as such
            // (an arm swing can look the same), so it doesn't hold the
            // screen on.
            InputEvent::Glance => {
                if !self.awake {
                    self.awake = true;
                    self.screen = Screen::Clock;
                    self.glance_until = Some(now + GLANCE_TIME);
                }
                return;
            }
            other => other,
        };
        // Touching or pressing during a glance: wake properly (and, as
        // with any wake-up, don't also act on it).
        if self.glance_until.take().is_some() {
            self.last_input = now;
            return;
        }
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

        if self.ringing.is_some() {
            match event {
                InputEvent::Button => self.stop_ringing(),
                InputEvent::Touch(touch) if touch.gesture == Gesture::Tap => {
                    if let Some(minute) = self.local_second {
                        self.alarm_state.snooze(minute);
                    }
                    self.stop_ringing();
                }
                _ => {}
            }
            return;
        }

        if let Some(editing) = self.editing {
            self.on_editor_input(editing, event);
            return;
        }

        match event {
            InputEvent::Phone(_) | InputEvent::Glance => unreachable!("handled above"),
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
                (Gesture::Tap, Screen::Alarms) => {
                    if let Some(row) = alarm_at(touch.x, touch.y) {
                        self.editing = Some(row);
                    }
                }
                // A quick switch, without opening the editor.
                (Gesture::LongPress, Screen::Alarms) => {
                    if let Some(row) = alarm_at(touch.x, touch.y) {
                        self.toggle(row);
                        motor::buzz(Pattern::NOTIFY);
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

    /// Switches a row's alarm on or off; for the timer, starts or stops it.
    fn toggle(&mut self, row: Editing) {
        match row {
            Editing::SmartWeekday | Editing::SmartWeekend => self.alarms.smart = !self.alarms.smart,
            Editing::At(i) => {
                let i = usize::from(i);
                self.alarms.at[i] = !self.alarms.at[i];
                if let Some(now) = self.local_second {
                    self.alarm_state.arm_at(i, &self.alarms, now);
                }
            }
            Editing::Timer(i) => {
                let i = usize::from(i);
                match (self.alarm_state.timers[i], self.local_second) {
                    (Some(_), _) => self.alarm_state.stop_timer(i),
                    (None, Some(now)) => self.alarm_state.start_timer(i, &self.alarms, now),
                    // Without the time there's nothing to count from.
                    (None, None) => {}
                }
            }
        }
    }

    fn on_editor_input(&mut self, editing: Editing, event: InputEvent) {
        let done = |app: &mut Self| {
            app.editing = None;
            // A changed time for an armed alarm moves it.
            if let (Editing::At(i), Some(now)) = (editing, app.local_second) {
                app.alarm_state.arm_at(usize::from(i), &app.alarms, now);
            }
        };
        let InputEvent::Touch(touch) = event else {
            if let InputEvent::Button = event {
                done(self);
            }
            return;
        };
        let up = match touch.gesture {
            Gesture::SwipeUp => 1,
            Gesture::SwipeDown => -1,
            Gesture::Tap if touch.y >= EDITOR_BUTTONS_Y => {
                if touch.x < 40 {
                    self.toggle(editing);
                } else if editing == Editing::SmartWeekday {
                    self.editing = Some(Editing::SmartWeekend);
                } else {
                    done(self);
                }
                return;
            }
            _ => return,
        };
        // Left half: hours; right half: minutes (5 at a time, but 1 for the
        // timer, where short durations are common).
        let step = match (touch.x < 40, editing) {
            (true, _) => 60,
            (false, Editing::Timer(_)) => 1,
            (false, _) => 5,
        };
        let value = match editing {
            Editing::SmartWeekday => &mut self.alarms.smart_weekday,
            Editing::SmartWeekend => &mut self.alarms.smart_weekend,
            Editing::At(i) => &mut self.alarms.at_time[usize::from(i)],
            Editing::Timer(i) => &mut self.alarms.timers[usize::from(i)],
        };
        let changed = i32::from(*value) + up * step;
        *value = if let Editing::Timer(_) = editing {
            changed.clamp(1, i32::from(alarm::MAX_TIMER)) as u16
        } else {
            changed.rem_euclid(24 * 60) as u16
        };
    }

    fn stop_ringing(&mut self) {
        self.ringing = None;
        motor::buzz(Pattern::STOP);
    }

    fn on_phone(&mut self, event: PhoneEvent, now: Instant) {
        // Anything from the phone gets the full screen.
        if self.glance_until.take().is_some() {
            self.last_input = now;
        }
        match event {
            PhoneEvent::Notification { app, who } => {
                // Long enough to read, and for long lines to scroll through
                // once. With the screen off, the sender is wrapped over the
                // whole screen (see `quiet_layout`), and only scrolls if
                // even that won't do.
                let who_overflow = if self.awake {
                    font::small_text_width(&who)
                } else if quiet_layout(&who).is_some() {
                    0
                } else {
                    font::small_text_width(&who) * 2
                };
                let overflow = font::small_text_width(&app).max(who_overflow) - BANNER_TEXT_WIDTH;
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

    /// Once a second: rings any alarm that's due. `local_second` is the
    /// time, if known; `stirring` whether the wearer just moved (for smart
    /// wake).
    pub fn check_alarms(&mut self, now: Instant, local_second: Option<i64>, stirring: bool) {
        self.local_second = local_second;
        let Some(second) = local_second else {
            return;
        };
        if let Some(ringing) = &self.ringing
            && now - ringing.since >= Duration::from_secs(alarm::RING_FOR as u64 * 60)
        {
            defmt::info!("alarm: rang out ({})", ringing.kind);
            self.stop_ringing();
        }
        if self.ringing.is_none()
            && let Some(kind) = self.alarm_state.due(&mut self.alarms, second, stirring)
        {
            defmt::info!("alarm: ringing ({})", kind);
            self.ringing = Some(Ringing { kind, since: now });
            self.editing = None;
            motor::buzz(Pattern::ALARM);
            self.awake = true;
            self.brief_wake = false;
            self.last_input = now;
        }
    }

    /// `keep_awake` holds the screen on regardless of input, e.g. while a
    /// pairing passkey is showing and the user is typing it on the phone.
    pub fn on_tick(&mut self, now: Instant, keep_awake: bool) {
        // A glance has its own timer: it isn't input, so the screen timeout
        // (which counts from the last input) would end it at once.
        if let Some(until) = self.glance_until {
            if now >= until {
                self.glance_until = None;
                self.awake = false;
            }
            return;
        }
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
        if keep_awake || self.call.is_some() || self.ringing.is_some() || self.brief_wake {
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
        let quiet = self.quiet_notification();
        let banner = self.banner.as_ref().is_some_and(|b| {
            let who = if quiet {
                quiet_layout(&b.who).is_none()
            } else {
                overflows(&b.who, 1, BANNER_TEXT_WIDTH)
            };
            overflows(&b.app, 1, BANNER_TEXT_WIDTH) || who
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
        if let Some(ringing) = &self.ringing {
            render_ringing(canvas, ringing, status);
            return;
        }
        if self.glancing() {
            render_glance(canvas, status);
            return;
        }
        if self.quiet_notification()
            && let Some(banner) = &self.banner
        {
            render_quiet_notification(canvas, banner, now);
            return;
        }
        if let Some(editing) = self.editing {
            render_editor(
                canvas,
                editing,
                &self.alarms,
                &self.alarm_state,
                status.local_second,
            );
            return;
        }
        let next_alarm = status
            .local_second
            .and_then(|minute| self.alarm_state.next(&self.alarms, minute));
        match self.screen {
            Screen::Clock => render_clock(canvas, status, next_alarm),
            Screen::Alarms => render_alarms(canvas, &self.alarms, &self.alarm_state, status),
            Screen::Music => match &self.music {
                Some(music) => render_music(canvas, music, now),
                None => render_clock(canvas, status, next_alarm),
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

fn render_clock(canvas: &mut Canvas, status: &Status, next_alarm: Option<(alarm::Kind, i64)>) {
    canvas.clear(BACKGROUND);
    render_battery(canvas, status.battery, status.uptime_secs);

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
    let digits = [hours / 10, hours % 10, minutes / 10, minutes % 10];
    for (&digit, x) in digits.iter().zip([4, 21, 43, 60]) {
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

    // The next alarm within a day, right-aligned: bell, then when it rings
    // (a smart alarm at its latest).
    if let (Some((kind, at)), Some(now)) = (next_alarm, status.local_second)
        && at - now <= 24 * 60 * 60
    {
        // A timer counts down; anything else shows when it rings.
        let time = match kind {
            alarm::Kind::Timer(_) => countdown(at - now),
            _ => {
                let mut time = heapless::String::new();
                time.push_str(&hhmm(at.div_euclid(60).rem_euclid(24 * 60)))
                    .expect("fits");
                time
            }
        };
        let x = 77 - font::small_text_width(&time);
        canvas.text(x, 68, &time, Color::TEXT);
        let (icon, width) = match kind {
            alarm::Kind::Timer(_) => (&font::HOURGLASS, font::HOURGLASS_W),
            _ => (&font::BELL, font::BELL_W),
        };
        canvas.sprite(x - width as i32 - 2, 67, width, icon, 1, Color::ACCENT);
    }
}

/// The glance: HH:MM and nothing else, in the middle.
fn render_glance(canvas: &mut Canvas, status: &Status) {
    canvas.clear(BACKGROUND);
    let Some(t) = &status.local_time else {
        return;
    };
    let digits = [t.hour / 10, t.hour % 10, t.minute / 10, t.minute % 10];
    for (&digit, x) in digits.iter().zip([4, 21, 43, 60]) {
        draw_digit(canvas, x, 29, u64::from(digit), 3, Color::TEXT);
    }
    for row in [2, 4] {
        canvas.fill_rect(38, 29 + row * 3, 3, 3, Color::TEXT);
    }
}

/// Time left, as short as it reads well: "M:SS" under 10 minutes (where
/// seconds matter), "42 MIN" under an hour, "2H05" beyond.
fn countdown(seconds: i64) -> heapless::String<8> {
    let mut text = heapless::String::new();
    let seconds = seconds.max(0);
    let result = if seconds < 10 * 60 {
        write!(text, "{}:{:02}", seconds / 60, seconds % 60)
    } else if seconds < 60 * 60 {
        write!(text, "{} MIN", (seconds + 59) / 60)
    } else {
        write!(text, "{}H{:02}", seconds / 3600, seconds / 60 % 60)
    };
    result.expect("fits in 8 for timers up to 16 hours");
    text
}

/// `minutes` (after midnight, or a duration) as HH:MM.
fn hhmm(minutes: i64) -> heapless::String<6> {
    let mut text = heapless::String::new();
    write!(text, "{:02}:{:02}", minutes / 60, minutes % 60).expect("fits in 6");
    text
}

// ==============================================================================
// Alarms
// ==============================================================================

/// What a touch at (x, y) on the alarms screen is on.
fn alarm_at(x: i32, y: i32) -> Option<Editing> {
    let right = u8::from(x >= 40);
    if y < SMART_TOP {
        None
    } else if y < ALARMS_TOP {
        Some(Editing::SmartWeekday)
    } else if y < TIMERS_TOP {
        Some(Editing::At(right))
    } else {
        Some(Editing::Timer(right))
    }
}

/// Smart wake across the top (label and state, then both times), then the
/// two alarms and the two timers in pairs of cells, each cell a label, its
/// time, and its state.
fn render_alarms(
    canvas: &mut Canvas,
    settings: &alarm::Settings,
    state: &alarm::State,
    status: &Status,
) {
    canvas.clear(BACKGROUND);
    canvas.text(4, 3, "ALARMS", Color::ACCENT);
    let switch = |on: bool| {
        if on {
            ("ON", Color::GOOD)
        } else {
            ("OFF", Color::TEXT_FAINT)
        }
    };

    let (text, color) = switch(settings.smart);
    canvas.text(4, SMART_TOP + 1, "SMART WAKE", Color::TEXT_DIM);
    canvas.text_right(77, SMART_TOP + 1, text, color);
    let mut times = heapless::String::<20>::new();
    write!(
        times,
        "{} / {}",
        hhmm(settings.smart_weekday.into()),
        hhmm(settings.smart_weekend.into())
    )
    .expect("fits in 20");
    canvas.text(4, SMART_TOP + 8, &times, Color::TEXT);

    // Hairlines between the bands, so the pairs read as pairs.
    for y in [ALARMS_TOP - 2, TIMERS_TOP - 2] {
        canvas.fill_rect(4, y, 72, 1, Color::TEXT_FAINT);
    }
    let cell = |canvas: &mut Canvas,
                column: usize,
                top: i32,
                label: &str,
                value: &str,
                status: (&str, Color)| {
        let x = 4 + 40 * column as i32;
        canvas.text(x, top + 1, label, Color::TEXT_DIM);
        canvas.text(x, top + 8, value, Color::TEXT);
        canvas.text(x, top + 15, status.0, status.1);
    };
    for i in 0..alarm::ALARMS {
        let time = hhmm(settings.at_time[i].into());
        cell(
            canvas,
            i,
            ALARMS_TOP,
            ["ALARM 1", "ALARM 2"][i],
            &time,
            switch(settings.at[i]),
        );
    }
    for t in 0..alarm::TIMERS {
        // While running: time left, ticking; otherwise the duration.
        let left = status.local_second.and_then(|now| state.remaining(t, now));
        let (value, running) = match left {
            Some(left) => (countdown(left), ("RUNNING", Color::GOOD)),
            None => (duration(settings.timers[t]), ("", Color::TEXT_FAINT)),
        };
        cell(
            canvas,
            t,
            TIMERS_TOP,
            ["TIMER 1", "TIMER 2"][t],
            &value,
            running,
        );
    }
}

/// A timer's duration (not counting down): "3 MIN", "1H30".
fn duration(minutes: u16) -> heapless::String<8> {
    let mut text = heapless::String::new();
    let result = if minutes < 60 {
        write!(text, "{minutes} MIN")
    } else {
        write!(text, "{}H{:02}", minutes / 60, minutes % 60)
    };
    result.expect("fits in 8");
    text
}

/// One setting, big, with how to change it.
fn render_editor(
    canvas: &mut Canvas,
    editing: Editing,
    settings: &alarm::Settings,
    state: &alarm::State,
    now: Option<i64>,
) {
    canvas.clear(BACKGROUND);
    // A running timer shows what's left (as MM:SS under an hour); anything
    // else, the setting being edited (HH:MM).
    let left = match editing {
        Editing::Timer(t) => now.and_then(|now| state.remaining(usize::from(t), now)),
        _ => None,
    };
    let (title, value, on) = match editing {
        Editing::SmartWeekday => ("SMART: WEEKDAYS", settings.smart_weekday, settings.smart),
        Editing::SmartWeekend => ("SMART: WEEKENDS", settings.smart_weekend, settings.smart),
        Editing::At(i) => (
            ["ALARM 1", "ALARM 2"][usize::from(i)],
            settings.at_time[usize::from(i)],
            settings.at[usize::from(i)],
        ),
        Editing::Timer(t) => (
            ["TIMER 1", "TIMER 2"][usize::from(t)],
            settings.timers[usize::from(t)],
            left.is_some(),
        ),
    };
    canvas.text_centered(40, 4, title, Color::ACCENT);

    // Two pairs of the clock's 2× digits (10 px cells, 2 px gaps), the
    // colon between; for settings, hours on the left half and minutes on
    // the right, matching where to swipe.
    let (pair, labels, color) = match left {
        Some(left) if left < 60 * 60 => ((left / 60, left % 60), ("MIN", "SEC"), Color::ACCENT),
        Some(left) => (
            (left / 3600, left / 60 % 60),
            ("HOURS", "MINUTES"),
            Color::ACCENT,
        ),
        None => (
            (i64::from(value / 60), i64::from(value % 60)),
            ("HOURS", "MINUTES"),
            Color::TEXT,
        ),
    };
    let digits = [pair.0 / 10, pair.0 % 10, pair.1 / 10, pair.1 % 10];
    for (&digit, x) in digits.iter().zip([12, 24, 44, 56]) {
        draw_digit(canvas, x, 22, digit as u64, 2, color);
    }
    for y in [26, 32] {
        canvas.fill_rect(39, y, 2, 2, Color::ACCENT);
    }
    canvas.text_centered(20, 44, labels.0, Color::TEXT_FAINT);
    canvas.text_centered(60, 44, labels.1, Color::TEXT_FAINT);
    let hint = if left.is_some() {
        "LEFT"
    } else {
        "SWIPE UP/DOWN"
    };
    canvas.text_centered(40, 51, hint, Color::TEXT_FAINT);

    let (left, left_color) = match (editing, on) {
        (Editing::Timer(_), true) => ("STOP", Color::BAD),
        (Editing::Timer(_), false) => ("START", Color::GOOD),
        (_, true) => ("ON", Color::GOOD),
        (_, false) => ("OFF", Color::TEXT_FAINT),
    };
    let right = if editing == Editing::SmartWeekday {
        "NEXT"
    } else {
        "DONE"
    };
    canvas.fill_rect(2, EDITOR_BUTTONS_Y, 36, 18, left_color);
    canvas.text_centered(20, EDITOR_BUTTONS_Y + 7, left, BACKGROUND);
    canvas.fill_rect(42, EDITOR_BUTTONS_Y, 36, 18, Color::ACCENT);
    canvas.text_centered(60, EDITOR_BUTTONS_Y + 7, right, BACKGROUND);
}

/// The time, big, and how to stop it.
fn render_ringing(canvas: &mut Canvas, ringing: &Ringing, status: &Status) {
    canvas.clear(BACKGROUND);
    let title = match ringing.kind {
        alarm::Kind::Smart => "GOOD MORNING",
        alarm::Kind::At(_) | alarm::Kind::Snooze => "ALARM",
        alarm::Kind::Timer(_) => "TIME'S UP",
    };
    canvas.text_centered(40, 6, title, Color::ACCENT);
    canvas.sprite(37, 15, font::BELL_W, &font::BELL, 1, Color::ACCENT);
    if let Some(second) = status.local_second {
        let time = second.div_euclid(60).rem_euclid(24 * 60);
        let digits = [time / 600, time / 60 % 10, time % 60 / 10, time % 10];
        for (&digit, x) in digits.iter().zip([12, 24, 44, 56]) {
            draw_digit(canvas, x, 28, digit as u64, 2, Color::TEXT);
        }
        for y in [32, 38] {
            canvas.fill_rect(39, y, 2, 2, Color::ACCENT);
        }
    }
    canvas.text_centered(40, 56, "TAP: SNOOZE", Color::TEXT_DIM);
    canvas.text_centered(40, 66, "BUTTON: STOP", Color::TEXT_DIM);
}

/// One big 5×7 digit at `scale`, with a 1 px drop shadow down and right.
fn draw_digit(canvas: &mut Canvas, x: i32, y: i32, digit: u64, scale: i32, color: Color) {
    let glyph = &font::DIGITS[digit as usize];
    canvas.sprite(x + 1, y + 1, font::DIGIT_W, glyph, scale, Color::SHADOW);
    canvas.sprite(x, y, font::DIGIT_W, glyph, scale, color);
}

/// The battery: an icon filled to the charge, and the percentage. While
/// charging, a bolt goes before the percentage and the fill creeps up a
/// pixel a second from the charge to full (an old handheld's charging
/// animation), since the "good" colour alone barely differs from text in
/// some themes.
fn render_battery(canvas: &mut Canvas, battery: &Battery, uptime_secs: u64) {
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
    let mut fill = (percent * 8).div_ceil(100) as i32;
    if battery.charging {
        let steps = 8 - fill + 1;
        fill += (uptime_secs % steps.max(1) as u64) as i32;
    }
    canvas.fill_rect(X + 1, Y + 1, fill.min(8), 3, color);

    let mut text = heapless::String::<8>::new();
    write!(text, "{percent}%").expect("at most 4 chars fit in 8");
    let text_x = X - 2 - font::small_text_width(&text);
    canvas.text(text_x, Y, &text, color);
    if battery.charging {
        let x = text_x - 2 - font::BOLT_W as i32;
        canvas.sprite(x, Y, font::BOLT_W, &font::BOLT, 1, Color::HEART);
    }
}

// ==============================================================================
// Info screen
// ==============================================================================

/// First value row, and the spacing between rows.
const INFO_TOP: i32 = 12;
const INFO_LINE: i32 = 8;

fn render_info(canvas: &mut Canvas, status: &Status, theme: &Theme) {
    canvas.clear(BACKGROUND);
    // The version without its build metadata (`0.9.0-pr.12`); the commit
    // goes at the bottom (see `build.rs`).
    canvas.text(4, 3, concat!("UR ", env!("UR_RELEASE")), Color::ACCENT);

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
    let y = y + INFO_LINE;
    // How long the last night's (or nap's) sleep was, not counting time
    // lying awake.
    canvas.text(4, y, "SLEEP", Color::TEXT_DIM);
    let mut slept = heapless::String::<12>::new();
    match status.sleep {
        Some(sleep) => write!(
            slept,
            "{}:{:02}",
            sleep.asleep_minutes / 60,
            sleep.asleep_minutes % 60
        )
        .expect("fits in 12"),
        None => slept.push_str("--:--").expect("fits in 12"),
    }
    canvas.text_right(77, y, &slept, Color::TEXT);

    // Which commit this build is from, for bug reports: "DEV" for a local
    // `cargo build`, which doesn't know.
    // TODO: tapping still buzzes (a motor test); drop that once alarms
    // exercise the motor.
    canvas.text(4, 74, "BUILD", Color::TEXT_DIM);
    let build = match env!("UR_BUILD") {
        "" => "DEV",
        build => build,
    };
    canvas.text_right(77, 74, build, Color::TEXT);
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
            // The heart's red: the one dash of colour in otherwise
            // single-hue themes (handheld), so progress pops.
            canvas.fill_rect(x, TOP, CELL - 1, CELL, Color::HEART);
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

/// Where a quiet notification's sender goes: everything below the app
/// name, which sits at the very top.
const QUIET_TOP: i32 = 12;
/// Line pitch of 1× text: 5 px glyphs, 2 accent rows, 2 rows' gap.
const QUIET_PITCH: i32 = 9;
/// The most lines a quiet notification's sender may wrap onto at 2× (as
/// many as fit below the app name: 4 × 18 px, less the last gap, is
/// exactly the 68 px there), and at 1× (seven 9 px lines), which it falls
/// back to when 2× won't do. "INTO THE / MISTS: / TALIA / ROSSI" is four
/// lines at 2×: a podcast's name and its episode's, say.
const QUIET_LINES_2X: usize = 4;
const QUIET_LINES: usize = 7;

/// One wrapped line: the longest sender line at 1× is 18 characters, plus
/// a hyphen; room for multi-byte letters.
type Line = heapless::String<48>;

/// How a quiet notification's sender fits on the screen: wrapped lines,
/// and the scale to draw them at. Big (2×) if it fits, else normal size
/// (1×); `None` if it's too long even for that, and has to scroll.
fn quiet_layout(who: &str) -> Option<(heapless::Vec<Line, QUIET_LINES>, i32)> {
    wrap(who, BANNER_TEXT_WIDTH, 2, QUIET_LINES_2X)
        .map(|lines| (lines, 2))
        .or_else(|| wrap(who, BANNER_TEXT_WIDTH, 1, QUIET_LINES).map(|lines| (lines, 1)))
}

/// `text` broken at spaces into lines of at most `width` pixels at
/// `scale`, if it fits in `max_lines` (at most [`QUIET_LINES`]). A word
/// too long for a line of its own is hyphenated, keeping at least three
/// letters for the next line ("KRISTIAN-" / "SEN").
fn wrap(
    text: &str,
    width: i32,
    scale: i32,
    max_lines: usize,
) -> Option<heapless::Vec<Line, QUIET_LINES>> {
    let fits = |s: &str| font::small_text_width(s) * scale <= width;
    let mut lines = heapless::Vec::new();
    let mut rest = text.trim();
    while !rest.is_empty() {
        // The longest prefix that fits, and the last space within it.
        let mut end = 0;
        let mut last_space = None;
        for (i, c) in rest.char_indices() {
            let next = i + c.len_utf8();
            if !fits(&rest[..next]) {
                break;
            }
            end = next;
            if c == ' ' {
                last_space = Some(i);
            }
        }
        if end == 0 {
            return None;
        }
        let mut line = Line::new();
        if end == rest.len() {
            line.push_str(rest).ok()?;
            rest = "";
        } else if let Some(space) = last_space {
            line.push_str(rest[..space].trim_end()).ok()?;
            rest = rest[space..].trim_start();
        } else {
            // Mid-word: back off until the hyphen fits and at least three
            // letters of the word go on to the next line.
            let word_end = rest.find(' ').unwrap_or(rest.len());
            let mut cut = end;
            loop {
                let hyphenated_fits = {
                    let mut probe = Line::new();
                    probe.push_str(&rest[..cut]).ok()?;
                    probe.push('-').ok()?;
                    fits(&probe)
                };
                let left_over = rest[cut..word_end].chars().count();
                if hyphenated_fits && left_over >= 3 {
                    break;
                }
                cut = rest[..cut].char_indices().last().map(|(i, _)| i)?;
                if cut == 0 {
                    return None;
                }
            }
            line.push_str(&rest[..cut]).ok()?;
            line.push('-').ok()?;
            rest = &rest[cut..];
        }
        if lines.len() == max_lines {
            return None;
        }
        lines.push(line).ok()?;
    }
    Some(lines)
}

/// A notification that arrived with the screen off: the whole screen,
/// white on black like a glance, the app small at the very top and the
/// sender wrapped and centred below it, as big as fits (see
/// [`quiet_layout`]; scrolling only if it's longer still).
fn render_quiet_notification(canvas: &mut Canvas, banner: &Banner, now: Instant) {
    let elapsed = now - banner.since;
    canvas.clear(BACKGROUND);
    canvas.marquee(
        3..3 + BANNER_TEXT_WIDTH,
        3,
        &banner.app,
        Color::TEXT_DIM,
        1,
        elapsed,
    );
    match quiet_layout(&banner.who) {
        Some((lines, scale)) => {
            // The capitals centred on the screen, which is where the eye
            // expects a name (centring the space below the app name put
            // two lines 8 px low), unless that would run into the app name:
            // three or more big lines start just below it instead. Accents
            // above the first line are left out of the centring, as most
            // names have none.
            let pitch = QUIET_PITCH * scale;
            let capitals = pitch * (lines.len() as i32 - 1) + 5 * scale;
            let top = (40 - capitals / 2).max(QUIET_TOP + font::ACCENT_ROWS * scale);
            for (i, line) in lines.iter().enumerate() {
                let y = top + pitch * i as i32;
                let x = 40 - font::small_text_width(line) * scale / 2;
                canvas.text_scaled(x, y, line, Color::TEXT, scale);
            }
        }
        None => canvas.marquee(
            3..3 + BANNER_TEXT_WIDTH,
            44,
            &banner.who,
            Color::TEXT,
            2,
            elapsed,
        ),
    }
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
