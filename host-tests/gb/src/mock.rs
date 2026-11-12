//! The watch against Gadgetbridge, both ways, with `gbmock` playing
//! Gadgetbridge: a model of its Bangle.js support, written from its
//! source (see that crate's docs for the commit).
//!
//! - **Phone → watch:** whatever the mock sends, for anything a phone can
//!   ask it to, goes through the firmware's line buffer and parser, and
//!   has to come out meaning what Gadgetbridge meant
//!   ([`means_what_gadgetbridge_meant`]).
//! - **Watch → phone:** whatever the firmware writes goes into the mock's
//!   receiver, which has to take it without complaint and end up with
//!   what the watch meant ([`understood`]).
//! - **Activity sync:** a [`Watch`] that answers fetches as `ble` does,
//!   and the mock's database after its upsert, over days of recording,
//!   fetching, held-back sleep and dropped connections
//!   ([`fuzz::activity_reaches_the_database_as_the_watch_sent_it`]).
//!
//! The Bolero harnesses are in [`fuzz`]; `cargo test` runs each briefly,
//! and to fuzz one properly:
//!
//! ```text
//! cargo +nightly bolero test mock::fuzz::NAME --engine libfuzzer -T 10min
//! ```

use std::collections::BTreeMap;

use gbmock::generate::{Action, Choices};
use gbmock::spec::{CallSpec, NavigationInfoSpec};
use gbmock::text::{crop_to_length, render_unicode_as_image};
use gbmock::{BatteryState, Event, Fixes, Gadgetbridge, Sent};

use super::alarm::{PHONE_ALARMS, Switch};
use super::gb::{self, Incoming, LineBuffer, Outgoing};
use super::gb_buckets::{self, Bucket, Fetch, Minute};
use super::phone::{PhoneEvent, Text};
use super::route::Turn;

// ==============================================================================
// Phone → watch
// ==============================================================================

/// `gb::MAX_LINE`: the longest line the watch takes.
const MAX_LINE: usize = 1024;

/// What the firmware makes of one thing the mock sent: the line its
/// buffer hands over, from the mock's own writes (cut at its MTU), parsed.
/// `None` if no line came out: one longer than the buffer is dropped.
fn received(sent: &Sent, parsed: impl FnOnce(Option<Incoming<'_>>)) -> Option<()> {
    let mut buffer = LineBuffer::new();
    let mut lines: Vec<Vec<u8>> = Vec::new();
    for packet in sent.packets() {
        buffer.push(packet, |line| lines.push(line.to_vec()));
    }
    assert!(
        lines.len() <= 1,
        "one write of Gadgetbridge's, several lines"
    );
    let line = lines.pop()?;
    parsed(gb::parse(&line));
    Some(())
}

/// Text as the watch keeps a caller's name or a track's: `N` bytes of
/// UTF-8, line breaks and tabs as spaces, cut where it stops fitting.
fn kept<const N: usize>(text: &str) -> heapless::String<N> {
    let mut out = heapless::String::new();
    for c in text.chars() {
        let c = if ('\u{8}'..='\u{d}').contains(&c) {
            ' '
        } else {
            c
        };
        if out.push(c).is_err() {
            break;
        }
    }
    out
}

/// Whether Gadgetbridge's own escaping garbles `text`, so that what
/// arrives isn't what it had, on a Bangle.js as here:
///
/// - DLE is always `\20` (`BangleJSDeviceSupport.java:427`), which with an
///   octal digit after it is another character altogether.
/// - A NUL starts an image ("Text as Bitmaps"), whether Gadgetbridge put
///   one there or the text just has a NUL in it.
///
/// Or whether the watch's line buffer may cut it ([`cut`]): then what
/// shows is a prefix, with an ellipsis.
fn garbled(text: &str) -> bool {
    let bytes = text.as_bytes();
    text.contains('\0')
        || bytes
            .windows(2)
            .any(|w| w[0] == 0x10 && (b'0'..=b'7').contains(&w[1]))
        || cut(text)
}

/// Whether `text`, as Gadgetbridge spells it, is longer than the watch's
/// line buffer keeps of one string (`gb::MAX_STRING`). What it shows of
/// one that is, is `host-tests/gb`'s own tests' business (a prefix that
/// reads the same on the watch, for all but a run of emoji).
fn cut(text: &str) -> bool {
    gbmock::json::string_to_wire(text).len() > gb::MAX_STRING
}

/// Checks that what the watch parsed from `sent` is what `action` meant,
/// for a Gadgetbridge set up as `gb` was when it sent it (`before`).
///
/// Returns whether the watch got the line at all.
fn means_what_gadgetbridge_meant(before: &Gadgetbridge, action: &Action, sent: &Sent) -> bool {
    let bitmaps = before.prefs.text_as_bitmaps;
    let render = |text: Option<&str>| render_unicode_as_image(text, bitmaps);
    let cropped = |text: &Option<String>, len| {
        render(Some(&crop_to_length(text.as_deref(), len))).expect("not null")
    };
    let context = || {
        format!(
            "{action:?}\nsent {:?}",
            String::from_utf8_lossy(&sent.bytes)
        )
    };

    let got = received(sent, |parsed| match (action, parsed) {
        // A line that isn't a message at all: a screenshot request, or
        // whatever Tasker sent.
        (Action::Screenshot, parsed) => assert_eq!(parsed, None, "{}", context()),
        (Action::UartTx(_), _) => {}

        (
            Action::SetTime | Action::Connect,
            Some(Incoming::SetTime {
                utc_seconds,
                offset_minutes,
            }),
        ) => {
            assert_eq!(utc_seconds, before.now_ms / 1000, "{}", context());
            // To the nearest minute, as the watch keeps it (either one,
            // for a zone half a minute between two: Gadgetbridge sends
            // hours as a `float`, which is that vague).
            let minutes = f64::from(before.zone_offset_ms) / 60_000.0;
            assert!(
                (f64::from(offset_minutes) - minutes).abs() < 0.501,
                "{offset_minutes} for {minutes}: {}",
                context()
            );
            if minutes.fract() == 0.0 {
                assert_eq!(f64::from(offset_minutes), minutes, "{}", context());
            }
        }
        // A time the watch refuses: outside 2020–2100, or a zone past
        // ±18 hours (which Java doesn't have).
        (Action::SetTime | Action::Connect, None) => {
            let seconds = before.now_ms / 1000;
            assert!(
                sent.bytes.starts_with(b"\x10setTime(")
                    && !(1_577_836_800..4_102_444_800).contains(&seconds),
                "{}",
                context()
            );
        }
        (Action::Connect, Some(Incoming::IsGpsActive)) => {}
        (Action::SetTime, Some(Incoming::Ignored("force_calendar_sync_start"))) => {}

        (
            Action::Notification(spec),
            Some(Incoming::Phone {
                event: PhoneEvent::Notification { app, who },
                text_as_bitmaps,
            }),
        ) => {
            let src = if spec.sms {
                "SMS Message".to_owned()
            } else {
                spec.source_name.clone().unwrap_or_default()
            };
            let title = cropped(&spec.title, 80);
            let sender = cropped(&spec.sender, 40);
            // The subject and the body never get past the line buffer.
            let texts = [&src, &title, &sender];
            // The warning: only for images, and Gadgetbridge only makes
            // those with the setting on.
            if !texts.iter().any(|text| cut(text)) {
                assert_eq!(
                    text_as_bitmaps,
                    texts.iter().any(|text| text.contains('\0')),
                    "{}",
                    context()
                );
            }
            if !texts.iter().any(|text| garbled(text)) {
                assert_eq!(app, Text::<24>::from(src.as_str()), "{}", context());
                let from_sender = Text::<40>::from(sender.as_str());
                let expected = if from_sender.is_empty() {
                    Text::<40>::from(title.as_str())
                } else {
                    from_sender
                };
                assert_eq!(who, expected, "{}", context());
            }
        }

        (
            Action::Call(spec),
            Some(Incoming::Phone {
                event,
                text_as_bitmaps: _,
            }),
        ) => {
            if spec.command == CallSpec::CALL_INCOMING {
                let name = render(spec.name.as_deref()).unwrap_or_default();
                let number = spec.number.clone().unwrap_or_default();
                let PhoneEvent::IncomingCall { caller } = event else {
                    panic!("not an incoming call: {}", context());
                };
                if !garbled(&name) && !garbled(&number) {
                    let name: heapless::String<40> = kept(&name);
                    let expected = if name.is_empty() { kept(&number) } else { name };
                    assert_eq!(caller, expected, "{}", context());
                }
            } else {
                assert_eq!(event, PhoneEvent::CallOver, "{}", context());
            }
        }

        (
            Action::MusicInfo(spec),
            Some(Incoming::Phone {
                event: PhoneEvent::MusicInfo { artist, track },
                ..
            }),
        ) => {
            let wanted =
                [&spec.artist, &spec.track].map(|text| render(text.as_deref()).unwrap_or_default());
            // (The album too: its images would throw the others off no
            // less.)
            let album = render(spec.album.as_deref()).unwrap_or_default();
            if !wanted.iter().any(|text| garbled(text)) && !garbled(&album) {
                assert_eq!(artist, kept::<40>(&wanted[0]), "{}", context());
                assert_eq!(track, kept::<40>(&wanted[1]), "{}", context());
            }
        }
        (
            Action::MusicState(spec),
            Some(Incoming::Phone {
                event: PhoneEvent::MusicState { playing },
                ..
            }),
        ) => assert_eq!(playing, spec.state == 0, "{}", context()),

        (
            Action::Find(start),
            Some(Incoming::Phone {
                event: PhoneEvent::Find { on },
                ..
            }),
        ) => assert_eq!(on, *start, "{}", context()),

        (
            Action::Alarms(alarms),
            Some(Incoming::Phone {
                event: PhoneEvent::Alarms(phone),
                ..
            }),
        ) => {
            // Those that are set, and a time of day (Gadgetbridge's own
            // are; the watch drops the rest), up to as many as it keeps.
            let wanted: Vec<_> = alarms
                .iter()
                .filter(|a| !a.unused)
                // As the watch reads a number: clamped at 0.
                .map(|a| (a.hour.max(0), a.minute.max(0), a))
                .filter(|(hour, minute, _)| *hour < 24 && *minute < 60)
                .take(PHONE_ALARMS)
                .collect();
            assert_eq!(phone.alarms().len(), wanted.len(), "{}", context());
            for (got, (hour, minute, alarm)) in phone.alarms().iter().zip(wanted) {
                assert_eq!(i32::from(got.time), hour * 60 + minute, "{}", context());
                let days = alarm.repetition.clamp(0, 255) as u8 & 0x7F;
                assert_eq!(got.days, days, "{}", context());
                let switch = if alarm.enabled {
                    Switch::On
                } else {
                    Switch::Off
                };
                assert_eq!(got.switch, switch, "{}", context());
            }
        }

        (
            Action::Navigation(spec),
            Some(Incoming::Phone {
                event: PhoneEvent::Route(step),
                ..
            }),
        ) => {
            let action = usize::try_from(spec.next_action)
                .ok()
                .filter(|&a| a > 0 && a < NavigationInfoSpec::ACTIONS.len())
                .map(|a| NavigationInfoSpec::ACTIONS[a]);
            let shows = |text: &Option<String>| {
                text.as_deref()
                    .is_some_and(|text| !kept::<40>(text).replace('\u{a0}', " ").trim().is_empty())
            };
            let garbled = [&spec.instruction, &spec.distance_to_turn]
                .iter()
                .any(|text| text.as_deref().is_some_and(garbled));
            match step {
                Some(step) => {
                    // Every action Gadgetbridge names is one the watch
                    // knows.
                    assert_eq!(step.turn, Turn::from_action(action), "{}", context());
                    if action.is_some() {
                        assert_ne!(step.turn, Turn::Unknown, "{}", context());
                    }
                }
                None if !garbled => assert!(
                    action.is_none() && !shows(&spec.instruction) && !shows(&spec.distance_to_turn),
                    "{}",
                    context()
                ),
                None => {}
            }
        }

        (
            Action::Gps(location),
            Some(Incoming::Gps {
                latitude_micro,
                longitude_micro,
                accuracy,
            }),
        ) => {
            // Millionths of a degree, cut (not rounded) from the digits
            // Java printed: within one of the double's own.
            for (micro, degrees) in [
                (latitude_micro, location.latitude),
                (longitude_micro, location.longitude),
            ] {
                let wanted = degrees * 1e6;
                // (Anything on Earth; a number past an `i64` saturates.)
                assert!(
                    wanted.abs() > 9e18
                        || (micro as f64 - wanted).abs() <= 1.0 + wanted.abs() * 1e-12,
                    "{micro} for {degrees}: {}",
                    context()
                );
            }
            // The accuracy, for any a phone could give (the watch's
            // single-precision reader isn't exact far outside it).
            let accuracy = accuracy.expect("always sent");
            let wanted = location.accuracy;
            if (1e-3..1e9).contains(&wanted) {
                assert!(
                    (accuracy - wanted).abs() <= wanted * 1e-5,
                    "{accuracy} for {wanted}: {}",
                    context()
                );
            }
        }

        (Action::FetchActivity | Action::FullSync, Some(Incoming::ActivityFetch { since_ms })) => {
            let wanted = match action {
                Action::FullSync => 0,
                _ if before.last_sync_ms != 0 => before.last_sync_ms,
                _ => before.now_ms - 86_400_000,
            };
            assert_eq!(since_ms, wanted, "{}", context());
        }

        (
            Action::LiveActivity(_)
            | Action::RealtimeHeartRate(_)
            | Action::RealtimeSteps(_)
            | Action::HeartRateInterval(_),
            Some(Incoming::Live {
                heart_rate: _,
                steps: _,
                interval,
            }),
        ) => {
            // The flags are checked against the mock's own afterwards
            // (it may send two of these for one action).
            assert!(interval.is_some(), "{}", context());
        }

        // What the watch has no use for, by the `t` Gadgetbridge gives it.
        (action, Some(Incoming::Ignored(kind))) => {
            let wanted: &[&str] = match action {
                Action::DeleteNotification(_) => &["notify-"],
                Action::Volume(_) => &["audio"],
                Action::Vibrate(_) => &["vibrate"],
                Action::FetchTracks(_) => &["listRecs"],
                Action::Weather(_) => &["weather"],
                Action::CalendarEvent(_) => &["calendar"],
                Action::DeleteCalendarEvent(_) | Action::DeleteCalendarEvents(_) => &["calendar-"],
                Action::CannedMessages(_) => &["canned_responses_sync"],
                Action::LoyaltyCards(_) => &["cards"],
                Action::AccelSender(_) => &["accelsender"],
                _ => &[],
            };
            assert!(wanted.contains(&kind), "{kind}: {}", context());
        }
        // A missed call's notification: checked as a notification when
        // it's made, in `gbmock`.
        (Action::RunDelayed, Some(Incoming::Phone { .. })) => {}

        (_, parsed) => panic!("{parsed:?}: {}", context()),
    });
    got.is_some()
}

// ==============================================================================
// Watch → phone
// ==============================================================================

/// Gives the mock one of the watch's messages, as the watch writes it, and
/// checks that Gadgetbridge takes it as meant. `gb` is connected, with
/// nothing waiting in it.
fn understood(gb: &mut Gadgetbridge, message: Outgoing) {
    let line = message.to_line();
    let context = || format!("{:?}", line.as_str());
    let intents = gb.prefs.intents;
    let live = gb.realtime_hrm || gb.realtime_step;
    let was_busy = gb.busy.is_some();
    let was_gps = gb.gps_update_setup;
    let samples_before = gb.samples.clone();
    // In writes of the size `ble` makes them: the MTU's 20 bytes at worst.
    for packet in line.as_bytes().chunks(20) {
        gb.receive(packet);
    }
    assert!(!gb.has_partial_line(), "{}", context());
    let events = gb.take_events();
    // Never anything Gadgetbridge complains about, or can't place.
    for event in &events {
        assert!(
            !matches!(
                event,
                Event::MalformedJson(_)
                    | Event::Uncaught(_)
                    | Event::IgnoredLine(_)
                    | Event::NotUnderstood(_)
            ),
            "{event:?} for {}",
            context()
        );
    }
    match message {
        Outgoing::Version => {
            let fw = format!("ur {}", env!("CARGO_PKG_VERSION"));
            assert_eq!(gb.firmware.as_deref(), Some(fw.as_str()), "{}", context());
            assert_eq!(gb.hardware.as_deref(), Some("PineTime"), "{}", context());
        }
        Outgoing::Status {
            percent,
            charging,
            millivolts,
        } => {
            let [
                Event::Battery {
                    level: Some(level),
                    state,
                    voltage: Some(voltage),
                },
            ] = events[..]
            else {
                panic!("{events:?} for {}", context());
            };
            let wanted = if charging {
                BatteryState::Charging
            } else {
                BatteryState::Normal
            };
            assert_eq!(state, wanted, "{}", context());
            // Gadgetbridge smooths the level (see `gbmock`), but never
            // shows one the watch can't have: at most 100.
            assert!((0..=100).contains(&level), "{}", context());
            // And never one further along than the watch said: not above
            // it while draining, nor below it while charging.
            let said = i32::from(percent.min(100));
            assert!(
                if charging {
                    level >= said
                } else {
                    level <= said
                },
                "{}",
                context()
            );
            // To the centivolt, as sent.
            let volts = (millivolts / 10) as f32 / 100.0;
            assert!(
                (voltage - volts).abs() <= volts * 1e-6,
                "{voltage}: {}",
                context()
            );
        }
        Outgoing::GpsPower(on) => {
            assert_eq!(gb.gps_update_setup, on, "{}", context());
            // Told once, however often the watch says so.
            let told = events.contains(&Event::LocationUpdates(on));
            assert_eq!(
                told,
                on != was_gps && (gb.prefs.gps_update || !on),
                "{}",
                context()
            );
        }
        Outgoing::ActivityFetch { done: false } => assert!(gb.busy.is_some(), "{}", context()),
        Outgoing::ActivityFetch { done: true } => {
            assert_eq!(gb.busy, None, "{}", context());
            assert_eq!(gb.last_sync_ms, gb.now_ms - 60_000, "{}", context());
            assert_eq!(
                events.contains(&Event::NewData),
                gb.fixes.new_data_on_fetch_end,
                "{}",
                context()
            );
            assert_eq!(
                events.contains(&Event::UnsetBusyWhenIdle),
                !was_busy,
                "{}",
                context()
            );
        }
        // Its whole point: Gadgetbridge says new data is in.
        Outgoing::NoTracks => assert!(events.contains(&Event::NewData), "{}", context()),
        Outgoing::Activity {
            timestamp_ms,
            steps,
            movement,
            heart_rate,
            kind,
        } => {
            // Stored, or merged into a sample within two minutes of it;
            // `activity_reaches_the_database_as_the_watch_sent_it` is
            // about which. Here: one stored on its own is as sent.
            let timestamp = (timestamp_ms / 1000) as i32;
            if samples_before
                .range(timestamp.saturating_sub(120)..=timestamp.saturating_add(120))
                .next()
                .is_none()
                && i32::try_from(timestamp_ms / 1000).is_ok()
            {
                let stored = gb
                    .samples
                    .get(&timestamp)
                    .unwrap_or_else(|| panic!("{}", context()));
                assert_eq!(i64::from(stored.steps), i64::from(steps), "{}", context());
                assert_eq!(stored.heart_rate, i32::from(heart_rate), "{}", context());
                assert_eq!(stored.raw_intensity, i32::from(movement), "{}", context());
                assert_eq!(stored.kind(), kind.unwrap_or("ACTIVITY"), "{}", context());
            }
        }
        Outgoing::Call(action) => {
            let wanted = match action {
                gb::CallAction::Accept => "ACCEPT",
                gb::CallAction::Reject => "REJECT",
                gb::CallAction::Ignore => "IGNORE",
            };
            assert_eq!(events, [Event::CallControl(wanted)], "{}", context());
        }
        Outgoing::Music(command) => {
            let wanted = match command {
                gb::MusicCommand::PlayPause => "PLAYPAUSE",
                gb::MusicCommand::Next => "NEXT",
                gb::MusicCommand::Previous => "PREVIOUS",
            };
            assert_eq!(events, [Event::MusicControl(wanted)], "{}", context());
        }
        Outgoing::Live { steps, heart_rate } => {
            // Never stored; charted while Gadgetbridge has it as live.
            assert_eq!(gb.samples, samples_before, "{}", context());
            let charted = events.iter().find_map(|event| match event {
                Event::RealtimeSample(sample) => Some((sample.steps, sample.heart_rate)),
                _ => None,
            });
            let wanted = live.then_some((i32::from(steps), i32::from(heart_rate)));
            assert_eq!(charted, wanted, "{}", context());
        }
        Outgoing::Button { code, gesture } => {
            if intents {
                let [
                    Event::Intent {
                        target,
                        action,
                        extras,
                        ..
                    },
                ] = &events[..]
                else {
                    panic!("{events:?} for {}", context());
                };
                assert_eq!(target, "broadcastreceiver", "{}", context());
                assert_eq!(action.as_deref(), Some("ur.watch.BUTTON"), "{}", context());
                // An `int` and a `String` extra, by their JSON types.
                assert_eq!(
                    extras[..],
                    [
                        (
                            "button".to_owned(),
                            gbmock::json::Json::Int(i32::from(code))
                        ),
                        (
                            "gesture".to_owned(),
                            gbmock::json::Json::Str(gesture.into())
                        ),
                    ],
                    "{}",
                    context()
                );
            } else {
                // Refused, with an answer the watch reads as just that.
                assert!(events.is_empty(), "{events:?} for {}", context());
                let sent = gb.take_sent();
                let [answer] = &sent[..] else {
                    panic!("{sent:?} for {}", context());
                };
                received(answer, |parsed| {
                    assert_eq!(
                        parsed,
                        Some(Incoming::Phone {
                            event: PhoneEvent::IntentsOff,
                            text_as_bitmaps: false
                        }),
                        "{}",
                        context()
                    );
                })
                .expect("it fits");
            }
        }
    }
    assert!(
        gb.take_sent().is_empty(),
        "an answer nobody asked for: {}",
        context()
    );
}

// ==============================================================================
// The watch's side of an activity fetch
// ==============================================================================

/// What a minute is to Gadgetbridge (`sleep::Kind::gadgetbridge_name`).
type Kind = Option<&'static str>;

/// How far back the watch may still change a minute's kind
/// (`stages::REVISED`): where the story below has it look at the night
/// again.
const REVISED: i64 = 164;
/// `activity::HISTORY_MINUTES`.
const HISTORY_MINUTES: i64 = 24 * 60;

/// The watch, as far as an activity fetch goes: its clock, its minutes,
/// and `ble::send_activity`'s arithmetic over them. (`ble` itself needs
/// the radio; this is its logic, restated, with the firmware's own
/// `gb_buckets::start`, `gb_buckets::Fetch` and `Outgoing` doing the work.)
struct Watch {
    /// UTC seconds, once the phone has set it.
    now: Option<i64>,
    /// The history: UTC minute to what was recorded in it.
    history: BTreeMap<i64, Minute<Kind>>,
    /// `system_off::synced_through`.
    synced: Option<i64>,
    /// `activity::gadgetbridge_hold`'s `from`: the minutes from here on
    /// may still change kind, so aren't sent.
    hold: Option<i64>,
    lines: LineBuffer,
    /// What `synced` becomes if the fetch under way completes.
    pending_sync: Option<i64>,
}

impl Watch {
    fn new() -> Self {
        Self {
            now: None,
            history: BTreeMap::new(),
            synced: None,
            hold: None,
            lines: LineBuffer::new(),
            pending_sync: None,
        }
    }

    /// Records a minute, dropping those a day older (`minute_ring`).
    fn record(&mut self, minute: i64, sample: Minute<Kind>) {
        self.history.insert(minute, sample);
        self.history = self.history.split_off(&(minute - HISTORY_MINUTES + 1));
    }

    /// Takes the phone's bytes; answers with the watch's messages, in
    /// order, as `ble` would send them.
    fn receive(&mut self, bytes: &[u8]) -> Vec<Outgoing> {
        let mut requests = Vec::new();
        self.lines.push(bytes, |line| match gb::parse(line) {
            Some(Incoming::SetTime { utc_seconds, .. }) => requests.push(Err(utc_seconds)),
            Some(Incoming::ActivityFetch { since_ms }) => requests.push(Ok(Some(since_ms))),
            Some(Incoming::IsGpsActive) => requests.push(Ok(None)),
            _ => {}
        });
        let mut out = Vec::new();
        for request in requests {
            match request {
                Err(utc_seconds) => self.now = Some(utc_seconds),
                Ok(None) => out.push(Outgoing::GpsPower(false)),
                Ok(Some(since_ms)) => out.extend(self.send_activity(since_ms)),
            }
        }
        out
    }

    /// `ble::send_activity`, less the sending: every message of a fetch.
    /// [`Watch::completed`] is what it does at the end, if every one of
    /// them went out.
    fn send_activity(&mut self, since_ms: i64) -> Vec<Outgoing> {
        let mut out = vec![Outgoing::ActivityFetch { done: false }];
        if let Some(now) = self.now {
            let now = now.div_euclid(60);
            let newest = self.history.keys().next_back().copied();
            let oldest = newest.map_or(i64::MIN / 2, |newest| {
                newest - HISTORY_MINUTES + 1 + gb_buckets::MINUTES
            });
            let since = gb_buckets::start(since_ms, self.synced, now);
            let mut fetch = Fetch::new(since, oldest, now);
            if let Some(hold) = self.hold {
                fetch.hold(hold);
            }
            while let Some(bucket) = fetch.next(
                self.history
                    .range(fetch.from()..)
                    .map(|(&minute, &sample)| (minute, sample)),
            ) {
                out.push(Outgoing::Activity {
                    timestamp_ms: bucket.timestamp_ms(),
                    steps: bucket.steps,
                    movement: bucket.movement,
                    heart_rate: bucket.heart_rate,
                    kind: bucket.kind,
                });
            }
            self.pending_sync = Some(fetch.synced_through());
        }
        out.extend(gb::FETCH_END);
        out
    }

    /// The fetch's messages all went out: the phone has everything before
    /// the bucket under way, or the first one held back
    /// (`system_off::on_synced`).
    fn completed(&mut self) {
        if let Some(synced) = self.pending_sync.take() {
            self.synced = Some(synced);
        }
    }

    /// The bucket `minute` is in, straight from the history: what
    /// Gadgetbridge should have for it, by `gb_buckets`' rules, worked out
    /// the long way round.
    fn bucket(&self, minute: i64) -> Option<Bucket<Kind>> {
        let start = gb_buckets::start_of(minute);
        let minutes: Vec<_> = self
            .history
            .range(start..start + gb_buckets::MINUTES)
            .map(|(_, sample)| *sample)
            .collect();
        let last = minutes.last()?;
        let seen: Vec<u32> = minutes
            .iter()
            .filter(|m| m.seen)
            .map(|m| u32::from(m.movement))
            .collect();
        // The kind most minutes have, the later on a tie.
        let votes = |kind: Kind| minutes.iter().filter(|m| m.kind == kind).count();
        let mut kind = last.kind;
        for minute in minutes.iter().rev() {
            if votes(minute.kind) > votes(kind) {
                kind = minute.kind;
            }
        }
        Some(Bucket {
            start,
            steps: minutes.iter().map(|m| u32::from(m.steps)).sum(),
            movement: if seen.is_empty() {
                0
            } else {
                seen.iter().sum::<u32>().div_ceil(seen.len() as u32) as u16
            },
            heart_rate: minutes
                .iter()
                .rev()
                .map(|m| m.heart_rate)
                .find(|&rate| rate != 0)
                .unwrap_or(0),
            kind,
        })
    }
}

// ==============================================================================
// Tests
// ==============================================================================

/// A Gadgetbridge with the watch connected and nothing waiting.
fn connected() -> Gadgetbridge {
    let mut gb = Gadgetbridge::new();
    gb.connect();
    gb.take_sent();
    gb
}

/// Does `action`, and checks everything it sent against what it meant.
/// Returns how many lines the watch got, and how many it dropped as too
/// long.
fn perform_and_check(gb: &mut Gadgetbridge, action: &Action) -> (usize, usize) {
    let before = gb.clone();
    gb.perform(action);
    // Tasker's lines are whatever Tasker sent, line feeds and all:
    // `the_stream_comes_out_as_its_lines` has those.
    if matches!(action, Action::UartTx(_)) {
        return (gb.take_sent().len(), 0);
    }
    let (mut got, mut dropped) = (0, 0);
    for sent in gb.take_sent() {
        if means_what_gadgetbridge_meant(&before, action, &sent) {
            got += 1;
        } else {
            // The only lines the watch may lose are those longer than
            // its buffer, and never a notification, whatever its
            // length (see `a_long_notification_beyond_latin1_arrives`).
            assert!(
                sent.bytes.len() - 1 > MAX_LINE && !matches!(action, Action::Notification(_)),
                "dropped {:?}",
                String::from_utf8_lossy(&sent.bytes)
            );
            dropped += 1;
        }
    }
    (got, dropped)
}

#[test]
fn the_connect_sequence_sets_the_clock() {
    let mut gb = Gadgetbridge::new();
    let mut watch = Watch::new();
    gb.connect();
    let mut answers = Vec::new();
    for sent in gb.take_sent() {
        for packet in sent.packets() {
            answers.extend(watch.receive(packet));
        }
    }
    assert_eq!(watch.now, Some(gb.now_ms / 1000));
    // The answer to `is_gps_active`.
    assert!(matches!(answers[..], [Outgoing::GpsPower(false)]));
    // And what `ble` sends by itself once the phone listens.
    understood(&mut gb, Outgoing::Version);
    understood(
        &mut gb,
        Outgoing::Status {
            percent: 87,
            charging: false,
            millivolts: 4012,
        },
    );
}

/// Every time zone a phone can be in, at the pinned commit's `setTime`:
/// the offset the watch takes is the phone's, to the minute. Gadgetbridge
/// sends it in hours as a Java `float` (`5.75`, `-9.5`, and
/// `2.7777778E-4` for a second), which the watch reads back and rounds.
#[test]
fn every_time_zone_arrives_as_its_minutes() {
    let mut gb = connected();
    // Every whole minute Java allows (±18 hours), which covers every
    // zone there is or was since zones were whole minutes; and seconds
    // around each quarter hour, for the zones of the 1800s.
    let minutes = (-18 * 60..=18 * 60).map(|minutes| minutes * 60_000);
    let seconds = (-72..=72).flat_map(|quarter: i32| {
        (-40..=40).map(move |s| (quarter * 900 + s).clamp(-64_800, 64_800) * 1000)
    });
    for offset_ms in minutes.chain(seconds) {
        gb.zone_offset_ms = offset_ms;
        assert_eq!(
            perform_and_check(&mut gb, &Action::SetTime),
            (2, 0),
            "{offset_ms}"
        );
    }
}

/// Every second of a year, sampled, and the ends of what the watch
/// believes: the clock the watch gets is the phone's.
#[test]
fn the_time_arrives_as_the_phones() {
    let mut gb = connected();
    for seconds in (1_577_836_800..4_102_444_800i64)
        .step_by(86_399 * 13)
        .chain([
            1_577_836_800,
            4_102_444_799,
            1_577_836_799,
            4_102_444_800,
            0,
            -1,
        ])
    {
        for ms in [0, 999] {
            gb.now_ms = seconds * 1000 + ms;
            assert_eq!(
                perform_and_check(&mut gb, &Action::SetTime),
                (2, 0),
                "{seconds}"
            );
        }
    }
}

/// A location within 111 m of the equator or the Greenwich meridian is
/// one Java prints with an exponent (`5.0E-4`).
#[test]
fn a_location_by_the_meridian() {
    let mut gb = connected();
    gb.prefs.use_updated_location = true;
    for micro in (-1500..=1500).step_by(7) {
        let location = gbmock::spec::Location {
            latitude: 51.4779,
            longitude: f64::from(micro) / 1e6,
            altitude: 46.0,
            speed: 1.5,
            bearing: Some(271.5),
            time: 1_794_312_000_123,
            satellites: None,
            accuracy: 3.9,
            provider: Some("fused".into()),
        };
        assert_eq!(perform_and_check(&mut gb, &Action::Gps(location)), (1, 0));
    }
}

/// Gadgetbridge sends each character beyond U+00FF as six bytes
/// (`\\uXXXX`), and crops a notification's body at 400 characters, its
/// title and subject at 80 and its sender at 40: up to 3,700 bytes a
/// line, against the watch's 1,024 (`gb::MAX_LINE`). The line buffer
/// leaves out what the watch never reads (the body, the subject) and cuts
/// long strings, so the notification arrives whatever its length.
/// (Before 47fa44d it was dropped whole from about 150 characters:
/// report, G1.)
#[test]
fn a_long_notification_beyond_latin1_arrives() {
    let mut gb = connected();
    let notification = |body: String| {
        Action::Notification(gbmock::spec::NotificationSpec {
            id: 17,
            source_name: Some("Signal".into()),
            title: Some("Анна".into()),
            body: Some(body),
            ..Default::default()
        })
    };
    for body in [
        "Привет ".repeat(20),
        "Привет".repeat(30),
        "中文".repeat(400),
        "blåbærsyltetøy ".repeat(40),
    ] {
        assert_eq!(perform_and_check(&mut gb, &notification(body)), (1, 0));
    }
}

/// What a phone at its defaults does with emoji: they arrive as ASCII
/// (`:-D`, `<3`, `:house_with_garden:`), since "Text as Bitmaps" is off
/// and Gadgetbridge then converts them itself
/// (`BangleJSDeviceSupport.java:1388-1390`). So the watch's own handling
/// of emoji (leaving them out) only ever sees those Gadgetbridge's
/// library doesn't know; the shortcodes for the rest it leaves out the
/// same way (see `phone::Shortcode`), and the emoticons stay, as text.
#[test]
fn emoji_arrive_as_ascii_by_default() {
    let mut gb = connected();
    gb.on_notification(&gbmock::spec::NotificationSpec {
        id: 1,
        source_name: Some("Signal".into()),
        title: Some("Familien \u{1F3E1}: Nils \u{2764}".into()),
        ..Default::default()
    });
    let sent = gb.take_sent();
    received(&sent[0], |parsed| {
        let Some(Incoming::Phone {
            event: PhoneEvent::Notification { who, .. },
            ..
        }) = parsed
        else {
            panic!("a notification");
        };
        assert_eq!(who.to_string(), "Familien: Nils <3");
    })
    .expect("it fits");
}

/// Every minute from 2020 to 2038, where Gadgetbridge's `int` of seconds
/// ends: its bucket is filed (`handleActivity`'s `(int) (ts / 1000)`,
/// `BangleJSDeviceSupport.java:864`) at exactly the bucket's last second,
/// the 299th of a five-minute stretch of UTC that the minute is in.
/// `mock_proofs::buckets_are_filed_apart` takes it from there: any two such
/// seconds are the same, or too far apart for Gadgetbridge to merge.
///
/// A loop rather than a proof: it's 9.5 million minutes, a moment's
/// work, where Kani ran past 20 minutes on the 64-bit divisions.
#[test]
fn every_bucket_is_filed_at_its_last_second() {
    let seconds = gb_buckets::MINUTES * 60;
    let first = 1_577_836_800 / 60;
    let last = (i64::from(i32::MAX) - (seconds - 1)) / 60;
    for minute in first..=last {
        let start = gb_buckets::start_of(minute);
        let bucket = Bucket {
            start,
            steps: 0,
            movement: 0,
            heart_rate: 0,
            kind: (),
        };
        let at = (bucket.timestamp_ms() / 1000) as i32;
        assert!(start <= minute && minute < start + gb_buckets::MINUTES);
        assert_eq!(i64::from(at), (start + gb_buckets::MINUTES) * 60 - 1);
        assert_eq!(i64::from(at) % seconds, seconds - 1);
    }
}

/// A fetch, end to end: the request as Gadgetbridge sends it, the
/// watch's answer into Gadgetbridge's database.
#[test]
fn a_fetch_fills_the_database() {
    let mut gb = connected();
    let mut watch = Watch::new();
    let now = gb.now_ms / 60_000;
    watch.now = Some(gb.now_ms / 1000);
    for minute in now - 30..now {
        let sample = Minute {
            steps: 10,
            movement: 100,
            heart_rate: if minute % 5 == 2 { 60 } else { 0 },
            seen: true,
            kind: None,
        };
        watch.record(minute, sample);
    }
    gb.on_fetch_activity();
    let request = gb.take_sent();
    let answers: Vec<_> = request[0]
        .packets()
        .flat_map(|p| watch.receive(p))
        .collect();
    for message in answers {
        gb.receive(message.to_line().as_bytes());
    }
    // 12:00 is the bucket under way; the six before it are whole.
    assert_eq!(gb.samples.len(), 6);
    for (timestamp, sample) in &gb.samples {
        assert_eq!(timestamp % 300, 299);
        assert_eq!(
            (sample.steps, sample.heart_rate, sample.raw_intensity),
            (50, 60, 100)
        );
        assert_eq!(sample.kind(), "ACTIVITY");
    }
    assert_eq!(gb.last_sync_ms, gb.now_ms - 60_000);
    assert_eq!(gb.busy, None);
}

/// TODO: steps lost, kept as what happens today (report, G7). A fetch
/// whose answer never reaches the phone, though the watch believes it
/// sent it (`ble`'s notifications aren't acknowledged: a link that drops
/// with them queued looks like success), costs the bucket that was under
/// way at the fetch before it.
///
/// The watch marks everything as synced. Gadgetbridge saw no end, so it
/// asks again from its last good fetch, a minute before that one ended;
/// `gb_buckets::start` takes the first bucket *after* the one that minute
/// is in, as Gadgetbridge may hold an older build's minute samples there.
/// But that bucket was still under way at the good fetch, and so was
/// never sent.
///
/// Once fixed, the sample at 12:04:59 is there, with 50 steps.
#[test]
fn a_fetch_lost_in_the_air_costs_the_bucket_before_it() {
    let mut gb = connected();
    let mut watch = Watch::new();
    let walk = |watch: &mut Watch, gb: &mut Gadgetbridge, minutes: i64| {
        let now = gb.now_ms / 60_000;
        for minute in now..now + minutes {
            let sample = Minute {
                steps: 10,
                movement: 100,
                heart_rate: 0,
                seen: true,
                kind: None,
            };
            watch.record(minute, sample);
        }
        gb.now_ms += minutes * 60_000;
        watch.now = Some(gb.now_ms / 1000);
    };
    let fetch = |watch: &mut Watch, gb: &mut Gadgetbridge| -> Vec<Outgoing> {
        gb.on_fetch_activity();
        let request = gb.take_sent();
        request[0]
            .packets()
            .flat_map(|p| watch.receive(p))
            .collect()
    };
    let steps = |gb: &Gadgetbridge| gb.samples.values().map(|s| s.steps).sum::<i32>();

    // 11:30 to 12:02, and a fetch that goes well: up to 12:00.
    gb.now_ms -= 30 * 60_000;
    walk(&mut watch, &mut gb, 32);
    for message in fetch(&mut watch, &mut gb) {
        gb.receive(message.to_line().as_bytes());
    }
    watch.completed();
    assert_eq!(steps(&gb), 300);

    // To 12:22, and a fetch the phone hears nothing of.
    walk(&mut watch, &mut gb, 20);
    let lost = fetch(&mut watch, &mut gb);
    assert_eq!(lost.len(), 1 + 4 + 2, "12:00 to 12:20, in four buckets");
    watch.completed();
    gb.disconnect();
    gb.connect();
    gb.take_sent();

    // A minute on, the phone asks again, from 12:01.
    walk(&mut watch, &mut gb, 1);
    for message in fetch(&mut watch, &mut gb) {
        gb.receive(message.to_line().as_bytes());
    }
    watch.completed();
    // The watch counted 500 steps to 12:20. Gadgetbridge has 450: all
    // but 12:00 to 12:05.
    let noon = (gb.now_ms / 1000 - 23 * 60) as i32;
    assert_eq!(noon % 3600, 0);
    assert!(!gb.samples.contains_key(&(noon + 299)));
    assert!(gb.samples.contains_key(&(noon + 599)));
    assert_eq!(steps(&gb), 450);
}

/// TODO: not the watch's doing, but its to live with (report, G4). A line
/// cut off by a lost connection stays in Gadgetbridge's buffer, and the
/// first line of the next connection, the firmware version, is glued to
/// it: "Malformed JSON from Bangle.js" on the phone, and no version.
#[test]
fn a_line_cut_by_a_disconnect_costs_the_next_connections_first() {
    let mut gb = connected();
    let line = Outgoing::Activity {
        timestamp_ms: 1_794_311_999_000,
        steps: 12,
        movement: 3,
        heart_rate: 0,
        kind: None,
    }
    .to_line();
    gb.receive(&line.as_bytes()[..20]);
    gb.disconnect();
    gb.connect();
    gb.take_sent();
    gb.receive(Outgoing::Version.to_line().as_bytes());
    assert!(matches!(gb.take_events()[..], [Event::MalformedJson(_)]));
    assert_eq!(gb.firmware, None);
    assert!(gb.samples.is_empty());
}

pub mod fuzz {
    use super::*;

    /// Anything Gadgetbridge can send, for anything a phone can have it
    /// do, in any order and with any settings: the watch's line buffer
    /// hands over each line whole (or drops one that's too long, and
    /// nothing else), the parser reads it as what Gadgetbridge meant
    /// (see `means_what_gadgetbridge_meant`), and the live activity
    /// request the watch ends up with is the one Gadgetbridge has.
    #[test]
    fn whatever_gadgetbridge_sends_means_what_it_meant() {
        bolero::check!().for_each(|bytes: &[u8]| {
            let mut choices = Choices::new(bytes);
            let mut gb = Gadgetbridge::new();
            for action in gbmock::generate::connect_sequence(&mut choices) {
                perform_and_check(&mut gb, &action);
            }
            while !choices.is_empty() {
                let action = gbmock::generate::action(&mut choices);
                perform_and_check(&mut gb, &action);
            }
        });
    }

    /// The whole stream, rather than a line at a time: everything a
    /// session of Gadgetbridge's sends, through one line buffer in the
    /// writes it was cut into, gives the watch its lines in order, each
    /// short one exactly as it was sent. A line that didn't fit, or had
    /// something left out or cut, takes nothing else with it.
    #[test]
    fn the_stream_comes_out_as_its_lines() {
        bolero::check!().for_each(|bytes: &[u8]| {
            let mut choices = Choices::new(bytes);
            let mut gb = Gadgetbridge::new();
            gb.connect();
            let mut buffer = LineBuffer::new();
            while !choices.is_empty() {
                // Now and then the watch sends something long enough to
                // raise the MTU Gadgetbridge cuts its writes at.
                if choices.chance(8) {
                    gb.receive(&vec![b' '; usize::from(choices.byte())]);
                }
                gb.perform(&gbmock::generate::action(&mut choices));
                if !gb.connected {
                    // What `ble` does on a disconnect.
                    buffer.clear();
                    gb.take_sent();
                    continue;
                }
                let sent = gb.take_sent();
                let mut lines: Vec<Vec<u8>> = Vec::new();
                for packet in sent.iter().flat_map(Sent::packets) {
                    buffer.push(packet, |line| lines.push(line.to_vec()));
                }
                // What was sent: each thing, cut at its line feeds
                // (Tasker's lines may have several), less the DLEs in
                // front and a `\r` behind.
                let stream: Vec<u8> = sent.iter().flat_map(|s| s.bytes.clone()).collect();
                let all: Vec<Vec<u8>> = stream
                    .split(|&b| b == b'\n')
                    .take(stream.iter().filter(|&&b| b == b'\n').count())
                    .map(|mut line| {
                        while let [0x10, rest @ ..] = line {
                            line = rest;
                        }
                        line.strip_suffix(b"\r").unwrap_or(line).to_vec()
                    })
                    .collect();
                // No line from nowhere.
                assert!(lines.len() <= all.len());
                // The buffer leaves out what the watch never reads and
                // cuts long strings, so a line may come out shorter than
                // it went in. One with nothing to leave out or cut (no
                // string can be long in a short line) comes out as it
                // went in, and in order.
                let unread = [&b"\"subject\""[..], b"\"body\"", b"\"tel\"", b"\"actions\""];
                let plain = |line: &&Vec<u8>| {
                    line.len() <= gb::MAX_STRING
                        && !unread
                            .iter()
                            .any(|key| line.windows(key.len()).any(|w| w == *key))
                };
                let mut out = lines.iter();
                for line in all.iter().filter(plain) {
                    assert!(
                        out.any(|got| got == line),
                        "lost {:?}",
                        String::from_utf8_lossy(line)
                    );
                }
            }
        });
    }

    /// Any message the watch can write, with any values, into a
    /// Gadgetbridge in any state a session leaves it in: taken without a
    /// complaint (no malformed JSON, no uncaught exception, nothing it
    /// can't place), and meaning what the watch meant (see `understood`).
    #[test]
    fn whatever_the_watch_sends_is_understood() {
        type Values = (u8, i64, u32, u16, u8, bool, u8);
        bolero::check!()
            .with_type::<(Vec<Values>, u16)>()
            .for_each(|(messages, settings)| {
                let mut gb = connected();
                gb.prefs.intents = settings & 1 != 0;
                gb.prefs.gps_update = settings & 2 != 0;
                gb.prefs.allow_high_mtu = settings & 4 != 0;
                if settings & 8 != 0 {
                    gb.fixes = Fixes::NONE;
                }
                if settings & 16 != 0 {
                    gb.live_activity(true);
                    gb.take_sent();
                }
                for &(which, timestamp_ms, steps, movement, heart_rate, flag, pick) in messages {
                    let kinds = [
                        None,
                        Some("LIGHT_SLEEP"),
                        Some("DEEP_SLEEP"),
                        Some("REM_SLEEP"),
                        Some("AWAKE_SLEEP"),
                        Some("NOT_WORN"),
                    ];
                    // `buttons::Gesture::name`.
                    let gestures = [
                        "press",
                        "double_press",
                        "hold_press",
                        "long_press",
                        "hold_release",
                    ];
                    let pick = usize::from(pick);
                    let message = match which % 11 {
                        0 => Outgoing::Version,
                        1 => Outgoing::Status {
                            percent: heart_rate,
                            charging: flag,
                            millivolts: steps,
                        },
                        2 => Outgoing::GpsPower(flag),
                        3 => Outgoing::ActivityFetch { done: flag },
                        4 => Outgoing::NoTracks,
                        5 | 6 => Outgoing::Activity {
                            timestamp_ms,
                            // At most a bucket's: five minutes of a
                            // `u16` each. (Past 2^31, Gadgetbridge's
                            // `int` would wrap.)
                            steps: steps % (5 * 65_536),
                            movement,
                            heart_rate,
                            kind: kinds[pick % kinds.len()],
                        },
                        7 => Outgoing::Call(
                            [
                                gb::CallAction::Accept,
                                gb::CallAction::Reject,
                                gb::CallAction::Ignore,
                            ][pick % 3],
                        ),
                        8 => Outgoing::Music(
                            [
                                gb::MusicCommand::PlayPause,
                                gb::MusicCommand::Next,
                                gb::MusicCommand::Previous,
                            ][pick % 3],
                        ),
                        9 => Outgoing::Live {
                            steps: movement,
                            heart_rate,
                        },
                        _ => Outgoing::Button {
                            code: heart_rate,
                            gesture: gestures[pick % gestures.len()],
                        },
                    };
                    understood(&mut gb, message);
                }
            });
    }

    /// One step of the story `activity_reaches_the_database…` tells.
    fn story(choices: &mut Choices<'_>, fixes: Fixes) {
        let kinds: [Kind; 6] = [
            None,
            None,
            Some("LIGHT_SLEEP"),
            Some("DEEP_SLEEP"),
            Some("REM_SLEEP"),
            Some("AWAKE_SLEEP"),
        ];
        let mut gb = Gadgetbridge::new();
        gb.fixes = fixes;
        let mut watch = Watch::new();
        // Each bucket delivered, by its timestamp: what the watch means
        // Gadgetbridge to have.
        let mut meant: BTreeMap<i32, Bucket<Kind>> = BTreeMap::new();
        // The minutes whose bucket has been delivered.
        let mut delivered: std::collections::BTreeSet<i64> = Default::default();
        // The minutes from here on haven't been sent, so may change kind.
        let mut settled = i64::MIN;
        // The earliest minute a fetch that completed has reached back to.
        let mut asked = i64::MAX;

        let connect = |gb: &mut Gadgetbridge, watch: &mut Watch| {
            gb.connect();
            watch.lines.clear();
            for sent in gb.take_sent() {
                for packet in sent.packets() {
                    for answer in watch.receive(packet) {
                        gb.receive(answer.to_line().as_bytes());
                    }
                }
            }
            gb.receive(Outgoing::Version.to_line().as_bytes());
        };
        connect(&mut gb, &mut watch);

        while !choices.is_empty() {
            let now = gb.now_ms.div_euclid(60_000);
            match choices.byte() % 8 {
                // Time passes, a minute at a time, each recorded (or, now
                // and then, not: the watch was off).
                0..=3 => {
                    let minutes = match choices.byte() {
                        0..=199 => i64::from(choices.below(20)) + 1,
                        200..=249 => i64::from(choices.below(300)) + 1,
                        _ => i64::from(choices.below(30 * 60)) + 1,
                    };
                    let recorded = choices.chance(230);
                    let (steps, rate, kind) = (choices.byte(), choices.byte(), choices.byte());
                    for minute in now..now + minutes {
                        if recorded {
                            let vary = (minute % 7) as u8;
                            watch.record(
                                minute,
                                Minute {
                                    steps: if steps % 3 == 0 {
                                        0
                                    } else {
                                        u16::from(steps) + u16::from(vary)
                                    },
                                    movement: u16::from(steps) * 16 * u16::from(vary % 3),
                                    heart_rate: if (minute + i64::from(rate)) % 4 == 0 {
                                        rate
                                    } else {
                                        0
                                    },
                                    seen: vary != 6,
                                    kind: kinds
                                        [usize::from(kind.wrapping_add(vary / 3)) % kinds.len()],
                                },
                            );
                        }
                    }
                    gb.now_ms += minutes * 60_000 + i64::from(choices.below(60_000));
                    // The watch's clock is the phone's: it was set on
                    // connecting, and both run.
                    watch.now = Some(gb.now_ms.div_euclid(1000));
                }
                // The watch looks at the night again: the kinds of minutes
                // not yet sent change, and it holds back from where they
                // may change again (`sleep_hold`), or lets go.
                4 => {
                    let back = i64::from(choices.below(REVISED as u32 + 30));
                    let kind = kinds[usize::from(choices.byte()) % kinds.len()];
                    for (_, sample) in watch.history.range_mut(settled.max(now - back)..) {
                        sample.kind = kind;
                    }
                    watch.hold = choices.chance(128).then_some(now - back);
                }
                // A fetch: Gadgetbridge asks, the watch answers, and the
                // connection may drop anywhere in the answer.
                5..=6 => {
                    let full = choices.chance(16);
                    if full {
                        gb.on_full_sync();
                    } else {
                        gb.on_fetch_activity();
                    }
                    // Where this fetch reaches back to: the first bucket
                    // after all Gadgetbridge may have, going by what it
                    // asks from (the last day, the first time; see
                    // `gb_buckets::start`). The watch starts there, or
                    // where its own last fetch stopped, if that's earlier.
                    let since_ms = if full {
                        0
                    } else if gb.last_sync_ms != 0 {
                        gb.last_sync_ms
                    } else {
                        gb.now_ms - 86_400_000
                    };
                    let reach =
                        gb_buckets::start_of(since_ms.div_euclid(60_000) + gb_buckets::REACH)
                            + gb_buckets::MINUTES;
                    let mut answers = Vec::new();
                    for sent in gb.take_sent() {
                        for packet in sent.packets() {
                            answers.extend(watch.receive(packet));
                        }
                    }
                    let cut = choices
                        .chance(24)
                        .then(|| choices.below(answers.len() as u32 * 60 + 1));
                    let mut written = 0;
                    let mut complete = true;
                    for answer in &answers {
                        let line = answer.to_line();
                        // How much of this line arrives: all of it, or up
                        // to where the connection drops.
                        let arrives = match cut {
                            Some(cut) if written + line.len() as u32 > cut => {
                                complete = false;
                                (cut.saturating_sub(written)) as usize
                            }
                            _ if !complete => 0,
                            _ => line.len(),
                        };
                        written += line.len() as u32;
                        gb.receive(&line.as_bytes()[..arrives]);
                        let Outgoing::Activity { timestamp_ms, .. } = *answer else {
                            continue;
                        };
                        let bucket = watch
                            .bucket(timestamp_ms.div_euclid(60_000))
                            .expect("a bucket sent is one in the history");
                        // What went out is what the history holds.
                        assert_eq!(bucket.timestamp_ms(), timestamp_ms);
                        assert_eq!(
                            answer.to_line(),
                            Outgoing::Activity {
                                timestamp_ms,
                                steps: bucket.steps,
                                movement: bucket.movement,
                                heart_rate: bucket.heart_rate,
                                kind: bucket.kind,
                            }
                            .to_line()
                        );
                        // Sent or not, the watch chose it as settled.
                        settled = settled.max(bucket.start + gb_buckets::MINUTES);
                        if arrives == line.len() {
                            delivered.extend(bucket.start..bucket.start + gb_buckets::MINUTES);
                        }
                        // Gadgetbridge has it once its closing brace is
                        // in: the line's end may be the next connection's
                        // first line, which Android's JSON reader never
                        // looks at.
                        if arrives >= line.len() - 2 {
                            meant.insert((timestamp_ms / 1000) as i32, bucket);
                        }
                    }
                    if complete {
                        watch.completed();
                        // Nothing still in the history, in a bucket the
                        // watch has whole and that's over and not held
                        // back, has been left out.
                        let now = watch.now.expect("set on connecting").div_euclid(60);
                        let newest = watch.history.keys().next_back().copied().unwrap_or(now);
                        // TODO: nor, ideally, anything older that the
                        // phone never asked for. Gadgetbridge's first
                        // fetch asks for a day; a watch that was off for
                        // some of it has older minutes, and those are
                        // never sent (report, G6).
                        asked = asked.min(reach);
                        let whole = gb_buckets::start_of(
                            newest - HISTORY_MINUTES + 2 * gb_buckets::MINUTES,
                        )
                        .max(asked);
                        let end = gb_buckets::start_of(now)
                            .min(watch.hold.map_or(i64::MAX, gb_buckets::start_of));
                        for (&minute, _) in watch.history.range(whole..end.max(whole)) {
                            assert!(
                                delivered.contains(&minute),
                                "minute {minute} never sent (now {now})"
                            );
                        }
                    } else {
                        watch.pending_sync = None;
                        gb.disconnect();
                        connect(&mut gb, &mut watch);
                    }
                    // The one complaint allowed: the line the connection
                    // cut, glued to the next connection's first.
                    let events = gb.take_events();
                    for event in &events {
                        assert!(
                            !matches!(event, Event::Uncaught(_) | Event::NotUnderstood(_)),
                            "{event:?}"
                        );
                        assert!(
                            complete || !matches!(event, Event::IgnoredLine(_)),
                            "{event:?}"
                        );
                        assert!(
                            !complete || !matches!(event, Event::MalformedJson(_)),
                            "{event:?}"
                        );
                    }
                }
                // The phone's time zone changes: nothing to a fetch, whose
                // times are all UTC.
                _ => {
                    gb.zone_offset_ms = choices.pick(gbmock::generate::ZONE_QUARTERS) * 900_000;
                    gb.on_set_time();
                    for sent in gb.take_sent() {
                        for packet in sent.packets() {
                            watch.receive(packet);
                        }
                    }
                }
            }

            // After every step: the database is what the watch sent, a
            // sample a bucket, each with its latest values.
            assert_eq!(
                gb.samples.keys().collect::<Vec<_>>(),
                meant.keys().collect::<Vec<_>>(),
                "the samples stored aren't the buckets sent"
            );
            for (timestamp, bucket) in &meant {
                let stored = gb.samples[timestamp];
                let got = (
                    stored.steps,
                    stored.heart_rate,
                    stored.raw_intensity,
                    stored.kind(),
                );
                let wanted = (
                    bucket.steps as i32,
                    i32::from(bucket.heart_rate),
                    i32::from(bucket.movement),
                    bucket.kind.unwrap_or("ACTIVITY"),
                );
                assert_eq!(got, wanted, "the bucket at {timestamp}");
            }
            // And so Gadgetbridge's step count is the watch's own, over
            // what's been sent.
            let stored: i64 = gb.samples.values().map(|s| i64::from(s.steps)).sum();
            let sent: i64 = meant.values().map(|b| i64::from(b.steps)).sum();
            assert_eq!(stored, sent);
        }
    }

    /// Days of a watch recording, revising the night's sleep, and holding
    /// some of it back, while Gadgetbridge fetches when it likes and the
    /// connection drops mid-answer: Gadgetbridge's database always holds
    /// exactly the buckets the watch sent, each as the watch last sent it
    /// (steps, heart rate, movement and kind), and a fetch that completes
    /// leaves out nothing the watch still has whole. With Gadgetbridge's
    /// fixes of November 2026 and without: the watch's rules (a bucket
    /// only ever whole, none whose kind may change) are what make the
    /// older one's upsert come out right too.
    #[test]
    fn activity_reaches_the_database_as_the_watch_sent_it() {
        bolero::check!().for_each(|bytes: &[u8]| {
            let Some((&first, bytes)) = bytes.split_first() else {
                return;
            };
            let fixes = if first % 2 == 0 {
                Fixes::ALL
            } else {
                Fixes::NONE
            };
            story(&mut Choices::new(bytes), fixes);
        });
    }
}
