//! Decimal numbers as Gadgetbridge writes them (`59.9139`, `-3.5`, `1.0E-4`,
//! `1762436800000`), parsed to their digits and a power of ten, and from
//! there to an integer at a chosen scale ([`Decimal::scaled`]: millionths
//! of a degree, milliseconds) or an `f32` ([`Decimal::to_f32`]).
//!
//! Not to `f64`, nor with `str::parse`: the M4F's FPU does single
//! precision only, so `f64` is soft-float routines (~3 KB of flash for
//! these few numbers), and `str::parse::<f32>` is correctly rounded for
//! every input, which takes 12 KB of tables (`dec2flt`'s powers of five)
//! and 4 KB of code. Integers are exact where it matters (coordinates,
//! timestamps, alarm times), and the `f32`s (a fix's accuracy, a time
//! zone in hours) are within an ulp or two, far finer than their use.

/// A parsed number: `digits` × 10^`exponent`, negated if `negative`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Decimal {
    negative: bool,
    digits: u64,
    exponent: i32,
}

/// Parses `[+-]digits[.digits][(e|E)[+-]digits]`, with digits on at least one
/// side of the point. `None` for anything else, including `inf` and `NaN`.
pub fn parse(text: &[u8]) -> Option<Decimal> {
    let (negative, mut rest) = match text {
        [b'-', rest @ ..] => (true, rest),
        [b'+', rest @ ..] => (false, rest),
        _ => (false, text),
    };

    // The significant digits as an integer, and the power of ten it's off by
    // (from the point, and from digits that don't fit in a u64, which are
    // dropped: 19 digits is already far past anything here needs).
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

    Some(Decimal {
        negative,
        digits,
        exponent,
    })
}

impl Decimal {
    /// The number times 10^`places`, toward zero to an integer, and
    /// saturating at `i64`'s ends.
    pub fn scaled(self, places: i32) -> i64 {
        let mut digits = self.digits;
        let exponent = self.exponent.saturating_add(places);
        // A step at a time, stopping once nothing more can change: the
        // exponent may be huge (`1e999999`), but past a u64's 20 digits
        // every step saturates, or gives zero.
        for _ in 0..exponent.unsigned_abs().min(20) {
            digits = if exponent > 0 {
                digits.saturating_mul(10)
            } else {
                digits / 10
            };
        }
        let magnitude = digits.min(i64::MAX as u64) as i64;
        if self.negative { -magnitude } else { magnitude }
    }

    /// The number as the nearest `f32`, or within an ulp or two of it
    /// (the digits round once to an `f32`, and again on scaling by a power
    /// of ten, exact only up to 10^10); infinite or zero far out of range.
    pub fn to_f32(self) -> f32 {
        let mut power = 1.0f32;
        // An f32 is infinite from 10^39 and zero below 10^-45, the digits
        // at most 10^20: past 70 nothing changes.
        for _ in 0..self.exponent.unsigned_abs().min(70) {
            power *= 10.0;
        }
        // Dividing by an exact power of ten rounds once, where multiplying
        // by an inexact negative power would round twice. (Zero first: `0e400`
        // would otherwise be zero times infinity.)
        let magnitude = self.digits as f32;
        let value = if self.digits == 0 {
            0.0
        } else if self.exponent >= 0 {
            magnitude * power
        } else {
            magnitude / power
        };
        if self.negative { -value } else { value }
    }
}
