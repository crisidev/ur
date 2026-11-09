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
//!   the edges than a tap need be ([`LONG_PRESS_MARGIN_X`] at the sides,
//!   twice [`TAP_MARGIN`], and [`LONG_PRESS_MARGIN_Y`] at the top and
//!   bottom, a quarter of the screen): with the wearer's arms folded, the
//!   other arm, resting against the watch, played or paused. That goes on
//!   a glance too, where a tap at the edge still counts: folding the arms
//!   turns the wrist, which can light a glance. Not on the awake clock
//!   face, though, where a long press anywhere plays or pauses: something
//!   woke the screen first, so it's likelier meant. Unless that was a
//!   touch, under [`CENTRE_AFTER_TOUCH_WAKE_MS`] before (see
//!   [`awake_long_press`]): the folded arm wakes the screen too.
//! - A long press must stay put (the touch task's [`SWIPE_DISTANCE`]
//!   tolerance), or it's a swipe, which does nothing here.
//! - In the quiet night (see [`crate::sleep::Night`]), nothing: bedding
//!   and pillows press and tap all night, and the touch controller sleeps
//!   anyway.
//!
//! Considered and declined: undoing the play or pause of a long press held
//! for seconds (a pressing object rather than a finger). It fires at
//! 0.6 s, so only an undo could help, and of the field logs' presses
//! held 2 s or more, all but one started near the top edge, which the
//! margin already turns away. (The one, held 2.2 s at 25 across and
//! 29 down, a folded arm's by the wearer pausing again 3 s later, no
//! margin stops; the wearer's own lifted within 1 s of the buzz.)
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
/// left and right edges where a long press doesn't play or pause with the
/// screen off or on a glance: see [`centred`].
pub const LONG_PRESS_MARGIN_X: i32 = 2 * TAP_MARGIN;
/// Canvas pixels (60 panel pixels, a quarter of the screen) at the
/// screen's top and bottom edges where a long press doesn't play or pause
/// with the screen off or on a glance: see [`centred`].
pub const LONG_PRESS_MARGIN_Y: i32 = 20;
/// Milliseconds after a touch woke the screen (from off, or a glance)
/// during which a long press on the clock face must be in the middle too:
/// see [`awake_long_press`].
pub const CENTRE_AFTER_TOUCH_WAKE_MS: u32 = 3000;
/// The canvas's width and height.
const SIZE: i32 = 80;

/// Whether `v`, a canvas coordinate, is further than `margin` from both
/// ends of the screen along its axis.
fn within(v: i32, margin: i32) -> bool {
    (margin..SIZE - margin).contains(&v)
}

/// Whether a touch starting at (`x`, `y`), in canvas pixels, starts away
/// from the screen's edge.
pub fn inside(x: i32, y: i32) -> bool {
    within(x, TAP_MARGIN) && within(y, TAP_MARGIN)
}

/// Whether a long press starting at (`x`, `y`), in canvas pixels, starts
/// in the middle of the screen (16..=63 across and 20..=59 down, away
/// from [`LONG_PRESS_MARGIN_X`] and [`LONG_PRESS_MARGIN_Y`]): what it
/// needs to play or pause, with the screen off or on a glance (and on the
/// clock face just after a touch woke it; see [`awake_long_press`]).
///
/// The wearer's other arm, resting against the watch with their arms
/// folded, played or paused. A day's field log (0.10.0-rc.1) caught it,
/// each time undone, on the watch or the phone, seconds or minutes later:
/// with the screen off, long presses started 11 and 12 canvas pixels from
/// the top edge (held 9.7 s and 1.3 s); on a glance, 3 from the top and
/// 64 down (so 15 from the bottom). With the arms folded, the other
/// forearm lies along the watch's top or bottom edge, so that's where it
/// presses, and where the margin is wider than at the sides. The touch
/// controller places such a press up to 15 canvas pixels in; the
/// bedding's phantom long presses at night were 4–11 from the top edge.
///
/// The wearer's own long presses, in that log and older ones, started
/// 30–53 down and 39–59 across: around the foot of the time, a bit right
/// of centre, where the other hand's finger comes in from the side. One,
/// on a glance, pausing what a folded arm had just played, was 59 across:
/// a margin of 20 at the sides, too, would have had it at its very edge,
/// and no press of the arm's started between a fifth and a quarter in
/// from a side. So a quarter at the top and bottom (rejecting 15 from the
/// edge with some to spare, and keeping 30–53 down), and a fifth, twice
/// the tap's margin, at the sides. (Neither stops a press the arm made at
/// 25 across and 29 down, nor bedding's once mid-screen: those look just
/// like the wearer's.)
///
/// Where it started is where it stayed: a long press moves less than
/// [`SWIPE_DISTANCE`] (under 7 canvas pixels) on the way.
///
/// [`SWIPE_DISTANCE`]: crate::gesture::SWIPE_DISTANCE
pub fn centred(x: i32, y: i32) -> bool {
    within(x, LONG_PRESS_MARGIN_X) && within(y, LONG_PRESS_MARGIN_Y)
}

/// Whether a long press on the awake clock face, starting at (`x`, `y`)
/// in canvas pixels, plays or pauses. `touch_woke`: a touch woke the
/// screen (from off, or a glance) under [`CENTRE_AFTER_TOUCH_WAKE_MS`]
/// ago.
///
/// Anywhere, normally: something woke the screen to get there, so it's
/// likelier meant than with the screen off. But a folded arm's touch can
/// be what woke it. In the field log of [`centred`], a 254 ms tap at the
/// top edge of a glance (where a tap counts, the wearer being taken to be
/// looking) woke the screen, and two long presses at the top edge, 0.61
/// and 2.75 s later, played and paused on the awake clock face. So just
/// after a touch woke the screen, the long press must be in the middle,
/// as on the glance: for 3 s, which takes in both, if barely the second.
/// Not longer, so that a long press at the edge after waking the screen
/// on purpose soon counts again. (The wearer's own, 1.3, 2.9, and 5 s
/// after waking the screen with a touch, were in the middle anyway.)
/// Measured, as the UI does, from the waking touch's report (a tap's or
/// swipe's lift) to the long press's (0.6 s after its finger landed).
pub fn awake_long_press(x: i32, y: i32, touch_woke: bool) -> bool {
    !touch_woke || centred(x, y)
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
