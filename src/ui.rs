//! Screens, navigation, and screen power.
//!
//! The UI is immediate-mode: [`App::render`] paints the current screen onto
//! the canvas from scratch, and the canvas works out what actually changed.
//! All state that affects what's shown lives in [`App`] or in the [`Status`]
//! snapshot passed to `render`. What's on screen is this module's; how
//! it looks is the chosen colour scheme's theme's (see [`crate::themes`]),
//! which draws every screen, as the screens below describe them in the
//! watch's own theme, retro.
//!
//! The screens sit around the clock face, in a line on each side, nearest
//! first (see [`crate::nav`] for how swipes move along them):
//!
//! - Left, the past: Today (steps, and the heart rate, by the hour), then
//!   Last night (sleep).
//! - Right, what's next: Alarms, then the phone's alarms, once it has sent
//!   some; then the settings, the look's ("LOOK") and the button grid's
//!   ("BUTTON GRID"), and Info.
//! - Above: the last [`HISTORY`] notifications, newest nearest, one a page,
//!   with when each arrived (see [`crate::arrival`]).
//! - Below: the button grid (see [`buttons`]), if it's on; Music, while
//!   the phone has media; Air, while a nearby room sensor (an Aranet4 or a
//!   BTHome thermometer) has been heard within the hour (see
//!   [`crate::aranet`]). Possibly none of them.
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
//! - On the clock, while a phone is connected, the media gestures of the
//!   screen off and a glance (see below): a double tap (the second tap
//!   within [`wake::DOUBLE_TAP_MS`] of the first) plays or pauses, and a
//!   tap, then a hold (a tap-and-hold, its hold starting as soon after the
//!   tap; see [`wake::follows`]), is the next track. A long press on its
//!   own does nothing. Both anywhere, bar the hold for
//!   [`wake::CENTRE_AFTER_TOUCH_WAKE_MS`] after a touch woke the screen,
//!   when it must be in the middle ([`wake::centred`]). Each buzzes, and
//!   shows what it did for a moment where a note shows while media plays,
//!   at the status bar's left end. The phone needn't have said it has
//!   media (see `App::media`). Without a phone, neither does anything. A
//!   long press acts (and buzzes) once the finger has been down long
//!   enough, before it lifts (see [`crate::touch`]); that goes for every
//!   long press.
//! - Long press elsewhere: on the alarms screen, switch an alarm on or off;
//!   on the info screen, offer storage mode (see [`crate::system_off`]),
//!   which a tap then confirms, and anything else cancels. Elsewhere,
//!   nothing.
//! - On the settings screens, tap a row for its next value. On the
//!   look's: the theme ("THEME": the next colour scheme, of every theme
//!   built in, for the side of the day showing, night after sunset, day
//!   after sunrise; each side remembers its own), which side's schemes
//!   show ("FORCE": NONE for the sun's, or DAY or NIGHT whatever the sun
//!   says; only the theme's side: the brightness and the rest still go by
//!   the sun), and the brightness ("LIGHT", auto or
//!   soft; see [`crate::brightness`]). On the button grid's: where its
//!   presses go ("BUTTONS": off, "BTHOME", "VIA APP" for Gadgetbridge on
//!   the phone, or "BOTH"), its layout ("GRID": six buttons, or two
//!   layers; over BTHome, the latter shows "*B04", a caveat: FAQ B04
//!   explains the events it sends then), and, on the last row ("BTHOME
//!   KEY", there only while presses go over BTHome), the key Home
//!   Assistant needs for them (see [`crate::bthome`]): a QR code to scan,
//!   then, at a tap, as text with the watch's address. Another tap, or
//!   anything else, closes it. THEME and FORCE are always there, so no
//!   row moves: with one scheme for the side showing, THEME can't change,
//!   nor FORCE with schemes of one side only in the build, and a tap on
//!   either shows a banner with the FAQ entry that says why ("ONLY ONE
//!   THEME", D01; "NO DAY THEMES" or "NO NIGHT THEMES", D02).
//! - On the button grid, taps and holds are the grid's (see [`buttons`]
//!   for how they become presses). While a press waits to see whether
//!   more follows, its buttons stay lit; once sent, the last one shows the
//!   code, in the "good" colour (or "bad", if the phone was all it could
//!   go to, and it wasn't connected, or it couldn't go to Home Assistant,
//!   which a banner adds, "SEE FAQ B05"), with a buzz. A hold held on
//!   is a long press too, shown and buzzed the same, as "LONG"; between
//!   and after what it sends, the held button shows pressed, until the
//!   finger lifts. A swipe,
//!   or the button, cancels what's waiting.
//! - Screens that take over (the alarm editor, a ringing alarm, a call,
//!   "find my watch", pairing, an update, the storage prompt) get every
//!   touch: swipes there don't navigate.
//! - While the screen is off, a button press, a deliberate tap (see
//!   [`wake::deliberate`]), or a double tap (two taps within
//!   [`wake::DOUBLE_TAP_MS`]): wake to the clock. Other touches don't, so
//!   a brush of a sleeve doesn't light the screen. (The waking input is
//!   *only* a wake-up; it isn't also acted on.) While a phone is
//!   connected, outside the quiet night, media gestures work without
//!   waking the screen: a double tap plays or pauses, and a tap, then a
//!   hold in the middle (see [`wake::second`]), is the next track. A long
//!   press on its own does nothing. Each buzzes, and that's all: the
//!   screen stays dark. A deliberate tap still wakes the screen at once,
//!   so only a gesture whose first tap is light stays dark; after a
//!   deliberate one, the second touch does the same on the clock face it
//!   woke to. See [`crate::wake`] for the guards against brushes.
//! - No input for [`SCREEN_TIMEOUT`] (longer on the music screen, for a
//!   long title to scroll by; never while a banner shows, or Home
//!   Assistant's key): turn the screen off. A long press held on (a
//!   grid hold dimming a light) is input until its finger lifts, up to
//!   [`crate::gesture::HOLD_LIMIT`]; not one that woke the screen.
//! - Turning the wrist to look (see [`crate::glance`]) while the screen is
//!   off: a glance, just the time in white on black at a low backlight, for
//!   [`GLANCE_TIME`]. While the phone navigates, the next turn instead,
//!   with the time smaller above it. A tap or press during it (one is
//!   enough: the wearer is looking) wakes the screen properly. While a
//!   phone is connected, outside the quiet night, the screen off's media
//!   gestures work during a glance too, and the glance stays (with what it
//!   did in a corner, for a moment): a double tap plays or pauses, and a
//!   tap, then a hold in the middle, is the next track. That takes a first
//!   tap away from the edge, and not a heavy one: a tap held at least
//!   [`wake::GLANCE_WAKE_TAP_MS`] wakes the screen at once, and the second
//!   touch does the same on the clock face it woke to. A deliberate tap
//!   short of that (see [`wake::deliberate`]) wakes the screen too, but
//!   only once no second touch can follow it (half a second on, or once
//!   a finger put down by then lifts), so its pair leaves the glance be.
//!   A lighter tap, or a long press, on its own does nothing.
//!
//! Alarms (see [`crate::alarm`] for when they ring):
//!
//! - On the alarms screen, tap a row (smart wake, alarm, timer) to edit it;
//!   long-press to switch it on or off (or start/stop the timer).
//! - In the editor: swipe up/down on the left half for hours, the right
//!   half for minutes; tap the bottom left to switch it on/off, the bottom
//!   right for the next page or done. The button also means done, as
//!   does the screen going off: each change takes effect, and is saved,
//!   as it's made, so there's nothing to keep or throw away.
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
//!   glance, as a history page shows it (see [`Theme::notification`]), and the
//!   screen sleeps again after. Either way, it's kept in
//!   the history above the clock; one arriving while that's showing goes
//!   straight to its first page instead of a banner.
//! - Calls: take over the screen, ringing, until the phone says the call is
//!   over (answered, rejected, ended) or the user acts.
//! - "Find my watch" (Gadgetbridge's "Find lost device"): takes over the
//!   screen ("HERE", with rings spreading round it) and buzzes as hard as
//!   the motor can ([`Pattern::FIND`]), until the button or any touch
//!   (whoever did has found it), the phone's "Found it!", or [`MAX_FIND`].
//!   Under a call or a ringing alarm, it waits its turn: the watch rings
//!   already. Whatever stops a call, an alarm, or this leaves the motor to
//!   what still rings beneath it (see `App::buzz_for_screen`), and a
//!   notification doesn't buzz over any of them.
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
//!   still does, and so does turning the wrist, bar while sleep tracking
//!   says the wearer is asleep (see `activity::motion_task`). Once it's
//!   on, touches work as usual.
//! - A double press of the button ends it: the first press wakes the
//!   screen (dimly) as usual, and the second, rather than turning it off
//!   again, brightens it.
//! - Everything shows in the dim look (white, and red for what comes
//!   second, on black, in retro; see [`crate::themes`]), at the glance's low
//!   backlight.
//! - With the screen off, the touch controller sleeps (see
//!   [`crate::touch::set_asleep`]).
//! - Alarms (which end the night), timers, "find my watch", and calls still
//!   ring and buzz.
//!
//! The quiet night yields to the wearer using the watch, for as long as
//! they do, and only to that: the screen woken (by the button, or a
//! glance) and touched. What moves a sleeper (turning over, bedding on the
//! screen) mustn't count, which is the point of the night; and once the
//! wearer stops, it's as quiet as before. What it does, and whether it
//! yields:
//!
//! | The quiet night | While the wearer uses the screen | Why |
//! |---|---|---|
//! | Screen off [`SCREEN_TIMEOUT`] after the last input | Yields: a long press held on is input until it lifts (up to [`crate::gesture::HOLD_LIMIT`], a minute), so a grid hold's release comes at the lift. The same by day | It's input that lasts; the screen going off put the touch controller to sleep, ending the hold 10 s in |
//! | The touch controller sleeps | Yields: only while the screen is off | A touch with the screen off is as likely bedding |
//! | Touches don't wake the screen, nor do the media gestures with it off | Kept | As above; the button wakes it. A touch that woke it from a glance doesn't hold it on either |
//! | No glances while asleep (as sleep tracking says) | Kept | That's turning over in bed; and the screen's on anyway |
//! | The dim look, at the low backlight | Kept | Eyes adapted to the dark, and maybe someone asleep alongside; a double press of the button ends the night for the full look |
//! | Notifications don't buzz or light the screen | Yields already: one waiting shows as a banner while the screen is on. Still no buzz | The banner is seen; the buzz would only wake someone alongside |
//! | No room sensor scans in the background | Yields already: the clock face, or the Air screen, scans (see [`aranet::Nearby::due`]) | |
//! | Media gestures | Yields already: the awake clock face's work as by day | |
//! | Navigation's turns don't buzz or glance | Kept | No one navigates in bed; up and walking, the night soon ends |
//! | Sleep tracking, and when the night ends | Kept: using the watch only counts through the wrist's movement | See [`sleep::Night`] on [`sleep::QUIET_AWAKE`] |

use core::num::NonZeroU32;

use embassy_time::{Duration, Instant};
use heapless::String;

use crate::InputEvent;
use crate::alarm;
use crate::aranet::Sighting;
use crate::arrival;
use crate::battery::Battery;
use crate::ble;
use crate::brightness;
use crate::bthome::Identity;
use crate::buttons::{self, Press};
use crate::canvas::{Canvas, Sweep, marquee_cycle};
use crate::clock::DateTime;
use crate::gb::{CallAction, MusicCommand, Outgoing};
use crate::motor::{self, Pattern};
use crate::nav::{self, Place, Side};
// Defined apart from the UI, so that `gb` (which decodes them) builds
// without it, in host tests; see `phone`.
use crate::palette::Palette;
use crate::phone::Text;
pub use crate::phone::{PhoneEvent, Unpaired};
use crate::route::Route;
use crate::sleep::{self, Change, Over};
use crate::themes::{
    self, CallButton, EditorButton, EditorField, Handles, MediaAction, RowKind, SettingRow, Theme,
};
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
/// How long after a touch woke the screen the clock face's tap-and-hold
/// must hold in the middle (see [`wake::awake_long_press`]).
const CENTRE_AFTER_TOUCH_WAKE: Duration =
    Duration::from_millis(wake::CENTRE_AFTER_TOUCH_WAKE_MS as u64);
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
/// Stop looking for the watch after this long, in case the phone never
/// says to (Gadgetbridge only does when its "Found it!" is tapped, or the
/// link may have dropped): long enough to go round a house listening, and
/// a tap on the phone starts it again.
const MAX_FIND: Duration = Duration::from_secs(120);
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
    /// The temperature in tenths of a °C (see [`crate::temperature`]),
    /// once it's been read: whole degrees (a multiple of ten) unless it's
    /// from a room sensor.
    pub temperature: Option<i16>,
    /// Whether that's a nearby room sensor's (an Aranet4's or a BTHome
    /// thermometer's), rather than the watch's own (or an estimate from it).
    pub temperature_from_sensor: bool,
    /// The nearest room sensor's latest reading, for the Air screen.
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
    /// One of the two settings screens.
    Settings(Page),
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

/// The most screens in a line: the notifications', or the one on the
/// right (the alarms, the phone's, the two settings screens, and info).
const LONGEST_LINE: usize = if HISTORY > 5 { HISTORY } else { 5 };
type Screens = heapless::Vec<Screen, LONGEST_LINE>;

/// An alarm ringing.
struct Ringing {
    kind: alarm::Kind,
    since: Instant,
}

/// Which setting the editor is changing.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Editing {
    SmartWeekday,
    SmartWeekend,
    /// Which of [`alarm::ALARMS`].
    At(u8),
    /// Which of [`alarm::TIMERS`].
    Timer(u8),
}

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
    /// That a grid press couldn't go to Home Assistant (see
    /// [`App::ha_unsent`]).
    Unsent,
    /// That THEME can't change: the build has one scheme for the side
    /// showing (a tap on it, see [`App::change`]).
    OneTheme,
    /// That FORCE can't change: the build has schemes of one side only.
    OneSide,
}

/// How many notifications the history above the clock keeps. Each costs
/// 68 bytes of RAM (its two texts, and when it arrived), where there's
/// little to spare.
const HISTORY: usize = 3;

struct Notification {
    app: Text<24>,
    /// Who it's from: Gadgetbridge's `sender`, or its title (see `gb`).
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
    /// The chosen scheme for each side of the day: indices into
    /// [`themes::schemes`]. Tapping THEME on the settings advances the one
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
    /// Since when the phone has been looking for the watch ("find my
    /// watch"), until someone finds it, the phone stops, or [`MAX_FIND`].
    finding: Option<Instant>,
    /// The alarm editor, while open.
    editing: Option<Editing>,
    /// The local minute, as of the last tick (for edits that need "now").
    local_second: Option<i64>,
    /// While glancing: until when.
    glance_until: Option<Instant>,
    /// When the last tap was, as the possible first of a pair (see
    /// [`wake::follows`]): with the screen off, to wake it (or, with a phone
    /// connected, for the media gestures, as on a glance); on the clock,
    /// for the same media gestures. One field for all, as RAM is tight: a tap only
    /// counts while the screen is in the same state (on or off) as for the
    /// first, as waking or any other input clears it. (Bar a deliberate
    /// tap that wakes the screen, from off or a glance, while a phone is
    /// connected: that one may be the first of a media gesture finished
    /// on the clock face it woke to; see `woke_tap`.)
    last_tap: Option<Instant>,
    /// Whether `last_tap` is the tap that woke the screen, from off or a
    /// glance: the touch after it is then taken as with the screen off,
    /// with its guards against brushes, rather than as the clock face's
    /// own (see [`wake::second`]). Cleared by the next input on the awake
    /// screen.
    woke_tap: bool,
    /// On a glance, whether `last_tap` was a deliberate but not heavy tap
    /// (see [`wake::GLANCE_WAKE_TAP_MS`]), whose wake waits to see whether
    /// a second touch follows it ([`wake::Action::WakeLater`]; see
    /// [`App::held_wake`]). Only counts
    /// while glancing: whatever else ends the glance takes the screen
    /// anyway, and the screen going off clears it, so it never outlives
    /// that into the next glance.
    glance_wake: bool,
    /// When the button was last pressed, for a double press (see
    /// [`DOUBLE_PRESS`]). Boot, until the first.
    last_press: Instant,
    /// Until when the clock face's tap-and-hold must hold in the middle,
    /// as on a glance: [`CENTRE_AFTER_TOUCH_WAKE`] after a touch last woke
    /// the screen, from off or a glance (see [`wake::awake_long_press`]).
    /// Boot, until the first. (A wake by anything else doesn't end it
    /// early: that soon, only the button can turn the screen off and on
    /// again, so that keeps the rule a moment longer at most.)
    centre_until: Instant,
    /// Whether it's the quiet night, and what decides when it ends. (8
    /// bytes, in a build with little RAM to spare.)
    night: sleep::Night,
    /// The brightness setting, as stored (`main` saves it when it changes).
    pub brightness: brightness::Setting,
    /// The settings' FORCE, as stored (`main` saves it when it changes): which
    /// side of the day's schemes the theme shows, if not the sun's.
    pub force: Force,
    /// Whether it's between local midnight and the wearer getting up (see
    /// [`brightness::small_hours`]).
    small_hours: bool,
    /// The user confirmed storage mode; `main` switches the watch off.
    pub storage_mode: bool,
    /// Pairing or an update has the screen (as of the last tick), and so
    /// every touch (see the module docs).
    takeover: bool,
    /// Just after a media gesture on the clock: what it did, and for how
    /// many more ticks to show it.
    media_hint: Option<(MediaAction, u8)>,
    /// The phone's directions, while it navigates (see [`crate::route`]).
    route: Option<Route>,
    /// Whether this connection's phone has been told to turn off "Text as
    /// Bitmaps" (see [`PhoneEvent::TextAsBitmaps`]): once is enough, when
    /// every message may carry them.
    told_bitmaps: bool,
    /// When the "NOT PAIRED" and "PAIRING LOST" banners may show again:
    /// a client that won't pair reconnects every minute or so, all day
    /// (see [`crate::unpaired`]).
    unpaired_banners: crate::unpaired::Banners,
    /// The button grid's setting, as stored (`main` saves it when it
    /// changes).
    pub buttons: buttons::Setting,
    /// The button grid's layout, as stored (likewise).
    pub grid_layout: buttons::Layout,
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
    /// Whether a long press begun on the awake screen is still held on,
    /// until its release: the screen stays on for it, up to
    /// [`crate::gesture::HOLD_LIMIT`] (see `App::screen_timeout`). Not a
    /// touch that woke the screen (from off, or a glance): in the quiet
    /// night that's as likely bedding as a finger.
    holding: bool,
}

/// How long the grid shows a press it sent.
const SENT_TIME: Duration = Duration::from_millis(800);

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ThemeChoice {
    /// Indices into `themes::schemes`, for daylight and for darkness.
    pub day: u8,
    pub night: u8,
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
            finding: None,
            editing: None,
            local_second: None,
            glance_until: None,
            last_tap: None,
            woke_tap: false,
            glance_wake: false,
            last_press: Instant::from_ticks(0),
            centre_until: Instant::from_ticks(0),
            night: sleep::Night::default(),
            brightness: brightness::Setting::default(),
            force: Force::default(),
            small_hours: false,
            storage_mode: false,
            takeover: false,
            media_hint: None,
            route: None,
            told_bitmaps: false,
            unpaired_banners: crate::unpaired::Banners::default(),
            buttons: buttons::Setting::Off,
            grid_layout: buttons::Layout::default(),
            grid: buttons::Grid::default(),
            sent: None,
            ha_presses: heapless::Vec::new(),
            holding: false,
        }
    }

    /// Checks what must hold of the UI's state whenever the loop isn't in
    /// the middle of changing it: after each wake is handled, and before
    /// each frame (see `ui_loop`). Only in builds with debug assertions
    /// (the simulator, and its fuzzer); the watch's does nothing here.
    ///
    /// Each of these is something the code elsewhere relies on without
    /// looking, so that breaking one shows up here, at the step that broke
    /// it, rather than as a screen that's stuck on or shows the wrong
    /// thing some steps later.
    pub fn check(&self) {
        if !cfg!(debug_assertions) {
            return;
        }
        // A glance is the clock face, lit, with the screen otherwise off:
        // it only starts from the screen off (which is on the clock), and
        // whatever wakes the screen properly, or takes it over, ends it
        // first. `on_tick` relies on it: while glancing it does nothing
        // but end the glance, so a call or an alarm ringing under one
        // would never time out, and a banner never go.
        // A deliberate tap's wake waits only on a glance, which the
        // screen going off ends (see `held_wake`).
        debug_assert!(
            !self.glance_wake || self.awake,
            "a glance's tap in the dark"
        );
        if self.glancing() {
            debug_assert!(self.awake, "a glance with the screen off");
            debug_assert!(self.screen == Screen::Clock, "a glance off the clock");
            debug_assert!(!self.brief_wake, "a glance in a notification's wake");
            debug_assert!(!self.ringing_any(), "a glance while ringing");
        }
        // A notification's own wake lasts as long as its banner, on the
        // clock it woke to: `on_tick` holds the screen on for it every
        // second, and only the banner going (or input) ends it. Without a
        // banner, the screen would stay on until the battery's empty.
        if self.brief_wake {
            debug_assert!(self.awake, "a brief wake with the screen off");
            debug_assert!(self.screen == Screen::Clock, "a brief wake elsewhere");
            // And the banner's time has started: it's on screen, as
            // nothing covers a brief wake (whatever would, ends it). One
            // waiting to be seen would hold the screen on until an input,
            // as the brief wake's end is the banner's.
            debug_assert!(
                self.banner.as_ref().is_some_and(|b| b.until != UNSEEN),
                "a brief wake with no banner, or one with no end"
            );
        }
        // What rings has the screen, lit, until it's dealt with: each
        // turns the screen on as it starts, and holds it on from then
        // (`on_tick`). And an alarm closes the editor as it starts to ring
        // (it couldn't be opened under one: the ringing takes the touch).
        debug_assert!(!self.ringing_any() || self.awake, "ringing in the dark");
        debug_assert!(self.ringing.is_none() || self.editing.is_none());
        // The editor is only open with the screen on, over Alarms, where
        // it opens: the screen going off closes it (`screen_off`), and
        // nothing moves to another screen under it, as it takes every
        // touch. Otherwise a wake shows it over the clock face it went to
        // (with `showing_clock` true, so the heart rate is measured for a
        // clock face nobody sees), and a banner waiting under it never
        // starts its time (see below).
        if self.editing.is_some() {
            debug_assert!(self.awake, "the alarm editor with the screen off");
            debug_assert!(self.screen == Screen::Alarms, "the alarm editor off Alarms");
        }

        // The screen showing is one that's there: on the clock, or in a
        // line (`place` takes anything else for the clock, and `render`
        // falls back to the clock face for a page with nothing to show).
        // Screens come and go (music with the phone, the air with its
        // sensor, the phone's alarms, the grid with its setting), and
        // each must take the wearer off its screen as it goes.
        debug_assert!(
            self.screen == Screen::Clock || self.place() != Place::Clock,
            "on a screen that isn't there"
        );
        // The key's screens are only reached from its row, which is only
        // there while presses go over BTHome, and nothing changes that
        // setting from them.
        debug_assert!(
            !matches!(self.screen, Screen::HaQr | Screen::HaKey) || self.buttons.bthome()
        );
        // Only a tap on the prompt confirms storage mode.
        debug_assert!(!self.storage_mode || self.screen == Screen::Storage);
        // A hint that's showing has ticks left (`on_tick` drops it rather
        // than count to zero).
        debug_assert!(
            self.media_hint
                .is_none_or(|(_, ticks)| (1..=MEDIA_HINT_TICKS).contains(&ticks))
        );
        // The history never shrinks, so once there's a notification to
        // have a banner (or not to have been seen), there always is.
        let none = self.notifications.list.is_empty();
        debug_assert!(!(none && self.banner_newest()), "a banner for nothing");
        debug_assert!(!(none && self.notifications.unseen));
        debug_assert!(self.call.as_ref().is_none_or(|c| c.since <= c.until));
        // The alarms' settings are times of day and the timers' lengths
        // within what the editor allows, as storage loads them
        // (`alarm::Settings::from_bytes`) and the editor keeps them: the
        // screens' "HH:MM" and the editor's two digits a side rely on it.
        let alarms = &self.alarms;
        let times = [alarms.smart_weekday, alarms.smart_weekend]
            .into_iter()
            .chain(alarms.at_time);
        debug_assert!(times.into_iter().all(|time| time < 24 * 60));
        debug_assert!(
            alarms
                .timers
                .iter()
                .all(|timer| (1..=alarm::MAX_TIMER).contains(timer))
        );
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
            && self.finding.is_none()
    }

    pub fn set_daytime(&mut self, daytime: bool) {
        self.daytime = daytime;
    }

    /// Whether a nearby room sensor has been heard recently enough for its
    /// screen to be there ([`aranet::RECENT`]). It goes away when not,
    /// leaving the clock if it was showing.
    pub fn set_air(&mut self, air: bool) {
        self.air = air;
        if !air && self.screen == Screen::Air {
            self.screen = Screen::Clock;
        }
    }

    /// Whether it's asking to switch to storage mode, which a tap would
    /// confirm: for the simulator's dev kit fuzzer, which mustn't switch
    /// the dev kit off.
    #[cfg(not(target_os = "none"))]
    pub fn offering_storage_mode(&self) -> bool {
        self.screen == Screen::Storage
    }

    /// Whether the Air screen is on screen, for a scan to refresh it.
    pub fn showing_air(&self) -> bool {
        self.awake
            && self.screen == Screen::Air
            && self.call.is_none()
            && self.ringing.is_none()
            && self.finding.is_none()
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

    /// The palette the screen shows in: the chosen scheme's, or in the
    /// dim look, its dim one.
    pub fn palette(&self) -> &'static Palette {
        let theme = self.theme();
        if self.dim() {
            theme.dim()
        } else {
            theme.palette()
        }
    }

    /// The colour scheme chosen for the time of day (see
    /// [`crate::themes`]), which draws every screen, dim or not.
    pub fn theme(&self) -> &'static themes::Scheme {
        let day = self.theme_side();
        let choice = if day {
            self.themes.day
        } else {
            self.themes.night
        };
        themes::get(day, usize::from(choice))
    }

    /// FORCE, as it applies: in a build with schemes of one side only,
    /// whatever was saved (by a build with both) pins nothing, as there's
    /// nothing to pin, and shows as NONE. It stays saved, for a build with
    /// both again.
    fn force(&self) -> Force {
        if themes::has(true) && themes::has(false) {
            self.force
        } else {
            Force::Sun
        }
    }

    /// Whether the theme shows a scheme for daylight, rather than for
    /// darkness: the sun's say (`daytime`), unless the settings' FORCE pins
    /// a side. Only the theme's: the brightness, and everything else
    /// that goes by the sun, still does.
    fn theme_side(&self) -> bool {
        match self.force() {
            Force::Sun => self.daytime,
            Force::Day => true,
            Force::Night => false,
        }
    }

    /// The next scheme for the side the theme shows.
    fn next_theme(&mut self) {
        let day = self.theme_side();
        let count = themes::count(day);
        let choice = if day {
            &mut self.themes.day
        } else {
            &mut self.themes.night
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
                // kinds of alarm stay together. The settings first, as
                // they're what's used: the look's, then the grid's.
                add(Screen::Settings(Page::Look));
                add(Screen::Settings(Page::Grid));
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
            Screen::HaQr | Screen::HaKey => Screen::Settings(Page::Grid),
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
            Shows::Unsent => ("BTHOME NOT SENT", "SEE FAQ B05"),
            Shows::OneTheme => ("ONLY ONE THEME", "SEE FAQ D01"),
            // The side there are none of, which FORCE would pick.
            Shows::OneSide if themes::has(true) => ("NO NIGHT THEMES", "SEE FAQ D02"),
            Shows::OneSide => ("NO DAY THEMES", "SEE FAQ D02"),
        };
        app.clear();
        who.clear();
        app.push_str(title).fits();
        who.push_str(hint).fits();
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
            //
            // A hold the screen stayed on for ends now: its time starts
            // from the lift, as from any input's (see `screen_timeout`).
            InputEvent::Touch(touch) if touch.gesture == Gesture::Release => {
                if core::mem::take(&mut self.holding) {
                    self.last_input = now;
                }
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
        // with any wake-up, don't also act on it). Bar the media gestures,
        // while a phone is connected: after a light tap, a second tap plays
        // or pauses, and a hold in the middle skips, both without waking
        // (a look at the wrist is all the wearer wanted), the glance
        // showing what it did. A long press on its own, or a light tap at
        // the edge, does nothing: most likely the other arm, folded over
        // the watch. A heavy tap (longer than a deliberate one from off)
        // wakes the screen at once, and the touch after it then plays,
        // pauses, or skips the same, on the clock face it woke to. A
        // deliberate but lighter tap is the first of a pair, as a light
        // one is, but if no second follows, it wakes the screen once none
        // can (see `held_wake`), as it would have at once on its own (see
        // `wake::glance`). Not in the quiet night, when bedding presses on
        // a wrist that turned in sleep could otherwise start the music;
        // there any touch only wakes the screen at once, as before.
        if let Some(until) = self.glance_until {
            let media = self.connected && !self.night.quiet();
            let waking = core::mem::take(&mut self.glance_wake);
            let action = match event {
                InputEvent::Touch(touch) if media => wake::glance(
                    wake_kind(touch.gesture),
                    touch.x,
                    touch.y,
                    touch.held_ms,
                    self.last_tap.is_some_and(|tap| follows(tap, touch, now)),
                    waking,
                ),
                _ => Action::Wake { first_tap: false },
            };
            let command = match action {
                Action::PlayPause => MusicCommand::PlayPause,
                Action::Next => MusicCommand::Next,
                // A long press, or a light tap at the edge: the glance goes
                // on as if it never happened. (Nor is a tap before it
                // still the first of a pair: the two touches must be one
                // after the other.)
                Action::Ignore => {
                    self.last_tap = None;
                    return;
                }
                // A light tap: the glance stays, in case a second follows.
                Action::FirstTap => {
                    self.last_tap = Some(now);
                    return;
                }
                // A deliberate but not heavy tap: the glance stays for now
                // too, and the screen wakes if no second follows.
                Action::WakeLater => {
                    self.last_tap = Some(now);
                    self.glance_wake = true;
                    return;
                }
                // Anything else wakes the screen properly. After a heavy
                // tap, a second may then skip, on the clock face it woke
                // to.
                wake => {
                    self.glance_until = None;
                    self.last_input = now;
                    if let InputEvent::Touch(_) = event {
                        self.centre_until = now + CENTRE_AFTER_TOUCH_WAKE;
                    }
                    self.woke_tap = wake == Action::Wake { first_tap: true };
                    self.last_tap = self.woke_tap.then_some(now);
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
        // after a light tap, a second tap plays or pauses, and a hold in
        // the middle skips, without waking it. A deliberate tap wakes it
        // at once, and may be the first of a media gesture finished on the
        // clock face it woke to. Anything else (a swipe, a short tap or
        // one at the edge, a long press on its own) is most likely a
        // brush, and is ignored, bar remembering a tap.
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
            let kind = wake_kind(touch.gesture);
            let recent_tap = self.last_tap.is_some_and(|tap| follows(tap, touch, now));
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
                // also be the first of a media gesture finished on the
                // clock face it wakes to, as on a glance.
                Action::Wake { first_tap: true } => {
                    wake_tap = Some(now);
                    None
                }
                // (Only a glance waits to wake, never the screen off; were
                // it to, waking at once is what a deliberate tap did.)
                Action::Wake { first_tap: false } | Action::WakeLater => None,
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
        // The first of a pair, if this may be its second. (Never a tap
        // from before the screen woke: waking only keeps the tap that woke
        // it, below and on a glance, for the next one.)
        let first_tap = self.last_tap.take().filter(|&tap| {
            self.awake && matches!(event, InputEvent::Touch(touch) if follows(tap, touch, now))
        });
        // After a tap that woke the screen, from off or a glance, the
        // touch that follows it is taken as with the screen off: a media
        // gesture's second (see `wake::second`). The clock face's own
        // gestures are the same, but a pair begun in the dark keeps the
        // dark's guards (away from the edge, the hold in the middle), as
        // its first tap was one the clock face never saw. Anything else
        // stands on its own: not the second of the clock face's double
        // tap either.
        let woke_tap = core::mem::take(&mut self.woke_tap);
        let (woken, first_tap) = match event {
            InputEvent::Touch(touch) if woke_tap => {
                let second = first_tap
                    .and_then(|_| wake::second(wake_kind(touch.gesture), touch.x, touch.y));
                let command = match second {
                    Some(Action::PlayPause) => Some(MusicCommand::PlayPause),
                    Some(Action::Next) => Some(MusicCommand::Next),
                    _ => None,
                };
                (command, None)
            }
            _ => (None, first_tap),
        };
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
            self.woke_tap = wake_tap.is_some();
            if let InputEvent::Touch(_) = event {
                self.centre_until = now + CENTRE_AFTER_TOUCH_WAKE;
            }
            return;
        }
        // A long press on the awake screen, still held: the screen stays
        // on until its release (see `screen_timeout`), whatever it does.
        if let InputEvent::Touch(touch) = event {
            self.holding = touch.gesture == Gesture::LongPress;
        }
        // Pairing or an update: what's beneath doesn't show, so touches
        // mustn't change it.
        if self.takeover && matches!(event, InputEvent::Touch(_)) {
            return;
        }

        // "Find my watch": whoever presses the button or touches the
        // screen has found it. That's all the input does, unless a call or
        // a ringing alarm has the screen over it: then it goes on to that,
        // as what the wearer saw and meant it for. (Gadgetbridge can't be
        // told: its "Found it!" stays up on the phone until tapped there.)
        if self.finding.is_some() {
            defmt::info!("find: found");
            if self.stop_finding() {
                return;
            }
        }

        if let Some(call) = &mut self.call {
            match event {
                InputEvent::Button if !call.silenced => {
                    call.silenced = true;
                    motor::buzz(Pattern::STOP);
                    send(Outgoing::Call(CallAction::Ignore));
                }
                InputEvent::Touch(touch)
                    if let Some(button) = (touch.gesture == Gesture::Tap)
                        .then(|| self.theme().call_button(touch.x, touch.y))
                        .flatten() =>
                {
                    let action = match button {
                        CallButton::Decline => CallAction::Reject,
                        CallButton::Answer => CallAction::Accept,
                    };
                    send(Outgoing::Call(action));
                    self.call = None;
                    // An alarm beneath it rings on.
                    self.buzz_for_screen();
                }
                _ => {}
            }
            return;
        }

        if self.ringing.is_some() {
            match event {
                InputEvent::Button => self.stop_ringing(),
                InputEvent::Touch(touch) if touch.gesture == Gesture::Tap => {
                    self.alarm_state.snooze(now.as_secs() as i64);
                    self.stop_ringing();
                }
                _ => {}
            }
            return;
        }

        if let Some(editing) = self.editing {
            self.on_editor_input(editing, event, now);
            return;
        }

        match event {
            InputEvent::Phone(_) | InputEvent::Glance => unreachable!("handled above"),
            // Only ever a reason to wake up, handled above.
            InputEvent::Attention => {}
            InputEvent::Button => match nav::home(self.place()) {
                Some(towards) => self.go_to(Screen::Clock, sweep(towards)),
                None => self.screen_off(),
            },
            InputEvent::Touch(touch) => match (touch.gesture, self.screen) {
                // Storage mode needs a long press, then a tap: hard to do by
                // accident, while anything else backs out.
                (Gesture::Tap, Screen::Storage)
                    if self.theme().storage_confirms(touch.x, touch.y) =>
                {
                    self.storage_mode = true
                }
                (_, Screen::Storage) => self.go_to(Screen::Info, Sweep::LeftToRight),
                (Gesture::LongPress, Screen::Info) => {
                    self.go_to(Screen::Storage, Sweep::RightToLeft);
                    motor::buzz(Pattern::NOTIFY);
                }
                (Gesture::Tap, Screen::HaQr) => self.go_to(Screen::HaKey, Sweep::RightToLeft),
                (_, Screen::HaQr | Screen::HaKey) => {
                    self.go_to(Screen::Settings(Page::Grid), Sweep::LeftToRight)
                }
                (Gesture::Tap | Gesture::LongPress, Screen::Buttons) => self.on_grid(touch, now),
                (Gesture::SwipeLeft, _) => self.navigate(nav::Swipe::Left),
                (Gesture::SwipeRight, _) => self.navigate(nav::Swipe::Right),
                (Gesture::SwipeUp, _) => self.navigate(nav::Swipe::Up),
                (Gesture::SwipeDown, _) => self.navigate(nav::Swipe::Down),
                (Gesture::Tap, Screen::Music)
                    if let Some(command) = self.theme().music_control(touch.x, touch.y) =>
                {
                    send(Outgoing::Music(command));
                    if let Some(music) = &mut self.music {
                        match command {
                            MusicCommand::PlayPause => music.toggle(),
                            MusicCommand::Next | MusicCommand::Previous => music.likely_playing(),
                        }
                    }
                }
                (Gesture::Tap, Screen::Alarms) => {
                    if let Some(row) = self.theme().alarm_at(touch.x, touch.y) {
                        self.editing = Some(row);
                    }
                }
                // A quick switch, without opening the editor.
                (Gesture::LongPress, Screen::Alarms) => {
                    if let Some(row) = self.theme().alarm_at(touch.x, touch.y) {
                        self.toggle(row, now);
                        motor::buzz(Pattern::NOTIFY);
                    }
                }
                (Gesture::Tap, Screen::Settings(page)) => {
                    let settings = page.settings(self.buttons);
                    let rows = self.settings_rows(settings);
                    let shown = themes::SettingsPage {
                        title: page.title(),
                        rows: &rows,
                    };
                    if let Some(row) = self.theme().settings_row(&shown, touch.x, touch.y)
                        && let Some(&setting) = settings.get(row)
                    {
                        self.change(setting, now);
                    }
                }
                // TODO: remove once something real uses the motor; for now
                // this is how we check it works.
                (Gesture::Tap, Screen::Info) => motor::buzz(Pattern::NOTIFY),
                // A media gesture begun with the screen off or on a glance,
                // whose first tap woke the screen (see `woken`, above).
                (_, Screen::Clock) if !notification_shown && woken.is_some() => {
                    if let Some(command) = woken {
                        self.media(command);
                    }
                }
                // Media, from the clock face, while a phone is connected
                // (see `media`): the screen off's gestures, so the wearer
                // needn't know whether the screen is on to use them. A
                // double tap plays or pauses, and a tap, then a hold (a
                // tap-and-hold), skips, each second touch within the
                // screen off's window (see `follows`). A single tap does
                // nothing, so it needn't wait to see whether a second
                // follows; nor does a long press on its own.
                //
                // Unlike with the screen off or on a glance, both anywhere:
                // something woke the screen to get here, so they're
                // likelier meant. Bar the hold just after a touch woke it,
                // which may have been a folded arm, still resting there:
                // then only in the middle, as on the glance (see
                // `wake::awake_long_press`).
                (Gesture::LongPress, Screen::Clock) if !notification_shown => {
                    let touch_woke = now < self.centre_until;
                    if first_tap.is_some() && wake::awake_long_press(touch.x, touch.y, touch_woke) {
                        self.media(MusicCommand::Next)
                    }
                }
                (Gesture::Tap, Screen::Clock) if !notification_shown => match first_tap {
                    Some(_) => self.media(MusicCommand::PlayPause),
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
        let layout = self.grid_layout;
        let button = self.theme().grid_button(layout, touch.x, touch.y);
        for press in self
            .grid
            .touch(layout, button, kind, started, now)
            .into_iter()
            .flatten()
        {
            self.send_press(press, now);
        }
    }

    /// When [`App::on_timer`] next has something to do: the grid's wait
    /// running out, or its last press to stop showing. While a finger is
    /// down (`finger_down`), only a hold becoming a long press, or what
    /// it sent to stop showing (the held button then shows pressed):
    /// anything else waits for that touch, whose event wakes the loop.
    ///
    /// The hold's only while the grid's still showing: left with the
    /// finger down (the button pressed meanwhile, say), `on_timer` won't
    /// make it a long press, so its time would stay due, and the loop would
    /// spin until the finger lifted. The hold itself stays, for its
    /// release. (What was sent is no such worry: `on_timer` forgets it
    /// once its time is up, wherever the screen is.)
    ///
    /// Or, on a glance, a deliberate tap's wake, once no second touch can
    /// follow it (see [`App::held_wake`]): not while a finger is down,
    /// which may be that second touch, and whose event decides.
    pub fn deadline(&self, finger_down: bool) -> Option<Instant> {
        let shown = self.sent.map(|(_, until, _)| until);
        if finger_down {
            let on_grid = self.awake && self.screen == Screen::Buttons;
            let long = self.grid.long_at().filter(|_| on_grid);
            return long.into_iter().chain(shown).min();
        }
        let held_wake = self.held_wake();
        self.grid
            .deadline()
            .into_iter()
            .chain(shown)
            .chain(held_wake)
            .min()
    }

    /// The UI loop woke for [`App::deadline`] (or near it): sends what the
    /// grid was waiting on, if it's waited long enough, unless a finger is
    /// down (`finger_down`; see [`buttons::Grid::expire`]); or a hold's
    /// long press, if the finger still is. Or wakes the screen from a
    /// glance for a deliberate tap (see [`App::held_wake`]).
    pub fn on_timer(&mut self, now: Instant, finger_down: bool) {
        if self.sent.is_some_and(|(_, until, _)| now >= until) {
            self.sent = None;
        }
        self.wake_held(now, finger_down);
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

    /// When a deliberate tap on a glance, whose wake waits to see whether
    /// a second touch follows it ([`wake::Action::WakeLater`]), may wake
    /// the screen, if one waits: once a second touch would no longer
    /// follow it (see [`wake::wake_later_due`]). Half a second, for a
    /// lone tap: the wake comes that much later than it did from a
    /// glance before, the price of a double tap or a tap-and-hold
    /// leaving the glance be unless its first tap is heavy (see
    /// `on_input`).
    fn held_wake(&self) -> Option<Instant> {
        let tap = self
            .last_tap
            .filter(|_| self.glance_wake && self.glancing())?;
        Some(tap + Duration::from_millis(u64::from(wake::DOUBLE_TAP_MS) + 1))
    }

    /// Wakes the screen from a glance, for a deliberate tap whose wake
    /// waits (see [`App::held_wake`]), if no second touch can now follow
    /// it, `finger_down` saying whether a finger is on the screen.
    fn wake_held(&mut self, now: Instant, finger_down: bool) {
        let Some(tap) = self
            .last_tap
            .filter(|_| self.glance_wake && self.glancing())
        else {
            return;
        };
        let since = now.checked_duration_since(tap).unwrap_or_default();
        let since_ms = u32::try_from(since.as_millis()).unwrap_or(u32::MAX);
        if !wake::wake_later_due(since_ms, finger_down) {
            return;
        }
        // For telling this wake from the tap's in a field log: it comes
        // half a second or more after the tap's input line.
        defmt::info!("glance: woken by a tap");
        // As the tap would have woken it at once (see `on_input`), its
        // time for a hold to be in the middle counting from it. It stays
        // the first of a pair finished on the clock face, as a tap that
        // wakes the screen from off is: no second can follow it by now,
        // but one whose event was on its way, its finger already up,
        // still counts.
        self.glance_until = None;
        self.glance_wake = false;
        self.last_input = now;
        self.centre_until = tap + CENTRE_AFTER_TOUCH_WAKE;
        self.woke_tap = true;
        self.last_tap = Some(tap);
    }

    /// Sends a grid press wherever the setting says, and shows it went.
    /// To the phone, it's sent from here, as media commands are; for Home
    /// Assistant, `main` picks it up (see [`App::take_ha_press`]).
    fn send_press(&mut self, press: Press, now: Instant) {
        defmt::info!("buttons: {=u8} {=str}", press.code, press.gesture.name());
        let to_phone = self.buttons.app() && self.connected;
        if to_phone {
            send(Outgoing::Button {
                code: press.code,
                gesture: press.gesture.name(),
            });
        }
        if self.buttons.bthome() {
            let queued = self.ha_presses.push(press);
            // There's room: one wake completes two presses at most (a
            // touch: see `buttons::Grid::touch`; a lift or a timer, one),
            // and the loop sends them all before the next wake (see
            // `ui_loop::UiLoop::handle`). A press dropped here would be
            // lost without a word, and if it's a hold's release, whatever
            // the hold started would run on.
            debug_assert!(queued.is_ok(), "no room for a grid press");
        }
        // A hold's end: the screen keeps showing the hold, which already
        // buzzed.
        if press.gesture == buttons::Gesture::Release {
            return;
        }
        // Somewhere it could go. (Home Assistant can't say whether it
        // heard; Gadgetbridge only says if it didn't pass it on. The
        // watch can say it couldn't send it at all, once it's tried: see
        // `ha_unsent`.)
        let sent = to_phone || self.buttons.bthome();
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

    /// A press from [`App::take_ha_press`] couldn't go out (see
    /// `bthome::send`; nearly always both Bluetooth connections in use):
    /// as a press that went nowhere, its button turns the "bad" colour,
    /// and it buzzes so; and a banner points at the README's FAQ, as
    /// "INTENTS OFF" does for the app.
    ///
    /// Even with BUTTONS on BOTH, where the phone may have had it: the
    /// phone is usually one of the two connections, so a grid that only
    /// went red with nowhere to go would never show this, and Home
    /// Assistant's automations would silently miss it. (A phone that's
    /// away is another matter: the status bar shows it, and there's
    /// nothing to fix.)
    pub fn ha_unsent(&mut self, press: Press, now: Instant) {
        // A hold's end shows nothing of its own (see `send_press`); its
        // start's colour has likely gone by now. The banner says it.
        if press.gesture != buttons::Gesture::Release {
            self.sent = Some((press, now + SENT_TIME, false));
        }
        self.show_banner(Shows::Unsent, now);
        motor::buzz(Pattern::TURN);
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
        // What the watch sent, for telling from a field log whether a skip
        // the phone ignored left the watch at all (or found the outbox
        // full: `send` drops rather than waits).
        let sent = ble::OUTBOX.try_send(Outgoing::Music(command)).is_ok();
        defmt::info!(
            "media: {} {}",
            command,
            if sent { "sent" } else { "dropped, outbox full" }
        );
        let action = match (command, &mut self.music) {
            // As the music screen does, the new state straight away, if
            // it's known.
            (MusicCommand::PlayPause, Some(music)) if music.playing.is_some() => {
                music.toggle();
                if music.playing() {
                    MediaAction::Play
                } else {
                    MediaAction::Pause
                }
            }
            // Otherwise, not a guess: the toggle itself. The phone says
            // which it was once its player changes.
            (MusicCommand::PlayPause, _) => MediaAction::PlayPause,
            (MusicCommand::Next, music) => {
                if let Some(music) = music {
                    music.likely_playing();
                }
                MediaAction::Next
            }
            (MusicCommand::Previous, music) => {
                if let Some(music) = music {
                    music.likely_playing();
                }
                MediaAction::Previous
            }
        };
        self.media_hint = Some((action, MEDIA_HINT_TICKS));
        motor::buzz(Pattern::NOTIFY);
    }

    /// Switches a row's alarm on or off; for the timer, starts or stops it.
    fn toggle(&mut self, row: Editing, now: Instant) {
        match row {
            Editing::SmartWeekday | Editing::SmartWeekend => self.alarms.smart = !self.alarms.smart,
            Editing::At(i) => self.alarms.at[usize::from(i)] ^= true,
            Editing::Timer(i) => {
                let i = usize::from(i);
                // A timer counts uptime, not the clock (see `alarm`), so
                // it runs before the phone has set the time too.
                match self.alarm_state.timers[i] {
                    Some(_) => self.alarm_state.stop_timer(i),
                    None => self
                        .alarm_state
                        .start_timer(i, &self.alarms, now.as_secs() as i64),
                }
            }
        }
    }

    fn on_editor_input(&mut self, editing: Editing, event: InputEvent, now: Instant) {
        let done = |app: &mut Self| app.editing = None;
        let InputEvent::Touch(touch) = event else {
            if let InputEvent::Button = event {
                done(self);
            }
            return;
        };
        let theme = self.theme();
        let up = match touch.gesture {
            Gesture::SwipeUp => 1,
            Gesture::SwipeDown => -1,
            Gesture::Tap => {
                match theme.editor_button(touch.x, touch.y) {
                    Some(EditorButton::Switch) => self.toggle(editing, now),
                    Some(EditorButton::Next) if editing == Editing::SmartWeekday => {
                        self.editing = Some(Editing::SmartWeekend)
                    }
                    Some(EditorButton::Next) => done(self),
                    None => {}
                }
                return;
            }
            _ => return,
        };
        // Hours, or minutes: 5 at a time, but 1 for the timer, where short
        // durations are common.
        let step = match (theme.editor_field(touch.x, touch.y), editing) {
            (EditorField::Hours, _) => 60,
            (EditorField::Minutes, Editing::Timer(_)) => 1,
            (EditorField::Minutes, _) => 5,
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

    /// Turns the screen off, closing the alarm editor if it's open. The
    /// editor is the one screen that isn't `screen` (it opens over Alarms),
    /// so the wake to the clock face that every wake is wouldn't close it:
    /// left open, it came back over the clock face (which `showing_clock`
    /// then took it for), and a banner waiting to be seen under it never
    /// started its time. Nothing is lost by closing it: each edit takes
    /// effect, and is saved, as it's made, so this is what DONE does, and
    /// what the other screens that change settings (the settings, the grid's
    /// layout) amount to when a wake takes the wearer off them.
    fn screen_off(&mut self) {
        self.awake = false;
        self.editing = None;
        // A glance's deliberate tap is moot (see `held_wake`), and the
        // next glance starts afresh.
        self.glance_wake = false;
    }

    fn stop_ringing(&mut self) {
        self.ringing = None;
        self.buzz_for_screen();
    }

    /// Stops looking for the watch, and its buzz, if it had the screen;
    /// under a call or a ringing alarm, those keep the motor (and the
    /// screen), and an alarm keeps its place in its ramp. Returns whether
    /// it had the screen.
    fn stop_finding(&mut self) -> bool {
        self.finding = None;
        let had_screen = self.call.is_none() && self.ringing.is_none();
        if had_screen {
            motor::buzz(Pattern::STOP);
        }
        had_screen
    }

    /// Whether a call, a ringing alarm, or the phone looking for the watch
    /// has the screen, and the motor: a notification's buzz would cut
    /// their ringing short.
    fn ringing_any(&self) -> bool {
        self.call.is_some() || self.ringing.is_some() || self.finding.is_some()
    }

    /// Buzzes for whatever rings on screen now, as `render` stacks them
    /// (a call, then a ringing alarm, then the phone looking for the
    /// watch), or stops the motor if nothing does. For when one of them
    /// starts or ends: the motor plays one pattern at a time, so ending a
    /// call with [`Pattern::STOP`] would silence an alarm still ringing
    /// beneath it. (That alarm starts its gentle ramp over: simpler than
    /// keeping its place, and it's only been interrupted.)
    fn buzz_for_screen(&self) {
        motor::buzz(match (&self.call, &self.ringing) {
            (Some(call), _) if call.silenced => Pattern::STOP,
            (Some(_), _) => Pattern::RING,
            (None, Some(ringing)) => match ringing.kind {
                alarm::Kind::Timer(_) => Pattern::TIMER,
                _ => Pattern::ALARM,
            },
            (None, None) if self.finding.is_some() => Pattern::FIND,
            (None, None) => Pattern::STOP,
        });
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
            // Not every time, though: the Info screen's phone row says so
            // meanwhile, for as long as the client stays.
            PhoneEvent::Unpaired(why) => {
                if self.unpaired_banners.due(why as usize, now.as_millis()) {
                    self.show_banner(Shows::Unpaired(why), now);
                }
                return;
            }
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
                    self.buzz_for_screen();
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
            // "Find my watch" (Gadgetbridge's "Find lost device"): the
            // watch takes over the screen, and buzzes as hard as it can,
            // until it's found (see `on_input`), the phone says stop, or
            // `MAX_FIND`. Even in the quiet night (in its dim look, as
            // calls ring): someone awake asked for it on purpose, and a
            // watch lying still off the wrist may well look asleep. It
            // doesn't end the night, though: whoever looked will likely
            // go back to sleep. Under a call or a ringing alarm, it waits
            // its turn on the screen and the motor: the watch is ringing
            // already. Asked again, it starts its time over.
            PhoneEvent::Find { on: true } => {
                defmt::info!("find: looking for the watch");
                self.finding = Some(now);
                self.awake = true;
                self.brief_wake = false;
                self.last_input = now;
                if self.call.is_none() && self.ringing.is_none() {
                    motor::buzz(Pattern::FIND);
                }
            }
            // Only if it's still looking: a stray stop mustn't silence a
            // call or an alarm.
            PhoneEvent::Find { on: false } => {
                if self.finding.is_some() {
                    defmt::info!("find: stopped by the phone");
                    self.stop_finding();
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
        if self.night.quiet() || self.ringing_any() {
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
            && !self.ringing_any()
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
        // Unless the screen is on only for the banner it replaces (which
        // came before the night started): then it's on screen already, and
        // its time starts now, as the wake's end is the banner's. Left to
        // `on_tick`, anything that kept it from starting there (as the
        // alarm editor once could) would hold the screen on all night.
        if self.night.quiet() {
            let until = if self.brief_wake {
                now + BANNER_TIME
            } else {
                UNSEEN
            };
            self.banner = Some(Banner { shows, until });
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
        // Not over a call, a ringing alarm, or the phone looking for the
        // watch, which would go quiet.
        if !self.ringing_any() {
            motor::buzz(Pattern::NOTIFY);
        }
        if !self.awake {
            self.awake = true;
            self.brief_wake = true;
            self.screen = Screen::Clock;
        }
    }

    /// Once a second: starts or ends the quiet night, and rings any alarm
    /// that's due. `local_second` is the time, if known; `wearer` whether
    /// they just stirred, in light sleep (for smart wake); `asleep` whether
    /// they're asleep (see [`crate::activity::asleep`]).
    pub fn check_alarms(
        &mut self,
        now: Instant,
        local_second: Option<i64>,
        wearer: alarm::Wearer,
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
        if let Some(ringing) = &self.ringing
            && now - ringing.since >= Duration::from_secs(alarm::RING_FOR as u64 * 60)
        {
            defmt::info!("alarm: rang out ({})", ringing.kind);
            self.stop_ringing();
        }
        // Durations first, as `alarm::State::due` did before they moved
        // to uptime: they don't need the time of day, which alarms at a
        // time of day do.
        if self.ringing.is_none()
            && let Some(kind) = self.alarm_state.elapsed(now.as_secs() as i64).or_else(|| {
                let second = local_second?;
                let due = self.alarm_state.due(
                    &mut self.alarms,
                    &mut self.phone_alarms,
                    second,
                    wearer,
                )?;
                // A one-off phone alarm turned itself off, so it doesn't
                // ring again after a reset: save them. Whichever alarm
                // rang, not only one of the phone's: the watch's own, at
                // the same minute, rings in their place.
                self.phone_alarms_changed |= due.phone_rang;
                // For the field log: which morning's time, and why now.
                if due.kind == alarm::Kind::Smart
                    && let Some(latest) = self.alarm_state.smart_rang()
                {
                    let of_day = latest.rem_euclid(24 * 60);
                    let (hh, mm) = ((of_day / 60) as u8, (of_day % 60) as u8);
                    if due.early {
                        defmt::info!(
                            "alarm: smart, for {=u8:02}:{=u8:02}, early: a stir in light sleep",
                            hh,
                            mm
                        );
                    } else {
                        defmt::info!("alarm: smart, for {=u8:02}:{=u8:02}, at its time", hh, mm);
                    }
                }
                Some(due.kind)
            })
        {
            defmt::info!("alarm: ringing ({})", kind);
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
            // Under a call, the call keeps the motor until it's over.
            self.buzz_for_screen();
            self.awake = true;
            self.brief_wake = false;
            self.last_input = now;
        }
    }

    /// `keep_awake` holds the screen on regardless of input, e.g. while a
    /// pairing passkey is showing and the user is typing it on the phone.
    /// `finger_down`: whether a finger is on the screen
    /// (`touch::finger_down`), for a hold (see `screen_timeout`).
    pub fn on_tick(&mut self, now: Instant, keep_awake: bool, finger_down: bool) {
        // That's pairing or an update, which take every touch too.
        self.takeover = keep_awake;
        // A hold whose finger has lifted is over, as at its release
        // (see `on_input`), which may not have come yet: the touch task
        // says the finger's up a moment before it sends that. Or never
        // will: a long press injected for testing (`debug`, the
        // simulator's `long`) has no finger, nor release. With the
        // screen off (past `HOLD_LIMIT`, below, or by the button), a
        // hold keeps it on no more either: woken again with the finger
        // still down, it goes off as after any wake. Its release still
        // goes, and does what a release does.
        if self.holding && !(self.awake && finger_down) {
            self.holding = false;
            self.last_input = now;
        }
        self.media_hint = match self.media_hint {
            Some((action, ticks)) if ticks > 1 => Some((action, ticks - 1)),
            _ => None,
        };
        if Route::expire(&mut self.route, now.as_millis()) {
            defmt::info!("route: no update for a while; dropped");
        }
        // A deliberate tap on a glance whose wake waits: normally the
        // timer's (see `deadline`), but the tick's too, should it come
        // first.
        self.wake_held(now, finger_down);
        // A glance has its own timer: it isn't input, so the screen timeout
        // (which counts from the last input) would end it at once. Not
        // while a deliberate tap's wake waits, though: that may wake the
        // screen yet, the tap having been meant to, or the tap be a media
        // gesture's first; it's never more than a moment past the
        // glance's time, bar a finger held on, whose touch decides.
        if let Some(until) = self.glance_until {
            if now >= until && self.held_wake().is_none() {
                self.glance_until = None;
                self.screen_off();
            }
            return;
        }
        // A banner waiting to be seen (see `UNSEEN`) starts its time once
        // it's on screen: awake, and not covered by something that takes
        // over the screen.
        if self.awake
            && !self.ringing_any()
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
                self.screen_off();
                self.brief_wake = false;
            }
        }
        if self.call.as_ref().is_some_and(|c| now >= c.until) {
            self.call = None;
            self.buzz_for_screen();
        }
        if self.finding.is_some_and(|since| now - since >= MAX_FIND) {
            defmt::info!("find: gave up");
            self.stop_finding();
        }
        if keep_awake || self.ringing_any() || self.brief_wake {
            self.last_input = now;
        }
        // Not while a banner shows: one that arrived late in the timeout
        // would otherwise go before it could be read. (It doesn't count as
        // input, so the screen goes off once it's gone, unless input came
        // in the meantime.) One still waiting to be seen (under the alarm
        // editor, say) isn't showing.
        let banner_showing = self.banner.as_ref().is_some_and(|b| b.until != UNSEEN);
        if self.awake && !banner_showing && now - self.last_input >= self.screen_timeout() {
            self.screen_off();
        }
    }

    /// How long the screen stays on after the last input: [`SCREEN_TIMEOUT`],
    /// or on the music screen, long enough for the title and artist to
    /// scroll by once, from wherever they are. They scroll from when the
    /// track started (see [`Theme::music`]), not from the last input, so a
    /// whole cycle shows every part of them. That's ~19 s for a 30-letter
    /// title, against the clock face's 10 s; the music screen is opened
    /// rarely enough that the extra time costs little.
    ///
    /// Home Assistant's key, as a QR code or as text, stays until a tap
    /// (or a swipe, or the button) takes it away: scanning it, or typing
    /// 32 digits from it, takes the phone, Home Assistant's pages, and
    /// often longer than any timeout would allow. It's shown rarely, and
    /// only on purpose, so the backlight it keeps on costs little.
    ///
    /// TODO: maybe a safety cap (minutes) for a watch put down with the
    /// key showing; the user asked for none.
    ///
    /// And whatever the screen, a long press held on keeps it on until
    /// its release, up to [`crate::gesture::HOLD_LIMIT`] (see
    /// [`crate::gesture::screen_timeout`]).
    fn screen_timeout(&self) -> Duration {
        let timeout = match (self.screen, &self.music) {
            (Screen::Music, Some(music)) => {
                let overflow = self.theme().music_overflow(&music.artist, &music.track);
                SCREEN_TIMEOUT.max(marquee_cycle(overflow))
            }
            (Screen::HaQr | Screen::HaKey, _) => Duration::MAX,
            _ => SCREEN_TIMEOUT,
        };
        crate::gesture::screen_timeout(timeout, self.holding)
    }

    /// Whether something on screen is scrolling, so frames are needed more
    /// often than once a second (see [`crate::canvas::MARQUEE_FRAME`]).
    pub fn animating(&self) -> bool {
        if !self.awake {
            return false;
        }
        let theme = self.theme();
        if let Some(call) = &self.call {
            return theme.call_overflow(&call.caller) > 0;
        }
        // Notifications never scroll: what doesn't fit ends in an
        // ellipsis (see retro's `messages`).
        match self.screen {
            Screen::Music => self
                .music
                .as_ref()
                .is_some_and(|m| theme.music_overflow(&m.artist, &m.track) > 0),
            _ => false,
        }
    }

    /// Draws the screen, with the chosen scheme's theme (see
    /// [`crate::themes`]).
    pub fn render(&self, canvas: &mut Canvas, status: &Status, now: Instant) {
        let theme = self.theme();
        // Updates, pairing, then calls take over the screen until they're
        // done.
        if let Some(percent) = status.update_progress {
            theme.update(canvas, percent, now);
            return;
        }
        if let Some(passkey) = status.passkey {
            theme.pairing(canvas, passkey);
            return;
        }
        if let Some(call) = &self.call {
            let call = themes::Call {
                caller: &call.caller,
                elapsed: now - call.since,
                silenced: call.silenced,
            };
            theme.call(canvas, &call);
            return;
        }
        if let Some(ringing) = &self.ringing {
            theme.ringing(canvas, ringing.kind, status);
            return;
        }
        if let Some(since) = self.finding {
            theme.find(canvas, now - since);
            return;
        }
        if self.glancing() {
            let glance = themes::Glance {
                status,
                route: self.route.as_ref().map(|route| &route.step),
                media: self.media_hint.map(|(action, _)| action),
            };
            theme.glance(canvas, &glance);
            return;
        }
        // Text kept as Latin-1 (see `phone::Text`), as UTF-8 to draw.
        let (mut app, mut who) = (AppText::new(), WhoText::new());
        if self.quiet_notification()
            && let Some(banner) = &self.banner
        {
            // Without the time now: the screen is the notification's
            // alone. A tap wakes it to the clock, with the notification as
            // a banner, for the time.
            self.banner_text(banner.shows, &mut app, &mut who);
            theme.notification(canvas, &app, &who, "");
            return;
        }
        if let Some(editing) = self.editing {
            let editor = themes::Editor {
                editing,
                settings: &self.alarms,
                state: &self.alarm_state,
                uptime_secs: status.uptime_secs as i64,
            };
            theme.editor(canvas, &editor);
            return;
        }
        let next_alarm = status.local_second.and_then(|second| {
            self.alarm_state.next(
                &self.alarms,
                &self.phone_alarms,
                second,
                status.uptime_secs as i64,
            )
        });
        // The clock face, there or in place of a page with nothing to show.
        let clock = |canvas: &mut Canvas, media| {
            let face = themes::Face {
                status,
                next_alarm,
                media,
                unseen: self.notifications.unseen,
                dim: self.dim(),
            };
            theme.clock(canvas, &face);
        };
        match self.screen {
            Screen::Clock => {
                // While media plays, a note; just after a media gesture,
                // what it did.
                let media = match (self.media_hint, &self.music) {
                    (Some((action, _)), _) => Some(themes::Media::Did(action)),
                    (None, Some(music)) if music.playing() => Some(themes::Media::Playing),
                    _ => None,
                };
                clock(canvas, media);
            }
            // From the day's history, only while it shows: walking it
            // costs a little time, while keeping it up to date would cost
            // RAM.
            Screen::Today => theme.today(canvas, crate::activity::today().as_ref(), status.steps),
            Screen::LastNight => theme.last_night(canvas, status.sleep),
            Screen::Alarms => theme.alarms(canvas, &self.alarms, &self.alarm_state, status),
            Screen::PhoneAlarms => theme.phone_alarms(canvas, &self.phone_alarms),
            // As a notification that arrived with the screen off shows,
            // with when it arrived in its corner, and the handles.
            Screen::Notification(i) => match self.notifications.list.get(usize::from(i)) {
                Some(n) => {
                    let mut arrived = String::<{ arrival::MAX_LEN }>::new();
                    if let (Some(at), Some(second)) = (n.arrived, status.local_second) {
                        // A minute that fits in 32 bits, as the arrival
                        // does: the phone only sets times from 2020 to
                        // 2100 (`gb::parse_set_time`). `arrival::MAX_LEN`
                        // is the longest label between two of those.
                        debug_assert!(u32::try_from(second.div_euclid(60)).is_ok());
                        arrival::write(&mut arrived, at.get().into(), second.div_euclid(60)).fits();
                    }
                    n.app.write_to(&mut app);
                    n.who.write_to(&mut who);
                    theme.notification(canvas, &app, &who, &arrived);
                }
                None => clock(canvas, None),
            },
            Screen::Music => match &self.music {
                Some(music) => {
                    let music = themes::Music {
                        artist: &music.artist,
                        track: &music.track,
                        playing: music.playing,
                        elapsed: now - music.since,
                    };
                    theme.music(canvas, &music);
                }
                None => clock(canvas, None),
            },
            Screen::Air => match &status.air {
                Some(sighting) => theme.air(canvas, sighting, now),
                None => clock(canvas, None),
            },
            Screen::Buttons => {
                let sent = self.sent.filter(|&(_, until, _)| now < until);
                let grid = themes::Grid {
                    layout: self.grid_layout,
                    grid: &self.grid,
                    sent: sent.map(|(press, _, ok)| (press, ok)),
                    dim: self.dim(),
                };
                theme.buttons(canvas, &grid);
            }
            Screen::Settings(page) => {
                let rows = self.settings_rows(page.settings(self.buttons));
                let shown = themes::SettingsPage {
                    title: page.title(),
                    rows: &rows,
                };
                theme.settings(canvas, &shown);
            }
            Screen::Info => theme.info(canvas, status),
            Screen::Storage => theme.storage_prompt(canvas),
            Screen::HaQr => theme.bthome_qr(canvas, status.bthome.as_ref()),
            Screen::HaKey => theme.bthome_key(canvas, status.bthome.as_ref()),
        }
        match self.place() {
            // Notifications nobody has seen. The clock face has no other
            // hints: its edges are full, and it's where the wearer learns
            // the layout from.
            Place::Clock if self.notifications.unseen => theme.handles(canvas, Handles::Unseen),
            Place::Line { side, index }
                if !matches!(self.screen, Screen::Storage | Screen::HaQr | Screen::HaKey) =>
            {
                let count = self.line(side).len();
                theme.handles(canvas, Handles::Line { side, index, count });
            }
            _ => {}
        }
        if let Some(banner) = &self.banner {
            self.banner_text(banner.shows, &mut app, &mut who);
            theme.banner(canvas, &app, &who);
        }
    }

    /// What shows for the few seconds before the watch switches itself
    /// off (see `ui_loop`), drawn by the chosen scheme in whatever palette
    /// the canvas is in.
    pub fn render_off(&self, canvas: &mut Canvas, reason: crate::system_off::Reason) {
        self.theme().off(canvas, reason);
    }
}

/// Queues a message for the phone (dropped if not connected or the queue is
/// full; none of these are worth retrying later).
fn send(message: Outgoing) {
    let _ = ble::OUTBOX.try_send(message);
}

/// Whether `touch`, reported `now`, comes soon enough after a tap that
/// lifted at `tap` to be the second of a pair (see [`wake::follows`]).
fn follows(tap: Instant, touch: crate::touch::Touch, now: Instant) -> bool {
    now.checked_duration_since(tap).is_some_and(|since| {
        let since_ms = u32::try_from(since.as_millis()).unwrap_or(u32::MAX);
        wake::follows(wake_kind(touch.gesture), since_ms, touch.held_ms)
    })
}

/// A touch's kind, as the screen off and a glance see it (see [`wake`]).
fn wake_kind(gesture: Gesture) -> wake::Kind {
    match gesture {
        Gesture::Tap => wake::Kind::Tap,
        Gesture::LongPress => wake::Kind::LongPress,
        _ => wake::Kind::Swipe,
    }
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

// ==============================================================================
// Text that fits
// ==============================================================================

/// For the result of writing into a fixed-size [`String`], which the sizes
/// chosen rule out overflowing: panics if it did anyway.
///
/// One function for every such write, not an `expect` at each: each of
/// those kept its own panic location and message, ~30 bytes of flash
/// apiece. A panic here still records which write it was, in the
/// breadcrumb's backtrace (see `field_log`).
pub(crate) trait Fits {
    fn fits(self);
}

impl<E> Fits for Result<(), E> {
    fn fits(self) {
        fits(self.is_ok());
    }
}

#[inline(never)]
fn fits(ok: bool) {
    assert!(ok, "text fits its buffer");
}

// ==============================================================================
// The settings' rows
// ==============================================================================
//
// Which settings there are, on which screen, and what each says, is the
// UI's; how a row looks, where it is, and so which a tap is on, the
// theme's (see `Theme::settings`). Every screen keeps each setting in its
// place: one that can't change in the build (THEME with one scheme for
// the side, FORCE with schemes of one side) is still there, saying so
// at a tap, rather than leaving a gap or moving the rows below it up.

/// The settings screens, side by side on the right-hand line.
#[derive(Clone, Copy, PartialEq, Eq, defmt::Format)]
enum Page {
    /// The look's: THEME, FORCE, LIGHT.
    Look,
    /// The button grid's: BUTTONS, GRID, BTHOME KEY.
    Grid,
}

impl Page {
    fn title(self) -> &'static str {
        match self {
            Page::Look => "LOOK",
            // Not "BUTTONS", which would only repeat its first row's label.
            Page::Grid => "BUTTON GRID",
        }
    }

    /// Its settings, top to bottom: BTHome's key only while the grid's
    /// presses go over BTHome, as it's of no use otherwise. It's last, so
    /// it comes and goes (at a tap on BUTTONS) without moving another row.
    fn settings(self, buttons: buttons::Setting) -> &'static [Setting] {
        const LOOK: [Setting; 3] = [Setting::Theme, Setting::Force, Setting::Light];
        const GRID: [Setting; 3] = [Setting::Buttons, Setting::Layout, Setting::BtHomeKey];
        match self {
            Page::Look => &LOOK,
            Page::Grid if buttons.bthome() => &GRID,
            Page::Grid => &GRID[..2],
        }
    }
}

/// One setting: a row on a settings screen.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Setting {
    Theme,
    Force,
    Light,
    Buttons,
    /// GRID: the grid's layout.
    Layout,
    BtHomeKey,
}

/// Which side of the day's schemes the theme shows (the settings' FORCE): as
/// the sun says, or always the day's or the night's.
#[derive(Clone, Copy, PartialEq, Eq, Default, defmt::Format)]
pub enum Force {
    #[default]
    Sun,
    Day,
    Night,
}

impl Force {
    /// As Settings shows it.
    pub fn name(self) -> &'static str {
        match self {
            Force::Sun => "NONE",
            Force::Day => "DAY",
            Force::Night => "NIGHT",
        }
    }

    /// What a tap on FORCE makes it.
    pub fn next(self) -> Self {
        match self {
            Force::Sun => Force::Day,
            Force::Day => Force::Night,
            Force::Night => Force::Sun,
        }
    }

    pub fn to_byte(self) -> u8 {
        self as u8
    }

    /// As stored; anything else, as nothing stored: the sun's.
    pub fn from_byte(byte: u8) -> Self {
        match byte {
            1 => Force::Day,
            2 => Force::Night,
            _ => Force::Sun,
        }
    }
}

impl App {
    /// `settings`' rows, in words, for the theme to draw.
    fn settings_rows(&self, settings: &[Setting]) -> heapless::Vec<SettingRow, 3> {
        settings
            .iter()
            .map(|&setting| self.setting_row(setting))
            .collect()
    }

    fn setting_row(&self, setting: Setting) -> SettingRow {
        let (label, value) = match setting {
            Setting::Theme => ("THEME", self.theme().palette().name),
            Setting::Force => ("FORCE", self.force().name()),
            Setting::Light => ("LIGHT", self.brightness.name()),
            Setting::Buttons => ("BUTTONS", self.buttons.name()),
            Setting::Layout => ("GRID", self.grid_layout.name()),
            Setting::BtHomeKey => ("BTHOME KEY", "SHOW"),
        };
        let kind = match setting {
            // Not a value, but somewhere to go.
            Setting::BtHomeKey => RowKind::Link,
            _ if self.changeable(setting) => RowKind::Choice,
            _ => RowKind::Fixed,
        };
        // Two layers reach Home Assistant unlike six buttons do (as dimmers
        // turned by the code): FAQ B04 says how, and how to automate them.
        // The asterisk, as on a footnote, says it's a caveat to the value
        // beside it, not a value of its own.
        let note = (setting == Setting::Layout
            && self.buttons.bthome()
            && self.grid_layout == buttons::Layout::TwoLayers)
            .then_some("*B04");
        SettingRow {
            label,
            value,
            kind,
            note,
        }
    }

    /// Whether a tap changes `setting`, in this build: THEME needs another
    /// scheme for the side showing, FORCE schemes of both sides to pick
    /// between.
    fn changeable(&self, setting: Setting) -> bool {
        match setting {
            Setting::Theme => themes::count(self.theme_side()) > 1,
            Setting::Force => themes::has(true) && themes::has(false),
            _ => true,
        }
    }

    /// A tap on `setting`'s row: its next value, or for one that can't
    /// change, a banner saying why, and where the README says how to get a
    /// build where it can.
    fn change(&mut self, setting: Setting, now: Instant) {
        if !self.changeable(setting) {
            if setting == Setting::Theme {
                defmt::info!("ui: one scheme for this side, in this build (FAQ D01)");
                self.show_banner(Shows::OneTheme, now);
            } else {
                defmt::info!("ui: schemes for one side only, in this build (FAQ D02)");
                self.show_banner(Shows::OneSide, now);
            }
            return;
        }
        match setting {
            Setting::Theme => self.next_theme(),
            Setting::Force => self.force = self.force.next(),
            Setting::Light => self.brightness = self.brightness.next(),
            Setting::Buttons => self.buttons = self.buttons.next(),
            Setting::Layout => self.grid_layout = self.grid_layout.next(),
            Setting::BtHomeKey => self.go_to(Screen::HaQr, Sweep::RightToLeft),
        }
    }
}

/// A notification's app, and who it's from, as UTF-8 to draw: room for
/// what [`Text`] keeps of each (see [`Text::write_to`]).
type AppText = String<{ 2 * 24 + 1 }>;
type WhoText = String<{ 2 * 40 + 1 }>;
