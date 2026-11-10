//! Host tests for the firmware's `glance` module, on poses read from the
//! sealed watch's field log (sensor axes, 1024 per g). `cargo test`.

// The poses and helpers are the tests'; `cargo build` has no use for them.
#![cfg_attr(not(test), allow(dead_code))]

#[path = "../../../src/glance.rs"]
#[allow(dead_code)]
mod glance;

use glance::{Detector, from_sensor};

/// Arm hanging, as logged (sensor axes).
const ARM_DOWN: [i16; 3] = [962, 486, 469];
/// Looking at the watch, as logged.
const LOOKING: [i16; 3] = [-739, -31, -724];
/// Flat on a desk, screen up (the dev kit).
const FLAT: [i16; 3] = [-42, 62, -996];
/// Mid-swing: far more than 1 g.
const SWING: [i16; 3] = [1620, 82, 496];

fn run(samples: &[[i16; 3]]) -> Vec<usize> {
    let mut detector = Detector::default();
    samples
        .iter()
        .enumerate()
        .filter_map(|(i, &s)| detector.push(from_sensor(s)).then_some(i))
        .collect()
}

fn main() {
    println!("run `cargo test`");
}

#[test]
fn raising_the_wrist_glances_once() {
    let samples = [
        ARM_DOWN, ARM_DOWN, SWING, LOOKING, LOOKING, LOOKING, LOOKING, LOOKING,
    ];
    // The second steady readable sample, then not again while held.
    assert_eq!(run(&samples), vec![4]);
}

#[test]
fn lowering_and_raising_again_glances_again() {
    let samples = [
        ARM_DOWN, LOOKING, LOOKING, ARM_DOWN, ARM_DOWN, LOOKING, LOOKING,
    ];
    assert_eq!(run(&samples), vec![2, 6]);
}

#[test]
fn a_watch_on_a_desk_never_glances() {
    assert_eq!(run(&[FLAT; 50]), Vec::<usize>::new());
}

#[test]
fn a_slow_raise_is_not_a_glance() {
    // Away long ago, then readable much later (e.g. resting on a desk).
    let mut samples = vec![ARM_DOWN];
    // On its side, screen facing sideways: neither away nor readable.
    samples.extend([[-102, -922, -410]; 20]);
    samples.extend([LOOKING; 5]);
    assert_eq!(run(&samples), Vec::<usize>::new());
}

/// Like `run`, with a pause (the wrist kept still, so sampling stopped)
/// before sample `pause_before`.
fn run_paused(samples: &[[i16; 3]], pause_before: usize) -> Vec<usize> {
    let mut detector = Detector::default();
    samples
        .iter()
        .enumerate()
        .filter_map(|(i, &s)| {
            if i == pause_before {
                detector.pause();
            }
            detector.push(from_sensor(s)).then_some(i)
        })
        .collect()
}

#[test]
fn raising_the_wrist_after_keeping_still_away_glances() {
    // The arm hung still long enough for sampling to stop, then rose.
    let samples = [ARM_DOWN, ARM_DOWN, ARM_DOWN, SWING, LOOKING, LOOKING];
    assert_eq!(run_paused(&samples, 3), vec![5]);
}

#[test]
fn a_pause_ends_a_recent_away() {
    // Away, then still on its side long enough for sampling to stop: as
    // with continuous sampling, turning readable after that isn't a glance.
    let side = [-102, -922, -410];
    let samples = [ARM_DOWN, side, side, LOOKING, LOOKING, LOOKING];
    assert_eq!(run(&samples), vec![4], "without the pause, it's recent");
    assert_eq!(run_paused(&samples, 3), Vec::<usize>::new());
}

#[test]
fn a_pause_while_readable_doesnt_glance_again() {
    let samples = [ARM_DOWN, LOOKING, LOOKING, LOOKING, LOOKING];
    assert_eq!(run_paused(&samples, 3), vec![2]);
}

#[test]
fn swinging_says_nothing() {
    assert_eq!(run(&[SWING; 20]), Vec::<usize>::new());
}

#[cfg(test)]
mod fuzz;

/// Proofs, for every sample the accelerometer can report at once, where
/// the tests replay logged poses and the fuzzing tries thousands.
/// `cargo kani` runs them (see CONTRIBUTING.md).
#[cfg(kani)]
mod proofs {
    use super::glance::{Detector, SETTLE, Vector, from_sensor};

    /// A sample as `activity` reads one: each axis 12 bits, signed (the
    /// sensor's 16, shifted down by 4).
    fn any_sample() -> Vector {
        let raw: [i16; 3] = kani::any();
        from_sensor(raw.map(|axis| axis >> 4))
    }

    /// Any three samples the accelerometer can report, from a detector
    /// just started: nothing overflows, and no glance comes before
    /// there's been a turn away and [`SETTLE`] samples since (so the
    /// third is the first that can be one).
    ///
    /// The arithmetic is what this is for. The detector squares each
    /// axis and sums the three in an `i32`, and turns the sensor's axes
    /// into the watch's by negating them in an `i16`: both overflow for
    /// what an `i16` can hold (three squares from 26,755 up; the
    /// negation, at -32,768), and neither can for 12 bits, ±2,048, which
    /// is 2 g: see `a_full_scale_sample_would_overflow` for the
    /// difference.
    #[kani::proof]
    #[kani::unwind(4)]
    fn any_sample_the_accelerometer_reports_is_safe() {
        let mut detector = Detector::default();
        for pushed in 1..=3u8 {
            let glance = detector.push(any_sample());
            assert!(!glance || pushed > SETTLE, "a glance with no turn away");
            kani::cover!(glance, "away, then readable twice");
        }
    }

    /// Why the bound above matters: samples of all 16 bits, as another
    /// sensor (or another shift) might give, overflow the sum of squares.
    /// A panic where overflow is checked, as here; wrapped, on the watch.
    #[kani::proof]
    #[kani::should_panic]
    fn a_full_scale_sample_would_overflow() {
        Detector::default().push(kani::any());
    }
}
