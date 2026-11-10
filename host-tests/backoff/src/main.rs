//! Host tests for the firmware's `backoff` module. `cargo test` runs them.

#[path = "../../../src/backoff.rs"]
#[allow(dead_code)]
mod backoff;

fn main() {
    println!("run `cargo test`");
}

#[cfg(test)]
mod tests {
    use super::backoff::*;

    #[test]
    fn waits_grow_then_hold() {
        let mut backoff = Backoff::new();
        let waits: Vec<u32> = (0..11).map(|_| backoff.failed().wait_ms).collect();
        assert_eq!(
            waits,
            [100, 200, 500, 1_000, 2_000, 5_000, 10_000, 30_000, 60_000, 60_000, 60_000]
        );
    }

    /// A bus wedged for a few seconds (five failures, as the dev kit's
    /// `wedge-i2c` causes) costs a short wait once it's fine again, not
    /// half a minute: the waits only reach 30 s after ~20 s of failures.
    #[test]
    fn short_wedge_short_waits() {
        let mut backoff = Backoff::new();
        let mut elapsed_ms = 0;
        let mut wait_ms = 0;
        while elapsed_ms < 5_000 {
            wait_ms = backoff.failed().wait_ms;
            elapsed_ms += wait_ms;
        }
        assert!(wait_ms <= 5_000, "{wait_ms}");
        let before_30s: u32 = WAITS_MS.iter().take_while(|&&w| w < 30_000).sum();
        assert!(before_30s >= 15_000, "{before_30s}");
    }

    /// The first retry is as quick as the old fixed 100 ms retry, which
    /// fixed most setup failures.
    #[test]
    fn first_retry_is_quick() {
        assert_eq!(Backoff::new().failed().wait_ms, 100);
    }

    /// Never gives up: the sealed watch must get its sensors back without
    /// a reboot, however long the bus stays stuck.
    #[test]
    fn never_gives_up() {
        let mut backoff = Backoff::new();
        for _ in 0..100_000 {
            let retry = backoff.failed();
            assert!(retry.wait_ms <= 60_000);
            assert!(retry.wait_ms >= 100);
        }
        assert_eq!(backoff.failed().failures, u16::MAX);
        assert_eq!(backoff.failed().wait_ms, 60_000);
    }

    /// Every failure while the waits grow is logged, then about one an
    /// hour: a dead sensor costs a line an hour, not one a minute.
    #[test]
    fn logs_compactly() {
        let mut backoff = Backoff::new();
        let mut logged = Vec::new();
        let mut elapsed_ms = 0u64;
        // A day of failures.
        while elapsed_ms < 24 * 3_600_000 {
            let retry = backoff.failed();
            if retry.log {
                logged.push(retry.failures);
            }
            elapsed_ms += u64::from(retry.wait_ms);
        }
        assert_eq!(&logged[..WAITS_MS.len()], &[1, 2, 3, 4, 5, 6, 7, 8, 9]);
        assert_eq!(&logged[WAITS_MS.len()..][..2], &[60, 120]);
        // Nine at the start, then one an hour.
        assert!((9 + 23..=9 + 25).contains(&logged.len()), "{logged:?}");
    }

    /// `loud` says beforehand what `failed` then decides, so a driver can
    /// keep its own transfer warnings in step with the retry lines.
    #[test]
    fn loud_predicts_failed() {
        let mut backoff = Backoff::new();
        for _ in 0..500 {
            let loud = backoff.loud();
            assert_eq!(backoff.failed().log, loud);
        }
    }

    #[test]
    fn worth_logging_thins_out() {
        let logged: Vec<u16> = (1..=1000).filter(|&n| worth_logging(n)).collect();
        assert_eq!(logged, [1, 2, 4, 8, 16, 32, 64, 128, 256, 512]);
        assert!(worth_logging(u16::MAX / 2 + 1));
        assert!(!worth_logging(u16::MAX));
    }
}
