//! Which minutes the activity history keeps as the clock moves: the
//! arithmetic of [`crate::activity`]'s ring of per-minute samples, apart
//! from the samples, so it runs in host tests (`host-tests/minute_ring`).
//!
//! The ring has a slot per minute of the last day, each minute (UTC
//! minutes since 1970) in slot `minute % len`, and remembers its newest
//! minute. A slot holds a minute between the newest minus `len` and the
//! newest; filing a sample for another minute first empties the slots
//! that no longer hold one of those.
//!
//! The clock normally only moves on, a minute at a time, or by a gap the
//! watch didn't see. But the phone can set it back: a few seconds (the
//! watch's crystal ran fast), or hours or days, a phone whose clock was
//! wrong being put right. Then the minutes after the new time are in the
//! future: their samples were filed under a clock that was ahead, and
//! kept, they'd go on being fetched (and shown, and slept through) as
//! times yet to come, while the minutes really recorded from then on
//! landed on top of them. Further back than the ring reaches, they landed
//! in other minutes' slots, filed under the wrong day. So going back
//! drops what's after the new time, and further back than a day, all of
//! it.

use core::ops::Range;

/// Filing a sample for `minute` in a ring of `len` minutes whose newest
/// is `newest` (if it has any): the minutes whose slots to empty first,
/// and the ring's newest minute after.
pub fn file(newest: Option<i64>, minute: i64, len: i64) -> (Range<i64>, i64) {
    let Some(newest) = newest else {
        return (minute..minute + 1, minute);
    };
    // The oldest minute the ring holds as of the newest.
    let oldest = newest - len + 1;
    if minute > newest {
        // On: the minutes skipped since the newest (all of them at most).
        ((newest + 1).max(minute - len + 1)..minute + 1, minute)
    } else if minute < newest {
        // Back: the minutes after this one, and this one too if it's
        // further back than the ring reaches (its slot is a newer
        // minute's, which goes with the rest).
        ((minute + 1).max(oldest)..newest + 1, minute)
    } else {
        (minute..minute, newest)
    }
}
