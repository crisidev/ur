//! What a Gadgetbridge fetch holds back: the minutes whose sleep, or sleep
//! stage, may still change.
//!
//! Gadgetbridge keeps the first kind it gets for a sample: its Bangle.js
//! support files each `act` sample through
//! `BangleJSSampleProvider.upsertSample`, which, for a sample it already
//! has, overwrites the steps, heart rate and intensity, but never the kind
//! (see [`crate::gb_buckets`]). And the watch revises the recent past's
//! kinds after the fact: a sleep is recognised only once it has lasted a
//! while, and staged from the hour either side of each minute. So a fetch
//! in the night, or soon after waking, would leave Gadgetbridge (and
//! Health Connect after it) with the night as it looked then, for good.
//! Instead a fetch sends only the buckets before the first minute whose
//! kind may still change (see [`hold`]), and the first fetch once they're
//! settled sends them all at once: the night arrives in the morning.
//! Everything after that minute waits, steps and heart rates too, since
//! they're the same samples (and a sleeping wearer barely steps).
//!
//! **When a kind is settled**, from how `crate::activity` revises them
//! every minute:
//!
//! - The classifier rewrites the newest [`sleep::WINDOW`] (90) minutes
//!   (see [`sleep`]), so they may still change between awake, asleep and
//!   restless: falling asleep is dated from the start of the stillness
//!   that made it, a minute's movement at the window's end is restless
//!   until it's clear whether it's waking, and so on.
//! - The last sleep (a night, or a nap: sleep with no wake longer than
//!   [`sleep::SAME_NIGHT_GAP`] minutes in it) is restaged, all of it, every
//!   minute while it reaches into that window (see [`crate::stages`]):
//!   each minute's stage depends on up to an hour either side, the time
//!   since falling asleep, and the bouts around it.
//!
//! So in principle a sleep's kinds settle [`sleep::WINDOW`] minutes after
//! its last minute asleep. The classifier is less tidy than that, though:
//! it re-reads its whole window every minute, starting from the kind of
//! the minute just before it, so as the window slides, a stretch it called
//! awake can turn back into sleep up to 89 minutes on. On simulated
//! mornings spent lying in after waking, it did, 70 to 89 minutes on; and
//! sleep within [`sleep::SAME_NIGHT_GAP`] minutes of the night's end
//! joins the night, which is then restaged, all of it. So a sleep settles
//! once its last minute asleep is [`sleep::WINDOW`] plus
//! [`sleep::SAME_NIGHT_GAP`] minutes back (2¼ hours): sleep the window
//! finds after that is a sleep of its own. Waking for less than that and
//! falling asleep again keeps the night waiting, so it all comes in one
//! piece. (The quiet night, the alarm, and the wearer's double press play
//! no part: they're about notifications.)
//!
//! **Provisional**, then, is everything from the earliest of:
//!
//! - the start of any sleep whose last minute asleep is less than 2¼
//!   hours back, counting wakes of up to [`sleep::SAME_NIGHT_GAP`] minutes
//!   as part of it, as `crate::activity` does. That's the last sleep,
//!   which is restaged while it reaches into the window; and any earlier
//!   one, which the window may still rewrite, and which is restaged again
//!   should what looked like a new sleep after it turn out not to be one (a
//!   few restless minutes at the window's end, which become awake).
//! - the stillness the newest minutes are in, which may yet become sleep
//!   (see [`sleep::unsettled_onset`]): at most [`sleep::ONSET_SPAN`]
//!   minutes, as one any longer is sleep already. This is what a false
//!   start of sleep holds back: a still wearer who gets up before it
//!   becomes sleep. (Its first minutes would otherwise reach Gadgetbridge
//!   as awake, the sleep that follows starting up to 20 minutes late
//!   there.)
//!
//! By day, with no sleep in the last 2¼ hours, that's nothing, or the
//! couple of dozen minutes of a wearer sitting still, which are stepless
//! anyway. A minute a fetch sends is then as the watch keeps it for good,
//! but for the same untidiness: the classifier can still turn an awake
//! minute near its window's far end into sleep (a stillness, say, that
//! reads differently once the window no longer starts before it). That
//! didn't happen on simulated nights (see `host-tests/gb_buckets`), only
//! on days made of random stretches of anything, with the watch switched
//! off right after lying still. Ruling it out would hold every fetch back
//! by the whole window, and so the day's steps by 90 minutes.
//!
//! **A hold can't last for ever**: from [`CAP`] minutes back, a fetch sends
//! whatever the watch has, settled or not. Otherwise a sleep that never
//! ends (a watch lying still on a nightstand, before the heart rate sensor
//! notices it's off) would keep everything back until the history (24
//! hours) drops it.
//!
//! This module knows nothing about the hardware or the clock, so it can be
//! tested on a computer (see `host-tests/gb_buckets`).

use crate::sleep::{self, Kind};

/// The most a hold keeps back, in minutes: a night and then some, but
/// with six hours to spare before the history drops what's held.
pub const CAP: i64 = 18 * 60;

/// What a fetch holds back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hold {
    /// The first minute whose kind may still change (UTC minutes since
    /// 1970): a fetch sends only the buckets that end before it.
    pub from: i64,
    /// Whether that's a sleep's, rather than only a stillness that may yet
    /// become one. The first fetch after a sleep's hold ends has
    /// Gadgetbridge pass the night on to Health Connect (see `ble`).
    pub sleep: bool,
}

/// What a fetch holds back now (see the module docs), if anything: at
/// `newest`, the newest minute recorded (the one under way), given each
/// minute's kind and evidence (outside the history: unknown, and unseen).
pub fn hold(
    newest: i64,
    kind: impl Fn(i64) -> Kind,
    evidence: impl Fn(i64) -> sleep::Minute,
) -> Option<Hold> {
    // The first minute asleep that's still restaged, or may be: a sleep
    // whose last minute asleep is `newest - WINDOW` is restaged once more
    // (see `activity::History::last_sleep`), and sleep the window finds
    // up to `SAME_NIGHT_GAP` minutes after one's end joins it.
    let reach = newest - (sleep::WINDOW + sleep::SAME_NIGHT_GAP) as i64;
    let asleep = (reach..=newest)
        .find(|&minute| kind(minute).is_sleep())
        .and_then(|minute| {
            // The start of its sleep, found as `activity` finds the last
            // sleep's, over the minutes up to it; from `CAP` back, as
            // anything before that is sent anyway.
            let oldest = minute + 1 - CAP;
            sleep::last_sleep(CAP as usize, |i| kind(oldest + i as i64))
                .map(|(start, _, _)| oldest + start as i64)
        });

    let mut minutes = [sleep::Minute::default(); sleep::ONSET_SPAN];
    let first = newest - sleep::ONSET_SPAN as i64 + 1;
    for (i, minute) in minutes.iter_mut().enumerate() {
        *minute = evidence(first + i as i64);
    }
    let still = sleep::unsettled_onset(&minutes).map(|i| first + i as i64);

    let from = asleep.into_iter().chain(still).min()?;
    Some(Hold {
        from: from.max(newest - CAP + 1),
        sleep: asleep.is_some(),
    })
}
