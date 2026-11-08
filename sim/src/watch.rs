//! The simulated watch: `main.rs`'s UI loop, against a clock we control.
//!
//! On the watch, the UI loop wakes once a second (a tick), on every input,
//! and every [`MARQUEE_FRAME`] while text scrolls; each pass renders a
//! frame (if the screen is on) and pushes what changed to the panel. Here
//! the same happens, in the same order, but time only moves when
//! [`Watch::advance`] moves it (see [`crate::time`]): a script's
//! hour passes in milliseconds, and the same script gives the same frames
//! every time.
//!
//! What this mirrors from `main.rs` has to be kept in step with it by
//! hand: it's the part of the UI loop that isn't hardware (building
//! [`Status`], the screen power and dim-look bookkeeping, the tick's alarm
//! checks), and it's short. What it leaves out is listed in README.md.

use std::future::Future;
use std::pin::pin;
use std::task::{Context, Poll, Waker};

use embassy_time::{Duration, Instant};

use crate::activity;
use crate::aranet::{self, Sighting};
use crate::battery::Battery;
use crate::ble;
use crate::brightness;
use crate::bthome::{self, Identity};
use crate::canvas::{Canvas, MARQUEE_FRAME, Sweep};
use crate::case::Pulse;
use crate::clock;
use crate::display::{Display, PANEL};
use crate::geo;
use crate::gesture::Tracking;
use crate::motor::{self, Pattern};
use crate::sleep;
use crate::system_off::Reason;
use crate::temperature;
use crate::theme::Theme;
use crate::ui::{self, App, Status, ThemeChoice, Unpaired};
use crate::{InputEvent, alarm};

/// How often the UI loop ticks.
const TICK: Duration = Duration::from_secs(1);

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
    canvas: Box<Canvas>,
    display: Display,
    battery: Battery,

    // What the hardware would tell the UI loop.
    pub battery_mv: u32,
    pub charging: bool,
    pub power_present: bool,
    pub connected: bool,
    pub unpaired: Option<Unpaired>,
    pub passkey: Option<u32>,
    pub update_progress: Option<u8>,
    pub heart_rate: Option<u8>,
    pub temperature: Option<i8>,
    /// The nearest Aranet4's reading, as a scan would have left it (the
    /// simulator doesn't scan; `aranet` sets it).
    pub air: Option<Sighting>,
    /// Day or night, whatever the sun says (see `theme`).
    pub daytime: Option<bool>,

    // The UI loop's own state.
    panel_awake: bool,
    first_frame: bool,
    next_tick: Instant,
    /// When the last pass ended: the marquee's next frame is due
    /// [`MARQUEE_FRAME`] after it.
    last_pass: Instant,

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
    /// A watch about to boot: no time, no phone. Its first frame sweeps in
    /// at the first command that shows anything, or once time passes, so a
    /// script's first lines can set up what it boots into (say, the
    /// battery's first reading).
    pub fn boot() -> Self {
        let now = Instant::now();
        Self {
            app: App::new(
                now,
                ThemeChoice { light: 0, dark: 0 },
                alarm::Settings::default(),
            ),
            canvas: Box::new(Canvas::new()),
            display: Display,
            battery: Battery::new(),
            battery_mv: DEFAULT_BATTERY_MV,
            charging: false,
            power_present: false,
            connected: false,
            unpaired: None,
            passkey: None,
            update_progress: None,
            heart_rate: None,
            temperature: Some(31),
            air: None,
            daytime: None,
            panel_awake: false,
            first_frame: true,
            next_tick: now + TICK,
            last_pass: now,
            backlight: brightness::Level::OFF,
            motor: None,
            finger: None,
            off: None,
            bthome_counter: 256,
            on_frame: None,
            verbose: false,
        }
    }

    /// Shows the first frame, if it hasn't been yet.
    fn power_on(&mut self) {
        if self.first_frame {
            self.pass();
        }
    }

    /// Starts afresh after switching off, as waking from System OFF does
    /// (the settings in flash survive; nothing else does, bar the clock,
    /// which here keeps running: see README.md).
    fn reboot(&mut self) {
        let themes = self.app.themes;
        let alarms = self.app.alarms;
        let phone_alarms = self.app.phone_alarms;
        let brightness = self.app.brightness;
        let buttons = self.app.buttons;
        self.app = App::new(Instant::now(), themes, alarms);
        self.app.phone_alarms = phone_alarms;
        self.app.brightness = brightness;
        self.app.buttons = buttons;
        self.battery = Battery::new();
        self.off = None;
        self.first_frame = true;
        self.next_tick = Instant::now() + TICK;
        self.log("booted");
        self.pass();
    }

    pub fn log(&self, message: impl AsRef<str>) {
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

    // ==========================================================================
    // Time
    // ==========================================================================

    /// Lets `duration` pass: every tick, marquee frame, long press, and
    /// button grid deadline due in it happens, in order.
    pub fn advance(&mut self, duration: Duration) {
        self.power_on();
        let end = Instant::now() + duration;
        loop {
            let frame = self.app.animating().then(|| self.last_pass + MARQUEE_FRAME);
            let press = self.finger.as_ref().and_then(Tracking::long_press_at);
            // As `main.rs`: not while a finger is down.
            let due = self.app.deadline().filter(|_| self.finger.is_none());
            let frame = frame.into_iter().chain(due).min();
            let next = [Some(self.next_tick), frame, press]
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
                self.next_tick += TICK;
                self.tick();
            } else {
                self.app.on_timer(next, self.finger.is_some());
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
        let now = Instant::now();
        // A stir after stillness: what a smart alarm waits for.
        let stirring = activity::movement() >= sleep::STILL_LIMIT;
        let night = self.app.quiet_night();
        self.app
            .check_alarms(now, local_second(), stirring, activity::asleep());
        let watching = self.passkey.is_some() || self.update_progress.is_some();
        self.app.on_tick(now, watching);
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
        self.app.on_input(event, Instant::now());
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

    /// Whether the touch controller is asleep, so touches don't register
    /// at all: with the screen off in the quiet night (see
    /// `touch::set_asleep`).
    fn touch_asleep(&self) -> bool {
        self.app.quiet_night() && !self.app.awake
    }

    /// A finger down at, or moved to, (`x`, `y`) in panel pixels (0–239),
    /// as the touch controller would report it.
    pub fn finger_at(&mut self, x: i32, y: i32) {
        if self.touch_asleep() || self.off.is_some() {
            return;
        }
        let at = (x.clamp(0, 239), y.clamp(0, 239));
        match &mut self.finger {
            Some(finger) => finger.moved_to(at),
            None => self.finger = Some(Tracking::new(at)),
        }
    }

    /// The finger lifted: what it did is decided now, unless it was a long
    /// press, which was reported while it was down.
    pub fn lift(&mut self) {
        let Some(finger) = self.finger.take() else {
            return;
        };
        if !finger.held {
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

    /// What every event may have set off: buzzes, messages for the phone,
    /// and settings `main.rs` would save.
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
        if self.app.take_phone_alarms_changed() {
            self.log("phone alarms changed");
        }
        // What `main.rs` would hand `bthome::send`: the advertisement, as
        // the watch would send it, with the demo key.
        while let Some(press) = self.app.take_ha_press() {
            let counter = self.bthome_counter;
            self.bthome_counter += 1;
            let battery = self.battery.percent() as u8;
            let (data, len) = bthome::advert(press, battery, counter, &DEMO_IDENTITY);
            let hex: String = data[..len].iter().map(|b| format!("{b:02x}")).collect();
            let slot = match press.gesture {
                crate::buttons::Gesture::Tap => 1,
                crate::buttons::Gesture::DoubleTap => 2,
                crate::buttons::Gesture::Hold => 3,
            };
            self.log(format!(
                "to Home Assistant: dimmer_{slot} rotate_right steps {} (counter {counter}): {hex}",
                press.code
            ));
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
    // The pass: what the UI loop does each time it wakes
    // ==========================================================================

    pub fn pass(&mut self) {
        if self.off.is_some() {
            return;
        }
        let now = Instant::now();
        let sample = (self.battery_mv * 4096 / (3600 * 2)) as i16;
        self.battery
            .update(sample, self.charging, self.power_present);
        let off = if self.battery.critical(now.as_secs() as u32) {
            Some(Reason::BatteryEmpty)
        } else if self.app.storage_mode {
            Some(Reason::Storage)
        } else {
            None
        };
        if let Some(reason) = off {
            return self.switch_off(reason);
        }
        if self.touch_asleep() {
            // The controller's asleep: whatever was in progress is moot.
            self.finger = None;
        }

        // Every pass, not only while the screen is on: its media gestures
        // work with it off too, and need to know there's a phone.
        self.app.set_connected(self.connected);
        // As `main.rs` does, less the scans: a recent Aranet4 reading has
        // its screen, and a fresh one gives the temperature (the simulator
        // has no wrist to estimate it on: `temperature` sets the rest).
        let now_ms = now.as_millis();
        self.app.set_air(
            self.air
                .is_some_and(|s| now_ms.saturating_sub(s.heard) < aranet::RECENT),
        );
        let from_aranet = self
            .air
            .filter(|s| s.age(now_ms) < aranet::STALE)
            .and_then(|s| s.reading.quarters())
            .map(|quarters| temperature::shown(None, quarters));
        if self.app.awake {
            let status = Status {
                uptime_secs: now.as_secs(),
                local_time: clock::now(),
                connected: self.connected,
                unpaired: self.unpaired,
                passkey: self.passkey,
                update_progress: self.update_progress,
                steps: activity::steps_today(),
                sleep: activity::last_sleep(),
                local_second: local_second(),
                heart_rate: self.heart_rate,
                temperature: from_aranet.or(self.temperature),
                temperature_from_aranet: from_aranet.is_some(),
                air: self.air,
                battery: &self.battery,
                bthome: Some(DEMO_IDENTITY),
            };
            self.app
                .set_daytime(self.daytime.unwrap_or_else(geo::is_daytime));
            // Left out: the UI loop's going dark to redraw into or out of
            // the dim look, and idle mode's timing, which are about what
            // the backlight shows mid-redraw. Here every frame is whole.
            self.canvas.set_theme(self.app.theme());
            self.app.render(&mut self.canvas, &status, now);
            let sweep = match self.app.take_transition() {
                Some(sweep) if self.panel_awake => Some(sweep),
                _ if self.first_frame => Some(Sweep::LeftToRight),
                _ => None,
            };
            let Self {
                canvas,
                display,
                on_frame,
                ..
            } = self;
            match sweep {
                Some(sweep) => run(canvas.transition(display, sweep), on_frame),
                None => run(canvas.flush(display), on_frame),
            }
            self.first_frame = false;
            PANEL.with_borrow_mut(|panel| panel.idle = self.app.dim());
            self.backlight = self.app.backlight(Instant::now());
            PANEL.with_borrow_mut(|panel| panel.backlight = self.backlight);
            if !self.panel_awake {
                self.panel_awake = true;
                self.log("screen on");
            }
        } else if self.panel_awake {
            self.backlight = brightness::Level::OFF;
            PANEL.with_borrow_mut(|panel| panel.backlight = self.backlight);
            self.panel_awake = false;
            self.log("screen off");
        }
        self.last_pass = Instant::now();
    }

    /// As `system_off::shut_down`: says why for a few seconds, then goes
    /// dark.
    fn switch_off(&mut self, reason: Reason) {
        self.log(format!("switching off: {reason:?}"));
        ui::render_off(&mut self.canvas, reason);
        let Self {
            canvas,
            display,
            on_frame,
            ..
        } = self;
        run(canvas.flush(display), on_frame);
        PANEL.with_borrow_mut(|panel| panel.idle = false);
        self.backlight = brightness::Level::MID;
        PANEL.with_borrow_mut(|panel| panel.backlight = self.backlight);
        self.panel_awake = true;
        self.off = Some(reason);
        self.finger = None;
        self.next_tick = Instant::now() + OFF_SCREEN;
    }

    /// Switched off: the off screen goes dark after its few seconds, and
    /// only the charger (or, in storage mode, the button) wakes the watch.
    fn off_tick(&mut self) {
        if self.panel_awake {
            self.backlight = brightness::Level::OFF;
            PANEL.with_borrow_mut(|panel| panel.backlight = self.backlight);
            self.panel_awake = false;
            self.log("off");
        }
        if self.power_present {
            self.reboot();
        }
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
            self.app.themes.light = i as u8;
            self.daytime = Some(true);
        } else if let Some(i) = find(&crate::theme::DARK) {
            self.app.themes.dark = i as u8;
            self.daytime = Some(false);
        }
        self.pass();
        Ok(())
    }

    /// Something changed outside the UI loop's own events (a sensor, the
    /// phone's connection): show it, as the next pass would.
    pub fn refresh(&mut self) {
        self.after_event();
        self.pass();
    }
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

/// Local seconds since 1970, as `main.rs`'s `local_second`.
pub fn local_second() -> Option<i64> {
    let utc = clock::now_utc_seconds()?;
    let offset = i64::from(clock::utc_offset_minutes()?);
    Some(utc + offset * 60)
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

/// Runs a canvas push to completion. Its awaits are the panel's (instant
/// here) and a transition's frame timer, which this lets through by moving
/// the clock a millisecond at a time, calling `on_frame` after each.
pub fn run<F: Future<Output = ()>>(future: F, on_frame: &mut Option<Box<dyn FnMut()>>) {
    let mut future = pin!(future);
    let mut context = Context::from_waker(Waker::noop());
    while future.as_mut().poll(&mut context) == Poll::Pending {
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
    }
}
