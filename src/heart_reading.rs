//! The heart rate as the rest of the firmware sees it: the latest reading
//! (and whether it's recent enough to show), and whether someone's waiting
//! for one: looking at the clock face, or at Gadgetbridge's live activity
//! (see [`crate::live`]).
//!
//! This is the hardware-free half of [`crate::heart`], which drives the
//! sensor and publishes what it measures here. It's a module of its own so
//! that the simulator (`sim/`) runs it too: the UI loop
//! ([`crate::ui_loop`]) reads [`current`] and calls [`set_watching`] there
//! as on the watch, with readings the simulator [`publish`]es instead of
//! the sensor.

use core::cell::Cell;
use core::sync::atomic::{AtomicBool, Ordering};

use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Instant};

/// How long a reading is shown on the clock face. Past this (longer than
/// the schedule waits at rest), "--" is more honest than an old number.
const STALE_AFTER: Duration = Duration::from_secs(30 * 60);

/// Whether the clock face is showing; set by the UI loop.
static WATCHING: AtomicBool = AtomicBool::new(false);
/// Whether Gadgetbridge's live activity wants the heart rate; set by the
/// UI loop.
static LIVE: AtomicBool = AtomicBool::new(false);
/// Raised when [`WATCHING`] or [`LIVE`] becomes true, to start a session
/// right away, and when `activity::still` changes (see [`wake`]).
pub static WAKE: Signal<CriticalSectionRawMutex, ()> = Signal::new();

/// Whether the last session saw skin; for sleep tracking, which must tell
/// a still wrist from a watch on the nightstand, and the temperature's
/// wear model.
static WORN: Mutex<CriticalSectionRawMutex, Cell<Option<bool>>> = Mutex::new(Cell::new(None));

/// Whether the watch is on a wrist, as of the last measurement session
/// (`None` before the first). Up to an hour stale (see
/// `heart_schedule::MAX_FAILED_INTERVAL`).
pub fn worn() -> Option<bool> {
    WORN.lock(Cell::get)
}

/// What a measurement session found: skin, or none.
pub fn set_worn(worn: Option<bool>) {
    WORN.lock(|w| w.set(worn));
}

/// The latest reading and when it was taken.
static LATEST: Mutex<CriticalSectionRawMutex, Cell<Option<(u8, Instant)>>> =
    Mutex::new(Cell::new(None));

/// Tells the heart-rate task whether someone's looking at the clock face.
/// Cheap; the UI loop calls it every pass.
pub fn set_watching(watching: bool) {
    let was = WATCHING.swap(watching, Ordering::Relaxed);
    if watching && !was {
        WAKE.signal(());
    }
}

/// Tells the heart-rate task whether Gadgetbridge's live activity wants
/// the heart rate: measured continuously, as while someone looks at the
/// clock face, but for as long as it asks. Cheap; the UI loop calls it
/// every pass.
pub fn set_live(live: bool) {
    let was = LIVE.swap(live, Ordering::Relaxed);
    if live && !was {
        WAKE.signal(());
    }
}

/// Has the heart-rate task look again at what it's waiting for: the wrist
/// just went still (a due session may start), or started moving (which
/// may bring the next session forward). Cheap.
pub fn wake() {
    WAKE.signal(());
}

/// Whether someone's waiting for a reading (see [`set_watching`] and
/// [`set_live`]): measure continuously.
pub fn watching() -> bool {
    WATCHING.load(Ordering::Relaxed) || LIVE.load(Ordering::Relaxed)
}

/// The latest heart rate and when it was measured, however old.
pub fn latest() -> Option<(u8, Instant)> {
    LATEST.lock(Cell::get)
}

/// The latest heart rate, if it's recent enough to show.
pub fn current(now: Instant) -> Option<u8> {
    latest()
        .filter(|&(_, at)| now.saturating_duration_since(at) <= STALE_AFTER)
        .map(|(bpm, _)| bpm)
}

/// A new reading, measured now: for the clock face, and this minute's
/// history.
pub fn publish(bpm: u8) {
    LATEST.lock(|latest| latest.set(Some((bpm, Instant::now()))));
    crate::activity::record_heart_rate(bpm);
}

/// Drops the latest reading, as switching off does to everything in RAM;
/// for the simulator, whose RAM outlives its switching off.
#[cfg(not(target_os = "none"))]
pub fn forget() {
    LATEST.lock(|latest| latest.set(None));
}
