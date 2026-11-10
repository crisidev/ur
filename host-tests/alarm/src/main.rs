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
        let mut phone = Phone::default();
        (from..to).find_map(|now| {
            state
                .due(settings, &mut phone, now * 60, stir(now))
                .map(|due| (now, due.kind))
        })
    }

    /// Everything that rings from minute `from` to `to`, checking once a
    /// minute, with nothing ever stopping the ringing early.
    fn rings(
        settings: &mut Settings,
        phone: &mut Phone,
        state: &mut State,
        from: i64,
        to: i64,
    ) -> Vec<(i64, Kind)> {
        (from..to)
            .filter_map(|now| {
                state
                    .due(settings, phone, now * 60, false)
                    .map(|due| (now, due.kind))
            })
            .collect()
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
            if let Some(due) = state.due(&mut s, &mut Phone::default(), now * 60, true) {
                rings.push((now, due.kind));
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
        // Checked from the first check on (as after a reset).
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
            .filter_map(|now| {
                state
                    .due(&mut s, &mut Phone::default(), now * 60, false)
                    .map(|due| (now, due.kind))
            })
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

    /// An uptime (seconds since boot) unlike any local time in these
    /// tests, so mixing the two up shows.
    const BOOTED: i64 = 12_345;

    #[test]
    fn the_timer_rings_after_its_duration() {
        let s = Settings {
            timers: [25, 3],
            ..Settings::default()
        };
        let mut state = State::default();
        let now = at(MONDAY, 10, 0) * 60;
        state.start_timer(0, &s, BOOTED);
        assert_eq!(
            state.next(&s, &Phone::default(), now, BOOTED),
            Some((Kind::Timer(0), now + 25 * 60))
        );
        assert_eq!(state.remaining(0, BOOTED + 61), Some(25 * 60 - 61));
        // To the second.
        assert_eq!(state.elapsed(BOOTED + 25 * 60 - 1), None);
        assert_eq!(state.elapsed(BOOTED + 25 * 60), Some(Kind::Timer(0)));
        assert_eq!(state.timers[0], None);
        // The duration is remembered.
        assert_eq!(s.timers[0], 25);
    }

    #[test]
    fn two_timers_run_side_by_side() {
        let s = Settings {
            timers: [5, 2],
            ..Settings::default()
        };
        let mut state = State::default();
        state.start_timer(0, &s, BOOTED);
        state.start_timer(1, &s, BOOTED + 30);
        let now = at(MONDAY, 10, 0) * 60;
        assert_eq!(
            state.next(&s, &Phone::default(), now, BOOTED),
            Some((Kind::Timer(1), now + 30 + 120))
        );
        let rings: Vec<_> = (BOOTED..BOOTED + 600)
            .filter_map(|uptime| state.elapsed(uptime).map(|k| (uptime - BOOTED, k)))
            .collect();
        assert_eq!(rings, vec![(150, Kind::Timer(1)), (300, Kind::Timer(0))]);
    }

    /// A timer is a duration: the phone setting the clock, or a new time
    /// zone, moves neither when it rings nor what it says is left. (When
    /// it counted local seconds, a clock set back 8.7 days left 209 hours
    /// on a 10-minute timer, which the editor's two digits of hours
    /// couldn't show: a panic.)
    #[test]
    fn a_timer_ignores_the_clock() {
        let s = Settings {
            timers: [10, 3],
            ..Settings::default()
        };
        let mut state = State::default();
        state.start_timer(0, &s, BOOTED);
        let now = at(MONDAY, 10, 0) * 60;
        // Three zones west, a minute in: the clock face's next alarm is
        // still 9 minutes away, at the new local time.
        let west = now + 60 - 3 * 3600;
        assert_eq!(
            state.next(&s, &Phone::default(), west, BOOTED + 60),
            Some((Kind::Timer(0), west + 9 * 60))
        );
        assert_eq!(state.remaining(0, BOOTED + 60), Some(9 * 60));
        assert_eq!(state.elapsed(BOOTED + 10 * 60 - 1), None);
        assert_eq!(state.elapsed(BOOTED + 10 * 60), Some(Kind::Timer(0)));
    }

    #[test]
    fn snooze_rings_again() {
        let mut state = State::default();
        state.snooze(BOOTED);
        let rang =
            (BOOTED..BOOTED + 3600).find_map(|uptime| state.elapsed(uptime).map(|k| (uptime, k)));
        assert_eq!(rang, Some((BOOTED + SNOOZE, Kind::Snooze)));
        // Before then, the clock face shows when, in local time.
        state.snooze(BOOTED);
        let now = at(MONDAY, 7, 0) * 60;
        assert_eq!(
            state.next(&Settings::default(), &Phone::default(), now, BOOTED),
            Some((Kind::Snooze, now + SNOOZE))
        );
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
            state.next(&s, &Phone::default(), at(MONDAY, 1, 0) * 60, BOOTED),
            Some((Kind::At(0), at(MONDAY, 6, 0) * 60))
        );
        assert_eq!(
            state.next(&s, &Phone::default(), at(MONDAY, 6, 30) * 60, BOOTED),
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

    // ==========================================================================
    // Alarms at a time of day, on weekdays, and from the phone
    // ==========================================================================

    /// Gadgetbridge's weekday bits (`Alarm.ALARM_MON` = 1 … `ALARM_SUN` = 64).
    const MON: u8 = 1;
    const FRI: u8 = 16;
    const SAT: u8 = 32;
    const SUN: u8 = 64;
    const WEEKDAYS: u8 = 0b0011111;
    const WEEKENDS: u8 = SAT | SUN;

    fn phone(alarms: &[(u16, u16, u8, bool)]) -> Phone {
        let mut phone = Phone::default();
        for &(h, m, days, on) in alarms {
            assert!(phone.push(h, m, days, on));
        }
        phone
    }

    #[test]
    fn next_at_follows_the_weekdays() {
        // Any day: later today, else tomorrow.
        assert_eq!(next_at(7 * 60, 0, at(MONDAY, 6, 0)), at(MONDAY, 7, 0));
        assert_eq!(next_at(7 * 60, 0, at(MONDAY, 7, 0)), at(MONDAY + 1, 7, 0));
        // Weekdays: Friday after its time goes to Monday.
        assert_eq!(
            next_at(7 * 60, WEEKDAYS, at(MONDAY + 4, 7, 30)),
            at(MONDAY + 7, 7, 0)
        );
        // Weekends, from a Monday.
        assert_eq!(
            next_at(9 * 60, WEEKENDS, at(MONDAY, 12, 0)),
            at(MONDAY + 5, 9, 0)
        );
        // Only Mondays, just after Monday's: a whole week on.
        assert_eq!(next_at(6 * 60, MON, at(MONDAY, 6, 0)), at(MONDAY + 7, 6, 0));
        // Only Sundays, the last bit.
        assert_eq!(next_at(0, SUN, at(MONDAY, 0, 0)), at(MONDAY + 6, 0, 0));
        // Midnight to midnight, every day, around a whole week.
        for day in MONDAY..MONDAY + 7 {
            assert_eq!(
                next_at(23 * 60 + 59, 0x7F, at(day, 23, 59)),
                at(day + 1, 23, 59)
            );
        }
    }

    #[test]
    fn a_repeating_phone_alarm_rings_on_its_weekdays() {
        let mut s = Settings::default();
        let mut p = phone(&[(6, 45, WEEKDAYS | SAT, true)]);
        let mut state = State::default();
        let rang = rings(
            &mut s,
            &mut p,
            &mut state,
            at(MONDAY, 0, 0),
            at(MONDAY + 14, 0, 0),
        );
        let expected: Vec<_> = (0..14)
            .filter(|d| d % 7 != 6)
            .map(|d| (at(MONDAY + d, 6, 45), Kind::Phone(0)))
            .collect();
        assert_eq!(rang, expected);
        // Still on, as it repeats.
        assert!(p.alarms()[0].on());
    }

    #[test]
    fn a_one_off_phone_alarm_rings_once() {
        let mut s = Settings::default();
        let mut p = phone(&[(5, 30, 0, true), (6, 0, FRI, false)]);
        let mut state = State::default();
        let rang = rings(
            &mut s,
            &mut p,
            &mut state,
            at(MONDAY, 12, 0),
            at(MONDAY + 7, 0, 0),
        );
        assert_eq!(rang, vec![(at(MONDAY + 1, 5, 30), Kind::Phone(0))]);
        assert_eq!(p.alarms()[0].switch, Switch::Rang);
    }

    #[test]
    fn a_rung_one_off_stays_off_until_the_phone_changes_it() {
        let mut s = Settings::default();
        let mut p = phone(&[(5, 30, 0, true), (8, 0, WEEKDAYS, true)]);
        let mut state = State::default();
        let rang = rings(
            &mut s,
            &mut p,
            &mut state,
            at(MONDAY, 0, 0),
            at(MONDAY, 6, 0),
        );
        assert_eq!(rang, vec![(at(MONDAY, 5, 30), Kind::Phone(0))]);

        // The phone resends everything (another alarm changed): the one-off
        // still says on, but it rang.
        p.sync(phone(&[(5, 30, 0, true), (8, 15, WEEKDAYS, true)]));
        assert_eq!(p.alarms()[0].switch, Switch::Rang);
        assert_eq!(p.alarms()[1].time, 8 * 60 + 15);
        let rang = rings(
            &mut s,
            &mut p,
            &mut state,
            at(MONDAY, 6, 0),
            at(MONDAY + 2, 0, 0),
        );
        assert_eq!(
            rang,
            vec![
                (at(MONDAY, 8, 15), Kind::Phone(1)),
                (at(MONDAY + 1, 8, 15), Kind::Phone(1))
            ]
        );

        // Switched off and on again in Gadgetbridge: set again.
        p.sync(phone(&[(5, 30, 0, false), (8, 15, WEEKDAYS, true)]));
        p.sync(phone(&[(5, 30, 0, true), (8, 15, WEEKDAYS, true)]));
        assert!(p.alarms()[0].on());
        // A new time is a new alarm, too.
        rings(
            &mut s,
            &mut p,
            &mut state,
            at(MONDAY + 2, 0, 0),
            at(MONDAY + 2, 6, 0),
        );
        assert_eq!(p.alarms()[0].switch, Switch::Rang);
        p.sync(phone(&[(5, 35, 0, true), (8, 15, WEEKDAYS, true)]));
        assert!(p.alarms()[0].on());
    }

    #[test]
    fn alarms_at_the_same_minute_ring_as_one() {
        let mut s = Settings {
            at: [true, false],
            at_time: [7 * 60, 0],
            ..Settings::default()
        };
        let mut p = phone(&[(7, 0, 0, true), (7, 0, WEEKDAYS, true), (7, 5, 0, true)]);
        let mut state = State::default();
        let rang = rings(
            &mut s,
            &mut p,
            &mut state,
            at(MONDAY, 6, 0),
            at(MONDAY, 8, 0),
        );
        assert_eq!(
            rang,
            vec![
                (at(MONDAY, 7, 0), Kind::At(0)),
                (at(MONDAY, 7, 5), Kind::Phone(2))
            ]
        );
        // Every one-off that rang is off, the watch's and the phone's.
        assert!(!s.at[0]);
        assert_eq!(p.alarms()[0].switch, Switch::Rang);
        assert!(p.alarms()[1].on());
    }

    /// `due` says when it turned off a one-off phone alarm, for the UI to
    /// save the phone's alarms: whichever alarm it says rings. Saved only
    /// when the phone's was the one ringing, a one-off at the same minute
    /// as the watch's own (which rings, being listed first) stayed on in
    /// storage, and rang again the next day after a reset.
    #[test]
    fn due_says_when_a_phone_alarm_turned_off() {
        // Checks a minute at a time from 06:59 to 07:01 on Monday; returns
        // what rang at 07:00, the only minute anything is set for.
        fn at_seven(s: &mut Settings, p: &mut Phone) -> Due {
            let mut state = State::default();
            let mut due = |minute| state.due(s, p, minute * 60, false);
            assert_eq!(due(at(MONDAY, 6, 59)), None);
            let rang = due(at(MONDAY, 7, 0)).expect("something rings at 07:00");
            assert_eq!(due(at(MONDAY, 7, 1)), None);
            rang
        }
        let watch = Settings {
            at: [true, false],
            at_time: [7 * 60, 0],
            ..Settings::default()
        };

        // The phone's one-off with the watch's: the watch's rings, and
        // both turn off.
        let (mut s, mut p) = (watch, phone(&[(7, 0, 0, true)]));
        assert_eq!(
            at_seven(&mut s, &mut p),
            Due {
                kind: Kind::At(0),
                phone_rang: true
            }
        );
        assert!(!s.at[0]);
        assert_eq!(p.alarms()[0].switch, Switch::Rang);

        // The phone's repeating alarm with the watch's one-off: the
        // phone's alarms are as they were.
        let (mut s, mut p) = (watch, phone(&[(7, 0, WEEKDAYS, true)]));
        assert_eq!(
            at_seven(&mut s, &mut p),
            Due {
                kind: Kind::At(0),
                phone_rang: false
            }
        );
        assert!(!s.at[0]);
        assert!(p.alarms()[0].on());

        // The phone's repeating alarm, listed first, with its one-off: the
        // repeating one rings, the one-off turns off.
        let mut s = Settings::default();
        let mut p = phone(&[(7, 0, WEEKDAYS, true), (7, 0, 0, true)]);
        assert_eq!(
            at_seven(&mut s, &mut p),
            Due {
                kind: Kind::Phone(0),
                phone_rang: true
            }
        );
        assert_eq!(p.alarms()[1].switch, Switch::Rang);

        // A repeating phone alarm alone changes nothing: no flash write
        // every morning.
        let mut p = phone(&[(7, 0, WEEKDAYS, true)]);
        assert_eq!(
            at_seven(&mut Settings::default(), &mut p),
            Due {
                kind: Kind::Phone(0),
                phone_rang: false
            }
        );

        // Nor does the smart wake, with a one-off phone alarm later on.
        let mut s = smart();
        let mut p = phone(&[(7, 30, 0, true)]);
        assert_eq!(
            at_seven(&mut s, &mut p),
            Due {
                kind: Kind::Smart,
                phone_rang: false
            }
        );
        assert!(p.alarms()[0].on());
    }

    #[test]
    fn alarms_held_up_by_another_ringing_still_ring() {
        let mut s = Settings::default();
        let mut p = phone(&[(7, 0, 0, true), (7, 2, 0, true)]);
        let mut state = State::default();
        let mut due = |minute| {
            state
                .due(&mut s, &mut p, minute * 60, false)
                .map(|due| due.kind)
        };
        assert_eq!(due(at(MONDAY, 6, 59)), None);
        assert_eq!(due(at(MONDAY, 7, 0)), Some(Kind::Phone(0)));
        // Ringing for three minutes, during which nothing checks.
        assert_eq!(due(at(MONDAY, 7, 3)), Some(Kind::Phone(1)));
        assert_eq!(due(at(MONDAY, 7, 3)), None);
    }

    #[test]
    fn a_jump_of_the_clock_skips_rather_than_ringing_for_the_past() {
        let mut s = Settings::default();
        let mut p = phone(&[(7, 0, 0x7F, true), (9, 0, 0x7F, true)]);
        let mut state = State::default();
        let mut due = |minute| {
            state
                .due(&mut s, &mut p, minute * 60, false)
                .map(|due| due.kind)
        };
        assert_eq!(due(at(MONDAY, 6, 0)), None);
        // The phone sets the clock three hours on (a new time zone): 07:00
        // went by unrung, and 09:00 is still to come.
        assert_eq!(due(at(MONDAY, 8, 30)), None);
        assert_eq!(due(at(MONDAY, 8, 59)), None);
        assert_eq!(due(at(MONDAY, 9, 0)), Some(Kind::Phone(1)));
        // And back six hours (westwards): 07:00 comes round again, and rings.
        assert_eq!(due(at(MONDAY, 3, 0)), None);
        assert_eq!(due(at(MONDAY, 6, 59)), None);
        assert_eq!(due(at(MONDAY, 7, 0)), Some(Kind::Phone(0)));
    }

    #[test]
    fn summer_time_rings_what_it_skipped() {
        // An alarm every day at 02:30, and one at 03:10. At 02:00, summer
        // time moves the clock to 03:00 (Gadgetbridge sends the new offset
        // a few seconds later, so the watch's clock jumps at 02:00:05).
        let mut s = Settings::default();
        let mut p = phone(&[(2, 30, 0x7F, true), (3, 10, 0x7F, true)]);
        let mut state = State::default();
        let sunday = MONDAY - 1;
        let mut rang = rings(
            &mut s,
            &mut p,
            &mut state,
            at(sunday, 1, 50),
            at(sunday, 2, 1),
        );
        rang.extend(rings(
            &mut s,
            &mut p,
            &mut state,
            at(sunday, 3, 0),
            at(sunday, 3, 30),
        ));
        // 02:30 rings as the clock reaches 03:00, rather than not at all;
        // 03:10 at its time.
        assert_eq!(
            rang,
            vec![
                (at(sunday, 3, 0), Kind::Phone(0)),
                (at(sunday, 3, 10), Kind::Phone(1)),
            ]
        );
    }

    #[test]
    fn winter_time_rings_each_alarm_once() {
        // An alarm every day at 02:30, and one at 03:10. At 03:00, winter
        // time moves the clock back to 02:00 (again a few seconds late, so
        // the 03:00 minute was checked before the clock went back).
        let mut s = Settings::default();
        let mut p = phone(&[(2, 30, 0x7F, true), (3, 10, 0x7F, true)]);
        let mut state = State::default();
        let sunday = MONDAY - 1;
        let mut rang = Vec::new();
        let minutes = (at(sunday, 1, 50)..at(sunday, 3, 1))
            .chain(at(sunday, 2, 0)..at(sunday, 3, 30))
            .chain(at(MONDAY, 2, 0)..at(MONDAY, 3, 0));
        for minute in minutes {
            if let Some(due) = state.due(&mut s, &mut p, minute * 60, false) {
                rang.push((minute, due.kind));
            }
        }
        // 02:30 rings in the first of the two hours only; 03:10 once, after
        // both; and the next night's 02:30 as usual.
        assert_eq!(
            rang,
            vec![
                (at(sunday, 2, 30), Kind::Phone(0)),
                (at(sunday, 3, 10), Kind::Phone(1)),
                (at(MONDAY, 2, 30), Kind::Phone(0)),
            ]
        );
    }

    #[test]
    fn a_clock_set_back_a_little_rings_nothing_twice() {
        // The phone's clock was 20 minutes fast, and is put right after
        // 07:00 rang: it doesn't ring again at the right 07:00.
        let mut s = Settings::default();
        let mut p = phone(&[(7, 0, 0x7F, true), (7, 30, 0x7F, true)]);
        let mut state = State::default();
        let mut due = |minute| {
            state
                .due(&mut s, &mut p, minute * 60, false)
                .map(|due| due.kind)
        };
        assert_eq!(due(at(MONDAY, 6, 59)), None);
        assert_eq!(due(at(MONDAY, 7, 0)), Some(Kind::Phone(0)));
        assert_eq!(due(at(MONDAY, 7, 5)), None);
        assert_eq!(due(at(MONDAY, 6, 45)), None);
        assert_eq!(due(at(MONDAY, 7, 0)), None);
        assert_eq!(due(at(MONDAY, 7, 29)), None);
        // What comes after the minutes repeated rings as usual.
        assert_eq!(due(at(MONDAY, 7, 30)), Some(Kind::Phone(1)));
    }

    // ==========================================================================
    // Missed minutes: what rings when no check fell in an alarm's minute
    // ==========================================================================

    /// How the wearer answers a ringing alarm, in [`as_the_ui_checks`].
    #[derive(Clone, Copy)]
    enum Answer {
        /// Nobody does: it rings out after `RING_FOR`.
        Never,
        /// Dismissed, this many seconds in.
        Dismiss(i64),
        /// Snoozed, this many seconds in.
        Snooze(i64),
    }

    /// Everything that rings from local minute `from` to `to`, checked
    /// every second as `App::check_alarms` checks: durations first, then
    /// the time of day, and neither while an alarm rings. Returns the
    /// minute each started ringing in, and what.
    fn as_the_ui_checks(
        settings: &mut Settings,
        phone: &mut Phone,
        state: &mut State,
        (from, to): (i64, i64),
        answer: Answer,
    ) -> Vec<(i64, Kind)> {
        let mut rang = Vec::new();
        let mut second = from * 60;
        while second < to * 60 {
            let uptime = BOOTED + second - from * 60;
            let due = state.elapsed(uptime).or_else(|| {
                state
                    .due(settings, phone, second, false)
                    .map(|due| due.kind)
            });
            if let Some(kind) = due {
                rang.push((second.div_euclid(60), kind));
                second += match answer {
                    Answer::Never => RING_FOR * 60,
                    Answer::Dismiss(after) => after,
                    Answer::Snooze(after) => {
                        state.snooze(uptime + after);
                        after
                    }
                };
            }
            second += 1;
        }
        rang
    }

    /// T2: the smart alarm's latest time passed while another alarm rang
    /// out over it, so no check fell in that minute. It rang only if one
    /// did, so it didn't ring that morning at all, for a wearer who, not
    /// having answered the first alarm, is still asleep.
    #[test]
    fn a_smart_alarm_rings_after_another_rings_out_over_its_time() {
        let mut s = Settings {
            at: [true, false],
            at_time: [6 * 60 + 58, 0],
            ..smart()
        };
        let mut state = State::default();
        let rang = as_the_ui_checks(
            &mut s,
            &mut Phone::default(),
            &mut state,
            (at(MONDAY, 6, 50), at(MONDAY, 12, 0)),
            Answer::Never,
        );
        // 06:58 rings until 07:01, then the smart alarm at once: three
        // minutes late, rather than not at all.
        assert_eq!(
            rang,
            vec![
                (at(MONDAY, 6, 58), Kind::At(0)),
                (at(MONDAY, 7, 1), Kind::Smart)
            ]
        );
    }

    /// Alarms whose times fall while another rings queue, each ringing
    /// once the one before is done, in the order of their times; the
    /// smart alarm among them.
    #[test]
    fn alarms_overlapping_in_their_ring_windows_queue() {
        let mut s = Settings {
            at: [true, false],
            at_time: [7 * 60 + 1, 0],
            ..smart()
        };
        let mut p = phone(&[(6, 58, WEEKDAYS, true), (7, 2, 0, true)]);
        let mut state = State::default();
        let rang = as_the_ui_checks(
            &mut s,
            &mut p,
            &mut state,
            (at(MONDAY, 6, 50), at(MONDAY, 12, 0)),
            Answer::Never,
        );
        assert_eq!(
            rang,
            vec![
                (at(MONDAY, 6, 58), Kind::Phone(0)),
                (at(MONDAY, 7, 1), Kind::Smart),
                (at(MONDAY, 7, 4), Kind::At(0)),
                (at(MONDAY, 7, 7), Kind::Phone(1)),
            ]
        );
        // The one-offs that rang late turned off all the same.
        assert!(!s.at[0]);
        assert_eq!(p.alarms()[1].switch, Switch::Rang);
        // A snooze, too, rings when the alarm ringing over it is done.
        let mut s = smart();
        let mut state = State::default();
        let rang = as_the_ui_checks(
            &mut s,
            &mut phone(&[(6, 49, WEEKDAYS, true)]),
            &mut state,
            (at(MONDAY, 6, 40), at(MONDAY, 7, 30)),
            Answer::Snooze(10),
        );
        assert_eq!(
            rang,
            vec![
                (at(MONDAY, 6, 49), Kind::Phone(0)),
                // Snoozed until 06:58:20; the smart alarm at 07:00, snoozed
                // in turn until 07:09:20 (one snooze at a time: a later
                // snooze replaces the one before).
                (at(MONDAY, 6, 58), Kind::Snooze),
                (at(MONDAY, 7, 0), Kind::Smart),
                (at(MONDAY, 7, 9), Kind::Snooze),
                (at(MONDAY, 7, 18), Kind::Snooze),
                (at(MONDAY, 7, 27), Kind::Snooze),
            ]
        );
    }

    /// T2's other half: summer time skips the smart alarm's latest time.
    /// It rings as the clock reaches 03:00, as alarms at a time of day do
    /// (`summer_time_rings_what_it_skipped`).
    #[test]
    fn summer_time_rings_a_smart_alarm_it_skipped() {
        let mut s = Settings {
            smart_weekday: 2 * 60 + 30,
            smart_weekend: 2 * 60 + 30,
            ..smart()
        };
        let (mut p, mut state) = (Phone::default(), State::default());
        let sunday = MONDAY - 1;
        let mut rang = rings(
            &mut s,
            &mut p,
            &mut state,
            at(sunday, 1, 0),
            at(sunday, 2, 0),
        );
        rang.extend(rings(
            &mut s,
            &mut p,
            &mut state,
            at(sunday, 3, 0),
            at(sunday, 12, 0),
        ));
        assert_eq!(rang, vec![(at(sunday, 3, 0), Kind::Smart)]);
    }

    /// Catching up has its bound: what passed more than `CATCH_UP`
    /// minutes before the next check doesn't ring, rather than hours late.
    #[test]
    fn a_smart_alarm_missed_by_more_than_catch_up_does_not_ring_late() {
        let mut s = smart();
        let mut p = Phone::default();
        // Checked at 06:59, then not until an hour after 07:00: it rings.
        let mut state = State::default();
        assert_eq!(
            state.due(&mut s, &mut p, at(MONDAY, 6, 59) * 60, false),
            None
        );
        let late = state.due(&mut s, &mut p, (at(MONDAY, 7, 0) + CATCH_UP) * 60, false);
        assert_eq!(late.map(|due| due.kind), Some(Kind::Smart));
        // A minute later than that, it doesn't, nor later on.
        let mut state = State::default();
        assert_eq!(
            state.due(&mut s, &mut p, at(MONDAY, 6, 59) * 60, false),
            None
        );
        let rang = rings(
            &mut s,
            &mut p,
            &mut state,
            at(MONDAY, 7, 0) + CATCH_UP + 1,
            at(MONDAY, 23, 0),
        );
        assert_eq!(rang, vec![]);
        // Nor after a reset, which starts checking afresh.
        let mut state = State::default();
        assert_eq!(
            state.due(&mut s, &mut p, at(MONDAY, 7, 5) * 60, false),
            None
        );
    }

    /// T4: an alarm at the smart alarm's own minute is the same wake-up.
    /// It was rung as the other alarm, and the smart alarm, not
    /// remembered as rung, rang again as soon as that was dismissed (or,
    /// worse, snoozed).
    #[test]
    fn a_smart_alarm_and_another_at_its_minute_ring_once() {
        for answer in [Answer::Dismiss(10), Answer::Snooze(10)] {
            let mut s = Settings {
                at: [true, false],
                at_time: [7 * 60, 0],
                ..smart()
            };
            let mut p = phone(&[(7, 0, 0, true)]);
            let mut state = State::default();
            let rang = as_the_ui_checks(
                &mut s,
                &mut p,
                &mut state,
                (at(MONDAY, 6, 59), at(MONDAY, 7, 5)),
                answer,
            );
            // Under the smart alarm's name, as `next` showed it.
            assert_eq!(rang, vec![(at(MONDAY, 7, 0), Kind::Smart)]);
            // Every one-off at that minute turned off, and the phone's
            // saved as such (see `due_says_when_a_phone_alarm_turned_off`).
            assert!(!s.at[0]);
            assert_eq!(p.alarms()[0].switch, Switch::Rang);
        }
        let mut s = smart();
        let mut p = phone(&[(7, 0, 0, true)]);
        let mut state = State::default();
        assert_eq!(
            state.due(&mut s, &mut p, at(MONDAY, 6, 59) * 60, false),
            None
        );
        assert_eq!(
            state.due(&mut s, &mut p, at(MONDAY, 7, 0) * 60, false),
            Some(Due {
                kind: Kind::Smart,
                phone_rang: true
            })
        );
    }

    /// `next` names what `due` rings: at a shared minute, and after winter
    /// time, which repeats an hour whose alarms rang the first time round.
    #[test]
    fn next_names_what_due_rings() {
        let mut s = Settings {
            at: [true, false],
            at_time: [7 * 60, 0],
            ..smart()
        };
        let mut p = Phone::default();
        let mut state = State::default();
        let now = at(MONDAY, 6, 0) * 60;
        assert_eq!(state.due(&mut s, &mut p, now, false), None);
        let (kind, when) = state.next(&s, &p, now, BOOTED).expect("two alarms are on");
        let due = state
            .due(&mut s, &mut p, when, false)
            .expect("next said it rings then");
        assert_eq!((kind, when), (due.kind, at(MONDAY, 7, 0) * 60));

        // An alarm every day at 02:30. At 03:00, winter time moves the
        // clock back to 02:00.
        let mut s = Settings::default();
        let mut p = phone(&[(2, 30, 0x7F, true)]);
        let mut state = State::default();
        let sunday = MONDAY - 1;
        let first = rings(
            &mut s,
            &mut p,
            &mut state,
            at(sunday, 2, 0),
            at(sunday, 3, 1),
        );
        assert_eq!(first, vec![(at(sunday, 2, 30), Kind::Phone(0))]);
        // In the hour repeated, 02:30 doesn't ring again, and isn't shown
        // as about to: the next is Monday's.
        let now = at(sunday, 2, 0) * 60;
        assert_eq!(
            state.next(&s, &p, now, BOOTED),
            Some((Kind::Phone(0), at(MONDAY, 2, 30) * 60))
        );
        let again = rings(
            &mut s,
            &mut p,
            &mut state,
            at(sunday, 2, 0),
            at(sunday, 3, 0),
        );
        assert_eq!(again, vec![]);
    }

    #[test]
    fn next_includes_phone_alarms() {
        let s = Settings::default();
        let p = phone(&[(6, 15, FRI, true), (5, 0, 0, false)]);
        let state = State::default();
        assert_eq!(
            state.next(&s, &p, at(MONDAY, 12, 0) * 60, BOOTED),
            Some((Kind::Phone(0), at(MONDAY + 4, 6, 15) * 60))
        );
    }

    #[test]
    fn phone_alarms_round_trip_and_reject_nonsense() {
        let mut p = phone(&[
            (5, 30, 0, true),
            (23, 59, WEEKENDS, false),
            (0, 0, 0x7F, true),
        ]);
        let mut state = State::default();
        rings(
            &mut Settings::default(),
            &mut p,
            &mut state,
            at(MONDAY, 5, 0),
            at(MONDAY, 6, 0),
        );
        assert_eq!(p.alarms()[0].switch, Switch::Rang);
        assert_eq!(Phone::from_bytes(p.to_bytes()), p);
        assert_eq!(Phone::from_bytes([0xFF; Phone::BYTES]).alarms(), &[]);
        // Nonsense times aren't taken; nor more than ten alarms.
        let mut p = Phone::default();
        assert!(!p.push(24, 0, 0, true));
        assert!(!p.push(7, 60, 0, true));
        for _ in 0..PHONE_ALARMS {
            assert!(p.push(7, 0, 0, true));
        }
        assert!(!p.push(7, 0, 0, true));
        assert_eq!(p.alarms().len(), PHONE_ALARMS);
    }
}

/// Kani proofs (`cargo kani` here): what the tests above check for some
/// inputs, for every input. The settings and the phone's alarms come from
/// any bytes at all (as from corrupt or future-format storage, through
/// `from_bytes`), and the clock reads anything from 1969 to 2200: then
/// nothing panics (no `expect` fails, no arithmetic overflows), and the
/// next alarm is at most a week and a day away.
#[cfg(kani)]
mod proofs {
    use super::alarm::*;

    /// Local seconds the clock can read: from a day before 1970 (zones
    /// west of UTC) to 2200.
    fn any_now() -> i64 {
        let now: i64 = kani::any();
        kani::assume((-86_400..7_258_118_400).contains(&now));
        now
    }

    /// Seconds since boot, which timers and the snooze count in: up to
    /// ten years.
    fn any_uptime() -> i64 {
        let uptime: i64 = kani::any();
        kani::assume((0..=10 * 365 * 86_400).contains(&uptime));
        uptime
    }

    fn any_settings() -> Settings {
        Settings::from_bytes(kani::any())
    }

    fn any_phone() -> Phone {
        Phone::from_bytes(kani::any())
    }

    /// Up to two alarms from the phone, as `push` takes them (any hour,
    /// minute and days; nonsense ones refused): what CBMC can take through
    /// `next`'s loops. Ten, through `from_bytes`, took it past 10 GB; the
    /// alarms are alike, so two show what any number would.
    fn any_two_phone_alarms() -> Phone {
        let mut phone = Phone::default();
        for _ in 0..2 {
            if kani::any() {
                phone.push(kani::any(), kani::any(), kani::any(), kani::any());
            }
        }
        phone
    }

    #[kani::proof]
    #[kani::unwind(9)]
    fn next_at_is_within_eight_days() {
        let time: u16 = kani::any();
        kani::assume(time < 24 * 60);
        // As `Phone::push` keeps them: bit 7 masked off.
        let days: u8 = kani::any::<u8>() & 0x7F;
        let after = any_now() / 60;
        let at = next_at(time, days, after);
        assert!(at > after && at - after <= 8 * 24 * 60);
    }

    #[kani::proof]
    #[kani::unwind(4)]
    fn next_smart_is_within_three_days() {
        let settings = any_settings();
        let now = any_now() / 60;
        let at = next_smart(&settings, now);
        assert!(at > now && at - now <= 3 * 24 * 60);
    }

    #[kani::proof]
    #[kani::unwind(12)]
    fn settings_and_phone_alarms_from_any_bytes_are_sane() {
        let settings = any_settings();
        assert!(settings.smart_weekday < 24 * 60 && settings.smart_weekend < 24 * 60);
        assert!(settings.at_time.iter().all(|&t| t < 24 * 60));
        assert!(
            settings
                .timers
                .iter()
                .all(|&t| (1..=MAX_TIMER).contains(&t))
        );
        let phone = any_phone();
        assert!(phone.alarms().len() <= PHONE_ALARMS);
        assert!(
            phone
                .alarms()
                .iter()
                .all(|a| a.time < 24 * 60 && a.days < 0x80)
        );
    }

    #[kani::proof]
    #[kani::unwind(9)]
    fn the_next_alarm_never_panics() {
        let (settings, phone) = (any_settings(), any_two_phone_alarms());
        let mut state = State::default();
        if kani::any() {
            state.start_timer(kani::any::<bool>() as usize, &settings, any_uptime());
        }
        if kani::any() {
            state.snooze(any_uptime());
        }
        let _ = state.next(&settings, &phone, any_now(), any_uptime());
    }

    /// Whether `minute` is one [`next_at_is_within_eight_days`] and
    /// [`next_smart_is_within_three_days`] cover (`any_now() / 60`).
    fn covered(minute: i64) -> bool {
        (-DAY..7_258_118_400 / 60).contains(&minute)
    }

    const DAY: i64 = 24 * 60;

    /// Stands in for `next_at` in [`checking_what_is_due_never_panics`]:
    /// any minute [`next_at_is_within_eight_days`] says it may give,
    /// having checked that it's asked only what that proof covers.
    fn some_next_at(time: u16, days: u8, after: i64) -> i64 {
        assert!(time < 24 * 60 && days < 0x80 && covered(after));
        let at: i64 = kani::any();
        kani::assume(after < at && at <= after + 8 * DAY);
        at
    }

    /// As [`some_next_at`], for `next_smart` and
    /// [`next_smart_is_within_three_days`] (which takes any settings).
    fn some_next_smart(_: &Settings, now: i64) -> i64 {
        assert!(covered(now));
        let at: i64 = kani::any();
        kani::assume(now < at && at <= now + 3 * DAY);
        at
    }

    /// Checking what's due, twice, at any two times (the second finds the
    /// minutes the first checked, whichever way the clock went since),
    /// with any settings and any ten alarms from the phone: nothing
    /// panics or overflows.
    ///
    /// With `next_at` and `next_smart` themselves, CBMC unwinds their
    /// loops over the days a dozen times a check, and hadn't finished in
    /// 20 minutes with two phone alarms and one check. They're proven on
    /// their own above, so here stubs stand in for them (hence `stubbing`
    /// in `Cargo.toml`): any answer those proofs allow, and an assertion
    /// that `due` asks only what they cover. That leaves `due` its own
    /// arithmetic and slicing to answer for, and is done in seconds.
    ///
    /// The clock stops four days short of 2200 and starts an hour into
    /// the day before 1970, so that the minutes `due` asks about (up to
    /// an hour either side of now, and three days on for the smart
    /// alarm) are ones those proofs cover.
    #[kani::proof]
    #[kani::unwind(12)]
    #[kani::stub(next_at, some_next_at)]
    #[kani::stub(next_smart, some_next_smart)]
    fn checking_what_is_due_never_panics() {
        let (mut settings, mut phone) = (any_settings(), any_phone());
        let mut state = State::default();
        for _ in 0..2 {
            let now = any_now();
            kani::assume((3600 - 86_400..7_258_118_400 - 4 * 86_400).contains(&now));
            let _ = state.due(&mut settings, &mut phone, now, kani::any());
        }
    }
}
