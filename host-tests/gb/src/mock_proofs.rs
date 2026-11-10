//! Kani proofs of the watch against Gadgetbridge (`gbmock`): what
//! `mock.rs` fuzzes, for every input, where the state is small enough.
//!
//! Run one at a time, memory-capped (see CONTRIBUTING.md):
//!
//! ```text
//! systemd-run --user --scope -p MemoryMax=8G -p OOMPolicy=continue --quiet \
//!     timeout 20m cargo kani --harness NAME
//! ```

use gbmock::{Fixes, NOT_MEASURED, Sample, upsert_merge, upsert_target};

use super::gb_buckets;

/// A bucket's length, in seconds.
const SECONDS: i64 = gb_buckets::MINUTES * 60;

/// Any two seconds buckets are filed at (the 299th of a five-minute
/// stretch, as `mock::every_bucket_is_filed_at_its_last_second` checks
/// for every minute to 2038; as a proof, its 64-bit divisions ran past 20
/// minutes): the same, or so far apart that Gadgetbridge's upsert never merges
/// them, as neither is in the other's window of two minutes either side
/// (`BangleJSSampleProvider.java:92-95`, with Java's wrapping `int`s).
/// And a bucket sent again goes into the sample it left the first time.
/// So a fetch's samples land one a bucket, whatever order they come in
/// and whatever is stored already.
#[kani::proof]
fn buckets_are_filed_apart() {
    let (at_a, at_b): (i32, i32) = (kani::any(), kani::any());
    kani::assume(at_a >= 0 && i64::from(at_a) % SECONDS == SECONDS - 1);
    kani::assume(at_b >= 0 && i64::from(at_b) % SECONDS == SECONDS - 1);

    let window = at_b.wrapping_sub(120)..=at_b.wrapping_add(120);
    if at_a == at_b {
        // The one sample there is the one the upsert goes into.
        assert_eq!(upsert_target(&[at_a], at_b), 0);
    } else {
        assert!(!window.contains(&at_a));
    }
    kani::cover!(at_a.checked_add(SECONDS as i32) == Some(at_b), "neighbours");
    kani::cover!(at_b == i32::MAX - 248, "the last bucket of 2038");
}

/// The kinds the watch sends (`sleep::Kind::gadgetbridge_name`), as
/// Gadgetbridge's codes: activity (no `act` at all), light, deep, REM
/// and awake sleep, and not worn.
fn any_kind_the_watch_sends() -> i32 {
    let kind: i32 = kani::any();
    kani::assume(matches!(
        kind,
        0x0000_0001 | 0x0000_0002 | 0x0000_0004 | 0x0100_0000 | 0x0200_0000 | 0x0000_0008
    ));
    kind
}

/// Any sample the watch can send (`Outgoing::Activity`'s types: five
/// minutes of `u16` steps, a `u8` heart rate, a `u16` of movement).
fn any_sample_the_watch_sends(timestamp: i32) -> Sample {
    let (steps, heart_rate, movement): (u32, u8, u16) = (kani::any(), kani::any(), kani::any());
    kani::assume(steps <= 5 * 65_535);
    Sample {
        timestamp,
        raw_kind: any_kind_the_watch_sends(),
        heart_rate: i32::from(heart_rate),
        steps: steps as i32,
        raw_intensity: i32::from(movement),
    }
}

/// A bucket sent again, over whatever Gadgetbridge has for it: what the
/// upsert (`BangleJSSampleProvider.java:116-127`) leaves is the bucket as
/// the watch now has it, **if** none of its steps, heart rate or movement
/// went from something to nothing (a zero never overwrites), and, before
/// Gadgetbridge's fix of 2026-11-10, its kind didn't change (the kind was
/// never overwritten). Those are the two things `gb_buckets` arranges: a
/// bucket only goes out whole, and only once its kind is settled.
///
/// And the other way round, each value that does go from something to
/// nothing stays as it was: Gadgetbridge can't be told.
#[kani::proof]
fn a_bucket_sent_again_replaces_what_gadgetbridge_has() {
    let timestamp: i32 = kani::any();
    let had = any_sample_the_watch_sends(timestamp);
    let now = any_sample_the_watch_sends(timestamp);
    let fixed: bool = kani::any();
    let fixes = if fixed { Fixes::ALL } else { Fixes::NONE };

    let mut stored = had;
    upsert_merge(&mut stored, &now, fixes);

    assert_eq!(stored.timestamp, timestamp);
    assert_eq!(
        stored.steps,
        if now.steps != 0 { now.steps } else { had.steps }
    );
    assert_eq!(
        stored.heart_rate,
        if now.heart_rate != 0 {
            now.heart_rate
        } else {
            had.heart_rate
        }
    );
    assert_eq!(
        stored.raw_intensity,
        if now.raw_intensity != 0 {
            now.raw_intensity
        } else {
            had.raw_intensity
        }
    );
    // The watch always sends `mov`, so never "not measured".
    assert!(stored.raw_intensity != NOT_MEASURED);
    assert_eq!(
        stored.raw_kind,
        if fixed { now.raw_kind } else { had.raw_kind }
    );

    let nothing_lost = (now.steps != 0 || had.steps == 0)
        && (now.heart_rate != 0 || had.heart_rate == 0)
        && (now.raw_intensity != 0 || had.raw_intensity == 0);
    if nothing_lost && (fixed || now.raw_kind == had.raw_kind) {
        assert_eq!(stored, now);
    }
    kani::cover!(stored != now, "a value Gadgetbridge keeps");
}

// Not here: that any time Gadgetbridge sets (`setTime(…);E.setTimeZone(…)`,
// any second from 2020 to 2100) parses to that second and zone. With the
// ten digits symbolic and the zone fixed, CBMC was still unwinding
// `gb::parse` (UTF-8 validation, `str::parse`, the search for the zone)
// after 20 minutes. `mock::the_time_arrives_as_the_phones` and
// `mock::every_time_zone_arrives_as_its_minutes` test it instead: seconds
// across the range and at its ends, and every zone to the minute.
