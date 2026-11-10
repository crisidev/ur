//! Fuzzing, with Bolero: the tests above replay a few poses from a field
//! log; these try any sequence of poses (the logged ones, and any the
//! accelerometer can report), and check what the module's docs promise
//! of it as a whole: that mid-swing readings say nothing, that a glance
//! isn't repeated while the pose holds, and that a pause stands in for
//! the still samples it replaces.
//!
//! `cargo test` runs each harness over a few hundred random inputs (and
//! any saved in a corpus), so they're regression tests too. To fuzz one
//! properly (nightly, for the sanitizer):
//!
//! ```text
//! cargo +nightly bolero test fuzz::any_poses_glance_as_documented --engine libfuzzer -T 10min
//! ```

use super::glance::{Detector, RECENT, SETTLE, Vector, from_sensor};

/// The tests' poses (sensor axes), and one on its side.
const POSES: [Vector; 5] = [
    super::ARM_DOWN,
    super::LOOKING,
    super::FLAT,
    super::SWING,
    [-102, -922, -410],
];

/// A sample in the watch's axes: one of [`POSES`], nudged, or anything
/// the accelerometer can report (12 bits, as `activity` reads it).
fn sample((pick, x, y, z): (u8, i16, i16, i16)) -> Vector {
    let raw = match POSES.get(usize::from(pick % 8)) {
        Some(&[px, py, pz]) => [px + (x >> 11), py + (y >> 11), pz + (z >> 11)],
        None => [x >> 4, y >> 4, z >> 4],
    };
    from_sensor(raw)
}

/// A sample's acceleration, squared, in g².
fn g2(v: Vector) -> f64 {
    v.map(|v| f64::from(v) / 1024.0).iter().map(|v| v * v).sum()
}

/// Whether a sample is clearly mid-swing (well outside the module's 0.7
/// to 1.3 g, which leaves its rounding out of it): it says nothing of the
/// pose.
fn swinging(v: Vector) -> bool {
    !(0.67f64.powi(2)..=1.33f64.powi(2)).contains(&g2(v))
}

/// Whether a sample is clearly about 1 g (well within it), as a watch
/// kept still reads.
fn still(v: Vector) -> bool {
    (0.73f64.powi(2)..=1.27f64.powi(2)).contains(&g2(v))
}

/// Any poses, in any order: a sample far from 1 g (mid-swing) never
/// glances, and changes nothing, as if it hadn't come; holding the pose
/// that glanced doesn't glance again; a glance comes only after
/// [`SETTLE`] samples; and a pause, after [`RECENT`] + [`SETTLE`] samples
/// of one pose, leaves the detector as any more of them would.
#[test]
fn any_poses_glance_as_documented() {
    bolero::check!()
        .with_type::<(Vec<(u8, i16, i16, i16)>, u16, Vec<(u8, i16, i16, i16)>)>()
        .for_each(|(before, held, after)| {
            let before: Vec<Vector> = before.iter().copied().map(sample).collect();
            let after: Vec<Vector> = after.iter().copied().map(sample).collect();

            // Mid-swing samples: never a glance, and as if they weren't
            // there.
            let mut all = Detector::default();
            let mut steady_only = Detector::default();
            let mut steady_seen = 0;
            let mut glances = Vec::new();
            for (i, &s) in before.iter().enumerate() {
                let glanced = all.push(s);
                if swinging(s) {
                    assert!(!glanced, "{s:?} mid-swing glanced");
                    continue;
                }
                steady_seen += 1;
                assert_eq!(steady_only.push(s), glanced, "{s:?}");
                if glanced {
                    assert!(
                        steady_seen >= usize::from(SETTLE),
                        "a glance after {steady_seen}"
                    );
                    glances.push(i);
                }
            }
            // Holding the pose that glanced doesn't glance again.
            for &i in glances.iter().take(3) {
                let mut detector = Detector::default();
                for &s in &before[..=i] {
                    detector.push(s);
                }
                for _ in 0..20 {
                    assert!(!detector.push(before[i]), "{:?} glanced again", before[i]);
                }
            }

            // A pause: the last pose held for `RECENT + SETTLE` samples,
            // then either a pause or more of it, then anything.
            let Some(&last) = before.last().filter(|&&s| still(s)) else {
                return;
            };
            let mut held_on = Detector::default();
            let mut paused = Detector::default();
            for &s in &before {
                held_on.push(s);
                paused.push(s);
            }
            for _ in 0..RECENT + SETTLE {
                held_on.push(last);
                paused.push(last);
            }
            for _ in 0..*held % 300 {
                held_on.push(last);
            }
            paused.pause();
            for &s in &after {
                assert_eq!(
                    paused.push(s),
                    held_on.push(s),
                    "{s:?} after a pause at {last:?}"
                );
            }
        });
}

/// The axes' mapping swaps x and y and turns them all round: done twice,
/// it's undone.
#[test]
fn the_axes_map_back() {
    bolero::check!()
        .with_type::<(i16, i16, i16)>()
        .for_each(|&(x, y, z)| {
            let v = [x >> 4, y >> 4, z >> 4];
            assert_eq!(from_sensor(from_sensor(v)), v);
        });
}
