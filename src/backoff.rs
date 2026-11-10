//! When a sensor's setup is retried, and which failures are worth a log
//! line.
//!
//! A sensor whose setup fails must keep trying: on a sealed watch, giving
//! up means no steps or heart rate until the next reboot, which may be
//! weeks away, and nobody would know why. Most failures are a transfer
//! caught by something else on the shared bus, and a retry a moment later
//! works; a bus that stays stuck may need seconds. So retries start quick
//! and slow down to [`WAITS_MS`]'s last, then carry on at that pace for as
//! long as it takes.
//!
//! Every failure costs log space, and the field log is flash, so the
//! retries log only while the waits still grow, then once in
//! [`LOG_EVERY`] (about hourly): enough to see that a sensor is still
//! failing, without a dead one filling the log.
//!
//! Hardware-free, for the host tests in `host-tests/backoff`.

/// How long to wait after each failed attempt in turn, in ms; the last
/// repeats for good. The first is short: an attempt cut short by
/// something else on the bus works when tried again at once.
/// The waits only get long once the failures have gone on for ~20 s: a
/// bus that's stuck that long is stuck for good, or nearly, but a run of
/// a few failures (a wedged bus soon recovered) shouldn't leave a sensor
/// waiting half a minute after it's fine again.
pub const WAITS_MS: [u32; 9] = [100, 200, 500, 1_000, 2_000, 5_000, 10_000, 30_000, 60_000];

/// Once the waits have stopped growing, one failure in this many is logged.
pub const LOG_EVERY: u16 = 60;

/// A sensor's run of failed setup attempts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Backoff {
    failures: u16,
}

/// What to do after a failed attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Retry {
    /// How many attempts have failed so far (1 after the first).
    pub failures: u16,
    /// How long to wait before the next attempt, in ms.
    pub wait_ms: u32,
    /// Whether to log this failure.
    pub log: bool,
}

impl Backoff {
    pub const fn new() -> Self {
        Self { failures: 0 }
    }

    /// Whether the attempt about to be made should log its failure, should
    /// it fail: what [`Backoff::failed`] will then say. For the drivers,
    /// whose transfers log their own failures.
    pub fn loud(&self) -> bool {
        logged(self.failures.saturating_add(1))
    }

    /// Records a failed attempt.
    pub fn failed(&mut self) -> Retry {
        // Saturating: at a minute apiece, 65 535 failures is six weeks, and
        // by then the wait and the logging no longer change anyway.
        self.failures = self.failures.saturating_add(1);
        let step = usize::from(self.failures - 1).min(WAITS_MS.len() - 1);
        Retry {
            failures: self.failures,
            wait_ms: WAITS_MS[step],
            log: logged(self.failures),
        }
    }
}

/// Whether the `n`th failure in a row (from 1) is logged.
fn logged(n: u16) -> bool {
    n <= WAITS_MS.len() as u16 || n.is_multiple_of(LOG_EVERY)
}

/// Whether the `n`th time in a row (from 1) that the same thing goes wrong
/// is worth a log line: the first two, then every power of two. For
/// failures that can repeat often, but whose every repeat says nothing new
/// (the bus found stuck again): a few lines a day even if it never ends.
pub fn worth_logging(n: u16) -> bool {
    n <= 2 || n.is_power_of_two()
}
