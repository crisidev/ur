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
        (from..to).find_map(|now| state.due(settings, now, stir(now)).map(|k| (now, k)))
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
        let rang = first_ring(&mut s, &mut state, at(MONDAY - 1, 23, 0), at(MONDAY, 8, 0), |t| {
            t % 7 == 0
        });
        let (when, kind) = rang.expect("rings");
        assert_eq!(kind, Kind::Smart);
        assert!(when >= at(MONDAY, 6, 30) && when < at(MONDAY, 7, 0), "rang at {when}");
    }

    #[test]
    fn smart_rings_at_its_time_without_a_stir() {
        let mut s = smart();
        let mut state = State::default();
        let rang = first_ring(&mut s, &mut state, at(MONDAY, 0, 0), at(MONDAY, 8, 0), |_| false);
        assert_eq!(rang, Some((at(MONDAY, 7, 0), Kind::Smart)));
    }

    #[test]
    fn smart_rings_once_a_morning_and_again_the_next() {
        let mut s = smart();
        let mut state = State::default();
        let mut rings = Vec::new();
        for now in at(MONDAY, 0, 0)..at(MONDAY + 2, 0, 0) {
            if let Some(kind) = state.due(&mut s, now, true) {
                rings.push((now, kind));
            }
        }
        // A stir every minute: each morning at the window's start.
        assert_eq!(
            rings,
            vec![(at(MONDAY, 6, 30), Kind::Smart), (at(MONDAY + 1, 6, 30), Kind::Smart)]
        );
    }

    #[test]
    fn smart_off_never_rings() {
        let mut s = Settings { smart: false, ..smart() };
        let mut state = State::default();
        assert_eq!(first_ring(&mut s, &mut state, at(MONDAY, 0, 0), at(MONDAY + 7, 0, 0), |_| true), None);
    }

    #[test]
    fn at_rings_once_then_turns_off() {
        let mut s = Settings { at: true, at_time: 6 * 60 + 15, ..Settings::default() };
        let mut state = State::default();
        // Armed lazily on the first check (as after a reset).
        let rang = first_ring(&mut s, &mut state, at(MONDAY, 12, 0), at(MONDAY + 3, 0, 0), |_| false);
        assert_eq!(rang, Some((at(MONDAY + 1, 6, 15), Kind::At)));
        assert!(!s.at);
        assert_eq!(first_ring(&mut s, &mut state, at(MONDAY + 1, 6, 16), at(MONDAY + 3, 0, 0), |_| false), None);
    }

    #[test]
    fn the_timer_rings_after_its_duration() {
        let mut s = Settings { timer: 25, ..Settings::default() };
        let mut state = State::default();
        state.start_timer(&s, at(MONDAY, 10, 0));
        assert_eq!(state.next(&s, at(MONDAY, 10, 0)), Some((Kind::Timer, at(MONDAY, 10, 25))));
        let rang = first_ring(&mut s, &mut state, at(MONDAY, 10, 0), at(MONDAY, 12, 0), |_| false);
        assert_eq!(rang, Some((at(MONDAY, 10, 25), Kind::Timer)));
        assert_eq!(state.timer_at, None);
        // The duration is remembered.
        assert_eq!(s.timer, 25);
    }

    #[test]
    fn snooze_rings_again() {
        let mut s = Settings::default();
        let mut state = State::default();
        state.snooze(at(MONDAY, 7, 0));
        let rang = first_ring(&mut s, &mut state, at(MONDAY, 7, 0), at(MONDAY, 8, 0), |_| false);
        assert_eq!(rang, Some((at(MONDAY, 7, 0) + SNOOZE, Kind::Snooze)));
    }

    #[test]
    fn next_picks_the_soonest() {
        let s = Settings { at: true, at_time: 6 * 60, ..smart() };
        let state = State::default();
        assert_eq!(state.next(&s, at(MONDAY, 1, 0)), Some((Kind::At, at(MONDAY, 6, 0))));
        assert_eq!(state.next(&s, at(MONDAY, 6, 30)), Some((Kind::Smart, at(MONDAY, 7, 0))));
    }

    #[test]
    fn settings_round_trip_and_reject_nonsense() {
        let s = Settings { smart: true, at: true, smart_weekday: 391, smart_weekend: 545, at_time: 1, timer: 90 };
        assert_eq!(Settings::from_bytes(s.to_bytes()), s);
        let garbage = Settings::from_bytes([0xFF; Settings::BYTES]);
        assert_eq!(garbage.smart_weekday, Settings::default().smart_weekday);
        assert_eq!(garbage.timer, Settings::default().timer);
    }
}
