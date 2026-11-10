//! Host tests for the firmware's `heart_schedule` module. `cargo test`
//! runs them.
//!
//! Most drive the schedule the way `heart::heart_task` does, through a
//! simulated wrist (see [`simulate`]): the accelerometer polls every 2 s,
//! sessions start once due and calm, and each one's outcome and length
//! come from the scenario.

#[path = "../../../src/heart_schedule.rs"]
#[allow(dead_code)]
mod heart_schedule;

fn main() {
    println!("run `cargo test`");
}

#[cfg(test)]
mod tests {
    use super::heart_schedule::*;

    const MINUTE: u32 = 60;
    const HOUR: u32 = 60 * MINUTE;
    /// How often the accelerometer polls, as in `activity`.
    const POLL: u32 = 2;

    /// The simulated wrist at one poll.
    #[derive(Clone, Copy)]
    struct Moment {
        /// Whether this poll saw steps.
        active: bool,
        /// Whether it saw vigorous movement without steps (gestures at a
        /// desk, say): active for the schedule, but not stepping.
        fidget: bool,
        /// Whether the wrist is calm enough to measure through.
        calm: bool,
        asleep: bool,
        /// How a session started now would end, and how long it'd take.
        outcome: Outcome,
        seconds: u32,
    }

    /// A still, awake wrist that reads 60 BPM in 13 s, like the field log's
    /// typical session.
    fn resting(_: u32) -> Moment {
        Moment {
            active: false,
            fidget: false,
            calm: true,
            asleep: false,
            outcome: Outcome::Reading(60),
            seconds: 13,
        }
    }

    #[derive(Debug, Clone, Copy)]
    struct Session {
        start: u32,
        end: u32,
        why: Why,
    }

    /// Runs `schedule` through `world` for `until` seconds, as the heart
    /// task would: background sessions only, each started once due and
    /// the wrist has been calm for [`START_CALM_POLLS`] polls. The
    /// accelerometer's counts start afresh, as at boot, except that the
    /// wearer is taken to have taken no steps for a while.
    fn simulate(
        schedule: &mut Schedule,
        from: u32,
        until: u32,
        world: impl Fn(u32) -> Moment,
    ) -> Vec<Session> {
        let mut sessions = Vec::new();
        let mut polls = 0u8;
        let mut calm = 0;
        let mut stepless = RESTING_STEPLESS_POLLS;
        let mut t = from;
        while t < until {
            let moment = world(t);
            if moment.active || moment.fidget {
                polls = polls.saturating_add(1);
            }
            let run = |n: u8, counts: bool| if counts { n.saturating_add(1) } else { 0 };
            calm = run(calm, moment.calm);
            stepless = run(stepless, !moment.active);
            let (due, why) = schedule.due(polls, moment.asleep);
            if t >= due && calm >= START_CALM_POLLS {
                let end = t + moment.seconds;
                let before = Before {
                    active_polls: polls,
                    stepless_polls: stepless,
                    asleep: moment.asleep,
                };
                schedule.ended(end, moment.seconds, moment.outcome, true, before);
                polls = 0;
                sessions.push(Session { start: t, end, why });
                t = end;
            }
            t += POLL;
        }
        sessions
    }

    /// The wait before each session after the first, from the end of the
    /// one before.
    fn gaps(sessions: &[Session]) -> Vec<u32> {
        sessions.windows(2).map(|w| w[1].start - w[0].end).collect()
    }

    /// LED seconds spent by sessions starting in `from..to`.
    fn led(sessions: &[Session], from: u32, to: u32) -> u32 {
        sessions
            .iter()
            .filter(|s| (from..to).contains(&s.start))
            .map(|s| s.end - s.start)
            .sum()
    }

    /// Rounds a simulated wait to the poll it fell on.
    fn about(wait: u32, expected: u32) -> bool {
        wait.abs_diff(expected) < POLL
    }

    /// A session's lead-up: awake, with `active_polls` since the last, but
    /// no steps for long.
    fn awake(active_polls: u8) -> Before {
        Before {
            active_polls,
            stepless_polls: u8::MAX,
            asleep: false,
        }
    }

    /// Asleep, likewise.
    fn sleeping() -> Before {
        Before {
            asleep: true,
            ..awake(0)
        }
    }

    #[test]
    fn the_state_stays_small() {
        // It lives in the heart task's static future, where RAM is tight.
        // (20 bytes until counting interruptions took one more, and
        // alignment three more; 28 with when the last reading came, 32
        // with the LED experiment's rounds.)
        assert!(size_of::<Schedule>() <= 32);
    }

    #[test]
    fn the_first_session_is_right_away() {
        let schedule = Schedule::new();
        assert_eq!(schedule.due(0, false), (0, Why::First));
        assert_eq!(schedule.due(200, true), (0, Why::First));
    }

    #[test]
    fn rest_backs_off_to_the_cap() {
        let mut schedule = Schedule::new();
        let sessions = simulate(&mut schedule, 0, 6 * HOUR, resting);
        let waits = gaps(&sessions);
        // Doubling from the minimum, then held at the cap.
        for (wait, expected) in waits.iter().zip([4, 8, 16, 20, 20, 20]) {
            assert!(about(*wait, expected * MINUTE), "{waits:?}");
        }
        assert!(waits.windows(2).all(|w| w[1] + POLL >= w[0]), "{waits:?}");
        assert!(waits[3..].iter().all(|&w| about(w, MAX_INTERVAL)));
        assert!(sessions[1..].iter().all(|s| s.why == Why::Rest));
        // About three sessions an hour once settled: ~40 s of LED, where
        // the fixed 10-minute schedule spent ~77.
        let settled = led(&sessions, 2 * HOUR, 6 * HOUR) / 4;
        assert!((26..=52).contains(&settled), "{settled} s an hour");
        assert_eq!(schedule.resting(), (60, 0));
    }

    #[test]
    fn movement_resets_the_back_off() {
        let mut schedule = Schedule::new();
        let mut sessions = simulate(&mut schedule, 0, 2 * HOUR, resting);
        let before = sessions.len();
        // A short walk: 30 s of steps, then sitting down again.
        let walk = 2 * HOUR + 5 * MINUTE;
        sessions.extend(simulate(&mut schedule, 2 * HOUR, 4 * HOUR, |t| Moment {
            active: (walk..walk + 30).contains(&t),
            calm: !(walk..walk + 30).contains(&t),
            ..resting(t)
        }));
        let after = &sessions[before..];
        // Measured at the first calm moment after the walk (the last
        // session was well over the minimum interval before), then backing
        // off again from the minimum.
        assert_eq!(after[0].why, Why::Moved);
        let settled = walk + 30 + u32::from(START_CALM_POLLS) * POLL;
        assert!(
            after[0].start >= walk + 30 && after[0].start < settled,
            "{after:?}"
        );
        let waits = gaps(after);
        for (wait, expected) in waits.iter().zip([4, 8, 16, 20]) {
            assert!(about(*wait, expected * MINUTE), "{waits:?}");
        }
    }

    #[test]
    fn movement_soon_after_a_session_waits_for_the_minimum_and_calm() {
        let mut schedule = Schedule::new();
        let sessions = simulate(&mut schedule, 0, HOUR, resting);
        let last = sessions.last().expect("an hour at rest measures").end;
        // Walking from right after the session, for five minutes.
        let walking = |t: u32| (last..last + 5 * MINUTE).contains(&t);
        let after = simulate(&mut schedule, last + POLL, last + HOUR, |t| Moment {
            active: walking(t),
            calm: !walking(t),
            ..resting(t)
        });
        // Due after two minutes, but the wrist is busy until five.
        assert_eq!(after[0].why, Why::Moved);
        assert!(after[0].start >= last + 5 * MINUTE);
        let settled = last + 5 * MINUTE + u32::from(START_CALM_POLLS) * POLL;
        assert!(after[0].start < settled, "{after:?}");

        // And with the walk already over by then, at the minimum.
        let mut schedule = Schedule::new();
        let sessions = simulate(&mut schedule, 0, HOUR, resting);
        let last = sessions.last().expect("an hour at rest measures").end;
        let walking = |t: u32| (last..last + 30).contains(&t);
        let after = simulate(&mut schedule, last + POLL, last + HOUR, |t| Moment {
            active: walking(t),
            calm: !walking(t),
            ..resting(t)
        });
        assert!(about(after[0].start - last, MIN_INTERVAL), "{after:?}");
    }

    #[test]
    fn a_flick_of_the_wrist_is_not_movement() {
        let mut schedule = Schedule::new();
        simulate(&mut schedule, 0, 2 * HOUR, resting);
        let (at, why) = schedule.due(MOVED_POLLS - 1, false);
        assert_eq!(why, Why::Rest);
        assert_eq!(
            schedule.due(MOVED_POLLS, false),
            (at - MAX_INTERVAL + MIN_INTERVAL, Why::Moved)
        );
    }

    #[test]
    fn movement_asleep_is_turning_over() {
        let mut schedule = Schedule::new();
        let asleep = |t: u32| Moment {
            asleep: true,
            outcome: Outcome::Reading(50),
            ..resting(t)
        };
        simulate(&mut schedule, 0, 2 * HOUR, asleep);
        assert_eq!(schedule.due(50, true).1, Why::Asleep);
        // Readings asleep teach the asleep estimate, not the awake one.
        assert_eq!(schedule.resting(), (0, 50));
        // Tossing and turning all night still measures at the asleep
        // interval, not the movement one.
        let sessions = simulate(&mut schedule, 2 * HOUR, 8 * HOUR, |t| Moment {
            active: t % 60 < 15,
            calm: t % 60 >= 15,
            ..asleep(t)
        });
        assert!(
            gaps(&sessions)
                .iter()
                .all(|&w| (ASLEEP_INTERVAL..ASLEEP_INTERVAL + 60).contains(&w)),
            "{:?}",
            gaps(&sessions)
        );
    }

    /// Asleep, readings come every [`ASLEEP_INTERVAL`] (for sleep staging),
    /// within the LED budget; falling asleep mid back-off doesn't wait out
    /// the long interval, and waking returns to the back-off.
    #[test]
    fn asleep_measures_every_five_minutes() {
        let mut schedule = Schedule::new();
        simulate(&mut schedule, 0, 2 * HOUR, resting);
        assert_eq!(schedule.due(0, false).1, Why::Rest);
        let last_awake = schedule.due(0, false).0 - MAX_INTERVAL;
        // Asleep now: due 5 minutes after the last reading, not 20.
        assert_eq!(
            schedule.due(0, true),
            (last_awake + ASLEEP_INTERVAL, Why::Asleep)
        );
        let night = |t: u32| Moment {
            asleep: true,
            outcome: Outcome::Reading(50),
            ..resting(t)
        };
        let sessions = simulate(&mut schedule, 2 * HOUR, 10 * HOUR, night);
        assert!(
            gaps(&sessions).iter().all(|&w| about(w, ASLEEP_INTERVAL)),
            "{:?}",
            gaps(&sessions)
        );
        // About 11 sessions of 13 s an hour: inside the budget.
        let per_hour = led(&sessions, 3 * HOUR, 9 * HOUR) / 6;
        assert!((130..=160).contains(&per_hour), "{per_hour} s/h");
        // Awake again: back to backing off.
        let morning = simulate(&mut schedule, 10 * HOUR, 12 * HOUR, resting);
        assert!(gaps(&morning).iter().any(|&w| about(w, MAX_INTERVAL)));
    }

    #[test]
    fn elevated_readings_keep_the_interval_short() {
        let mut schedule = Schedule::new();
        simulate(&mut schedule, 0, 3 * HOUR, resting);
        assert_eq!(schedule.resting().0, 60);
        // Sitting still but with the heart rate up (after a run, say),
        // coming down 1 BPM a minute from 105.
        let up = 3 * HOUR;
        let sessions = simulate(&mut schedule, up, 5 * HOUR, |t| {
            let bpm = 105 - ((t - up) / MINUTE).min(45) as u8;
            Moment {
                outcome: Outcome::Reading(bpm),
                ..resting(t)
            }
        });
        // The first is at the old resting interval: nothing said to look
        // sooner. Then every few minutes while the readings are 15 over
        // resting, then backing off again.
        let elevated: Vec<_> = sessions.iter().filter(|s| s.why == Why::Elevated).collect();
        assert!(elevated.len() >= 3, "{sessions:?}");
        let waits = gaps(&sessions);
        assert!(
            waits
                .iter()
                .take(elevated.len())
                .all(|&w| about(w, ELEVATED_INTERVAL)),
            "{waits:?}"
        );
        assert!(sessions.last().expect("measured").why == Why::Rest);
        // The resting estimate rose by a beat for each elevated reading,
        // and came back down after.
        assert!(schedule.resting().0 <= 62, "{:?}", schedule.resting());
    }

    #[test]
    fn asleep_readings_compare_against_the_asleep_estimate() {
        let mut schedule = Schedule::new();
        simulate(&mut schedule, 0, 3 * HOUR, resting);
        let asleep = |bpm| {
            move |t| Moment {
                asleep: true,
                outcome: Outcome::Reading(bpm),
                ..resting(t)
            }
        };
        // The first nights' readings, before an asleep estimate exists, go
        // against the awake one: sleep only lowers the heart rate.
        simulate(&mut schedule, 3 * HOUR, 9 * HOUR, asleep(48));
        assert_eq!(schedule.resting(), (60, 48));
        // 66 is ordinary awake, but well up asleep (a nightmare, say).
        let mut woken = schedule.clone();
        schedule.ended(9 * HOUR, 13, Outcome::Reading(66), true, sleeping());
        assert_eq!(schedule.due(0, true).1, Why::Elevated);
        woken.ended(9 * HOUR, 13, Outcome::Reading(66), true, awake(0));
        assert_eq!(woken.due(0, false).1, Why::Rest);
    }

    #[test]
    fn the_resting_estimate_tracks_the_median_and_falls_quickly() {
        let mut schedule = Schedule::new();
        // Booted mid-walk: the first reading is far above rest.
        let mut t = 0;
        let mut read = |schedule: &mut Schedule, bpm| {
            t += 20 * MINUTE;
            schedule.ended(t, 13, Outcome::Reading(bpm), true, awake(0));
        };
        read(&mut schedule, 100);
        // It's down near 60 within a few resting readings.
        for _ in 0..12 {
            read(&mut schedule, 60);
        }
        assert!(schedule.resting().0 <= 62, "{:?}", schedule.resting());
        // A run of scattered resting readings: it settles near their median,
        // and an occasional high one barely moves it.
        for bpm in [58, 61, 59, 60, 62, 85, 60, 59, 61, 60, 90, 60, 58, 61, 60] {
            read(&mut schedule, bpm);
        }
        assert!(
            (58..=62).contains(&schedule.resting().0),
            "{:?}",
            schedule.resting()
        );
    }

    #[test]
    fn readings_soon_after_steps_dont_teach_the_resting_estimate() {
        let mut schedule = Schedule::new();
        schedule.ended(100, 13, Outcome::Reading(60), true, awake(0));
        schedule.ended(400, 13, Outcome::Reading(61), true, awake(0));
        assert_eq!(schedule.resting().0, 61);
        // Two minutes after a walk.
        let walked = Before {
            active_polls: MOVED_POLLS,
            stepless_polls: 60,
            asleep: false,
        };
        schedule.ended(700, 13, Outcome::Reading(110), true, walked);
        assert_eq!(schedule.resting().0, 61);
        assert_eq!(schedule.due(0, false).1, Why::Elevated);
    }

    /// At a desk, the hands move vigorously every minute or two, but the
    /// wearer is at rest: readings there teach the resting estimate (which
    /// stayed empty for a whole afternoon when any movement since the last
    /// session disqualified them), whether looked at or not.
    #[test]
    fn readings_at_a_desk_are_resting() {
        let mut schedule = Schedule::new();
        let desk = |t: u32| Moment {
            fidget: t % 120 < 10,
            calm: t % 120 >= 10,
            outcome: Outcome::Reading(66),
            ..resting(t)
        };
        simulate(&mut schedule, 0, 2 * HOUR, desk);
        assert_eq!(schedule.resting(), (66, 0));
        // A reading while looking at the watch counts too.
        let mut schedule = Schedule::new();
        schedule.ended(100, 30, Outcome::Reading(64), false, awake(MOVED_POLLS));
        schedule.ended(400, 30, Outcome::Reading(66), false, awake(MOVED_POLLS));
        assert_eq!(schedule.resting(), (65, 0));
    }

    /// One reading doesn't make an estimate: it starts once two agree.
    /// Otherwise a single misreading (a field log's desk afternoon had a 42
    /// against a Garmin's 67) would make every ordinary reading after it
    /// count as elevated for ~20 readings, measuring every four minutes.
    #[test]
    fn the_resting_estimate_starts_from_two_agreeing_readings() {
        let mut schedule = Schedule::new();
        let mut t = 0;
        let mut read = |schedule: &mut Schedule, bpm| {
            t += 10 * MINUTE;
            schedule.ended(t, 13, Outcome::Reading(bpm), true, awake(0));
        };
        read(&mut schedule, 42);
        assert_eq!(schedule.resting(), (0, 0));
        read(&mut schedule, 67);
        assert_eq!(schedule.resting(), (0, 0));
        assert_eq!(schedule.due(0, false).1, Why::Rest);
        read(&mut schedule, 64);
        assert_eq!(schedule.resting(), (66, 0));
        // And from then on as before.
        read(&mut schedule, 70);
        assert_eq!(schedule.resting(), (67, 0));
    }

    /// A session waits for the wrist to stay calm for a few seconds: at a
    /// desk, a hand rests a moment between movements, and a session started
    /// then is spent on a wrist that is soon moving again.
    #[test]
    fn sessions_wait_for_lasting_calm() {
        let mut schedule = Schedule::new();
        // Calm for one poll in three, all afternoon: never starts.
        let fidgety = |t: u32| Moment {
            calm: t.is_multiple_of(6),
            ..resting(t)
        };
        assert!(simulate(&mut schedule, 0, 4 * HOUR, fidgety).is_empty());
        // Calm for six seconds in thirty: starts at the third calm poll.
        let settling = |t: u32| Moment {
            calm: t % 30 >= 24,
            ..resting(t)
        };
        let sessions = simulate(&mut schedule, 0, HOUR, settling);
        assert_eq!(sessions[0].start, 28);
    }

    #[test]
    fn busy_sessions_give_up_once_their_first_windows_are_in() {
        // Not before the first windows have had their chance.
        assert!(!give_up(GIVE_UP_AFTER - 1, false, 11, 11));
        // A wrist busy at most checks, with nothing accepted.
        assert!(give_up(GIVE_UP_AFTER, false, 7, 12));
        // But not once a window has passed, nor while mostly calm.
        assert!(!give_up(GIVE_UP_AFTER, true, 12, 12));
        assert!(!give_up(GIVE_UP_AFTER, false, 6, 12));
        assert!(!give_up(29, false, 14, 29));
        // Saturated counts don't wrap.
        assert!(give_up(300, false, u8::MAX, u8::MAX - 1));
    }

    #[test]
    fn restless_sessions_stop_before_their_first_windows_are_in() {
        assert!(interrupt(0, false, true, false));
        assert!(interrupt(INTERRUPT_BEFORE - 1, false, true, false));
        // Not once it's had its first windows' chance, nor after one
        // passed, nor on a wrist that keeps still enough, nor asleep.
        assert!(!interrupt(INTERRUPT_BEFORE, false, true, false));
        assert!(!interrupt(3, true, true, false));
        assert!(!interrupt(3, false, false, false));
        assert!(!interrupt(3, false, true, true));
    }

    /// An interrupted session doesn't count: it's still due, and starts
    /// again at the next still moment, until `INTERRUPTIONS` in a row,
    /// which back off like any failure. Their LED time is spent, though.
    #[test]
    fn interrupted_sessions_try_again_until_too_many() {
        let mut schedule = Schedule::new();
        schedule.ended(100, 13, Outcome::Reading(60), true, awake(0));
        let due = schedule.due(0, false);
        let budget = schedule.budget(due.0);
        let mut t = due.0;
        for _ in 1..INTERRUPTIONS {
            schedule.ended(t + 4, 4, Outcome::Interrupted, true, awake(0));
            assert!(schedule.interrupted());
            assert_eq!(schedule.due(0, false), due);
            t += 30;
        }
        assert!(schedule.budget(t) < budget);
        schedule.ended(t + 4, 4, Outcome::Interrupted, true, awake(0));
        assert!(!schedule.interrupted());
        assert_eq!(
            schedule.due(0, false),
            (t + 4 + FAILED_INTERVAL, Why::Retry)
        );
        // A counted session starts the count again.
        let t = t + 4 + FAILED_INTERVAL;
        schedule.ended(t + 4, 4, Outcome::Interrupted, true, awake(0));
        schedule.ended(t + 60, 13, Outcome::Reading(60), true, awake(0));
        assert!(!schedule.interrupted());
        schedule.ended(t + 400, 4, Outcome::Interrupted, true, awake(0));
        assert!(schedule.interrupted());
    }

    /// A desk where the wrist keeps still for 8 s in every 40, just long
    /// enough to start a session but not to finish one: tries come three
    /// at a time, a still moment apart, then back off as failures do, so
    /// the LED stays well within the budget.
    #[test]
    fn interruptions_dont_spin() {
        let mut schedule = Schedule::new();
        let sessions = simulate(&mut schedule, 0, 4 * HOUR, |t| Moment {
            calm: t % 40 < 8,
            outcome: Outcome::Interrupted,
            seconds: 4,
            ..resting(t)
        });
        let waits = gaps(&sessions);
        let n = usize::from(INTERRUPTIONS);
        for (i, wait) in waits.iter().enumerate() {
            if (i + 1) % n == 0 {
                assert!(*wait >= FAILED_INTERVAL, "{waits:?}");
            } else {
                assert!(*wait < 60, "{waits:?}");
            }
        }
        // An hour without a reading, they come every ten minutes rather
        // than backing off (see `OVERDUE`), still well within the budget.
        assert!(
            led(&sessions, HOUR, 2 * HOUR) < LED_BUDGET / 2,
            "{sessions:?}"
        );
        assert!(sessions.iter().any(|s| s.why == Why::Overdue));
    }

    /// Giving up counts as a session without a reading: the failure
    /// back-off applies, from when it gave up, until half an hour after
    /// the last reading.
    #[test]
    fn giving_up_backs_off_like_any_failure() {
        let mut schedule = Schedule::new();
        schedule.ended(100, 13, Outcome::Reading(60), true, awake(0));
        let mut t = 400;
        for retry in [5, 10] {
            schedule.ended(
                t + GIVE_UP_AFTER,
                GIVE_UP_AFTER,
                Outcome::NoReading,
                true,
                awake(0),
            );
            let (due, why) = schedule.due(0, false);
            assert_eq!((due, why), (t + GIVE_UP_AFTER + retry * MINUTE, Why::Retry));
            t = due;
        }
        // 20 minutes would pass the half hour: the overdue retry's 10.
        schedule.ended(t + GIVE_UP_AFTER, 12, Outcome::NoReading, true, awake(0));
        assert_eq!(
            schedule.due(0, false),
            (t + GIVE_UP_AFTER + OVERDUE_RETRY, Why::Overdue)
        );
    }

    #[test]
    fn failures_dont_spin() {
        // A wrist that never gives a reading (or a watch on a desk), calm
        // enough to try, and moving between tries: the worst case for
        // retrying.
        let mut schedule = Schedule::new();
        let sessions = simulate(&mut schedule, 0, 8 * HOUR, |t| Moment {
            active: t % 60 < 15,
            calm: t % 60 >= 15,
            outcome: Outcome::NoReading,
            seconds: 30,
            ..resting(t)
        });
        let waits = gaps(&sessions);
        // At least five minutes between tries, doubling; half an hour
        // without a reading, every ten minutes (see `OVERDUE`).
        assert!(waits.iter().all(|&w| w >= FAILED_INTERVAL), "{waits:?}");
        let overdue = sessions
            .iter()
            .position(|s| s.start >= OVERDUE)
            .expect("overdue");
        assert!(sessions[1..overdue].iter().all(|s| s.why == Why::Retry));
        assert!(sessions[overdue].start < OVERDUE + MINUTE, "{sessions:?}");
        assert!(sessions[overdue..].iter().all(|s| s.why == Why::Overdue));
        assert!(
            waits[overdue..].iter().all(|&w| w < OVERDUE_RETRY + MINUTE),
            "{waits:?}"
        );

        // Off the wrist and still (a nightstand), up to an hour: there's no
        // reading to be had, overdue or not.
        let mut schedule = Schedule::new();
        let sessions = simulate(&mut schedule, 0, 12 * HOUR, |t| Moment {
            outcome: Outcome::OffWrist,
            seconds: 4,
            ..resting(t)
        });
        let waits = gaps(&sessions);
        for (wait, expected) in waits.iter().zip([5, 10, 20, 40, 60, 60, 60]) {
            assert!(about(*wait, expected * MINUTE), "{waits:?}");
        }

        // On something skin-like, or a wrist that won't read, the back-off
        // gives way to every ten minutes half an hour in, before it gets to
        // 20 minutes.
        let mut schedule = Schedule::new();
        let sessions = simulate(&mut schedule, 0, 12 * HOUR, |t| Moment {
            outcome: Outcome::NoReading,
            seconds: 30,
            ..resting(t)
        });
        let waits = gaps(&sessions);
        for (wait, expected) in waits.iter().zip([5, 10]) {
            assert!(about(*wait, expected * MINUTE), "{waits:?}");
        }
        assert!(about(sessions[3].start, OVERDUE), "{sessions:?}");
        assert!(waits[3..].iter().all(|&w| about(w, OVERDUE_RETRY)));
    }

    /// Asleep, a reading is overdue after the same half hour: a night's
    /// failures (a restless spell, a faint pulse) back off 5 and 10
    /// minutes, then retry every 10 rather than waiting 20.
    #[test]
    fn a_night_without_readings_is_overdue_after_half_an_hour_too() {
        let mut schedule = Schedule::new();
        schedule.ended(100, 10, Outcome::Reading(52), true, sleeping());
        assert_eq!(schedule.due(0, true), (100 + ASLEEP_INTERVAL, Why::Asleep));
        let mut t = 100 + ASLEEP_INTERVAL;
        let mut due = Vec::new();
        for _ in 0..4 {
            schedule.ended(t + 30, 30, Outcome::NoReading, true, sleeping());
            let next = schedule.due(0, true);
            due.push(next);
            t = next.0;
        }
        assert_eq!(
            due,
            [
                (430 + 5 * MINUTE, Why::Retry),
                (760 + 10 * MINUTE, Why::Retry),
                (1390 + 10 * MINUTE, Why::Overdue),
                (2020 + 10 * MINUTE, Why::Overdue),
            ]
        );
        assert!(due[2].0 >= 100 + OVERDUE);
    }

    /// Lying still awake for an hour without a reading (a field log,
    /// 2026-11-09): calm failures retry every ten minutes, not 20, 40, 60,
    /// and by the time an hour has gone by, a reading is overdue, so they
    /// go on doing so.
    #[test]
    fn calm_failures_retry_sooner_for_a_while() {
        let mut schedule = Schedule::new();
        let sessions = simulate(&mut schedule, 0, 6 * HOUR, |t| Moment {
            outcome: Outcome::CalmNoReading,
            seconds: 30,
            ..resting(t)
        });
        let waits = gaps(&sessions);
        let expected = [5, 10, 10, 10, 10, 10, 10, 10, 10];
        for (wait, expected) in waits.iter().zip(expected) {
            assert!(about(*wait, expected * MINUTE), "{waits:?}");
        }
        // The hour after the first failure takes 7 sessions, not 4, all of
        // them within the LED budget.
        let first_hour = sessions.iter().filter(|s| s.start < HOUR).count();
        assert_eq!(first_hour, 7, "{sessions:?}");
        assert!(sessions[1..].iter().all(|s| s.why == Why::Retry));

        // A busy failure in between starts the count again, from wherever
        // the ordinary back-off has got to.
        let mut schedule = Schedule::new();
        schedule.ended(100, 13, Outcome::Reading(60), true, awake(0));
        for t in [400, 700] {
            schedule.ended(t, 12, Outcome::NoReading, true, awake(0));
        }
        assert_eq!(schedule.due(0, false), (700 + 10 * MINUTE, Why::Retry));
        // (Its 20 minutes give way to the overdue retry at half an hour.)
        schedule.ended(1300, 30, Outcome::NoReading, true, awake(0));
        assert_eq!(schedule.due(0, false), (100 + OVERDUE, Why::Overdue));
        schedule.ended(1900, 30, Outcome::CalmNoReading, true, awake(0));
        assert_eq!(schedule.due(0, false), (1900 + 10 * MINUTE, Why::Retry));
        // And a reading ends it.
        schedule.ended(3100, 13, Outcome::Reading(60), true, awake(0));
        schedule.ended(3400, 30, Outcome::CalmNoReading, true, awake(0));
        assert_eq!(schedule.due(0, false), (3400 + 5 * MINUTE, Why::Retry));
    }

    #[test]
    fn calm_throughout_allows_a_twitch() {
        assert!(calm_throughout(0, 30));
        assert!(calm_throughout(7, 30));
        assert!(!calm_throughout(8, 30));
        assert!(calm_throughout(0, 0));
        assert!(calm_throughout(u8::MAX / 4, u8::MAX));
        assert!(!calm_throughout(u8::MAX, u8::MAX));
    }

    #[test]
    fn brighter_seconds_cost_more_budget() {
        assert_eq!(budget_seconds(0, 30_000), 30);
        assert_eq!(budget_seconds(0, 29_001), 30);
        assert_eq!(budget_seconds(1, 10_000), 16);
        assert_eq!(budget_seconds(2, 15_000), 36);
        assert_eq!(budget_seconds(3, 15_000), 48);
        assert_eq!(budget_seconds(0, 0), 0);
    }

    #[test]
    fn brighter_goes_up_two_drives_as_far_as_headroom_allows() {
        let plenty = LED_BUDGET;
        // A wrist's usual level at 12.5 mA ×2: 30 mA, ×2.4.
        assert_eq!(brighter(0, 0, 2500, plenty), Some(2));
        assert_eq!(brighter(0, 0, 3413, plenty), Some(2));
        // Too bright for 30 mA, not for 20.
        assert_eq!(brighter(0, 0, 3500, plenty), Some(1));
        assert_eq!(brighter(0, 0, 5200, plenty), None);
        // From 30 mA, only 40 is left.
        assert_eq!(brighter(2, 1, 2500, plenty), Some(3));
        assert_eq!(brighter(2, 1, 6200, plenty), None);
        assert_eq!(brighter(1, 1, 2000, plenty), Some(3));
        assert_eq!(brighter(3, 1, 100, plenty), None);
        // At most twice a session.
        assert_eq!(brighter(1, BRIGHTER_STEPS, 1000, plenty), None);
    }

    #[test]
    fn brighter_leaves_the_next_session_its_reserve() {
        // 15 s at 30 mA is 36 s of budget, at 20 mA 24.
        let needed = |drive| budget_seconds(drive, BRIGHTER_FOR * 1000) + SESSION_RESERVE;
        assert_eq!(brighter(0, 0, 2000, needed(2)), Some(2));
        assert_eq!(brighter(0, 0, 2000, needed(2) - 1), Some(1));
        assert_eq!(brighter(0, 0, 2000, needed(1) - 1), None);
        assert_eq!(brighter(0, 0, 2000, 0), None);
    }

    #[test]
    fn a_reading_ends_the_failure_back_off() {
        let mut schedule = Schedule::new();
        for t in [0, 300] {
            schedule.ended(t + 30, 30, Outcome::NoReading, true, awake(0));
        }
        assert_eq!(schedule.due(0, false), (330 + 10 * MINUTE, Why::Retry));
        schedule.ended(2200, 13, Outcome::Reading(60), true, awake(0));
        // The resting back-off carries on from the minimum.
        assert_eq!(schedule.due(0, false), (2200 + 2 * MIN_INTERVAL, Why::Rest));
    }

    #[test]
    fn moving_a_watch_that_was_off_the_wrist_retries_sooner() {
        let mut schedule = Schedule::new();
        let mut t = 0;
        for _ in 0..5 {
            let (due, _) = schedule.due(0, false);
            t = t.max(due) + 10;
            schedule.ended(t, 10, Outcome::OffWrist, true, awake(0));
        }
        assert_eq!(
            schedule.due(0, false),
            (t + MAX_FAILED_INTERVAL, Why::Retry)
        );
        // Picked up (to be put on, most likely).
        assert_eq!(
            schedule.due(MOVED_POLLS, false),
            (t + FAILED_INTERVAL, Why::Retry)
        );
        // A wrist that gave no reading, though, is only held to the
        // resting interval (with a reading under half an hour ago, so not
        // overdue).
        schedule.ended(t + 100, 13, Outcome::Reading(60), true, awake(0));
        for after in [150, 300, 450, 600] {
            schedule.ended(t + after, 30, Outcome::NoReading, true, awake(0));
        }
        assert_eq!(
            schedule.due(MOVED_POLLS, false),
            (t + 600 + MAX_INTERVAL, Why::Retry)
        );
    }

    #[test]
    fn the_led_budget_caps_a_bad_day() {
        // The worst wrist: moving between sessions, and every reading far
        // above resting, each taking the full 30 s. Unbudgeted, that's a
        // session every two minutes: 15 minutes of LED an hour.
        let mut schedule = Schedule::new();
        schedule.ended(0, 13, Outcome::Reading(60), true, awake(0));
        let sessions = simulate(&mut schedule, 60, 10 * HOUR, |t| Moment {
            active: t % 60 < 15,
            calm: t % 60 >= 15,
            outcome: Outcome::Reading(120),
            seconds: 30,
            ..resting(t)
        });
        assert!(sessions.iter().any(|s| s.why == Why::Budget));
        // The bucket starts full, so the first hour may spend up to twice
        // the budget; after that, the budget an hour, give or take a
        // session's worth at the window's edges.
        assert!(led(&sessions, 0, HOUR) <= 2 * LED_BUDGET);
        for hour in 1..9 {
            let spent = led(&sessions, hour * HOUR, (hour + 1) * HOUR);
            assert!(
                spent <= LED_BUDGET + SESSION_RESERVE,
                "hour {hour}: {spent} s"
            );
            assert!(
                spent >= LED_BUDGET - SESSION_RESERVE,
                "hour {hour}: {spent} s"
            );
        }
        // And an hour later it's full again (and then some: without a
        // reading meanwhile, one is overdue by then).
        let t = sessions.last().expect("measured").end;
        assert!(schedule.budget(t + HOUR) >= LED_BUDGET);
    }

    #[test]
    fn watching_spends_the_budget_but_isnt_held_to_it() {
        let mut schedule = Schedule::new();
        schedule.ended(100, 13, Outcome::Reading(60), true, awake(0));
        // Five minutes staring at the clock face.
        schedule.ended(500, 300, Outcome::Reading(62), false, awake(0));
        assert_eq!(schedule.budget(500), 0);
        // The background waits for the budget to earn back a session, ~10
        // minutes, rather than the 4 minutes it would otherwise.
        let (at, why) = schedule.due(0, false);
        assert_eq!(why, Why::Budget);
        assert_eq!(at, 500 + SESSION_RESERVE * 3600 / LED_BUDGET);
    }

    #[test]
    fn watching_without_a_reading_is_neither_a_measurement_nor_a_failure() {
        let mut schedule = Schedule::new();
        schedule.ended(100, 13, Outcome::Reading(60), true, awake(0));
        let before = schedule.due(0, false);
        schedule.ended(200, 20, Outcome::NoReading, false, awake(0));
        schedule.ended(300, 20, Outcome::OffWrist, false, awake(0));
        assert_eq!(schedule.due(0, false), before);
        // But a reading while watching is a measurement.
        schedule.ended(400, 20, Outcome::Reading(61), false, awake(0));
        assert_eq!(schedule.due(0, false).0, 400 + 2 * 2 * MIN_INTERVAL);
    }

    #[test]
    fn bright_starts_go_up_two_drives_as_far_as_headroom_and_budget_allow() {
        let plenty = LED_BUDGET;
        // The desk's median peak at 12.5 mA ×2 (2026-11-09): 30 mA.
        assert_eq!(bright_start(0, 3098, plenty), Some(2));
        // Its 90th percentile: 20 mA.
        assert_eq!(bright_start(0, 4035, plenty), Some(1));
        assert_eq!(bright_start(0, 5200, plenty), None);
        // A whole session at 30 mA is 72 s of budget, at 20 mA 48.
        assert_eq!(bright_start(0, 2000, 72), Some(2));
        assert_eq!(bright_start(0, 2000, 71), Some(1));
        assert_eq!(bright_start(0, 2000, 47), None);
    }

    /// The LED experiment's arms take turns by round: every try of a round
    /// (interrupted, failed, retried) keeps its arm, and only a reading
    /// starts the next round, on the other arm. The field log that showed
    /// the need (2026-11-10): a round's first try, bright, stopped for a
    /// moving wrist after 6 s, and its retry read, dim; the next round's
    /// first try was bright again, and so on, so bright had the scheduled
    /// tries (often cut short) and dim the retries (at a still moment).
    #[test]
    fn every_try_of_a_round_keeps_its_arm() {
        let mut schedule = Schedule::new();
        let mut t = 0;
        let mut first_tries = Vec::new();
        // Each round: a first try that's stopped for a moving wrist, one
        // that fails outright, a retry that reads; or, every other round,
        // a first try that reads.
        for round in 1..=8 {
            let tries: &[Outcome] = if round % 2 == 1 {
                &[
                    Outcome::Interrupted,
                    Outcome::NoReading,
                    Outcome::Reading(60),
                ]
            } else {
                &[Outcome::Reading(60)]
            };
            let mut arms = Vec::new();
            for &outcome in tries {
                arms.push(schedule.begin(false));
                t += 300;
                schedule.ended(t, 13, outcome, true, awake(0));
            }
            assert!(
                arms.iter().all(|&a| a == arms[0]),
                "round {round}: {arms:?}"
            );
            assert_eq!(arms[0].0, round);
            first_tries.push(arms[0].1);
        }
        // Dim first, then turns, however many tries each round took.
        assert_eq!(first_tries, [Arm::Dim, Arm::Bright].repeat(4));
    }

    /// Asleep, a session is on neither arm, and doesn't take a turn: the
    /// first awake session of a round does, even if the round started
    /// asleep. A watched reading ends a round too (the next background
    /// session is due afresh), and a watched failure doesn't.
    #[test]
    fn rounds_asleep_or_watched_dont_take_a_turn() {
        let mut schedule = Schedule::new();
        assert_eq!(schedule.begin(false), (1, Arm::Dim));
        schedule.ended(100, 13, Outcome::Reading(60), true, awake(0));
        // A night's rounds.
        for (round, t) in (2..6).zip((400..).step_by(300)) {
            assert_eq!(schedule.begin(true), (round, Arm::Asleep));
            schedule.ended(t, 13, Outcome::Reading(50), true, sleeping());
        }
        // Waking up mid-round: the awake retry takes the turn.
        assert_eq!(schedule.begin(true), (6, Arm::Asleep));
        schedule.ended(2000, 30, Outcome::NoReading, true, sleeping());
        assert_eq!(schedule.begin(false), (6, Arm::Bright));
        // Falling asleep again within the round, then waking, keeps it.
        schedule.ended(2300, 30, Outcome::NoReading, true, awake(0));
        assert_eq!(schedule.begin(true), (6, Arm::Asleep));
        schedule.ended(2600, 30, Outcome::NoReading, true, sleeping());
        assert_eq!(schedule.begin(false), (6, Arm::Bright));
        // Someone looks and gets no reading: still the same round.
        schedule.ended(2900, 30, Outcome::NoReading, true, awake(0));
        schedule.ended(3000, 20, Outcome::OffWrist, false, awake(0));
        assert_eq!(schedule.begin(false), (6, Arm::Bright));
        // Someone looks and gets one: the round is over.
        schedule.ended(3300, 30, Outcome::NoReading, true, awake(0));
        schedule.ended(3400, 20, Outcome::Reading(64), false, awake(0));
        assert_eq!(schedule.begin(false), (7, Arm::Dim));
    }

    /// An hour without a reading, the budget refills twice as fast, into a
    /// bucket twice as deep; a reading puts it back.
    #[test]
    fn an_overdue_reading_doubles_the_budget_until_one_comes() {
        let mut schedule = Schedule::new();
        schedule.ended(100, 13, Outcome::Reading(60), true, awake(0));
        // Five minutes watching without a reading empties it.
        schedule.ended(400, 300, Outcome::NoReading, false, awake(0));
        assert_eq!(schedule.budget(400), 0);
        let overdue = 100 + OVERDUE;
        // Until then, at the usual rate, and no more than the usual cap.
        assert_eq!(schedule.budget(400 + 600), 30);
        assert_eq!(schedule.budget(overdue - 100), 70);
        let mut schedule = Schedule::new();
        schedule.ended(100, 13, Outcome::Reading(60), true, awake(0));
        schedule.ended(overdue - 300, 300, Outcome::NoReading, false, awake(0));
        assert_eq!(schedule.budget(overdue), 15);
        // Then twice as fast, up to twice as much.
        assert_eq!(schedule.budget(overdue + 900), 105);
        assert_eq!(schedule.budget(overdue + HOUR), OVERDUE_BUDGET);
        assert_eq!(schedule.budget(overdue + 10 * HOUR), OVERDUE_BUDGET);
        // Spending some, and earning it back, keeps the part-earned second.
        schedule.ended(overdue + HOUR, 100, Outcome::NoReading, true, awake(0));
        for t in 1..=100 {
            schedule.ended(
                overdue + HOUR + 5 * t,
                0,
                Outcome::NoReading,
                false,
                awake(0),
            );
        }
        assert_eq!(schedule.budget(overdue + HOUR + 500), OVERDUE_BUDGET - 50);
        // A reading: the usual cap again, and the usual rate.
        schedule.ended(overdue + 2 * HOUR, 13, Outcome::Reading(60), true, awake(0));
        assert_eq!(schedule.budget(overdue + 2 * HOUR), LED_BUDGET);
        schedule.ended(overdue + 2 * HOUR, 150, Outcome::NoReading, false, awake(0));
        assert_eq!(schedule.budget(overdue + 2 * HOUR + 600), 60);
    }

    /// The background waits for an overdue budget to hold a session at the
    /// faster rate, and only at the faster rate once it's overdue.
    #[test]
    fn an_overdue_budget_affords_sessions_sooner() {
        let mut schedule = Schedule::new();
        schedule.ended(100, 13, Outcome::Reading(60), true, awake(0));
        let overdue = 100 + OVERDUE;
        // A reading that empties it puts off being overdue: the usual rate,
        // past when it would have been.
        schedule.ended(overdue - 400, 600, Outcome::Reading(60), false, awake(0));
        assert_eq!(schedule.budget(overdue + 200), 600 / 20);
        let mut schedule = Schedule::new();
        schedule.ended(100, 13, Outcome::Reading(60), true, awake(0));
        schedule.ended(overdue - 420, 600, Outcome::NoReading, false, awake(0));
        schedule.ended(overdue - 300, 30, Outcome::NoReading, true, awake(0));
        // 0 left at `overdue - 300`: 15 earned by `overdue`, 15 more in
        // 150 s.
        assert_eq!(schedule.due(0, false), (overdue + 150, Why::Budget));
    }

    /// A whole day overdue, every session failing calm and going on
    /// brighter as often as the budget allows (30 s dim, then 15 s at
    /// 30 mA and 15 s at 40 mA, 114 s of budget in all), as `heart` does:
    /// the doubled budget caps the LED, the worst case docs/power-plan.md
    /// works from.
    #[test]
    fn the_doubled_budget_caps_an_overdue_day() {
        let mut schedule = Schedule::new();
        let (mut t, mut spent) = (0, Vec::new());
        while t < 24 * HOUR {
            let (due, _) = schedule.due(0, false);
            t = t.max(due);
            let budget = schedule.budget(t);
            let (mut cost, mut drive, mut steps) = (30, 0, 0);
            while let Some(next) = brighter(drive, steps, 2500, budget.saturating_sub(cost)) {
                cost += budget_seconds(next, BRIGHTER_FOR * 1000);
                (drive, steps) = (next, steps + 1);
            }
            t += 60;
            schedule.ended(t, cost, Outcome::CalmNoReading, true, awake(0));
            spent.push((t, cost));
        }
        for hour in 2..23 {
            let spent: u32 = spent
                .iter()
                .filter(|(t, _)| (hour * HOUR..(hour + 1) * HOUR).contains(t))
                .map(|(_, s)| s)
                .sum();
            assert!(spent <= OVERDUE_BUDGET + 114, "hour {hour}: {spent} s");
            assert!(spent > LED_BUDGET + 114, "hour {hour}: {spent} s");
        }
    }
}
