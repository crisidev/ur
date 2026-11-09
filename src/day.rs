//! A day's activity hour by hour, for the Today screen (see [`crate::ui`]):
//! steps, the heart rate's range, and a resting heart rate; and which
//! steps today's total counts, for the clock face.
//!
//! [`crate::activity`] feeds it the day's per-minute samples; this only
//! adds them up. Hardware-free, so it runs in host tests
//! (`host-tests/day`).

pub const HOURS: usize = 24;

/// Readings an hour needs before its average can count as the resting
/// heart rate: fewer could be one calm moment in a busy hour. Asleep, the
/// heart rate is measured every 5 minutes (12 an hour); awake and at rest,
/// every 20 at the most (3).
pub const RESTING_READINGS: u8 = 3;

/// One hour's activity.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct Hour {
    /// Saturating, though no wrist takes 65535 steps in an hour.
    pub steps: u16,
    /// The lowest and highest heart rate measured; 0 if none was.
    pub heart_min: u8,
    pub heart_max: u8,
    /// For the hour's average: at most 60 readings of at most 255. (Public
    /// for `compare`, which copies days out for the simulator.)
    pub heart_sum: u16,
    pub heart_count: u8,
}

impl Hour {
    /// The average heart rate, if it was measured often enough to say
    /// (see [`RESTING_READINGS`]).
    fn resting(&self) -> Option<u8> {
        (self.heart_count >= RESTING_READINGS)
            .then(|| (self.heart_sum / u16::from(self.heart_count)) as u8)
    }
}

/// From local midnight: hour 0 is midnight to 1 am.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct Day {
    pub hours: [Hour; HOURS],
}

impl Day {
    /// Adds a minute's sample: `minute` of the day (0 at local midnight),
    /// its steps, and its heart rate (0 if none was measured). Minutes
    /// outside the day are ignored.
    pub fn add(&mut self, minute: usize, steps: u16, heart_rate: u8) {
        let Some(hour) = self.hours.get_mut(minute / 60) else {
            return;
        };
        hour.steps = hour.steps.saturating_add(steps);
        if heart_rate > 0 {
            if hour.heart_count == 0 {
                hour.heart_min = heart_rate;
                hour.heart_max = heart_rate;
            } else {
                hour.heart_min = hour.heart_min.min(heart_rate);
                hour.heart_max = hour.heart_max.max(heart_rate);
            }
            hour.heart_sum = hour.heart_sum.saturating_add(u16::from(heart_rate));
            hour.heart_count = hour.heart_count.saturating_add(1);
        }
    }

    /// The resting heart rate: the lowest hourly average (of the hours
    /// measured often enough, see [`RESTING_READINGS`]). Like Garmin's
    /// (the lowest 30-minute average of the day), this counts the night,
    /// whose hours after midnight are usually the lowest.
    ///
    /// TODO: a 30-minute window, as Garmin's, if hours turn out too
    /// coarse; the samples are per minute.
    pub fn resting(&self) -> Option<u8> {
        self.hours.iter().filter_map(Hour::resting).min()
    }

    /// The lowest and highest heart rate measured all day.
    pub fn heart_range(&self) -> Option<(u8, u8)> {
        let measured = self.hours.iter().filter(|h| h.heart_count > 0);
        let low = measured.clone().map(|h| h.heart_min).min()?;
        let high = measured.map(|h| h.heart_max).max()?;
        Some((low, high))
    }

    /// The most steps in any one hour.
    pub fn most_steps(&self) -> u16 {
        self.hours.iter().map(|h| h.steps).max().unwrap_or(0)
    }
}

// ==============================================================================
// Today's step total
// ==============================================================================
//
// The clock face shows a running total of today's steps, kept apart from
// the per-minute history (which is UTC, and holds a day) because the
// total counts steps the history can't: those before the phone set the
// clock. It starts over at local midnight. But the local date also
// changes without midnight passing: a flight across time zones, or the
// phone setting a wrong clock right. Flying west across midnight, the
// date goes back (and comes round again an hour or more later), and a
// total that merely starts over on every new date counted the evening
// after landing as a new day, then started over again at the real
// midnight; flying east, it started over at the change, but not from the
// new zone's midnight. Either way the total disagreed with the Today
// screen, which adds up the history since local midnight.
//
// So, as phones do, a date that changed other than by midnight passing
// takes today's total from the history: the steps since the new zone's
// midnight, exactly what the Today screen shows. Midnight passing is the
// same rule (there's nothing in the history since midnight yet), so
// there's no telling the two apart. The history always reaches back far
// enough: local midnight is never more than a day ago. The cost: the
// steps counted before the phone first set the clock that day aren't in
// the history, so they're dropped.
//
// A change that keeps the date (a time zone an hour over, summer time)
// leaves the total alone, though the Today screen, from the new zone's
// midnight, may then miss the steps between the two midnights: the date
// didn't change, so nothing says the total is wrong, and it may count
// steps the history lacks.

/// The local date (days since 1970) of `utc_minute` (minutes since 1970,
/// UTC), `offset` minutes ahead of UTC.
pub fn date(utc_minute: i64, offset: i16) -> i64 {
    (utc_minute + i64::from(offset)).div_euclid(MINUTES_A_DAY)
}

/// The UTC minute of the local midnight that began `utc_minute`'s day,
/// `offset` minutes ahead of UTC.
pub fn midnight(utc_minute: i64, offset: i16) -> i64 {
    utc_minute - (utc_minute + i64::from(offset)).rem_euclid(MINUTES_A_DAY)
}

const MINUTES_A_DAY: i64 = 24 * 60;

/// Today's step total on local date `today`, from `steps` counted on
/// `counted_on` (a local date, or `None` if the clock wasn't known then),
/// and `since_midnight`, which adds up the history's steps since today's
/// local midnight (and is only asked when needed).
///
/// - Counted today: as counted.
/// - Counted before the clock was known: today's, as the phone sets the
///   clock soon after the watch starts.
/// - Counted on another date: the history since midnight (see above),
///   even if some were counted before the clock was known (a total
///   restored after the watch was off keeps its date).
pub fn steps_today(
    counted_on: Option<i64>,
    steps: u32,
    today: i64,
    since_midnight: impl FnOnce() -> u32,
) -> u32 {
    match counted_on {
        Some(date) if date != today => since_midnight(),
        _ => steps,
    }
}
