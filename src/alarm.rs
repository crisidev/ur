//! Alarms: when they ring. (The UI sets them and shows them ringing; this
//! module only decides.)
//!
//! Three kinds:
//!
//! - **Smart wake**: "wake me before 07:00, when my sleep allows". Separate
//!   times for weekdays and weekends, and one switch for both (off for a
//!   holiday). In the [`SMART_WINDOW`] before the time it rings at the first
//!   sign of light sleep: a stir. Being woken mid-stir rather than from
//!   deep stillness is the whole point. At the time itself it rings
//!   regardless.
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
/// day still ringing once each, as if it hadn't: an hour, what summer
/// time moves the clock by (Gadgetbridge sends the new offset seconds
/// after the change, while connected).
///
/// - Forward: the hour that summer time skips, or checks held up by
///   another alarm ringing for up to [`RING_FOR`]. What the clock moved
///   over rings at once: an alarm at 02:30 rings at 03:00 the night summer
///   time starts, as a phone's does, rather than not at all.
/// - Back: the hour that winter time repeats. What already rang in the
///   minutes repeated doesn't ring again: an alarm at 02:30 rings once
///   that night, not twice.
///
/// A clock that moves further (set by the phone, or a new time zone)
/// skips what it jumped over rather than ringing for it, and rings again
/// for what comes round again.
const CATCH_UP: i64 = 60;

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

/// 0 = Monday … 6 = Sunday, of local day `day` (days since 1970-01-01,
/// a Thursday).
fn weekday(day: i64) -> i64 {
    (day + 3).rem_euclid(7)
}

/// The smart alarm's next time (local minutes), strictly after minute
/// `now`.
pub fn next_smart(settings: &Settings, now: i64) -> i64 {
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
    /// watch's and the phone's): the next check rings those whose time
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
        let minute = now.div_euclid(60);
        let smart = settings
            .smart
            .then(|| (Kind::Smart, self.pending_smart(settings, minute) * 60));
        let alarms = at_a_time(settings, phone)
            .map(|(kind, time, days)| (kind, next_at(time, days, minute) * 60));
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

    /// The smart alarm occurrence (local minutes) still to ring: the current
    /// one if we're in its window and it hasn't rung, else the next.
    fn pending_smart(&self, settings: &Settings, minute: i64) -> i64 {
        // An occurrence whose time has just passed (by up to a minute) is
        // still due, in case a check was late.
        let next = next_smart(settings, minute - 1);
        if self.smart_rang == Some(next as i32) {
            next_smart(settings, next)
        } else {
            next
        }
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
    /// start ringing now (`now` in local seconds). `stirring` says the
    /// wearer just moved after being still (see `crate::activity`), which
    /// is what a smart alarm waits for in its window. Turns off what rings
    /// (a one-off alarm), and remembers the smart one.
    pub fn due(
        &mut self,
        settings: &mut Settings,
        phone: &mut Phone,
        now: i64,
        stirring: bool,
    ) -> Option<Kind> {
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
        let after = match self.checked.map(i64::from) {
            Some(checked) if (minute - CATCH_UP..=minute + CATCH_UP).contains(&checked) => checked,
            _ => minute - 1,
        };
        // The earliest alarm in those minutes rings. Others at the very
        // same minute ring as one with it (two alarms at 07:00 are one
        // wake-up), and ones later in the minutes left get their turn once
        // it's done.
        let earliest = at_a_time(settings, phone)
            .map(|(kind, time, days)| (kind, next_at(time, days, after)))
            .filter(|&(_, at)| at <= minute)
            .min_by_key(|&(_, at)| at);
        if let Some((kind, at)) = earliest {
            self.checked = Some(at as i32);
            // One-off alarms turn themselves off, all of those ringing.
            for i in 0..ALARMS {
                if settings.at[i] && next_at(settings.at_time[i], 0, after) == at {
                    settings.at[i] = false;
                }
            }
            for alarm in &mut phone.alarms[..usize::from(phone.count)] {
                if alarm.on() && alarm.days == 0 && next_at(alarm.time, 0, after) == at {
                    alarm.switch = Switch::Rang;
                }
            }
            return Some(kind);
        }
        // (Not back to `minute` while the clock repeats checked minutes.)
        self.checked = Some(after.max(minute) as i32);
        if settings.smart {
            let latest = self.pending_smart(settings, minute);
            let in_window = minute >= latest - SMART_WINDOW;
            if minute >= latest || (in_window && stirring) {
                self.smart_rang = Some(latest as i32);
                return Some(Kind::Smart);
            }
        }
        None
    }
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
