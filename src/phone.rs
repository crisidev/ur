//! What the phone tells the UI: notifications, calls, music, alarms,
//! directions, and trouble with the link.
//!
//! [`crate::gb`] decodes these from Gadgetbridge's messages, [`crate::ble`]
//! queues them for the UI loop, and [`crate::ui`] acts on them. They live
//! here, rather than in `ui.rs` where they're used, so that the protocol
//! code builds without the UI (and its canvas, fonts, and motor): in its
//! host tests (`host-tests/gb`), and in the simulator's replay of raw
//! Gadgetbridge lines. Hardware-free.

use heapless::String;

use crate::{alarm, route};

/// Things the phone tells the UI, via [`crate::ble`].
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum PhoneEvent {
    Notification {
        app: String<24>,
        who: String<40>,
    },
    IncomingCall {
        caller: String<40>,
    },
    /// The call was answered, rejected, or ended, on either side.
    CallOver,
    MusicInfo {
        artist: String<40>,
        track: String<40>,
    },
    MusicState {
        playing: bool,
    },
    /// "Find my watch" from the phone: ring until told to stop.
    Find {
        on: bool,
    },
    /// A client connected but never secured the link (see
    /// `ble::not_paired`). Shown like a notification.
    Unpaired(Unpaired),
    /// The phone sent text with Gadgetbridge's "Text as Bitmaps" images in
    /// it (see `gb::Images`), which we can't show: tell the wearer how to
    /// turn it off. Shown like a notification, once a connection.
    TextAsBitmaps,
    /// The alarms set in Gadgetbridge, all of them.
    Alarms(alarm::Phone),
    /// The next turn while the phone navigates; `None` once it stops.
    Route(Option<route::Step>),
    /// Gadgetbridge didn't pass a button grid press on, as its "Allow
    /// Intents" setting is off. Shown like a notification.
    IntentsOff,
}

/// Why a connected client can't use the watch: its link never became
/// encrypted.
#[derive(Clone, Copy, PartialEq, Eq, Debug, defmt::Format)]
pub enum Unpaired {
    /// Not a device we have a bond with: it has to pair.
    NotPaired,
    /// A device we have a bond with, which didn't use it: the phone forgot
    /// its side of the bond, and has to forget the watch and pair again.
    PairingLost,
}

impl Unpaired {
    pub fn title(self) -> &'static str {
        match self {
            Self::NotPaired => "NOT PAIRED",
            Self::PairingLost => "PAIRING LOST",
        }
    }

    /// What to do about it: README.md's FAQ entry, by its ID only, as the
    /// steps don't fit on the screen.
    pub fn hint(self) -> &'static str {
        match self {
            Self::NotPaired => "SEE FAQ C01",
            Self::PairingLost => "SEE FAQ C02",
        }
    }
}

/// Logged without contents: names and titles stay out of the logs.
impl defmt::Format for PhoneEvent {
    fn format(&self, f: defmt::Formatter) {
        let kind = match self {
            Self::Notification { .. } => "Notification",
            Self::IncomingCall { .. } => "IncomingCall",
            Self::CallOver => "CallOver",
            Self::MusicInfo { .. } => "MusicInfo",
            Self::MusicState { .. } => "MusicState",
            Self::Find { .. } => "Find",
            Self::Unpaired(_) => "Unpaired",
            Self::TextAsBitmaps => "TextAsBitmaps",
            Self::Alarms(_) => "Alarms",
            Self::Route(_) => "Route",
            Self::IntentsOff => "IntentsOff",
        };
        defmt::write!(f, "{=str}", kind)
    }
}
