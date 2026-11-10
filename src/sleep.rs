//! Sleep, from wrist movement: which minutes were spent asleep, restless,
//! awake, or with the watch off the wrist.
//!
//! This is actigraphy, the method of research sleep watches: during sleep
//! the wrist is still, apart from brief stirs. The rules, applied to each
//! minute's movement (from [`crate::activity`]) and to whether the heart
//! rate sensor last saw skin:
//!
//! - **Falling asleep** is [`ONSET`] minutes of stillness, give or take
//!   stirs of up to [`STIR`] minutes; the sleep is dated from the start of
//!   the stillness, not the moment it qualified.
//!   Within a night (after a wake of up to [`SAME_NIGHT_GAP`] minutes),
//!   [`RESUME`] minutes are enough.
//! - **Waking** is [`WAKE_RUN`] minutes of movement, or walking (at least
//!   [`WAKE_STEPS`] steps in a minute). It's dated from the first of them.
//! - Shorter movement within sleep is a stir: up to [`STIR`] minutes stay
//!   sleep, longer is **restless** (Gadgetbridge's "awake during sleep").
//! - **Not worn** is when the heart rate sensor's last look found no skin.
//!
//! Sleep comes out of this as [`Kind::Light`]; [`crate::stages`] then
//! relabels it light, deep, or REM from the heart rate and the stirs. (This
//! used to call the inside of long, nearly motionless stretches deep, the
//! usual stand-in from movement alone; against a Garmin it was right about
//! a fifth of deep sleep, and saw no REM.)
//!
//! Since sleep is recognised only after the fact, classification runs over
//! a trailing window ([`WINDOW`] minutes, plus [`CONTEXT`] before it) every
//! minute, revising its recent past. Minutes older than the window are
//! final, as far as sleep and waking go (their stage can still change for a
//! while: see [`crate::sleep_hold`]).
//!
//! The thresholds are first guesses in our movement units (Bangle.js'
//! 8192-per-g; ~25 lying on a desk, hundreds when handled), to be tuned from
//! field logs of real nights.
//!
//! [`Night`] builds on the classifier's verdict to decide when the quiet
//! night (no buzzing or lit screen for notifications) starts and ends.
//!
//! This module knows nothing about the hardware or the clock, so it can be
//! tested on a computer (see `host-tests/sleep`).

/// What a minute was spent doing.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
#[cfg_attr(target_os = "none", derive(defmt::Format))]
#[repr(u8)]
pub enum Kind {
    /// No data (the watch was off, or the minute isn't classified yet).
    #[default]
    Unknown = 0,
    Awake = 1,
    Light = 2,
    Deep = 3,
    /// Awake for a while within sleep.
    Restless = 4,
    NotWorn = 5,
    /// Rapid-eye-movement (dreaming) sleep: see [`crate::stages`].
    Rem = 6,
}

impl Kind {
    pub fn from_bits(bits: u8) -> Self {
        match bits {
            1 => Self::Awake,
            2 => Self::Light,
            3 => Self::Deep,
            4 => Self::Restless,
            5 => Self::NotWorn,
            6 => Self::Rem,
            _ => Self::Unknown,
        }
    }

    pub fn is_sleep(self) -> bool {
        matches!(self, Self::Light | Self::Deep | Self::Rem | Self::Restless)
    }

    /// Whether this is a stage of actual sleep (not restless).
    pub fn is_stage(self) -> bool {
        matches!(self, Self::Light | Self::Deep | Self::Rem)
    }

    /// The name Gadgetbridge's `ActivityKind` uses, for activity samples.
    /// `None` for plain activity, which is Gadgetbridge's default.
    pub fn gadgetbridge_name(self) -> Option<&'static str> {
        match self {
            Self::Unknown | Self::Awake => None,
            Self::Light => Some("LIGHT_SLEEP"),
            Self::Deep => Some("DEEP_SLEEP"),
            // Gadgetbridge's Bangle.js support doesn't declare REM, but
            // takes any `ActivityKind` name: its charts draw REM (only the
            // legend leaves it out), and its Health Connect export writes
            // it as REM. See `crate::stages`.
            Self::Rem => Some("REM_SLEEP"),
            Self::Restless => Some("AWAKE_SLEEP"),
            Self::NotWorn => Some("NOT_WORN"),
        }
    }
}

/// One minute's evidence.
#[derive(Clone, Copy, Debug, Default)]
pub struct Minute {
    /// Whether the watch recorded this minute at all.
    pub seen: bool,
    pub movement: u16,
    pub steps: u16,
    /// Whether the heart rate sensor last saw skin, if it has looked.
    pub worn: Option<bool>,
}

// ==============================================================================
// Thresholds
// ==============================================================================

/// Movement (averaged over three minutes) below which a minute is still.
/// TODO: tune from field logs.
pub const STILL_LIMIT: u16 = 120;
/// Minutes of stillness that make sleep. Long enough that sitting still
/// through a TV episode with the odd fidget doesn't count.
pub const ONSET: usize = 20;
/// Minutes of stillness that make sleep again soon after waking (within
/// [`SAME_NIGHT_GAP`]): falling back asleep in the night is quicker, and a
/// false wake (a cluster of stirs) shouldn't cost 20 minutes of sleep.
pub const RESUME: usize = 10;
/// Minutes of movement that end sleep. Two stirs in a row already make
/// five (see [`STIR`]), so this is one more.
pub const WAKE_RUN: usize = 6;
/// Steps in a minute that end sleep at once: that's getting up.
pub const WAKE_STEPS: u16 = 10;
/// Moving minutes within sleep that are just a stir (light sleep). Three,
/// because the three-minute smoothing turns one twitchy minute into three
/// moving ones.
pub const STIR: usize = 3;
/// How many recent minutes are re-classified (and rewritten) every minute.
/// Must cover the longest look-back of any rule (onset: [`ONSET`]), with
/// room to spare.
pub const WINDOW: usize = 90;
/// How many minutes before the window are classified along with it, as
/// context, but not rewritten. Without it, the oldest minutes of every pass
/// (the ones about to become final) would always look like the start of a
/// stretch, with no stillness or sleep before them to go on.
pub const CONTEXT: usize = 30;

// ==============================================================================
// Classification
// ==============================================================================

/// Classifies `minutes` (oldest first) into `out`, given the class of the
/// minute just before them (final, from an earlier pass).
pub fn classify(before: Kind, minutes: &[Minute], out: &mut [Kind]) {
    assert_eq!(minutes.len(), out.len(), "one class per minute");
    let n = minutes.len();

    let still = |t: usize| is_still(minutes, t);
    // The length of the run of minutes from `t` for which `f` holds.
    let run = |t: usize, f: &dyn Fn(usize) -> bool| (t..n).take_while(|&u| f(u)).count();
    // The length of the stillness from `t`, bridging stirs of up to `STIR`
    // minutes, up to its last still minute.
    let stillness = |t: usize| {
        let mut end = t;
        let mut u = t;
        while u < n {
            if still(u) {
                u += 1;
                end = u;
            } else {
                let gap = run(u, &|v| minutes[v].seen && !still(v));
                // An unseen minute (gap 0) ends the stillness too.
                if gap == 0 || gap > STIR || u + gap == n {
                    break;
                }
                u += gap;
            }
        }
        end - t
    };

    let mut asleep = before.is_sleep();
    // When we were last asleep (as a minute index, -1 being just before
    // `minutes`), for the quicker return to sleep within a night.
    let mut last_asleep: Option<isize> = asleep.then_some(-1);
    let mut t = 0;
    while t < n {
        let minute = minutes[t];
        if !minute.seen {
            out[t] = Kind::Unknown;
            t += 1;
            continue;
        }
        if minute.worn == Some(false) {
            out[t] = Kind::NotWorn;
            asleep = false;
            t += 1;
            continue;
        }
        if !asleep {
            // Sleep starts here if the stillness from here lasts long
            // enough. (At the end of the window that's not known yet; this
            // minute stays awake for now, and a later pass revises it.)
            let length = if still(t) { stillness(t) } else { 0 };
            let same_night =
                last_asleep.is_some_and(|last| t as isize - last <= SAME_NIGHT_GAP as isize);
            if length >= if same_night { RESUME } else { ONSET } {
                asleep = true;
                // Stirs inside are sorted out below, as within any sleep.
                out[t] = Kind::Light;
                t += 1;
            } else {
                out[t] = Kind::Awake;
                t += 1;
            }
            continue;
        }
        // Asleep.
        last_asleep = Some(t as isize);
        if minute.steps >= WAKE_STEPS {
            asleep = false;
            out[t] = Kind::Awake;
            t += 1;
            continue;
        }
        if still(t) {
            out[t] = Kind::Light;
            t += 1;
            continue;
        }
        let moving = run(t, &|u| {
            minutes[u].seen && minutes[u].worn != Some(false) && !still(u)
        });
        let ends_window = t + moving == n;
        let kind = if moving >= WAKE_RUN {
            asleep = false;
            Kind::Awake
        } else if moving > STIR || ends_window {
            // At the window's end, unfinished movement is restless until
            // it's clear whether it becomes waking.
            Kind::Restless
        } else {
            Kind::Light
        };
        out[t..t + moving].fill(kind);
        t += moving;
    }
}

/// Whether minute `t` of `minutes` is still: seen, stepless, and with its
/// movement averaged with each neighbour's below [`STILL_LIMIT`], so one
/// twitchy minute in the middle of stillness (or one still minute while
/// active) doesn't flip the verdict.
fn is_still(minutes: &[Minute], t: usize) -> bool {
    let range = t.saturating_sub(1)..(t + 2).min(minutes.len());
    let seen = minutes[range].iter().filter(|m| m.seen);
    let (sum, count) = seen.fold((0u32, 0u32), |(s, c), m| (s + u32::from(m.movement), c + 1));
    let smoothed = (sum / count.max(1)) as u16;
    minutes[t].seen && minutes[t].steps == 0 && smoothed < STILL_LIMIT
}

/// How many of the newest minutes [`unsettled_onset`] needs: a stillness
/// [`ONSET`] minutes long is sleep already, so one that may yet become
/// sleep is shorter, and followed by at most a stir of up to [`STIR`]
/// minutes and the minute under way; with a minute to spare, and the
/// oldest only as its newer neighbour's (for the smoothing).
pub const ONSET_SPAN: usize = ONSET + STIR + 2 + 1;

/// Where, among the newest of `minutes` (oldest first; the newest still
/// under way, as at a classification: [`ONSET_SPAN`] of them are enough),
/// a sleep may yet be found to have started that [`classify`] doesn't see
/// yet: the first minute of the stillness they end with, which becomes
/// sleep, dated from its start, once it has lasted [`ONSET`] minutes (or
/// [`RESUME`]). Bridges stirs of up to [`STIR`] minutes, and a trailing
/// one that may yet turn out to be a stir. A minute not worn can't fall
/// asleep, but stays in the stillness of the ones before it, as in
/// [`classify`]. `None` if the newest minutes aren't still, or not worn.
///
/// The newest minute is under way, so its own and its predecessor's
/// smoothed movement aren't final: they count as still if they may yet
/// turn out so, should the wrist not move for the rest of the minute.
/// (Its steps so far can only grow.)
///
/// Sleep that has started is a sleep's to settle (see `crate::sleep_hold`).
pub fn unsettled_onset(minutes: &[Minute]) -> Option<usize> {
    let n = minutes.len();
    // The newest three, the newest without movement from here on.
    let tail = n.saturating_sub(3);
    let mut ending = [Minute::default(); 3];
    let ending = &mut ending[..n - tail];
    ending.copy_from_slice(&minutes[tail..]);
    if let Some(newest) = ending.last_mut() {
        newest.movement = 0;
    }
    let still = |t: usize| {
        if t + 2 >= n {
            is_still(ending, t - tail)
        } else {
            is_still(minutes, t)
        }
    };

    let mut from = None;
    // Moving minutes in a row, back from the last still one.
    let mut moving = 0;
    // From the second-oldest: the oldest is only its neighbour.
    for t in (1..n).rev() {
        if !minutes[t].seen {
            break;
        }
        if still(t) {
            moving = 0;
            if minutes[t].worn != Some(false) {
                from = Some(t);
            }
        } else {
            moving += 1;
            if moving > STIR {
                break;
            }
        }
    }
    from
}

// ==============================================================================
// Summaries
// ==============================================================================

/// Awake minutes that still count as the same night (getting up for the
/// bathroom, lying awake for a while).
pub const SAME_NIGHT_GAP: usize = 45;

/// The most recent sleep in `minutes` minutes (oldest first), whose kinds
/// `kind` gives by index: (index of its first minute, index after its
/// last, minutes actually asleep). Wakes shorter than [`SAME_NIGHT_GAP`]
/// are part of it; sleep still going on counts.
///
/// The kinds come from a function rather than a slice so that the watch
/// can read them straight out of its history, instead of copying a day's
/// worth (1.4 KB) onto the stack first.
pub fn last_sleep(minutes: usize, kind: impl Fn(usize) -> Kind) -> Option<(usize, usize, usize)> {
    let end = (0..minutes).rev().find(|&i| kind(i).is_sleep())? + 1;
    let mut start = end - 1;
    let mut gap = 0;
    for i in (0..end).rev() {
        if kind(i).is_sleep() {
            start = i;
            gap = 0;
        } else {
            gap += 1;
            if gap > SAME_NIGHT_GAP {
                break;
            }
        }
    }
    let asleep = (start..end).filter(|&i| kind(i).is_stage()).count();
    Some((start, end, asleep))
}

// ==============================================================================
// The quiet night
// ==============================================================================

/// The quiet night: from falling asleep until the morning, during which the
/// UI keeps notifications from buzzing or lighting the screen (see
/// `crate::ui`). This decides only when it starts and ends.
///
/// It starts when the wearer falls asleep: when [`Night::update`] first
/// sees `asleep` turn true. It ends at the first of:
///
/// - **An alarm** ringing ([`Night::end`]): the morning has come. (Not
///   a timer: one ringing at night is no sign of morning.)
/// - **The wearer saying so**, with a double press of the button
///   ([`Night::end`] too): up before the alarm, they needn't wait out
///   [`QUIET_AWAKE`] for the watch to act as by day.
/// - **Being awake for longer than [`QUIET_AWAKE`]**: the morning came
///   without an alarm. Being awake per the classifier isn't enough by
///   itself: getting up and walking (to the bathroom, say) ends sleep at
///   once there, and the wearer is likely back in bed within minutes.
/// - **Local noon** ([`noon_between`]), a backstop for a classifier stuck
///   on "asleep" (e.g. a watch lying still off the wrist, before the heart
///   rate sensor has said so), so that quiet can't last all day. Noon,
///   because it's past any morning, and well before any night.
///
/// Starting needs `asleep` to *turn* true, not just be true: after an alarm
/// (or noon, or the button) ends the night with the wearer still asleep
/// (as the classifier sees it: lying still, reading, after the button),
/// it stays over until they've been awake and then fallen asleep again.
/// Otherwise the alarm would end the night only for it to start again the
/// next minute.
///
/// Using the watch in the night (reading it, holding a grid button) is
/// awake only through the movement it takes, as the classifier sees it:
/// nothing here hears of touches. A minute or two of it is a stir at
/// most. [`WAKE_RUN`] minutes of fidgeting with it are waking, which
/// starts the [`QUIET_AWAKE`] count; settling then, [`RESUME`] still
/// minutes are asleep again, well within it, and the night goes on. Only
/// moving the wrist for about `WAKE_RUN + QUIET_AWAKE - RESUME` (18)
/// minutes without settling ends it, as being up would. A wrist held
/// still while reading counts as still, and so as asleep.
///
/// A **nap** is quiet too, but ends sooner: sleep that starts in the
/// daytime ([`nap_hours`]) ends after [`NAP_AWAKE`] minutes awake, not
/// [`QUIET_AWAKE`]. Waking from a nap is waking up, not a trip to the
/// bathroom; and noon, which ends any night, would cut a nap short anyway.
///
/// Nothing here survives a reset. That fails safe: after one, the night is
/// over, and starts again once the classifier (whose history is lost too)
/// sees [`ONSET`] minutes of stillness. So a reset at night costs ~20
/// minutes of the screen lighting up for notifications, and a reset can't
/// leave the watch quiet through the day.
///
/// Times are in monotonic seconds (since boot), as `u32` to keep this to 8
/// bytes in RAM; that lasts 136 years of uptime.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Night {
    quiet: bool,
    /// Whether the wearer was asleep at the last update, to see them fall
    /// asleep.
    asleep: bool,
    /// When the wearer was last asleep. Meaningful only while quiet: it's
    /// set on falling asleep, which starts the night.
    last_asleep: u32,
    /// Whether this is a nap (see [`nap_hours`]), which ends sooner.
    nap: bool,
}

/// Minutes awake that end a quiet night without an alarm. Long enough for a
/// trip to the bathroom and back to sleep, short enough that a morning
/// without an alarm isn't silent for long. (The user's pick; shorter than
/// the classifier's [`SAME_NIGHT_GAP`], which only decides what counts as
/// one night in the summary.)
pub const QUIET_AWAKE: usize = 22;

/// Minutes awake that end a nap's quiet: once up from a nap, the wearer is
/// most likely up for the day, so a few minutes of being awake (enough to
/// tell a stir that the classifier calls waking from actually getting up)
/// are enough.
pub const NAP_AWAKE: usize = 7;

/// Whether falling asleep at `local_second` (local seconds since 1970)
/// makes a nap rather than a night: between 09:00 and 20:00. Clock hours,
/// not daylight: an Oslo summer has the sun up until 23:00, and its winter
/// sets at 15:30. Unknown time is a night (the longer quiet).
pub fn nap_hours(local_second: Option<i64>) -> bool {
    const HOUR: i64 = 60 * 60;
    local_second.is_some_and(|s| (9 * HOUR..20 * HOUR).contains(&s.rem_euclid(24 * HOUR)))
}

/// How a quiet night changed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Change {
    /// The wearer fell asleep: the night (or a nap) starts.
    Quiet { nap: bool },
    /// The night ended this way.
    Over(Over),
}

/// Why a night ended (other than an alarm or the button, which
/// [`Night::end`] reports).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Over {
    /// Awake for longer than [`QUIET_AWAKE`] (or [`NAP_AWAKE`]).
    Awake,
    /// Local noon came.
    Noon,
}

impl Night {
    /// Whether it's the quiet night now.
    pub fn quiet(&self) -> bool {
        self.quiet
    }

    /// Call regularly (every second or minute; it only compares times) with
    /// whether the wearer is asleep (`crate::activity::asleep`), the time in
    /// monotonic seconds, whether local noon has passed since the last
    /// call ([`noon_between`]), and whether falling asleep now would make a
    /// nap ([`nap_hours`]). Returns what changed, if anything.
    pub fn update(
        &mut self,
        asleep: bool,
        now: u32,
        noon: bool,
        nap_hours: bool,
    ) -> Option<Change> {
        let fell_asleep = asleep && !self.asleep;
        self.asleep = asleep;
        if asleep {
            self.last_asleep = now;
        }
        if !self.quiet {
            self.quiet = fell_asleep;
            self.nap = nap_hours;
            return fell_asleep.then_some(Change::Quiet { nap: nap_hours });
        }
        let awake_limit = if self.nap { NAP_AWAKE } else { QUIET_AWAKE };
        let over = if noon {
            Over::Noon
        } else if now.wrapping_sub(self.last_asleep) > awake_limit as u32 * 60 {
            Over::Awake
        } else {
            return None;
        };
        self.quiet = false;
        Some(Change::Over(over))
    }

    /// Call when an alarm rings (not a timer), or the wearer ends the night
    /// by hand (a double press of the button): the night is over. Returns
    /// whether it was quiet until now.
    pub fn end(&mut self) -> bool {
        core::mem::replace(&mut self.quiet, false)
    }
}

/// Whether local noon falls in `(before, now]` (local seconds since 1970,
/// as in `crate::alarm`).
pub fn noon_between(before: i64, now: i64) -> bool {
    const DAY: i64 = 24 * 60 * 60;
    const NOON: i64 = 12 * 60 * 60;
    // The last noon at or before `now`.
    let noon = (now - NOON).div_euclid(DAY) * DAY + NOON;
    before < noon
}
