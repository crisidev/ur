//! JSON as Gadgetbridge handles it: Android's `org.json` for what it holds
//! and reads, and its own writer for what it sends a Bangle.js.
//!
//! Neither is strict JSON:
//!
//! - **Writing** ([`Json::to_wire`]) is `jsonToStringInternal`
//!   (`BangleJSDeviceSupport.java:405-480`): JavaScript string escapes,
//!   `atob("…")` where base64 is shorter, numbers as Java prints them.
//! - **Reading** ([`Object::parse`]) is Android's `org.json.JSONTokener`
//!   and `JSONObject` (AOSP `libcore/json/src/main/java/org/json/`; not in
//!   Gadgetbridge's tree, so cited by method name): lenient about quotes,
//!   separators, comments, and what follows the object; integers with a
//!   leading zero are octal; and the getters coerce between types.

use crate::java;

/// A value in a `JSONObject`: what `put` stored, or what the tokener read.
#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    /// `JSONObject.NULL`, from the literal `null`. (Putting a Java `null`
    /// removes the key instead; see [`Object::put`].)
    Null,
    Bool(bool),
    /// A Java `Integer`.
    Int(i32),
    /// A Java `Long`.
    Long(i64),
    /// A Java `Double` (a `Float` put through `put(String, double)` is
    /// widened to one).
    Double(f64),
    Str(String),
    Array(Vec<Json>),
    Object(Object),
}

/// A `JSONObject`: Android's keeps its members in a `LinkedHashMap`, so in
/// the order they were first put.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Object(Vec<(String, Json)>);

/// A `JSONException`, with (roughly) Android's message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JsonException(pub String);

type Result<T> = std::result::Result<T, JsonException>;

fn exception<T>(message: impl Into<String>) -> Result<T> {
    Err(JsonException(message.into()))
}

impl From<bool> for Json {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}
impl From<i32> for Json {
    fn from(value: i32) -> Self {
        Self::Int(value)
    }
}
impl From<i64> for Json {
    fn from(value: i64) -> Self {
        Self::Long(value)
    }
}
impl From<f64> for Json {
    fn from(value: f64) -> Self {
        Self::Double(value)
    }
}
impl From<&str> for Json {
    fn from(value: &str) -> Self {
        Self::Str(value.into())
    }
}
impl From<String> for Json {
    fn from(value: String) -> Self {
        Self::Str(value)
    }
}
impl From<Object> for Json {
    fn from(value: Object) -> Self {
        Self::Object(value)
    }
}
impl From<Vec<Json>> for Json {
    fn from(value: Vec<Json>) -> Self {
        Self::Array(value)
    }
}

/// What [`Object::put`] takes: a value, or a Java `null` (`None`).
pub trait Put {
    fn into_value(self) -> Option<Json>;
}

impl<T: Into<Json>> Put for T {
    fn into_value(self) -> Option<Json> {
        Some(self.into())
    }
}

impl Put for Option<Json> {
    fn into_value(self) -> Option<Json> {
        self
    }
}

// ==============================================================================
// Building and writing: phone → watch
// ==============================================================================

impl Object {
    pub fn new() -> Self {
        Self::default()
    }

    /// `JSONObject.put(String, Object)`: replaces the value of a key that's
    /// there (where it is), adds one that isn't, and **removes** the key if
    /// the value is a Java `null`. That last one is why a notification
    /// without a phone number has no `tel` at all.
    pub fn put(&mut self, key: &str, value: impl Put) -> &mut Self {
        let at = self.0.iter().position(|(k, _)| k == key);
        match (value.into_value(), at) {
            (Some(value), Some(at)) => self.0[at].1 = value,
            (Some(value), None) => self.0.push((key.into(), value)),
            (None, Some(at)) => {
                self.0.remove(at);
            }
            (None, None) => {}
        }
        self
    }

    /// [`Object::put`] of a string that may be `null`.
    pub fn put_str(&mut self, key: &str, value: Option<&str>) -> &mut Self {
        self.put(key, value.map(Json::from))
    }

    pub fn members(&self) -> &[(String, Json)] {
        &self.0
    }

    /// `jsonToString` (`BangleJSDeviceSupport.java:483-491`): the object as
    /// it goes on the wire, in ISO-8859-1 bytes.
    pub fn to_wire(&self) -> Vec<u8> {
        let mut out = Vec::new();
        write_object(self, &mut out);
        out
    }
}

impl Json {
    /// `jsonToStringInternal` (`BangleJSDeviceSupport.java:405-480`).
    pub fn to_wire(&self) -> Vec<u8> {
        let mut out = Vec::new();
        write_value(self, &mut out);
        out
    }
}

fn write_object(object: &Object, out: &mut Vec<u8>) {
    // BangleJSDeviceSupport.java:459-474. Keys go out as they are: they're
    // Gadgetbridge's own, all plain ASCII.
    out.push(b'{');
    for (i, (key, value)) in object.0.iter().enumerate() {
        if i > 0 {
            out.push(b',');
        }
        out.push(b'"');
        out.extend(key.chars().map(|c| c as u32 as u8));
        out.extend_from_slice(b"\":");
        write_value(value, out);
    }
    out.push(b'}');
}

fn write_value(value: &Json, out: &mut Vec<u8>) {
    match value {
        Json::Str(text) => out.extend(string_to_wire(text)),
        // BangleJSDeviceSupport.java:445-458.
        Json::Array(items) => {
            out.push(b'[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                write_value(item, out);
            }
            out.push(b']');
        }
        Json::Object(object) => write_object(object, out),
        // BangleJSDeviceSupport.java:475-479: `null`, or the value's own
        // `toString()` (`JSONObject.NULL`'s is "null" too).
        Json::Null => out.extend_from_slice(b"null"),
        Json::Bool(b) => out.extend_from_slice(if *b { b"true" } else { b"false" }),
        Json::Int(n) => out.extend(n.to_string().bytes()),
        Json::Long(n) => out.extend(n.to_string().bytes()),
        Json::Double(n) => out.extend(java::double_to_string(*n).bytes()),
    }
}

/// A string as Gadgetbridge writes it for Espruino
/// (`BangleJSDeviceSupport.java:406-444`): a JavaScript string literal, or
/// `atob("…")` of its ISO-8859-1 bytes where that's shorter. In ISO-8859-1
/// bytes, which is what `uartTx` sends (`:391`).
///
/// Java strings are UTF-16, so a character beyond U+FFFF is two `\uXXXX`.
///
/// Faithful to its quirks:
///
/// - A character below 8 is `\N`, or `\x0N` if the next is an octal digit
///   (`:417-421`); but DLE is always `\20` (`:427`), so DLE followed by an
///   octal digit reads back in JavaScript as one other character (`\205`).
/// - What's escaped as `\xHH` is the range 0xC2–0xF4 ("unicode start
///   chars", `:430-432`), 127 and 173, so 0x80–0xC1 and 0xF5–0xFF go out
///   raw.
/// - Base64 is only chosen when nothing is beyond U+00FF (`:438`): one curly
///   quote in a text of accented letters, and every one of those takes
///   four bytes.
pub fn string_to_wire(text: &str) -> Vec<u8> {
    let units: Vec<u16> = text.encode_utf16().collect();
    let mut json: Vec<u8> = vec![b'"'];
    let mut has_unicode = false;
    for (i, &unit) in units.iter().enumerate() {
        let ch = u32::from(unit);
        let next = units.get(i + 1).map_or(0, |&n| u32::from(n));
        if ch > 255 {
            has_unicode = true;
        }
        match ch {
            0..=7 => {
                if (u32::from(b'0')..=u32::from(b'7')).contains(&next) {
                    json.extend(format!("\\x0{ch}").bytes());
                } else {
                    json.extend(format!("\\{ch}").bytes());
                }
            }
            8 => json.extend_from_slice(b"\\b"),
            9 => json.extend_from_slice(b"\\t"),
            10 => json.extend_from_slice(b"\\n"),
            11 => json.extend_from_slice(b"\\v"),
            12 => json.extend_from_slice(b"\\f"),
            16 => json.extend_from_slice(b"\\20"),
            34 => json.extend_from_slice(b"\\\""),
            92 => json.extend_from_slice(b"\\\\"),
            _ if ch < 32 || ch == 127 || ch == 173 || (0xC2..=0xF4).contains(&ch) => {
                json.extend(format!("\\x{:02x}", ch & 255).bytes());
            }
            _ if ch > 255 => json.extend(format!("\\u{:04x}", ch & 65535).bytes()),
            _ => json.push(ch as u8),
        }
    }
    // `json.length()` here counts the opening quote but not the closing one.
    if !has_unicode && json.len() > 5 + units.len() * 4 / 3 {
        let bytes: Vec<u8> = units.iter().map(|&u| u as u8).collect();
        let base64 = java::base64_default(&bytes).replace('\n', "");
        return format!("atob(\"{base64}\")").into_bytes();
    }
    json.push(b'"');
    json
}

// ==============================================================================
// Reading: watch → phone
// ==============================================================================

impl Object {
    /// `new JSONObject(String)`: the first value in the text, which must be
    /// an object. **Whatever follows it is never looked at.**
    pub fn parse(text: &str) -> Result<Self> {
        let chars: Vec<char> = text.chars().collect();
        let mut tokener = Tokener {
            text: &chars,
            at: 0,
        };
        match tokener.next_value()? {
            Json::Object(object) => Ok(object),
            other => exception(format!(
                "Value {} of type {} cannot be converted to JSONObject",
                other.display(),
                other.type_name()
            )),
        }
    }

    /// `JSONObject.has`: whether the key is there at all, `null` included.
    pub fn has(&self, key: &str) -> bool {
        self.find(key).is_some()
    }

    fn find(&self, key: &str) -> Option<&Json> {
        self.0.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    /// `JSONObject.get`: throws if the key is missing.
    pub fn get(&self, key: &str) -> Result<&Json> {
        match self.find(key) {
            Some(value) => Ok(value),
            None => exception(format!("No value for {key}")),
        }
    }

    fn mismatch<T>(key: &str, value: &Json, wanted: &str) -> Result<T> {
        exception(format!(
            "Value {} at {key} of type {} cannot be converted to {wanted}",
            value.display(),
            value.type_name()
        ))
    }

    /// `JSONObject.getString`: any value but a missing one, as text
    /// (`JSON.toString`: a number or boolean becomes its `String.valueOf`).
    pub fn get_string(&self, key: &str) -> Result<String> {
        Ok(self.get(key)?.as_string())
    }

    /// `JSONObject.optString(key, fallback)`.
    pub fn opt_string(&self, key: &str, fallback: &str) -> String {
        self.find(key).map_or(fallback.into(), Json::as_string)
    }

    /// `JSONObject.getBoolean`.
    pub fn get_boolean(&self, key: &str) -> Result<bool> {
        let value = self.get(key)?;
        value
            .as_boolean()
            .map_or_else(|| Self::mismatch(key, value, "boolean"), Ok)
    }

    /// `JSONObject.optBoolean(key, fallback)`.
    pub fn opt_boolean(&self, key: &str, fallback: bool) -> bool {
        self.find(key)
            .and_then(Json::as_boolean)
            .unwrap_or(fallback)
    }

    /// `JSONObject.getInt`.
    pub fn get_int(&self, key: &str) -> Result<i32> {
        let value = self.get(key)?;
        value
            .as_int()
            .map_or_else(|| Self::mismatch(key, value, "int"), Ok)
    }

    /// `JSONObject.optInt(key, fallback)`.
    pub fn opt_int(&self, key: &str, fallback: i32) -> i32 {
        self.find(key).and_then(Json::as_int).unwrap_or(fallback)
    }

    /// `JSONObject.getLong`.
    pub fn get_long(&self, key: &str) -> Result<i64> {
        let value = self.get(key)?;
        value
            .as_long()
            .map_or_else(|| Self::mismatch(key, value, "long"), Ok)
    }

    /// `JSONObject.optLong(key, fallback)`.
    pub fn opt_long(&self, key: &str, fallback: i64) -> i64 {
        self.find(key).and_then(Json::as_long).unwrap_or(fallback)
    }

    /// `JSONObject.getDouble`.
    pub fn get_double(&self, key: &str) -> Result<f64> {
        let value = self.get(key)?;
        value
            .as_double()
            .map_or_else(|| Self::mismatch(key, value, "double"), Ok)
    }

    /// `JSONObject.getJSONArray`.
    pub fn get_array(&self, key: &str) -> Result<&[Json]> {
        match self.get(key)? {
            Json::Array(items) => Ok(items),
            other => Self::mismatch(key, other, "JSONArray"),
        }
    }

    /// `JSONObject.getJSONObject`.
    pub fn get_object(&self, key: &str) -> Result<&Object> {
        match self.get(key)? {
            Json::Object(object) => Ok(object),
            other => Self::mismatch(key, other, "JSONObject"),
        }
    }
}

impl Json {
    fn type_name(&self) -> &'static str {
        match self {
            Self::Null => "org.json.JSONObject$1",
            Self::Bool(_) => "java.lang.Boolean",
            Self::Int(_) => "java.lang.Integer",
            Self::Long(_) => "java.lang.Long",
            Self::Double(_) => "java.lang.Double",
            Self::Str(_) => "java.lang.String",
            Self::Array(_) => "org.json.JSONArray",
            Self::Object(_) => "org.json.JSONObject",
        }
    }

    /// `String.valueOf(value)`, near enough for messages and for
    /// [`Json::as_string`] (objects and arrays print as this crate's wire
    /// form rather than `org.json`'s own writer's; nothing reads those).
    fn display(&self) -> String {
        match self {
            Self::Str(text) => text.clone(),
            other => other.to_wire().iter().map(|&b| char::from(b)).collect(),
        }
    }

    /// `JSON.toString(Object)`: a string as it is, anything else through
    /// `String.valueOf`. Never fails for a value that's there.
    pub fn as_string(&self) -> String {
        self.display()
    }

    /// `JSON.toBoolean(Object)`: a boolean, or the strings `true`/`false` in
    /// any case.
    pub fn as_boolean(&self) -> Option<bool> {
        match self {
            Self::Bool(b) => Some(*b),
            Self::Str(text) if text.eq_ignore_ascii_case("true") => Some(true),
            Self::Str(text) if text.eq_ignore_ascii_case("false") => Some(false),
            _ => None,
        }
    }

    /// `JSON.toDouble(Object)`: any number, or a string that parses as one.
    ///
    /// TODO: `Double.parseDouble` also takes hexadecimal floats and a
    /// trailing `d` or `f`; Rust's parser stands in for it.
    pub fn as_double(&self) -> Option<f64> {
        match self {
            Self::Int(n) => Some(f64::from(*n)),
            Self::Long(n) => Some(*n as f64),
            Self::Double(n) => Some(*n),
            Self::Str(text) => text.trim().parse().ok(),
            _ => None,
        }
    }

    /// `JSON.toInteger(Object)`: a number's `intValue()` (a `long` wraps, a
    /// `double` truncates and saturates), or a string's `(int)
    /// Double.parseDouble`.
    pub fn as_int(&self) -> Option<i32> {
        match self {
            Self::Int(n) => Some(*n),
            Self::Long(n) => Some(*n as i32),
            Self::Double(n) => Some(*n as i32),
            Self::Str(_) => self.as_double().map(|n| n as i32),
            _ => None,
        }
    }

    /// `JSON.toLong(Object)`.
    pub fn as_long(&self) -> Option<i64> {
        match self {
            Self::Int(n) => Some(i64::from(*n)),
            Self::Long(n) => Some(*n),
            Self::Double(n) => Some(*n as i64),
            Self::Str(_) => self.as_double().map(|n| n as i64),
            _ => None,
        }
    }
}

/// `org.json.JSONTokener`.
struct Tokener<'a> {
    text: &'a [char],
    at: usize,
}

impl Tokener<'_> {
    fn syntax<T>(&self, message: &str) -> Result<T> {
        exception(format!("{message} at character {}", self.at))
    }

    /// `nextCleanInternal`: the next character that isn't whitespace or in
    /// a comment (`/* */`, `//` or `#` to the end of the line).
    fn next_clean(&mut self) -> Result<Option<char>> {
        while let Some(&c) = self.text.get(self.at) {
            self.at += 1;
            match c {
                '\t' | ' ' | '\n' | '\r' => continue,
                '/' => match self.text.get(self.at) {
                    Some('*') => {
                        self.at += 1;
                        let rest = &self.text[self.at..];
                        match rest.windows(2).position(|w| w == ['*', '/']) {
                            Some(end) => self.at += end + 2,
                            None => return self.syntax("Unterminated comment"),
                        }
                    }
                    Some('/') => self.skip_to_end_of_line(),
                    _ => return Ok(Some(c)),
                },
                '#' => self.skip_to_end_of_line(),
                _ => return Ok(Some(c)),
            }
        }
        Ok(None)
    }

    fn skip_to_end_of_line(&mut self) {
        while let Some(&c) = self.text.get(self.at) {
            self.at += 1;
            if c == '\r' || c == '\n' {
                break;
            }
        }
    }

    /// `nextValue`.
    fn next_value(&mut self) -> Result<Json> {
        match self.next_clean()? {
            None => self.syntax("End of input"),
            Some('{') => self.read_object(),
            Some('[') => self.read_array(),
            Some(quote @ ('\'' | '"')) => Ok(Json::Str(self.next_string(quote)?)),
            Some(_) => {
                self.at -= 1;
                self.read_literal()
            }
        }
    }

    /// `nextString`: up to the closing quote, single or double.
    fn next_string(&mut self, quote: char) -> Result<String> {
        let mut out = String::new();
        // UTF-16, as Java's: two `\uXXXX` halves make one character.
        let mut units: Vec<u16> = Vec::new();
        let flush = |units: &mut Vec<u16>, out: &mut String| {
            out.push_str(&String::from_utf16_lossy(units));
            units.clear();
        };
        while let Some(&c) = self.text.get(self.at) {
            self.at += 1;
            if c == quote {
                flush(&mut units, &mut out);
                return Ok(out);
            }
            if c != '\\' {
                units.extend(c.encode_utf16(&mut [0; 2]).iter());
                continue;
            }
            // `readEscapeCharacter`.
            let Some(&escaped) = self.text.get(self.at) else {
                return self.syntax("Unterminated escape sequence");
            };
            self.at += 1;
            match escaped {
                'u' => {
                    let Some(hex) = self.text.get(self.at..self.at + 4) else {
                        return self.syntax("Unterminated escape sequence");
                    };
                    let hex: String = hex.iter().collect();
                    self.at += 4;
                    match u16::from_str_radix(&hex, 16) {
                        Ok(unit) if !hex.starts_with(['+', '-']) => units.push(unit),
                        _ => return self.syntax("Invalid escape sequence"),
                    }
                }
                't' => units.push(u16::from(b'\t')),
                'b' => units.push(8),
                'n' => units.push(u16::from(b'\n')),
                'r' => units.push(u16::from(b'\r')),
                'f' => units.push(12),
                // Quotes, the backslash, and anything else: itself.
                other => units.extend(other.encode_utf16(&mut [0; 2]).iter()),
            }
        }
        self.syntax("Unterminated string")
    }

    /// `readLiteral`: `null`, a boolean, a number, or an unquoted string.
    fn read_literal(&mut self) -> Result<Json> {
        let start = self.at;
        while let Some(&c) = self.text.get(self.at) {
            if "{}[]/\\:,=;# \t\x0c\r\n".contains(c) {
                break;
            }
            self.at += 1;
        }
        let literal: String = self.text[start..self.at].iter().collect();
        if literal.is_empty() {
            return self.syntax("Expected literal value");
        }
        if literal.eq_ignore_ascii_case("null") {
            return Ok(Json::Null);
        }
        if literal.eq_ignore_ascii_case("true") {
            return Ok(Json::Bool(true));
        }
        if literal.eq_ignore_ascii_case("false") {
            return Ok(Json::Bool(false));
        }
        // An integer, if there's no point in it: hexadecimal after `0x`,
        // and **octal after any other leading zero**.
        if !literal.contains('.') {
            let (number, base) = if let Some(hex) = literal
                .strip_prefix("0x")
                .or_else(|| literal.strip_prefix("0X"))
            {
                (hex, 16)
            } else if literal.starts_with('0') && literal.len() > 1 {
                (&literal[1..], 8)
            } else {
                (literal.as_str(), 10)
            };
            if let Ok(long) = i64::from_str_radix(number, base) {
                return Ok(match i32::try_from(long) {
                    Ok(int) => Json::Int(int),
                    Err(_) => Json::Long(long),
                });
            }
        }
        // Else a double, if it parses as one (see `Json::as_double` for
        // what Rust's parser misses of Java's).
        // `Double.valueOf` knows these two by name (and `JSONObject.put`
        // then refuses them).
        match literal.trim_start_matches(['+', '-']) {
            "NaN" => return Ok(Json::Double(f64::NAN)),
            "Infinity" => return Ok(Json::Double(f64::INFINITY)),
            _ => {}
        }
        if let Ok(double) = literal.parse::<f64>()
            && literal
                .bytes()
                .all(|b| b.is_ascii_digit() || b"+-.eE".contains(&b))
        {
            return Ok(Json::Double(double));
        }
        Ok(Json::Str(literal))
    }

    /// `readObject`.
    fn read_object(&mut self) -> Result<Json> {
        let mut result = Object::new();
        match self.next_clean()? {
            Some('}') => return Ok(Json::Object(result)),
            Some(_) => self.at -= 1,
            None => {}
        }
        loop {
            let name = match self.next_value()? {
                Json::Str(name) => name,
                Json::Null => return self.syntax("Names cannot be null"),
                _ => return self.syntax("Names must be strings"),
            };
            // `:` or `=`, or `=>`.
            match self.next_clean()? {
                Some(':' | '=') => {}
                _ => return self.syntax(&format!("Expected ':' after {name}")),
            }
            if self.text.get(self.at) == Some(&'>') {
                self.at += 1;
            }
            let value = self.next_value()?;
            // `JSONObject.put` refuses NaN and infinities.
            if matches!(value, Json::Double(d) if !d.is_finite()) {
                return exception("Forbidden numeric value");
            }
            result.put(&name, value);
            match self.next_clean()? {
                Some('}') => return Ok(Json::Object(result)),
                Some(';' | ',') => continue,
                _ => return self.syntax("Unterminated object"),
            }
        }
    }

    /// `readArray`.
    fn read_array(&mut self) -> Result<Json> {
        let mut result = Vec::new();
        let mut trailing_separator = false;
        loop {
            match self.next_clean()? {
                None => return self.syntax("Unterminated array"),
                Some(']') => {
                    if trailing_separator {
                        result.push(Json::Null);
                    }
                    return Ok(Json::Array(result));
                }
                Some(',' | ';') => {
                    result.push(Json::Null);
                    trailing_separator = true;
                    continue;
                }
                Some(_) => self.at -= 1,
            }
            result.push(self.next_value()?);
            match self.next_clean()? {
                Some(']') => return Ok(Json::Array(result)),
                Some(',' | ';') => trailing_separator = true,
                _ => return self.syntax("Unterminated array"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wire(text: &str) -> String {
        string_to_wire(text)
            .iter()
            .map(|&b| char::from(b))
            .collect()
    }

    #[test]
    fn strings_are_escaped_for_espruino() {
        assert_eq!(wire("Signal"), r#""Signal""#);
        assert_eq!(wire("a\"b\\c"), r#""a\"b\\c""#);
        assert_eq!(wire("a\nb\tc"), r#""a\nb\tc""#);
        // Below 8: `\N`, unless an octal digit follows.
        assert_eq!(wire("\0x"), r#""\0x""#);
        assert_eq!(wire("\u{0}1"), r#""\x001""#);
        assert_eq!(wire("\u{0}8"), r#""\08""#);
        // DLE is `\20` whatever follows.
        assert_eq!(wire("\u{10}5"), r#""\205""#);
        // 0xC2–0xF4 are escaped; the rest of Latin-1 isn't.
        assert_eq!(wire("på"), r#""p\xe5""#);
        assert_eq!(
            wire("\u{a0}\u{c1}\u{f5}\u{ff}"),
            "\"\u{a0}\u{c1}\u{f5}\u{ff}\""
        );
        // (In enough plain text that base64 wouldn't be shorter.)
        assert_eq!(
            wire("abcdefghijklmnopqrst\u{ad}\u{7f}\r"),
            r#""abcdefghijklmnopqrst\xad\x7f\x0d""#
        );
        // Beyond Latin-1: `\uXXXX`, a surrogate pair for an emoji.
        assert_eq!(wire("\u{2019}"), r#""\u2019""#);
        assert_eq!(wire("\u{1F600}"), r#""\ud83d\ude00""#);
    }

    #[test]
    fn base64_where_its_shorter() {
        // 4 letters, 13 bytes escaped + the quote = 14 > 5 + 5.
        assert_eq!(wire("æøåé"), r#"atob("5vjl6Q==")"#);
        // One character beyond Latin-1 rules it out.
        // (And ø, at 0xF8, is past the range that's escaped.)
        assert_eq!(wire("æøåé\u{2019}"), "\"\\xe6\u{f8}\\xe5\\xe9\\u2019\"");
        // Not when it isn't shorter: 7 > 5 + 2 is false at the boundary.
        assert_eq!(wire("åb"), r#""\xe5b""#);
    }

    #[test]
    fn objects_keep_their_order_and_drop_nulls() {
        let mut o = Object::new();
        o.put("t", "notify")
            .put("id", 17)
            .put_str("src", None)
            .put("title", "Hei")
            .put_str("tel", None)
            .put("reply", true);
        assert_eq!(
            o.to_wire(),
            br#"{"t":"notify","id":17,"title":"Hei","reply":true}"#
        );
        o.put("id", 18).put("title", None::<Json>);
        assert_eq!(o.to_wire(), br#"{"t":"notify","id":18,"reply":true}"#);
        let mut o = Object::new();
        o.put("lon", 0.0005).put("ts", 1762436800000i64);
        assert_eq!(o.to_wire(), br#"{"lon":5.0E-4,"ts":1762436800000}"#);
    }

    #[test]
    fn the_tokener_is_lenient() {
        let o = Object::parse(r#"{"t":"act","ts":1762436800000,"stp":12,"volt":4.05}"#).unwrap();
        assert_eq!(o.get("ts"), Ok(&Json::Long(1762436800000)));
        assert_eq!(o.get("stp"), Ok(&Json::Int(12)));
        assert_eq!(o.get("volt"), Ok(&Json::Double(4.05)));
        // Unquoted names and strings, single quotes, `=` and `;`, comments,
        // and anything at all after the object.
        let o = Object::parse("{t='act'; n=>yes /* really */ } and then some").unwrap();
        assert_eq!(o.get_string("t").unwrap(), "act");
        assert_eq!(o.get_string("n").unwrap(), "yes");
        // A later member replaces an earlier one of the same name.
        let o = Object::parse(r#"{"a":1,"a":2}"#).unwrap();
        assert_eq!(o.get_int("a"), Ok(2));
    }

    #[test]
    fn leading_zeros_are_octal() {
        let o = Object::parse(r#"{"a":010,"b":0x10,"c":08,"d":0,"e":-010,"f":00}"#).unwrap();
        assert_eq!(o.get("a"), Ok(&Json::Int(8)));
        assert_eq!(o.get("b"), Ok(&Json::Int(16)));
        // Not octal, so on to the next thing it could be: a double.
        assert_eq!(o.get("c"), Ok(&Json::Double(8.0)));
        assert_eq!(o.get("d"), Ok(&Json::Int(0)));
        assert_eq!(o.get("e"), Ok(&Json::Int(-10)));
        assert_eq!(o.get("f"), Ok(&Json::Int(0)));
    }

    #[test]
    fn getters_coerce() {
        let o =
            Object::parse(r#"{"s":"12.7","n":3000000000,"d":1e12,"b":"TRUE","z":null}"#).unwrap();
        assert_eq!(o.get_int("s"), Ok(12));
        assert_eq!(o.get_int("n"), Ok(3000000000i64 as i32));
        assert_eq!(o.get_int("d"), Ok(i32::MAX));
        assert_eq!(o.get_boolean("b"), Ok(true));
        assert_eq!(o.get_string("n").unwrap(), "3000000000");
        assert!(o.has("z"));
        assert_eq!(o.get_string("z").unwrap(), "null");
        assert!(o.get_int("z").is_err());
        assert!(o.get_int("missing").is_err());
        assert_eq!(o.opt_int("missing", -1), -1);
        assert_eq!(o.opt_int("b", -1), -1);
    }

    #[test]
    fn what_doesnt_parse() {
        for text in [
            "",
            "[1]",
            "{",
            r#"{"a"}"#,
            r#"{"a":}"#,
            r#"{"a":1"#,
            r#"{"a":1 "b":2}"#,
            r#"{"a":"unterminated}"#,
            r#"{"a":"\u12"}"#,
            r#"{null:1}"#,
            r#"{1:1}"#,
            r#"{"a":NaN}"#,
        ] {
            assert!(Object::parse(text).is_err(), "{text:?} parsed");
        }
    }
}
