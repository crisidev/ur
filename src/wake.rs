//! What a touch does while the screen is off: wake it, control the
//! phone's media, or nothing.
//!
//! A touch on a dark screen is as likely a sleeve, a pocket, or bedding as
//! a finger, so most do nothing. What does something:
//!
//! - A deliberate tap (see [`deliberate`]) wakes the screen.
//! - A double tap (two taps within the UI's `DOUBLE_TAP`) wakes it too;
//!   while the phone has media, it skips to the next track instead,
//!   without waking it.
//! - While the phone has media, a long press plays or pauses, also
//!   without waking it. The touch task reports it while the finger is
//!   still down, once it's been still for long enough, and the buzz then
//!   tells the wearer it took (see [`crate::touch`]).
//!
//! The media gestures are the clock face's, so they work the same whether
//! the screen is on, glancing, or off: the wearer can pause or skip
//! without looking. A deliberate tap still wakes the screen at once, and
//! may be the first of a double tap, on the clock face it woke to, as it
//! may on a glance.
//!
//! Guards against brushes, from the field logs (see [`deliberate`]):
//!
//! - Every touch that does something here must start away from the
//!   screen's edge ([`TAP_MARGIN`]): brushes, and the bedding's phantom
//!   long presses (two of nearly 10 s, one night, both at the top edge),
//!   start there. That goes for both taps of a double tap: three phantom
//!   pairs in a night's log each had a tap at the edge.
//! - A long press must stay put (the touch task's [`SWIPE_DISTANCE`]
//!   tolerance), or it's a swipe, which does nothing here.
//! - In the quiet night (see [`crate::sleep::Night`]), nothing: bedding
//!   and pillows press and tap all night, and the touch controller sleeps
//!   anyway.
//!
//! TODO: a long press held for seconds (a pressing object rather than a
//! finger) could undo its play or pause. Not done: it fires at 0.6 s, so
//! only an undo could help, and the field logs' presses that long all
//! started at the edge, which the margin already turns away.
//!
//! Hardware-free, so it runs in host tests (`host-tests/wake`).
//!
//! [`SWIPE_DISTANCE`]: crate::touch

/// A touch's kind, as far as the dark screen cares.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Tap,
    LongPress,
    /// A swipe, which never does anything with the screen off.
    Swipe,
}

/// What a touch with the screen off does.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    /// Nothing. It isn't the first of a double tap either.
    Ignore,
    /// Nothing yet, but it may be the first of a double tap.
    FirstTap,
    /// Wake the screen, to the clock. `first_tap`: the tap may also be the
    /// first of a double tap on the clock it woke to (while the phone has
    /// media, for the next track).
    Wake { first_tap: bool },
    /// Play or pause the phone's media, leaving the screen off.
    PlayPause,
    /// Skip to the next track, leaving the screen off.
    Next,
}

/// The shortest tap that counts as [`deliberate`], in milliseconds.
pub const DELIBERATE_TAP_MS: u16 = 40;
/// Canvas pixels (24 panel pixels) at the screen's edge where touches,
/// with the screen off, do nothing.
pub const TAP_MARGIN: i32 = 8;
/// The canvas's width and height.
const SIZE: i32 = 80;

/// Whether a touch starting at (`x`, `y`), in canvas pixels, starts away
/// from the screen's edge.
pub fn inside(x: i32, y: i32) -> bool {
    let inside = |v: i32| (TAP_MARGIN..SIZE - TAP_MARGIN).contains(&v);
    inside(x) && inside(y)
}

/// Whether a tap looks meant, rather than a brush: what, with the screen
/// off, a single tap needs to be to wake it. Any touch used to wake it, and
/// brushes against a sleeve or a blanket did.
///
/// From the sealed watch's field logs: brushes were short (8–37 ms), and
/// all started at the screen's edge, within ~20 panel pixels (one, at
/// 146 ms, too). Deliberate taps lasted 41–135 ms, and the ones that
/// woke the screen landed well inside. So: a tap of at least
/// [`DELIBERATE_TAP_MS`] that starts outside the outer tenth of the screen
/// ([`TAP_MARGIN`]).
pub fn deliberate(kind: Kind, x: i32, y: i32, held_ms: u16) -> bool {
    kind == Kind::Tap && held_ms >= DELIBERATE_TAP_MS && inside(x, y)
}

/// What a touch does with the screen off: of `kind`, starting at (`x`,
/// `y`) in canvas pixels, after `held_ms`. `recent_tap`: a tap that this
/// may be the second of came within the double tap's time (one
/// [`Action::FirstTap`] said may count). `media`: the phone has media to
/// control. `quiet`: it's the quiet night.
pub fn screen_off(
    kind: Kind,
    x: i32,
    y: i32,
    held_ms: u16,
    recent_tap: bool,
    media: bool,
    quiet: bool,
) -> Action {
    if quiet || !inside(x, y) {
        return Action::Ignore;
    }
    match kind {
        Kind::LongPress if media => Action::PlayPause,
        Kind::Tap if recent_tap && media => Action::Next,
        Kind::Tap if recent_tap => Action::Wake { first_tap: false },
        Kind::Tap if deliberate(kind, x, y, held_ms) => Action::Wake { first_tap: media },
        Kind::Tap => Action::FirstTap,
        Kind::LongPress | Kind::Swipe => Action::Ignore,
    }
}
