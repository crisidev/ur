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
//! TODO: once Gadgetbridge updates a stored sample's kind
//! (<https://codeberg.org/Freeyourgadget/Gadgetbridge/issues/7296>), the
//! hold could go, and a night be sent as it goes, revisions and all: a
//! re-sent bucket would correct the kind. (Fetches would then resend the
//! minutes that may have been revised, as they did before the hold: the
//! classifier's window, the stages' hour before it and a bout, 164
//! minutes; but never back over a phone's samples from builds that sent
//! one a minute, see `gb_buckets`.) That includes sleep revised to
//! awake, as an awake bucket leaves out `act` (see
//! `sleep::Kind::gadgetbridge_name`), which Gadgetbridge reads as its
//! default, `ACTIVITY`.
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
//! joins the night, which is then restaged, all of it. So left to itself,
//! a sleep settles once its last minute asleep is [`sleep::WINDOW`] plus
//! [`sleep::SAME_NIGHT_GAP`] minutes back (2¼ hours): sleep the window
//! finds after that is a sleep of its own. Waking for less than that and
//! falling asleep again keeps the night waiting, so it all comes in one
//! piece. (The quiet night, the alarm, and the wearer's double press play
//! no part: they're about notifications.)
//!
//! **Once the wearer is clearly up, the watch settles the night itself**
//! ([`settles`]), rather than wait out the 2¼ hours: everything before
//! the minute under way becomes final (see [`Settled`]). The classifier
//! no longer rewrites it, and the staging no longer touches the sleep
//! before it; so what a fetch sends of it from then on is, by
//! construction, what the watch keeps. Clearly up is all of:
//!
//! - **Awake for longer than [`sleep::SAME_NIGHT_GAP`] minutes** since the
//!   last minute asleep. Then no sleep found later can join the night,
//!   which is what tells a night from a nap (see [`sleep::last_sleep`]):
//!   once those minutes are final, a lie-in after them is a sleep of its
//!   own, staged on its own, as it would be anyway if it came that late.
//!   And by then the stages are final too, as the minutes either side
//!   that a stage depends on are all in (see `crate::stages`).
//! - **Having walked, or taken the watch off**: at least [`UP_MINUTES`]
//!   minutes, since the last minute asleep, of [`sleep::WAKE_STEPS`] steps
//!   or more, or off the wrist (the heart rate sensor's last look found no
//!   skin). Either is what the classifier itself calls awake, whatever
//!   came before, and rules out the late rewrites above: those come from
//!   lying still in bed, which the classifier can take for sleep after
//!   all as its window slides. Two minutes, not one, so that one wild step
//!   count doesn't do. Off the wrist covers a watch taken off on waking,
//!   to charge it, say, without a walk first. In the night, it settles
//!   only a watch off for longer than a wake within a night, which splits
//!   the night in two anyway (slipped off, or the sensor wrong all that
//!   while; on the field logs' four nights, it never said "off" while
//!   asleep), and then changes no kind: those minutes are off the wrist,
//!   whatever comes after.
//! - **Not lying still now**: no stillness that may yet become sleep (see
//!   [`sleep::unsettled_onset`]), which settling would cut short, dating
//!   the sleep from the settling rather than from going still.
//! - **A sleep that began before the classifier's window**, as a night
//!   does: not a few still minutes among the window's oldest that it has
//!   only just found to be sleep, and may take back at its next pass
//!   (the untidiness below), which a fetch has likely sent as awake
//!   already. A nap of under 45 minutes or so waits the 2¼ hours.
//!
//! The minutes after the settling go as they come too, but for any a nap
//! may yet claim. They can't join the night, but the classifier, sliding
//! on, can still find sleep among the oldest of its window (the
//! untidiness below), in a nap, say, soon after getting up: while the
//! night was held, so were they, and that never mattered. Now, until the
//! night would have gone anyway, a fetch holds back from the first
//! minute of any stillness long enough for a nap ([`sleep::RESUME`]
//! minutes, the least the classifier takes), while the classifier could
//! still find one there: while any minute from it until the classifier
//! next wakes (walking, off the wrist, or six minutes on the move) is in
//! its window (see [`nap`]). A nap then reaches Gadgetbridge as the
//! watch settles it, a sleep of its own, like one later in the day. The
//! rest the classifier calls awake whatever it reads before them, so the
//! morning's steps go as they come while the wearer keeps busy.
//!
//! That lets a night go 47 minutes after its last minute asleep, rather
//! than 136. On the field logs' four mornings (2026-11-08 to 11), it
//! settled each then, and kept the very kinds the night would have
//! settled to otherwise. On a Garmin's 338 nights (Health Connect, 2025-11
//! to 2026-11, its steps standing in for ur's, which agreed with them
//! within 1% over those days), the wearer had walked two minutes by then
//! on all but 6 mornings (which wait the 2¼ hours, as before; 90% went by
//! 49 minutes), and no wake in the night was that long and walked (the
//! longest walked ones, to the bathroom, were 31 minutes, by the Garmin,
//! which counts lying awake as awake where ur counts it as sleep). On
//! simulated nights (a wearer far more restless than these), 6 to 13%
//! came out different in some minutes: mostly the stages at the end of an
//! earlier sleep, which the watch keeps where it would otherwise have put
//! them back to light (the classifier writes its whole window as light,
//! and only the last sleep is restaged), and a few minutes of sleep at a
//! night's edge that the classifier would have dropped later. In them,
//! half the morning's minutes after the settling went within a minute,
//! where they used to wait 20 or so; and none that went
//! changed after, napping soon after waking (in up to 40% of them, by
//! design) or not, taking the watch off or not, but those that the watch
//! without settling sends and changes too (1 to 13 minutes in 1000
//! nights). On the field logs, the mornings went as they came but for
//! two stretches the watch took for a nap: lying on a desk, before the
//! heart rate sensor noticed it was off. The Garmin's nights had no nap
//! within four hours of waking.
//!
//! Settling after 30 minutes up was looked at too, and left: the
//! field logs' mornings came out as well, but sleep from 30 to 45 minutes
//! after a night would no longer join it (a sleep more, split off, in up
//! to one simulated night in 12), and on the Garmin's nights, it would
//! have settled two nights at a 30-minute walk to the bathroom, going back
//! to sleep after. Settling on other signs (the screen in use, the alarm
//! dismissed) would take other modules' state for little gain: the
//! walking comes within the 46 minutes anyway.
//!
//! **Provisional**, then, is everything from the earliest of these that's
//! not settled:
//!
//! - the first minute a nap may yet claim, after a settling, while the
//!   settled sleep would still be held otherwise, as above.
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
//! off right after lying still; nor, once settling held back the naps
//! after it (it did happen without that, in a nap after getting up, and
//! without the window's age, in a few still minutes of an evening).
//! Ruling it out would hold every fetch back by the whole window, and so
//! the day's steps by 90 minutes.
//!
//! **A hold can't last for ever**: from [`CAP`] minutes back, a fetch sends
//! whatever the watch has, settled or not. Otherwise a sleep that never
//! ends (a watch lying still on a nightstand, before the heart rate sensor
//! notices it's off) would keep everything back until the history (24
//! hours) drops it.
//!
//! What's settled stays so until a reset, which loses it: the history the
//! phone has fetched is gone then too, and what it hasn't is restored
//! (see `crate::system_off`), and held back as if never settled.
//!
//! This module knows nothing about the hardware or the clock, so it can be
//! tested on a computer (see `host-tests/gb_buckets`).

use crate::sleep::{self, Kind};

/// The most a hold keeps back, in minutes: a night and then some, but
/// with six hours to spare before the history drops what's held.
pub const CAP: i64 = 18 * 60;

/// Minutes walking (of [`sleep::WAKE_STEPS`] steps or more) or off the
/// wrist since the last minute asleep that, with the rest of [`settles`],
/// make the wearer clearly up.
pub const UP_MINUTES: usize = 2;

// ==============================================================================
// Settling
// ==============================================================================

/// The minutes the watch has settled for good: those before a minute (UTC
/// minutes since 1970), or none. `crate::activity` keeps one, and every
/// pass over the history goes by it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Settled(Option<i64>);

impl Settled {
    /// Nothing settled (by the watch: what's older than the classifier's
    /// window is final anyway).
    pub const NONE: Self = Self(None);

    /// Everything before `minute`.
    pub const fn before(minute: i64) -> Self {
        Self(Some(minute))
    }

    /// The first minute that isn't settled (`i64::MIN` if none is), so
    /// that comparisons need no `Option`.
    pub fn first_unsettled(self) -> i64 {
        self.0.unwrap_or(i64::MIN)
    }

    /// The first minute the classifier rewrites at `newest` (up to
    /// `newest`): the first of its window (see [`sleep::WINDOW`]), as
    /// always, but never a settled one.
    ///
    /// It still reads the settled minutes, in its window and the context
    /// before it, as it would have: reading from the last settled one
    /// instead looked tidier, but then a nap soon after lost the night
    /// that made it quick to fall asleep again ([`sleep::RESUME`]), and
    /// turned up only later, when the context slid past, in minutes
    /// already sent as awake.
    pub fn rewritten(self, newest: i64) -> i64 {
        // At most the newest: that's where the watch settles from, and it
        // forgets what it settled should the clock go back.
        (newest - sleep::WINDOW as i64 + 1).max(self.first_unsettled().min(newest))
    }

    /// Whether the sleep from `start` until `end` (the minute after its
    /// last) is restaged at `newest`: while it reaches into the window the
    /// classifier rewrites (see `crate::activity`), unless it's settled.
    /// No sleep has some minutes settled and others not: the watch
    /// settles only after more than [`sleep::SAME_NIGHT_GAP`] minutes
    /// awake, so none that comes later joins one before.
    pub fn restaged(self, newest: i64, start: i64, end: i64) -> bool {
        newest < end + sleep::WINDOW as i64 && start >= self.first_unsettled()
    }
}

/// Whether the wearer is clearly up at `newest`, the minute under way, so
/// that the sleep before is settled now (see the module docs), given what
/// is already (`settled`), and each minute's kind and evidence (outside
/// the history: unknown, and unseen). If so, the last minute asleep, and
/// everything before `newest` is settled.
pub fn settles(
    newest: i64,
    settled: Settled,
    kind: impl Fn(i64) -> Kind,
    evidence: impl Fn(i64) -> sleep::Minute,
) -> Option<i64> {
    // A sleep that may still change: one the hold would keep back, as its
    // last minute asleep is recent enough, and that's not settled already.
    let reach =
        (newest - (sleep::WINDOW + sleep::SAME_NIGHT_GAP) as i64).max(settled.first_unsettled());
    let last = (reach..=newest)
        .rev()
        .find(|&minute| kind(minute).is_sleep())?;
    // Awake for longer than a wake within a night, counting whole minutes
    // only (not the one under way, which may yet become sleep).
    if newest - last - 1 <= sleep::SAME_NIGHT_GAP as i64 {
        return None;
    }
    let up = (last + 1..newest)
        .filter(|&minute| up(evidence(minute)))
        .count();
    if up < UP_MINUTES || still(newest, evidence).is_some() {
        return None;
    }
    // A sleep that began before the classifier's window, as a night does:
    // not one the window has only just found among its oldest minutes,
    // which the next pass may take back (see below), and whose minutes
    // were likely sent already, as awake.
    let oldest = (last + 1 - CAP).max(settled.first_unsettled());
    let (start, _, _) =
        sleep::last_sleep((last + 1 - oldest) as usize, |i| kind(oldest + i as i64))?;
    if oldest + start as i64 > newest - sleep::WINDOW as i64 {
        return None;
    }
    Some(last)
}

/// Whether a minute shows the wearer up, for [`settles`]: walking, or the
/// watch off the wrist. Either way the classifier calls it awake, whatever
/// came before (see [`sleep::classify`]).
fn up(minute: sleep::Minute) -> bool {
    minute.steps >= sleep::WAKE_STEPS || minute.worn == Some(false)
}

/// Whether the classifier wakes at `minute`, whatever came before: a
/// minute that's up (see [`up`]), or the first of [`sleep::WAKE_RUN`]
/// moving ones, all recorded by `newest`.
fn wakes(minute: i64, newest: i64, evidence: impl Fn(i64) -> sleep::Minute) -> bool {
    let moving = |minute: i64| {
        let m = evidence(minute);
        m.seen && m.worn != Some(false) && !still_at(minute, newest, &evidence)
    };
    up(evidence(minute))
        || (minute + sleep::WAKE_RUN as i64 - 1 <= newest
            && (minute..minute + sleep::WAKE_RUN as i64).all(moving))
}

/// Whether `minute` is still, as the classifier sees it at `newest` (see
/// [`sleep::still_among`]).
fn still_at(minute: i64, newest: i64, evidence: impl Fn(i64) -> sleep::Minute) -> bool {
    let after = (minute < newest).then(|| evidence(minute + 1));
    sleep::still_among(Some(evidence(minute - 1)), evidence(minute), after)
}

/// After a settling (`first_unsettled`, the first minute it left), the
/// first minute from it that a nap may yet claim, as far as the evidence
/// goes, if any: the minutes before it go as they come.
///
/// The classifier calls a minute sleep only from the start of a stillness
/// long enough for it ([`sleep::RESUME`] minutes, the least it ever
/// takes, bridging stirs as it does), until it wakes (see [`wakes`]). And
/// it was awake at the last waking before the settling, which the
/// settling needs: so it's those stretches, from such a stillness to the
/// waking after it, that may yet be sleep, while any of their minutes are
/// in its window; and any stillness the newest minutes are in, which may
/// grow into one. The rest it calls awake whatever its context, so they
/// can go as they come: the morning's steps, while the wearer keeps busy.
fn nap(newest: i64, first_unsettled: i64, evidence: impl Fn(i64) -> sleep::Minute) -> Option<i64> {
    let window = newest - sleep::WINDOW as i64 + 1;
    let reach = first_unsettled - (sleep::WINDOW + sleep::SAME_NIGHT_GAP) as i64;
    let mut minute = (reach..first_unsettled)
        .rev()
        .find(|&minute| wakes(minute, newest, &evidence))
        .unwrap_or(reach)
        + 1;
    let still = |minute: i64| still_at(minute, newest, &evidence);
    while minute <= newest {
        if !still(minute) {
            minute += 1;
            continue;
        }
        // The stillness from here, as `sleep::classify` measures it: its
        // still minutes and the stirs between them, up to its last still
        // one; unfinished if it, or a stir after it, reaches the newest.
        let start = minute;
        let mut end = minute;
        let mut unfinished = false;
        while minute <= newest {
            if still(minute) {
                minute += 1;
                end = minute;
                continue;
            }
            let gap = (minute..=newest)
                .take_while(|&m| evidence(m).seen && !still(m))
                .count() as i64;
            if gap == 0 || gap > sleep::STIR as i64 {
                break;
            }
            if minute + gap > newest {
                unfinished = true;
                break;
            }
            minute += gap;
        }
        unfinished |= end > newest;
        // Sleep starts at a still minute on the wrist, from which the
        // stillness is long enough (or may yet be), and lasts until the
        // classifier wakes: perhaps at the next minute off the wrist, in
        // the middle of the stillness, after which it may start again.
        let worn = |m: i64| still(m) && evidence(m).worn != Some(false);
        let mut onset = (start..end).find(|&m| worn(m));
        while let Some(from) = onset {
            if !unfinished && end - from < sleep::RESUME as i64 {
                break;
            }
            let woke = (from..=newest).find(|&m| wakes(m, newest, &evidence));
            if woke.is_none_or(|woke| woke >= window) {
                return Some(from.max(first_unsettled));
            }
            onset = woke.and_then(|woke| (woke + 1..end).find(|&m| worn(m)));
        }
        minute = minute.max(end);
    }
    None
}

/// The first minute of the stillness the newest minutes are in, should it
/// yet become sleep (see [`sleep::unsettled_onset`]).
fn still(newest: i64, evidence: impl Fn(i64) -> sleep::Minute) -> Option<i64> {
    let mut minutes = [sleep::Minute::default(); sleep::ONSET_SPAN];
    let first = newest - sleep::ONSET_SPAN as i64 + 1;
    for (i, minute) in minutes.iter_mut().enumerate() {
        *minute = evidence(first + i as i64);
    }
    sleep::unsettled_onset(&minutes).map(|i| first + i as i64)
}

// ==============================================================================
// Holding back
// ==============================================================================

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
/// `newest`, the newest minute recorded (the one under way), given what's
/// `settled`, and each minute's kind and evidence (outside the history:
/// unknown, and unseen).
pub fn hold(
    newest: i64,
    settled: Settled,
    kind: impl Fn(i64) -> Kind,
    evidence: impl Fn(i64) -> sleep::Minute,
) -> Option<Hold> {
    let first_unsettled = settled.first_unsettled();
    // The first minute asleep that's still restaged, or may be: a sleep
    // whose last minute asleep is `newest - WINDOW` is restaged once more
    // (see `activity::History::last_sleep`), and sleep the window finds
    // up to `SAME_NIGHT_GAP` minutes after one's end joins it. Unless it's
    // settled.
    let recent = newest - (sleep::WINDOW + sleep::SAME_NIGHT_GAP) as i64;
    let reach = recent.max(first_unsettled);
    // A settled sleep that would otherwise still be held: it goes, and so
    // do the minutes after it, but for any a nap may yet claim. Not that
    // a nap may join the night (it can't), but the classifier, sliding
    // on, can still find sleep among the oldest of its window, as by day
    // (see below); while the night was held, that never mattered. (Not a
    // sleep's hold: the night has gone, and should go on to Health
    // Connect.)
    let after_settled = (recent..reach.min(newest + 1))
        .any(|minute| kind(minute).is_sleep())
        .then(|| nap(newest, first_unsettled, &evidence))
        .flatten();
    let asleep = (reach..=newest)
        .find(|&minute| kind(minute).is_sleep())
        .and_then(|minute| {
            // The start of its sleep, found as `activity` finds the last
            // sleep's, over the minutes up to it; from `CAP` back, as
            // anything before that is sent anyway, and not before what's
            // settled (no sleep reaches back over that: see
            // `Settled::restaged`).
            let oldest = (minute + 1 - CAP).max(first_unsettled);
            let minutes = (minute + 1 - oldest) as usize;
            sleep::last_sleep(minutes, |i| kind(oldest + i as i64))
                .map(|(start, _, _)| oldest + start as i64)
        });

    // A stillness that began before the settling may continue after it,
    // and become sleep from the first unsettled minute.
    let still = still(newest, evidence).map(|minute| minute.max(first_unsettled));

    let from = asleep.into_iter().chain(still).chain(after_settled).min()?;
    // Both were found among the minutes up to the newest: a hold from a
    // later minute would hold nothing back, while saying a night is
    // still to come.
    debug_assert!(from <= newest, "held from {from}, after {newest}");
    Some(Hold {
        from: from.max(newest - CAP + 1),
        sleep: asleep.is_some(),
    })
}
