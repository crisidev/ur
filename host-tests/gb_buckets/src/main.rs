//! Host tests for the firmware's `gb_buckets` module: the activity history
//! in 5-minute buckets, as Gadgetbridge gets it. Against a reference
//! bucket built straight from the history, and a model of Gadgetbridge's
//! Bangle.js sample store (`BangleJSSampleProvider.upsertSample`), which
//! is what the buckets are for. And its `sleep_hold` module, which holds
//! back the minutes whose sleep stages may still change: against a model
//! of the watch classifying and staging a night minute by minute (with the
//! real `sleep` and `stages`), fetched from on any schedule. `cargo test`
//! runs them.

// Used by the tests only.
#[path = "../../../src/gb_buckets.rs"]
#[allow(dead_code)]
mod gb_buckets;
#[path = "../../../src/minute_ring.rs"]
mod minute_ring;
#[path = "../../../src/sleep.rs"]
#[allow(dead_code)]
mod sleep;
#[path = "../../../src/sleep_hold.rs"]
#[allow(dead_code)]
mod sleep_hold;
#[path = "../../../src/stages.rs"]
#[allow(dead_code)]
mod stages;

fn main() {
    println!("run `cargo test`");
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use quickcheck::{Arbitrary, Gen, QuickCheck};

    use super::gb_buckets::{
        Bucket, Fetch, MINUTES, Minute, REACH, start, start_of, synced_after_set,
    };
    use super::minute_ring;

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
    /// only if the new one's is non-zero, and never its kind (`rawKind`),
    /// which stays the first it got; else it adds the new one. "Nearest" as its loop picks it, which compares each
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

    /// What Gadgetbridge asks a fetch from (`ts`, UTC milliseconds), at
    /// `now_seconds`: its clock a minute before its last fetch ended
    /// (`saveLastSyncTimestamp`, `last_sync_seconds`), or before any, a
    /// day back (`getLastSuccessfulSyncTime`); or, with `ahead`, from its
    /// future (its clock was ahead at the last fetch, and has been put
    /// right since).
    fn asked_ms(last_sync_seconds: Option<i64>, now_seconds: i64, ahead: bool) -> i64 {
        match last_sync_seconds {
            _ if ahead => (now_seconds + 86_400) * 1000,
            Some(synced) => (synced - 60) * 1000,
            None => (now_seconds - 86_400) * 1000,
        }
    }

    /// The watch's history at `now` (a UTC minute): what `full` holds of
    /// its 24 hours, and the current minute's steps so far (half).
    fn history_at(full: &History, now: i64) -> History {
        let oldest = now - DAY + 1;
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
        history
    }

    /// The buckets of one fetch at `now_seconds`, as `ble` answers it: from
    /// [`start`], given what Gadgetbridge asked and where the watch's last
    /// fetch stopped (`synced`, which it updates), sent to `store`.
    /// Returns where the fetch started, and up to where the phone then has
    /// everything.
    fn fetch(
        store: &mut BTreeMap<i64, Stored>,
        history: &History,
        asked_ms: i64,
        synced: &mut Option<i64>,
        now_seconds: i64,
    ) -> (i64, i64) {
        let now = now_seconds.div_euclid(60);
        let oldest = now - DAY + 1;
        let fetch = Fetch::new(start(asked_ms, *synced, now), oldest, now);
        let from = fetch.from();
        *synced = Some(fetch.synced_through());
        for bucket in run(fetch, history, oldest) {
            send(store, &bucket);
        }
        (from, synced.expect("just set") + 1)
    }

    /// A day of fetches, as Gadgetbridge makes them: each `gaps[i]`
    /// minutes (and `seconds[i]` seconds) after the last, asking from a
    /// minute before the last one ended (`saveLastSyncTimestamp`); or,
    /// now and then, from its future (see [`asked_ms`]). The watch's
    /// history at each is what `full` holds up to then (see
    /// [`history_at`]). Returns the store, and where the first fetch
    /// started and the last ended.
    fn fetches(full: &History, gaps: &[(u16, u8, bool)]) -> (BTreeMap<i64, Stored>, i64, i64) {
        let mut store = BTreeMap::new();
        let mut now_seconds = (full.keys().next().copied().unwrap_or(NOON) - 30) * 60;
        let mut last_sync_seconds = None::<i64>;
        let mut synced = None;
        let (mut first, mut last) = (None, i64::MIN);
        for &(gap, seconds, ahead) in gaps {
            now_seconds += i64::from(gap % 400) * 60 + i64::from(seconds % 60);
            let history = history_at(full, now_seconds.div_euclid(60));
            let asked = asked_ms(last_sync_seconds, now_seconds, ahead);
            let (from, to) = fetch(&mut store, &history, asked, &mut synced, now_seconds);
            first.get_or_insert(from);
            last = to;
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
    /// way), after which the phone's copy ends (or where it started, if
    /// that's later).
    #[test]
    fn a_fetch_keeps_every_step() {
        fn property(Minutes(history): Minutes, since: u16, now: u16) -> bool {
            let since = NOON + i64::from(since % 4000) - 500;
            let now = NOON + i64::from(now % 4000);
            let fetch = Fetch::new(since, i64::MIN / 2, now);
            let (from, synced) = (fetch.from(), fetch.synced_through());
            let buckets = run(fetch, &history, i64::MIN / 2);
            buckets.iter().map(|b| b.steps).sum::<u32>() == steps_in(&history, from, start_of(now))
                && from == start_of(since)
                && from <= since
                && synced + 1 == start_of(now).max(from)
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
        assert_eq!(fetch.synced_through(), NOON + 4);
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
    /// ended; sometimes from its future), on a history
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

    /// Gadgetbridge keeps the first kind it gets for a sample: a sample
    /// sent again with a revised kind changes its steps, not its kind.
    #[test]
    fn the_kind_is_never_updated() {
        let mut store = BTreeMap::new();
        let sample = |steps, kind| Stored {
            steps,
            heart_rate: 0,
            intensity: 0,
            kind,
        };
        upsert(&mut store, NOON * 60, sample(5, None));
        upsert(&mut store, NOON * 60, sample(7, Some("LIGHT_SLEEP")));
        assert_eq!(store[&(NOON * 60)], sample(7, None));
    }

    // ==========================================================================
    // Where a fetch starts
    // ==========================================================================

    /// Knowing where the phone's copy ends, a fetch starts there, however
    /// much later the phone asks (a hold kept the rest back), or from its
    /// future.
    #[test]
    fn a_fetch_starts_where_the_last_stopped() {
        let now = NOON + 600;
        let synced = Some(NOON + 99);
        assert_eq!(start((now - 2) * 60_000, synced, now), NOON + 100);
        assert_eq!(start((now + DAY) * 60_000, synced, now), NOON + 100);
    }

    /// Without that, or if the phone asks from earlier than the watch
    /// thinks it has, from the first bucket after the minute it asked
    /// from, plus [`REACH`]: what its last fetch may have sent.
    #[test]
    fn otherwise_after_all_the_phone_may_have() {
        let now = NOON + 600;
        // Asked from 12:06:30: may have up to 12:08.
        let asked = (NOON + 6) * 60_000 + 30_000;
        assert_eq!(REACH, 2);
        assert_eq!(start(asked, None, now), NOON + 10);
        assert_eq!(start(asked, Some(NOON + 99), now), NOON + 10);
        // Asked from 12:07: may have up to 12:09, the bucket's last minute.
        assert_eq!(start((NOON + 7) * 60_000, None, now), NOON + 10);
        // From 12:08: up to 12:10, the next bucket's first.
        assert_eq!(start((NOON + 8) * 60_000, None, now), NOON + 15);
    }

    /// From its future, without knowing where its copy ends: the phone may
    /// have anything up to now, so nothing from before the bucket after
    /// the one under way.
    #[test]
    fn from_the_future_without_knowing_nothing_before_now() {
        let now = NOON + 602;
        assert_eq!(start((now + DAY) * 60_000, None, now), NOON + 605);
        let history: History = (0..610).map(|i| (NOON + i, minute(10, 0, None))).collect();
        let fetch = Fetch::new(start((now + DAY) * 60_000, None, now), NOON, now);
        assert_eq!(fetch.synced_through(), NOON + 604);
        assert_eq!(run(fetch, &history, NOON), []);
    }

    // ==========================================================================
    // After an update from a build that sent a sample a minute
    // ==========================================================================

    /// How far before what the phone asked the builds that sent a sample a
    /// minute (0.9.x, 0.10.0-rc.1 to rc.3) started a fetch (their
    /// `stages::REVISED`).
    const OLD_REVISED: i64 = 164;

    /// What came of an older build's fetches, an update, and the new
    /// build's fetches.
    struct Updated {
        /// Gadgetbridge's samples at the update, and in the end.
        before: BTreeMap<i64, Stored>,
        after: BTreeMap<i64, Stored>,
        /// The newest minute the older build sent: the minute under way at
        /// its last fetch, by the watch's clock.
        newest_old: i64,
        /// The history the new build had (restored, and recorded since).
        history: History,
        /// Where the new build's first fetch started, the oldest minute its
        /// history held then, and up to where its last left the phone
        /// everything.
        from: i64,
        oldest: i64,
        to: i64,
    }

    /// Gadgetbridge fetching from an older build, `before` apart (as in
    /// [`fetches`]), with the watch's clock `skew` seconds ahead of the
    /// phone's (under a minute): each fetch sends a sample a minute,
    /// stamped at its start, from [`OLD_REVISED`] before what the phone
    /// asks through the minute under way (its steps so far), filed through
    /// [`upsert`] as Gadgetbridge files them. Then `update.0` minutes after
    /// the last fetch, the watch restarts into the new build, `update.1`
    /// minutes later; the older build saves what the phone hadn't fetched
    /// (from [`OLD_REVISED`] before the last minute it sent), for the new
    /// build to restore, if `update.2`. Without knowing what the phone
    /// has (the record doesn't say), the new build answers fetches `after`
    /// apart as in [`fetches`], the first asking from just before the
    /// older build's last fetch ended.
    fn update(
        full: &History,
        before: &[(u16, u8)],
        skew: u8,
        update: (u16, u16, bool),
        after: &[(u16, u8, bool)],
    ) -> Updated {
        let skew = i64::from(skew % 60);
        let mut store = BTreeMap::new();
        let mut now_seconds = (full.keys().next().copied().unwrap_or(NOON) - 30) * 60;
        let mut last_sync_seconds = None::<i64>;
        let (mut newest_old, mut synced_old) = (i64::MIN, i64::MIN);
        for &(gap, seconds) in before {
            now_seconds += i64::from(gap % 400) * 60 + i64::from(seconds % 60);
            let watch_now = (now_seconds + skew).div_euclid(60);
            let asked = asked_ms(last_sync_seconds, now_seconds, false).div_euclid(60_000);
            let history = history_at(full, watch_now);
            let mut last = None;
            for (&minute, sample) in history.range(asked - OLD_REVISED..) {
                let stored = Stored {
                    steps: u32::from(sample.steps),
                    heart_rate: sample.heart_rate,
                    intensity: sample.movement,
                    kind: sample.kind,
                };
                upsert(&mut store, minute * 60, stored);
                last = Some(minute);
            }
            newest_old = watch_now;
            // As those builds' `system_off::on_synced` was told.
            synced_old = last.unwrap_or(i64::MIN).max(asked);
            last_sync_seconds = Some(now_seconds);
        }
        let before_update = store.clone();

        // The update: the older build saves, from its own `unsynced_from`
        // (a day at most), the history to the minute it restarts in.
        let (to_update, off, record) = update;
        // Mostly soon after a fetch (an update comes through Gadgetbridge),
        // now and then much later.
        now_seconds += i64::from(match to_update % 4 {
            0 => to_update % 2000,
            _ => to_update % 30,
        }) * 60
            + i64::from(to_update % 60);
        let restart = (now_seconds + skew).div_euclid(60);
        let mut history: History = if record {
            let first = (synced_old - OLD_REVISED).max(restart - DAY + 1);
            full.range(first..restart).map(|(&m, &s)| (m, s)).collect()
        } else {
            History::new()
        };
        // The new build starts `off` minutes later, recording from then on.
        // Mostly an update's restart; now and then hours off (a switch-off).
        let boot = restart
            + 1
            + i64::from(match off % 8 {
                0 => off % 600,
                _ => off % 3,
            });
        history.extend(full.range(boot..).map(|(&m, &s)| (m, s)));
        now_seconds = now_seconds.max(boot * 60 - skew);

        let mut synced = None;
        let (mut first, mut oldest, mut to) = (None, 0, i64::MIN);
        for (i, &(gap, seconds, ahead)) in after.iter().enumerate() {
            now_seconds += i64::from(gap % 400) * 60 + i64::from(seconds % 60);
            let watch_seconds = now_seconds + skew;
            let watch_now = watch_seconds.div_euclid(60);
            // A first fetch from the phone's future is the case the module
            // docs give up on: see `from_the_future_without_knowing`.
            let asked = asked_ms(last_sync_seconds, now_seconds, ahead && i > 0);
            let at = history_at(&history, watch_now);
            let (from, end) = fetch(&mut store, &at, asked, &mut synced, watch_seconds);
            if first.is_none() {
                first = Some(from);
                oldest = watch_now - DAY + 1;
            }
            to = end;
            last_sync_seconds = Some(now_seconds);
        }
        Updated {
            before: before_update,
            after: store,
            newest_old,
            history,
            from: first.unwrap_or(i64::MAX),
            oldest,
            to,
        }
    }

    /// The bug this is for, in the model: a walk Gadgetbridge has as an
    /// older build's samples a minute apart, sent again as buckets (as the
    /// first fetch after the update to rc.4 did, from 164 minutes back),
    /// counts steps twice.
    #[test]
    fn buckets_over_minute_samples_count_twice() {
        let full: History = (0..30).map(|i| (NOON + i, minute(10, 0, None))).collect();
        let mut store = BTreeMap::new();
        for (&minute, sample) in &full {
            let stored = Stored {
                steps: u32::from(sample.steps),
                heart_rate: 0,
                intensity: sample.movement,
                kind: None,
            };
            upsert(&mut store, minute * 60, stored);
        }
        let had: u32 = store.values().map(|s| s.steps).sum();
        assert_eq!(had, 100);
        for bucket in run(Fetch::new(NOON, NOON, NOON + 30), &full, NOON) {
            send(&mut store, &bucket);
        }
        let has: u32 = store.values().map(|s| s.steps).sum();
        assert!(has > 300, "{had} then {has} of the watch's 300");
    }

    /// The property [`start`] is for: whenever the update comes, and
    /// however Gadgetbridge fetches before and after it, the new build
    /// never touches a sample the older one sent (so never counts its
    /// steps twice), and sends every step from the first bucket after the
    /// older build's last minute, but for at most the 5 minutes before it:
    /// each bucket whole, as the watch has it.
    #[test]
    fn an_update_from_minute_samples_counts_no_step_twice() {
        fn property(
            Minutes(full): Minutes,
            before: Vec<(u16, u8)>,
            skew: u8,
            updating: (u16, u16, bool),
            after: Vec<(u16, u8, bool)>,
        ) -> bool {
            if before.is_empty() || after.is_empty() {
                return true;
            }
            let updated = update(&full, &before, skew, updating, &after);
            let Updated {
                before: ref had,
                after: ref has,
                newest_old,
                ref history,
                from,
                oldest,
                to,
            } = updated;
            let untouched = had.iter().all(|(t, s)| has.get(t) == Some(s));
            let exact = has
                .iter()
                .filter(|(t, _)| !had.contains_key(t))
                .all(|(&t, stored)| {
                    let start = start_of(t.div_euclid(60));
                    reference(history, start).is_some_and(|b| {
                        b.timestamp_ms() / 1000 == t
                            && stored.steps == b.steps
                            && stored.heart_rate == b.heart_rate
                            && stored.intensity == b.movement
                            && stored.kind == b.kind
                    })
                });
            let sum = |store: &BTreeMap<i64, Stored>| store.values().map(|s| s.steps).sum::<u32>();
            let gained = sum(has) - sum(had);
            let all = steps_in(history, from, to) == gained;
            // Lost: the minutes between, at most 5, or those the history
            // had already dropped.
            let after_old = from > newest_old;
            let boundary = from <= (newest_old + 1 + MINUTES).max(start_of(oldest + MINUTES - 1));
            if !(untouched && exact && all && after_old && boundary) {
                eprintln!(
                    "untouched {untouched}, exact {exact}, gained {gained} of {}, \
                     newest old {newest_old}, from {from} (oldest {oldest}), to {to}",
                    steps_in(history, from, to)
                );
                return false;
            }
            true
        }
        quickcheck().quickcheck(
            property
                as fn(Minutes, Vec<(u16, u8)>, u8, (u16, u16, bool), Vec<(u16, u8, bool)>) -> bool,
        );
    }

    // ==========================================================================
    // A clock put right
    // ==========================================================================

    /// The watch's history as `activity::History` files it: the minutes
    /// with data, and the newest, through `minute_ring::file`, so that
    /// filing a minute before the newest (the clock went back) drops the
    /// ones after it, and a jump on drops what's then a day old.
    #[derive(Default)]
    struct Ring {
        minutes: History,
        newest: Option<i64>,
    }

    impl Ring {
        /// Adds `steps` to `minute`'s sample, as `activity` does every few
        /// seconds.
        fn record(&mut self, minute: i64, steps: u16) {
            let (stale, newest) = minute_ring::file(self.newest, minute, DAY);
            for dropped in stale {
                self.minutes.remove(&dropped);
            }
            self.newest = Some(newest);
            let sample = self
                .minutes
                .entry(minute)
                .or_insert_with(|| self::minute(0, 0, None));
            sample.steps += steps;
        }

        /// The oldest minute `History::since` reads.
        fn oldest(&self) -> i64 {
            self.newest.map_or(i64::MIN / 2, |newest| newest - DAY + 1)
        }
    }

    /// The steps the wearer takes in real minute `real`: never none, so
    /// that every bucket sent overwrites what Gadgetbridge had there.
    fn steps_at(real: i64) -> u16 {
        1 + real.rem_euclid(7) as u16
    }

    /// A phone whose clock goes wrong and is put right, and a watch that
    /// records all along, minute by minute from [`NOON`] (real minutes) to
    /// `until`. The phone's clock is `lead` minutes ahead from real minute
    /// `wrong` to `right`, and right otherwise; it sets the watch's clock
    /// to its own at `wrong` and at `right` (as Gadgetbridge does on
    /// connecting, and when the phone's time changes), and with
    /// `on_connect`, fetches right then too, before the watch files its
    /// next sample. It also fetches half a minute into each real minute
    /// in `fetches`, each asking from a minute before its last fetch ended
    /// by its own clock (`saveLastSyncTimestamp`), the first from a day
    /// back.
    #[derive(Clone, Debug)]
    struct WrongClock {
        lead: i64,
        wrong: i64,
        right: i64,
        until: i64,
        on_connect: bool,
        fetches: Vec<i64>,
    }

    /// What came of a [`WrongClock`]: Gadgetbridge's samples, the watch's
    /// history, and where the phone's copy ends after the last fetch.
    struct PutRight {
        store: BTreeMap<i64, Stored>,
        ring: Ring,
        end: i64,
    }

    fn put_right(clock: &WrongClock) -> PutRight {
        let mut store = BTreeMap::new();
        let mut ring = Ring::default();
        let mut synced = None;
        let mut last_sync_ms = None::<i64>;
        let mut end = i64::MIN;
        // One fetch, as `ble::send_activity` answers it, at the watch's
        // `watch_seconds` and the phone's `phone_ms`.
        let mut fetch = |ring: &Ring,
                         synced: &mut Option<i64>,
                         store: &mut BTreeMap<i64, Stored>,
                         watch_seconds: i64,
                         phone_ms: i64| {
            let asked = last_sync_ms.map_or(phone_ms - 86_400_000, |last| last - 60_000);
            let now = watch_seconds.div_euclid(60);
            let oldest = ring.oldest() + MINUTES;
            let fetch = Fetch::new(start(asked, *synced, now), oldest, now);
            *synced = Some(fetch.synced_through());
            for bucket in run(fetch, &ring.minutes, ring.oldest()) {
                send(store, &bucket);
            }
            last_sync_ms = Some(phone_ms);
            end = synced.expect("just set") + 1;
        };
        for real in NOON..clock.until {
            let lead = if (clock.wrong..clock.right).contains(&real) {
                clock.lead
            } else {
                0
            };
            let watch = real + lead;
            if real == clock.wrong || real == clock.right {
                // The phone sets the watch's clock to its own, and the
                // watch takes it as `ble` does.
                synced = synced_after_set(synced, watch);
                if clock.on_connect {
                    fetch(&ring, &mut synced, &mut store, watch * 60, watch * 60_000);
                }
            }
            ring.record(watch, steps_at(real));
            if clock.fetches.contains(&real) {
                let seconds = watch * 60 + 30;
                fetch(&ring, &mut synced, &mut store, seconds, seconds * 1000);
            }
        }
        PutRight { store, ring, end }
    }

    /// The buckets from the one the clock was put right in (`right`) to
    /// where the phone's copy ends that Gadgetbridge doesn't have as the
    /// watch does: their starts, with the steps it has (if any) and the
    /// watch's.
    fn lost_after(right: i64, put: &PutRight) -> Vec<(i64, Option<u32>, u32)> {
        let from = start_of(right).max(start_of(put.ring.oldest() + MINUTES - 1));
        (from..put.end)
            .step_by(MINUTES as usize)
            .filter_map(|start| {
                let bucket = reference(&put.ring.minutes, start)?;
                let stored = put.store.get(&(bucket.timestamp_ms() / 1000));
                let stored = stored.map(|s| s.steps);
                (stored != Some(bucket.steps)).then_some((start, stored, bucket.steps))
            })
            .collect()
    }

    /// The phone's clock 3 hours ahead from 13:00 to 17:00, and so the
    /// watch's: the phone fetches at a quarter past and a quarter to, and
    /// puts the watch's clock right at 17:00. Its next fetch, at 17:15,
    /// asks from 19:44:30, its clock at the last fetch (19:45:30 for
    /// 16:45:30): the phone's future. Everything the watch recorded from
    /// 17:00 reaches the phone all the same. (It used not to: the
    /// watch's sync point was 19:44, so the fetch at 17:15 sent nothing,
    /// and the one at 17:45 started at 17:20.)
    #[test]
    fn a_clock_put_right_from_hours_ahead_loses_nothing() {
        let clock = WrongClock {
            lead: 180,
            wrong: NOON + 60,
            right: NOON + 300,
            until: NOON + 600,
            on_connect: false,
            fetches: (NOON + 15..NOON + 600).step_by(30).collect(),
        };
        let put = put_right(&clock);
        assert!(put.end > clock.right + 200);
        assert_eq!(lost_after(clock.right, &put), []);
    }

    /// The same, 30 minutes ahead: the phone's fetch at 17:15, after the
    /// clock is put right, asks from 17:14:30 (its clock at 16:45:30 was
    /// 17:15:30), which is no longer its future; but the watch's last
    /// fetch, at 17:15 by the clock that was ahead, sent up to 17:14. The
    /// minutes from 17:00 by the clock put right reach the phone all the
    /// same. (They used not to, up to 17:15: the watch took the phone to
    /// have them.)
    #[test]
    fn a_clock_put_right_from_minutes_ahead_loses_nothing() {
        let clock = WrongClock {
            lead: 30,
            wrong: NOON + 60,
            right: NOON + 300,
            until: NOON + 600,
            on_connect: false,
            fetches: (NOON + 15..NOON + 600).step_by(30).collect(),
        };
        let put = put_right(&clock);
        assert!(put.end > clock.right + 200);
        assert_eq!(lost_after(clock.right, &put), []);
    }

    /// The phone's clock a day and a half ahead, and Gadgetbridge
    /// fetching the moment it puts the watch's clock right, before the
    /// watch has filed a sample by it: its history is a day and a half
    /// ahead of the clock then, so all of it is about to go.
    #[test]
    fn a_fetch_as_the_clock_is_put_right_from_days_ahead_loses_nothing() {
        let clock = WrongClock {
            lead: DAY * 3 / 2,
            wrong: NOON + 60,
            right: NOON + 300,
            until: NOON + 600,
            on_connect: true,
            fetches: (NOON + 15..NOON + 600).step_by(30).collect(),
        };
        let put = put_right(&clock);
        assert!(put.end > clock.right + 200);
        assert_eq!(lost_after(clock.right, &put), []);
    }

    /// Any lead (up to two days), for any time, fetched from on any
    /// schedule: once the phone has put the watch's clock right,
    /// Gadgetbridge gets every bucket from the one it was put right in
    /// as the watch has it.
    #[test]
    fn a_clock_put_right_loses_nothing() {
        fn property(lead: u16, wrong: (u8, u16), on_connect: bool, gaps: Vec<u8>) -> bool {
            let lead = 1 + i64::from(lead) % (2 * DAY);
            let wrong_at = NOON + i64::from(wrong.0);
            let right = wrong_at + 1 + i64::from(wrong.1 % 600);
            let mut fetches = Vec::new();
            let mut at = NOON;
            for gap in gaps {
                at += 1 + i64::from(gap % 120);
                fetches.push(at);
            }
            let until = at + 1;
            let clock = WrongClock {
                lead,
                wrong: wrong_at,
                right,
                until,
                on_connect,
                fetches,
            };
            let put = put_right(&clock);
            let lost = lost_after(right, &put);
            if !lost.is_empty() {
                eprintln!("{clock:?}: lost {lost:?}");
            }
            lost.is_empty()
        }
        quickcheck().quickcheck(property as fn(u16, (u8, u16), bool, Vec<u8>) -> bool);
    }

    /// The buckets before real minute `wrong` (when the phone's clock
    /// went wrong), from `from`, that Gadgetbridge doesn't have as the
    /// wearer walked them: their starts, with the steps it has (if any)
    /// and the steps taken.
    fn lost_before(wrong: i64, from: i64, put: &PutRight) -> Vec<(i64, Option<u32>, u32)> {
        (from..start_of(wrong))
            .step_by(MINUTES as usize)
            .filter_map(|start| {
                let steps = (start..start + MINUTES)
                    .map(|m| u32::from(steps_at(m)))
                    .sum();
                let timestamp = (start + MINUTES) * 60 - 1;
                let stored = put.store.get(&timestamp).map(|s| s.steps);
                (stored != Some(steps)).then_some((start, stored, steps))
            })
            .collect()
    }

    /// The other way: the phone's clock jumps 3 hours ahead at 13:00, a
    /// quarter of an hour after its last fetch, and sets the watch's. The
    /// next fetch, at 16:15 by both, asks from 12:44:30, and the watch
    /// sends from 12:45, where its last fetch stopped: the quarter of an
    /// hour before the jump arrives at the right time (and what comes
    /// after it 3 hours late, as the clocks say).
    #[test]
    fn a_clock_set_hours_ahead_loses_nothing_before() {
        let clock = WrongClock {
            lead: 180,
            wrong: NOON + 60,
            right: NOON + 300,
            until: NOON + 600,
            on_connect: false,
            fetches: (NOON + 15..NOON + 600).step_by(30).collect(),
        };
        let put = put_right(&clock);
        assert_eq!(lost_before(clock.wrong, NOON + 15, &put), []);
    }

    /// Set more than a day ahead, the clock files its next minute where
    /// the whole history is over a day old (see `minute_ring`): what the
    /// phone hadn't fetched before the jump is gone, unless Gadgetbridge
    /// fetches as it sets the clock, before the watch files that minute.
    /// (A known loss, not a bug in where fetches start: the history keeps
    /// a day by the clock, and the watch can't tell a wrong clock from a
    /// long time off the phone.)
    #[test]
    fn a_clock_set_days_ahead_keeps_what_was_unfetched_only_if_fetched_then() {
        let clock = |on_connect| WrongClock {
            lead: 2 * DAY,
            wrong: NOON + 60,
            right: NOON + 300,
            until: NOON + 600,
            on_connect,
            fetches: (NOON + 15..NOON + 600).step_by(30).collect(),
        };
        let put = put_right(&clock(true));
        assert_eq!(lost_before(NOON + 60, NOON + 15, &put), []);
        let put = put_right(&clock(false));
        let lost: Vec<i64> = lost_before(NOON + 60, NOON + 15, &put)
            .into_iter()
            .map(|(start, stored, _)| {
                assert_eq!(stored, None);
                start
            })
            .collect();
        assert_eq!(lost, [NOON + 45, NOON + 50, NOON + 55]);
    }

    // ==========================================================================
    // Holding back what may still change
    // ==========================================================================

    use super::sleep::{self, Kind as Class};
    use super::sleep_hold::{CAP, Hold, Settled, UP_MINUTES, hold, settles};
    use super::stages;

    const ACTIVE: sleep::Minute = sleep::Minute {
        seen: true,
        movement: 800,
        steps: 30,
        worn: Some(true),
    };
    const STILL: sleep::Minute = sleep::Minute {
        seen: true,
        movement: 30,
        steps: 0,
        worn: Some(true),
    };
    /// How long after a sleep's end it's settled.
    const SETTLED: i64 = (sleep::WINDOW + sleep::SAME_NIGHT_GAP) as i64;

    /// [`hold`] at `newest`, with the last sleep from `sleep.0` until
    /// `sleep.1` (light, as are the minutes `extra` covers), active up to
    /// `still` and the evidence `rest` from then on.
    fn hold_at(
        newest: i64,
        last_sleep: Option<(i64, i64)>,
        extra: std::ops::Range<i64>,
        still: i64,
        rest: sleep::Minute,
    ) -> Option<Hold> {
        let asleep = |minute: i64| {
            last_sleep.is_some_and(|(start, end)| (start..end).contains(&minute))
                || extra.contains(&minute)
        };
        hold(
            newest,
            Settled::NONE,
            |minute| {
                if asleep(minute) {
                    Class::Light
                } else {
                    Class::Awake
                }
            },
            |minute| if minute < still { ACTIVE } else { rest },
        )
    }

    /// An active wearer, with no sleep in the last 90 minutes: nothing is
    /// held back.
    #[test]
    fn by_day_nothing_is_held() {
        let newest = NOON + 500;
        assert_eq!(hold_at(newest, None, 0..0, newest + 1, STILL), None);
        // A sleep that ended 135 minutes ago is settled.
        let night = Some((NOON, newest - SETTLED));
        assert_eq!(hold_at(newest, night, 0..0, newest + 1, STILL), None);
    }

    /// A sleep is held back from its start while the classifier's window
    /// could still find sleep that would join it, and no longer: until
    /// its last minute asleep is 135 minutes back.
    #[test]
    fn a_sleep_is_held_until_135_minutes_after_it() {
        let night = Some((NOON, NOON + 400));
        let held = Some(Hold {
            from: NOON,
            sleep: true,
        });
        for newest in [NOON + 30, NOON + 399, NOON + 400, NOON + 400 + SETTLED - 1] {
            assert_eq!(hold_at(newest, night, 0..0, i64::MAX, STILL), held);
        }
        assert_eq!(
            hold_at(NOON + 400 + SETTLED, night, 0..0, i64::MAX, STILL),
            None
        );
    }

    /// An earlier sleep whose end is still in the window is held back too,
    /// all of it, though the watch no longer stages it: the classifier
    /// still rewrites its end, and should the later sleep turn out not to
    /// be one, the watch stages it again.
    #[test]
    fn an_earlier_sleep_in_the_window_is_held() {
        let newest = NOON + 430;
        let nap = Some((NOON + 400, NOON + 420));
        let held = hold_at(newest, nap, NOON + 300..NOON + 360, i64::MAX, STILL);
        assert_eq!(
            held,
            Some(Hold {
                from: NOON + 300,
                sleep: true,
            })
        );
    }

    /// A wearer gone still may be falling asleep: the stillness is held
    /// back (as from where it's still to the classifier, whose smoothing
    /// takes a minute's neighbours in), but it's not a sleep. Unless the
    /// watch isn't worn, which can't fall asleep.
    #[test]
    fn a_stillness_is_held() {
        let newest = NOON + 500;
        assert_eq!(
            hold_at(newest, None, 0..0, newest - 9, STILL),
            Some(Hold {
                from: newest - 8,
                sleep: false,
            })
        );
        let off = sleep::Minute {
            worn: Some(false),
            ..STILL
        };
        assert_eq!(hold_at(newest, None, 0..0, newest - 9, off), None);
        // The minute under way may yet turn out still, if the wrist stays
        // so for the rest of it; the one before it then too, but not one
        // with steps.
        assert_eq!(
            hold_at(newest, None, 0..0, newest - 1, STILL),
            Some(Hold {
                from: newest,
                sleep: false,
            })
        );
    }

    /// A sleep that never ends is held back at most [`CAP`] minutes.
    #[test]
    fn a_hold_is_capped() {
        let newest = NOON + 20 * 60;
        let night = Some((NOON, newest + 1));
        assert_eq!(
            hold_at(newest, night, 0..0, i64::MAX, STILL),
            Some(Hold {
                from: newest - CAP + 1,
                sleep: true,
            })
        );
    }

    // ==========================================================================
    // Settling, once the wearer is clearly up
    // ==========================================================================

    const WALKING: sleep::Minute = sleep::Minute {
        seen: true,
        movement: 800,
        steps: sleep::WAKE_STEPS,
        worn: Some(true),
    };
    /// Up, but not walking: moving about without steps.
    const FIDGETING: sleep::Minute = sleep::Minute {
        steps: 0,
        ..WALKING
    };

    /// [`settles`] at `newest`, after a night ending at `NOON` (its last
    /// minute asleep `NOON - 1`), given the evidence of each minute after
    /// it, and with nothing settled.
    fn settles_at(newest: i64, after: impl Fn(i64) -> sleep::Minute) -> Option<i64> {
        settles(
            newest,
            Settled::NONE,
            |minute| {
                if (NOON - 400..NOON).contains(&minute) {
                    Class::Light
                } else {
                    Class::Awake
                }
            },
            |minute| if minute < NOON { STILL } else { after(minute) },
        )
    }

    /// Up and walking: the night settles once more than
    /// `SAME_NIGHT_GAP` whole minutes have passed since its last minute
    /// asleep, so that no sleep found later can join it.
    #[test]
    fn a_night_settles_once_up_for_longer_than_a_wake_in_it() {
        let gap = sleep::SAME_NIGHT_GAP as i64;
        // The minute under way, `NOON + gap + 1`, doesn't count yet.
        assert_eq!(settles_at(NOON + gap, |_| WALKING), None);
        assert_eq!(settles_at(NOON + gap + 1, |_| WALKING), Some(NOON - 1));
        // Nor does it after the hold would have let the night go anyway:
        // there's nothing left to settle.
        assert_eq!(settles_at(NOON + SETTLED, |_| WALKING), None);
    }

    /// Off the wrist counts as up, as walking does.
    #[test]
    fn off_the_wrist_counts_as_up() {
        let off = sleep::Minute {
            worn: Some(false),
            ..STILL
        };
        let gap = sleep::SAME_NIGHT_GAP as i64;
        assert_eq!(settles_at(NOON + gap, |_| off), None);
        assert_eq!(settles_at(NOON + gap + 1, |_| off), Some(NOON - 1));
        // One minute off is one minute up.
        let once = |minute: i64| if minute == NOON + 3 { off } else { FIDGETING };
        assert_eq!(settles_at(NOON + 60, once), None);
        let walked = |minute: i64| {
            if minute == NOON + 9 {
                WALKING
            } else {
                once(minute)
            }
        };
        assert_eq!(settles_at(NOON + 60, walked), Some(NOON - 1));
    }

    /// Up, but having walked less than `UP_MINUTES` minutes (one wild step
    /// count, say), or lying still now: not settled.
    #[test]
    fn a_night_settles_only_after_walking_and_not_while_still() {
        let newest = NOON + 60;
        assert_eq!(settles_at(newest, |_| FIDGETING), None);
        let walked = |minutes: i64| {
            move |minute: i64| {
                if minute < NOON + minutes {
                    WALKING
                } else {
                    FIDGETING
                }
            }
        };
        assert_eq!(settles_at(newest, walked(UP_MINUTES as i64 - 1)), None);
        assert_eq!(
            settles_at(newest, walked(UP_MINUTES as i64)),
            Some(NOON - 1)
        );
        // Walked, then lay down: still for the last ten minutes, which
        // may yet become sleep.
        let lay_down = |minute: i64| if minute < newest - 10 { WALKING } else { STILL };
        assert_eq!(settles_at(newest, lay_down), None);
    }

    /// A sleep entirely within the classifier's window (a nap, or a few
    /// still minutes the window has only just found to be sleep, which it
    /// may take back) isn't settled early; one that began before it is.
    #[test]
    fn only_a_sleep_older_than_the_window_settles() {
        let newest = NOON + 60;
        let sleep_from = |start: i64| {
            move |minute: i64| {
                if (start..NOON).contains(&minute) {
                    Class::Light
                } else {
                    Class::Awake
                }
            }
        };
        let walking = |minute: i64| if minute < NOON { STILL } else { WALKING };
        let window = newest - sleep::WINDOW as i64 + 1;
        for (start, settled) in [(window, false), (window - 1, true)] {
            assert_eq!(
                settles(newest, Settled::NONE, sleep_from(start), walking).is_some(),
                settled,
                "a sleep from {}",
                start - NOON
            );
        }
    }

    /// Once settled, nothing before is held back, and no sleep; nor the
    /// minutes after it, while the wearer is up and about. But a stillness
    /// after it that's long enough for a nap is, from its start (never
    /// from before the settling), while the classifier may still find a
    /// nap in it, until the night would have gone anyway.
    #[test]
    fn a_settled_night_is_held_no_more() {
        let night = |minute: i64| {
            if (NOON..NOON + 400).contains(&minute) {
                Class::Light
            } else {
                Class::Awake
            }
        };
        let settled = Settled::before(NOON + 447);
        for newest in [NOON + 447, NOON + 460, NOON + 400 + SETTLED] {
            assert_eq!(hold(newest, settled, night, |_| WALKING), None);
        }
        // Up and about, then on the sofa from 470, then up again from
        // `up`: still for `RESUME` minutes, as the classifier sees them,
        // which smooths each minute's movement with its neighbours'.
        let sofa = |up: i64| {
            move |minute: i64| {
                if (NOON + 470..up).contains(&minute) {
                    STILL
                } else {
                    WALKING
                }
            }
        };
        let rest = NOON + 470 + sleep::RESUME as i64 + 2;
        // From its first still minute, the one before being a walking one.
        let held = Some(Hold {
            from: NOON + 471,
            sleep: false,
        });
        assert_eq!(hold(rest + 5, settled, night, sofa(rest)), held);
        // Still held once up, while the night would have been; then as by
        // day.
        let night_end = NOON + 400 + SETTLED;
        assert_eq!(hold(night_end - 1, settled, night, sofa(rest)), held);
        assert_eq!(hold(night_end, settled, night, sofa(rest)), None);
        // Shorter than a nap: nothing held once up.
        assert_eq!(hold(rest + 5, settled, night, sofa(rest - 1)), None);
        // Lying still since before the settling: from the settling.
        let newest = NOON + 460;
        let still = |minute: i64| if minute < newest - 10 { WALKING } else { STILL };
        assert_eq!(
            hold(newest, Settled::before(newest - 5), night, still),
            Some(Hold {
                from: newest - 5,
                sleep: false,
            })
        );
        // Unsettled, the night itself is held.
        assert_eq!(
            hold(newest, Settled::NONE, night, |_| WALKING),
            Some(Hold {
                from: NOON,
                sleep: true,
            })
        );
    }

    /// The classifier rewrites its window, but never a settled minute.
    #[test]
    fn the_classifier_keeps_off_what_is_settled() {
        let newest = NOON + 1000;
        let window = newest - sleep::WINDOW as i64 + 1;
        assert_eq!(Settled::NONE.rewritten(newest), window);
        assert_eq!(Settled::before(window - 50).rewritten(newest), window);
        assert_eq!(Settled::before(window + 10).rewritten(newest), window + 10);
        // Settled past the newest (the clock went back, before the
        // watch forgot it): as if settled at the newest.
        assert_eq!(Settled::before(newest + 30).rewritten(newest), newest);
    }

    /// A sleep is restaged while it reaches into the window, unless it's
    /// settled.
    #[test]
    fn a_settled_sleep_is_not_restaged() {
        let newest = NOON + 1000;
        let end = newest - 50;
        assert!(Settled::NONE.restaged(newest, NOON, end));
        assert!(!Settled::NONE.restaged(newest, NOON, newest - sleep::WINDOW as i64));
        assert!(!Settled::before(newest - 3).restaged(newest, NOON, end));
        assert!(Settled::before(NOON).restaged(newest, NOON, end));
    }

    /// A fetch held back ends at the start of the held minute's bucket.
    #[test]
    fn a_held_fetch_ends_at_a_bucket() {
        let history: History = (0..30).map(|i| (NOON + i, minute(10, 0, None))).collect();
        let mut fetch = Fetch::new(NOON, NOON, NOON + 30);
        fetch.hold(NOON + 12);
        assert_eq!(fetch.synced_through(), NOON + 9);
        let buckets = run(fetch, &history, NOON);
        assert_eq!(buckets.len(), 2);
        assert_eq!(buckets.last().map(|b| b.start), Some(NOON + 5));
    }

    // ==========================================================================
    // A night, as the watch revises it and Gadgetbridge fetches it
    // ==========================================================================

    /// xorshift64*, for each minute's evidence from a night's seed.
    struct Rng(u64);

    impl Rng {
        fn new(seed: u64) -> Self {
            Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
        }
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
        }
        fn range(&mut self, low: u16, high: u16) -> u16 {
            low + (self.next() % u64::from(high - low + 1)) as u16
        }
        fn chance(&mut self, percent: u64) -> bool {
            self.next() % 100 < percent
        }
    }

    /// What the wearer is doing (as in `host-tests/sleep`).
    #[derive(Clone, Copy, Debug, PartialEq)]
    enum Truth {
        Active,
        /// Still-ish, with fidgets: may or may not pass for sleep.
        Lounging,
        Asleep,
        DeepAsleep,
        Rem,
        /// Up in the night.
        Up,
        OffWrist,
        /// The watch switched off: minutes it never saw.
        Off,
    }

    /// A day and night: what the wearer does, in spans of minutes, and a
    /// seed for the evidence. Always ends with five hours up and about, or
    /// with the watch off the wrist, so that the night is settled by the
    /// end.
    #[derive(Clone, Debug)]
    struct Night {
        spans: Vec<(Truth, usize)>,
        seed: u64,
    }

    impl Arbitrary for Night {
        fn arbitrary(g: &mut Gen) -> Self {
            let length =
                |g: &mut Gen, low: usize, high: usize| low + usize::arbitrary(g) % (high - low + 1);
            // The evening: up and about, then maybe on the sofa.
            let mut spans = vec![
                (Truth::Active, length(g, 30, 230)),
                (Truth::Lounging, length(g, 0, 150)),
            ];
            // The night: stages, now and then interrupted by getting up,
            // taking the watch off, or the watch switching off.
            for _ in 0..length(g, 1, 10) {
                let stage = *g
                    .choose(&[Truth::Asleep, Truth::DeepAsleep, Truth::Rem])
                    .expect("not empty");
                spans.push((stage, length(g, 5, 90)));
                if u8::arbitrary(g) % 4 == 0 {
                    let interruption = *g
                        .choose(&[Truth::Up, Truth::OffWrist, Truth::Off])
                        .expect("not empty");
                    spans.push((interruption, length(g, 1, 80)));
                }
            }
            // The morning: maybe lying in for a while, then up for the day,
            // long enough for the night to settle; or taking the watch
            // off; or up for a while, then napping (or lying on the sofa)
            // before the night would have gone by itself, then up again.
            spans.push((Truth::Lounging, length(g, 0, 60)));
            match u8::arbitrary(g) % 3 {
                0 => spans.push((Truth::OffWrist, 300)),
                1 => {
                    spans.push((Truth::Active, length(g, 20, 90)));
                    let nap = *g
                        .choose(&[Truth::Lounging, Truth::Asleep, Truth::DeepAsleep])
                        .expect("not empty");
                    spans.push((nap, length(g, 5, 60)));
                    spans.push((Truth::Active, 300));
                }
                _ => spans.push((Truth::Active, 300)),
            }
            spans.retain(|&(_, length)| length > 0);
            Self {
                spans,
                seed: u64::arbitrary(g),
            }
        }
    }

    impl Night {
        /// Each minute's evidence and heart rate (0 if none).
        fn minutes(&self) -> Vec<(sleep::Minute, u8)> {
            let mut rng = Rng::new(self.seed);
            let mut minutes = Vec::new();
            for &(truth, length) in &self.spans {
                for _ in 0..length {
                    let (movement, steps) = match truth {
                        Truth::Active => (
                            rng.range(300, 2000),
                            if rng.chance(50) {
                                rng.range(20, 110)
                            } else {
                                0
                            },
                        ),
                        Truth::Lounging => (
                            if rng.chance(15) {
                                rng.range(250, 600)
                            } else {
                                rng.range(60, 200)
                            },
                            0,
                        ),
                        Truth::Asleep | Truth::Rem => (
                            if rng.chance(4) {
                                rng.range(200, 700)
                            } else {
                                rng.range(25, 145)
                            },
                            0,
                        ),
                        Truth::DeepAsleep => (rng.range(15, 45), 0),
                        Truth::Up => (rng.range(400, 1500), rng.range(0, 40)),
                        Truth::OffWrist | Truth::Off => (rng.range(18, 30), 0),
                    };
                    let bpm = match truth {
                        Truth::Active | Truth::Up => rng.range(75, 95),
                        Truth::Lounging => rng.range(60, 72),
                        Truth::Asleep => rng.range(52, 58),
                        Truth::DeepAsleep => rng.range(48, 52),
                        Truth::Rem => rng.range(58, 66),
                        Truth::OffWrist | Truth::Off => 0,
                    };
                    let measured = rng.chance(20);
                    minutes.push((
                        sleep::Minute {
                            seen: truth != Truth::Off,
                            movement,
                            steps,
                            worn: Some(truth != Truth::OffWrist),
                        },
                        if measured { bpm as u8 } else { 0 },
                    ));
                }
            }
            minutes
        }
    }

    /// The watch's history, as `activity` keeps and revises it every
    /// minute: each minute's evidence and heart rate, and its kind, which
    /// the classifier and the staging rewrite. Minute `i` is UTC minute
    /// `NOON + i`.
    struct Watch {
        truth: Vec<(sleep::Minute, u8)>,
        kinds: Vec<Class>,
        /// The minute under way.
        newest: usize,
        /// What the watch has settled (`activity::History::settled`).
        settled: Settled,
        /// Whether it settles at all: off, it's the watch before it did,
        /// for comparison.
        settling: bool,
    }

    impl Watch {
        fn new(truth: Vec<(sleep::Minute, u8)>) -> Self {
            let kinds = vec![Class::Unknown; truth.len()];
            Self {
                truth,
                kinds,
                newest: 0,
                settled: Settled::NONE,
                settling: true,
            }
        }

        /// Minute `t`'s evidence: half of it so far for the minute under
        /// way, as at its first poll, when the watch classifies.
        fn evidence(&self, t: usize) -> sleep::Minute {
            let (minute, _) = self.truth[t];
            if t == self.newest {
                sleep::Minute {
                    movement: minute.movement / 2,
                    steps: minute.steps / 2,
                    ..minute
                }
            } else {
                minute
            }
        }

        /// The next minute starts: `activity_task`'s once-a-minute pass
        /// (`History::classify_sleep`, `History::last_sleep`, then
        /// `History::settle`). Minute `t` is UTC minute `NOON + t`.
        fn tick(&mut self, now: usize) {
            self.newest = now;
            let utc = |t: usize| NOON + t as i64;
            let window = (now + 1).saturating_sub(sleep::WINDOW);
            let read = window.saturating_sub(sleep::CONTEXT);
            let rewrite = (self.settled.rewritten(utc(now)) - NOON).max(0) as usize;
            let before = if read == 0 {
                Class::Unknown
            } else {
                self.kinds[read - 1]
            };
            let evidence: Vec<_> = (read..=now).map(|t| self.evidence(t)).collect();
            let mut out = vec![Class::Unknown; evidence.len()];
            sleep::classify(before, &evidence, &mut out);
            self.kinds[rewrite..=now].copy_from_slice(&out[rewrite - read..]);

            let oldest = (now + 1).saturating_sub(DAY as usize);
            if let Some((start, end, _)) =
                sleep::last_sleep(now + 1 - oldest, |i| self.kinds[oldest + i])
                && self
                    .settled
                    .restaged(utc(now), utc(oldest + start), utc(oldest + end))
            {
                stages::stage(self, (oldest + start) as i64, (oldest + end) as i64);
            }

            let settles = settles(
                utc(now),
                self.settled,
                |minute| self.kind(minute),
                |minute| self.evidence_at(minute),
            );
            if settles.is_some() && self.settling {
                self.settled = Settled::before(utc(now));
            }
        }

        /// UTC minute `minute`'s kind, as the history has it.
        fn kind(&self, minute: i64) -> Class {
            match usize::try_from(minute - NOON) {
                Ok(t) if t <= self.newest => self.kinds[t],
                _ => Class::Unknown,
            }
        }

        /// UTC minute `minute`'s evidence, as the history has it.
        fn evidence_at(&self, minute: i64) -> sleep::Minute {
            match usize::try_from(minute - NOON) {
                Ok(t) if t <= self.newest => self.evidence(t),
                _ => sleep::Minute::default(),
            }
        }

        /// `activity::gadgetbridge_hold`.
        fn hold(&self) -> Option<Hold> {
            hold(
                NOON + self.newest as i64,
                self.settled,
                |minute| self.kind(minute),
                |minute| self.evidence_at(minute),
            )
        }

        /// The history as a fetch reads it (`activity::History::since`):
        /// the minutes with data, the one under way as far as it's got.
        fn history(&self) -> History {
            self.minutes((0..=self.newest).map(|t| (t, self.evidence(t))))
        }

        /// The history as recorded in the end: every minute whole.
        fn recorded(&self) -> History {
            self.minutes(self.truth.iter().enumerate().map(|(t, &(m, _))| (t, m)))
        }

        /// Those of `minutes` with data, as a fetch reads them.
        fn minutes(&self, minutes: impl Iterator<Item = (usize, sleep::Minute)>) -> History {
            minutes
                .map(|(t, m)| (t, m, self.truth[t].1))
                .filter(|&(_, m, bpm)| m.seen || bpm != 0)
                .map(|(t, m, bpm)| {
                    let minute = Minute {
                        steps: m.steps,
                        movement: m.movement,
                        heart_rate: bpm,
                        seen: m.seen,
                        kind: self.kinds[t].gadgetbridge_name(),
                    };
                    (NOON + t as i64, minute)
                })
                .collect()
        }
    }

    /// Staging works on the model's history in place, as on the watch's.
    impl stages::Record for Watch {
        fn minute(&self, minute: i64) -> (Class, Option<u16>, u8) {
            let Ok(t) = usize::try_from(minute) else {
                return (Class::Unknown, None, 0);
            };
            if t > self.newest {
                return (Class::Unknown, None, 0);
            }
            let m = self.evidence(t);
            (self.kinds[t], m.seen.then_some(m.movement), self.truth[t].1)
        }

        fn newest(&self) -> i64 {
            self.newest as i64
        }

        fn set_stage(&mut self, minute: i64, kind: Class) {
            self.kinds[minute as usize] = kind;
        }
    }

    /// What came of a night fetched from on a schedule.
    struct Fetched {
        watch: Watch,
        store: BTreeMap<i64, Stored>,
        /// Where the first fetch started, and the last one ended.
        from: i64,
        to: i64,
        /// Whether the last fetch held back a sleep.
        last_held_sleep: bool,
    }

    /// `night`, with Gadgetbridge fetching as in [`fetches`] (`gaps`
    /// apart, mostly a few minutes, now and then hours) as `ble` answers:
    /// from where the last fetch stopped (`system_off::synced_through`), or
    /// after what it asks if that's earlier (see [`start`]), and holding
    /// back what `sleep_hold` says. Then one more fetch at the
    /// end, and the watch runs on for another window, so that what it has
    /// then is final.
    fn fetch_night(night: &Night, gaps: &[(u16, u8, bool)]) -> Fetched {
        let mut watch = Watch::new(night.minutes());
        let length = watch.truth.len();
        let mut store = BTreeMap::new();
        let mut last_sync_seconds = None::<i64>;
        let mut synced = None::<i64>;
        let (mut first, mut to) = (None, i64::MIN);
        let mut last_held_sleep = false;
        // Fetch times, in seconds from the first minute.
        let mut at = Vec::new();
        let mut seconds = 0;
        for &(gap, second, ahead) in gaps {
            let minutes = match gap % 8 {
                0 => i64::from(gap % 400),
                _ => 1 + i64::from(gap % 30),
            };
            seconds += minutes * 60 + i64::from(second % 60);
            at.push((seconds, ahead));
        }
        let end = (length - sleep::WINDOW - sleep::CONTEXT - 1) as i64;
        at.retain(|&(seconds, _)| seconds.div_euclid(60) < end);
        at.push((end * 60 + 30, false));
        let mut at = at.into_iter().peekable();
        for now in 0..length {
            watch.tick(now);
            while let Some((seconds, ahead)) = at.next_if(|&(s, _)| s.div_euclid(60) == now as i64)
            {
                let now_minute = NOON + now as i64;
                let asked = asked_ms(last_sync_seconds, NOON * 60 + seconds, ahead);
                let hold = watch.hold();
                let mut fetch = Fetch::new(
                    start(asked, synced, now_minute),
                    now_minute - DAY + 1,
                    now_minute,
                );
                if let Some(hold) = hold {
                    fetch.hold(hold.from);
                }
                first.get_or_insert(fetch.from());
                synced = Some(fetch.synced_through());
                to = fetch.synced_through() + 1;
                let history = watch.history();
                for bucket in run(fetch, &history, now_minute - DAY + 1) {
                    send(&mut store, &bucket);
                }
                last_held_sleep = hold.is_some_and(|hold| hold.sleep);
                last_sync_seconds = Some(NOON * 60 + seconds);
            }
        }
        Fetched {
            watch,
            store,
            from: first.unwrap_or(0),
            to,
            last_held_sleep,
        }
    }

    /// A night slept through, then `morning`: when (in minutes after the
    /// night's last minute asleep) a fetch would first send all of the
    /// night, and checks that the watch ends up with the night it would
    /// have settled to by itself, and that once the night has gone, so
    /// does every minute after it as it comes, but for any held back by
    /// `held`, given the minute under way.
    fn released_after(seed: u64, morning: Truth, held: impl Fn(usize) -> bool) -> usize {
        let night = Night {
            spans: vec![
                (Truth::Active, 60),
                (Truth::Asleep, 60),
                (Truth::DeepAsleep, 90),
                (Truth::Rem, 30),
                (Truth::Asleep, 120),
                (Truth::Rem, 40),
                (Truth::Asleep, 60),
                (morning, 300),
            ],
            seed,
        };
        let mut watch = Watch::new(night.minutes());
        let mut unsettled = Watch::new(night.minutes());
        unsettled.settling = false;
        let mut released = None;
        for now in 0..watch.truth.len() {
            watch.tick(now);
            unsettled.tick(now);
            let hold = watch.hold();
            if released.is_none() && now > 60 + 400 && hold.is_none_or(|h| h.from > NOON + 460) {
                released = Some(now);
            }
            if released.is_some() && !held(now) {
                assert_eq!(hold, None, "seed {seed}: held at {now}");
            }
        }
        assert_eq!(watch.kinds, unsettled.kinds, "seed {seed}");
        let last = (0..watch.kinds.len())
            .rev()
            .find(|&t| unsettled.kinds[t].is_sleep())
            .expect("a night");
        released.expect("released") - last
    }

    /// A night slept through, then up and about: the night is no longer
    /// held back once the wearer has been up for longer than
    /// `SAME_NIGHT_GAP` minutes, rather than for 135; nor are the
    /// minutes after it, which go as they come; and it's the night the
    /// watch would have settled to by itself.
    #[test]
    fn a_night_goes_soon_after_getting_up() {
        for seed in 0..50 {
            let after = released_after(seed, Truth::Active, |_| false);
            assert_eq!(after, sleep::SAME_NIGHT_GAP + 2, "seed {seed}");
        }
    }

    /// The same with the watch taken off on waking (to charge it, say),
    /// without walking first: off the wrist counts as up.
    #[test]
    fn a_night_goes_soon_after_taking_the_watch_off() {
        for seed in 0..50 {
            let after = released_after(seed, Truth::OffWrist, |_| false);
            assert_eq!(after, sleep::SAME_NIGHT_GAP + 2, "seed {seed}");
        }
    }

    /// A watch off the wrist in the night for less than a wake that ends
    /// a night (slipped off, or the heart rate sensor wrong for a while)
    /// settles nothing: the night still goes whole in the morning.
    #[test]
    fn off_the_wrist_in_the_night_settles_nothing() {
        for (seed, off) in (0..30).zip([1, 5, 20, 40, 44].into_iter().cycle()) {
            let night = Night {
                spans: vec![
                    (Truth::Active, 60),
                    (Truth::DeepAsleep, 120),
                    (Truth::OffWrist, off),
                    (Truth::Asleep, 120),
                    (Truth::Active, 300),
                ],
                seed,
            };
            let mut watch = Watch::new(night.minutes());
            for now in 0..60 + 240 + off {
                watch.tick(now);
                assert_eq!(watch.settled, Settled::NONE, "seed {seed}, off {off}");
            }
        }
    }

    /// The property the hold is for: whenever Gadgetbridge fetches, over a
    /// day and a night, what it ends up with is the night as the watch
    /// settles it. Each stored sample's kind (which Gadgetbridge never
    /// updates) is the watch's final one, as are its steps, heart rate and
    /// movement; and it has every step from where the first fetch started
    /// to where the last ended, the night included.
    #[test]
    fn gadgetbridge_gets_the_settled_night() {
        fn property(night: Night, gaps: Vec<(u16, u8, bool)>) -> bool {
            let fetched = fetch_night(&night, &gaps);
            let full = fetched.watch.recorded();
            let wrong: Vec<_> = fetched
                .store
                .iter()
                .filter(|&(&timestamp, stored)| {
                    let start = start_of(timestamp.div_euclid(60));
                    reference(&full, start).is_none_or(|b| {
                        b.timestamp_ms() / 1000 != timestamp
                            || stored.steps != b.steps
                            || stored.heart_rate != b.heart_rate
                            || stored.intensity != b.movement
                            || stored.kind != b.kind
                    })
                })
                .collect();
            let steps = fetched.store.values().map(|s| s.steps).sum::<u32>();
            let expected = steps_in(&full, fetched.from, fetched.to);
            if !wrong.is_empty() || steps != expected || fetched.last_held_sleep {
                eprintln!(
                    "{night:?}\n{gaps:?}\n{} wrong, e.g. {:?}; {steps} steps of {expected}; held at the end: {}",
                    wrong.len(),
                    wrong.first(),
                    fetched.last_held_sleep,
                );
                return false;
            }
            true
        }
        QuickCheck::new()
            .tests(1_000)
            .quickcheck(property as fn(Night, Vec<(u16, u8, bool)>) -> bool);
    }
}

/// Proofs, for `cargo kani`: what the tests above check for the histories
/// quickcheck makes, for every history of a few minutes, wherever a fetch
/// starts and ends.
///
/// Minutes are within ±2^40 of 1970 (two million years): the bucket
/// arithmetic adds a bucket's length unchecked, and would overflow at the
/// ends of an `i64`.
#[cfg(kani)]
mod proofs {
    use super::gb_buckets::{Fetch, MINUTES, Minute, start_of, synced_after_set};

    fn any_minute() -> i64 {
        let minute: i64 = kani::any();
        kani::assume((-(1 << 40)..1 << 40).contains(&minute));
        minute
    }

    /// For every minute, before 1970 too: its bucket starts on a
    /// multiple of the bucket's length, and holds it.
    #[kani::proof]
    fn a_minute_is_in_its_bucket() {
        let minute = any_minute();
        let start = start_of(minute);
        assert!(start.rem_euclid(MINUTES) == 0);
        assert!(start <= minute && minute < start + MINUTES);
    }

    /// For every fetch (asked from any minute, of a history from any
    /// minute, at any time, with or without minutes held back): it
    /// starts at a bucket's start, no earlier than the bucket asked for,
    /// nor than the history's oldest minute (so never at a bucket the
    /// history has only the end of), or if that's after the bucket under
    /// way (a clock put right), than that bucket; and the phone's copy
    /// ends just before a bucket's start (`synced_through`): never before
    /// where the fetch started (what's before that the phone has), and
    /// past it, no later than now, nor than the first minute held back,
    /// nor a whole bucket short of the sooner of those.
    #[kani::proof]
    fn a_fetch_is_of_whole_buckets() {
        let (since, oldest, now) = (any_minute(), any_minute(), any_minute());
        let mut fetch = Fetch::new(since, oldest, now);
        let start = fetch.from();
        assert!(start.rem_euclid(MINUTES) == 0);
        assert!(start >= start_of(since) && start >= oldest.min(start_of(now)));
        assert!(start < since.max(oldest) + MINUTES);
        // The first minute the phone won't have.
        let lacks = |fetch: &Fetch| fetch.synced_through() + 1;
        assert_eq!(lacks(&fetch), start_of(now).max(start));
        if kani::any() {
            let hold = any_minute();
            fetch.hold(hold);
            let lacks = lacks(&fetch);
            assert!(lacks.rem_euclid(MINUTES) == 0);
            assert!(lacks >= start);
            if lacks > start {
                assert!(lacks <= hold && lacks <= now);
                assert!(lacks > hold.min(now) - MINUTES);
            }
        }
    }

    /// For every sync point and every time the phone sets the clock to:
    /// the sync point is then before that minute, and where it already
    /// was, it stays.
    #[kani::proof]
    fn a_clock_set_puts_the_sync_point_before_it() {
        let (synced, now) = (any_minute(), any_minute());
        let after = synced_after_set(Some(synced), now).expect("kept");
        assert!(after < now);
        assert!(after == synced || synced >= now);
        assert!(synced_after_set(None, now).is_none());
    }

    /// 2026-11-09 12:00 UTC, in minutes since 1970: a bucket's start.
    const NOON: i64 = 29_903_760;
    const _: () = assert!(NOON % MINUTES == 0);

    /// The minutes with data that the next proof's history has, after
    /// [`NOON`]: most of one bucket (its first minute missing, and one
    /// in the middle), and the start of the next, after a gap.
    const HISTORY: [i64; 6] = [1, 2, 4, 5, 6, 8];

    /// For a history of those minutes, each with any steps, movement,
    /// heart rate and one of three kinds, and every fetch that starts
    /// and ends within the quarter of an hour around them: the next
    /// bucket never panics (each minute's place in the bucket is within
    /// it), and
    ///
    /// - it's the bucket of the first minute from where the fetch has
    ///   got to, whole (on a bucket's start, over before the fetch's
    ///   end), and the fetch moves on to the bucket after;
    /// - its steps are the sum of its minutes', its heart rate the last
    ///   one measured, its movement the mean over the minutes the
    ///   accelerometer recorded, rounded up, and its kind one that most
    ///   of its minutes have;
    /// - there's none only when the history has no minute left in a
    ///   bucket that's over.
    ///
    /// The minutes' times are fixed, and the fetch's within a few
    /// buckets of them: with every time free, CBMC went past 7 GB (the
    /// bucket arithmetic for any minute is the two proofs above; the
    /// tests' quickcheck varies the history's shape).
    #[kani::proof]
    #[kani::unwind(8)]
    fn a_bucket_is_its_minutes() {
        let minutes = HISTORY.map(|after| {
            let sample = Minute {
                steps: kani::any(),
                movement: kani::any(),
                heart_rate: kani::any(),
                seen: kani::any(),
                kind: kani::any::<u8>(),
            };
            kani::assume(sample.kind < 3);
            (NOON + after, sample)
        });
        // Asked from, and at, any minute from a bucket before the
        // history to two after it; the history reaches back a day.
        let (since, now): (i8, i8) = kani::any();
        kani::assume((-5..15).contains(&since) && (-5..15).contains(&now));
        let mut fetch = Fetch::new(
            NOON + i64::from(since),
            NOON - 24 * 60,
            NOON + i64::from(now),
        );
        // `end`: the first minute the phone won't have, the bucket under
        // way's start (or the fetch's own, if that's later: then it sends
        // nothing, and the asserts below hold for either).
        let (from, end) = (fetch.from(), fetch.synced_through() + 1);
        let first = minutes.iter().find(|(minute, _)| *minute >= from);

        let Some(bucket) = fetch.next(minutes) else {
            assert!(first.is_none_or(|(minute, _)| start_of(*minute) >= end));
            return;
        };
        let Some((first, _)) = first else {
            unreachable!("a bucket has a minute");
        };
        assert_eq!(bucket.start, start_of(*first));
        assert!(bucket.start >= from && bucket.start + MINUTES <= end);
        assert_eq!(fetch.from(), bucket.start + MINUTES);

        let within = |minute: i64| bucket.start <= minute && minute < bucket.start + MINUTES;
        let (mut steps, mut heart_rate, mut movement, mut seen) = (0u32, 0u8, 0u32, 0u32);
        let mut votes = [0u8; 3];
        for (minute, sample) in minutes {
            if !within(minute) {
                continue;
            }
            steps += u32::from(sample.steps);
            if sample.heart_rate != 0 {
                heart_rate = sample.heart_rate;
            }
            if sample.seen {
                movement += u32::from(sample.movement);
                seen += 1;
            }
            votes[usize::from(sample.kind)] += 1;
        }
        assert_eq!(bucket.steps, steps);
        assert_eq!(bucket.heart_rate, heart_rate);
        let mean = if seen == 0 {
            0
        } else {
            movement.div_ceil(seen)
        };
        assert_eq!(u32::from(bucket.movement), mean);
        let most = votes[usize::from(bucket.kind)];
        assert!(most > 0 && votes.iter().all(|&votes| votes <= most));
    }
}
