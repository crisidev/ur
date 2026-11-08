//! Turn-by-turn directions from the phone's navigation app, for the glance.
//!
//! Gadgetbridge forwards the next manoeuvre to a Bangle.js as
//!
//! ```text
//! {"t":"nav","instr":"Storgata","distance":"150 m","action":"left","eta":"11:55"}
//! ```
//!
//! every time the navigation app updates it, and `{"t":"nav"}` (no fields)
//! when navigation ends. Every field is optional (see `onSetNavigationInfo`
//! in its `BangleJSDeviceSupport.java`, and `NavigationInfoSpec.kt`):
//!
//! - `instr` is whatever text the app gives: Google Maps' notification text
//!   (usually the street, "Storgata toward Karl Johans gate"), OsmAnd's last
//!   voice prompt, CoMaps' next street.
//! - `distance` is a display string in the phone's units and locale ("150
//!   m", "1,2 km", "0.3 mi", "500 ft", or OsmAnd's "150m").
//! - `action` is the manoeuvre, by name (see [`Turn::from_action`]); missing
//!   when the app didn't say, or Gadgetbridge didn't recognise Google Maps'
//!   icon.
//!
//! Google Maps and CoMaps send the empty message when navigation stops;
//! OsmAnd never does. So directions also expire after [`STALE_AFTER_MS`]
//! without an update.
//!
//! This module keeps the latest step, decides when it's gone stale, and
//! when a turn is near enough to buzz for. Hardware-free, so it runs in host
//! tests (`host-tests/route`). Times are milliseconds since boot.

use heapless::String;

/// Longest instruction kept: two lines of the small font on the glance.
pub const STREET_LEN: usize = 32;
/// Longest distance kept ("1,234 km").
pub const DISTANCE_LEN: usize = 8;

/// Directions with no update for this long are dropped. Navigation apps
/// update the distance as you move, and Google Maps at least once a minute
/// for its ETA, so three minutes without one means navigation has most
/// likely ended (OsmAnd never says so) or the phone has gone. Standing
/// still longer only hides the directions until the next update.
pub const STALE_AFTER_MS: u64 = 3 * 60 * 1000;

/// A turn this close gets a buzz (once per turn). On foot (~1.4 m/s) that's
/// about half a minute's warning; on a bike (~5 m/s), ten seconds: soon
/// enough to read the glance and act, late enough not to be mistaken for
/// the turn before.
///
/// TODO: tune on real walks and rides; maybe scale with speed.
pub const NEAR_METRES: u32 = 50;

/// The next manoeuvre. Gadgetbridge's `NavigationInfoSpec.ACTION_*`, by the
/// names its Bangle.js driver sends.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[cfg_attr(target_os = "none", derive(defmt::Format))]
pub enum Turn {
    Straight,
    Left,
    SlightLeft,
    SharpLeft,
    Right,
    SlightRight,
    SharpRight,
    KeepLeft,
    KeepRight,
    UturnLeft,
    UturnRight,
    RoundaboutLeft,
    RoundaboutRight,
    RoundaboutStraight,
    RoundaboutUturn,
    Finish,
    OffRoute,
    /// No action, or one we don't know (Gadgetbridge drops "merge", its
    /// newest, from what it sends a Bangle.js).
    Unknown,
}

impl Turn {
    pub fn from_action(action: Option<&str>) -> Self {
        match action {
            Some("continue") => Self::Straight,
            Some("left") => Self::Left,
            Some("left_slight") => Self::SlightLeft,
            Some("left_sharp") => Self::SharpLeft,
            Some("right") => Self::Right,
            Some("right_slight") => Self::SlightRight,
            Some("right_sharp") => Self::SharpRight,
            Some("keep_left") => Self::KeepLeft,
            Some("keep_right") => Self::KeepRight,
            Some("uturn_left") => Self::UturnLeft,
            Some("uturn_right") => Self::UturnRight,
            Some("roundabout_left") => Self::RoundaboutLeft,
            Some("roundabout_right") => Self::RoundaboutRight,
            Some("roundabout_straight") => Self::RoundaboutStraight,
            Some("roundabout_uturn") => Self::RoundaboutUturn,
            Some("finish") => Self::Finish,
            Some("offroute") => Self::OffRoute,
            _ => Self::Unknown,
        }
    }

    /// Whether getting near deserves a buzz: a real manoeuvre. Carrying
    /// straight on doesn't (apps say "continue" for long stretches), and
    /// off route the distance means nothing.
    fn worth_a_buzz(self) -> bool {
        !matches!(self, Self::Straight | Self::OffRoute)
    }
}

/// One `nav` message with something in it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Step {
    pub turn: Turn,
    pub street: String<STREET_LEN>,
    pub distance: String<DISTANCE_LEN>,
}

/// A distance as Gadgetbridge passes it on ("150 m", "1,2 km", "0.3 mi",
/// "500 ft", "150m"), in metres. `None` if it isn't one.
pub fn metres(distance: &str) -> Option<u32> {
    // One pass over the bytes, by hand rather than with `str::parse` and
    // `trim`, which cost far more flash: some locales write a decimal comma
    // ("1,2 km"), and thousandths are plenty. Spaces anywhere are skipped.
    let mut value: u64 = 0;
    let mut digits = 0;
    // Digits after the decimal point, once there is one.
    let mut decimals: Option<u32> = None;
    let mut unit = [0u8; 2];
    let mut unit_len = 0;
    for &byte in distance.as_bytes() {
        match byte {
            b'0'..=b'9' if unit_len == 0 => {
                digits += 1;
                if decimals.is_none_or(|d| d < 3) {
                    value = value.checked_mul(10)?.checked_add(u64::from(byte - b'0'))?;
                    decimals = decimals.map(|d| d + 1);
                }
            }
            b'.' | b',' if unit_len == 0 && decimals.is_none() => decimals = Some(0),
            b' ' => {}
            _ => {
                *unit.get_mut(unit_len)? = byte.to_ascii_lowercase();
                unit_len += 1;
            }
        }
    }
    if digits == 0 {
        return None;
    }
    let thousandths = value.checked_mul(10u64.pow(3 - decimals.unwrap_or(0)))?;
    // Millimetres per unit.
    let per_unit = match &unit[..unit_len] {
        b"m" => 1000,
        b"km" => 1_000_000,
        b"ft" => 305,
        b"yd" => 914,
        b"mi" => 1_609_344,
        _ => return None,
    };
    u32::try_from(thousandths.checked_mul(per_unit)? / 1_000_000).ok()
}

/// The directions being followed: the latest step, and what we've done
/// about it.
#[derive(Debug)]
pub struct Route {
    pub step: Step,
    /// The step's distance in metres, if it parsed.
    metres: Option<u32>,
    /// When the phone last sent a step.
    updated_ms: u64,
    /// Whether we've buzzed for this turn.
    warned: bool,
}

impl Route {
    /// Takes a `nav` message (`None` for the empty one: navigation ended)
    /// into `route`. Returns whether to buzz: the turn just came within
    /// [`NEAR_METRES`].
    pub fn update(route: &mut Option<Route>, step: Option<Step>, now_ms: u64) -> bool {
        let Some(step) = step else {
            *route = None;
            return false;
        };
        let metres = metres(&step.distance);
        // The same turn as before, getting nearer? Apps send a new step for
        // each update of the distance; a different manoeuvre or street is
        // the next turn. So is the distance jumping back up (two turns in a
        // row the same way onto streets of the same name, or a reroute),
        // with some slack for GPS jitter.
        let previous = route
            .as_ref()
            .filter(|r| r.step.turn == step.turn && r.step.street == step.street)
            .filter(|r| match (r.metres, metres) {
                (Some(before), Some(now)) => now <= before + 30,
                _ => true,
            });
        let warned = previous.is_some_and(|r| r.warned);
        let buzz = !warned && step.turn.worth_a_buzz() && metres.is_some_and(|m| m <= NEAR_METRES);
        *route = Some(Route {
            step,
            metres,
            updated_ms: now_ms,
            warned: warned || buzz,
        });
        buzz
    }

    /// Drops directions that have gone stale (see [`STALE_AFTER_MS`]).
    /// Returns whether it did.
    pub fn expire(route: &mut Option<Route>, now_ms: u64) -> bool {
        let stale = route
            .as_ref()
            .is_some_and(|r| now_ms.saturating_sub(r.updated_ms) >= STALE_AFTER_MS);
        if stale {
            *route = None;
        }
        stale
    }
}
