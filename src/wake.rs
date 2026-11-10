//! What a touch does while the screen is off (or glancing): wake it,
//! control the phone's media, or nothing.
//!
//! A touch on a dark screen is as likely a sleeve, a pocket, or bedding as
//! a finger, so most do nothing. What does something:
//!
//! - A deliberate tap (see [`deliberate`]) wakes the screen, at once.
//! - Without a phone, a double tap (two taps within [`DOUBLE_TAP_MS`])
//!   wakes it too.
//! - While a phone is connected, two touches play, pause, or skip, without
//!   waking it: a double tap plays or pauses, and a tap, then a touch held
//!   (a tap-and-hold), skips to the next track (see [`second`], and
//!   [`follows`] for how soon the second must come). The touch task
//!   reports the hold while the finger is still down, once it's been still
//!   for long enough, and the buzz then tells the wearer it took (see
//!   [`crate::gesture`]).
//!
//! The media gestures work the same whether the screen is off or glancing,
//! and on the clock face a tap from either woke (see below): the wearer can
//! pause or skip without looking. They need only a connected phone, not
//! word from it of what plays: Gadgetbridge only says when that changes,
//! so after the watch restarts there may be none for a long while, and its
//! commands act on the phone's player regardless. The awake clock face
//! has the same gestures, so the wearer needn't know whether the screen
//! is on to use them, with looser guards (see `crate::ui`, and
//! [`awake_long_press`]): something woke the screen to get there.
//!
//! A deliberate tap wakes the screen at once, so for a media gesture to
//! leave it off, its first tap must be light (under [`DELIBERATE_TAP_MS`]).
//! If it isn't, the second touch still counts, on the clock face the first
//! woke to: the same gesture does the same, however firm its first tap.
//! The sealed watch's field log (0.9.0-m3.1) has the wearer's double taps
//! from off with their first tap held 47–67 ms, so all of them woke the
//! screen; 0.9.0's milestones tried waiting 300 ms after a deliberate tap
//! for a second before waking, but the wearer would rather have a single
//! tap's wake prompt, and tap lighter to stay in the dark.
//!
//! The same goes on a glance (see [`glance`]), where the screen is already
//! lit and the wearer looking: a deliberate tap wakes it properly at once,
//! and a light tap's pair plays, pauses, or skips, leaving the glance be.
//!
//! Guards against brushes, from the field logs (see [`deliberate`]):
//!
//! - Nothing happens on one touch alone, bar a deliberate tap's wake. A
//!   long press with the screen off or on a glance used to play or pause,
//!   and an arm folded over the watch, resting against it, kept doing so
//!   (see [`centred`]); now it takes a tap first, which a resting arm
//!   rarely makes just before.
//! - Every touch that does something here must start away from the
//!   screen's edge ([`TAP_MARGIN`]): brushes, and the bedding's phantom
//!   long presses (two of nearly 10 s, one night, both at the top edge),
//!   start there. That goes for both touches of a pair: three phantom
//!   pairs in a night's log each had a tap at the edge, and so did a
//!   folded arm's tap-and-holds (see [`glance`]). On a glance, a tap at
//!   the edge still wakes the screen, if deliberate, but can't be part
//!   of a media gesture.
//! - The hold of a tap-and-hold must start in the middle of the screen,
//!   further from the edges than a tap need be ([`centred`]), as the long
//!   press that played or paused before it had to: a folded arm's long
//!   presses landed up to 15 canvas pixels in from the top or bottom.
//! - A hold must stay put (the touch task's [`SWIPE_DISTANCE`]
//!   tolerance), or it's a swipe, which does nothing here.
//! - In the quiet night (see [`crate::sleep::Night`]), nothing: bedding
//!   and pillows press and tap all night, and the touch controller sleeps
//!   anyway.
//!
//! What still gets through: a pair of light taps the arm made 9–10 canvas
//! pixels from the bottom edge, 117 ms apart, which skipped with the
//! screen off in 0.10.0-rc.1's field log, would now play or pause. A
//! wider margin at the top and bottom for taps too would stop it, but
//! double taps that look like the wearer's have landed 14 from the top,
//! and 60 down.
//!
//! TODO: decide whether a double tap's taps need a wider margin at the
//! top and bottom (12, say), once a field log shows whether the folded
//! arm's double taps keep playing or pausing.
//!
//! Hardware-free, so it runs in host tests (`host-tests/wake`).
//!
//! [`SWIPE_DISTANCE`]: crate::gesture::SWIPE_DISTANCE

/// A touch's kind, as far as the dark screen cares.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Tap,
    /// A touch held still: the touch task reports it as a long press, while
    /// the finger is still down.
    LongPress,
    /// A swipe, which never does anything with the screen off.
    Swipe,
}

/// What a touch with the screen off does.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    /// Nothing. It isn't the first of a pair either.
    Ignore,
    /// Nothing yet, but it may be the first of a pair (see [`second`]).
    FirstTap,
    /// Wake the screen, to the clock. `first_tap`: the tap may also be the
    /// first of a media gesture's pair, whose second comes on the clock
    /// face it woke to (while a phone is connected; see [`second`]).
    Wake { first_tap: bool },
    /// Play or pause the phone's media, leaving the screen off.
    PlayPause,
    /// Skip to the next track, leaving the screen off.
    Next,
}

/// The shortest tap that counts as [`deliberate`], in milliseconds.
pub const DELIBERATE_TAP_MS: u16 = 40;
/// How soon after a tap's lift, in milliseconds, the next touch must come
/// for the two to be a pair (see [`follows`]): a double tap, or a
/// tap-and-hold; on the awake clock face too.
pub const DOUBLE_TAP_MS: u32 = 500;
/// Canvas pixels (24 panel pixels) at the screen's edge where touches,
/// with the screen off, do nothing.
pub const TAP_MARGIN: i32 = 8;
/// Canvas pixels (48 panel pixels, a fifth of the screen) at the screen's
/// left and right edges where a hold doesn't skip with the screen off or
/// on a glance (nor on the clock face just after a touch woke it): see
/// [`centred`].
pub const LONG_PRESS_MARGIN_X: i32 = 2 * TAP_MARGIN;
/// Canvas pixels (60 panel pixels, a quarter of the screen) at the
/// screen's top and bottom edges where a hold doesn't skip with the
/// screen off or on a glance (nor a long press play or pause on the clock
/// face just after a touch woke it): see [`centred`].
pub const LONG_PRESS_MARGIN_Y: i32 = 20;
/// Milliseconds after a touch woke the screen (from off, or a glance)
/// during which a tap-and-hold's hold on the clock face must be in the
/// middle too: see [`awake_long_press`].
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

/// Whether a touch held still, starting at (`x`, `y`), in canvas pixels,
/// starts in the middle of the screen (16..=63 across and 20..=59 down,
/// away from [`LONG_PRESS_MARGIN_X`] and [`LONG_PRESS_MARGIN_Y`]): what a
/// tap-and-hold's hold needs to skip, with the screen off or on a glance
/// (and on the clock face just after a touch woke it; see
/// [`awake_long_press`]).
///
/// A long press on its own used to play or pause with the screen off or
/// on a glance, and the wearer's other arm, resting against the watch
/// with their arms folded, played or paused. A day's field log
/// (0.10.0-rc.1) caught it, each time undone, on the watch or the phone,
/// seconds or minutes later: with the screen off, long presses started 11
/// and 12 canvas pixels from the top edge (held 9.7 s and 1.3 s); on a
/// glance, 3 from the top and 64 down (so 15 from the bottom). With the
/// arms folded, the other forearm lies along the watch's top or bottom
/// edge, so that's where it presses, and where the margin is wider than
/// at the sides. The touch controller places such a press up to 15 canvas
/// pixels in; the bedding's phantom long presses at night were 4–11 from
/// the top edge.
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
/// like the wearer's. That a hold must now follow a tap does.)
///
/// Where it started is where it stayed: a hold moves less than
/// [`SWIPE_DISTANCE`] (under 7 canvas pixels) on the way.
///
/// [`SWIPE_DISTANCE`]: crate::gesture::SWIPE_DISTANCE
pub fn centred(x: i32, y: i32) -> bool {
    within(x, LONG_PRESS_MARGIN_X) && within(y, LONG_PRESS_MARGIN_Y)
}

/// Whether a touch of `kind`, reported `since_ms` after a tap lifted, and
/// down for `held_ms` by then, comes soon enough after it to be the second
/// of a pair: within [`DOUBLE_TAP_MS`].
///
/// For a tap, from the first's lift to the second's, as for a double tap
/// on the awake clock face (and as it always was): so the second must
/// start within half a second, less its own length (the wearer's from
/// off, in 0.9.0-m3.1's field log: 37–57 ms, 126–156 ms after the
/// first).
///
/// For a hold, from the first's lift to the hold's start: it's only
/// reported once it's been down [`LONG_PRESS`] (0.6 s), which would leave
/// no time at all if counted as a tap's lift is. Counting from its start
/// gives it the window a tap gets, plus the few tens of milliseconds a
/// second tap is down; as for the button grid's holds, which also count
/// from where the finger landed (see `crate::buttons`).
///
/// The hold itself is the touch task's long press, at 0.6 s, buzzing
/// then, rather than a shorter hold of its own: the wearer already knows
/// it (from the clock face, the alarms, and the button grid), and the
/// buzz says when to let go. A tap-and-hold so takes a little over a
/// second, start to buzz. A second tap held 0.6 s or more is a hold, not a
/// tap; the field logs' deliberate taps last at most 135 ms.
///
/// [`LONG_PRESS`]: crate::gesture::LONG_PRESS
pub fn follows(kind: Kind, since_ms: u32, held_ms: u16) -> bool {
    let gap = match kind {
        Kind::LongPress => since_ms.saturating_sub(held_ms.into()),
        Kind::Tap | Kind::Swipe => since_ms,
    };
    gap <= DOUBLE_TAP_MS
}

/// The media gesture a touch of `kind`, starting at (`x`, `y`) in canvas
/// pixels, makes as the second of a pair (see [`follows`]), with the
/// screen off, on a glance, or on the clock face its first tap woke: a tap
/// plays or pauses (a double tap), and a hold, in the middle
/// ([`centred`]), skips to the next track (a tap-and-hold). `None` for
/// anything else, which isn't a media gesture, and is taken on its own.
/// Both must start away from the edge ([`inside`]); the first tap's place
/// is for whoever remembered it to check.
///
/// Play or pause is the double tap, the quicker and likelier of the two,
/// and skipping the hold: as the wearer asked, so that neither is a lone
/// long press, which an arm folded over the watch made.
pub fn second(kind: Kind, x: i32, y: i32) -> Option<Action> {
    match kind {
        Kind::Tap if inside(x, y) => Some(Action::PlayPause),
        Kind::LongPress if centred(x, y) => Some(Action::Next),
        _ => None,
    }
}

/// Whether a long press on the awake clock face, starting at (`x`, `y`)
/// in canvas pixels, skips to the next track, as a tap-and-hold's hold
/// (it [`follows`] a tap; on its own, it does nothing). `touch_woke`: a
/// touch woke the screen (from off, or a glance) under
/// [`CENTRE_AFTER_TOUCH_WAKE_MS`] ago.
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
/// That was when a long press on its own played or paused there; now it
/// takes a tap first, which a resting arm rarely makes just before, but
/// the guard stays, as the arm has made tap-and-holds too (see
/// [`glance`]). Measured, as the UI does, from the waking touch's report (a tap's or
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
/// `y`) in canvas pixels, after `held_ms`. `recent_tap`: it [`follows`] a
/// tap that may be the first of a pair (one [`Action::FirstTap`] said
/// may). `phone`: a phone is connected, whose media to control. `quiet`:
/// it's the quiet night.
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
    if recent_tap {
        // With a phone, a media gesture's second touch; without one, a
        // double tap is a way to wake the screen.
        match (phone, second(kind, x, y)) {
            (true, Some(action)) => return action,
            (false, _) if kind == Kind::Tap => return Action::Wake { first_tap: false },
            _ => {}
        }
    }
    match kind {
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
/// A light tap's pair plays, pauses, or skips (see [`second`]), and the
/// glance goes on. A long press on its own is ignored, as with the screen
/// off, and so is a light tap at the edge. A swipe wakes the screen
/// properly. Unlike with the screen off, a deliberate tap at the edge
/// wakes it too: the wearer is looking, so it's unlikely a brush, and
/// before the media gestures, any touch woke a glance. It can't be the
/// first of a media gesture, though: in 0.10.0-rc.1's field log, a
/// folded arm's 254 ms tap at the top edge of a glance woke the screen,
/// and the arm's hold there 9 ms later would have skipped on the clock
/// face (it played or paused, as a long press did then). Another of its
/// holds, at the top edge with the screen off, followed a 9 ms tap there
/// by 361 ms.
pub fn glance(kind: Kind, x: i32, y: i32, held_ms: u16, recent_tap: bool) -> Action {
    if recent_tap && let Some(action) = second(kind, x, y) {
        return action;
    }
    match kind {
        Kind::Swipe => Action::Wake { first_tap: false },
        Kind::LongPress => Action::Ignore,
        Kind::Tap if held_ms >= DELIBERATE_TAP_MS => Action::Wake {
            first_tap: inside(x, y),
        },
        Kind::Tap if inside(x, y) => Action::FirstTap,
        Kind::Tap => Action::Ignore,
    }
}
