//! Alarms: when they ring. (The UI sets them and shows them ringing; this
//! module only decides.)
//!
//! Three kinds:
//!
//! - **Smart wake**: "wake me before 07:00, when my sleep allows". Separate
//!   times for weekdays and weekends, and one switch for both (off for a
//!   holiday). In the [`SMART_WINDOW`] before the time it rings at a stir
//!   in light sleep (see [Smart wake](#smart-wake)). At the time itself it
//!   rings regardless, or as soon as the watch can if it couldn't check
//!   then (see [`CATCH_UP`]).
//! - **Alarms**: two, each at a time of day, the next time it comes round.
//!   Each turns itself off after ringing.
//! - **Phone alarms**: up to [`PHONE_ALARMS`], set in Gadgetbridge and
//!   synced to the watch (see [`Phone`]). Each rings at a time of day, on
//!   the weekdays it repeats on, or once (then turning itself off).
//! - **Timers**: two, each ringing after a duration. Each remembers its last
//!   duration, as that's usually the one wanted again (the tea, the
//!   laundry).
//!
//! Times are local, so they follow the phone's time zone. Everything is
//! computed in "local seconds since 1970" (UTC plus the UTC offset), which
//! makes "the next 07:00" plain arithmetic; times of day are minutes after
//! midnight.
//!
//! Durations (timers and the snooze) are different: ten minutes is ten
//! minutes whatever the clock says, so they count in seconds of uptime,
//! which only ever go forward. The phone setting the clock (or a new time
//! zone) moves when 07:00 is, but not when the tea is ready. (Uptime
//! starts again at a reset, but so does this module's state: nothing
//! armed survives one.)
//!
//! Like [`crate::sleep`], this knows nothing of hardware or the clock, so it
//! runs in host tests (`host-tests/alarm`).
//!
//! # Smart wake
//!
//! Being woken from light sleep is easy, from deep sleep groggy; REM is in
//! between. So in its window, the smart alarm rings at a stir (the wrist
//! moving now, [`Wearer::stirring`]) only once the ten minutes before it
//! were light sleep ([`Wearer::light`], `crate::stages::light`): none of
//! them staged deep or REM, as staged so far, and not all of them still.
//! A wearer awake in bed counts as light: there's nothing to spare them.
//!
//! It used to ring at the first stir in the window, from whatever sleep.
//! But asleep, the wrist stirs about once a minute, so it rang within a
//! few minutes of the window opening, from light, REM, and deep sleep
//! alike. On four mornings in a row (2026-11-08 to 11-11) it rang 0.3 to
//! 2.6 minutes into its window: from what a Garmin worn alongside called
//! REM sleep, deep sleep (a stir out of 20 still minutes), REM, and
//! light sleep. This rule would have rung at the last of those only.
//!
//! The rule comes from replaying the watch's field logs from those four
//! nights (every poll's movement and every heart rate reading, through
//! the firmware's own classifier and stages, minute by minute as the
//! watch ran them; `host-tests/alarm` does that) against the Garmin's
//! stages, for wake times every 2 minutes from 05:00 to when the wearer
//! actually woke (440 windows):
//!
//! | rings at a stir…          | light | REM | deep | awake | at its time | minutes in |
//! |---------------------------|-------|-----|------|-------|-------------|------------|
//! | anywhere (the old rule)   | 50%   | 37% | 11%  | 1%    | 0%          | 4          |
//! | after 10 minutes of light | 64%   | 25% | 6%   | 5%    | 26%         | 16         |
//! | never (a plain alarm)     | 48%   | 36% | 14%  | 2%    | 100%        | 30         |
//!
//! Of the rings before the time, the old rule's were 11% from deep sleep
//! (48 of 438) and 37% from REM; this rule's are none or one of about 325
//! from deep sleep and 21% from REM. The deep sleep left is that of
//! alarms that come at their time, as a plain alarm's would. (For wake
//! times from 06:30 only, nearer the real ones, the old rule rang 55% in
//! light sleep and 4% in deep, this one 71% and none, 11% at its time.)
//!
//! Each half of the rule does its own part. Deep sleep is still, and the
//! live stages hardly ever say deep after the fifth hour of sleep
//! (`crate::stages::DEEP_UNTIL`), when alarms ring: the stir before rules
//! it out. Alone, though, a stir after another let 4% of rings come from
//! deep sleep, and 43% from REM. Leaving out what the live stages call
//! REM (the Garmin's REM, too, in 86% of those minutes on these nights)
//! takes away about a third of the REM wake-ups; alone, it let 5% come
//! from deep sleep. Five minutes of light sleep instead of ten, or
//! fifteen for the stir, did worse on one or the other.
//!
//! Four nights are few, of one wearer, and the Garmin's stages are only
//! its estimate; the windows overlap (a stir rings many of them), so
//! they're not 440 separate tests. The deep part rests on what sleep is
//! like as much as on the counts; the REM part on the live REM being
//! right, as it was on these nights. The cost is plain: about a quarter
//! of the time no light sleep comes in time, and the alarm rings at its
//! time, from whatever sleep, as a plain alarm would.

/// How long before a smart alarm's time it may ring early, in minutes.
pub const SMART_WINDOW: i64 = 30;
/// How long a snooze lasts, in seconds.
pub const SNOOZE: i64 = 9 * 60;
/// How long an alarm rings before giving up (and counting as dismissed),
/// in minutes.
pub const RING_FOR: i64 = 3;
/// How many timers there are.
pub const TIMERS: usize = 2;
/// How many one-off alarms there are.
pub const ALARMS: usize = 2;
/// How many alarms the phone can sync: Gadgetbridge's Bangle.js support
/// has ten slots (`BangleJSCoordinator.getAlarmSlotCount`), and a device
/// can't declare fewer.
pub const PHONE_ALARMS: usize = 10;
/// How far the local time may move, in minutes, with alarms at a time of
/// day (and the smart alarm's latest time) still ringing once each, as if
/// it hadn't: an hour, what summer time moves the clock by (Gadgetbridge
/// sends the new offset seconds after the change, while connected).
///
/// - Forward: the hour that summer time skips, or checks held up by
///   another alarm ringing for up to [`RING_FOR`] (alarms queue: the next
///   rings once the one before is done). What the clock moved over rings
///   at once: an alarm at 02:30 rings at 03:00 the night summer time
///   starts, as a phone's does, rather than not at all.
/// - Back: the hour that winter time repeats. What already rang in the
///   minutes repeated doesn't ring again: an alarm at 02:30 rings once
///   that night, not twice.
///
/// A clock that moves further (set by the phone, or a new time zone)
/// skips what it jumped over rather than ringing for it, and rings again
/// for what comes round again. So an alarm never rings more than an hour
/// late. (A watch that was off, or without the time until the phone
/// connected, starts checking afresh: what came round meanwhile doesn't
/// ring at all.)
pub const CATCH_UP: i64 = 60;

const DAY: i64 = 24 * 60;

/// The settings, as stored.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[cfg_attr(target_os = "none", derive(defmt::Format))]
pub struct Settings {
    pub smart: bool,
    /// Minutes after local midnight.
    pub smart_weekday: u16,
    pub smart_weekend: u16,
    /// Each one-off alarm's switch and time (minutes after midnight).
    pub at: [bool; ALARMS],
    pub at_time: [u16; ALARMS],
    /// Each timer's duration, in minutes.
    pub timers: [u16; TIMERS],
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            smart: false,
            smart_weekday: 7 * 60,
            smart_weekend: 9 * 60,
            at: [false; ALARMS],
            at_time: [7 * 60, 7 * 60 + 30],
            timers: [10, 3],
        }
    }
}

impl Settings {
    pub const BYTES: usize = 13;

    pub fn to_bytes(self) -> [u8; Self::BYTES] {
        let flags = u8::from(self.smart) | u8::from(self.at[0]) << 1 | u8::from(self.at[1]) << 2;
        let mut bytes = [flags; Self::BYTES];
        let words = [
            self.smart_weekday,
            self.smart_weekend,
            self.at_time[0],
            self.at_time[1],
            self.timers[0],
            self.timers[1],
        ];
        for (i, word) in words.into_iter().enumerate() {
            bytes[1 + 2 * i..3 + 2 * i].copy_from_slice(&word.to_le_bytes());
        }
        bytes
    }

    /// Out-of-range values (corrupt or from a future format) fall back to
    /// the defaults rather than ringing at nonsense times.
    pub fn from_bytes(bytes: [u8; Self::BYTES]) -> Self {
        let word = |i: usize| u16::from_le_bytes([bytes[i], bytes[i + 1]]);
        let default = Self::default();
        let time = |t: u16, fallback: u16| if t < DAY as u16 { t } else { fallback };
        let timer = |t: u16, fallback: u16| {
            if (1..=MAX_TIMER).contains(&t) {
                t
            } else {
                fallback
            }
        };
        Self {
            smart: bytes[0] & 1 != 0,
            smart_weekday: time(word(1), default.smart_weekday),
            smart_weekend: time(word(3), default.smart_weekend),
            at: [bytes[0] & 2 != 0, bytes[0] & 4 != 0],
            at_time: [
                time(word(5), default.at_time[0]),
                time(word(7), default.at_time[1]),
            ],
            timers: [
                timer(word(9), default.timers[0]),
                timer(word(11), default.timers[1]),
            ],
        }
    }
}

/// The longest timer, in minutes.
pub const MAX_TIMER: u16 = 16 * 60;

/// Which alarm is ringing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[cfg_attr(target_os = "none", derive(defmt::Format))]
pub enum Kind {
    Smart,
    /// Which of the [`ALARMS`].
    At(u8),
    /// Which of the [`TIMERS`].
    Timer(u8),
    Snooze,
    /// Which of the [`Phone`] alarms.
    Phone(u8),
}

/// What the watch sees of its wearer, which a smart alarm goes by in its
/// window (see [Smart wake](self#smart-wake)).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Wearer {
    /// The wrist is moving now, after being still (`crate::activity`'s
    /// latest movement at least `crate::sleep::STILL_LIMIT`).
    pub stirring: bool,
    /// The minutes before now were light sleep, or awake
    /// (`crate::stages::light`).
    pub light: bool,
}

/// What [`State::due`] found to ring.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Due {
    pub kind: Kind,
    /// Whether it's the smart alarm, ringing before its time, at a stir
    /// in light sleep.
    pub early: bool,
    /// Whether it turned off any of the phone's one-off alarms (to
    /// [`Switch::Rang`]), which then need saving. Not the same as `kind`
    /// being the phone's: every one-off ringing at that minute turns off,
    /// whichever of them is the one said to ring (the watch's own come
    /// first), and a repeating phone alarm changes nothing. (The watch's
    /// own one-offs, in [`Settings`], are saved by comparing the settings
    /// before and after, so they need no such flag.)
    pub phone_rang: bool,
}

/// 0 = Monday … 6 = Sunday, of local day `day` (days since 1970-01-01,
/// a Thursday).
fn weekday(day: i64) -> i64 {
    (day + 3).rem_euclid(7)
}

/// The smart alarm's next time (local minutes), strictly after minute
/// `now`.
pub fn next_smart(settings: &Settings, now: i64) -> i64 {
    // Times of day, as `Settings::from_bytes` and the editor keep them:
    // a later one would make "today's" land on another day, and so ring
    // on the wrong kind of day (a weekend's time on a Monday).
    debug_assert!(settings.smart_weekday < DAY as u16 && settings.smart_weekend < DAY as u16);
    let today = now.div_euclid(DAY);
    (today..today + 3)
        .map(|day| {
            let time = if weekday(day) < 5 {
                settings.smart_weekday
            } else {
                settings.smart_weekend
            };
            day * DAY + i64::from(time)
        })
        .find(|&at| at > now)
        .expect("one of three days has its time ahead")
}

/// The next time (local minutes) that `time` comes round, strictly after
/// minute `after`, on one of `days` (bit 0 Monday … bit 6 Sunday, as
/// Gadgetbridge sends them), or on any day if `days` is 0.
pub fn next_at(time: u16, days: u8, after: i64) -> i64 {
    // A time of day, on Gadgetbridge's seven weekdays: what
    // `Settings::from_bytes`, the editor and `Phone::push` let through.
    // A later time would ring on the day after the one `days` names.
    debug_assert!(time < DAY as u16 && days < 0x80);
    let today = after.div_euclid(DAY);
    (today..today + 8)
        .filter(|&day| days == 0 || days >> weekday(day) & 1 != 0)
        .map(|day| day * DAY + i64::from(time))
        .find(|&at| at > after)
        .expect("every weekday comes round within eight days")
}

// ==============================================================================
// Alarms from the phone
// ==============================================================================

/// One alarm from the phone: four bytes, as there may be ten of them in a
/// build with little RAM to spare.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PhoneAlarm {
    /// Minutes after local midnight.
    pub time: u16,
    /// The weekdays it rings on, bit 0 Monday … bit 6 Sunday
    /// (Gadgetbridge's `rep`); 0 rings once.
    pub days: u8,
    pub switch: Switch,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[cfg_attr(target_os = "none", derive(defmt::Format))]
#[repr(u8)]
pub enum Switch {
    Off,
    On,
    /// A one-off alarm that has rung, which turned it off. Gadgetbridge
    /// doesn't hear of that (the protocol only goes from phone to watch), so
    /// it still has the alarm on, and sends it as such along with any later
    /// change to the others; see [`Phone::sync`].
    Rang,
}

impl PhoneAlarm {
    pub fn on(&self) -> bool {
        self.switch == Switch::On
    }
}

/// The alarms set in Gadgetbridge's alarm screen, as it last sent them.
///
/// Gadgetbridge only ever sends them (all at once, whenever one changes
/// there), and never hears back, so the phone is in charge: on the watch
/// they're read-only, as anything changed here would be silently undone
/// by the next sync. They sit alongside the watch's own alarms, which are
/// for setting on the wrist.
///
/// Gadgetbridge has no smart-wake flag for a Bangle.js, so these ring at
/// their time; the smart wake stays the watch's own.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Phone {
    alarms: [PhoneAlarm; PHONE_ALARMS],
    count: u8,
}

impl Default for Phone {
    fn default() -> Self {
        Self {
            alarms: [PhoneAlarm {
                time: 0,
                days: 0,
                switch: Switch::Off,
            }; PHONE_ALARMS],
            count: 0,
        }
    }
}

impl Phone {
    pub const BYTES: usize = 1 + 4 * PHONE_ALARMS;

    /// The alarms, in Gadgetbridge's order.
    pub fn alarms(&self) -> &[PhoneAlarm] {
        &self.alarms[..usize::from(self.count)]
    }

    /// Adds an alarm as received from the phone (`hour`, `minute`, `rep`,
    /// `on`); returns `false`, ignoring it, if it's nonsense or there's no
    /// room.
    pub fn push(&mut self, hour: u16, minute: u16, days: u8, on: bool) -> bool {
        let count = usize::from(self.count);
        if hour >= 24 || minute >= 60 || count == PHONE_ALARMS {
            return false;
        }
        self.alarms[count] = PhoneAlarm {
            time: hour * 60 + minute,
            // Bit 7 means nothing; Gadgetbridge never sets it.
            days: days & 0x7F,
            switch: if on { Switch::On } else { Switch::Off },
        };
        self.count += 1;
        true
    }

    /// Takes the alarms the phone just sent in place of these.
    ///
    /// A one-off alarm that has rung here comes back from the phone still
    /// on, with every later sync (see [`Switch::Rang`]); it stays off,
    /// rather than ringing again at the same time the next day. Switching
    /// it off and on again in Gadgetbridge, or changing its time, sets it
    /// again, as it then arrives off, or as a different alarm.
    pub fn sync(&mut self, mut new: Phone) {
        for alarm in &mut new.alarms[..usize::from(new.count)] {
            let rang = PhoneAlarm {
                switch: Switch::Rang,
                ..*alarm
            };
            if alarm.on() && alarm.days == 0 && self.alarms().contains(&rang) {
                alarm.switch = Switch::Rang;
            }
        }
        *self = new;
    }

    pub fn to_bytes(self) -> [u8; Self::BYTES] {
        let mut bytes = [0; Self::BYTES];
        bytes[0] = self.count;
        let (chunks, _) = bytes[1..].as_chunks_mut::<4>();
        for (alarm, out) in self.alarms.iter().zip(chunks) {
            let [lo, hi] = alarm.time.to_le_bytes();
            *out = [lo, hi, alarm.days, alarm.switch as u8];
        }
        bytes
    }

    /// Like [`Settings::from_bytes`], drops what doesn't make sense rather
    /// than ringing at nonsense times.
    pub fn from_bytes(bytes: [u8; Self::BYTES]) -> Self {
        let mut phone = Self::default();
        let count = usize::from(bytes[0]).min(PHONE_ALARMS);
        for &[lo, hi, days, switch] in bytes[1..].as_chunks::<4>().0.iter().take(count) {
            let time = u16::from_le_bytes([lo, hi]);
            let switch = match switch {
                1 => Switch::On,
                2 => Switch::Rang,
                _ => Switch::Off,
            };
            if phone.push(time / 60, time % 60, days, switch == Switch::On) {
                phone.alarms[usize::from(phone.count) - 1].switch = switch;
            }
        }
        phone
    }
}

/// What's armed, beyond the settings, and what has already rung. Times
/// are local seconds unless they say otherwise.
#[derive(Clone, Copy, Debug, Default)]
pub struct State {
    /// When each running timer rings, in seconds of uptime (see the
    /// module docs).
    pub timers: [Option<i64>; TIMERS],
    /// When a snoozed alarm rings again, in seconds of uptime.
    pub snooze_at: Option<i64>,
    /// The smart alarm occurrence (its latest time, local minutes) that has
    /// already rung, so it rings once per morning. (Minutes fit in 32 bits
    /// for millennia, and RAM is tight.)
    smart_rang: Option<i32>,
    /// The last minute (local) checked for alarms at a time of day (the
    /// watch's, the phone's, and the smart alarm's latest time): the next
    /// check rings those whose time
    /// fell in the minutes since. Ahead of the clock for up to
    /// [`CATCH_UP`] after it moved back (winter time). Not stored: after a
    /// reset, checking starts afresh from the current minute.
    ///
    /// This is all it takes to ring each occurrence once, however many
    /// alarms there are, so nothing per alarm needs to be kept in RAM. A
    /// one-off alarm rings the first time its time comes round after it's
    /// switched on, since earlier minutes were already checked.
    checked: Option<i32>,
}

impl State {
    /// Starts `timer` at `uptime` (seconds).
    pub fn start_timer(&mut self, timer: usize, settings: &Settings, uptime: i64) {
        // What `Settings::from_bytes` and the editor keep a duration to;
        // the editor's digits (and `ui`'s drawing of what remains) have
        // no room for more (see F1 in the overnight findings).
        debug_assert!((1..=MAX_TIMER).contains(&settings.timers[timer]));
        self.timers[timer] = Some(uptime + i64::from(settings.timers[timer]) * 60);
    }

    pub fn stop_timer(&mut self, timer: usize) {
        self.timers[timer] = None;
    }

    /// Seconds until `timer` rings, if it's running, at `uptime` (seconds).
    pub fn remaining(&self, timer: usize, uptime: i64) -> Option<i64> {
        self.timers[timer].map(|at| (at - uptime).max(0))
    }

    /// Snoozes the alarm ringing at `uptime` (seconds).
    pub fn snooze(&mut self, uptime: i64) {
        self.snooze_at = Some(uptime + SNOOZE);
    }

    /// The next thing to ring, and when (local seconds), for the clock face,
    /// at local second `now`, which is `uptime` (seconds). A smart alarm
    /// counts at its latest.
    pub fn next(
        &self,
        settings: &Settings,
        phone: &Phone,
        now: i64,
        uptime: i64,
    ) -> Option<(Kind, i64)> {
        // After the minutes `due` has checked, as it will see them: what
        // it will ring next, and not what it won't (an alarm in the hour
        // winter time repeats, which rang the first time round). One held
        // up by another ringing shows its time, just past.
        let after = self.after(now.div_euclid(60));
        let smart = settings
            .smart
            .then(|| (Kind::Smart, self.pending_smart(settings, after) * 60));
        let alarms = at_a_time(settings, phone)
            .map(|(kind, time, days)| (kind, next_at(time, days, after) * 60));
        // Durations end in uptime: as local seconds, they're as far from now.
        let local = |at: i64| now + (at - uptime);
        let timers =
            (0..TIMERS).filter_map(|i| self.timers[i].map(|t| (Kind::Timer(i as u8), local(t))));
        let snooze = self.snooze_at.map(|t| (Kind::Snooze, local(t)));
        [smart, snooze]
            .into_iter()
            .flatten()
            .chain(alarms)
            .chain(timers)
            .min_by_key(|&(_, t)| t)
    }

    /// The last minute (local) checked, as the check at `minute` sees it:
    /// it looks at the minutes after this, up to `minute`. Normally the
    /// minute before; further back after a hold-up or the clock moving
    /// forward, and ahead of `minute` after it moved back (by up to
    /// [`CATCH_UP`] either way).
    fn after(&self, minute: i64) -> i64 {
        // Forward, it's the minute before plus up to `CATCH_UP` skipped:
        // summer time's jump from 02:00 to 03:00 can come between checks
        // at 01:59 and 03:00, when the watch's clock is a second or two
        // behind the phone's.
        match self.checked.map(i64::from) {
            Some(checked) if (minute - 1 - CATCH_UP..=minute + CATCH_UP).contains(&checked) => {
                checked
            }
            _ => minute - 1,
        }
    }

    /// The smart alarm occurrence (its latest time, local minutes) still
    /// to ring, the minutes up to `after` having been checked: the first
    /// after them, unless it rang early (at a stir in its window), then
    /// the next. Its latest time may have passed while checks were held
    /// up: it's then overdue, and rings at the next check.
    fn pending_smart(&self, settings: &Settings, after: i64) -> i64 {
        let next = next_smart(settings, after);
        if self.smart_rang == Some(kept(next)) {
            next_smart(settings, next)
        } else {
            next
        }
    }

    /// The latest time (local minutes) of the smart alarm's last
    /// occurrence to ring, if one has since a reset.
    pub fn smart_rang(&self) -> Option<i64> {
        self.smart_rang.map(i64::from)
    }

    /// Checks, every second or so, whether a duration (the snooze, or a
    /// timer) has run out at `uptime` (seconds), and if so, disarms it.
    /// It doesn't need the time of day, so this goes first: [`State::due`]
    /// may not be able to run.
    pub fn elapsed(&mut self, uptime: i64) -> Option<Kind> {
        if self.snooze_at.is_some_and(|t| uptime >= t) {
            self.snooze_at = None;
            return Some(Kind::Snooze);
        }
        for (i, timer) in self.timers.iter_mut().enumerate() {
            if timer.is_some_and(|t| uptime >= t) {
                *timer = None;
                return Some(Kind::Timer(i as u8));
            }
        }
        None
    }

    /// Checks, every second or so, whether an alarm at a time of day should
    /// start ringing now (`now` in local seconds). `wearer` is what a smart
    /// alarm waits for in its window: a stir in light sleep. Turns off what
    /// rings (a one-off alarm, saying so for the phone's: see [`Due`]), and
    /// remembers the smart one.
    pub fn due(
        &mut self,
        settings: &mut Settings,
        phone: &mut Phone,
        now: i64,
        wearer: Wearer,
    ) -> Option<Due> {
        let minute = now.div_euclid(60);
        // The minutes not yet checked: normally just this one, or more
        // after a hold-up or the clock moving forward (see `CATCH_UP`).
        // After it moved back by up to `CATCH_UP`, none: the minutes up to
        // the last checked were checked already, and nothing rings until
        // the clock is past them again.
        //
        // TODO: an alarm switched on in those minutes, for a time in them
        // (between 02:00 and 03:00 the night winter time starts), waits
        // for the next day.
        let after = self.after(minute);
        // The smart alarm's latest time is an alarm's time like the others,
        // so it catches up as theirs do, rather than ringing only if a
        // check falls in its very minute: another alarm ringing out over
        // it, or summer time, can leave none there.
        let smart = settings.smart.then(|| self.pending_smart(settings, after));
        // The earliest alarm in those minutes rings. Others at the very
        // same minute ring as one with it (two alarms at 07:00 are one
        // wake-up, under the smart alarm's name if it's one of them, as
        // `next` showed it), and ones later in the minutes left get their
        // turn once it's done.
        let earliest = smart
            .map(|latest| (Kind::Smart, latest))
            .into_iter()
            .chain(
                at_a_time(settings, phone)
                    .map(|(kind, time, days)| (kind, next_at(time, days, after))),
            )
            .filter(|&(_, at)| at <= minute)
            .min_by_key(|&(_, at)| at);
        if let Some((kind, at)) = earliest {
            // An occurrence not checked before, and not in the future:
            // what makes each ring once. One at or before `after` would
            // ring again at every check (once a second) until the clock
            // moved on; `next_at` and `next_smart` promise "strictly after".
            debug_assert!(after < at && at <= minute);
            self.checked = Some(kept(at));
            // Checked, it won't ring again; remembered, it won't even
            // should the clock go back by more than `CATCH_UP`.
            if smart == Some(at) {
                self.smart_rang = Some(kept(at));
            }
            // One-off alarms turn themselves off, all of those ringing.
            for i in 0..ALARMS {
                if settings.at[i] && next_at(settings.at_time[i], 0, after) == at {
                    settings.at[i] = false;
                }
            }
            let mut phone_rang = false;
            for alarm in &mut phone.alarms[..usize::from(phone.count)] {
                if alarm.on() && alarm.days == 0 && next_at(alarm.time, 0, after) == at {
                    alarm.switch = Switch::Rang;
                    phone_rang = true;
                }
            }
            return Some(Due {
                kind,
                early: false,
                phone_rang,
            });
        }
        // (Not back to `minute` while the clock repeats checked minutes.)
        self.checked = Some(kept(after.max(minute)));
        // Nothing is overdue, but the smart alarm may ring early, at a
        // stir in light sleep in its window.
        if let Some(latest) = smart
            && wearer.stirring
            && wearer.light
            && minute >= latest - SMART_WINDOW
        {
            self.smart_rang = Some(kept(latest));
            return Some(Due {
                kind: Kind::Smart,
                early: true,
                phone_rang: false,
            });
        }
        None
    }
}

/// A local minute, as [`State`] keeps it: in 32 bits.
fn kept(minute: i64) -> i32 {
    // Minutes since 1970 fit until the year 6053, and the phone's time is
    // only believed for this century (`gb`'s `SANE_UTC_SECONDS`). One
    // that didn't fit would be kept as some other minute: an alarm
    // ringing again, or not at all.
    debug_assert!(i32::try_from(minute).is_ok());
    minute as i32
}

/// The alarms at a time of day that are on, the watch's and the phone's,
/// with their time and weekdays (0 for any day).
fn at_a_time<'a>(
    settings: &'a Settings,
    phone: &'a Phone,
) -> impl Iterator<Item = (Kind, u16, u8)> + 'a {
    let own = (0..ALARMS)
        .filter(|&i| settings.at[i])
        .map(|i| (Kind::At(i as u8), settings.at_time[i], 0));
    let phone = phone
        .alarms()
        .iter()
        .enumerate()
        .filter(|(_, alarm)| alarm.on())
        .map(|(i, alarm)| (Kind::Phone(i as u8), alarm.time, alarm.days));
    own.chain(phone)
}
