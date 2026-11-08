//! Host tests for the firmware's `sleep` module, on synthetic days and
//! nights.
//!
//! `cargo run` prints each scenario's timeline (one character per minute);
//! `cargo test` asserts on them. `cargo run -- <decoded field log>` replays
//! a real day from a field log instead (see `replay`).
//!
//! The movement levels are guesses in the firmware's units (~25 on a desk,
//! hundreds awake), like the classifier's thresholds. These tests check the
//! *logic* (onset, waking, stirs, deep stretches, not-worn, and that
//! classifying a trailing window every minute agrees with classifying the
//! whole night at once), not that the thresholds match real wrists; field
//! logs decide that.

#[path = "../../../src/sleep.rs"]
#[allow(dead_code)]
mod sleep;

use sleep::{Kind, Minute};

// ==============================================================================
// Synthetic minutes
// ==============================================================================

/// xorshift64*, so runs are reproducible without dependencies.
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
    fn chance(&mut self, p: f64) -> bool {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64 <= p
    }
}

/// What the wearer is doing, minute by minute.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Truth {
    Active,
    Sitting,
    /// Watching TV: still-ish, with fidgets.
    Lounging,
    Asleep,
    /// A long, very still stretch of sleep.
    DeepAsleep,
    /// Up in the night.
    Up,
    OffWrist,
}

fn minute(rng: &mut Rng, truth: Truth) -> Minute {
    let (movement, steps) = match truth {
        Truth::Active => (
            rng.range(300, 2000),
            if rng.chance(0.5) {
                rng.range(20, 110)
            } else {
                0
            },
        ),
        Truth::Sitting => (rng.range(120, 400), 0),
        Truth::Lounging => (
            if rng.chance(0.15) {
                rng.range(250, 600)
            } else {
                rng.range(90, 220)
            },
            0,
        ),
        // A stir now and then: a minute or two of rolling over.
        Truth::Asleep => (
            if rng.chance(0.04) {
                rng.range(200, 700)
            } else {
                rng.range(25, 105)
            },
            0,
        ),
        Truth::DeepAsleep => (rng.range(15, 45), 0),
        Truth::Up => (rng.range(400, 1500), rng.range(10, 40)),
        Truth::OffWrist => (rng.range(18, 30), 0),
    };
    Minute {
        seen: true,
        movement,
        steps,
        worn: Some(truth != Truth::OffWrist),
    }
}

/// A day from 18:00 to 18:00, as (truth, minutes) spans.
fn build(rng: &mut Rng, spans: &[(Truth, usize)]) -> (Vec<Truth>, Vec<Minute>) {
    let mut truth = Vec::new();
    let mut minutes = Vec::new();
    for &(kind, length) in spans {
        for _ in 0..length {
            truth.push(kind);
            minutes.push(minute(rng, kind));
        }
    }
    (truth, minutes)
}

fn a_night(rng: &mut Rng) -> (Vec<Truth>, Vec<Minute>) {
    build(
        rng,
        &[
            (Truth::Active, 120),   // 18:00 dinner, chores
            (Truth::Lounging, 150), // 20:00 TV
            (Truth::Sitting, 30),   // 22:30 reading
            (Truth::Asleep, 60),    // 23:00 asleep
            (Truth::DeepAsleep, 70),
            (Truth::Asleep, 110),
            (Truth::Up, 6), // 03:00 bathroom
            (Truth::Asleep, 50),
            (Truth::DeepAsleep, 45),
            (Truth::Asleep, 140), // until 07:00
            (Truth::Active, 300),
            (Truth::Sitting, 360),
            (Truth::Active, 39),
        ],
    )
}

// ==============================================================================
// Running the classifier
// ==============================================================================

/// Classifies everything at once.
fn batch(minutes: &[Minute]) -> Vec<Kind> {
    let mut out = vec![Kind::Unknown; minutes.len()];
    sleep::classify(Kind::Unknown, minutes, &mut out);
    out
}

/// Classifies as the firmware does: every minute, the trailing window and
/// its context, seeded with the (final) class of the minute before them,
/// writing back only the window.
fn incremental(minutes: &[Minute]) -> Vec<Kind> {
    let mut kinds = vec![Kind::Unknown; minutes.len()];
    for now in 0..minutes.len() {
        let window = (now + 1).saturating_sub(sleep::WINDOW);
        let start = window.saturating_sub(sleep::CONTEXT);
        let before = if start == 0 {
            Kind::Unknown
        } else {
            kinds[start - 1]
        };
        let mut out = vec![Kind::Unknown; now + 1 - start];
        sleep::classify(before, &minutes[start..=now], &mut out);
        kinds[window..=now].copy_from_slice(&out[window - start..]);
    }
    kinds
}

/// What the firmware's `activity::asleep()` says each minute: whether the
/// newest minute of that minute's pass (the leading edge, where unfinished
/// movement is still restless) is sleep.
fn leading_edge(minutes: &[Minute]) -> Vec<bool> {
    let mut kinds = vec![Kind::Unknown; minutes.len()];
    let mut asleep = Vec::with_capacity(minutes.len());
    for now in 0..minutes.len() {
        let window = (now + 1).saturating_sub(sleep::WINDOW);
        let start = window.saturating_sub(sleep::CONTEXT);
        let before = if start == 0 {
            Kind::Unknown
        } else {
            kinds[start - 1]
        };
        let mut out = vec![Kind::Unknown; now + 1 - start];
        sleep::classify(before, &minutes[start..=now], &mut out);
        kinds[window..=now].copy_from_slice(&out[window - start..]);
        asleep.push(kinds[now].is_sleep());
    }
    asleep
}

/// Runs [`sleep::Night`] over a day of `asleep` verdicts (one a minute,
/// from 18:00 as in [`a_night`]), with an alarm ringing at minute `alarm`
/// if given. Returns whether each minute was quiet.
fn nights(asleep: &[bool], alarm: Option<usize>) -> Vec<bool> {
    // Local seconds of 18:00 on some day.
    const EVENING: i64 = 20_000 * 24 * 60 * 60 + 18 * 60 * 60;
    let mut night = sleep::Night::default();
    let mut quiet = Vec::with_capacity(asleep.len());
    for (t, &asleep) in asleep.iter().enumerate() {
        let local = EVENING + t as i64 * 60;
        let noon = t > 0 && sleep::noon_between(local - 60, local);
        night.update(asleep, t as u32 * 60, noon, sleep::nap_hours(Some(local)));
        if alarm == Some(t) {
            night.alarm();
        }
        quiet.push(night.quiet());
    }
    quiet
}

fn timeline(kinds: &[Kind]) -> String {
    kinds
        .iter()
        .map(|k| match k {
            Kind::Unknown => ' ',
            Kind::Awake => '.',
            Kind::Light => 'l',
            Kind::Deep => 'D',
            Kind::Restless => 'r',
            Kind::NotWorn => 'x',
        })
        .collect()
}

fn asleep_minutes(kinds: &[Kind]) -> usize {
    kinds
        .iter()
        .filter(|k| matches!(k, Kind::Light | Kind::Deep))
        .count()
}

// ==============================================================================
// Report
// ==============================================================================

/// Replays a decoded field log (`defmt-print` output): rebuilds each
/// minute's movement, steps, and worn verdict from the accelerometer and
/// heart rate lines, and classifies them, to tune the thresholds on real
/// nights. Uptimes become wall-clock time via the `clock:` lines.
fn replay(path: &str) {
    let text = std::fs::read_to_string(path).expect("a readable log");
    // (uptime seconds, UTC seconds) anchors, from `clock: <utc> UTC = ...`.
    let mut anchor: Option<(f64, i64)> = None;
    let mut minutes: std::collections::BTreeMap<i64, (u64, u64, u16, Option<bool>)> =
        Default::default();
    let mut worn: Option<bool> = None;
    for line in text.lines() {
        let Some((uptime, rest)) = line.split_once(' ') else {
            continue;
        };
        let Ok(uptime) = uptime.parse::<f64>() else {
            continue;
        };
        let message = rest.trim_start_matches(|c: char| c.is_ascii_uppercase() || c == ' ');
        if message.contains("booting") {
            anchor = None; // uptime restarts
        }
        if let Some(rest) = message.strip_prefix("clock: ") {
            if let Some(utc) = rest.split(' ').next().and_then(|u| u.parse::<i64>().ok()) {
                anchor = Some((uptime, utc));
            }
            continue;
        }
        if message.starts_with("heart: ") {
            if message.contains(" BPM") && !message.contains("window") {
                worn = Some(true);
            } else if message.contains("not on a wrist") || message.contains("too much light") {
                worn = Some(false);
            } else if message.contains("no reading") {
                worn = None;
            }
            continue;
        }
        // "activity: 36 samples, movement 1653, 0 new steps (counter 0)"
        // Only the per-batch lines: warnings such as "activity: FIFO out of
        // step (header 0x84 at 247)" carry numbers too, which would read as
        // 247 steps.
        let Some(rest) = message
            .strip_prefix("activity: ")
            .filter(|rest| rest.contains(" samples, movement "))
        else {
            continue;
        };
        let numbers: Vec<u64> = rest
            .split(|c: char| !c.is_ascii_digit())
            .filter(|s| !s.is_empty())
            .filter_map(|s| s.parse().ok())
            .collect();
        let [samples, movement, steps, ..] = numbers[..] else {
            continue;
        };
        let Some((at, utc)) = anchor else { continue };
        let minute = (utc + (uptime - at) as i64).div_euclid(60);
        let entry = minutes.entry(minute).or_insert((0, 0, 0, None));
        entry.0 += movement * samples;
        entry.1 += samples;
        // Saturating, as the firmware's history does.
        entry.2 = entry
            .2
            .saturating_add(steps.min(u64::from(u16::MAX)) as u16);
        entry.3 = worn;
    }
    let (Some(&first), Some(&last)) = (minutes.keys().next(), minutes.keys().last()) else {
        println!("no activity lines with wall-clock anchors in {path}");
        return;
    };
    let evidence: Vec<Minute> = (first..=last)
        .map(|m| match minutes.get(&m) {
            Some(&(weighted, samples, steps, worn)) => Minute {
                seen: true,
                movement: (weighted / samples.max(1)).min(u64::from(u16::MAX)) as u16,
                steps,
                worn,
            },
            None => Minute::default(),
        })
        .collect();
    let kinds = incremental(&evidence);
    println!(
        "{} minutes; asleep {}; last sleep {:?}",
        kinds.len(),
        asleep_minutes(&kinds),
        sleep::last_sleep(&kinds)
    );
    // One row per hour, labelled in UTC.
    for (hour, row) in timeline(&kinds).as_bytes().chunks(60).enumerate() {
        let start = (first + hour as i64 * 60) * 60;
        println!(
            "  {:02}:{:02}Z {}",
            start.rem_euclid(86_400) / 3600,
            start.rem_euclid(3600) / 60,
            std::str::from_utf8(row).unwrap()
        );
    }
}

fn main() {
    if let Some(path) = std::env::args().nth(1) {
        return replay(&path);
    }
    for seed in 0..3 {
        let mut rng = Rng::new(seed);
        let (truth, minutes) = a_night(&mut rng);
        let kinds = incremental(&minutes);
        if std::env::var("BATCH").is_ok() {
            let b = batch(&minutes);
            for (hour, row) in timeline(&b)
                .as_bytes()
                .chunks(60)
                .enumerate()
                .skip(4)
                .take(5)
            {
                println!(
                    "B {:02}:00 {}",
                    (18 + hour) % 24,
                    std::str::from_utf8(row).unwrap()
                );
            }
            let t: String = truth
                .iter()
                .map(|t| match t {
                    Truth::DeepAsleep => 'D',
                    Truth::Asleep => 'l',
                    Truth::Up => 'U',
                    _ => '.',
                })
                .collect();
            for (hour, row) in t.as_bytes().chunks(60).enumerate().skip(4).take(5) {
                println!(
                    "T {:02}:00 {}",
                    (18 + hour) % 24,
                    std::str::from_utf8(row).unwrap()
                );
            }
        }
        let real = truth
            .iter()
            .filter(|t| matches!(t, Truth::Asleep | Truth::DeepAsleep))
            .count();
        println!(
            "seed {seed}: asleep {} min (truth {real}), last sleep {:?}",
            asleep_minutes(&kinds),
            sleep::last_sleep(&kinds)
        );
        for (hour, row) in timeline(&kinds).as_bytes().chunks(60).enumerate() {
            println!(
                "  {:02}:00 {}",
                (18 + hour) % 24,
                std::str::from_utf8(row).unwrap()
            );
        }
    }
}

// ==============================================================================
// Tests
// ==============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// The night is found, its length is about right, and the day isn't
    /// mistaken for sleep.
    #[test]
    fn a_night_is_measured() {
        for seed in 0..20 {
            let mut rng = Rng::new(seed);
            let (truth, minutes) = a_night(&mut rng);
            let kinds = incremental(&minutes);
            let real = truth
                .iter()
                .filter(|t| matches!(t, Truth::Asleep | Truth::DeepAsleep))
                .count();
            let (start, end, asleep) = sleep::last_sleep(&kinds).expect("a night");
            // Onset is dated back to the start of the stillness; the last
            // reading minutes before sleep are still enough to blur it.
            assert!(
                asleep.abs_diff(real) <= 30,
                "seed {seed}: asleep {asleep}, truth {real}\n{}",
                timeline(&kinds)
            );
            // One night, bathroom trip included: 23:00 to 07:00.
            assert!(start.abs_diff(300) <= 30, "seed {seed}: starts at {start}");
            assert!(end.abs_diff(781) <= 10, "seed {seed}: ends at {end}");
            // No sleep in the daytime or evening.
            let day = kinds[..270].iter().chain(&kinds[800..]);
            assert!(
                !day.clone().any(|k| k.is_sleep()),
                "seed {seed}:\n{}",
                timeline(&kinds)
            );
            // Some deep sleep, during the very still stretches.
            assert!(kinds.contains(&Kind::Deep), "seed {seed}");
        }
    }

    /// Getting up in the night is awake, not sleep.
    #[test]
    fn getting_up_wakes() {
        let mut rng = Rng::new(7);
        let (truth, minutes) = a_night(&mut rng);
        let kinds = incremental(&minutes);
        for (t, (truth, kind)) in truth.iter().zip(&kinds).enumerate() {
            if *truth == Truth::Up {
                assert_eq!(*kind, Kind::Awake, "minute {t}");
            }
        }
    }

    /// Classifying a trailing window every minute ends up where classifying
    /// the whole day at once does, apart from a few minutes at boundaries.
    #[test]
    fn incremental_matches_batch() {
        for seed in 0..20 {
            let mut rng = Rng::new(seed);
            let (_, minutes) = a_night(&mut rng);
            let (a, b) = (incremental(&minutes), batch(&minutes));
            let differ = a.iter().zip(&b).filter(|(x, y)| x != y).count();
            assert!(
                differ <= 10,
                "seed {seed}: {differ} minutes differ\n{}\n{}",
                timeline(&a),
                timeline(&b)
            );
        }
    }

    /// An evening of TV, however still, isn't sleep.
    #[test]
    fn lounging_is_not_sleep() {
        for seed in 0..20 {
            let mut rng = Rng::new(seed);
            let (_, minutes) = build(&mut rng, &[(Truth::Lounging, 240)]);
            let kinds = incremental(&minutes);
            assert!(
                !kinds.iter().any(|k| k.is_sleep()),
                "seed {seed}:\n{}",
                timeline(&kinds)
            );
        }
    }

    /// A watch on the nightstand isn't a sleeping wearer.
    #[test]
    fn off_the_wrist_is_not_worn() {
        let mut rng = Rng::new(1);
        let (_, minutes) = build(
            &mut rng,
            &[
                (Truth::Active, 30),
                (Truth::OffWrist, 300),
                (Truth::Active, 30),
            ],
        );
        let kinds = incremental(&minutes);
        assert!(kinds[30..330].iter().all(|k| *k == Kind::NotWorn));
        assert!(!kinds.iter().any(|k| k.is_sleep()));
    }

    /// Without the heart rate sensor's verdict (it hasn't looked yet), a
    /// motionless watch reads as sleep: a known limitation, pinned here so
    /// a change is noticed.
    #[test]
    fn off_the_wrist_without_heart_rate_looks_like_sleep() {
        let mut rng = Rng::new(1);
        let (_, mut minutes) = build(&mut rng, &[(Truth::OffWrist, 120)]);
        for m in &mut minutes {
            m.worn = None;
        }
        let kinds = incremental(&minutes);
        assert!(kinds.iter().any(|k| k.is_sleep()));
    }

    /// Minutes the watch didn't see stay unknown, and don't count.
    #[test]
    fn gaps_are_unknown() {
        let mut rng = Rng::new(3);
        let (_, mut minutes) = a_night(&mut rng);
        for m in &mut minutes[400..460] {
            *m = Minute::default();
        }
        let kinds = incremental(&minutes);
        assert!(kinds[400..460].iter().all(|k| *k == Kind::Unknown));
    }

    // ==========================================================================
    // The quiet night
    // ==========================================================================

    /// `asleep()` (the leading edge, restless counting as sleep) holds
    /// through a night's stirs: it turns false on getting up, and on the odd
    /// run of [`sleep::WAKE_RUN`] restless minutes. When only light and deep
    /// sleep counted, these synthetic nights flipped it 6 to 18 times.
    #[test]
    fn asleep_holds_through_stirs() {
        for seed in 0..20 {
            let mut rng = Rng::new(seed);
            let (truth, minutes) = a_night(&mut rng);
            let asleep = leading_edge(&minutes);
            // From well into the night (onset takes 20 minutes) to 07:00.
            let wakes = (330..781)
                .filter(|&t| asleep[t - 1] && !asleep[t])
                .collect::<Vec<_>>();
            let up = truth.iter().position(|t| *t == Truth::Up).expect("a trip");
            assert!(
                wakes.len() <= 4 && wakes.iter().any(|w| w.abs_diff(up) <= 5),
                "seed {seed}: wakes at {wakes:?}, up at {up}"
            );
        }
    }

    /// A night from the classifier: quiet from falling asleep, through the
    /// bathroom trip, until [`sleep::QUIET_AWAKE`] after getting up in
    /// the morning, and not again during the day.
    #[test]
    fn a_night_is_quiet() {
        for seed in 0..20 {
            let mut rng = Rng::new(seed);
            let (_, minutes) = a_night(&mut rng);
            let asleep = leading_edge(&minutes);
            let quiet = nights(&asleep, None);
            let start = quiet.iter().position(|&q| q).expect("a quiet night");
            let end = quiet.iter().rposition(|&q| q).expect("an end") + 1;
            // Asleep at 23:00 (300), recognised 20 minutes later.
            assert!((300..=330).contains(&start), "seed {seed}: from {start}");
            // Quiet whenever asleep in between. (The classifier can wake
            // for longer than `QUIET_AWAKE` within the night, which ends
            // the quiet; falling asleep again starts it again.)
            assert!(
                (start..end).all(|t| quiet[t] || !asleep[t]),
                "seed {seed}: asleep but not quiet"
            );
            // Up at 07:00 (781), and `QUIET_AWAKE` minutes awake after that.
            let gap = sleep::QUIET_AWAKE;
            assert!(
                (781 + gap..=781 + gap + 5).contains(&end),
                "seed {seed}: until {end}"
            );
            assert!(!quiet[end..].contains(&true), "seed {seed}: quiet again");
        }
    }

    /// An alarm ends the night, even if the wearer sleeps on through it.
    #[test]
    fn an_alarm_ends_the_night() {
        for seed in 0..20 {
            let mut rng = Rng::new(seed);
            let (_, minutes) = a_night(&mut rng);
            // 06:45, still asleep.
            let quiet = nights(&leading_edge(&minutes), Some(765));
            assert!(quiet[764], "seed {seed}");
            assert!(!quiet[765..].contains(&true), "seed {seed}");
        }
    }

    /// A watch the classifier thinks is asleep all day (still, off the
    /// wrist, before the heart rate sensor's verdict) is quiet until noon,
    /// and then not again.
    #[test]
    fn noon_ends_a_night_that_never_wakes() {
        let mut rng = Rng::new(1);
        let (_, mut minutes) = build(
            &mut rng,
            &[(Truth::Active, 240), (Truth::OffWrist, 24 * 60 - 240)],
        );
        for m in &mut minutes {
            m.worn = None;
        }
        let asleep = leading_edge(&minutes);
        assert!(asleep[1080..].iter().all(|&a| a), "asleep all morning");
        let quiet = nights(&asleep, None);
        // 22:00 plus onset, until noon (18 hours after 18:00).
        assert!(quiet[300..1080].iter().all(|&q| q));
        assert!(!quiet[1080..].contains(&true));
    }

    /// Minute by minute: being awake ends the night only after more than
    /// [`sleep::QUIET_AWAKE`] minutes, and only falling asleep (again)
    /// starts it.
    #[test]
    fn night_rules() {
        use sleep::{Change, Night, Over};
        let gap = sleep::QUIET_AWAKE as u32 * 60;
        let mut night = Night::default();
        assert!(!night.quiet(), "not quiet after a reset");
        assert_eq!(night.update(false, 0, false, false), None);
        assert_eq!(
            night.update(true, 60, false, false),
            Some(Change::Quiet { nap: false })
        );
        // Up for a while in the night, and back to sleep.
        assert_eq!(night.update(false, 120, false, false), None);
        assert_eq!(night.update(false, 60 + gap, false, false), None);
        assert_eq!(night.update(true, 120 + gap, false, false), None);
        assert!(night.quiet());
        // Up for good.
        let up = 180 + gap;
        assert_eq!(night.update(false, up, false, false), None);
        assert_eq!(night.update(false, up + gap - 60, false, false), None);
        assert_eq!(
            night.update(false, up + gap, false, false),
            Some(Change::Over(Over::Awake))
        );
        // Asleep again: a new night, which an alarm ends.
        assert_eq!(
            night.update(true, 100_000, false, false),
            Some(Change::Quiet { nap: false })
        );
        assert!(night.alarm());
        assert!(!night.alarm(), "already over");
        // Still asleep after the alarm: no new night until awake and
        // asleep again.
        assert_eq!(night.update(true, 100_060, false, false), None);
        assert_eq!(night.update(false, 100_120, false, false), None);
        assert_eq!(
            night.update(true, 100_180, false, false),
            Some(Change::Quiet { nap: false })
        );
        // Noon ends it, asleep or not.
        assert_eq!(
            night.update(true, 100_240, true, false),
            Some(Change::Over(Over::Noon))
        );
        assert_eq!(night.update(true, 100_300, false, false), None);
    }

    /// A nap (falling asleep in the daytime) ends after [`sleep::NAP_AWAKE`]
    /// minutes awake, not [`sleep::QUIET_AWAKE`]; the night after it
    /// doesn't inherit that.
    #[test]
    fn naps_end_sooner() {
        use sleep::{Change, Night, Over};
        let nap = sleep::NAP_AWAKE as u32 * 60;
        let mut night = Night::default();
        assert_eq!(
            night.update(true, 60, false, true),
            Some(Change::Quiet { nap: true })
        );
        assert_eq!(night.update(false, 120, false, true), None);
        assert_eq!(night.update(false, 120 + nap - 60, false, true), None);
        assert_eq!(
            night.update(false, 120 + nap + 60, false, true),
            Some(Change::Over(Over::Awake))
        );
        // A night, later: the longer limit.
        assert_eq!(
            night.update(true, 50_000, false, false),
            Some(Change::Quiet { nap: false })
        );
        assert_eq!(night.update(false, 50_060, false, false), None);
        assert_eq!(night.update(false, 50_060 + nap + 60, false, false), None);
        assert!(night.quiet());
    }

    #[test]
    fn nap_hours_are_daytime() {
        const DAY: i64 = 24 * 60 * 60;
        let at = |hh: i64, mm: i64| Some(20_000 * DAY + (hh * 60 + mm) * 60);
        assert!(!sleep::nap_hours(at(8, 59)));
        assert!(sleep::nap_hours(at(9, 0)));
        assert!(sleep::nap_hours(at(15, 30)));
        assert!(!sleep::nap_hours(at(20, 0)));
        assert!(!sleep::nap_hours(at(1, 25)));
        assert!(!sleep::nap_hours(None), "unknown time is a night");
    }

    #[test]
    fn noon_is_found() {
        const DAY: i64 = 24 * 60 * 60;
        let at = |day: i64, hh: i64, mm: i64| day * DAY + (hh * 60 + mm) * 60;
        assert!(sleep::noon_between(at(5, 11, 59), at(5, 12, 0)));
        assert!(!sleep::noon_between(at(5, 12, 0), at(5, 12, 1)));
        assert!(!sleep::noon_between(at(5, 13, 0), at(5, 23, 0)));
        assert!(!sleep::noon_between(at(5, 23, 0), at(6, 11, 0)));
        // Over a gap (say, the phone setting the clock), still found.
        assert!(sleep::noon_between(at(5, 23, 0), at(6, 13, 0)));
        // Before 1970, too.
        assert!(sleep::noon_between(at(-3, 11, 0), at(-3, 12, 30)));
    }
}
