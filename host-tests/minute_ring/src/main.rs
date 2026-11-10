//! Host tests for the firmware's `minute_ring` module: which minutes the
//! activity history keeps as the clock moves. `cargo test` runs them.

// Used only by the tests (and the proofs).
#[path = "../../../src/minute_ring.rs"]
#[allow(dead_code)]
mod minute_ring;

fn main() {
    println!("run `cargo test`");
}

#[cfg(test)]
mod tests {
    use super::minute_ring::file;

    /// A day of minutes, as the firmware's ring has.
    const DAY: i64 = 24 * 60;
    /// 2026-11-09 12:00 UTC, in minutes since 1970.
    const NOON: i64 = 29_903_760;

    /// A ring as `activity::History` keeps one, but with each slot holding
    /// the minute filed in it (and how many times), rather than a sample.
    struct Ring {
        slots: Vec<Option<(i64, u32)>>,
        newest: Option<i64>,
    }

    impl Ring {
        fn new(len: usize) -> Self {
            Self {
                slots: vec![None; len],
                newest: None,
            }
        }

        fn slot(&self, minute: i64) -> usize {
            minute.rem_euclid(self.slots.len() as i64) as usize
        }

        /// Files a sample for `minute`, as `History::at` does.
        fn file(&mut self, minute: i64) {
            let (stale, newest) = file(self.newest, minute, self.slots.len() as i64);
            for m in stale {
                let slot = self.slot(m);
                self.slots[slot] = None;
            }
            self.newest = Some(newest);
            let slot = self.slot(minute);
            let entry = self.slots[slot].get_or_insert((minute, 0));
            entry.1 += 1;
        }

        /// The minutes with samples, oldest first.
        fn minutes(&self) -> Vec<i64> {
            let mut minutes: Vec<i64> = self.slots.iter().flatten().map(|&(m, _)| m).collect();
            minutes.sort();
            minutes
        }

        /// Checks what every filing must leave: each sample in its own
        /// minute's slot, filed under that minute alone, and none newer
        /// than the newest or older than the ring reaches.
        fn check(&self) {
            let newest = self.newest.expect("filed at least once");
            for (i, slot) in self.slots.iter().enumerate() {
                if let Some((minute, _)) = *slot {
                    assert_eq!(self.slot(minute), i, "minute {minute} in slot {i}");
                    assert!(minute <= newest, "minute {minute} after {newest}");
                    assert!(
                        newest - minute < self.slots.len() as i64,
                        "minute {minute} too old for {newest}"
                    );
                }
            }
        }
    }

    #[test]
    fn the_first_minute_starts_the_ring() {
        assert_eq!(file(None, NOON, DAY), (NOON..NOON + 1, NOON));
    }

    #[test]
    fn the_same_minute_again_keeps_everything() {
        assert_eq!(file(Some(NOON), NOON, DAY), (NOON..NOON, NOON));
    }

    #[test]
    fn moving_on_empties_the_minutes_skipped() {
        assert_eq!(
            file(Some(NOON), NOON + 1, DAY),
            (NOON + 1..NOON + 2, NOON + 1)
        );
        // An hour unseen (the watch had no time, say).
        assert_eq!(
            file(Some(NOON), NOON + 60, DAY),
            (NOON + 1..NOON + 61, NOON + 60)
        );
        // Days on: the whole ring, once.
        let (stale, newest) = file(Some(NOON), NOON + 3 * DAY, DAY);
        assert_eq!((stale.end - stale.start, newest), (DAY, NOON + 3 * DAY));
    }

    #[test]
    fn going_back_drops_the_minutes_after() {
        // A few seconds back, over a minute's start: the newest minute
        // goes (a second or two of it was recorded).
        assert_eq!(file(Some(NOON), NOON - 1, DAY), (NOON..NOON + 1, NOON - 1));
        // Two hours back: the two hours after.
        assert_eq!(
            file(Some(NOON), NOON - 120, DAY),
            (NOON - 119..NOON + 1, NOON - 120)
        );
    }

    #[test]
    fn going_back_over_a_day_drops_everything() {
        // The phone's clock was three days ahead, and is put right. Before,
        // what was recorded from then on landed in the slots of minutes
        // three days later, as those minutes.
        let (stale, newest) = file(Some(NOON), NOON - 3 * DAY, DAY);
        assert_eq!(stale, NOON - DAY + 1..NOON + 1);
        assert_eq!(newest, NOON - 3 * DAY);
    }

    #[test]
    fn a_ring_put_back_keeps_only_what_is_before_the_new_time() {
        let mut ring = Ring::new(DAY as usize);
        for minute in NOON - 600..=NOON {
            ring.file(minute);
        }
        // Back an hour: the hour after the new time is gone, and the
        // minute filed again is the same minute's.
        ring.file(NOON - 60);
        ring.check();
        let minutes = ring.minutes();
        assert_eq!(minutes.first(), Some(&(NOON - 600)));
        assert_eq!(minutes.last(), Some(&(NOON - 60)));
        let slot = ring.slot(NOON - 60);
        assert_eq!(ring.slots[slot], Some((NOON - 60, 2)));
        // Recording goes on from there.
        for minute in NOON - 59..=NOON + 30 {
            ring.file(minute);
        }
        ring.check();
        assert_eq!(ring.minutes().len(), 631);
    }

    /// xorshift64*, so runs are reproducible without dependencies.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
        }
    }

    #[test]
    fn any_moves_of_the_clock_keep_every_sample_in_its_minute() {
        // A small ring, so moves of a few times its length come up often.
        const LEN: usize = 16;
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        for _ in 0..200 {
            let mut ring = Ring::new(LEN);
            let mut minute = NOON;
            for _ in 0..200 {
                // Mostly a minute on, sometimes a gap, sometimes back.
                minute += match rng.next() % 10 {
                    0..=5 => 1,
                    6 => 0,
                    7 => (rng.next() % (3 * LEN as u64)) as i64,
                    _ => -((rng.next() % (3 * LEN as u64)) as i64),
                };
                let before = ring.minutes();
                ring.file(minute);
                ring.check();
                assert_eq!(ring.newest, Some(minute));
                // What's kept from before is everything that was before
                // the new minute and still in reach.
                let kept: Vec<i64> = before
                    .into_iter()
                    .filter(|&m| m <= minute && minute - m < LEN as i64)
                    .collect();
                let mut now = ring.minutes();
                if !kept.contains(&minute) {
                    now.retain(|&m| m != minute);
                }
                assert_eq!(now, kept);
            }
        }
    }
}

/// Kani proofs (`cargo kani` here, one at a time with `--harness`): what
/// the tests above check for chosen jumps of the clock, for every jump,
/// on or back, by a minute or by centuries. Nothing panics or overflows;
/// there are never more slots to empty than the ring has; and after
/// filing, the ring holds exactly the minutes it should: the ones it had
/// that are no later than the new minute and within a ring's length of
/// it.
///
/// Minutes are within ±2^40 of 1970 (two million years): `file` adds and
/// subtracts a ring's length unchecked, and would overflow at the ends of
/// an `i64`.
#[cfg(kani)]
mod proofs {
    use super::minute_ring::file;

    /// A day of minutes, as the firmware's ring has.
    const DAY: i64 = 24 * 60;

    fn any_minute() -> i64 {
        let minute: i64 = kani::any();
        kani::assume((-(1 << 40)..1 << 40).contains(&minute));
        minute
    }

    /// For a ring of any length up to a million minutes, with or without
    /// a newest minute: the minutes to empty are a range (not backwards)
    /// of no more than the ring's length, so emptying them takes one
    /// pass over the ring at most; and the newest minute after is the one
    /// filed, wherever the clock went. An empty ring empties just the
    /// slot it files into.
    #[kani::proof]
    fn the_stale_minutes_are_at_most_the_ring() {
        let (newest, minute) = (kani::any::<bool>().then(any_minute), any_minute());
        let len: i64 = kani::any();
        kani::assume((1..=1 << 20).contains(&len));
        let (stale, after) = file(newest, minute, len);
        assert!(stale.start <= stale.end && stale.end - stale.start <= len);
        assert_eq!(after, minute);
        match newest {
            None => assert_eq!(stale, (minute..minute + 1)),
            Some(newest) if newest == minute => assert!(stale.is_empty()),
            Some(_) => {}
        }
    }

    /// For the firmware's ring, any newest minute, and any minute to
    /// file: take any minute the ring held before (one of the `DAY` up
    /// to the newest). Its slot is emptied exactly when it no longer
    /// belongs: when it's after the minute filed (the clock went back,
    /// and it's now in the future), or a day or more before it (the clock
    /// went on past it). So nothing in the future survives, nothing is
    /// filed on top of another day's sample, and nothing else is lost.
    ///
    /// The ring empties by slot (`minute % DAY`), as `activity` does, not
    /// by minute: a stale minute's slot is emptied as that of whichever
    /// minute in the range shares it.
    #[kani::proof]
    fn a_slot_is_emptied_exactly_when_its_minute_no_longer_belongs() {
        let (newest, minute, held) = (any_minute(), any_minute(), any_minute());
        kani::assume(newest - DAY < held && held <= newest);
        let (stale, _) = file(Some(newest), minute, DAY);

        // The first minute from the range's start in the same slot.
        let shares = stale.start + (held - stale.start).rem_euclid(DAY);
        let emptied = shares < stale.end;
        let belongs = minute - DAY < held && held <= minute;
        assert_eq!(emptied, !belongs);
    }
}
