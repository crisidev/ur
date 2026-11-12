//! The UI loop's every pass: what it does when it wakes (a tick, an input,
//! a timer), what it saves, when it renders and with what, and when the
//! screen goes on and off. `main` runs it on the watch, and the simulator
//! (`sim/`) runs the very same code on a computer.
//!
//! What touches hardware goes through [`Board`]: the panel's power, the
//! backlight, the battery's and the die's sensors, the watchdog, the
//! button grid's advertisements, and switching off. `main` implements it
//! with the real thing; the simulator with stand-ins. Everything else (the
//! decisions) is here, once, so the two can't drift apart. What other
//! tasks share with the loop (the phone's connection, the room sensors heard,
//! today's activity, the heart rate) it reads from their modules, as on
//! the watch; the simulator includes or stands in for those.
//!
//! The loop itself is short, and `main`'s:
//!
//! ```text
//! let (mut app, mut ui) = UiLoop::boot(Stored::load().await);
//! loop {
//!     ui.pass(&mut app, &mut board, canvas).await;   // render, screen power
//!     let wake = /* the next tick, input, or timer */;
//!     ui.handle(&mut app, &mut board, wake).await;   // act on it, save
//! }
//! ```
//!
//! The simulator runs the same three steps, but decides itself when the
//! next tick or timer is due ([`TICK`], [`timer_at`]), on a clock it
//! moves, rather than waiting for them.

use core::cell::Cell;
use core::sync::atomic::Ordering;

use defmt::{debug, info};
use embassy_time::{Duration, Instant};

use crate::battery::{Battery, ChargeGlances, FullCharge, PlugIn, ToFull};
use crate::bthome::{self, Identity};
use crate::buttons::Press;
use crate::canvas::{self, Canvas, Sweep};
use crate::display::Display;
use crate::heart_reading as heart;
use crate::system_off::Reason;
use crate::touch::Touch;
use crate::ui::{App, Force, Status, ThemeChoice};
use crate::{
    activity, alarm, aranet, ble, brightness, buttons, clock, debug, dfu, gb, geo, glance, live,
    power, sleep, storage, temperature, themes, touch, ui,
};

// ==============================================================================
// What wakes the loop
// ==============================================================================

#[derive(defmt::Format)]
pub enum InputEvent {
    Button,
    Touch(Touch),
    /// Something needs the user to look (e.g. a pairing passkey): wake the
    /// screen if it's off, and redraw.
    Attention,
    /// A notification, call, or media update from the phone.
    Phone(ui::PhoneEvent),
    /// The wrist turned to look at the watch (see [`glance`]).
    Glance,
}

/// Why the loop woke.
pub enum Wake {
    /// Once a second.
    Tick,
    Input(InputEvent),
    /// The time [`timer_at`] gave: the next frame of scrolling text (which
    /// only redraws), or something the button grid waits for.
    Timer,
}

/// How often the loop ticks.
pub const TICK: Duration = Duration::from_secs(1);

/// When the loop's timer is due, after a pass, if at all: while text is
/// scrolling, the next animation frame, and when the button grid has
/// something due (see `App::deadline`), which, while a finger is down
/// (`finger_down`), is only a hold becoming a long press: otherwise that
/// touch decides, and its event wakes the loop.
pub fn timer_at(app: &App, finger_down: bool) -> Option<Instant> {
    let frame = app
        .animating()
        .then(|| Instant::now() + canvas::MARQUEE_FRAME);
    let due = app.deadline(finger_down);
    frame.into_iter().chain(due).min()
}

// ==============================================================================
// The hardware
// ==============================================================================

/// What the loop needs of the hardware. On the watch, `main`'s; in the
/// simulator, stand-ins that keep what they're told.
///
/// The panel's methods return the driver's own futures, rather than being
/// `async fn`s: an `async fn` here would wrap each in a state machine of
/// its own, and that costs flash.
pub trait Board {
    /// The die temperature, in quarter degrees (see [`temperature`]).
    fn die_temperature(&mut self) -> i32;
    /// The battery's voltage, as a raw SAADC sample (see
    /// [`Battery::update`]), into `sample`.
    fn sample_battery<'a>(&'a mut self, sample: &'a mut [i16; 1]) -> impl Future<Output = ()> + 'a;
    /// Whether the battery is charging, and whether the charger has power.
    /// Read every pass, at least once a second: the power arriving wakes
    /// the screen (see [`PlugIn`]).
    fn charger(&mut self) -> (bool, bool);
    /// Whether the sun is up where the watch is: [`geo::is_daytime`] on the
    /// watch. (The simulator's `theme` can say otherwise, to show a theme
    /// whatever the time.)
    fn daytime(&mut self) -> bool;
    /// Whether a finger is on the screen (`touch::finger_down`).
    fn finger_down(&mut self) -> bool;

    /// The panel, for the canvas to push pixels through.
    fn display(&mut self) -> &mut Display;
    /// See `Display::wake`, `sleep`, `show` and `set_idle`.
    fn wake_panel(&mut self) -> impl Future<Output = ()>;
    fn sleep_panel(&mut self) -> impl Future<Output = ()>;
    fn show_panel(&mut self) -> impl Future<Output = ()>;
    fn set_idle(&mut self, idle: bool) -> impl Future<Output = ()>;
    /// Waits for the panel's next refresh (`display::REFRESH`).
    fn wait_for_refresh(&mut self) -> impl Future<Output = ()>;
    fn backlight(&mut self, level: brightness::Level);

    /// The button grid's key and address for Home Assistant
    /// (`bthome::identity`), for the Settings screen.
    fn bthome(&mut self) -> Option<Identity>;
    /// Advertises a press of the button grid, laid out as `layout`, or
    /// with none, the battery alone (`bthome::send`); `false` if it can't.
    fn advertise(
        &mut self,
        press: Option<Press>,
        layout: buttons::Layout,
        battery: u8,
    ) -> impl Future<Output = bool>;

    /// Switches the watch off ([`crate::system_off::shut_down`]), showing
    /// why: what `canvas` holds, drawn by `App::render_off`. On the watch
    /// this never returns: the next thing that happens is a boot.
    async fn switch_off(&mut self, reason: Reason, canvas: &mut Canvas, panel_awake: bool);
    /// Feeds the watchdog, unless the button is held: holding it long
    /// enough to starve the watchdog is the user's way back into the
    /// bootloader (and to rollback), even when this firmware is otherwise
    /// wedged in a way that still runs this loop. (No watchdog in the
    /// simulator.)
    fn pet_watchdog(&mut self);
}

// ==============================================================================
// Settings in flash
// ==============================================================================
//
// Everything the UI saves goes through here: loaded at boot, and saved
// after whichever wake changed it. A new setting is a field in `Settings`,
// and a line in each of `Stored::load`, `apply` and `save_changes`. The
// phone's alarms and the temperature's calibration have rules of their
// own for when they're saved, so they're loaded beside the settings.

/// The settings in flash that the UI changes, to see what a wake changed.
#[derive(Clone, Copy)]
pub struct Settings {
    themes: ThemeChoice,
    force: Force,
    brightness: brightness::Setting,
    buttons: buttons::Setting,
    grid: buttons::Layout,
    alarms: alarm::Settings,
}

impl Settings {
    pub fn of(app: &App) -> Self {
        Self {
            themes: app.themes,
            force: app.force,
            brightness: app.brightness,
            buttons: app.buttons,
            grid: app.grid_layout,
            alarms: app.alarms,
        }
    }

    /// The UI as it boots, at `now`, with these settings.
    fn apply(self, now: Instant) -> App {
        let mut app = App::new(now, self.themes, self.alarms);
        app.force = self.force;
        app.brightness = self.brightness;
        app.buttons = self.buttons;
        app.grid_layout = self.grid;
        app
    }
}

/// What boot loads from flash.
pub struct Stored {
    settings: Settings,
    phone_alarms: alarm::Phone,
    calibration: temperature::Calibration,
}

impl Stored {
    /// What's in flash, or the defaults for what isn't; and where the
    /// phone last said the watch is, for the sunrise.
    ///
    /// Plain values, rather than the [`App`] they go into, so that the
    /// boot's future doesn't hold an `App` beside the one the UI loop's
    /// future keeps (the futures are static RAM).
    pub async fn load() -> Self {
        let load_u8 = async |key| storage::load::<1>(key).await.map_or(0, |[value]| value);
        // Each side's scheme by its ID; or, from before IDs, retro's index
        // (see `themes::migrate`), until the wearer next picks one. With
        // neither, that's retro's first scheme for the side (index 0, as
        // 0.10 took nothing saved to be), or this build's first if it
        // hasn't retro. In a loop, rather than an `async` closure called
        // for each side, which would put a copy of its awaits in this
        // future for each.
        let mut chosen = [0; 2];
        for (day, key, old) in [
            (false, storage::key::THEME_NIGHT, storage::key::THEME_DARK),
            (true, storage::key::THEME_DAY, storage::key::THEME_LIGHT),
        ] {
            let id = match storage::load::<4>(key).await {
                Some(id) => u32::from_le_bytes(id),
                None => themes::migrate(day, load_u8(old).await),
            };
            chosen[usize::from(day)] = themes::find(day, id) as u8;
        }
        let themes = ThemeChoice {
            night: chosen[0],
            day: chosen[1],
        };
        let alarms = storage::load(storage::key::ALARMS)
            .await
            .map(alarm::Settings::from_bytes)
            .unwrap_or_default();
        let phone_alarms = storage::load(storage::key::PHONE_ALARMS)
            .await
            .map(alarm::Phone::from_bytes)
            .unwrap_or_default();
        let force = Force::from_byte(load_u8(storage::key::FORCE).await);
        let brightness = brightness::Setting::from_byte(load_u8(storage::key::BRIGHTNESS).await);
        let buttons = buttons::Setting::from_byte(load_u8(storage::key::BUTTONS).await);
        let grid = buttons::Layout::from_byte(load_u8(storage::key::GRID).await);
        if let Some(location) = storage::load(storage::key::LOCATION).await {
            // An old fix, for the sunrise: not one to judge the room
            // sensors by.
            geo::set_location(geo::Location::from_bytes(location), false);
        }
        let calibration = storage::load(storage::key::CALIBRATION)
            .await
            .map(temperature::Calibration::from_bytes)
            .unwrap_or_default();
        Self {
            settings: Settings {
                themes,
                force,
                brightness,
                buttons,
                grid,
                alarms,
            },
            phone_alarms,
            calibration,
        }
    }
}

/// Saves the settings that changed since `before`.
pub async fn save_changes(before: Settings, app: &mut App) {
    // In a loop, as in `Stored::load`.
    for (day, key, now, then) in [
        (
            false,
            storage::key::THEME_NIGHT,
            app.themes.night,
            before.themes.night,
        ),
        (
            true,
            storage::key::THEME_DAY,
            app.themes.day,
            before.themes.day,
        ),
    ] {
        if now != then {
            let id = themes::id(day, now.into());
            storage::save(key, &id.to_le_bytes()).await;
        }
    }
    if app.force != before.force {
        info!("force: {=str}", app.force.name());
        storage::save(storage::key::FORCE, &[app.force.to_byte()]).await;
    }
    if app.brightness != before.brightness {
        info!("brightness: {=str}", app.brightness.name());
        storage::save(storage::key::BRIGHTNESS, &[app.brightness.to_byte()]).await;
    }
    if app.buttons != before.buttons {
        info!("buttons: {}", app.buttons);
        storage::save(storage::key::BUTTONS, &[app.buttons.to_byte()]).await;
    }
    if app.grid_layout != before.grid {
        info!("grid: {=str}", app.grid_layout.name());
        storage::save(storage::key::GRID, &[app.grid_layout.to_byte()]).await;
    }
    // Edited, switched, or (a one-off alarm) rung and switched off.
    if app.alarms != before.alarms {
        storage::save(storage::key::ALARMS, &app.alarms.to_bytes()).await;
    }
    // Synced, or a one-off rung and switched off. By the flag rather than
    // by comparing, so a sync is saved even when nothing in it changed:
    // what the watch has always done, and cheap (syncs are rare).
    if app.take_phone_alarms_changed() {
        storage::save(storage::key::PHONE_ALARMS, &app.phone_alarms.to_bytes()).await;
    }
}

// ==============================================================================
// The loop
// ==============================================================================

/// Local seconds since 1970 (UTC plus the phone's UTC offset), once the
/// phone has set the clock; what [`alarm`] works in.
pub fn local_second() -> Option<i64> {
    let utc = clock::now_utc_seconds()?;
    let offset = i64::from(clock::utc_offset_minutes()?);
    Some(utc + offset * 60)
}

/// What the loop keeps between passes, besides the UI's own state (the
/// [`App`]).
pub struct UiLoop {
    /// Public for the simulator's status line.
    pub battery: Battery,
    /// Whether the charger's power just arrived, to wake the screen.
    plug_in: PlugIn,
    /// When to show a glance on the charger, with the time to full.
    charge_glances: ChargeGlances,
    /// When the watch became full on the charger, to say so.
    full_charge: FullCharge,
    /// Whether the panel (and backlight) are on. It starts "asleep" so the
    /// first pass goes through the wake path: render the first frame, then
    /// turn the backlight on. (Waking an already-awake ST7789 is harmless.)
    panel_awake: bool,
    /// The panel's memory holds noise after power-up; sweeping the first
    /// frame in covers every block of it, and doubles as a retro boot
    /// effect.
    first_frame: bool,
    /// For battery-life analysis from the field log.
    next_battery_log: Instant,
    last_power_report: Instant,
    /// Whether the last frame was in the dim look; see [`UiLoop::pass`].
    was_dim: bool,
    /// The clock face's temperature (tenths of a degree), once read, and
    /// where from; and what goes into it (see `temperature`): the die's
    /// own, in quarter degrees, whether the watch is worn, and the
    /// estimate's calibration, which a nearby room sensor teaches.
    temperature: Option<(i16, temperature::Source)>,
    die: Option<i32>,
    wear: temperature::Wear,
    calibration: temperature::Calibration,
    pairing: temperature::Pairing,
    /// Whether the calibration has learnt since it was last saved, and
    /// when that was (see [`UiLoop::pass`]).
    calibration_unsaved: bool,
    calibration_saved: Instant,
    /// When the room sensor's reading last looked at was heard, so each is
    /// looked at once.
    sighting_seen: Option<u64>,
    /// [`geo::times_set`] as last looked at, to notice a new location.
    location_seen: u8,
    /// The battery as last advertised on its own to Home Assistant, for
    /// six buttons (see [`bthome::battery_due`]); `None` while they're not
    /// on for it, so turning them on sends it.
    battery_sent: Option<u8>,
    /// Gadgetbridge's live activity, while it asks (see [`live`]).
    live: live::Live,
}

impl UiLoop {
    /// Starts the UI, with what's in flash.
    pub fn boot(stored: Stored) -> (App, Self) {
        let now = Instant::now();
        let mut app = stored.settings.apply(now);
        app.phone_alarms = stored.phone_alarms;
        #[cfg(feature = "compare")]
        crate::compare::boot(now, &app);
        let ui = Self {
            battery: Battery::new(),
            plug_in: PlugIn::new(),
            charge_glances: ChargeGlances::new(),
            full_charge: FullCharge::new(),
            panel_awake: false,
            first_frame: true,
            // The first reading is still settling (it's smoothed); log
            // after a minute.
            next_battery_log: now + Duration::from_secs(60),
            last_power_report: now,
            was_dim: false,
            temperature: None,
            die: None,
            wear: temperature::Wear::new(now.as_millis()),
            calibration: stored.calibration,
            pairing: temperature::Pairing::new(),
            calibration_unsaved: false,
            calibration_saved: now,
            sighting_seen: None,
            location_seen: 0,
            battery_sent: None,
            live: live::Live::new(),
        };
        (app, ui)
    }

    /// Reads what the UI shows, then, if the screen is on, renders a full
    /// frame and pushes what changed; or switches the screen off.
    ///
    /// Returns why, if the watch switched off (critically low, or storage
    /// mode chosen); on the watch, [`Board::switch_off`] never returns.
    pub async fn pass(
        &mut self,
        app: &mut App,
        board: &mut impl Board,
        canvas: &mut Canvas,
    ) -> Option<Reason> {
        let now = Instant::now();
        let now_ms = now.as_millis();
        self.temperature(board, now, now_ms);
        // What the calibration has learnt, at most hourly: it may learn
        // every quarter hour, and a reset only loses what was learnt since.
        if self.calibration_unsaved
            && self.calibration_saved.elapsed() >= Duration::from_secs(60 * 60)
        {
            storage::save(storage::key::CALIBRATION, &self.calibration.to_bytes()).await;
            self.calibration_unsaved = false;
            self.calibration_saved = Instant::now();
        }
        let mut samples = [0i16; 1];
        board.sample_battery(&mut samples).await;
        let sample = debug::battery_sample().unwrap_or(samples[0]);
        let (charging, power_present) = board.charger();
        let uptime = now.as_secs() as u32;
        // For fitting the time to full (see `battery`) from field logs.
        if self.battery.charging && !charging && power_present {
            info!("charger: charged");
        }
        self.battery.update(sample, charging, power_present, uptime);
        // No heart rate on the charger (see `heart_schedule`'s "The
        // charger"), whether it's charging or full.
        heart::set_charger(power_present);
        // Set on the charger: the screen wakes, so the wearer sees at once,
        // from this very frame's battery, that it sits right on the cradle
        // (see `PlugIn` for a wobbly one).
        if self.plug_in.update(power_present, uptime) {
            #[cfg(feature = "compare")]
            crate::compare::charger(now);
            let woke = app.on_charger(now);
            info!("charger: plugged in; woke the screen: {=bool}", woke);
        }
        // Full, having charged there: the watch buzzes and says so, so it
        // can come off (see `battery::FullCharge`, and `App::on_charged`
        // for when it waits).
        let percent = self.battery.percent() as u8;
        if self
            .full_charge
            .update(charging, power_present, percent, uptime)
        {
            #[cfg(feature = "compare")]
            crate::compare::charged(now);
            let shown = app.on_charged(now);
            info!("charger: full; shown at once: {=bool}", shown);
        }
        // And every few minutes there, until full, a glance with the time
        // to full (see `battery::ChargeGlances`).
        let full = self.full_charge.full();
        if self.charge_glances.due(power_present, full, uptime) {
            let to_full = self.battery.to_full(uptime);
            let shown = app.charge_glance(now);
            if shown {
                #[cfg(feature = "compare")]
                crate::compare::charge_glance(now);
                // The minutes it said, or 0 once soon, 1000 if full (in
                // the moment before `FullCharge` says so).
                let minutes = match to_full {
                    Some(ToFull::Minutes(minutes)) => minutes,
                    Some(ToFull::Soon) | None => 0,
                    Some(ToFull::Full) => 1000,
                };
                info!("charger: glance, {=u16} min to full", minutes);
            }
            self.charge_glances.after(uptime);
        }
        let battery = &self.battery;
        let report = ble::BatteryReport {
            percent: battery.percent() as u8,
            charging: battery.charging,
            millivolts: battery.millivolts(),
        };
        ble::BATTERY.lock(|cell| cell.set(report));
        // Critically low, or storage mode chosen: save what the phone hasn't
        // fetched, and switch off until the charger (or the button) wakes
        // the watch.
        let off = if self.battery.critical(now.as_secs() as u32) {
            Some(Reason::BatteryEmpty)
        } else if app.storage_mode {
            Some(Reason::Storage)
        } else {
            None
        };
        if let Some(reason) = off {
            // Why, drawn here, where the UI's chosen theme is known.
            app.render_off(canvas, reason);
            board.switch_off(reason, canvas, self.panel_awake).await;
            return Some(reason);
        }
        // For battery-life analysis from the field log.
        if now >= self.next_battery_log {
            clock::log_now();
            if now - self.last_power_report >= Duration::from_secs(60 * 60) {
                power::report(now - self.last_power_report);
                self.last_power_report = now;
            }
            info!(
                "battery: {} mV, {}%, charging: {}",
                report.millivolts, report.percent, report.charging
            );
            self.next_battery_log = now + Duration::from_secs(10 * 60);
        }

        // Every pass, not only while the screen is on: its media gestures
        // work with it off too, and need to know there's a phone to send
        // them to.
        let connected = ble::CONNECTED.load(Ordering::Relaxed);
        app.set_connected(connected);
        self.live_activity(now, connected);
        let air = ble::NEARBY.lock(|nearby| {
            nearby
                .borrow()
                .best()
                .is_some_and(|s| now_ms.saturating_sub(s.heard) < aranet::RECENT)
        });
        app.set_air(air);
        app.set_charger(power_present, now);
        // Whatever the last wake did, and the phone or a sensor going
        // just now, the UI is in a state it can draw (and time out of).
        app.check();
        #[cfg(feature = "compare")]
        crate::compare::state(now, connected, air, power_present);
        // Apply screen power changes. Waking renders before the backlight
        // comes on, so the old frame never flashes up.
        if app.awake {
            if !self.panel_awake {
                board.wake_panel().await;
            }
            let temperature = self.temperature;
            let status = Status {
                uptime_secs: now.as_secs(),
                local_time: clock::now(),
                connected,
                unpaired: ble::UNPAIRED.lock(Cell::get),
                passkey: ble::PASSKEY.lock(Cell::get),
                update_progress: dfu::PROGRESS.lock(Cell::get),
                steps: activity::steps_today(),
                sleep: activity::last_sleep(),
                local_second: local_second(),
                heart_rate: heart::current(now),
                temperature: temperature.map(|(tenths, _)| tenths),
                temperature_from_sensor: temperature
                    .is_some_and(|(_, source)| source == temperature::Source::Sensor),
                // Read again rather than kept from above: kept, it would
                // sit in this task's future across every await between.
                air: ble::NEARBY.lock(|nearby| nearby.borrow().best()),
                battery: &self.battery,
                bthome: board.bthome(),
            };
            let daytime = board.daytime();
            app.set_daytime(daytime);
            // Into or out of the dim white-on-black look (a glance, a quiet
            // notification) with the panel lit: go dark for the redraw.
            // Otherwise the old frame (often a light theme, at full
            // backlight) flashes up while the new one is drawn over it.
            let relighting = self.panel_awake && app.dim() != self.was_dim;
            if relighting {
                board.backlight(brightness::Level::OFF);
            }
            self.was_dim = app.dim();
            // A frame to be shown in the dark (on waking, or relighting) is
            // shown out of idle mode, at the normal frame rate, so it's
            // whole by the time the backlight comes on (see
            // `Display::set_idle`); so is anything not in the dim look,
            // which needs all its colours.
            if !self.panel_awake || relighting || !app.dim() {
                board.set_idle(false).await;
            }
            canvas.set_palette(app.palette());
            // Nothing between this and the render may await: see
            // `compare::rendering`.
            #[cfg(feature = "compare")]
            crate::compare::rendering(now, &status, daytime, activity::today());
            app.render(canvas, &status, now);
            match app.take_transition() {
                // Waking straight into a transition would animate over a
                // stale frame; just show the new one.
                Some(sweep) if self.panel_awake => canvas.transition(board.display(), sweep).await,
                _ if self.first_frame => {
                    canvas.transition(board.display(), Sweep::LeftToRight).await
                }
                _ => canvas.flush(board.display()).await,
            }
            self.first_frame = false;
            #[cfg(feature = "compare")]
            crate::compare::rendered();
            // Light only a finished frame: after waking, the output is blank
            // until shown (see `Display::sleep`); after a redraw in the dark,
            // the panel's scan needs a refresh to reach the bottom rows.
            if !self.panel_awake {
                board.show_panel().await;
            } else if relighting {
                board.wait_for_refresh().await;
            }
            board.set_idle(app.dim()).await;
            // Low for a glance, and back up if it turns into a proper wake;
            // brightening while a wake alarm rings (see `brightness`).
            board.backlight(app.backlight(Instant::now()));
            if !self.panel_awake {
                self.panel_awake = true;
                power::on(power::Meter::Panel);
                power::on(power::Meter::Backlight);
            }
        } else if self.panel_awake {
            board.backlight(brightness::Level::OFF);
            power::off(power::Meter::Panel);
            power::off(power::Meter::Backlight);
            // In sleep the ST7789 keeps its frame memory, so the canvas's
            // idea of what's on the panel stays valid across sleep/wake.
            board.sleep_panel().await;
            self.panel_awake = false;
        }
        // Someone looking at the clock face gets a fresh heart rate, but one
        // is enough: the LED (3–8 mA) needn't stay on for the rest of the
        // glance once there's a reading from the last few minutes.
        //
        // Saturating: `now` was read at the top of this pass, and the heart
        // task may have recorded a reading during one of the awaits since,
        // timestamped after it. `now - at` then panicked (embassy-time's
        // `Instant` subtraction `unwrap!`s), which a field log caught right
        // after a redraw.
        let fresh = heart::latest()
            .is_some_and(|(_, at)| now.saturating_duration_since(at) < Duration::from_secs(5 * 60));
        heart::set_watching(app.showing_clock() && !fresh);
        // A new location from the phone: room sensors stay put, so if
        // it's far from where they were heard, stop looking for them (see
        // `aranet::Nearby::located`).
        let set = geo::times_set();
        if set != self.location_seen {
            self.location_seen = set;
            let here = geo::location();
            let here = aranet::Place {
                latitude_micro: here.latitude_micro,
                longitude_micro: here.longitude_micro,
            };
            if ble::NEARBY.lock(|nearby| nearby.borrow_mut().located(here, now_ms)) {
                info!("aranet: far from the sensors; forgot them");
            }
        }
        // Listen for room sensors, when it's time (see `aranet::Nearby::due`).
        let context = aranet::Context {
            quiet_night: app.quiet_night(),
            off_wrist: !self.wear.worn(),
            showing_clock: app.showing_clock(),
            showing_air: app.showing_air(),
            updating: dfu::PROGRESS.lock(Cell::get).is_some(),
        };
        let due = ble::NEARBY.lock(|nearby| {
            let mut nearby = nearby.borrow_mut();
            let due = nearby.due(now_ms, context);
            if due.is_some() {
                nearby.begin(now_ms);
            }
            due
        });
        if let Some(why) = due {
            debug!("aranet: scanning ({})", why);
            ble::SCAN.signal(());
        }
        glance::set_screen_on(app.awake);
        // Taps don't wake the screen in the quiet night (see `ui`), so the
        // touch controller can sleep while it's off.
        touch::set_asleep(app.quiet_night() && !app.awake);
        board.pet_watchdog();
        clock::checkpoint();
        None
    }

    /// Gadgetbridge's live activity (see [`live`]): what it asked for
    /// since the last pass, its report if one's due, and whether the heart
    /// rate sensor should run for it.
    fn live_activity(&mut self, now: Instant, connected: bool) {
        let seconds = now.as_secs() as u32;
        let steps = activity::steps_counted();
        if let Some(request) = live::take() {
            if request.wanted() {
                info!(
                    "live: heart rate {}, steps {}, every {} s",
                    request.heart_rate, request.steps, request.interval
                );
            }
            if let Some(ended) = self.live.ask(request, seconds, steps) {
                info!("live: ended ({=str})", ended.as_str());
            }
        }
        let reading = heart::latest().map(|(bpm, at)| (bpm, at.as_secs() as u32));
        match self.live.poll(seconds, connected, steps, reading) {
            live::Poll::Idle => {}
            live::Poll::Report(report) => {
                // Dropped if the queue's full: the next one is due soon.
                let _ = ble::OUTBOX.try_send(gb::Outgoing::Live {
                    steps: report.steps,
                    heart_rate: report.heart_rate,
                });
            }
            live::Poll::Ended(ended) => info!("live: ended ({=str})", ended.as_str()),
        }
        heart::set_live(self.live.heart_rate());
    }

    /// The temperature for the clock face: from a nearby room sensor, the
    /// wrist's estimate, or the die's own (see `temperature::pick`); and
    /// what a room sensor's reading teaches the estimate.
    ///
    /// Not an `async fn` (saving what it learnt is the pass's): a state
    /// machine of its own would cost flash.
    fn temperature(&mut self, board: &mut impl Board, now: Instant, now_ms: u64) {
        self.wear.update(
            now_ms,
            heart::worn() == Some(false),
            activity::still(),
            heart::latest().map(|(_, at)| at.as_millis()),
        );
        // At the top of each minute of uptime, rather than at a stored
        // time, to save the RAM. A pass delayed past that second just
        // waits for the next minute, and an extra pass within it (an input)
        // reads again, which is harmless.
        let sample = self.die.is_none() || now.as_secs().is_multiple_of(temperature::SAMPLE_EVERY);
        if sample {
            self.die = Some(board.die_temperature());
        }
        let (best, fresh) = ble::NEARBY.lock(|nearby| {
            let nearby = nearby.borrow();
            (nearby.best(), nearby.fresh(now_ms))
        });
        let heard = best.map(|s| s.heard);
        let new_sighting = heard != self.sighting_seen;
        self.sighting_seen = heard;
        if let Some(die) = self.die
            && (sample || new_sighting)
        {
            // A new room sensor's reading (an Aranet4's or a BTHome
            // thermometer's: both measure the air), recent enough to pair
            // with the die's temperature now, on a settled wrist: learn
            // from it.
            if new_sighting
                && let Some(sighting) = best
                && sighting.age(now_ms) <= temperature::PAIR_AGE
                && let Some(air) = sighting.reading.quarters()
                && self
                    .pairing
                    .offer(now_ms, die, air, self.wear.worn_for(now_ms))
            {
                self.calibration.learn(die, air);
                self.calibration_unsaved = true;
                let (k, c) = self.calibration.line();
                info!(
                    "temperature: learnt die {}/4 C, air {}/4 C; slope {}, height {}, {} pairs",
                    die,
                    air,
                    k,
                    c,
                    self.calibration.pairs()
                );
            }
            let sensor = fresh.and_then(|s| s.reading.hundredths).map(i32::from);
            let (hundredths, source) =
                temperature::pick(sensor, self.wear.worn(), die, &self.calibration);
            // A change of source shows the new value at once, rather than
            // holding the old within a degree (or a tenth).
            let previous = self
                .temperature
                .filter(|&(_, s)| s == source)
                .map(|(d, _)| d);
            let shown = (temperature::shown(previous, hundredths, source), source);
            if self.temperature != Some(shown) {
                info!(
                    "temperature: {}/10 C ({}), die {}/4 C",
                    shown.0, source, die
                );
            }
            self.temperature = Some(shown);
        }
    }

    /// Acts on what woke the loop; the future it returns sends the button
    /// grid's presses, and saves whatever settings that changed.
    ///
    /// Not an `async fn`, so that the future doesn't keep `wake` (an input,
    /// with a notification's text) while it saves: it's static RAM.
    pub fn handle<'a, B: Board>(
        &'a mut self,
        app: &'a mut App,
        board: &'a mut B,
        wake: Wake,
    ) -> impl Future<Output = ()> + 'a {
        let before = Settings::of(app);
        match wake {
            Wake::Timer => {
                // Maybe only a frame; then this does nothing.
                let (at, finger_down) = (Instant::now(), board.finger_down());
                #[cfg(feature = "compare")]
                crate::compare::timer(at, finger_down);
                app.on_timer(at, finger_down);
                // Whatever was due is done, or no longer waits: were it
                // still due, the loop's timer would fire again at once,
                // and again, spinning (and starving every other task)
                // for as long as that lasted. See `App::deadline` for
                // the cases that could.
                debug_assert!(
                    app.deadline(finger_down).is_none_or(|due| due > at),
                    "a timer that's still due once handled"
                );
            }
            Wake::Tick => {
                // A stir after stillness, in light sleep: what a smart
                // alarm waits for.
                let wearer = alarm::Wearer {
                    stirring: activity::movement() >= sleep::STILL_LIMIT,
                    light: activity::light(),
                };
                let (at, second, asleep) = (Instant::now(), local_second(), activity::asleep());
                app.check_alarms(at, second, wearer, asleep);
                // Pairing and updates need the user to watch the screen.
                let watching = ble::PASSKEY.lock(Cell::get).is_some()
                    || dfu::PROGRESS.lock(Cell::get).is_some();
                // A finger held on keeps the screen on (see `App::on_tick`).
                let finger_down = board.finger_down();
                let ticked = Instant::now();
                #[cfg(feature = "compare")]
                crate::compare::tick(at, second, wearer, asleep, ticked, watching, finger_down);
                app.on_tick(ticked, watching, finger_down)
            }
            Wake::Input(event) => {
                info!("input: {}", event);
                // What woke the screen, if this does: not every input does
                // (a single tap, say; see `wake::follows`).
                let wake = match event {
                    InputEvent::Button => power::Count::WakeButton,
                    InputEvent::Touch(_) => power::Count::WakeTouch,
                    InputEvent::Phone(_) => power::Count::WakePhone,
                    InputEvent::Attention => power::Count::WakeAttention,
                    InputEvent::Glance => power::Count::WakeGlance,
                };
                let was_awake = app.awake;
                let at = Instant::now();
                #[cfg(feature = "compare")]
                crate::compare::input(&event, at);
                app.on_input(event, at);
                if !was_awake && app.awake {
                    power::count(wake);
                }
            }
        }
        // Each wake leaves the UI's state whole (see `App::check`): here,
        // right after the wake that broke it, if one did.
        app.check();
        let battery = self.battery.percent() as u8;
        // Six buttons' presses leave the battery out (see `bthome`), so
        // Home Assistant hears it on its own: once the reading has had a
        // minute from boot to settle, or as soon as they're turned on for
        // it, then at each 5% step. Sent from here rather than from
        // `pass`, as this future already sends presses: there, it would
        // grow the UI loop's future (static RAM).
        let mut battery_due = false;
        if !(app.buttons.bthome() && app.grid_layout == buttons::Layout::Six) {
            self.battery_sent = None;
        } else if Instant::now().as_secs() >= 60 && bthome::battery_due(self.battery_sent, battery)
        {
            info!("bthome: battery {=u8}%", battery);
            self.battery_sent = Some(battery);
            battery_due = true;
        }
        async move {
            // The button grid's presses for Home Assistant, from an input
            // or its wait running out; the grid shows those that couldn't
            // go, before the pass renders it.
            while let Some(press) = app.take_ha_press() {
                let sent = board.advertise(Some(press), app.grid_layout, battery).await;
                // (Microseconds before `ha_unsent`'s instant, as the
                // simulator has it: a frame between them is all but
                // impossible.)
                #[cfg(feature = "compare")]
                crate::compare::ha_press(Instant::now(), press, sent);
                if !sent {
                    app.ha_unsent(press, Instant::now());
                }
            }
            // TODO: a battery reading that couldn't go waits for the next
            // 5% step, as if it had.
            if battery_due {
                board.advertise(None, app.grid_layout, battery).await;
            }
            save_changes(before, app).await;
        }
    }
}
