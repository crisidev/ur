//! Gadgetbridge's Bangle.js protocol: what we speak over Bluetooth.
//!
//! We present ourselves to Gadgetbridge as a Bangle.js (an Espruino watch),
//! because its Bangle.js support has everything we want: notifications with
//! the app and sender as separate fields, call start *and* end, music, phone
//! GPS forwarding, and activity/sleep samples that land in Gadgetbridge's
//! database (and from there in Health Connect). We only ever implemented what
//! Gadgetbridge's driver (`BangleJSDeviceSupport.java`) sends and expects.
//!
//! The transport is the Nordic UART service: a byte stream each way.
//!
//! **Phone → watch** is lines of text, each starting with `\x10` (Espruino's
//! "don't echo this" marker) and ending in `\n`, in ISO-8859-1. Two shapes:
//!
//! ```text
//! \x10GB({"t":"notify","id":17,"src":"Signal","sender":"Alice",…})
//! \x10setTime(1762436800);E.setTimeZone(1.0);(s=>s&&…)(…)
//! ```
//!
//! The second is JavaScript for a real Bangle.js; we recognise its shape and
//! pull out the UTC timestamp and the UTC offset (hours, possibly fractional).
//!
//! **Watch → phone** is lines of plain JSON ending in `\r\n`. (Gadgetbridge
//! drops the character before each `\n`, expecting a `\r`.)

use core::fmt::Write as _;

use defmt::{Format, debug, warn};
use heapless::String;

// ==============================================================================
// Incoming: line assembly
// ==============================================================================

/// Longest line we accept. Gadgetbridge caps notification fields at 80
/// (title, subject), 40 (sender), and 400 (body) characters, so a notification
/// line stays under ~700 bytes. Longer lines are dropped whole.
const MAX_LINE: usize = 1024;

/// Reassembles lines from the arbitrary chunks Gadgetbridge writes (it splits
/// lines at its idea of the MTU).
pub struct LineBuffer {
    buffer: heapless::Vec<u8, MAX_LINE>,
    /// Set when the current line outgrew the buffer: skip to its end.
    overflowed: bool,
}

impl LineBuffer {
    pub const fn new() -> Self {
        Self {
            buffer: heapless::Vec::new(),
            overflowed: false,
        }
    }

    /// Forgets any partial line, e.g. one cut off by a disconnect.
    pub fn clear(&mut self) {
        self.buffer.clear();
        self.overflowed = false;
    }

    /// Adds received bytes, calling `on_line` for each completed line
    /// (without the leading `\x10` and trailing `\r\n`).
    pub fn push(&mut self, data: &[u8], mut on_line: impl FnMut(&[u8])) {
        for &byte in data {
            if byte == b'\n' {
                if !self.overflowed {
                    let mut line = &self.buffer[..];
                    while let [b'\x10', rest @ ..] = line {
                        line = rest;
                    }
                    if let [rest @ .., b'\r'] = line {
                        line = rest;
                    }
                    on_line(line);
                } else {
                    warn!("gb: dropped a line longer than {} bytes", MAX_LINE);
                }
                self.buffer.clear();
                self.overflowed = false;
            } else if self.buffer.push(byte).is_err() {
                self.overflowed = true;
            }
        }
    }
}

// ==============================================================================
// Incoming: messages
// ==============================================================================

/// The messages we act on. Everything else Gadgetbridge sends (calendar,
/// weather, …) is logged and ignored.
pub enum Incoming<'a> {
    /// From `setTime(…);E.setTimeZone(…)`.
    SetTime {
        utc_seconds: i64,
        offset_minutes: i16,
    },
    /// `{"t":"gps",…}`: a location from the phone.
    Gps { latitude: f64, longitude: f64 },
    /// `{"t":"is_gps_active"}`: Gadgetbridge asking whether we want GPS.
    IsGpsActive,
    /// `{"t":"actfetch","ts":…}`: send activity samples since `ts` (UTC
    /// milliseconds).
    ActivityFetch { since_ms: i64 },
    /// Anything else in `GB(…)`, for the caller to dispatch on `t` (the
    /// object is still available to read fields from).
    Other { kind: &'a str, object: Object<'a> },
}

/// Parses one line. `None` for lines that aren't one of the two shapes.
pub fn parse(line: &[u8]) -> Option<Incoming<'_>> {
    if let Some(rest) = line.strip_prefix(b"setTime(") {
        return parse_set_time(rest);
    }
    let json = line.strip_prefix(b"GB(")?.strip_suffix(b")")?;
    let object = Object(json);
    let kind = object.get("t")?.as_raw_str()?;
    Some(match kind {
        "gps" => Incoming::Gps {
            latitude: object.get("lat")?.as_f64()?,
            longitude: object.get("lon")?.as_f64()?,
        },
        "is_gps_active" => Incoming::IsGpsActive,
        "actfetch" => Incoming::ActivityFetch {
            since_ms: object.get("ts").and_then(|v| v.as_f64()).unwrap_or(0.0) as i64,
        },
        _ => Incoming::Other { kind, object },
    })
}

/// `1762436800);E.setTimeZone(1.0);…` → UTC seconds and offset.
fn parse_set_time(rest: &[u8]) -> Option<Incoming<'static>> {
    let end = rest.iter().position(|&b| b == b')')?;
    let utc_seconds = core::str::from_utf8(&rest[..end]).ok()?.parse().ok()?;
    let zone = find(rest, b"E.setTimeZone(")?;
    let zone = &rest[zone + b"E.setTimeZone(".len()..];
    let end = zone.iter().position(|&b| b == b')')?;
    let hours = crate::decimal::parse(&zone[..end])? as f32;
    // Offsets are whole quarter-hours in practice (e.g. +5:45); rounding to
    // minutes absorbs float noise in Gadgetbridge's division.
    let offset_minutes = libm::roundf(hours * 60.0) as i16;
    Some(Incoming::SetTime {
        utc_seconds,
        offset_minutes,
    })
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

// ==============================================================================
// A minimal JSON reader
// ==============================================================================
//
// Just enough for Gadgetbridge's objects: look up a member by key and read
// it as a string, number, or boolean, or as an array of objects (alarms).
// Other nested values are skipped over, not interpreted. No allocation;
// values borrow from the line.

/// A JSON object's text, from `{` to `}`.
#[derive(Clone, Copy)]
pub struct Object<'a>(&'a [u8]);

/// One member's value, still as raw text.
#[derive(Clone, Copy)]
pub struct Value<'a>(&'a [u8]);

impl<'a> Object<'a> {
    pub fn get(&self, key: &str) -> Option<Value<'a>> {
        let mut rest = self.0.strip_prefix(b"{")?;
        loop {
            rest = skip_whitespace(rest);
            if rest.first() == Some(&b'}') {
                return None;
            }
            let (member_key, after_key) = take_value(rest)?;
            rest = skip_whitespace(after_key).strip_prefix(b":")?;
            let (value, after_value) = take_value(skip_whitespace(rest))?;
            if member_key.strip_prefix(b"\"")?.strip_suffix(b"\"")? == key.as_bytes() {
                return Some(Value(value));
            }
            rest = skip_whitespace(after_value);
            rest = rest.strip_prefix(b",").unwrap_or(rest);
        }
    }
}

impl<'a> Value<'a> {
    /// The string's contents if it needs no unescaping (e.g. `"gps"`).
    pub fn as_raw_str(&self) -> Option<&'a str> {
        let inner = self.0.strip_prefix(b"\"")?.strip_suffix(b"\"")?;
        if inner.contains(&b'\\') {
            return None;
        }
        core::str::from_utf8(inner).ok()
    }

    /// Decodes a string value into `out`, truncating if it doesn't fit.
    ///
    /// Gadgetbridge's strings for a Bangle.js are JavaScript rather than
    /// strict JSON (see `jsonToStringInternal` in its driver):
    ///
    /// - String literals use JavaScript escapes: `\xHH` for control
    ///   characters and the Latin-1 range 0xC2–0xF4 (å, æ, ø, é, … all fall
    ///   in it), `\uHHHH` beyond Latin-1, one- or two-digit octal like `\0`
    ///   and `\20`, and `\b \t \n \v \f`. Other bytes are raw ISO-8859-1.
    /// - Where escaping would cost more than base64, the "string" is instead
    ///   the expression `atob("…")`, base64 of the ISO-8859-1 bytes. Names
    ///   heavy in accented letters arrive this way.
    pub fn decode_into<const N: usize>(&self, out: &mut String<N>) -> Option<()> {
        if let Some(base64) = self
            .0
            .strip_prefix(b"atob(\"")
            .and_then(|v| v.strip_suffix(b"\")"))
        {
            for byte in decode_base64(base64) {
                if out.push(char::from(byte)).is_err() {
                    break;
                }
            }
            return Some(());
        }

        let inner = self.0.strip_prefix(b"\"")?.strip_suffix(b"\"")?;
        let mut bytes = inner.iter().copied().peekable();
        let hex = |bytes: &mut core::iter::Peekable<_>, digits: usize| -> Option<u32> {
            let text: heapless::Vec<u8, 4> = bytes.by_ref().take(digits).collect();
            u32::from_str_radix(core::str::from_utf8(&text).ok()?, 16).ok()
        };
        while let Some(byte) = bytes.next() {
            let c = if byte != b'\\' {
                // Latin-1 maps straight onto U+0000–U+00FF.
                char::from(byte)
            } else {
                match bytes.next()? {
                    b'x' => char::from_u32(hex(&mut bytes, 2)?)?,
                    b'u' => {
                        let unit = hex(&mut bytes, 4)?;
                        // A UTF-16 surrogate pair spells one character
                        // (emoji, mostly), which the font lacks anyway.
                        if (0xD800..0xDC00).contains(&unit) && bytes.peek() == Some(&b'\\') {
                            bytes.next();
                            bytes.next(); // 'u'
                            let low = hex(&mut bytes, 4)?;
                            char::from_u32(0x10000 + ((unit - 0xD800) << 10) + (low - 0xDC00))
                                .unwrap_or('?')
                        } else {
                            char::from_u32(unit).unwrap_or('?')
                        }
                    }
                    digit @ b'0'..=b'7' => {
                        // Octal: JavaScript reads up to three digits.
                        let mut value = u32::from(digit - b'0');
                        for _ in 0..2 {
                            match bytes.peek() {
                                Some(&d @ b'0'..=b'7') => {
                                    value = value * 8 + u32::from(d - b'0');
                                    bytes.next();
                                }
                                _ => break,
                            }
                        }
                        char::from_u32(value)?
                    }
                    // Line breaks and tabs would only mess up one-line labels.
                    b'n' | b'r' | b't' | b'v' | b'f' | b'b' => ' ',
                    other => char::from(other), // \" \\ \/
                }
            };
            if out.push(c).is_err() {
                break;
            }
        }
        Some(())
    }

    /// The value as an object, if it is one.
    pub fn as_object(&self) -> Option<Object<'a>> {
        self.0.starts_with(b"{").then_some(Object(self.0))
    }

    /// The elements of an array (none if it isn't one, and up to any that
    /// doesn't parse).
    pub fn items(&self) -> impl Iterator<Item = Value<'a>> + use<'a> {
        let mut rest = self.0.strip_prefix(b"[");
        core::iter::from_fn(move || {
            let here = skip_whitespace(rest.take()?);
            if here.first() == Some(&b']') {
                return None;
            }
            let (value, after) = take_value(here)?;
            let after = skip_whitespace(after);
            rest = Some(after.strip_prefix(b",").unwrap_or(after));
            Some(Value(value))
        })
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self.0 {
            b"true" => Some(true),
            b"false" => Some(false),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        crate::decimal::parse(self.0)
    }
}

fn skip_whitespace(s: &[u8]) -> &[u8] {
    let start = s
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .unwrap_or(s.len());
    &s[start..]
}

/// Splits one JSON value (any kind) off the front of `s`.
fn take_value(s: &[u8]) -> Option<(&[u8], &[u8])> {
    let end = match *s.first()? {
        b'"' => {
            let mut i = 1;
            loop {
                match *s.get(i)? {
                    b'\\' => i += 2,
                    b'"' => break i + 1,
                    _ => i += 1,
                }
            }
        }
        b'{' | b'[' => {
            // Skip to the matching bracket, minding brackets inside strings.
            let (mut depth, mut i, mut in_string) = (0usize, 0, false);
            loop {
                match (*s.get(i)?, in_string) {
                    (b'\\', true) => i += 1,
                    (b'"', _) => in_string = !in_string,
                    (b'{' | b'[', false) => depth += 1,
                    (b'}' | b']', false) => {
                        depth -= 1;
                        if depth == 0 {
                            break i + 1;
                        }
                    }
                    _ => {}
                }
                i += 1;
            }
        }
        // Numbers, true, false, null: up to the next delimiter.
        _ => s
            .iter()
            .position(|&b| matches!(b, b',' | b'}' | b']') || b.is_ascii_whitespace())
            .unwrap_or(s.len()),
    };
    Some(s.split_at(end))
}

// ==============================================================================
// Outgoing
// ==============================================================================

/// Messages from the watch to Gadgetbridge.
#[derive(Clone, Copy, Format)]
pub enum Outgoing {
    /// Firmware version, shown in Gadgetbridge's device details.
    Version,
    /// Battery level and charging state.
    Status {
        percent: u8,
        charging: bool,
        millivolts: u32,
    },
    /// Ask Gadgetbridge to start (or stop) forwarding phone GPS.
    GpsPower(bool),
    /// Bracket an activity-sample transfer.
    ActivityFetch { done: bool },
    /// One minute of activity.
    Activity {
        utc_minute: i64,
        steps: u16,
        movement: u16,
        /// 0 if not measured, which is also what Gadgetbridge assumes when
        /// `hrm` is missing.
        heart_rate: u8,
        /// Gadgetbridge's `ActivityKind` name (e.g. `LIGHT_SLEEP`); `None`
        /// for plain activity, its default. Its sleep charts, and its
        /// Health Connect export, build sleep sessions from these.
        kind: Option<&'static str>,
    },
    /// Answer an incoming call.
    Call(CallAction),
    /// Control the phone's media player.
    Music(MusicCommand),
}

#[derive(Clone, Copy, Format)]
pub enum CallAction {
    Accept,
    Reject,
    /// Silence the phone's ringer, leaving the call ringing.
    Ignore,
}

#[derive(Clone, Copy, Format)]
pub enum MusicCommand {
    PlayPause,
    Next,
    Previous,
}

impl Outgoing {
    /// Renders the message as one `\r\n`-terminated JSON line.
    pub fn to_line(self) -> String<128> {
        let mut line = String::new();
        let result = match self {
            Self::Version => write!(
                line,
                r#"{{"t":"ver","fw":"ur {}","hw":"PineTime"}}"#,
                env!("CARGO_PKG_VERSION")
            ),
            Self::Status {
                percent,
                charging,
                millivolts,
            } => write!(
                line,
                r#"{{"t":"status","bat":{},"chg":{},"volt":{}.{:02}}}"#,
                percent,
                u8::from(charging),
                millivolts / 1000,
                millivolts % 1000 / 10
            ),
            Self::GpsPower(on) => write!(line, r#"{{"t":"gps_power","status":{}}}"#, on),
            Self::ActivityFetch { done } => write!(
                line,
                r#"{{"t":"actfetch","state":"{}"}}"#,
                if done { "end" } else { "start" }
            ),
            Self::Activity {
                utc_minute,
                steps,
                movement,
                heart_rate,
                kind,
            } => write!(
                line,
                r#"{{"t":"act","ts":{},"stp":{},"mov":{},"hrm":{}"#,
                utc_minute * 60_000,
                steps,
                movement,
                heart_rate
            )
            .and_then(|()| match kind {
                Some(kind) => write!(line, r#","act":"{}"}}"#, kind),
                None => write!(line, "}}"),
            }),
            // Gadgetbridge upper-cases these and looks them up in its call
            // and media event enums.
            Self::Call(action) => write!(
                line,
                r#"{{"t":"call","n":"{}"}}"#,
                match action {
                    CallAction::Accept => "accept",
                    CallAction::Reject => "reject",
                    CallAction::Ignore => "ignore",
                }
            ),
            Self::Music(command) => write!(
                line,
                r#"{{"t":"music","n":"{}"}}"#,
                match command {
                    MusicCommand::PlayPause => "playpause",
                    MusicCommand::Next => "next",
                    MusicCommand::Previous => "previous",
                }
            ),
        };
        result
            .and_then(|()| line.push_str("\r\n").map_err(|_| core::fmt::Error))
            .expect("every message fits in 128 bytes");
        debug!("gb: sending {}", line.as_str());
        line
    }
}

/// Decodes standard base64 (padding optional, anything else skipped).
fn decode_base64(text: &[u8]) -> impl Iterator<Item = u8> + '_ {
    let value = |c: u8| match c {
        b'A'..=b'Z' => Some(c - b'A'),
        b'a'..=b'z' => Some(c - b'a' + 26),
        b'0'..=b'9' => Some(c - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    };
    let mut sextets = text.iter().filter_map(move |&c| value(c));
    let mut pending: u32 = 0;
    let mut bits = 0;
    core::iter::from_fn(move || {
        while bits < 8 {
            pending = (pending << 6) | u32::from(sextets.next()?);
            bits += 6;
        }
        bits -= 8;
        Some((pending >> bits) as u8)
    })
}
