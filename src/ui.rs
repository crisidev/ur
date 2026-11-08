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
//!   some; then Settings, and Info.
//! - Above: the last [`HISTORY`] notifications, newest nearest, one a page,
//!   with when each arrived (see [`crate::arrival`]).
//! - Below: the button grid (see [`buttons`]), if it's on; Music, while
//!   the phone has media; Air, while a nearby Aranet4 has been heard
//!   within the hour (see [`crate::aranet`]). Possibly none of them.
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
//!   shows while media plays, at the status bar's left end. The phone
//!   needn't have said it has media (see `App::media`). Without a phone,
//!   neither does anything. A long press acts (and buzzes) once the
//!   finger has been down long enough, before it lifts (see
//!   [`crate::touch`]); that goes for every long press. Both work with
//!   the screen off, too (see below).
//! - Long press elsewhere: on the alarms screen, switch an alarm on or off;
//!   on the info screen, offer storage mode (see [`crate::system_off`]),
//!   which a tap then confirms, and anything else cancels. Elsewhere,
//!   nothing.
//! - On the settings screen, tap a row for its next value: the colour
//!   theme ("THEME", the next within the current group: dark themes after
//!   sunset, light ones after sunrise; each group remembers its own), the
//!   brightness ("LIGHT", auto or soft; see [`crate::brightness`]), the
//!   button grid ("BUTTONS": off, or where its presses go), and, on the
//!   last row ("HA KEY"), the key Home Assistant needs for them (see
//!   [`crate::bthome`]): a QR code to scan, then, at a tap, as text with
//!   the watch's address. Another tap, or anything else, closes it.
//! - On the button grid, taps and holds are the grid's (see [`buttons`]
//!   for how they become presses). While a press waits to see whether
//!   more follows, its buttons stay lit; once sent, the last one shows the
//!   code, in the "good" colour (or "bad", if the phone was all it could
//!   go to, and it wasn't connected), with a buzz. A swipe, or the
//!   button, cancels what's waiting.
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
//! - Notifications: a banner over the bottom of the screen (the app, then
//!   who it's from) for [`BANNER_TIME`] and a short buzz. If the screen was
//!   off, the notification gets the whole (dim) screen instead, like a
//!   glance, as a history page shows it (see [`render_card`]), and the
//!   screen sleeps again after. Either way, it's kept in
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
//! - Everything shows in the dim look (white, and red for what comes
//!   second, on black; see [`theme::GLANCE`]), at the glance's low
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
use crate::aranet::{self, Band, Sighting};
use crate::arrival;
use crate::battery::Battery;
use crate::ble;
use crate::brightness;
use crate::bthome::Identity;
use crate::buttons::{self, Press};
use crate::canvas::{Canvas, Color, Sweep, marquee_cycle};
use crate::clock::DateTime;
use crate::day::Day;
use crate::font;
use crate::gb::{CallAction, MusicCommand, Outgoing};
use crate::motor::{self, Pattern};
use crate::nav::{self, Place, Side};
// Defined apart from the UI, so that `gb` (which decodes them) builds
// without it, in host tests; see `phone`.
use crate::phone::Text;
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
    /// The temperature in °C (see [`crate::temperature`]), once it's been
    /// read.
    pub temperature: Option<i8>,
    /// Whether that's a nearby Aranet4's, rather than the watch's own (or
    /// an estimate from it).
    pub temperature_from_aranet: bool,
    /// The nearest Aranet4's latest reading, for the Air screen.
    pub air: Option<Sighting>,
    /// The most recent sleep within the last day.
    pub sleep: Option<crate::activity::Sleep>,
    /// Local seconds since 1970 (see [`crate::alarm`]), once the time is
    /// known.
    pub local_second: Option<i64>,
    pub battery: &'a Battery,
    /// Home Assistant's key for the button grid, and the address it comes
    /// from, once loaded (see [`crate::bthome`]).
    pub bthome: Option<Identity>,
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
    /// The button grid (see [`buttons`]).
    Buttons,
    Settings,
    /// A nearby Aranet4's readings.
    Air,
    Info,
    /// Asking whether to switch to storage mode: in no line, only reached
    /// by a long press on the info screen.
    Storage,
    /// Home Assistant's key, for the button grid, as a QR code to scan:
    /// in no line, only reached by a tap on the settings screen's last
    /// row.
    HaQr,
    /// The same, as text, with the watch's address: a tap on the QR code.
    HaKey,
}

/// The most screens in a line: the notifications', or the one below (the
/// grid, music, settings, and info).
const LONGEST_LINE: usize = if HISTORY > 4 { HISTORY } else { 4 };
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
    /// [`App::banner_text`]).
    shows: Shows,
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
    /// That Gadgetbridge's "Text as Bitmaps" is on (see
    /// [`PhoneEvent::TextAsBitmaps`]), and where the README says how to
    /// turn it off.
    TextAsBitmaps,
    /// That Gadgetbridge didn't pass a grid press on (see
    /// [`PhoneEvent::IntentsOff`]).
    IntentsOff,
    /// That an update was refused (see [`PhoneEvent::UpdateRefused`]).
    UpdateRefused,
}

/// How many notifications the history above the clock keeps. Each costs
/// 68 bytes of RAM (its two texts, and when it arrived), where there's
/// little to spare.
const HISTORY: usize = 3;

struct Notification {
    app: Text<24>,
    /// Its title: who it's from, in a chat (see `gb`).
    who: Text<40>,
    /// When it arrived, in local minutes since 1970 (see [`arrival`]), if
    /// the time was known then. The whole minute rather than the minute
    /// of the day, so a page can say which day: with only a few kept,
    /// they can be days old. It costs nothing over a `u16` minute of the
    /// day, which the texts' 4-byte alignment would pad to 4 bytes
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
    fn push(&mut self, app: Text<24>, who: Text<40>, arrived: Option<NonZeroU32>) {
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
    /// Whether this connection's phone has been told to turn off "Text as
    /// Bitmaps" (see [`PhoneEvent::TextAsBitmaps`]): once is enough, when
    /// every message may carry them.
    told_bitmaps: bool,
    /// The button grid's setting, as stored (`main` saves it when it
    /// changes).
    pub buttons: buttons::Setting,
    /// What the grid's waiting on (see [`buttons::Grid`]).
    grid: buttons::Grid,
    /// The grid's last press, for the screen: until when it shows, and
    /// whether it went anywhere.
    sent: Option<(Press, Instant, bool)>,
    /// Presses for Home Assistant, for `main` to send (see
    /// [`App::take_ha_press`]): sealing them needs flash, which is async.
    /// Two, as one touch can complete two (see [`buttons::Grid::touch`]).
    ha_presses: heapless::Vec<Press, 2>,
    /// Whether there's an Aranet4 reading to show (see [`App::set_air`]).
    air: bool,
}

/// How long the grid shows a press it sent.
const SENT_TIME: Duration = Duration::from_millis(800);

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
            air: false,
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
            told_bitmaps: false,
            buttons: buttons::Setting::Off,
            grid: buttons::Grid::default(),
            sent: None,
            ha_presses: heapless::Vec::new(),
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

    /// Whether a nearby Aranet4 has been heard recently enough for its
    /// screen to be there ([`aranet::RECENT`]). It goes away when not,
    /// leaving the clock if it was showing.
    pub fn set_air(&mut self, air: bool) {
        self.air = air;
        if !air && self.screen == Screen::Air {
            self.screen = Screen::Clock;
        }
    }

    /// Whether the Air screen is on screen, for a scan to refresh it.
    pub fn showing_air(&self) -> bool {
        self.awake && self.screen == Screen::Air && self.call.is_none() && self.ringing.is_none()
    }

    /// Media info is only meaningful while connected: forget it on
    /// disconnect (which also removes the music screen).
    pub fn set_connected(&mut self, connected: bool) {
        self.connected = connected;
        if !connected {
            self.music = None;
            self.told_bitmaps = false;
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
                // Settings and Info sit here, a few swipes away, rather
                // than at the end of the line below, behind the grid,
                // music, and the air, which come and go (so the way to
                // them would change); after the phone's alarms, so the two
                // kinds of alarm stay together. Settings first, as it's
                // the one that's used.
                add(Screen::Settings);
                add(Screen::Info);
            }
            Side::Above => {
                for i in 0..self.notifications.list.len() {
                    add(Screen::Notification(i as u8));
                }
            }
            Side::Below => {
                if self.buttons.on() {
                    add(Screen::Buttons);
                }
                if self.music.is_some() {
                    add(Screen::Music);
                }
                // Possibly none of them, like the notifications above:
                // then there's nothing below, and a swipe up does
                // nothing (see `nav::swipe`).
                if self.air {
                    add(Screen::Air);
                }
            }
        }
        line
    }

    /// Where the current screen is. The storage prompt counts as the info
    /// screen it came from, and the key as the settings.
    fn place(&self) -> Place {
        let screen = match self.screen {
            Screen::Storage => Screen::Info,
            Screen::HaQr | Screen::HaKey => Screen::Settings,
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
            // Leaving the grid (by a swipe or the button) cancels what it
            // was waiting on.
            if self.grid.cancel() {
                defmt::info!("buttons: cancelled");
            }
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

    /// What a banner says: the app (or a title) in `app`, then who it's
    /// from (or what to do) in `who`.
    fn banner_text(&self, shows: Shows, app: &mut AppText, who: &mut WhoText) {
        let (title, hint) = match shows {
            Shows::Newest => {
                match self.notifications.list.first() {
                    Some(n) => {
                        n.app.write_to(app);
                        n.who.write_to(who);
                    }
                    None => {
                        app.clear();
                        who.clear();
                    }
                }
                return;
            }
            Shows::Unpaired(why) => (why.title(), why.hint()),
            Shows::TextAsBitmaps => ("TEXT AS BITMAPS", "SEE FAQ N02"),
            Shows::IntentsOff => ("INTENTS OFF", "SEE FAQ C04"),
            Shows::UpdateRefused => ("UPDATE REFUSED", "SEE FAQ U02"),
        };
        app.clear();
        who.clear();
        app.push_str(title).expect("titles are short");
        who.push_str(hint).expect("hints are short");
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
            // A long press's finger lifting: only a button grid hold's
            // end, which goes wherever its start did (see
            // `buttons::Grid::lift`). Nothing else acts on it, nor counts
            // it as a touch: the long press was the touch.
            InputEvent::Touch(touch) if touch.gesture == Gesture::Release => {
                if let Some(press) = self.grid.lift() {
                    self.send_press(press, now);
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
                (Gesture::Tap, Screen::HaQr) => self.go_to(Screen::HaKey, Sweep::RightToLeft),
                (_, Screen::HaQr | Screen::HaKey) => {
                    self.go_to(Screen::Settings, Sweep::LeftToRight)
                }
                (Gesture::Tap | Gesture::LongPress, Screen::Buttons) => self.on_grid(touch, now),
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
                (Gesture::Tap, Screen::Settings) => match settings_row(touch.y) {
                    Some(Row::Theme) => self.next_theme(),
                    Some(Row::Light) => self.brightness = self.brightness.next(),
                    Some(Row::Buttons) => self.buttons = self.buttons.next(),
                    Some(Row::HaKey) => self.go_to(Screen::HaQr, Sweep::RightToLeft),
                    None => {}
                },
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

    /// A tap or hold on the button grid.
    fn on_grid(&mut self, touch: crate::touch::Touch, now: Instant) {
        let kind = match touch.gesture {
            Gesture::LongPress => buttons::Touch::Hold,
            _ => buttons::Touch::Tap,
        };
        let started = now
            .checked_sub(Duration::from_millis(touch.held_ms.into()))
            .unwrap_or(now);
        let button = buttons::at(touch.x, touch.y);
        for press in self
            .grid
            .touch(button, kind, started, now)
            .into_iter()
            .flatten()
        {
            self.send_press(press, now);
        }
    }

    /// When [`App::on_timer`] next has something to do: the grid's wait
    /// running out, or its last press to stop showing.
    pub fn deadline(&self) -> Option<Instant> {
        let shown = self.sent.map(|(_, until, _)| until);
        self.grid.deadline().into_iter().chain(shown).min()
    }

    /// The UI loop woke for [`App::deadline`] (or near it): sends what the
    /// grid was waiting on, if it's waited long enough, unless a finger is
    /// down (`finger_down`; see [`buttons::Grid::expire`]).
    pub fn on_timer(&mut self, now: Instant, finger_down: bool) {
        if self.sent.is_some_and(|(_, until, _)| now >= until) {
            self.sent = None;
        }
        // Gone from the grid (the screen off, say) without going through
        // `go_to`: nothing waits.
        if !(self.awake && self.screen == Screen::Buttons) {
            self.grid.cancel();
            return;
        }
        if let Some(press) = self.grid.expire(now, finger_down) {
            self.send_press(press, now);
        }
    }

    /// Sends a grid press wherever the setting says, and shows it went.
    /// To the phone, it's sent from here, as media commands are; for Home
    /// Assistant, `main` picks it up (see [`App::take_ha_press`]).
    fn send_press(&mut self, press: Press, now: Instant) {
        defmt::info!("buttons: {=u8} {=str}", press.code, press.gesture.name());
        let to_phone = self.buttons.phone() && self.connected;
        if to_phone {
            send(Outgoing::Button {
                code: press.code,
                gesture: press.gesture.name(),
            });
        }
        if self.buttons.ha() {
            let _ = self.ha_presses.push(press);
        }
        // A hold's end: the screen keeps showing the hold, which already
        // buzzed.
        if press.gesture == buttons::Gesture::Release {
            return;
        }
        // Somewhere it could go. (Home Assistant can't say whether it
        // heard; Gadgetbridge only says if it didn't pass it on.)
        let sent = to_phone || self.buttons.ha();
        self.sent = Some((press, now + SENT_TIME, sent));
        motor::buzz(if sent { Pattern::NOTIFY } else { Pattern::TURN });
    }

    /// A grid press for Home Assistant, if any are waiting to be sent.
    pub fn take_ha_press(&mut self) -> Option<Press> {
        if self.ha_presses.is_empty() {
            None
        } else {
            Some(self.ha_presses.remove(0))
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
        // These show as a banner: a notification (kept in the history,
        // too), that a client can't connect, or that the phone's text
        // comes as images.
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
            // After the message that carried the images (see `ble`), so
            // this replaces its banner rather than the other way round:
            // what it said is in the history, as well as we could show it.
            // Not over a call, whose screen and ring it would interrupt;
            // the next message after it will do.
            PhoneEvent::TextAsBitmaps => {
                if !self.told_bitmaps && self.call.is_none() {
                    defmt::info!("ui: the phone sends text as bitmaps (FAQ N02)");
                    self.told_bitmaps = true;
                    self.show_banner(Shows::TextAsBitmaps, now);
                }
                return;
            }
            PhoneEvent::IntentsOff => return self.show_banner(Shows::IntentsOff, now),
            PhoneEvent::UpdateRefused => return self.show_banner(Shows::UpdateRefused, now),
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
            | PhoneEvent::TextAsBitmaps
            | PhoneEvent::IntentsOff
            | PhoneEvent::UpdateRefused
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
                until: UNSEEN,
            });
            return;
        }
        if self.glance_until.take().is_some() {
            self.last_input = now;
        }
        // With the screen off, it gets the whole screen.
        self.banner = Some(Banner {
            shows,
            until: now + BANNER_TIME,
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
            self.banner = Some(Banner {
                shows,
                until: now + BANNER_TIME,
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
        // Notifications never scroll: what doesn't fit ends in an
        // ellipsis (see `render_card` and `render_banner`).
        match self.screen {
            Screen::Music => self.music.as_ref().is_some_and(|m| {
                overflows(&m.artist, 1, MUSIC_TEXT_WIDTH)
                    || overflows(&m.track, 2, MUSIC_TEXT_WIDTH)
            }),
            _ => false,
        }
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
        // Text kept as Latin-1 (see `phone::Text`), as UTF-8 to draw.
        let (mut app, mut who) = (AppText::new(), WhoText::new());
        if self.quiet_notification()
            && let Some(banner) = &self.banner
        {
            // The time now in its corner: a wrist raised for a buzz
            // usually wants that too.
            self.banner_text(banner.shows, &mut app, &mut who);
            let mut time = String::<{ arrival::MAX_LEN }>::new();
            if let Some(second) = status.local_second {
                // Arriving now: "HH:MM", as the history's are written.
                let minute = second.div_euclid(60);
                arrival::write(&mut time, minute, minute).expect("fits in MAX_LEN");
            }
            render_card(canvas, &app, &who, &time);
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
                // The slot is there only while an icon is: a fixed width
                // (see `render_clock`) so the temperature doesn't shift as
                // one icon replaces another (a gesture's, then the note),
                // but no empty gap once the media is paused or stopped
                // and the gesture's icon has gone. The temperature moves
                // over once then, which reads as the icon going away.
                let media = icon.is_some().then_some(icon);
                render_clock(canvas, status, next_alarm, media)
            }
            // From the day's history, only while it shows: walking it
            // costs a little time, while keeping it up to date would cost
            // RAM.
            Screen::Today => render_today(canvas, crate::activity::today().as_ref(), status.steps),
            Screen::LastNight => render_last_night(canvas, status.sleep),
            Screen::Alarms => render_alarms(canvas, &self.alarms, &self.alarm_state, status),
            Screen::PhoneAlarms => render_phone_alarms(canvas, &self.phone_alarms),
            // As a notification that arrived with the screen off shows,
            // with when it arrived in its corner, and the handles (see
            // `edge_bar`).
            Screen::Notification(i) => match self.notifications.list.get(usize::from(i)) {
                Some(n) => {
                    let mut arrived = String::<{ arrival::MAX_LEN }>::new();
                    if let (Some(at), Some(second)) = (n.arrived, status.local_second) {
                        arrival::write(&mut arrived, at.get().into(), second.div_euclid(60))
                            .expect("fits in MAX_LEN");
                    }
                    n.app.write_to(&mut app);
                    n.who.write_to(&mut who);
                    render_card(canvas, &app, &who, &arrived);
                }
                None => render_clock(canvas, status, next_alarm, None),
            },
            Screen::Music => match &self.music {
                Some(music) => render_music(canvas, music, now),
                None => render_clock(canvas, status, next_alarm, None),
            },
            Screen::Air => match &status.air {
                Some(sighting) => render_air(canvas, sighting, now),
                None => render_clock(canvas, status, next_alarm, None),
            },
            Screen::Buttons => {
                let sent = self.sent.filter(|&(_, until, _)| now < until);
                // The dim look is white on black: dark.
                let dark = self.dim() || !self.daytime;
                let sent = sent.map(|(press, _, ok)| (press, ok));
                render_buttons(canvas, &self.grid, sent, dark);
            }
            Screen::Settings => {
                render_settings(canvas, self.chosen_theme(), self.brightness, self.buttons)
            }
            Screen::Info => render_info(canvas, status),
            Screen::Storage => render_storage_prompt(canvas),
            Screen::HaQr => render_ha_qr(canvas, status.bthome.as_ref()),
            Screen::HaKey => render_ha_key(canvas, status.bthome.as_ref()),
        }
        match self.place() {
            // Notifications nobody has seen: an accent-coloured handle on
            // the edge they're behind. The clock face has no other hints:
            // its edges are full, and it's where the wearer learns the
            // layout from.
            Place::Clock if self.notifications.unseen => {
                edge_bar(canvas, Side::Above, Color::ACCENT);
            }
            Place::Line { side, index }
                if !matches!(self.screen, Screen::Storage | Screen::HaQr | Screen::HaKey) =>
            {
                render_hint(canvas, side, index, self.line(side).len());
            }
            _ => {}
        }
        if let Some(banner) = &self.banner {
            self.banner_text(banner.shows, &mut app, &mut who);
            render_banner(canvas, &app, &who);
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
//    3..7    status bar: media, or no phone and time-sync state, then the
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

/// `media` is an icon for the status bar's left end, while a phone is
/// connected: its width, rows, and top.
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

    // Status bar, left: the media icon while a phone is connected (media
    // comes from the phone); without one, a crossed-out Bluetooth rune,
    // and "NO TIME" if we don't know the time (in which case the clock
    // shows time since boot). A connected phone is the usual state, so it
    // gets no icon: what's worth a look is its absence.
    //
    // "Connected" is the phone's link (`ble::CONNECTED`): a client that
    // never authenticated (see `Status::unpaired`), or a bonded phone
    // that's out of range, both get the cross, as neither delivers
    // anything.
    let temperature_x = if status.connected {
        // One width for every media icon (the widest, the note's), each
        // centred in it, so nothing after it moves as the icon changes.
        const MEDIA_SLOT: u32 = font::NOTE_W;
        match media {
            Some(icon) => {
                if let Some((width, rows, y)) = icon {
                    let left = 3 + (MEDIA_SLOT.saturating_sub(width) / 2) as i32;
                    canvas.sprite(left, y, width, rows, 1, Color::ACCENT);
                }
                Some(3 + MEDIA_SLOT as i32 + 3)
            }
            None => Some(3),
        }
    } else {
        canvas.sprite(3, 2, font::BLUETOOTH_W, &font::BLUETOOTH, 1, Color::ACCENT);
        let cross_x = 3 + font::BLUETOOTH_W as i32 + 1;
        canvas.sprite(cross_x, 6, font::CROSS_W, &font::CROSS, 1, Color::ACCENT);
        let x = cross_x + font::CROSS_W as i32;
        if status.local_time.is_none() {
            // 2 px rather than the usual 3, so "NO TIME" (27 px) ends at
            // x = 40, clear of the battery's widest (charging at 100%: the
            // bolt starts at x = 43).
            canvas.text(x + 2, 3, "NO TIME", Color::TEXT_FAINT);
            None
        } else {
            Some(x + 3)
        }
    };

    // Then the temperature, in whole °C, packed against the left. The
    // widest ("-10°", 15 px; nothing in the sensor's -40 to 85 °C is
    // wider) ends at x = 29 after the crossed-out rune, and x = 26 after
    // the widest media icon (the note, 6 px), clear of the battery's
    // widest (charging at 100%: the bolt starts at x = 43). "NO TIME" and
    // a temperature can't both fit beside that, so until the phone sets
    // the clock (just after a cold boot), "NO TIME" wins.
    //
    // A reading from a room sensor (a nearby Aranet4; see
    // `crate::temperature`), rather than the watch's own or an estimate
    // from it, is underlined at the degree sign: a short line a pixel
    // below it, like a ground line under the reading, in the labels'
    // colour, so it's a mark on the reading rather than part of it. (A
    // little house before the reading said the same, but read as one more
    // status icon. Also tried: a dot under the degree sign, which read as
    // "?"; a line on the baseline under it, as "±"; a dot after the
    // reading, as a full stop; and underlining the whole reading, which
    // is louder than a mark needs to be.)
    // TODO: °F, for those who'd want it.
    if let (Some(degrees), Some(x)) = (status.temperature, temperature_x) {
        let mut text = heapless::String::<6>::new();
        write!(text, "{degrees}°").expect("an i8 is at most 4 chars, and ° is 2 bytes");
        let end = canvas.text(x, 3, &text, Color::TEXT);
        if status.temperature_from_aranet {
            // `end` is past the degree sign's spacing column: the sign
            // itself is the 3 columns before that.
            canvas.fill_rect(end - 4, 9, 3, 1, Color::TEXT_DIM);
        }
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

    let mut lines = [font::Line::default(); 2];
    let street = step.street.trim();
    match font::fill(street, font::Face::Small, 76, &mut lines) {
        count @ 0..=2 => {
            for (i, &line) in lines[..count].iter().enumerate() {
                let width = font::small_text_width(&street[line.start..line.end])
                    + font::small_text_width(line.tail.as_str())
                    + if line.tail == font::Tail::None { 0 } else { 1 };
                let y = STREET_Y + i as i32 * 7;
                canvas.line(
                    40 - width / 2,
                    y,
                    street,
                    line,
                    font::Face::Small,
                    Color::TEXT_DIM,
                );
            }
        }
        // Too long even for two lines (one very long word): its start.
        _ => {
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
    // one bar, and each one's time below its name, in that stage's colour
    // (roles of their own: the general ones collide in some themes).
    let light = sleep
        .asleep_minutes
        .saturating_sub(sleep.deep_minutes + sleep.rem_minutes);
    let stages = [
        ("DEEP", sleep.deep_minutes, Color::DEEP),
        ("REM", sleep.rem_minutes, Color::REM),
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

/// The watch's state, for a look (and for bug reports). The settings that
/// used to be at its foot have a screen of their own, before it.
fn render_info(canvas: &mut Canvas, status: &Status) {
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

    // Storage mode is a long press here, which nothing else would show.
    // TODO: tapping still buzzes (a motor test); drop that once alarms
    // exercise the motor.
    canvas.text_centered(40, 68, "HOLD: STORAGE MODE", Color::TEXT_FAINT);
}

// ==============================================================================
// Settings
// ==============================================================================
//
// One setting a row, each a tap for its next value, with the rows as tall
// as a fingertip (17 canvas pixels, ~6 mm), between hairlines.

/// Where the rows start, and how tall each is.
const SETTINGS_TOP: i32 = 12;
const SETTINGS_ROW: i32 = 17;

/// The settings screen's rows, top to bottom.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Row {
    Theme,
    Light,
    Buttons,
    HaKey,
}

const ROWS: [Row; 4] = [Row::Theme, Row::Light, Row::Buttons, Row::HaKey];

/// Which row a tap at `y` is on.
fn settings_row(y: i32) -> Option<Row> {
    let row = y.checked_sub(SETTINGS_TOP)? / SETTINGS_ROW;
    ROWS.get(usize::try_from(row).ok()?).copied()
}

fn render_settings(
    canvas: &mut Canvas,
    theme: &Theme,
    light: brightness::Setting,
    grid: buttons::Setting,
) {
    canvas.clear(BACKGROUND);
    canvas.text(4, 3, "SETTINGS", Color::ACCENT);
    for (i, row) in ROWS.into_iter().enumerate() {
        let top = SETTINGS_TOP + SETTINGS_ROW * i as i32;
        if i > 0 {
            canvas.fill_rect(4, top, 72, 1, Color::TEXT_FAINT);
        }
        let (label, value, color) = match row {
            Row::Theme => ("THEME", theme.name, Color::TEXT),
            Row::Light => ("LIGHT", light.name(), Color::TEXT),
            Row::Buttons => ("BUTTONS", grid.name(), Color::TEXT),
            // Not a value, but somewhere to go.
            Row::HaKey => ("HA KEY", "SHOW", Color::TEXT_DIM),
        };
        let y = top + (SETTINGS_ROW - 5) / 2 + 1;
        canvas.text(4, y, label, Color::TEXT_DIM);
        canvas.text_right(77, y, value, color);
    }
}

/// Home Assistant's key as a QR code, filling the screen (see
/// [`Canvas::qr`]), for a phone's camera: what it reads is the key as
/// text (32 hex digits, which Home Assistant takes in capitals too), to
/// paste into its BTHome integration. A tap shows it as text instead.
fn render_ha_qr(canvas: &mut Canvas, identity: Option<&Identity>) {
    canvas.clear(BACKGROUND);
    let Some(identity) = identity else {
        canvas.text_centered(40, 36, "NO KEY YET", Color::TEXT_DIM);
        return;
    };
    canvas.qr(&identity.key);
}

const HEX: &[u8; 16] = b"0123456789ABCDEF";

/// Home Assistant's key, for typing into its BTHome integration: 32 hex
/// digits, big, in four rows of two groups of four, and the address it'll
/// find the watch at, for picking it out among what it discovers.
fn render_ha_key(canvas: &mut Canvas, identity: Option<&Identity>) {
    canvas.clear(BACKGROUND);
    canvas.text_centered(40, 3, "HA KEY", Color::ACCENT);
    let Some(identity) = identity else {
        canvas.text_centered(40, 36, "NO KEY YET", Color::TEXT_DIM);
        return;
    };
    let digit = |byte: u8, high: bool| HEX[usize::from(if high { byte >> 4 } else { byte & 15 })];
    for (row, bytes) in identity.key.chunks(4).enumerate() {
        let mut text = [b' '; 9];
        for (i, &byte) in bytes.iter().enumerate() {
            // A space between the two groups.
            let at = 2 * i + usize::from(i >= 2);
            text[at] = digit(byte, true);
            text[at + 1] = digit(byte, false);
        }
        let text = core::str::from_utf8(&text).expect("ASCII");
        // 2×: 8 px a character, so 72 px (bar the last one's gap).
        canvas.text_scaled(5, 13 + 13 * row as i32, text, Color::TEXT, 2);
    }
    // As people write it: most significant byte first.
    let mut address = heapless::String::<17>::new();
    for (i, &byte) in identity.address.iter().rev().enumerate() {
        if i > 0 {
            let _ = address.push(':');
        }
        let _ = address.push(char::from(digit(byte, true)));
        let _ = address.push(char::from(digit(byte, false)));
    }
    canvas.text_centered(40, 69, &address, Color::TEXT_DIM);
}

// ==============================================================================
// The button grid
// ==============================================================================
//
// Nine buttons in a 3×3 grid, unlabelled: each 22 px (~8 mm) in the middle
// of its third of the screen, which is what a touch hits (see
// `buttons::at`), so the gaps between them belong to the nearest. They
// stay clear of the edges' handles.
//
// Drawn as an old arcade cabinet's or console's buttons, in the theme's
// colours: a chunky keycap with a lit edge and a glint at its top left,
// standing on its own dark side, 4 px deep. Pressed, it sinks: the cap
// drops 3 px into its well, the side gone and the well's shadow above it.
//
//    raised             pressed
//    ┌────────┐         ┌────────┐
//    │▓▓▓▓▓▓▓▓│ edge    │████████│ the well's shadow
//    │▓ ▪     │ glint   │█       │
//    │▓  cap  │         │   cap  │
//    │████████│ side    │        │
//    └────────┘         └────────┘

/// How a grid button looks.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Key {
    Up,
    /// Held down: a sequence's first button, while its second waits.
    Down,
    /// Held down and lit: what's waiting to see whether more follows.
    Lit,
    /// Just sent, with its code: lit in the "good" colour, or the "bad"
    /// one if it went nowhere.
    Sent(Press, bool),
}

/// The grid. `dark` picks the shading for a dark theme (or the dim look),
/// where edges catch light against black; on a light one, the ink is the
/// dark and the paper the light.
fn render_buttons(
    canvas: &mut Canvas,
    grid: &buttons::Grid,
    sent: Option<(Press, bool)>,
    dark: bool,
) {
    canvas.clear(BACKGROUND);
    let (first, second) = grid.waiting();
    for button in 1..=9u8 {
        let x = 2 + 27 * i32::from((button - 1) % 3);
        let y = 2 + 27 * i32::from((button - 1) / 3);
        let key = match (sent, first, second) {
            (Some((press, ok)), ..) if press.last() == button => Key::Sent(press, ok),
            (_, _, Some(second)) if second == button => Key::Lit,
            (_, Some(first), Some(_)) if first == button => Key::Down,
            (_, Some(first), None) if first == button => Key::Lit,
            _ => Key::Up,
        };
        draw_key(canvas, x, y, key, dark);
    }
}

/// One 22 px button at (`x`, `y`).
fn draw_key(canvas: &mut Canvas, x: i32, y: i32, key: Key, dark: bool) {
    const SIZE: i32 = 22;
    /// How deep the side is, and so how far a pressed cap sinks (less a
    /// pixel, so it still sits in a well).
    const DEPTH: i32 = 4;
    // The outline, the cap, its lit edge, and the side (and the well's
    // shadow).
    let (outline, cap, light, side) = if dark {
        (
            Color::TEXT_FAINT,
            Color::TRACK,
            Color::TEXT_DIM,
            Color::SHADOW,
        )
    } else {
        (Color::TEXT, Color::TRACK, BACKGROUND, Color::TEXT)
    };
    let (face, down) = match key {
        Key::Up => (cap, false),
        Key::Down => (cap, true),
        Key::Lit => (Color::ACCENT, true),
        Key::Sent(_, true) => (Color::GOOD, true),
        Key::Sent(_, false) => (Color::BAD, true),
    };
    canvas.fill_rect(x, y, SIZE, SIZE, outline);
    // Rounded off, a pixel at each corner.
    for (cx, cy) in [
        (x, y),
        (x + SIZE - 1, y),
        (x, y + SIZE - 1),
        (x + SIZE - 1, y + SIZE - 1),
    ] {
        canvas.pixel(cx, cy, BACKGROUND);
    }
    let (inner, width) = (x + 1, SIZE - 2);
    // Where the cap's top is: sunk into its well when pressed.
    let top = if down { y + DEPTH } else { y + 1 };
    if down {
        canvas.fill_rect(inner, y + 1, width, DEPTH - 1, side);
        canvas.fill_rect(inner, top, width, y + SIZE - 1 - top, face);
        // The well's shadow falls on the cap's left edge too.
        canvas.fill_rect(inner, top, 1, y + SIZE - 1 - top, side);
    } else {
        let cap_height = SIZE - 2 - DEPTH;
        canvas.fill_rect(inner, top, width, cap_height, face);
        // The cap's lower lip, then its side.
        canvas.fill_rect(inner, top + cap_height - 1, width, 1, outline);
        canvas.fill_rect(inner, top + cap_height, width, DEPTH, side);
        // Light from the top left: its edge, and a glint.
        canvas.fill_rect(inner, top, width - 1, 1, light);
        canvas.fill_rect(inner, top + 1, 1, cap_height - 2, light);
        canvas.fill_rect(x + 3, top + 2, 2, 1, light);
        canvas.pixel(x + 3, top + 3, light);
    }
    if let Key::Sent(press, _) = key {
        let mut code = heapless::String::<2>::new();
        write!(code, "{}", press.code).expect("at most 2 digits");
        let width = font::small_text_width(&code) * 2;
        canvas.text_scaled(x + 1 + (SIZE - width) / 2, top + 1, &code, BACKGROUND, 2);
        let how = match press.gesture {
            buttons::Gesture::Tap => "TAP",
            buttons::Gesture::DoubleTap => "2X",
            // Never shown: a release leaves the hold's on the screen.
            buttons::Gesture::Hold | buttons::Gesture::Release => "HOLD",
        };
        canvas.text_centered(x + 1 + SIZE / 2, top + 12, how, BACKGROUND);
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

// Notifications look like the watch's other screens: their app is the
// title, top left in the accent colour, as TODAY's or ALARMS' is, with the
// time top right, as MUSIC's state is. Then a rule, and who it's from,
// in a face that makes it the most important thing on the screen. The
// banner is the same in short, over the bottom of whatever shows.
//
// Who it's from is the 3×5 at 2× if it fits on one line (up to about 9
// letters): a poster, the size the watch's other big text is. Anything
// longer is in the medium face (see `font::medium_glyph`), balanced over
// as few lines as it takes (see `font::wrap`), never the small 1×, which
// is too small to read at a glance. That's two sizes, chosen by one rule,
// rather than the three there were: the 2× split over two lines when they
// came out about equal, and 1× otherwise, so a name's size depended more
// on how its words fell than on what it was.

/// Text columns of the banner and the notification card, as of a call:
/// four columns (12 of the panel's pixels) clear of each edge, for a
/// little more air than the three there were. Four still leave room for
/// 18 letters at 1× and 9 at 2×, as three did; a fifth would take a
/// letter off each.
const BANNER_TEXT: core::ops::Range<i32> = 4..76;
const BANNER_TEXT_WIDTH: i32 = BANNER_TEXT.end - BANNER_TEXT.start;

/// A notification's app, and who it's from, as UTF-8 to draw: room for
/// what [`Text`] keeps of each (see [`Text::write_to`]).
type AppText = String<{ 2 * 24 + 1 }>;
type WhoText = String<{ 2 * 40 + 1 }>;

/// Line pitch of the medium face: 7 px capitals, 2 accent rows, and a
/// row's gap, so an accent never touches the line above.
const MEDIUM_PITCH: i32 = font::MEDIUM_HEIGHT + font::ACCENT_ROWS + 1;

/// The most lines who it's from takes in the card: 40 letters at about 11
/// a line, with room to spare for long words.
const CARD_LINES: usize = 5;

/// Where who it's from starts on the card: two rows below the rule, plus
/// the accents' rows.
const CARD_WHO_Y: i32 = 12 + 2 + font::ACCENT_ROWS;

/// A notification, with the whole screen (one that arrived with the screen
/// off, in the dim look, or a page of the history): the app top left in
/// the accent colour, and `time` top right (the time now, or when it
/// arrived), a rule under them, as on Today, and who it's from below,
/// left-aligned, big if it's short.
fn render_card(canvas: &mut Canvas, app: &str, who: &str, time: &str) {
    canvas.clear(BACKGROUND);
    // The app gives way to the time: 4 px between them, and an ellipsis.
    let time_width = font::small_text_width(time);
    let gap = if time.is_empty() { 0 } else { time_width + 4 };
    small_fitted(
        canvas,
        BANNER_TEXT.start,
        3,
        app,
        BANNER_TEXT_WIDTH - gap,
        Color::ACCENT,
    );
    canvas.text_right(BANNER_TEXT.end, 3, time, Color::TEXT_DIM);
    // Dashed, 2 px on and 1 off: Today's chart baselines, in a colour
    // that shows in every look.
    for x in (BANNER_TEXT.start..BANNER_TEXT.end).step_by(3) {
        canvas.fill_rect(x, 11, 2, 1, Color::TEXT_FAINT);
    }

    if font::small_text_width(who) * 2 <= BANNER_TEXT_WIDTH {
        // The 2× accents are four rows tall, two more than the medium's.
        let y = CARD_WHO_Y + font::ACCENT_ROWS;
        canvas.text_scaled(BANNER_TEXT.start, y, who, Color::TEXT, 2);
        return;
    }
    let mut lines = [font::Line::default(); CARD_LINES];
    let count = font::wrap(who, font::Face::Medium, BANNER_TEXT_WIDTH, &mut lines);
    for (i, &line) in lines[..count].iter().enumerate() {
        let y = CARD_WHO_Y + MEDIUM_PITCH * i as i32;
        canvas.line(
            BANNER_TEXT.start,
            y,
            who,
            line,
            font::Face::Medium,
            Color::TEXT,
        );
    }
}

/// `text` in the small font at (`x`, `y`), in `room` px, ending in an
/// ellipsis if it doesn't fit.
fn small_fitted(canvas: &mut Canvas, x: i32, y: i32, text: &str, room: i32, color: Color) {
    let mut line = [font::Line::default()];
    if font::wrap(text, font::Face::Small, room, &mut line) > 0 {
        canvas.line(x, y, text, line[0], font::Face::Small, color);
    }
}

/// Where the banner's box starts with who it's from on one line, and on
/// two (see [`render_banner`]).
const BANNER_TOP: [i32; 2] = [56, 46];

/// Over the bottom of the screen: the app (or what happened) small in the
/// accent colour, and who it's from (or what to do) in the medium face,
/// on one line or two, ending in an ellipsis if even two won't do. The
/// history has the rest. One line covers the clock face's heart rate and
/// steps; two, from just under the seconds, the alarm too, which a 4 s
/// banner can afford; the time and date always show. Who it's from ends
/// 4 px above the bottom edge, as the text clears the sides.
fn render_banner(canvas: &mut Canvas, app: &str, who: &str) {
    let mut lines = [font::Line::default(); 2];
    let count = font::wrap(who, font::Face::Medium, BANNER_TEXT_WIDTH, &mut lines).max(1);
    let top = BANNER_TOP[count - 1];
    canvas.fill_rect(0, top, 80, 80 - top, BACKGROUND);
    canvas.fill_rect(0, top, 80, 1, Color::ACCENT);
    small_fitted(
        canvas,
        BANNER_TEXT.start,
        top + 4,
        app,
        BANNER_TEXT_WIDTH,
        Color::ACCENT,
    );
    for (i, &line) in lines[..count].iter().enumerate() {
        // The app's 5 rows, then 2 clear above the accents.
        let y = top + 4 + 5 + 2 + font::ACCENT_ROWS + MEDIUM_PITCH * i as i32;
        canvas.line(
            BANNER_TEXT.start,
            y,
            who,
            line,
            font::Face::Medium,
            Color::TEXT,
        );
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

// ==============================================================================
// Air
// ==============================================================================
//
// Canvas layout (80×80), y ranges inclusive:
//
//    3..7    AIR (left); how old the reading is (right)
//   13..33   CO₂ in ppm, in the time's digits at 3×, centred
//   38..42   "PPM CO2" (left); the band, GOOD/FAIR/POOR (right)
//   46..48   three segments, the band's lit: it reads without colour
//   54..63   temperature and humidity at 2×
//   69..73   pressure (left); the Aranet4's battery (right)

/// A nearby Aranet4's latest reading (see [`crate::aranet`]): the CO₂
/// big, since that's what the watch can't otherwise know and what to act
/// on (open a window), then what else it measures.
///
/// Not on the clock face: CO₂ changes over tens of minutes, and its number
/// would crowd a face that's read at a glance, many times a day. The
/// temperature there comes from the same reading (see
/// `crate::temperature`).
///
/// The readings are as old as the last scan, at most a quarter of an hour
/// with a sensor around (opening this screen scans again if the last is a
/// minute old), so the age is always shown; past [`aranet::STALE`], the
/// numbers go faint.
fn render_air(canvas: &mut Canvas, sighting: &Sighting, now: Instant) {
    canvas.clear(BACKGROUND);
    canvas.text(4, 3, "AIR", Color::ACCENT);
    let reading = &sighting.reading;
    let age = sighting.age(now.as_millis());
    let mut text = heapless::String::<12>::new();
    match age {
        0..60 => text.push_str("NOW").expect("fits"),
        60..3600 => write!(text, "{} MIN", age / 60).expect("fits"),
        _ => write!(text, "{} H", age / 3600).expect("fits"),
    }
    let stale = age >= aranet::STALE;
    canvas.text_right(76, 3, &text, Color::TEXT_DIM);
    let value = if stale {
        Color::TEXT_FAINT
    } else {
        Color::TEXT
    };

    // CO₂: up to four digits of 15 px with 2 px gaps, centred; dashes
    // while the sensor calibrates.
    const CO2_Y: i32 = 13;
    match reading.co2 {
        Some(ppm) => {
            let ppm = u64::from(ppm.min(9999));
            let digits = ppm.max(1).ilog10() as i32 + 1;
            let mut x = 40 - (digits * 17 - 2) / 2;
            for place in (0..digits).rev() {
                draw_digit(
                    canvas,
                    x,
                    CO2_Y,
                    ppm / 10u64.pow(place as u32) % 10,
                    3,
                    value,
                );
                x += 17;
            }
        }
        None => {
            for x in [6, 23, 40, 57] {
                canvas.fill_rect(x, CO2_Y + 9, 15, 3, value);
            }
        }
    }
    canvas.text(4, 38, "PPM CO2", Color::TEXT_DIM);
    // The band: the word, and which third of the bar is lit, so it reads
    // in every theme, even those of one hue.
    if let Some(band) = reading.band {
        let color = match band {
            Band::Good => Color::GOOD,
            Band::Fair => Color::TEXT,
            Band::Poor => Color::BAD,
        };
        canvas.text_right(76, 38, band.word(), color);
        for (i, segment) in [Band::Good, Band::Fair, Band::Poor].into_iter().enumerate() {
            let lit = if segment == band { color } else { Color::TRACK };
            canvas.fill_rect(4 + 25 * i as i32, 46, 22, 3, lit);
        }
    }

    // Temperature and humidity, at 2×.
    let mut text = heapless::String::<8>::new();
    match reading.tenths() {
        Some(tenths) => write!(text, "{}.{}°", tenths / 10, tenths % 10).expect("fits"),
        None => text.push_str("--.-°").expect("fits"),
    }
    canvas.text_scaled(4, 54, &text, value, 2);
    let mut text = heapless::String::<4>::new();
    write!(text, "{}%", reading.humidity.min(100)).expect("fits");
    canvas.text_scaled(76 - 2 * font::small_text_width(&text), 54, &text, value, 2);

    // Pressure, and the Aranet4's own battery, red when it wants changing.
    let mut text = heapless::String::<10>::new();
    if let Some(pressure) = reading.pressure {
        write!(text, "{} HPA", (pressure + 5) / 10).expect("fits");
    }
    canvas.text(4, 69, &text, Color::TEXT_DIM);
    let mut text = heapless::String::<8>::new();
    write!(text, "BAT {}%", reading.battery.min(100)).expect("fits");
    let color = if reading.battery <= 15 {
        Color::BAD
    } else {
        Color::TEXT_DIM
    };
    canvas.text_right(76, 69, &text, color);
}

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
