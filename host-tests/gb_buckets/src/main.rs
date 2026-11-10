//! Host tests for the firmware's `gb_buckets` module: the activity history
//! in 5-minute buckets, as Gadgetbridge gets it. Against a reference
//! bucket built straight from the history, and a model of Gadgetbridge's
//! Bangle.js sample store (`BangleJSSampleProvider.upsertSample`), which
//! is what the buckets are for. `cargo test` runs them.

// Used by the tests only.
#[path = "../../../src/gb_buckets.rs"]
#[allow(dead_code)]
mod gb_buckets;

fn main() {
    println!("run `cargo test`");
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use quickcheck::{Arbitrary, Gen, QuickCheck};

    use super::gb_buckets::{Bucket, Fetch, MINUTES, Minute, start_of};

    /// What `ble` buckets by: Gadgetbridge's `ActivityKind` name, `None`
    /// for plain activity.
    type Kind = Option<&'static str>;
    const KINDS: [Kind; 4] = [
        None,
        Some("LIGHT_SLEEP"),
        Some("DEEP_SLEEP"),
        Some("AWAKE_SLEEP"),
    ];

    /// 2026-11-09 12:00 UTC, in minutes since 1970: a bucket's start.
    const NOON: i64 = 29_903_760;
    /// The history's length (`activity::HISTORY_MINUTES`).
    const DAY: i64 = 24 * 60;
    /// How far before the phone's last fetch a fetch starts
    /// (`stages::REVISED`).
    const REVISED: i64 = 164;

    // ==========================================================================
    // Histories
    // ==========================================================================

    /// A history: the minutes with data, by UTC minute, as
    /// `activity::History::since` gives them.
    type History = BTreeMap<i64, Minute<Kind>>;

    fn minute(steps: u16, heart_rate: u8, kind: Kind) -> Minute<Kind> {
        Minute {
            steps,
            movement: steps,
            heart_rate,
            seen: true,
            kind,
        }
    }

    /// Arbitrary minutes: mostly consecutive, with gaps (the watch off),
    /// minutes with only a heart rate, and runs of one kind (as sleep
    /// comes).
    #[derive(Clone, Debug)]
    struct Minutes(History);

    impl Arbitrary for Minutes {
        fn arbitrary(g: &mut Gen) -> Self {
            let mut history = History::new();
            let mut at = NOON + i64::from(u8::arbitrary(g) % 7);
            let mut kind = None;
            for _ in 0..usize::arbitrary(g) % (g.size() * 8) {
                at += match u8::arbitrary(g) % 16 {
                    0 => 1 + i64::from(u8::arbitrary(g)),
                    _ => 1,
                };
                if u8::arbitrary(g) % 8 == 0 {
                    kind = *g.choose(&KINDS).expect("not empty");
                }
                let seen = u8::arbitrary(g) % 16 != 0;
                let heart_rate = match u8::arbitrary(g) % 4 {
                    0 => u8::arbitrary(g).max(1),
                    _ => 0,
                };
                if !seen && heart_rate == 0 {
                    // `History::since` skips these.
                    continue;
                }
                history.insert(
                    at,
                    Minute {
                        steps: if seen { u16::arbitrary(g) } else { 0 },
                        movement: if seen { u16::arbitrary(g) } else { 0 },
                        heart_rate,
                        seen,
                        kind: if seen { kind } else { None },
                    },
                );
            }
            Self(history)
        }
    }

    /// All of `fetch`'s buckets, from `history` (which `oldest` clips, as
    /// the ring does).
    fn run(mut fetch: Fetch, history: &History, oldest: i64) -> Vec<Bucket<Kind>> {
        let mut buckets = Vec::new();
        while let Some(bucket) = fetch.next(
            history
                .range(fetch.from().max(oldest)..)
                .map(|(&minute, &sample)| (minute, sample)),
        ) {
            buckets.push(bucket);
        }
        buckets
    }

    /// The bucket starting at `start`, built straight from all of its
    /// minutes in `history`, as the module docs describe it; `None` if it
    /// has none.
    fn reference(history: &History, start: i64) -> Option<Bucket<Kind>> {
        let minutes: Vec<_> = history.range(start..start + MINUTES).collect();
        let seen: Vec<_> = minutes.iter().filter(|(_, m)| m.seen).collect();
        // The kind most minutes have; of those, the latest minute's.
        let votes = |kind: Kind| minutes.iter().filter(|(_, m)| m.kind == kind).count();
        let most = minutes.iter().map(|(_, m)| votes(m.kind)).max()?;
        let kind = minutes
            .iter()
            .rev()
            .map(|(_, m)| m.kind)
            .find(|&kind| votes(kind) == most)?;
        Some(Bucket {
            start,
            steps: minutes.iter().map(|(_, m)| u32::from(m.steps)).sum(),
            movement: if seen.is_empty() {
                0
            } else {
                let sum: u32 = seen.iter().map(|(_, m)| u32::from(m.movement)).sum();
                sum.div_ceil(seen.len() as u32) as u16
            },
            heart_rate: minutes
                .iter()
                .rev()
                .map(|(_, m)| m.heart_rate)
                .find(|&bpm| bpm != 0)
                .unwrap_or(0),
            kind,
        })
    }

    fn steps_in(history: &History, from: i64, to: i64) -> u32 {
        history
            .range(from..to.max(from))
            .map(|(_, m)| u32::from(m.steps))
            .sum()
    }

    fn quickcheck() -> QuickCheck {
        QuickCheck::new().tests(2_000)
    }

    // ==========================================================================
    // A model of Gadgetbridge's sample store
    // ==========================================================================

    /// A sample as Gadgetbridge stores it, by its timestamp in seconds.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct Stored {
        steps: u32,
        heart_rate: u8,
        intensity: u16,
        kind: Kind,
    }

    /// `BangleJSSampleProvider.upsertSample` (Gadgetbridge 4b09bc6135,
    /// lines 90–126), as `handleActivity` calls it for each non-live `act`
    /// sample: if it has samples within ±2 minutes (inclusive), it
    /// overwrites the "nearest" one's steps, heart rate and intensity, each
    /// only if the new one's is non-zero, and keeps its kind; else it adds
    /// the new one. "Nearest" as its loop picks it, which compares each
    /// candidate's distance to the new sample with its distance to the
    /// nearest so far (not the new sample's).
    fn upsert(store: &mut BTreeMap<i64, Stored>, timestamp: i64, sample: Stored) {
        let near: Vec<i64> = store
            .range(timestamp - 120..=timestamp + 120)
            .map(|(&t, _)| t)
            .collect();
        let Some(&first) = near.first() else {
            store.insert(timestamp, sample);
            return;
        };
        let mut nearest = first;
        for &t in &near {
            if (timestamp - t).abs() < (nearest - t).abs() {
                nearest = t;
            }
        }
        let stored = store.get_mut(&nearest).expect("one of them");
        if sample.heart_rate != 0 {
            stored.heart_rate = sample.heart_rate;
        }
        if sample.steps != 0 {
            stored.steps = sample.steps;
        }
        if sample.intensity != 0 {
            stored.intensity = sample.intensity;
        }
    }

    fn send(store: &mut BTreeMap<i64, Stored>, bucket: &Bucket<Kind>) {
        let timestamp = bucket.timestamp_ms() / 1000;
        upsert(
            store,
            timestamp,
            Stored {
                steps: bucket.steps,
                heart_rate: bucket.heart_rate,
                intensity: bucket.movement,
                kind: bucket.kind,
            },
        );
    }

    /// A day of fetches, as Gadgetbridge makes them: each `gaps[i]`
    /// minutes (and `seconds[i]` seconds) after the last, asking from a
    /// minute before the last one ended (`saveLastSyncTimestamp`), less
    /// [`REVISED`], as `ble` asks for it; or, now and then, for everything
    /// (`gb::fetch_from`'s fetch from the phone's future). The watch's
    /// history at each is what `full` holds up to then: its 24 hours, and
    /// the current minute's steps so far. Returns the store, and where the
    /// first fetch started and the last ended.
    fn fetches(full: &History, gaps: &[(u16, u8, bool)]) -> (BTreeMap<i64, Stored>, i64, i64) {
        let mut store = BTreeMap::new();
        let mut now_seconds = (full.keys().next().copied().unwrap_or(NOON) - 30) * 60;
        let mut last_sync_seconds = None::<i64>;
        let (mut first, mut last) = (None, i64::MIN);
        for &(gap, seconds, everything) in gaps {
            now_seconds += i64::from(gap % 400) * 60 + i64::from(seconds % 60);
            let now = now_seconds.div_euclid(60);
            let oldest = now - DAY + 1;
            // The current minute is under way: half its steps so far.
            let mut history: History = full.range(oldest..now).map(|(&m, &s)| (m, s)).collect();
            if let Some(&current) = full.get(&now) {
                history.insert(
                    now,
                    Minute {
                        steps: current.steps / 2,
                        ..current
                    },
                );
            }
            let since = match last_sync_seconds {
                Some(synced) if !everything => (synced - 60).div_euclid(60) - REVISED,
                _ => 0,
            };
            let fetch = Fetch::new(since, oldest, now);
            first.get_or_insert(fetch.from());
            last = fetch.sent_until();
            for bucket in run(fetch, &history, oldest) {
                send(&mut store, &bucket);
            }
            last_sync_seconds = Some(now_seconds);
        }
        (store, first.unwrap_or(0), last)
    }

    // ==========================================================================
    // Steps
    // ==========================================================================

    /// Whatever the history, and wherever a fetch starts and ends, its
    /// buckets hold exactly the steps of the minutes from where it starts
    /// (a bucket's start) to where it ends (the start of the bucket under
    /// way).
    #[test]
    fn a_fetch_keeps_every_step() {
        fn property(Minutes(history): Minutes, since: u16, now: u16) -> bool {
            let since = NOON + i64::from(since % 4000) - 500;
            let now = NOON + i64::from(now % 4000);
            let fetch = Fetch::new(since, i64::MIN / 2, now);
            let (from, to) = (fetch.from(), fetch.sent_until());
            let buckets = run(fetch, &history, i64::MIN / 2);
            buckets.iter().map(|b| b.steps).sum::<u32>() == steps_in(&history, from, to)
                && from == start_of(since)
                && from <= since
                && to == start_of(now)
        }
        quickcheck().quickcheck(property as fn(Minutes, u16, u16) -> bool);
    }

    /// And from before the first minute to after the last, that's all of
    /// them.
    #[test]
    fn a_whole_history_keeps_every_step() {
        fn property(Minutes(history): Minutes) -> bool {
            let end = history.keys().last().copied().unwrap_or(NOON) + MINUTES;
            let buckets = run(Fetch::new(0, i64::MIN / 2, end), &history, i64::MIN / 2);
            buckets.iter().map(|b| b.steps).sum::<u32>()
                == history.values().map(|m| u32::from(m.steps)).sum::<u32>()
        }
        quickcheck().quickcheck(property as fn(Minutes) -> bool);
    }

    // ==========================================================================
    // What the buckets look like
    // ==========================================================================

    /// Each bucket starts on a multiple of 5 minutes, and is stamped with
    /// its last second; so each is at least 5 minutes from the next, more
    /// than the 2 minutes either side within which Gadgetbridge merges
    /// samples. And each is the whole bucket, as built straight from all
    /// its minutes: never part of one, wherever the fetch started.
    #[test]
    fn buckets_are_whole_and_apart() {
        fn property(Minutes(history): Minutes, since: u16, now: u16) -> bool {
            let since = NOON + i64::from(since % 4000) - 500;
            let now = NOON + i64::from(now % 4000);
            let buckets = run(Fetch::new(since, i64::MIN / 2, now), &history, i64::MIN / 2);
            let apart = buckets
                .windows(2)
                .all(|w| w[1].timestamp_ms() - w[0].timestamp_ms() >= MINUTES * 60_000);
            let stamped = buckets.iter().all(|b| {
                b.start % MINUTES == 0 && b.timestamp_ms() == (b.start + MINUTES) * 60_000 - 1_000
            });
            let whole = buckets
                .iter()
                .all(|b| reference(&history, b.start) == Some(*b));
            // And every bucket with data in the span is there.
            let all = (start_of(since)..start_of(now))
                .step_by(MINUTES as usize)
                .filter_map(|start| reference(&history, start))
                .eq(buckets.iter().copied());
            apart && stamped && whole && all
        }
        quickcheck().quickcheck(property as fn(Minutes, u16, u16) -> bool);
    }

    /// A bucket from 12:00 UTC is stamped 12:04:59.
    #[test]
    fn stamped_with_its_last_second() {
        let bucket: Bucket<Kind> = Bucket {
            start: NOON,
            steps: 1,
            movement: 1,
            heart_rate: 0,
            kind: None,
        };
        assert_eq!(bucket.timestamp_ms(), NOON * 60_000 + 299_000);
    }

    #[test]
    fn heart_rate_is_the_latest_reading() {
        let history = History::from([
            (NOON, minute(10, 61, None)),
            (NOON + 1, minute(10, 75, None)),
            (NOON + 2, minute(10, 0, None)),
        ]);
        let buckets = run(Fetch::new(NOON, NOON, NOON + MINUTES), &history, NOON);
        assert_eq!(buckets[0].heart_rate, 75);
        assert_eq!(buckets[0].steps, 30);
    }

    /// The kind most minutes have; of a tie, the later minute's.
    #[test]
    fn kind_by_vote() {
        let light = Some("LIGHT_SLEEP");
        let deep = Some("DEEP_SLEEP");
        let bucket = |kinds: &[Kind]| {
            let history: History = kinds
                .iter()
                .enumerate()
                .map(|(i, &kind)| (NOON + i as i64, minute(0, 0, kind)))
                .collect();
            run(Fetch::new(NOON, NOON, NOON + MINUTES), &history, NOON)[0].kind
        };
        assert_eq!(bucket(&[None, light, light]), light);
        assert_eq!(bucket(&[light, None, light]), light);
        assert_eq!(bucket(&[deep, deep, light]), deep);
        assert_eq!(bucket(&[None, light, deep]), deep);
        assert_eq!(bucket(&[light, None]), None);
        assert_eq!(bucket(&[deep, light, light, None, deep]), deep);
        assert_eq!(bucket(&[deep, deep, light, light, None]), light);
        assert_eq!(bucket(&[light, deep, deep, deep, light]), deep);
    }

    /// The mean of the minutes the accelerometer recorded, rounded up.
    #[test]
    fn movement_is_the_mean_of_the_recorded_minutes() {
        let mut history = History::from([
            (NOON, minute(0, 0, None)),
            (NOON + 1, minute(1, 0, None)),
            // Only a heart rate: no movement measured.
            (
                NOON + 2,
                Minute {
                    seen: false,
                    ..minute(0, 70, None)
                },
            ),
        ]);
        let movement = |history: &History| {
            run(Fetch::new(NOON, NOON, NOON + MINUTES), history, NOON)[0].movement
        };
        assert_eq!(movement(&history), 1);
        history.get_mut(&NOON).expect("there").movement = 4;
        assert_eq!(movement(&history), 3);
    }

    // ==========================================================================
    // Fetching again
    // ==========================================================================

    /// A fetch asked from mid-bucket starts at the bucket's start, so it
    /// sends the bucket whole, as before.
    #[test]
    fn a_fetch_from_mid_bucket_sends_the_whole_bucket() {
        let history: History = (0..10).map(|i| (NOON + i, minute(10, 0, None))).collect();
        let first = run(Fetch::new(NOON, NOON, NOON + 10), &history, NOON);
        let again = run(Fetch::new(NOON + 2, NOON, NOON + 10), &history, NOON);
        assert_eq!(first.len(), 2);
        assert_eq!(again, first);
        assert_eq!(again[0].steps, 50);
    }

    /// The bucket under way isn't sent, even its finished minutes, and even
    /// when none of them has steps yet; it goes out whole once it's over.
    #[test]
    fn the_bucket_under_way_waits() {
        let history = History::from([(NOON, minute(0, 0, None)), (NOON + 1, minute(0, 0, None))]);
        assert_eq!(run(Fetch::new(NOON, NOON, NOON + 2), &history, NOON), []);
        let history: History = (0..5)
            .map(|i| (NOON + i, minute(if i == 4 { 40 } else { 0 }, 0, None)))
            .collect();
        let fetch = Fetch::new(NOON, NOON, NOON + 5);
        assert_eq!(fetch.sent_until(), NOON + 5);
        assert_eq!(run(fetch, &history, NOON)[0].steps, 40);
    }

    /// A bucket whose first minutes the history has dropped (it keeps a
    /// day) isn't sent: the phone may have it whole.
    #[test]
    fn a_bucket_partly_dropped_isnt_sent() {
        let history: History = (0..10).map(|i| (NOON + i, minute(10, 0, None))).collect();
        let oldest = NOON + 1;
        let fetch = Fetch::new(0, oldest, NOON + 10);
        assert_eq!(fetch.from(), NOON + 5);
        let buckets = run(fetch, &history, oldest);
        assert_eq!(buckets.len(), 1);
        assert_eq!(buckets[0].start, NOON + 5);
    }

    /// Two fetches, the second later and from anywhere, with the minutes in
    /// between recorded: the buckets both send are the same.
    #[test]
    fn a_bucket_sent_again_is_the_same() {
        fn property(Minutes(full): Minutes, since: (u16, u16), now: (u16, u16)) -> bool {
            let first_now = NOON + i64::from(now.0 % 3000);
            let second_now = first_now + i64::from(now.1 % 600);
            let then: History = full.range(..first_now).map(|(&m, &s)| (m, s)).collect();
            let first = run(
                Fetch::new(NOON + i64::from(since.0 % 3000) - 100, 0, first_now),
                &then,
                0,
            );
            let second = run(
                Fetch::new(NOON + i64::from(since.1 % 3000) - 100, 0, second_now),
                &full,
                0,
            );
            first.iter().all(|a| {
                second
                    .iter()
                    .find(|b| b.start == a.start)
                    .is_none_or(|b| a == b)
            })
        }
        quickcheck().quickcheck(property as fn(Minutes, (u16, u16), (u16, u16)) -> bool);
    }

    // ==========================================================================
    // In Gadgetbridge
    // ==========================================================================

    /// The bug the buckets fix, in the model: a 10-minute walk at 100
    /// steps a minute, sent a sample a minute (as before), keeps 400 of its
    /// 1,000 steps in Gadgetbridge; one in three samples survives, with the
    /// last merged minute's steps.
    #[test]
    fn minutes_lose_steps_in_gadgetbridge() {
        let mut store = BTreeMap::new();
        for i in 0..10 {
            let sample = Stored {
                steps: 100,
                heart_rate: 0,
                intensity: 100,
                kind: None,
            };
            upsert(&mut store, (NOON + i) * 60, sample);
        }
        assert_eq!(store.len(), 4);
        assert_eq!(store.values().map(|s| s.steps).sum::<u32>(), 400);
    }

    /// The same walk in buckets, fetched every minute: all 1,000 steps.
    #[test]
    fn buckets_keep_every_step_in_gadgetbridge() {
        let full: History = (0..10).map(|i| (NOON + i, minute(100, 0, None))).collect();
        // From half an hour before (see `fetches`), then every minute.
        let mut gaps = vec![(30, 0, false)];
        gaps.extend([(1, 0, false); 14]);
        let (store, _, _) = fetches(&full, &gaps);
        assert_eq!(store.values().map(|s| s.steps).sum::<u32>(), 1_000);
    }

    /// Fetches as Gadgetbridge makes them (each from just before the last
    /// ended, less what's revised; sometimes for everything), on a history
    /// that grows in between and drops what's a day old: Gadgetbridge ends
    /// up with every step from where the first fetch started to where the
    /// last ended, and each bucket exactly as the watch has it, kind and
    /// all.
    #[test]
    fn gadgetbridge_keeps_every_step() {
        fn property(Minutes(full): Minutes, gaps: Vec<(u16, u8, bool)>) -> bool {
            if gaps.is_empty() {
                return true;
            }
            let (store, from, to) = fetches(&full, &gaps);
            let exact = store.iter().all(|(&timestamp, stored)| {
                let start = start_of(timestamp.div_euclid(60));
                reference(&full, start).is_some_and(|b| {
                    b.timestamp_ms() / 1000 == timestamp
                        && stored.steps == b.steps
                        && stored.heart_rate == b.heart_rate
                        && stored.intensity == b.movement
                        && stored.kind == b.kind
                })
            });
            exact && store.values().map(|s| s.steps).sum::<u32>() == steps_in(&full, from, to)
        }
        quickcheck().quickcheck(property as fn(Minutes, Vec<(u16, u8, bool)>) -> bool);
    }
}
