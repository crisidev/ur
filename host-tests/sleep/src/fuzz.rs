//! Fuzzing, with Bolero: the tests above stage nights of a few made-up
//! shapes; these stage any record (any classes, movement and heart rates,
//! any sleep span within it or past its ends, any newest minute), and
//! hold the result to what `stages`' docs promise, rather than to which
//! stage a minute should be.
//!
//! `cargo test` runs each harness over a few hundred random inputs (and
//! any saved in a corpus), so they're regression tests too. To fuzz one
//! properly (nightly, for the sanitizer):
//!
//! ```text
//! cargo +nightly bolero test fuzz::any_record_stages_as_documented --engine libfuzzer -T 10min
//! ```

use super::sleep::Kind;
use super::stages::{self, DEEP_UNTIL, MIN_BOUT, REM_AFTER};

/// A per-minute record of any minutes, from minute 0, as far as `newest`.
#[derive(Clone)]
struct Night {
    kinds: Vec<Kind>,
    movement: Vec<Option<u16>>,
    heart: Vec<u8>,
    newest: i64,
}

impl stages::Record for Night {
    fn minute(&self, minute: i64) -> (Kind, Option<u16>, u8) {
        match usize::try_from(minute) {
            Ok(t) if minute <= self.newest && t < self.kinds.len() => {
                (self.kinds[t], self.movement[t], self.heart[t])
            }
            _ => (Kind::Unknown, None, 0),
        }
    }

    fn newest(&self) -> i64 {
        self.newest
    }

    fn set_stage(&mut self, minute: i64, kind: Kind) {
        let t = usize::try_from(minute).expect("staged minutes are in the record");
        assert!(
            minute <= self.newest,
            "staged minute {minute} after the newest"
        );
        assert!(
            self.kinds[t].is_stage(),
            "staged minute {minute} was {:?}",
            self.kinds[t]
        );
        self.kinds[t] = kind;
    }
}

/// Any record, staged over any span: only the span's minutes of sleep that
/// are a stage change, and only to light, deep or REM; staging again, or
/// from other stages, changes nothing (staging reads only whether a
/// minute is sleep); without a heart rate reading in the sleep, it's all
/// light; deep comes only before [`DEEP_UNTIL`] and REM only from
/// [`REM_AFTER`] after the onset; and a minute of deep or REM is in a bout
/// of at least [`MIN_BOUT`] minutes of it (restless minutes within it
/// stay restless).
#[test]
fn any_record_stages_as_documented() {
    bolero::check!()
        .with_type::<(Vec<(u8, u16, u8)>, u8, (i16, u16), u16)>()
        .for_each(|(minutes, times, (onset, span), newest)| {
            // Long nights too: the first byte says how many times over.
            let minutes = minutes.repeat(1 + usize::from(*times) % 32);
            // Mostly sleep (as the classifier leaves it, light), with
            // heart rates from the sensor's range, or none.
            let kinds: Vec<Kind> = minutes
                .iter()
                .map(|&(k, ..)| {
                    if k < 0xA0 {
                        Kind::Light
                    } else {
                        Kind::from_bits(k % 8)
                    }
                })
                .collect();
            let movement = minutes
                .iter()
                .map(|&(_, m, _)| (m & 0x8000 == 0).then_some(m % 600))
                .collect();
            let heart = minutes
                .iter()
                .map(|&(_, _, bpm)| if bpm & 1 == 0 { 0 } else { 30 + bpm % 150 })
                .collect();
            let len = kinds.len() as i64;
            let night = Night {
                kinds,
                movement,
                heart,
                newest: if newest & 1 == 0 {
                    len - 1
                } else {
                    i64::from(newest >> 1) - 10
                },
            };
            let onset = i64::from(*onset) % (len + 20);
            let end = onset + i64::from(*span) % (len + 20);

            let mut staged = night.clone();
            stages::stage(&mut staged, onset, end);
            let sleep = onset..end;
            // The minutes it may stage: the sleep's, as far as the newest.
            let staging = |t: usize| {
                sleep.contains(&(t as i64)) && t as i64 <= night.newest && night.kinds[t].is_stage()
            };
            for (t, (&before, &after)) in night.kinds.iter().zip(&staged.kinds).enumerate() {
                if staging(t) {
                    assert!(after.is_stage(), "minute {t}: {before:?} to {after:?}");
                } else {
                    assert_eq!(
                        before, after,
                        "minute {t}, outside {sleep:?} or not a stage"
                    );
                }
            }

            // Again: the same. From all light, or other stages: the same.
            let mut again = staged.clone();
            stages::stage(&mut again, onset, end);
            assert_eq!(again.kinds, staged.kinds, "staged twice");
            let mut shuffled = night.clone();
            for (t, kind) in shuffled.kinds.iter_mut().enumerate() {
                if staging(t) {
                    *kind = [Kind::Light, Kind::Deep, Kind::Rem][t % 3];
                }
            }
            stages::stage(&mut shuffled, onset, end);
            assert_eq!(shuffled.kinds, staged.kinds, "staged from other stages");

            // Without a reading, nothing to tell stages apart by.
            let readings = sleep
                .clone()
                .filter(|&t| t >= 0 && t < len && t <= night.newest)
                .any(|t| night.kinds[t as usize].is_sleep() && night.heart[t as usize] > 0);
            for (t, &kind) in staged.kinds.iter().enumerate() {
                if !staging(t) {
                    continue;
                }
                let t = t as i64;
                match kind {
                    Kind::Deep | Kind::Rem if !readings => {
                        panic!("{kind:?} at {t} without a reading")
                    }
                    Kind::Deep => assert!(t < onset + DEEP_UNTIL, "deep at {t}, from {onset}"),
                    Kind::Rem => assert!(t >= onset + REM_AFTER, "REM at {t}, from {onset}"),
                    _ => {}
                }
            }

            // Bouts: each deep or REM minute among at least `MIN_BOUT` in a
            // row of it, or restless.
            let kinds = &staged.kinds;
            for (t, &kind) in kinds.iter().enumerate() {
                if !staging(t) || !matches!(kind, Kind::Deep | Kind::Rem) {
                    continue;
                }
                let part = |k: Kind| k == kind || k == Kind::Restless;
                let first = (0..=t)
                    .rev()
                    .take_while(|&u| part(kinds[u]))
                    .last()
                    .expect("t itself");
                let last = (t..kinds.len())
                    .take_while(|&u| part(kinds[u]))
                    .last()
                    .expect("t itself");
                assert!(
                    last + 1 - first >= MIN_BOUT as usize,
                    "{kind:?} at {t}: a bout of {first}..={last}"
                );
            }
        });
}
