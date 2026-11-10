//! The bits of Java's standard library that decide what goes on the wire:
//! how numbers print, and Android's base64.
//!
//! These aren't Gadgetbridge's code, so they carry no line references; they
//! follow the Java Language Specification and the JDK's documentation.

/// `Double.toString(double)`: the shortest digits that read back as the
/// same value, as a plain decimal from 10^-3 up to 10^7, and in "computerized
/// scientific notation" (`1.0E-4`, `1.2345678E7`) outside it. Always with a
/// digit after the point.
///
/// So a longitude within 100 m of the Greenwich meridian arrives as
/// `5.0E-4`, not `0.0005`.
///
/// TODO: JDKs before 19 (and Android's runtime before it took that change)
/// printed some values with a digit more than the shortest (JDK-4511638).
/// Not modelled: the digits here are always the shortest, which is also
/// what those versions print for nearly every value.
pub fn double_to_string(value: f64) -> String {
    if value.is_nan() {
        return "NaN".into();
    }
    if value.is_infinite() {
        return if value > 0.0 { "Infinity" } else { "-Infinity" }.into();
    }
    layout(value.is_sign_negative(), &format!("{:e}", value.abs()))
}

/// `Float.toString(float)`: as [`double_to_string`], with a `float`'s
/// (fewer) shortest digits.
pub fn float_to_string(value: f32) -> String {
    if value.is_nan() {
        return "NaN".into();
    }
    if value.is_infinite() {
        return if value > 0.0 { "Infinity" } else { "-Infinity" }.into();
    }
    layout(value.is_sign_negative(), &format!("{:e}", value.abs()))
}

/// Lays out Rust's shortest scientific form (`1.2345e-4`, `5e0`) as Java
/// does.
fn layout(negative: bool, scientific: &str) -> String {
    let (mantissa, exponent) = scientific
        .split_once('e')
        .expect("`{:e}` always has an exponent");
    let exponent: i32 = exponent.parse().expect("`{:e}`'s exponent is an integer");
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    let mut out = String::new();
    if negative {
        out.push('-');
    }
    if digits.bytes().all(|d| d == b'0') {
        out.push_str("0.0");
    } else if (-3..7).contains(&exponent) {
        if exponent < 0 {
            out.push_str("0.");
            for _ in 0..(-exponent - 1) {
                out.push('0');
            }
            out.push_str(&digits);
        } else {
            let whole = exponent as usize + 1;
            for i in 0..whole {
                out.push(digits.as_bytes().get(i).map_or('0', |&d| char::from(d)));
            }
            out.push('.');
            if digits.len() > whole {
                out.push_str(&digits[whole..]);
            } else {
                out.push('0');
            }
        }
    } else {
        out.push_str(&digits[..1]);
        out.push('.');
        if digits.len() > 1 {
            out.push_str(&digits[1..]);
        } else {
            out.push('0');
        }
        out.push('E');
        out.push_str(&exponent.to_string());
    }
    out
}

/// `android.util.Base64.encodeToString(bytes, Base64.DEFAULT)`: the standard
/// alphabet, padded, with a line feed after every 76 characters and at the
/// end (if there's anything at all). Gadgetbridge strips the line feeds in
/// some places and not in others, so they're kept here.
pub fn base64_default(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    let mut on_line = 0;
    for chunk in bytes.chunks(3) {
        let group = u32::from(chunk[0]) << 16
            | u32::from(chunk.get(1).copied().unwrap_or(0)) << 8
            | u32::from(chunk.get(2).copied().unwrap_or(0));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(ALPHABET[(group >> (18 - 6 * i) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
        on_line += 4;
        if on_line == 76 {
            out.push('\n');
            on_line = 0;
        }
    }
    if on_line > 0 {
        out.push('\n');
    }
    out
}

/// `android.util.Base64.decode(text, Base64.DEFAULT)`, leniently: anything
/// outside the alphabet is skipped. (Android throws on some of it; nothing
/// here depends on which.)
pub fn base64_decode(text: &str) -> Vec<u8> {
    let value = |c: u8| match c {
        b'A'..=b'Z' => Some(c - b'A'),
        b'a'..=b'z' => Some(c - b'a' + 26),
        b'0'..=b'9' => Some(c - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    };
    let (mut pending, mut bits, mut out) = (0u32, 0, Vec::new());
    for sextet in text.bytes().filter_map(value) {
        pending = (pending << 6) | u32::from(sextet);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((pending >> bits) as u8);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doubles_print_as_java_prints_them() {
        for (value, text) in [
            (0.0, "0.0"),
            (-0.0, "-0.0"),
            (1.0, "1.0"),
            (59.9139, "59.9139"),
            (-3.5, "-3.5"),
            (100.0, "100.0"),
            (0.001, "0.001"),
            (0.0005, "5.0E-4"),
            (0.00012345, "1.2345E-4"),
            (9999999.0, "9999999.0"),
            (1.0e7, "1.0E7"),
            (12345678.9, "1.23456789E7"),
            (1762436800000.0, "1.7624368E12"),
            (123456.789, "123456.789"),
            (-0.000015, "-1.5E-5"),
        ] {
            assert_eq!(double_to_string(value), text);
        }
        // A `float` widened to a `double` (as `JSONObject.put(String, double)`
        // takes one) shows the digits the `float` never had.
        assert_eq!(double_to_string(f64::from(3.9f32)), "3.9000000953674316");
    }

    #[test]
    fn floats_print_as_java_prints_them() {
        for (value, text) in [
            (0.0, "0.0"),
            (1.0, "1.0"),
            (-3.5, "-3.5"),
            (5.75, "5.75"),
            (12.75, "12.75"),
            (-9.5, "-9.5"),
            (0.5, "0.5"),
            // An offset of one second, in hours.
            (1.0 / 3600.0, "2.7777778E-4"),
        ] {
            assert_eq!(float_to_string(value), text);
        }
    }

    #[test]
    fn base64_breaks_lines_as_android_does() {
        assert_eq!(base64_default(b""), "");
        assert_eq!(base64_default(b"a"), "YQ==\n");
        assert_eq!(base64_default(b"ab"), "YWI=\n");
        assert_eq!(base64_default(b"abc"), "YWJj\n");
        let long = base64_default(&[0u8; 58]);
        assert_eq!(long.find('\n'), Some(76));
        assert!(long.ends_with("==\n"));
        assert_eq!(base64_decode(&long), [0u8; 58]);
        // Exactly one line's worth has the one line feed.
        assert_eq!(base64_default(&[0u8; 57]).matches('\n').count(), 1);
    }
}
