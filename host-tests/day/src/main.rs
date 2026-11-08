//! Host tests for the firmware's `day` module: a day's activity, hour by
//! hour, for the Today screen. `cargo test` runs them.

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
