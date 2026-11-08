//! The button grid: unlabelled buttons below the clock face, for Home
//! Assistant or the phone to act on, in one of two [`Layout`]s.
//!
//! **Six buttons**, one layer ([`Layout::Six`], the default): two rows of
//! three, numbered in reading order, each its own [`Press`], code 1–6.
//!
//! ```text
//!   1 2 3
//!   4 5 6
//! ```
//!
//! Each can be tapped, double-tapped, or held: 6 × 3 = 18 events. A
//! double tap is unambiguous here, as nothing else starts with a tap.
//!
//! **Two layers** ([`Layout::TwoLayers`]): nine buttons, a phone's
//! keypad, numbered in reading order:
//!
//! ```text
//!   1 2 3
//!   4 5 6
//!   7 8 9
//! ```
//!
//! A **single** button is one of the nine on its own; a **sequence** is
//! one tapped, then another (or the same one again). Each is a [`Press`],
//! with a code that reads as what was pressed: 1–9 for a single, and the
//! two digits for a sequence, 23 for 2 then 3: the first button the tens,
//! the second the ones. So codes run 1–99, and none ends in 0 (nor has a
//! 0 at all): 9 + 81 of them. On the last button:
//!
//! - **A single** can be tapped, or held. Not double-tapped: tapping 1
//!   twice is the sequence 1 then 1 (code 11).
//! - **A sequence** can be tapped (2, then 3), double-tapped (2, then 3
//!   twice), or held (2, then hold 3). So 1, 1, 1 is 11 double-tapped.
//!
//! That's 9 × 2 + 81 × 3 = 261 distinct events. What each does is up to
//! whoever receives them (see `crate::bthome` and `gb::Outgoing::Button`).
//!
//! A hold also says when it ends, in either layout: lifting the finger
//! sends a [`Gesture::Release`] with the same code (see [`Grid::lift`]),
//! for what should last as long as the finger stays down, like dimming a
//! light.
//!
//! Telling them apart takes waiting, since every tap might be the start of
//! something longer: [`Grid`] holds a tap until it can't be anything else.
//!
//! - With two layers, after a first tap, another touch that *starts* within
//!   [`WAIT_SECOND`] of its lift makes a sequence; otherwise it was a
//!   single tap, sent then. So single taps arrive that much late, which
//!   the grid shows (the button stays lit while it waits).
//! - After the tap that could be double-tapped (a sequence's second, or
//!   with six buttons, any first tap), a tap on the same button starting
//!   within [`WAIT_DOUBLE`] makes it a double tap; otherwise it was a
//!   tap, sent then.
//! - A hold needs no waiting: it's sent the moment it's recognised, while
//!   the finger is still down (see [`crate::gesture::LONG_PRESS`]). It can
//!   only be a single's, or end a sequence: nothing follows it.
//! - Anything that can't continue what's waiting (a touch that started
//!   too late, a tap on another button, or a hold, after a tap that only
//!   a double tap could continue) sends what was waiting as it stood, and
//!   starts afresh.
//! - A swipe, or the side button, cancels what's waiting: nothing is sent
//!   (see [`Grid::cancel`]). A hold's release is never cancelled: once
//!   its start went, something may be waiting for its end.
//!
//! The waits are measured from one touch's lift to the next one's start,
//! not between the moments each was recognised: a hold is only recognised
//! [`crate::gesture::LONG_PRESS`] after it starts, and a second button's
//! hold must still count as part of the sequence. So a wait can't run out
//! while a finger is down (see [`Grid::expire`]); the touch decides when
//! it ends.
//!
//! Hardware-free, so it runs in host tests (`host-tests/buttons`).

use embassy_time::{Duration, Instant};

/// After a first tap, how long another touch has to start for the two to
/// be a sequence. Moving to another button and tapping it takes a few
/// hundred milliseconds; misreading a sequence as two singles would send
/// two wrong events, which is worse than a single arriving late.
///
/// TODO: tune from field logs (`buttons:` lines), once there are some.
pub const WAIT_SECOND: Duration = Duration::from_millis(800);
/// After a tap that a double tap could continue (a sequence's second; with
/// six buttons, any), how long a tap on the same button has to start for
/// a double tap: a quick double tap leaves 100–200 ms between lift and
/// press.
pub const WAIT_DOUBLE: Duration = Duration::from_millis(400);

/// What the last button of a [`Press`] did.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[cfg_attr(target_os = "none", derive(defmt::Format))]
pub enum Gesture {
    Tap,
    /// With two layers, only ever the end of a sequence.
    DoubleTap,
    /// Held down: sent the moment it's recognised, while the finger is
    /// still down.
    Hold,
    /// The finger lifted after a [`Gesture::Hold`] (of the same code).
    Release,
}

impl Gesture {
    /// As Gadgetbridge's intent carries it, and the logs show it: BTHome's
    /// names for the same button events (see `crate::bthome`), so a
    /// gesture reads the same wherever it went.
    pub fn name(self) -> &'static str {
        match self {
            Self::Tap => "press",
            Self::DoubleTap => "double_press",
            Self::Hold => "hold_press",
            Self::Release => "hold_release",
        }
    }
}

/// One of the events (or a hold's release): which code, and what its last
/// button did.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[cfg_attr(target_os = "none", derive(defmt::Format))]
pub struct Press {
    /// The button, 1–6, with six buttons. With two layers, 1–9 for a
    /// single, 11–99 (no 0s) for a sequence: its two buttons as digits.
    pub code: u8,
    pub gesture: Gesture,
}

impl Press {
    fn single(button: u8, gesture: Gesture) -> Self {
        Self {
            code: button,
            gesture,
        }
    }

    fn sequence(first: u8, button: u8, gesture: Gesture) -> Self {
        Self {
            code: first * 10 + button,
            gesture,
        }
    }

    /// The button it ended on.
    pub fn last(self) -> u8 {
        self.code % 10
    }
}

/// Which button a touch at (`x`, `y`), in canvas pixels, is on: the
/// screen in thirds across, and in halves (six buttons) or thirds (two
/// layers) down, so even the gaps between the buttons as drawn belong to
/// one.
pub fn at(layout: Layout, x: i32, y: i32) -> u8 {
    let third = |v: i32| (v.clamp(0, 79) * 3 / 80) as u8;
    let row = match layout {
        Layout::Six => (y.clamp(0, 79) / 40) as u8,
        Layout::TwoLayers => third(y),
    };
    row * 3 + third(x) + 1
}

/// What a touch on a button was, for [`Grid::touch`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Touch {
    Tap,
    Hold,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum State {
    Idle,
    /// One tap, on `button`: a single tap, unless another touch starts
    /// by `until` (with six buttons, a tap on the same button: a double
    /// tap).
    First {
        button: u8,
        until: Instant,
    },
    /// `first`, then a tap on `button`: the sequence's tap, unless
    /// `button` is tapped again, starting by `until`.
    Second {
        first: u8,
        button: u8,
        until: Instant,
    },
    /// A hold of `code` was sent, and its finger is still down: idle, but
    /// for the release that goes on the lift (see [`Grid::lift`]).
    Held {
        code: u8,
    },
}

/// The grid's state between touches: what's waiting to see whether more
/// follows (see the module docs).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Grid {
    state: State,
}

impl Default for Grid {
    fn default() -> Self {
        Self { state: State::Idle }
    }
}

impl Grid {
    /// A touch on `button` (see [`at`]) in `layout`, which started at
    /// `started`, and was recognised `now` (a tap's lift; a hold, while
    /// still down). Returns what that completes, in order: at most two
    /// presses, one that was waiting and can't go on, and one this touch
    /// completes.
    pub fn touch(
        &mut self,
        layout: Layout,
        button: u8,
        touch: Touch,
        started: Instant,
        now: Instant,
    ) -> [Option<Press>; 2] {
        let mut done = [None, None];
        // A hold's finger lifts before the next touch can start, so its
        // release has gone (see `touch::finish`); in case it hadn't, this
        // touch starts afresh all the same.
        if let State::Held { .. } = self.state {
            self.state = State::Idle;
        }
        // Too late to continue what was waiting, which is complete as it
        // stood. (The UI loop wakes at the deadline, but only expires it
        // with no finger down; this was that finger.)
        if self.deadline().is_some_and(|until| started > until) {
            done[0] = self.flush();
        }
        // A tap only a double tap can continue (a sequence's second; with
        // six buttons, any first), and this isn't one: what was waiting
        // completes as it stood, and this touch starts afresh.
        let last = match (self.state, layout) {
            (State::Second { button, .. }, _) | (State::First { button, .. }, Layout::Six) => {
                Some(button)
            }
            _ => None,
        };
        if last.is_some_and(|last| touch == Touch::Hold || button != last) {
            done[0] = self.flush();
        }
        let (state, press) = match (self.state, touch) {
            (State::Idle | State::Held { .. }, Touch::Tap) => (
                State::First {
                    button,
                    until: now
                        + match layout {
                            Layout::Six => WAIT_DOUBLE,
                            Layout::TwoLayers => WAIT_SECOND,
                        },
                },
                None,
            ),
            (State::Idle | State::Held { .. }, Touch::Hold) => (
                State::Held { code: button },
                Some(Press::single(button, Gesture::Hold)),
            ),
            // With six buttons, the same button again (anything else was
            // flushed above): a double tap.
            (State::First { .. }, Touch::Tap) if layout == Layout::Six => {
                (State::Idle, Some(Press::single(button, Gesture::DoubleTap)))
            }
            (State::First { button: first, .. }, Touch::Tap) => (
                State::Second {
                    first,
                    button,
                    until: now + WAIT_DOUBLE,
                },
                None,
            ),
            (State::First { button: first, .. }, Touch::Hold) => (
                State::Held {
                    code: first * 10 + button,
                },
                Some(Press::sequence(first, button, Gesture::Hold)),
            ),
            // The same button again (the rest was flushed above).
            (State::Second { first, .. }, _) => (
                State::Idle,
                Some(Press::sequence(first, button, Gesture::DoubleTap)),
            ),
        };
        self.state = state;
        done[1] = press;
        done
    }

    /// The finger lifted after a long press: the release of the hold it
    /// sent, if it sent one. Whatever screen it lifted on, as the hold's
    /// start has gone, and something may be waiting for its end (a light
    /// dimming, say).
    pub fn lift(&mut self) -> Option<Press> {
        let State::Held { code } = self.state else {
            return None;
        };
        self.state = State::Idle;
        Some(Press {
            code,
            gesture: Gesture::Release,
        })
    }

    /// Completes what's waiting if its wait is over at `now`, unless a
    /// finger is down: that touch may have started in time, and decides
    /// (see [`Grid::touch`]).
    pub fn expire(&mut self, now: Instant, finger_down: bool) -> Option<Press> {
        match self.deadline() {
            Some(until) if now >= until && !finger_down => self.flush(),
            _ => None,
        }
    }

    /// Forgets what's waiting, unsent; returns whether anything was. Not
    /// a hold's release (see [`Grid::lift`]).
    pub fn cancel(&mut self) -> bool {
        match self.state {
            State::Idle | State::Held { .. } => false,
            State::First { .. } | State::Second { .. } => {
                self.state = State::Idle;
                true
            }
        }
    }

    /// When what's waiting has waited long enough to be sent, if anything
    /// is waiting.
    pub fn deadline(&self) -> Option<Instant> {
        match self.state {
            State::Idle | State::Held { .. } => None,
            State::First { until, .. } | State::Second { until, .. } => Some(until),
        }
    }

    /// What's waiting, for the screen: the first button tapped, and, once
    /// there's a second (with two layers), that one.
    pub fn waiting(&self) -> (Option<u8>, Option<u8>) {
        match self.state {
            State::Idle | State::Held { .. } => (None, None),
            State::First { button, .. } => (Some(button), None),
            State::Second { first, button, .. } => (Some(first), Some(button)),
        }
    }

    /// What's waiting, completed as it stands.
    fn flush(&mut self) -> Option<Press> {
        let press = match self.state {
            State::Idle | State::Held { .. } => None,
            State::First { button, .. } => Some(Press::single(button, Gesture::Tap)),
            State::Second { first, button, .. } => {
                Some(Press::sequence(first, button, Gesture::Tap))
            }
        };
        self.state = State::Idle;
        press
    }
}

/// How the grid is laid out, and so what its presses are (see the module
/// docs): six buttons, each tapped, double-tapped or held, or nine that
/// make codes 1–99 (none ending in 0), alone or in pairs: two layers.
///
/// Six by default: they suit Home Assistant best (each button and gesture
/// its own trigger in its automation editor, no YAML), and press at once
/// rather than waiting to see whether a second button follows. Two
/// layers are for those who want more than six things at hand.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
#[cfg_attr(target_os = "none", derive(defmt::Format))]
pub enum Layout {
    #[default]
    Six,
    TwoLayers,
}

impl Layout {
    /// As stored (`storage::key::GRID`). Unknown values read as six.
    pub fn from_byte(byte: u8) -> Self {
        match byte {
            1 => Self::TwoLayers,
            _ => Self::Six,
        }
    }

    pub fn to_byte(self) -> u8 {
        self as u8
    }

    /// The other one (a tap on the Settings screen).
    pub fn next(self) -> Self {
        match self {
            Self::Six => Self::TwoLayers,
            Self::TwoLayers => Self::Six,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Six => "6 BUTTONS",
            Self::TwoLayers => "2 LAYERS",
        }
    }
}

/// Whether the grid is on, and where its presses go: to Home Assistant
/// (over BTHome, see `crate::bthome`), to the phone (as an Android intent,
/// through Gadgetbridge; see `gb::Outgoing::Button`), or both.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
#[cfg_attr(target_os = "none", derive(defmt::Format))]
pub enum Setting {
    /// No grid: the screen isn't there. The default, as the grid does
    /// nothing until set up at the other end.
    #[default]
    Off,
    Ha,
    Phone,
    Both,
}

impl Setting {
    /// As stored (`storage::key::BUTTONS`). Unknown values read as off.
    pub fn from_byte(byte: u8) -> Self {
        match byte {
            1 => Self::Ha,
            2 => Self::Phone,
            3 => Self::Both,
            _ => Self::Off,
        }
    }

    pub fn to_byte(self) -> u8 {
        self as u8
    }

    /// The next choice, round the cycle (a tap on the Settings screen).
    pub fn next(self) -> Self {
        match self {
            Self::Off => Self::Ha,
            Self::Ha => Self::Phone,
            Self::Phone => Self::Both,
            Self::Both => Self::Off,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Off => "OFF",
            Self::Ha => "HA",
            Self::Phone => "PHONE",
            Self::Both => "HA+PHONE",
        }
    }

    pub fn on(self) -> bool {
        self != Self::Off
    }

    /// Whether presses go to Home Assistant.
    pub fn ha(self) -> bool {
        matches!(self, Self::Ha | Self::Both)
    }

    /// Whether presses go to the phone.
    pub fn phone(self) -> bool {
        matches!(self, Self::Phone | Self::Both)
    }
}
