//! The activity history as Gadgetbridge gets it: in 3-minute buckets.
//!
//! The watch keeps a sample a minute (see [`crate::activity`]), but
//! Gadgetbridge's Bangle.js support can't store samples that close
//! together. It files each `act` sample of a fetch through
//! `BangleJSSampleProvider.upsertSample`, which looks for samples it already
//! has within ±2 minutes (inclusive) of the new one and, if it finds any,
//! overwrites the nearest one's steps, heart rate and intensity (each only
//! if non-zero) instead of adding the new one. So of a run of samples a
//! minute apart, one in three survived, carrying the steps of the last
//! minute merged into it: a field day's 10,346 steps reached Health Connect
//! as 4,206. (The Bangle.js it was written for sends 10-minute records.)
//!
//! So a fetch sends one sample per UTC-aligned bucket of [`MINUTES`]
//! minutes: 3 is the shortest that leaves no other bucket within ±2
//! minutes of any one, so each lands on its own, and sums lose nothing.
//! Longer buckets would lose more for nothing: on a field day's log, 3
//! kept all 123 background heart rate readings and 5 lost 5 of them (the
//! schedule measures every 2–4 minutes while active, see
//! [`crate::heart_schedule`]); and on a night's minute-level sleep stages,
//! 3 changed 2–6% of minutes and 5 changed 6–7% (stage totals within a
//! couple of minutes either way). It's 480 samples a day, a third of the
//! minutes.
//!
//! Per bucket:
//!
//! - **Steps** are summed, so totals are exact.
//! - **Heart rate**: the bucket's latest reading. A real reading, never an
//!   average of several; the latest is the one nearest the bucket's
//!   timestamp (its end, below). Earlier readings in the same bucket are
//!   dropped: readings less than 3 minutes apart, which the background
//!   schedule rarely takes (none of a field day's 123), but measuring
//!   continuously does (while the clock face is looked at, or
//!   Gadgetbridge's live activity is open). Both Gadgetbridge's charts and
//!   Health Connect then get a reading every 3 minutes, which is as often
//!   as the schedule measures anyway.
//! - **Kind** (activity, or a sleep stage; see
//!   [`crate::sleep::Kind::gadgetbridge_name`]): the one most of its
//!   minutes have, ties going to the later minute's. Sleep sessions and
//!   stages in Gadgetbridge (and so in Health Connect) are then at 3-minute
//!   resolution, which is finer than the stages themselves (deep and REM
//!   bouts last at least 8 minutes, see [`crate::stages::MIN_BOUT`]); a
//!   lone restless minute among asleep ones is lost.
//! - **Movement** (`mov`, Gadgetbridge's intensity, which only its charts
//!   use): the mean over the minutes the accelerometer recorded, rounded up
//!   so that any movement stays non-zero.
//!
//! **The timestamp** is the bucket's last second (`hh:m2:59` for a bucket
//! from `hh:m0`). Gadgetbridge takes a sample as the end of the time it
//! describes: its time-in-each-activity tallies count from the previous
//! sample to it (`ActivityAnalysis`), its sleep stages run from the
//! previous sample to it (`SleepSessionSegmenter`), and its Health Connect
//! export writes a step count stamped `t` as the minute ending at `t`
//! rounded up to a whole minute (`AbstractActivitySampleSyncer`). But its
//! own daily totals take a day as midnight to 23:59:59 (`DailyTotals`), so
//! a sample stamped at midnight would count to the next day. The last
//! second is in the bucket for both: its steps land in its last minute in
//! Health Connect, and on its own day in either. (The minute samples this
//! replaced were stamped at their start, so their steps landed a minute
//! early in Health Connect.)
//!
//! **Re-fetching.** A bucket is only ever sent whole, so that sending it
//! again (every fetch resends the recent past; see
//! [`crate::stages::REVISED`]) overwrites Gadgetbridge's copy with the same
//! or a revised value, never a smaller part of it:
//!
//! - A fetch starts at a bucket's start, not mid-bucket ([`Fetch::new`]).
//! - The bucket still under way isn't sent: it would go out partial, and
//!   while the next fetch's whole count would overwrite its steps,
//!   Gadgetbridge never overwrites a sample's kind. It goes out with the
//!   first fetch after it ends. So Gadgetbridge lags the watch by up to 3
//!   minutes.
//! - Nor is a bucket whose first minutes the history has already dropped
//!   (it keeps 24 hours): Gadgetbridge may have it whole.
//! - What the watch saves before switching off starts at a bucket's start
//!   too (see `system_off`), so a restored history holds whole buckets.
//!
//! TODO: a clock set back by the phone drops the history's minutes after
//! the new time (see [`crate::minute_ring`]); a bucket sent before that and
//! again after it would overwrite Gadgetbridge's steps with fewer. The
//! phone only corrects the clock by seconds, so this is a bucket at most.

/// Minutes per bucket. Must be over 2 (see the module docs), and divide a
/// day, so buckets line up with midnight in every time zone (whose UTC
/// offsets are all whole quarter hours).
pub const MINUTES: i64 = 3;

const _: () = assert!(MINUTES > 2 && 15 % MINUTES == 0);

/// The start of the bucket `minute` (UTC minutes since 1970) is in.
pub fn start_of(minute: i64) -> i64 {
    minute.div_euclid(MINUTES) * MINUTES
}

/// One minute of the history, as a bucket takes it in.
#[derive(Clone, Copy, Debug)]
pub struct Minute<K> {
    pub steps: u16,
    pub movement: u16,
    /// 0 if not measured.
    pub heart_rate: u8,
    /// Whether the accelerometer recorded the minute (else `movement`
    /// means nothing).
    pub seen: bool,
    /// What Gadgetbridge is told the minute was (see the module docs).
    pub kind: K,
}

/// One bucket: what one `act` sample carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bucket<K> {
    /// Its first minute (UTC minutes since 1970).
    pub start: i64,
    pub steps: u32,
    pub movement: u16,
    pub heart_rate: u8,
    pub kind: K,
}

impl<K> Bucket<K> {
    /// The timestamp to send, in UTC milliseconds: its last second (see
    /// the module docs).
    pub fn timestamp_ms(&self) -> i64 {
        (self.start + MINUTES) * 60_000 - 1_000
    }
}

/// The buckets one fetch sends, in order.
pub struct Fetch {
    /// Where the next bucket starts, at the earliest.
    next: i64,
    /// The start of the bucket under way: none from here on.
    end: i64,
}

impl Fetch {
    /// A fetch of the minutes from `since` on (as the phone asked, less
    /// what's been revised since), at `now`, of a history whose oldest
    /// minute is `oldest` (all UTC minutes since 1970): from the start of
    /// `since`'s bucket, or of the first bucket the history has whole if
    /// that's later, to the end of the last bucket that's over.
    pub fn new(since: i64, oldest: i64, now: i64) -> Self {
        Self {
            next: start_of(since).max(start_of(oldest + MINUTES - 1)),
            end: start_of(now),
        }
    }

    /// Where to read the history from for [`Fetch::next`].
    pub fn from(&self) -> i64 {
        self.next
    }

    /// Up to where the phone has everything once all the buckets are sent:
    /// the start of the bucket under way.
    pub fn sent_until(&self) -> i64 {
        self.end
    }

    /// The next bucket, from `minutes`: the history from [`Fetch::from`]
    /// on, oldest first, skipping minutes without data (as
    /// `activity::History::since` gives them). Reads only as far as the
    /// bucket's last minute (and one past it). `None` once the buckets
    /// that are over have all been sent.
    pub fn next<K: Copy + PartialEq>(
        &mut self,
        minutes: impl IntoIterator<Item = (i64, Minute<K>)>,
    ) -> Option<Bucket<K>> {
        let from = self.next;
        let mut minutes = minutes
            .into_iter()
            .skip_while(|&(minute, _)| minute < from)
            .peekable();
        let start = start_of(minutes.peek()?.0);
        if start >= self.end {
            self.next = self.end;
            return None;
        }
        self.next = start + MINUTES;

        let mut bucket = Bucket {
            start,
            steps: 0,
            movement: 0,
            heart_rate: 0,
            kind: minutes.peek()?.1.kind,
        };
        // Each minute's kind by its place in the bucket, for the vote.
        let mut kinds = [None; MINUTES as usize];
        let (mut movement, mut seen) = (0u32, 0u32);
        while let Some((minute, sample)) = minutes.next_if(|&(minute, _)| minute < start + MINUTES)
        {
            bucket.steps += u32::from(sample.steps);
            if sample.heart_rate != 0 {
                bucket.heart_rate = sample.heart_rate;
            }
            if sample.seen {
                movement += u32::from(sample.movement);
                seen += 1;
            }
            kinds[(minute - start) as usize] = Some(sample.kind);
        }
        if seen > 0 {
            bucket.movement = movement.div_ceil(seen) as u16;
        }
        // The kind most minutes have; from the latest minute back, so that
        // ties go to the later one.
        let votes = |kind| kinds.iter().filter(|&&k| k == Some(kind)).count();
        let mut most = 0;
        for kind in kinds.iter().rev().flatten() {
            if votes(*kind) > most {
                most = votes(*kind);
                bucket.kind = *kind;
            }
        }
        Some(bucket)
    }
}
