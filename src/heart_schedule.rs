//! When to measure the heart rate in the background: an interval that
//! adapts to what the wearer is doing, within an hourly LED budget.
//!
//! [`crate::heart`] measures in sessions of ~13–30 s of LED, each costing
//! ~2.7 mA while it runs. At rest, the reading barely changes from one
//! session to the next (a night's field log matched a Garmin to a median
//! 1 BPM at a reading every 10 minutes), so measuring often buys nothing.
//! What's worth catching is *change*: activity starting, or the heart rate
//! still up after it. Hence:
//!
//! - **At rest the interval backs off**, doubling after each ordinary
//!   reading from [`MIN_INTERVAL`] up to [`MAX_INTERVAL`].
//! - **Movement resets it** to [`MIN_INTERVAL`]: steps, or vigorous
//!   movement, in at least [`MOVED_POLLS`] of the accelerometer's 2-second
//!   polls since the last session, may mean activity is starting. (The
//!   session itself still waits for the wrist to be still, which `heart`
//!   checks: a moving wrist gives no reading. So in practice this means
//!   "at the first calm moment after the movement, but no sooner than
//!   [`MIN_INTERVAL`] after the last session".) Asleep, movement is
//!   turning over, not activity, and doesn't count.
//! - **Asleep, it stays at [`ASLEEP_INTERVAL`]** (5 minutes): sleep
//!   staging reads REM from the heart rate rising a few beats over the
//!   night's baseline, and deep from it staying flat, which a reading every
//!   20 minutes can't see. (Simulated on 60 of a Garmin's nights,
//!   agreement with its stages fell from ~64% at a reading every 5 minutes
//!   to ~59% at 20.)
//! - **An elevated reading keeps it short** ([`ELEVATED_INTERVAL`]): at
//!   least [`ELEVATED_BY`] over the wearer's resting heart rate, learnt
//!   from earlier readings (separately asleep and awake; see
//!   [`Schedule::resting`]).
//! - **A session without a reading backs off on its own**
//!   ([`FAILED_INTERVAL`] doubling to [`MAX_FAILED_INTERVAL`]), which
//!   movement doesn't undo: the usual causes (a fidgety wrist, the watch
//!   on a desk) would otherwise retry every few minutes all day. Only a
//!   watch that was *off* the wrist and then moved retries sooner: it was
//!   likely picked up to be put on.
//! - **An LED budget** ([`LED_BUDGET`] seconds an hour, as a token bucket)
//!   caps what all of the above can spend, however the day goes.
//!
//! Sessions while the clock face shows aren't scheduled (someone is
//! looking, so `heart` measures right away), but their readings count as
//! measurements, and their LED time comes out of the budget.
//!
//! Time is whole seconds since boot, as `u32` (136 years). Like
//! [`crate::sleep`], this knows nothing of hardware or the clock, so it
//! runs in host tests (`host-tests/heart_schedule`).

// ==============================================================================
// Constants
// ==============================================================================

/// The shortest wait between sessions: after movement, and the start of
/// the back-off. Long enough that one bout of fidgeting can't make
/// back-to-back sessions; short enough to catch a heart rate ramping up
/// within a couple of minutes of activity starting.
pub const MIN_INTERVAL: u32 = 2 * 60;

/// The longest wait at rest. "Every 15–20 minutes at rest is probably
/// fine": three sessions an hour, half of the fixed 10 minutes this
/// replaced (whose ~13 s sessions cost ~77 s of LED an hour in a night's
/// field log).
pub const MAX_INTERVAL: u32 = 20 * 60;

/// The interval while asleep, for sleep staging (see the module docs).
/// Twelve ~12 s sessions an hour: ~144 s of LED, inside [`LED_BUDGET`],
/// and ~0.65 mAh more a night than backing off to [`MAX_INTERVAL`].
pub const ASLEEP_INTERVAL: u32 = 5 * 60;

/// The wait after an elevated reading (see [`ELEVATED_BY`]), to follow a
/// heart rate that is up (or coming down after exercise; recovery takes
/// ~5–20 minutes) a few points at a time. Not [`MIN_INTERVAL`]: if the
/// resting estimate is off, or the wearer is simply up and about, this
/// repeats for as long as the readings stay high, and four minutes keeps
/// that within [`LED_BUDGET`] (15 sessions of ~13 s an hour).
pub const ELEVATED_INTERVAL: u32 = 4 * 60;

/// BPM above the resting estimate that counts as elevated. Resting
/// readings scatter a few BPM (the PPG's median error against a Garmin
/// was 1 BPM, sitting up versus lying down a few more), so this is well
/// clear of noise while still catching a stroll's 20–30 BPM.
///
/// TODO: tune from field logs with readings after activity.
pub const ELEVATED_BY: u8 = 15;

/// The wait after a session without a reading; it doubles with each
/// further failure in a row, up to [`MAX_FAILED_INTERVAL`]. A failure at
/// rest is worth a retry soonish (calm is necessary for a reading, not
/// sufficient), but not a tight loop: each failure costs a whole
/// `SESSION_LIMIT` (30 s) of LED, or climbs to 40 mA off the wrist.
pub const FAILED_INTERVAL: u32 = 5 * 60;

/// The longest wait after failures: the watch is likely off the wrist (on
/// a nightstand, in a drawer). Showing the clock face still measures at
/// once, which is how putting the watch back on usually starts.
pub const MAX_FAILED_INTERVAL: u32 = 60 * 60;

/// Accelerometer polls (2 s each) with steps or vigorous movement, since
/// the last session, that count as "the wrist has moved". Six is ~12 s:
/// a few steps across a room, not a single flick of the wrist (turning it
/// to look at the watch, or a gesture at a desk) whose bursts reach the
/// same movement levels for a poll or two.
///
/// TODO: tune from a field log with walking and desk work in it; a long
/// interval at a desk can collect six stray polls too.
pub const MOVED_POLLS: u8 = 6;

/// LED seconds per hour that sessions may spend, background and watched
/// alike. 180 s is what the fixed schedule this replaced spent at worst
/// (six failing 30 s sessions an hour), and ~2.3× what a calm hour of it
/// actually used: room for a busy hour of elevated readings, but a bad
/// day (a fidgety wrist failing session after session) can't drain more
/// than ~0.14 mA on average.
pub const LED_BUDGET: u32 = 180;

/// Budget a background session needs before it may start: its longest
/// run, `SESSION_LIMIT`, so one session can't overdraw the bucket by
/// much. (Watched sessions ignore this; someone is looking.)
pub const SESSION_RESERVE: u32 = 30;

/// Seconds for the bucket to earn one second of LED back.
const REFILL_EVERY: u32 = 3600 / LED_BUDGET;
const _: () = assert!(3600 % LED_BUDGET == 0, "the refill rate must be whole");
const _: () = assert!(SESSION_RESERVE <= LED_BUDGET);

/// Consecutive failures counted (beyond this the wait is capped anyway).
const MAX_FAILURES: u8 = 5;

// Intervals are stored as `u16` to keep the heart task's state small.
const _: () = assert!(MAX_FAILED_INTERVAL <= u16::MAX as u32);
const _: () = assert!(LED_BUDGET <= u16::MAX as u32);

// ==============================================================================
// The schedule
// ==============================================================================

/// Why the next background session is due when it is; for the log.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[cfg_attr(target_os = "none", derive(defmt::Format))]
pub enum Why {
    /// No session yet since boot: measure right away, so the clock face
    /// has a number.
    First,
    /// The back-off interval at rest.
    Rest,
    /// The wrist moved since the last session.
    Moved,
    /// Asleep: the steady interval for sleep staging.
    Asleep,
    /// The last reading was elevated.
    Elevated,
    /// The last session(s) had no reading.
    Retry,
    /// Due already by one of the above, but the LED budget is spent until
    /// then.
    Budget,
}

/// How a session ended.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Outcome {
    /// A reading, in BPM (the last, if the session gave several).
    Reading(u8),
    /// No reading: a busy wrist, a poor signal, or a desk (which reflects
    /// the LED much like skin, without a pulse).
    NoReading,
    /// No skin, or ambient light flooding the sensor: not on a wrist.
    OffWrist,
}

/// The scheduling state, 20 bytes: the heart task holds it across every
/// await, so it lives in that task's static future, where RAM is tight.
#[derive(Clone, Debug)]
pub struct Schedule {
    /// When the last counted session ended.
    last: u32,
    /// The wait after `last` at rest, in seconds; 0 before the first
    /// session (which makes it due at once).
    interval: u16,
    /// Consecutive sessions without a reading.
    failures: u8,
    /// Whether the last failure found no skin (see [`Outcome::OffWrist`]).
    off_wrist: bool,
    /// Whether the last reading was elevated (so `interval` is
    /// [`ELEVATED_INTERVAL`] for that reason; for [`Why`]).
    elevated: bool,
    /// Resting estimates, awake and asleep, in BPM; 0 before the first.
    resting_awake: u8,
    resting_asleep: u8,
    /// LED seconds left in the budget as of `refilled_at`.
    tokens: u16,
    /// When `tokens` was last brought up to date.
    refilled_at: u32,
}

impl Default for Schedule {
    fn default() -> Self {
        Self::new()
    }
}

impl Schedule {
    pub const fn new() -> Self {
        Self {
            last: 0,
            interval: 0,
            failures: 0,
            off_wrist: false,
            elevated: false,
            resting_awake: 0,
            resting_asleep: 0,
            tokens: LED_BUDGET as u16,
            refilled_at: 0,
        }
    }

    /// When the next background session is due (seconds since boot), and
    /// why. `active_polls` is how many accelerometer polls since the last
    /// session had steps or vigorous movement; `asleep`, whether the sleep
    /// classifier thinks the wearer is asleep.
    ///
    /// Due isn't "start now": `heart` also waits for a still wrist.
    pub fn due(&self, active_polls: u8, asleep: bool) -> (u32, Why) {
        if self.interval == 0 {
            return (0, Why::First);
        }
        let moved = moved(active_polls, asleep);
        let (wait, why) = if self.failures > 0 {
            let retry = (FAILED_INTERVAL << (self.failures - 1)).min(MAX_FAILED_INTERVAL);
            // Movement after a failure only shortens the wait if the watch
            // was off the wrist (it was probably picked up to be worn), and
            // otherwise only caps it at the resting interval: something
            // changed, but the last attempt at that wrist failed.
            let wait = match (moved, self.off_wrist) {
                (false, _) => retry,
                (true, true) => retry.min(FAILED_INTERVAL),
                (true, false) => retry.min(MAX_INTERVAL),
            };
            (wait, Why::Retry)
        } else if moved {
            (MIN_INTERVAL, Why::Moved)
        } else if self.elevated {
            (u32::from(self.interval), Why::Elevated)
        } else if asleep {
            // Falling asleep mid back-off shouldn't wait out a long
            // interval.
            (u32::from(self.interval).min(ASLEEP_INTERVAL), Why::Asleep)
        } else {
            (u32::from(self.interval), Why::Rest)
        };
        let at = self.last.saturating_add(wait);

        // The budget: when will the bucket hold a whole session's worth?
        let tokens = u32::from(self.tokens);
        if tokens < SESSION_RESERVE {
            let affordable = self.refilled_at + (SESSION_RESERVE - tokens) * REFILL_EVERY;
            if affordable > at {
                return (affordable, Why::Budget);
            }
        }
        (at, why)
    }

    /// Records a session that ended at `now`, having had the LED on for
    /// `led_seconds`. `background` says whether the schedule started it
    /// (rather than someone looking at the clock face); `active_polls` and
    /// `asleep` are as for [`Self::due`], as they stood when it started.
    pub fn ended(
        &mut self,
        now: u32,
        led_seconds: u32,
        outcome: Outcome,
        background: bool,
        active_polls: u8,
        asleep: bool,
    ) {
        self.refill(now);
        self.tokens = self
            .tokens
            .saturating_sub(led_seconds.min(u32::from(u16::MAX)) as u16);

        // Someone looked at the clock face and no reading came of it: it
        // says nothing about the wrist the schedule cares about (the
        // watch may well be in a hand, or on a desk), so it neither
        // counts as a measurement nor as a failure. The LED time still
        // came out of the budget above.
        if !background && !matches!(outcome, Outcome::Reading(_)) {
            return;
        }
        let moved = moved(active_polls, asleep);
        self.last = now;
        match outcome {
            Outcome::Reading(bpm) => {
                self.failures = 0;
                self.off_wrist = false;
                // Compared against the estimate for the wearer's state.
                // Asleep before a sleeping estimate exists, the awake one
                // stands in: sleep only lowers the heart rate, so it can't
                // make a reading look elevated that isn't.
                let resting = match (asleep, self.resting_asleep) {
                    (true, r) if r != 0 => r,
                    _ => self.resting_awake,
                };
                self.elevated = resting != 0 && bpm >= resting.saturating_add(ELEVATED_BY);
                // Only a background reading of a wrist that has been still
                // since the last session is a resting one. (A watched
                // reading comes right after the wrist turned to look.)
                if background && !moved {
                    let estimate = if asleep {
                        &mut self.resting_asleep
                    } else {
                        &mut self.resting_awake
                    };
                    track_resting(estimate, bpm);
                }
                // The back-off: after movement (or at boot) it starts over
                // from the minimum; an elevated reading holds it short.
                let base = if moved || self.interval == 0 {
                    MIN_INTERVAL
                } else {
                    u32::from(self.interval)
                };
                let next = if self.elevated {
                    ELEVATED_INTERVAL
                } else if asleep {
                    ASLEEP_INTERVAL
                } else {
                    (base * 2).min(MAX_INTERVAL)
                };
                self.interval = next as u16;
            }
            Outcome::NoReading | Outcome::OffWrist => {
                self.failures = (self.failures + 1).min(MAX_FAILURES);
                self.off_wrist = outcome == Outcome::OffWrist;
                self.elevated = false;
                // The resting back-off carries on from where it was once a
                // reading comes; only the first session sets it.
                if self.interval == 0 {
                    self.interval = MIN_INTERVAL as u16;
                }
            }
        }
    }

    /// The resting heart rate estimates, awake and asleep (0: none yet).
    pub fn resting(&self) -> (u8, u8) {
        (self.resting_awake, self.resting_asleep)
    }

    /// LED seconds left in the budget at `now`.
    pub fn budget(&self, now: u32) -> u32 {
        self.tokens_at(now)
    }

    fn tokens_at(&self, now: u32) -> u32 {
        let earned = now.saturating_sub(self.refilled_at) / REFILL_EVERY;
        (u32::from(self.tokens) + earned).min(LED_BUDGET)
    }

    /// Brings `tokens` up to date, keeping the part-earned second (so
    /// frequent refills don't lose time to rounding).
    fn refill(&mut self, now: u32) {
        let tokens = self.tokens_at(now);
        if tokens == LED_BUDGET {
            self.refilled_at = now;
        } else {
            let earned = tokens - u32::from(self.tokens);
            self.refilled_at += earned * REFILL_EVERY;
        }
        self.tokens = tokens as u16;
    }
}

/// Whether the accelerometer's polls since the last session say the wrist
/// moved in a way that may be activity starting.
fn moved(active_polls: u8, asleep: bool) -> bool {
    // Asleep, movement is turning over. Waking up is recognised by the
    // sleep classifier within minutes (at once on walking), after which
    // movement counts again.
    !asleep && active_polls >= MOVED_POLLS
}

/// Moves a resting estimate towards a resting reading: the running median
/// estimator (one step towards each reading), which needs one byte where a
/// window of readings would need dozens, and which a few readings taken
/// while still recovering from exercise barely move.
///
/// Asymmetric, though: up by 1 BPM a reading, but down by a quarter of the
/// gap. Readings well above rest are the common outlier (recovery after
/// activity, or the first reading after boot being taken mid-afternoon),
/// readings well below it are rare, and an estimate stuck too high would
/// hide exactly the elevated readings this is for. Too low, it only costs
/// some extra sessions, which [`LED_BUDGET`] bounds.
///
/// Elevated readings teach it too, a beat at a time. Leaving them out would
/// track recovery after exercise a little longer, but an estimate that
/// started out low (readings lying in bed before the sleep classifier
/// notices sleep, say) could then never climb back, and every ordinary
/// reading after would count as elevated. This way a reading
/// [`ELEVATED_BY`] + n over the estimate stops counting after about n
/// readings.
fn track_resting(estimate: &mut u8, bpm: u8) {
    if *estimate == 0 {
        *estimate = bpm;
    } else if bpm > *estimate {
        *estimate += 1;
    } else if bpm < *estimate {
        *estimate -= (*estimate - bpm).div_ceil(4);
    }
}
