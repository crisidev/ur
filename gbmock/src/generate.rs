//! Things for Gadgetbridge to do, chosen by a fuzzer's bytes: every call
//! [`Gadgetbridge`] has, with the variety a phone gives them.
//!
//! [`Choices`] reads the bytes: each choice takes as few as it needs, and
//! running out gives zeros, so every input makes something and a small
//! edit to the bytes is a small edit to what's done. No dependency on any
//! one fuzzer: Bolero, libFuzzer and a seeded generator all have bytes.

use crate::device::Gadgetbridge;
use crate::spec::*;

/// A fuzzer's input, a choice at a time.
pub struct Choices<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Choices<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, at: 0 }
    }

    /// Whether the bytes have run out (every choice from here is its
    /// first).
    pub fn is_empty(&self) -> bool {
        self.at >= self.bytes.len()
    }

    pub fn byte(&mut self) -> u8 {
        let byte = self.bytes.get(self.at).copied().unwrap_or(0);
        self.at += 1;
        byte
    }

    /// A number below `n` (which isn't 0): one byte's worth for a small
    /// `n`, four for a large one.
    pub fn below(&mut self, n: u32) -> u32 {
        if n <= 256 {
            u32::from(self.byte()) % n
        } else {
            self.u32() % n
        }
    }

    pub fn u32(&mut self) -> u32 {
        u32::from_le_bytes([self.byte(), self.byte(), self.byte(), self.byte()])
    }

    /// Any `int`, but mostly a small one: the edges matter more than the
    /// middle.
    pub fn int(&mut self) -> i32 {
        match self.byte() {
            0..=199 => i32::from(self.byte()),
            200..=219 => -i32::from(self.byte()),
            220..=229 => i32::MAX,
            230..=239 => i32::MIN,
            _ => self.u32() as i32,
        }
    }

    /// True for `in_256` of a byte's 256 values.
    pub fn chance(&mut self, in_256: u8) -> bool {
        self.byte() < in_256
    }

    pub fn pick<T: Copy>(&mut self, from: &[T]) -> T {
        from[self.below(from.len() as u32) as usize]
    }

    /// Text of up to about `typical` UTF-16 units (now and then four times
    /// that, to pass whatever Gadgetbridge crops it to), of everything a
    /// phone's notifications are made of: words, Latin-1, other scripts,
    /// emoji (those Gadgetbridge turns into ASCII and those it doesn't),
    /// typographic punctuation, and the characters its escaping treats
    /// specially (quotes, backslashes, NUL, DLE, DEL, the soft hyphen).
    pub fn text(&mut self, typical: u32) -> String {
        const WORDS: &[&str] = &[
            "Kari",
            "Ola",
            "Nordmann",
            "Signal",
            "Dinner",
            "on",
            "Sunday",
            "moved",
            "to",
            "3pm",
            "Storgata",
            "OK",
            "#plans",
            "ok?",
            "12:30",
            "-",
            "&",
            "Oslo",
            "Ærlig",
            "Øystein",
            "Åse",
            "blåbærsyltetøy",
            "café",
            "naïve",
            "Привет",
            "мир",
            "Ελλάδα",
            "中文",
            "日本語",
            "한국어",
            "مرحبا",
            "Łódź",
            "žluťoučký",
            "07",
            "5",
            "0",
        ];
        const SPECIALS: &[&str] = &[
            "\u{1F600}",
            "\u{2764}",
            "\u{1F44D}",
            "\u{1F3E1}",
            "\u{1F9D1}\u{200D}\u{1F4BB}",
            "\u{1F1F3}\u{1F1F4}",
            "\u{2019}",
            "\u{201C}",
            "\u{201D}",
            "\u{2013}",
            "\u{2026}",
            "\u{A0}",
            "\"",
            "\\",
            "\0",
            "\u{10}",
            "\u{7F}",
            "\u{AD}",
            "\n",
            "\t",
            "\r",
            "\u{1}",
            "\u{7}",
            "\u{8}",
            "\u{B}",
            "\u{C}",
            "\u{1B}",
            "\u{80}",
            "\u{C1}",
            "\u{C2}",
            "\u{F4}",
            "\u{F5}",
            "\u{FF}",
            "\u{100}",
            "\u{FEFF}",
            "\u{FE0F}",
            "\u{200B}",
            "'",
            ")",
            "}",
            "\u{2190}",
            "\u{3001}",
        ];
        let limit = match self.byte() {
            0..=24 => typical * 4,
            byte => u32::from(byte) % (typical + 1),
        };
        let mut out = String::new();
        let mut units = 0;
        while units < limit {
            let piece = match self.byte() {
                byte @ 0..=159 => WORDS[usize::from(byte) % WORDS.len()],
                byte @ 160..=219 => SPECIALS[usize::from(byte) % SPECIALS.len()],
                _ => " ",
            };
            out.push_str(piece);
            units += piece.encode_utf16().count() as u32;
            if self.chance(160) {
                out.push(' ');
                units += 1;
            }
        }
        out
    }

    /// Text that may be `null`.
    pub fn maybe_text(&mut self, typical: u32) -> Option<String> {
        if self.chance(40) {
            None
        } else {
            Some(self.text(typical))
        }
    }

    fn float(&mut self) -> f32 {
        match self.byte() {
            0..=199 => self.below(100_000) as f32 / 100.0,
            200..=229 => -(self.below(100_000) as f32) / 100.0,
            230..=239 => 0.0,
            240..=244 => f32::NAN,
            245..=249 => f32::INFINITY,
            _ => f32::from_bits(self.u32()),
        }
    }
}

/// One thing Gadgetbridge does (see [`Gadgetbridge::perform`]).
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Connect,
    Disconnect,
    SetTime,
    Notification(NotificationSpec),
    DeleteNotification(i32),
    Call(CallSpec),
    /// What a missed call left to send, a second on.
    RunDelayed,
    MusicInfo(MusicSpec),
    MusicState(MusicStateSpec),
    Volume(f32),
    Alarms(Vec<Alarm>),
    Find(bool),
    Vibrate(i32),
    FetchActivity,
    FullSync,
    FetchTracks(Option<String>),
    /// The live activity screen opening or closing.
    LiveActivity(bool),
    /// The heart rate dialog opening or closing, or the live activity
    /// screen's pulse (`true`).
    RealtimeHeartRate(bool),
    RealtimeSteps(bool),
    HeartRateInterval(i32),
    Gps(Location),
    Navigation(NavigationInfoSpec),
    /// New weather: remembered, and sent.
    Weather(WeatherSpec),
    CalendarEvent(CalendarEventSpec),
    DeleteCalendarEvent(i64),
    DeleteCalendarEvents(Vec<i64>),
    CannedMessages(Vec<String>),
    LoyaltyCards(Vec<LoyaltyCard>),
    AccelSender(bool),
    Screenshot,
    UartTx(String),
    /// A setting changed: which (an index into the settings, in
    /// [`crate::device::Prefs`]'s order), and to what.
    Pref(u8, bool),
    /// Time passes on the phone.
    Wait {
        ms: i64,
    },
    /// The phone's clock is set, to anything.
    Clock {
        ms: i64,
    },
    /// The phone's time zone changes (or its daylight saving does).
    Zone {
        offset_ms: i32,
    },
}

impl Gadgetbridge {
    /// Does `action`. Anything it sends is in [`Gadgetbridge::take_sent`].
    pub fn perform(&mut self, action: &Action) {
        match action {
            Action::Connect => self.connect(),
            Action::Disconnect => self.disconnect(),
            Action::SetTime => self.on_set_time(),
            Action::Notification(spec) => self.on_notification(spec),
            Action::DeleteNotification(id) => self.on_delete_notification(*id),
            Action::Call(spec) => self.on_set_call_state(spec),
            Action::RunDelayed => self.run_delayed(),
            Action::MusicInfo(spec) => self.on_set_music_info(spec),
            Action::MusicState(spec) => self.on_set_music_state(*spec),
            Action::Volume(volume) => self.on_set_phone_volume(*volume),
            Action::Alarms(alarms) => self.on_set_alarms(alarms),
            Action::Find(start) => self.on_find_device(*start),
            Action::Vibrate(n) => self.on_set_constant_vibration(*n),
            Action::FetchActivity => self.on_fetch_activity(),
            Action::FullSync => self.on_full_sync(),
            Action::FetchTracks(id) => self.on_fetch_tracks(id.as_deref()),
            Action::LiveActivity(open) => self.live_activity(*open),
            Action::RealtimeHeartRate(on) => self.on_enable_realtime_heart_rate_measurement(*on),
            Action::RealtimeSteps(on) => self.on_enable_realtime_steps(*on),
            Action::HeartRateInterval(s) => self.on_set_heart_rate_measurement_interval(*s),
            Action::Gps(location) => self.on_set_gps_location(location),
            Action::Navigation(spec) => self.on_set_navigation_info(spec),
            Action::Weather(spec) => {
                self.weather = Some(spec.clone());
                self.on_send_weather();
            }
            Action::CalendarEvent(spec) => self.on_add_calendar_event(spec),
            Action::DeleteCalendarEvent(id) => self.on_delete_calendar_event(*id),
            Action::DeleteCalendarEvents(ids) => self.delete_calendar_events(ids),
            Action::CannedMessages(messages) => self.on_set_canned_messages(messages),
            Action::LoyaltyCards(cards) => self.on_set_loyalty_cards(cards),
            Action::AccelSender(enable) => self.enable_accel_sender(*enable),
            Action::Screenshot => self.on_screenshot_req(),
            Action::UartTx(line) => self.on_uart_tx_intent(line),
            Action::Pref(which, on) => {
                let p = &mut self.prefs;
                let prefs = [
                    &mut p.sync_time,
                    &mut p.allow_high_mtu,
                    &mut p.send_app_notifications,
                    &mut p.text_as_bitmaps,
                    &mut p.missed_call_notification,
                    &mut p.intents,
                    &mut p.internet_build,
                    &mut p.internet_access,
                    &mut p.gps_update,
                    &mut p.use_updated_location,
                    &mut p.sync_calendar,
                    &mut p.wake_on_open,
                ];
                let count = prefs.len();
                for (i, pref) in prefs.into_iter().enumerate() {
                    if i == usize::from(*which) % count {
                        *pref = *on;
                    }
                }
            }
            Action::Wait { ms } => self.now_ms = self.now_ms.saturating_add(*ms),
            Action::Clock { ms } => self.now_ms = *ms,
            Action::Zone { offset_ms } => self.zone_offset_ms = *offset_ms,
        }
    }
}

/// The time zones' offsets from UTC, in quarter hours: every one in use
/// in 2026, and the ends Java allows (±18 hours).
pub const ZONE_QUARTERS: &[i32] = &[
    -72, -48, -44, -40, -38, -36, -32, -28, -24, -20, -16, -14, -12, -10, -8, -4, 0, 4, 8, 12, 14,
    16, 18, 20, 22, 23, 24, 26, 28, 32, 35, 36, 38, 40, 42, 44, 48, 51, 52, 55, 56, 72,
];

/// The next thing for Gadgetbridge to do, by `c`'s bytes.
///
/// Weighted towards what a phone does all day (notifications, music,
/// fetches), with everything else it can do now and then.
pub fn action(c: &mut Choices<'_>) -> Action {
    match c.byte() % 64 {
        0..=9 => Action::Notification(notification(c)),
        10 => Action::DeleteNotification(c.int()),
        11..=14 => Action::Call(CallSpec {
            command: match c.byte() {
                0..=99 => CallSpec::CALL_INCOMING,
                100..=159 => CallSpec::CALL_END,
                160..=239 => i32::from(c.byte() % 7),
                _ => c.int(),
            },
            name: c.maybe_text(30),
            number: if c.chance(200) {
                Some(format!("+47{}", 10_000_000 + c.below(90_000_000)))
            } else {
                c.maybe_text(12)
            },
        }),
        15 => Action::RunDelayed,
        16..=18 => Action::MusicInfo(MusicSpec {
            artist: c.maybe_text(30),
            album: c.maybe_text(30),
            track: c.maybe_text(40),
            duration: c.int(),
            track_count: c.int(),
            track_nr: c.int(),
        }),
        19..=21 => Action::MusicState(MusicStateSpec {
            state: c.pick(&[0, 0, 1, 1, 2, -1, 3, 100, -128]),
            position: c.int(),
            play_rate: c.int(),
            shuffle: c.pick(&[-1, 0, 1]),
            repeat: c.pick(&[-1, 0, 1]),
        }),
        22 => Action::Volume(c.float()),
        23..=25 => {
            let count = c.below(13);
            Action::Alarms(
                (0..count)
                    .map(|_| Alarm {
                        hour: if c.chance(230) {
                            c.below(24) as i32
                        } else {
                            c.int()
                        },
                        minute: if c.chance(230) {
                            c.below(60) as i32
                        } else {
                            c.int()
                        },
                        repetition: if c.chance(230) {
                            c.below(128) as i32
                        } else {
                            c.int()
                        },
                        enabled: c.chance(200),
                        unused: c.chance(30),
                    })
                    .collect(),
            )
        }
        26 => Action::Find(c.chance(150)),
        27 => Action::Vibrate(c.int()),
        28..=32 => Action::FetchActivity,
        33 => Action::FullSync,
        34 => Action::FetchTracks(c.maybe_text(10)),
        35..=36 => Action::LiveActivity(c.chance(128)),
        37 => Action::RealtimeHeartRate(c.chance(128)),
        38 => Action::RealtimeSteps(c.chance(128)),
        39 => Action::HeartRateInterval(c.int()),
        40..=42 => Action::Gps(Location {
            // Mostly on Earth; near zero now and then, where Java prints
            // exponents.
            latitude: coordinate(c, 90.0),
            longitude: coordinate(c, 180.0),
            altitude: f64::from(c.float()),
            speed: c.float(),
            bearing: if c.chance(128) { Some(c.float()) } else { None },
            time: i64::from(c.u32()) * 1000,
            satellites: if c.chance(128) { Some(c.int()) } else { None },
            accuracy: c.float(),
            provider: c
                .pick(&[Some("gps"), Some("network"), Some("fused"), None])
                .map(Into::into),
        }),
        43..=45 => Action::Navigation(NavigationInfoSpec {
            instruction: c.maybe_text(30),
            distance_to_turn: match c.byte() {
                0..=149 => Some(format!("{} m", c.below(2000))),
                150..=199 => Some(format!("{},{}\u{a0}km", c.below(100), c.below(10))),
                200..=229 => None,
                _ => Some(c.text(8)),
            },
            next_action: if c.chance(230) {
                c.below(19) as i32
            } else {
                c.int()
            },
            eta: if c.chance(128) {
                Some(format!("{:02}:{:02}", c.below(24), c.below(60)))
            } else {
                None
            },
        }),
        46 => Action::Weather(WeatherSpec {
            current_temp: 250 + c.below(60) as i32,
            today_max_temp: c.int(),
            today_min_temp: c.int(),
            current_humidity: c.below(101) as i32,
            precip_probability: c.below(101) as i32,
            uv_index: c.float(),
            current_condition_code: c.below(1000) as i32,
            current_condition: c.maybe_text(15),
            wind_speed: c.float(),
            wind_direction: c.below(360) as i32,
            location: c.maybe_text(15),
            ..WeatherSpec::default()
        }),
        47 => Action::CalendarEvent(CalendarEventSpec {
            id: i64::from(c.int()),
            kind: c.int(),
            timestamp: c.int(),
            duration_in_seconds: c.int(),
            title: c.maybe_text(40),
            description: c.maybe_text(100),
            location: c.maybe_text(30),
            cal_name: c.maybe_text(15),
            color: c.int(),
            all_day: c.chance(60),
        }),
        48 => Action::DeleteCalendarEvent(i64::from(c.int())),
        49 => {
            let count = c.below(4);
            Action::DeleteCalendarEvents((0..count).map(|_| i64::from(c.int())).collect())
        }
        50 => {
            let count = c.below(5);
            Action::CannedMessages((0..count).map(|_| c.text(30)).collect())
        }
        51 => {
            let count = c.below(4);
            Action::LoyaltyCards(
                (0..count)
                    .map(|_| LoyaltyCard {
                        id: c.int(),
                        name: c.maybe_text(30),
                        value: c.text(20),
                        format: c
                            .pick(&[Some("QR_CODE"), Some("CODE_39"), None])
                            .map(Into::into),
                        expiry: if c.chance(100) {
                            Some(i64::from(c.u32()) * 1000)
                        } else {
                            None
                        },
                        color: c.int(),
                        balance: c.maybe_text(10),
                        note: c.maybe_text(60),
                    })
                    .collect(),
            )
        }
        52 => Action::AccelSender(c.chance(128)),
        53 => Action::Screenshot,
        54 => Action::UartTx(c.text(30)),
        55..=56 => Action::Pref(c.byte(), c.chance(128)),
        57..=58 => Action::Wait {
            // Up to a minute, an hour, or three days.
            ms: match c.byte() {
                0..=127 => i64::from(c.below(60_000)),
                128..=229 => i64::from(c.below(3_600_000)),
                _ => i64::from(c.below(3 * 86_400_000)),
            },
        },
        59 => Action::Zone {
            offset_ms: if c.chance(230) {
                c.pick(ZONE_QUARTERS) * 15 * 60_000
            } else {
                c.below(2 * 18 * 3_600_000 + 1) as i32 - 18 * 3_600_000
            },
        },
        60 => Action::Clock {
            ms: match c.byte() {
                // Around now; then anywhere a 32-bit count of seconds
                // reaches; then anywhere at all.
                0..=179 => 1_794_312_000_000 + i64::from(c.int()) * 1000,
                180..=229 => i64::from(c.u32()) * 1000,
                _ => (i64::from(c.u32()) << 32 | i64::from(c.u32())) >> c.below(20),
            },
        },
        61 => Action::SetTime,
        62 => Action::Connect,
        _ => Action::Disconnect,
    }
}

fn coordinate(c: &mut Choices<'_>, most: f64) -> f64 {
    match c.byte() {
        0..=199 => (f64::from(c.u32()) / f64::from(u32::MAX) * 2.0 - 1.0) * most,
        200..=229 => (f64::from(c.below(2001)) - 1000.0) / 1_000_000.0,
        230..=239 => 0.0,
        240..=244 => f64::NAN,
        _ => f64::from(c.float()),
    }
}

/// A notification as a phone's apps make them.
pub fn notification(c: &mut Choices<'_>) -> NotificationSpec {
    let mut spec = NotificationSpec {
        id: c.int(),
        source_name: c.maybe_text(12),
        sms: c.chance(30),
        title: c.maybe_text(30),
        subject: c.maybe_text(20),
        body: c.maybe_text(120),
        sender: c.maybe_text(20),
        phone_number: c.maybe_text(12),
        actions: Vec::new(),
    };
    for _ in 0..c.below(4) {
        spec.actions.push(NotificationAction {
            kind: c.pick(&[
                ActionKind::WearableReply,
                ActionKind::Simple,
                ActionKind::Other,
            ]),
            title: c.maybe_text(15),
            handle: i64::from(c.u32()),
        });
    }
    spec
}

/// What Gadgetbridge does when the watch connects, after
/// [`Gadgetbridge::connect`] (the time, and the GPS question): what the
/// rest of the app has waiting for a device that's just come up, in an
/// order the bytes choose.
///
/// TODO: Gadgetbridge's own order isn't modelled. On a device becoming
/// initialized, `DeviceCommunicationService` (`:277-286`) sends the
/// notifications it cached while disconnected, a pending Sleep as Android
/// action, and the loyalty cards if they sync automatically; its other
/// receivers (music, weather, calendar, the auto-fetch on unlocking the
/// phone) each send when their own trigger next fires. So any of these,
/// in any order, is what a connection's first seconds look like.
pub fn connect_sequence(c: &mut Choices<'_>) -> Vec<Action> {
    let mut actions = vec![Action::Connect];
    for _ in 0..c.below(6) {
        actions.push(match c.byte() % 8 {
            0 | 1 => Action::Notification(notification(c)),
            2 => Action::FetchActivity,
            3 => Action::MusicInfo(MusicSpec {
                artist: c.maybe_text(20),
                album: c.maybe_text(20),
                track: c.maybe_text(20),
                duration: c.int(),
                track_count: 1,
                track_nr: 1,
            }),
            4 => Action::MusicState(MusicStateSpec {
                state: c.pick(&[0, 1, 2]),
                ..MusicStateSpec::default()
            }),
            5 => Action::LoyaltyCards(Vec::new()),
            6 => Action::SetTime,
            _ => Action::AccelSender(c.chance(128)),
        });
    }
    actions
}
