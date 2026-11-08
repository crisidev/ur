//! Turning the wrist to look at the watch: when to show the glance.
//!
//! The accelerometer's own gesture (Bosch's "wrist wear wakeup") never
//! fired: the feature-engine config this chip runs doesn't have it (see
//! `activity::SETTINGS_LEN`). So this recognises the movement itself,
//! from the acceleration (i.e. which way is down) sampled a few times a
//! second while the screen is off:
//!
//! - **Readable**: the screen faces up and towards the wearer, with 12
//!   o'clock pointing up-ish. Measured on the wrist, looking at the watch
//!   reads about (x 0, y 0.7 g, z 0.7 g) in the watch's axes (x to 3
//!   o'clock, y to 12 o'clock, z out of the screen).
//! - **Away**: anything clearly not, e.g. the arm hanging (y ≈ −1 g) or the
//!   screen facing sideways or down.
//! - A glance is going from away to readable, then staying readable for
//!   [`SETTLE`] samples. Staying readable (a watch flat on a desk, or a
//!   wrist resting on a keyboard) doesn't glance again until it's been away.
//!
//! Samples are only taken while the wrist moves: once the accelerometer
//! reports it still, sampling stops until its motion interrupt fires (see
//! `activity::motion_task`), and [`Detector::pause`] stands in for the
//! identical samples in between.
//!
//! Like `sleep`, this is free of hardware, so it runs in host tests
//! (`host-tests/glance`).

use core::sync::atomic::{AtomicBool, Ordering};

/// Whether the screen is on (then there's nothing to watch for); the UI
/// loop keeps it current.
static SCREEN_ON: AtomicBool = AtomicBool::new(true);

pub fn set_screen_on(on: bool) {
    SCREEN_ON.store(on, Ordering::Relaxed);
}

pub fn screen_on() -> bool {
    SCREEN_ON.load(Ordering::Relaxed)
}

/// Acceleration in the watch's axes, 1024 per g.
pub type Vector = [i16; 3];

/// The watch's axes from the sensor's: (−y, −x, −z), as measured on the
/// dev kit lying flat, standing upright, and on its side.
pub fn from_sensor([x, y, z]: Vector) -> Vector {
    [-y, -x, -z]
}

const G: i32 = 1024;

/// Samples a pose must hold to count as a glance (at ~5 per second).
pub const SETTLE: u8 = 2;
/// How recently the watch must have been away for a readable pose to be a
/// glance, in samples.
pub const RECENT: u8 = 8;

fn readable([x, y, z]: Vector) -> bool {
    let (x, y, z) = (i32::from(x), i32::from(y), i32::from(z));
    z >= G / 2 && (-G / 5..=G * 9 / 10).contains(&y) && x.abs() <= G / 2
}

fn away([_, y, z]: Vector) -> bool {
    let (y, z) = (i32::from(y), i32::from(z));
    z < G * 3 / 10 || y < -G * 2 / 5
}

/// Plausibly the watch at rest-ish: the acceleration is about 1 g. Readings
/// far from it are mid-swing (or a misread), and say nothing about pose.
fn steady([x, y, z]: Vector) -> bool {
    let magnitude2 = i32::from(x).pow(2) + i32::from(y).pow(2) + i32::from(z).pow(2);
    (G * 7 / 10).pow(2) <= magnitude2 && magnitude2 <= (G * 13 / 10).pow(2)
}

#[derive(Default)]
pub struct Detector {
    /// Samples since the watch was last away (saturating), if ever.
    since_away: Option<u8>,
    /// Consecutive readable samples.
    readable_for: u8,
    /// A glance was shown for the current readable stretch.
    shown: bool,
}

impl Detector {
    /// Takes the next sample (watch axes); returns whether to show the
    /// glance now.
    pub fn push(&mut self, sample: Vector) -> bool {
        if !steady(sample) {
            return false;
        }
        if away(sample) {
            self.since_away = Some(0);
            self.readable_for = 0;
            self.shown = false;
            return false;
        }
        self.since_away = self.since_away.map(|n| n.saturating_add(1));
        if !readable(sample) {
            self.readable_for = 0;
            return false;
        }
        self.readable_for = self.readable_for.saturating_add(1);
        let recent = self.since_away.is_some_and(|n| n <= RECENT + SETTLE);
        if self.readable_for >= SETTLE && recent && !self.shown {
            self.shown = true;
            return true;
        }
        false
    }

    /// Forgets the past, e.g. when the screen turns on (the wearer is
    /// looking already) or off again.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Samples stop for a while: the wrist has kept still for at least
    /// [`RECENT`] + [`SETTLE`] samples, and will until it moves (the
    /// accelerometer's motion interrupt says when). Leaves the detector as
    /// those samples, all the same pose, would have: still away, it's
    /// ready to glance as soon as the wrist turns; anywhere else, the
    /// chance has passed, as the readable stretch is long over or the away
    /// pose is no longer recent.
    pub fn pause(&mut self) {
        if self.since_away != Some(0) {
            self.since_away = None;
        }
    }
}
