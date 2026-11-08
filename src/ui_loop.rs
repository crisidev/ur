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

use crate::battery::Battery;
use crate::bthome::{self, Identity};
use crate::buttons::Press;
use crate::canvas::{self, Canvas, Sweep};
use crate::display::Display;
use crate::heart_reading as heart;
use crate::system_off::Reason;
use crate::touch::Touch;
use crate::ui::{App, Status, ThemeChoice};
use crate::{
    activity, alarm, aranet, ble, brightness, buttons, clock, debug, dfu, geo, glance, power,
    sleep, storage, temperature, touch, ui,
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
/// something due (see `App::deadline`), unless a finger is down
/// (`finger_down`): that touch decides, and its event wakes the loop.
pub fn timer_at(app: &App, finger_down: bool) -> Option<Instant> {
    let frame = app
        .animating()
        .then(|| Instant::now() + canvas::MARQUEE_FRAME);
    let due = app.deadline().filter(|_| !finger_down);
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
    /// with none, the battery alone (`bthome::send`).
    fn advertise(
        &mut self,
        press: Option<Press>,
        layout: buttons::Layout,
        battery: u8,
    ) -> impl Future<Output = ()>;

    /// Switches the watch off ([`crate::system_off::shut_down`]), showing
    /// why. On the watch this never returns: the next thing that happens is
    /// a boot.
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
    brightness: brightness::Setting,
    buttons: buttons::Setting,
    grid: buttons::Layout,
    alarms: alarm::Settings,
}

impl Settings {
    pub fn of(app: &App) -> Self {
        Self {
            themes: app.themes,
            brightness: app.brightness,
            buttons: app.buttons,
            grid: app.grid_layout,
            alarms: app.alarms,
        }
    }

    /// The UI as it boots, at `now`, with these settings.
    fn apply(self, now: Instant) -> App {
        let mut app = App::new(now, self.themes, self.alarms);
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
        let themes = ThemeChoice {
            light: load_u8(storage::key::THEME_LIGHT).await,
            dark: load_u8(storage::key::THEME_DARK).await,
        };
        let alarms = storage::load(storage::key::ALARMS)
            .await
            .map(alarm::Settings::from_bytes)
            .unwrap_or_default();
        let phone_alarms = storage::load(storage::key::PHONE_ALARMS)
            .await
            .map(alarm::Phone::from_bytes)
            .unwrap_or_default();
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
    if app.themes.light != before.themes.light {
        storage::save(storage::key::THEME_LIGHT, &[app.themes.light]).await;
    }
    if app.themes.dark != before.themes.dark {
        storage::save(storage::key::THEME_DARK, &[app.themes.dark]).await;
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
    /// Since when the panel (and backlight) have been on, for `power`.
    panel_since: Instant,
    /// Whether the last frame was in the dim look; see [`UiLoop::pass`].
    was_dim: bool,
    /// The clock face's temperature, once read, and where from; and what
    /// goes into it (see `temperature`): the die's own, in quarter degrees,
    /// whether the watch is worn, and the estimate's calibration, which a
    /// nearby room sensor teaches.
    temperature: Option<(i8, temperature::Source)>,
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
            panel_awake: false,
            first_frame: true,
            // The first reading is still settling (it's smoothed); log
            // after a minute.
            next_battery_log: now + Duration::from_secs(60),
            last_power_report: now,
            panel_since: now,
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
        self.battery.update(sample, charging, power_present);
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
            board.switch_off(reason, canvas, self.panel_awake).await;
            return Some(reason);
        }
        // For battery-life analysis from the field log.
        if now >= self.next_battery_log {
            clock::log_now();
            if now - self.last_power_report >= Duration::from_secs(60 * 60) {
                if self.panel_awake {
                    power::add(power::Meter::Panel, self.panel_since);
                    power::add(power::Meter::Backlight, self.panel_since);
                    self.panel_since = now;
                }
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
        let air = ble::NEARBY.lock(|nearby| {
            nearby
                .borrow()
                .best()
                .is_some_and(|s| now_ms.saturating_sub(s.heard) < aranet::RECENT)
        });
        app.set_air(air);
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
                temperature: temperature.map(|(degrees, _)| degrees),
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
            canvas.set_theme(app.theme());
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
                self.panel_since = Instant::now();
            }
        } else if self.panel_awake {
            board.backlight(brightness::Level::OFF);
            power::add(power::Meter::Panel, self.panel_since);
            power::add(power::Meter::Backlight, self.panel_since);
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
            let sensor = fresh.and_then(|s| s.reading.quarters());
            let (quarters, source) =
                temperature::pick(sensor, self.wear.worn(), die, &self.calibration);
            // A change of source shows the new value at once, rather than
            // holding the old within a degree.
            let previous = self
                .temperature
                .filter(|&(_, s)| s == source)
                .map(|(d, _)| d);
            let shown = (temperature::shown(previous, quarters), source);
            if self.temperature != Some(shown) {
                info!("temperature: {} C ({}), die {}/4 C", shown.0, source, die);
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
            }
            Wake::Tick => {
                // A stir after stillness: what a smart alarm waits for.
                let stirring = activity::movement() >= sleep::STILL_LIMIT;
                let (at, second, asleep) = (Instant::now(), local_second(), activity::asleep());
                app.check_alarms(at, second, stirring, asleep);
                // Pairing and updates need the user to watch the screen.
                let watching = ble::PASSKEY.lock(Cell::get).is_some()
                    || dfu::PROGRESS.lock(Cell::get).is_some();
                let ticked = Instant::now();
                #[cfg(feature = "compare")]
                crate::compare::tick(at, second, stirring, asleep, ticked, watching);
                app.on_tick(ticked, watching)
            }
            Wake::Input(event) => {
                info!("input: {}", event);
                // What woke the screen, if this does: not every input does
                // (a single tap, say; see `ui::DOUBLE_TAP`).
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
        let battery = self.battery.percent() as u8;
        // Six buttons' presses leave the battery out (see `bthome`), so
        // Home Assistant hears it on its own: once the reading has had a
        // minute from boot to settle, or as soon as they're turned on for
        // it, then at each 5% step. Sent from here rather than from
        // `pass`, as this future already sends presses: there, it would
        // grow the UI loop's future (static RAM).
        let mut battery_due = false;
        if !(app.buttons.ha() && app.grid_layout == buttons::Layout::Six) {
            self.battery_sent = None;
        } else if Instant::now().as_secs() >= 60 && bthome::battery_due(self.battery_sent, battery)
        {
            info!("bthome: battery {=u8}%", battery);
            self.battery_sent = Some(battery);
            battery_due = true;
        }
        async move {
            // The button grid's presses for Home Assistant, from an input
            // or its wait running out.
            while let Some(press) = app.take_ha_press() {
                board.advertise(Some(press), app.grid_layout, battery).await;
            }
            if battery_due {
                board.advertise(None, app.grid_layout, battery).await;
            }
            save_changes(before, app).await;
        }
    }
}
