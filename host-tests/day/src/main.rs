//! Host tests for the firmware's `day` module: a day's activity, hour by
//! hour, for the Today screen, and which steps today's total counts.
//! `cargo test` runs them.

#[path = "../../../src/day.rs"]
mod day;

fn main() {
    println!("run `cargo test`");
}

#[cfg(test)]
mod tests {
    use super::day::*;

    #[test]
    fn steps_add_up_by_the_hour() {
        let mut day = Day::default();
        day.add(0, 10, 0);
        day.add(59, 5, 0);
        day.add(60, 7, 0);
        day.add(23 * 60 + 59, 1, 0);
        assert_eq!(day.hours[0].steps, 15);
        assert_eq!(day.hours[1].steps, 7);
        assert_eq!(day.hours[23].steps, 1);
        assert_eq!(day.most_steps(), 15);
    }

    #[test]
    fn minutes_beyond_the_day_are_ignored() {
        let mut day = Day::default();
        day.add(24 * 60, 100, 80);
        assert_eq!(day, Day::default());
    }

    #[test]
    fn an_empty_day_has_no_heart_rate() {
        let day = Day::default();
        assert_eq!(day.heart_range(), None);
        assert_eq!(day.resting(), None);
        assert_eq!(day.most_steps(), 0);
    }

    #[test]
    fn each_hour_keeps_its_heart_rate_range() {
        let mut day = Day::default();
        for (minute, bpm) in [(0, 60), (5, 52), (10, 0), (15, 71)] {
            day.add(minute, 0, bpm);
        }
        day.add(120, 0, 130);
        assert_eq!((day.hours[0].heart_min, day.hours[0].heart_max), (52, 71));
        assert_eq!((day.hours[1].heart_min, day.hours[1].heart_max), (0, 0));
        assert_eq!(day.heart_range(), Some((52, 130)));
    }

    #[test]
    fn resting_is_the_calmest_hour_measured_often_enough() {
        let mut day = Day::default();
        // A night hour, measured every 5 minutes: averages 50.
        for i in 0..12 {
            day.add(3 * 60 + 5 * i, 0, if i % 2 == 0 { 48 } else { 52 });
        }
        // An afternoon hour, three readings: averages 70.
        for i in 0..3 {
            day.add(15 * 60 + 20 * i, 0, 70);
        }
        // One calm moment in a busy hour isn't resting: too few readings.
        day.add(18 * 60, 0, 45);
        day.add(18 * 60 + 30, 0, 110);
        assert_eq!(day.resting(), Some(50));
    }

    #[test]
    fn too_few_readings_say_nothing_about_resting() {
        let mut day = Day::default();
        for minute in [0, 120, 240] {
            day.add(minute, 0, 60);
        }
        assert_eq!(day.resting(), None);
        assert_eq!(day.heart_range(), Some((60, 60)));
    }
}

/// Today's step total across midnight, time zones, and summer time:
/// through a model of the watch that keeps it as `activity` does (a
/// total and its date, and a per-minute history in UTC), checked against
/// what the Today screen adds up from the history.
#[cfg(test)]
mod steps_today {
    use std::collections::BTreeMap;

    use super::day::{self, Day};

    const OSLO: i16 = 60;
    const OSLO_SUMMER: i16 = 120;
    const LONDON: i16 = 0;

    /// The UTC minute (since 1970) of a local time `offset` minutes
    /// ahead of UTC.
    fn at(date: (i64, i64, i64), (hour, minute): (i64, i64), offset: i16) -> i64 {
        // Howard Hinnant's days-from-civil, as `clock` has it.
        let (year, month, day) = date;
        let year = if month <= 2 { year - 1 } else { year };
        let era = year.div_euclid(400);
        let year_of_era = year - era * 400;
        let day_of_year = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
        let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
        let days = era * 146_097 + day_of_era - 719_468;
        days * 1440 + hour * 60 + minute - i64::from(offset)
    }

    /// The watch, as far as today's steps go.
    #[derive(Default)]
    struct Watch {
        /// UTC minute and offset, once the phone has set the clock.
        clock: Option<(i64, i16)>,
        /// The total, and the date it was counted on (as `activity::Today`).
        total: Option<(Option<i64>, u32)>,
        /// Steps by UTC minute.
        history: BTreeMap<i64, u32>,
    }

    impl Watch {
        /// The phone sets the clock; the activity task's next poll
        /// follows within seconds.
        fn set(&mut self, utc_minute: i64, offset: i16) {
            self.clock = Some((utc_minute, offset));
            self.walk(0);
        }

        /// Moves the clock on, as time passes, to the next poll.
        fn wait(&mut self, minutes: i64) {
            self.clock.as_mut().expect("the clock is set").0 += minutes;
            self.walk(0);
        }

        fn since_midnight(&self, (utc_minute, offset): (i64, i16)) -> u32 {
            let midnight = day::midnight(utc_minute, offset);
            self.history
                .range(midnight..=utc_minute)
                .map(|(_, s)| s)
                .sum()
        }

        /// `activity::Today::on`.
        fn on(&self, now: (i64, i16), (counted_on, steps): (Option<i64>, u32)) -> (i64, u32) {
            let today = day::date(now.0, now.1);
            let total = day::steps_today(counted_on, steps, today, || self.since_midnight(now));
            (today, total)
        }

        /// A poll's steps (the activity task polls every 2 s, with or
        /// without any): in today's total (`activity::add_steps_today`),
        /// then in the history.
        fn walk(&mut self, steps: u32) {
            self.total = Some(match (self.total, self.clock) {
                (Some(total), Some(now)) => {
                    let (today, total) = self.on(now, total);
                    (Some(today), total + steps)
                }
                (Some((counted_on, total)), None) => (counted_on, total + steps),
                (None, now) => (now.map(|(m, offset)| day::date(m, offset)), steps),
            });
            if let Some((now, _)) = self.clock
                && steps > 0
            {
                *self.history.entry(now).or_default() += steps;
            }
        }

        /// The clock face's total (`activity::steps_today`).
        fn count(&self) -> u32 {
            let now = self.clock.expect("the clock is set");
            self.total.map_or(0, |total| self.on(now, total).1)
        }

        /// The Today screen's (`activity::today`).
        fn chart(&self) -> u32 {
            let (utc_minute, offset) = self.clock.expect("the clock is set");
            let midnight = day::midnight(utc_minute, offset);
            let mut day = Day::default();
            for (&minute, &steps) in self.history.range(midnight..=utc_minute) {
                day.add((minute - midnight) as usize, steps as u16, 0);
            }
            day.hours.iter().map(|h| u32::from(h.steps)).sum()
        }

        /// Both, which must agree.
        fn steps(&self) -> u32 {
            assert_eq!(self.count(), self.chart(), "the clock face and Today");
            self.count()
        }
    }

    #[test]
    fn dates_and_midnights() {
        let noon = at((2026, 11, 9), (12, 0), LONDON);
        assert_eq!(noon, 29_903_760);
        assert_eq!(day::midnight(noon, OSLO), at((2026, 11, 9), (0, 0), OSLO));
        // Already the 10th in Auckland, still the 9th in Honolulu.
        assert_eq!(day::date(noon, 13 * 60), day::date(noon, LONDON) + 1);
        assert_eq!(day::date(noon, -10 * 60), day::date(noon, LONDON));
        assert_eq!(
            day::midnight(noon, 13 * 60),
            at((2026, 11, 10), (0, 0), 13 * 60)
        );
        assert_eq!(
            day::midnight(noon, -10 * 60),
            at((2026, 11, 9), (0, 0), -10 * 60)
        );
    }

    #[test]
    fn midnight_starts_over() {
        let mut watch = Watch::default();
        watch.set(at((2026, 11, 9), (23, 50), OSLO), OSLO);
        watch.walk(500);
        assert_eq!(watch.steps(), 500);
        watch.wait(15);
        assert_eq!(watch.steps(), 0);
        watch.walk(10);
        assert_eq!(watch.steps(), 10);
    }

    /// The simulator's repro (tmp/tzaudit/steps-west.txt): the clock face
    /// showed 5 steps after landing, under Today's 1,205.
    #[test]
    fn flying_west_across_midnight() {
        let mut watch = Watch::default();
        watch.set(at((2026, 11, 9), (23, 40), OSLO), OSLO);
        watch.walk(1000);
        watch.wait(30);
        watch.walk(200);
        assert_eq!(watch.steps(), 200);
        // Landing in London, where it's still the 9th: the evening's
        // steps, Oslo's after midnight too.
        watch.set(at((2026, 11, 9), (23, 12), LONDON), LONDON);
        assert_eq!(watch.steps(), 1200);
        watch.walk(5);
        assert_eq!(watch.steps(), 1205);
        // London's midnight.
        watch.wait(58);
        assert_eq!(watch.steps(), 0);
        watch.walk(7);
        assert_eq!(watch.steps(), 7);
    }

    #[test]
    fn flying_east_across_midnight() {
        let mut watch = Watch::default();
        watch.set(at((2026, 11, 9), (12, 0), LONDON), LONDON);
        watch.walk(3000);
        watch.set(at((2026, 11, 9), (23, 40), LONDON), LONDON);
        watch.walk(50);
        assert_eq!(watch.steps(), 3050);
        // Landing in Oslo, already the 10th: the steps since Oslo's
        // midnight, an hour before London's.
        watch.set(at((2026, 11, 10), (0, 41), OSLO), OSLO);
        assert_eq!(watch.steps(), 50);
        watch.walk(8);
        assert_eq!(watch.steps(), 58);
    }

    /// A new zone on the same date keeps the total, even the steps
    /// between the two zones' midnights, which the Today screen no
    /// longer shows: the date didn't change, so nothing says the total
    /// is wrong, and the history may not have all of it (the steps
    /// before the phone set the clock).
    #[test]
    fn a_new_zone_on_the_same_date_keeps_the_total() {
        let mut watch = Watch::default();
        watch.set(at((2026, 11, 9), (0, 30), OSLO), OSLO);
        watch.walk(40);
        watch.set(at((2026, 11, 9), (15, 0), OSLO), OSLO);
        watch.walk(100);
        assert_eq!(watch.steps(), 140);
        watch.set(at((2026, 11, 9), (14, 1), LONDON), LONDON);
        assert_eq!(watch.count(), 140);
        assert_eq!(watch.chart(), 100);
        watch.walk(1);
        assert_eq!(watch.count(), 141);
    }

    /// Central European summer time: it starts and ends in the small
    /// hours, so the date stays, and so does the total.
    #[test]
    fn summer_time_keeps_the_total() {
        let mut watch = Watch::default();
        // Ends 2026-10-25: 03:00 CEST is 02:00 CET.
        watch.set(at((2026, 10, 25), (1, 30), OSLO_SUMMER), OSLO_SUMMER);
        watch.walk(20);
        watch.wait(30);
        watch.set(at((2026, 10, 25), (2, 0), OSLO), OSLO);
        assert_eq!(watch.count(), 20);
        watch.walk(5);
        assert_eq!(watch.count(), 25);
        // Starts 2026-03-29: 02:00 CET is 03:00 CEST.
        watch.set(at((2026, 3, 29), (1, 50), OSLO), OSLO);
        watch.walk(30);
        watch.wait(10);
        watch.set(at((2026, 3, 29), (3, 0), OSLO_SUMMER), OSLO_SUMMER);
        assert_eq!(watch.steps(), 30);
        watch.walk(4);
        assert_eq!(watch.steps(), 34);
    }

    /// Where summer time ends at midnight (Chile's, 2026-04-05: 00:00
    /// -03:00 is 23:00 -04:00 on the 4th), the date goes back, as flying
    /// west does.
    #[test]
    fn summer_time_ending_at_midnight() {
        const SUMMER: i16 = -3 * 60;
        const WINTER: i16 = -4 * 60;
        let mut watch = Watch::default();
        watch.set(at((2026, 4, 4), (18, 0), SUMMER), SUMMER);
        watch.walk(900);
        watch.set(at((2026, 4, 4), (23, 59), SUMMER), SUMMER);
        watch.wait(1);
        watch.walk(10);
        assert_eq!(watch.steps(), 10);
        watch.set(at((2026, 4, 4), (23, 0), WINTER), WINTER);
        assert_eq!(watch.steps(), 910);
        watch.wait(60);
        assert_eq!(watch.steps(), 0);
    }

    /// Steps counted before the phone sets the clock are today's.
    #[test]
    fn steps_before_the_clock_count_for_today() {
        let mut watch = Watch::default();
        watch.walk(300);
        watch.set(at((2026, 11, 9), (8, 0), OSLO), OSLO);
        // Not in the history, so not on the Today screen.
        assert_eq!((watch.count(), watch.chart()), (300, 0));
        watch.walk(20);
        assert_eq!(watch.count(), 320);
    }

    /// A total restored after the watch was off (`system_off`) keeps its
    /// date; on another, the steps counted before the phone set the
    /// clock are dropped with it (the known cost: they're not in the
    /// history).
    #[test]
    fn a_restored_total_on_another_date() {
        let restored = Some(day::date(at((2026, 11, 9), (22, 0), OSLO), OSLO));
        let mut watch = Watch {
            total: Some((restored, 5000)),
            ..Watch::default()
        };
        watch.walk(30);
        watch.set(at((2026, 11, 9), (23, 0), OSLO), OSLO);
        assert_eq!(watch.count(), 5030);
        watch.set(at((2026, 11, 10), (7, 0), OSLO), OSLO);
        assert_eq!(watch.steps(), 0);
    }
}

/// The whole day against a plainer sum of the same samples, for random
/// days: what the proofs below leave out (see
/// `proofs::a_day_rests_exactly_when_an_hour_has_readings_enough`).
/// `cargo test` tries a few hundred days; `cargo bolero test
/// properties::any_day_rests_within_its_range` fuzzes for more.
#[cfg(test)]
mod properties {
    use super::day::*;

    /// Any samples, each minute at most once, as `activity::today` adds
    /// them (some beyond the day, which don't count): the day's sums are
    /// what adding them up by hand gives, and the resting heart rate, the
    /// lowest hourly average, lies within the day's range.
    #[test]
    fn any_day_rests_within_its_range() {
        bolero::check!()
            .with_type::<Vec<(u16, u16, u8)>>()
            .for_each(|samples| {
                let mut day = Day::default();
                let mut seen = [false; HOURS * 60 + 60];
                // Each hour's steps and readings, by hand.
                let mut steps = [0u32; HOURS];
                let mut readings: [Vec<u32>; HOURS] = Default::default();
                for &(minute, stepped, heart_rate) in samples {
                    let minute = usize::from(minute) % seen.len();
                    if core::mem::replace(&mut seen[minute], true) {
                        continue;
                    }
                    day.add(minute, stepped, heart_rate);
                    if minute < HOURS * 60 {
                        steps[minute / 60] += u32::from(stepped);
                        if heart_rate > 0 {
                            readings[minute / 60].push(heart_rate.into());
                        }
                    }
                }

                let most = steps.iter().max().expect("24 hours");
                assert_eq!(u32::from(day.most_steps()), (*most).min(u16::MAX.into()));

                let all = || readings.iter().flatten().copied();
                let range = all().min().zip(all().max());
                assert_eq!(day.heart_range().map(|(l, h)| (l.into(), h.into())), range);

                let resting = readings
                    .iter()
                    .filter(|hour| hour.len() >= usize::from(RESTING_READINGS))
                    .map(|hour| hour.iter().sum::<u32>() / hour.len() as u32)
                    .min();
                assert_eq!(day.resting().map(u32::from), resting);
                if let (Some(resting), Some((low, high))) = (resting, range) {
                    assert!(low <= resting && resting <= high);
                }
            });
    }
}

/// Proofs about a day's sums, for every sample and every day, where the
/// tests above add up chosen ones. `cargo kani` runs them (one at a time
/// with `--harness`: CBMC takes memory).
///
/// They lean on what `activity::today` does: it builds a day afresh from
/// the history, a minute at a time, each once. So no hour gets more than
/// 60 readings, which is what `Hour::heart_sum`'s docs count on; see
/// `more_readings_than_minutes_would_misreport` for why it matters.
#[cfg(kani)]
mod proofs {
    use super::day::*;

    /// The most heart rate readings an hour gets: one a minute.
    const READINGS: u8 = 60;

    /// Whether an hour is one that adding up at most [`READINGS`]
    /// readings can give: none and nothing to say of them, or a lowest no
    /// higher than the highest, and a sum between that many of each.
    fn sound(hour: &Hour) -> bool {
        let count = u16::from(hour.heart_count);
        if count == 0 {
            return hour.heart_sum == 0 && hour.heart_min == 0 && hour.heart_max == 0;
        }
        let (min, max) = (u16::from(hour.heart_min), u16::from(hour.heart_max));
        hour.heart_count <= READINGS
            && 1 <= min
            && min <= max
            && min * count <= hour.heart_sum
            && hour.heart_sum <= max * count
    }

    /// Any hour at all, sound or not.
    fn any_hour() -> Hour {
        Hour {
            steps: kani::any(),
            heart_min: kani::any(),
            heart_max: kani::any(),
            heart_sum: kani::any(),
            heart_count: kani::any(),
        }
    }

    fn any_day() -> Day {
        any_morning(HOURS)
    }

    /// Any day with nothing after its first `hours` hours.
    fn any_morning(hours: usize) -> Day {
        let mut day = Day::default();
        for hour in &mut day.hours[..hours] {
            *hour = any_hour();
        }
        day
    }

    /// Any sample, for any minute (in the day or not), added to any day:
    /// nothing panics or overflows, a minute outside the day changes
    /// nothing, and one inside changes its own hour alone. If that hour
    /// was sound with room for a reading, it still is, with the steps
    /// added (saturating) and the reading, if there was one, counted
    /// once: so by induction every day `activity::today` builds is sound.
    #[kani::proof]
    #[kani::unwind(25)]
    fn a_minute_goes_into_its_hour_alone() {
        let before = any_day();
        let (minute, steps, heart_rate): (usize, u16, u8) = kani::any();
        let mut day = before;
        day.add(minute, steps, heart_rate);

        if minute >= HOURS * 60 {
            assert_eq!(day, before);
            return;
        }
        let other: usize = kani::any();
        kani::assume(other < HOURS && other != minute / 60);
        assert_eq!(day.hours[other], before.hours[other]);

        let (was, is) = (before.hours[minute / 60], day.hours[minute / 60]);
        assert_eq!(is.steps, was.steps.saturating_add(steps));
        if heart_rate == 0 {
            // Not measured: the hour's heart rate is as it was.
            assert_eq!(Hour { steps: 0, ..is }, Hour { steps: 0, ..was });
        } else if sound(&was) && was.heart_count < READINGS {
            assert!(sound(&is));
            assert_eq!(is.heart_count, was.heart_count + 1);
            assert_eq!(is.heart_sum, was.heart_sum + u16::from(heart_rate));
            assert!(is.heart_min <= heart_rate && heart_rate <= is.heart_max);
            // The first reading is both ends; later ones only widen them.
            if was.heart_count == 0 {
                assert_eq!((is.heart_min, is.heart_max), (heart_rate, heart_rate));
            } else {
                assert!(is.heart_min <= was.heart_min && was.heart_max <= is.heart_max);
                assert!(is.heart_min == was.heart_min || is.heart_min == heart_rate);
                assert!(is.heart_max == was.heart_max || is.heart_max == heart_rate);
            }
        }
    }

    /// One hour of readings, any sound one, in an otherwise empty day: its average is the resting heart rate exactly when it
    /// has [`RESTING_READINGS`], lies between its lowest and highest
    /// reading (an average of readings can't be lower or higher than
    /// all of them), and so fits the `u8` it's cut down to.
    /// `ui::render_today` prints it into three characters.
    ///
    /// In the day's first hour, rather than any: CBMC then knows the
    /// other hours are empty, and is done in seconds, where with the
    /// hour unknown it didn't finish in the minutes it was given.
    #[kani::proof]
    #[kani::unwind(25)]
    fn an_hours_average_is_between_its_ends() {
        let hour = any_hour();
        kani::assume(sound(&hour));
        let mut day = Day::default();
        day.hours[0] = hour;

        match day.resting() {
            Some(resting) => {
                assert!(hour.heart_count >= RESTING_READINGS);
                assert!(hour.heart_min <= resting && resting <= hour.heart_max);
                let average = hour.heart_sum / u16::from(hour.heart_count);
                assert_eq!(u16::from(resting), average);
            }
            None => assert!(hour.heart_count < RESTING_READINGS),
        }
    }

    /// For a day whose hours each have a lowest reading no higher than
    /// their highest, if any (as sound hours do), nothing panics or
    /// overflows, and:
    ///
    /// - the range is the lowest and highest of the hours measured, lowest
    ///   first, and there is one exactly when any hour was measured
    ///   (`ui::render_today` draws each hour's bar from its `heart_min`
    ///   to its `heart_max` on a scale from the range, so a bar never
    ///   leaves the chart);
    /// - the most steps is some hour's, and no hour has more.
    fn has_its_range_and_its_busiest_hour(day: Day) {
        let most = day.most_steps();
        let range = day.heart_range();
        let (mut measured, mut busiest, mut none_busier) = (false, false, true);
        let (mut lowest, mut highest, mut within) = (false, false, true);
        for hour in &day.hours {
            busiest |= hour.steps == most;
            none_busier &= hour.steps <= most;
            if hour.heart_count > 0 {
                kani::assume(1 <= hour.heart_min && hour.heart_min <= hour.heart_max);
                measured = true;
                if let Some((low, high)) = range {
                    lowest |= hour.heart_min == low;
                    highest |= hour.heart_max == high;
                    within &= low <= hour.heart_min && hour.heart_max <= high;
                }
            }
        }

        assert!(busiest && none_busier);
        assert_eq!(range.is_some(), measured);
        if let Some((low, high)) = range {
            assert!(1 <= low && low <= high);
            assert!(lowest && highest && within);
        }
    }

    /// [`has_its_range_and_its_busiest_hour`], for any day with nothing
    /// after 6 am: seconds for CBMC, where a whole day takes it minutes.
    #[kani::proof]
    #[kani::unwind(25)]
    fn a_morning_has_its_range_and_its_busiest_hour() {
        has_its_range_and_its_busiest_hour(any_morning(6));
    }

    /// [`has_its_range_and_its_busiest_hour`], for any day at all.
    #[kani::proof]
    #[kani::unwind(25)]
    fn a_day_has_its_range_and_its_busiest_hour() {
        has_its_range_and_its_busiest_hour(any_day());
    }

    /// For any day at all, sound or not (the simulator makes them from
    /// bytes a dev kit sent, see `compare::Pass::day`), working out the
    /// resting heart rate doesn't panic, overflow or divide by zero, and
    /// there is one exactly when some hour has [`RESTING_READINGS`].
    ///
    /// That it's the lowest of those hours' averages is left to the
    /// property test (`tests::any_day_rests_within_its_range`): here,
    /// CBMC would have to show its division and `Hour::resting`'s agree,
    /// hour by hour, and hadn't for a three-hour day in two minutes (nor
    /// for a whole one in 15).
    #[kani::proof]
    #[kani::unwind(25)]
    fn a_day_rests_exactly_when_an_hour_has_readings_enough() {
        let day = any_day();
        let resting = day.resting();
        let mut rested = false;
        for hour in &day.hours {
            rested |= hour.heart_count >= RESTING_READINGS;
        }
        assert_eq!(resting.is_some(), rested);
    }

    /// What one reading a minute is worth: an hour given more than 255
    /// (its count's most) keeps adding to its sum but stops counting, so
    /// its average climbs past every reading, and past 255 wraps round:
    /// 256 readings of 255 BPM rest at 0. This proof is
    /// [`an_hours_average_is_between_its_ends`]'s claim for an hour of
    /// more readings, and is expected to fail.
    ///
    /// Nothing does that today (see the module docs); it's here so that
    /// whoever feeds a `Day` differently finds out.
    #[kani::proof]
    #[kani::should_panic]
    #[kani::unwind(25)]
    fn more_readings_than_minutes_would_misreport() {
        let mut day = Day::default();
        // An hour that has had 255 readings of any one rate, soundly.
        let rate: u8 = kani::any();
        kani::assume(rate > 0);
        day.hours[0] = Hour {
            steps: 0,
            heart_min: rate,
            heart_max: rate,
            heart_sum: u16::from(rate) * 255,
            heart_count: 255,
        };
        day.add(0, 0, rate);
        assert_eq!(day.resting(), Some(rate));
    }
}
