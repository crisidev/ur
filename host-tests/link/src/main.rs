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
