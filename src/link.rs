//! Keeping Gadgetbridge's link working: noticing a phone that talks to the
//! UART but doesn't listen to it, and when to tell it the battery.
//! Hardware-free, for its host tests (`host-tests/link`); [`crate::ble`]
//! feeds it what happens on each connection.
//!
//! ## A phone that doesn't listen
//!
//! Everything the watch says (its version, the battery, activity) reaches
//! Gadgetbridge as notifications on the UART's TX characteristic, which it
//! subscribes to as it connects. That subscription needs a paired link, and
//! when it's refused (the phone asked before the link was encrypted, as a
//! first pairing can), Gadgetbridge carries on as if it had worked: it
//! shows the watch connected, with no battery and no firmware version, and
//! never hears anything we send. Nothing short of reconnecting fixes it, and
//! nothing on the phone says to.
//!
//! So [`Watch`] gives each connection that reached for the UART
//! [`GRACE_MS`] (from then, or from when its link became authenticated, if
//! that's later) to be both subscribed and writing to it, as Gadgetbridge
//! always is once set up (it sends the time, and asks about the GPS, at
//! every connect). One that isn't is disconnected, once: Gadgetbridge
//! reconnects at once, and sets up afresh over a link that's now paired.
//!
//! "Reached for the UART" (wrote to it, or was refused a write to it or
//! to its subscription) is what keeps this to the phone. The watch's other
//! clients never touch the UART: a computer fetching the field log only
//! uses the log service, an updater only the DFU service; so they're
//! never judged, however long they stay. An update on the phone's own link
//! (Android shares one link among its apps) isn't cut short either.
//!
//! Once only, until a connection works: if the phone comes back and still
//! doesn't listen, reconnecting evidently doesn't help, and doing it again
//! would only cycle. That's the caller's flag (it outlives connections),
//! passed to [`Watch::check`].
//!
//! ## The battery
//!
//! [`StatusReports`]: Gadgetbridge hears the battery when the phone arrives,
//! when its percentage or charging changes, and every
//! [`STATUS_EVERY_MS`] regardless, so a report it missed is made up for
//! within the half hour rather than at the next change, which overnight on
//! the charger, or at 100%, can be hours away.

/// How long a connection that reached for the UART has, once its link is
/// authenticated, to be subscribed to it and writing to it. Gadgetbridge
/// does both within a second of connecting; a new pairing takes longer,
/// but this only starts counting once that's done.
pub const GRACE_MS: u64 = 10_000;

/// How often the battery is reported to the phone even when it hasn't
/// changed.
pub const STATUS_EVERY_MS: u64 = 30 * 60 * 1000;

/// What [`Watch::check`] finds of the link at its deadline.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Link {
    /// Encrypted with a passkey bond: what the UART needs.
    pub authenticated: bool,
    /// Subscribed to the UART's TX notifications.
    pub subscribed: bool,
    /// Has written to the UART (it's "the phone"; see `ble::serve`).
    pub talking: bool,
    /// A firmware update is under way.
    pub updating: bool,
}

/// What to do about a connection, from [`Watch::check`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// It listens and talks: the link works.
    Working,
    /// It doesn't listen (or talk): disconnect it, for it to reconnect.
    Reconnect,
    /// It doesn't, and the last such connection was already disconnected
    /// for it: leave it be (and say so).
    Broken,
    /// Nothing to judge yet: the link isn't authenticated (it can't
    /// subscribe until it is, and the user is told about that otherwise),
    /// or an update is under way.
    Wait,
}

/// One connection's watch over its UART; see the module docs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Watch {
    /// When to [`check`](Self::check), in milliseconds since boot.
    due: Option<u64>,
    /// Whether the client has reached for the UART.
    reached: bool,
    /// Whether it's been judged (or won't be): nothing more to do.
    done: bool,
}

impl Watch {
    /// The client wrote to the UART, or was refused a write to it or to its
    /// subscription. Starts the clock, the first time.
    pub fn reached_for_uart(&mut self, now: u64) {
        if !self.reached && !self.done {
            self.reached = true;
            self.due = Some(now + GRACE_MS);
        }
    }

    /// The link became authenticated (encrypted with a passkey bond): from
    /// now, a client that reached for the UART can subscribe, so it gets
    /// [`GRACE_MS`] from here.
    pub fn authenticated(&mut self, now: u64) {
        if self.reached && !self.done {
            self.due = Some(now + GRACE_MS);
        }
    }

    /// When to call [`check`](Self::check), if at all.
    pub fn due(&self) -> Option<u64> {
        self.due
    }

    /// Judges the link, at [`due`](Self::due). `reconnected` is whether the
    /// last connection judged was disconnected for this ([`Verdict::Reconnect`])
    /// and none has worked since.
    pub fn check(&mut self, link: Link, reconnected: bool) -> Verdict {
        self.due = None;
        if self.done {
            return Verdict::Wait;
        }
        // Not done: authenticating later sets a new deadline.
        if !link.authenticated {
            return Verdict::Wait;
        }
        self.done = true;
        if link.updating {
            Verdict::Wait
        } else if link.subscribed && link.talking {
            Verdict::Working
        } else if reconnected {
            Verdict::Broken
        } else {
            Verdict::Reconnect
        }
    }
}

/// When to report the battery to the phone; see the module docs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StatusReports {
    /// The last report's percentage and charging state, and when it was
    /// made (milliseconds since boot).
    last: Option<(u8, bool, u64)>,
}

impl StatusReports {
    /// Whether to report `percent` and `charging` now: the first time,
    /// when either changed, and [`STATUS_EVERY_MS`] after the last report.
    /// (Not the voltage: it jitters by millivolts from sample to sample.)
    pub fn due(&mut self, percent: u8, charging: bool, now: u64) -> bool {
        let due = match self.last {
            Some((p, c, at)) => {
                (p, c) != (percent, charging) || now.saturating_sub(at) >= STATUS_EVERY_MS
            }
            None => true,
        };
        if due {
            self.last = Some((percent, charging, now));
        }
        due
    }
}
