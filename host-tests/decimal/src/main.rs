//! Host tests for the firmware's `decimal` module: Gadgetbridge's numbers,
//! parsed without core's float tables. `cargo test` runs them.

#[path = "../../../src/decimal.rs"]
#[allow(dead_code)]
mod decimal;

fn main() {
    println!("run `cargo test`");
}

#[cfg(test)]
mod tests {
    use super::decimal::parse;
    use quickcheck::{QuickCheck, TestResult};

    fn parsed(text: &str) -> Option<f64> {
        parse(text.as_bytes())
    }

    /// What Gadgetbridge sends: coordinates, time zones, timestamps, alarm
    /// fields, all exactly as `str::parse` reads them.
    #[test]
    fn gadgetbridge_numbers() {
        for text in [
            "59.9139",
            "10.7522",
            "-33.86882",
            "151.20929",
            "1.0",
            "5.75",
            "-3.5",
            "0",
            "0.0",
            "-0.0",
            "7",
            "31",
            "1762436800",
            "1762436800000",
            "1.0E-4",
            "-2.5e3",
            "1E7",
            "1e+2",
        ] {
            assert_eq!(parsed(text), Some(text.parse().unwrap()), "{text}");
        }
    }

    /// Past the fast path (here 17 significant digits, as Java prints some
    /// doubles): off by at most an ulp.
    #[test]
    fn long_coordinates() {
        for text in [
            "59.913868800000006",
            "-0.12775829999999999",
            "151.20929000000001",
        ] {
            let want: f64 = text.parse().unwrap();
            let got = parsed(text).unwrap();
            assert!(
                (got.to_bits() as i64 - want.to_bits() as i64).abs() <= 1,
                "{text}: {got}"
            );
        }
    }

    #[test]
    fn rejects_what_is_not_a_number() {
        for text in [
            "", "-", "+", ".", "-.", "e5", "1e", "1e+", "1.2.3", "1,5", "NaN", "inf", "\"1\"",
            "1 ", " 1", "true", "1x",
        ] {
            assert_eq!(parsed(text), None, "{text:?}");
        }
    }

    /// Forms `str::parse` takes and JSON doesn't: still fine.
    #[test]
    fn lenient_forms() {
        assert_eq!(parsed(".5"), Some(0.5));
        assert_eq!(parsed("5."), Some(5.0));
        assert_eq!(parsed("+5"), Some(5.0));
    }

    #[test]
    fn extremes() {
        assert_eq!(parsed("0e400"), Some(0.0));
        assert_eq!(parsed("1e400"), Some(f64::INFINITY));
        assert_eq!(parsed("-1e400"), Some(f64::NEG_INFINITY));
        assert_eq!(parsed("1e-400"), Some(0.0));
        assert_eq!(parsed("1e99999999999"), Some(f64::INFINITY));
        // More digits than a u64 holds: the extra ones are dropped, before
        // the point keeping their place.
        let close = |text: &str, want: f64| {
            let got = parsed(text).unwrap();
            assert!(((got - want) / want).abs() < 1e-15, "{text}: {got}");
        };
        close("123456789012345678901234", 1.2345678901234568e23);
        close("0.1234567890123456789012345", 0.12345678901234568);
    }

    /// Within the fast path (≤ 15 significant digits, exponent ≤ 22), exactly
    /// `str::parse`'s correctly rounded answer.
    #[test]
    fn exact_in_the_fast_path() {
        fn property(mantissa: i64, exponent: i8) -> TestResult {
            let mantissa = mantissa % 1_000_000_000_000_000;
            let exponent = i32::from(exponent) % 23;
            let text = format!("{mantissa}e{exponent}");
            let want: f64 = text.parse().unwrap();
            TestResult::from_bool(parsed(&text) == Some(want))
        }
        QuickCheck::new()
            .tests(100_000)
            .quickcheck(property as fn(i64, i8) -> TestResult);
    }

    /// Anything as `{}` prints it, however long or far from 1: to 14
    /// significant digits of `str::parse`'s (the error grows with the
    /// exponent, from the inexact powers of ten past 10^22). Except right
    /// at the ends of the range, where that error tips over into infinity
    /// or zero.
    #[test]
    fn close_everywhere() {
        fn property(value: f64) -> TestResult {
            if !value.is_finite() || (value != 0.0 && !(1e-290..1e290).contains(&value.abs())) {
                return TestResult::discard();
            }
            for text in [
                format!("{value}"),
                format!("{value:e}"),
                format!("{:.7}", value),
            ] {
                let want: f64 = text.parse().unwrap();
                let got = parsed(&text).unwrap();
                if want != got && ((got - want) / want).abs() > 1e-14 {
                    println!("{text}: {got} vs {want}");
                    return TestResult::failed();
                }
            }
            TestResult::passed()
        }
        QuickCheck::new()
            .tests(100_000)
            .quickcheck(property as fn(f64) -> TestResult);
    }
}
