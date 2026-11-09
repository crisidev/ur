//! Host tests for the firmware's `decimal` module: Gadgetbridge's numbers,
//! parsed without core's float tables or `f64`. `cargo test` runs them.

#[path = "../../../src/decimal.rs"]
#[allow(dead_code)]
mod decimal;

fn main() {
    println!("run `cargo test`");
}

#[cfg(test)]
mod tests {
    use super::decimal::{Decimal, parse};
    use quickcheck::{QuickCheck, TestResult};

    fn parsed(text: &str) -> Option<Decimal> {
        parse(text.as_bytes())
    }

    fn scaled(text: &str, places: i32) -> i64 {
        parsed(text).unwrap().scaled(places)
    }

    /// What Gadgetbridge sends: coordinates, time zones, timestamps, alarm
    /// fields, all within the digits an `f32` holds, so exactly as
    /// `str::parse` reads them.
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
            let want: f32 = text.parse().unwrap();
            assert_eq!(parsed(text).unwrap().to_f32(), want, "{text}");
        }
    }

    /// Coordinates in millionths of a degree, and timestamps, exactly.
    #[test]
    fn scaled_exactly() {
        assert_eq!(scaled("59.9139", 6), 59_913_900);
        assert_eq!(scaled("-33.86882", 6), -33_868_820);
        assert_eq!(scaled("-1.0E-4", 6), -100);
        assert_eq!(scaled("1762436800000", 0), 1_762_436_800_000);
        assert_eq!(scaled("1.7624368E12", 0), 1_762_436_800_000);
        // Past the millionths (17 significant digits, as Java prints some
        // doubles): toward zero.
        assert_eq!(scaled("59.913868800000006", 6), 59_913_868);
        assert_eq!(scaled("-0.12775829999999999", 6), -127_758);
        // Alarm fields: toward zero, too.
        assert_eq!(scaled("7.9", 0), 7);
        assert_eq!(scaled("-7.9", 0), -7);
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
        assert_eq!(parsed(".5").unwrap().to_f32(), 0.5);
        assert_eq!(parsed("5.").unwrap().to_f32(), 5.0);
        assert_eq!(parsed("+5").unwrap().to_f32(), 5.0);
    }

    #[test]
    fn extremes() {
        assert_eq!(parsed("0e400").unwrap().to_f32(), 0.0);
        assert_eq!(parsed("1e400").unwrap().to_f32(), f32::INFINITY);
        assert_eq!(parsed("-1e400").unwrap().to_f32(), f32::NEG_INFINITY);
        assert_eq!(parsed("1e-400").unwrap().to_f32(), 0.0);
        assert_eq!(parsed("1e99999999999").unwrap().to_f32(), f32::INFINITY);
        assert_eq!(scaled("1e400", 0), i64::MAX);
        assert_eq!(scaled("-1e99999999999", 6), -i64::MAX);
        assert_eq!(scaled("1e-400", 6), 0);
        assert_eq!(scaled("0e400", 0), 0);
        // More digits than a u64 holds: the extra ones are dropped, before
        // the point keeping their place.
        assert_eq!(scaled("123456789012345678901234", -10), 12_345_678_901_234);
        assert_eq!(
            scaled("0.1234567890123456789012345", 18),
            123_456_789_012_345_678
        );
    }

    /// `scaled` is the exact value, toward zero, wherever it fits.
    #[test]
    fn scaled_is_exact() {
        fn property(mantissa: i64, exponent: i8, places: u8) -> TestResult {
            let mantissa = mantissa % 1_000_000_000_000_000;
            let exponent = i32::from(exponent) % 20;
            let places = i32::from(places % 10);
            let text = format!("{mantissa}e{exponent}");
            let shift = exponent + places;
            let want = if shift >= 0 {
                match 10i128
                    .checked_pow(shift as u32)
                    .and_then(|p| p.checked_mul(i128::from(mantissa)))
                {
                    Some(want) if want.unsigned_abs() <= i64::MAX as u128 => want as i64,
                    _ => return TestResult::discard(),
                }
            } else {
                // Toward zero, as Rust's integer division is.
                (i128::from(mantissa) / 10i128.pow(shift.unsigned_abs())) as i64
            };
            TestResult::from_bool(scaled(&text, places) == want)
        }
        QuickCheck::new()
            .tests(100_000)
            .quickcheck(property as fn(i64, i8, u8) -> TestResult);
    }

    /// Anything as `{}` prints an `f32`, however long or far from 1: within
    /// two ulps of `str::parse`'s, wherever the value is a normal `f32`
    /// (scaling by a power of ten past 10^38 tips over into infinity).
    #[test]
    fn close_to_f32_everywhere() {
        fn property(value: f32) -> TestResult {
            if !value.is_normal() || !(1e-30..1e30).contains(&value.abs()) {
                return TestResult::discard();
            }
            for text in [
                format!("{value}"),
                format!("{value:e}"),
                format!("{value:.3}"),
            ] {
                let want: f32 = text.parse().unwrap();
                let got = parsed(&text).unwrap().to_f32();
                if want != 0.0 && (got.to_bits() as i64 - want.to_bits() as i64).abs() > 2 {
                    println!("{text}: {got} vs {want}");
                    return TestResult::failed();
                }
            }
            TestResult::passed()
        }
        QuickCheck::new()
            .tests(100_000)
            .quickcheck(property as fn(f32) -> TestResult);
    }
}
