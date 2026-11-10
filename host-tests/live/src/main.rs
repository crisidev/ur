//! Host tests for the firmware's `live` module: Gadgetbridge's live
//! activity, driven as the UI loop drives it (a poll a second). `cargo
//! test` runs them.

#[path = "../../../src/live.rs"]
#[allow(dead_code)]
mod live;

fn main() {
    println!("run `cargo test`");
}

#[cfg(test)]
mod tests {
    use super::live::*;

    /// What Gadgetbridge sends when its live activity chart opens: steps
    /// first, then the heart rate too (`transmitActivityStatus`, once per
    /// change), at its default interval.
    const STEPS: Request = Request {
        heart_rate: false,
        steps: true,
        interval: 10,
    };
    const BOTH: Request = Request {
        heart_rate: true,
        steps: true,
        interval: 10,
    };
    const STOP: Request = Request {
        heart_rate: false,
        steps: false,
        interval: 10,
    };

    fn report(steps: u16, heart_rate: u8) -> Poll {
        Poll::Report(Report { steps, heart_rate })
    }

    #[test]
    fn intervals() {
        assert_eq!(Request::new(true, true, Some(10)).interval, 10);
        assert_eq!(Request::new(true, true, Some(1)).interval, 1);
        // Missing, or what a Bangle.js would run as fast as it could.
        assert_eq!(Request::new(true, true, None).interval, DEFAULT_INTERVAL);
        assert_eq!(Request::new(true, true, Some(0)).interval, DEFAULT_INTERVAL);
        assert_eq!(
            Request::new(true, true, Some(-5)).interval,
            DEFAULT_INTERVAL
        );
        // Longer than a session runs.
        assert_eq!(
            Request::new(true, true, Some(i64::MAX)).interval,
            MAX_INTERVAL
        );
        assert!(!Request::new(false, false, Some(10)).wanted());
        assert!(Request::new(false, true, Some(10)).wanted());
        assert!(Request::new(true, false, Some(10)).wanted());
    }

    /// The only test of the mailbox (a static, which tests running in
    /// parallel would share).
    #[test]
    fn the_latest_request_counts() {
        assert_eq!(take(), None);
        ask(STEPS);
        ask(Request::new(true, false, Some(MAX_INTERVAL.into())));
        assert_eq!(
            take(),
            Some(Request {
                heart_rate: true,
                steps: false,
                interval: MAX_INTERVAL
            })
        );
        assert_eq!(take(), None);
        ask(STOP);
        assert_eq!(take(), Some(STOP));
    }

    /// Opening the chart, ten seconds' reports, and closing it, as
    /// Gadgetbridge does it.
    #[test]
    fn a_session() {
        let mut live = Live::new();
        assert_eq!(live.poll(100, true, 500, None), Poll::Idle);
        assert_eq!(live.ask(STEPS, 100, 500), None);
        assert!(!live.heart_rate());
        assert_eq!(live.ask(BOTH, 100, 500), None);
        assert!(live.heart_rate());
        // The first report an interval in, as a Bangle.js's.
        for now in 100..110 {
            assert_eq!(live.poll(now, true, 500, None), Poll::Idle, "{now}");
        }
        assert_eq!(live.poll(110, true, 512, Some((72, 105))), report(12, 72));
        assert_eq!(live.poll(111, true, 512, Some((72, 105))), Poll::Idle);
        assert_eq!(live.poll(120, true, 515, Some((74, 118))), report(3, 74));
        // Closing the chart: the steps, then the heart rate.
        assert_eq!(
            live.ask(Request::new(true, false, Some(10)), 125, 515),
            None
        );
        assert_eq!(live.ask(STOP, 125, 515), Some(Ended::Asked));
        assert!(!live.heart_rate());
        assert_eq!(live.poll(130, true, 520, Some((74, 128))), Poll::Idle);
        // Stopping what isn't running says nothing.
        assert_eq!(live.ask(STOP, 131, 520), None);
    }

    /// Only a reading from this session, and a fresh one, is reported.
    #[test]
    fn readings() {
        let mut live = Live::new();
        live.ask(BOTH, 100, 0);
        // From before the session (the clock face's, say): none.
        assert_eq!(live.poll(110, true, 0, Some((60, 99))), report(0, 0));
        assert_eq!(live.poll(120, true, 0, Some((61, 100))), report(0, 61));
        // Up to `FRESH` old.
        assert_eq!(live.poll(130, true, 0, Some((61, 100))), report(0, 61));
        assert_eq!(live.poll(140, true, 0, Some((62, 110))), report(0, 62));
        assert_eq!(live.poll(150, true, 0, Some((62, 110))), report(0, 0));
        // At a longer interval, anything since the last report.
        live.ask(Request::new(true, true, Some(60)), 200, 0);
        assert_eq!(live.poll(260, true, 0, Some((63, 201))), report(0, 63));
        assert_eq!(live.poll(320, true, 0, Some((63, 261))), report(0, 63));
        assert_eq!(live.poll(380, true, 0, Some((63, 319))), report(0, 0));
        // Steps only: a fresh reading from elsewhere is still reported.
        live.ask(STEPS, 400, 0);
        assert_eq!(live.poll(410, true, 0, Some((64, 405))), report(0, 64));
    }

    #[test]
    fn steps() {
        let mut live = Live::new();
        live.ask(STEPS, 100, 9_000);
        assert_eq!(live.poll(110, true, 9_040, None), report(40, 0));
        assert_eq!(live.poll(120, true, 9_047, None), report(7, 0));
        assert_eq!(live.poll(130, true, 9_047, None), report(0, 0));
        // A new interval mid-session doesn't lose the steps since the
        // last report.
        live.ask(Request::new(false, true, Some(5)), 131, 20);
        assert_eq!(live.poll(136, true, 9_070, None), report(23, 0));
        // More than a report holds.
        assert_eq!(live.poll(141, true, 109_070, None), report(u16::MAX, 0));
    }

    /// The count of steps wraps, rather than starting over: the steps
    /// across the wrap are the difference all the same.
    #[test]
    fn the_count_wraps() {
        let mut live = Live::new();
        live.ask(STEPS, 100, u32::MAX - 2);
        assert_eq!(live.poll(110, true, 4, None), report(7, 0));
        assert_eq!(live.poll(120, true, 7, None), report(3, 0));
    }

    /// Whatever the steps do between polls, the reports add up to them.
    #[test]
    fn reports_add_up() {
        let mut live = Live::new();
        let (start, mut steps, mut reported) = (1000, 3_000u32, 0u32);
        live.ask(Request::new(true, true, Some(7)), start, steps);
        // Up to a report.
        for now in start..=start + 7 * 80 {
            steps += (now * 7919 % 13) / 4;
            if let Poll::Report(report) = live.poll(now, true, steps, None) {
                reported += u32::from(report.steps);
                assert_eq!((now - start) % 7, 0, "{now}");
            }
        }
        assert_eq!(3_000 + reported, steps);
    }

    #[test]
    fn a_disconnect_ends_it() {
        let mut live = Live::new();
        live.ask(BOTH, 100, 0);
        assert_eq!(live.poll(105, true, 0, None), Poll::Idle);
        assert_eq!(
            live.poll(106, false, 0, None),
            Poll::Ended(Ended::Disconnected)
        );
        assert!(!live.heart_rate());
        assert_eq!(live.poll(110, true, 0, None), Poll::Idle);
    }

    /// No time limit: the chart is open on the phone, so a session runs
    /// until Gadgetbridge stops it (or the phone goes), however long.
    #[test]
    fn it_runs_until_stopped() {
        let mut live = Live::new();
        live.ask(BOTH, 100, 0);
        let mut reports = 0;
        for now in 100..=100 + 24 * 60 * 60 {
            match live.poll(now, true, 0, None) {
                Poll::Idle => {}
                Poll::Report(_) => reports += 1,
                Poll::Ended(ended) => panic!("ended at {now}: {ended:?}"),
            }
        }
        assert_eq!(reports, 24 * 60 * 60 / 10);
        assert!(live.heart_rate());
        assert_eq!(live.ask(STOP, 100 + 24 * 60 * 60, 0), Some(Ended::Asked));
    }

    /// A poll that comes late (a busy UI loop) reports once, and the next
    /// report is an interval after it, not a burst to catch up.
    #[test]
    fn late_polls_dont_burst() {
        let mut live = Live::new();
        live.ask(BOTH, 100, 0);
        assert_eq!(live.poll(135, true, 0, None), report(0, 0));
        assert_eq!(live.poll(136, true, 0, None), Poll::Idle);
        assert_eq!(live.poll(144, true, 0, None), Poll::Idle);
        assert_eq!(live.poll(145, true, 0, None), report(0, 0));
    }
}

/// Kani proofs (`cargo kani` here, one at a time with `--harness`): what
/// the tests above check for chosen requests and sessions, for every
/// one. Nothing panics or overflows; a request's interval is always one
/// we follow; the request handed to the UI loop arrives as it was; and a
/// session reports every step once, and only a heart rate that's its own
/// and fresh.
///
/// Time is seconds since boot, up to ten years: a session adds its
/// interval to it unchecked, which would overflow in the last ten minutes
/// of a `u32` of seconds (136 years).
#[cfg(kani)]
mod proofs {
    use super::live::*;

    /// Seconds since boot: up to ten years.
    const UPTIME: u32 = 10 * 365 * 86_400;

    fn any_time(after: u32) -> u32 {
        let now: u32 = kani::any();
        kani::assume(after <= now && now < UPTIME);
        now
    }

    /// Any request Gadgetbridge can make: any `int` at all, or none.
    fn any_request() -> Request {
        Request::new(kani::any(), kani::any(), kani::any())
    }

    /// For every `int` Gadgetbridge could send, or none: the interval is
    /// one we follow (1 s to `MAX_INTERVAL`), its own if that's one, the
    /// longest if it's longer, and the default if it's missing or under
    /// a second.
    #[kani::proof]
    fn a_request_has_an_interval_we_follow() {
        let (heart_rate, steps, int): (bool, bool, Option<i64>) = kani::any();
        let request = Request::new(heart_rate, steps, int);
        assert_eq!((request.heart_rate, request.steps), (heart_rate, steps));
        assert!((1..=MAX_INTERVAL).contains(&request.interval));
        match int {
            Some(int) if int >= 1 => {
                assert_eq!(i64::from(request.interval), int.min(MAX_INTERVAL.into()))
            }
            _ => assert_eq!(request.interval, DEFAULT_INTERVAL),
        }
        assert_eq!(request.wanted(), heart_rate || steps);
    }

    /// For every request, with any interval a `u16` holds: what `ask` is
    /// given, `take` gives back, once; and of two asked, only the latest.
    #[kani::proof]
    fn what_is_asked_is_what_is_taken() {
        let any = || Request {
            heart_rate: kani::any(),
            steps: kani::any(),
            interval: kani::any(),
        };
        assert_eq!(take(), None);
        let (first, latest) = (any(), any());
        ask(first);
        assert_eq!(take(), Some(first));
        assert_eq!(take(), None);
        ask(first);
        ask(latest);
        assert_eq!(take(), Some(latest));
        assert_eq!(take(), None);
    }

    /// For every session, started at any time and step count, perhaps
    /// restarted later with another interval, through two reports:
    ///
    /// - Nothing is due before a whole interval has passed since the
    ///   (re)start or the last report, and a report is, from then.
    /// - The steps of the two reports add up to the steps counted since
    ///   the session first started, the count wrapping or not: none
    ///   lost to the restart, none counted twice. (Over 65535 in one
    ///   report would be cut down to that; here there aren't.)
    /// - The sensor is on exactly when the latest request wants it.
    #[kani::proof]
    fn a_session_reports_every_step_once() {
        let mut live = Live::new();
        assert_eq!(
            live.poll(kani::any(), kani::any(), kani::any(), None),
            Poll::Idle
        );
        assert!(!live.heart_rate());

        let (mut request, mut now) = (any_request(), any_time(0));
        kani::assume(request.wanted());
        let start: u32 = kani::any();
        assert_eq!(live.ask(request, now, start), None);
        if kani::any() {
            (request, now) = (any_request(), any_time(now));
            kani::assume(request.wanted());
            assert_eq!(live.ask(request, now, kani::any()), None);
        }
        assert_eq!(live.heart_rate(), request.heart_rate);

        let (mut counted, mut reported) = (start, 0u32);
        for _ in 0..2 {
            // Too soon: nothing, and nothing changes.
            let soon = any_time(now);
            kani::assume(soon - now < u32::from(request.interval));
            assert_eq!(live.poll(soon, true, kani::any(), None), Poll::Idle);

            let (due, steps) = (any_time(now), kani::any::<u32>());
            kani::assume(due - now >= u32::from(request.interval));
            kani::assume(steps.wrapping_sub(counted) <= u32::from(u16::MAX));
            let Poll::Report(report) = live.poll(due, true, steps, None) else {
                unreachable!("a report is due");
            };
            assert_eq!(report.heart_rate, 0);
            reported += u32::from(report.steps);
            (now, counted) = (due, steps);
        }
        assert_eq!(reported, counted.wrapping_sub(start));
    }

    /// For every session and every heart rate reading, at a report: the
    /// reading is reported only if it's from this session (not from
    /// before it started, or restarted) and fresh: no older than `FRESH`,
    /// or than the interval if that's longer. Otherwise the report says
    /// none (0), as it does with no reading.
    #[kani::proof]
    fn a_report_has_only_a_fresh_reading_of_its_session() {
        let mut live = Live::new();
        let (request, started) = (any_request(), any_time(0));
        kani::assume(request.wanted());
        assert_eq!(live.ask(request, started, kani::any()), None);

        let now = any_time(started);
        kani::assume(now - started >= u32::from(request.interval));
        let (bpm, at): (u8, u32) = kani::any();
        kani::assume(at <= now);
        let Poll::Report(report) = live.poll(now, true, kani::any(), Some((bpm, at))) else {
            unreachable!("a report is due");
        };
        let old = now - at > FRESH && now - at > u32::from(request.interval);
        if at < started || old {
            assert_eq!(report.heart_rate, 0);
        } else {
            assert_eq!(report.heart_rate, bpm);
        }
    }

    /// For every session, at any later time: the phone going away ends
    /// it, and Gadgetbridge asking for nothing does, each saying so once;
    /// after either, nothing more is reported, the sensor is off, and
    /// asking for nothing again says nothing.
    #[kani::proof]
    fn a_session_ends_once() {
        let mut live = Live::new();
        let stop = Request::new(false, false, kani::any());
        assert_eq!(live.ask(stop, kani::any(), kani::any()), None);

        let (request, started) = (any_request(), any_time(0));
        kani::assume(request.wanted());
        assert_eq!(live.ask(request, started, kani::any()), None);
        let now = any_time(started);
        if kani::any() {
            let poll = live.poll(now, false, kani::any(), kani::any());
            assert_eq!(poll, Poll::Ended(Ended::Disconnected));
        } else {
            assert_eq!(live.ask(stop, now, kani::any()), Some(Ended::Asked));
        }
        assert!(!live.heart_rate());
        let later = any_time(now);
        let poll = live.poll(later, kani::any(), kani::any(), kani::any());
        assert_eq!(poll, Poll::Idle);
        assert_eq!(live.ask(stop, later, kani::any()), None);
    }
}
