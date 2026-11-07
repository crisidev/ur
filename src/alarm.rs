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
//! - **At**: one alarm at a time of day, the next time it comes round. It
//!   turns itself off after ringing.
//! - **Timer**: rings after a duration. The last duration is remembered, as
//!   that's usually the one wanted again (the tea, the laundry).
//!
//! Times are local, so they follow the phone's time zone. Everything is
//! computed in "local minutes since 1970" (UTC minutes plus the UTC offset),
//! which makes "the next 07:00" plain arithmetic.
//!
//! Like [`crate::sleep`], this knows nothing of hardware or the clock, so it
//! runs in host tests (`host-tests/alarm`).

/// How long before a smart alarm's time it may ring early.
pub const SMART_WINDOW: i64 = 30;
/// How long a snooze lasts.
pub const SNOOZE: i64 = 9;
/// How long an alarm rings before giving up (and counting as dismissed).
pub const RING_FOR: i64 = 3;

const DAY: i64 = 24 * 60;

/// The settings, as stored.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[cfg_attr(target_os = "none", derive(defmt::Format))]
pub struct Settings {
    pub smart: bool,
    /// Minutes after local midnight.
    pub smart_weekday: u16,
    pub smart_weekend: u16,
    pub at: bool,
    pub at_time: u16,
    /// The timer's duration, in minutes.
    pub timer: u16,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            smart: false,
            smart_weekday: 7 * 60,
            smart_weekend: 9 * 60,
            at: false,
            at_time: 7 * 60,
            timer: 10,
        }
    }
}

impl Settings {
    pub const BYTES: usize = 9;

    pub fn to_bytes(self) -> [u8; Self::BYTES] {
        let [a, b] = self.smart_weekday.to_le_bytes();
        let [c, d] = self.smart_weekend.to_le_bytes();
        let [e, f] = self.at_time.to_le_bytes();
        let [g, h] = self.timer.to_le_bytes();
        [u8::from(self.smart) | u8::from(self.at) << 1, a, b, c, d, e, f, g, h]
    }

    /// Out-of-range values (corrupt or from a future format) fall back to
    /// the defaults rather than ringing at nonsense times.
    pub fn from_bytes(bytes: [u8; Self::BYTES]) -> Self {
        let word = |i: usize| u16::from_le_bytes([bytes[i], bytes[i + 1]]);
        let default = Self::default();
        let time = |t: u16, fallback: u16| if t < DAY as u16 { t } else { fallback };
        Self {
            smart: bytes[0] & 1 != 0,
            smart_weekday: time(word(1), default.smart_weekday),
            smart_weekend: time(word(3), default.smart_weekend),
            at: bytes[0] & 2 != 0,
            at_time: time(word(5), default.at_time),
            timer: match word(7) {
                1..=999 => word(7),
                _ => default.timer,
            },
        }
    }
}

/// Which alarm is ringing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[cfg_attr(target_os = "none", derive(defmt::Format))]
pub enum Kind {
    Smart,
    At,
    Timer,
    Snooze,
}

/// 0 = Monday … 6 = Sunday, of local day `day` (days since 1970-01-01,
/// a Thursday).
fn weekday(day: i64) -> i64 {
    (day + 3).rem_euclid(7)
}

/// The smart alarm's next time (local minutes), strictly after `now`.
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
/// `now`.
pub fn next_daily(time: u16, now: i64) -> i64 {
    let today = now.div_euclid(DAY) * DAY + i64::from(time);
    if today > now { today } else { today + DAY }
}

/// What's armed, beyond the settings: the times that make one-off and
/// running alarms (local minutes), and what has already rung.
#[derive(Clone, Copy, Debug, Default)]
pub struct State {
    /// When the timer (if running) rings.
    pub timer_at: Option<i64>,
    /// When a snoozed alarm rings again.
    pub snooze_at: Option<i64>,
    /// The smart alarm occurrence (its latest time) that has already rung,
    /// so it rings once per morning.
    smart_rang: Option<i64>,
    /// The "at" alarm occurrence it's armed for: fixed when it's switched
    /// on (or first checked after a reset, since this isn't stored), so
    /// that ringing (which switches it off) is for that one only.
    at_armed: Option<i64>,
}

impl State {
    /// Call when the "at" alarm is switched on (or its time changes).
    pub fn arm_at(&mut self, settings: &Settings, now: i64) {
        self.at_armed = settings.at.then(|| next_daily(settings.at_time, now));
    }

    pub fn start_timer(&mut self, settings: &Settings, now: i64) {
        self.timer_at = Some(now + i64::from(settings.timer));
    }

    pub fn snooze(&mut self, now: i64) {
        self.snooze_at = Some(now + SNOOZE);
    }

    /// The next time anything rings (local minutes), for the clock face.
    /// A smart alarm counts at its latest.
    pub fn next(&self, settings: &Settings, now: i64) -> Option<(Kind, i64)> {
        let smart = settings.smart.then(|| (Kind::Smart, self.pending_smart(settings, now)));
        let at = settings.at.then(|| {
            (Kind::At, self.at_armed.unwrap_or_else(|| next_daily(settings.at_time, now)))
        });
        let timer = self.timer_at.map(|t| (Kind::Timer, t));
        let snooze = self.snooze_at.map(|t| (Kind::Snooze, t));
        [smart, at, timer, snooze].into_iter().flatten().min_by_key(|&(_, t)| t)
    }

    /// The smart alarm occurrence still to ring: the current one if we're
    /// in its window and it hasn't rung, else the next.
    fn pending_smart(&self, settings: &Settings, now: i64) -> i64 {
        // An occurrence whose time has just passed (by up to a minute) is
        // still due, in case a check was late.
        let next = next_smart(settings, now - 1);
        if self.smart_rang == Some(next) {
            next_smart(settings, next)
        } else {
            next
        }
    }

    /// Checks, once a minute or more often, whether an alarm should start
    /// ringing now. `stirring` says the wearer just moved after being
    /// still (see `crate::activity`), which is what a smart alarm waits for
    /// in its window. Turns off what rings (the "at" alarm, the timer), and
    /// remembers the smart one.
    pub fn due(&mut self, settings: &mut Settings, now: i64, stirring: bool) -> Option<Kind> {
        if self.snooze_at.is_some_and(|t| now >= t) {
            self.snooze_at = None;
            return Some(Kind::Snooze);
        }
        if self.timer_at.is_some_and(|t| now >= t) {
            self.timer_at = None;
            return Some(Kind::Timer);
        }
        if settings.at && self.at_armed.is_none() {
            self.arm_at(settings, now);
        }
        if settings.at
            && let Some(t) = self.at_armed
            && now >= t
        {
            settings.at = false;
            self.at_armed = None;
            return Some(Kind::At);
        }
        if settings.smart {
            let latest = self.pending_smart(settings, now);
            let in_window = now >= latest - SMART_WINDOW;
            if now >= latest || (in_window && stirring) {
                self.smart_rang = Some(latest);
                return Some(Kind::Smart);
            }
        }
        None
    }
}
