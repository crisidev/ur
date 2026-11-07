//! Host tests for the firmware's `sleep` module, on synthetic days and
//! nights.
//!
//! `cargo run` prints each scenario's timeline (one character per minute);
//! `cargo test` asserts on them.
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
        Truth::Active => (rng.range(300, 2000), if rng.chance(0.5) { rng.range(20, 110) } else { 0 }),
        Truth::Sitting => (rng.range(120, 400), 0),
        Truth::Lounging => (
            if rng.chance(0.15) { rng.range(250, 600) } else { rng.range(90, 220) },
            0,
        ),
        // A stir now and then: a minute or two of rolling over.
        Truth::Asleep => (
            if rng.chance(0.04) { rng.range(200, 700) } else { rng.range(25, 105) },
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
            (Truth::Active, 120),  // 18:00 dinner, chores
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
        let before = if start == 0 { Kind::Unknown } else { kinds[start - 1] };
        let mut out = vec![Kind::Unknown; now + 1 - start];
        sleep::classify(before, &minutes[start..=now], &mut out);
        kinds[window..=now].copy_from_slice(&out[window - start..]);
    }
    kinds
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
    kinds.iter().filter(|k| matches!(k, Kind::Light | Kind::Deep)).count()
}

// ==============================================================================
// Report
// ==============================================================================

fn main() {
    for seed in 0..3 {
        let mut rng = Rng::new(seed);
        let (truth, minutes) = a_night(&mut rng);
        let kinds = incremental(&minutes);
        if std::env::var("BATCH").is_ok() {
            let b = batch(&minutes);
            for (hour, row) in timeline(&b).as_bytes().chunks(60).enumerate().skip(4).take(5) {
                println!("B {:02}:00 {}", (18 + hour) % 24, std::str::from_utf8(row).unwrap());
            }
            let t: String = truth.iter().map(|t| match t { Truth::DeepAsleep => 'D', Truth::Asleep => 'l', Truth::Up => 'U', _ => '.' }).collect();
            for (hour, row) in t.as_bytes().chunks(60).enumerate().skip(4).take(5) {
                println!("T {:02}:00 {}", (18 + hour) % 24, std::str::from_utf8(row).unwrap());
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
            println!("  {:02}:00 {}", (18 + hour) % 24, std::str::from_utf8(row).unwrap());
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
            assert!(!day.clone().any(|k| k.is_sleep()), "seed {seed}:\n{}", timeline(&kinds));
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
            assert!(differ <= 10, "seed {seed}: {differ} minutes differ\n{}\n{}", timeline(&a), timeline(&b));
        }
    }

    /// An evening of TV, however still, isn't sleep.
    #[test]
    fn lounging_is_not_sleep() {
        for seed in 0..20 {
            let mut rng = Rng::new(seed);
            let (_, minutes) = build(&mut rng, &[(Truth::Lounging, 240)]);
            let kinds = incremental(&minutes);
            assert!(!kinds.iter().any(|k| k.is_sleep()), "seed {seed}:\n{}", timeline(&kinds));
        }
    }

    /// A watch on the nightstand isn't a sleeping wearer.
    #[test]
    fn off_the_wrist_is_not_worn() {
        let mut rng = Rng::new(1);
        let (_, minutes) = build(&mut rng, &[(Truth::Active, 30), (Truth::OffWrist, 300), (Truth::Active, 30)]);
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
}
