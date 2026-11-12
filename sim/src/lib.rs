//! `ur-sim`: the watch's UI on a computer.
//!
//! This runs the firmware's own UI code (`src/ui.rs`, and the canvas,
//! palettes, fonts, alarms, clock, sunrise, and navigation logic it uses) on
//! the host, included here with `#[path]` as the host tests do. What needs
//! hardware is replaced by a stand-in with the same interface: the panel
//! ([`display`]) collects the pixels the canvas pushes, the accelerometer's
//! steps and sleep ([`activity`]) are whatever the simulator is told, and
//! Bluetooth ([`ble`]) is a queue the simulator prints, and the internal
//! flash ([`flash`]) a model of NOR flash under the firmware's own storage.
//! [`watch`] then runs the firmware's UI loop ([`ui_loop`], as `main.rs`
//! does): ticks, inputs, rendering, screen power and saving settings,
//! against a clock it controls ([`time`], ticking at the watch's rate), so
//! scripts are deterministic and can skip hours ahead.
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
#[path = "../../src/bthome_sensor.rs"]
#[allow(dead_code)]
mod bthome_sensor;
#[path = "../../src/buttons.rs"]
#[allow(dead_code)]
mod buttons;
#[path = "../../src/calendar.rs"]
#[allow(dead_code)]
mod calendar;
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
#[path = "../../src/gb_buckets.rs"]
#[allow(dead_code)]
mod gb_buckets;
#[path = "../../src/geo.rs"]
#[allow(dead_code)]
mod geo;
#[path = "../../src/gesture.rs"]
#[allow(dead_code)]
mod gesture;
#[path = "../../src/glance.rs"]
#[allow(dead_code)]
mod glance;
#[path = "../../src/heart_reading.rs"]
#[allow(dead_code)]
mod heart_reading;
#[path = "../../src/live.rs"]
mod live;
#[path = "../../src/motor.rs"]
#[allow(dead_code)]
mod motor;
#[path = "../../src/nav.rs"]
#[allow(dead_code)]
mod nav;
#[path = "../../src/on_time.rs"]
mod on_time;
#[path = "../../src/palette.rs"]
#[allow(dead_code)]
mod palette;
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
#[path = "../../src/sleep_hold.rs"]
#[allow(dead_code)]
mod sleep_hold;
#[path = "../../src/stages.rs"]
#[allow(dead_code)]
mod stages;
#[path = "../../src/storage.rs"]
#[allow(dead_code)]
mod storage;
#[path = "../../src/sun.rs"]
#[allow(dead_code)]
mod sun;
#[path = "../../src/temperature.rs"]
#[allow(dead_code)]
mod temperature;
#[path = "../../src/themes/mod.rs"]
mod themes;
#[path = "../../src/unpaired.rs"]
#[allow(dead_code)]
mod unpaired;

/// What every built-in theme must have, whatever it draws: a scheme at
/// least, and schemes with names and IDs (see `palette::id`) of their own
/// across every theme, as Settings' THEME row and flash know them. With
/// `--features themes-all` (as CI runs it), every theme in the repository.
#[cfg(test)]
mod every_theme {
    use crate::themes::{THEMES, Theme as _};

    #[test]
    fn has_schemes_with_names_and_ids_of_their_own() {
        let mut names = Vec::new();
        for theme in THEMES {
            assert!(
                !theme.night.is_empty() || !theme.day.is_empty(),
                "{} has no schemes",
                theme.name
            );
            for scheme in theme.night.iter().chain(theme.day) {
                names.push((scheme.palette().name, scheme.palette().id));
            }
        }
        // Each fits retro's THEME row (as every theme's must on its own),
        // the measure for a name the watch shows: where Retro is built in,
        // as with `themes-all`. (Each theme also asserts its own row fits
        // what it shows, as it draws it.)
        #[cfg(feature = "theme-retro")]
        {
            for (name, _) in &names {
                assert!(
                    crate::themes::retro::name_fits(name),
                    "{name} is too long for retro's THEME row"
                );
            }
            // 50 px of the small font: "TOKYO STORMS" (47) fits, one letter
            // more (51) doesn't.
            assert!(crate::themes::retro::name_fits("TOKYO STORMS"));
            assert!(!crate::themes::retro::name_fits("TOKYO STORMSS"));
        }
        for (i, &(name, id)) in names.iter().enumerate() {
            for &(other, other_id) in &names[i + 1..] {
                assert_ne!(name, other, "two schemes called {name}");
                assert_ne!(id, other_id, "{name} and {other} have one ID");
            }
        }
    }
}
#[path = "../../src/ui.rs"]
#[allow(dead_code)]
mod ui;
#[path = "../../src/ui_loop.rs"]
mod ui_loop;
#[path = "../../src/wake.rs"]
#[allow(dead_code)]
mod wake;

// ==============================================================================
// Stand-ins for the hardware's
// ==============================================================================

#[path = "stub/activity.rs"]
mod activity;
#[path = "stub/ble.rs"]
mod ble;
#[path = "stub/display.rs"]
mod display;
#[path = "stub/power.rs"]
mod power;

/// `src/dfu.rs`: only how far an update has got, which `update` sets.
mod dfu {
    use core::cell::Cell;

    use embassy_sync::blocking_mutex::Mutex;
    use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;

    pub static PROGRESS: Mutex<CriticalSectionRawMutex, Cell<Option<u8>>> =
        Mutex::new(Cell::new(None));
}

/// `src/debug.rs`: only the backdating of a test notification, which the
/// simulator's `notify` offers too, and the injected battery voltage,
/// which the simulator's `battery` sets as the measured one instead.
mod debug {
    use std::sync::atomic::{AtomicI64, Ordering};

    static BACKDATE_MINUTES: AtomicI64 = AtomicI64::new(0);

    pub fn take_backdate_minutes() -> i64 {
        BACKDATE_MINUTES.swap(0, Ordering::Relaxed)
    }

    pub fn backdate_next(minutes: i64) {
        BACKDATE_MINUTES.store(minutes, Ordering::Relaxed);
    }

    pub fn battery_sample() -> Option<i16> {
        None
    }
}

/// `src/version.rs`: the version the Info screen shows, which the
/// `version` command can set, so that screenshots don't change with each
/// release (the tour and the themes' pin one); build.rs's otherwise.
mod version {
    use std::sync::Mutex;

    static SHOWN: Mutex<Option<(&'static str, &'static str)>> = Mutex::new(None);

    pub fn shown() -> (&'static str, &'static str) {
        SHOWN
            .lock()
            .expect("nothing panics holding it")
            .unwrap_or((env!("UR_RELEASE"), env!("UR_BUILD")))
    }

    pub fn set(version: (&'static str, &'static str)) {
        *SHOWN.lock().expect("nothing panics holding it") = Some(version);
    }
}

/// `src/system_off.rs`: only why the watch switches off, for its last
/// screen. Switching off itself is [`watch`]'s.
mod system_off {
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    pub enum Reason {
        BatteryEmpty,
        Storage,
    }
}

/// `src/touch.rs`, the touch controller's driver: only its types, which
/// are [`crate::gesture`]'s, and whether the UI loop has put it to sleep.
/// [`watch`] feeds the gesture recogniser itself.
mod touch {
    use std::sync::atomic::{AtomicBool, Ordering};

    pub use crate::gesture::{Gesture, Touch};

    static ASLEEP: AtomicBool = AtomicBool::new(false);

    pub fn set_asleep(asleep: bool) {
        ASLEEP.store(asleep, Ordering::Relaxed);
    }

    /// Whether touches go unnoticed (see `set_asleep`).
    pub fn asleep() -> bool {
        ASLEEP.load(Ordering::Relaxed)
    }
}

pub use ui_loop::InputEvent;

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
    aranet::Why,
    buttons::Setting,
    temperature::Source,
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
mod flash;
mod panel;
mod platform;
mod record;
mod script;
mod settings;
mod shot;
#[cfg(any(feature = "window", target_arch = "wasm32"))]
mod status;
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
#[cfg(not(target_arch = "wasm32"))]
mod fuzz;
/// For the coverage-guided fuzzer in `fuzz/`.
#[cfg(not(target_arch = "wasm32"))]
pub use fuzz::{host_one, host_script};
/// For its other target: the simulator against a model of Gadgetbridge.
#[cfg(not(target_arch = "wasm32"))]
pub mod gadgetbridge;
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
