//! Themes: how every screen looks, and where its buttons are.
//!
//! A theme is a type that draws every screen and says where a touch on
//! it lands ([`Theme`], with no default methods). Its colour schemes,
//! what the settings' THEME row picks between, are statics of that type:
//! retro ([`retro`], the watch's own look) is one type, and ARCADE,
//! TERMINAL, HANDHELD and PARCHMENT are its schemes, each with a palette
//! of its own. A theme with one look has one scheme (or one a side). The
//! only things a scheme must have are a palette and a dim one (see
//! [`Theme::palette`]): what the 16 colour roles mean, and how a scheme
//! differs from another, is the theme's business.
//!
//! Each theme lives in a directory of its own, `src/themes/<name>/`: its
//! code, a README with screenshots, and the simulator script that takes
//! them (`shots.txt`, into `shots/`). See CONTRIBUTING.md for adding a
//! theme or a scheme.
//!
//! Each theme is a Cargo feature, `theme-<name>` (`theme-retro` the
//! default; `themes-all`, every one), so a build has only the themes it
//! names: the rest aren't compiled, which keeps the image small. The UI
//! holds a scheme as a [`Scheme`], an enum with a variant a built-in
//! theme, and calls the theme through a `match` on it, rather than a
//! `dyn Theme`: with one theme built in (the default, and every theme's
//! own package) that's a direct call, which inlines into the UI's
//! rendering as if there were no themes at all.
//!
//! A scheme is for darkness or for daylight: each theme lists its
//! schemes by side ([`BuiltIn`]), and may have either side, or both, and
//! several a side. The UI uses the night schemes from sunset, the day
//! ones from sunrise, and the settings' THEME row steps through the side in
//! use: every built-in theme's schemes for it, in [`THEMES`]' order. A
//! build with no scheme for a side (only night themes, say) uses the
//! other side's then.
//!
//! Every theme draws the dim look too (a glance, a notification on a dark
//! screen, the quiet night), in its scheme's [`Theme::dim`] palette,
//! which the canvas swaps in: the same drawing, other colours. A theme
//! may also draw differently there (retro's clock face does).

use embassy_time::{Duration, Instant};

use crate::alarm;
use crate::aranet::Sighting;
use crate::bthome::Identity;
use crate::buttons::{self, Press};
use crate::canvas::Canvas;
use crate::day::Day;
use crate::gb::MusicCommand;
use crate::nav::Side;
use crate::palette::Palette;
use crate::route;
use crate::system_off::Reason;
use crate::ui::{Editing, Status};

pub mod helpers;
#[cfg(feature = "theme-retro")]
pub mod retro;

// ==============================================================================
// The trait
// ==============================================================================

/// How a theme looks: its colours, every screen, and where a touch on
/// each lands. Each screen's method starts from whatever the canvas
/// holds, and clears it first; [`Theme::handles`] and [`Theme::banner`]
/// draw over the screen beneath.
///
/// No default methods: a new theme says how it draws each screen, and
/// where its buttons are, rather than inheriting another theme's without
/// anyone noticing. A theme draws every screen with its own code, never
/// another theme's: it may use [`helpers`] (mechanics that choose no
/// colour, place, size or font) and the shared crate modules (the canvas,
/// fonts, palettes, and the data a screen shows), but no other theme's
/// module. One that starts from another's look starts from a copy of its
/// code (see AUTHORING-THEMES.md), and the build refuses a theme feature
/// that turns on another (see `manifests.rs`).
///
/// Every method has `&self`, the scheme being shown, whose palette the
/// canvas is in (or, in the dim look, its `dim()` palette): a theme that
/// draws every scheme alike, by colour roles, needn't look at it.
pub trait Theme: Sync {
    /// The theme's name, as people read it: "Retro", "Living Tokyo".
    /// Where the watch's firmware is listed and chosen (`ur-sim
    /// --list-themes`, the web page's picker, a release's notes), not on
    /// the watch, which shows the scheme's name ([`Theme::palette`]). The
    /// same for every scheme of the theme. Its module, feature and
    /// package go by its directory's name instead.
    fn theme_name(&self) -> &'static str;

    // Colours.

    /// The scheme's palette, with its name: what THEME shows, and what
    /// the choice is saved as (its [`Palette::id`]).
    fn palette(&self) -> &Palette;
    /// The dim look's palette. The panel shows the dim look in idle mode
    /// (see `display::Display::set_idle`), which keeps only the top bit of
    /// each of red, green, and blue: every colour here must be one of
    /// those eight (each channel fully on or off), or it comes out as
    /// another, and a grey can only be a half-tone of two
    /// ([`Palette::columns`]). [`Palette::idle_mode_safe`] says whether it
    /// is, as a `const fn` to assert on it with (retro's does);
    /// `host-tests/theme` checks retro's too.
    fn dim(&self) -> &Palette;

    // The clock, and the screens around it (see `crate::ui`).

    /// The clock face, the screen the watch comes back to.
    fn clock(&self, canvas: &mut Canvas, face: &Face);
    /// A glance (see `crate::glance`): the time, bigger, and nothing else;
    /// or the next turn, while the phone navigates. Always in the dim
    /// look.
    fn glance(&self, canvas: &mut Canvas, glance: &Glance);
    /// Today: the steps and heart rate, by the hour.
    fn today(&self, canvas: &mut Canvas, day: Option<&Day>, steps: Option<u32>);
    /// Last night's sleep, if any was tracked in the last day.
    fn last_night(&self, canvas: &mut Canvas, sleep: Option<crate::activity::Sleep>);
    /// The alarms, smart wake and timers (see [`Theme::alarm_at`]).
    fn alarms(
        &self,
        canvas: &mut Canvas,
        settings: &alarm::Settings,
        state: &alarm::State,
        status: &Status,
    );
    /// The phone's alarms, read-only.
    fn phone_alarms(&self, canvas: &mut Canvas, phone: &alarm::Phone);
    /// A notification with the whole screen: a page of the history, or
    /// one that arrived with the screen off (then in the dim look).
    /// `arrived` says when, in the history ([`crate::arrival`]); empty
    /// for one that has just come.
    fn notification(&self, canvas: &mut Canvas, app: &str, who: &str, arrived: &str);
    /// What the phone is playing, with its controls (see
    /// [`Theme::music_control`]).
    fn music(&self, canvas: &mut Canvas, music: &Music);
    /// A nearby room sensor's latest reading (see [`crate::aranet`]).
    fn air(&self, canvas: &mut Canvas, sighting: &Sighting, now: Instant);
    /// The button grid (see [`crate::buttons`], and
    /// [`Theme::grid_button`]).
    fn buttons(&self, canvas: &mut Canvas, grid: &Grid);
    /// A settings screen, the look's or the button grid's: its title,
    /// and its rows, a setting each, as the UI words them (see
    /// [`SettingsPage`]).
    fn settings(&self, canvas: &mut Canvas, page: &SettingsPage);
    /// The watch's state: battery, uptime, the phone, the build.
    fn info(&self, canvas: &mut Canvas, status: &Status);
    /// Asking whether to switch to storage mode (see
    /// [`Theme::storage_confirms`]).
    fn storage_prompt(&self, canvas: &mut Canvas);
    /// Home Assistant's key, as a QR code to scan (see
    /// [`Canvas::qr`]), or that there's none yet. A tap anywhere shows it
    /// as text.
    fn bthome_qr(&self, canvas: &mut Canvas, identity: Option<&Identity>);
    /// The same key as text, with the watch's address.
    fn bthome_key(&self, canvas: &mut Canvas, identity: Option<&Identity>);

    // Drawn over the screen beneath.

    /// The handles on a screen's edges, saying where a swipe leads (see
    /// [`Handles`]).
    fn handles(&self, canvas: &mut Canvas, handles: Handles);
    /// A notification, or something the wearer should know, over the
    /// bottom of the screen: the app (or what happened) and who it's from
    /// (or what to do).
    fn banner(&self, canvas: &mut Canvas, app: &str, who: &str);

    // Screens that take over (and every touch).

    /// The alarm editor, over the alarms (see [`Theme::editor_button`]
    /// and [`Theme::editor_field`]).
    fn editor(&self, canvas: &mut Canvas, editor: &Editor);
    /// An alarm or a timer ringing: a tap anywhere snoozes, the button
    /// stops it.
    fn ringing(&self, canvas: &mut Canvas, kind: alarm::Kind, status: &Status);
    /// "Find my watch": `elapsed` is how long the phone has looked. Any
    /// touch finds it.
    fn find(&self, canvas: &mut Canvas, elapsed: Duration);
    /// A call ringing (see [`Theme::call_button`]).
    fn call(&self, canvas: &mut Canvas, call: &Call);
    /// Pairing: the passkey to type on the phone.
    fn pairing(&self, canvas: &mut Canvas, passkey: u32);
    /// A firmware update arriving: how far along, in percent.
    fn update(&self, canvas: &mut Canvas, percent: u8, now: Instant);
    /// The few seconds before the watch switches itself off, and why.
    fn off(&self, canvas: &mut Canvas, reason: Reason);

    // Where a touch lands, at (`x`, `y`) in canvas pixels, on the screens
    // with something to tap: each where the theme draws it. Swipes that
    // navigate, the clock face's media gestures, and taps anywhere (to
    // snooze, say) are the UI's.

    /// Which alarm a tap or long press on the alarms screen is on: a tap
    /// opens it in the editor, a long press switches it on or off.
    fn alarm_at(&self, x: i32, y: i32) -> Option<Editing>;
    /// Which of the editor's buttons a tap is on, if any.
    fn editor_button(&self, x: i32, y: i32) -> Option<EditorButton>;
    /// Which part of the time a swipe up or down in the editor changes.
    fn editor_field(&self, x: i32, y: i32) -> EditorField;
    /// Which of a settings screen's rows (its index in `page.rows`, as
    /// [`Theme::settings`] draws them) a tap is on, if any.
    fn settings_row(&self, page: &SettingsPage, x: i32, y: i32) -> Option<usize>;
    /// Which button of the grid, in `layout`, a touch is on: 1 to
    /// `layout.buttons()`, every touch on one (the grid takes a touch
    /// anywhere as a press; see [`buttons::Grid::touch`]).
    fn grid_button(&self, layout: buttons::Layout, x: i32, y: i32) -> u8;
    /// Which of a call's buttons a tap is on, if any.
    fn call_button(&self, x: i32, y: i32) -> Option<CallButton>;
    /// Which of the music's controls a tap is on, if any.
    fn music_control(&self, x: i32, y: i32) -> Option<MusicCommand>;
    /// Whether a tap here on storage mode's prompt confirms it (anything
    /// else backs out).
    fn storage_confirms(&self, x: i32, y: i32) -> bool;

    // How far long names scroll, which the UI needs to know beyond the
    // drawing: it draws frames more often while something scrolls, and
    // keeps the music on for one whole scroll.

    /// How many pixels too wide for its room `caller` is on a call's
    /// screen (none, or less, if it fits): what [`Canvas::marquee`]
    /// scrolls.
    fn call_overflow(&self, caller: &str) -> i32;
    /// The same for the music screen's artist and track: the most either
    /// overflows by.
    fn music_overflow(&self, artist: &str, track: &str) -> i32;
}

/// A button of the alarm editor (see [`Theme::editor_button`]).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum EditorButton {
    /// On or off (for a timer, start or stop).
    Switch,
    /// The next page (smart wake's weekends), or done.
    Next,
}

/// What a swipe in the alarm editor changes (see [`Theme::editor_field`]).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum EditorField {
    Hours,
    Minutes,
}

/// A button of a ringing call (see [`Theme::call_button`]).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CallButton {
    Decline,
    Answer,
}

// ==============================================================================
// What the screens show
// ==============================================================================
//
// Beyond `ui::Status`, which every screen may read: what the UI holds for
// each screen, as plain values, so a theme needs nothing of the UI's own
// state to draw it.

/// The clock face's own.
pub struct Face<'a> {
    pub status: &'a Status<'a>,
    /// The next alarm or timer, and when it rings (local seconds).
    pub next_alarm: Option<(alarm::Kind, i64)>,
    /// While a phone is connected: what to show of its media, if anything.
    pub media: Option<Media>,
    /// Notifications came in that nobody has seen. Retro shows it as a
    /// handle (see [`Handles::Unseen`]), which comes after; a theme may
    /// show it on the face instead.
    #[allow(dead_code)] // Retro doesn't: not read in a build of it alone.
    pub unseen: bool,
    /// In the dim look (see `crate::ui`).
    pub dim: bool,
}

/// What the clock face shows of the phone's media.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Media {
    /// It's playing.
    Playing,
    /// What a media gesture just did, for a moment (see `ui::App::media`).
    Did(MediaAction),
}

/// What a media gesture did (see `ui::App::media`).
#[derive(Clone, Copy, PartialEq, Eq, defmt::Format)]
pub enum MediaAction {
    /// Play or pause, when what it's become is known.
    Play,
    Pause,
    /// Play or pause, when it isn't: the toggle itself.
    PlayPause,
    Next,
    Previous,
}

/// A glance's.
pub struct Glance<'a> {
    pub status: &'a Status<'a>,
    /// While the phone navigates: the next turn, shown instead of the
    /// time alone.
    pub route: Option<&'a route::Step>,
    /// What a media gesture just did, for a moment.
    pub media: Option<MediaAction>,
}

/// The music screen's.
pub struct Music<'a> {
    pub artist: &'a str,
    pub track: &'a str,
    /// Whether it's playing, as far as the watch knows (see `ui::Music`).
    pub playing: Option<bool>,
    /// Since the track changed, for scrolling long names (see
    /// [`Canvas::marquee`], and [`Theme::music_overflow`]).
    pub elapsed: Duration,
}

/// The button grid's.
pub struct Grid<'a> {
    pub layout: buttons::Layout,
    /// What it's waiting on, and which button is held.
    pub grid: &'a buttons::Grid,
    /// The last press it sent, while that shows, and whether it went
    /// anywhere.
    pub sent: Option<(Press, bool)>,
    /// In the dim look.
    pub dim: bool,
}

/// A settings screen's: the look's (THEME, FORCE, LIGHT) or the button
/// grid's (BUTTONS, GRID, and BTHOME KEY while its presses go over
/// BTHome). Every row is in words already, so a theme draws both screens,
/// and any the UI adds, the same way: it only lays out and styles rows.
/// Each setting has a fixed place on its screen, so none moves under a
/// finger tapping it round; BTHOME KEY, the one that comes and goes, is
/// last.
pub struct SettingsPage<'a> {
    pub title: &'static str,
    /// Top to bottom; three at most.
    pub rows: &'a [SettingRow],
}

/// A row on a settings screen: a setting's name, and its value.
pub struct SettingRow {
    pub label: &'static str,
    pub value: &'static str,
    /// What a tap on it does, for the theme to style it by.
    pub kind: RowKind,
    /// A caveat to the value, beside it: GRID's "*B04" (FAQ B04), as two
    /// layers reach Home Assistant unlike six buttons do.
    pub note: Option<&'static str>,
}

/// What a tap on a setting's row does.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    /// Its next value.
    Choice,
    /// Nothing, but a banner with an FAQ code: it can't change in this
    /// build (THEME with one scheme for the side, FORCE with schemes of one
    /// side only).
    Fixed,
    /// Another screen: the value says what it shows (BTHOME KEY's SHOW).
    Link,
}

/// The alarm editor's.
pub struct Editor<'a> {
    pub editing: Editing,
    pub settings: &'a alarm::Settings,
    pub state: &'a alarm::State,
    pub uptime_secs: i64,
}

/// A ringing call's.
pub struct Call<'a> {
    pub caller: &'a str,
    /// Since it started ringing, for scrolling a long name (see
    /// [`Theme::call_overflow`]).
    pub elapsed: Duration,
    /// The button silenced it.
    pub silenced: bool,
}

/// Where a swipe leads from the screen showing.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Handles {
    /// On the clock: notifications came in that nobody has seen, above.
    Unseen,
    /// `index` screens out along the line on `side` of the clock, of
    /// `count`: home is the other way, and further out this way if
    /// there's more.
    Line {
        side: Side,
        index: usize,
        count: usize,
    },
}

// ==============================================================================
// For any theme
// ==============================================================================

/// The day's steps that make the goal: retro's clock face fills its ten
/// blocks at 1,000 a block, and from there counts 10,000 a block.
///
/// Fixed, because there's nowhere to learn the wearer's own: Gadgetbridge
/// keeps a step goal (its "fitness_goal" setting, 8,000 by default) for its
/// charts, but never sends it to a Bangle.js. Its driver has no message
/// for the user's settings (the call that would send them,
/// `sendSettings`, is commented out in `BangleJSDeviceSupport`), and its
/// `act` message only switches live heart rate and steps on and off.
/// 10,000 is the round number most people know.
///
/// TODO: a goal set on the watch: a tap-to-cycle row on Settings, like
/// THEME and LIGHT (say 6k, 8k, 10k, 12k), kept with the settings in
/// flash. Retro's scales ("×1K", "×10K") would then follow the goal.
pub const STEP_GOAL: u32 = 10_000;

// ==============================================================================
// The built-in themes and their schemes
// ==============================================================================

/// The built-in themes, a line each: its module (`pub mod` above, under
/// its feature), its type, and its feature. From the one list, under each
/// line's feature, come [`Scheme`]'s variants, `with_theme!`'s arms, and
/// [`THEMES`], so none can miss a theme the others have; and the build
/// scripts check every `src/themes/<name>/` has its line here and its
/// features in both manifests (see `manifests.rs`).
///
/// `$d` is a `$`, passed in, for the `with_theme!` it writes.
macro_rules! themes {
    ($d:tt $($module:ident: $Type:ident if $feature:literal;)+) => {
        /// A colour scheme of a built-in theme: a variant a theme, holding
        /// the scheme (a static of the theme's type). It's a [`Theme`]
        /// itself, by a `match` on the variant (see the module docs), so
        /// the UI needn't know which theme it is.
        #[derive(Clone, Copy)]
        pub enum Scheme {
            $(
                #[cfg(feature = $feature)]
                $Type(&'static $module::$Type),
            )+
        }

        /// Calls `$call` with `$theme` bound to the scheme in `$scheme`,
        /// whatever its theme.
        macro_rules! with_theme {
            ($d scheme:expr, $d theme:ident => $d call:expr) => {
                match $d scheme {
                    $(
                        #[cfg(feature = $feature)]
                        Scheme::$Type($d theme) => $d call,
                    )+
                }
            };
        }

        /// The themes built in, in the order their schemes come in
        /// THEME's cycle.
        pub const THEMES: &[BuiltIn] = &[
            $(
                #[cfg(feature = $feature)]
                $module::BUILT_IN,
            )+
        ];
    };
}

themes! {$
    retro: Retro if "theme-retro";
}

impl Theme for Scheme {
    fn theme_name(&self) -> &'static str {
        with_theme!(*self, theme => theme.theme_name())
    }
    fn palette(&self) -> &Palette {
        with_theme!(*self, theme => theme.palette())
    }
    fn dim(&self) -> &Palette {
        with_theme!(*self, theme => theme.dim())
    }
    fn clock(&self, canvas: &mut Canvas, face: &Face) {
        with_theme!(*self, theme => theme.clock(canvas, face))
    }
    fn glance(&self, canvas: &mut Canvas, glance: &Glance) {
        with_theme!(*self, theme => theme.glance(canvas, glance))
    }
    fn today(&self, canvas: &mut Canvas, day: Option<&Day>, steps: Option<u32>) {
        with_theme!(*self, theme => theme.today(canvas, day, steps))
    }
    fn last_night(&self, canvas: &mut Canvas, sleep: Option<crate::activity::Sleep>) {
        with_theme!(*self, theme => theme.last_night(canvas, sleep))
    }
    fn alarms(
        &self,
        canvas: &mut Canvas,
        settings: &alarm::Settings,
        state: &alarm::State,
        status: &Status,
    ) {
        with_theme!(*self, theme => theme.alarms(canvas, settings, state, status))
    }
    fn phone_alarms(&self, canvas: &mut Canvas, phone: &alarm::Phone) {
        with_theme!(*self, theme => theme.phone_alarms(canvas, phone))
    }
    fn notification(&self, canvas: &mut Canvas, app: &str, who: &str, arrived: &str) {
        with_theme!(*self, theme => theme.notification(canvas, app, who, arrived))
    }
    fn music(&self, canvas: &mut Canvas, music: &Music) {
        with_theme!(*self, theme => theme.music(canvas, music))
    }
    fn air(&self, canvas: &mut Canvas, sighting: &Sighting, now: Instant) {
        with_theme!(*self, theme => theme.air(canvas, sighting, now))
    }
    fn buttons(&self, canvas: &mut Canvas, grid: &Grid) {
        with_theme!(*self, theme => theme.buttons(canvas, grid))
    }
    fn settings(&self, canvas: &mut Canvas, page: &SettingsPage) {
        with_theme!(*self, theme => theme.settings(canvas, page))
    }
    fn info(&self, canvas: &mut Canvas, status: &Status) {
        with_theme!(*self, theme => theme.info(canvas, status))
    }
    fn storage_prompt(&self, canvas: &mut Canvas) {
        with_theme!(*self, theme => theme.storage_prompt(canvas))
    }
    fn bthome_qr(&self, canvas: &mut Canvas, identity: Option<&Identity>) {
        with_theme!(*self, theme => theme.bthome_qr(canvas, identity))
    }
    fn bthome_key(&self, canvas: &mut Canvas, identity: Option<&Identity>) {
        with_theme!(*self, theme => theme.bthome_key(canvas, identity))
    }
    fn handles(&self, canvas: &mut Canvas, handles: Handles) {
        with_theme!(*self, theme => theme.handles(canvas, handles))
    }
    fn banner(&self, canvas: &mut Canvas, app: &str, who: &str) {
        with_theme!(*self, theme => theme.banner(canvas, app, who))
    }
    fn editor(&self, canvas: &mut Canvas, editor: &Editor) {
        with_theme!(*self, theme => theme.editor(canvas, editor))
    }
    fn ringing(&self, canvas: &mut Canvas, kind: alarm::Kind, status: &Status) {
        with_theme!(*self, theme => theme.ringing(canvas, kind, status))
    }
    fn find(&self, canvas: &mut Canvas, elapsed: Duration) {
        with_theme!(*self, theme => theme.find(canvas, elapsed))
    }
    fn call(&self, canvas: &mut Canvas, call: &Call) {
        with_theme!(*self, theme => theme.call(canvas, call))
    }
    fn pairing(&self, canvas: &mut Canvas, passkey: u32) {
        with_theme!(*self, theme => theme.pairing(canvas, passkey))
    }
    fn update(&self, canvas: &mut Canvas, percent: u8, now: Instant) {
        with_theme!(*self, theme => theme.update(canvas, percent, now))
    }
    fn off(&self, canvas: &mut Canvas, reason: Reason) {
        with_theme!(*self, theme => theme.off(canvas, reason))
    }
    fn alarm_at(&self, x: i32, y: i32) -> Option<Editing> {
        with_theme!(*self, theme => theme.alarm_at(x, y))
    }
    fn editor_button(&self, x: i32, y: i32) -> Option<EditorButton> {
        with_theme!(*self, theme => theme.editor_button(x, y))
    }
    fn editor_field(&self, x: i32, y: i32) -> EditorField {
        with_theme!(*self, theme => theme.editor_field(x, y))
    }
    fn settings_row(&self, page: &SettingsPage, x: i32, y: i32) -> Option<usize> {
        with_theme!(*self, theme => theme.settings_row(page, x, y))
    }
    fn grid_button(&self, layout: buttons::Layout, x: i32, y: i32) -> u8 {
        with_theme!(*self, theme => theme.grid_button(layout, x, y))
    }
    fn call_button(&self, x: i32, y: i32) -> Option<CallButton> {
        with_theme!(*self, theme => theme.call_button(x, y))
    }
    fn music_control(&self, x: i32, y: i32) -> Option<MusicCommand> {
        with_theme!(*self, theme => theme.music_control(x, y))
    }
    fn storage_confirms(&self, x: i32, y: i32) -> bool {
        with_theme!(*self, theme => theme.storage_confirms(x, y))
    }
    fn call_overflow(&self, caller: &str) -> i32 {
        with_theme!(*self, theme => theme.call_overflow(caller))
    }
    fn music_overflow(&self, artist: &str, track: &str) -> i32 {
        with_theme!(*self, theme => theme.music_overflow(artist, track))
    }
}

/// A theme built into this firmware: its name (its directory's, and its
/// feature's, `theme-<name>`; people read [`BuiltIn::display_name`]),
/// and its schemes, by side.
pub struct BuiltIn {
    #[allow(dead_code)] // For the simulator, which lists them.
    pub name: &'static str,
    pub night: &'static [Scheme],
    pub day: &'static [Scheme],
}

impl BuiltIn {
    /// The theme's name as people read it ([`Theme::theme_name`]).
    #[allow(dead_code)] // For the simulator, which lists them.
    pub fn display_name(&self) -> &'static str {
        let scheme = self.night.first().or(self.day.first());
        scheme.expect("every theme has a scheme").theme_name()
    }
}

// What follows is worked out at compile time, from `THEMES`: every
// built-in theme's schemes for a side, one after the other, as a static
// each, so that a lookup at runtime is an index into it.

/// A built-in theme's schemes for daylight (`day`) or darkness.
const fn of(theme: &BuiltIn, day: bool) -> &'static [Scheme] {
    if day { theme.day } else { theme.night }
}

/// Whether this build has a scheme of its own for daylight (`day`) or
/// darkness. Without, that side uses the other's (see [`schemes`]).
pub const fn has(day: bool) -> bool {
    let mut i = 0;
    while i < THEMES.len() {
        if !of(&THEMES[i], day).is_empty() {
            return true;
        }
        i += 1;
    }
    false
}

/// How many schemes there are for a side, the other's if it has none.
const fn len(day: bool) -> usize {
    let day = if has(day) { day } else { !day };
    let mut n = 0;
    let mut i = 0;
    while i < THEMES.len() {
        n += of(&THEMES[i], day).len();
        i += 1;
    }
    n
}

/// Every built-in theme's schemes for a side (the other's, if it has
/// none), one after the other.
const fn flatten<const N: usize>(day: bool) -> [Scheme; N] {
    let day = if has(day) { day } else { !day };
    // Something to fill the array with first: any scheme will do, as
    // every place is filled below. (And the build's one check that it
    // has a theme.)
    let Some(theme) = THEMES.first() else {
        panic!("no theme: build with one (`--features theme-retro`, the default), or `themes-all`")
    };
    let first = match (theme.night.first(), theme.day.first()) {
        (Some(&scheme), _) | (None, Some(&scheme)) => scheme,
        (None, None) => panic!("a theme with no schemes"),
    };
    let mut out = [first; N];
    let (mut n, mut i) = (0, 0);
    while i < THEMES.len() {
        let schemes = of(&THEMES[i], day);
        let mut j = 0;
        while j < schemes.len() {
            out[n] = schemes[j];
            (n, j) = (n + 1, j + 1);
        }
        i += 1;
    }
    out
}

static NIGHT: [Scheme; len(false)] = flatten(false);
static DAY: [Scheme; len(true)] = flatten(true);

/// The schemes for daylight (`day`) or darkness, in THEME's order. A
/// build with none for a side (only night themes, say) has the other
/// side's then.
pub fn schemes(day: bool) -> &'static [Scheme] {
    if day { &DAY } else { &NIGHT }
}

/// How many schemes there are for daylight (`day`) or darkness.
pub fn count(day: bool) -> usize {
    schemes(day).len()
}

/// The `index`th scheme for daylight (`day`) or darkness, wrapping round.
pub fn get(day: bool, index: usize) -> &'static Scheme {
    let schemes = schemes(day);
    &schemes[index % schemes.len()]
}

// ==============================================================================
// Saving the choice
// ==============================================================================
//
// Settings save the chosen scheme for each side by its ID (see
// `palette::id`), not by its place in `schemes`: which schemes there are,
// and in what order, varies from build to build, and an index saved by
// one would pick another scheme in the next.

/// The ID of the `index`th scheme for daylight (`day`) or darkness, to
/// save.
pub fn id(day: bool, index: usize) -> u32 {
    get(day, index).palette().id
}

/// Where the scheme saved as `id` is among those for daylight (`day`) or
/// darkness, if it's built in; otherwise the first, as on a watch that
/// had saved nothing.
pub fn find(day: bool, id: u32) -> usize {
    schemes(day)
        .iter()
        .position(|scheme| scheme.palette().id == id)
        .unwrap_or(0)
}

/// The ID of what 0.10 and earlier saved: an index into retro's schemes
/// for that side of the day, as they were then (wrapping round, as they
/// did then), so an upgraded watch keeps its schemes. Only names: the
/// schemes needn't be built in (a build without retro shows its first).
pub fn migrate(day: bool, index: u8) -> u32 {
    use crate::palette::id;
    const OLD: [[u32; 2]; 2] = [
        [id("ARCADE"), id("TERMINAL")],
        [id("HANDHELD"), id("PARCHMENT")],
    ];
    OLD[usize::from(day)][usize::from(index & 1)]
}
