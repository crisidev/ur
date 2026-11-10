//! Which reads of the accelerometer's step counter to believe.
//!
//! The BMA421 keeps a running count of steps since its setup at boot,
//! which [`crate::activity`] reads every couple of seconds; the steps
//! taken since the last read are the difference. The count only ever goes
//! up, but a read can be garbage: right after a broken FIFO batch it has
//! read 0 for one poll, then the true count again (7 times in one night),
//! and a read the chip answers with its filler (`e0 e0 e0 e0`, as it does
//! for a second FIFO burst in a row) would be 3.8 billion. Taken at
//! face value, either is billions of steps today and a saturated minute,
//! which the sleep classifier reads as getting up.
//!
//! So a read is only counted if no wrist could have walked it since the
//! last one counted: not lower, and not more than [`most_steps`] higher.
//! Anything else is ignored, and the count carries on from the last good
//! one.
//!
//! That alone can get stuck. A garbage read that passes for a count (a
//! misread low byte, say, after a gap in the reads the bound has grown
//! over) leaves every true count below it ignored until the wearer has
//! walked the difference, which could take days. So an ignored read is
//! remembered, and if the next one follows on from it as a counter would,
//! the count picks up from there instead ([`Read::Moved`]). Two
//! consistent reads in a row are always taken (see the proofs in
//! `host-tests/step_counter`), so whatever the count was, counting
//! resumes within two polls of the chip counting sensibly. The price: the
//! steps across a move (a poll's or two) are lost, and two identical
//! garbage reads in a row move the count to them, after which the true
//! count is ignored once, then picked up again.
//!
//! Hardware-free, for the host tests in `host-tests/step_counter`.

/// The fastest a wrist steps, per second: 300 steps a minute, more than
/// any sprinter's cadence (~200–220). The chip counts by the arm's swing,
/// two steps a swing, so it can't count faster than an arm swings.
pub const MAX_PER_SECOND: u32 = 5;

/// Steps the chip may add at once on top of that: it holds back the first
/// steps of a walk until four swings in a row (8 steps) have shown it's
/// one, then adds them all together; the field logs have runs starting
/// with 8 to 12 at once. More than twice that, for margin.
pub const MAX_BURST: u32 = 32;

/// The most steps the counter can rise by in `seconds` (whole seconds, as
/// the caller's clock gives them): a second more than that, as both ends'
/// times are truncated, and a burst.
pub fn most_steps(seconds: u32) -> u32 {
    MAX_PER_SECOND
        .saturating_mul(seconds.saturating_add(1))
        .saturating_add(MAX_BURST)
}

/// What a read of the counter came to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Read {
    /// A count that follows on from the last: this many new steps.
    Counted(u32),
    /// A count no wrist could have reached from the last (see
    /// [`most_steps`]): ignored, and remembered, in case the next read
    /// follows on from it.
    Ignored,
    /// A count that follows on from the one ignored just before, as a
    /// counter would: the count carries on from here, with this many new
    /// steps since the ignored one.
    Moved(u32),
}

/// The step counter's last believable count, and when it was read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StepCounter {
    /// The count steps are counted from.
    last: u32,
    /// When it was read, in seconds (any clock, as long as it's the same
    /// one throughout and doesn't go backwards).
    at: u32,
    /// The read ignored just before, and when: what the next read is
    /// checked against if it doesn't follow on from `last`.
    ignored: Option<(u32, u32)>,
}

impl StepCounter {
    /// A counter set up at `now`, from 0 (as the chip's setup leaves it).
    pub const fn new(now: u32) -> Self {
        Self {
            last: 0,
            at: now,
            ignored: None,
        }
    }

    /// The count steps are counted from, for the log.
    pub fn last(&self) -> u32 {
        self.last
    }

    /// Judges a read of `count`, made at `now`.
    pub fn read(&mut self, count: u32, now: u32) -> Read {
        if let Some(steps) = follows(count, now, self.last, self.at) {
            self.take(count, now);
            Read::Counted(steps)
        } else if let Some(steps) = self
            .ignored
            .and_then(|(ignored, at)| follows(count, now, ignored, at))
        {
            self.take(count, now);
            Read::Moved(steps)
        } else {
            self.ignored = Some((count, now));
            Read::Ignored
        }
    }

    fn take(&mut self, count: u32, now: u32) {
        (self.last, self.at, self.ignored) = (count, now, None);
    }
}

/// How many steps `count` (at `now`) is on from `before` (at `then`), if a
/// counter could have got there: not lower, and no more than
/// [`most_steps`] in the time between.
fn follows(count: u32, now: u32, before: u32, then: u32) -> Option<u32> {
    let steps = count.checked_sub(before)?;
    (steps <= most_steps(now.saturating_sub(then))).then_some(steps)
}
