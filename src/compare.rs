//! What the UI loop does and renders with, for checking the simulator
//! (`sim/`) against a dev kit, frame for frame (`ur-sim --devkit`). Only in
//! builds with the `compare` feature.
//!
//! The simulator runs the same UI code, so given the same inputs at the
//! same instants, and the same readings to render with, it must draw the
//! same frames. What a dev kit has that the simulator can't know is
//! exported here, in [`UR_COMPARE`], for the simulator to read over the
//! debug probe, as `scripts/screenshot.py` reads the canvas:
//!
//! - [`Boot`]: the instant the UI started, and the settings it loaded from
//!   flash.
//! - [`Event`]s: everything that changes the UI's state (ticks and
//!   inputs), with the exact instants and readings the UI loop handed it. The simulator replays
//!   them in order. A ring: the reader must keep up.
//! - [`Pass`]: what the last frame was rendered with (`ui::Status`, and
//!   today's activity), and a seqlock around it, so a reader can tell a
//!   frame whose canvas it read whole.
//!
//! Everything is `repr(C)` plain integers, so the simulator, which
//! includes this file too, reads the bytes straight into the same types.
//! It costs ~2.6 KB of RAM, so it isn't in releases.

use core::cell::UnsafeCell;

use embassy_time::Instant;

use crate::day::{Day, HOURS};
use crate::gesture::Gesture;
use crate::ui::{PhoneEvent, Status, Unpaired};
use crate::{InputEvent, alarm};

/// Set in [`Head::magic`] once [`Boot`] is written.
pub const MAGIC: u32 = 0x7572_4350; // "urCP"

/// How many events the ring keeps: a tick a second, plus inputs, so the
/// reader must look about once a minute. (The probe's bridge sometimes
/// stalls for many seconds.)
pub const EVENTS: usize = 64;

/// The events come last, so a reader can read everything else in one go
/// (the [`Head`]), and only the events it hasn't seen: the probe's reads
/// slow the CPU, and saving to flash, in a radio timeslot, has no time to
/// spare.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Export {
    pub head: Head,
    pub events: [Event; EVENTS],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Head {
    pub magic: u32,
    /// Events written so far; the latest is `events[(written - 1) % EVENTS]`.
    pub written: u32,
    /// Bumped as a pass starts writing [`Head::pass`], and set to match it
    /// once that pass's frame is on the panel (in the canvas's `front`).
    pub started: u32,
    pub finished: u32,
    pub boot: Boot,
    pub pass: Pass,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Boot {
    /// `App::new`'s instant, in ticks.
    pub at: u64,
    pub theme_light: u8,
    pub theme_dark: u8,
    pub brightness: u8,
    pub buttons: u8,
    pub alarms: [u8; alarm::Settings::BYTES],
    pub phone_alarms: [u8; alarm::Phone::BYTES],
}

/// What [`Event::kind`] says happened.
pub mod kind {
    pub const TICK: u8 = 1;
    pub const BUTTON: u8 = 2;
    pub const TOUCH: u8 = 3;
    pub const ATTENTION: u8 = 4;
    pub const PHONE: u8 = 5;
    pub const GLANCE: u8 = 6;
    /// The UI loop's timer (see `App::on_timer`).
    pub const TIMER: u8 = 7;
}

/// What [`Event::phone`] says the phone sent: the content isn't kept (the
/// simulator knows what it injected), only which message it was.
pub mod phone {
    pub const NOTIFICATION: u8 = 1;
    pub const INCOMING_CALL: u8 = 2;
    pub const CALL_OVER: u8 = 3;
    pub const MUSIC_INFO: u8 = 4;
    pub const MUSIC_STATE_PAUSED: u8 = 5;
    pub const MUSIC_STATE_PLAYING: u8 = 6;
    pub const FIND: u8 = 7;
    pub const UNPAIRED: u8 = 8;
    pub const ALARMS: u8 = 9;
    pub const ROUTE: u8 = 10;
    pub const TEXT_AS_BITMAPS: u8 = 11;
    pub const INTENTS_OFF: u8 = 12;
}

/// Tick flags: what `App::check_alarms` and `App::on_tick` were told;
/// and for the timer, `App::on_timer`.
pub mod flag {
    pub const STIRRING: u8 = 1;
    pub const ASLEEP: u8 = 2;
    pub const WATCHING: u8 = 4;
    pub const FINGER_DOWN: u8 = 8;
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Event {
    /// The instant the App was handed, in ticks: a tick's `check_alarms`,
    /// an input's `on_input`.
    pub at: u64,
    /// A tick's `on_tick` instant (a moment after `at`).
    pub ticked: u64,
    /// A tick's local second for `check_alarms`, or [`NONE_I64`].
    pub local_second: i64,
    pub kind: u8,
    /// [`flag`]s, for a tick.
    pub flags: u8,
    /// A touch's gesture (as [`gesture_code`]), and where it started.
    pub gesture: u8,
    pub x: u8,
    pub y: u8,
    /// Which [`phone`] message.
    pub phone: u8,
    pub held_ms: u16,
}

/// What `ui::Status` held for the pass, and today's activity.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Pass {
    /// The pass's `now`, in ticks.
    pub now: u64,
    pub uptime_secs: u64,
    /// `Status::local_second`, or [`NONE_I64`].
    pub local_second: i64,
    pub sleep_start_utc_minute: i64,
    pub sleep_end_utc_minute: i64,
    pub battery_mv: u32,
    /// Or [`NONE_U32`].
    pub passkey: u32,
    /// Or [`NONE_U32`].
    pub steps: u32,
    /// `Status::local_time`, if `has_time`.
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    pub has_time: u8,
    /// `clock::utc_offset_minutes()`, if `has_offset`.
    pub offset_minutes: i16,
    pub has_offset: u8,
    pub has_sleep: u8,
    pub sleep_asleep_minutes: u16,
    pub sleep_deep_minutes: u16,
    pub sleep_rem_minutes: u16,
    pub charging: u8,
    pub power_present: u8,
    pub connected: u8,
    /// 0 none, 1 not paired, 2 pairing lost.
    pub unpaired: u8,
    /// Or 255 for none.
    pub update_progress: u8,
    /// Or 0 for none.
    pub heart_rate: u8,
    pub temperature: i8,
    /// 0 none, 1 the watch's (or estimated from it), 2 a nearby Aranet4's.
    pub has_temperature: u8,
    /// What the App was told it is (`App::set_daytime`).
    pub daytime: u8,
    pub has_today: u8,
    /// `activity::today()`: each hour's steps (little-endian), lowest and
    /// highest heart rate, and the sum (little-endian) and count of its
    /// readings.
    pub today: [[u8; 7]; HOURS],
}

pub const NONE_I64: i64 = i64::MIN;
pub const NONE_U32: u32 = u32::MAX;

pub fn gesture_code(gesture: Gesture) -> u8 {
    match gesture {
        Gesture::Tap => 1,
        Gesture::LongPress => 2,
        Gesture::SwipeUp => 3,
        Gesture::SwipeDown => 4,
        Gesture::SwipeLeft => 5,
        Gesture::SwipeRight => 6,
    }
}

/// For the simulator, which reads the events.
#[cfg(not(target_os = "none"))]
pub fn gesture_from_code(code: u8) -> Option<Gesture> {
    Some(match code {
        1 => Gesture::Tap,
        2 => Gesture::LongPress,
        3 => Gesture::SwipeUp,
        4 => Gesture::SwipeDown,
        5 => Gesture::SwipeLeft,
        6 => Gesture::SwipeRight,
        _ => return None,
    })
}

#[cfg(not(target_os = "none"))]
impl Pass {
    /// For the simulator, which renders with the dev kit's day.
    pub fn day(&self) -> Option<Day> {
        if self.has_today == 0 {
            return None;
        }
        let mut day = Day::default();
        for (hour, bytes) in day.hours.iter_mut().zip(&self.today) {
            let [s0, s1, min, max, sum0, sum1, count] = *bytes;
            hour.steps = u16::from_le_bytes([s0, s1]);
            hour.heart_min = min;
            hour.heart_max = max;
            hour.heart_sum = u16::from_le_bytes([sum0, sum1]);
            hour.heart_count = count;
        }
        Some(day)
    }
}

// ==============================================================================
// Writing it, on the watch
// ==============================================================================

const PASS: Pass = Pass {
    now: 0,
    uptime_secs: 0,
    local_second: 0,
    sleep_start_utc_minute: 0,
    sleep_end_utc_minute: 0,
    battery_mv: 0,
    passkey: 0,
    steps: 0,
    year: 0,
    month: 0,
    day: 0,
    hour: 0,
    minute: 0,
    second: 0,
    has_time: 0,
    offset_minutes: 0,
    has_offset: 0,
    has_sleep: 0,
    sleep_asleep_minutes: 0,
    sleep_deep_minutes: 0,
    sleep_rem_minutes: 0,
    charging: 0,
    power_present: 0,
    connected: 0,
    unpaired: 0,
    update_progress: 0,
    heart_rate: 0,
    temperature: 0,
    has_temperature: 0,
    daytime: 0,
    has_today: 0,
    today: [[0; 7]; HOURS],
};

/// Only the UI task writes it, and the debugger reads it.
#[repr(transparent)]
pub struct Shared(UnsafeCell<Export>);

// SAFETY: Written only from the UI task (one thread); the debugger reads it
// from outside, and copes with torn reads through the seqlock and the
// event count.
unsafe impl Sync for Shared {}

#[cfg_attr(target_os = "none", unsafe(no_mangle))]
pub static UR_COMPARE: Shared = Shared(UnsafeCell::new(Export {
    head: Head {
        magic: 0,
        written: 0,
        started: 0,
        finished: 0,
        boot: Boot {
            at: 0,
            theme_light: 0,
            theme_dark: 0,
            brightness: 0,
            buttons: 0,
            alarms: [0; alarm::Settings::BYTES],
            phone_alarms: [0; alarm::Phone::BYTES],
        },
        pass: PASS,
    },
    events: [Event {
        at: 0,
        ticked: 0,
        local_second: 0,
        kind: 0,
        flags: 0,
        gesture: 0,
        x: 0,
        y: 0,
        phone: 0,
        held_ms: 0,
    }; EVENTS],
}));

fn with<R>(f: impl FnOnce(&mut Export) -> R) -> R {
    // SAFETY: Only the UI task calls this, never reentrantly, so this is
    // the only reference while it lives.
    f(unsafe { &mut *UR_COMPARE.0.get() })
}

/// The UI started, at `at`, with these settings from flash.
pub fn boot(at: Instant, app: &crate::ui::App) {
    with(|export| {
        export.head.boot = Boot {
            at: at.as_ticks(),
            theme_light: app.themes.light,
            theme_dark: app.themes.dark,
            brightness: app.brightness.to_byte(),
            buttons: app.buttons.to_byte(),
            alarms: app.alarms.to_bytes(),
            phone_alarms: app.phone_alarms.to_bytes(),
        };
        export.head.magic = MAGIC;
    });
}

fn push(event: Event) {
    with(|export| {
        let head = &mut export.head;
        export.events[head.written as usize % EVENTS] = event;
        head.written = head.written.wrapping_add(1);
    });
}

fn event(kind: u8, at: Instant) -> Event {
    Event {
        at: at.as_ticks(),
        ticked: 0,
        local_second: NONE_I64,
        kind,
        flags: 0,
        gesture: 0,
        x: 0,
        y: 0,
        phone: 0,
        held_ms: 0,
    }
}

pub fn tick(
    at: Instant,
    local_second: Option<i64>,
    stirring: bool,
    asleep: bool,
    ticked: Instant,
    watching: bool,
) {
    push(Event {
        ticked: ticked.as_ticks(),
        local_second: local_second.unwrap_or(NONE_I64),
        flags: (u8::from(stirring) * flag::STIRRING)
            | (u8::from(asleep) * flag::ASLEEP)
            | (u8::from(watching) * flag::WATCHING),
        ..event(kind::TICK, at)
    });
}

/// The UI loop's timer went off, about to call `App::on_timer` with `at`.
pub fn timer(at: Instant, finger_down: bool) {
    push(Event {
        flags: u8::from(finger_down) * flag::FINGER_DOWN,
        ..event(kind::TIMER, at)
    });
}

/// An input, about to be handed to `App::on_input` with `at`.
pub fn input(input: &InputEvent, at: Instant) {
    let event = match input {
        InputEvent::Button => event(kind::BUTTON, at),
        InputEvent::Attention => event(kind::ATTENTION, at),
        InputEvent::Glance => event(kind::GLANCE, at),
        InputEvent::Touch(touch) => Event {
            gesture: gesture_code(touch.gesture),
            x: touch.x as u8,
            y: touch.y as u8,
            held_ms: touch.held_ms,
            ..event(kind::TOUCH, at)
        },
        InputEvent::Phone(message) => Event {
            phone: match message {
                PhoneEvent::Notification { .. } => phone::NOTIFICATION,
                PhoneEvent::IncomingCall { .. } => phone::INCOMING_CALL,
                PhoneEvent::CallOver => phone::CALL_OVER,
                PhoneEvent::MusicInfo { .. } => phone::MUSIC_INFO,
                PhoneEvent::MusicState { playing: false } => phone::MUSIC_STATE_PAUSED,
                PhoneEvent::MusicState { playing: true } => phone::MUSIC_STATE_PLAYING,
                PhoneEvent::Find { .. } => phone::FIND,
                PhoneEvent::Unpaired(_) => phone::UNPAIRED,
                PhoneEvent::Alarms(_) => phone::ALARMS,
                PhoneEvent::Route(_) => phone::ROUTE,
                PhoneEvent::TextAsBitmaps => phone::TEXT_AS_BITMAPS,
                PhoneEvent::IntentsOff => phone::INTENTS_OFF,
            },
            ..event(kind::PHONE, at)
        },
    };
    push(event);
}

/// A pass is about to render, at `now`, with `status` and `today`. Call
/// with nothing in between that could let another task run (no awaits),
/// so `today` is what the render will see.
pub fn rendering(now: Instant, status: &Status, daytime: bool, today: Option<Day>) {
    with(|export| {
        let head = &mut export.head;
        head.started = head.started.wrapping_add(1);
        let time = status.local_time;
        let sleep = status.sleep;
        let offset = crate::clock::utc_offset_minutes();
        let mut bytes = [[0; 7]; HOURS];
        if let Some(day) = &today {
            for (out, hour) in bytes.iter_mut().zip(&day.hours) {
                let [s0, s1] = hour.steps.to_le_bytes();
                let [sum0, sum1] = hour.heart_sum.to_le_bytes();
                *out = [
                    s0,
                    s1,
                    hour.heart_min,
                    hour.heart_max,
                    sum0,
                    sum1,
                    hour.heart_count,
                ];
            }
        }
        head.pass = Pass {
            now: now.as_ticks(),
            uptime_secs: status.uptime_secs,
            local_second: status.local_second.unwrap_or(NONE_I64),
            sleep_start_utc_minute: sleep.map_or(0, |s| s.start_utc_minute),
            sleep_end_utc_minute: sleep.map_or(0, |s| s.end_utc_minute),
            battery_mv: status.battery.millivolts(),
            passkey: status.passkey.unwrap_or(NONE_U32),
            steps: status.steps.unwrap_or(NONE_U32),
            year: time.map_or(0, |t| t.year),
            month: time.map_or(0, |t| t.month),
            day: time.map_or(0, |t| t.day),
            hour: time.map_or(0, |t| t.hour),
            minute: time.map_or(0, |t| t.minute),
            second: time.map_or(0, |t| t.second),
            has_time: u8::from(time.is_some()),
            offset_minutes: offset.unwrap_or(0),
            has_offset: u8::from(offset.is_some()),
            has_sleep: u8::from(sleep.is_some()),
            sleep_asleep_minutes: sleep.map_or(0, |s| s.asleep_minutes),
            sleep_deep_minutes: sleep.map_or(0, |s| s.deep_minutes),
            sleep_rem_minutes: sleep.map_or(0, |s| s.rem_minutes),
            charging: u8::from(status.battery.charging),
            power_present: u8::from(status.battery.power_present),
            connected: u8::from(status.connected),
            unpaired: match status.unpaired {
                None => 0,
                Some(Unpaired::NotPaired) => 1,
                Some(Unpaired::PairingLost) => 2,
            },
            update_progress: status.update_progress.unwrap_or(255),
            heart_rate: status.heart_rate.unwrap_or(0),
            temperature: status.temperature.unwrap_or(0),
            // 2: from a nearby Aranet4 (the clock face's little house).
            has_temperature: match (status.temperature, status.temperature_from_aranet) {
                (None, _) => 0,
                (Some(_), false) => 1,
                (Some(_), true) => 2,
            },
            daytime: u8::from(daytime),
            has_today: u8::from(today.is_some()),
            today: bytes,
        };
    });
}

/// The pass's frame is on the panel.
pub fn rendered() {
    with(|export| export.head.finished = export.head.started);
}
