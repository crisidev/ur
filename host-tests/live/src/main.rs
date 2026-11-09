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
        // Midnight: today's count starts over.
        assert_eq!(live.poll(120, true, 7, None), report(7, 0));
        assert_eq!(live.poll(130, true, 7, None), report(0, 0));
        // A new interval mid-session doesn't lose the steps since the
        // last report.
        live.ask(Request::new(false, true, Some(5)), 131, 20);
        assert_eq!(live.poll(136, true, 30, None), report(23, 0));
        // More than a report holds (after a day without the time, say).
        assert_eq!(live.poll(141, true, 100_030, None), report(u16::MAX, 0));
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
