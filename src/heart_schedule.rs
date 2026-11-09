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
//!   likely picked up to be put on. A session that failed on a *calm*
//!   wrist backs off only to [`CALM_RETRY_INTERVAL`], for up to
//!   [`CALM_RETRIES`] in a row: stillness is when a reading is possible at
//!   all, and the signal of a still wrist comes and goes.
//! - **An LED budget** ([`LED_BUDGET`] seconds an hour, as a token bucket)
//!   caps what all of the above can spend, however the day goes.
//!
//! Sessions while the clock face shows aren't scheduled (someone is
//! looking, so `heart` measures right away), but their readings count as
//! measurements, and their LED time comes out of the budget.
//!
//! Two rules about the sessions themselves live here too, being as
//! hardware-free: a background session only starts once the wrist has
//! been calm for a few seconds ([`START_CALM_POLLS`]), and gives up early
//! if it stays busy ([`give_up`]). At a desk, a hand rests for a moment
//! between movements, and sessions started in that moment spent their
//! 30 s of LED on a wrist that was soon moving again.
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
/// Shorter doesn't pay: simulated on two nights with a Garmin alongside and
/// 60 of its own, readings every 2 or 3 minutes staged sleep no better than
/// every 5, even with `crate::stages`' thresholds refitted to them.
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

/// The longest wait after a session that failed on a calm wrist (see
/// [`Outcome::CalmNoReading`]), for up to [`CALM_RETRIES`] such failures in
/// a row.
///
/// Lying still awake for an hour (a field log, 2026-11-09, with a Garmin
/// alongside reading 51-73), the back-off above allowed three sessions, at
/// 10 and then 20 minutes apart, and none read: the pulse was a few counts,
/// beside slower swings that came and went. A desk's busy wrist is what
/// the back-off is for, and calm failures are rarer (3 of 17 sessions in
/// that log, 5 of 104 in a night's, none of a desk day's 39), so this
/// costs little: an hour of calm failures takes 7 sessions instead of 4,
/// ~95 s more of LED, once (see [`CALM_RETRIES`]).
pub const CALM_RETRY_INTERVAL: u32 = 10 * 60;

/// Calm failures in a row after which the back-off goes on as for any
/// failure. A watch lying on something that reflects the LED like skin
/// (and keeps as still as a wrist) fails calm too, and would otherwise
/// spend the whole [`LED_BUDGET`] on it, all night; six (an hour) cover
/// lying still for a while.
pub const CALM_RETRIES: u8 = 6;

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

/// Accelerometer polls in a row (2 s each) calm enough to measure through
/// (`heart::CALM`) before a background session may start.
///
/// One calm poll used to be enough, and at a desk it rarely lasted: in a
/// field log of an afternoon there (2026-11-08), 10 of 13 failed
/// background sessions had started within two calm polls of movement, as
/// had the one that misread 42 BPM (see `ppg::MAX_SLOW_DRIFT`), and the
/// wrist averaged 245-1906 through them (the calm limit is 150). Three
/// polls, ~6 s, would have held those eleven back, and cost a still wrist
/// nothing: asleep, or at rest, nearly every poll is calm.
pub const START_CALM_POLLS: u8 = 3;

/// Seconds into a background session at which it gives up if no window
/// has passed the signal checks yet and the wrist has been busy (above
/// `heart::CALM`) at most of its level checks (see [`give_up`]).
///
/// The first two windows end at ~8.5 and ~10.5 s. Replaying 129 field-log
/// sessions (`host-tests/ppg`), giving up at 12 s cut 45 failing sessions
/// short, saving ~860 s of LED (~19 s each), and lost three background
/// readings (at a desk, from windows that only passed after 15 s of a busy
/// wrist); at 10 s it would have lost six.
pub const GIVE_UP_AFTER: u32 = 12;

/// Polls (2 s each) without steps after which a reading counts as resting
/// (see [`Schedule::ended`]): three minutes, by when the heart rate has
/// mostly come down after a walk.
///
/// Steps, not movement: at a desk, polls too vigorous to measure through
/// come every minute or two (gestures, typing), while steps stop for many
/// minutes (in a field log, stretches of 2-250 s without vigorous polls,
/// against 400-1600 s without steps). Counting either kept the resting
/// estimate empty all afternoon, as every reading followed some movement.
///
/// TODO: exercise without steps (cycling, weights) counts as rest this way;
/// its readings only nudge the estimate up a beat each (see
/// `track_resting`), but a long session could.
pub const RESTING_STEPLESS_POLLS: u8 = 90;

/// How far apart (BPM) the first two resting readings may be for the
/// estimate to start from them.
const SEED_AGREEMENT: u8 = 6;

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
    /// No reading, though the wrist was calm through the session (see
    /// [`calm_throughout`]): a weak pulse, or something that reflects like
    /// skin without one. Retried sooner than [`Outcome::NoReading`] (see
    /// [`CALM_RETRY_INTERVAL`]).
    CalmNoReading,
    /// No skin, or ambient light flooding the sensor: not on a wrist.
    OffWrist,
}

/// What the wrist had been doing when a session started, from `activity`;
/// for [`Schedule::ended`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Before {
    /// Polls with steps or vigorous movement since the last session (as
    /// for [`Schedule::due`]).
    pub active_polls: u8,
    /// Polls in a row without steps, up to the session (saturating).
    pub stepless_polls: u8,
    /// Whether the sleep classifier thought the wearer asleep.
    pub asleep: bool,
}

/// A resting heart rate estimate.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Resting {
    /// In BPM; 0 before it has started.
    bpm: u8,
    /// Before it has: the last resting reading, waiting for another to
    /// agree with it (0: none yet).
    candidate: u8,
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
    /// Consecutive calm failures (see [`Outcome::CalmNoReading`]), up to
    /// the latest; saturating.
    calm_failures: u8,
    /// Whether the last reading was elevated (so `interval` is
    /// [`ELEVATED_INTERVAL`] for that reason; for [`Why`]).
    elevated: bool,
    /// Resting estimates, awake and asleep.
    resting_awake: Resting,
    resting_asleep: Resting,
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
            calm_failures: 0,
            elevated: false,
            resting_awake: Resting {
                bpm: 0,
                candidate: 0,
            },
            resting_asleep: Resting {
                bpm: 0,
                candidate: 0,
            },
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
            let retry = if (1..=CALM_RETRIES).contains(&self.calm_failures) {
                retry.min(CALM_RETRY_INTERVAL)
            } else {
                retry
            };
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
    /// (rather than someone looking at the clock face); `before`, what the
    /// wrist had been doing when it started.
    pub fn ended(
        &mut self,
        now: u32,
        led_seconds: u32,
        outcome: Outcome,
        background: bool,
        before: Before,
    ) {
        let Before {
            active_polls,
            stepless_polls,
            asleep,
        } = before;
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
                self.calm_failures = 0;
                // Compared against the estimate for the wearer's state.
                // Asleep before a sleeping estimate exists, the awake one
                // stands in: sleep only lowers the heart rate, so it can't
                // make a reading look elevated that isn't.
                let resting = match (asleep, self.resting_asleep.bpm) {
                    (true, r) if r != 0 => r,
                    _ => self.resting_awake.bpm,
                };
                self.elevated = resting != 0 && bpm >= resting.saturating_add(ELEVATED_BY);
                // Resting readings: asleep, background ones (a watched one
                // means the wearer is awake after all); awake, any reading
                // once the wearer has taken no steps for a few minutes
                // (see `RESTING_STEPLESS_POLLS`), watched ones included:
                // turning the wrist to look doesn't raise the heart rate.
                let at_rest = if asleep {
                    background
                } else {
                    stepless_polls >= RESTING_STEPLESS_POLLS
                };
                if at_rest {
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
            Outcome::NoReading | Outcome::CalmNoReading | Outcome::OffWrist => {
                self.failures = (self.failures + 1).min(MAX_FAILURES);
                self.off_wrist = outcome == Outcome::OffWrist;
                self.calm_failures = if outcome == Outcome::CalmNoReading {
                    self.calm_failures.saturating_add(1)
                } else {
                    0
                };
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
        (self.resting_awake.bpm, self.resting_asleep.bpm)
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

/// Whether a background session should give up now, `seconds` in, with no
/// reading: no window has passed the signal checks yet (`accepted`), and
/// the wrist was busy at more than half of the `checks` (one a second) so
/// far (`busy`). A busy wrist's next windows would hold most of that
/// movement, so they'd be rejected too, and the session's remaining LED
/// time would likely be spent for nothing. It counts as a session without
/// a reading, so the schedule backs off as after any other.
pub fn give_up(seconds: u32, accepted: bool, busy: u8, checks: u8) -> bool {
    seconds >= GIVE_UP_AFTER && !accepted && u16::from(busy) * 2 > u16::from(checks)
}

/// Whether a session that ran out of time without a reading found the
/// wrist calm (see [`Outcome::CalmNoReading`]): busy (above `heart::CALM`)
/// at no more than a quarter of its level checks. A few busy seconds are
/// a twitch, or turning over; more, and it's a wrist that won't keep still,
/// which the usual back-off is for.
pub fn calm_throughout(busy: u8, checks: u8) -> bool {
    u16::from(busy) * 4 <= u16::from(checks)
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
///
/// It starts from the mean of the first two resting readings that agree
/// (within [`SEED_AGREEMENT`]), not the first alone: starting from one
/// misreading low (a field log's desk afternoon had a 42 against a
/// Garmin's 67), it would take ~20 readings to climb back, all of them
/// counting as elevated meanwhile.
fn track_resting(estimate: &mut Resting, bpm: u8) {
    let Resting {
        bpm: current,
        candidate,
    } = estimate;
    if *current == 0 {
        if *candidate != 0 && candidate.abs_diff(bpm) <= SEED_AGREEMENT {
            *current = (u16::from(*candidate) + u16::from(bpm)).div_ceil(2) as u8;
        } else {
            *candidate = bpm;
        }
    } else if bpm > *current {
        *current += 1;
    } else if bpm < *current {
        *current -= (*current - bpm).div_ceil(4);
    }
}
