//! Host tests for the firmware's `unpaired` module: how often the wearer
//! hears about clients that don't pair, and how long such a client may
//! stay connected. `cargo test` runs them.

#[path = "../../../src/unpaired.rs"]
#[allow(dead_code)]
mod unpaired;

fn main() {
    println!("run `cargo test`");
}

#[cfg(test)]
mod tests {
    use super::unpaired::*;

    const SECOND: u64 = 1000;
    const MINUTE: u64 = 60 * SECOND;
    /// `phone::Unpaired`'s kinds, as `ui` passes them.
    const NOT_PAIRED: usize = 0;
    const PAIRING_LOST: usize = 1;

    // ==========================================================================
    // The banner
    // ==========================================================================

    #[test]
    fn the_first_banner_shows_even_at_boot() {
        assert!(Banners::default().due(NOT_PAIRED, 0));
        assert!(Banners::default().due(PAIRING_LOST, 1));
    }

    /// A stranger reconnecting every ~45 s (as [`Stay`] cuts it off) for
    /// an hour gets a banner every five minutes, and never sooner.
    #[test]
    fn a_reconnecting_client_is_shown_every_five_minutes() {
        let mut banners = Banners::default();
        let start = 3 * 60 * MINUTE + 123;
        let mut shown = Vec::new();
        let mut now = start;
        while now < start + 60 * MINUTE {
            if banners.due(NOT_PAIRED, now) {
                shown.push(now);
            }
            now += 45 * SECOND + 300;
        }
        assert_eq!(shown.len(), 12, "{shown:?}");
        for pair in shown.windows(2) {
            let gap = pair[1] - pair[0];
            assert!(
                (BANNER_EVERY_MS..BANNER_EVERY_MS + 46 * SECOND).contains(&gap),
                "{gap} ms between banners"
            );
        }
    }

    /// Never early, whatever the milliseconds: the seconds kept are
    /// rounded to wait at least the whole interval.
    #[test]
    fn the_interval_is_never_cut_short() {
        for first in [0, 1, 999, 1000, 1001, 59_999, 123_456_789] {
            let mut banners = Banners::default();
            assert!(banners.due(NOT_PAIRED, first));
            assert!(!banners.due(NOT_PAIRED, first + BANNER_EVERY_MS - 1));
            assert!(banners.due(NOT_PAIRED, first + BANNER_EVERY_MS + 999));
        }
    }

    /// The wearer's own phone saying PAIRING LOST isn't hidden by a
    /// stranger's NOT PAIRED a moment before, nor the other way round;
    /// and a refused banner doesn't push its own next one further away.
    #[test]
    fn each_kind_keeps_its_own_time() {
        let mut banners = Banners::default();
        assert!(banners.due(NOT_PAIRED, MINUTE));
        assert!(banners.due(PAIRING_LOST, MINUTE + SECOND));
        assert!(!banners.due(NOT_PAIRED, 2 * MINUTE));
        assert!(!banners.due(PAIRING_LOST, 2 * MINUTE));
        assert!(!banners.due(NOT_PAIRED, 5 * MINUTE));
        assert!(banners.due(NOT_PAIRED, 6 * MINUTE));
        assert!(!banners.due(PAIRING_LOST, 6 * MINUTE));
        assert!(banners.due(PAIRING_LOST, 6 * MINUTE + 2 * SECOND));
    }

    // ==========================================================================
    // Staying
    // ==========================================================================

    #[test]
    fn an_authenticated_link_stays() {
        let mut stay = Stay::new(5 * SECOND);
        assert_eq!(stay.cut_off_at(true), None);
        stay.pairing_started(6 * SECOND);
        assert_eq!(stay.cut_off_at(true), None);
        stay.pairing_ended(20 * SECOND);
        assert_eq!(stay.cut_off_at(true), None);
    }

    /// One that never pairs goes once the SMP timeout (30 s from our
    /// Security Request, sent as it connects) has passed, and a pairing
    /// started just before it could have shown its passkey.
    #[test]
    fn a_client_that_never_pairs_goes_after_the_grace() {
        let connected = 12 * MINUTE + 345;
        let stay = Stay::new(connected);
        let at = stay.cut_off_at(false).expect("a deadline");
        assert!(at >= connected + GRACE_MS, "{at} is early");
        assert!(at < connected + GRACE_MS + SECOND, "{at} is late");
    }

    /// The wearer pairing their phone: the passkey shows a second after
    /// connecting, and they take Android's whole 30 s to type it. Never
    /// cut off along the way; the deadline moves to the pairing's own.
    #[test]
    fn a_slow_first_pairing_is_not_cut_short() {
        let connected = 7 * SECOND + 500;
        let mut stay = Stay::new(connected);
        let shown = connected + SECOND;
        stay.pairing_started(shown);
        assert!(stay.pairing());
        let at = stay.cut_off_at(false).expect("a deadline while pairing");
        assert!(at >= shown + PAIRING_MS);
        // Typing ends Android's 30 s later, the rest takes seconds.
        assert!(at > shown + 30 * SECOND + 10 * SECOND);
        // Done: authenticated, and staying.
        stay.pairing_ended(shown + 35 * SECOND);
        assert!(!stay.pairing());
        assert_eq!(stay.cut_off_at(true), None);
    }

    /// A pairing that started at the last moment (just before the SMP
    /// timeout) still reaches its passkey before the grace runs out.
    #[test]
    fn a_pairing_started_late_reaches_its_passkey() {
        let connected = 100 * SECOND + 999;
        let mut stay = Stay::new(connected);
        let passkey = connected + 30 * SECOND + 2 * SECOND;
        assert!(stay.cut_off_at(false).expect("a deadline") > passkey);
        stay.pairing_started(passkey);
        assert!(stay.cut_off_at(false).expect("a deadline") >= passkey + PAIRING_MS);
    }

    /// A pairing that failed (a mistyped passkey) leaves the client a
    /// fresh grace to try again on the same link, then it goes.
    #[test]
    fn a_failed_pairing_gets_a_fresh_grace() {
        let mut stay = Stay::new(0);
        stay.pairing_started(2 * SECOND);
        let failed = 20 * SECOND + 1;
        stay.pairing_ended(failed);
        let at = stay.cut_off_at(false).expect("a deadline");
        assert!((failed + GRACE_MS..failed + GRACE_MS + SECOND).contains(&at));
    }

    /// A pairing whose passkey nobody types (or that the SMP timer ended
    /// silently) can't hold the slot, nor keep the passkey up, forever.
    #[test]
    fn a_stalled_pairing_is_cut_off() {
        let mut stay = Stay::new(0);
        stay.pairing_started(3 * SECOND);
        let at = stay.cut_off_at(false).expect("a deadline");
        assert!(at <= 3 * SECOND + PAIRING_MS + SECOND);
    }

    /// `check` says to disconnect at the deadline, not before, and once
    /// only: the link lingers while the disconnection goes through, and
    /// mustn't be disconnected (and logged) again at every turn. A failure
    /// reported meanwhile doesn't rearm it, and still counts as a pairing.
    #[test]
    fn the_cut_off_comes_once() {
        let mut stay = Stay::new(0);
        stay.pairing_started(SECOND);
        let at = stay.cut_off_at(false).expect("a deadline");
        assert!(!stay.check(false, at - 1));
        assert!(!stay.check(true, at));
        assert!(stay.check(false, at));
        assert!(!stay.check(false, at));
        assert!(stay.pairing());
        assert_eq!(stay.cut_off_at(false), None);
        stay.pairing_ended(at + 100);
        assert_eq!(stay.cut_off_at(false), None);
        assert!(!stay.check(false, at + 10 * MINUTE));
    }

    /// Decades after boot, nothing overflows.
    #[test]
    fn far_from_boot() {
        let late = u64::from(u32::MAX / 2) * 1000;
        let stay = Stay::new(late);
        assert!(stay.cut_off_at(false).expect("a deadline") > late);
        let mut banners = Banners::default();
        assert!(banners.due(NOT_PAIRED, late));
        assert!(!banners.due(NOT_PAIRED, late + MINUTE));
    }
}
