//! Host tests for the firmware's `glance` module, on poses read from the
//! sealed watch's field log (sensor axes, 1024 per g). `cargo test`.

#[path = "../../../src/glance.rs"]
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
