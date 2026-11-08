//! A day's activity hour by hour, for the Today screen (see [`crate::ui`]):
//! steps, the heart rate's range, and a resting heart rate.
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
