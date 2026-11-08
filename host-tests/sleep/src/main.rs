//! Host tests for the firmware's `sleep` and `stages` modules, on
//! synthetic days and nights.
//!
//! `cargo run` prints each scenario's timeline (one character per minute);
//! `cargo test` asserts on them. `cargo run -- <decoded field log>
//! [<reference stages>]` replays a real day from a field log instead (see
//! `replay`).
//!
//! The movement levels are guesses in the firmware's units (~25 on a desk,
//! hundreds awake), like the classifier's thresholds, and the heart rates a
//! caricature of a night's. These tests check the *logic* (onset, waking,
//! stirs, stages, not-worn, and that classifying a trailing window every
//! minute agrees with classifying the whole night at once), not that the
//! thresholds match real wrists; field logs decide that.

#[path = "../../../src/sleep.rs"]
#[allow(dead_code)]
mod sleep;
#[path = "../../../src/stages.rs"]
#[allow(dead_code)]
mod stages;

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
    /// Light sleep.
    Asleep,
    /// A long, very still stretch of sleep.
    DeepAsleep,
    /// REM sleep: moving like light sleep, the heart rate up.
    Rem,
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
        Truth::Asleep | Truth::Rem => (
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
            (Truth::Asleep, 50),
            (Truth::Rem, 25),
            (Truth::Asleep, 35),
            (Truth::Up, 6), // 03:00 bathroom
            (Truth::Asleep, 50),
            (Truth::DeepAsleep, 45),
            (Truth::Asleep, 50),
            (Truth::Rem, 30),
            (Truth::Asleep, 60), // until 07:00
            (Truth::Active, 300),
            (Truth::Sitting, 360),
            (Truth::Active, 39),
        ],
    )
}

/// Heart rate readings for `truth` (one every 5 minutes, as the watch
/// takes them asleep), from their own random numbers so that adding them
/// leaves the movement as it was. Asleep, a baseline drifting down 1 BPM
/// every 80 minutes from 23:00 (minute 300); light sleep a few beats over
/// it, deep at it, REM well over it.
fn heart_rates(seed: u64, truth: &[Truth]) -> Vec<u8> {
    let mut rng = Rng::new(seed.wrapping_add(1_000_000));
    truth
        .iter()
        .enumerate()
        .map(|(t, truth)| {
            let base = 56 - (t.saturating_sub(300).min(480) / 80) as u16;
            let bpm = match truth {
                Truth::Active | Truth::Up => rng.range(75, 95),
                Truth::Sitting | Truth::Lounging => rng.range(62, 72),
                Truth::Asleep => base + rng.range(1, 4),
                Truth::DeepAsleep => base + rng.range(0, 2) - 1,
                Truth::Rem => base + rng.range(9, 13),
                Truth::OffWrist => 0,
            };
            if t % 5 == 0 { bpm as u8 } else { 0 }
        })
        .collect()
}

// ==============================================================================
// Running the classifier
// ==============================================================================

#[cfg(test)]
/// Classifies everything at once.
fn batch(minutes: &[Minute]) -> Vec<Kind> {
    let mut out = vec![Kind::Unknown; minutes.len()];
    sleep::classify(Kind::Unknown, minutes, &mut out);
    out
}

/// The firmware's per-minute record, as `crate::activity`'s history
/// holds it, up to minute `newest`.
struct Record<'a> {
    kinds: &'a mut [Kind],
    minutes: &'a [Minute],
    heart: &'a [u8],
    newest: i64,
}

impl stages::Record for Record<'_> {
    fn minute(&self, minute: i64) -> (Kind, Option<u16>, u8) {
        if !(0..=self.newest).contains(&minute) {
            return (Kind::Unknown, None, 0);
        }
        let t = minute as usize;
        let m = self.minutes[t];
        (
            self.kinds[t],
            m.seen.then_some(m.movement),
            self.heart.get(t).copied().unwrap_or(0),
        )
    }

    fn newest(&self) -> i64 {
        self.newest
    }

    fn set_stage(&mut self, minute: i64, kind: Kind) {
        self.kinds[minute as usize] = kind;
    }
}

/// Runs as the firmware does, every minute: classifies the trailing window
/// and its context, seeded with the (final) class of the minute before
/// them, writing back only the window; then re-stages the last sleep while
/// it reaches into that window (`activity::History::last_sleep`). Returns
/// the classes at the end, and whether the newest minute was sleep at each
/// minute (`activity::asleep()`: the leading edge, where unfinished
/// movement is still restless). `heart` may be shorter than `minutes`
/// (empty for no readings).
fn firmware(minutes: &[Minute], heart: &[u8]) -> (Vec<Kind>, Vec<bool>) {
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
        if let Some((start, end, _)) = sleep::last_sleep(&kinds[..=now])
            && now < end + sleep::WINDOW
        {
            let mut record = Record {
                kinds: &mut kinds,
                minutes,
                heart,
                newest: now as i64,
            };
            stages::stage(&mut record, start as i64, end as i64);
        }
        asleep.push(kinds[now].is_sleep());
    }
    (kinds, asleep)
}

#[cfg(test)]
/// Classifies as the firmware does, without heart rate readings (so all
/// sleep is light).
fn incremental(minutes: &[Minute]) -> Vec<Kind> {
    firmware(minutes, &[]).0
}

#[cfg(test)]
/// What the firmware's `activity::asleep()` says each minute.
fn leading_edge(minutes: &[Minute]) -> Vec<bool> {
    firmware(minutes, &[]).1
}

#[cfg(test)]
/// Stages every sleep of `kinds` (as classified, with sleep as light)
/// once, at the end: what the minute-by-minute staging should come to.
fn staged_at_once(kinds: &[Kind], minutes: &[Minute], heart: &[u8]) -> Vec<Kind> {
    let mut kinds: Vec<Kind> = kinds
        .iter()
        .map(|&k| if k.is_stage() { Kind::Light } else { k })
        .collect();
    let mut until = kinds.len();
    while let Some((start, end, _)) = sleep::last_sleep(&kinds[..until]) {
        let newest = kinds.len() as i64 - 1;
        let mut record = Record {
            kinds: &mut kinds,
            minutes,
            heart,
            newest,
        };
        stages::stage(&mut record, start as i64, end as i64);
        until = start;
    }
    kinds
}

#[cfg(test)]
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
            night.end();
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
            Kind::Rem => 'R',
        })
        .collect()
}

fn asleep_minutes(kinds: &[Kind]) -> usize {
    kinds.iter().filter(|k| k.is_stage()).count()
}

// ==============================================================================
// Report
// ==============================================================================

/// Replays a decoded field log (`defmt-print` output): rebuilds each
/// minute's movement, steps, worn verdict, and heart rate from the
/// accelerometer and heart rate lines, and classifies and stages them as
/// the firmware does, to tune the thresholds on real nights. Uptimes become
/// wall-clock time via the `clock:` lines.
///
/// With `reference`, a file of `<UTC minute> <stage>` lines (`awake`,
/// `light`, `deep`, or `rem`; see `scripts/hc-stages.py` for a Garmin's,
/// from a Health Connect export), also prints those stages under ours, and
/// how well they agree.
fn replay(path: &str, reference: Option<&str>) {
    let text = std::fs::read_to_string(path).expect("a readable log");
    // (uptime seconds, UTC seconds) anchors, from `clock: <utc> UTC = ...`.
    let mut anchor: Option<(f64, i64)> = None;
    let mut minutes: std::collections::BTreeMap<i64, (u64, u64, u16, Option<bool>)> =
        Default::default();
    let mut heart: std::collections::BTreeMap<i64, u8> = Default::default();
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
        let minute = anchor.map(|(at, utc)| (utc + (uptime - at) as i64).div_euclid(60));
        if let Some(rest) = message.strip_prefix("heart: ") {
            if message.contains(" BPM") && !message.contains("window") {
                worn = Some(true);
                // "heart: 56 BPM after 24 s (...)", or "heart: 56 BPM" for
                // the readings after the first: what the history records.
                if let Some((bpm, _)) = rest.split_once(" BPM")
                    && let (Ok(bpm), Some(minute)) = (bpm.parse::<u8>(), minute)
                {
                    heart.insert(minute, bpm);
                }
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
        let Some(minute) = minute else { continue };
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
    let heart: Vec<u8> = (first..=last)
        .map(|m| heart.get(&m).copied().unwrap_or(0))
        .collect();
    let (kinds, _) = firmware(&evidence, &heart);
    let stages = stages::Minutes::count(kinds.iter().copied());
    println!(
        "{} minutes, {} heart rate readings; asleep {} (light {}, deep {}, REM {}); last sleep {:?}",
        kinds.len(),
        heart.iter().filter(|&&b| b > 0).count(),
        asleep_minutes(&kinds),
        stages.light,
        stages.deep,
        stages.rem,
        sleep::last_sleep(&kinds)
    );

    let reference: Option<Vec<Option<Kind>>> = reference.map(|path| {
        let text = std::fs::read_to_string(path).expect("a readable reference");
        let mut stages = vec![None; kinds.len()];
        for line in text.lines() {
            let Some((minute, stage)) = line.split_once(' ') else {
                continue;
            };
            let Ok(minute) = minute.parse::<i64>() else {
                continue;
            };
            let kind = match stage.trim() {
                "awake" => Kind::Awake,
                "light" => Kind::Light,
                "deep" => Kind::Deep,
                "rem" => Kind::Rem,
                _ => continue,
            };
            if (first..=last).contains(&minute) {
                stages[(minute - first) as usize] = Some(kind);
            }
        }
        stages
    });
    let reference_row = |range: std::ops::Range<usize>| -> String {
        let stages = reference.as_ref().expect("a reference");
        let kinds: Vec<Kind> = stages[range]
            .iter()
            .map(|k| k.unwrap_or(Kind::Unknown))
            .collect();
        timeline(&kinds)
    };

    // One row per hour, labelled in UTC.
    let ours = timeline(&kinds);
    for (hour, row) in ours.as_bytes().chunks(60).enumerate() {
        let start = (first + hour as i64 * 60) * 60;
        println!(
            "  {:02}:{:02}Z {}",
            start.rem_euclid(86_400) / 3600,
            start.rem_euclid(3600) / 60,
            std::str::from_utf8(row).expect("ASCII")
        );
        if reference.is_some() {
            let at = hour * 60;
            println!("     ref {}", reference_row(at..at + row.len()));
        }
    }
    if let Some(reference) = &reference {
        compare(&kinds, reference);
    }
}

/// Prints how our classes compare with reference stages, minute by minute:
/// a confusion matrix, the share of the reference's asleep minutes we
/// agree on (restless counting as light), and Cohen's kappa over the
/// minutes both call light, deep, or REM.
fn compare(ours: &[Kind], reference: &[Option<Kind>]) {
    const STAGES: [Kind; 4] = [Kind::Awake, Kind::Light, Kind::Deep, Kind::Rem];
    const ROWS: [Kind; 7] = [
        Kind::Awake,
        Kind::Restless,
        Kind::Light,
        Kind::Deep,
        Kind::Rem,
        Kind::NotWorn,
        Kind::Unknown,
    ];
    let name = |k: Kind| format!("{k:?}").to_lowercase();
    let count = |o: Kind, r: Kind| {
        ours.iter()
            .zip(reference)
            .filter(|&(&x, &y)| x == o && y == Some(r))
            .count()
    };
    print!("\n{:>10}", "ours \\ ref");
    for r in STAGES {
        print!("{:>7}", name(r));
    }
    println!();
    for o in ROWS {
        let row: Vec<usize> = STAGES.iter().map(|&r| count(o, r)).collect();
        if row.iter().sum::<usize>() > 0 {
            print!("{:>10}", name(o));
            for n in row {
                print!("{n:>7}");
            }
            println!();
        }
    }
    let asleep: Vec<(Kind, Kind)> = ours
        .iter()
        .zip(reference)
        .filter_map(|(&o, &r)| r.filter(|r| r.is_stage()).map(|r| (o, r)))
        .collect();
    let agree = asleep
        .iter()
        .filter(|&&(o, r)| o == r || (r == Kind::Light && o == Kind::Restless))
        .count();
    let both: Vec<(Kind, Kind)> = asleep
        .iter()
        .copied()
        .filter(|(o, _)| o.is_stage())
        .collect();
    if both.is_empty() {
        println!("no minutes that both call sleep");
        return;
    }
    let n = both.len() as f64;
    let observed = both.iter().filter(|(o, r)| o == r).count() as f64 / n;
    let chance: f64 = [Kind::Light, Kind::Deep, Kind::Rem]
        .iter()
        .map(|&k| {
            let o = both.iter().filter(|(o, _)| *o == k).count() as f64;
            let r = both.iter().filter(|(_, r)| *r == k).count() as f64;
            o * r / (n * n)
        })
        .sum();
    println!(
        "agreement over the reference's {} asleep minutes {:.0}%; kappa (light/deep/REM) {:.2}",
        asleep.len(),
        100.0 * agree as f64 / asleep.len() as f64,
        (observed - chance) / (1.0 - chance)
    );
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if let Some(path) = args.get(1) {
        return replay(path, args.get(2).map(String::as_str));
    }
    for seed in 0..3 {
        let mut rng = Rng::new(seed);
        let (truth, minutes) = a_night(&mut rng);
        let (kinds, _) = firmware(&minutes, &heart_rates(seed, &truth));
        let real = truth
            .iter()
            .filter(|t| matches!(t, Truth::Asleep | Truth::DeepAsleep | Truth::Rem))
            .count();
        println!(
            "seed {seed}: asleep {} min (truth {real}), last sleep {:?}",
            asleep_minutes(&kinds),
            sleep::last_sleep(&kinds)
        );
        let truth: String = truth
            .iter()
            .map(|t| match t {
                Truth::DeepAsleep => 'D',
                Truth::Asleep => 'l',
                Truth::Rem => 'R',
                Truth::Up => 'U',
                _ => '.',
            })
            .collect();
        for (hour, (row, truth)) in timeline(&kinds)
            .as_bytes()
            .chunks(60)
            .zip(truth.as_bytes().chunks(60))
            .enumerate()
        {
            println!(
                "  {:02}:00 {}",
                (18 + hour) % 24,
                std::str::from_utf8(row).expect("ASCII")
            );
            if (4..14).contains(&hour) {
                println!("   truth {}", std::str::from_utf8(truth).expect("ASCII"));
            }
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
                .filter(|t| matches!(t, Truth::Asleep | Truth::DeepAsleep | Truth::Rem))
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
            // Without heart rate readings, all of it light.
            assert!(
                !kinds.iter().any(|k| matches!(k, Kind::Deep | Kind::Rem)),
                "seed {seed}"
            );
        }
    }

    // ==========================================================================
    // Stages
    // ==========================================================================

    /// The night's stages, from the heart rate: deep in the first deep
    /// stretch, REM in the REM ones, and nothing that the rules rule out.
    /// (The synthetic light sleep stirs less than a real one, so some of it
    /// passes for deep; that's not what this checks.)
    #[test]
    fn a_night_is_staged() {
        for seed in 0..20 {
            let mut rng = Rng::new(seed);
            let (truth, minutes) = a_night(&mut rng);
            let (kinds, _) = firmware(&minutes, &heart_rates(seed, &truth));
            let share = |range: std::ops::Range<usize>, kind: Kind| {
                kinds[range.clone()].iter().filter(|&&k| k == kind).count() as f64
                    / range.len() as f64
            };
            // 00:00–01:10 deep, 02:00–02:25 and 05:31–06:01 REM.
            let msg = || format!("seed {seed}:\n{}", timeline(&kinds));
            assert!(share(360..430, Kind::Deep) > 0.8, "{}", msg());
            // (Some nights' classifier wakes in the middle of a REM
            // stretch, leaving too short a bout on one side: together.)
            let rem = share(480..505, Kind::Rem) * 25.0 + share(691..721, Kind::Rem) * 30.0;
            assert!(rem / 55.0 > 0.6, "{}", msg());
            let (start, end, asleep) = sleep::last_sleep(&kinds).expect("a night");
            assert!(
                !kinds[start..start + stages::REM_AFTER as usize].contains(&Kind::Rem),
                "REM in the first hour: {}",
                msg()
            );
            assert!(
                !kinds[start + stages::DEEP_UNTIL as usize..end].contains(&Kind::Deep),
                "deep after five hours: {}",
                msg()
            );
            let stages = stages::Minutes::count(kinds[start..end].iter().copied());
            assert_eq!(usize::from(stages.asleep()), asleep);
        }
    }

    /// Staging relabels sleep only: what's sleep, restless, or awake is
    /// the same with heart rate readings as without.
    #[test]
    fn staging_keeps_sleep_and_waking() {
        for seed in 0..20 {
            let mut rng = Rng::new(seed);
            let (truth, minutes) = a_night(&mut rng);
            let (staged, asleep) = firmware(&minutes, &heart_rates(seed, &truth));
            let (plain, plain_asleep) = firmware(&minutes, &[]);
            assert_eq!(asleep, plain_asleep, "seed {seed}: asleep() differs");
            for (t, (a, b)) in staged.iter().zip(&plain).enumerate() {
                assert!(
                    a == b || (a.is_stage() && *b == Kind::Light),
                    "seed {seed}, minute {t}: {a:?} vs {b:?}"
                );
            }
        }
    }

    /// Re-staging every minute, as the night goes on, ends up where staging
    /// the whole night at once does.
    #[test]
    fn staging_incrementally_matches_at_once() {
        for seed in 0..20 {
            let mut rng = Rng::new(seed);
            let (truth, minutes) = a_night(&mut rng);
            let heart = heart_rates(seed, &truth);
            let (kinds, _) = firmware(&minutes, &heart);
            let once = staged_at_once(&kinds, &minutes, &heart);
            assert_eq!(
                timeline(&kinds),
                timeline(&once),
                "seed {seed}\n{}\n{}",
                timeline(&kinds),
                timeline(&once)
            );
        }
    }

    /// A hand-made sleep for the rules: `minutes` of still sleep with a
    /// heart rate reading every minute at 50 BPM, but 62 at `raised`.
    fn hand_made(
        minutes: usize,
        raised: &[std::ops::Range<usize>],
    ) -> (Vec<Kind>, Vec<Minute>, Vec<u8>) {
        let kinds = vec![Kind::Light; minutes];
        let still = Minute {
            seen: true,
            movement: 30,
            steps: 0,
            worn: Some(true),
        };
        let mut heart = vec![50; minutes];
        for range in raised {
            heart[range.clone()].fill(62);
        }
        (kinds, vec![still; minutes], heart)
    }

    fn stage_once(kinds: &mut [Kind], minutes: &[Minute], heart: &[u8]) {
        let newest = kinds.len() as i64 - 1;
        let end = kinds.len() as i64;
        let mut record = Record {
            kinds,
            minutes,
            heart,
            newest,
        };
        stages::stage(&mut record, 0, end);
    }

    /// Still and calm is deep, but only for the first five hours; a raised
    /// heart rate is REM, but not in the first hour (when it's light: too
    /// raised for deep). The level averages the readings within 3 minutes,
    /// so 40..100 raised is deep until 40 (39 has one raised neighbour of
    /// 7), and 100 is deep again.
    #[test]
    #[allow(clippy::single_range_in_vec_init)] // a list of raised ranges
    fn stages_follow_the_hours() {
        let (mut kinds, minutes, heart) = hand_made(400, &[40..100]);
        stage_once(&mut kinds, &minutes, &heart);
        let expect = |range: std::ops::Range<usize>, kind| {
            assert!(
                kinds[range.clone()].iter().all(|&k| k == kind),
                "{range:?} not {kind:?}:\n{}",
                timeline(&kinds)
            )
        };
        let (rem_after, deep_until) = (stages::REM_AFTER as usize, stages::DEEP_UNTIL as usize);
        expect(0..40, Kind::Deep);
        expect(40..rem_after, Kind::Light);
        expect(rem_after..100, Kind::Rem);
        expect(100..deep_until, Kind::Deep);
        expect(deep_until..400, Kind::Light);
    }

    /// Stirring keeps it from being deep, however calm the heart.
    #[test]
    fn stirs_are_not_deep() {
        let (mut kinds, mut minutes, heart) = hand_made(200, &[]);
        // A stir every third minute from 60 to 140.
        for t in (60..140).step_by(3) {
            minutes[t].movement = 400;
        }
        stage_once(&mut kinds, &minutes, &heart);
        assert!(
            kinds[70..130].iter().all(|&k| k == Kind::Light),
            "{}",
            timeline(&kinds)
        );
        assert!(
            kinds[..40].iter().all(|&k| k == Kind::Deep),
            "{}",
            timeline(&kinds)
        );
    }

    /// Bouts: short ones are light, and short light gaps between two of the
    /// same stage are filled. (After five hours, so the rest is light, not
    /// deep. A raised stretch of `n` minutes makes `n + 2` REM minutes, as
    /// the level averages over 3 minutes either side.)
    #[test]
    fn bouts_are_cleaned_up() {
        let raised = |start: usize, n: usize| 300 + start..300 + start + n;
        let rem = |kinds: &[Kind], start: usize, n: usize| {
            kinds[299 + start..301 + start + n]
                .iter()
                .all(|&k| k == Kind::Rem)
        };
        // Too short (6 minutes), and just long enough (8).
        let (mut kinds, minutes, heart) = hand_made(500, &[raised(10, 4), raised(50, 6)]);
        stage_once(&mut kinds, &minutes, &heart);
        assert!(!kinds[300..].contains(&Kind::Deep));
        assert!(
            !kinds[300..340].contains(&Kind::Rem),
            "{}",
            timeline(&kinds)
        );
        assert!(rem(&kinds, 50, 6), "{}", timeline(&kinds));
        // Gaps of 5 light minutes (filled) and 6 (not).
        let (mut kinds, minutes, heart) = hand_made(
            500,
            &[
                raised(10, 10),
                raised(27, 10),
                raised(100, 10),
                raised(118, 10),
            ],
        );
        stage_once(&mut kinds, &minutes, &heart);
        assert!(
            kinds[309..338].iter().all(|&k| k == Kind::Rem),
            "{}",
            timeline(&kinds)
        );
        assert!(
            rem(&kinds, 100, 10) && rem(&kinds, 118, 10),
            "{}",
            timeline(&kinds)
        );
        assert!(
            kinds[411..417].iter().all(|&k| k == Kind::Light),
            "{}",
            timeline(&kinds)
        );
    }

    /// Restless minutes stay restless, without splitting a bout; a wake
    /// does split it.
    #[test]
    fn bouts_and_wakes() {
        let (mut kinds, minutes, heart) = hand_made(500, &[310..320, 350..356]);
        kinds[314] = Kind::Restless;
        kinds[352] = Kind::Awake;
        stage_once(&mut kinds, &minutes, &heart);
        assert_eq!(kinds[314], Kind::Restless);
        assert!(
            kinds[309..314]
                .iter()
                .chain(&kinds[315..321])
                .all(|&k| k == Kind::Rem)
        );
        // 349..352 and 353..357: each too short.
        assert_eq!(kinds[352], Kind::Awake);
        assert!(
            !kinds[340..370].contains(&Kind::Rem),
            "{}",
            timeline(&kinds)
        );
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
        assert!(night.end());
        assert!(!night.end(), "already over");
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
