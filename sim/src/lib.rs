//! `ur-sim`: the watch's UI on a computer.
//!
//! This runs the firmware's own UI code (`src/ui.rs`, and the canvas,
//! themes, fonts, alarms, clock, sunrise, and navigation logic it uses) on
//! the host, included here with `#[path]` as the host tests do. What needs
//! hardware is replaced by a stand-in with the same interface: the panel
//! ([`display`]) collects the pixels the canvas pushes, the accelerometer's
//! steps and sleep ([`activity`]) are whatever the simulator is told, and
//! Bluetooth ([`ble`]) is a queue the simulator prints. [`watch`] then plays
//! the part of `main.rs`'s UI loop: ticks, inputs, rendering, and screen
//! power, against a clock it controls ([`time`], ticking at the watch's
//! rate), so scripts are deterministic and can skip hours ahead.
//!
//! A library, so the browser's build (a WebAssembly module) shares all of
//! it with the command line's (`src/main.rs`).
//!
//! See README.md here for how to use it, and what it can't simulate.

// ==============================================================================
// The firmware's modules
// ==============================================================================
//
// At the crate root, where their `crate::…` paths expect each other. Not
// everything in them is used here.

#[path = "../../src/alarm.rs"]
#[allow(dead_code)]
mod alarm;
#[path = "../../src/aranet.rs"]
#[allow(dead_code)]
mod aranet;
#[path = "../../src/arrival.rs"]
#[allow(dead_code)]
mod arrival;
#[path = "../../src/battery.rs"]
#[allow(dead_code)]
mod battery;
#[path = "../../src/brightness.rs"]
#[allow(dead_code)]
mod brightness;
#[path = "../../src/bthome.rs"]
#[allow(dead_code)]
mod bthome;
#[path = "../../src/buttons.rs"]
#[allow(dead_code)]
mod buttons;
#[path = "../../src/canvas.rs"]
#[allow(dead_code)]
mod canvas;
#[path = "../../src/clock.rs"]
#[allow(dead_code)]
mod clock;
#[path = "../../src/compare.rs"]
#[allow(dead_code)]
mod compare;
#[path = "../../src/day.rs"]
#[allow(dead_code)]
mod day;
#[path = "../../src/decimal.rs"]
#[allow(dead_code)]
mod decimal;
#[path = "../../src/font.rs"]
#[allow(dead_code)]
mod font;
#[path = "../../src/gb.rs"]
#[allow(dead_code)]
mod gb;
#[path = "../../src/geo.rs"]
#[allow(dead_code)]
mod geo;
#[path = "../../src/gesture.rs"]
#[allow(dead_code)]
mod gesture;
#[path = "../../src/motor.rs"]
#[allow(dead_code)]
mod motor;
#[path = "../../src/nav.rs"]
#[allow(dead_code)]
mod nav;
#[path = "../../src/phone.rs"]
mod phone;
#[path = "../../src/qr.rs"]
#[allow(dead_code)]
mod qr;
#[path = "../../src/route.rs"]
#[allow(dead_code)]
mod route;
#[path = "../../src/sleep.rs"]
#[allow(dead_code)]
mod sleep;
#[path = "../../src/temperature.rs"]
#[allow(dead_code)]
mod temperature;
#[path = "../../src/theme.rs"]
#[allow(dead_code)]
mod theme;
#[path = "../../src/ui.rs"]
#[allow(dead_code)]
mod ui;
#[path = "../../src/wake.rs"]
#[allow(dead_code)]
mod wake;

// ==============================================================================
// Stand-ins for the hardware's
// ==============================================================================

#[path = "stub/activity.rs"]
mod activity;
#[path = "stub/display.rs"]
mod display;

/// `src/ble.rs`: only the queue of messages for the phone, which
/// [`watch`] empties and prints.
mod ble {
    use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
    use embassy_sync::channel::Channel;

    use crate::gb::Outgoing;

    pub static OUTBOX: Channel<CriticalSectionRawMutex, Outgoing, 8> = Channel::new();
}

/// `src/debug.rs`: only the backdating of a test notification, which the
/// simulator's `notify` offers too.
mod debug {
    use std::sync::atomic::{AtomicI64, Ordering};

    static BACKDATE_MINUTES: AtomicI64 = AtomicI64::new(0);

    pub fn take_backdate_minutes() -> i64 {
        BACKDATE_MINUTES.swap(0, Ordering::Relaxed)
    }

    pub fn backdate_next(minutes: i64) {
        BACKDATE_MINUTES.store(minutes, Ordering::Relaxed);
    }
}

/// `src/system_off.rs`: only why the watch switches off, for its last
/// screen.
mod system_off {
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    pub enum Reason {
        BatteryEmpty,
        Storage,
    }
}

/// `src/touch.rs`, the touch controller's driver: only its types, which
/// are [`crate::gesture`]'s. [`watch`] feeds the gesture recogniser itself.
mod touch {
    pub use crate::gesture::{Gesture, Touch};
}

/// As `main.rs`'s: every user input, in order, for the UI loop.
#[derive(defmt::Format)]
pub enum InputEvent {
    Button,
    Touch(touch::Touch),
    Attention,
    Phone(ui::PhoneEvent),
    Glance,
}

// Some pure modules derive `defmt::Format` only for the watch
// (`#[cfg_attr(target_os = "none", …)]`), so their host tests needn't
// depend on defmt; but the UI logs them. Their logs go nowhere here (see
// below), so a placeholder does.
macro_rules! format_on_the_host {
    ($($ty:ty),* $(,)?) => {$(
        impl defmt::Format for $ty {
            fn format(&self, f: defmt::Formatter) {
                defmt::write!(f, "?")
            }
        }
    )*};
}
format_on_the_host!(
    alarm::Kind,
    alarm::Settings,
    alarm::Switch,
    gesture::Gesture,
    gesture::Touch,
    nav::Side,
    route::Turn,
    sleep::Kind,
);

// The firmware logs through defmt, whose binary log needs the firmware ELF
// to decode; on the host there's no such table, so its logs go nowhere.
// The simulator prints what happens instead (see `watch::Watch::log`).
//
// TODO: decode them (the format strings are in the `.defmt.*` sections of
// this very binary), for the firmware's own view of what it did.
#[defmt::global_logger]
struct Logger;

// SAFETY: Does nothing, so there's nothing to synchronise.
unsafe impl defmt::Logger for Logger {
    fn acquire() {}
    unsafe fn flush() {}
    unsafe fn release() {}
    unsafe fn write(_bytes: &[u8]) {}
}

/// Where `defmt::panic!` and `defmt::unwrap!` end up (as embassy-time's
/// `Instant` subtraction does when it would go negative).
#[defmt::panic_handler]
fn defmt_panic() -> ! {
    panic!("the firmware panicked through defmt (its message isn't decoded here)")
}

// Not in the browser: WebAssembly keeps a `#[link_section]` static only if
// it's plain bytes, and the timestamp's isn't. Its logs go nowhere anyway,
// so there, every log's timestamp is nothing (defmt calls this for each).
#[cfg(not(target_arch = "wasm32"))]
defmt::timestamp!("{=u64:us}", embassy_time::Instant::now().as_micros());
#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
fn _defmt_timestamp(_: defmt::Formatter<'_>) {}

// ==============================================================================
// The simulator's own
// ==============================================================================

mod case;
mod platform;
mod record;
mod script;
mod shot;
mod time;
mod watch;

// The command line's: a terminal, files, a probe for the dev kit,
// comparing screenshots, and the window.
#[cfg(not(target_arch = "wasm32"))]
mod cli;
#[cfg(not(target_arch = "wasm32"))]
mod devkit;
#[cfg(not(target_arch = "wasm32"))]
mod diff;
#[cfg(all(feature = "window", not(target_arch = "wasm32")))]
mod window;

// The browser's: see web/.
#[cfg(target_arch = "wasm32")]
mod web;

/// The command line (see `src/main.rs`, which is only this).
#[cfg(not(target_arch = "wasm32"))]
pub fn main() -> std::process::ExitCode {
    cli::main()
}
