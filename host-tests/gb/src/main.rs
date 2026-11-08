//! Host tests for the firmware's `gb` module: Gadgetbridge's Bangle.js
//! protocol, fed lines shaped exactly as Gadgetbridge sends them (from its
//! `BangleJSDeviceSupport.java`, and a real Gadgetbridge log), with
//! fictional contents. `cargo test` runs them.

// `gb` and what it uses, at the crate root where their `crate::…` paths
// expect each other.
#[path = "../../../src/alarm.rs"]
#[allow(dead_code)]
mod alarm;
#[path = "../../../src/decimal.rs"]
mod decimal;
#[path = "../../../src/gb.rs"]
#[allow(dead_code)]
mod gb;
#[path = "../../../src/phone.rs"]
#[allow(dead_code)]
mod phone;
#[path = "../../../src/route.rs"]
#[allow(dead_code)]
mod route;

// `gb` logs through defmt, whose binary log needs the firmware's ELF to
// decode, and the host has nowhere to send it anyway: its logs go nowhere
// (as in the simulator).
#[defmt::global_logger]
struct Logger;

// SAFETY: Does nothing, so there's nothing to synchronise.
unsafe impl defmt::Logger for Logger {
    fn acquire() {}
    unsafe fn flush() {}
    unsafe fn release() {}
    unsafe fn write(_bytes: &[u8]) {}
}

defmt::timestamp!("");

fn main() {
    println!("run `cargo test`");
}

#[cfg(test)]
mod tests {
    use quickcheck::{QuickCheck, TestResult};

    use super::alarm::{PhoneAlarm, Switch};
    use super::gb::{self, Incoming, LineBuffer};
    use super::phone::PhoneEvent;
    use super::route::{Step, Turn};

    // ==========================================================================
    // Helpers
    // ==========================================================================

    /// Parses a line as `LineBuffer` hands it over (no `\x10`, no `\n`).
    fn parse(line: &[u8]) -> Option<Incoming<'_>> {
        gb::parse(line)
    }

    /// The UI event a line becomes; panics if it becomes anything else.
    fn event(line: &[u8]) -> PhoneEvent {
        match parse(line) {
            Some(Incoming::Phone(event)) => event,
            _ => panic!("not a UI event: {}", String::from_utf8_lossy(line)),
        }
    }

    fn s<const N: usize>(text: &str) -> heapless::String<N> {
        text.try_into().expect("test strings fit")
    }

    /// `text` cut to what fits in `N` bytes of UTF-8, at a character
    /// boundary, as the firmware's fixed-size strings cut it.
    fn fit<const N: usize>(text: &str) -> String {
        let mut out = String::new();
        for c in text.chars() {
            if out.len() + c.len_utf8() > N {
                break;
            }
            out.push(c);
        }
        out
    }

    /// Gadgetbridge's `jsonToStringInternal` for a string, ported: how it
    /// spells `text` in a line for a Bangle.js, as the ISO-8859-1 bytes on
    /// the wire. JavaScript-ish rather than JSON: `\xHH` for control
    /// characters and the first bytes of UTF-8 sequences (0xC2–0xF4, which
    /// covers å, æ, ø, é, …), `\uHHHH` per UTF-16 unit beyond Latin-1,
    /// octal for the lowest control characters, raw Latin-1 for the rest;
    /// or, when that's longer than base64, `atob("…")` of the Latin-1
    /// bytes.
    fn gadgetbridge_string(text: &str) -> Vec<u8> {
        let units: Vec<u16> = text.encode_utf16().collect();
        let mut json = b"\"".to_vec();
        let mut has_unicode = false;
        for (i, &unit) in units.iter().enumerate() {
            let ch = u32::from(unit);
            let next = units.get(i + 1).map_or(0, |&u| u32::from(u));
            has_unicode |= ch > 255;
            let escaped = match ch {
                // A digit after `\N` would read as more octal.
                0..8 if (u32::from(b'0')..=u32::from(b'7')).contains(&next) => {
                    format!("\\x0{ch}")
                }
                0..8 => format!("\\{ch}"),
                8 => "\\b".into(),
                9 => "\\t".into(),
                10 => "\\n".into(),
                11 => "\\v".into(),
                12 => "\\f".into(),
                16 => "\\20".into(),
                34 => "\\\"".into(),
                92 => "\\\\".into(),
                c if c < 32 || c == 127 || c == 173 || (0xC2..=0xF4).contains(&c) => {
                    format!("\\x{c:02x}")
                }
                c if c > 255 => format!("\\u{c:04x}"),
                c => {
                    json.push(c as u8);
                    continue;
                }
            };
            json.extend_from_slice(escaped.as_bytes());
        }
        // Java compares lengths in UTF-16 units, before the closing quote.
        if !has_unicode && json.len() > 5 + units.len() * 4 / 3 {
            let latin1: Vec<u8> = text.chars().map(|c| c as u8).collect();
            return format!("atob(\"{}\")", base64(&latin1)).into_bytes();
        }
        json.push(b'"');
        json
    }

    fn base64(bytes: &[u8]) -> String {
        const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for chunk in bytes.chunks(3) {
            let b = [
                chunk[0],
                *chunk.get(1).unwrap_or(&0),
                *chunk.get(2).unwrap_or(&0),
            ];
            let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
            for i in 0..4 {
                if i <= chunk.len() {
                    out.push(char::from(ALPHABET[(n >> (18 - 6 * i) & 63) as usize]));
                } else {
                    out.push('=');
                }
            }
        }
        out
    }

    /// A notification as Gadgetbridge's `onNotification` sends it, every
    /// field present, `title` and `sender` spelled as it spells them.
    fn notify_line(src: &str, title: &str, sender: &str) -> Vec<u8> {
        let mut line = br#"GB({"t":"notify","id":1762436801,"src":"#.to_vec();
        line.extend(gadgetbridge_string(src));
        line.extend(br#","title":"#);
        line.extend(gadgetbridge_string(title));
        line.extend(br#","subject":"","body":"Are we still on for Saturday?","sender":"#);
        line.extend(gadgetbridge_string(sender));
        line.extend(br#","tel":"","reply":true})"#);
        line
    }

    // ==========================================================================
    // Lines: reassembly from the UART's writes
    // ==========================================================================

    /// Two lines as one stream, split where Gadgetbridge splits them (at
    /// its idea of the MTU, here 20 bytes), and elsewhere.
    #[test]
    fn lines_are_reassembled_from_any_split() {
        let stream = b"\x10setTime(1794150000);E.setTimeZone(1.0);(s=>s&&(s.timezone=1.0,require('Storage').write('setting.json',s)))(require('Storage').readJSON('setting.json',1))\n\x10GB({\"t\":\"is_gps_active\"})\n";
        for chunk in [1, 3, 20, 64, 244, stream.len()] {
            let mut buffer = LineBuffer::new();
            let mut lines: Vec<Vec<u8>> = Vec::new();
            for piece in stream.chunks(chunk) {
                buffer.push(piece, |line| lines.push(line.to_vec()));
            }
            assert_eq!(lines.len(), 2, "chunks of {chunk}");
            assert!(lines[0].starts_with(b"setTime(1794150000);"), "{chunk}");
            assert_eq!(lines[1], br#"GB({"t":"is_gps_active"})"#, "{chunk}");
        }
    }

    #[test]
    fn line_ends_and_markers_are_stripped() {
        let mut buffer = LineBuffer::new();
        let mut lines: Vec<Vec<u8>> = Vec::new();
        // A trailing `\r`, a doubled `\x10`, and a line with neither.
        buffer.push(
            b"\x10GB({\"t\":\"find\",\"n\":true})\r\n\x10\x10ab\ncd\n",
            |line| lines.push(line.to_vec()),
        );
        assert_eq!(
            lines,
            [
                br#"GB({"t":"find","n":true})"#.to_vec(),
                b"ab".to_vec(),
                b"cd".to_vec()
            ]
        );
    }

    #[test]
    fn overlong_lines_are_dropped_whole() {
        let mut buffer = LineBuffer::new();
        let mut lines: Vec<Vec<u8>> = Vec::new();
        let long = vec![b'x'; 3000];
        buffer.push(&long, |line| lines.push(line.to_vec()));
        buffer.push(b"tail\nGB({\"t\":\"find\",\"n\":false})\n", |line| {
            lines.push(line.to_vec())
        });
        // The long line, tail and all, is gone; the next survives.
        assert_eq!(lines, [br#"GB({"t":"find","n":false})"#.to_vec()]);
    }

    #[test]
    fn clearing_forgets_a_partial_line() {
        let mut buffer = LineBuffer::new();
        let mut lines: Vec<Vec<u8>> = Vec::new();
        buffer.push(b"\x10GB({\"t\":\"noti", |line| lines.push(line.to_vec()));
        // The connection dropped mid-line; the next one starts afresh.
        buffer.clear();
        buffer.push(b"\x10GB({\"t\":\"is_gps_active\"})\n", |line| {
            lines.push(line.to_vec())
        });
        assert_eq!(lines, [br#"GB({"t":"is_gps_active"})"#.to_vec()]);
    }

    // ==========================================================================
    // setTime
    // ==========================================================================

    /// Gadgetbridge's `transmitTime`: the time, the zone, and JavaScript
    /// saving the zone in the Bangle.js's settings, which we skip.
    fn set_time_line(seconds: i64, zone: &str) -> Vec<u8> {
        format!(
            "setTime({seconds});E.setTimeZone({zone});(s=>s&&(s.timezone={zone},require('Storage').write('setting.json',s)))(require('Storage').readJSON('setting.json',1))"
        )
        .into_bytes()
    }

    #[test]
    fn set_time_with_its_zone() {
        // Java's `Float.toString` of the offset in hours, as Gadgetbridge
        // writes it, and the minutes it means.
        let zones = [
            ("1.0", 60),
            ("2.0", 120),
            ("0.0", 0),
            ("-5.0", -300),
            ("5.5", 330),   // India
            ("5.75", 345),  // Nepal
            ("-3.5", -210), // Newfoundland
            ("12.75", 765), // Chatham Islands
            ("-9.5", -570), // Marquesas
            ("14.0", 840),  // Line Islands
        ];
        for (zone, minutes) in zones {
            assert_eq!(
                parse(&set_time_line(1794150000, zone)),
                Some(Incoming::SetTime {
                    utc_seconds: 1794150000,
                    offset_minutes: minutes
                }),
                "{zone}"
            );
        }
    }

    #[test]
    fn set_time_without_the_settings_script() {
        assert_eq!(
            parse(b"setTime(1794150000);E.setTimeZone(1.0);"),
            Some(Incoming::SetTime {
                utc_seconds: 1794150000,
                offset_minutes: 60
            })
        );
    }

    #[test]
    fn set_time_needs_both_numbers() {
        for line in [
            &b"setTime(1794150000);"[..],
            b"setTime();E.setTimeZone(1.0);",
            b"setTime(soon);E.setTimeZone(1.0);",
            b"setTime(1794150000);E.setTimeZone();",
            b"setTime(1794150000);E.setTimeZone(CET);",
            b"setTime(1794150000",
            b"setTime(1794150000);E.setTimeZone(1.0",
        ] {
            assert_eq!(parse(line), None, "{}", String::from_utf8_lossy(line));
        }
    }

    // ==========================================================================
    // notify
    // ==========================================================================

    #[test]
    fn a_message_from_a_messaging_app() {
        // Messaging apps put the sender in the title, and leave `sender`
        // empty.
        let line = br#"GB({"t":"notify","id":1762436801,"src":"Signal","title":"Kari Nordmann","subject":"","body":"Are we still on for Saturday?","sender":"","tel":"","reply":true})"#;
        assert_eq!(
            event(line),
            PhoneEvent::Notification {
                app: s("Signal"),
                who: s("Kari Nordmann")
            }
        );
    }

    #[test]
    fn the_sender_wins_over_the_title() {
        let line = br#"GB({"t":"notify","id":5,"src":"SMS Message","title":"New message","subject":"","body":"Your parcel has shipped","sender":"Posten","tel":"+4799999999"})"#;
        assert_eq!(
            event(line),
            PhoneEvent::Notification {
                app: s("SMS Message"),
                who: s("Posten")
            }
        );
    }

    #[test]
    fn a_notification_with_actions_and_nothing_to_show() {
        // `actions` is an array of objects, which the reader steps over; no
        // title or sender leaves `who` empty.
        let line = br#"GB({"t":"notify","id":9,"src":"Calendar","title":"","subject":"","body":"Standup [room 4]","sender":"","tel":"","actions":[{"id":-1234,"name":"Snooze","reply":false},{"id":-1235,"name":"Mark \"done\"","reply":false}]})"#;
        assert_eq!(
            event(line),
            PhoneEvent::Notification {
                app: s("Calendar"),
                who: s("")
            }
        );
    }

    #[test]
    fn javascript_escapes() {
        let cases: &[(&[u8], &str)] = &[
            // Latin-1 letters in the 0xC2–0xF4 range, escaped.
            (br#""\xc5se \xd8yvind""#, "Åse Øyvind"),
            (br#""Ren\xe9e""#, "Renée"),
            // Latin-1 outside it, raw (here `ø`, 0xF8, and `©`, 0xA9).
            (b"\"Bj\xf8rn \xa9\"", "Bjørn ©"),
            // Beyond Latin-1: one `\u` per UTF-16 unit.
            (br#""\u0141ukasz \u20ac""#, "Łukasz €"),
            // An emoji, as a surrogate pair.
            (br#""Party \ud83c\udf89""#, "Party 🎉"),
            // Quotes and backslashes.
            (br#""\"Ola\" \\ Kari""#, "\"Ola\" \\ Kari"),
            // Line breaks and tabs become spaces, for one-line labels.
            (br#""Ola\nKari\tNordmann""#, "Ola Kari Nordmann"),
            (br#""a\bb\vc\fd""#, "a b c d"),
            // Octal: `\0`, Gadgetbridge's `\20` for DLE, and `\x0N` where a
            // digit follows.
            (br#""a\0b\20c\x035""#, "a\0b\u{10}c\u{3}5"),
        ];
        for &(spelled, text) in cases {
            let mut line = br#"GB({"t":"notify","id":1,"src":"Signal","title":"#.to_vec();
            line.extend_from_slice(spelled);
            line.extend_from_slice(br#","sender":""})"#);
            let PhoneEvent::Notification { who, .. } = event(&line) else {
                panic!("a notification");
            };
            assert_eq!(who, text, "{}", String::from_utf8_lossy(spelled));
        }
    }

    /// Gadgetbridge sends notification text as Android hands it over:
    /// plain text, not HTML, so `&` and `<` arrive as themselves and there
    /// are no entities to decode (an `&amp;` would be the app's own, and
    /// show on the phone too). Smart quotes and dashes arrive as `\u`
    /// escapes (Gadgetbridge only swaps them for ASCII when it renders
    /// text as images), for the font to draw as their ASCII look-alikes.
    #[test]
    fn symbols_arrive_as_they_are() {
        for title in [
            "Tom & Jerry <3",
            "Don't \"panic\" #1 @ben?",
            "!\"#$%&'()*+,-./:;<=>?@[\\]^_`{|}~",
            "\u{201c}Don\u{2019}t\u{201d} \u{2013} Ida\u{2026}",
            "AT&amp;T",
        ] {
            let line = notify_line("Mail", title, "");
            let PhoneEvent::Notification { who, .. } = event(&line) else {
                panic!("a notification");
            };
            assert_eq!(who, title, "{}", String::from_utf8_lossy(&line));
        }
        // As Gadgetbridge spells them (see `gadgetbridge_string`).
        assert_eq!(
            gadgetbridge_string("\u{201c}Don\u{2019}t\u{201d}"),
            br#""\u201cDon\u2019t\u201d""#
        );
    }

    #[test]
    fn base64_strings() {
        // Gadgetbridge sends `atob("…")` where escaping costs more: names
        // heavy in accented letters. "ÆØÅ æøå" in ISO-8859-1.
        let spelled = gadgetbridge_string("ÆØÅ æøå");
        assert!(spelled.starts_with(b"atob(\""), "{spelled:?}");
        assert_eq!(spelled, br#"atob("xtjFIOb45Q==")"#);
        let line = notify_line("Signal", "", "ÆØÅ æøå");
        assert_eq!(
            event(&line),
            PhoneEvent::Notification {
                app: s("Signal"),
                who: s("ÆØÅ æøå")
            }
        );
        // Without padding, too.
        let line = br#"GB({"t":"notify","src":"Signal","title":atob("xtjF")})"#;
        let PhoneEvent::Notification { who, .. } = event(line) else {
            panic!("a notification");
        };
        assert_eq!(who, "ÆØÅ");
    }

    #[test]
    fn long_fields_are_cut_at_a_character() {
        // 40 bytes of sender: 20 two-byte letters, the 21st cut.
        let name = "Øyvind Åsmund Ødegård Ærlig Østby Æsøy";
        let line = notify_line("A messaging app with a long name", "", name);
        let PhoneEvent::Notification { app, who } = event(&line) else {
            panic!("a notification");
        };
        assert_eq!(app.as_str(), fit::<24>("A messaging app with a long name"));
        assert_eq!(who.as_str(), fit::<40>(name));
        assert!(who.len() <= 40);
    }

    /// The decoder against Gadgetbridge's encoder (ported above), for any
    /// text: what reaches the screen is the text, but for line breaks and
    /// tabs (spaces, unless base64 carried them), cut to the field's size.
    #[test]
    fn any_text_survives_the_trip() {
        fn property(sender: String, src: String) -> TestResult {
            // Gadgetbridge's `\20` for DLE runs into a following octal digit
            // (JavaScript reads `\205`): its bug, which the watch can't undo.
            let dle_digit = |t: &str| {
                t.chars()
                    .zip(t.chars().skip(1))
                    .any(|(a, b)| a == '\u{10}' && ('0'..='7').contains(&b))
            };
            // An empty sender falls back to the title, tested elsewhere.
            if sender.is_empty() || dle_digit(&sender) || dle_digit(&src) {
                return TestResult::discard();
            }
            let shown = |text: &str| -> String {
                if gadgetbridge_string(text).starts_with(b"atob(") {
                    return text.into();
                }
                text.chars()
                    .map(|c| match c {
                        '\u{8}' | '\t' | '\n' | '\u{b}' | '\u{c}' => ' ',
                        c => c,
                    })
                    .collect()
            };
            let line = notify_line(&src, "title", &sender);
            let PhoneEvent::Notification { app, who } = event(&line) else {
                return TestResult::failed();
            };
            TestResult::from_bool(
                who.as_str() == fit::<40>(&shown(&sender))
                    && app.as_str() == fit::<24>(&shown(&src)),
            )
        }
        QuickCheck::new()
            .tests(5000)
            .quickcheck(property as fn(String, String) -> TestResult);
    }

    // ==========================================================================
    // call
    // ==========================================================================

    #[test]
    fn an_incoming_call_names_the_caller() {
        let line =
            br#"GB({"t":"call","cmd":"incoming","name":"Kari Nordmann","number":"+4799999999"})"#;
        assert_eq!(
            event(line),
            PhoneEvent::IncomingCall {
                caller: s("Kari Nordmann")
            }
        );
        // No contact: Gadgetbridge leaves `name` out (Java's `put` of null).
        let line = br#"GB({"t":"call","cmd":"incoming","number":"+4799999999"})"#;
        assert_eq!(
            event(line),
            PhoneEvent::IncomingCall {
                caller: s("+4799999999")
            }
        );
        // Withheld: neither.
        let line = br#"GB({"t":"call","cmd":"incoming"})"#;
        assert_eq!(event(line), PhoneEvent::IncomingCall { caller: s("") });
    }

    #[test]
    fn every_other_call_command_ends_the_call() {
        // `CallSpec.CALL_*`, lower-cased, as Gadgetbridge sends them.
        for cmd in [
            "undefined",
            "accept",
            "outgoing",
            "reject",
            "start",
            "end",
            "ignore",
            "",
        ] {
            let line = format!(
                r#"GB({{"t":"call","cmd":"{cmd}","name":"Kari Nordmann","number":"+4799999999"}})"#
            );
            assert_eq!(event(line.as_bytes()), PhoneEvent::CallOver, "{cmd}");
        }
    }

    // ==========================================================================
    // musicinfo, musicstate
    // ==========================================================================

    #[test]
    fn music_info() {
        let line = br#"GB({"t":"musicinfo","artist":"Ida Moreau","album":"Late Trains","track":"Night Signals","dur":215,"c":12,"n":3})"#;
        assert_eq!(
            event(line),
            PhoneEvent::MusicInfo {
                artist: s("Ida Moreau"),
                track: s("Night Signals")
            }
        );
        // A podcast app with no artist: left out.
        let line = br#"GB({"t":"musicinfo","track":"Episode 12: Tides","dur":3600,"c":0,"n":0})"#;
        assert_eq!(
            event(line),
            PhoneEvent::MusicInfo {
                artist: s(""),
                track: s("Episode 12: Tides")
            }
        );
    }

    #[test]
    fn music_state() {
        // As in a real Gadgetbridge log: `shuffle` and `repeat` -1 when
        // unknown.
        for (state, playing) in [
            ("play", true),
            ("pause", false),
            ("stop", false),
            ("", false),
        ] {
            let line = format!(
                r#"GB({{"t":"musicstate","state":"{state}","position":155,"shuffle":-1,"repeat":-1}})"#
            );
            assert_eq!(
                event(line.as_bytes()),
                PhoneEvent::MusicState { playing },
                "{state}"
            );
        }
    }

    // ==========================================================================
    // find
    // ==========================================================================

    #[test]
    fn find_my_watch() {
        assert_eq!(
            event(br#"GB({"t":"find","n":true})"#),
            PhoneEvent::Find { on: true }
        );
        assert_eq!(
            event(br#"GB({"t":"find","n":false})"#),
            PhoneEvent::Find { on: false }
        );
        // Anything but `true` stops it.
        assert_eq!(
            event(br#"GB({"t":"find"})"#),
            PhoneEvent::Find { on: false }
        );
        assert_eq!(
            event(br#"GB({"t":"find","n":1})"#),
            PhoneEvent::Find { on: false }
        );
    }

    // ==========================================================================
    // alarm
    // ==========================================================================

    fn alarms(line: &[u8]) -> Vec<PhoneAlarm> {
        match event(line) {
            PhoneEvent::Alarms(alarms) => alarms.alarms().to_vec(),
            other => panic!("not alarms: {other:?}"),
        }
    }

    fn alarm(hour: u16, minute: u16, days: u8, switch: Switch) -> PhoneAlarm {
        PhoneAlarm {
            time: hour * 60 + minute,
            days,
            switch,
        }
    }

    #[test]
    fn alarms_with_their_weekdays() {
        // `rep` is Gadgetbridge's `Alarm.ALARM_MON` (1) … `ALARM_SUN` (64).
        let line = br#"GB({"t":"alarm","d":[{"h":6,"m":45,"rep":31,"on":true},{"h":9,"m":0,"rep":96,"on":false},{"h":7,"m":5,"rep":0,"on":true},{"h":21,"m":30,"rep":127,"on":true},{"h":12,"m":0,"rep":42,"on":true}]})"#;
        assert_eq!(
            alarms(line),
            [
                alarm(6, 45, 0b001_1111, Switch::On),  // weekdays
                alarm(9, 0, 0b110_0000, Switch::Off),  // weekends
                alarm(7, 5, 0, Switch::On),            // once
                alarm(21, 30, 0b111_1111, Switch::On), // every day
                alarm(12, 0, 0b010_1010, Switch::On),  // Tue, Thu, Sat
            ]
        );
    }

    #[test]
    fn no_alarms() {
        assert_eq!(alarms(br#"GB({"t":"alarm","d":[]})"#), []);
        assert_eq!(alarms(br#"GB({"t":"alarm"})"#), []);
    }

    #[test]
    fn alarms_missing_fields_or_making_no_sense() {
        let line = br#"GB({"t":"alarm","d":[{"h":6,"m":45},{"m":0,"rep":1,"on":true},{"h":25,"m":0,"on":true},{"h":7,"m":60,"on":true},{"h":8,"m":15,"rep":300,"on":true},7,"x"]})"#;
        assert_eq!(
            alarms(line),
            [
                // No `rep`: once; no `on`: on, as a Bangle.js takes it.
                alarm(6, 45, 0, Switch::On),
                // No hour, hour 25, minute 60, and the non-objects: dropped.
                // A `rep` past a byte saturates, and bit 7 means nothing.
                alarm(8, 15, 0x7F, Switch::On),
            ]
        );
    }

    #[test]
    fn at_most_ten_alarms() {
        let items: Vec<String> = (0..12)
            .map(|i| format!(r#"{{"h":{i},"m":0,"rep":0,"on":true}}"#))
            .collect();
        let line = format!(r#"GB({{"t":"alarm","d":[{}]}})"#, items.join(","));
        let kept = alarms(line.as_bytes());
        assert_eq!(kept.len(), 10);
        assert_eq!(kept[9], alarm(9, 0, 0, Switch::On));
    }

    // ==========================================================================
    // nav
    // ==========================================================================

    fn step(line: &[u8]) -> Option<Step> {
        match event(line) {
            PhoneEvent::Route(step) => step,
            other => panic!("not directions: {other:?}"),
        }
    }

    #[test]
    fn navigation_steps() {
        let line = br#"GB({"t":"nav","instr":"Storgata","distance":"150 m","action":"left","eta":"11:55"})"#;
        assert_eq!(
            step(line),
            Some(Step {
                turn: Turn::Left,
                street: s("Storgata"),
                distance: s("150 m"),
            })
        );
    }

    #[test]
    fn every_action_gadgetbridge_names() {
        // Its `navActions`, in `NavigationInfoSpec.ACTION_*` order from 1.
        let actions = [
            ("continue", Turn::Straight),
            ("left", Turn::Left),
            ("left_slight", Turn::SlightLeft),
            ("left_sharp", Turn::SharpLeft),
            ("right", Turn::Right),
            ("right_slight", Turn::SlightRight),
            ("right_sharp", Turn::SharpRight),
            ("keep_left", Turn::KeepLeft),
            ("keep_right", Turn::KeepRight),
            ("uturn_left", Turn::UturnLeft),
            ("uturn_right", Turn::UturnRight),
            ("offroute", Turn::OffRoute),
            ("roundabout_right", Turn::RoundaboutRight),
            ("roundabout_left", Turn::RoundaboutLeft),
            ("roundabout_straight", Turn::RoundaboutStraight),
            ("roundabout_uturn", Turn::RoundaboutUturn),
            ("finish", Turn::Finish),
        ];
        for (action, turn) in actions {
            let line = format!(
                r#"GB({{"t":"nav","instr":"Karl Johans gate","distance":"1,2 km","action":"{action}","eta":"12:10"}})"#
            );
            assert_eq!(
                step(line.as_bytes()).map(|s| s.turn),
                Some(turn),
                "{action}"
            );
        }
    }

    #[test]
    fn navigation_without_some_fields() {
        // No action (Gadgetbridge didn't recognise Google Maps' icon).
        let line = br#"GB({"t":"nav","instr":"Storgata","distance":"150 m","eta":"11:55"})"#;
        assert_eq!(step(line).map(|s| s.turn), Some(Turn::Unknown));
        // Only an action.
        let line = br#"GB({"t":"nav","action":"finish"})"#;
        assert_eq!(
            step(line),
            Some(Step {
                turn: Turn::Finish,
                street: s(""),
                distance: s(""),
            })
        );
    }

    #[test]
    fn navigation_ends() {
        assert_eq!(step(br#"GB({"t":"nav"})"#), None);
        // Fields that are there but empty, or only spaces, say as little.
        assert_eq!(step(br#"GB({"t":"nav","instr":" ","distance":""})"#), None);
    }

    #[test]
    fn navigation_text_is_tidied() {
        // Google Maps' no-break space between number and unit is a raw
        // 0xA0 byte (Latin-1, outside what Gadgetbridge escapes); the font
        // has no such glyph. Padding is trimmed, and long text cut.
        let line = b"GB({\"t\":\"nav\",\"instr\":\"  Kongens gate toward Christian IV's gate \",\"distance\":\"1,2\xa0km\",\"action\":\"right\"})";
        assert_eq!(
            step(line),
            Some(Step {
                turn: Turn::Right,
                street: s("Kongens gate toward Christian IV"),
                distance: s("1,2 km"),
            })
        );
    }

    // ==========================================================================
    // gps, is_gps_active, actfetch
    // ==========================================================================

    #[test]
    fn a_location() {
        // As in a real Gadgetbridge log (fictional place), from the network
        // provider: no `course`.
        let line = br#"GB({"t":"gps","lat":59.9127,"lon":10.7461,"alt":23.0,"speed":0.0,"time":1794150001234,"satellites":0,"hdop":13.230999946594238,"externalSource":true,"gpsSource":"network"})"#;
        assert_eq!(
            parse(line),
            Some(Incoming::Gps {
                latitude: 59.9127,
                longitude: 10.7461
            })
        );
        // West and south, with a `course`, and Java's scientific notation
        // for a coordinate near zero.
        let line = br#"GB({"t":"gps","lat":-33.8688,"lon":-1.0E-4,"alt":12.5,"speed":4.3,"course":271.5,"time":1794150001234,"satellites":9,"hdop":3.9,"externalSource":true,"gpsSource":"gps"})"#;
        assert_eq!(
            parse(line),
            Some(Incoming::Gps {
                latitude: -33.8688,
                longitude: -0.0001
            })
        );
    }

    #[test]
    fn a_location_needs_both_coordinates() {
        assert_eq!(parse(br#"GB({"t":"gps","lat":59.9127,"alt":23.0})"#), None);
        assert_eq!(parse(br#"GB({"t":"gps","lat":59.9127,"lon":null})"#), None);
        assert_eq!(parse(br#"GB({"t":"gps","lat":"59.9","lon":"10.7"})"#), None);
    }

    #[test]
    fn gps_and_activity_requests() {
        assert_eq!(
            parse(br#"GB({"t":"is_gps_active"})"#),
            Some(Incoming::IsGpsActive)
        );
        assert_eq!(
            parse(br#"GB({"t":"actfetch","ts":1794063600000})"#),
            Some(Incoming::ActivityFetch {
                since_ms: 1794063600000
            })
        );
        // No `ts`: everything.
        assert_eq!(
            parse(br#"GB({"t":"actfetch"})"#),
            Some(Incoming::ActivityFetch { since_ms: 0 })
        );
    }

    // ==========================================================================
    // Everything else
    // ==========================================================================

    #[test]
    fn messages_we_dont_act_on() {
        let lines: &[(&[u8], &str)] = &[
            (br#"GB({"t":"notify-","id":1762436801})"#, "notify-"),
            (
                br#"GB({"t":"calendar","id":7,"type":0,"timestamp":1794200000,"durationInSeconds":3600,"title":"Dentist","description":"","location":"","calName":"Personal","color":-16777216,"allDay":false})"#,
                "calendar",
            ),
            (br#"GB({"t":"calendar-","id":7})"#, "calendar-"),
            (
                br#"GB({"t":"weather","temp":278,"hi":281,"lo":274,"hum":80,"rain":60,"uv":1,"code":500,"txt":"Light rain","wind":5.1,"wdir":220,"loc":"Oslo"})"#,
                "weather",
            ),
            (br#"GB({"t":"act","hrm":false,"stp":false,"int":10})"#, "act"),
            (br#"GB({"t":"vibrate","n":2})"#, "vibrate"),
            (br#"GB({"t":"force_calendar_sync_start"})"#, "force_calendar_sync_start"),
        ];
        for &(line, kind) in lines {
            assert_eq!(parse(line), Some(Incoming::Ignored(kind)), "{kind}");
        }
    }

    #[test]
    fn malformed_lines() {
        for line in [
            &b""[..],
            b"hello",
            b"GB(",
            b"GB()",
            b"GB({})",
            b"GB({\"t\":\"notify\"",
            b"GB({\"t\":1})",
            b"GB({\"t\":null})",
            // A kind with an escape in it is no kind we know.
            br#"GB({"t":"no\x74ify"})"#,
            b"GB({\"t\":\"notify\"}) ",
            b"GB [\"t\",\"notify\"]",
            b"\x03\x10reset();",
        ] {
            assert_eq!(parse(line), None, "{}", String::from_utf8_lossy(line));
        }
    }

    #[test]
    fn a_line_cut_short_shows_what_arrived() {
        // A line that lost its end but still ends in `)` (a field cut where
        // a `)` happened to be): the fields before the cut are read, the
        // cut one is empty.
        let line = br#"GB({"t":"notify","id":3,"src":"Signal","title":"Kari (work)"#;
        assert_eq!(
            event(line),
            PhoneEvent::Notification {
                app: s("Signal"),
                who: s("")
            }
        );
    }

    /// Every prefix of every kind of line, and arbitrary bytes, parse to
    /// something or nothing, without panicking (overflow checks are on in
    /// this crate's release profile, too).
    #[test]
    fn nothing_panics() {
        let lines: Vec<Vec<u8>> = vec![
            set_time_line(1794150000, "5.75"),
            notify_line("Signal", "Åse", "\u{1F389} Ola\n\"Kari\" ÆØÅ æøå \u{10}5"),
            br#"GB({"t":"call","cmd":"incoming","name":"Kari Nordmann","number":"+4799999999"})"#.to_vec(),
            br#"GB({"t":"alarm","d":[{"h":6,"m":45,"rep":31,"on":true},{"h":9,"m":0,"rep":96,"on":false}]})"#.to_vec(),
            b"GB({\"t\":\"nav\",\"instr\":\"Storgata\",\"distance\":\"1,2\xa0km\",\"action\":\"left\"})".to_vec(),
            br#"GB({"t":"gps","lat":-33.8688,"lon":-1.0E-4,"alt":12.5})"#.to_vec(),
            br#"GB({"t":"notify","src":"Signal","title":"\ud83c\u0041 \ud83c\n \udf89 \u12 \x4 \777"})"#.to_vec(),
        ];
        for line in &lines {
            for end in 0..=line.len() {
                let _ = parse(&line[..end]);
                // And as if it were cut, then closed.
                let mut closed = line[..end].to_vec();
                closed.extend_from_slice(b"\"})");
                let _ = parse(&closed);
            }
        }
        fn arbitrary(bytes: Vec<u8>) -> bool {
            let _ = gb::parse(&bytes);
            let mut line = b"GB({\"t\":\"notify\",\"title\":".to_vec();
            line.extend(&bytes);
            let _ = gb::parse(&line);
            true
        }
        QuickCheck::new()
            .tests(5000)
            .quickcheck(arbitrary as fn(Vec<u8>) -> bool);
    }

    // ==========================================================================
    // Outgoing
    // ==========================================================================

    #[test]
    fn replies() {
        use super::gb::{CallAction, MusicCommand, Outgoing};
        let cases = [
            (
                Outgoing::Status {
                    percent: 87,
                    charging: false,
                    millivolts: 3987,
                },
                r#"{"t":"status","bat":87,"chg":0,"volt":3.98}"#,
            ),
            (
                Outgoing::GpsPower(true),
                r#"{"t":"gps_power","status":true}"#,
            ),
            (
                Outgoing::ActivityFetch { done: false },
                r#"{"t":"actfetch","state":"start"}"#,
            ),
            (
                Outgoing::Activity {
                    utc_minute: 29_902_500,
                    steps: 42,
                    movement: 310,
                    heart_rate: 71,
                    kind: None,
                },
                r#"{"t":"act","ts":1794150000000,"stp":42,"mov":310,"hrm":71}"#,
            ),
            (
                Outgoing::Activity {
                    utc_minute: 29_902_500,
                    steps: 0,
                    movement: 3,
                    heart_rate: 0,
                    kind: Some("LIGHT_SLEEP"),
                },
                r#"{"t":"act","ts":1794150000000,"stp":0,"mov":3,"hrm":0,"act":"LIGHT_SLEEP"}"#,
            ),
            (
                Outgoing::Call(CallAction::Ignore),
                r#"{"t":"call","n":"ignore"}"#,
            ),
            (
                Outgoing::Music(MusicCommand::PlayPause),
                r#"{"t":"music","n":"playpause"}"#,
            ),
        ];
        for (message, json) in cases {
            assert_eq!(message.to_line().as_str(), format!("{json}\r\n"));
        }
    }
}
