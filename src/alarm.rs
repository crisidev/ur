//! Alarms: when they ring. (The UI sets them and shows them ringing; this
//! module only decides.)
//!
//! Three kinds:
//!
//! - **Smart wake**: "wake me before 07:00, when my sleep allows". Separate
//!   times for weekdays and weekends, and one switch for both (off for a
//!   holiday). In the [`SMART_WINDOW`] before the time it rings at the first
//!   sign of light sleep: a stir. Being woken mid-stir rather than from
//!   deep stillness is the whole point. At the time itself it rings
//!   regardless.
//! - **Alarms**: two, each at a time of day, the next time it comes round.
//!   Each turns itself off after ringing.
//! - **Timers**: two, each ringing after a duration. Each remembers its last
//!   duration, as that's usually the one wanted again (the tea, the
//!   laundry).
//!
//! Times are local, so they follow the phone's time zone. Everything is
//! computed in "local seconds since 1970" (UTC plus the UTC offset), which
//! makes "the next 07:00" plain arithmetic; times of day are minutes after
//! midnight.
//!
//! Like [`crate::sleep`], this knows nothing of hardware or the clock, so it
//! runs in host tests (`host-tests/alarm`).

/// How long before a smart alarm's time it may ring early, in minutes.
pub const SMART_WINDOW: i64 = 30;
/// How long a snooze lasts, in seconds.
pub const SNOOZE: i64 = 9 * 60;
/// How long an alarm rings before giving up (and counting as dismissed),
/// in minutes.
pub const RING_FOR: i64 = 3;
/// How many timers there are.
pub const TIMERS: usize = 2;
/// How many one-off alarms there are.
pub const ALARMS: usize = 2;

const DAY: i64 = 24 * 60;

/// The settings, as stored.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[cfg_attr(target_os = "none", derive(defmt::Format))]
pub struct Settings {
    pub smart: bool,
    /// Minutes after local midnight.
    pub smart_weekday: u16,
    pub smart_weekend: u16,
    /// Each one-off alarm's switch and time (minutes after midnight).
    pub at: [bool; ALARMS],
    pub at_time: [u16; ALARMS],
    /// Each timer's duration, in minutes.
    pub timers: [u16; TIMERS],
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            smart: false,
            smart_weekday: 7 * 60,
            smart_weekend: 9 * 60,
            at: [false; ALARMS],
            at_time: [7 * 60, 7 * 60 + 30],
            timers: [10, 3],
        }
    }
}

impl Settings {
    pub const BYTES: usize = 13;

    pub fn to_bytes(self) -> [u8; Self::BYTES] {
        let flags = u8::from(self.smart) | u8::from(self.at[0]) << 1 | u8::from(self.at[1]) << 2;
        let mut bytes = [flags; Self::BYTES];
        let words = [
            self.smart_weekday,
            self.smart_weekend,
            self.at_time[0],
            self.at_time[1],
            self.timers[0],
            self.timers[1],
        ];
        for (i, word) in words.into_iter().enumerate() {
            bytes[1 + 2 * i..3 + 2 * i].copy_from_slice(&word.to_le_bytes());
        }
        bytes
    }

    /// Out-of-range values (corrupt or from a future format) fall back to
    /// the defaults rather than ringing at nonsense times.
    pub fn from_bytes(bytes: [u8; Self::BYTES]) -> Self {
        let word = |i: usize| u16::from_le_bytes([bytes[i], bytes[i + 1]]);
        let default = Self::default();
        let time = |t: u16, fallback: u16| if t < DAY as u16 { t } else { fallback };
        let timer = |t: u16, fallback: u16| if (1..=MAX_TIMER).contains(&t) { t } else { fallback };
        Self {
            smart: bytes[0] & 1 != 0,
            smart_weekday: time(word(1), default.smart_weekday),
            smart_weekend: time(word(3), default.smart_weekend),
            at: [bytes[0] & 2 != 0, bytes[0] & 4 != 0],
            at_time: [
                time(word(5), default.at_time[0]),
                time(word(7), default.at_time[1]),
            ],
            timers: [
                timer(word(9), default.timers[0]),
                timer(word(11), default.timers[1]),
            ],
        }
    }
}

/// The longest timer, in minutes.
pub const MAX_TIMER: u16 = 16 * 60;

/// Which alarm is ringing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[cfg_attr(target_os = "none", derive(defmt::Format))]
pub enum Kind {
    Smart,
    /// Which of the [`ALARMS`].
    At(u8),
    /// Which of the [`TIMERS`].
    Timer(u8),
    Snooze,
}

/// 0 = Monday … 6 = Sunday, of local day `day` (days since 1970-01-01,
/// a Thursday).
fn weekday(day: i64) -> i64 {
    (day + 3).rem_euclid(7)
}

/// The smart alarm's next time (local minutes), strictly after minute
/// `now`.
pub fn next_smart(settings: &Settings, now: i64) -> i64 {
    let today = now.div_euclid(DAY);
    (today..today + 3)
        .map(|day| {
            let time = if weekday(day) < 5 {
                settings.smart_weekday
            } else {
                settings.smart_weekend
            };
            day * DAY + i64::from(time)
        })
        .find(|&at| at > now)
        .expect("one of three days has its time ahead")
}

/// The next time (local minutes) a daily `time` comes round, strictly after
/// minute `now`.
pub fn next_daily(time: u16, now: i64) -> i64 {
    let today = now.div_euclid(DAY) * DAY + i64::from(time);
    if today > now { today } else { today + DAY }
}

/// What's armed, beyond the settings, and what has already rung. Times
/// are local seconds unless they say otherwise.
#[derive(Clone, Copy, Debug, Default)]
pub struct State {
    /// When each running timer rings.
    pub timers: [Option<i64>; TIMERS],
    /// When a snoozed alarm rings again.
    pub snooze_at: Option<i64>,
    /// The smart alarm occurrence (its latest time, local minutes) that has
    /// already rung, so it rings once per morning.
    smart_rang: Option<i64>,
    /// The occurrence (local minutes) each one-off alarm is armed for: fixed
    /// when it's switched on (or first checked after a reset, since this
    /// isn't stored), so that ringing (which switches it off) is for that
    /// one only.
    at_armed: [Option<i64>; ALARMS],
}

impl State {
    /// Call when a one-off alarm is switched on (or its time changes).
    pub fn arm_at(&mut self, alarm: usize, settings: &Settings, now: i64) {
        self.at_armed[alarm] = settings.at[alarm]
            .then(|| next_daily(settings.at_time[alarm], now.div_euclid(60)));
    }

    pub fn start_timer(&mut self, timer: usize, settings: &Settings, now: i64) {
        self.timers[timer] = Some(now + i64::from(settings.timers[timer]) * 60);
    }

    pub fn stop_timer(&mut self, timer: usize) {
        self.timers[timer] = None;
    }

    /// Seconds until `timer` rings, if it's running.
    pub fn remaining(&self, timer: usize, now: i64) -> Option<i64> {
        self.timers[timer].map(|at| (at - now).max(0))
    }

    pub fn snooze(&mut self, now: i64) {
        self.snooze_at = Some(now + SNOOZE);
    }

    /// The next thing to ring, and when (local seconds), for the clock face.
    /// A smart alarm counts at its latest.
    pub fn next(&self, settings: &Settings, now: i64) -> Option<(Kind, i64)> {
        let minute = now.div_euclid(60);
        let smart = settings
            .smart
            .then(|| (Kind::Smart, self.pending_smart(settings, minute) * 60));
        let alarms = (0..ALARMS).filter(|&i| settings.at[i]).map(|i| {
            let at = self.at_armed[i].unwrap_or_else(|| next_daily(settings.at_time[i], minute));
            (Kind::At(i as u8), at * 60)
        });
        let timers = (0..TIMERS).filter_map(|i| self.timers[i].map(|t| (Kind::Timer(i as u8), t)));
        let snooze = self.snooze_at.map(|t| (Kind::Snooze, t));
        [smart, snooze]
            .into_iter()
            .flatten()
            .chain(alarms)
            .chain(timers)
            .min_by_key(|&(_, t)| t)
    }

    /// The smart alarm occurrence (local minutes) still to ring: the current
    /// one if we're in its window and it hasn't rung, else the next.
    fn pending_smart(&self, settings: &Settings, minute: i64) -> i64 {
        // An occurrence whose time has just passed (by up to a minute) is
        // still due, in case a check was late.
        let next = next_smart(settings, minute - 1);
        if self.smart_rang == Some(next) {
            next_smart(settings, next)
        } else {
            next
        }
    }

    /// Checks, every second or so, whether an alarm should start ringing
    /// now (`now` in local seconds). `stirring` says the wearer just moved
    /// after being still (see `crate::activity`), which is what a smart
    /// alarm waits for in its window. Turns off what rings (a one-off alarm,
    /// a timer), and remembers the smart one.
    pub fn due(&mut self, settings: &mut Settings, now: i64, stirring: bool) -> Option<Kind> {
        if self.snooze_at.is_some_and(|t| now >= t) {
            self.snooze_at = None;
            return Some(Kind::Snooze);
        }
        for (i, timer) in self.timers.iter_mut().enumerate() {
            if timer.is_some_and(|t| now >= t) {
                *timer = None;
                return Some(Kind::Timer(i as u8));
            }
        }
        let minute = now.div_euclid(60);
        for i in 0..ALARMS {
            if settings.at[i] && self.at_armed[i].is_none() {
                self.arm_at(i, settings, now);
            }
            if settings.at[i]
                && let Some(t) = self.at_armed[i]
                && minute >= t
            {
                settings.at[i] = false;
                self.at_armed[i] = None;
                return Some(Kind::At(i as u8));
            }
        }
        if settings.smart {
            let latest = self.pending_smart(settings, minute);
            let in_window = minute >= latest - SMART_WINDOW;
            if minute >= latest || (in_window && stirring) {
                self.smart_rang = Some(latest);
                return Some(Kind::Smart);
            }
        }
        None
    }
}
