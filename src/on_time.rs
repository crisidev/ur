//! Time spent in a state, split into reporting periods: the bookkeeping
//! behind [`crate::power`]'s hourly on-time counters, without the clock, so
//! the host tests can drive it.
//!
//! A state that's on when a period's report is taken counts towards that
//! period up to the report, and towards the next from then on. Counting a
//! state only when it ended put a whole night of keeping still, or a day's
//! connection to the phone, into the hour it ended in (far more than an
//! hour's worth), and nothing into the hours before.
//!
//! Times are milliseconds since boot, truncated to `u32`: they wrap every
//! 49.7 days, which the wrapping arithmetic below handles as long as no
//! stretch between two calls on one counter is that long. Reports, which
//! call every counter, come hourly.

use core::sync::atomic::{AtomicU32, Ordering};

/// One state's time on, since the last [`OnTime::take`].
///
/// The state can be on more than once at a time (two clients connected,
/// say): each [`OnTime::on`] is matched by an [`OnTime::off`], and the time
/// counts once for each, as each draws its own current.
///
/// Atomics only so it can live in a `static`: the firmware's tasks all run
/// on the one thread-mode executor, so no update interrupts another.
pub struct OnTime {
    /// Milliseconds on since the last report, up to `since`.
    total: AtomicU32,
    /// How many times the state is on now.
    open: AtomicU32,
    /// When `total` was last brought up to date.
    since: AtomicU32,
}

impl OnTime {
    pub const fn new() -> Self {
        Self {
            total: AtomicU32::new(0),
            open: AtomicU32::new(0),
            since: AtomicU32::new(0),
        }
    }

    /// The state turned on at `now`.
    pub fn on(&self, now: u32) {
        self.catch_up(now);
        self.open.fetch_add(1, Ordering::Relaxed);
    }

    /// The state turned off at `now`. An `off` without an `on` is ignored.
    pub fn off(&self, now: u32) {
        self.catch_up(now);
        let open = self.open.load(Ordering::Relaxed);
        self.open.store(open.saturating_sub(1), Ordering::Relaxed);
    }

    /// Milliseconds on since the last call, up to `now`, whether or not the
    /// state is still on. If it is, the rest goes to the next call.
    pub fn take(&self, now: u32) -> u32 {
        self.catch_up(now);
        self.total.swap(0, Ordering::Relaxed)
    }

    /// Adds the time on since the last update, up to `now`.
    fn catch_up(&self, now: u32) {
        let since = self.since.swap(now, Ordering::Relaxed);
        let open = self.open.load(Ordering::Relaxed);
        // While off, `since` may be long stale (and wrapped), but nothing
        // is added then.
        if open > 0 {
            let elapsed = now.wrapping_sub(since).saturating_mul(open);
            let total = self.total.load(Ordering::Relaxed);
            self.total
                .store(total.saturating_add(elapsed), Ordering::Relaxed);
        }
    }
}
