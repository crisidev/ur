//! Host tests for the firmware's `on_time` module, which splits the time
//! `power`'s states spend on into the hourly reports. `cargo test` runs
//! them.

#[path = "../../../src/on_time.rs"]
#[allow(dead_code)]
mod on_time;

fn main() {
    println!("run `cargo test`");
}

#[cfg(test)]
mod tests {
    use super::on_time::OnTime;

    const SECOND: u32 = 1000;
    const MINUTE: u32 = 60 * SECOND;
    const HOUR: u32 = 60 * MINUTE;

    /// A phone that connects at 10:29 and stays until 14:15 (the field log
    /// that showed the bug: 169 s in the first hour, then 0 until it left)
    /// counts the whole of every hour in between.
    #[test]
    fn connection_spanning_hours() {
        let connected = OnTime::new();
        // Reports at the top of each hour, from 10:00.
        let at = |hh: u32, mm: u32| (hh - 10) * HOUR + mm * MINUTE;
        connected.on(at(10, 29));
        assert_eq!(connected.take(at(11, 0)), 31 * MINUTE);
        assert_eq!(connected.take(at(12, 0)), HOUR);
        assert_eq!(connected.take(at(13, 0)), HOUR);
        assert_eq!(connected.take(at(14, 0)), HOUR);
        connected.off(at(14, 15));
        assert_eq!(connected.take(at(15, 0)), 15 * MINUTE);
        assert_eq!(connected.take(at(16, 0)), 0);
    }

    /// A still night: each hour gets its share, and none more than an hour.
    #[test]
    fn still_across_hours() {
        let still = OnTime::new();
        // Reports come when the UI loop gets to them, not on the dot.
        let reports = [HOUR + 7, 2 * HOUR + 3 * SECOND, 3 * HOUR + 120];
        still.on(HOUR / 2);
        assert_eq!(still.take(reports[0]), reports[0] - HOUR / 2);
        assert_eq!(still.take(reports[1]), reports[1] - reports[0]);
        // Moving for a minute, then still again past the next report.
        still.off(2 * HOUR + 30 * MINUTE);
        still.on(2 * HOUR + 31 * MINUTE);
        assert_eq!(
            still.take(reports[2]),
            30 * MINUTE - 3 * SECOND + (reports[2] - (2 * HOUR + 31 * MINUTE)),
        );
    }

    /// States that end within the hour count as before: the sum of their
    /// intervals.
    #[test]
    fn ending_mid_hour() {
        let panel = OnTime::new();
        panel.on(5 * SECOND);
        panel.off(12 * SECOND);
        panel.on(20 * MINUTE);
        panel.off(20 * MINUTE + 3 * SECOND);
        assert_eq!(panel.take(HOUR), 10 * SECOND);
        assert_eq!(panel.take(2 * HOUR), 0);
    }

    /// Two clients connected at once count twice, as they did when each
    /// connection was added up as it ended; each ending only closes its
    /// own.
    #[test]
    fn overlapping() {
        let connected = OnTime::new();
        connected.on(0);
        connected.on(10 * MINUTE);
        connected.off(20 * MINUTE);
        assert_eq!(connected.take(HOUR), 20 * MINUTE + 50 * MINUTE);
        connected.off(HOUR + MINUTE);
        assert_eq!(connected.take(2 * HOUR), MINUTE);
    }

    /// An `off` with nothing on doesn't count, nor leave the next `on`
    /// cancelled out.
    #[test]
    fn unmatched_off() {
        let flash = OnTime::new();
        flash.off(SECOND);
        flash.on(2 * SECOND);
        assert_eq!(flash.take(3 * SECOND), SECOND);
    }

    /// Milliseconds since boot wrap past `u32::MAX` after 49.7 days; an
    /// interval across the wrap still counts what it lasted.
    #[test]
    fn wrapping_clock() {
        let still = OnTime::new();
        let before_wrap = u32::MAX - 10 * SECOND + 1;
        still.on(before_wrap);
        assert_eq!(still.take(20 * SECOND), 30 * SECOND);
        still.off(30 * SECOND);
        assert_eq!(still.take(40 * SECOND), 10 * SECOND);
        // And a stale start from before the wrap doesn't count once off.
        still.on(50 * SECOND);
        assert_eq!(still.take(60 * SECOND), 10 * SECOND);
    }

    /// Against the obvious: for random intervals and report times, each
    /// report is each interval's overlap with its period, summed. Nested
    /// intervals included, as with two clients.
    #[test]
    fn matches_interval_overlap() {
        // A small LCG: deterministic, and no dependency.
        let mut seed: u64 = 0x2026_1109;
        let mut random = |below: u32| {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((seed >> 33) % u64::from(below)) as u32
        };
        for _ in 0..200 {
            // Up to two overlapping streams of intervals (as two connection
            // slots), and reports about hourly, over a day.
            let mut events = Vec::new();
            let mut intervals = Vec::new();
            for _ in 0..1 + random(2) {
                let mut t = random(HOUR);
                while t < 24 * HOUR {
                    let end = t + random(3 * HOUR);
                    intervals.push((t, end));
                    events.push((t, true));
                    events.push((end, false));
                    t = end + random(2 * HOUR);
                }
            }
            let mut reports = Vec::new();
            let mut t = 0;
            while t < 30 * HOUR {
                t += HOUR + random(2 * MINUTE);
                reports.push(t);
            }
            // Ends sort before starts at the same instant; either order
            // gives the same totals.
            events.sort();

            let meter = OnTime::new();
            let mut next = events.iter().peekable();
            let mut previous = 0;
            for &report in &reports {
                while let Some(&&(at, on)) = next.peek() {
                    if at > report {
                        break;
                    }
                    if on {
                        meter.on(at);
                    } else {
                        meter.off(at);
                    }
                    next.next();
                }
                let expected: u32 = intervals
                    .iter()
                    .map(|&(start, end)| end.min(report).saturating_sub(start.max(previous)))
                    .sum();
                assert_eq!(meter.take(report), expected, "{intervals:?} up to {report}");
                previous = report;
            }
        }
    }
}

/// Kani proofs (`cargo kani` here, one at a time with `--harness`): what
/// the tests above check from chosen times, from every time a `u32` of
/// milliseconds holds, the ones just before it wraps included, and
/// across the wrap. Nothing panics or overflows, and every millisecond
/// on lands in exactly one report.
///
/// Times are a start and the stretches between calls (any `u32` each, so
/// under the 49.7 days the module allows between two calls on a counter),
/// added up wrapping, as the firmware's truncated clock does.
#[cfg(kani)]
mod proofs {
    use super::on_time::OnTime;

    /// A state on from any time, through two reports and off before a
    /// third, any stretch apart: each report has exactly the time since
    /// the one before (up to the state turning off), and the one after
    /// has none, however long it waits.
    #[kani::proof]
    fn each_report_has_the_time_since_the_last() {
        let (start, first, second, third, fourth, fifth): (u32, u32, u32, u32, u32, u32) =
            kani::any();
        let on_time = OnTime::new();
        let mut now = start;
        // Off since boot: nothing yet, even if the clock has wrapped.
        assert_eq!(on_time.take(now), 0);

        on_time.on(now);
        now = now.wrapping_add(first);
        assert_eq!(on_time.take(now), first);
        now = now.wrapping_add(second);
        assert_eq!(on_time.take(now), second);

        // Off part-way through the third: that part alone.
        now = now.wrapping_add(third);
        on_time.off(now);
        now = now.wrapping_add(fourth);
        assert_eq!(on_time.take(now), third);
        now = now.wrapping_add(fifth);
        assert_eq!(on_time.take(now), 0);
    }

    /// A state on twice over (two clients connected, say) counts once
    /// for each, saturating rather than wrapping round if that's more
    /// than a `u32` holds; and turning off the second leaves the first
    /// counting.
    #[kani::proof]
    fn overlapping_stretches_count_once_each() {
        let (start, alone, both, after): (u32, u32, u32, u32) = kani::any();
        let on_time = OnTime::new();
        let mut now = start;
        on_time.on(now);
        now = now.wrapping_add(alone);
        on_time.on(now);
        now = now.wrapping_add(both);
        let total = u64::from(alone) + 2 * u64::from(both);
        assert_eq!(u64::from(on_time.take(now)), total.min(u32::MAX.into()));

        on_time.off(now);
        now = now.wrapping_add(after);
        assert_eq!(on_time.take(now), after);
    }

    /// An `off` without an `on` is ignored, any number of them: the next
    /// `on` still counts, from when it came, and its `off` still ends it.
    #[kani::proof]
    fn an_off_without_an_on_is_ignored() {
        let (start, off, on, after): (u32, u32, u32, u32) = kani::any();
        let on_time = OnTime::new();
        let mut now = start;
        on_time.off(now);
        now = now.wrapping_add(off);
        on_time.off(now);
        assert_eq!(on_time.take(now), 0);

        on_time.on(now);
        now = now.wrapping_add(on);
        on_time.off(now);
        now = now.wrapping_add(after);
        assert_eq!(on_time.take(now), on);
    }
}
