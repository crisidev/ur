//! Decimal numbers as Gadgetbridge writes them (`59.9139`, `-3.5`, `1.0E-4`,
//! `1762436800000`), parsed to `f64`.
//!
//! `str::parse::<f64>` would do, but it's correctly rounded for every input,
//! which takes 12 KB of tables (`dec2flt`'s powers of five) and 4 KB of code:
//! 4% of our flash, for a handful of coordinates and timestamps. This is
//! Clinger's fast path alone: the digits as an integer, times or divided by
//! an exact power of ten. That's correctly rounded whenever the digits fit
//! in 53 bits and the exponent is at most 22, which covers every number
//! Gadgetbridge sends but the odd 17-digit coordinate; beyond it, the result
//! is off in its last digit or so (more for huge exponents), which nothing
//! here can notice: locations are kept to a millionth of a degree. Nor can
//! the other shortcut: a power of ten past 10^308 is infinite, so `1e-400`
//! is zero, and so is `12e-309` (a subnormal, otherwise).

/// Parses `[+-]digits[.digits][(e|E)[+-]digits]`, with digits on at least one
/// side of the point. `None` for anything else, including `inf` and `NaN`.
pub fn parse(text: &[u8]) -> Option<f64> {
    let (negative, mut rest) = match text {
        [b'-', rest @ ..] => (true, rest),
        [b'+', rest @ ..] => (false, rest),
        _ => (false, text),
    };

    // The significant digits as an integer, and the power of ten it's off by
    // (from the point, and from digits that don't fit in a u64, which are
    // dropped: 19 digits is already far past an f64's 17).
    let mut digits: u64 = 0;
    let mut exponent: i32 = 0;
    let mut any_digits = false;
    let mut after_point = false;
    while let [byte, tail @ ..] = rest {
        match byte {
            b'0'..=b'9' => {
                any_digits = true;
                match digits
                    .checked_mul(10)
                    .and_then(|d| d.checked_add(u64::from(byte - b'0')))
                {
                    Some(d) => {
                        digits = d;
                        exponent -= i32::from(after_point);
                    }
                    None => exponent += i32::from(!after_point),
                }
            }
            b'.' if !after_point => after_point = true,
            _ => break,
        }
        rest = tail;
    }
    if !any_digits {
        return None;
    }

    if let [b'e' | b'E', tail @ ..] = rest {
        let (sign, mut tail) = match tail {
            [b'-', tail @ ..] => (-1, tail),
            [b'+', tail @ ..] => (1, tail),
            _ => (1, tail),
        };
        let mut written: i32 = 0;
        let mut any = false;
        while let [byte @ b'0'..=b'9', after @ ..] = tail {
            any = true;
            // Saturating: 1e999999 is just infinity, 1e-999999 zero.
            written = written
                .saturating_mul(10)
                .saturating_add(i32::from(byte - b'0'));
            tail = after;
        }
        if !any {
            return None;
        }
        exponent = exponent.saturating_add(sign * written);
        rest = tail;
    }
    if !rest.is_empty() {
        return None;
    }

    // Dividing by an exact power of ten rounds once, where multiplying by an
    // inexact negative power would round twice. (Zero first: `0e400` would
    // otherwise be zero times infinity.)
    let magnitude = digits as f64;
    let value = if digits == 0 {
        0.0
    } else if exponent >= 0 {
        magnitude * power_of_ten(exponent.unsigned_abs())
    } else {
        magnitude / power_of_ten(exponent.unsigned_abs())
    };
    Some(if negative { -value } else { value })
}

/// 10^`exponent`, exact up to 10^22 (every factor on the way is a power of
/// ten that an f64 holds exactly); infinity from about 10^309.
fn power_of_ten(mut exponent: u32) -> f64 {
    let (mut result, mut square) = (1.0, 10.0);
    while exponent > 0 {
        if exponent & 1 == 1 {
            result *= square;
        }
        square *= square;
        exponent >>= 1;
    }
    result
}
