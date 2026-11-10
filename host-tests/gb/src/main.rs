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
    use super::phone::{PhoneEvent, Text, TextBuilder};
    use super::route::{Step, Turn};

    // ==========================================================================
    // Helpers
    // ==========================================================================

    /// Parses a line as `LineBuffer` hands it over (no `\x10`, no `\n`).
    fn parse(line: &[u8]) -> Option<Incoming<'_>> {
        gb::parse(line)
    }

    /// The UI event a line becomes; panics if it becomes anything else,
    /// or carries "Text as Bitmaps" images.
    fn event(line: &[u8]) -> PhoneEvent {
        match with_bitmaps(line) {
            (event, false) => event,
            (_, true) => panic!("text as bitmaps: {}", String::from_utf8_lossy(line)),
        }
    }

    /// The UI event a line becomes, and whether its text had images in it.
    fn with_bitmaps(line: &[u8]) -> (PhoneEvent, bool) {
        match parse(line) {
            Some(Incoming::Phone {
                event,
                text_as_bitmaps,
            }) => (event, text_as_bitmaps),
            _ => panic!("not a UI event: {}", String::from_utf8_lossy(line)),
        }
    }

    fn s<const N: usize>(text: &str) -> heapless::String<N> {
        text.try_into().expect("test strings fit")
    }

    /// `text` as a notification keeps it to show (see `TextBuilder`).
    fn t<const N: usize>(text: &str) -> Text<N> {
        text.into()
    }

    /// Gadgetbridge's `jsonToStringInternal` for a string, ported: how it
    /// spells `text` in a line for a Bangle.js, as the ISO-8859-1 bytes on
    /// the wire. JavaScript-ish rather than JSON: `\xHH` for control
    /// characters and the first bytes of UTF-8 sequences (0xC2–0xF4, which
    /// covers å, æ, ø, é, …), `\uHHHH` per UTF-16 unit beyond Latin-1,
    /// octal for the lowest control characters, raw Latin-1 for the rest;
    /// or, when that's longer than base64, `atob("…")` of the Latin-1
    /// bytes.
    pub(super) fn gadgetbridge_string(text: &str) -> Vec<u8> {
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
    // Lines: what the buffer leaves out, so that long ones fit
    // ==========================================================================

    /// `line` as Gadgetbridge sends it (`\x10`, the line, `\n`, in writes
    /// of 20 bytes), as the buffer hands it over: `None` if it doesn't.
    fn received(line: &[u8]) -> Option<Vec<u8>> {
        let sent = [b"\x10", line, b"\n"].concat();
        let mut buffer = LineBuffer::new();
        let mut lines: Vec<Vec<u8>> = Vec::new();
        for piece in sent.chunks(20) {
            buffer.push(piece, |line| lines.push(line.to_vec()));
        }
        assert!(lines.len() <= 1, "one line in, {} out", lines.len());
        let line = lines.pop()?;
        assert!(line.len() <= gb::MAX_LINE);
        Some(line)
    }

    /// The UI event `line` becomes once it's through the buffer.
    fn event_received(line: &[u8]) -> PhoneEvent {
        let kept = received(line).unwrap_or_else(|| {
            panic!(
                "dropped ({} bytes): {}",
                line.len(),
                String::from_utf8_lossy(line)
            )
        });
        event(&kept)
    }

    /// Gadgetbridge's `cropToLength`: at most `len` UTF-16 units, the last
    /// three of them `...` where it's cut. (A surrogate pair it cuts in
    /// half goes out as one `\uXXXX`, which Rust's strings can't hold: a
    /// U+FFFD here, the same length on the wire.)
    fn crop(text: &str, len: usize) -> String {
        let units: Vec<u16> = text.encode_utf16().collect();
        if units.len() <= len {
            return text.into();
        }
        String::from_utf16_lossy(&units[..len - 3]) + "..."
    }

    /// A notification as Gadgetbridge's `onNotification` sends it, every
    /// field, cropped and spelled as it does them.
    #[derive(Clone, Debug, Default)]
    struct Notification {
        src: String,
        title: String,
        subject: String,
        body: String,
        sender: String,
        tel: Option<String>,
        actions: Vec<String>,
    }

    impl Notification {
        fn line(&self) -> Vec<u8> {
            let mut line = br#"GB({"t":"notify","id":-2147483648,"src":"#.to_vec();
            line.extend(gadgetbridge_string(&self.src));
            for (key, text, len) in [
                ("title", &self.title, 80),
                ("subject", &self.subject, 80),
                ("body", &self.body, 400),
                ("sender", &self.sender, 40),
            ] {
                line.extend(format!(r#","{key}":"#).bytes());
                line.extend(gadgetbridge_string(&crop(text, len)));
            }
            if let Some(tel) = &self.tel {
                line.extend(br#","tel":"#);
                line.extend(gadgetbridge_string(tel));
            }
            line.extend(br#","reply":true"#);
            if !self.actions.is_empty() {
                line.extend(br#","actions":["#);
                for (i, action) in self.actions.iter().enumerate() {
                    if i > 0 {
                        line.push(b',');
                    }
                    line.extend(br#"{"title":"#);
                    line.extend(gadgetbridge_string(&crop(action, 40)));
                    line.push(b'}');
                }
                line.push(b']');
            }
            line.extend(b"})");
            line
        }
    }

    /// Bug G1: Gadgetbridge sends each character beyond Latin-1 as
    /// `\uXXXX`, 6 bytes, and crops a body at 400 characters, so a
    /// notification line reaches ~3,700 bytes; one past 1024 used to be
    /// dropped whole, never shown nor buzzed. Now its body (which is never
    /// shown) is left out as it arrives, and it fits.
    #[test]
    fn a_long_notification_beyond_latin1_arrives() {
        let notification = |body: String| Notification {
            src: "Signal".into(),
            title: "Анна".into(),
            body,
            ..Default::default()
        };
        let anna = PhoneEvent::Notification {
            app: t("Signal"),
            who: t("Анна"),
        };
        // 140 characters arrived before too...
        let short = notification("Привет ".repeat(20));
        assert!(short.line().len() <= gb::MAX_LINE);
        assert_eq!(event_received(&short.line()), anna);
        // ...180 didn't, nor 400 of CJK...
        for body in ["Привет".repeat(30), "你好世界".repeat(100)] {
            let long = notification(body);
            assert!(long.line().len() > gb::MAX_LINE, "{}", long.line().len());
            assert_eq!(event_received(&long.line()), anna);
        }
        // ...and in Latin-1, even the longest fits: base64 takes over.
        let latin = notification("blåbærsyltetøy ".repeat(40));
        assert!(gadgetbridge_string(&crop(&latin.body, 400)).starts_with(b"atob("));
        assert_eq!(event_received(&latin.line()), anna);
    }

    /// The longest title Gadgetbridge sends, 80 Cyrillic letters (480
    /// bytes), is cut at `MAX_STRING`, past the 40 characters a
    /// notification shows, so it shows as it would whole: the letters
    /// the font lacks as `?`, and an ellipsis where it doesn't fit.
    #[test]
    fn the_longest_title_shows_as_it_would_whole() {
        let title = "Ж".repeat(80);
        let line = Notification {
            src: "Telegram".into(),
            title: title.clone(),
            subject: "Тема".repeat(20),
            body: "Ж".repeat(400),
            ..Default::default()
        }
        .line();
        let PhoneEvent::Notification { app, who } = event_received(&line) else {
            panic!("a notification");
        };
        assert_eq!(app, t("Telegram"));
        assert_eq!(who, t(&title));
        assert_eq!(who.to_string(), "?".repeat(39) + "\u{2026}");
    }

    /// What's left out is followed through every escape Gadgetbridge
    /// writes, so that a quote, bracket or comma in the text doesn't end
    /// it early, nor one escaped `\\` before a quote late; in an escaped
    /// string and in base64; and in the `actions` array of objects.
    #[test]
    fn what_is_left_out_ends_where_it_ends() {
        let tricky = [
            r#"He said "}), "sender":"Eve", {"body": \"#.to_string(),
            "back\\slash\\\" \u{1F389} \\u0041 \u{10}5 \t\n\r end \\".into(),
            "Привет, \"Анна\"! ".repeat(30),
            "æøå \"}\\ ".repeat(60),
        ];
        for text in &tricky {
            let line = Notification {
                src: "Mail".into(),
                title: "Kari".into(),
                subject: text.clone(),
                body: text.clone(),
                sender: "Ola Nordmann".into(),
                tel: Some(text.clone()),
                actions: vec![text.clone(), "Mark \"read\"".into(), text.clone()],
            }
            .line();
            let kept = received(&line).expect("it fits");
            assert_eq!(
                kept,
                br#"GB({"t":"notify","id":-2147483648,"src":"Mail","title":"Kari","subject":"","body":"","sender":"Ola Nordmann","tel":"","reply":true,"actions":""})"#,
                "{text:?}"
            );
        }
    }

    /// Other lines, and whatever isn't a member's value, are kept as they
    /// come: `setTime`, nested objects, numbers, and keys we do read that
    /// only look like the ones we don't.
    #[test]
    fn what_we_read_is_kept() {
        for line in [
            set_time_line(1794150000, "5.75"),
            br#"GB({"t":"alarm","d":[{"h":6,"m":45,"rep":31,"on":true},{"h":9,"m":0,"rep":96,"on":false}]})"#.to_vec(),
            br#"GB({"t":"notify","id":5,"src":"SMS Message","title":"body","sender":"\"subject\"","x":{"body":"in an object"},"y":["body"]})"#.to_vec(),
            br#"GB( { "t" : "find" , "n" : true } )"#.to_vec(),
            // Not a message: kept whole, however it looks.
            br#"setTime({"body":"x"})"#.to_vec(),
        ] {
            assert_eq!(received(&line), Some(line.clone()), "{}", String::from_utf8_lossy(&line));
        }
        // Any kind of value, and spaced out: `""` in its place.
        assert_eq!(
            received(br#"GB({"t":"notify" , "body" : [1,{"a":"]"}] , "tel":null,"subject":atob("SGk="),"title":"Kari"})"#),
            Some(br#"GB({"t":"notify" , "body" :"", "tel":"","subject":"","title":"Kari"})"#.to_vec())
        );
    }

    /// A string longer than `MAX_STRING` is cut between characters, never
    /// into an escape, nor between a surrogate pair's halves, and ends in
    /// `…` (`…`); in base64, without.
    #[test]
    fn long_strings_are_cut_between_characters() {
        let max = gb::MAX_STRING;
        let cut = |spelled: &[u8]| -> Vec<u8> {
            let mut line = br#"GB({"t":"call","cmd":"incoming","name":"#.to_vec();
            line.extend(spelled);
            line.extend(b"})");
            let kept = received(&line).expect("it fits");
            kept[br#"GB({"t":"call","cmd":"incoming","name":"#.len()..kept.len() - 2].to_vec()
        };
        let quoted = |inner: &str| format!("\"{inner}\"").into_bytes();
        // Up to `MAX_STRING`, whole.
        let whole = "a".repeat(max);
        assert_eq!(cut(&quoted(&whole)), quoted(&whole));
        // One more, cut.
        assert_eq!(
            cut(&quoted(&"a".repeat(max + 1))),
            quoted(&format!("{whole}\\u2026"))
        );
        // An escape started before the cut is kept whole.
        let escaped = format!("{}\\u0416{}", "a".repeat(max - 1), "a".repeat(9));
        assert_eq!(
            cut(&quoted(&escaped)),
            quoted(&format!("{}\\u0416\\u2026", "a".repeat(max - 1)))
        );
        // Octal, which runs for as many digits as follow (up to three).
        let octal = format!("{}\\205{}", "a".repeat(max - 2), "a".repeat(9));
        assert_eq!(
            cut(&quoted(&octal)),
            quoted(&format!("{}\\205\\u2026", "a".repeat(max - 2)))
        );
        // A high surrogate before the cut goes, rather than take the
        // ellipsis for its low half.
        let pair = format!("{}\\ud83c\\udf89bb", "a".repeat(max - 6));
        assert_eq!(
            cut(&quoted(&pair)),
            quoted(&format!("{}\\u2026", "a".repeat(max - 6)))
        );
        // A quote after an escape's `\` doesn't end the string; a quote
        // where an escape's digits should be does (as the reader has it).
        let quote = format!("{}\\\"{}", "a".repeat(max - 1), "a".repeat(9));
        assert_eq!(
            cut(&quoted(&quote)),
            quoted(&format!("{}\\\"\\u2026", "a".repeat(max - 1)))
        );
        // Base64: just cut.
        let base64 = "QUJD".repeat(max);
        assert_eq!(
            cut(format!("atob(\"{base64}\")").as_bytes()),
            format!("atob(\"{}\")", &base64[..max]).into_bytes()
        );
    }

    /// Strings we show are cut rather than lost: a caller, a track and
    /// artist, a street, a title with "Text as Bitmaps" images in it; and
    /// emoji, which show as nothing, still end in an ellipsis once cut.
    #[test]
    fn long_text_we_show_is_cut_rather_than_lost() {
        let long = "Чайковский".repeat(30);
        let long = String::from_utf8(gadgetbridge_string(&long)).expect("escapes are ASCII");
        let line =
            format!(r#"GB({{"t":"call","cmd":"incoming","name":{long},"number":"+4799999999"}})"#);
        assert_eq!(
            event_received(line.as_bytes()),
            PhoneEvent::IncomingCall {
                // 40 bytes: 20 Cyrillic letters.
                caller: s(&"Чайковский".repeat(2))
            }
        );
        let line = format!(
            r#"GB({{"t":"musicinfo","artist":{long},"album":{long},"track":{long},"dur":1234,"c":12,"n":3}})"#
        );
        let PhoneEvent::MusicInfo { artist, track } = event_received(line.as_bytes()) else {
            panic!("music");
        };
        assert_eq!((artist.chars().count(), track.chars().count()), (20, 20));
        let line = format!(
            r#"GB({{"t":"nav","instr":{long},"distance":{long},"action":"left","eta":{long}}})"#
        );
        assert!(matches!(
            event_received(line.as_bytes()),
            PhoneEvent::Route(Some(_))
        ));

        // Images: big ones, as they are, in the title.
        // (Gadgetbridge crops text before it draws it.)
        let image = as_bitmaps(&[Ok("Mamma "), Err((60, 24, 3)), Err((60, 24, 3))]);
        let image = String::from_utf8(gadgetbridge_string(&image)).expect("escapes are ASCII");
        let line = format!(r#"GB({{"t":"notify","src":"Signal","title":{image},"sender":""}})"#);
        let line = line.as_bytes();
        assert!(line.len() > gb::MAX_LINE);
        let kept = received(line).expect("it fits");
        let (PhoneEvent::Notification { who, .. }, true) = with_bitmaps(&kept) else {
            panic!("a notification with images");
        };
        assert_eq!(who.to_string(), "Mamma ?");

        // Emoji (12 bytes each), past `MAX_STRING`, before a name.
        let title = format!("{}Kari", "\u{1F389}".repeat(30));
        let line = Notification {
            src: "Signal".into(),
            title,
            ..Default::default()
        }
        .line();
        let PhoneEvent::Notification { who, .. } = event_received(&line) else {
            panic!("a notification");
        };
        assert_eq!(who.to_string(), "\u{2026}");
    }

    /// The most a notification can bring of what we keep: an app name
    /// as long as any (Gadgetbridge doesn't crop it), each field we show
    /// at its longest in the longest spelling (`\uXXXX`), and the rest
    /// as long as Gadgetbridge sends them. It fits, with the room
    /// `MAX_STRING` leaves.
    #[test]
    fn the_longest_notification_fits() {
        let line = Notification {
            src: "Ж".repeat(1000),
            title: "Ж".repeat(1000),
            subject: "Ж".repeat(1000),
            body: "Ж".repeat(1000),
            sender: "Ж".repeat(1000),
            tel: Some("Ж".repeat(1000)),
            actions: vec!["Ж".repeat(1000); 10],
        }
        .line();
        let kept = received(&line).expect("it fits");
        let PhoneEvent::Notification { app, who } = event(&kept) else {
            panic!("a notification");
        };
        assert_eq!(app.to_string(), "?".repeat(23) + "\u{2026}");
        // `src` and `title` cut, `sender` whole at Gadgetbridge's 40.
        assert_eq!(who.to_string(), "?".repeat(37) + "...");
        assert!(
            kept.len() <= gb::MAX_LINE - 100,
            "{} bytes: little room to spare",
            kept.len()
        );
    }

    /// Any notification Gadgetbridge could send (any text, cropped as it
    /// crops it) arrives, through the buffer, and shows the app and who
    /// it's from as `TextBuilder` makes of them; or, where a string was
    /// longer than `MAX_STRING` (an app's name, or a title of emoji), as
    /// much of that as was kept, and an ellipsis.
    #[test]
    fn any_notification_arrives() {
        /// Whether `shown` is what `TextBuilder` makes of `text`, given
        /// how the buffer cuts it.
        fn shows<const N: usize>(shown: &Text<N>, text: &str) -> bool {
            let whole = t::<N>(text);
            let spelled = gadgetbridge_string(text);
            let base64 = spelled.starts_with(b"atob(");
            let contents = spelled.len() - if base64 { 8 } else { 2 };
            if contents <= gb::MAX_STRING {
                return *shown == whole;
            }
            let shown = shown.to_string();
            let whole = whole.to_string();
            let core = match shown.strip_suffix('\u{2026}') {
                Some(core) => core,
                // Only base64 is cut without an ellipsis.
                None if base64 => &shown,
                None => return false,
            };
            whole
                .strip_suffix('\u{2026}')
                .unwrap_or(&whole)
                .starts_with(core.trim_end())
        }
        fn property(
            src: String,
            (title, subject, body, sender): (String, String, String, String),
            tel: Option<String>,
            actions: Vec<String>,
        ) -> TestResult {
            let notification = Notification {
                src,
                title,
                subject,
                body,
                sender,
                tel,
                // A notification's buttons are few.
                actions: actions.into_iter().take(5).collect(),
            };
            // As in `any_text_survives_the_trip`: images, and
            // Gadgetbridge's own bug with DLE before a digit.
            let fine = |t: &str| {
                !t.contains('\0')
                    && !t
                        .chars()
                        .zip(t.chars().skip(1))
                        .any(|(a, b)| a == '\u{10}' && ('0'..='7').contains(&b))
            };
            if ![&notification.src, &notification.title, &notification.sender]
                .iter()
                .all(|t| fine(t))
            {
                return TestResult::discard();
            }
            let line = notification.line();
            let Some(kept) = received(&line) else {
                return TestResult::error(format!("dropped: {}", String::from_utf8_lossy(&line)));
            };
            let Some(Incoming::Phone {
                event: PhoneEvent::Notification { app, who },
                text_as_bitmaps: false,
            }) = parse(&kept)
            else {
                return TestResult::error(format!("not read: {}", String::from_utf8_lossy(&kept)));
            };
            let title = crop(&notification.title, 80);
            let sender = crop(&notification.sender, 40);
            let mut from_sender = TextBuilder::<40>::new();
            sender.chars().for_each(|c| from_sender.push(c));
            let from = if from_sender.is_empty() {
                title
            } else {
                sender
            };
            TestResult::from_bool(shows(&app, &notification.src) && shows(&who, &from))
        }
        // Strings of up to 400 characters, for the body.
        QuickCheck::new()
            .tests(3000)
            .rng(quickcheck::Gen::new(400))
            .quickcheck(
                property
                    as fn(
                        String,
                        (String, String, String, String),
                        Option<String>,
                        Vec<String>,
                    ) -> TestResult,
            );
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
    fn implausible_times_are_refused() {
        for (seconds, zone) in [
            (0, "1.0"),
            (1_577_836_799, "1.0"), // a second before 2020
            (4_102_444_800, "1.0"), // 2100
            (-1794150000, "1.0"),
            (1794150000, "18.25"),
            (1794150000, "-18.25"),
            (1794150000, "1e9"),
            (1794150000, "-1e9"),
        ] {
            assert_eq!(
                parse(&set_time_line(seconds, zone)),
                None,
                "{seconds} {zone}"
            );
        }
        assert_eq!(
            parse(b"setTime(99999999999999999999);E.setTimeZone(1.0);"),
            None
        );
        for (seconds, zone, minutes) in [
            (1_577_836_800, "18.0", 1080),
            (4_102_444_799, "-18.0", -1080),
        ] {
            assert_eq!(
                parse(&set_time_line(seconds, zone)),
                Some(Incoming::SetTime {
                    utc_seconds: seconds,
                    offset_minutes: minutes
                }),
                "{seconds} {zone}"
            );
        }
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
                app: t("Signal"),
                who: t("Kari Nordmann")
            }
        );
    }

    /// `sender` is shown when there is one, over the title: an SMS's
    /// number, say, though the title has the contact's name.
    #[test]
    fn the_sender_wins_over_the_title() {
        let line = br#"GB({"t":"notify","id":5,"src":"SMS Message","title":"New message","subject":"","body":"Your parcel has shipped","sender":"Posten","tel":"+4799999999"})"#;
        assert_eq!(
            event(line),
            PhoneEvent::Notification {
                app: t("SMS Message"),
                who: t("Posten")
            }
        );
        let line = br#"GB({"t":"notify","id":5,"src":"SMS Message","title":"Nils Holm","subject":"","body":"Hei","sender":"+47 912 34 567","tel":"+4791234567"})"#;
        let PhoneEvent::Notification { who, .. } = event(line) else {
            panic!("a notification");
        };
        assert_eq!(who.to_string(), "+47 912 34 567");
    }

    /// Without a `sender` (most messaging apps), the title, whole: a group
    /// chat's "Group: Person" isn't split. Also for a `sender` that's
    /// blank, or only emoji, which would show nothing.
    #[test]
    fn the_title_without_a_sender() {
        let line = notify_line("WhatsApp", "Familien (12 messages): Åse", "");
        let PhoneEvent::Notification { who, .. } = event(&line) else {
            panic!("a notification");
        };
        assert_eq!(who.to_string(), "Familien (12 messages): Åse");
        let line = br#"GB({"t":"notify","src":"SMS Message","title":"Posten","sender":" 
 "})"#;
        let PhoneEvent::Notification { who, .. } = event(line) else {
            panic!("a notification");
        };
        assert_eq!(who.to_string(), "Posten");
        let line =
            br#"GB({"t":"notify","src":"Telegram","title":"Kari","sender":"\ud83c\udf89\u200d"})"#;
        let PhoneEvent::Notification { who, .. } = event(line) else {
            panic!("a notification");
        };
        assert_eq!(who.to_string(), "Kari");
        // Neither shows anything: nothing.
        let line = br#"GB({"t":"notify","src":"Telegram","title":"\ud83c\udf89\ud83c\udf82","sender":""})"#;
        let PhoneEvent::Notification { who, .. } = event(line) else {
            panic!("a notification");
        };
        assert!(who.is_empty(), "{who:?}");
    }

    #[test]
    fn a_notification_with_actions_and_nothing_to_show() {
        // `actions` is an array of objects, which the reader steps over; no
        // title or sender leaves `who` empty.
        let line = br#"GB({"t":"notify","id":9,"src":"Calendar","title":"","subject":"","body":"Standup [room 4]","sender":"","tel":"","actions":[{"id":-1234,"name":"Snooze","reply":false},{"id":-1235,"name":"Mark \"done\"","reply":false}]})"#;
        assert_eq!(
            event(line),
            PhoneEvent::Notification {
                app: t("Calendar"),
                who: t("")
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
            // Octal: `\1`, Gadgetbridge's `\20` for DLE, and `\x0N` where a
            // digit follows. (`\0` starts an image: see `text_as_bitmaps`.)
            (br#""a\1b\20c\x035""#, "a\u{1}b\u{10}c\u{3}5"),
        ];
        for &(spelled, text) in cases {
            // A caller's name keeps the text as decoded (a notification's
            // is cleaned up: see `text_as_shown`).
            let mut line = br#"GB({"t":"call","cmd":"incoming","name":"#.to_vec();
            line.extend_from_slice(spelled);
            line.extend_from_slice(br#"})"#);
            let PhoneEvent::IncomingCall { caller } = event(&line) else {
                panic!("a call");
            };
            assert_eq!(caller, text, "{}", String::from_utf8_lossy(spelled));
        }
    }

    /// Gadgetbridge sends notification text as Android hands it over:
    /// plain text, not HTML, so `&` and `<` arrive as themselves and there
    /// are no entities to decode (an `&amp;` would be the app's own, and
    /// show on the phone too). Smart quotes and dashes arrive as `\u`
    /// escapes (Gadgetbridge only swaps them for ASCII when it renders
    /// text as images), and are kept as their ASCII look-alikes, as the
    /// font draws them.
    #[test]
    fn symbols_arrive_as_they_are() {
        for (title, shown) in [
            ("Tom & Jerry <3", "Tom & Jerry <3"),
            ("Don't \"panic\" #1 @ben?", "Don't \"panic\" #1 @ben?"),
            (
                "!\"#$%&'()*+,-./:;<=>?@[\\]^_`{|}~",
                "!\"#$%&'()*+,-./:;<=>?@[\\]^_`{|}~",
            ),
            (
                "\u{201c}Don\u{2019}t\u{201d} \u{2013} Ida\u{2026}",
                "\"Don't\" - Ida\u{2026}",
            ),
            ("AT&amp;T", "AT&amp;T"),
        ] {
            let line = notify_line("Mail", title, "");
            let PhoneEvent::Notification { who, .. } = event(&line) else {
                panic!("a notification");
            };
            assert_eq!(who.to_string(), shown, "{}", String::from_utf8_lossy(&line));
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
                app: t("Signal"),
                who: t("ÆØÅ æøå")
            }
        );
        // Without padding, too.
        let line = br#"GB({"t":"notify","src":"Signal","title":atob("xtjF")})"#;
        let PhoneEvent::Notification { who, .. } = event(line) else {
            panic!("a notification");
        };
        assert_eq!(who.to_string(), "ÆØÅ");
    }

    /// 40 characters of title, æ, ø and å as many as any (in UTF-8 they
    /// took two bytes each, and a name of them was cut at 20), and an
    /// ellipsis where it was cut.
    #[test]
    fn long_fields_keep_characters_and_end_in_an_ellipsis() {
        let name = "Øyvind Åsmund Ødegård Ærlig Østby Æsøy";
        let line = notify_line("A messaging app with a long name", name, "");
        let PhoneEvent::Notification { app, who } = event(&line) else {
            panic!("a notification");
        };
        assert_eq!(who.to_string(), name);
        assert_eq!(app.to_string(), "A messaging app with a\u{2026}");
        let name = "Sameiet Bjørnstjerne Bjørnsons gate 12, oppgang B";
        let line = notify_line("Signal", name, "");
        let PhoneEvent::Notification { who, .. } = event(&line) else {
            panic!("a notification");
        };
        assert_eq!(
            who.to_string(),
            "Sameiet Bjørnstjerne Bjørnsons gate 12,\u{2026}"
        );
    }

    /// What a notification's text becomes on the watch (`TextBuilder`):
    /// emoji and invisible characters out, spaces collapsed, typographic
    /// punctuation as ASCII, and `?` only for letters the font lacks.
    #[test]
    fn text_as_shown() {
        for (sent, shown) in [
            // Emoji go, and the spaces around them collapse, with none
            // left before the punctuation that followed.
            ("Familien \u{1F3E1}: Nils Holm", "Familien: Nils Holm"),
            (
                "Familien \u{1F3E1} (12 messages): Åse",
                "Familien (12 messages): Åse",
            ),
            (
                "\u{1F389}\u{1F382} Bursdagsgjengen 2026",
                "Bursdagsgjengen 2026",
            ),
            ("Ida \u{2764}\u{FE0F}", "Ida"),
            ("Hei\u{1F60A}Kari", "Hei Kari"),
            ("Bra \u{1F44D}!", "Bra!"),
            // A family, joined, with skin tones; a flag; a keycap.
            (
                "Team \u{1F468}\u{1F3FD}\u{200D}\u{1F469}\u{200D}\u{1F467} Ur",
                "Team Ur",
            ),
            ("Norge \u{1F1F3}\u{1F1F4}", "Norge"),
            ("Room 1\u{FE0F}\u{20E3}", "Room 1"),
            // Symbols phones draw as emoji, all over Unicode.
            (
                "\u{23F0} Alarm \u{2600} \u{2705} \u{2B50} \u{25B6} \u{3299}",
                "Alarm",
            ),
            // Direction marks around a phone number.
            ("\u{202A}+47 912 34 567\u{202C}", "+47 912 34 567"),
            // Typographic punctuation, as the font draws it.
            (
                "\u{201c}Ida\u{201d} \u{2013} 5\u{2212}3\u{2026}",
                "\"Ida\" - 5-3\u{2026}",
            ),
            // Letters the font lacks are missing, and say so.
            ("Łukasz Борис", "?ukasz ?????"),
            // Whitespace of every kind, once, and none at the ends.
            ("  Ola \n\t Kari\u{a0}\u{a0}Nordmann  ", "Ola Kari Nordmann"),
            ("\u{1F389}", ""),
            ("", ""),
        ] {
            let mut builder = TextBuilder::<40>::new();
            sent.chars().for_each(|c| builder.push(c));
            assert_eq!(builder.finish().to_string(), shown, "{sent:?}");
        }
        // Cut with an ellipsis, in place of a space before it too.
        assert_eq!(t::<8>("Ola Kari Nordmann").to_string(), "Ola Kar\u{2026}");
        assert_eq!(t::<8>("Ola Kar Nordmann").to_string(), "Ola Kar\u{2026}");
        assert_eq!(t::<8>("Ola Kari").to_string(), "Ola Kari");
        assert_eq!(t::<8>("Ola Kari \u{1F389}").to_string(), "Ola Kari");
        assert_eq!(t::<8>("Ola Kari!").to_string(), "Ola Kar\u{2026}");
        assert_eq!(t::<4>("Ola  K").to_string(), "Ola\u{2026}");
    }

    /// The decoder against Gadgetbridge's encoder (ported above), for any
    /// text: what reaches the screen is what `TextBuilder` makes of the
    /// text (tested below). Text with a NUL in it is left out: that starts
    /// an image (see `text_as_bitmaps`), for a Bangle.js too.
    #[test]
    fn any_text_survives_the_trip() {
        fn property(title: String, src: String) -> TestResult {
            // Gadgetbridge's `\20` for DLE runs into a following octal digit
            // (JavaScript reads `\205`): its bug, which the watch can't undo.
            let dle_digit = |t: &str| {
                t.chars()
                    .zip(t.chars().skip(1))
                    .any(|(a, b)| a == '\u{10}' && ('0'..='7').contains(&b))
            };
            if dle_digit(&title) || dle_digit(&src) || title.contains('\0') || src.contains('\0') {
                return TestResult::discard();
            }
            let line = notify_line(&src, &title, "");
            let PhoneEvent::Notification { app, who } = event(&line) else {
                return TestResult::failed();
            };
            TestResult::from_bool(who == t(&title) && app == t(&src))
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

    /// Parses `line` on a thread of its own, and fails if that takes more
    /// than a second: on the watch, a parse that never ends stalls the
    /// Bluetooth task until the watchdog resets the watch.
    fn parses_in_time(line: Vec<u8>) {
        let (done, finished) = std::sync::mpsc::channel();
        let shown = String::from_utf8_lossy(&line).into_owned();
        std::thread::spawn(move || {
            let _ = gb::parse(&line);
            let _ = done.send(());
        });
        assert!(
            finished
                .recv_timeout(std::time::Duration::from_secs(1))
                .is_ok(),
            "never finished parsing {shown}"
        );
    }

    #[test]
    fn alarm_lists_that_dont_parse_end() {
        // A `}` where an item should start once had the array's items
        // repeat forever (as empty values, none of them an alarm).
        for list in [
            "[}]",
            "[ }",
            r#"[{"h":6,"m":45},}]"#,
            r#"[{"h":6,"m":45}}]"#,
            "[1}",
            "[,}",
            "[:}",
        ] {
            parses_in_time(format!(r#"GB({{"t":"alarm","d":{list}}})"#).into_bytes());
        }
        fn property(list: Vec<u8>) -> bool {
            let mut line = br#"GB({"t":"alarm","d":["#.to_vec();
            line.extend(list);
            line.extend(b"})");
            parses_in_time(line);
            true
        }
        QuickCheck::new()
            .tests(2000)
            .quickcheck(property as fn(Vec<u8>) -> bool);
    }

    /// Whatever follows a message's kind, every kind's parse ends.
    #[test]
    fn every_kind_of_message_parses_in_time() {
        fn property(kind: u8, rest: Vec<u8>) -> bool {
            let kinds = [
                "notify",
                "call",
                "musicinfo",
                "musicstate",
                "find",
                "alarm",
                "nav",
                "gps",
                "actfetch",
                "act",
                "weather",
            ];
            let kind = kinds[usize::from(kind) % kinds.len()];
            let mut line = format!(r#"GB({{"t":"{kind}","#).into_bytes();
            line.extend(rest);
            line.extend(b"})");
            parses_in_time(line);
            true
        }
        QuickCheck::new()
            .tests(2000)
            .quickcheck(property as fn(u8, Vec<u8>) -> bool);
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
                latitude_micro: 59_912_700,
                longitude_micro: 10_746_100,
                // `hdop` is Android's accuracy, in metres.
                accuracy: Some(13.231),
            })
        );
        // West and south, with a `course`, and Java's scientific notation
        // for a coordinate near zero.
        let line = br#"GB({"t":"gps","lat":-33.8688,"lon":-1.0E-4,"alt":12.5,"speed":4.3,"course":271.5,"time":1794150001234,"satellites":9,"hdop":3.9,"externalSource":true,"gpsSource":"gps"})"#;
        assert_eq!(
            parse(line),
            Some(Incoming::Gps {
                latitude_micro: -33_868_800,
                longitude_micro: -100,
                accuracy: Some(3.9),
            })
        );
        // Without one, it doesn't say how far off it may be.
        assert_eq!(
            parse(br#"GB({"t":"gps","lat":59.9127,"lon":10.7461})"#),
            Some(Incoming::Gps {
                latitude_micro: 59_912_700,
                longitude_micro: 10_746_100,
                accuracy: None,
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

    /// Gadgetbridge's live activity requests, as its `transmitActivityStatus`
    /// sends them: opening the chart asks for steps, then the heart rate
    /// too; closing it stops one, then the other.
    #[test]
    fn live_activity_requests() {
        let live = |heart_rate, steps, interval| Incoming::Live {
            heart_rate,
            steps,
            interval,
        };
        assert_eq!(
            parse(br#"GB({"t":"act","hrm":false,"stp":true,"int":10})"#),
            Some(live(false, true, Some(10)))
        );
        assert_eq!(
            parse(br#"GB({"t":"act","hrm":true,"stp":true,"int":10})"#),
            Some(live(true, true, Some(10)))
        );
        assert_eq!(
            parse(br#"GB({"t":"act","hrm":false,"stp":false,"int":10})"#),
            Some(live(false, false, Some(10)))
        );
        // A Bangle.js's own stop, on disconnecting, has numbers.
        assert_eq!(
            parse(br#"GB({"t":"act","stp":0,"hrm":0,"int":0})"#),
            Some(live(false, false, Some(0)))
        );
        // Whatever's missing or odd asks for nothing (and `live` picks
        // the interval).
        assert_eq!(parse(br#"GB({"t":"act"})"#), Some(live(false, false, None)));
        assert_eq!(
            parse(br#"GB({"t":"act","hrm":"yes","int":"10"})"#),
            Some(live(false, false, None))
        );
        assert_eq!(
            parse(br#"GB({"t":"act","hrm":true,"int":2.5})"#),
            Some(live(true, false, Some(2)))
        );
    }

    // ==========================================================================
    // Text as Bitmaps
    // ==========================================================================

    /// An image as Gadgetbridge's `bitmapToEspruinoArray` lays it out, for
    /// a word drawn `width` × `height` with `bpp` bits a pixel (1 for
    /// plain text, `MONOCHROME_TRANSPARENT`; 3 for a word with emoji,
    /// `RGB_3BPP_TRANSPARENT`): width, height, bits per pixel plus 128 for
    /// transparency, the transparent colour, then the pixels, a bit at a
    /// time, most significant first. The pixels are a fixed jumble that
    /// takes in every byte (NUL, quotes, backslashes, line breaks, DLE,
    /// digits after low bytes), to exercise every escape. Bar one: a DLE
    /// (16) before a digit, which Gadgetbridge spells so that it reads as
    /// another byte (see `any_text_survives_the_trip`); a Bangle.js
    /// misreads those images too. Nor does one end in a DLE, before
    /// whatever text follows.
    fn espruino_image(width: u8, height: u8, bpp: u8, seed: u32) -> Vec<u8> {
        let bits = usize::from(width) * usize::from(height) * usize::from(bpp);
        let mut image = vec![width, height, bpp + 128, 0];
        let mut state = seed;
        image.extend((0..bits.div_ceil(8)).map(|i| {
            state = state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            // Every so often, a byte the escapes treat specially.
            match i % 7 {
                0 => [0, b'"', b'\\', b'\n', 16, 5, 0xC5][(state >> 16) as usize % 7],
                1 => b'0' + (state >> 16) as u8 % 8,
                _ => (state >> 16) as u8,
            }
        }));
        for i in 4..image.len() {
            let next = image.get(i + 1).copied().unwrap_or(b'0');
            if image[i] == 16 && (b'0'..=b'7').contains(&next) {
                image[i] = 17;
            }
        }
        image
    }

    /// What `renderUnicodeAsImage` makes of `words`: each `Ok` as is,
    /// each `Err` (width, height, bpp) as `\0` and an image, joined, as
    /// the Java string (each image byte a char, as ISO-8859-1 reads it).
    fn as_bitmaps(words: &[Result<&str, (u8, u8, u8)>]) -> String {
        let mut text = String::new();
        for (seed, word) in words.iter().enumerate() {
            match *word {
                Ok(word) => text.push_str(word),
                Err((width, height, bpp)) => {
                    text.push('\0');
                    let image = espruino_image(width, height, bpp, seed as u32);
                    text.extend(image.into_iter().map(char::from));
                }
            }
        }
        text
    }

    /// With "Text as Bitmaps" on, "Mamma 妈妈 ❤️" comes as "Mamma ", an
    /// image of 妈妈, " ", and an image of the heart (in colour): each
    /// image shows as a `?`, and the line says it had images, both as
    /// escaped text and as base64 (whichever Gadgetbridge picked).
    #[test]
    fn text_as_bitmaps() {
        let sender = as_bitmaps(&[Ok("Mamma "), Err((37, 21, 1)), Ok(" "), Err((19, 21, 3))]);
        let spelled = gadgetbridge_string(&sender);
        let line = notify_line("Signal", "", &sender);
        assert_eq!(
            with_bitmaps(&line),
            (
                PhoneEvent::Notification {
                    app: t("Signal"),
                    who: t("Mamma ? ?")
                },
                true
            ),
            "{}",
            String::from_utf8_lossy(&spelled)
        );
        // Both of Gadgetbridge's spellings: escaped, and base64.
        let mut spellings = [false; 2];
        for width in 1..60 {
            let sender = as_bitmaps(&[Ok("Kari "), Err((width, 21, 1)), Ok("!")]);
            spellings[usize::from(gadgetbridge_string(&sender).starts_with(b"atob("))] = true;
            let line = notify_line("Signal", "", &sender);
            let (PhoneEvent::Notification { who, .. }, true) = with_bitmaps(&line) else {
                panic!(
                    "a notification with images: {}",
                    String::from_utf8_lossy(&line)
                );
            };
            assert_eq!(who.to_string(), "Kari ?!", "{width}");
        }
        assert_eq!(spellings, [true; 2], "both spellings tried");
        // Without, nothing is said.
        assert_eq!(with_bitmaps(&notify_line("Signal", "", "Mamma")).1, false);
    }

    /// Images are noticed wherever Gadgetbridge puts them in text we
    /// read: in a notification's title (which is read, if not always
    /// shown), in `sender` and `src`, past what fits, and in calls,
    /// music, and directions. Not in a subject or body, which the buffer
    /// leaves out: those images used to matter for making the line too
    /// long to keep, which they no longer do.
    #[test]
    fn text_as_bitmaps_anywhere() {
        let image = as_bitmaps(&[Err((24, 21, 1))]);
        let image = String::from_utf8(gadgetbridge_string(&image)).expect("escapes are ASCII");
        let long = as_bitmaps(&[Ok(&"Ola ".repeat(12)), Err((24, 21, 1))]);
        let long = String::from_utf8(gadgetbridge_string(&long)).expect("escapes are ASCII");
        let lines = [
            format!(r#"GB({{"t":"notify","src":"Signal","title":{image},"sender":"Kari"}})"#),
            format!(r#"GB({{"t":"notify","src":{image},"sender":"Kari"}})"#),
            format!(r#"GB({{"t":"notify","src":"Signal","sender":{long}}})"#),
            format!(r#"GB({{"t":"call","cmd":"incoming","name":{image},"number":"+4799999999"}})"#),
            format!(r#"GB({{"t":"musicinfo","artist":"Ida Moreau","track":{image}}})"#),
            format!(r#"GB({{"t":"nav","instr":{image},"distance":"150 m","action":"left"}})"#),
        ];
        for line in lines {
            assert!(with_bitmaps(line.as_bytes()).1, "{line}");
        }
        for line in [
            format!(r#"GB({{"t":"notify","src":"Mail","subject":{image},"sender":"Kari"}})"#),
            format!(r#"GB({{"t":"notify","src":"Signal","body":{image},"sender":"Kari"}})"#),
        ] {
            let kept = received(line.as_bytes()).expect("it fits");
            assert_eq!(
                with_bitmaps(&kept),
                (
                    PhoneEvent::Notification {
                        app: t(if line.contains("Mail") {
                            "Mail"
                        } else {
                            "Signal"
                        }),
                        who: t("Kari")
                    },
                    false
                ),
                "{line}"
            );
        }
        let (event, _) = with_bitmaps(
            format!(r#"GB({{"t":"call","cmd":"incoming","name":{image},"number":"+4799999999"}})"#)
                .as_bytes(),
        );
        assert_eq!(event, PhoneEvent::IncomingCall { caller: s("?") });
    }

    /// Any images in any text: each is a `?`, and the rest is as sent.
    #[test]
    fn text_as_bitmaps_survive_the_trip() {
        fn property(words: Vec<(String, u8, u8, bool)>) -> TestResult {
            // A few words, and images of the size of one or two words of
            // text (`PREF_BANGLEJS_TEXT_BITMAP_SIZE` is 18 px by default),
            // so that most lines fit.
            let words: Vec<(String, u8, u8, bool)> = words
                .into_iter()
                .take(3)
                .map(|(word, width, height, colour)| {
                    // Latin-1 words, without NULs or controls (tested
                    // above), as `renderUnicodeAsImage` leaves them.
                    let word = word
                        .chars()
                        .filter(|&c| (' '..='\u{ff}').contains(&c) && c != '\u{10}')
                        .collect();
                    (word, width % 40 + 1, height % 24 + 1, colour)
                })
                .collect();
            let mut parts = Vec::new();
            let mut shown = String::new();
            for (word, width, height, colour) in &words {
                parts.push(Ok(word.as_str()));
                parts.push(Err((*width, *height, if *colour { 3 } else { 1 })));
                shown.push_str(word);
                shown.push('?');
            }
            if words.is_empty() {
                return TestResult::discard();
            }
            let line = notify_line("Signal", "", &as_bitmaps(&parts));
            // Lines too long for the watch are dropped whole anyway.
            if line.len() > 1024 {
                return TestResult::discard();
            }
            let (PhoneEvent::Notification { who, .. }, true) = with_bitmaps(&line) else {
                return TestResult::failed();
            };
            TestResult::from_bool(who == t(&shown))
        }
        QuickCheck::new()
            .tests(2000)
            .quickcheck(property as fn(Vec<(String, u8, u8, bool)>) -> TestResult);
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
                app: t("Signal"),
                who: t("")
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
                    timestamp_ms: 1_794_150_179_000,
                    steps: 142,
                    movement: 310,
                    heart_rate: 71,
                    kind: None,
                },
                r#"{"t":"act","ts":1794150179000,"stp":142,"mov":310,"hrm":71}"#,
            ),
            (
                Outgoing::Activity {
                    timestamp_ms: 1_794_150_179_000,
                    steps: 0,
                    movement: 3,
                    heart_rate: 0,
                    kind: Some("LIGHT_SLEEP"),
                },
                r#"{"t":"act","ts":1794150179000,"stp":0,"mov":3,"hrm":0,"act":"LIGHT_SLEEP"}"#,
            ),
            // A live report: `rt`, and no `ts` (Gadgetbridge dates it).
            (
                Outgoing::Live {
                    steps: 12,
                    heart_rate: 72,
                },
                r#"{"t":"act","stp":12,"hrm":72,"rt":1}"#,
            ),
            // The longest.
            (
                Outgoing::Live {
                    steps: u16::MAX,
                    heart_rate: u8::MAX,
                },
                r#"{"t":"act","stp":65535,"hrm":255,"rt":1}"#,
            ),
            (
                Outgoing::Call(CallAction::Ignore),
                r#"{"t":"call","n":"ignore"}"#,
            ),
            (
                Outgoing::Music(MusicCommand::PlayPause),
                r#"{"t":"music","n":"playpause"}"#,
            ),
            // The longest a grid press gets.
            (
                Outgoing::Button {
                    code: 99,
                    gesture: "double_tap",
                },
                r#"{"t":"intent","action":"ur.watch.BUTTON","extra":{"button":99,"gesture":"double_tap"}}"#,
            ),
        ];
        for (message, json) in cases {
            assert_eq!(message.to_line().as_str(), format!("{json}\r\n"));
        }
    }

    #[test]
    fn fetch_end() {
        // The end marker, then the empty track list that gets Gadgetbridge
        // to broadcast new data (and so export to Health Connect): in that
        // order, so the fetch is over before the export is set off.
        let lines: Vec<_> = super::gb::FETCH_END
            .iter()
            .map(|message| message.to_line())
            .collect();
        assert_eq!(
            lines,
            [
                "{\"t\":\"actfetch\",\"state\":\"end\"}\r\n",
                "{\"t\":\"actTrksList\",\"list\":[]}\r\n",
            ]
        );
    }

    #[test]
    fn intents_off() {
        // What Gadgetbridge answers a grid press with, its "Allow Intents"
        // setting off (`uartTxJSONError`).
        assert_eq!(
            parse(
                br#"GB({"t":"intent","err":"Android Intents not enabled, check Gadgetbridge Device Settings"})"#
            ),
            Some(Incoming::Phone {
                event: PhoneEvent::IntentsOff,
                text_as_bitmaps: false
            })
        );
        // Anything else about intents isn't for us.
        assert_eq!(
            parse(br#"GB({"t":"intent","action":"x"})"#),
            Some(Incoming::Ignored("intent"))
        );
    }
}

/// Fuzzing, with Bolero: what the tests above try with a few thousand
/// random lines, a coverage-guided fuzzer tries with as many as it's given
/// time for, keeping the ones that reach new code.
///
/// `cargo test` runs each harness over a few hundred random inputs (and
/// any saved in a corpus), so they're regression tests too. To fuzz one
/// properly (nightly, for the sanitizer):
///
/// ```text
/// cargo +nightly bolero test fuzz::any_line_is_safe_to_parse --engine libfuzzer -T 10min
/// ```
#[cfg(test)]
mod fuzz {
    use super::alarm::PHONE_ALARMS;
    use super::gb::{self, CallAction, Incoming, LineBuffer, MusicCommand, Outgoing};
    use super::phone::PhoneEvent;
    use super::tests::gadgetbridge_string;

    /// What a fuzzed line starts with, and ends with, picked by its first
    /// byte: nothing, or a message of each kind up to where its fields
    /// go, or a field's string, so the fuzzer needn't find `GB({"t":"`
    /// by itself before it gets anywhere.
    const FRAMES: [(&str, &str); 20] = [
        ("", ""),
        ("GB(", ")"),
        ("GB({", "})"),
        ("setTime(", ""),
        ("setTime(1762436800);E.setTimeZone(", ");"),
        (r#"GB({"t":"notify","#, "})"),
        (r#"GB({"t":"notify","src":"Signal","title":""#, r#""})"#),
        (
            r#"GB({"t":"notify","sender":"Kari","title":atob(""#,
            r#"")})"#,
        ),
        (r#"GB({"t":"call","cmd":"incoming","name":""#, r#""})"#),
        (r#"GB({"t":"call","#, "})"),
        (r#"GB({"t":"musicinfo","artist":""#, r#""})"#),
        (r#"GB({"t":"musicstate","#, "})"),
        (r#"GB({"t":"find","#, "})"),
        (r#"GB({"t":"alarm","d":["#, "]})"),
        (r#"GB({"t":"alarm","d":[{"h":7,"m":30,"#, "}]})"),
        (r#"GB({"t":"nav","instr":""#, r#""})"#),
        (r#"GB({"t":"nav","#, "})"),
        (r#"GB({"t":"gps","lat":59.9139,"lon":"#, "})"),
        (r#"GB({"t":"act","#, "})"),
        (r#"GB({"t":"actfetch","ts":"#, "})"),
    ];

    /// A string of the UI's is one line, as its labels need: no control
    /// characters (line breaks and tabs arrive as spaces).
    fn one_line(text: &str) {
        assert!(
            !text.chars().any(|c| ('\u{8}'..='\u{d}').contains(&c)),
            "a line break or tab in {text:?}"
        );
    }

    /// What `parse` promises of what it returns, whatever it was given.
    fn check(parsed: &Incoming<'_>, line: &[u8]) {
        match parsed {
            // Only a time a phone could mean (see `parse_set_time`).
            Incoming::SetTime {
                utc_seconds,
                offset_minutes,
            } => {
                assert!((1_577_836_800..4_102_444_800).contains(utc_seconds));
                assert!((-18 * 60..=18 * 60).contains(offset_minutes));
            }
            Incoming::Gps { accuracy, .. } => {
                assert!(!accuracy.is_some_and(f32::is_nan), "an accuracy that's NaN");
            }
            Incoming::IsGpsActive | Incoming::ActivityFetch { .. } | Incoming::Live { .. } => {}
            // The kind is the line's own `t`.
            Incoming::Ignored(kind) => {
                let quoted = format!("\"{kind}\"");
                assert!(line.windows(quoted.len()).any(|w| w == quoted.as_bytes()));
            }
            Incoming::Phone { event, .. } => match event {
                // As `TextBuilder` leaves text: no space at either end,
                // nor two together, and nothing but what the font draws.
                PhoneEvent::Notification { app, who } => {
                    for text in [app.to_string(), who.to_string()] {
                        assert!(!text.starts_with(' ') && !text.ends_with(' '));
                        assert!(!text.contains("  "), "two spaces in {text:?}");
                        assert!(!text.chars().any(char::is_control));
                        assert!(
                            text.chars()
                                .all(|c| u32::from(c) < 0x100 || c == '\u{2026}'),
                            "beyond Latin-1 in {text:?}"
                        );
                    }
                    assert!(app.chars().count() <= 24 && who.chars().count() <= 40);
                }
                PhoneEvent::IncomingCall { caller } => one_line(caller),
                PhoneEvent::MusicInfo { artist, track } => {
                    one_line(artist);
                    one_line(track);
                }
                // Only times of day there are (see `alarm::Phone::push`).
                PhoneEvent::Alarms(alarms) => {
                    assert!(alarms.alarms().len() <= PHONE_ALARMS);
                    for alarm in alarms.alarms() {
                        assert!(alarm.time < 24 * 60 && alarm.days < 0x80);
                    }
                }
                // Trimmed, with plain spaces for Google Maps' no-break
                // ones; and a step says something.
                PhoneEvent::Route(Some(step)) => {
                    for text in [step.street.as_str(), step.distance.as_str()] {
                        one_line(text);
                        assert!(!text.starts_with(' ') && !text.ends_with(' '));
                        assert!(!text.contains('\u{a0}'));
                    }
                }
                _ => {}
            },
        }
    }

    /// No line panics the parser (or overflows, or reads out of bounds:
    /// the fuzzer builds with the address sanitizer), whether it's
    /// nothing like a message or all but one; and what it makes of a line
    /// is something the UI can take (see `check`).
    ///
    /// The line arrives the way it would, too: through a [`LineBuffer`],
    /// which hands over the same line however Gadgetbridge cuts it up.
    #[test]
    fn any_line_is_safe_to_parse() {
        bolero::check!().for_each(|bytes: &[u8]| {
            let Some((&frame, rest)) = bytes.split_first() else {
                return;
            };
            let (start, end) = FRAMES[usize::from(frame) % FRAMES.len()];
            let line = [start.as_bytes(), rest, end.as_bytes()].concat();
            let parsed = gb::parse(&line);
            if let Some(parsed) = &parsed {
                check(parsed, &line);
            }

            // Now as bytes on the air, if it's a line that can be sent:
            // `\n` ends one, and the buffer strips what frames it. A
            // message may come out with less in it (see `LineBuffer`), but
            // for one where nothing's left out, what's read is the same;
            // and for any, what's read is still something the UI can take.
            let sendable = !line.contains(&b'\n')
                && line.first() != Some(&b'\x10')
                && line.last() != Some(&b'\r');
            if sendable {
                let sent = [b"\x10", &line[..], b"\r\n"].concat();
                // Cut anywhere, by the frame byte's other bits.
                let cut = usize::from(frame) * 7 % sent.len();
                let mut buffer = LineBuffer::new();
                let mut lines = 0;
                let mut receive = |received: &[u8]| {
                    lines += 1;
                    let read = gb::parse(received);
                    if received == &line[..] {
                        assert_eq!(read, parsed);
                    } else {
                        assert!(line.starts_with(b"GB({"), "only messages change");
                    }
                    if let Some(read) = &read {
                        check(read, received);
                    }
                };
                buffer.push(&sent[..cut], &mut receive);
                buffer.push(&sent[cut..], &mut receive);
                // A message may fit only once what's left out is, or
                // (with more `""`s than what they stand for) not.
                if line.starts_with(b"GB({") {
                    assert!(lines <= 1);
                } else {
                    assert_eq!(lines, usize::from(line.len() <= 1024 - 2));
                }
            }
        });
    }

    /// Whatever bytes arrive, in whatever pieces, the [`LineBuffer`]
    /// hands over the lines a reading of its docs would: what's between
    /// the `\n`s, less the `\x10`s in front and one `\r` behind, but for
    /// lines longer than it holds (1024 bytes), dropped whole, and the
    /// unfinished line at the end, kept for more. Bar `GB({…})` messages,
    /// which it may keep less of: see [`any_message_keeps_its_shape`].
    #[test]
    fn any_bytes_come_out_as_their_lines() {
        bolero::check!()
            .with_type::<(Vec<u8>, Vec<u8>)>()
            .for_each(|(bytes, cuts)| {
                // Long lines too: the first byte says how many times over.
                let Some((&times, bytes)) = bytes.split_first() else {
                    return;
                };
                let bytes = bytes.repeat(1 + usize::from(times) % 40);

                let mut expected: Vec<&[u8]> = bytes.split(|&b| b == b'\n').collect();
                let message = |line: &&[u8]| {
                    let mut line = *line;
                    while let Some(rest) = line.strip_prefix(b"\x10") {
                        line = rest;
                    }
                    line.starts_with(b"GB({")
                };
                if expected.iter().any(message) {
                    return;
                }
                expected.pop(); // unfinished
                let expected: Vec<&[u8]> = expected
                    .into_iter()
                    .filter(|line| line.len() <= 1024)
                    .map(|mut line| {
                        while let Some(rest) = line.strip_prefix(b"\x10") {
                            line = rest;
                        }
                        line.strip_suffix(b"\r").unwrap_or(line)
                    })
                    .collect();

                let mut buffer = LineBuffer::new();
                let mut lines: Vec<Vec<u8>> = Vec::new();
                let mut rest = &bytes[..];
                for &cut in cuts {
                    let (now, later) = rest.split_at(usize::from(cut).min(rest.len()));
                    buffer.push(now, |line| lines.push(line.to_vec()));
                    rest = later;
                }
                buffer.push(rest, |line| lines.push(line.to_vec()));
                assert_eq!(lines, expected);
            });
    }

    /// Whatever follows `GB({` (Gadgetbridge's lines, made of the
    /// fuzzer's strings, or any bytes), what the [`LineBuffer`] keeps of
    /// it:
    ///
    /// - is the same however it was cut up on the way;
    /// - fits, if Gadgetbridge sent it, and reads as a message the UI can
    ///   take (see `check`);
    /// - is kept as it is if the buffer takes it in again: what's left
    ///   out stays out, and what's cut is cut at the same place, so the
    ///   strings left end where they did, and nothing more goes.
    #[test]
    fn any_message_keeps_its_shape() {
        bolero::check!()
            .with_type::<(u8, Vec<u8>, Vec<u16>, Vec<u16>)>()
            .for_each(|(how, bytes, text, cuts)| {
                // A Gadgetbridge message, with `text` (as UTF-16 units, as
                // Java has them; lone surrogates as U+FFFD) in the fields
                // we read and those we don't, as long as `how` says:
                // or any bytes.
                let text = String::from_utf16_lossy(&text[..text.len().min(500)]);
                let text = text.repeat(1 + usize::from(how >> 4));
                let spelled = |len: usize| {
                    let text: String = text.chars().take(len).collect();
                    String::from_utf8_lossy(&gadgetbridge_string(&text)).into_owned()
                };
                let line = match how % 5 {
                    0 => format!(
                        r#"GB({{"t":"notify","id":17,"src":{},"title":{},"subject":{},"body":{},"sender":{},"tel":{},"reply":true,"actions":[{{"title":{}}}]}})"#,
                        spelled(usize::MAX),
                        spelled(80),
                        spelled(80),
                        spelled(400),
                        spelled(40),
                        spelled(usize::MAX),
                        spelled(40),
                    )
                    .into_bytes(),
                    1 => format!(
                        r#"GB({{"t":"musicinfo","artist":{},"album":{},"track":{},"dur":1,"c":2,"n":3}})"#,
                        spelled(usize::MAX),
                        spelled(usize::MAX),
                        spelled(usize::MAX),
                    )
                    .into_bytes(),
                    2 => format!(
                        r#"GB({{"t":"call","cmd":"incoming","name":{},"number":{}}})"#,
                        spelled(usize::MAX),
                        spelled(usize::MAX),
                    )
                    .into_bytes(),
                    3 => format!(
                        r#"GB({{"t":"nav","instr":{},"distance":{},"action":"left","eta":{}}})"#,
                        spelled(usize::MAX),
                        spelled(usize::MAX),
                        spelled(usize::MAX),
                    )
                    .into_bytes(),
                    _ => [b"GB({", &bytes[..]].concat(),
                };
                let from_gadgetbridge = how % 5 != 4;
                if line.contains(&b'\n') {
                    return;
                }
                let sent = [b"\x10", &line[..], b"\n"].concat();

                let through = |pieces: &mut dyn Iterator<Item = &[u8]>| {
                    let mut buffer = LineBuffer::new();
                    let mut lines: Vec<Vec<u8>> = Vec::new();
                    for piece in pieces {
                        buffer.push(piece, |line| lines.push(line.to_vec()));
                    }
                    assert!(lines.len() <= 1);
                    lines.pop()
                };
                let whole = through(&mut core::iter::once(&sent[..]));
                let mut rest = &sent[..];
                let mut pieces = Vec::new();
                for &cut in cuts {
                    let (now, later) = rest.split_at(usize::from(cut).min(rest.len()));
                    pieces.push(now);
                    rest = later;
                }
                pieces.push(rest);
                assert_eq!(through(&mut pieces.into_iter()), whole);

                let Some(kept) = whole else {
                    assert!(!from_gadgetbridge, "dropped: {}", String::from_utf8_lossy(&line));
                    return;
                };
                assert!(kept.len() <= gb::MAX_LINE);
                let read = gb::parse(&kept);
                if let Some(read) = &read {
                    check(read, &kept);
                }
                if from_gadgetbridge {
                    assert!(matches!(read, Some(Incoming::Phone { .. })), "{read:?}");
                }
                // (With the `\r` the buffer took off its end, if any.)
                let resent = [b"\x10", &kept[..], b"\r\n"].concat();
                let again = through(&mut core::iter::once(&resent[..]));
                assert_eq!(again.as_deref(), Some(&kept[..]), "{}", String::from_utf8_lossy(&kept));
            });
    }

    /// Every message the watch can send, with any numbers in it, fits its
    /// line (`to_line` panics otherwise), is one line of JSON, with the
    /// values it was given, by a JSON parser that isn't ours; and by
    /// ours, wrapped as the phone would send it, has the `t` it should.
    #[test]
    fn any_message_is_a_line_of_json() {
        type Numbers = (u8, i64, u16, u16, u8, u32, bool);
        bolero::check!().with_type::<Numbers>().for_each(
            |&(which, timestamp_ms, steps, movement, heart_rate, millivolts, flag)| {
                let kind = [
                    None,
                    Some("LIGHT_SLEEP"),
                    Some("DEEP_SLEEP"),
                    Some("REM_SLEEP"),
                ][usize::from(heart_rate) % 4];
                // `buttons::Gesture::name`.
                let gesture = [
                    "press",
                    "double_press",
                    "hold_press",
                    "long_press",
                    "hold_release",
                ][usize::from(steps) % 5];
                let (message, t) = match which % 10 {
                    0 => (Outgoing::Version, "ver"),
                    1 => (
                        Outgoing::Status {
                            percent: heart_rate,
                            charging: flag,
                            millivolts,
                        },
                        "status",
                    ),
                    2 => (Outgoing::GpsPower(flag), "gps_power"),
                    3 => (Outgoing::ActivityFetch { done: flag }, "actfetch"),
                    4 => (Outgoing::NoTracks, "actTrksList"),
                    5 => (
                        // A bucket's steps: more than a minute's.
                        Outgoing::Activity {
                            timestamp_ms,
                            steps: millivolts,
                            movement,
                            heart_rate,
                            kind,
                        },
                        "act",
                    ),
                    6 => (
                        Outgoing::Call(
                            [CallAction::Accept, CallAction::Reject, CallAction::Ignore]
                                [usize::from(steps) % 3],
                        ),
                        "call",
                    ),
                    7 => (
                        Outgoing::Music(
                            [
                                MusicCommand::PlayPause,
                                MusicCommand::Next,
                                MusicCommand::Previous,
                            ][usize::from(steps) % 3],
                        ),
                        "music",
                    ),
                    8 => (Outgoing::Live { steps, heart_rate }, "act"),
                    _ => (
                        Outgoing::Button {
                            code: heart_rate,
                            gesture,
                        },
                        "intent",
                    ),
                };
                let line = message.to_line();
                let json = line.strip_suffix("\r\n").expect("ends in \\r\\n");
                assert!(!json.contains(['\r', '\n']), "more than one line");

                let value: serde_json::Value = serde_json::from_str(json).expect("JSON");
                assert_eq!(value["t"], t);
                match message {
                    Outgoing::Status { .. } => {
                        assert_eq!(value["bat"], heart_rate);
                        assert_eq!(value["chg"], u8::from(flag));
                        // Volts, to the hundredth below.
                        let volts = value["volt"].as_f64().expect("a number");
                        let hundredths = f64::from(millivolts / 10);
                        assert!((volts * 100.0 - hundredths).abs() < 0.5);
                    }
                    Outgoing::GpsPower(on) => assert_eq!(value["status"], on),
                    Outgoing::ActivityFetch { done } => {
                        assert_eq!(value["state"], if done { "end" } else { "start" });
                    }
                    Outgoing::Activity { .. } => {
                        assert_eq!(value["ts"], timestamp_ms);
                        assert_eq!(value["stp"], millivolts);
                        assert_eq!(value["mov"], movement);
                        assert_eq!(value["hrm"], heart_rate);
                        assert_eq!(value.get("act").and_then(|kind| kind.as_str()), kind);
                    }
                    Outgoing::Live { .. } => {
                        assert_eq!(value["stp"], steps);
                        assert_eq!(value["hrm"], heart_rate);
                        assert_eq!(value["rt"], 1);
                    }
                    Outgoing::Button { .. } => {
                        assert_eq!(value["action"], "ur.watch.BUTTON");
                        assert_eq!(value["extra"]["button"], heart_rate);
                        assert_eq!(value["extra"]["gesture"], gesture);
                    }
                    _ => {}
                }

                // And as our own reader takes it: the kinds the phone
                // sends too come back as those, the rest by their `t`.
                let wrapped = format!("GB({json})");
                let read = gb::parse(wrapped.as_bytes()).expect("a message");
                match message {
                    Outgoing::ActivityFetch { .. } => {
                        assert_eq!(read, Incoming::ActivityFetch { since_ms: 0 });
                    }
                    Outgoing::Activity { steps, .. } => assert_eq!(
                        read,
                        Incoming::Live {
                            heart_rate: heart_rate != 0,
                            steps: steps != 0,
                            interval: None
                        }
                    ),
                    Outgoing::Live { .. } => assert_eq!(
                        read,
                        Incoming::Live {
                            heart_rate: heart_rate != 0,
                            steps: steps != 0,
                            interval: None
                        }
                    ),
                    Outgoing::Call(_) => assert_eq!(
                        read,
                        Incoming::Phone {
                            event: PhoneEvent::CallOver,
                            text_as_bitmaps: false
                        }
                    ),
                    _ => assert_eq!(read, Incoming::Ignored(t)),
                }
            },
        );
    }
}

/// Proofs, for `cargo kani`: what the fuzzer tries for some text, for
/// every run of characters up to a length.
#[cfg(kani)]
mod proofs {
    use super::phone::TextBuilder;

    /// How many characters the text below keeps, and how many are pushed
    /// at it: enough to fill it, overflow it, and go on after.
    const KEPT: usize = 3;
    const PUSHED: usize = 5;

    /// For every run of [`PUSHED`] characters (any of Unicode's: letters,
    /// spaces, controls, emoji, the invisible ones) pushed at a text
    /// that keeps [`KEPT`]: `TextBuilder::finish`'s own check holds (no
    /// 0 before the text's end and only 0s after, no space at either end
    /// nor two in a row, no control character but the ellipsis's, and
    /// `is_empty` agreeing), so `Text` needs no length; it shows no more
    /// characters than it keeps; and nothing panics on the way.
    #[kani::proof]
    #[kani::unwind(6)]
    fn any_characters_build_a_text_that_needs_no_length() {
        let mut builder: TextBuilder<KEPT> = TextBuilder::new();
        for _ in 0..PUSHED {
            builder.push(kani::any());
        }
        let empty = builder.is_empty();
        let text = builder.finish();
        assert_eq!(text.is_empty(), empty);
        assert!(text.chars().count() <= KEPT);
    }
}
