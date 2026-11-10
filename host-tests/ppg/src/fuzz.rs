//! Fuzzing, with Bolero: the tests above feed synthetic pulses of known
//! rates and check what's read; these feed any samples (and any polls,
//! at any times), and check what the module promises whatever the light
//! does: no panic, verdicts every [`ppg::ESTIMATE_EVERY`] samples once
//! the window is full, rates within the 40–200 BPM reported, the
//! qualities each verdict is documented with, nothing NaN that isn't
//! meant to be, and a measured sample period near the datasheet's.
//!
//! `cargo test` runs each harness over a few hundred random inputs (and
//! any saved in a corpus), so they're regression tests too. To fuzz one
//! properly (nightly, for the sanitizer):
//!
//! ```text
//! cargo +nightly bolero test fuzz::any_light_reads_as_a_heart_rate_or_not --engine libfuzzer -T 10min
//! ```

use super::ppg::{
    Consensus, Conversions, ESTIMATE_EVERY, Estimate, NOMINAL_PERIOD_US, Ppg, Rejection, WINDOW,
};

/// The rates the module reports between.
const RATES: core::ops::RangeInclusive<f32> = 40.0..=200.0;

/// What an estimate must be, of a window that wasn't rejected outright:
/// at a reported rate, as periodic as `qualities`, and from a light level
/// that moved by a finite, non-negative fraction.
fn check(estimate: &Estimate, qualities: core::ops::Range<f32>) {
    let bpm = estimate.bpm();
    assert!(RATES.contains(&bpm), "{bpm} BPM from {estimate:?}");
    assert!(estimate.period > 0.0, "{estimate:?}");
    // A correlation is at most 1.
    assert!(
        qualities.contains(&estimate.quality) && estimate.quality <= 1.0,
        "{estimate:?} not in {qualities:?}"
    );
    assert!(
        estimate.drift >= 0.0 && estimate.drift.is_finite(),
        "{estimate:?}"
    );
}

/// Any light, through [`Ppg`] and [`Consensus`] as `heart` uses them:
/// nothing panics; a verdict comes every [`ESTIMATE_EVERY`] samples once
/// the first [`WINDOW`] are in (and the sensor's warm-up and the filters'
/// settling, under a second, before them); accepted and faint windows are
/// at reported rates, of the qualities their docs give (accepted at 0.55
/// or more, faint at 0.45 to 0.6), drifting ones slow; and any reading
/// is a rate the watch reports.
///
/// The light is a pulse (any rate, size and shape, on any level), with
/// noise and bursts of motion, as a wrist's is, so that every verdict
/// comes up; or, now and then, any counts at all.
#[test]
fn any_light_reads_as_a_heart_rate_or_not() {
    bolero::check!()
        .with_type::<(u32, u16, u8, u8, Vec<(i16, u8)>, u8, bool)>()
        .for_each(|&(level, size, period, times, ref noise, calm_bits, any)| {
            let level = if any { level } else { level % (1 << 20) };
            // A beat every 6 to 45 samples: 36 to 267 BPM.
            let period = 6.0 + f32::from(period % 40);
            let samples = 300 + usize::from(times) * 8;
            let raw = |i: usize| -> u32 {
                let (noise, burst) = noise.get(i % noise.len().max(1)).copied().unwrap_or((0, 0));
                if any {
                    return level
                        .wrapping_add(u32::from(noise as u16))
                        .wrapping_mul(u32::from(burst) + 1);
                }
                // A dip a beat, sharper than a sine as a pulse is, plus
                // noise, plus (where `burst` says) motion many times its
                // size.
                let phase = (i as f32 / period).fract();
                let dip = f32::from(size % 2000) * (1.0 - phase).powi(3);
                let motion = if burst > 0xE0 {
                    f32::from(noise) * 8.0
                } else {
                    0.0
                };
                let x = level as f32 - dip + f32::from(noise % 64) + motion;
                x.max(0.0) as u32
            };

            let mut ppg = Ppg::new();
            let mut consensus = Consensus::new();
            let mut last_verdict: Option<usize> = None;
            for i in 0..samples {
                let Some(verdict) = ppg.push(raw(i)) else {
                    continue;
                };
                let n = i + 1;
                // The first once the window is full (after warming up and
                // settling, under a second), then every `ESTIMATE_EVERY`.
                match last_verdict {
                    None => assert!(
                        (WINDOW..=WINDOW + 27).contains(&n),
                        "first verdict after {n}"
                    ),
                    Some(last) => assert_eq!(n - last, ESTIMATE_EVERY),
                }
                last_verdict = Some(n);

                let calm = calm_bits >> (n / ESTIMATE_EVERY % 8) & 1 != 0;
                let reading = match verdict {
                    Ok(estimate) => {
                        check(&estimate, 0.55..1.0 + f32::EPSILON);
                        consensus.push(estimate.bpm(), calm)
                    }
                    Err(Rejection::Faint(estimate)) => {
                        check(&estimate, 0.45..0.6);
                        consensus.push_faint(estimate.bpm(), calm, calm_bits & 1 != 0)
                    }
                    Err(Rejection::Drifting { drift, bpm }) => {
                        assert!(drift > 0.1, "drifting by {drift}");
                        assert!((40.0..50.0).contains(&bpm), "drifting at {bpm}");
                        consensus.reject();
                        None
                    }
                    Err(Rejection::Aperiodic { quality, bpm }) => {
                        assert!(!quality.is_nan(), "{verdict:?}");
                        // Either there was no peak (a NaN rate), or it was
                        // too weak, or out of range.
                        assert!(
                            bpm.is_nan() || quality < 0.45 || !RATES.contains(&bpm),
                            "{verdict:?}"
                        );
                        consensus.reject();
                        None
                    }
                    Err(Rejection::Unsteady { ratio }) => {
                        assert!(ratio >= 1.0, "{verdict:?}");
                        consensus.reject();
                        None
                    }
                    Err(Rejection::Flat) => {
                        consensus.reject();
                        None
                    }
                };
                if let Some(bpm) = reading {
                    assert!((40..=200).contains(&bpm), "a reading of {bpm} BPM");
                }
            }
            if samples >= WINDOW + 27 {
                assert!(last_verdict.is_some(), "no verdict in {samples} samples");
            }
        });
}

/// Any polls of the sensor, at any (increasing) times, any value: nothing
/// panics; every sample emitted is a value that was polled; the measured
/// period stays within what's believed (0.75 to 1.33 times the nominal);
/// and no more samples come out than conversions could have happened in
/// the time (not more than two per period, with the slack for torn reads
/// and late polls).
#[test]
fn any_polls_make_conversions_at_the_sensors_pace() {
    bolero::check!()
        .with_type::<(Vec<(u8, u16)>, u8)>()
        .for_each(|(polls, restart)| {
            let nominal = NOMINAL_PERIOD_US as f32;
            let mut conversions = Conversions::new();
            let mut now = 0u64;
            let mut seen = Vec::new();
            let mut emitted = 0usize;
            for (i, &(value, wait)) in polls.iter().enumerate() {
                // Polls every few ms, now and then much later.
                now += u64::from(wait) * if wait > 0xF000 { 64 } else { 1 };
                if i == usize::from(*restart) {
                    conversions.restart();
                }
                // Few distinct values, so that repeats come up.
                let raw = u32::from(value % 4) * 1000;
                seen.push(raw);
                conversions.poll(raw, now, |sample| {
                    assert!(seen.contains(&sample), "{sample} was never polled");
                    emitted += 1;
                });
                let period = conversions.period_us();
                assert!(
                    (0.75 * nominal..1.33 * nominal).contains(&period),
                    "a period of {period} µs"
                );
            }
            let most = 2 * (now as f32 / (0.75 * nominal)) as usize + 2;
            assert!(emitted <= most, "{emitted} samples in {now} µs");
        });
}
