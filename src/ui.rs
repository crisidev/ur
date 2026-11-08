//! Screens, navigation, and screen power.
//!
//! The UI is immediate-mode: [`App::render`] paints the current screen onto
//! the canvas from scratch, and the canvas works out what actually changed.
//! All state that affects what's shown lives in [`App`] or in the [`Status`]
//! snapshot passed to `render`.
//!
//! The screens sit around the clock face, in a line on each side, nearest
//! first (see [`crate::nav`] for how swipes move along them):
//!
//! - Left, the past: Today (steps, and the heart rate, by the hour), then
//!   Last night (sleep).
//! - Right, what's next: Alarms, then the phone's alarms, once it has sent
//!   some.
//! - Above: the last [`HISTORY`] notifications, newest nearest, one a page,
//!   with when each arrived (see [`crate::arrival`]).
//! - Below: Music, while the phone has media, then Info.
//!
//! Screens away from the clock show a short bar, a handle, in the middle of
//! each edge a swipe leads somewhere from: the one facing the clock (home),
//! and the far one if the line goes on. On the clock, an accent-coloured
//! handle on the top edge means notifications came in that haven't been
//! seen (see [`Notifications::unseen`]).
//!
//! Navigation:
//!
//! - Swipe: the content follows the finger, so swiping left brings in
//!   the screen to the right. Along a line, the same swipe goes further
//!   out and the opposite one comes back; there's no wrapping round, and
//!   no sideways step into another line. The transition sweeps the way
//!   the content moves.
//! - Button: back to the clock, sweeping towards it; on the clock, turn the
//!   screen off. While a call rings: silence it. In the quiet night, a
//!   double press (two within [`DOUBLE_PRESS`]) ends it (see below).
//! - On the clock, while a phone is connected: long press to play or
//!   pause, double tap (two taps within [`DOUBLE_TAP`]) for the next
//!   track. Each buzzes, and shows what it did for a moment where a note
//!   shows while media plays, beside the Bluetooth rune. The phone needn't
//!   have said it has media (see `App::media`). Without a phone, neither
//!   does anything. A long press acts (and buzzes) once the finger has
//!   been down long enough, before it lifts (see [`crate::touch`]); that
//!   goes for every long press. Both work with the screen off, too (see
//!   below).
//! - Long press elsewhere: on the alarms screen, switch an alarm on or off;
//!   on the info screen, offer storage mode (see [`crate::system_off`]),
//!   which a tap then confirms, and anything else cancels. Elsewhere,
//!   nothing.
//! - On the info screen, tap the theme ("THEME") row for the next colour
//!   theme within the current group (dark themes after sunset, light ones
//!   after sunrise; each group remembers its own), or the brightness
//!   ("LIGHT") row to switch between auto and soft (see
//!   [`crate::brightness`]).
//! - Screens that take over (the alarm editor, a ringing alarm, a call,
//!   pairing, an update, the storage prompt) get every touch: swipes there
//!   don't navigate.
//! - While the screen is off, a button press, a deliberate tap (see
//!   [`wake::deliberate`]), or a double tap (two taps within
//!   [`DOUBLE_TAP`]): wake to the clock. Other touches don't, so a brush of
//!   a sleeve doesn't light the screen. (The waking input is *only* a
//!   wake-up; it isn't also acted on.) While a phone is connected, outside
//!   the quiet night, the clock's media gestures work too, without waking
//!   the screen: a long press plays or pauses, and a double tap is the
//!   next track. Each buzzes, and that's all: the screen stays dark. A
//!   deliberate tap still wakes the screen at once, so only a double tap
//!   whose first tap is light stays dark; after a deliberate one, the
//!   second is the next track on the clock face it woke to. See
//!   [`crate::wake`] for the guards against brushes.
//! - No input for [`SCREEN_TIMEOUT`] (longer on the music screen, for a
//!   long title to scroll by; never while a banner shows): turn the screen
//!   off.
//! - Turning the wrist to look (see [`crate::glance`]) while the screen is
//!   off: a glance, just the time in white on black at a low backlight,
//!   for [`GLANCE_TIME`]. While the phone navigates, the next turn instead,
//!   with the time smaller above it. A tap or press during it (one is
//!   enough: the wearer is looking) wakes the screen properly. While a
//!   phone is connected, outside the quiet night, the clock's media
//!   gestures work during a glance too, and the glance stays (with what
//!   it did in a corner, for a moment): a long press plays or pauses, and
//!   a double tap is the next track. As from off, that takes a light
//!   first tap: a deliberate one (held at least
//!   [`wake::DELIBERATE_TAP_MS`]) wakes the screen at once, and the second
//!   is the next track on the clock face it woke to. A light tap on its
//!   own does nothing.
//!
//! Alarms (see [`crate::alarm`] for when they ring):
//!
//! - On the alarms screen, tap a row (smart wake, alarm, timer) to edit it;
//!   long-press to switch it on or off (or start/stop the timer).
//! - In the editor: swipe up/down on the left half for hours, the right
//!   half for minutes; tap the bottom left to switch it on/off, the bottom
//!   right for the next page or done. The button also means done.
//! - The phone's alarms (see [`alarm::Phone`]) have a screen of their own,
//!   read-only: they're set in Gadgetbridge.
//! - Ringing takes over the screen: tap to snooze, button to stop. Alarms
//!   start buzzing gently and build up (see [`Pattern::ALARM`]), with the
//!   backlight low and brightening over a minute; timers buzz at full
//!   strength straight away.
//!
//! From the phone:
//!
//! - Notifications: a two-line banner (app, then sender) for
//!   [`BANNER_TIME`] and a short buzz. If the screen was off, the
//!   notification gets the whole (dim, white on black) screen instead, like
//!   a glance, and the screen sleeps again after. Either way, it's kept in
//!   the history above the clock; one arriving while that's showing goes
//!   straight to its first page instead of a banner.
//! - Calls: take over the screen, ringing, until the phone says the call is
//!   over (answered, rejected, ended) or the user acts.
//! - Media info: enables the music screen.
//! - Directions, while the phone navigates (see [`crate::route`]): only on
//!   a glance. As a turn comes near, a double tap, and with the screen off
//!   a glance at it.
//!
//! The quiet night (see [`sleep::Night`] for when it starts and ends: from
//! falling asleep until the morning's first alarm, or [`sleep::QUIET_AWAKE`]
//! minutes awake):
//!
//! - Notifications neither buzz nor light the screen. The last one waits
//!   as a banner for the next time the screen is properly on (and all of
//!   them in the history).
//! - Touches don't wake the screen (pillows and bedding tap it); the button
//!   and turning the wrist still do. Once it's on, touches work as usual.
//! - A double press of the button ends it: the first press wakes the
//!   screen (dimly) as usual, and the second, rather than turning it off
//!   again, brightens it.
//! - Everything shows in the dim white-on-black look, at the glance's low
//!   backlight.
//! - With the screen off, the touch controller sleeps (see
//!   [`crate::touch::set_asleep`]).
//! - Alarms (which end the night), timers, "find my watch", and calls still
//!   ring and buzz.

use core::fmt::Write as _;
use core::num::NonZeroU32;

use embassy_time::{Duration, Instant};
use heapless::String;

use crate::InputEvent;
use crate::alarm;
use crate::arrival;
use crate::battery::Battery;
use crate::ble;
use crate::brightness;
use crate::canvas::{Canvas, Color, Sweep, marquee_cycle};
use crate::clock::DateTime;
use crate::day::Day;
use crate::font;
use crate::gb::{CallAction, MusicCommand, Outgoing};
use crate::motor::{self, Pattern};
use crate::nav::{self, Place, Side};
// Defined apart from the UI, so that `gb` (which decodes them) builds
// without it, in host tests; see `phone`.
pub use crate::phone::{PhoneEvent, Unpaired};
use crate::route::{self, Route, Turn};
use crate::sleep::{self, Change, Over};
use crate::theme::{self, Theme};
use crate::touch::Gesture;
use crate::wake::{self, Action};

/// How long the screen stays on after the last input (see
/// [`App::screen_timeout`] for when it's longer). Glances by button
/// dominate the screen's time on, so this is a big lever on the battery
/// (~45 µA going from 15 s to 10 s; see docs/power-plan.md, #10), and a
/// look at the clock rarely needs more.
const SCREEN_TIMEOUT: Duration = Duration::from_secs(10);
/// How long a glance shows the time.
const GLANCE_TIME: Duration = Duration::from_secs(5);
/// With the screen off, a tap that isn't deliberate (see
/// [`wake::deliberate`]) still wakes it as the second of two taps this
/// close together; while a phone is connected, that's the next track
/// instead. On a glance or the clock, two taps this close are the next
/// track.
const DOUBLE_TAP: Duration = Duration::from_millis(500);
/// In the quiet night, two presses of the button this close together end
/// it (see [`App::on_input`]).
const DOUBLE_PRESS: Duration = Duration::from_millis(500);
/// How long a notification banner shows.
const BANNER_TIME: Duration = Duration::from_secs(4);
/// A banner's `until` while it waits to be seen: a notification that came
/// in the quiet night. Its time starts once it's on screen (see
/// [`App::on_tick`]).
const UNSEEN: Instant = Instant::MAX;
/// Stop ringing after this long, in case the phone never says the call is
/// over (e.g. the connection dropped mid-call).
const MAX_RING: Duration = Duration::from_secs(45);
/// How many ticks (seconds) a media gesture's icon shows on the clock face:
/// 1–2 s, depending on when in the second it came.
const MEDIA_HINT_TICKS: u8 = 2;

/// Everything screens may display that the UI doesn't own itself.
pub struct Status<'a> {
    pub uptime_secs: u64,
    /// Local wall-clock time, once the phone has set it.
    pub local_time: Option<DateTime>,
    /// Whether the phone is connected over Bluetooth, which it can only be
    /// paired (see `ble::CONNECTED`).
    pub connected: bool,
    /// Whether a client is connected over a link that never became
    /// encrypted, and why (see [`Unpaired`]).
    pub unpaired: Option<Unpaired>,
    /// While pairing: the passkey to type on the phone.
    pub passkey: Option<u32>,
    /// While a firmware update is arriving: how far along it is, in percent.
    pub update_progress: Option<u8>,
    /// Today's steps (once the time is known).
    pub steps: Option<u32>,
    /// The latest heart rate, unless there's none recent enough to show.
    pub heart_rate: Option<u8>,
    /// The watch's own temperature in °C (see [`crate::temperature`]),
    /// once it's been read.
    pub temperature: Option<i8>,
    /// The most recent sleep within the last day.
    pub sleep: Option<crate::activity::Sleep>,
    /// Local seconds since 1970 (see [`crate::alarm`]), once the time is
    /// known.
    pub local_second: Option<i64>,
    pub battery: &'a Battery,
}

#[derive(Clone, Copy, PartialEq, Eq, defmt::Format)]
enum Screen {
    Clock,
    Today,
    LastNight,
    Alarms,
    PhoneAlarms,
    /// One of the [`Notifications`], by its index (0 the newest).
    Notification(u8),
    Music,
    Info,
    /// Asking whether to switch to storage mode: in no line, only reached
    /// by a long press on the info screen.
    Storage,
}

/// The most screens in a line (the notifications' is the longest).
const LONGEST_LINE: usize = if HISTORY > 2 { HISTORY } else { 2 };
type Screens = heapless::Vec<Screen, LONGEST_LINE>;

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
    /// Its text lives elsewhere, rather than in a copy (see
    /// [`App::banner_lines`]).
    shows: Shows,
    /// When it appeared, for scrolling long lines.
    since: Instant,
    until: Instant,
}

/// What a banner shows.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Shows {
    /// The newest of the [`Notifications`].
    Newest,
    /// That a client can't connect, and why. Not a notification, so not
    /// kept: the Info screen's phone row keeps saying so meanwhile.
    Unpaired(Unpaired),
}

/// How many notifications the history above the clock keeps. Each costs
/// 76 bytes of RAM (its two strings, and when it arrived), where there's
/// little to spare.
const HISTORY: usize = 3;

struct Notification {
    app: String<24>,
    who: String<40>,
    /// When it arrived, in local minutes since 1970 (see [`arrival`]), if
    /// the time was known then. The whole minute rather than the minute
    /// of the day, so a page can say which day: with only a few kept,
    /// they can be days old. It costs nothing over a `u16` minute of the
    /// day, which the strings' 4-byte alignment would pad to 4 bytes
    /// anyway; `NonZero`, so that `None` is free too (minute 0, in 1970,
    /// is never a real time).
    arrived: Option<NonZeroU32>,
}

/// The latest notifications, for the history above the clock.
#[derive(Default)]
struct Notifications {
    /// Newest first.
    list: heapless::Vec<Notification, HISTORY>,
    /// Whether any came in that the wearer hasn't seen: that they haven't
    /// opened the history since. Only opening it counts, not touching the
    /// watch while a banner (or a notification with the screen to itself)
    /// shows: a banner is brief, and a touch then is as likely to be for
    /// the screen beneath. Opening the history shows its first page, the
    /// newest, and one arriving while it's open goes straight there (see
    /// [`App::show_banner`]), so each is seen once its page shows. Seeing
    /// one doesn't remove it: they stay until newer ones push them out.
    unseen: bool,
}

impl Notifications {
    /// Keeps a new notification, forgetting the oldest if there's no room.
    fn push(&mut self, app: String<24>, who: String<40>, arrived: Option<NonZeroU32>) {
        if self.list.is_full() {
            self.list.pop();
        }
        let _ = self.list.insert(0, Notification { app, who, arrived });
        self.unseen = true;
    }
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
    /// Whether it's playing: as the phone last said, or as guessed (see
    /// [`Music::likely_playing`]); `None` while there's nothing to go on.
    /// Gadgetbridge only sends the state when it changes, so after the
    /// watch restarts mid-song, it may never come.
    playing: Option<bool>,
    /// When the track last changed, for scrolling long names.
    since: Instant,
}

impl Music {
    fn new(now: Instant) -> Self {
        Self {
            artist: String::new(),
            track: String::new(),
            playing: None,
            since: now,
        }
    }

    fn playing(&self) -> bool {
        self.playing == Some(true)
    }

    /// Something that most likely means it's playing happened (the track
    /// changed, or the wearer skipped to the next): believe so, unless the
    /// phone has said otherwise, which it will again if it changes.
    fn likely_playing(&mut self) {
        self.playing.get_or_insert(true);
    }

    /// Play or pause was asked for: show the new state straight away; the
    /// phone confirms (or corrects) it shortly after. Not knowing counts as
    /// paused, as it shows.
    fn toggle(&mut self) {
        self.playing = Some(!self.playing());
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
    /// [`theme::DARK`]. Tapping THEME on the info screen advances the one
    /// in use.
    pub themes: ThemeChoice,
    connected: bool,
    notifications: Notifications,
    banner: Option<Banner>,
    /// The screen woke only to show a notification; sleep again after it.
    brief_wake: bool,
    call: Option<Call>,
    /// What the phone is playing, since it last told us.
    music: Option<Music>,
    /// Alarm settings, as stored (`main` saves them when they change).
    pub alarms: alarm::Settings,
    /// The phone's alarms, as stored (`main` saves them when
    /// [`App::take_phone_alarms_changed`] says so).
    pub phone_alarms: alarm::Phone,
    phone_alarms_changed: bool,
    alarm_state: alarm::State,
    ringing: Option<Ringing>,
    /// The alarm editor, while open.
    editing: Option<Editing>,
    /// The local minute, as of the last tick (for edits that need "now").
    local_second: Option<i64>,
    /// While glancing: until when.
    glance_until: Option<Instant>,
    /// When the last tap was, as the possible first of a double tap (see
    /// [`DOUBLE_TAP`]): with the screen off, to wake it (or, with a phone
    /// connected, for the next track); on a glance or the clock, for the
    /// next track. One field for all, as RAM is tight: a tap only counts
    /// while the screen is in the same state (on or off) as for the first,
    /// as waking or any other input clears it. (Bar a deliberate tap that
    /// wakes the screen, from off or a glance, while a phone is connected:
    /// that one may be the first of a double tap on the clock face it
    /// woke to.)
    last_tap: Option<Instant>,
    /// When the button was last pressed, for a double press (see
    /// [`DOUBLE_PRESS`]). Boot, until the first.
    last_press: Instant,
    /// Whether it's the quiet night, and what decides when it ends. (8
    /// bytes, in a build with little RAM to spare.)
    night: sleep::Night,
    /// The brightness setting, as stored (`main` saves it when it changes).
    pub brightness: brightness::Setting,
    /// Whether it's between local midnight and the wearer getting up (see
    /// [`brightness::small_hours`]).
    small_hours: bool,
    /// The user confirmed storage mode; `main` switches the watch off.
    pub storage_mode: bool,
    /// Pairing or an update has the screen (as of the last tick), and so
    /// every touch (see the module docs).
    takeover: bool,
    /// Just after a media gesture on the clock: what it did, as a media
    /// control's icon, and for how many more ticks to show it.
    media_hint: Option<(&'static [u8; 5], u8)>,
    /// The phone's directions, while it navigates (see [`crate::route`]).
    route: Option<Route>,
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
            notifications: Notifications::default(),
            banner: None,
            brief_wake: false,
            call: None,
            music: None,
            alarms,
            phone_alarms: alarm::Phone::default(),
            phone_alarms_changed: false,
            alarm_state: alarm::State::default(),
            ringing: None,
            editing: None,
            local_second: None,
            glance_until: None,
            last_tap: None,
            last_press: Instant::from_ticks(0),
            night: sleep::Night::default(),
            brightness: brightness::Setting::default(),
            small_hours: false,
            storage_mode: false,
            takeover: false,
            media_hint: None,
            route: None,
        }
    }

    /// Whether it's the quiet night (see the module docs).
    pub fn quiet_night(&self) -> bool {
        self.night.quiet()
    }

    /// How bright the backlight should be now, while the screen is on.
    pub fn backlight(&self, now: Instant) -> brightness::Level {
        let waking = self
            .ringing
            .as_ref()
            .filter(|r| !matches!(r.kind, alarm::Kind::Timer(_)))
            .map(|r| (now - r.since).as_millis());
        brightness::level(
            self.brightness,
            self.daytime,
            self.small_hours,
            self.dim(),
            waking,
        )
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

    /// Whether the screen shows the dim white-on-black look (a glance, a
    /// notification that arrived with the screen off, or anything in the
    /// quiet night), for the backlight.
    pub fn dim(&self) -> bool {
        self.glancing() || self.quiet_notification() || self.night.quiet()
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
            &theme::GLANCE
        } else {
            self.chosen_theme()
        }
    }

    /// The theme chosen for the time of day, whether or not it's showing
    /// (it isn't while dim).
    fn chosen_theme(&self) -> &'static Theme {
        let (group, choice): (&[&Theme], u8) = if self.daytime {
            (&theme::LIGHT, self.themes.light)
        } else {
            (&theme::DARK, self.themes.dark)
        };
        group[usize::from(choice) % group.len()]
    }

    /// The next theme in the group currently in use.
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

    /// Whether the phone's alarms changed (synced, or a one-off rang and
    /// turned itself off) since last asked, so they need saving.
    pub fn take_phone_alarms_changed(&mut self) -> bool {
        core::mem::take(&mut self.phone_alarms_changed)
    }

    /// The line of screens on `side` of the clock, nearest first. The
    /// phone's alarms only have a screen once the phone has sent some, and
    /// the music screen only exists while the phone has told us about
    /// media.
    fn line(&self, side: Side) -> Screens {
        let mut line = Screens::new();
        let mut add = |screen| {
            let _ = line.push(screen);
        };
        match side {
            Side::Left => {
                add(Screen::Today);
                add(Screen::LastNight);
            }
            Side::Right => {
                add(Screen::Alarms);
                if !self.phone_alarms.alarms().is_empty() {
                    add(Screen::PhoneAlarms);
                }
            }
            Side::Above => {
                for i in 0..self.notifications.list.len() {
                    add(Screen::Notification(i as u8));
                }
            }
            Side::Below => {
                if self.music.is_some() {
                    add(Screen::Music);
                }
                add(Screen::Info);
            }
        }
        line
    }

    /// Where the current screen is. The storage prompt counts as the info
    /// screen it came from.
    fn place(&self) -> Place {
        let screen = match self.screen {
            Screen::Storage => Screen::Info,
            screen => screen,
        };
        Side::ALL
            .into_iter()
            .find_map(|side| {
                let index = self.line(side).iter().position(|&s| s == screen)?;
                Some(Place::Line { side, index })
            })
            .unwrap_or(Place::Clock)
    }

    /// Follows a swipe, if it goes anywhere (see [`nav::swipe`]).
    fn navigate(&mut self, swipe: nav::Swipe) {
        let Some((place, towards)) = nav::swipe(self.place(), swipe, |side| self.line(side).len())
        else {
            return;
        };
        let screen = match place {
            Place::Clock => Screen::Clock,
            Place::Line { side, index } => self.line(side)[index],
        };
        self.go_to(screen, sweep(towards));
    }

    /// Switches screens, with a transition.
    fn go_to(&mut self, screen: Screen, sweep: Sweep) {
        if screen != self.screen {
            self.screen = screen;
            self.transition = Some(sweep);
        }
        // Opening the history is seeing what's new in it, and a banner for
        // the newest would only cover it.
        if let Screen::Notification(_) = screen {
            self.notifications.unseen = false;
            if self.banner_newest() {
                self.banner = None;
            }
        }
    }

    /// Whether there's a banner for the newest notification (showing, or
    /// waiting to).
    fn banner_newest(&self) -> bool {
        self.banner
            .as_ref()
            .is_some_and(|b| b.shows == Shows::Newest)
    }

    /// A banner's two lines: the app (or a title), then the sender (or
    /// what to do).
    fn banner_lines(&self, shows: Shows) -> (&str, &str) {
        match shows {
            Shows::Newest => self
                .notifications
                .list
                .first()
                .map_or(("", ""), |n| (&n.app, &n.who)),
            Shows::Unpaired(why) => (why.title(), why.hint()),
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
        // In the quiet night, a double press of the button ends it: the
        // wearer is up, and wants the watch as by day, without waiting for
        // `Night` to see them awake for a while. The first press did what
        // a press does (most likely, woke the screen, dimly); the second,
        // instead of its usual (turning the screen off again, from the
        // clock), leaves the screen on, now bright. Outside the quiet
        // night, a double press is just two presses.
        if let InputEvent::Button = event {
            let previous = core::mem::replace(&mut self.last_press, now);
            if self.night.quiet() && now - previous <= DOUBLE_PRESS {
                self.night.end();
                defmt::info!("night: over (button)");
                // Up for the day, for the brightness, as after an alarm.
                self.small_hours = false;
                self.glance_until = None;
                self.brief_wake = false;
                self.last_tap = None;
                self.last_input = now;
                if !self.awake {
                    self.awake = true;
                    self.screen = Screen::Clock;
                }
                return;
            }
        }
        // Touching or pressing during a glance: wake properly (and, as
        // with any wake-up, don't also act on it). Bar the clock face's
        // media gestures, while a phone is connected: a long press plays
        // or pauses, and a double tap whose first tap is light skips, both
        // without waking (a look at the wrist is all the wearer wanted),
        // the glance showing what it did. A deliberate tap wakes the screen
        // at once, and a second then skips, on the clock face it woke to
        // (see `wake::glance`). Not in the quiet night, when bedding
        // presses on a wrist that turned in sleep could otherwise start
        // the music; there any touch only wakes the screen, as before.
        if let Some(until) = self.glance_until {
            let media = self.connected && !self.night.quiet();
            let recent_tap = self.last_tap.is_some_and(|tap| now - tap <= DOUBLE_TAP);
            let action = match event {
                InputEvent::Touch(touch) if media => match touch.gesture {
                    Gesture::LongPress => Action::PlayPause,
                    Gesture::Tap => wake::glance(touch.held_ms, recent_tap),
                    _ => Action::Wake { first_tap: false },
                },
                _ => Action::Wake { first_tap: false },
            };
            let command = match action {
                Action::PlayPause => MusicCommand::PlayPause,
                Action::Next => MusicCommand::Next,
                // A light tap: the glance stays, in case a second follows.
                Action::FirstTap => {
                    self.last_tap = Some(now);
                    return;
                }
                // Anything else wakes the screen properly. After a
                // deliberate tap, a second may then skip, on the clock face
                // it woke to.
                wake => {
                    self.glance_until = None;
                    self.last_input = now;
                    self.last_tap = (wake == Action::Wake { first_tap: true }).then_some(now);
                    return;
                }
            };
            self.media(command);
            self.last_tap = None;
            // Long enough for what it did to show (see `render`).
            let hint = Duration::from_secs(u64::from(MEDIA_HINT_TICKS));
            self.glance_until = Some(until.max(now + hint));
            return;
        }
        // With the screen off, a touch only wakes it if it's a deliberate
        // tap, or the second of a double tap; while a phone is connected,
        // a long press plays or pauses, and a double tap whose first tap
        // is light skips, without waking it. A deliberate tap wakes it at
        // once, and may be the first of a double tap on the clock face it
        // woke to. Anything else (a swipe, a short tap or one at the edge)
        // is most likely a brush, and is ignored, bar remembering a tap.
        // See `wake` for the rules, and the field logs behind them.
        //
        // In the quiet night, no touch does anything: bedding and pillows
        // make deliberate-looking taps, and pairs of them (one night, a
        // 193 ms tap mid-screen, and two taps 150 ms apart, both lit the
        // screen). The button and a glance still wake it.
        let mut wake_tap = None;
        if !self.awake
            && let InputEvent::Touch(touch) = event
        {
            let kind = match touch.gesture {
                Gesture::Tap => wake::Kind::Tap,
                Gesture::LongPress => wake::Kind::LongPress,
                _ => wake::Kind::Swipe,
            };
            let recent_tap = self.last_tap.is_some_and(|tap| now - tap <= DOUBLE_TAP);
            let quiet = self.night.quiet();
            self.last_tap = None;
            let command = match wake::screen_off(
                kind,
                touch.x,
                touch.y,
                touch.held_ms,
                recent_tap,
                self.connected,
                quiet,
            ) {
                Action::Ignore => return,
                Action::FirstTap => {
                    self.last_tap = Some(now);
                    return;
                }
                // Waking, below; while a phone is connected, the tap may
                // also be the first of a double tap on the clock face it
                // wakes to, as on a glance.
                Action::Wake { first_tap: true } => {
                    wake_tap = Some(now);
                    None
                }
                Action::Wake { first_tap: false } => None,
                Action::PlayPause => Some(MusicCommand::PlayPause),
                Action::Next => Some(MusicCommand::Next),
            };
            // A media gesture: the buzz says it took, and the screen stays
            // off. (It used to flash the dim glance with what it did, but
            // the buzz was confirmation enough, and play, pause, and next
            // are told apart by what the phone then plays.)
            if let Some(command) = command {
                // For telling a sleeve's doing from the wearer's in a
                // field log.
                defmt::info!("media: {} with the screen off", command);
                self.media(command);
                return;
            }
        }
        // The first of a double tap on the clock, if this may be the
        // second. (Never a tap from before the screen woke: waking only
        // keeps the tap that woke it, below and on a glance, for the next
        // one.)
        let first_tap = self
            .last_tap
            .take()
            .filter(|&tap| self.awake && now - tap <= DOUBLE_TAP);
        // A user input (or a request for attention) from here on. (Touching
        // or pressing while a banner shows doesn't count as seeing the
        // notification: only opening the history does; see
        // `Notifications::unseen`.)
        //
        // A notification that has the screen to itself (it arrived with
        // the screen off) isn't the clock face, whose gestures it doesn't
        // take: a touch there only makes it a proper wake-up.
        let notification_shown = self.quiet_notification();
        self.last_input = now;
        self.brief_wake = false;
        if !self.awake {
            self.awake = true;
            self.screen = Screen::Clock;
            self.last_tap = wake_tap;
            return;
        }
        // Pairing or an update: what's beneath doesn't show, so touches
        // mustn't change it.
        if self.takeover && matches!(event, InputEvent::Touch(_)) {
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
            InputEvent::Button => match nav::home(self.place()) {
                Some(towards) => self.go_to(Screen::Clock, sweep(towards)),
                None => self.awake = false,
            },
            InputEvent::Touch(touch) => match (touch.gesture, self.screen) {
                // Storage mode needs a long press, then a tap: hard to do by
                // accident, while anything else backs out.
                (Gesture::Tap, Screen::Storage) => self.storage_mode = true,
                (_, Screen::Storage) => self.go_to(Screen::Info, Sweep::LeftToRight),
                (Gesture::LongPress, Screen::Info) => {
                    self.go_to(Screen::Storage, Sweep::RightToLeft);
                    motor::buzz(Pattern::NOTIFY);
                }
                (Gesture::SwipeLeft, _) => self.navigate(nav::Swipe::Left),
                (Gesture::SwipeRight, _) => self.navigate(nav::Swipe::Right),
                (Gesture::SwipeUp, _) => self.navigate(nav::Swipe::Up),
                (Gesture::SwipeDown, _) => self.navigate(nav::Swipe::Down),
                (Gesture::Tap, Screen::Music) if touch.y >= MUSIC_CONTROLS_Y => {
                    // Left, middle, and right thirds.
                    let command = match touch.x {
                        ..27 => MusicCommand::Previous,
                        27..53 => MusicCommand::PlayPause,
                        _ => MusicCommand::Next,
                    };
                    send(Outgoing::Music(command));
                    if let Some(music) = &mut self.music {
                        match command {
                            MusicCommand::PlayPause => music.toggle(),
                            MusicCommand::Next | MusicCommand::Previous => music.likely_playing(),
                        }
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
                (Gesture::Tap, Screen::Info) if INFO_THEME_TAP.contains(&touch.y) => {
                    self.next_theme();
                }
                (Gesture::Tap, Screen::Info) if INFO_LIGHT_TAP.contains(&touch.y) => {
                    self.brightness = self.brightness.next();
                }
                // TODO: remove once something real uses the motor; for now
                // this is how we check it works.
                (Gesture::Tap, Screen::Info) => motor::buzz(Pattern::NOTIFY),
                // Media, from the clock face, while a phone is connected
                // (see `media`): long press to play or pause, double tap for the
                // next track. A single tap does nothing, so it needn't wait
                // to see whether a second follows.
                (Gesture::LongPress, Screen::Clock) if !notification_shown => {
                    self.media(MusicCommand::PlayPause)
                }
                (Gesture::Tap, Screen::Clock) if !notification_shown => match first_tap {
                    Some(_) => self.media(MusicCommand::Next),
                    None => self.last_tap = Some(now),
                },
                _ => {}
            },
        }
    }

    /// Controls the phone's media from the clock face, while a phone is
    /// connected: buzzes, and shows what it did in the media icon's place
    /// for a moment (see [`MEDIA_HINT_TICKS`]), so the wearer knows it
    /// took.
    ///
    /// The phone needn't have said it has media: Gadgetbridge only sends
    /// what plays when it changes, so after the watch restarts there may be
    /// no word for a whole album. Its commands act on whatever player the
    /// phone has anyway (`playpause` toggles it, as a headset's button
    /// does), so they're sent regardless.
    fn media(&mut self, command: MusicCommand) {
        if !self.connected {
            return;
        }
        send(Outgoing::Music(command));
        let icon = match (command, &mut self.music) {
            // As the music screen does, the new state straight away, if
            // it's known.
            (MusicCommand::PlayPause, Some(music)) if music.playing.is_some() => {
                music.toggle();
                if music.playing() {
                    &font::PLAY
                } else {
                    &font::PAUSE
                }
            }
            // Otherwise, not a guess: the toggle's own icon. The phone says
            // which it was once its player changes.
            (MusicCommand::PlayPause, _) => &font::PLAY_PAUSE,
            (MusicCommand::Next, music) => {
                if let Some(music) = music {
                    music.likely_playing();
                }
                &font::NEXT
            }
            (MusicCommand::Previous, music) => {
                if let Some(music) = music {
                    music.likely_playing();
                }
                &font::PREVIOUS
            }
        };
        self.media_hint = Some((icon, MEDIA_HINT_TICKS));
        motor::buzz(Pattern::NOTIFY);
    }

    /// Switches a row's alarm on or off; for the timer, starts or stops it.
    fn toggle(&mut self, row: Editing) {
        match row {
            Editing::SmartWeekday | Editing::SmartWeekend => self.alarms.smart = !self.alarms.smart,
            Editing::At(i) => self.alarms.at[usize::from(i)] ^= true,
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
        let done = |app: &mut Self| app.editing = None;
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
        // New alarms from the phone change nothing on screen bar the
        // phone's alarms screen: no buzz, no light.
        if let PhoneEvent::Alarms(alarms) = event {
            defmt::info!("alarm: {} from the phone", alarms.alarms().len());
            self.phone_alarms.sync(alarms);
            self.phone_alarms_changed = true;
            if self.phone_alarms.alarms().is_empty() && self.screen == Screen::PhoneAlarms {
                self.screen = Screen::Alarms;
            }
            return;
        }
        // Directions only show on a glance; a turn coming up buzzes, and
        // shows itself if the screen is off.
        if let PhoneEvent::Route(step) = event {
            let was_navigating = self.route.is_some();
            let near = Route::update(&mut self.route, step, now.as_millis());
            if self.route.is_some() != was_navigating {
                defmt::info!("route: navigating: {}", self.route.is_some());
            }
            if near {
                self.turn_near(now);
            }
            return;
        }
        // Both of these show as a banner: a notification (kept in the
        // history, too), or that a client can't connect.
        let event = match event {
            PhoneEvent::Notification { app, who } => {
                // The time as of the last tick, at most a second old. (A
                // test notification may be backdated, to see how older
                // ones show.)
                let arrived = self
                    .local_second
                    .map(|second| second.div_euclid(60) - crate::debug::take_backdate_minutes())
                    .and_then(|minute| u32::try_from(minute).ok())
                    .and_then(NonZeroU32::new);
                self.notifications.push(app, who, arrived);
                return self.show_banner(Shows::Newest, now);
            }
            PhoneEvent::Unpaired(why) => return self.show_banner(Shows::Unpaired(why), now),
            event => event,
        };
        // Anything else from the phone gets the full screen, bar media
        // news, which shows nowhere on a glance: the phone confirming a
        // play or pause from one (see `on_input`) mustn't end it.
        if !matches!(
            event,
            PhoneEvent::MusicInfo { .. } | PhoneEvent::MusicState { .. }
        ) && self.glance_until.take().is_some()
        {
            self.last_input = now;
        }
        match event {
            PhoneEvent::Notification { .. }
            | PhoneEvent::Unpaired(_)
            | PhoneEvent::Alarms(_)
            | PhoneEvent::Route(_) => unreachable!("handled above"),
            // Calls ring and light the screen even in the quiet night: one
            // at night is likely to matter. (In the dim look, though.)
            // TODO: make this a setting, if calls at night should be quiet
            // too.
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
                    music.likely_playing();
                }
                music.artist = artist;
                music.track = track;
            }
            PhoneEvent::MusicState { playing } => {
                self.music.get_or_insert_with(|| Music::new(now)).playing = Some(playing)
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

    /// The next turn is near (see [`route::NEAR_METRES`]): a double tap on
    /// the wrist, and, with the screen off, a glance at the turn, so a
    /// look is enough even when the wrist didn't turn the way a glance
    /// needs (on handlebars, say). Not in the quiet night (no one
    /// navigates in their sleep; a stale route shouldn't wake them), nor
    /// over a call or a ringing alarm, which need the motor and screen
    /// more.
    fn turn_near(&mut self, now: Instant) {
        if self.night.quiet() || self.call.is_some() || self.ringing.is_some() {
            return;
        }
        if let Some(route) = &self.route {
            defmt::info!("route: turn near ({})", route.step.turn);
        }
        motor::buzz(Pattern::TURN);
        if !self.awake {
            self.awake = true;
            self.screen = Screen::Clock;
            self.glance_until = Some(now + GLANCE_TIME);
        }
    }

    /// Shows a banner (or keeps it for later, in the quiet night).
    fn show_banner(&mut self, shows: Shows, now: Instant) {
        // Reading the history: a new notification goes straight to its
        // first page, whole, rather than in a banner over an older one
        // (which it has just moved along a page).
        if shows == Shows::Newest
            && self.awake
            && matches!(self.screen, Screen::Notification(_))
            && self.call.is_none()
            && self.ringing.is_none()
        {
            self.go_to(Screen::Notification(0), sweep(Side::Below));
            // Its text scrolls from the last input (see `render`).
            self.last_input = now;
            if !self.night.quiet() {
                motor::buzz(Pattern::NOTIFY);
            }
            return;
        }
        // In the quiet night, it makes no sound or light: it waits until
        // the screen is properly on (see `on_tick`), even if that's now.
        if self.night.quiet() {
            self.banner = Some(Banner {
                shows,
                since: now,
                until: UNSEEN,
            });
            return;
        }
        if self.glance_until.take().is_some() {
            self.last_input = now;
        }
        // With the screen off, it gets the whole screen.
        let (app, who) = self.banner_lines(shows);
        let until = now + shown_for(app, who, !self.awake);
        self.banner = Some(Banner {
            shows,
            since: now,
            until,
        });
        // Not over a ringing alarm, which would go quiet.
        if self.ringing.is_none() {
            motor::buzz(Pattern::NOTIFY);
        }
        if !self.awake {
            self.awake = true;
            self.brief_wake = true;
            self.screen = Screen::Clock;
        }
    }

    /// Once a second: starts or ends the quiet night, and rings any alarm
    /// that's due. `local_second` is the time, if known; `stirring` whether
    /// the wearer just moved (for smart wake); `asleep` whether they're
    /// asleep (see [`crate::activity::asleep`]).
    pub fn check_alarms(
        &mut self,
        now: Instant,
        local_second: Option<i64>,
        stirring: bool,
        asleep: bool,
    ) {
        let before = core::mem::replace(&mut self.local_second, local_second);
        let noon = match (before, local_second) {
            (Some(before), Some(now)) => sleep::noon_between(before, now),
            _ => false,
        };
        // Truncated to 32 bits to keep `Night` small; that's 136 years.
        let nap_hours = sleep::nap_hours(local_second);
        let change = self
            .night
            .update(asleep, now.as_secs() as u32, noon, nap_hours);
        // The night ending is getting up, for the brightness.
        self.small_hours = brightness::small_hours(
            self.small_hours,
            before.zip(local_second),
            matches!(change, Some(Change::Over(_))),
        );
        match change {
            Some(Change::Quiet { nap: false }) => defmt::info!("night: quiet"),
            Some(Change::Quiet { nap: true }) => defmt::info!("night: quiet (nap)"),
            Some(Change::Over(Over::Awake)) => defmt::info!("night: over (awake)"),
            Some(Change::Over(Over::Noon)) => defmt::info!("night: over (noon)"),
            None => {}
        }
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
            && let Some(kind) =
                self.alarm_state
                    .due(&mut self.alarms, &mut self.phone_alarms, second, stirring)
        {
            defmt::info!("alarm: ringing ({})", kind);
            // A one-off phone alarm turned itself off. (Saving after a
            // repeating one too costs a flash write a day: not worth
            // telling them apart for.)
            if let alarm::Kind::Phone(_) = kind {
                self.phone_alarms_changed = true;
            }
            // The morning's first alarm (a snooze is still the morning)
            // ends the night, so it rings with the screen bright. Not a
            // timer: that rings in the dim look, and the night goes on.
            if !matches!(kind, alarm::Kind::Timer(_)) {
                if self.night.end() {
                    defmt::info!("night: over (alarm)");
                }
                // Waking up, even without a quiet night: the small hours
                // are over.
                self.small_hours = false;
            }
            self.ringing = Some(Ringing { kind, since: now });
            self.editing = None;
            // Not a glance any more: that would end, and turn the screen
            // off, mid-ring (and keep the backlight low).
            self.glance_until = None;
            motor::buzz(match kind {
                alarm::Kind::Timer(_) => Pattern::TIMER,
                _ => Pattern::ALARM,
            });
            self.awake = true;
            self.brief_wake = false;
            self.last_input = now;
        }
    }

    /// `keep_awake` holds the screen on regardless of input, e.g. while a
    /// pairing passkey is showing and the user is typing it on the phone.
    pub fn on_tick(&mut self, now: Instant, keep_awake: bool) {
        // That's pairing or an update, which take every touch too.
        self.takeover = keep_awake;
        self.media_hint = match self.media_hint {
            Some((icon, ticks)) if ticks > 1 => Some((icon, ticks - 1)),
            _ => None,
        };
        if Route::expire(&mut self.route, now.as_millis()) {
            defmt::info!("route: no update for a while; dropped");
        }
        // A glance has its own timer: it isn't input, so the screen timeout
        // (which counts from the last input) would end it at once.
        if let Some(until) = self.glance_until {
            if now >= until {
                self.glance_until = None;
                self.awake = false;
            }
            return;
        }
        // A banner waiting to be seen (see `UNSEEN`) starts its time once
        // it's on screen: awake, and not covered by something that takes
        // over the screen.
        if self.awake
            && self.call.is_none()
            && self.ringing.is_none()
            && self.editing.is_none()
            && let Some(shows) = self
                .banner
                .as_ref()
                .filter(|b| b.until == UNSEEN)
                .map(|b| b.shows)
        {
            let (app, who) = self.banner_lines(shows);
            let until = now + shown_for(app, who, false);
            self.banner = Some(Banner {
                shows,
                since: now,
                until,
            });
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
        // Not while a banner shows: one that arrived late in the timeout
        // would otherwise go before it could be read. (It doesn't count as
        // input, so the screen goes off once it's gone, unless input came
        // in the meantime.) One still waiting to be seen (under the alarm
        // editor, say) isn't showing.
        let banner_showing = self.banner.as_ref().is_some_and(|b| b.until != UNSEEN);
        if self.awake && !banner_showing && now - self.last_input >= self.screen_timeout() {
            self.awake = false;
        }
    }

    /// How long the screen stays on after the last input: [`SCREEN_TIMEOUT`],
    /// or on the music screen, long enough for the title and artist to
    /// scroll by once, from wherever they are. They scroll from when the
    /// track started (see `render_music`), not from the last input, so a
    /// whole cycle shows every part of them. That's ~19 s for a 30-letter
    /// title, against the clock face's 10 s; the music screen is opened
    /// rarely enough that the extra time costs little.
    ///
    /// TODO: the notification history's pages could use the same, for a
    /// sender too long to wrap (see `quiet_layout`); none has been seen.
    fn screen_timeout(&self) -> Duration {
        let overflow =
            |text: &str, scale: i32| font::small_text_width(text) * scale - MUSIC_TEXT_WIDTH;
        match (self.screen, &self.music) {
            (Screen::Music, Some(music)) => {
                let overflow = overflow(&music.track, 2).max(overflow(&music.artist, 1));
                SCREEN_TIMEOUT.max(marquee_cycle(overflow))
            }
            _ => SCREEN_TIMEOUT,
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
        // A whole-screen notification's sender only scrolls if it won't
        // wrap; a banner's, if it won't fit on its line.
        let scrolls = |app: &str, who: &str, whole_screen: bool| {
            let who = if whole_screen {
                quiet_layout(who, QUIET_LINES_2X, &mut Lines::new()).is_none()
            } else {
                overflows(who, 1, BANNER_TEXT_WIDTH)
            };
            overflows(app, 1, BANNER_TEXT_WIDTH) || who
        };
        let banner = self.banner.as_ref().is_some_and(|b| {
            let (app, who) = self.banner_lines(b.shows);
            scrolls(app, who, self.quiet_notification())
        });
        let screen = match self.screen {
            Screen::Music => self.music.as_ref().is_some_and(|m| {
                overflows(&m.artist, 1, MUSIC_TEXT_WIDTH)
                    || overflows(&m.track, 2, MUSIC_TEXT_WIDTH)
            }),
            Screen::Notification(i) => self
                .notifications
                .list
                .get(usize::from(i))
                .is_some_and(|n| scrolls(&n.app, &n.who, true)),
            _ => false,
        };
        banner || screen
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
            match &self.route {
                Some(route) => render_route_glance(canvas, status, &route.step),
                None => render_glance(canvas, status),
            }
            // Just after a media gesture, what it did, in the top right
            // corner, which both layouts leave free.
            if let Some((icon, _)) = self.media_hint {
                canvas.sprite(65, 3, font::MEDIA_HINT_W, icon, 2, Color::TEXT);
            }
            return;
        }
        if self.quiet_notification()
            && let Some(banner) = &self.banner
        {
            let (app, who) = self.banner_lines(banner.shows);
            render_quiet_notification(canvas, app, who, now - banner.since, QUIET_LINES_2X);
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
        let next_alarm = status.local_second.and_then(|minute| {
            self.alarm_state
                .next(&self.alarms, &self.phone_alarms, minute)
        });
        match self.screen {
            Screen::Clock => {
                // While media plays, a note; just after a media gesture,
                // what it did.
                let icon = match (self.media_hint, &self.music) {
                    (Some((icon, _)), _) => Some((font::MEDIA_HINT_W, &icon[..], 3)),
                    (None, Some(music)) if music.playing() => {
                        Some((font::NOTE_W, &font::NOTE[..], 2))
                    }
                    _ => None,
                };
                // The slot stays while the phone has media at all, playing
                // or not, so the temperature beside it doesn't shift as
                // the icon changes (or goes, when paused).
                let media = (self.music.is_some() || self.media_hint.is_some()).then_some(icon);
                render_clock(canvas, status, next_alarm, media)
            }
            // From the day's history, only while it shows: walking it
            // costs a little time, while keeping it up to date would cost
            // RAM.
            Screen::Today => render_today(canvas, crate::activity::today().as_ref(), status.steps),
            Screen::LastNight => render_last_night(canvas, status.sleep),
            Screen::Alarms => render_alarms(canvas, &self.alarms, &self.alarm_state, status),
            Screen::PhoneAlarms => render_phone_alarms(canvas, &self.phone_alarms),
            // Like a notification that arrived with the screen off, its
            // text scrolling from when the page opened (the swipe there),
            // but a line short at 2×, to leave the bottom for when it
            // arrived, and the handle (see `edge_bar`) below that.
            Screen::Notification(i) => match self.notifications.list.get(usize::from(i)) {
                Some(n) => {
                    render_quiet_notification(
                        canvas,
                        &n.app,
                        &n.who,
                        now - self.last_input,
                        QUIET_LINES_2X - 1,
                    );
                    if let (Some(arrived), Some(second)) = (n.arrived, status.local_second) {
                        render_arrived(canvas, arrived.get().into(), second.div_euclid(60));
                    }
                }
                None => render_clock(canvas, status, next_alarm, None),
            },
            Screen::Music => match &self.music {
                Some(music) => render_music(canvas, music, now),
                None => render_clock(canvas, status, next_alarm, None),
            },
            Screen::Info => render_info(canvas, status, self.chosen_theme(), self.brightness),
            Screen::Storage => render_storage_prompt(canvas),
        }
        match self.place() {
            // Notifications nobody has seen: an accent-coloured handle on
            // the edge they're behind. The clock face has no other hints:
            // its edges are full, and it's where the wearer learns the
            // layout from.
            Place::Clock if self.notifications.unseen => {
                edge_bar(canvas, Side::Above, Color::ACCENT);
            }
            Place::Line { side, index } if self.screen != Screen::Storage => {
                render_hint(canvas, side, index, self.line(side).len());
            }
            _ => {}
        }
        if let Some(banner) = &self.banner {
            let (app, who) = self.banner_lines(banner.shows);
            render_banner(canvas, app, who, now - banner.since);
        }
    }
}

/// Queues a message for the phone (dropped if not connected or the queue is
/// full; none of these are worth retrying later).
fn send(message: Outgoing) {
    let _ = ble::OUTBOX.try_send(message);
}

/// The transition for moving towards a screen on `side`: the content
/// moves away from it, so the new screen comes in from there.
fn sweep(towards: Side) -> Sweep {
    match towards {
        Side::Left => Sweep::LeftToRight,
        Side::Right => Sweep::RightToLeft,
        Side::Above => Sweep::TopToBottom,
        Side::Below => Sweep::BottomToTop,
    }
}

/// A short bar along `edge` of the screen, in the middle: a handle, as on
/// a drawer, saying there's a screen that way to swipe to. (Dots, one a
/// screen, read as specks on the screen; chevrons, as part of the text
/// beside them.) It takes the outer 2 pixels, and screens keep one more
/// clear: their text starts 3 or 4 in.
fn edge_bar(canvas: &mut Canvas, edge: Side, color: Color) {
    const LENGTH: i32 = 12;
    const MIDDLE: i32 = 40 - LENGTH / 2;
    let (x, y, w, h) = match edge {
        Side::Left => (0, MIDDLE, 2, LENGTH),
        Side::Right => (78, MIDDLE, 2, LENGTH),
        Side::Above => (MIDDLE, 0, LENGTH, 2),
        Side::Below => (MIDDLE, 78, LENGTH, 2),
    };
    canvas.fill_rect(x, y, w, h, color);
}

/// Where a swipe leads from a screen `index` out along the line on `side`,
/// of `count`: home, always, and further out if there's more.
fn render_hint(canvas: &mut Canvas, side: Side, index: usize, count: usize) {
    edge_bar(canvas, side.opposite(), Color::TEXT_DIM);
    if index + 1 < count {
        edge_bar(canvas, side, Color::TEXT_DIM);
    }
}

// ==============================================================================
// Clock face
// ==============================================================================
//
// Canvas layout (80×80), y ranges inclusive:
//
//    3..7    status bar: Bluetooth or time-sync state, then the watch's
//            temperature (left); battery % and icon (right)
//   13..34   HH:MM in 5×7 digits at 3× (15×21 each), with a drop shadow
//   38..44   date, YYYY-MM-DD, under the hours (left); seconds, small and
//            in the accent colour, under the minutes (right)
//   56..62   heart rate (left); the next alarm or timer (right)
//   68..74   a foot, then ten blocks for the day's steps (x 10..58),
//            and their scale, ×1K or ×10K (from x = 61)
//
// Everything below the status bar lines up with the time's outer edges
// (x = 4 and 75). The top half is the time; the bottom half is the
// readings, packed into two rows, with the steps drawn as a share of the
// day's goal rather than a number: the exact count is on Today, one
// swipe right.

const BACKGROUND: Color = Color::BACKGROUND;

/// The day's steps that fill the clock face's ten blocks at 1,000 a
/// block; from there, the blocks count 10,000 each (see `render_clock`).
///
/// Fixed, because there's nowhere to learn the wearer's own: Gadgetbridge
/// keeps a step goal (its "fitness_goal" setting, 8,000 by default) for its
/// charts, but never sends it to a Bangle.js. Its driver has no message
/// for the user's settings (the call that would send them,
/// `sendSettings`, is commented out in `BangleJSDeviceSupport`), and its
/// `act` message only switches live heart rate and steps on and off.
/// 10,000 is the round number most people know.
///
/// TODO: a goal set on the watch: a tap-to-cycle row on Info, like THEME
/// and LIGHT (say 6k, 8k, 10k, 12k), kept with the settings in flash. The
/// meter's scales ("×1K", "×10K") would then follow the goal.
const STEP_GOAL: u32 = 10_000;

/// `media` is an icon for the status bar, beside the Bluetooth rune: its
/// width, rows, and top.
fn render_clock(
    canvas: &mut Canvas,
    status: &Status,
    next_alarm: Option<(alarm::Kind, i64)>,
    // `None` without media; otherwise the icon to show in the media slot,
    // if any: (width, rows, y).
    media: Option<Option<(u32, &[u8], i32)>>,
) {
    canvas.clear(BACKGROUND);
    render_battery(canvas, status.battery, status.uptime_secs);

    // Status bar, left: Bluetooth when connected, and then the media icon
    // (media comes from the phone, so only while connected); a note if we
    // don't know the time (in which case the clock shows time since boot).
    let temperature_x = if status.connected {
        canvas.sprite(3, 2, font::BLUETOOTH_W, &font::BLUETOOTH, 1, Color::ACCENT);
        let x = 3 + font::BLUETOOTH_W as i32 + 3;
        // One width for every media icon (the widest, the note's), each
        // centred in it, so nothing after it moves as the icon changes.
        const MEDIA_SLOT: u32 = font::NOTE_W;
        match media {
            Some(icon) => {
                if let Some((width, rows, y)) = icon {
                    let left = x + (MEDIA_SLOT.saturating_sub(width) / 2) as i32;
                    canvas.sprite(left, y, width, rows, 1, Color::ACCENT);
                }
                Some(x + MEDIA_SLOT as i32 + 3)
            }
            None => Some(x),
        }
    } else if status.local_time.is_none() {
        canvas.text(3, 3, "NO TIME", Color::TEXT_FAINT);
        None
    } else {
        Some(3)
    };

    // Then the watch's own temperature, in whole °C, packed against the
    // left. The widest ("-10°", 15 px; nothing in the sensor's -40 to
    // 85 °C is wider) after the Bluetooth icon and the widest media icon
    // (the note, 6 px) ends at x = 34, clear of the battery's widest (charging at
    // 100%: the bolt starts at x = 43). "NO TIME" (27 px) and a temperature can't both fit beside
    // that, so until the phone sets the clock (just after a cold boot),
    // the note wins.
    // TODO: °F, for those who'd want it.
    if let (Some(degrees), Some(x)) = (status.temperature, temperature_x) {
        let mut text = heapless::String::<6>::new();
        write!(text, "{degrees}°").expect("an i8 is at most 4 chars, and ° is 2 bytes");
        canvas.text(x, 3, &text, Color::TEXT);
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

    // Under the time, a footnote line, lined up with its outer edges: the
    // date under the hours, and the seconds under the minutes. The seconds
    // are rarely what a look at the watch is for (the colon's blink already
    // says it's alive), so they're small, the time's own 5×7 glyphs at 1×
    // (without the shadow, which at 1× would only thicken them), and in the
    // accent colour, so they're still easy to find.
    const NOTE_Y: i32 = 39;
    let mut date = heapless::String::<10>::new();
    match &status.local_time {
        Some(t) => {
            write!(date, "{:04}-{:02}-{:02}", t.year, t.month, t.day).expect("exactly 10 chars")
        }
        None => date.push_str("----------").expect("exactly 10 chars"),
    }
    canvas.text(4, NOTE_Y, &date, Color::TEXT_DIM);
    for (digit, x) in [seconds / 10, seconds % 10].into_iter().zip([64, 70]) {
        let glyph = &font::DIGITS[digit as usize];
        canvas.sprite(x, NOTE_Y - 1, font::DIGIT_W, glyph, 1, Color::ACCENT);
    }

    // The readings, in one row: the heart rate from the left edge, and the
    // next alarm within a day up to the right one, each an icon, 2 px, and
    // its value, the icon a row above the value's capitals.
    const READING_Y: i32 = 57;
    let mut bpm = heapless::String::<3>::new();
    match status.heart_rate {
        Some(rate) => write!(bpm, "{rate}").expect("a u8 is at most 3 digits"),
        None => bpm.push_str("--").expect("2 chars fit in 3"),
    }
    canvas.sprite(
        4,
        READING_Y - 1,
        font::HEART_W,
        &font::HEART,
        1,
        Color::HEART,
    );
    canvas.sprite(
        4,
        READING_Y - 1,
        font::HEART_W,
        &font::HEART_SHINE,
        1,
        Color::HEART_SHINE,
    );
    canvas.text(4 + font::HEART_W as i32 + 2, READING_Y, &bpm, Color::TEXT);

    if let (Some((kind, at)), Some(now)) = (next_alarm, status.local_second)
        && at - now <= 24 * 60 * 60
    {
        // A timer counts down; anything else shows when it rings (a smart
        // alarm at its latest). A timer's "42 MIN" is written "42M" here:
        // the full form, after the widest heart rate ("188"), would leave
        // the two groups 3 px apart, reading as one.
        let left = (at - now).max(0);
        let time = match kind {
            alarm::Kind::Timer(_) if (10 * 60..60 * 60).contains(&left) => {
                let mut time = heapless::String::new();
                write!(time, "{}M", (left + 59) / 60).expect("fits in 8");
                time
            }
            alarm::Kind::Timer(_) => countdown(left),
            _ => {
                let mut time = heapless::String::new();
                time.push_str(&hhmm(at.div_euclid(60).rem_euclid(24 * 60)))
                    .expect("fits");
                time
            }
        };
        let x = 76 - font::small_text_width(&time);
        canvas.text(x, READING_Y, &time, Color::TEXT);
        let (icon, width) = match kind {
            alarm::Kind::Timer(_) => (&font::HOURGLASS, font::HOURGLASS_W),
            _ => (&font::BELL, font::BELL_W),
        };
        let icon_x = x - width as i32 - 2;
        canvas.sprite(icon_x, READING_Y - 1, width, icon, 1, Color::ACCENT);
    }

    // The day's steps, as ten blocks, like an old game's health bar: "six
    // of ten" can be counted at a glance, where a number has to be read.
    // Up to [`STEP_GOAL`], each block is 1,000 steps; from the goal on,
    // the bar starts again at 10,000 a block, so a long day keeps
    // filling it, and the change of scale is what says the goal was
    // reached. The scale stands to the bar's right, "×1K" or "×10K".
    // Each block is a whole multiple (999 steps fill none; 19,999 one
    // block of 10,000), and from 100,000 the bar stays full. Before the
    // time is known there's no day to count, so it's an empty track, at
    // 1,000 a block.
    //
    // Foot, 2 px, the bar, 2 px, and the widest scale ("×10K", 15 px)
    // take exactly the row's 72 px (x = 4..75), with the blocks 4 px wide
    // and 1 px apart: 5 px wide, they'd leave the scale no room. The
    // scale starts where the bar ends, so it reads as the bar's, and the
    // bar never moves as it changes.
    const STEPS_Y: i32 = 69;
    const BLOCKS: u32 = 10;
    const BLOCK_W: i32 = 4;
    const BAR_X: i32 = 4 + font::FOOT_W as i32 + 2;
    const SCALE_X: i32 = BAR_X + BLOCKS as i32 * (BLOCK_W + 1) - 1 + 2;
    const _: () = assert!(STEP_GOAL == 10_000, "the scales say 1K and 10K");
    canvas.sprite(
        4,
        STEPS_Y - 1,
        font::FOOT_W,
        &font::FOOT,
        1,
        Color::TEXT_DIM,
    );
    let steps = status.steps.unwrap_or(0);
    let (per_block, scale) = if steps < STEP_GOAL {
        (STEP_GOAL / BLOCKS, "×1K")
    } else {
        (STEP_GOAL, "×10K")
    };
    let full = (steps / per_block).min(BLOCKS);
    for i in 0..BLOCKS {
        let color = if i < full {
            Color::ACCENT
        } else {
            Color::TRACK
        };
        let x = BAR_X + i as i32 * (BLOCK_W + 1);
        canvas.fill_rect(x, STEPS_Y, BLOCK_W, 5, color);
    }
    canvas.text(SCALE_X, STEPS_Y, scale, Color::TEXT_DIM);
}

/// The glance: the hours above the minutes, as big as fits with room
/// around them, and nothing else.
///
/// The sizes follow from three rules. The digits scale by whole numbers
/// (they're pixel art). The block keeps a margin of about a tenth of the
/// screen (8 px), as one big focal element needs room to read as placed
/// rather than spilling over. And by proximity, the gap between the
/// hours' digits (one glyph cell) is half the gap between hours and
/// minutes (two cells), so they read as two numbers rather than a grid of
/// four digits. 5× (25×35 digits) can't keep the margin; 4× (20×28) can:
/// a 45×65 block, with 7–8 px above and below.
fn render_glance(canvas: &mut Canvas, status: &Status) {
    canvas.clear(BACKGROUND);
    let Some(t) = &status.local_time else {
        return;
    };
    const SCALE: i32 = 4;
    const WIDTH: i32 = font::DIGIT_W as i32 * SCALE;
    const HEIGHT: i32 = 7 * SCALE;
    const GAP_X: i32 = SCALE;
    const GAP_Y: i32 = 2 * SCALE;
    // Centred, counting the 1 px shadow.
    const LEFT: i32 = (80 - (2 * WIDTH + GAP_X + 1)) / 2;
    const TOP: i32 = (80 - (2 * HEIGHT + GAP_Y + 1)) / 2;
    for (row, value) in [t.hour, t.minute].into_iter().enumerate() {
        let y = TOP + row as i32 * (HEIGHT + GAP_Y);
        let value = u64::from(value);
        draw_digit(canvas, LEFT, y, value / 10, SCALE, Color::TEXT);
        draw_digit(
            canvas,
            LEFT + WIDTH + GAP_X,
            y,
            value % 10,
            SCALE,
            Color::TEXT,
        );
    }
}

/// The glance while the phone navigates: the next turn takes the middle of
/// the screen, as an arrow with the distance under it, then the street
/// (two lines of the small font at most, as Gadgetbridge gives it). The
/// time stays, smaller, at the top: a glance is still for the time too,
/// but on the way somewhere, the turn is what a look at the wrist is for.
///
/// From the top: the time (2×, 10 px), the arrow (16×16 at 2×), the
/// distance (2×), and the street (1×), with 4 px between them; it just
/// fits, with the street's second line ending a pixel above the bottom.
fn render_route_glance(canvas: &mut Canvas, status: &Status, step: &route::Step) {
    canvas.clear(BACKGROUND);
    const TIME_Y: i32 = 3;
    const ARROW_Y: i32 = 17;
    const DISTANCE_Y: i32 = ARROW_Y + 32 + 4;
    const STREET_Y: i32 = DISTANCE_Y + 10 + 4;
    let centred = |canvas: &mut Canvas, y: i32, text: &str, scale: i32, color: Color| {
        let width = font::small_text_width(text) * scale;
        canvas.text_scaled(40 - width / 2, y, text, color, scale);
    };
    if let Some(t) = &status.local_time {
        let time = hhmm(i64::from(t.hour) * 60 + i64::from(t.minute));
        centred(canvas, TIME_Y, &time, 2, Color::TEXT_DIM);
    }

    let (arrow, mirror) = match step.turn {
        Turn::Straight => (&font::TURN_STRAIGHT, false),
        Turn::Right => (&font::TURN_RIGHT, false),
        Turn::Left => (&font::TURN_RIGHT, true),
        Turn::SlightRight | Turn::KeepRight => (&font::TURN_SLIGHT_RIGHT, false),
        Turn::SlightLeft | Turn::KeepLeft => (&font::TURN_SLIGHT_RIGHT, true),
        Turn::SharpRight => (&font::TURN_SHARP_RIGHT, false),
        Turn::SharpLeft => (&font::TURN_SHARP_RIGHT, true),
        // Gadgetbridge doesn't know which side the traffic keeps to, so a
        // U-turn at a roundabout is drawn as a plain one, going round left.
        Turn::UturnLeft | Turn::RoundaboutUturn => (&font::TURN_UTURN_LEFT, false),
        Turn::UturnRight => (&font::TURN_UTURN_LEFT, true),
        Turn::RoundaboutRight => (&font::TURN_ROUNDABOUT_RIGHT, false),
        Turn::RoundaboutLeft => (&font::TURN_ROUNDABOUT_RIGHT, true),
        Turn::RoundaboutStraight => (&font::TURN_ROUNDABOUT_STRAIGHT, false),
        Turn::Finish => (&font::TURN_FINISH, false),
        Turn::OffRoute => (&font::TURN_OFF_ROUTE, false),
        Turn::Unknown => (&font::TURN_UNKNOWN, false),
    };
    let arrow_x = 40 - font::TURN_W as i32;
    canvas.sprite16(arrow_x, ARROW_Y, arrow, mirror, 2, Color::TEXT);

    // "1,234 km" fits at 2× (62 px); the 1× fallback is for safety.
    let scale = if font::small_text_width(&step.distance) * 2 <= 76 {
        2
    } else {
        1
    };
    centred(canvas, DISTANCE_Y, &step.distance, scale, Color::TEXT);

    let mut lines = Lines::new();
    match wrap(&step.street, 76, 1, 2, &mut lines) {
        Some(()) => {
            for (i, line) in lines.iter().enumerate() {
                centred(canvas, STREET_Y + i as i32 * 7, line, 1, Color::TEXT_DIM);
            }
        }
        // Too long even for two lines (one very long word): its start.
        None => {
            canvas.text(2, STREET_Y, &step.street, Color::TEXT_DIM);
        }
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
// The past: today and last night
// ==============================================================================

/// Today's charts: a column per hour from midnight, [`CHART_BAR`] wide and
/// [`CHART_PITCH`] apart, from [`CHART_LEFT`] (24 of them end at x = 75).
const CHART_LEFT: i32 = 4;
const CHART_PITCH: i32 = 3;
const CHART_BAR: i32 = 2;

/// Today's steps and heart rate: for each, a figure for the day, then a
/// chart hour by hour since midnight, over a faint baseline that shows
/// how much of the day is left.
fn render_today(canvas: &mut Canvas, day: Option<&Day>, steps: Option<u32>) {
    canvas.clear(BACKGROUND);
    canvas.text(4, 3, "TODAY", Color::ACCENT);
    // Figures end at x = 74, leaving the right edge to the hint.
    let day = day.copied().unwrap_or_default();
    let column = |hour: usize| CHART_LEFT + CHART_PITCH * hour as i32;

    // Steps: the step counter's total (which, unlike the history, counts
    // steps from before the phone set the clock), then the hours, scaled
    // to the busiest, but to at least 500 steps, so a quiet day looks it.
    const STEPS_BASE: i32 = 30;
    const STEPS_HEIGHT: u32 = 10;
    canvas.sprite(4, 11, font::FOOT_W, &font::FOOT, 1, Color::TEXT_DIM);
    canvas.text(10, 12, "STEPS", Color::TEXT_DIM);
    let mut text = heapless::String::<10>::new();
    match steps {
        Some(steps) => write!(text, "{steps}").expect("a u32 is at most 10 digits"),
        None => text.push_str("--").expect("fits"),
    }
    canvas.text_right(75, 12, &text, Color::TEXT);
    let most = u32::from(day.most_steps()).max(500);
    for (i, hour) in day.hours.iter().enumerate() {
        let height = (u32::from(hour.steps) * STEPS_HEIGHT).div_ceil(most) as i32;
        canvas.fill_rect(
            column(i),
            STEPS_BASE - height,
            CHART_BAR,
            height,
            Color::ACCENT,
        );
        canvas.fill_rect(column(i), STEPS_BASE, CHART_BAR, 1, Color::TRACK);
    }

    // The heart rate: resting (see `Day::resting`), then each hour's
    // range, from its lowest reading to its highest, on a scale from the
    // day's lowest to its highest in whole tens, but at least 40 BPM
    // tall, so a calm day doesn't look like a wild one.
    // TODO: the scale's ends as figures, if the shape alone isn't enough.
    const HEART_BASE: i32 = 64;
    const HEART_HEIGHT: i32 = 20;
    canvas.sprite(4, 35, font::HEART_W, &font::HEART, 1, Color::HEART);
    canvas.sprite(
        4,
        35,
        font::HEART_W,
        &font::HEART_SHINE,
        1,
        Color::HEART_SHINE,
    );
    canvas.text(13, 36, "RESTING", Color::TEXT_DIM);
    let mut text = heapless::String::<3>::new();
    match day.resting() {
        Some(bpm) => write!(text, "{bpm}").expect("a u8 is at most 3 digits"),
        None => text.push_str("--").expect("fits"),
    }
    canvas.text_right(75, 36, &text, Color::TEXT);
    if let Some((low, high)) = day.heart_range() {
        let floor = i32::from(low) / 10 * 10;
        let ceiling = ((i32::from(high) + 9) / 10 * 10).max(floor + 40);
        // The top of the row `bpm` is drawn in.
        let y = |bpm: u8| {
            HEART_BASE - 1 - (i32::from(bpm) - floor) * (HEART_HEIGHT - 1) / (ceiling - floor)
        };
        for (i, hour) in day.hours.iter().enumerate() {
            if hour.heart_max > 0 {
                let top = y(hour.heart_max);
                let height = y(hour.heart_min) - top + 1;
                canvas.fill_rect(column(i), top, CHART_BAR, height, Color::HEART);
            }
        }
    }
    for i in 0..day.hours.len() {
        canvas.fill_rect(column(i), HEART_BASE, CHART_BAR, 1, Color::TRACK);
    }
    for hour in [0, 6, 12, 18] {
        let mut label = heapless::String::<2>::new();
        write!(label, "{hour}").expect("fits in 2");
        canvas.text(column(hour), HEART_BASE + 4, &label, Color::TEXT_FAINT);
    }
}

/// The last night's (or nap's) sleep: how long, big, from when to when,
/// and in which stages.
fn render_last_night(canvas: &mut Canvas, sleep: Option<crate::activity::Sleep>) {
    canvas.clear(BACKGROUND);
    canvas.text(4, 3, "LAST NIGHT", Color::ACCENT);
    let Some(sleep) = sleep else {
        canvas.text_centered(40, 36, "NO SLEEP YET", Color::TEXT_FAINT);
        return;
    };

    // Time asleep (not counting time lying awake) as H:MM, in the clock's
    // 3× digits, laid out as its are (2 px between digits, 7 px for the
    // colon), and centred.
    const TOP: i32 = 14;
    const SCALE: i32 = 3;
    const DIGIT: i32 = font::DIGIT_W as i32 * SCALE;
    let (hours, minutes) = (sleep.asleep_minutes / 60, sleep.asleep_minutes % 60);
    let mut digits = heapless::Vec::<u16, 4>::new();
    if hours >= 10 {
        let _ = digits.push(hours / 10 % 10);
    }
    let _ = digits.push(hours % 10);
    let _ = digits.push(minutes / 10);
    let _ = digits.push(minutes % 10);
    let width = DIGIT * digits.len() as i32 + 2 * (digits.len() as i32 - 2) + 7;
    let mut x = 40 - width / 2;
    let colon_after = digits.len() - 2;
    for (i, &digit) in digits.iter().enumerate() {
        if i == colon_after {
            for row in [2, 4] {
                let y = TOP + row * SCALE;
                canvas.fill_rect(x + 1, y + 1, 3, 3, Color::SHADOW);
                canvas.fill_rect(x, y, 3, 3, Color::ACCENT);
            }
            x += 3 + 2;
        }
        draw_digit(canvas, x, TOP, u64::from(digit), SCALE, Color::TEXT);
        x += DIGIT + 2;
    }
    canvas.text_centered(40, 40, "ASLEEP", Color::TEXT_DIM);

    // From falling asleep to waking, in local time (as the offset is now).
    if let Some(offset) = crate::clock::utc_offset_minutes() {
        let local = |utc_minute: i64| hhmm((utc_minute + i64::from(offset)).rem_euclid(24 * 60));
        let mut window = heapless::String::<16>::new();
        write!(
            window,
            "{} - {}",
            local(sleep.start_utc_minute),
            local(sleep.end_utc_minute)
        )
        .expect("fits in 16");
        canvas.text_centered(40, 50, &window, Color::TEXT);
    }

    // The stages (see `crate::stages`): their shares of the time asleep as
    // one bar, and each one's time below its name.
    let light = sleep
        .asleep_minutes
        .saturating_sub(sleep.deep_minutes + sleep.rem_minutes);
    let stages = [
        ("DEEP", sleep.deep_minutes, Color::ACCENT),
        ("REM", sleep.rem_minutes, Color::HEART),
        ("LIGHT", light, Color::TEXT_DIM),
    ];
    const BAR: i32 = 72;
    let total = i32::from(sleep.asleep_minutes.max(1));
    let mut x = 4;
    for (i, &(name, minutes, color)) in stages.iter().enumerate() {
        // The last takes what's left, so rounding can't leave a gap.
        let width = if i + 1 == stages.len() {
            4 + BAR - x
        } else {
            i32::from(minutes) * BAR / total
        };
        canvas.fill_rect(x, 60, width, 4, color);
        x += width;
        let center = 16 + 24 * i as i32;
        canvas.text_centered(center, 67, name, color);
        canvas.text_centered(center, 74, &hhmm(i64::from(minutes)), Color::TEXT);
    }
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

/// The phone's alarms, a row each: the time, the weekdays it rings on (or
/// "ONCE"), and whether it's on. Ten rows fit, as many as Gadgetbridge
/// sends.
fn render_phone_alarms(canvas: &mut Canvas, phone: &alarm::Phone) {
    canvas.clear(BACKGROUND);
    canvas.text(4, 3, "PHONE ALARMS", Color::ACCENT);
    for (row, alarm) in phone.alarms().iter().enumerate() {
        let y = 11 + 7 * row as i32;
        let on = alarm.on();
        let text = if on { Color::TEXT } else { Color::TEXT_FAINT };
        canvas.text(4, y, &hhmm(alarm.time.into()), text);
        if alarm.days == 0 {
            canvas.text(26, y, "ONCE", text);
        } else {
            // Its weekdays bright, the others faint, so the row reads as
            // a little calendar.
            for (day, letter) in ["M", "T", "W", "T", "F", "S", "S"].into_iter().enumerate() {
                let color = if on && alarm.days >> day & 1 != 0 {
                    Color::TEXT
                } else {
                    Color::TEXT_FAINT
                };
                canvas.text(26 + 4 * day as i32, y, letter, color);
            }
        }
        let (state, color) = if on {
            ("ON", Color::GOOD)
        } else {
            ("OFF", Color::TEXT_FAINT)
        };
        canvas.text_right(77, y, state, color);
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
        alarm::Kind::At(_) | alarm::Kind::Phone(_) | alarm::Kind::Snooze => "ALARM",
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
/// The rows a tap changes, theme then brightness, at the bottom below a
/// hairline, and further apart than the others: a fingertip covers a few
/// rows, and a row alone is only ~3 mm tall. Each one's tap area runs to
/// halfway to the other, and to the hairline or the edge.
const INFO_THEME: i32 = 57;
const INFO_LIGHT: i32 = 69;
const INFO_THEME_TAP: core::ops::Range<i32> = 50..65;
const INFO_LIGHT_TAP: core::ops::Range<i32> = 65..80;

fn render_info(canvas: &mut Canvas, status: &Status, theme: &Theme, light: brightness::Setting) {
    canvas.clear(BACKGROUND);
    // The version without its build metadata (`0.9.0-pr.12`); the commit
    // goes in the BUILD row (see `build.rs`).
    canvas.text(4, 3, concat!("UR ", env!("UR_RELEASE")), Color::ACCENT);

    let battery = status.battery;
    let mv = battery.millivolts();
    let secs = status.uptime_secs;
    let mut values: [heapless::String<12>; 3] = Default::default();
    write!(
        values[0],
        "{}% {}.{:02}V",
        battery.percent(),
        mv / 1000,
        (mv % 1000) / 10
    )
    .expect("fits in 12");
    write!(values[1], "{}", battery.status()).expect("fits in 12");
    write!(
        values[2],
        "{:02}:{:02}:{:02}",
        secs / 3600,
        (secs / 60) % 60,
        secs % 60
    )
    .expect("fits in 12 until uptime exceeds ~11 years");
    // "PAIRED" rather than "CONNECTED": the phone only counts as here once
    // it has written over an authenticated link (see `ble`), which takes
    // its bond, so this says both that it's connected and that it's paired
    // (and bonded: ur always stores the keys it pairs with).
    let phone = if status.connected {
        "PAIRED"
    } else if let Some(why) = status.unpaired {
        why.title()
    } else {
        "WAITING"
    };
    // Which commit this build is from, for bug reports: "DEV" for a local
    // `cargo build`, which doesn't know.
    let build = match env!("UR_BUILD") {
        "" => "DEV",
        build => build,
    };
    let rows = [
        ("BATTERY", values[0].as_str()),
        ("POWER", &values[1]),
        ("UPTIME", &values[2]),
        ("PHONE", phone),
        ("BUILD", build),
    ];
    for (i, (label, value)) in rows.into_iter().enumerate() {
        let y = INFO_TOP + INFO_LINE * i as i32;
        canvas.text(4, y, label, Color::TEXT_DIM);
        canvas.text_right(77, y, value, Color::TEXT);
    }

    // The settings a tap changes. (Last night's sleep, which used to be
    // here, has a screen of its own.)
    // TODO: tapping elsewhere still buzzes (a motor test); drop that once
    // alarms exercise the motor.
    canvas.fill_rect(4, INFO_THEME_TAP.start + 1, 72, 1, Color::TEXT_FAINT);
    for (y, label, value) in [
        (INFO_THEME, "THEME", theme.name),
        (INFO_LIGHT, "LIGHT", light.name()),
    ] {
        canvas.text(4, y, label, Color::TEXT_DIM);
        canvas.text_right(77, y, value, Color::TEXT);
    }
}

// ==============================================================================
// Switching off
// ==============================================================================

/// Lines of a message about switching off, centred, from `y` down.
fn render_lines(canvas: &mut Canvas, y: i32, lines: &[&str], color: Color) {
    for (i, line) in lines.iter().enumerate() {
        canvas.text_centered(40, y + INFO_LINE * i as i32, line, color);
    }
}

/// Asks whether to switch to storage mode (see [`crate::system_off`]).
fn render_storage_prompt(canvas: &mut Canvas) {
    canvas.clear(BACKGROUND);
    canvas.text_centered(40, 6, "STORAGE MODE", Color::ACCENT);
    let explanation = ["OFF UNTIL THE", "BUTTON OR THE", "CHARGER WAKES IT"];
    render_lines(canvas, 20, &explanation, Color::TEXT);
    render_lines(
        canvas,
        54,
        &["TAP: OFF NOW", "ELSE: CANCEL"],
        Color::TEXT_DIM,
    );
}

/// What shows for the few seconds before the watch switches itself off.
pub fn render_off(canvas: &mut Canvas, reason: crate::system_off::Reason) {
    use crate::system_off::Reason;
    canvas.clear(BACKGROUND);
    let (title, color, wake): (_, _, &[&str]) = match reason {
        Reason::BatteryEmpty => ("BATTERY EMPTY", Color::BAD, &["CHARGE TO", "WAKE IT"]),
        Reason::Storage => (
            "STORAGE MODE",
            Color::ACCENT,
            &["BUTTON OR", "CHARGER", "WAKES IT"],
        ),
    };
    canvas.text_centered(40, 14, title, color);
    render_lines(canvas, 30, &["SWITCHING OFF"], Color::TEXT_DIM);
    render_lines(canvas, 46, wake, Color::TEXT);
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

/// How long a notification shows: long enough to read, and for long lines
/// to scroll through once. With the `whole_screen` to itself (it arrived
/// with the screen off), the sender is wrapped over it (see
/// [`quiet_layout`]), and only scrolls if even that won't do.
fn shown_for(app: &str, who: &str, whole_screen: bool) -> Duration {
    let who_overflow = if !whole_screen {
        font::small_text_width(who)
    } else if quiet_layout(who, QUIET_LINES_2X, &mut Lines::new()).is_some() {
        0
    } else {
        font::small_text_width(who) * 2
    };
    let overflow = font::small_text_width(app).max(who_overflow) - BANNER_TEXT_WIDTH;
    BANNER_TIME.max(marquee_cycle(overflow))
}

/// Across the bottom rows (free on the clock face): app, then sender.
/// `elapsed` is how long it's been showing.
fn render_banner(canvas: &mut Canvas, app: &str, who: &str, elapsed: Duration) {
    canvas.fill_rect(0, 63, 80, 17, BACKGROUND);
    canvas.fill_rect(0, 63, 80, 1, Color::ACCENT);
    canvas.marquee(3..3 + BANNER_TEXT_WIDTH, 67, app, Color::ACCENT, 1, elapsed);
    canvas.marquee(3..3 + BANNER_TEXT_WIDTH, 74, who, Color::TEXT, 1, elapsed);
}

/// Where a quiet notification's sender goes: everything below the app
/// name, which sits at the very top.
const QUIET_TOP: i32 = 12;
/// Line pitch of 1× text: 5 px glyphs, 2 accent rows, 2 rows' gap.
const QUIET_PITCH: i32 = 9;
/// The most lines a quiet notification's sender may wrap onto at 2× (as
/// many as fit below the app name: 4 × 18 px, less the last gap, is
/// exactly the 68 px there), and at 1× (seven 9 px lines), which it falls
/// back to when 2× won't do. "NIGHT / SIGNALS: / IDA / MOREAU" is four
/// lines at 2×: a podcast's name and its episode's, say.
const QUIET_LINES_2X: usize = 4;
const QUIET_LINES: usize = 7;

/// One wrapped line: the longest sender line at 1× is 18 characters, plus
/// a hyphen; room for multi-byte letters.
type Line = heapless::String<48>;
/// Wrapped lines. At 0.4 KB, these are filled in where the caller keeps
/// them rather than returned: returned (in an `Option`, and again through
/// `quiet_layout`), each function on the way built its own copy, and
/// kept a frame for it.
type Lines = heapless::Vec<Line, QUIET_LINES>;

/// How a quiet notification's sender fits on the screen: wrapped into
/// `lines`, and the scale to draw them at. Big (2×) if it fits in
/// `lines_2x` lines (at most [`QUIET_LINES_2X`]), else normal size (1×);
/// `None` if it's too long even for that, and has to scroll.
fn quiet_layout(who: &str, lines_2x: usize, lines: &mut Lines) -> Option<i32> {
    if wrap(who, BANNER_TEXT_WIDTH, 2, lines_2x, lines).is_some() {
        Some(2)
    } else {
        wrap(who, BANNER_TEXT_WIDTH, 1, QUIET_LINES, lines).map(|()| 1)
    }
}

/// `text` broken at spaces into `lines` of at most `width` pixels at
/// `scale`; `None` if it doesn't fit in `max_lines` (at most
/// [`QUIET_LINES`]). A word too long for a line of its own is hyphenated,
/// keeping at least three letters for the next line ("KRISTIAN-" / "SEN").
fn wrap(text: &str, width: i32, scale: i32, max_lines: usize, lines: &mut Lines) -> Option<()> {
    let fits = |s: &str| font::small_text_width(s) * scale <= width;
    lines.clear();
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
    Some(())
}

/// A notification that arrived with the screen off (or a page of the
/// history): the whole screen, white on black like a glance (or in the
/// theme), the app small at the very top and the sender wrapped and
/// centred below it, as big as fits in `lines_2x` lines (see
/// [`quiet_layout`]; scrolling only if it's longer still). `elapsed` is
/// how long it's been showing.
fn render_quiet_notification(
    canvas: &mut Canvas,
    app: &str,
    who: &str,
    elapsed: Duration,
    lines_2x: usize,
) {
    canvas.clear(BACKGROUND);
    canvas.marquee(
        3..3 + BANNER_TEXT_WIDTH,
        3,
        app,
        Color::TEXT_DIM,
        1,
        elapsed,
    );
    let mut lines = Lines::new();
    match quiet_layout(who, lines_2x, &mut lines) {
        Some(scale) => {
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
        None => canvas.marquee(3..3 + BANNER_TEXT_WIDTH, 44, who, Color::TEXT, 2, elapsed),
    }
}

/// Where a history page shows when its notification arrived: small, and
/// centred near the bottom, where the sender never reaches (three lines at
/// 2× end at y = 62; at 1×, the longest sender, 40 bytes, takes at most
/// four lines, ending at y = 55), above the handle at y = 78.
const ARRIVED_Y: i32 = 70;

/// When a history page's notification arrived, at local minute `at`, as
/// of local minute `now` (see [`arrival::write`]).
fn render_arrived(canvas: &mut Canvas, at: i64, now: i64) {
    let mut text = heapless::String::<{ arrival::MAX_LEN }>::new();
    arrival::write(&mut text, at, now).expect("fits in MAX_LEN");
    canvas.text_centered(40, ARRIVED_Y, &text, Color::TEXT_DIM);
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
        match music.playing {
            Some(true) => "PLAYING",
            Some(false) => "PAUSED",
            None => "",
        },
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
    let play_pause = if music.playing() {
        &font::PAUSE
    } else {
        &font::PLAY
    };
    for (x, icon) in [(6, &font::PREVIOUS), (33, play_pause), (60, &font::NEXT)] {
        canvas.sprite(x, MUSIC_CONTROLS_Y + 6, 5, icon, 3, Color::TEXT);
    }
}
