//! The simulator's clock: embassy's time driver, at the watch's tick rate
//! (its RTC's 32 768 Hz, so durations round as they do there), moved only
//! when told.
//!
//! Nothing here wakes a waiting future: everything that waits (a
//! transition's frame timer, and the panel's transfers) is polled again
//! once the clock has moved on to the earliest time any of them asked to
//! be woken at (see `watch::run`), and a timer checks the time when
//! polled.
//!
//! Before it moves, a recording (`crate::record`) looks at the screen:
//! whatever changed, changed at the instant the clock is leaving.

use std::sync::atomic::{AtomicU64, Ordering};
use std::task::Waker;

use embassy_time::Duration;
use embassy_time_driver::Driver;

static NOW: AtomicU64 = AtomicU64::new(0);
/// The earliest wake-up asked for since [`take_wake`] last looked, or
/// `u64::MAX` for none.
static WAKE: AtomicU64 = AtomicU64::new(u64::MAX);

struct Clock;

impl Driver for Clock {
    fn now(&self) -> u64 {
        NOW.load(Ordering::Relaxed)
    }

    fn schedule_wake(&self, at: u64, _waker: &Waker) {
        WAKE.fetch_min(at, Ordering::Relaxed);
    }
}

embassy_time_driver::time_driver_impl!(static CLOCK: Clock = Clock);

/// Moves the clock to `at`, if that's later (to a dev kit's instant).
#[cfg(not(target_arch = "wasm32"))]
pub fn set(at: embassy_time::Instant) {
    crate::record::clock_moving(at);
    NOW.fetch_max(at.as_ticks(), Ordering::Relaxed);
}

pub fn advance(by: Duration) {
    crate::record::clock_moving(embassy_time::Instant::now() + by);
    NOW.fetch_add(by.as_ticks(), Ordering::Relaxed);
}

/// Starts the clock again from 0, as the watch's uptime does when it boots
/// after switching off. Everything that keeps an `Instant` must have
/// forgotten it.
pub fn restart() {
    crate::record::clock_moving(embassy_time::Instant::now());
    crate::record::clock_restarting();
    NOW.store(0, Ordering::Relaxed);
    // Wake-ups asked for on the old clock mean nothing on the new.
    WAKE.store(u64::MAX, Ordering::Relaxed);
}

/// The earliest time a timer has asked to be woken at since the last
/// call, if any.
pub fn take_wake() -> Option<embassy_time::Instant> {
    let at = WAKE.swap(u64::MAX, Ordering::Relaxed);
    (at != u64::MAX).then(|| embassy_time::Instant::from_ticks(at))
}
