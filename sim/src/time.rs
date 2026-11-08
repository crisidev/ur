//! The simulator's clock: embassy's time driver, at the watch's tick rate
//! (its RTC's 32 768 Hz, so durations round as they do there), moved only
//! when told.
//!
//! Nothing here wakes a waiting future: everything that waits (only a
//! transition's frame timer) is polled again each time the clock moves
//! (see `watch::run`), and a timer checks the time when polled.

use std::sync::atomic::{AtomicU64, Ordering};
use std::task::Waker;

use embassy_time::{Duration, Instant};
use embassy_time_driver::Driver;

static NOW: AtomicU64 = AtomicU64::new(0);

struct Clock;

impl Driver for Clock {
    fn now(&self) -> u64 {
        NOW.load(Ordering::Relaxed)
    }

    fn schedule_wake(&self, _at: u64, _waker: &Waker) {}
}

embassy_time_driver::time_driver_impl!(static CLOCK: Clock = Clock);

/// Moves the clock to `at`, if that's later.
pub fn set(at: Instant) {
    NOW.fetch_max(at.as_ticks(), Ordering::Relaxed);
}

pub fn advance(by: Duration) {
    NOW.fetch_add(by.as_ticks(), Ordering::Relaxed);
}
