//! Sleep stages: which of a sleep's minutes were light, deep, or REM sleep,
//! from the heart rate, the wrist's stirs, and the time since falling
//! asleep.
//!
//! [`crate::sleep`] decides when the wearer slept (from movement alone);
//! this relabels the minutes it calls sleep ([`Kind::Light`]) as light,
//! deep, or REM. Restless and awake minutes stay as they are. Telling stages
//! apart properly takes brain waves; a wrist has two weaker signs:
//!
//! - **The heart rate rises in REM**, and wanders, and is flat and at its
//!   lowest in deep sleep. Measured against a *local* baseline, because it
//!   drifts down through the night: a whole-night baseline makes late REM
//!   look calm and early light sleep look aroused. So each minute's
//!   **rise** is the mean of the readings within [`LEVEL`] minutes (or the
//!   nearest reading), minus the 10th percentile of those within
//!   [`LOCAL`] minutes.
//! - **The wrist stirs less in deep sleep**: a Garmin's deep sleep had a
//!   stir in ~4% of minutes, light 16%, REM 20%. Each minute's **stirs** are
//!   how many minutes within [`STIR_SPAN`] had a three-minute average
//!   movement of at least [`STIR_LIMIT`].
//!
//! The rules, for each minute of sleep:
//!
//! - **Deep**: at most [`DEEP_STIRS`] stirs, a rise of at most
//!   [`DEEP_RISE`], and less than [`DEEP_UNTIL`] minutes after falling
//!   asleep (deep sleep is front-loaded: a Garmin finds almost none after
//!   the fifth hour).
//! - **REM**: a rise of at least [`REM_RISE`], at least [`REM_AFTER`]
//!   minutes after falling asleep (the first REM comes about an hour in).
//! - **Light**: everything else, and all of it if the sleep has no heart
//!   rate readings at all.
//!
//! Then, since real stages come in bouts (a Garmin's deep and REM bouts
//! are mostly 10–40 minutes), light gaps shorter than [`FILL`] minutes
//! between two bouts of the same stage are filled, and deep or REM bouts
//! shorter than [`MIN_BOUT`] minutes become light.
//!
//! On one night (2026-11-08) this agreed with a Garmin vívoactive 5 on 67%
//! of its asleep minutes (Cohen's kappa 0.51 over light/deep/REM), where
//! deep-as-stillness agreed on 46% (kappa 0.07). The heart rate thresholds
//! come from simulations over 60 of the Garmin's nights, at the watch's
//! reading every 5 minutes asleep (`crate::heart_schedule::ASLEEP_INTERVAL`);
//! the movement ones from that one night, so they're the first to revisit
//! as more nights come in. (Both are estimates of the Garmin's estimates,
//! not of a sleep lab's.)
//!
//! [`stage`] runs over a whole sleep, every minute while it's recent, in
//! place on the per-minute record (`crate::activity`'s history), in one
//! pass and with ~500 bytes of stack: nothing is copied out. That re-does
//! the whole night each minute (some ten thousand reads of the record by
//! morning: milliseconds), which keeps it simple: a minute's stage depends
//! on up to an hour of readings either side, and on bouts that end later.
//!
//! The smart alarm (`crate::alarm`) doesn't use stages: it rings at a stir
//! in its window, and a minute's stage is only settled an hour later. A
//! stir in REM rings it as one in light sleep does, which is as it should
//! be: waking from REM is fine; it's waking from deep sleep that leaves
//! you groggy, and deep sleep rarely stirs.
//!
//! This module knows nothing about the hardware or the clock, so it can be
//! tested on a computer (see `host-tests/sleep`).

use crate::sleep::{self, Kind};

// ==============================================================================
// Thresholds
// ==============================================================================

/// Minutes either side of a minute whose heart rate readings make its
/// level.
pub const LEVEL: i64 = 3;
/// Minutes either side of a minute whose heart rate readings make its
/// baseline (their 10th percentile).
pub const LOCAL: i64 = 60;
/// Movement (averaged over three minutes) from which a minute is a stir.
/// Lower than [`sleep::STILL_LIMIT`]: this counts the small movements that
/// stay sleep, too.
pub const STIR_LIMIT: u16 = 80;
/// Minutes either side of a minute in which its stirs are counted.
pub const STIR_SPAN: i64 = 12;
/// The most stirs (out of `2 * STIR_SPAN + 1` minutes) deep sleep has.
pub const DEEP_STIRS: usize = 6;
/// The most a heart rate rises above its baseline (BPM) in deep sleep.
pub const DEEP_RISE: i32 = 6;
/// Minutes after falling asleep from which there's no more deep sleep.
pub const DEEP_UNTIL: i64 = 5 * 60;
/// The least a heart rate rises above its baseline (BPM) in REM sleep.
/// (Below [`DEEP_RISE`]: in between, a still wrist makes it deep.)
pub const REM_RISE: i32 = 5;
/// Minutes after falling asleep before there's any REM sleep.
pub const REM_AFTER: i64 = 60;
/// Deep or REM bouts shorter than this many minutes become light.
pub const MIN_BOUT: i64 = 8;
/// Light gaps shorter than this many minutes between two bouts of the same
/// stage are filled.
pub const FILL: i64 = 6;

/// How many recent minutes' stages may still change, so that the phone's
/// next sync starts this far before what it last got: the classifier's
/// [`sleep::WINDOW`], plus the baseline's reach back from it, plus a
/// bout's worth for the clean-up. (A chain of short bouts, each just
/// filled into the next, could reach further back; rare, and costing a few
/// stale minutes in Gadgetbridge at worst.)
pub const REVISED: usize = sleep::WINDOW + LOCAL as usize + (MIN_BOUT + FILL) as usize;

// ==============================================================================
// The record
// ==============================================================================

/// Per-minute evidence that stages can be read from and written into: in
/// the firmware, `crate::activity`'s history.
pub trait Record {
    /// The minute's class, its movement (`None` if the watch didn't see it,
    /// or it's outside the record), and its heart rate (0 if none).
    fn minute(&self, minute: i64) -> (Kind, Option<u16>, u8);
    /// The newest minute recorded (the one in progress).
    fn newest(&self) -> i64;
    /// Relabels a minute of sleep.
    fn set_stage(&mut self, minute: i64, kind: Kind);
}

// ==============================================================================
// Staging
// ==============================================================================

/// Stages the sleep from minute `onset` (its first) until `end` (the minute
/// after its last), in place: every minute that's [`Kind::is_stage`]
/// becomes light, deep, or REM. Minutes in between that aren't sleep (a
/// wake within the night) stay as they are and separate bouts; restless
/// ones stay restless, but are staged along with their neighbours, so as
/// not to split a bout.
///
/// One pass over the sleep: each minute's features from sliding windows,
/// and the clean-up from a few runs held back (the bout being built, and a
/// light gap that may yet be filled), written out once they're settled.
/// Writing behind the pass is safe, as staging reads only whether a minute
/// is sleep, never which stage.
#[inline(never)] // its ~500-byte frame only while it runs
pub fn stage<R: Record>(record: &mut R, onset: i64, end: i64) {
    let reading = |record: &R, minute: i64| reading(record, onset..end, minute);

    // The windows, as they stand for the first minute.
    let mut baseline = Readings::new();
    for minute in onset..onset + LOCAL + 1 {
        if let Some(bpm) = reading(record, minute) {
            baseline.insert(bpm);
        }
    }
    let mut stirs = (onset - STIR_SPAN..onset + STIR_SPAN + 1)
        .filter(|&m| stir(record, m))
        .count();
    // The nearest readings before and after a minute, for when there's
    // none within `LEVEL` of it: the last one passed, and the next one
    // ahead (found by scanning on from `scanned`, so each minute is scanned
    // once however far apart readings are).
    let mut before: Option<(i64, u8)> = None;
    let mut after: Option<(i64, u8)> = None;
    let mut scanned = onset;

    let mut bouts = Bouts::default();
    for t in onset..end {
        if t > onset {
            if let Some(bpm) = reading(record, t - 1 - LOCAL) {
                baseline.remove(bpm);
            }
            if let Some(bpm) = reading(record, t + LOCAL) {
                baseline.insert(bpm);
            }
            stirs -= usize::from(stir(record, t - 1 - STIR_SPAN));
            stirs += usize::from(stir(record, t + STIR_SPAN));
        }
        if let Some(bpm) = reading(record, t) {
            before = Some((t, bpm));
        }

        let kind = record.minute(t).0;
        let stage = if kind.is_sleep() {
            // The level: the mean of the readings within `LEVEL`, as a sum
            // and a count, or else the nearest reading (the earlier one of
            // two as near).
            let (sum, count) = (t - LEVEL..t + LEVEL + 1)
                .filter_map(|m| reading(record, m))
                .fold((0i32, 0i32), |(s, c), bpm| (s + i32::from(bpm), c + 1));
            let level = if count > 0 {
                Some((sum, count))
            } else {
                if after.is_some_and(|(m, _)| m <= t) {
                    after = None;
                }
                if after.is_none() {
                    scanned = scanned.max(t + 1);
                    while scanned < end && after.is_none() {
                        after = reading(record, scanned).map(|bpm| (scanned, bpm));
                        scanned += 1;
                    }
                }
                match (before, after) {
                    (Some((b, bpm)), Some((a, _))) if t - b <= a - t => Some((i32::from(bpm), 1)),
                    (_, Some((_, bpm))) | (Some((_, bpm)), None) => Some((i32::from(bpm), 1)),
                    (None, None) => None,
                }
            };
            match level {
                // No readings in the whole sleep: nothing to tell stages
                // apart by. TODO: deep from stillness alone, as a fallback?
                None => Kind::Light,
                Some((sum, count)) => {
                    // Rise = sum / count - baseline, compared in tenths of
                    // a BPM times `count`, to stay in integers.
                    let base = baseline.tenth_percentile_x10().unwrap_or(sum * 10 / count);
                    let rise = 10 * sum - count * base;
                    let since = t - onset;
                    if stirs <= DEEP_STIRS && rise <= 10 * DEEP_RISE * count && since < DEEP_UNTIL {
                        Kind::Deep
                    } else if rise >= 10 * REM_RISE * count && since >= REM_AFTER {
                        Kind::Rem
                    } else {
                        Kind::Light
                    }
                }
            }
        } else {
            // Not sleep: a gap between bouts.
            Kind::Unknown
        };
        bouts.push(record, t, stage);
    }
    bouts.finish(record);
}

// Not inlined, here and below: inlined into each of their several uses,
// these made staging 2.6 KB of flash.

/// A minute's heart rate reading, if it's one that counts: one of this
/// sleep's (not, say, from a nap that ended within the hour before).
#[inline(never)]
fn reading<R: Record>(record: &R, sleep: core::ops::Range<i64>, minute: i64) -> Option<u8> {
    if !sleep.contains(&minute) {
        return None;
    }
    let (kind, _, bpm) = record.minute(minute);
    (kind.is_sleep() && bpm > 0).then_some(bpm)
}

/// Whether a minute's movement, averaged with its neighbours' (those the
/// watch saw), makes it a stir. Any minute's, sleep or not: getting up
/// right after a stretch says it wasn't deep. (Not minutes still to come,
/// though their neighbour has been seen.)
#[inline(never)]
fn stir<R: Record>(record: &R, minute: i64) -> bool {
    if minute > record.newest() {
        return false;
    }
    let (sum, count) = (minute - 1..minute + 2)
        .filter_map(|m| record.minute(m).1)
        .fold((0u32, 0u32), |(s, c), m| (s + u32::from(m), c + 1));
    count > 0 && sum >= u32::from(STIR_LIMIT) * count
}

/// The heart rate readings in the baseline's window, sorted, for its
/// percentile. At most one a minute, so the window's length bounds them.
struct Readings {
    sorted: [u8; 2 * LOCAL as usize + 1],
    len: usize,
}

impl Readings {
    fn new() -> Self {
        Self {
            sorted: [0; 2 * LOCAL as usize + 1],
            len: 0,
        }
    }

    fn insert(&mut self, bpm: u8) {
        // Can't overflow: one reading a minute, and the window's minutes
        // are added once and removed once.
        let at = self.sorted[..self.len].partition_point(|&b| b < bpm);
        self.sorted.copy_within(at..self.len, at + 1);
        self.sorted[at] = bpm;
        self.len += 1;
    }

    fn remove(&mut self, bpm: u8) {
        if let Some(at) = self.sorted[..self.len].iter().position(|&b| b == bpm) {
            self.sorted.copy_within(at + 1..self.len, at);
            self.len -= 1;
        }
    }

    /// The 10th percentile, interpolated between the nearest two readings
    /// (as numpy's and Python's `statistics` default does), in tenths of a
    /// BPM: exact, as the interpolation is in tenths too.
    fn tenth_percentile_x10(&self) -> Option<i32> {
        let n = self.len.checked_sub(1)?;
        let (low, frac) = (n / 10, (n % 10) as i32);
        let at = |i: usize| i32::from(self.sorted[i.min(n)]);
        Some(10 * at(low) + (at(low + 1) - at(low)) * frac)
    }
}

/// A run of minutes with the same stage ([`Kind::Unknown`] for a gap
/// between bouts).
#[derive(Clone, Copy)]
struct Run {
    kind: Kind,
    start: i64,
    len: i64,
}

/// The clean-up, a run at a time: holds back the bout being built and a
/// light gap after it that may yet be filled, and writes out runs once
/// they're settled.
#[derive(Default)]
struct Bouts {
    /// The run of minutes being collected.
    current: Option<Run>,
    /// The last finished run, after any filling: the bout being built.
    bout: Option<Run>,
    /// A short light run after a deep or REM `bout`, filled if the next
    /// run is the same stage as the bout.
    gap: Option<Run>,
}

impl Bouts {
    fn push<R: Record>(&mut self, record: &mut R, minute: i64, kind: Kind) {
        match &mut self.current {
            Some(run) if run.kind == kind => run.len += 1,
            _ => {
                let run = Run {
                    kind,
                    start: minute,
                    len: 1,
                };
                if let Some(done) = self.current.replace(run) {
                    self.finished(record, done);
                }
            }
        }
    }

    fn finished<R: Record>(&mut self, record: &mut R, run: Run) {
        let staged = |kind| matches!(kind, Kind::Deep | Kind::Rem);
        if let Some(gap) = self.gap.take() {
            let bout = self.bout.as_mut().expect("a gap follows a bout");
            if run.kind == bout.kind {
                bout.len += gap.len + run.len;
                return;
            }
            let bout = self.bout.replace(run).expect("a gap follows a bout");
            write(record, bout);
            write(record, gap);
            return;
        }
        if run.kind == Kind::Light && run.len < FILL && self.bout.is_some_and(|b| staged(b.kind)) {
            self.gap = Some(run);
            return;
        }
        if let Some(bout) = self.bout.replace(run) {
            write(record, bout);
        }
    }

    fn finish<R: Record>(&mut self, record: &mut R) {
        if let Some(run) = self.current.take() {
            self.finished(record, run);
        }
        for run in [self.bout.take(), self.gap.take()].into_iter().flatten() {
            write(record, run);
        }
    }
}

/// Writes a settled run out: too short a deep or REM bout is light.
/// Restless minutes in it stay restless.
fn write<R: Record>(record: &mut R, run: Run) {
    let kind = match run.kind {
        Kind::Unknown => return,
        Kind::Deep | Kind::Rem if run.len < MIN_BOUT => Kind::Light,
        kind => kind,
    };
    for minute in run.start..run.start + run.len {
        if record.minute(minute).0.is_stage() {
            record.set_stage(minute, kind);
        }
    }
}

// ==============================================================================
// Summary
// ==============================================================================

/// Minutes of each stage in a sleep.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Minutes {
    pub light: u16,
    pub deep: u16,
    pub rem: u16,
}

impl Minutes {
    /// Counts the stages of `kinds`.
    pub fn count(kinds: impl IntoIterator<Item = Kind>) -> Self {
        let mut minutes = Self::default();
        for kind in kinds {
            match kind {
                Kind::Light => minutes.light += 1,
                Kind::Deep => minutes.deep += 1,
                Kind::Rem => minutes.rem += 1,
                _ => {}
            }
        }
        minutes
    }

    /// Minutes actually asleep: all stages.
    pub fn asleep(&self) -> u16 {
        self.light + self.deep + self.rem
    }
}
