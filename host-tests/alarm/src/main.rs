//! Host tests for the firmware's `alarm` module. `cargo test` runs them.

#[path = "../../../src/alarm.rs"]
#[allow(dead_code)]
mod alarm;

fn main() {
    println!("run `cargo test`");
}

#[cfg(test)]
mod tests {
    use super::alarm::*;

    const DAY: i64 = 24 * 60;
    /// Local minute of `hh:mm` on day `day` (days since 1970-01-01).
    fn at(day: i64, hh: i64, mm: i64) -> i64 {
        day * DAY + hh * 60 + mm
    }
    /// 2026-11-09, a Monday; +4 is Friday, +5 Saturday, +6 Sunday.
    const MONDAY: i64 = 20_766;

    fn smart() -> Settings {
        Settings {
            smart: true,
            smart_weekday: 7 * 60,
            smart_weekend: 9 * 60,
            ..Settings::default()
        }
    }

    /// Runs minute by minute from `from` to `to`; returns when (and what)
    /// rang first. `stir` says which minutes the wearer stirs in.
    fn first_ring(
        settings: &mut Settings,
        state: &mut State,
        from: i64,
        to: i64,
        stir: impl Fn(i64) -> bool,
    ) -> Option<(i64, Kind)> {
        // A check a minute (the firmware checks every second; minutes are
        // enough for these).
        (from..to).find_map(|now| state.due(settings, now * 60, stir(now)).map(|k| (now, k)))
    }

    #[test]
    fn the_monday_check() {
        // The test's calendar matches the firmware's (1970-01-01 a Thursday).
        assert_eq!((MONDAY + 3).rem_euclid(7), 0);
    }

    #[test]
    fn weekdays_and_weekends_use_their_own_time() {
        let s = smart();
        // Sunday evening: Monday's weekday time is next.
        assert_eq!(next_smart(&s, at(MONDAY - 1, 22, 0)), at(MONDAY, 7, 0));
        // Friday evening: Saturday's weekend time.
        assert_eq!(next_smart(&s, at(MONDAY + 4, 23, 0)), at(MONDAY + 5, 9, 0));
        // Saturday at 08:00: still Saturday's 09:00.
        assert_eq!(next_smart(&s, at(MONDAY + 5, 8, 0)), at(MONDAY + 5, 9, 0));
        // Sunday after 09:00: Monday's 07:00.
        assert_eq!(next_smart(&s, at(MONDAY + 6, 9, 30)), at(MONDAY + 7, 7, 0));
    }

    #[test]
    fn smart_rings_at_the_first_stir_in_its_window() {
        let mut s = smart();
        let mut state = State::default();
        // Stirring all night: only stirs inside the window count.
        let rang = first_ring(
            &mut s,
            &mut state,
            at(MONDAY - 1, 23, 0),
            at(MONDAY, 8, 0),
            |t| t % 7 == 0,
        );
        let (when, kind) = rang.expect("rings");
        assert_eq!(kind, Kind::Smart);
        assert!(
            when >= at(MONDAY, 6, 30) && when < at(MONDAY, 7, 0),
            "rang at {when}"
        );
    }

    #[test]
    fn smart_rings_at_its_time_without_a_stir() {
        let mut s = smart();
        let mut state = State::default();
        let rang = first_ring(
            &mut s,
            &mut state,
            at(MONDAY, 0, 0),
            at(MONDAY, 8, 0),
            |_| false,
        );
        assert_eq!(rang, Some((at(MONDAY, 7, 0), Kind::Smart)));
    }

    #[test]
    fn smart_rings_once_a_morning_and_again_the_next() {
        let mut s = smart();
        let mut state = State::default();
        let mut rings = Vec::new();
        for now in at(MONDAY, 0, 0)..at(MONDAY + 2, 0, 0) {
            if let Some(kind) = state.due(&mut s, now * 60, true) {
                rings.push((now, kind));
            }
        }
        // A stir every minute: each morning at the window's start.
        assert_eq!(
            rings,
            vec![
                (at(MONDAY, 6, 30), Kind::Smart),
                (at(MONDAY + 1, 6, 30), Kind::Smart)
            ]
        );
    }

    #[test]
    fn smart_off_never_rings() {
        let mut s = Settings {
            smart: false,
            ..smart()
        };
        let mut state = State::default();
        assert_eq!(
            first_ring(
                &mut s,
                &mut state,
                at(MONDAY, 0, 0),
                at(MONDAY + 7, 0, 0),
                |_| true
            ),
            None
        );
    }

    #[test]
    fn at_rings_once_then_turns_off() {
        let mut s = Settings {
            at: [true, false],
            at_time: [6 * 60 + 15, 0],
            ..Settings::default()
        };
        let mut state = State::default();
        // Armed lazily on the first check (as after a reset).
        let rang = first_ring(
            &mut s,
            &mut state,
            at(MONDAY, 12, 0),
            at(MONDAY + 3, 0, 0),
            |_| false,
        );
        assert_eq!(rang, Some((at(MONDAY + 1, 6, 15), Kind::At(0))));
        assert!(!s.at[0]);
        assert_eq!(
            first_ring(
                &mut s,
                &mut state,
                at(MONDAY + 1, 6, 16),
                at(MONDAY + 3, 0, 0),
                |_| false
            ),
            None
        );
    }

    #[test]
    fn two_alarms_ring_each_at_its_time() {
        let mut s = Settings {
            at: [true, true],
            at_time: [6 * 60, 6 * 60 + 20],
            ..Settings::default()
        };
        let mut state = State::default();
        let rings: Vec<_> = (at(MONDAY, 0, 0)..at(MONDAY + 2, 0, 0))
            .filter_map(|now| state.due(&mut s, now * 60, false).map(|k| (now, k)))
            .collect();
        assert_eq!(
            rings,
            vec![
                (at(MONDAY, 6, 0), Kind::At(0)),
                (at(MONDAY, 6, 20), Kind::At(1))
            ]
        );
        assert_eq!(s.at, [false, false]);
    }

    #[test]
    fn the_timer_rings_after_its_duration() {
        let mut s = Settings {
            timers: [25, 3],
            ..Settings::default()
        };
        let mut state = State::default();
        let start = at(MONDAY, 10, 0) * 60;
        state.start_timer(0, &s, start);
        assert_eq!(
            state.next(&s, start),
            Some((Kind::Timer(0), start + 25 * 60))
        );
        assert_eq!(state.remaining(0, start + 61), Some(25 * 60 - 61));
        // To the second.
        assert_eq!(state.due(&mut s, start + 25 * 60 - 1, false), None);
        assert_eq!(
            state.due(&mut s, start + 25 * 60, false),
            Some(Kind::Timer(0))
        );
        assert_eq!(state.timers[0], None);
        // The duration is remembered.
        assert_eq!(s.timers[0], 25);
    }

    #[test]
    fn two_timers_run_side_by_side() {
        let mut s = Settings {
            timers: [5, 2],
            ..Settings::default()
        };
        let mut state = State::default();
        let start = at(MONDAY, 10, 0) * 60;
        state.start_timer(0, &s, start);
        state.start_timer(1, &s, start + 30);
        assert_eq!(
            state.next(&s, start),
            Some((Kind::Timer(1), start + 30 + 120))
        );
        let rings: Vec<_> = (start..start + 600)
            .filter_map(|now| state.due(&mut s, now, false).map(|k| (now - start, k)))
            .collect();
        assert_eq!(rings, vec![(150, Kind::Timer(1)), (300, Kind::Timer(0))]);
    }

    #[test]
    fn snooze_rings_again() {
        let mut s = Settings::default();
        let mut state = State::default();
        state.snooze(at(MONDAY, 7, 0) * 60);
        let rang = first_ring(
            &mut s,
            &mut state,
            at(MONDAY, 7, 0),
            at(MONDAY, 8, 0),
            |_| false,
        );
        assert_eq!(rang, Some((at(MONDAY, 7, 0) + SNOOZE / 60, Kind::Snooze)));
    }

    #[test]
    fn next_picks_the_soonest() {
        let s = Settings {
            at: [true, false],
            at_time: [6 * 60, 0],
            ..smart()
        };
        let state = State::default();
        assert_eq!(
            state.next(&s, at(MONDAY, 1, 0) * 60),
            Some((Kind::At(0), at(MONDAY, 6, 0) * 60))
        );
        assert_eq!(
            state.next(&s, at(MONDAY, 6, 30) * 60),
            Some((Kind::Smart, at(MONDAY, 7, 0) * 60))
        );
    }

    #[test]
    fn settings_round_trip_and_reject_nonsense() {
        let s = Settings {
            smart: true,
            at: [true, false],
            smart_weekday: 391,
            smart_weekend: 545,
            at_time: [1, 1000],
            timers: [90, 7],
        };
        assert_eq!(Settings::from_bytes(s.to_bytes()), s);
        let garbage = Settings::from_bytes([0xFF; Settings::BYTES]);
        assert_eq!(garbage.smart_weekday, Settings::default().smart_weekday);
        assert_eq!(garbage.timers, Settings::default().timers);
    }
}
