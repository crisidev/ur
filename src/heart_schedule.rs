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
//!   wrist backs off only to [`CALM_RETRY_INTERVAL`]: stillness is when a
//!   reading is possible at all, and the signal of a still wrist comes and
//!   goes.
//! - **An LED budget** ([`LED_BUDGET`] seconds an hour, as a token bucket)
//!   caps what all of the above can spend, however the day goes.
//! - **An overdue reading spends more** ([`OVERDUE`]): half an hour
//!   without one, and the budget doubles and failures retry every
//!   [`OVERDUE_RETRY`], until the next reading. A gap in the data is worth
//!   more than the battery it takes to avoid.
//!
//! Sessions while the clock face shows aren't scheduled (someone is
//! looking, so `heart` measures right away), but their readings count as
//! measurements, and their LED time comes out of the budget.
//!
//! Two rules about the sessions themselves live here too, being as
//! hardware-free: a background session only starts once the wrist has
//! been calm for a few seconds ([`START_CALM_POLLS`]), and, if it runs out
//! of time on a calm wrist with a faint pulse, may go on brighter
//! ([`brighter`]). Once started, it runs its full length however the
//! wrist moves (see below), unless it reads first or finds no skin.
//!
//! # Trying through movement
//!
//! Until 0.10.0-rc.3, a background session stopped early for a busy
//! wrist: in its first 12 s if the wrist got restless (movement above 300)
//! before any window had passed the signal checks, to try again at the
//! next still moment (every third stop in a row counting as a failure),
//! and at 12 s if the wrist had been busy (above `heart::CALM`) at most of
//! its checks. The evidence for stopping was sound as far as it went:
//!
//! - Replaying three days of field logs against a Garmin with every
//!   signal check off, windows at 301-700 landed within 5 BPM of it 15% of
//!   the time, against 12% when paired with a Garmin reading from hours
//!   away: no pulse to be had. At 151-300 it was 38% (23% by chance), and
//!   calm, 46% (21%). 12 s stretches of a desk session read 31% of the time
//!   when calm throughout, 16% at worst 151-300, and 1% at worst 301-700.
//! - Replayed on the field logs' sessions (two nights and three desk days),
//!   stopping for a restless wrist would have lost one desk reading of 14,
//!   and two of 136 at night, and saved 12-27% of the desk sessions' LED.
//!   Giving up at 12 s cut 45 failing sessions of 129 short, saving ~860 s
//!   of LED, and lost three background readings (at a desk, from windows
//!   that only passed after 15 s of a busy wrist).
//! - Simulated on three desk days' movement, trying again at the next
//!   still moment made 0.43, 1.8 and 0.93 background readings an hour where
//!   giving up made 0.25, 1.7 and 0.76, for 10-19 s more LED an hour.
//!   Retrying failed sessions sooner instead (from 2 minutes rather than
//!   5) took 26-48 s more for 0.37, 1.6 and 0.99.
//!
//! But a desk morning on the sealed watch (2026-11-10) showed what it
//! cost: every session on a busy wrist ended after 3-19 s, and nine went
//! by in 51 minutes without a reading. The user chose readings over
//! battery ("we should be trying through movement"), and the field log
//! gains what it never had: full-length PPG under movement (its samples
//! and the wrist's movement, a second at a time, at `trace`), the data a
//! motion-tolerant estimator would be developed against. So a session now
//! runs its full `heart::SESSION_LIMIT` through movement. Accuracy doesn't
//! rest on stopping: every window still has to pass the signal checks,
//! sustained movement still vetoes it, a slow one on a busy wrist is still
//! rejected, and a reading still takes three accepted windows that agree,
//! or two in a row on a calm wrist (see `ppg::Consensus`), so a moving
//! wrist mostly just spends the LED. Asleep, a session gave up at 12 s too
//! (4 of 82 that night, on turning over), and runs on as well now: once
//! the wrist settles, the pulse is as before.
//!
//! What running on gains can only partly be replayed, as a recording ends
//! where the firmware that made it stopped. Replaying the awake sessions
//! of the field logs of 2026-11-07 to 11-10 with and without the old
//! stops: the sealed watch's 91 background sessions, recorded under them,
//! read 29 times rather than 27; the 195 older ones (whose logs don't say
//! which were background, so glances are among them), as if all had been,
//! 57 rather than 38, and of the 61 recorded to 28 s or more, 12 rather
//! than 7; the nights' 195 sessions read 183 times either way. Over all
//! the replayed sessions that's ~20% more LED. Each failure now costs a
//! whole session (30 s of LED, 72 budget seconds at 30 mA) where a stop
//! cost 3-19 s, which is why [`LED_BUDGET`] grew.
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
///
/// Not from 2 minutes: simulated on three desk days' movement (with
/// sessions that then gave up at 12 s), retrying from 2 minutes made 0.37,
/// 1.6 and 0.99 background readings an hour against 0.25, 1.7 and 0.76,
/// for 26-48 s more LED an hour; with a failure now a whole 30 s session,
/// that's ~2.5 times the LED for at best a reading every few hours, and it
/// would spend the budget that [`OVERDUE`]'s retries need. A desk that
/// keeps failing reaches those (every 10 minutes) half an hour after its
/// last reading anyway, after waits of 5 and 10 minutes.
pub const FAILED_INTERVAL: u32 = 5 * 60;

/// The longest wait after a session that failed on a calm wrist (see
/// [`Outcome::CalmNoReading`]).
///
/// Lying still awake for an hour (a field log, 2026-11-09, with a Garmin
/// alongside reading 51-73), the back-off above allowed three sessions, at
/// 10 and then 20 minutes apart, and none read: the pulse was a few counts,
/// beside slower swings that came and went. A desk's busy wrist is what
/// the back-off is for, and calm failures are rarer (3 of 17 sessions in
/// that log, 5 of 104 in a night's, none of a desk day's 39), so this
/// costs little: an hour of calm failures takes 7 sessions instead of 4,
/// ~95 s more of LED.
///
/// This used to stop after six in a row (an hour), in case the watch lay on
/// something that reflects the LED like skin (and keeps as still as a
/// wrist), which fails calm too. An hour of failures makes a reading
/// overdue, though, which retries as often (see [`OVERDUE_RETRY`]), and the
/// doubled budget is what bounds such a surface now.
pub const CALM_RETRY_INTERVAL: u32 = 10 * 60;

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
/// alike: room for a busy hour of elevated readings, or of tries at a desk,
/// but a bad day (a fidgety wrist failing session after session) can't
/// drain more than ~0.23 mA on average.
///
/// Seconds at the lowest drive, 12.5 mA, which is all sessions use but
/// for the occasional [`brighter`] phase and the bright arm (see [`Arm`]):
/// a second at 30 mA costs 2.4 (see [`budget_seconds`]).
///
/// It was 180 s (what the fixed schedule this replaced spent at worst, six
/// failing 30 s sessions an hour) while a session at a desk stopped within
/// 3-19 s on a busy wrist. Now that it runs its 30 s (see "Trying through
/// movement" in the module docs), a desk hour of glances (~13 s each) and
/// rounds of tries that read at the third goes past 180: simulated over 8
/// hours with a glance every 7.5 minutes, 180 held back 29 of 54 sessions
/// until the budget had earned them (31 of 51 all on the bright arm), and
/// 300 none of 78 (17 of 78). 300 also lets every try of an overdue desk
/// on the bright arm start at 30 mA (432 budget seconds an hour, inside
/// [`OVERDUE_BUDGET`]), where at 180 a third to a half of them stepped
/// down. The user would rather spend battery than lose readings: the cap
/// goes from ~3.3 to ~5.6 mAh a day, and what an hour at a desk actually
/// spends is in docs/power-plan.md.
pub const LED_BUDGET: u32 = 300;

/// Budget a background session needs before it may start: its longest
/// run, `SESSION_LIMIT`, so one session can't overdraw the bucket by
/// much. (Watched sessions ignore this; someone is looking.)
pub const SESSION_RESERVE: u32 = 30;

/// Seconds for the bucket to earn one second of LED back.
const REFILL_EVERY: u32 = 3600 / LED_BUDGET;
const _: () = assert!(3600 % LED_BUDGET == 0, "the refill rate must be whole");
const _: () = assert!(
    REFILL_EVERY.is_multiple_of(2),
    "the overdue refill rate must be whole"
);
const _: () = assert!(SESSION_RESERVE <= LED_BUDGET);

/// How long without a reading (any, watched or not; or since boot) before
/// one is overdue, asleep or awake. Overdue, the [`LED_BUDGET`] doubles
/// ([`OVERDUE_BUDGET`]) and a failure is retried within [`OVERDUE_RETRY`],
/// so that more sessions, and more of them brighter (see [`brighter`] and
/// [`bright_start`]), have a go, until a reading comes and it all returns
/// to normal.
///
/// Half an hour. Awake, readings come every 2-20 minutes when they come at
/// all, and failures back off 5, 10, 20 minutes, so half an hour is two or
/// three tries in a row that failed. It was an hour until 0.10.0-rc.3, and
/// a desk morning on the sealed watch (2026-11-10) went 51 minutes and
/// nine failed sessions without a reading, short of it, so the faster
/// retries never came; the user chose to retry more often. At a desk
/// that won't read, that's a try every 10 minutes from half an hour in,
/// rather than at 40 and then every 10 from an hour.
///
/// The same by night, for one rule: asleep, sessions come every 5
/// minutes, so half an hour is six missed in a row, which a night that
/// reads (95% of sessions on 2026-11-10) rarely sees, and the night costs
/// nothing more until it does. A night without readings is more often a
/// watch on the nightstand, which this leaves alone (see
/// [`Outcome::OffWrist`]), than a faint pulse.
///
/// At its worst (a whole day overdue, every session failing), it doubles
/// the LED's hourly cap: from ~0.23 mA to ~0.46 mA on average, ~5.6 mAh a
/// day more (see docs/power-plan.md). Overdue half an hour sooner moves
/// each failing spell's doubling earlier, not that cap: at most 150 budget
/// seconds more a spell, ~0.12 mAh.
pub const OVERDUE: u32 = 30 * 60;

/// The LED budget, and its refill rate, while a reading is overdue (see
/// [`OVERDUE`]).
pub const OVERDUE_BUDGET: u32 = 2 * LED_BUDGET;

/// The longest wait after a failure while a reading is overdue (see
/// [`OVERDUE`]), unless the watch was off the wrist: as for a calm failure
/// (see [`CALM_RETRY_INTERVAL`]), and with no limit on how many, as the
/// doubled budget bounds them. At a desk, failures (a busy wrist) otherwise
/// back off to 20 minutes, or an hour with no movement in between; with
/// this, a desk day that hasn't read for half an hour tries six times an
/// hour at its still moments.
///
/// Not sooner: each try is a whole session (30 s of LED, 72 budget seconds
/// on the bright arm), so six an hour take 432 budget seconds of the
/// doubled budget's 600 on the bright arm, leaving the rest for glances
/// (and, should the budget run short, [`bright_start`] steps down rather
/// than hold a try back).
pub const OVERDUE_RETRY: u32 = CALM_RETRY_INTERVAL;

/// Consecutive failures counted (beyond this the wait is capped anyway).
const MAX_FAILURES: u8 = 5;

// Intervals are stored as `u16` to keep the heart task's state small.
const _: () = assert!(MAX_FAILED_INTERVAL <= u16::MAX as u32);
const _: () = assert!(OVERDUE_BUDGET <= u16::MAX as u32);

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
    /// The last session(s) had no reading, and neither has any for half an
    /// hour (see [`OVERDUE`]): sooner than [`Why::Retry`] would be.
    Overdue,
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

/// The scheduling state, 32 bytes: the heart task holds it across every
/// await, so it lives in that task's static future, where RAM is tight.
#[derive(Clone, Debug)]
pub struct Schedule {
    /// When the last counted session ended.
    last: u32,
    /// When the last reading came (0: none since boot), for [`OVERDUE`].
    last_reading: u32,
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
    /// The LED experiment's rounds (see [`Arm`]): the latest round's
    /// number (0: none yet), whether it's still going (no reading since
    /// it started), whether it has chosen its awake arm yet, and whether
    /// the arm last chosen was bright.
    round: u16,
    in_round: bool,
    armed: bool,
    bright: bool,
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
            last_reading: 0,
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
            round: 0,
            in_round: false,
            armed: false,
            // So that the first round to choose is dim, as before.
            bright: true,
        }
    }

    /// When the next background session is due (seconds since boot), and
    /// why. `active_polls` is how many accelerometer polls since the last
    /// session had steps or vigorous movement; `asleep`, whether the sleep
    /// classifier thinks the wearer is asleep.
    ///
    /// Due isn't "start now": `heart` also waits for a still wrist.
    pub fn due(&self, active_polls: u8, asleep: bool) -> (u32, Why) {
        let (at, why) = self.due_unbudgeted(active_polls, asleep);

        // The budget: when will the bucket hold a whole session's worth?
        // That holds the first session after boot back too: the clock
        // face looked at before it, without a reading, spends the budget
        // all the same.
        let affordable = self.holds(SESSION_RESERVE);
        if affordable > at {
            return (affordable, Why::Budget);
        }
        (at, why)
    }

    /// [`Self::due`], as if the budget were unlimited.
    fn due_unbudgeted(&self, active_polls: u8, asleep: bool) -> (u32, Why) {
        if self.interval == 0 {
            return (0, Why::First);
        }
        let moved = moved(active_polls, asleep);
        let (wait, why) = if self.failures > 0 {
            let retry = (FAILED_INTERVAL << (self.failures - 1)).min(MAX_FAILED_INTERVAL);
            let retry = if self.calm_failures > 0 {
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
        let mut at = self.last.saturating_add(wait);
        let mut why = why;
        // A reading overdue: a failure is retried sooner, but not off the
        // wrist (a nightstand), where there's no reading to be had.
        if self.failures > 0 && !self.off_wrist {
            let soonest = (self.last + OVERDUE_RETRY).max(self.overdue_from());
            if soonest < at {
                (at, why) = (soonest, Why::Overdue);
            }
        }
        (at, why)
    }

    /// Records a session that ended at `now`, having had the LED on for
    /// `led_seconds`, at 12.5 mA or its equivalent (see
    /// [`budget_seconds`]). `background` says whether the schedule started it
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
                // No longer overdue: back to the usual budget (see
                // `OVERDUE`), keeping no more than it holds.
                self.last_reading = now;
                // The round of tries is over, watched or not (see `Arm`).
                self.in_round = false;
                self.tokens = self.tokens.min(LED_BUDGET as u16);
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
        self.refilled(now).0
    }

    /// When a reading becomes overdue (see [`OVERDUE`]), unless one comes.
    fn overdue_from(&self) -> u32 {
        self.last_reading.saturating_add(OVERDUE)
    }

    /// The budget at `now`, and when its last whole second was earned (to
    /// keep the part-earned one, so frequent refills don't lose time to
    /// rounding): at [`LED_BUDGET`]'s rate, into a bucket that size, until
    /// a reading is overdue, and twice that after.
    fn refilled(&self, now: u32) -> (u32, u32) {
        let from = self.overdue_from();
        let earn = |(tokens, at): (u32, u32), until: u32, every: u32, cap: u32| {
            if until <= at {
                return (tokens, at);
            }
            let earned = (until - at) / every;
            if tokens + earned >= cap {
                (cap.max(tokens), until)
            } else {
                (tokens + earned, at + earned * every)
            }
        };
        let usual = earn(
            (u32::from(self.tokens), self.refilled_at),
            now.min(from),
            REFILL_EVERY,
            LED_BUDGET,
        );
        if now <= from {
            return usual;
        }
        // Overdue: what was part-earned at the usual rate is lost, at most
        // one second's worth, once.
        let (tokens, at) = usual;
        earn(
            (tokens, at.max(from)),
            now,
            REFILL_EVERY / 2,
            OVERDUE_BUDGET,
        )
    }

    /// When the bucket will hold `want` seconds, if no session spends any
    /// before then.
    fn holds(&self, want: u32) -> u32 {
        let tokens = u32::from(self.tokens);
        if tokens >= want {
            return 0;
        }
        let from = self.overdue_from();
        let usual = self.refilled_at + (want - tokens) * REFILL_EVERY;
        if usual <= from {
            return usual;
        }
        // Earned at the usual rate until overdue, twice as fast after.
        let by_then = from.saturating_sub(self.refilled_at) / REFILL_EVERY;
        self.refilled_at.max(from) + (want - tokens - by_then) * (REFILL_EVERY / 2)
    }

    /// Brings `tokens` up to date.
    fn refill(&mut self, now: u32) {
        let (tokens, at) = self.refilled(now);
        self.tokens = tokens as u16;
        self.refilled_at = at;
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

/// Whether a session that ran out of time without a reading found the
/// wrist calm (see [`Outcome::CalmNoReading`]): busy (above `heart::CALM`)
/// at no more than a quarter of its level checks. A few busy seconds are
/// a twitch, or turning over; more, and it's a wrist that won't keep still,
/// which the usual back-off is for.
///
/// It also keeps a busy wrist's session from going on [`brighter`]: now
/// that a session runs its full length through movement, a desk session
/// often runs out on a wrist that was busy for much of it, and more light
/// doesn't steady the sensor on the skin. (Its last window must be calm
/// too, so a wrist that settles only at the end doesn't qualify either.)
pub fn calm_throughout(busy: u8, checks: u8) -> bool {
    u16::from(busy) * 4 <= u16::from(checks)
}

// ==============================================================================
// A brighter LED for a faint pulse (an experiment)
// ==============================================================================

/// The LED's current at each drive setting (`heart`'s PDRIVE 0..=3), in
/// tenths of a milliamp, from the HRS3300 datasheet: 12.5, 20, 30, 40 mA.
/// The light that comes back, and so the counts, scale with it (as far as
/// the datasheet goes; tomorrow's log will tell).
pub const DRIVE_CURRENT: [u32; 4] = [125, 200, 300, 400];

/// Brighter phases a background session may add (see [`brighter`]).
pub const BRIGHTER_STEPS: u8 = 2;

/// How long a brighter phase runs, at most: half a second warming up and
/// settling, the first 8 s window, and three more after it, 2 s apart, so
/// a calm pair or three accepted windows can form.
pub const BRIGHTER_FOR: u32 = 15;

/// The highest a brighter phase's peak may be expected to reach, in counts:
/// half the 14-bit ADC's range. The prediction scales the last second's
/// peak with the current, and the LED's light may not be quite linear in
/// it, while the level check steps back down at 90% of full scale; half
/// leaves the pulse and a shift of the sensor room before that.
pub const BRIGHTER_PEAK_LIMIT: u32 = 1 << 13;

/// What `millis` of LED at `drive` cost the [`LED_BUDGET`], which counts
/// seconds at the lowest drive (12.5 mA), rounded up.
pub fn budget_seconds(drive: u8, millis: u32) -> u32 {
    let weighted = u64::from(millis) * u64::from(DRIVE_CURRENT[usize::from(drive & 3)]);
    weighted.div_ceil(1000 * u64::from(DRIVE_CURRENT[0])) as u32
}

/// The brighter drive a background session should go on at, if any, now
/// that a phase (the session's usual 30 s at its starting drive, or a
/// brighter phase's [`BRIGHTER_FOR`]) has run out without a reading on a
/// calm wrist, its last window faint or aperiodic: it has `peak` counts
/// at `drive`, has taken `steps` so far, and `spare` seconds of the
/// [`LED_BUDGET`] would be left after this session's so far.
///
/// An experiment: does more light lift a faint pulse out of the noise?
/// A still wrist at a desk, or lying awake, had a pulse of 2.9-5 counts
/// on a light level of 1,600-4,000 at 12.5 mA (2026-11-09), and the level
/// control never moves off its lowest drive there, as that only steps up
/// for a level under ~1,000. If the noise is the sensor's own (shot noise,
/// or the ADC's), 30 mA would raise the pulse 2.4× against noise that
/// grows only ~1.5×; if it's the light itself flickering (the interference
/// one analysis found to scale with the level), it gains nothing. Field
/// logs say which.
///
/// Only once the session has had its usual chance at the dim drive, so a
/// session that reads at the dim drive reads exactly as before (and so the
/// replay of recorded sessions, which can't show a brighter LED, still
/// holds): stepping at the first faint window instead would have cut short
/// 9 of the 91 background sessions that read at night (2026-11-08), which
/// read from later windows; at 14 s, 3.
///
/// Up two drive settings at a time (30 mA from 12.5), or one if that would
/// take the peak past [`BRIGHTER_PEAK_LIMIT`], and only while `spare`
/// covers the phase and still leaves [`SESSION_RESERVE`] for the next
/// session: a brighter phase is the budget's to spend only when it's not
/// needed for ordinary sessions.
pub fn brighter(drive: u8, steps: u8, peak: u32, spare: u32) -> Option<u8> {
    if steps >= BRIGHTER_STEPS {
        return None;
    }
    step_up(drive, peak, |next| {
        spare >= budget_seconds(next, BRIGHTER_FOR * 1000) + SESSION_RESERVE
    })
}

/// The drive an awake background session in the experiment's bright arm
/// should start at, rather than its level's `drive` (see [`Arm`]):
/// two settings up (30 mA from 12.5), or one if the `peak` expected at
/// `drive` (the last session's last second, scaled to this level) would
/// pass [`BRIGHTER_PEAK_LIMIT`] at 30 mA, or none; and only if the
/// `budget` covers a whole session (`SESSION_RESERVE` seconds) at it.
///
/// Why start brighter, rather than step up late as [`brighter`] does: at a
/// desk, a still moment lasts seconds, and a session on a wrist that's
/// busy for much of its 30 s never takes the late step (see
/// [`calm_throughout`]). Whether more light helps against the tendons and
/// fingers moving the sensor is what the arms compare.
///
/// TODO: once the arms have answered that, start bright only when a
/// reading looks hard on a calm wrist (the last session failed, or read a
/// faint pulse, i.e. a small peak-to-mean against its DC), rather than
/// every other awake round: where a dim session reads easily, the light
/// only costs charge and risks saturating. Every other round for now, for
/// a quicker comparison (user, 2026-11-09).
pub fn bright_start(drive: u8, peak: u32, budget: u32) -> Option<u8> {
    step_up(drive, peak, |next| {
        budget >= budget_seconds(next, SESSION_RESERVE * 1000)
    })
}

/// Which LED a background session starts with: the experiment comparing a
/// brighter LED with the usual one where readings are hardest, at a desk.
///
/// Awake, *rounds* take turns: one starts at its level's drive (`Dim`,
/// 12.5 mA on a wrist), the next two drive settings up (`Bright`, 30 mA;
/// see [`bright_start`] for when it's 20 mA or the usual drive after all),
/// keeping the gain. Taking turns rather than all bright gives a field log
/// both, at the same desk on the same day, under the same rules, so it can
/// say whether more light reads more (and for what LED) instead of
/// comparing days whose wrists differ. If it does, half the desk rounds
/// have it meanwhile; if it doesn't, half pay for it.
///
/// A round is a reading's worth of tries: the background session the
/// schedule had due for its own reasons, and every one after it (retried,
/// overdue) until a reading, background or watched, ends it (see
/// [`Schedule::begin`]). Its first awake session chooses the
/// arm, and the rest keep it. Taking turns by session instead (0.10.0-rc.3)
/// handed each try's retry to the other arm, so whenever a round's first
/// try failed and its retry read, the next round's first try was on the
/// same arm as this one's: in a field log (2026-11-10), one arm got nearly
/// all the first tries, which come at the scheduled time and were often cut
/// short by movement, and the other the retries, which wait for a still
/// moment. By round, both arms get first tries and retries alike, and a
/// round's outcome (read or not, after how many tries and how much LED) is
/// what to compare.
///
/// Strict turns rather than a pseudo-random choice per round: nothing the
/// wrist does decides a round's parity (a round ends at a reading, not at
/// a time), so turns carry no bias, and they keep a short log's arms
/// even.
///
/// Asleep (`Asleep`), sessions stay at their level's drive, as night
/// readings come 88% of the time without it, and an asleep session doesn't
/// choose its round's arm (an awake one later in the round does). Watched
/// sessions stay there too, as they're not in the comparison. A session
/// that fails calm on either arm may still go on brighter at its end
/// ([`brighter`]).
///
/// TODO: once field logs have judged it, make the winner the rule.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[cfg_attr(target_os = "none", derive(defmt::Format))]
pub enum Arm {
    Asleep,
    Dim,
    Bright,
}

impl Schedule {
    /// Starts a background session (`asleep`, if the sleep classifier
    /// thinks so): its round, numbered from 1 since boot (wrapping), and
    /// its [`Arm`]. A round starts with the first session after a reading
    /// (or boot), and an awake round alternates its arm with the last
    /// awake one's, dim first.
    pub fn begin(&mut self, asleep: bool) -> (u16, Arm) {
        if !self.in_round {
            self.round = self.round.wrapping_add(1);
            self.in_round = true;
            self.armed = false;
        }
        if asleep {
            return (self.round, Arm::Asleep);
        }
        if !self.armed {
            self.bright = !self.bright;
            self.armed = true;
        }
        let arm = if self.bright { Arm::Bright } else { Arm::Dim };
        (self.round, arm)
    }
}

/// The brightest drive up to two settings above `drive` at which `peak`,
/// scaled by the current, stays within [`BRIGHTER_PEAK_LIMIT`], and that
/// `affordable` allows.
fn step_up(drive: u8, peak: u32, affordable: impl Fn(u8) -> bool) -> Option<u8> {
    let now = DRIVE_CURRENT[usize::from(drive & 3)];
    (drive + 1..=(drive + 2).min(3)).rev().find(|&next| {
        let current = DRIVE_CURRENT[usize::from(next)];
        let predicted = u64::from(peak) * u64::from(current) / u64::from(now);
        predicted <= u64::from(BRIGHTER_PEAK_LIMIT) && affordable(next)
    })
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
