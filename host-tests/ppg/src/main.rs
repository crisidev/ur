//! Host tests for the firmware's `ppg` module, against synthetic PPG.
//!
//! `cargo run --release` prints a report across scenarios; `cargo test
//! --release` asserts on them. `cargo run --release -- SESSIONS
//! [--strict]` replays sessions from a field log (see
//! `scripts/ppg_sessions.py`), `--strict` as before calm pairs.

#[path = "../../../src/ppg.rs"]
#[allow(dead_code)]
mod ppg;

/// For [`heart_schedule::brighter`], which the replay applies.
#[path = "../../../src/heart_schedule.rs"]
#[allow(dead_code)]
mod heart_schedule;

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
/// vetoes those during movement). With `pairs`, a window the gate lets
/// through counts as calm, for `Consensus`'s calm pairs (the firmware
/// judges that from the accelerometer, which synthetic signals don't
/// have); without, none does, as before calm pairs.
fn run(polls: &[Poll], gate: impl Fn(f64) -> bool, pairs: bool) -> Outcome {
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
            let period = conversions.period_us();
            outcome.windows.push((t, verdict.map(|e| e.bpm_at(period))));
            let reading = match verdict {
                _ if !gate(t) => {
                    consensus.reject();
                    None
                }
                Ok(e) => consensus.push(e.bpm_at(period), pairs),
                Err(Rejection::Faint(e)) => consensus.push_faint(e.bpm_at(period), pairs, false),
                Err(_) => {
                    consensus.reject();
                    None
                }
            };
            if let Some(bpm) = reading {
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
    /// Accepted windows more than 10% off the true rate.
    wrong_windows: usize,
}

fn summarise(
    scenario: &Scenario,
    seeds: u64,
    gate: impl Fn(f64) -> bool + Copy,
    pairs: bool,
) -> Summary {
    let mut s = Summary {
        runs: 0,
        with_reading: 0,
        worst_reading_error: 0.0,
        mean_time_to_reading: 0.0,
        windows: 0,
        accepted_windows: 0,
        worst_window_error: 0.0,
        wrong_windows: 0,
    };
    for seed in 0..seeds {
        let scenario = Scenario {
            seed: scenario.seed * 1000 + seed,
            ..scenario.clone()
        };
        let (polls, beats) = generate_with_beats(&scenario);
        let outcome = run(&polls, gate, pairs);
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
                let error = error(bpm, t - 8.0, t);
                s.worst_window_error = s.worst_window_error.max(error);
                if error > 0.1 * scenario.bpm.unwrap_or(0.0) {
                    s.wrong_windows += 1;
                }
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
        "{name:<34} readings {:>3}/{:<3} worst err {:>5.1}  t1 {:>5.1}s | windows ok {:>4}/{:<4} worst err {:>5.1} >10% off {:>3}",
        s.with_reading,
        s.runs,
        s.worst_reading_error,
        s.mean_time_to_reading,
        s.accepted_windows,
        s.windows,
        s.worst_window_error,
        s.wrong_windows
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

/// What `heart::MOTION_LIMIT` is: movement above which the firmware
/// doesn't trust a window (when most of its seconds are above it).
const MOTION_LIMIT: u16 = 1500;
/// Samples per level check in the firmware (`heart::LEVEL_CHECK_SAMPLES`):
/// one movement reading each.
const LEVEL_CHECK_SAMPLES: usize = 27;
/// What `heart::CALM` is: movement above which the wrist is busy.
const CALM: u16 = 150;

/// A session extracted from a field log by `scripts/ppg_sessions.py`, one
/// per line: UTC start, a reference rate (another watch's, or 0), the
/// conversion period, the firmware's outcome, then raw samples, after a
/// `|`, the movement at each level check (older extracts have none), and
/// after another, `asleep` if the sleep classifier said so, and
/// `background` if the schedule started the session (older extracts never
/// say, so replay as watched), the LED's `drive=N` (0 if not said), and
/// `brighter` if the segment carries on the one before it with a brighter
/// LED (see `heart_schedule::brighter`), `cont` if it carries on after a
/// level change, and `arm=dim` or `arm=bright` for an awake background
/// session in the LED experiment (see `heart_schedule::Arm`), with its
/// `round=N` of tries (logs after 0.10.0-rc.3).
struct Recorded {
    start: String,
    reference: f32,
    period: f32,
    outcome: String,
    samples: Vec<u32>,
    movement: Vec<u16>,
    asleep: bool,
    background: bool,
    drive: u8,
    brighter: bool,
    cont: bool,
    /// The LED experiment's arm: `Some(true)` bright, `Some(false)` dim.
    bright: Option<bool>,
    /// The round of tries the session belongs to, which share an arm.
    round: Option<u16>,
}

fn parse_sessions(text: &str) -> Vec<Recorded> {
    let number = |v: &str| -> f32 { v.parse().expect("a number") };
    text.lines()
        .map(|line| {
            let (signal, rest) = line.split_once(" | ").unwrap_or((line, ""));
            let (movement, sleep) = rest.split_once(" | ").unwrap_or((rest, ""));
            let mut fields = signal.split_whitespace();
            let mut next = || fields.next().expect("session line has its header");
            let (start, reference, period, outcome) = (next(), next(), next(), next());
            Recorded {
                start: start.into(),
                reference: number(reference),
                period: number(period),
                outcome: outcome.into(),
                samples: fields.map(|v| v.parse().expect("a number")).collect(),
                movement: movement
                    .split_whitespace()
                    .map(|v| v.parse().expect("a number"))
                    .collect(),
                asleep: sleep.split_whitespace().any(|f| f == "asleep"),
                background: sleep.split_whitespace().any(|f| f == "background"),
                drive: sleep
                    .split_whitespace()
                    .find_map(|f| f.strip_prefix("drive="))
                    .map_or(0, |d| d.parse().expect("a drive")),
                brighter: sleep.split_whitespace().any(|f| f == "brighter"),
                cont: sleep.split_whitespace().any(|f| f == "cont"),
                bright: sleep
                    .split_whitespace()
                    .find_map(|f| f.strip_prefix("arm="))
                    .map(|arm| arm == "bright"),
                round: sleep
                    .split_whitespace()
                    .find_map(|f| f.strip_prefix("round="))
                    .map(|r| r.parse().expect("a round")),
            }
        })
        .collect()
}

/// A window's verdict in a replay: the estimator's, or the firmware's
/// movement vetoes over an estimate.
enum Verdict {
    Window(Result<ppg::Estimate, Rejection>),
    Moving,
    SlowWhileBusy,
}

/// Every window of a recorded session, with the sample it ended at, as the
/// firmware would have judged it, and whether the wrist was calm (at most
/// `CALM`) through it.
fn windows(session: &Recorded) -> Vec<(usize, Verdict, bool)> {
    let mut ppg = Ppg::new();
    let mut verdicts = Vec::new();
    for (i, &sample) in session.samples.iter().enumerate() {
        let Some(verdict) = ppg.push(sample) else {
            continue;
        };
        let checks = ((i + 1) / LEVEL_CHECK_SAMPLES).min(session.movement.len());
        let recent = &session.movement[checks.saturating_sub(8)..checks];
        let moving = recent.iter().filter(|&&m| m > MOTION_LIMIT).count();
        // Older extracts have no movement: not calm, to be safe.
        let calm = !recent.is_empty() && recent.iter().all(|&m| m <= CALM);
        verdicts.push((
            i,
            match verdict {
                Ok(_) if moving * 2 > recent.len() => Verdict::Moving,
                Ok(estimate)
                    if ppg::slow_while_busy(
                        estimate.bpm_at(session.period),
                        recent,
                        CALM,
                        session.asleep,
                    ) =>
                {
                    Verdict::SlowWhileBusy
                }
                verdict => Verdict::Window(verdict),
            },
            calm,
        ));
    }
    verdicts
}

/// What `heart::SESSION_LIMIT` is: when a session's first phase runs out.
const SESSION_LIMIT: f32 = 30.0;

/// How long a segment's phase runs, at most: a session's first, or a
/// brighter one.
///
/// A session runs its phase out on a busy wrist too (it used to stop
/// early; see `heart_schedule`'s "Trying through movement"), so a replay
/// ends at the reading, or where the recording does.
fn phase_limit(session: &Recorded) -> f32 {
    if session.brighter {
        heart_schedule::BRIGHTER_FOR as f32
    } else {
        SESSION_LIMIT
    }
}

/// The brighter drive the firmware would go on at once this segment's
/// phase ran out without a reading, as `heart` decides it (see
/// `heart_schedule::brighter`), with the budget to spare. `ended`: the
/// replay's reading came first.
fn goes_brighter(
    session: &Recorded,
    windows: &[(usize, Verdict, bool)],
    ended: bool,
) -> Option<u8> {
    let limit = phase_limit(session);
    let length = session.samples.len() as f32 * session.period / 1e6;
    // A recording ends a part-second after its phase did.
    if !session.background || ended || length < limit - 1.0 {
        return None;
    }
    let end = (limit * 1e6 / session.period) as usize;
    let last_failed =
        windows
            .iter()
            .rev()
            .find(|(i, _, _)| *i < end)
            .is_some_and(|(_, verdict, calm)| {
                *calm
                    && matches!(
                        verdict,
                        Verdict::Window(Err(Rejection::Faint(_) | Rejection::Aperiodic { .. }))
                    )
            });
    let checks = (end.min(session.samples.len()) / LEVEL_CHECK_SAMPLES).min(session.movement.len());
    let busy = session.movement[..checks]
        .iter()
        .filter(|&&m| m > CALM)
        .count();
    // The firmware's counts are the whole session's; a brighter segment
    // has only its own phase's, which were calm for it to start.
    let calm = heart_schedule::calm_throughout(busy as u8, checks as u8);
    let second = checks.checked_sub(1)?;
    let peak = session.samples[second * LEVEL_CHECK_SAMPLES..][..LEVEL_CHECK_SAMPLES]
        .iter()
        .copied()
        .max()?;
    // A bright arm's start counts as a step, as in `heart::measure`.
    let steps = if session.cont && session.bright == Some(true) {
        // Stepped back down from a bright start: for good.
        heart_schedule::BRIGHTER_STEPS
    } else {
        u8::from(session.brighter || (session.bright == Some(true) && session.drive > 0))
    };
    (last_failed && calm)
        .then(|| heart_schedule::brighter(session.drive, steps, peak, heart_schedule::LED_BUDGET))
        .flatten()
}

/// A segment's light, for comparing one drive with another on the same
/// wrist: its mean level (DC), the median of its seconds' peak over their
/// mean (about the pulse's height, at rest), and the median quality of its
/// windows that had one.
struct Light {
    dc: f32,
    pulse: f32,
    quality: f32,
}

fn light(session: &Recorded, windows: &[(usize, Verdict, bool)]) -> Light {
    let samples = &session.samples;
    let dc = samples.iter().sum::<u32>() as f32 / samples.len().max(1) as f32;
    let median = |mut v: Vec<f32>| {
        v.sort_by(f32::total_cmp);
        v.get(v.len() / 2).copied().unwrap_or(f32::NAN)
    };
    // The first second holds the LED's warm-up.
    let pulse = median(
        samples
            .as_chunks::<LEVEL_CHECK_SAMPLES>()
            .0
            .iter()
            .skip(1)
            .map(|second| {
                let mean = second.iter().sum::<u32>() as f32 / second.len() as f32;
                *second.iter().max().expect("a whole second") as f32 - mean
            })
            .collect(),
    );
    let quality = median(
        windows
            .iter()
            .filter_map(|(_, verdict, _)| match verdict {
                Verdict::Window(Ok(e) | Err(Rejection::Faint(e))) => Some(e.quality),
                Verdict::Window(Err(Rejection::Aperiodic { quality, .. })) => Some(*quality),
                _ => None,
            })
            .filter(|q| q.is_finite())
            .collect(),
    );
    Light { dc, pulse, quality }
}

/// The reading the firmware would make from a recorded session's windows
/// before `end` (seconds): the BPM, and when. `strict`: as before calm
/// pairs (see `replay`).
fn reading(
    session: &Recorded,
    windows: &[(usize, Verdict, bool)],
    end: f32,
    strict: bool,
) -> Option<(u8, f32)> {
    let mut consensus = Consensus::new();
    for (i, verdict, calm) in windows {
        let at = *i as f32 * session.period / 1e6;
        if at >= end {
            return None;
        }
        let pushed = match verdict {
            Verdict::Window(Ok(estimate)) => {
                consensus.push(estimate.bpm_at(session.period), *calm && !strict)
            }
            Verdict::Window(Err(Rejection::Faint(estimate))) if !strict => {
                consensus.push_faint(estimate.bpm_at(session.period), *calm, session.asleep)
            }
            _ => {
                consensus.reject();
                None
            }
        };
        if let Some(bpm) = pushed {
            return Some((bpm, at));
        }
    }
    None
}

/// One arm of the LED experiment (see `heart_schedule::Arm`), over a
/// replay: what the sessions that started on it made of their LED, and
/// its rounds of tries.
#[derive(Default)]
struct Arm {
    sessions: usize,
    /// Sessions by the drive they started at (a bright arm falls back to
    /// 20 mA, or the usual drive, without the headroom or budget).
    drives: [usize; 4],
    /// Readings, in the replay and by the firmware that recorded them.
    read: usize,
    recorded: usize,
    /// LED seconds, and their cost at 12.5 mA's rate (the budget's).
    led: f32,
    weighted: f32,
    /// Error against the reference, over the readings that have one.
    error: f32,
    referenced: usize,
    /// Each session's first phase: pulse over DC, and median window
    /// quality (its median over the sessions is what's printed: a few
    /// sessions on a moving wrist would swamp a mean).
    pulse_dc: Vec<f32>,
    quality: Vec<f32>,
    /// Rounds (logs after 0.10.0-rc.3), and whether each read, in the
    /// replay and as recorded: the fair comparison, as a round's arm is
    /// chosen before any of its tries, while which sessions there are
    /// depends on how the ones before them went.
    rounds: std::collections::BTreeMap<u16, (bool, bool)>,
}

impl Arm {
    fn print(&self, name: &str) {
        let median = |v: &[f32]| {
            let mut v: Vec<_> = v.iter().copied().filter(|x| x.is_finite()).collect();
            v.sort_by(f32::total_cmp);
            v.get(v.len() / 2).copied().unwrap_or(f32::NAN)
        };
        println!(
            "  {name:6} {:3} sessions (at drive {:?}), {:2} read ({} recorded), {:.2} per \
             LED minute ({:.2} at 12.5 mA's cost), LED {:.0} s; MAE {:.1} BPM over {}; \
             pulse/DC {:.2}‰, quality {:.2}",
            self.sessions,
            self.drives,
            self.read,
            self.recorded,
            self.read as f32 * 60.0 / self.led.max(1.0),
            self.read as f32 * 60.0 / self.weighted.max(1.0),
            self.led,
            self.error / self.referenced.max(1) as f32,
            self.referenced,
            median(&self.pulse_dc) * 1000.0,
            median(&self.quality),
        );
        if !self.rounds.is_empty() {
            let count =
                |f: fn(&(bool, bool)) -> bool| self.rounds.values().filter(|r| f(r)).count();
            println!(
                "  {:6} {:3} rounds, {:2} read ({} recorded), {:.1} sessions a round",
                "",
                self.rounds.len(),
                count(|r| r.0),
                count(|r| r.1),
                self.sessions as f32 / self.rounds.len() as f32,
            );
        }
    }
}

/// Replays sessions extracted from a field log through `Ppg` and
/// `Consensus`, vetoing windows on movement as the firmware does, and
/// prints each window's verdict next to the reference, and the reading the
/// firmware would now make. `strict` replays the rule before calm pairs
/// (see `ppg::Consensus`): only three accepted windows make a reading.
///
/// A session ends at its reading, or where its recording does: the totals
/// count the LED time to then. (Recordings end where the firmware that
/// made them stopped, up to 0.10.0-rc.3 often within seconds on a busy
/// wrist, so sessions are a lower bound on what a longer one would have
/// read.)
///
/// Where the firmware would go on with a brighter LED (see
/// `heart_schedule::brighter`), it says so; a brighter phase that was
/// recorded is replayed as part of its session, and its light and windows
/// compared with the phase before it, on the same wrist seconds earlier.
fn replay(path: &str, strict: bool) {
    let text = std::fs::read_to_string(path).expect("readable sessions file");
    let (mut sessions, mut read, mut referenced) = (0, 0, 0);
    let (mut error, mut wrong, mut read_at, mut led) = (0.0, 0, 0.0, 0.0);
    // Brighter phases: the firmware's choices to go on brighter in the
    // replay, the brighter segments recorded, and of those, how many read;
    // and the light at the drive before each, to compare.
    let (mut would_brighten, mut brighter, mut brighter_read) = (0, 0, 0);
    let mut before: Option<Light> = None;
    let mut compared = Vec::new();
    // The LED experiment's arms: dim, bright.
    let mut arms = [Arm::default(), Arm::default()];
    for session in parse_sessions(&text) {
        let seconds = |i: usize| i as f32 * session.period / 1e6;
        let mut length = seconds(session.samples.len());
        if session.brighter {
            length = length.min(phase_limit(&session) + 1.0);
        }
        let mut verdicts = Vec::new();
        let windows = windows(&session);
        let reading = reading(&session, &windows, length, strict);
        let lit = light(&session, &windows);
        let (lit_pulse, lit_dc, lit_quality) = (lit.pulse, lit.dc, lit.quality);
        if session.brighter {
            brighter += 1;
            brighter_read += usize::from(reading.is_some());
            verdicts.push(format!("(drive {}", session.drive));
            if let Some(dim) = &before {
                verdicts.push(format!(
                    "after DC {:.0} pulse {:.1} q {:.2}; now DC {:.0} pulse {:.1} q {:.2})",
                    dim.dc, dim.pulse, dim.quality, lit.dc, lit.pulse, lit.quality
                ));
                compared.push((
                    dim.pulse / dim.dc,
                    lit.pulse / lit.dc,
                    dim.quality,
                    lit.quality,
                ));
            } else {
                verdicts.push("without the phase before)".into());
            }
        }
        if let Some(drive) = goes_brighter(&session, &windows, reading.is_some()) {
            would_brighten += 1;
            verdicts.push(format!("[brighter: drive {drive}]"));
        }
        before = Some(lit);
        for (_, verdict, calm) in windows {
            verdicts.push(match verdict {
                Verdict::Moving => "moving".into(),
                Verdict::SlowWhileBusy => "slow-busy".into(),
                Verdict::Window(Ok(estimate)) => format!(
                    "{:.0}({:.2},{:.0}%)",
                    estimate.bpm_at(session.period),
                    estimate.quality,
                    estimate.drift * 100.0
                ),
                Verdict::Window(Err(Rejection::Faint(estimate))) => format!(
                    "f{:.0}({:.2}{})",
                    estimate.bpm_at(session.period),
                    estimate.quality,
                    if calm { ",calm" } else { "" }
                ),
                Verdict::Window(Err(Rejection::Flat)) => "flat".into(),
                Verdict::Window(Err(Rejection::Unsteady { ratio })) => format!("U{ratio:.1}"),
                Verdict::Window(Err(Rejection::Aperiodic { quality, bpm })) => {
                    format!("a{bpm:.0}({quality:.2})")
                }
                Verdict::Window(Err(Rejection::Drifting { drift, bpm })) => {
                    format!("d{bpm:.0}({:.0}%)", drift * 100.0)
                }
            });
        }
        // A brighter phase, or a new level, is the same session going on.
        sessions += usize::from(!session.brighter && !session.cont);
        // The LED is on until the reading, or the end; plus the half
        // second before the first sample.
        let lit_for = 0.5 + reading.map_or(length, |(_, at)| at);
        led += lit_for;
        if let Some(bright) = session.bright {
            let arm = &mut arms[usize::from(bright)];
            if !session.brighter && !session.cont {
                let name = if bright { "bright" } else { "dim" };
                verdicts.insert(0, format!("[{name} at drive {}]", session.drive));
                arm.sessions += 1;
                arm.drives[usize::from(session.drive & 3)] += 1;
                arm.pulse_dc.push(lit_pulse / lit_dc);
                arm.quality.push(lit_quality);
            }
            arm.led += lit_for;
            arm.weighted += lit_for
                * heart_schedule::DRIVE_CURRENT[usize::from(session.drive & 3)] as f32
                / heart_schedule::DRIVE_CURRENT[0] as f32;
            let recorded = session.outcome.parse::<u8>().is_ok();
            arm.recorded += usize::from(recorded);
            if let Some(round) = session.round {
                let read = arm.rounds.entry(round).or_default();
                *read = (read.0 || reading.is_some(), read.1 || recorded);
            }
            if let Some((bpm, _)) = reading {
                arm.read += 1;
                if session.reference > 0.0 {
                    arm.referenced += 1;
                    arm.error += (f32::from(bpm) - session.reference).abs();
                }
            }
        }
        let verdict = match reading {
            Some((bpm, at)) => {
                read += 1;
                read_at += at;
                if session.reference > 0.0 {
                    let off = (f32::from(bpm) - session.reference).abs();
                    referenced += 1;
                    error += off;
                    wrong += usize::from(off > 8.0);
                }
                format!("{bpm} BPM at {at:.0} s")
            }
            None => "none".into(),
        };
        println!(
            "{} ref {:3.0} fw {:>4} replay {verdict:16} {}",
            session.start,
            session.reference,
            session.outcome,
            verdicts.join(" ")
        );
    }
    println!(
        "{read}/{sessions} sessions read, at {:.1} s on average; of {referenced} with a \
         reference, MAE {:.1} BPM, {wrong} more than 8 BPM off",
        read_at / read.max(1) as f32,
        error / referenced.max(1) as f32
    );
    println!("LED {led:.0} s in all");
    println!(
        "{would_brighten} would go on brighter; {brighter} brighter phases recorded, \
         {brighter_read} of them read"
    );
    if !compared.is_empty() {
        // Pulse over DC is what more light can't change if the pulse is
        // all that's lit; quality is what it should improve if the noise
        // is the sensor's own rather than the light's.
        let n = compared.len() as f32;
        let mean = |f: fn(&(f32, f32, f32, f32)) -> f32| {
            compared
                .iter()
                .map(f)
                .filter(|v| v.is_finite())
                .sum::<f32>()
                / n
        };
        let better = compared.iter().filter(|c| c.3 > c.2).count();
        println!(
            "brighter vs the phase before: pulse/DC {:.2}‰ -> {:.2}‰, median window \
             quality {:.2} -> {:.2}, better in {better} of {}",
            mean(|c| c.0 * 1000.0),
            mean(|c| c.1 * 1000.0),
            mean(|c| c.2),
            mean(|c| c.3),
            compared.len()
        );
    }
    if arms.iter().any(|arm| arm.sessions > 0) {
        // Awake background sessions, taking turns at a dim and a bright
        // start: the same wrist, day, and rules.
        println!("LED experiment, awake (dim vs bright start):");
        arms[0].print("dim");
        arms[1].print("bright");
    }
}

fn main() {
    if let Some(path) = std::env::args().nth(1) {
        return replay(
            &path,
            std::env::args().nth(2).as_deref() == Some("--strict"),
        );
    }
    println!("== pulses (20 seeds each; readings within a 30 s session, 40 s with motion) ==");
    for (name, scenario) in scenarios() {
        print_row(&name, &summarise(&scenario, 20, no_gate, true));
    }
    println!("== no pulse (20 seeds each, 300 s), three windows must agree ==");
    for (name, scenario) in garbage() {
        print_row(&name, &summarise(&scenario, 20, no_gate, false));
    }
    println!("== no pulse, calm pairs, with the accelerometer veto (200 seeds) ==");
    for (name, scenario) in garbage() {
        print_row(
            &name,
            &summarise(&scenario, 200, accelerometer_gate(&scenario), true),
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
            let s = summarise(&scenario, SEEDS, no_gate, true);
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

    /// No pulse, no reading from three agreeing windows: with the
    /// accelerometer veto in place, as in the firmware, and without it. At a
    /// desk the hand moves too little for the veto (`heart::MOTION_LIMIT`),
    /// but plenty to make bursts in the signal, so the signal checks must
    /// keep motion out on their own. (Before motion was masked rather than
    /// rejected whole, sustained random motion passed for a pulse in 18 of
    /// these 20 runs; the steadiness check on what's left after masking now
    /// catches it.)
    #[test]
    fn garbage_never_yields_a_reading() {
        for (name, scenario) in garbage() {
            let s = summarise(&scenario, SEEDS, accelerometer_gate(&scenario), false);
            assert_eq!(s.with_reading, 0, "{name}: got a reading from no pulse");
            let s = summarise(&scenario, SEEDS, no_gate, false);
            assert_eq!(s.with_reading, 0, "{name}: got a reading without the veto");
        }
    }

    /// On a calm wrist two faint windows in a row are enough (see
    /// `ppg::Consensus`), and filtered noise can do that by chance: rarely.
    /// Over 200 runs of 300 s (~2000 sessions' worth) per scenario, that's
    /// what's bounded here, against regressions: 2-4 runs of white noise
    /// made a reading, about one in 500-1000 sessions of a still wrist with
    /// no pulse to be seen. (Motion is never calm, so the veto keeps it
    /// out.)
    #[test]
    fn garbage_rarely_pairs_on_a_calm_wrist() {
        for (name, scenario) in garbage() {
            let s = summarise(&scenario, 200, accelerometer_gate(&scenario), true);
            assert!(
                s.with_reading <= 8,
                "{name}: {} of 200 runs got a reading from no pulse",
                s.with_reading
            );
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
            let s = summarise(&scenario, SEEDS, no_gate, false);
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
        let outcome = run(&polls, no_gate, true);
        assert!(!outcome.windows.is_empty());
        assert!(
            outcome
                .windows
                .iter()
                .all(|(_, v)| *v == Err(Rejection::Flat))
        );
    }

    /// A short movement is masked out of the windows it falls in: most are
    /// read from the pulse either side of it, at the true rate, and the
    /// rest rejected. A window that's more than half movement is always
    /// rejected (the movement's edges, where it starts and stops, are
    /// gentle enough to pass for pulse).
    #[test]
    fn motion_is_masked_or_rejected() {
        // (start s, duration s, whether some windows over it may be read)
        for (start, duration, readable) in
            [(12.0, 1.5, true), (12.0, 2.0, true), (10.0, 6.0, false)]
        {
            let scenario = Scenario {
                motion: vec![(start, duration, 1200.0)],
                ..Scenario::pulse(70.0, 9)
            };
            let (mut over, mut read) = (0, 0);
            for seed in 0..SEEDS {
                let scenario = Scenario {
                    seed,
                    ..scenario.clone()
                };
                let (polls, beats) = generate_with_beats(&scenario);
                for (t, verdict) in run(&polls, no_gate, true).windows {
                    // The window ending at `t` covers the 8 s before it.
                    let overlap = t.min(start + duration) - (t - 8.0).max(start);
                    if overlap <= 0.0 {
                        continue;
                    }
                    over += 1;
                    let Ok(bpm) = verdict else { continue };
                    read += 1;
                    let truth = realised_bpm(&beats, t - 8.0, t);
                    assert!(
                        overlap < 4.0,
                        "{duration} s at {start} s, seed {seed}: read {bpm:.1} BPM at {t:.1} s"
                    );
                    assert!(
                        (f64::from(bpm) - truth).abs() <= tolerance(&scenario),
                        "{duration} s at {start} s, seed {seed}: read {bpm:.1} BPM at {t:.1} s, \
                         truth {truth:.1}"
                    );
                }
            }
            println!("{duration} s of motion: {read}/{over} windows over it read");
            if duration <= 2.0 {
                assert!(read * 2 > over, "{duration} s: only {read}/{over} read");
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
            let measured = f64::from(run(&generate(&scenario), no_gate, true).period_us);
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

    /// The accepted windows' rates in a recorded session.
    fn accepted(session: &Recorded) -> Vec<f32> {
        windows(session)
            .into_iter()
            .filter_map(|(_, verdict, _)| match verdict {
                Verdict::Window(Ok(estimate)) => Some(estimate.bpm_at(session.period)),
                _ => None,
            })
            .collect()
    }

    /// Desk sessions from a field log in which the sensor shifted on the
    /// skin, and the shifting passed for a slow pulse: one read 42 BPM
    /// with a Garmin on the other wrist at 67-69. (Its third window isn't
    /// in the log, which ends each session at its last whole second.) And
    /// one from the next day in which a busy wrist (movement 450-950)
    /// swung the light slowly while its level held, at 44-46 BPM with the
    /// Garmin at 71: the warm-up fix let its third window through too,
    /// which made it a reading, until a slow window needed a calm wrist.
    #[test]
    fn a_shifting_sensor_is_not_a_slow_pulse() {
        let text = include_str!("../fixtures/desk-drift.txt");
        for session in parse_sessions(text) {
            let slow: Vec<_> = accepted(&session)
                .into_iter()
                .filter(|&bpm| bpm < 50.0)
                .collect();
            assert!(slow.is_empty(), "{}: accepted {slow:?}", session.start);
        }
    }

    /// The same night's sessions with slow genuine pulses (45-50 BPM,
    /// asleep, the Garmin agreeing): every window the firmware accepted is
    /// still accepted. (The reference is the Garmin's nearest sample, on
    /// the other wrist, which can be a few BPM off a 12 s session.)
    #[test]
    fn slow_night_pulses_are_still_read() {
        let text = include_str!("../fixtures/night-slow.txt");
        for session in parse_sessions(text) {
            let rates = accepted(&session);
            assert!(rates.len() >= 2, "{}: only {rates:?}", session.start);
            for bpm in rates {
                assert!(
                    (bpm - session.reference).abs() <= 10.0,
                    "{}: {bpm:.0} BPM against {}",
                    session.start,
                    session.reference
                );
            }
        }
    }

    /// Sessions (an afternoon at 59 BPM, a night at 46) whose first
    /// conversion came out darker than the rest, as most do. Fed to the
    /// filters, that step drowned the first window's pulse (it was read as
    /// aperiodic, quality ~0.3); skipped, all three windows to a reading
    /// agree with the Garmin.
    #[test]
    fn the_first_window_survives_the_led_warming_up() {
        let text = include_str!("../fixtures/warmup.txt");
        for session in parse_sessions(text) {
            let [first, second, ..] = session.samples[..] else {
                panic!("{}: too short", session.start);
            };
            assert!(
                first * 100 < second * 97,
                "{}: no warm-up step",
                session.start
            );
            let rates = accepted(&session);
            assert!(rates.len() >= 3, "{}: only {rates:?}", session.start);
            assert!(
                (rates[0] - session.reference).abs() <= 3.0,
                "{}: first window {:.1} BPM against {}",
                session.start,
                rates[0],
                session.reference
            );
        }
    }

    /// Desk sessions (today's one of the field log, 2026-11-09, and the
    /// day before's) with a pulse between the hand's bursts: each has a
    /// window with a burst in it that used to be rejected whole, short of
    /// the three a reading needs. Masked out, all of them agree with the
    /// Garmin.
    #[test]
    fn desk_bursts_are_masked_out() {
        let text = include_str!("../fixtures/desk-burst.txt");
        for session in parse_sessions(text) {
            let rates = accepted(&session);
            assert!(rates.len() >= 3, "{}: only {rates:?}", session.start);
            for bpm in rates {
                assert!(
                    (bpm - session.reference).abs() <= 5.0,
                    "{}: {bpm:.0} BPM against {}",
                    session.start,
                    session.reference
                );
            }
        }
    }

    /// The reading a recorded session would now make.
    fn replayed(session: &Recorded, strict: bool) -> Option<(u8, f32)> {
        reading(session, &windows(session), f32::INFINITY, strict)
    }

    /// Calm sessions from field logs whose pulse was too faint for three
    /// windows to pass on their own: lying still awake (2026-11-09, the
    /// Garmin at 67 then 65, two windows of quality 0.46-0.47 near the end),
    /// and asleep (45-49 BPM, the Garmin at 47). Two in a row agree.
    #[test]
    fn a_calm_faint_pulse_is_read() {
        let text = include_str!("../fixtures/calm-faint.txt");
        for session in parse_sessions(text) {
            assert_eq!(replayed(&session, true), None, "{}", session.start);
            let (bpm, at) = replayed(&session, false).expect("a reading");
            assert!(
                (f32::from(bpm) - session.reference).abs() <= 3.0,
                "{}: {bpm} BPM at {at:.0} s against {}",
                session.start,
                session.reference
            );
        }
    }

    /// The same hour's other two sessions, as calm, in which no pulse can
    /// be seen (slower swings, at 30-46 BPM, outweigh it): no reading.
    #[test]
    fn a_calm_wrist_without_a_visible_pulse_is_not_read() {
        let text = include_str!("../fixtures/calm-pulseless.txt");
        for session in parse_sessions(text) {
            assert_eq!(replayed(&session, false), None, "{}", session.start);
        }
    }

    /// Those sessions would go on brighter, had the schedule started
    /// them: they ran out calm, on a faint or aperiodic window. Sessions
    /// that read don't, nor do ones on a restless wrist.
    #[test]
    fn a_calm_wrist_without_a_visible_pulse_goes_brighter() {
        let text = include_str!("../fixtures/calm-pulseless.txt");
        for mut session in parse_sessions(text) {
            session.background = true;
            let windows = windows(&session);
            assert_eq!(
                goes_brighter(&session, &windows, false),
                Some(2),
                "{}",
                session.start
            );
            // Not twice from the same light, nor past 40 mA.
            session.brighter = true;
            session.drive = 3;
            assert_eq!(
                goes_brighter(&session, &windows, false),
                None,
                "{}",
                session.start
            );
        }
        for session in parse_sessions(include_str!("../fixtures/desk-calm.txt")) {
            let windows = windows(&session);
            let ended = replayed(&session, false).is_some();
            assert!(ended, "{}", session.start);
            assert_eq!(
                goes_brighter(&session, &windows, ended),
                None,
                "{}",
                session.start
            );
        }
        // A restless desk's sessions (recorded only to 12 s) weren't calm
        // throughout, as the firmware judges it at the end of the phase.
        for session in parse_sessions(include_str!("../fixtures/desk-restless.txt")) {
            let busy = session.movement.iter().filter(|&&m| m > CALM).count();
            let checks = session.movement.len();
            assert!(
                !heart_schedule::calm_throughout(busy as u8, checks as u8),
                "{}",
                session.start
            );
        }
    }

    /// The LED experiment's arms (see `heart_schedule::Arm`) and rounds
    /// parse, and a bright start counts as a brighter step, as
    /// `heart::measure` counts it.
    #[test]
    fn arms_are_parsed() {
        let line = "1 70 37500.0 none 2500 2510 | 30 | background drive=2 arm=bright";
        let session = &parse_sessions(line)[0];
        assert_eq!(
            (session.bright, session.drive, session.cont),
            (Some(true), 2, false)
        );
        let session = &parse_sessions(&line.replace("drive=2 arm=bright", "cont arm=dim"))[0];
        assert_eq!(
            (session.bright, session.drive, session.cont),
            (Some(false), 0, true)
        );
        assert_eq!(
            parse_sessions(&line.replace(" arm=bright", ""))[0].bright,
            None
        );
        assert_eq!(session.round, None);
        let line = line.replace("arm=bright", "arm=bright round=12");
        assert_eq!(parse_sessions(&line)[0].round, Some(12));
    }

    /// Background sessions at a desk (2026-11-09, the Garmin at 84 and
    /// 80) whose wrist started typing again within seconds: every window
    /// is rejected, so running on through the movement (as sessions now
    /// do) makes no wrong reading of it.
    #[test]
    fn a_restless_desk_is_not_misread() {
        let text = include_str!("../fixtures/desk-restless.txt");
        for session in parse_sessions(text) {
            assert!(session.background, "{}", session.start);
            assert_eq!(replayed(&session, false), None, "{}", session.start);
            assert!(
                windows(&session)
                    .iter()
                    .all(|(_, verdict, _)| !matches!(verdict, Verdict::Window(Ok(_)))),
                "{}",
                session.start
            );
        }
    }

    /// Background sessions at a desk that day (the Garmin at 82 and 67)
    /// whose wrist kept still: both read, one at 28 s.
    #[test]
    fn a_still_desk_is_read() {
        let text = include_str!("../fixtures/desk-calm.txt");
        for session in parse_sessions(text) {
            assert!(session.background, "{}", session.start);
            let (bpm, at) = replayed(&session, false).expect("a reading");
            assert!(
                (f32::from(bpm) - session.reference).abs() <= 5.0,
                "{}: {bpm} BPM at {at:.0} s against {}",
                session.start,
                session.reference
            );
        }
    }

    #[test]
    fn consensus_needs_agreement() {
        let mut c = Consensus::new();
        assert_eq!(c.push(70.0, false), None);
        assert_eq!(c.push(71.0, false), None);
        assert_eq!(c.push(72.4, false), Some(71));
        // One outlier spoils the next two readings.
        assert_eq!(c.push(140.0, false), None);
        assert_eq!(c.push(71.0, false), None);
        assert_eq!(c.push(70.0, false), None);
        assert_eq!(c.push(70.5, false), Some(71));
        // Rejected windows in between don't matter.
        let mut c = Consensus::new();
        assert_eq!(c.push(70.0, false), None);
        c.reject();
        assert_eq!(c.push(71.0, false), None);
        c.reject();
        assert_eq!(c.push(72.0, false), Some(71));
    }

    /// On a calm wrist, two windows in a row make a reading if they agree
    /// closely, faint or not; anything else between them, or a busy wrist
    /// through either, and they don't.
    #[test]
    fn calm_windows_in_a_row_agree() {
        let mut c = Consensus::new();
        assert_eq!(c.push_faint(61.8, true, false), None);
        assert_eq!(c.push_faint(62.4, true, false), Some(62));
        // Accepted and faint together.
        let mut c = Consensus::new();
        assert_eq!(c.push(64.0, true), None);
        assert_eq!(c.push_faint(65.0, true, false), Some(65));
        // Too far apart.
        let mut c = Consensus::new();
        assert_eq!(c.push_faint(60.0, true, false), None);
        assert_eq!(c.push_faint(63.5, true, false), None);
        // A rejected window between them.
        let mut c = Consensus::new();
        assert_eq!(c.push_faint(62.0, true, false), None);
        c.reject();
        assert_eq!(c.push_faint(62.0, true, false), None);
        // Not calm, either of them.
        let mut c = Consensus::new();
        assert_eq!(c.push_faint(62.0, false, false), None);
        assert_eq!(c.push_faint(62.0, true, false), None);
        assert_eq!(c.push_faint(62.0, false, false), None);
        // Faint windows never count towards the three accepted ones.
        let mut c = Consensus::new();
        assert_eq!(c.push(70.0, false), None);
        assert_eq!(c.push_faint(70.0, false, false), None);
        assert_eq!(c.push(70.0, false), None);
        assert_eq!(c.push_faint(70.0, false, false), None);
        assert_eq!(c.push(70.0, false), Some(70));
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
