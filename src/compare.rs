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
//! - [`Event`]s: everything that changes the UI's state (ticks, inputs,
//!   and what a pass tells the App besides: whether a phone is connected,
//!   a room sensor heard, and it's daytime), with the exact instants and
//!   readings the UI loop handed it. The simulator replays them in order.
//!   A ring: the reader must keep up.
//! - [`Pass`]: what the last frame was rendered with (`ui::Status`,
//!   with the room sensor's reading and the BTHome identity, and today's
//!   activity), and a seqlock around it, so a reader can tell a
//!   frame whose canvas it read whole.
//! - [`Push`]: what pushing that frame to the panel sent, and how long it
//!   took, against which the simulator checks its model of the panel's
//!   timing.
//! - [`Music`]: the text of the latest music info (artist and track),
//!   the one message whose content the simulator can't know in advance:
//!   a phone connected to the dev kit sends its own as it plays.
//!
//! Everything is `repr(C)` plain integers, so the simulator, which
//! includes this file too, reads the bytes straight into the same types.
//! It costs ~2.4 KB of RAM, so it isn't in releases.

use core::cell::UnsafeCell;
use core::sync::atomic::{Ordering, compiler_fence};

use embassy_time::Instant;
use heapless::String;

use crate::aranet::{Band, Kind, Sighting};
use crate::day::{Day, HOURS};
use crate::gesture::Gesture;
use crate::ui::{PhoneEvent, Status, Unpaired};
use crate::{InputEvent, alarm, buttons};

/// Set in [`Head::magic`] once [`Boot`] is written.
pub const MAGIC: u32 = 0x7572_4350; // "urCP"

/// How many events the ring keeps: a tick a second, plus inputs, so the
/// reader must look about once a minute. (The probe's bridge sometimes
/// stalls for many seconds.) 60 rather than 64 pays for [`Export::music`]:
/// this build has little RAM to spare.
pub const EVENTS: usize = 60;

/// The capacity of `PhoneEvent::MusicInfo`'s artist and track (checked
/// where they're copied: see [`music`]).
pub const MUSIC_TEXT: usize = 40;

/// The events come after the [`Head`], so a reader can read everything
/// else in one go, and only the events it hasn't seen: the probe's reads
/// slow the CPU, and saving to flash, in a radio timeslot, has no time to
/// spare. The music comes last, read only to replay a music info.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Export {
    pub head: Head,
    pub events: [Event; EVENTS],
    /// The latest music info.
    pub music: Music,
}

/// A music info's text, as the UI got it.
///
/// Only the latest is kept (RAM is short): a phone sends one per track, so
/// a reader that keeps up with the events reads each before the next.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Music {
    /// Which music info this is, counting from 1 at boot (wrapping): its
    /// event's [`Event::held_ms`]. A reader that finds a later number has
    /// missed this one's text.
    pub number: u16,
    pub artist_len: u8,
    pub track_len: u8,
    pub artist: [u8; MUSIC_TEXT],
    pub track: [u8; MUSIC_TEXT],
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
    /// The [`state`] flags last logged (as a [`kind::STATE`] event).
    pub state: u32,
    pub push: Push,
    pub boot: Boot,
    pub pass: Pass,
}

/// What the last pass sent to the panel (`src/display.rs` counts it), and
/// when: from its first address window to the frame's being finished.
///
/// Only the last pass's. Keeping the last pass that pushed anything (as
/// the ticks after a transition usually push nothing) took a static flag,
/// and with it the dev kit rebooted, three runs out of three, right after
/// `devkit-compare.txt`'s long press saved the alarms to flash; without
/// it, and on `main`, it didn't. Why isn't known (a layout-sensitive bug
/// elsewhere, perhaps): TODO find out, then keep the last push.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Push {
    /// In ticks; 0 if the pass pushed nothing.
    pub started: u64,
    pub finished: u64,
    /// Address windows opened (`Display::start_pixels`).
    pub windows: u32,
    /// Pixel transfers (`Display::pixels`), and the EasyDMA chunks of at
    /// most 255 bytes that the nRF52832 sends them in.
    pub transfers: u32,
    pub chunks: u32,
    /// Pixel bytes.
    pub bytes: u32,
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
    pub grid: u8,
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
    /// What the pass tells the App besides events changed (see
    /// [`super::state`]): [`super::state`] flags in [`Event::flags`].
    pub const STATE: u8 = 8;
    /// A button grid press handed to BTHome (see [`super::ha_press`]): its
    /// code in [`Event::x`], its gesture in [`Event::gesture`] (as
    /// [`super::ha_gesture_code`]), and [`Event::flags`] 1 if it went out.
    pub const HA_PRESS: u8 = 9;
}

/// What [`Event::phone`] says the phone sent: the content isn't kept (the
/// simulator knows what it injected), only which message it was; bar a
/// music info's, which is in [`Export::music`].
pub mod phone {
    pub const NOTIFICATION: u8 = 1;
    pub const INCOMING_CALL: u8 = 2;
    pub const CALL_OVER: u8 = 3;
    pub const MUSIC_INFO: u8 = 4;
    pub const MUSIC_STATE_PAUSED: u8 = 5;
    pub const MUSIC_STATE_PLAYING: u8 = 6;
    /// "Find my watch": on (see [`FIND_OFF`] for off).
    pub const FIND: u8 = 7;
    pub const UNPAIRED: u8 = 8;
    pub const ALARMS: u8 = 9;
    pub const ROUTE: u8 = 10;
    pub const TEXT_AS_BITMAPS: u8 = 11;
    pub const INTENTS_OFF: u8 = 12;
    pub const UPDATE_REFUSED: u8 = 13;
    pub const FIND_OFF: u8 = 14;
}

/// Tick flags: what `App::check_alarms` and `App::on_tick` were told;
/// and for the timer, `App::on_timer`.
pub mod flag {
    pub const STIRRING: u8 = 1;
    pub const ASLEEP: u8 = 2;
    pub const WATCHING: u8 = 4;
    pub const FINGER_DOWN: u8 = 8;
}

/// [`kind::STATE`] flags: what the pass last told the App
/// (`App::set_connected`, `App::set_air`, `App::set_daytime`).
pub mod state {
    pub const CONNECTED: u8 = 1;
    pub const AIR: u8 = 2;
    pub const DAYTIME: u8 = 4;
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
    /// How long a touch was held; for a music info, its number (see
    /// [`Music::number`]).
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
    /// `Status::temperature`, in tenths of a degree, if `has_temperature`.
    /// Here, beside the other i16, so it takes no padding.
    pub temperature: i16,
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
    /// 0 none, 1 the watch's (or estimated from it), 2 a nearby room
    /// sensor's.
    pub has_temperature: u8,
    /// What the App was told it is (`App::set_daytime`).
    pub daytime: u8,
    pub has_today: u8,
    /// `activity::today()`: each hour's steps (little-endian), lowest and
    /// highest heart rate, and the sum (little-endian) and count of its
    /// readings.
    pub today: [[u8; 7]; HOURS],
    /// `Status::air`.
    pub air: Air,
    /// `Status::bthome`, if `has_bthome`: the key page shows it.
    pub bthome_key: [u8; 16],
    pub bthome_address: [u8; 6],
    pub has_bthome: u8,
}

/// A room sensor's [`Sighting`], for the Air screen; `kind` 0 for none.
/// An `Option` field is in `present`'s bit (see [`present`]), its value 0
/// without.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Air {
    pub heard: u64,
    pub co2: u16,
    pub hundredths: i16,
    pub pressure: u16,
    pub interval: u16,
    pub age: u16,
    pub address: [u8; 6],
    pub rssi: i8,
    pub humidity: u8,
    pub battery: u8,
    /// 0 none, then `Band`'s variants from 1.
    pub band: u8,
    /// 0 no sighting, 1 an Aranet4, 2 a BTHome sensor, 3 an irregular one.
    pub kind: u8,
    pub present: u8,
}

/// [`Air::present`]'s bits.
pub mod present {
    pub const CO2: u8 = 1;
    pub const HUNDREDTHS: u8 = 2;
    pub const PRESSURE: u8 = 4;
    pub const HUMIDITY: u8 = 8;
    pub const BATTERY: u8 = 16;
}

impl Air {
    const NONE: Self = Self {
        heard: 0,
        co2: 0,
        hundredths: 0,
        pressure: 0,
        interval: 0,
        age: 0,
        address: [0; 6],
        rssi: 0,
        humidity: 0,
        battery: 0,
        band: 0,
        kind: 0,
        present: 0,
    };

    fn from(sighting: Option<&Sighting>) -> Self {
        let Some(sighting) = sighting else {
            return Self::NONE;
        };
        let reading = &sighting.reading;
        let bit = |is: bool, bit: u8| if is { bit } else { 0 };
        Self {
            heard: sighting.heard,
            co2: reading.co2.unwrap_or(0),
            hundredths: reading.hundredths.unwrap_or(0),
            pressure: reading.pressure.unwrap_or(0),
            interval: reading.interval,
            age: reading.age,
            address: sighting.address,
            rssi: sighting.rssi,
            humidity: reading.humidity.unwrap_or(0),
            battery: reading.battery.unwrap_or(0),
            band: match reading.band {
                None => 0,
                Some(Band::Good) => 1,
                Some(Band::Fair) => 2,
                Some(Band::Poor) => 3,
            },
            kind: match reading.kind {
                Kind::Aranet4 => 1,
                Kind::BTHome { irregular: false } => 2,
                Kind::BTHome { irregular: true } => 3,
            },
            present: bit(reading.co2.is_some(), present::CO2)
                | bit(reading.hundredths.is_some(), present::HUNDREDTHS)
                | bit(reading.pressure.is_some(), present::PRESSURE)
                | bit(reading.humidity.is_some(), present::HUMIDITY)
                | bit(reading.battery.is_some(), present::BATTERY),
        }
    }

    /// For the simulator, which renders the dev kit's.
    #[cfg(not(target_os = "none"))]
    pub fn sighting(&self) -> Option<Sighting> {
        let kind = match self.kind {
            1 => Kind::Aranet4,
            2 => Kind::BTHome { irregular: false },
            3 => Kind::BTHome { irregular: true },
            _ => return None,
        };
        let has = |bit: u8| self.present & bit != 0;
        Some(Sighting {
            address: self.address,
            rssi: self.rssi,
            heard: self.heard,
            reading: crate::aranet::Reading {
                kind,
                co2: has(present::CO2).then_some(self.co2),
                hundredths: has(present::HUNDREDTHS).then_some(self.hundredths),
                pressure: has(present::PRESSURE).then_some(self.pressure),
                humidity: has(present::HUMIDITY).then_some(self.humidity),
                battery: has(present::BATTERY).then_some(self.battery),
                band: match self.band {
                    1 => Some(Band::Good),
                    2 => Some(Band::Fair),
                    3 => Some(Band::Poor),
                    _ => None,
                },
                interval: self.interval,
                age: self.age,
            },
        })
    }
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
        Gesture::Release => 7,
    }
}

/// A grid press's gesture, as a [`kind::HA_PRESS`] event says.
pub fn ha_gesture_code(gesture: buttons::Gesture) -> u8 {
    match gesture {
        buttons::Gesture::Tap => 1,
        buttons::Gesture::DoubleTap => 2,
        buttons::Gesture::Hold => 3,
        buttons::Gesture::LongPress => 4,
        buttons::Gesture::Release => 5,
    }
}

/// Which [`phone`] message `message` is, as [`Event::phone`] says.
pub fn phone_code(message: &PhoneEvent) -> u8 {
    match message {
        PhoneEvent::Notification { .. } => phone::NOTIFICATION,
        PhoneEvent::IncomingCall { .. } => phone::INCOMING_CALL,
        PhoneEvent::CallOver => phone::CALL_OVER,
        PhoneEvent::MusicInfo { .. } => phone::MUSIC_INFO,
        PhoneEvent::MusicState { playing: false } => phone::MUSIC_STATE_PAUSED,
        PhoneEvent::MusicState { playing: true } => phone::MUSIC_STATE_PLAYING,
        PhoneEvent::Find { on: true } => phone::FIND,
        PhoneEvent::Find { on: false } => phone::FIND_OFF,
        PhoneEvent::Unpaired(_) => phone::UNPAIRED,
        PhoneEvent::Alarms(_) => phone::ALARMS,
        PhoneEvent::Route(_) => phone::ROUTE,
        PhoneEvent::TextAsBitmaps => phone::TEXT_AS_BITMAPS,
        PhoneEvent::IntentsOff => phone::INTENTS_OFF,
        PhoneEvent::UpdateRefused => phone::UPDATE_REFUSED,
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
        7 => Gesture::Release,
        _ => return None,
    })
}

/// The inverse of [`ha_gesture_code`].
#[cfg(not(target_os = "none"))]
pub fn ha_gesture_from_code(code: u8) -> Option<buttons::Gesture> {
    Some(match code {
        1 => buttons::Gesture::Tap,
        2 => buttons::Gesture::DoubleTap,
        3 => buttons::Gesture::Hold,
        4 => buttons::Gesture::LongPress,
        5 => buttons::Gesture::Release,
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
    air: Air::NONE,
    bthome_key: [0; 16],
    bthome_address: [0; 6],
    has_bthome: 0,
};

const PUSH: Push = Push {
    started: 0,
    finished: 0,
    windows: 0,
    transfers: 0,
    chunks: 0,
    bytes: 0,
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
        state: 0,
        push: PUSH,
        boot: Boot {
            at: 0,
            theme_light: 0,
            theme_dark: 0,
            brightness: 0,
            buttons: 0,
            grid: 0,
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
    music: Music {
        number: 0,
        artist_len: 0,
        track_len: 0,
        artist: [0; MUSIC_TEXT],
        track: [0; MUSIC_TEXT],
    },
}));

fn with<R>(f: impl FnOnce(&mut Export) -> R) -> R {
    // SAFETY: Only the UI task calls this, never reentrantly, so this is
    // the only reference while it lives.
    f(unsafe { &mut *UR_COMPARE.0.get() })
}

/// Keeps the stores before this before those after it, in memory and not
/// only in program order. The reader reads as the core runs (`esprobe
/// fast-dump` never halts it), word by word: a counter it checks (`magic`,
/// `written`, `started` and `finished`, a music info's `number`) has to
/// change on the right side of what it guards. A compiler fence is enough:
/// the Cortex-M4 has one core and makes its stores to RAM in order.
fn publish() {
    compiler_fence(Ordering::SeqCst);
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
            grid: app.grid_layout.to_byte(),
            alarms: app.alarms.to_bytes(),
            phone_alarms: app.phone_alarms.to_bytes(),
        };
        publish();
        export.head.magic = MAGIC;
    });
}

fn push(event: Event) {
    with(|export| {
        let head = &mut export.head;
        export.events[head.written as usize % EVENTS] = event;
        publish();
        head.written = head.written.wrapping_add(1);
    });
}

/// Keeps a music info's text, in place of the last, before its event is
/// pushed (so a reader that sees the event finds the text); returns its
/// number.
///
/// The parameters' types are `PhoneEvent::MusicInfo`'s fields': if their
/// capacity changes, this stops compiling, rather than cutting text short.
fn music(artist: &String<MUSIC_TEXT>, track: &String<MUSIC_TEXT>) -> u16 {
    with(|export| {
        let entry = &mut export.music;
        let number = entry.number.wrapping_add(1);
        entry.number = number;
        publish();
        entry.artist[..artist.len()].copy_from_slice(artist.as_bytes());
        entry.artist_len = artist.len() as u8;
        entry.track[..track.len()].copy_from_slice(track.as_bytes());
        entry.track_len = track.len() as u8;
        number
    })
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
    finger_down: bool,
) {
    push(Event {
        ticked: ticked.as_ticks(),
        local_second: local_second.unwrap_or(NONE_I64),
        flags: (u8::from(stirring) * flag::STIRRING)
            | (u8::from(asleep) * flag::ASLEEP)
            | (u8::from(watching) * flag::WATCHING)
            | (u8::from(finger_down) * flag::FINGER_DOWN),
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
            phone: phone_code(message),
            held_ms: match message {
                PhoneEvent::MusicInfo { artist, track } => music(artist, track),
                _ => 0,
            },
            ..event(kind::PHONE, at)
        },
    };
    push(event);
}

/// The pass told the App, at `now`, whether a phone is connected and
/// whether a room sensor was heard lately: logged if either changed, so
/// a reader replaying the events tells its App the same in between.
pub fn state(now: Instant, connected: bool, air: bool) {
    update_state(now, |flags| {
        (flags & state::DAYTIME)
            | (u8::from(connected) * state::CONNECTED)
            | (u8::from(air) * state::AIR)
    });
}

fn update_state(now: Instant, update: impl FnOnce(u8) -> u8) {
    let changed = with(|export| {
        let old = export.head.state as u8;
        let new = update(old);
        export.head.state = u32::from(new);
        (new != old).then_some(new)
    });
    if let Some(flags) = changed {
        push(Event {
            flags,
            ..event(kind::STATE, now)
        });
    }
}

/// The UI loop handed `press` to BTHome at `now`, and it went out
/// (`sent`) or couldn't (so the App was told, at `now`). Whether it did
/// is up to the radio, which the simulator hasn't got.
pub fn ha_press(now: Instant, press: buttons::Press, sent: bool) {
    push(Event {
        x: press.code,
        gesture: ha_gesture_code(press.gesture),
        flags: u8::from(sent),
        ..event(kind::HA_PRESS, now)
    });
}

/// A pass is about to render, at `now`, with `status` and `today`. Call
/// with nothing in between that could let another task run (no awaits),
/// so `today` is what the render will see.
pub fn rendering(now: Instant, status: &Status, daytime: bool, today: Option<Day>) {
    // The App was just told whether it's daytime (only while awake).
    update_state(now, |flags| {
        (flags & !state::DAYTIME) | (u8::from(daytime) * state::DAYTIME)
    });
    with(|export| {
        let head = &mut export.head;
        head.started = head.started.wrapping_add(1);
        publish();
        head.push = PUSH;
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
            // 2: from a nearby room sensor (to the tenth on the clock face).
            has_temperature: match (status.temperature, status.temperature_from_sensor) {
                (None, _) => 0,
                (Some(_), false) => 1,
                (Some(_), true) => 2,
            },
            daytime: u8::from(daytime),
            has_today: u8::from(today.is_some()),
            today: bytes,
            air: Air::from(status.air.as_ref()),
            bthome_key: status.bthome.map_or([0; 16], |identity| identity.key),
            bthome_address: status.bthome.map_or([0; 6], |identity| identity.address),
            has_bthome: u8::from(status.bthome.is_some()),
        };
    });
}

/// The pass's frame is on the panel.
pub fn rendered() {
    with(|export| {
        if export.head.push.windows > 0 {
            export.head.push.finished = Instant::now().as_ticks();
        }
        publish();
        export.head.finished = export.head.started;
    });
}

/// The display opened an address window for pixels.
pub fn window() {
    with(|export| {
        let push = &mut export.head.push;
        if push.windows == 0 {
            *push = Push {
                started: Instant::now().as_ticks(),
                ..PUSH
            };
        }
        push.windows += 1;
    });
}

/// The display sent `bytes` of pixels, in one transfer.
pub fn pixels(bytes: usize) {
    with(|export| {
        let push = &mut export.head.push;
        push.transfers += 1;
        push.chunks += bytes.div_ceil(255) as u32;
        push.bytes += bytes as u32;
    });
}
