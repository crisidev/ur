//! The simulated watch: the firmware's UI loop ([`crate::ui_loop`]),
//! against a clock we control.
//!
//! On the watch, `main.rs` runs the UI loop: it wakes once a second (a
//! tick), on every input, and when its timer is due (scrolling text, the
//! button grid), has [`UiLoop::handle`] act on that, then has
//! [`UiLoop::pass`] render a frame (if the screen is on) and push what
//! changed to the panel. Here the same code runs, in the same order, but
//! time only moves when [`Watch::advance`] moves it (see [`crate::time`]):
//! a script's hour passes in milliseconds, and the same script gives the
//! same frames every time.
//!
//! What's left here is what `main.rs` has instead: deciding when the next
//! tick or timer is due (on the watch, a ticker and a timer do), and the
//! hardware the loop drives ([`Hardware`], stand-ins for `main.rs`'s), plus
//! what the simulator itself does: the finger, the motor, switching off
//! and back on, and saying what happens.

use std::future::Future;
use std::pin::pin;
use std::task::{Context, Poll, Waker};

use embassy_time::{Duration, Instant, Timer};

use crate::aranet::Sighting;
use crate::brightness;
use crate::bthome::{self, Identity};
use crate::buttons::Press;
use crate::canvas::Canvas;
use crate::case::Pulse;
use crate::clock;
use crate::display::{Display, PANEL};
use crate::flash::Flash;
use crate::geo;
use crate::gesture::Tracking;
use crate::heart_reading;
use crate::motor::{self, Pattern};
use crate::storage;
use crate::system_off::Reason;
use crate::theme::Theme;
use crate::touch;
use crate::ui::{self, App, Unpaired};
use crate::ui_loop::{self, Board, Settings, Stored, UiLoop, Wake};
use crate::{InputEvent, ble, dfu};

pub use crate::ui_loop::local_second;

/// How long the switching-off screen shows (`system_off::SHOW_FOR`).
const OFF_SCREEN: Duration = Duration::from_secs(3);

/// The battery's voltage until told otherwise: about 70%.
const DEFAULT_BATTERY_MV: u32 = 3990;

/// The simulated watch's key for Home Assistant (bthome.io's example), and
/// its address (the README's example).
pub const DEMO_IDENTITY: Identity = Identity {
    key: [
        0x23, 0x1d, 0x39, 0xc1, 0xd7, 0xcc, 0x1a, 0xb1, 0xae, 0xe2, 0x24, 0xcd, 0x09, 0x6d, 0xb9,
        0x32,
    ],
    address: [0x9A, 0x78, 0x56, 0x34, 0x12, 0xE5],
};

pub struct Watch {
    pub app: App,
    ui: UiLoop,
    canvas: Box<Canvas>,
    display: Display,

    // What the hardware would tell the UI loop.
    pub battery_mv: u32,
    pub charging: bool,
    pub power_present: bool,
    /// The die's temperature, in °C.
    pub temperature: i8,
    /// Day or night, whatever the sun says (see `theme`).
    pub daytime: Option<bool>,

    // What the Bluetooth task would tell it (see `publish`).
    pub connected: bool,
    pub unpaired: Option<Unpaired>,
    pub passkey: Option<u32>,
    pub update_progress: Option<u8>,

    /// Whether the first pass has run (see `power_on`).
    started: bool,
    next_tick: Instant,
    /// When the UI loop's timer is next due, as the last pass left it
    /// (see [`ui_loop::timer_at`]).
    timer: Option<Instant>,

    /// The backlight, as the UI loop last set it; `OFF` with the screen off.
    pub backlight: brightness::Level,
    /// What the motor is playing, and since when.
    pub motor: Option<(Pattern, Instant)>,
    /// The finger on the screen (from a script's `press`, or the mouse), as
    /// the touch task tracks it.
    finger: Option<Tracking>,
    /// The watch switched itself off (storage mode, or an empty battery):
    /// only the button (storage mode) or the charger wakes it.
    pub off: Option<Reason>,
    /// The phone's time when the watch switched off, and when that was:
    /// what it sets again when it reconnects after the watch boots.
    phone_time: Option<(i64, i16, Instant)>,
    /// When the clock started again from 0 (a reboot), on the old clock:
    /// for [`Watch::advance`], to carry on for what's left of its wait.
    restarted_at: Option<Instant>,
    /// The BTHome counter (see `bthome::send`), for the advertisements
    /// printed.
    bthome_counter: u32,

    /// Called while a transition animates, after each millisecond of it,
    /// for the window to show its frames.
    pub on_frame: Option<Box<dyn FnMut()>>,
    /// Print each tick, not only what happens.
    pub verbose: bool,
}

impl Watch {
    /// A watch about to boot, with `flash` (its settings, if any): no time,
    /// no phone. Its first frame sweeps in at the first command that shows
    /// anything, or once time passes, so a script's first lines can set up
    /// what it boots into (say, the battery's first reading).
    pub fn boot(flash: Flash) -> Self {
        storage::init(flash);
        let (app, ui) = start();
        let now = Instant::now();
        Self {
            app,
            ui,
            canvas: Box::new(Canvas::new()),
            display: Display,
            battery_mv: DEFAULT_BATTERY_MV,
            charging: false,
            power_present: false,
            temperature: 31,
            daytime: None,
            connected: false,
            unpaired: None,
            passkey: None,
            update_progress: None,
            started: false,
            next_tick: now + ui_loop::TICK,
            timer: None,
            backlight: brightness::Level::OFF,
            motor: None,
            finger: None,
            off: None,
            phone_time: None,
            restarted_at: None,
            bthome_counter: 256,
            on_frame: None,
            verbose: false,
        }
    }

    /// Shows the first frame, if it hasn't been yet.
    fn power_on(&mut self) {
        if !self.started {
            self.pass();
        }
    }

    /// Boots again after switching off, as waking from System OFF does:
    /// RAM is lost (the time, the heart rate, the Aranet4s heard, uptime),
    /// the settings in flash are what the UI starts with, and the phone, if
    /// it's there, reconnects and sets the time again.
    ///
    /// Kept, as the watch keeps them in its SPI flash: today's steps and
    /// the activity history (`system_off`'s record). Kept as they are, as
    /// the world outside the watch: the phone's connection, and the
    /// location (which the watch saves in flash when the phone sends one;
    /// the simulator doesn't yet).
    ///
    /// TODO: save `location`s and `gb` GPS fixes to flash as `ble` does,
    /// and forget the location here, for a reboot that loads it.
    fn reboot(&mut self) {
        let off_for = self.phone_time.map(|(_, _, at)| Instant::now() - at);
        self.restarted_at = Some(Instant::now());
        clock::lose();
        heart_reading::forget();
        ble::NEARBY.lock(|nearby| *nearby.borrow_mut() = crate::aranet::Nearby::new());
        crate::time::restart();
        self.passkey = None;
        self.update_progress = None;
        // As `main`, at boot.
        clock::restore();
        (self.app, self.ui) = start();
        self.off = None;
        self.motor = None;
        // Its `Instant` is from before the restart.
        crate::record::motor(None);
        self.finger = None;
        self.started = false;
        self.next_tick = Instant::now() + ui_loop::TICK;
        self.timer = None;
        self.log("booted");
        if let (true, Some((utc, offset, _)), Some(off_for)) =
            (self.connected, self.phone_time.take(), off_for)
        {
            clock::set(utc + off_for.as_secs() as i64, offset);
            self.log("the phone reconnected, and set the time");
        }
        self.pass();
    }

    pub fn log(&self, message: impl AsRef<str>) {
        log(message);
    }

    // ==========================================================================
    // Time
    // ==========================================================================

    /// Lets `duration` pass: every tick, timer, and long press due in it
    /// happens, in order.
    pub fn advance(&mut self, duration: Duration) {
        self.power_on();
        // A reboot before this wait (the button's) has nothing to carry on.
        self.restarted_at = None;
        let mut end = Instant::now() + duration;
        loop {
            // Rebooted (by the charger, at a tick): the clock started again
            // from 0, and the wait goes on for what was left of it.
            if let Some(at) = self.restarted_at.take() {
                end = Instant::from_ticks(0) + (end - at);
            }
            let press = self.finger.as_ref().and_then(Tracking::long_press_at);
            let next = [Some(self.next_tick), self.timer, press]
                .into_iter()
                .flatten()
                .min()
                .expect("the tick is always due");
            if next > end {
                set_time(end);
                return;
            }
            set_time(next);
            if press == Some(next) {
                self.long_press();
            } else if next == self.next_tick {
                self.next_tick += ui_loop::TICK;
                self.tick();
            } else {
                self.handle(Wake::Timer);
                self.after_event();
                self.pass();
            }
        }
    }

    /// Lets time pass until the local wall-clock time is next `hour:minute`,
    /// and the UI loop's tick has seen it (the ticks count seconds from
    /// boot, not from the wall clock's minutes, so that's up to a second
    /// later).
    pub fn advance_to_local(&mut self, hour: u8, minute: u8) -> Result<(), String> {
        let now = local_second().ok_or("the time isn't set (see `time`)")?;
        let target = i64::from(hour) * 3600 + i64::from(minute) * 60;
        let wait = (target - now.rem_euclid(86_400)).rem_euclid(86_400);
        // A wait of 0 means "it's that minute now"; go to the next one.
        let wait = if wait == 0 { 86_400 } else { wait };
        self.advance(Duration::from_secs(wait as u64));
        self.advance(self.next_tick - Instant::now());
        Ok(())
    }

    fn tick(&mut self) {
        if self.off.is_some() {
            return self.off_tick();
        }
        let night = self.app.quiet_night();
        self.handle(Wake::Tick);
        if self.app.quiet_night() != night {
            self.log(format!("quiet night: {}", self.app.quiet_night()));
        }
        if self.verbose {
            self.log("tick");
        }
        self.after_event();
        self.pass();
    }

    // ==========================================================================
    // Inputs
    // ==========================================================================

    /// One input, handled as the UI loop does, then a frame.
    pub fn input(&mut self, event: InputEvent) {
        self.power_on();
        if let Some(reason) = self.off {
            // Off, only the charger wakes the watch, or in storage mode
            // the button too.
            if reason == Reason::Storage && matches!(event, InputEvent::Button) {
                return self.reboot();
            }
            return self.log("ignored: the watch is off");
        }
        self.log(format!("input: {}", describe(&event)));
        let themes = self.app.themes;
        let brightness = self.app.brightness;
        let buttons = self.app.buttons;
        self.handle(Wake::Input(event));
        if self.app.themes != themes {
            self.log(format!("theme: {}", self.app.theme().name));
        }
        if self.app.brightness != brightness {
            self.log(format!("brightness: {}", self.app.brightness.name()));
        }
        if self.app.buttons != buttons {
            self.log(format!("buttons: {}", self.app.buttons.name()));
        }
        self.after_event();
        self.pass();
    }

    /// A finger down at, or moved to, (`x`, `y`) in panel pixels (0–239),
    /// as the touch controller would report it: not at all while the UI
    /// loop has it asleep (see `touch::set_asleep`).
    pub fn finger_at(&mut self, x: i32, y: i32) {
        if touch::asleep() || self.off.is_some() {
            return;
        }
        let at = (x.clamp(0, 239), y.clamp(0, 239));
        match &mut self.finger {
            Some(finger) => finger.moved_to(at),
            None => self.finger = Some(Tracking::new(at)),
        }
    }

    /// The finger lifted: what it did is decided now, unless it was a long
    /// press, which was reported while it was down; its lift is a release.
    pub fn lift(&mut self) {
        let Some(finger) = self.finger.take() else {
            return;
        };
        if finger.held {
            self.input(InputEvent::Touch(finger.release()));
        } else {
            self.input(InputEvent::Touch(finger.classify()));
        }
    }

    fn long_press(&mut self) {
        let finger = self
            .finger
            .as_mut()
            .expect("only due while a finger is down");
        finger.held = true;
        let touch = finger.classify();
        self.input(InputEvent::Touch(touch));
    }

    /// What every event may have set off: buzzes, and messages for the
    /// phone.
    fn after_event(&mut self) {
        while let Some(pattern) = motor::take_request() {
            self.log(format!("buzz: {}", pattern_name(pattern)));
            self.motor = (pattern != Pattern::STOP).then(|| (pattern, Instant::now()));
            // For `--watch` recordings to shake the watch, as the window
            // does: the recorder can't reach the watch.
            crate::record::motor(self.motor);
        }
        while let Ok(message) = ble::OUTBOX.try_receive() {
            let line = message.to_line();
            let line = line.trim_end();
            if self.connected {
                self.log(format!("to the phone: {line}"));
            } else {
                self.log(format!("to the phone, but not connected: {line}"));
            }
        }
    }

    /// Whether the motor is playing a pattern: one that repeats until
    /// stopped, or one that hasn't finished.
    pub fn buzzing(&self) -> bool {
        self.motor.is_some_and(|(pattern, since)| {
            let length: u64 = pattern.steps().iter().map(|&ms| u64::from(ms)).sum();
            pattern.repeat_from().is_some() || (Instant::now() - since).as_millis() < length
        })
    }

    /// Where the motor is in its pattern now, to shake the drawn watch by
    /// (see [`motor_pulse`]).
    #[cfg(any(feature = "window", target_arch = "wasm32"))]
    pub fn motor_pulse(&self) -> Option<Pulse> {
        motor_pulse(self.motor, Instant::now())
    }

    // ==========================================================================
    // The UI loop
    // ==========================================================================

    /// What the Bluetooth and update tasks would have told the UI loop, in
    /// their stand-ins, where it reads them.
    fn publish(&self) {
        ble::CONNECTED.store(self.connected, std::sync::atomic::Ordering::Relaxed);
        ble::UNPAIRED.lock(|cell| cell.set(self.unpaired));
        ble::PASSKEY.lock(|cell| cell.set(self.passkey));
        dfu::PROGRESS.lock(|cell| cell.set(self.update_progress));
    }

    /// The stand-ins for `main.rs`'s hardware, over the watch's state; and
    /// what the UI loop takes beside them, and `on_frame`, to run it with.
    #[allow(clippy::type_complexity)]
    fn hardware(
        &mut self,
    ) -> (
        &mut App,
        &mut UiLoop,
        &mut Canvas,
        &mut Option<Box<dyn FnMut()>>,
        Hardware<'_>,
    ) {
        let Self {
            app,
            ui,
            canvas,
            display,
            backlight,
            on_frame,
            battery_mv,
            charging,
            power_present,
            temperature,
            daytime,
            finger,
            bthome_counter,
            ..
        } = self;
        let board = Hardware {
            display,
            backlight,
            battery_mv: *battery_mv,
            charger: (*charging, *power_present),
            temperature: *temperature,
            daytime: *daytime,
            finger_down: finger.is_some(),
            bthome_counter,
        };
        (app, ui, canvas, on_frame, board)
    }

    /// Has the UI loop act on what woke it, and save what that changed.
    fn handle(&mut self, wake: Wake) {
        self.publish();
        let (app, ui, _, on_frame, mut board) = self.hardware();
        run(ui.handle(app, &mut board, wake), on_frame);
    }

    /// A pass of the UI loop: what it does each time it wakes, after
    /// handling what woke it.
    pub fn pass(&mut self) {
        if self.off.is_some() {
            return;
        }
        self.started = true;
        self.publish();
        let (app, ui, canvas, on_frame, mut board) = self.hardware();
        if let Some(reason) = run(ui.pass(app, &mut board, canvas), on_frame) {
            self.off = Some(reason);
            self.finger = None;
            self.timer = None;
            // The clock stops while the watch is off; the phone's doesn't.
            self.phone_time = clock::now_utc_seconds()
                .zip(clock::utc_offset_minutes())
                .map(|(utc, offset)| (utc, offset, Instant::now()));
            clock::forget();
            self.log("off");
            return;
        }
        if touch::asleep() {
            // The controller's asleep: whatever was in progress is moot.
            self.finger = None;
        }
        self.timer = ui_loop::timer_at(&self.app, self.finger.is_some());
    }

    /// Switched off: only the charger (or, in storage mode, the button)
    /// wakes the watch.
    fn off_tick(&mut self) {
        if self.power_present {
            self.reboot();
        }
    }

    /// Changes settings as the UI would (rather than through it, as a
    /// script's `alarm` does), and saves them as the UI loop does.
    pub fn change_settings(&mut self, change: impl FnOnce(&mut App)) {
        let before = Settings::of(&self.app);
        change(&mut self.app);
        run(ui_loop::save_changes(before, &mut self.app), &mut None);
        self.refresh();
    }

    /// A room sensor's reading (an Aranet4's, or a BTHome thermometer's),
    /// as a scan hears it (`None`: none heard since boot).
    pub fn hear_aranet(&mut self, sighting: Option<Sighting>) {
        ble::NEARBY.lock(|nearby| {
            let mut nearby = nearby.borrow_mut();
            let Some(sighting) = sighting else {
                *nearby = crate::aranet::Nearby::new();
                return;
            };
            let now = Instant::now().as_millis();
            nearby.begin(now);
            nearby.hear(sighting.address, sighting.rssi, sighting.reading, now);
            nearby.end(now);
        });
        self.refresh();
    }

    /// Saves what the panel shows as a PNG. With `palette`, in that theme's
    /// colours instead, as `scripts/screenshot.py --theme` does: the canvas
    /// is pushed again in them, saved, then put back.
    pub fn screenshot(
        &mut self,
        path: &std::path::Path,
        palette: Option<&'static Theme>,
    ) -> Result<(), String> {
        let Some(palette) = palette else {
            return crate::shot::save(path);
        };
        let Self {
            canvas,
            display,
            on_frame,
            app,
            ..
        } = self;
        canvas.set_theme(palette);
        run(canvas.flush(display), on_frame);
        let saved = crate::shot::save(path);
        canvas.set_theme(app.theme());
        run(canvas.flush(display), on_frame);
        saved
    }

    /// Sets the theme the UI picks: the group the theme is in (light or
    /// dark) from now on, whatever the sun says, and that theme within it.
    pub fn choose_theme(&mut self, name: &str) -> Result<(), String> {
        let theme = find_theme(name)?;
        let find = |group: &[&'static Theme]| group.iter().position(|t| core::ptr::eq(*t, theme));
        if let Some(i) = find(&crate::theme::LIGHT) {
            self.daytime = Some(true);
            self.change_settings(|app| app.themes.light = i as u8);
        } else if let Some(i) = find(&crate::theme::DARK) {
            self.daytime = Some(false);
            self.change_settings(|app| app.themes.dark = i as u8);
        }
        Ok(())
    }

    /// Something changed outside the UI loop's own events (a sensor, the
    /// phone's connection): show it, as the next pass would.
    pub fn refresh(&mut self) {
        self.after_event();
        self.pass();
    }
}

/// What's in flash, and the UI it starts.
fn start() -> (App, UiLoop) {
    UiLoop::boot(run(Stored::load(), &mut None))
}

// ==============================================================================
// The hardware
// ==============================================================================

/// Stand-ins for what `main.rs`'s `Hardware` drives: they keep what
/// they're told, for the window, screenshots and `expect`.
struct Hardware<'a> {
    display: &'a mut Display,
    backlight: &'a mut brightness::Level,
    battery_mv: u32,
    charger: (bool, bool),
    temperature: i8,
    daytime: Option<bool>,
    finger_down: bool,
    bthome_counter: &'a mut u32,
}

impl Hardware<'_> {
    fn set_backlight(&mut self, level: brightness::Level) {
        *self.backlight = level;
        PANEL.with_borrow_mut(|panel| panel.backlight = level);
    }
}

impl Board for Hardware<'_> {
    fn die_temperature(&mut self) -> i32 {
        i32::from(self.temperature) * 4
    }

    async fn sample_battery(&mut self, sample: &mut [i16; 1]) {
        // What the SAADC reads through the 1:2 divider (as
        // `debug::battery_sample`).
        sample[0] = (self.battery_mv * 4096 / (3600 * 2)) as i16;
    }

    fn charger(&mut self) -> (bool, bool) {
        self.charger
    }

    fn daytime(&mut self) -> bool {
        self.daytime.unwrap_or_else(geo::is_daytime)
    }

    fn finger_down(&mut self) -> bool {
        self.finger_down
    }

    fn display(&mut self) -> &mut Display {
        self.display
    }

    // The panel keeps its frame while asleep, and here every frame is
    // whole the moment it's pushed: waking, sleeping and showing only
    // matter to what the backlight lights, which the window draws from
    // `backlight`.
    fn wake_panel(&mut self) -> impl Future<Output = ()> {
        log("screen on");
        async {}
    }

    fn sleep_panel(&mut self) -> impl Future<Output = ()> {
        log("screen off");
        async {}
    }

    async fn show_panel(&mut self) {}

    fn set_idle(&mut self, idle: bool) -> impl Future<Output = ()> {
        PANEL.with_borrow_mut(|panel| panel.idle = idle);
        async {}
    }

    fn wait_for_refresh(&mut self) -> impl Future<Output = ()> {
        // `display::REFRESH`.
        Timer::after(Duration::from_millis(20))
    }

    fn backlight(&mut self, level: brightness::Level) {
        self.set_backlight(level);
    }

    fn bthome(&mut self) -> Option<Identity> {
        Some(DEMO_IDENTITY)
    }

    /// What `bthome::send` would advertise, with the demo key, printed.
    fn send_press(&mut self, press: Press, battery: u8) -> impl Future<Output = ()> {
        let counter = *self.bthome_counter;
        *self.bthome_counter += 1;
        let (data, len) = bthome::advert(press, battery, counter, &DEMO_IDENTITY);
        let hex: String = data[..len].iter().map(|b| format!("{b:02x}")).collect();
        // What Home Assistant fires for it (see `bthome`'s docs).
        let dimmer = match press.gesture {
            crate::buttons::Gesture::Tap => Some(1),
            crate::buttons::Gesture::DoubleTap => Some(2),
            crate::buttons::Gesture::Hold => Some(3),
            crate::buttons::Gesture::Release => None,
        };
        let dimmer = dimmer.map_or(String::new(), |slot| {
            format!(", dimmer_{slot} rotate_right steps {}", press.code)
        });
        log(format!(
            "to Home Assistant: button {}{dimmer} (counter {counter}): {hex}",
            press.gesture.name()
        ));
        async {}
    }

    /// As `system_off::shut_down`: says why for a few seconds, then goes
    /// dark. (Saving the activity, and putting the sensors to sleep, have
    /// nothing to simulate: the simulator's activity outlives switching
    /// off anyway.)
    async fn switch_off(&mut self, reason: Reason, canvas: &mut Canvas, _panel_awake: bool) {
        log(format!("switching off: {reason:?}"));
        ui::render_off(canvas, reason);
        canvas.flush(self.display).await;
        PANEL.with_borrow_mut(|panel| panel.idle = false);
        self.set_backlight(brightness::Level::MID);
        Timer::after(OFF_SCREEN).await;
        self.set_backlight(brightness::Level::OFF);
    }

    /// There's no watchdog to feed.
    fn pet_watchdog(&mut self) {}
}

// ==============================================================================
// Odds and ends
// ==============================================================================

/// Prints what happened, with the uptime and the wall-clock time.
pub fn log(message: impl AsRef<str>) {
    let uptime = Instant::now().as_millis();
    let wall = clock::now()
        .map(|t| format!(" {:02}:{:02}:{:02}", t.hour, t.minute, t.second))
        .unwrap_or_default();
    crate::platform::log(&format!(
        "[{:>6}.{:03}{wall}] {}",
        uptime / 1000,
        uptime % 1000,
        message.as_ref()
    ));
}

/// A theme by name, in any case.
pub fn find_theme(name: &str) -> Result<&'static Theme, String> {
    let themes = || crate::theme::DARK.iter().chain(&crate::theme::LIGHT);
    themes()
        .find(|t| t.name.eq_ignore_ascii_case(name))
        .copied()
        .ok_or_else(|| {
            let names: Vec<_> = themes().map(|t| t.name).collect();
            format!("no theme {name}; there's {}", names.join(", "))
        })
}

/// Moves the clock to `at` (never backwards).
/// Where `motor` (a pattern, and since when) is in its pattern at `now`:
/// in which pulse, and how far into it, or into the pause after it.
/// `None` if nothing has played.
pub fn motor_pulse(motor: Option<(Pattern, Instant)>, now: Instant) -> Option<Pulse> {
    let (pattern, since) = motor?;
    let steps = pattern.steps();
    let ms = |us: u64| us as f32 / 1000.0;
    // Microseconds, not milliseconds: a pulse eases in over a few tens of
    // them, and the window's frames come every 16.
    let mut t = (now - since).as_micros();
    // The last pulse's length, and how long the motor's been off since.
    let mut on = None;
    let mut off = 0;
    let mut i = 0;
    loop {
        let Some(&step) = steps.get(i) else {
            match pattern.repeat_from() {
                Some(from) if from < steps.len() => i = from,
                // Played out: the last pulse winds down, then nothing.
                _ => {
                    return on.map(|on| Pulse {
                        on_ms: ms(on),
                        off_ms: Some(ms(off + t)),
                    });
                }
            }
            continue;
        };
        let step = u64::from(step) * 1000;
        let pulse = i % 2 == 0;
        if t < step {
            return Some(if pulse {
                Pulse {
                    on_ms: ms(t),
                    off_ms: None,
                }
            } else {
                Pulse {
                    on_ms: ms(on.unwrap_or(0)),
                    off_ms: Some(ms(off + t)),
                }
            });
        }
        if pulse {
            on = Some(step);
            off = 0;
        } else {
            off += step;
        }
        t -= step;
        i += 1;
    }
}

fn set_time(at: Instant) {
    let now = Instant::now();
    if at > now {
        crate::time::advance(at - now);
    }
}

/// Runs a future of the firmware's to completion. Its awaits are the
/// stand-ins' (instant here) and timers (a transition's frames, the
/// switching-off screen), which this lets through by moving the clock a
/// millisecond at a time, calling `on_frame` after each.
pub fn run<F: Future>(future: F, on_frame: &mut Option<Box<dyn FnMut()>>) -> F::Output {
    let mut future = pin!(future);
    let mut context = Context::from_waker(Waker::noop());
    loop {
        if let Poll::Ready(output) = future.as_mut().poll(&mut context) {
            return output;
        }
        crate::time::advance(Duration::from_millis(1));
        if let Some(on_frame) = on_frame {
            on_frame();
        }
    }
}

pub fn pattern_name(pattern: Pattern) -> &'static str {
    [
        (Pattern::STOP, "stop"),
        (Pattern::NOTIFY, "notify"),
        (Pattern::TURN, "turn"),
        (Pattern::RING, "ring"),
        (Pattern::TIMER, "timer"),
        (Pattern::ALARM, "alarm"),
    ]
    .into_iter()
    .find(|&(p, _)| p == pattern)
    .map_or("(another pattern)", |(_, name)| name)
}

fn describe(event: &InputEvent) -> String {
    use crate::ui::PhoneEvent;
    match event {
        InputEvent::Button => "button".into(),
        InputEvent::Glance => "glance".into(),
        InputEvent::Attention => "attention".into(),
        InputEvent::Touch(touch) => format!(
            "{} at ({},{}), {} ms",
            gesture_name(touch.gesture),
            touch.x,
            touch.y,
            touch.held_ms
        ),
        InputEvent::Phone(event) => match event {
            PhoneEvent::Notification { app, who } => format!("notification: {app}: {who}"),
            PhoneEvent::IncomingCall { caller } => format!("incoming call: {caller}"),
            PhoneEvent::CallOver => "call over".into(),
            PhoneEvent::MusicInfo { artist, track } => format!("music: {artist}: {track}"),
            PhoneEvent::MusicState { playing } => format!("music playing: {playing}"),
            PhoneEvent::Find { on } => format!("find my watch: {on}"),
            PhoneEvent::Unpaired(why) => format!("unpaired: {}", why.title()),
            PhoneEvent::TextAsBitmaps => "text as bitmaps".into(),
            PhoneEvent::Alarms(alarms) => {
                format!("{} alarms from the phone", alarms.alarms().len())
            }
            PhoneEvent::Route(Some(step)) => {
                format!(
                    "navigation: {:?} {} {}",
                    step.turn, step.distance, step.street
                )
            }
            PhoneEvent::Route(None) => "navigation over".into(),
            PhoneEvent::IntentsOff => "Gadgetbridge: intents not allowed".into(),
            PhoneEvent::UpdateRefused => "update refused".into(),
        },
    }
}

fn gesture_name(gesture: crate::gesture::Gesture) -> &'static str {
    use crate::gesture::Gesture;
    match gesture {
        Gesture::Tap => "tap",
        Gesture::LongPress => "long press",
        Gesture::SwipeUp => "swipe up",
        Gesture::SwipeDown => "swipe down",
        Gesture::SwipeLeft => "swipe left",
        Gesture::SwipeRight => "swipe right",
        Gesture::Release => "release",
    }
}
