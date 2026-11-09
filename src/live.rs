//! Gadgetbridge's live activity: the heart rate and steps as they happen,
//! for its "Live activity" chart.
//!
//! Gadgetbridge asks a Bangle.js for them with a line (see [`crate::gb`])
//!
//! ```text
//! GB({"t":"act","hrm":true,"stp":true,"int":10})
//! ```
//!
//! when that chart opens (Charts, then its "Live activity" tab), and with
//! both `false` when it closes, or the phone's screen goes off (the
//! chart's `onPause`). `hrm` asks for the heart rate, `stp` for steps, and
//! `int` is the seconds between reports: 10, unless something called its
//! `onSetHeartRateMeasurementInterval`. A Bangle.js (its `android` app)
//! then turns its heart rate sensor on if asked, and every `int` seconds
//! sends
//!
//! ```text
//! {"t":"act","stp":3,"hrm":72,"rt":1}
//! ```
//!
//! the steps since the last report, and the latest heart rate (0 for none,
//! which Gadgetbridge ignores). `rt` marks it live: Gadgetbridge charts it,
//! but doesn't store it (it only stores the history, from `actfetch`). It
//! stops when asked, and when the phone disconnects. We do the same, and
//! the heart rate task measures continuously meanwhile, as it does while
//! someone looks at the clock face (see [`crate::heart`]).
//!
//! Gadgetbridge has nothing else for a Bangle.js's heart rate: its one-shot
//! "measure heart rate" (`onHeartRateTest`) is off for one
//! (`supportsManualHeartRateMeasurement` is false: "the current code for
//! onHeartRateTest looks completely broken").
//!
//! One thing we add: a [`LIMIT`]. The LED costs ~2.7 mA measuring (up to
//! ~27 mA with nothing against the sensor, as on a desk, which reflects
//! enough light that it never gives up), and a stop that never comes (the
//! phone keeping the connection with Gadgetbridge stuck, say) would
//! otherwise spend it until the battery is flat. Gadgetbridge can't keep a
//! session alive past the limit: its chart repeats its request every
//! second "to keep it measuring", but its Bangle.js driver passes on only
//! changes. Opening the chart again starts a new session.
//!
//! The UI loop ([`crate::ui_loop`]) runs this, once a second, as on the
//! watch so in the simulator: [`take`] what Gadgetbridge asked for (which
//! `ble` hands over with [`ask`]), and [`Live::poll`] for the next report.
//! Time is whole seconds since boot; the module knows nothing of hardware
//! or the clock, so it runs in host tests (`host-tests/live`).

use core::sync::atomic::{AtomicU32, Ordering};

// ==============================================================================
// Constants
// ==============================================================================

/// The longest a session runs without being asked again, in seconds. Long
/// enough to watch the heart rate through a few minutes of effort and the
/// recovery after; short enough that a forgotten session costs at most
/// ~0.5 mAh on a wrist (~4.5 mAh at the LED's top drive), against ~1.5
/// mAh a day for the background measurements (see
/// [`crate::heart_schedule::LED_BUDGET`]).
///
/// TODO: the wearer may want longer (a workout watched on the phone);
/// see whether anyone does.
pub const LIMIT: u32 = 10 * 60;

/// The interval when Gadgetbridge gives none (or none we can use):
/// Gadgetbridge's own default.
pub const DEFAULT_INTERVAL: u16 = 10;

/// The longest interval we follow: any longer, and a session ends before
/// its first report.
pub const MAX_INTERVAL: u16 = LIMIT as u16;

/// The oldest a reading may be and still be reported (if the interval is
/// shorter), in seconds. A clean signal gives one every ~2 s; reporting an
/// older one would show Gadgetbridge a number the wrist has moved on from,
/// so after this it reports none until the signal is back.
pub const FRESH: u32 = 30;

// ==============================================================================
// What Gadgetbridge asks for
// ==============================================================================

/// One request from Gadgetbridge: what to report, and how often. Neither
/// (`heart_rate` and `steps` both false) stops a session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Request {
    pub heart_rate: bool,
    pub steps: bool,
    /// Seconds between reports, 1..=[`MAX_INTERVAL`].
    pub interval: u16,
}

impl Request {
    /// From a request's fields as Gadgetbridge sends them: `int` in
    /// seconds, if it's there. One under a second (a Bangle.js would report
    /// as fast as it could), or missing, gets [`DEFAULT_INTERVAL`].
    pub fn new(heart_rate: bool, steps: bool, interval: Option<i64>) -> Self {
        let interval = match interval {
            Some(seconds) if seconds >= 1 => seconds.min(i64::from(MAX_INTERVAL)) as u16,
            _ => DEFAULT_INTERVAL,
        };
        Self {
            heart_rate,
            steps,
            interval,
        }
    }

    /// Whether this asks for anything at all.
    pub fn wanted(&self) -> bool {
        self.heart_rate || self.steps
    }
}

/// The latest request not yet taken, packed: bit 31 says there is one,
/// bit 0 is `heart_rate`, bit 1 `steps`, and bits 8..24 the interval. An
/// atomic rather than a mutex, so this module needs nothing but `core`.
static ASKED: AtomicU32 = AtomicU32::new(0);
const PENDING: u32 = 1 << 31;

/// Hands Gadgetbridge's request to the UI loop, replacing any it hasn't
/// taken yet: only the latest counts.
pub fn ask(request: Request) {
    let packed = PENDING
        | u32::from(request.heart_rate)
        | u32::from(request.steps) << 1
        | u32::from(request.interval) << 8;
    ASKED.store(packed, Ordering::Relaxed);
}

/// The request [`ask`] was last given, if not yet taken.
pub fn take() -> Option<Request> {
    let packed = ASKED.swap(0, Ordering::Relaxed);
    (packed & PENDING != 0).then_some(Request {
        heart_rate: packed & 1 != 0,
        steps: packed & 2 != 0,
        interval: (packed >> 8) as u16,
    })
}

// ==============================================================================
// The session
// ==============================================================================

/// Why a session ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ended {
    /// Gadgetbridge said stop.
    Asked,
    /// The phone went away.
    Disconnected,
    /// It ran for [`LIMIT`].
    TimedOut,
}

impl Ended {
    /// For the log.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Asked => "asked",
            Self::Disconnected => "disconnected",
            Self::TimedOut => "time limit",
        }
    }
}

/// One report for Gadgetbridge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Report {
    /// Steps since the last report (or the session's start).
    pub steps: u16,
    /// The latest heart rate, in BPM, if it's from this session and
    /// fresh (see [`FRESH`]); 0 for none.
    pub heart_rate: u8,
}

/// What [`Live::poll`] found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Poll {
    /// Nothing to do (no session, or no report due yet).
    Idle,
    Report(Report),
    Ended(Ended),
}

/// What [`Request::steps`] asked for isn't kept: like a Bangle.js, we
/// report steps either way (they cost nothing to count), and a fresh
/// heart rate whenever there is one (only the sensor waits to be asked).
#[derive(Clone, Copy)]
struct Session {
    heart_rate: bool,
    interval: u16,
    /// When it (last) started, for [`LIMIT`], and to tell this session's
    /// readings from older ones.
    started: u32,
    /// When the next report is due.
    next: u32,
    /// Today's steps at the last report, to report the difference.
    steps: u32,
}

/// The live session, if one is running. The UI loop keeps it.
pub struct Live {
    session: Option<Session>,
}

impl Live {
    pub const fn new() -> Self {
        Self { session: None }
    }

    /// Acts on Gadgetbridge's `request`, at `now`, with today's step count
    /// at `steps`: starts a session, or restarts one (with the new
    /// interval, and the full [`LIMIT`] again), or stops one, saying so.
    pub fn ask(&mut self, request: Request, now: u32, steps: u32) -> Option<Ended> {
        if !request.wanted() {
            return self.session.take().map(|_| Ended::Asked);
        }
        // Steps since the last report carry over a change of interval.
        let steps = self.session.map_or(steps, |s| s.steps);
        self.session = Some(Session {
            heart_rate: request.heart_rate,
            interval: request.interval,
            started: now,
            next: now + u32::from(request.interval),
            steps,
        });
        None
    }

    /// Whether a session wants the heart rate sensor on.
    pub fn heart_rate(&self) -> bool {
        self.session.is_some_and(|s| s.heart_rate)
    }

    /// What's due at `now`, given whether the phone is `connected`,
    /// today's step count (`steps`), and the latest heart rate reading
    /// and when it was taken (`reading`), if any. Call it once a second
    /// or so: a report late by a call is sent then, and the next comes an
    /// interval after it.
    pub fn poll(
        &mut self,
        now: u32,
        connected: bool,
        steps: u32,
        reading: Option<(u8, u32)>,
    ) -> Poll {
        let Some(session) = &mut self.session else {
            return Poll::Idle;
        };
        let ended = if !connected {
            Some(Ended::Disconnected)
        } else if now.saturating_sub(session.started) >= LIMIT {
            Some(Ended::TimedOut)
        } else {
            None
        };
        if let Some(ended) = ended {
            self.session = None;
            return Poll::Ended(ended);
        }
        if now < session.next {
            return Poll::Idle;
        }
        session.next = now + u32::from(session.interval);
        // Today's count starts over at midnight: then what's counted since
        // is the best guess at the steps since the last report.
        let walked = if steps >= session.steps {
            steps - session.steps
        } else {
            steps
        };
        session.steps = steps;
        let fresh = FRESH.max(u32::from(session.interval));
        let heart_rate = reading
            .filter(|&(_, at)| at >= session.started && now.saturating_sub(at) <= fresh)
            .map_or(0, |(bpm, _)| bpm);
        Poll::Report(Report {
            steps: walked.min(u32::from(u16::MAX)) as u16,
            heart_rate,
        })
    }
}
