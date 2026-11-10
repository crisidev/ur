//! Host tests for the firmware's `link` module: noticing a Gadgetbridge
//! that doesn't listen to the UART, and when to report the battery.
//! `cargo test` runs them.

// Used only by the tests.
#[path = "../../../src/link.rs"]
#[allow(dead_code)]
mod link;

fn main() {
    println!("run `cargo test`");
}

#[cfg(test)]
mod tests {
    use super::link::*;

    /// Gadgetbridge set up as it should be.
    const WORKING: Link = Link {
        authenticated: true,
        subscribed: true,
        talking: true,
        updating: false,
    };

    /// Runs `watch` to its deadline, if it has one, and checks there.
    fn check_at_due(watch: &mut Watch, link: Link, reconnected: bool) -> (u64, Verdict) {
        let due = watch.due().expect("a deadline");
        (due, watch.check(link, reconnected))
    }

    /// A bonded phone: encrypted at once, then subscribed and writing.
    #[test]
    fn a_working_phone_is_left_alone() {
        let mut watch = Watch::default();
        watch.authenticated(300);
        assert_eq!(watch.due(), None, "nothing reached for the UART yet");
        watch.reached_for_uart(600);
        assert_eq!(
            check_at_due(&mut watch, WORKING, false),
            (600 + GRACE_MS, Verdict::Working)
        );
        assert_eq!(watch.due(), None);
    }

    /// The case this is for: the subscription was refused before pairing
    /// finished, and Gadgetbridge went on regardless.
    #[test]
    fn a_phone_that_doesnt_listen_is_reconnected_once() {
        let deaf = Link {
            subscribed: false,
            ..WORKING
        };
        let mut watch = Watch::default();
        watch.reached_for_uart(500);
        // The passkey takes a while to type: nothing to judge before then.
        let unpaired = Link {
            authenticated: false,
            subscribed: false,
            talking: false,
            updating: false,
        };
        assert_eq!(
            check_at_due(&mut watch, unpaired, false),
            (500 + GRACE_MS, Verdict::Wait)
        );
        assert_eq!(watch.due(), None);
        watch.authenticated(20_000);
        assert_eq!(
            check_at_due(&mut watch, deaf, false),
            (20_000 + GRACE_MS, Verdict::Reconnect)
        );
        // Once per connection.
        assert_eq!(watch.due(), None);
        watch.authenticated(40_000);
        watch.reached_for_uart(40_000);
        assert_eq!(watch.due(), None);

        // The next connection is no better: leave it be.
        let mut next = Watch::default();
        next.reached_for_uart(0);
        assert_eq!(check_at_due(&mut next, deaf, true).1, Verdict::Broken);
    }

    /// Writes refused before pairing, and none since: Gadgetbridge sends
    /// the time once, at connect, and then nothing until there's news.
    #[test]
    fn a_phone_that_doesnt_talk_is_reconnected_too() {
        let mut watch = Watch::default();
        watch.reached_for_uart(0);
        watch.authenticated(15_000);
        let silent = Link {
            talking: false,
            ..WORKING
        };
        assert_eq!(
            check_at_due(&mut watch, silent, false).1,
            Verdict::Reconnect
        );
    }

    /// A computer fetching the field log, or an updater: bonded, never
    /// subscribed to the UART, never writing to it.
    #[test]
    fn other_clients_are_never_judged() {
        let mut watch = Watch::default();
        watch.authenticated(0);
        watch.authenticated(60_000);
        assert_eq!(watch.due(), None);
    }

    /// An update on the phone's own link (another app on the phone).
    #[test]
    fn an_update_is_not_cut_short() {
        let mut watch = Watch::default();
        watch.reached_for_uart(0);
        let updating = Link {
            subscribed: false,
            updating: true,
            ..WORKING
        };
        assert_eq!(check_at_due(&mut watch, updating, false).1, Verdict::Wait);
        watch.authenticated(30_000);
        assert_eq!(watch.due(), None, "not judged later either");
    }

    /// Authenticating again (or late) only ever moves the deadline later.
    #[test]
    fn the_grace_counts_from_the_later_of_both() {
        let mut watch = Watch::default();
        watch.authenticated(0);
        watch.reached_for_uart(2_000);
        assert_eq!(watch.due(), Some(2_000 + GRACE_MS));
        watch.authenticated(5_000);
        assert_eq!(watch.due(), Some(5_000 + GRACE_MS));
        watch.reached_for_uart(7_000);
        assert_eq!(watch.due(), Some(5_000 + GRACE_MS), "only the first write");
    }

    #[test]
    fn the_battery_is_reported_on_change_and_every_half_hour() {
        let mut reports = StatusReports::default();
        assert!(reports.due(80, false, 1_000), "the first time");
        assert!(!reports.due(80, false, 61_000));
        assert!(reports.due(79, false, 121_000), "a new percentage");
        assert!(reports.due(79, true, 181_000), "charging");
        assert!(!reports.due(79, true, 181_000 + STATUS_EVERY_MS - 1));
        assert!(reports.due(79, true, 181_000 + STATUS_EVERY_MS));
        assert!(!reports.due(79, true, 181_000 + STATUS_EVERY_MS + 60_000));
    }
}

/// Kani proofs (`cargo kani` here, one at a time with `--harness`): what
/// the tests above check for chosen connections, for every order of what
/// can happen on one, at any times. Nothing panics or overflows; a
/// connection is judged at most once, a grace period after it reached
/// for the UART or was authenticated, and never if it didn't reach; it's
/// disconnected only for not listening on a link that could; and the
/// battery is reported when it's first known, when it changes, and every
/// half hour.
///
/// Times are milliseconds since boot, under 2^63 (300 million years):
/// the deadline is a grace period after one, added unchecked.
#[cfg(kani)]
mod proofs {
    use super::link::*;

    fn any_time(after: u64) -> u64 {
        let now: u64 = kani::any();
        kani::assume(after <= now && now < 1 << 63);
        now
    }

    fn any_link() -> Link {
        Link {
            authenticated: kani::any(),
            subscribed: kani::any(),
            talking: kani::any(),
            updating: kani::any(),
        }
    }

    /// One connection, through any six happenings in any order: the
    /// client reaching for the UART, the link becoming authenticated, or
    /// the deadline (if there is one) coming and the link being judged
    /// as it then is.
    ///
    /// - There's a deadline only once the client has reached for the
    ///   UART, and it's `GRACE_MS` after it first did, or after the link
    ///   was authenticated, if that's later.
    /// - A verdict other than `Wait` comes at most once, and it and an
    ///   update each end the watch: no deadline after.
    /// - `Working` means authenticated, subscribed and talking;
    ///   `Reconnect` and `Broken`, authenticated but not both of those;
    ///   and none of them comes during an update.
    /// - `Reconnect` comes only if the last connection wasn't
    ///   disconnected for the same, and `Broken` only if it was: no
    ///   cycle of reconnecting.
    #[kani::proof]
    #[kani::unwind(7)]
    fn a_connection_is_judged_once_after_its_grace() {
        let mut watch = Watch::default();
        let reconnected: bool = kani::any();
        let mut now = 0;
        // When it first reached for the UART, the deadline that gives,
        // and whether it's been judged for good.
        let (mut reached, mut deadline, mut judged) = (false, None, false);
        assert_eq!(watch.due(), None);
        for _ in 0..6 {
            now = any_time(now);
            match kani::any::<u8>() % 3 {
                0 => {
                    watch.reached_for_uart(now);
                    if !reached && !judged {
                        deadline = Some(now + GRACE_MS);
                    }
                    reached = true;
                }
                1 => {
                    watch.authenticated(now);
                    if reached && !judged {
                        deadline = Some(now + GRACE_MS);
                    }
                }
                _ => {
                    // `ble` only judges at the deadline.
                    kani::assume(watch.due().is_some_and(|due| due <= now));
                    let link = any_link();
                    let verdict = watch.check(link, reconnected);
                    deadline = None;
                    assert!(!judged);
                    let listens = link.subscribed && link.talking;
                    match verdict {
                        Verdict::Wait => assert!(!link.authenticated || link.updating),
                        Verdict::Working => assert!(listens),
                        Verdict::Reconnect => assert!(!listens && !reconnected),
                        Verdict::Broken => assert!(!listens && reconnected),
                    }
                    if link.authenticated {
                        assert_eq!(verdict == Verdict::Wait, link.updating);
                        judged = true;
                    }
                }
            }
            assert_eq!(watch.due(), deadline);
            assert!(reached || watch.due().is_none());
        }
    }

    /// Through any four looks at the battery, at any times (not going
    /// back): it's reported the first time, whenever the percentage or
    /// charging differs from the last report, and once `STATUS_EVERY_MS`
    /// has passed since the last report; and not otherwise. So an
    /// unchanged battery is reported at most every half hour, and a
    /// report the phone missed is made up for by the next one, within
    /// the half hour.
    #[kani::proof]
    #[kani::unwind(5)]
    fn the_battery_is_reported_on_change_and_every_half_hour() {
        let mut reports = StatusReports::default();
        let mut now = 0;
        let mut reported: Option<(u8, bool, u64)> = None;
        for _ in 0..4 {
            now = any_time(now);
            let (percent, charging): (u8, bool) = kani::any();
            let due = reports.due(percent, charging, now);
            let expected = match reported {
                None => true,
                Some((p, c, at)) => (p, c) != (percent, charging) || now - at >= STATUS_EVERY_MS,
            };
            assert_eq!(due, expected);
            if due {
                reported = Some((percent, charging, now));
            }
        }
    }
}
