//! Retro: the watch's own look, after old handhelds, arcade cabinets and
//! terminals. Pixel art on the 80×80 canvas: the 5×7 digits big with a
//! drop shadow, the 3×5 font for everything else, ten blocks for the
//! day's steps like an old game's health bar, and chunky keycaps for the
//! button grid. Its colour schemes, for the night and for the day, are
//! in [`schemes`], with the black, white and red dim look ([`GLANCE`]).
//!
//! The screens are drawn in the files beside this one, a file a group of
//! screens; [`Retro`]'s [`Theme`] says which draws which.

// Its own code is held to dead code in every build, even one where the
// shared code a theme draws with isn't (see `main.rs`), so that a copy
// of it, built alone, still says what it no longer uses.
#![warn(dead_code)]

use embassy_time::{Duration, Instant};

use crate::alarm;
use crate::aranet::Sighting;
use crate::bthome::Identity;
use crate::buttons;
use crate::canvas::{Canvas, Color};
use crate::day::Day;
use crate::font;
use crate::gb::MusicCommand;
use crate::nav::Side;
use crate::palette::Palette;
use crate::system_off::Reason;
use crate::ui::{Editing, Status};

use super::helpers::{self, digit_glyph};
use super::{
    BuiltIn, Call, CallButton, Editor, EditorButton, EditorField, Face, Glance, Grid, Handles,
    MediaAction, Music, Scheme, SettingsPage, Theme,
};

mod air;
mod alarms;
mod clock;
mod glance;
mod grid;
mod messages;
mod past;
mod schemes;
mod settings;
mod system;

pub use schemes::{ARCADE, GLANCE, Retro};
#[allow(unused_imports)] // For the simulator's tests.
pub use settings::name_fits;

/// Retro in the firmware's list of themes (see [`super::THEMES`]).
pub const BUILT_IN: BuiltIn = BuiltIn {
    name: "retro",
    night: &wrap(schemes::NIGHT),
    day: &wrap(schemes::DAY),
};

/// Retro's schemes for a side, as the UI holds them.
const fn wrap<const N: usize>(list: [&'static Retro; N]) -> [Scheme; N] {
    let mut out = [Scheme::Retro(&ARCADE); N];
    let mut i = 0;
    while i < N {
        out[i] = Scheme::Retro(list[i]);
        i += 1;
    }
    out
}

/// Where retro draws what can be tapped, for the drawing and the taps
/// alike (see [`Theme::alarm_at`] and the like).
mod layout {
    /// The alarms screen: smart wake across the top, then the two alarms
    /// side by side, then the two timers side by side, each half of the
    /// screen's width. These are the tops of those bands.
    pub const SMART_TOP: i32 = 11;
    pub const ALARMS_TOP: i32 = 30;
    pub const TIMERS_TOP: i32 = 53;
    /// Where the editor's buttons start: on/off on the left half, next or
    /// done on the right. Above them, a swipe up or down on the left half
    /// changes the hours, on the right the minutes.
    pub const EDITOR_BUTTONS_Y: i32 = 60;
    /// Where the settings' rows start, and how tall each is: three fill
    /// the screen under the title (see `settings`).
    pub const SETTINGS_TOP: i32 = 10;
    pub const SETTINGS_ROW: i32 = 20;
    /// Where a call's buttons start: decline on the left half, answer on
    /// the right.
    pub const CALL_BUTTONS_Y: i32 = 55;
    /// How wide a caller's name may be, at 2×, before it scrolls: columns
    /// 4..76.
    pub const CALL_TEXT_WIDTH: i32 = 72;
    /// Where the music's controls start: previous, play or pause, and
    /// next, in thirds.
    pub const MUSIC_CONTROLS_Y: i32 = 52;
    /// How wide the track (at 2×) and the artist (at 1×) may be before
    /// they scroll: columns 3..77.
    pub const MUSIC_TEXT_WIDTH: i32 = 74;
}

impl Theme for Retro {
    fn theme_name(&self) -> &'static str {
        "Retro"
    }

    fn palette(&self) -> &Palette {
        &self.palette
    }

    fn dim(&self) -> &Palette {
        &GLANCE
    }

    fn clock(&self, canvas: &mut Canvas, face: &Face) {
        clock::render_clock(canvas, face);
    }

    fn glance(&self, canvas: &mut Canvas, glance: &Glance) {
        glance::render_glance(canvas, glance);
    }

    fn today(&self, canvas: &mut Canvas, day: Option<&Day>, steps: Option<u32>) {
        past::render_today(canvas, day, steps);
    }

    fn last_night(&self, canvas: &mut Canvas, sleep: Option<crate::activity::Sleep>) {
        past::render_last_night(canvas, sleep);
    }

    fn alarms(
        &self,
        canvas: &mut Canvas,
        settings: &alarm::Settings,
        state: &alarm::State,
        status: &Status,
    ) {
        alarms::render_alarms(canvas, settings, state, status);
    }

    fn phone_alarms(&self, canvas: &mut Canvas, phone: &alarm::Phone) {
        alarms::render_phone_alarms(canvas, phone);
    }

    fn notification(&self, canvas: &mut Canvas, app: &str, who: &str, arrived: &str) {
        messages::render_card(canvas, app, who, arrived);
    }

    fn music(&self, canvas: &mut Canvas, music: &Music) {
        messages::render_music(canvas, music);
    }

    fn air(&self, canvas: &mut Canvas, sighting: &Sighting, now: Instant) {
        air::render_air(canvas, sighting, now);
    }

    fn buttons(&self, canvas: &mut Canvas, grid: &Grid) {
        // Shaded for a dark palette, where edges catch light against
        // black, or a light one, where the ink is the dark: by the palette
        // it shows in, the dim look's (white on black) included.
        let palette = if grid.dim { self.dim() } else { self.palette() };
        grid::render_buttons(canvas, grid, palette.is_dark());
    }

    fn settings(&self, canvas: &mut Canvas, page: &SettingsPage) {
        settings::render_settings(canvas, page);
    }

    fn info(&self, canvas: &mut Canvas, status: &Status) {
        settings::render_info(canvas, status);
    }

    fn storage_prompt(&self, canvas: &mut Canvas) {
        system::render_storage_prompt(canvas);
    }

    fn bthome_qr(&self, canvas: &mut Canvas, identity: Option<&Identity>) {
        settings::render_ha_qr(canvas, identity);
    }

    fn bthome_key(&self, canvas: &mut Canvas, identity: Option<&Identity>) {
        settings::render_ha_key(canvas, identity);
    }

    fn handles(&self, canvas: &mut Canvas, handles: Handles) {
        match handles {
            // An accent-coloured handle on the edge they're behind.
            Handles::Unseen => edge_bar(canvas, Side::Above, Color::ACCENT),
            // Home, always, and further out if there's more.
            Handles::Line { side, index, count } => {
                edge_bar(canvas, side.opposite(), Color::TEXT_DIM);
                if index + 1 < count {
                    edge_bar(canvas, side, Color::TEXT_DIM);
                }
            }
        }
    }

    fn banner(&self, canvas: &mut Canvas, app: &str, who: &str) {
        messages::render_banner(canvas, app, who);
    }

    fn editor(&self, canvas: &mut Canvas, editor: &Editor) {
        alarms::render_editor(canvas, editor);
    }

    fn ringing(&self, canvas: &mut Canvas, kind: alarm::Kind, status: &Status) {
        alarms::render_ringing(canvas, kind, status);
    }

    fn find(&self, canvas: &mut Canvas, elapsed: Duration) {
        alarms::render_find(canvas, elapsed);
    }

    fn call(&self, canvas: &mut Canvas, call: &Call) {
        messages::render_call(canvas, call);
    }

    fn pairing(&self, canvas: &mut Canvas, passkey: u32) {
        system::render_pairing(canvas, passkey);
    }

    fn update(&self, canvas: &mut Canvas, percent: u8, now: Instant) {
        system::render_update(canvas, percent, now);
    }

    fn off(&self, canvas: &mut Canvas, reason: Reason) {
        system::render_off(canvas, reason);
    }

    fn charged(&self, canvas: &mut Canvas, status: &Status) {
        glance::render_charged(canvas, status);
    }

    fn alarm_at(&self, x: i32, y: i32) -> Option<Editing> {
        use layout::{ALARMS_TOP, SMART_TOP, TIMERS_TOP};
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

    fn editor_button(&self, x: i32, y: i32) -> Option<EditorButton> {
        match (y >= layout::EDITOR_BUTTONS_Y, x < 40) {
            (false, _) => None,
            (true, true) => Some(EditorButton::Switch),
            (true, false) => Some(EditorButton::Next),
        }
    }

    fn editor_field(&self, x: i32, _y: i32) -> EditorField {
        if x < 40 {
            EditorField::Hours
        } else {
            EditorField::Minutes
        }
    }

    fn settings_row(&self, page: &SettingsPage, _x: i32, y: i32) -> Option<usize> {
        settings::row_at(y, page.rows.len())
    }

    fn grid_button(&self, layout: buttons::Layout, x: i32, y: i32) -> u8 {
        buttons::at(layout, x, y)
    }

    fn call_button(&self, x: i32, y: i32) -> Option<CallButton> {
        match (y >= layout::CALL_BUTTONS_Y, x < 40) {
            (false, _) => None,
            (true, true) => Some(CallButton::Decline),
            (true, false) => Some(CallButton::Answer),
        }
    }

    fn music_control(&self, x: i32, y: i32) -> Option<MusicCommand> {
        if y < layout::MUSIC_CONTROLS_Y {
            return None;
        }
        // Left, middle, and right thirds.
        Some(match x {
            ..27 => MusicCommand::Previous,
            27..53 => MusicCommand::PlayPause,
            _ => MusicCommand::Next,
        })
    }

    fn storage_confirms(&self, _x: i32, _y: i32) -> bool {
        // Anywhere: a long press, then a tap, is hard to do by accident.
        true
    }

    fn call_overflow(&self, caller: &str) -> i32 {
        helpers::overflow(caller, 2, layout::CALL_TEXT_WIDTH)
    }

    fn music_overflow(&self, artist: &str, track: &str) -> i32 {
        let room = layout::MUSIC_TEXT_WIDTH;
        helpers::overflow(track, 2, room).max(helpers::overflow(artist, 1, room))
    }
}

// ==============================================================================
// Shared by the screens
// ==============================================================================

const BACKGROUND: Color = Color::BACKGROUND;

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

/// A media action's icon, as the clock face's status bar and the glance
/// show it: here, as both draw it, so that each file can be replaced
/// without the other.
fn action_icon(action: MediaAction) -> &'static [u8; 5] {
    match action {
        MediaAction::Play => &font::PLAY,
        MediaAction::Pause => &font::PAUSE,
        MediaAction::PlayPause => &font::PLAY_PAUSE,
        MediaAction::Next => &font::NEXT,
        MediaAction::Previous => &font::PREVIOUS,
    }
}

/// One big 5×7 digit at `scale`, with a 1 px drop shadow down and right.
fn draw_digit(canvas: &mut Canvas, x: i32, y: i32, digit: u64, scale: i32, color: Color) {
    let glyph = digit_glyph(digit);
    canvas.sprite(x + 1, y + 1, font::DIGIT_W, glyph, scale, Color::SHADOW);
    canvas.sprite(x, y, font::DIGIT_W, glyph, scale, color);
}
