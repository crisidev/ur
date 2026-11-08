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
//! \x10GB({"t":"notify","id":17,"src":"Signal","title":"Alice",…})
//! \x10setTime(1762436800);E.setTimeZone(1.0);(s=>s&&…)(…)
//! ```
//!
//! The second is JavaScript for a real Bangle.js; we recognise its shape and
//! pull out the UTC timestamp and the UTC offset (hours, possibly fractional).
//!
//! **Watch → phone** is lines of plain JSON ending in `\r\n`. (Gadgetbridge
//! drops the character before each `\n`, expecting a `\r`.)
//!
//! This module is the whole protocol, and hardware-free: [`LineBuffer`]
//! reassembles lines from what the UART receives, [`parse`] says what each
//! means ([`Incoming`]: a request for [`crate::ble`] to act on, or a
//! [`PhoneEvent`] for the UI), and [`Outgoing`] renders our replies.
//! `ble` only moves the bytes. So the host tests (`host-tests/gb`) can
//! feed it Gadgetbridge's lines as sent, and so can the simulator (`gb`).

use core::fmt::Write as _;

use defmt::{Format, debug, warn};
use heapless::String;

use crate::phone::{PhoneEvent, TextBuilder};
use crate::{alarm, route};

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

/// What a line from the phone means. The first few need the connection
/// (they set the clock, or want a reply), so [`crate::ble`] handles them;
/// [`Incoming::Phone`] is for the UI.
#[derive(Debug, PartialEq)]
pub enum Incoming<'a> {
    /// From `setTime(…);E.setTimeZone(…)`.
    SetTime {
        utc_seconds: i64,
        offset_minutes: i16,
    },
    /// `{"t":"gps",…}`: a location from the phone, and how far off it may
    /// be, in metres, if it says (in `hdop`, which Gadgetbridge fills with
    /// Android's `Location.getAccuracy()`, not a dilution of precision).
    Gps {
        latitude: f64,
        longitude: f64,
        accuracy: Option<f32>,
    },
    /// `{"t":"is_gps_active"}`: Gadgetbridge asking whether we want GPS.
    IsGpsActive,
    /// `{"t":"actfetch","ts":…}`: send activity samples since `ts` (UTC
    /// milliseconds).
    ActivityFetch { since_ms: i64 },
    /// A notification, call, music, find-my-watch, alarm, or navigation
    /// message, translated for the UI (see [`phone_event`]), and whether
    /// its text came with Gadgetbridge's "Text as Bitmaps" images in it
    /// (see [`Images`]), which the UI should tell the wearer to turn off.
    Phone {
        event: PhoneEvent,
        text_as_bitmaps: bool,
    },
    /// Any other `GB(…)` message (calendar, weather, …), by its `t`: we
    /// don't act on it.
    Ignored(&'a str),
}

/// Parses one line (as [`LineBuffer`] hands it over). `None` for lines that
/// aren't one of the two shapes, or lack what their kind needs.
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
            accuracy: object
                .get("hdop")
                .and_then(|v| v.as_f64())
                .map(|m| m as f32),
        },
        "is_gps_active" => Incoming::IsGpsActive,
        "actfetch" => Incoming::ActivityFetch {
            since_ms: object.get("ts").and_then(|v| v.as_f64()).unwrap_or(0.0) as i64,
        },
        _ => {
            let mut text_as_bitmaps = false;
            match phone_event(kind, object, &mut text_as_bitmaps) {
                Some(event) => Incoming::Phone {
                    event,
                    text_as_bitmaps,
                },
                None => Incoming::Ignored(kind),
            }
        }
    })
}

/// Translates the phone's notification, call, music, find-my-watch, alarm,
/// and navigation messages for the UI. Only the fields we show are copied; in
/// particular a notification's body never leaves this function. Sets
/// `bitmaps` if any of its text, shown or not, had images in it (see
/// [`Images`]).
fn phone_event(kind: &str, object: Object<'_>, bitmaps: &mut bool) -> Option<PhoneEvent> {
    fn text<const N: usize>(object: Object<'_>, key: &str, bitmaps: &mut bool) -> String<N> {
        let mut out = String::new();
        if let Some(value) = object.get(key) {
            *bitmaps |= value.decode_into(&mut out);
        }
        out
    }
    /// The text of a notification, as it's kept to show (see
    /// [`TextBuilder`]).
    fn shown<const N: usize>(object: Object<'_>, key: &str, bitmaps: &mut bool) -> TextBuilder<N> {
        let mut out = TextBuilder::new();
        if let Some(value) = object.get(key) {
            *bitmaps |= value.decode(&mut |c| out.push(c));
        }
        out
    }
    Some(match kind {
        "notify" => {
            // What isn't shown is still read, for images: Gadgetbridge
            // renders the subject and body the same way as the title.
            for key in ["subject", "body"] {
                if let Some(value) = object.get(key) {
                    *bitmaps |= value.decode(&mut |_| {});
                }
            }
            // The title is who it's from, in chats (the contact, or the
            // group); Gadgetbridge's `sender` is set by fewer apps, and
            // then often to something else (an SMS's number, with the
            // contact's name as the title). So the title, shown as it
            // comes, whatever app it's from, and `sender` only when the
            // title is empty: not when it was only emoji, which shows
            // nothing (see `TextBuilder`).
            let title: TextBuilder<40> = shown(object, "title", bitmaps);
            let sender: TextBuilder<40> = shown(object, "sender", bitmaps);
            let who = if title.blank() { sender } else { title };
            PhoneEvent::Notification {
                app: shown(object, "src", bitmaps).finish(),
                who: who.finish(),
            }
        }
        "call" => match object.get("cmd").and_then(|v| v.as_raw_str()) {
            Some("incoming") => {
                let name: String<40> = text(object, "name", bitmaps);
                let caller = if name.is_empty() {
                    text(object, "number", bitmaps)
                } else {
                    name
                };
                PhoneEvent::IncomingCall { caller }
            }
            // Answered, rejected, ended, …: the call no longer needs us.
            _ => PhoneEvent::CallOver,
        },
        "musicinfo" => PhoneEvent::MusicInfo {
            artist: text(object, "artist", bitmaps),
            track: text(object, "track", bitmaps),
        },
        "musicstate" => PhoneEvent::MusicState {
            playing: object.get("state").and_then(|v| v.as_raw_str()) == Some("play"),
        },
        // Gadgetbridge's answer to an intent (a button grid press) when
        // its "Allow Intents" setting is off: `{"t":"intent","err":"…"}`.
        // It never sends one otherwise.
        "intent" if object.get("err").is_some() => PhoneEvent::IntentsOff,
        "find" => PhoneEvent::Find {
            on: object.get("n").and_then(|v| v.as_bool()) == Some(true),
        },
        // `{"t":"alarm","d":[{"h":7,"m":30,"rep":31,"on":true},…]}`: every
        // alarm set in Gadgetbridge's alarm screen, whenever one changes
        // there (see `onSetAlarms` in its driver). `rep` has a bit per
        // weekday, Monday first; 0 is once.
        "alarm" => {
            let mut alarms = alarm::Phone::default();
            let items = object.get("d").map(|d| d.items()).into_iter().flatten();
            for alarm in items.filter_map(|v| v.as_object()) {
                let number = |key| alarm.get(key).and_then(|v| v.as_f64());
                let (Some(hour), Some(minute)) = (number("h"), number("m")) else {
                    continue;
                };
                let days = number("rep").unwrap_or(0.0) as u8;
                // A Bangle.js takes a missing `on` as on.
                let on = alarm.get("on").and_then(|v| v.as_bool()) != Some(false);
                if !alarms.push(hour as u16, minute as u16, days, on) {
                    warn!("gb: dropped an alarm ({}:{})", hour as u16, minute as u16);
                }
            }
            PhoneEvent::Alarms(alarms)
        }
        // `{"t":"nav","instr":"Storgata","distance":"150 m","action":"left",
        // "eta":"11:55"}`: the next turn while the phone navigates, every
        // field optional; `{"t":"nav"}` when navigation ends (see `route`).
        "nav" => {
            // Decoded at a size `text` is already used with (each size is
            // another copy of it), then copied over, with
            // Google Maps' no-break spaces (between number and unit), which
            // the font lacks, as plain ones.
            fn spaced<const N: usize>(
                object: Object<'_>,
                key: &str,
                bitmaps: &mut bool,
            ) -> String<N> {
                let decoded: String<40> = text(object, key, bitmaps);
                let mut out: String<N> = String::new();
                for c in decoded.chars() {
                    let c = if c == '\u{a0}' { ' ' } else { c };
                    // Trimmed, in case they come padded.
                    if c != ' ' || !out.is_empty() {
                        let _ = out.push(c);
                    }
                }
                while out.ends_with(' ') {
                    out.pop();
                }
                out
            }
            let street = spaced::<{ route::STREET_LEN }>(object, "instr", bitmaps);
            let distance = spaced::<{ route::DISTANCE_LEN }>(object, "distance", bitmaps);
            let action = object.get("action").and_then(|v| v.as_raw_str());
            let navigating = !street.is_empty() || !distance.is_empty() || action.is_some();
            PhoneEvent::Route(navigating.then(|| route::Step {
                turn: route::Turn::from_action(action),
                street,
                distance,
            }))
        }
        _ => return None,
    })
}

/// The times [`parse_set_time`] believes: 2020-01-01 to 2100-01-01 UTC.
const SANE_UTC_SECONDS: core::ops::Range<i64> = 1_577_836_800..4_102_444_800;

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
    // A time no phone could mean is refused rather than believed: the
    // clock, the activity history's minutes, and the alarms all build on
    // it, and the arithmetic there assumes a sane date. Offsets stop at
    // ±18 hours, as Java's (and so Gadgetbridge's) do.
    if !SANE_UTC_SECONDS.contains(&utc_seconds) || offset_minutes.unsigned_abs() > 18 * 60 {
        warn!("gb: refused an implausible time");
        return None;
    }
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
    /// Returns whether it carried images (see [`Images`]), which are left
    /// out, a `?` in each one's place.
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
    pub fn decode_into<const N: usize>(&self, out: &mut String<N>) -> bool {
        // Past the first character that doesn't fit, nothing more is
        // shown, but the rest is still read, for images further on.
        let mut full = false;
        self.decode(&mut |c| {
            if !full && out.push(c).is_err() {
                full = true;
            }
        })
    }

    /// [`Value::decode_into`]'s work, handing each character to `show`:
    /// one copy of the decoder, rather than one for each size of string
    /// it fills (~1 KB of flash each).
    fn decode(&self, show: &mut dyn FnMut(char)) -> bool {
        let mut images = Images::default();
        // Line breaks and tabs would only mess up one-line labels.
        let mut text = |c: char| {
            if let Some(c) = images.strip(c) {
                show(if ('\u{8}'..='\u{d}').contains(&c) {
                    ' '
                } else {
                    c
                });
            }
        };
        if let Some(base64) = self
            .0
            .strip_prefix(b"atob(\"")
            .and_then(|v| v.strip_suffix(b"\")"))
        {
            for byte in decode_base64(base64) {
                text(char::from(byte));
            }
            return images.found;
        }

        let Some(inner) = self
            .0
            .strip_prefix(b"\"")
            .and_then(|v| v.strip_suffix(b"\""))
        else {
            return false;
        };
        let mut bytes = inner.iter().copied().peekable();
        let hex = |bytes: &mut core::iter::Peekable<_>, digits: usize| -> Option<u32> {
            let text: heapless::Vec<u8, 4> = bytes.by_ref().take(digits).collect();
            u32::from_str_radix(core::str::from_utf8(&text).ok()?, 16).ok()
        };
        // Up to the end, or to an escape that doesn't parse (a line cut
        // short), showing what came before it.
        let mut next = || -> Option<char> {
            let byte = bytes.next()?;
            Some(if byte != b'\\' {
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
                            // A high half without its low half (a corrupt
                            // line) is just another character we can't show,
                            // rather than arithmetic that wraps round (or
                            // panics, in the simulator's debug builds).
                            low.checked_sub(0xDC00)
                                .filter(|low| *low < 0x400)
                                .and_then(|low| {
                                    char::from_u32(0x10000 + ((unit - 0xD800) << 10) + low)
                                })
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
                    // As the characters they stand for: inside an image,
                    // they're its bytes.
                    b'b' => '\u{8}',
                    b't' => '\t',
                    b'n' => '\n',
                    b'v' => '\u{b}',
                    b'f' => '\u{c}',
                    b'r' => '\r',
                    other => char::from(other), // \" \\ \/
                }
            })
        };
        while let Some(c) = next() {
            text(c);
        }
        images.found
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
            let after = after.strip_prefix(b",").unwrap_or(after);
            // An item that takes up nothing (a stray `}` where one should
            // start) would come round again forever, stalling the
            // Bluetooth task until the watchdog reset the watch: it ends
            // what parses instead.
            if after.len() == here.len() {
                return None;
            }
            rest = Some(after);
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

/// Gadgetbridge's "Text as Bitmaps" (a per-device setting for a
/// Bangle.js, off by default; `renderUnicodeAsImage` in its driver) draws
/// each word with characters beyond Latin-1 on the phone, and sends it as
/// an image in the text: a `\0`, then the image as Espruino draws it
/// (`bitmapToEspruinoArray`): its width, its height, its bits per pixel
/// (plus 128 if one colour is transparent, then that colour), and its
/// pixels, packed, a byte for each 8 bits. A Bangle.js draws them inline;
/// we have no use for a picture of a word in some other font, so this
/// leaves them out, with a `?` where each was (as for the characters the
/// font lacks), and says it saw one, so the UI can say how to turn it off
/// (README FAQ N02).
#[derive(Default)]
struct Images {
    state: Image,
    /// Whether the text had an image in it.
    found: bool,
}

/// Where [`Images`] is in the text.
#[derive(Default)]
enum Image {
    #[default]
    Outside,
    /// After the `\0`: the header's bytes read so far (width, height, bits
    /// per pixel), and how many.
    Header([u8; 3], usize),
    /// The pixels' bytes still to come, after the transparent colour if
    /// there is one.
    Pixels { left: u32, transparent: bool },
}

impl Images {
    /// One decoded character in; what to show of it out.
    fn strip(&mut self, c: char) -> Option<char> {
        // In an image, each character is a byte (Latin-1).
        let byte = c as u32 as u8;
        match &mut self.state {
            Image::Outside if c == '\0' => {
                self.found = true;
                self.state = Image::Header([0; 3], 0);
                return Some('?');
            }
            Image::Outside => return Some(c),
            Image::Header(header, read) => {
                header[*read] = byte;
                *read += 1;
                if *read == header.len() {
                    let [width, height, bpp] = *header;
                    let bits = u32::from(width) * u32::from(height) * u32::from(bpp & 0x7F);
                    self.state = Image::Pixels {
                        left: bits.div_ceil(8),
                        transparent: bpp & 0x80 != 0,
                    };
                }
            }
            Image::Pixels { transparent, .. } if *transparent => *transparent = false,
            Image::Pixels { left, .. } => *left -= 1,
        }
        if let Image::Pixels {
            left: 0,
            transparent: false,
        } = self.state
        {
            self.state = Image::Outside;
        }
        None
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
    /// A press on the button grid (see [`crate::buttons`]), as an Android
    /// broadcast intent for Tasker and the like: Gadgetbridge sends any
    /// `intent` message on as one, if its "Allow Intents" setting is on
    /// (and answers [`PhoneEvent::IntentsOff`] if not). Action
    /// `ur.watch.BUTTON`, with the extras `button` (the code, a number)
    /// and `gesture` (`tap`, `double_tap`, or `hold`).
    Button { code: u8, gesture: &'static str },
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
            Self::Button { code, gesture } => write!(
                line,
                r#"{{"t":"intent","action":"ur.watch.BUTTON","extra":{{"button":{},"gesture":"{}"}}}}"#,
                code, gesture
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
