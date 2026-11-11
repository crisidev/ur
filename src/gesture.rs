//! Telling taps, long presses, and swipes apart, from where a touch
//! started, where it went, and how long it lasted.
//!
//! [`crate::touch`] feeds [`Tracking`] the touch controller's reports; this
//! decides what the touch was. It's apart from the driver so the
//! simulator (`sim/`) can run the same recognition on a mouse.
//!
//! A touch further than [`SWIPE_DISTANCE`] from where it started is a
//! swipe, named for the way the finger moved. A long press is reported
//! the moment the finger has been down for [`LONG_PRESS`] without moving,
//! while it's still down, so whatever it does (and its buzz) comes then,
//! and the wearer knows when to let go rather than guessing. That touch
//! is then spent: moving the finger does nothing more, and lifting it is
//! only a [`Gesture::Release`], for what lasts while the finger stays
//! down (the button grid's holds; everything else ignores it). Every
//! long press gets its release, even one only seen at the lift, or a
//! touch cut short (see [`Tracking::lifted`] and [`Tracking::lost`]).
//! While it's held, the screen stays on, up to [`HOLD_LIMIT`] (see
//! [`screen_timeout`]). A
//! finger that moves past the tap tolerance before then is on its way to
//! a swipe, and the long press is off for that touch, even if it comes
//! back. Anything else is a tap.
//!
//! A lift isn't always the end of the touch: the touch controller
//! sometimes reports a finger held still as lifted, and pressed again
//! ~10 ms later at the same spot (see [`REJOIN_WINDOW`]). So a lift only
//! ends the touch once [`REJOIN_WINDOW`] has passed without the finger
//! coming back near where it lifted ([`REJOIN_DISTANCE`]); if it does,
//! the touch goes on as if it had never lifted (see [`Tracking::lift`]).
//!
//! Hardware-free, so it runs in host tests (`host-tests/gesture`).

use embassy_time::{Duration, Instant};

use crate::canvas;

/// Minimum travel, in panel pixels, for a touch to be a swipe rather than a
/// tap. From logged touches on the dev kit: taps drift at most ~11 px, while
/// lazy swipes can be as short as ~24 px.
pub const SWIPE_DISTANCE: i32 = 20;
/// A stationary touch at least this long is a long press, reported as soon
/// as it's been held this long.
pub const LONG_PRESS: Duration = Duration::from_millis(600);
/// The longest a long press held on keeps the screen on (see
/// [`screen_timeout`]), and the touch controller's own limit for one,
/// after which it resets itself, ending the touch (see `crate::touch`).
/// Well above any hold a person means: a grid hold dimming a light takes
/// seconds, and the field logs' longest hold was ~10 s. But a sleeve,
/// or bedding, resting on the awake screen looks the same as a finger
/// held on, and this is the most screen it can keep on: a minute of the
/// backlight, and in the quiet night, a minute of its low level.
pub const HOLD_LIMIT: Duration = Duration::from_secs(60);
/// How soon after a lift a press near where it lifted continues the same
/// touch (see [`Tracking::lift`]), and so how long every lift waits
/// before it's reported.
///
/// The sealed watch's field log (0.10.0-rc.6, uptime 42880–42891) has
/// the wearer's holds, meant as a tap-and-hold's (skip to the next
/// track), chopped by the touch controller into pieces of 67–552 ms:
/// each piece starting 7–12 ms after the last one lifted, exactly where
/// it ended. So none reached [`LONG_PRESS`], and two pieces made a
/// double tap, which played or paused instead. Others started 20–21 ms
/// after (one report missed): within a 9.2 s hold on the dev kit, and
/// within a hold of 815 ms in 0.10.0-rc.1's field log. The chip's
/// reports come every ~10 ms, so these are its next report or the one
/// after. The quickest of the wearer's double taps in the field logs
/// left 41 ms between the taps (and more often 80 ms or more); a finger
/// has to leave the glass and come back. So 30 ms: past the 21 ms with
/// a little to spare for the touch task's own delays, and under the
/// 41 ms by as much.
///
/// It costs every lift those 30 ms before its tap or swipe is acted on:
/// not something to see, and a double tap's two taps are both late by
/// the same, so the time between them (see `wake::follows`) is the
/// same.
pub const REJOIN_WINDOW: Duration = Duration::from_millis(30);
/// How near, in panel pixels, where a touch lifted a press within
/// [`REJOIN_WINDOW`] must be to continue it (either way): a tap's
/// tolerance, [`SWIPE_DISTANCE`]. The chopped holds in the field logs
/// went on exactly where they lifted, or a pixel off; a press further
/// than a tap drifts, that soon, isn't the same finger come back, but
/// something else touching (a second finger, a sleeve), so a new touch.
pub const REJOIN_DISTANCE: i32 = SWIPE_DISTANCE;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[cfg_attr(target_os = "none", derive(defmt::Format))]
pub enum Gesture {
    Tap,
    LongPress,
    SwipeUp,
    SwipeDown,
    SwipeLeft,
    SwipeRight,
    /// The finger lifted after a [`Gesture::LongPress`] (see
    /// [`Tracking::release`]).
    Release,
}

#[derive(Clone, Copy, Debug)]
#[cfg_attr(target_os = "none", derive(defmt::Format))]
pub struct Touch {
    pub gesture: Gesture,
    /// Where the touch started, in canvas pixels.
    pub x: i32,
    pub y: i32,
    /// How long the finger was down, in milliseconds (saturating). Not a
    /// `Duration`: its 8 bytes, 8-aligned, grew every queued input event,
    /// and static RAM has no room to spare.
    pub held_ms: u16,
}

/// The touch in progress: where and when it started, and the latest
/// position, in panel pixels.
pub struct Tracking {
    pub start: (i32, i32),
    pub last: (i32, i32),
    pub since: Instant,
    /// It went beyond the tap tolerance ([`SWIPE_DISTANCE`]) at some point,
    /// so it can't become a long press.
    moved: bool,
    /// It was reported as a long press already, while still down.
    pub held: bool,
    /// When the finger last lifted, if it hasn't come back since: the
    /// touch is over once [`REJOIN_WINDOW`] has passed (see
    /// [`Tracking::lift`]).
    lifted_at: Option<Instant>,
    /// How many times the finger came back after a lift (saturating):
    /// for the log, to see how often the controller does it.
    pub rejoined: u8,
}

impl Tracking {
    pub fn new(at: (i32, i32)) -> Self {
        debug_assert!(on_panel(at), "a touch starting off the panel");
        Self {
            start: at,
            last: at,
            since: Instant::now(),
            moved: false,
            held: false,
            lifted_at: None,
            rejoined: 0,
        }
    }

    /// The finger is (still, or again) at `at`: whether that's this
    /// touch. Not if it lifted, and `at` is too late or too far for the
    /// finger to have come back (see [`REJOIN_WINDOW`] and
    /// [`REJOIN_DISTANCE`]): this touch is over then, as of its lift
    /// ([`Tracking::lifted`]), and `at` starts another.
    pub fn pressed(&mut self, at: (i32, i32)) -> bool {
        if let Some(lifted_at) = self.lifted_at {
            let (dx, dy) = (at.0 - self.last.0, at.1 - self.last.1);
            if lifted_at.elapsed() > REJOIN_WINDOW || dx.abs().max(dy.abs()) >= REJOIN_DISTANCE {
                return false;
            }
            // As if it had never lifted: the same start, its time down
            // counting on from the press, and its long press (if not
            // yet reported) still due then.
            self.lifted_at = None;
            self.rejoined = self.rejoined.saturating_add(1);
        }
        self.moved_to(at);
        true
    }

    /// The finger lifted. Maybe not for good: until [`Tracking::over_at`],
    /// it may come back ([`Tracking::pressed`]), and the touch goes on.
    /// Further lifts before then (the controller sends another report
    /// after a lift) change nothing.
    pub fn lift(&mut self) {
        self.lifted_at.get_or_insert_with(Instant::now);
    }

    /// When a touch that lifted is over, the finger not having come back:
    /// then it's reported, as [`Tracking::lifted`] says. `None` while the
    /// finger is down.
    pub fn over_at(&self) -> Option<Instant> {
        self.lifted_at.map(|at| at + REJOIN_WINDOW)
    }

    /// The finger is now at `at`.
    pub fn moved_to(&mut self, at: (i32, i32)) {
        debug_assert!(on_panel(at), "a touch moving off the panel");
        self.last = at;
        let (dx, dy) = (at.0 - self.start.0, at.1 - self.start.1);
        self.moved |= dx.abs().max(dy.abs()) >= SWIPE_DISTANCE;
    }

    /// When this touch becomes a long press, if it still can. Not while
    /// the finger is lifted: if it comes back, it's due again (at once,
    /// if its time has come).
    pub fn long_press_at(&self) -> Option<Instant> {
        (!self.moved && !self.held && self.lifted_at.is_none()).then_some(self.since + LONG_PRESS)
    }

    pub fn classify(&self) -> Touch {
        let dx = self.last.0 - self.start.0;
        let dy = self.last.1 - self.start.1;
        let gesture = if dx.abs().max(dy.abs()) >= SWIPE_DISTANCE {
            // Named for the direction the finger moved.
            match (dx.abs() > dy.abs(), dx < 0, dy < 0) {
                (true, true, _) => Gesture::SwipeLeft,
                (true, false, _) => Gesture::SwipeRight,
                (false, _, true) => Gesture::SwipeUp,
                (false, _, false) => Gesture::SwipeDown,
            }
        } else if !self.moved && self.down() >= LONG_PRESS {
            // Usually reported while held (see `touch::touch_task`); this
            // is for a lift that beat the timer.
            Gesture::LongPress
        } else {
            Gesture::Tap
        };
        self.touch(gesture)
    }

    /// A long press's finger lifting: where it started, and how long it
    /// was down in all.
    pub fn release(&self) -> Touch {
        self.touch(Gesture::Release)
    }

    /// The finger lifted: what to report, in order. A long press already
    /// reported while held leaves only its release. Otherwise the touch's
    /// one event, as [`Tracking::classify`] has it; and if that's a long
    /// press (a lift that beat the timer: the touch task reads the lift's
    /// report just after [`LONG_PRESS`], say), its release straight
    /// after, as for one reported while held. Whatever a long press
    /// started (a button grid hold, sent to Home Assistant) ends only with
    /// its release: without one, it would run on.
    pub fn lifted(&self) -> (Touch, Option<Touch>) {
        if self.held {
            return (self.release(), None);
        }
        let event = self.classify();
        let release = (event.gesture == Gesture::LongPress).then(|| self.release());
        // Only `release` says a touch is one: were `classify` to, this
        // touch's one event would be a release of nothing.
        debug_assert!(event.gesture != Gesture::Release);
        (event, release)
    }

    /// The touch is cut short, unfinished: its lift will never be seen
    /// (the touch task put the controller to sleep with the finger down).
    /// What it was is moot, bar a long press already reported, whose
    /// release must still go, for the same reason as in
    /// [`Tracking::lifted`]: the release comes early, rather than never.
    /// (Not for a touch whose finger lifted, and may yet come back
    /// ([`Tracking::over_at`]): that lift was seen, and it's over as of
    /// then, as [`Tracking::lifted`] says.)
    pub fn lost(&self) -> Option<Touch> {
        self.held.then(|| self.release())
    }

    /// How long the finger has been down: until now, or until it lifted,
    /// not counting the wait to see whether it comes back.
    fn down(&self) -> Duration {
        self.lifted_at.unwrap_or_else(Instant::now) - self.since
    }

    fn touch(&self, gesture: Gesture) -> Touch {
        let scale = canvas::SCALE as i32;
        Touch {
            gesture,
            x: self.start.0 / scale,
            y: self.start.1 / scale,
            held_ms: self.down().as_millis().min(u64::from(u16::MAX)) as u16,
        }
    }
}

/// Whether `at` is on the panel, in its pixels: what [`Tracking`] is only
/// ever given. The touch driver drops the controller's misreads beyond it
/// (`touch::PANEL`), and the simulator's mouse is on its window. The
/// arithmetic here counts on it: differences of two positions that far
/// apart at most, and a start that scales to a place on the canvas, for
/// the screens' hit tests (`host-tests/gesture` proves both for positions
/// on the panel, and that any `i32` would overflow).
fn on_panel(at: (i32, i32)) -> bool {
    let side = 0..(canvas::WIDTH * canvas::SCALE) as i32;
    side.contains(&at.0) && side.contains(&at.1)
}

/// How long the screen stays on after the last input: `timeout`, as the
/// screen has it, or while a long press begun on the awake screen is
/// `held` on, until [`HOLD_LIMIT`] after it (if that's longer). The hold's
/// release counts as an input, so the screen goes off `timeout` after
/// the lift, not mid-hold: by day or in the quiet night, someone holding
/// a grid button to dim a light sees the button lit until they let go,
/// and the release goes at the real lift (in the quiet night, the screen
/// going off would put the touch controller to sleep, and send the
/// release early; see [`Tracking::lost`]). The limit is for what isn't a
/// finger (see [`HOLD_LIMIT`]): past it, the screen goes off as if the
/// hold had ended.
pub fn screen_timeout(timeout: Duration, held: bool) -> Duration {
    if held {
        timeout.max(HOLD_LIMIT)
    } else {
        timeout
    }
}
