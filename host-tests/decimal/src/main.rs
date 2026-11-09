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

/// Proofs about every short number there is, where the tests above try
/// chosen and random ones. `cargo kani` runs them (one at a time with
/// `--harness`: CBMC takes memory).
#[cfg(kani)]
mod proofs {
    use super::decimal::parse;

    /// The longest text the proofs of any bytes take: enough for a sign,
    /// a point, and an exponent of its own sign and digits at once
    /// (`-1.5e-30`), and for an exponent far past any number
    /// (`1e999999`).
    const LONGEST: usize = 8;

    /// Any bytes, up to [`LONGEST`] of them.
    fn any_text() -> ([u8; LONGEST], usize) {
        let text: [u8; LONGEST] = kani::any();
        let len: usize = kani::any();
        kani::assume(len <= LONGEST);
        (text, len)
    }

    /// Whether `text` is `[+-]digits[.digits][(e|E)[+-]digits]`, with
    /// digits on at least one side of the point: `parse`'s docs, read
    /// off one part at a time.
    fn is_a_number(text: &[u8]) -> bool {
        let digits = |from: usize| {
            let mut to = from;
            while to < text.len() && text[to].is_ascii_digit() {
                to += 1;
            }
            to
        };
        let is = |at: usize, a: u8, b: u8| at < text.len() && (text[at] == a || text[at] == b);

        let mut at = usize::from(is(0, b'+', b'-'));
        let whole = digits(at) - at;
        at += whole;
        let mut fraction = 0;
        if is(at, b'.', b'.') {
            at += 1;
            fraction = digits(at) - at;
            at += fraction;
        }
        if whole + fraction == 0 {
            return false;
        }
        if is(at, b'e', b'E') {
            at += 1;
            at += usize::from(is(at, b'+', b'-'));
            let exponent = digits(at) - at;
            if exponent == 0 {
                return false;
            }
            at += exponent;
        }
        at == text.len()
    }

    /// Any eight bytes or fewer: nothing panics or overflows, parsing
    /// them or scaling what they say by any power of ten at all; they're
    /// a number exactly when they're written as `parse`'s docs say; and
    /// the number has the sign it was written with (so a coordinate can't
    /// change hemisphere, however it saturates).
    #[kani::proof]
    #[kani::unwind(21)]
    fn any_short_text_is_a_number_or_not() {
        let (text, len) = any_text();
        let text = &text[..len];
        let parsed = parse(text);
        assert_eq!(parsed.is_some(), is_a_number(text));
        if let Some(number) = parsed {
            let scaled = number.scaled(kani::any());
            if text[0] == b'-' {
                assert!(scaled <= 0);
            } else {
                assert!(scaled >= 0);
            }
            kani::cover!(scaled == i64::MAX, "saturated");
            kani::cover!(scaled == -i64::MAX, "saturated, negative");
        }
    }

    /// The same bytes as an `f32`: never NaN, whatever the exponent
    /// (`0e99`, zero times infinity, is the one that would be), and with
    /// the sign it was written with. `gb::parse_set_time` rounds one of
    /// these to minutes, and compares it.
    ///
    /// But for negative exponents of two digits or more (`1e-45`): from
    /// 39, the power of ten `to_f32` divides by is infinite, as an `f32`,
    /// which gives the zero it means to, but CBMC's own NaN check takes
    /// any division by infinity for one (its `--nan-check`, which Kani
    /// always asks for; only infinity by infinity is), and fails the
    /// proof on it whatever is asserted here. The tests cover those
    /// (`extremes`).
    #[kani::proof]
    #[kani::unwind(71)]
    fn any_short_text_is_never_nan() {
        let (text, len) = any_text();
        let text = &text[..len];
        for at in 0..LONGEST - 3 {
            let tiny = at + 3 < len && (text[at] | 0x20) == b'e' && text[at + 1] == b'-';
            kani::assume(!tiny);
        }
        if let Some(number) = parse(text) {
            let float = number.to_f32();
            assert!(!float.is_nan());
            assert_eq!(float.is_sign_negative(), text[0] == b'-');
            kani::cover!(float.is_infinite(), "infinite");
            kani::cover!(text[len - 1] == b'.', "ends in a point");
        }
    }

    /// A number written out, any of the shape asked for: a `-` or not,
    /// `whole` digits (any, so shorter numbers come with zeros in
    /// front), then, with `fraction`, a point and that many more, then,
    /// with `exponent`, an `E` and it. Returns its text, whether it's
    /// negative, and its digits as one integer.
    ///
    /// The shape is the caller's to fix, and only the digits are left
    /// open: with their number, where the point goes, and the exponent
    /// open too, CBMC has every byte of the text to tell apart at every
    /// step, and a power of ten it doesn't know to multiply by, and
    /// didn't finish.
    fn any_number(
        before: usize,
        fraction: usize,
        exponent: Option<i8>,
    ) -> ([u8; 24], usize, bool, i64) {
        let mut text = [0u8; 24];
        let mut len = 0;
        let mut write = |byte: u8| {
            text[len] = byte;
            len += 1;
        };
        let negative: bool = kani::any();
        if negative {
            write(b'-');
        }
        let mut digits: i64 = 0;
        for place in 0..before + fraction {
            if place == before {
                write(b'.');
            }
            let digit: u8 = kani::any();
            kani::assume(digit <= 9);
            write(b'0' + digit);
            digits = digits * 10 + i64::from(digit);
        }
        if let Some(exponent) = exponent {
            write(b'E');
            if exponent < 0 {
                write(b'-');
            }
            let exponent = exponent.unsigned_abs();
            if exponent >= 10 {
                write(b'0' + exponent / 10);
            }
            write(b'0' + exponent % 10);
        }
        (text, len, negative, digits)
    }

    /// What's written comes back exactly: any number of the shape given
    /// (see `any_number`), scaled by `places`, is its digits moved as
    /// many places as its point, its exponent and `places` come to: with
    /// zeros after them to the left, or cut off toward zero where they go
    /// past the point.
    fn scales_exactly(whole: usize, fraction: usize, exponent: Option<i8>, places: i32) {
        let (text, len, negative, digits) = any_number(whole, fraction, exponent);
        let number = parse(&text[..len]).expect("a number");
        let scaled = number.scaled(places);
        let shift = i32::from(exponent.unwrap_or(0)) - fraction as i32 + places;
        let power = 10i64.pow(shift.unsigned_abs());
        let magnitude = if shift >= 0 {
            digits * power
        } else {
            digits / power
        };
        assert_eq!(scaled, if negative { -magnitude } else { magnitude });
    }

    /// Whole numbers come back as written, to 13 digits (or fewer, after
    /// zeros): alarm fields,
    /// intervals, and timestamps in milliseconds (`1762436800000`).
    #[kani::proof]
    #[kani::unwind(21)]
    fn whole_numbers_come_back_as_written() {
        scales_exactly(13, 0, None, 0);
    }

    /// Coordinates in millionths of a degree, exactly: `59.9139` is
    /// 59913900, `-33.86882` is -33868820. With four digits after the
    /// point here, and five in the next.
    #[kani::proof]
    #[kani::unwind(21)]
    fn coordinates_scale_to_millionths() {
        scales_exactly(3, 4, None, 6);
    }

    #[kani::proof]
    #[kani::unwind(21)]
    fn longer_coordinates_scale_to_millionths() {
        scales_exactly(3, 5, None, 6);
    }

    /// Past the millionths (Java prints some doubles to 17 digits), a
    /// coordinate is cut off toward zero, either side of it: here with
    /// eight digits after the point.
    #[kani::proof]
    #[kani::unwind(21)]
    fn finer_coordinates_are_cut_toward_zero() {
        scales_exactly(3, 8, None, 6);
    }

    /// A fraction made whole is cut off toward zero, too: an alarm's
    /// `7.9` is 7, and `-7.9` is -7.
    #[kani::proof]
    #[kani::unwind(21)]
    fn fractions_made_whole_are_cut_toward_zero() {
        scales_exactly(4, 2, None, 0);
    }

    /// With an exponent, as Java writes large and small doubles: a
    /// timestamp like `1.7624E7` is 17624000, and `-1.000E-4` degrees is
    /// -100 millionths. (Five digits of five places each, not the
    /// thirteen of a timestamp in milliseconds, `1.7624368E12`: each
    /// place the digits move left is another multiplication CBMC has to
    /// show can't overflow, and that one it gave up on after 160 s.)
    #[kani::proof]
    #[kani::unwind(21)]
    fn a_large_exponent_moves_the_digits_left() {
        scales_exactly(1, 4, Some(7), 0);
    }

    #[kani::proof]
    #[kani::unwind(21)]
    fn a_small_exponent_moves_the_digits_right() {
        scales_exactly(1, 3, Some(-4), 6);
    }

    /// As an `f32`, a number without an exponent is the nearest there
    /// is, as `str::parse` would give: its digits are exact in an `f32`
    /// (seven of them are under 2^24), and so is the power of ten that
    /// divides them, which leaves one rounding. A time zone of `5.75`
    /// hours is 345 minutes, not 344.99.
    #[kani::proof]
    #[kani::unwind(71)]
    fn written_digits_are_the_nearest_f32() {
        let fraction = 2;
        let (text, len, negative, digits) = any_number(5, fraction, None);
        let number = parse(&text[..len]).expect("a number");
        let nearest = digits as f32 / 100.0;
        assert_eq!(number.to_f32(), if negative { -nearest } else { nearest });
    }
}
