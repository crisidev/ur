//! Where screens sit around the clock face, and where a swipe goes.
//!
//! The clock is the hub, with a line of screens on each side of it, nearest
//! first (which screens, [`crate::ui`] decides):
//!
//! ```text
//!                      above (notifications)
//!   left (the past) <-  CLOCK  -> right (what's next)
//!                      below (music, info)
//! ```
//!
//! A swipe brings in the screen on the side opposite the finger's travel:
//! the content follows the finger, so swiping left brings in what's to the
//! right. Within a line, the same swipe goes further out, and the opposite
//! one comes back, to the clock at the end. Nothing else moves: no
//! wrapping round at a line's end (with four lines, "back" would become
//! ambiguous), and no sideways step from one line into another (from
//! Alarms to Info, say), so the way back is always the reverse swipe.
//!
//! Hardware-free, so it runs in host tests (`host-tests/nav`).

/// A side of the clock, where a line of screens sits.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[cfg_attr(target_os = "none", derive(defmt::Format))]
pub enum Side {
    Left,
    Right,
    Above,
    Below,
}

impl Side {
    pub const ALL: [Self; 4] = [Self::Left, Self::Right, Self::Above, Self::Below];

    pub fn opposite(self) -> Self {
        match self {
            Self::Left => Self::Right,
            Self::Right => Self::Left,
            Self::Above => Self::Below,
            Self::Below => Self::Above,
        }
    }
}

/// Which way the finger moved.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Swipe {
    Left,
    Right,
    Up,
    Down,
}

impl Swipe {
    /// The side the swipe moves towards: where the screen it brings in
    /// comes from.
    pub fn towards(self) -> Side {
        match self {
            Self::Left => Side::Right,
            Self::Right => Side::Left,
            Self::Up => Side::Below,
            Self::Down => Side::Above,
        }
    }
}

/// Where a screen is: the clock, or `index` screens out along the line on
/// `side` (0 next to the clock).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Place {
    Clock,
    Line { side: Side, index: usize },
}

/// Where `swipe` goes from `place`, given how many screens each line has
/// right now; `None` if nowhere (the swipe does nothing). Also returns the
/// side the move goes towards, which is what the transition shows.
///
/// A place past its line's end (a screen that just went away) comes back
/// from wherever the line now ends.
pub fn swipe(place: Place, swipe: Swipe, len: impl Fn(Side) -> usize) -> Option<(Place, Side)> {
    let towards = swipe.towards();
    let to = match place {
        Place::Clock if len(towards) > 0 => Place::Line {
            side: towards,
            index: 0,
        },
        Place::Clock => return None,
        Place::Line { side, index } if towards == side && index + 1 < len(side) => Place::Line {
            side,
            index: index + 1,
        },
        Place::Line { side, index } if towards == side.opposite() => {
            match index.min(len(side)).checked_sub(1) {
                Some(index) => Place::Line { side, index },
                None => Place::Clock,
            }
        }
        // Further out from the end of a line, or sideways.
        Place::Line { .. } => return None,
    };
    Some((to, towards))
}

/// The side to sweep towards on the way home to the clock from `place`;
/// `None` if already there.
pub fn home(place: Place) -> Option<Side> {
    match place {
        Place::Clock => None,
        Place::Line { side, .. } => Some(side.opposite()),
    }
}
