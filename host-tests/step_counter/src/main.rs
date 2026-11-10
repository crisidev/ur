//! Host tests for the firmware's `step_counter` module: which reads of
//! the accelerometer's step counter to believe. `cargo test` runs them;
//! `cargo kani` the proofs at the end.

#[path = "../../../src/step_counter.rs"]
#[allow(dead_code)]
mod step_counter;

fn main() {
    println!("run `cargo test`");
}

#[cfg(test)]
mod tests {
    use super::step_counter::*;

    /// What the chip returns for a read it isn't ready for: `e0 e0 e0 e0`.
    const FILLER: u32 = u32::from_le_bytes([0xe0; 4]);
    /// The activity task's poll.
    const POLL: u32 = 2;

    /// Feeds `counts`, one a poll from `start` seconds, to a counter set
    /// up at 0 s; what each read came to.
    fn reads(counter: &mut StepCounter, start: u32, counts: &[u32]) -> Vec<Read> {
        counts
            .iter()
            .enumerate()
            .map(|(i, &count)| counter.read(count, start + POLL * i as u32))
            .collect()
    }

    #[test]
    fn a_walk_counts_every_step() {
        let mut counter = StepCounter::new(0);
        let counts = [0, 0, 10, 14, 18, 22, 22];
        assert_eq!(
            reads(&mut counter, POLL, &counts),
            [0, 0, 10, 4, 4, 4, 0].map(Read::Counted)
        );
        assert_eq!(counter.last(), 22);
    }

    #[test]
    fn a_walk_starts_with_a_burst() {
        // The chip holds the first 8 steps back, then adds them at once,
        // with the poll's own.
        let mut counter = StepCounter::new(0);
        assert_eq!(counter.read(12, POLL), Read::Counted(12));
    }

    #[test]
    fn the_filler_is_ignored_and_counting_carries_on() {
        let mut counter = StepCounter::new(0);
        assert_eq!(
            reads(&mut counter, 100, &[100, FILLER, 104, 108]),
            [
                Read::Counted(100),
                Read::Ignored,
                Read::Counted(4),
                Read::Counted(4)
            ]
        );
        assert_eq!(counter.last(), 108);
    }

    #[test]
    fn a_zero_after_a_broken_batch_is_ignored() {
        // What the field logs show: 0 for one poll, then the true count.
        let mut counter = StepCounter::new(0);
        assert_eq!(
            reads(&mut counter, 1000, &[5000, 0, 5003]),
            [Read::Counted(5000), Read::Ignored, Read::Counted(3)]
        );
    }

    #[test]
    fn a_jump_no_wrist_could_walk_is_ignored() {
        let mut counter = StepCounter::new(0);
        assert_eq!(counter.read(1000, 1000), Read::Counted(1000));
        // A poll later: one byte of the count misread.
        assert_eq!(counter.read(1000 + 256, 1002), Read::Ignored);
        assert_eq!(counter.read(1004, 1004), Read::Counted(4));
    }

    #[test]
    fn the_bound_grows_with_the_time_between_reads() {
        // Reads that failed for ten minutes (a stuck bus) while the wearer
        // walked: the steps are real, and counted.
        let mut counter = StepCounter::new(0);
        assert_eq!(counter.read(80, 10), Read::Counted(80));
        assert_eq!(counter.read(1280, 610), Read::Counted(1200));
        assert!(most_steps(600) >= 1200);
        // But not over a poll.
        assert!(most_steps(POLL) < 100);
    }

    #[test]
    fn filler_twice_moves_the_count_but_loses_almost_nothing() {
        // Two identical garbage reads in a row look like a counter that
        // stopped: the count moves there (counting nothing), then back
        // to the true count two polls later.
        let mut counter = StepCounter::new(0);
        let counts = [100, FILLER, FILLER, 104, 108, 112];
        assert_eq!(
            reads(&mut counter, 100, &counts),
            [
                Read::Counted(100),
                Read::Ignored,
                Read::Moved(0),
                Read::Ignored,
                Read::Moved(4),
                Read::Counted(4),
            ]
        );
        assert_eq!(counter.last(), 112);
    }

    #[test]
    fn a_count_below_the_last_is_followed_once_it_counts_on() {
        // Whatever put the count counted from too high (a garbage read
        // taken for a count, a counter that started again): the old code
        // ignored every count below it, so steps stopped until they passed
        // it again, perhaps days later.
        let mut counter = StepCounter::new(0);
        let counts = [9000, 0, 3, 7, 7];
        assert_eq!(
            reads(&mut counter, 2000, &counts),
            [
                Read::Counted(9000),
                Read::Ignored,
                Read::Moved(3),
                Read::Counted(4),
                Read::Counted(0),
            ]
        );
    }

    #[test]
    fn a_count_stuck_high_is_left_within_two_polls() {
        // However the count got high (here, a counter at billions that
        // then drops to the truth), the truth is followed again two reads
        // after it starts.
        let mut counter = StepCounter::new(0);
        assert_eq!(counter.read(4, POLL), Read::Counted(4));
        assert_eq!(counter.read(FILLER - 30, 2 * POLL), Read::Ignored);
        assert_eq!(counter.read(FILLER - 10, 3 * POLL), Read::Moved(20));
        let truth = [8, 12, 16, 20];
        assert_eq!(
            reads(&mut counter, 4 * POLL, &truth),
            [
                Read::Ignored,
                Read::Moved(4),
                Read::Counted(4),
                Read::Counted(4)
            ]
        );
    }

    #[test]
    fn garbage_alternating_with_the_truth_never_counts() {
        let mut counter = StepCounter::new(0);
        let mut steps = 0;
        for poll in 1..=100u32 {
            let count = if poll % 2 == 0 { FILLER } else { poll * 2 };
            if let Read::Counted(n) | Read::Moved(n) = counter.read(count, poll * POLL) {
                steps += n;
            }
        }
        assert_eq!(steps, 99 * 2);
    }

    #[test]
    fn the_clock_can_start_anywhere() {
        let mut counter = StepCounter::new(u32::MAX - 1);
        assert_eq!(counter.read(10, u32::MAX), Read::Counted(10));
        assert_eq!(most_steps(u32::MAX), u32::MAX);
    }
}

/// Proofs, for every count and time, of what the tests above check for
/// some: nothing panics or overflows, no read counts more than a wrist
/// could walk, and two consistent reads are always followed, so the count
/// can't get stuck. `cargo kani` runs them (see CONTRIBUTING.md).
///
/// Times only go forward, as the activity task's clock (seconds since
/// boot) does.
#[cfg(kani)]
mod proofs {
    use super::step_counter::*;

    /// Reads of any count, each any time after the one before, from a
    /// counter set up at any time: each counts no more than a wrist could
    /// walk since the count it carries on from was read, and the count
    /// only moves when something is counted.
    #[kani::proof]
    #[kani::unwind(4)]
    fn no_read_counts_more_than_a_wrist_walks() {
        let mut now: u32 = kani::any();
        let mut counter = StepCounter::new(now);
        // When the count counted from was read, and the read ignored
        // since, if any: what the module keeps, kept here too.
        let mut last_at = now;
        let mut ignored_at = None;
        for _ in 0..3 {
            let Some(later) = now.checked_add(kani::any()) else {
                return;
            };
            now = later;
            let before = counter.last();
            let count: u32 = kani::any();
            match counter.read(count, now) {
                Read::Counted(steps) => {
                    assert_eq!(before.checked_add(steps), Some(count));
                    assert!(steps <= most_steps(now - last_at));
                    (last_at, ignored_at) = (now, None);
                }
                Read::Moved(steps) => {
                    let at = ignored_at.expect("only after an ignored read");
                    assert!(steps <= count && steps <= most_steps(now - at));
                    (last_at, ignored_at) = (now, None);
                }
                Read::Ignored => {
                    assert_eq!(counter.last(), before);
                    ignored_at = Some(now);
                    continue;
                }
            }
            assert_eq!(counter.last(), count);
        }
    }

    /// After any reads at all (up to two, of any count, so with or
    /// without one ignored), two more as a working counter gives them (the
    /// second no lower, and no more than a wrist walks in the time
    /// between): the second is always taken, so counting resumes however
    /// the count got where it was.
    #[kani::proof]
    #[kani::unwind(3)]
    fn two_consistent_reads_are_always_followed() {
        let mut now: u32 = kani::any();
        let mut counter = StepCounter::new(now);
        for _ in 0..2 {
            if kani::any() {
                let Some(later) = now.checked_add(kani::any()) else {
                    return;
                };
                now = later;
                counter.read(kani::any(), now);
            }
        }
        let (first, wait): (u32, u32) = kani::any();
        let (steps, between): (u32, u32) = kani::any();
        kani::assume(steps <= most_steps(between));
        let (Some(at), Some(second)) = (now.checked_add(wait), first.checked_add(steps)) else {
            return;
        };
        let Some(then) = at.checked_add(between) else {
            return;
        };
        counter.read(first, at);
        let read = counter.read(second, then);
        assert!(matches!(read, Read::Counted(_) | Read::Moved(_)));
        assert_eq!(counter.last(), second);
        kani::cover!(matches!(read, Read::Moved(_)), "moved");
    }
}
