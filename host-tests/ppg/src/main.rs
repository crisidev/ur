//! Host tests for the firmware's `ppg` module, against synthetic PPG.
//!
//! `cargo run --release` prints a report across scenarios; `cargo test
//! --release` asserts on them.

#[path = "../../../src/ppg.rs"]
#[allow(dead_code)]
mod ppg;

use ppg::{Consensus, Conversions, Ppg, Rejection};

// ==============================================================================
// Synthetic signals
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
    fn uniform(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
    fn gauss(&mut self) -> f64 {
        let (u, v) = (self.uniform().max(1e-12), self.uniform());
        (-2.0 * u.ln()).sqrt() * (2.0 * std::f64::consts::PI * v).cos()
    }
}

#[derive(Clone)]
struct Scenario {
    /// Mean heart rate; `None` for no pulse at all.
    bpm: Option<f64>,
    /// Beat-to-beat interval variation (standard deviation, as a fraction).
    hrv: f64,
    /// Constant light level, in counts.
    dc: f64,
    /// Pulse amplitude, in counts.
    ac: f64,
    /// White sensor noise (standard deviation), in counts.
    noise: f64,
    /// Breathing (0.25 Hz) amplitude, in counts.
    breathing: f64,
    /// Slow drift (0.02 Hz) amplitude, in counts.
    drift: f64,
    /// Arm movements: (start s, duration s, amplitude in counts). Each also
    /// shifts the light level, as the sensor settles somewhere new.
    motion: Vec<(f64, f64, f64)>,
    seconds: f64,
    seed: u64,
}

impl Scenario {
    fn pulse(bpm: f64, seed: u64) -> Self {
        Self {
            bpm: Some(bpm),
            hrv: 0.03,
            dc: 8000.0,
            ac: 80.0,
            noise: 8.0,
            breathing: 40.0,
            drift: 240.0,
            motion: Vec::new(),
            seconds: 30.0,
            seed,
        }
    }
}

/// The pulse's shape over one beat (phase 0..1): a sharp systolic peak and
/// a smaller, later diastolic wave after the dicrotic notch.
fn pulse_shape(phase: f64) -> f64 {
    let g = |centre: f64, width: f64| (-((phase - centre) / width).powi(2)).exp();
    g(0.2, 0.08) + 0.4 * g(0.5, 0.1)
}

/// How far off the sensor's oscillator may be (either way).
const OSCILLATOR_TOLERANCE: f64 = 0.05;
/// The firmware's poll interval.
const POLL_US: u64 = 10_000;
/// Chance that the first poll after a conversion catches the sensor
/// mid-update and reads a mix of old and new bytes.
const TORN_CHANCE: f64 = 0.03;
/// Chance that a poll is delayed (by up to `LATE_US`) because the executor
/// was busy elsewhere.
const LATE_CHANCE: f64 = 0.02;
const LATE_US: f64 = 15_000.0;

/// One poll of the sensor's result register, as the firmware sees it.
#[derive(Clone, Copy)]
struct Poll {
    raw: u32,
    at_us: u64,
    /// Which conversion the sensor was showing (for checking the tracker).
    #[cfg_attr(not(test), allow(dead_code))]
    conversion: usize,
}

/// Polls as the firmware would make them: the HRS3300 converts on its own
/// oscillator (nominally every `ppg::NOMINAL_PERIOD_US`, up to 5% off),
/// each result quantised and clipped to 14 bits; the firmware polls every
/// 10 ms with some jitter, occasionally late, and occasionally reads a
/// result mid-update.
fn generate_with_beats(s: &Scenario) -> (Vec<Poll>, Vec<f64>) {
    let mut rng = Rng::new(s.seed);
    let conversion_s = f64::from(ppg::NOMINAL_PERIOD_US) / 1e6
        * (1.0 + 2.0 * OSCILLATOR_TOLERANCE * (rng.uniform() - 0.5));
    let first_conversion_s = rng.uniform() * conversion_s;
    let drift_phase = rng.uniform() * 6.3;
    let breath_phase = rng.uniform() * 6.3;

    // Beat times, with independent interval jitter.
    let mut beats = vec![-rng.uniform()];
    if let Some(bpm) = s.bpm {
        while *beats.last().unwrap() < s.seconds + 2.0 {
            let interval = 60.0 / bpm * (1.0 + s.hrv * rng.gauss());
            beats.push(beats.last().unwrap() + interval);
        }
    }

    // Each movement: three random wobbles between 0.7 and 3 Hz.
    let motions: Vec<_> = s
        .motion
        .iter()
        .map(|&(start, duration, amplitude)| {
            let wobbles: Vec<(f64, f64)> = (0..3)
                .map(|_| (0.7 + 2.3 * rng.uniform(), rng.uniform() * 6.3))
                .collect();
            let shift = amplitude * (rng.uniform() - 0.5) * 4.0;
            (start, duration, amplitude, wobbles, shift)
        })
        .collect();

    let light = |t: f64| {
        let tau = std::f64::consts::TAU;
        let mut v = s.dc
            + s.drift * (tau * 0.02 * t + drift_phase).sin()
            + s.breathing * (tau * 0.25 * t + breath_phase).sin();
        if s.bpm.is_some() {
            let k = beats.partition_point(|&b| b <= t).saturating_sub(1);
            let phase = (t - beats[k]) / (beats[k + 1] - beats[k]);
            // More blood, less light back.
            v -= s.ac * pulse_shape(phase);
        }
        for (start, duration, amplitude, wobbles, shift) in &motions {
            if t >= *start && t < start + duration {
                let envelope = (std::f64::consts::PI * (t - start) / duration).sin();
                let wobble: f64 = wobbles.iter().map(|(f, p)| (tau * f * t + p).sin()).sum();
                v += amplitude * envelope * wobble;
            }
            if t >= *start {
                v += shift * ((t - start) / duration).min(1.0);
            }
        }
        v
    };

    // Every conversion result, then the polls that observe them.
    let conversions: Vec<u32> = (0..)
        .map(|c| first_conversion_s + f64::from(c) * conversion_s)
        .take_while(|&t| t < s.seconds + 1.0)
        .map(|t| {
            (light(t) + s.noise * rng.gauss())
                .round()
                .clamp(0.0, 16383.0) as u32
        })
        .collect();
    let mut polls = Vec::new();
    let mut last_seen = None;
    let mut at_us = 0u64;
    let mut previous_t = 0.0f64;
    while (at_us as f64) < s.seconds * 1e6 {
        let mut late = rng.uniform() * 2000.0;
        if rng.uniform() < LATE_CHANCE {
            late += rng.uniform() * LATE_US;
        }
        // A late poll delays the next one too (embassy's `Ticker` catches
        // up rather than overlapping).
        let t = ((at_us as f64 + late) / 1e6).max(previous_t + 0.0005);
        previous_t = t;
        at_us += POLL_US;
        let since_first = (t - first_conversion_s) / conversion_s;
        if since_first < 0.0 {
            continue;
        }
        let latest = since_first.floor() as usize;
        let mut raw = conversions[latest];
        if last_seen != Some(latest) && latest > 0 && rng.uniform() < TORN_CHANCE {
            // High byte from the old result, the rest from the new.
            raw = (conversions[latest - 1] & !0xFF) | (raw & 0xFF);
            // ...and the next poll sees it whole.
        } else {
            last_seen = Some(latest);
        }
        polls.push(Poll {
            raw,
            at_us: (t * 1e6) as u64,
            conversion: latest,
        });
    }
    (polls, beats)
}

#[cfg(test)]
fn generate(s: &Scenario) -> Vec<Poll> {
    generate_with_beats(s).0
}

/// The actual mean heart rate between two times, from the beats in it.
/// With beat-to-beat variation this differs from the scenario's nominal
/// rate, and it's what a reading should match.
fn realised_bpm(beats: &[f64], from: f64, to: f64) -> f64 {
    let inside: Vec<f64> = beats
        .iter()
        .copied()
        .filter(|&b| b >= from && b <= to)
        .collect();
    match inside.as_slice() {
        [first, .., last] => 60.0 * (inside.len() - 1) as f64 / (last - first),
        _ => f64::NAN,
    }
}

// ==============================================================================
// Running the estimator
// ==============================================================================

#[derive(Default)]
struct Outcome {
    /// (time s, BPM or why not) for every window, at the measured rate.
    windows: Vec<(f64, Result<f32, Rejection>)>,
    /// (time s, BPM) for every consensus reading.
    readings: Vec<(f64, u8)>,
    /// The conversion period measured by the end.
    period_us: f32,
}

/// Feeds polls through conversion tracking, the estimator, and consensus,
/// as the firmware does. `gate` may veto windows by time (the firmware
/// vetoes those during movement).
fn run(polls: &[Poll], gate: impl Fn(f64) -> bool) -> Outcome {
    let mut conversions = Conversions::new();
    let mut ppg = Ppg::new();
    let mut consensus = Consensus::new();
    let mut outcome = Outcome::default();
    for poll in polls {
        let t = poll.at_us as f64 / 1e6;
        let mut verdicts = Vec::new();
        conversions.poll(poll.raw, poll.at_us, |raw| {
            if let Some(verdict) = ppg.push(raw) {
                verdicts.push(verdict);
            }
        });
        for verdict in verdicts {
            let verdict = verdict.map(|e| e.bpm_at(conversions.period_us()));
            outcome.windows.push((t, verdict));
            if let Ok(bpm) = verdict
                && gate(t)
                && let Some(bpm) = consensus.push(bpm)
            {
                outcome.readings.push((t, bpm));
            }
        }
    }
    outcome.period_us = conversions.period_us();
    outcome
}

fn no_gate(_: f64) -> bool {
    true
}

/// Window accuracy and reading outcomes over many seeds of one scenario.
struct Summary {
    runs: usize,
    /// Runs with at least one reading.
    with_reading: usize,
    /// Largest error of any reading, in BPM.
    worst_reading_error: f64,
    /// Time of the first reading, averaged over runs that had one.
    mean_time_to_reading: f64,
    windows: usize,
    accepted_windows: usize,
    worst_window_error: f64,
}

fn summarise(scenario: &Scenario, seeds: u64, gate: impl Fn(f64) -> bool + Copy) -> Summary {
    let mut s = Summary {
        runs: 0,
        with_reading: 0,
        worst_reading_error: 0.0,
        mean_time_to_reading: 0.0,
        windows: 0,
        accepted_windows: 0,
        worst_window_error: 0.0,
    };
    for seed in 0..seeds {
        let scenario = Scenario {
            seed: scenario.seed * 1000 + seed,
            ..scenario.clone()
        };
        let (polls, beats) = generate_with_beats(&scenario);
        let outcome = run(&polls, gate);
        // Any reading at all is infinitely wrong without a pulse.
        let error = |bpm: f32, from: f64, to: f64| match scenario.bpm {
            Some(_) => (f64::from(bpm) - realised_bpm(&beats, from, to)).abs(),
            None => f64::INFINITY,
        };
        s.runs += 1;
        if let Some(&(t, _)) = outcome.readings.first() {
            s.with_reading += 1;
            s.mean_time_to_reading += t;
        }
        // A reading is the median of the last three accepted windows'
        // estimates (8 s each, 2 s apart, though rejected windows may come
        // between them), so it should match the true rate of one of the
        // recent windows.
        for &(t, bpm) in &outcome.readings {
            let error = [0.0, 2.0, 4.0, 6.0, 8.0]
                .map(|back| error(f32::from(bpm), t - back - 8.0, t - back))
                .into_iter()
                .fold(f64::INFINITY, f64::min);
            s.worst_reading_error = s.worst_reading_error.max(error);
        }
        for &(t, verdict) in &outcome.windows {
            s.windows += 1;
            if let Ok(bpm) = verdict {
                s.accepted_windows += 1;
                s.worst_window_error = s.worst_window_error.max(error(bpm, t - 8.0, t));
            }
        }
    }
    if s.with_reading > 0 {
        s.mean_time_to_reading /= s.with_reading as f64;
    }
    s
}

fn print_row(name: &str, s: &Summary) {
    println!(
        "{name:<34} readings {:>3}/{:<3} worst err {:>5.1}  t1 {:>5.1}s | windows ok {:>4}/{:<4} worst err {:>5.1}",
        s.with_reading,
        s.runs,
        s.worst_reading_error,
        s.mean_time_to_reading,
        s.accepted_windows,
        s.windows,
        s.worst_window_error
    );
}

/// The scenarios, by name, shared by the report and the tests.
fn scenarios() -> Vec<(String, Scenario)> {
    let mut all = Vec::new();
    for bpm in [
        45.0, 50.0, 60.0, 72.0, 90.0, 110.0, 130.0, 150.0, 165.0, 180.0,
    ] {
        all.push((format!("clean {bpm}"), Scenario::pulse(bpm, 1)));
    }
    for bpm in [50.0, 75.0, 120.0, 170.0] {
        all.push((
            format!("noisy {bpm} (noise 0.3 AC)"),
            Scenario {
                noise: 24.0,
                ..Scenario::pulse(bpm, 2)
            },
        ));
        all.push((
            format!("wander {bpm} (breath 2 AC)"),
            Scenario {
                breathing: 160.0,
                drift: 1600.0,
                ..Scenario::pulse(bpm, 3)
            },
        ));
        all.push((
            format!("weak {bpm} (AC 6, noise 1.5)"),
            Scenario {
                ac: 6.0,
                noise: 1.5,
                breathing: 3.0,
                drift: 20.0,
                ..Scenario::pulse(bpm, 4)
            },
        ));
        all.push((
            format!("hrv 8% {bpm}"),
            Scenario {
                hrv: 0.08,
                ..Scenario::pulse(bpm, 5)
            },
        ));
        all.push((
            format!("motion {bpm} (2 bursts, 15 AC)"),
            Scenario {
                motion: vec![(4.0, 2.0, 1200.0), (20.0, 1.5, 1200.0)],
                seconds: 40.0,
                ..Scenario::pulse(bpm, 6)
            },
        ));
    }
    all
}

/// Scenarios with no pulse: none should ever produce a reading.
fn garbage() -> Vec<(String, Scenario)> {
    let base = Scenario {
        bpm: None,
        seconds: 300.0,
        ..Scenario::pulse(0.0, 7)
    };
    vec![
        (
            "no contact (flat, noise 2)".into(),
            Scenario {
                dc: 300.0,
                noise: 2.0,
                breathing: 0.0,
                drift: 0.0,
                ..base.clone()
            },
        ),
        (
            "exactly constant".into(),
            Scenario {
                noise: 0.0,
                breathing: 0.0,
                drift: 0.0,
                ..base.clone()
            },
        ),
        (
            "noise only (sigma 30)".into(),
            Scenario {
                noise: 30.0,
                ..base.clone()
            },
        ),
        (
            "breathing + drift only".into(),
            Scenario {
                noise: 2.0,
                breathing: 200.0,
                drift: 2000.0,
                ..base.clone()
            },
        ),
        (
            "constant motion, no pulse".into(),
            Scenario {
                motion: (0..60).map(|i| (f64::from(i) * 5.0, 4.0, 300.0)).collect(),
                ..base.clone()
            },
        ),
    ]
}

/// What the firmware's accelerometer veto does: a window is only used if
/// no movement overlapped it.
fn accelerometer_gate(s: &Scenario) -> impl Fn(f64) -> bool + Copy + '_ {
    let window_s = ppg::WINDOW as f64 / f64::from(ppg::SAMPLE_HZ);
    move |t| {
        s.motion
            .iter()
            .all(|&(start, duration, _)| start + duration < t - window_s || start > t)
    }
}

/// How far off a reading may be from the true rate: 3.5 BPM (readings are
/// rounded to whole BPM, which accounts for up to half of that), or 3% at
/// high rates, where one sample of lag is a lot of BPM.
#[cfg(test)]
fn tolerance(scenario: &Scenario) -> f64 {
    (scenario.bpm.expect("a pulse") * 0.03).max(3.5)
}

// ==============================================================================
// Replaying real wrists
// ==============================================================================

/// Replays sessions extracted from a field log (`scripts/ppg_sessions.py`
/// writes them: UTC start, a reference rate, the conversion period, the
/// firmware's outcome, then raw samples) through `Ppg` and `Consensus`, and
/// prints each window's verdict next to the reference.
fn replay(path: &str) {
    let text = std::fs::read_to_string(path).expect("readable sessions file");
    let (mut sessions, mut read, mut right) = (0, 0, 0);
    for line in text.lines() {
        let mut fields = line.split_whitespace();
        let mut next = || fields.next().expect("session line has its header");
        let (start, reference, period, outcome) = (next(), next(), next(), next());
        let reference: f32 = reference.parse().expect("a number");
        let period: f32 = period.parse().expect("a number");
        let samples: Vec<u32> = fields.map(|v| v.parse().expect("a number")).collect();
        let mut ppg = Ppg::new();
        let mut consensus = Consensus::new();
        let mut verdicts = Vec::new();
        let mut reading = None;
        for (i, &sample) in samples.iter().enumerate() {
            let Some(verdict) = ppg.push(sample) else {
                continue;
            };
            verdicts.push(match verdict {
                Ok(estimate) => {
                    let bpm = estimate.bpm_at(period);
                    if let Some(agreed) = consensus.push(bpm)
                        && reading.is_none()
                    {
                        reading = Some((agreed, i as f32 * period / 1e6));
                    }
                    format!("{bpm:.0}({:.2})", estimate.quality)
                }
                Err(Rejection::Flat) => "flat".into(),
                Err(Rejection::Unsteady { ratio }) => format!("U{ratio:.1}"),
                Err(Rejection::Aperiodic { quality, bpm }) => format!("a{bpm:.0}({quality:.2})"),
            });
        }
        sessions += 1;
        let verdict = match reading {
            Some((bpm, at)) => {
                read += 1;
                if reference > 0.0 && (f32::from(bpm) - reference).abs() <= 8.0 {
                    right += 1;
                }
                format!("{bpm} BPM at {at:.0} s")
            }
            None => "none".into(),
        };
        println!(
            "{start} ref {reference:3.0} fw {outcome:>4} replay {verdict:16} {}",
            verdicts.join(" ")
        );
    }
    println!("{read}/{sessions} sessions read, {right} within 8 BPM of the reference");
}

fn main() {
    if let Some(path) = std::env::args().nth(1) {
        return replay(&path);
    }
    println!("== pulses (20 seeds each; readings within a 30 s session, 40 s with motion) ==");
    for (name, scenario) in scenarios() {
        print_row(&name, &summarise(&scenario, 20, no_gate));
    }
    println!("== no pulse (20 seeds each, 300 s) ==");
    for (name, scenario) in garbage() {
        print_row(&name, &summarise(&scenario, 20, no_gate));
    }
    println!("== no pulse, with the accelerometer veto ==");
    for (name, scenario) in garbage() {
        print_row(
            &name,
            &summarise(&scenario, 20, accelerometer_gate(&scenario)),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEEDS: u64 = 20;

    /// Every reading from a real pulse is accurate, and most sessions get
    /// one. Noisy and weak fast pulses are the hard case (a narrow pulse
    /// puts little of its energy in the band), so they only have to be
    /// accurate when they're read at all.
    #[test]
    fn pulses_are_measured_accurately() {
        for (name, scenario) in scenarios() {
            let s = summarise(&scenario, SEEDS, no_gate);
            assert!(
                s.worst_reading_error <= tolerance(&scenario),
                "{name}: a reading was {:.1} BPM off",
                s.worst_reading_error
            );
            let hard = name.starts_with("noisy") || name.starts_with("weak");
            let required = match scenario.bpm {
                Some(bpm) if hard && bpm > 120.0 => 0,
                _ if hard => SEEDS as usize * 7 / 10,
                _ => SEEDS as usize * 9 / 10,
            };
            assert!(
                s.with_reading >= required,
                "{name}: only {}/{} sessions got a reading",
                s.with_reading,
                s.runs
            );
        }
    }

    /// No pulse, no reading: with the accelerometer veto in place, as in
    /// the firmware. (Without it, sustained random motion occasionally
    /// passes for a pulse; see the report.)
    #[test]
    fn garbage_never_yields_a_reading() {
        for (name, scenario) in garbage() {
            let s = summarise(&scenario, SEEDS, accelerometer_gate(&scenario));
            assert_eq!(s.with_reading, 0, "{name}: got a reading from no pulse");
        }
    }

    /// Everything except sustained motion is rejected even without the
    /// accelerometer.
    #[test]
    fn garbage_without_motion_is_rejected_by_the_signal_alone() {
        for (name, scenario) in garbage() {
            if !scenario.motion.is_empty() {
                continue;
            }
            let s = summarise(&scenario, SEEDS, no_gate);
            assert_eq!(s.accepted_windows, 0, "{name}: accepted a window");
        }
    }

    #[test]
    fn constant_input_is_flat() {
        // A value that never changes still yields samples, one per period.
        let polls: Vec<Poll> = (0..3000)
            .map(|i| Poll {
                raw: 5000,
                at_us: i * POLL_US,
                conversion: 0,
            })
            .collect();
        let outcome = run(&polls, no_gate);
        assert!(!outcome.windows.is_empty());
        assert!(
            outcome
                .windows
                .iter()
                .all(|(_, v)| *v == Err(Rejection::Flat))
        );
    }

    /// Windows containing a movement are rejected from the signal alone.
    #[test]
    fn motion_windows_are_rejected() {
        let scenario = Scenario {
            motion: vec![(12.0, 2.0, 1200.0)],
            ..Scenario::pulse(70.0, 9)
        };
        for seed in 0..SEEDS {
            let outcome = run(
                &generate(&Scenario {
                    seed,
                    ..scenario.clone()
                }),
                no_gate,
            );
            for (t, verdict) in outcome.windows {
                // The window ending at `t` covers the 8 s before it; the
                // burst peaks at 13 s.
                if t - 8.0 < 13.0 && t > 13.0 {
                    assert!(verdict.is_err(), "seed {seed}: accepted window at {t} s");
                }
            }
        }
    }

    /// The measured conversion rate tracks the sensor's actual oscillator,
    /// despite late polls, torn reads, and repeated values.
    #[test]
    fn conversion_rate_is_measured() {
        for seed in 0..SEEDS {
            let scenario = Scenario {
                seed,
                ..Scenario::pulse(70.0, 10)
            };
            // The same draw `generate` makes first.
            let mut rng = Rng::new(scenario.seed);
            let truth = f64::from(ppg::NOMINAL_PERIOD_US)
                * (1.0 + 2.0 * OSCILLATOR_TOLERANCE * (rng.uniform() - 0.5));
            let measured = f64::from(run(&generate(&scenario), no_gate).period_us);
            assert!(
                (measured - truth).abs() / truth < 0.002,
                "seed {seed}: measured {measured:.0} µs, truth {truth:.0} µs"
            );
        }
    }

    /// Repeated values (a flat stretch) still count as conversions.
    #[test]
    fn repeats_are_inferred() {
        let mut conversions = Conversions::new();
        let mut emitted = Vec::new();
        let period = u64::from(ppg::NOMINAL_PERIOD_US);
        // Conversions 1, 2, 2, 2, 3, polled every 10 ms.
        let value = |t: u64| match t / period {
            0 => 1,
            1..=3 => 2,
            _ => 3,
        };
        for t in (0..6 * period).step_by(10_000) {
            conversions.poll(value(t), t, |v| emitted.push(v));
        }
        assert_eq!(emitted, [1, 2, 2, 2, 3]);
    }

    #[test]
    fn consensus_needs_agreement() {
        let mut c = Consensus::new();
        assert_eq!(c.push(70.0), None);
        assert_eq!(c.push(71.0), None);
        assert_eq!(c.push(72.4), Some(71));
        // One outlier spoils the next two readings.
        assert_eq!(c.push(140.0), None);
        assert_eq!(c.push(71.0), None);
        assert_eq!(c.push(70.0), None);
        assert_eq!(c.push(70.5), Some(71));
    }

    /// The tracker emits exactly one sample per conversion: after each
    /// poll, the count emitted is the number of conversions before the one
    /// showing (give or take the one just starting, or a torn read).
    #[test]
    fn every_conversion_is_counted_once() {
        for seed in 0..SEEDS {
            let scenario = Scenario {
                seed,
                ..Scenario::pulse(70.0, 11)
            };
            let polls = generate(&scenario);
            let mut conversions = Conversions::new();
            let mut emitted = 0usize;
            let first = polls[0].conversion;
            for poll in &polls {
                conversions.poll(poll.raw, poll.at_us, |_| emitted += 1);
                let expected = poll.conversion - first;
                assert!(
                    emitted.abs_diff(expected) <= 1,
                    "seed {seed}: emitted {emitted}, expected {expected} at {} µs",
                    poll.at_us
                );
            }
        }
    }
}
