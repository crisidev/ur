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
