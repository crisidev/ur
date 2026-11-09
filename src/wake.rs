//! What a touch does while the screen is off (or glancing): wake it,
//! control the phone's media, or nothing.
//!
//! A touch on a dark screen is as likely a sleeve, a pocket, or bedding as
//! a finger, so most do nothing. What does something:
//!
//! - A deliberate tap (see [`deliberate`]) wakes the screen, at once.
//! - A double tap (two taps within the UI's `DOUBLE_TAP`) wakes it too;
//!   while a phone is connected, it skips to the next track instead,
//!   without waking it.
//! - While a phone is connected, a long press in the middle of the screen
//!   (see [`centred`]) plays or pauses, also without waking it. The touch
//!   task reports it while the finger is still down, once it's been still
//!   for long enough, and the buzz then tells the wearer it took (see
//!   [`crate::gesture`]).
//!
//! The media gestures are the clock face's, so they work the same whether
//! the screen is on, glancing, or off (bar where a long press must start;
//! see below): the wearer can pause or skip without looking. They need
//! only a connected phone, not word from it of what plays: Gadgetbridge
//! only says when that changes, so after the watch restarts there may be
//! none for a long while, and its commands act on the phone's player
//! regardless.
//!
//! A deliberate tap wakes the screen at once, so for a double tap to skip
//! without waking it, its first tap must be light (under
//! [`DELIBERATE_TAP_MS`]). If it isn't, the second tap still skips, on the
//! clock face the first woke to. The sealed watch's field log (0.9.0-m3.1)
//! has the wearer's double taps from off with their first tap held
//! 47–67 ms, so all of them woke the screen; 0.9.0's milestones tried waiting 300 ms after a
//! deliberate tap for a second before waking, but the wearer would rather
//! have a single tap's wake prompt, and tap lighter to skip in the dark.
//!
//! The same goes on a glance (see [`glance`]), where the screen is already
//! lit and the wearer looking: a deliberate tap wakes it properly at once,
//! a light double tap skips, leaving the glance be, and a long press in
//! the middle plays or pauses.
//!
//! Guards against brushes, from the field logs (see [`deliberate`]):
//!
//! - Every touch that does something here must start away from the
//!   screen's edge ([`TAP_MARGIN`]): brushes, and the bedding's phantom
//!   long presses (two of nearly 10 s, one night, both at the top edge),
//!   start there. That goes for both taps of a double tap: three phantom
//!   pairs in a night's log each had a tap at the edge.
//! - A long press must start in the middle of the screen, further from
//!   every edge than a tap need be ([`LONG_PRESS_MARGIN`], twice
//!   [`TAP_MARGIN`]): with the wearer's arms folded, the other arm resting
//!   against the watch's edge played or paused. That goes on a glance too,
//!   where a tap at the edge still counts: folding the arms turns the
//!   wrist, which can light a glance. Not on the awake clock face, though,
//!   where a long press anywhere plays or pauses: something woke the
//!   screen first, so it's likelier meant.
//! - A long press must stay put (the touch task's [`SWIPE_DISTANCE`]
//!   tolerance), or it's a swipe, which does nothing here.
//! - In the quiet night (see [`crate::sleep::Night`]), nothing: bedding
//!   and pillows press and tap all night, and the touch controller sleeps
//!   anyway.
//!
//! Considered and declined: undoing the play or pause of a long press held
//! for seconds (a pressing object rather than a finger). It fires at
//! 0.6 s, so only an undo could help, and the field logs' presses that
//! long all started at the edge, which the margin already turns away.
//!
//! Hardware-free, so it runs in host tests (`host-tests/wake`).
//!
//! [`SWIPE_DISTANCE`]: crate::gesture::SWIPE_DISTANCE

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
    /// first of a double tap on the clock face it woke to (while a phone is
    /// connected, for the next track).
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
/// Canvas pixels (48 panel pixels, a fifth of the screen) at the screen's
/// edge where a long press doesn't play or pause with the screen off or on
/// a glance: see [`centred`].
pub const LONG_PRESS_MARGIN: i32 = 2 * TAP_MARGIN;
/// The canvas's width and height.
const SIZE: i32 = 80;

/// Whether (`x`, `y`), in canvas pixels, is further than `margin` from
/// every edge of the screen.
fn within(x: i32, y: i32, margin: i32) -> bool {
    let within = |v: i32| (margin..SIZE - margin).contains(&v);
    within(x) && within(y)
}

/// Whether a touch starting at (`x`, `y`), in canvas pixels, starts away
/// from the screen's edge.
pub fn inside(x: i32, y: i32) -> bool {
    within(x, y, TAP_MARGIN)
}

/// Whether a long press starting at (`x`, `y`), in canvas pixels, starts
/// in the middle of the screen (the central 48 by 48 canvas pixels, away
/// from [`LONG_PRESS_MARGIN`]): what it needs to play or pause, with the
/// screen off or on a glance.
///
/// The wearer's other arm, resting against the watch's edge with their
/// arms folded, played or paused. No field log caught that, but the
/// touch controller puts a press from the edge near it, though not always
/// within [`TAP_MARGIN`]: the phantom long presses of bedding, at night,
/// started 4–11 canvas pixels from the top edge (the one at 11 was far
/// enough in to play or pause). The long presses that did play or pause,
/// in a 0.9.0 milestone's field log, started 30–40 down and 45–55
/// across, at the foot of the time; others in older logs, 44–52 down and
/// 39–57 across. So twice the tap's margin: all of the latter count, none
/// of the former. (Bedding pressed mid-screen too, once, which no margin
/// stops: the quiet night does.) Where it started is where it stayed: a
/// long press moves less than [`SWIPE_DISTANCE`] (under 7 canvas pixels)
/// on the way.
///
/// [`SWIPE_DISTANCE`]: crate::gesture::SWIPE_DISTANCE
pub fn centred(x: i32, y: i32) -> bool {
    within(x, y, LONG_PRESS_MARGIN)
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
/// [`Action::FirstTap`] said may count). `phone`: a phone is connected,
/// whose media to control. `quiet`: it's the quiet night.
pub fn screen_off(
    kind: Kind,
    x: i32,
    y: i32,
    held_ms: u16,
    recent_tap: bool,
    phone: bool,
    quiet: bool,
) -> Action {
    if quiet || !inside(x, y) {
        return Action::Ignore;
    }
    match kind {
        Kind::LongPress if phone && centred(x, y) => Action::PlayPause,
        Kind::Tap if recent_tap && phone => Action::Next,
        Kind::Tap if recent_tap => Action::Wake { first_tap: false },
        Kind::Tap if deliberate(kind, x, y, held_ms) => Action::Wake { first_tap: phone },
        Kind::Tap => Action::FirstTap,
        Kind::LongPress | Kind::Swipe => Action::Ignore,
    }
}

/// What a touch does on a glance, while a phone is connected (otherwise,
/// and in the quiet night, any touch there wakes the screen properly): of
/// `kind`, starting at (`x`, `y`) in canvas pixels, after `held_ms`.
/// `recent_tap`: as for [`screen_off`].
///
/// A long press in the middle ([`centred`]) plays or pauses; one nearer
/// the edge is ignored, as with the screen off, and the glance goes on. A
/// swipe wakes the screen properly. Unlike with the screen off, a tap at
/// the edge counts, and only how long it was held tells a light tap from
/// a deliberate one: the wearer is looking, so it's unlikely a brush, and
/// before the media gestures, any touch woke a glance.
pub fn glance(kind: Kind, x: i32, y: i32, held_ms: u16, recent_tap: bool) -> Action {
    match kind {
        Kind::LongPress if centred(x, y) => Action::PlayPause,
        Kind::LongPress => Action::Ignore,
        Kind::Swipe => Action::Wake { first_tap: false },
        Kind::Tap if recent_tap => Action::Next,
        Kind::Tap if held_ms >= DELIBERATE_TAP_MS => Action::Wake { first_tap: true },
        Kind::Tap => Action::FirstTap,
    }
}
