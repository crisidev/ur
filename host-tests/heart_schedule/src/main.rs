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
        assert!(size_of::<Schedule>() <= 20);
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

    /// Giving up counts as a session without a reading: the failure
    /// back-off applies, from when it gave up.
    #[test]
    fn giving_up_backs_off_like_any_failure() {
        let mut schedule = Schedule::new();
        schedule.ended(100, 13, Outcome::Reading(60), true, awake(0));
        let mut t = 400;
        for retry in [5, 10, 20] {
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
        // At least five minutes between tries, doubling, and movement keeps
        // it to no more than the resting interval.
        assert!(waits.iter().all(|&w| w >= FAILED_INTERVAL), "{waits:?}");
        assert!(
            waits[3..].iter().all(|&w| w + 60 >= MAX_INTERVAL),
            "{waits:?}"
        );
        assert!(sessions[1..].iter().all(|s| s.why == Why::Retry));

        // Without movement (a nightstand), up to an hour.
        let mut schedule = Schedule::new();
        let sessions = simulate(&mut schedule, 0, 12 * HOUR, |t| Moment {
            outcome: Outcome::NoReading,
            seconds: 30,
            ..resting(t)
        });
        let waits = gaps(&sessions);
        for (wait, expected) in waits.iter().zip([5, 10, 20, 40, 60, 60, 60]) {
            assert!(about(*wait, expected * MINUTE), "{waits:?}");
        }
    }

    /// Lying still awake for an hour without a reading (a field log,
    /// 2026-11-09): calm failures retry every ten minutes, not 20, 40, 60,
    /// until `CALM_RETRIES` of them, and then back off as usual, in case
    /// it's a still, skin-like surface rather than a wrist.
    #[test]
    fn calm_failures_retry_sooner_for_a_while() {
        let mut schedule = Schedule::new();
        let sessions = simulate(&mut schedule, 0, 6 * HOUR, |t| Moment {
            outcome: Outcome::CalmNoReading,
            seconds: 30,
            ..resting(t)
        });
        let waits = gaps(&sessions);
        let expected = [5, 10, 10, 10, 10, 10, 60, 60, 60];
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
        schedule.ended(1300, 30, Outcome::NoReading, true, awake(0));
        assert_eq!(schedule.due(0, false), (1300 + 20 * MINUTE, Why::Retry));
        schedule.ended(2500, 30, Outcome::CalmNoReading, true, awake(0));
        assert_eq!(schedule.due(0, false), (2500 + 10 * MINUTE, Why::Retry));
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
    fn a_reading_ends_the_failure_back_off() {
        let mut schedule = Schedule::new();
        for t in [0, 300, 900] {
            schedule.ended(t + 30, 30, Outcome::NoReading, true, awake(0));
        }
        assert_eq!(schedule.due(0, false), (930 + 20 * MINUTE, Why::Retry));
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
        // resting interval.
        schedule.ended(t + 300, 30, Outcome::NoReading, true, awake(0));
        schedule.ended(t + 600, 30, Outcome::NoReading, true, awake(0));
        schedule.ended(t + 900, 30, Outcome::NoReading, true, awake(0));
        schedule.ended(t + 1200, 30, Outcome::NoReading, true, awake(0));
        assert_eq!(
            schedule.due(MOVED_POLLS, false),
            (t + 1200 + MAX_INTERVAL, Why::Retry)
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
        // And a calm hour later, it's full again.
        let t = sessions.last().expect("measured").end;
        assert_eq!(schedule.budget(t + HOUR), LED_BUDGET);
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
}
