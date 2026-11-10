//! `BangleJSDeviceSupport`, as a state machine: [`Gadgetbridge`].
//!
//! Its `on…` methods are the Java class's, and send what they send
//! ([`Gadgetbridge::take_sent`]); [`Gadgetbridge::receive`] is its
//! `onCharacteristicChanged`, and does what it does with the watch's
//! bytes: [`Gadgetbridge::samples`] is the activity table of its database,
//! [`Gadgetbridge::events`] everything else that leaves the class (toasts,
//! broadcasts, device events), and the public fields the state it keeps.
//!
//! Line references are to
//! `service/devices/banglejs/BangleJSDeviceSupport.java` at the pinned
//! commit unless they name another file.

use std::collections::BTreeMap;

use crate::java;
use crate::json::{Json, JsonException, Object};
use crate::kinds;
use crate::spec::*;
use crate::text::{crop_to_length, render_unicode_as_image};

/// `ActivitySample.NOT_MEASURED` (`model/ActivitySample.java:39`).
pub const NOT_MEASURED: i32 = -1;

/// The settings the Bangle.js support reads, with Gadgetbridge's defaults.
#[derive(Clone, Debug, PartialEq)]
pub struct Prefs {
    /// `GBPrefs.syncTime()`: set the watch's clock on connecting (`:369-371`).
    pub sync_time: bool,
    /// `PREF_ALLOW_HIGH_MTU` (`:362`).
    pub allow_high_mtu: bool,
    /// `PREF_SEND_APP_NOTIFICATIONS` (`:1469`).
    pub send_app_notifications: bool,
    /// `PREF_BANGLEJS_TEXT_BITMAP`, "Text as Bitmaps" (`:1389`).
    pub text_as_bitmaps: bool,
    /// `PREF_BANGLEJS_NOTIFICATION_MISSED_CALL_ENABLE` (`:1603`).
    pub missed_call_notification: bool,
    /// `PREF_DEVICE_INTENTS`, "Allow Intents" (`:1063`).
    pub intents: bool,
    /// `GBApplication.hasInternetAccess()` (`:920`).
    pub internet_build: bool,
    /// `PREF_DEVICE_INTERNET_ACCESS` (`:926`).
    pub internet_access: bool,
    /// `PREF_DEVICE_GPS_UPDATE` (`:1274`).
    pub gps_update: bool,
    /// `use_updated_location_if_available` (`:1300`).
    pub use_updated_location: bool,
    /// `sync_calendar` (`:1004`, `:1907`).
    pub sync_calendar: bool,
    /// `PREF_NOTIFICATION_WAKE_ON_OPEN` (`:789`).
    pub wake_on_open: bool,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            sync_time: true,
            allow_high_mtu: true,
            send_app_notifications: true,
            text_as_bitmaps: false,
            missed_call_notification: false,
            intents: false,
            internet_build: false,
            internet_access: false,
            gps_update: false,
            use_updated_location: false,
            sync_calendar: false,
            wake_on_open: false,
        }
    }
}

/// The three fixes Gadgetbridge made on 2026-11-09 and -10, each of which
/// changes what a fetch leaves in its database. On by default (the pinned
/// commit); off, the mock is the Gadgetbridge that phones had until then
/// (its releases up to 2026.11.1 at least).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fixes {
    /// 7712415234 (#7289): the `actfetch` end broadcasts that new data is
    /// in (`:847`), which Health Connect's "Sync after device sync" waits
    /// for. Before it only an (empty) track list did, with a toast.
    pub new_data_on_fetch_end: bool,
    /// 289bb4a382 (#7297): a sample without `mov` no longer overwrites the
    /// stored intensity with -1 (`BangleJSSampleProvider.java:122`).
    pub keeps_intensity_without_mov: bool,
    /// deffcca650 (#7296): an upsert updates the stored sample's kind
    /// (`BangleJSSampleProvider.java:125-127`). Before it, a sample kept
    /// the kind it first arrived with.
    pub upsert_updates_kind: bool,
}

impl Fixes {
    /// The pinned commit.
    pub const ALL: Self = Self {
        new_data_on_fetch_end: true,
        keeps_intensity_without_mov: true,
        upsert_updates_kind: true,
    };
    /// Before 7712415234.
    pub const NONE: Self = Self {
        new_data_on_fetch_end: false,
        keeps_intensity_without_mov: false,
        upsert_updates_kind: false,
    };
}

/// A `BangleJSActivitySample`: a row of the activity table (its key, the
/// timestamp, included).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sample {
    /// UTC seconds, as a Java `int`.
    pub timestamp: i32,
    /// An `ActivityKind` code (see [`crate::kinds`]).
    pub raw_kind: i32,
    pub heart_rate: i32,
    pub steps: i32,
    /// [`NOT_MEASURED`] without one.
    pub raw_intensity: i32,
}

impl Sample {
    /// `BangleJSSampleProvider.normalizeIntensity`
    /// (`devices/banglejs/BangleJSSampleProvider.java:78-80`): what the
    /// charts draw, meant to be 0 to 1.
    pub fn intensity(&self) -> f32 {
        self.raw_intensity as f32 / 2048.0
    }

    /// The `ActivityKind` constant's name, as `normalizeType`
    /// (`BangleJSSampleProvider.java:68-70`) finds it by code.
    pub fn kind(&self) -> &'static str {
        kinds::KINDS
            .iter()
            .find(|(_, code)| *code == self.raw_kind)
            .map_or("UNKNOWN", |(name, _)| name)
    }
}

/// Which of the stored samples within two minutes of a new one at
/// `timestamp` its values go into: an index into `near` (their timestamps,
/// ascending, as the query orders them), which isn't empty.
///
/// `BangleJSSampleProvider.upsertSample`'s loop
/// (`devices/banglejs/BangleJSSampleProvider.java:104-112`), which is
/// **not** "the nearest": it compares each candidate's distance from the
/// new sample with its distance from the *current choice*, not with the
/// current choice's distance from the new sample. With stored samples at
/// `t-10` and `t+100`, it picks `t+100` (100 < 110): a new sample between
/// two stored ones always goes into the later.
pub fn upsert_target(near: &[i32], timestamp: i32) -> usize {
    let mut nearest = 0;
    for (i, &s) in near.iter().enumerate() {
        let cur_dist = near[nearest].wrapping_sub(s).wrapping_abs();
        let new_dist = timestamp.wrapping_sub(s).wrapping_abs();
        if new_dist < cur_dist {
            nearest = i;
        }
    }
    nearest
}

/// Merges a new sample into the stored one [`upsert_target`] chose
/// (`BangleJSSampleProvider.java:116-127`): each value only if the new
/// sample has one. The stored sample keeps its own timestamp.
pub fn upsert_merge(stored: &mut Sample, new: &Sample, fixes: Fixes) {
    if new.heart_rate != 0 {
        stored.heart_rate = new.heart_rate;
    }
    if new.steps != 0 {
        stored.steps = new.steps;
    }
    if new.raw_intensity != 0
        && (new.raw_intensity != NOT_MEASURED || !fixes.keeps_intensity_without_mov)
    {
        stored.raw_intensity = new.raw_intensity;
    }
    if fixes.upsert_updates_kind && new.raw_kind != kinds::UNKNOWN {
        stored.raw_kind = new.raw_kind;
    }
}

/// `GB.INFO`, `GB.WARN`, `GB.ERROR`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Info,
    Warn,
    Error,
}

/// `BatteryState`, those the Bangle.js support uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BatteryState {
    Unknown,
    Normal,
    Charging,
}

/// What the Bangle.js support does that leaves the class: what a user or
/// another part of Gadgetbridge would notice.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// `GB.toast`: text on the phone's screen.
    Toast { level: Level, text: String },
    /// A line that began with `{` and didn't parse, or lacked what its
    /// handler `get`s (`:537-540`). Shown as an error toast,
    /// "Malformed JSON from Bangle.js: …".
    MalformedJson(String),
    /// An exception nothing in the class catches (an
    /// `IllegalArgumentException` from an `Enum.valueOf`, say): it leaves
    /// `onCharacteristicChanged`, and the rest of that packet's lines wait
    /// for the next. The watch should never cause one.
    Uncaught(String),
    /// A line that isn't JSON nor a screenshot (`:546-548`), logged only.
    IgnoredLine(String),
    /// JSON without a `t` (`:535-536`), or with one the class doesn't know
    /// (`:635-637`). Logged only.
    NotUnderstood(String),
    /// `GBDeviceEventVersionInfo` (`:563-570`): shown in the device's
    /// details.
    Version {
        fw: Option<String>,
        hw: Option<String>,
    },
    /// `GBDeviceEventFindPhone` (`:571-576`): the phone rings, or stops.
    FindPhone(bool),
    /// `GBDeviceEventMusicControl` (`:577-581`), by the event's name.
    MusicControl(&'static str),
    /// `GBDeviceEventCallControl` (`:582-586`), by the event's name.
    CallControl(&'static str),
    /// `GBDeviceEventBatteryInfo` (`:751-775`): the level after smoothing.
    Battery {
        level: Option<i32>,
        state: BatteryState,
        voltage: Option<f32>,
    },
    /// `GBDeviceEventNotificationControl` (`:780-836`).
    NotificationControl {
        event: &'static str,
        handle: i64,
        phone_number: Option<String>,
        reply: Option<String>,
    },
    /// `WakeActivity.start` (`:789-791`).
    WakePhone,
    /// `DeviceService.ACTION_REALTIME_SAMPLES` (`:901-906`): a live sample,
    /// for the live activity chart and the heart rate dialog. Not stored.
    RealtimeSample(Sample),
    /// `SleepAsAndroidSender.onHrChanged` (`:879-881`).
    SleepAsAndroidHeartRate(i32),
    /// `GB.signalActivityDataFinish`: Android's `ACTION_NEW_DATA`, which
    /// charts refresh on, and Health Connect's "Sync after device sync"
    /// exports on.
    NewData,
    /// `GBDevice.unsetBusyTask` with no busy task: an error in the log.
    UnsetBusyWhenIdle,
    /// An Android intent (`:1061-1133`).
    Intent {
        target: String,
        action: Option<String>,
        package: Option<String>,
        class: Option<String>,
        /// The extras, as the JSON values `BundleUtils.addToBundle` types
        /// them by (`util/BundleUtils.java:47-…`).
        extras: Vec<(String, Json)>,
    },
    /// Location updates from the phone asked for, or no longer
    /// (`:1268-1296`, `:233-239`).
    LocationUpdates(bool),
    /// A screenshot (`:541-545`).
    Screenshot(Vec<u8>),
    /// A file written to the phone's storage (`:1148-1177`).
    File {
        name: String,
        mode: &'static str,
        contents: String,
    },
    /// An HTTP request made (`:958`).
    Http { url: String, method: String },
    /// `CalendarReceiver.forceSync` (`:1055`).
    CalendarSync,
    /// `SleepAsAndroidSender.onAccelChanged` (`:740-746`).
    Acceleration,
}

/// `GBDevice`'s busy task: what the device's card says it's doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BusyTask {
    /// `R.string.busy_task_fetch_activity_data`.
    FetchingActivity,
    /// `R.string.busy_task_fetch_sports_details`.
    FetchingSports,
}

/// Something `uartTx` wrote (`:390-402`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sent {
    /// The whole string, in ISO-8859-1: for a message, `\x10`, the line,
    /// and `\n`.
    pub bytes: Vec<u8>,
    /// The size it was cut to for the writes.
    pub mtu: usize,
}

impl Sent {
    /// The writes to the characteristic, one after the other.
    pub fn packets(&self) -> impl Iterator<Item = &[u8]> {
        self.bytes.chunks(self.mtu)
    }

    /// The message's JSON, if it's a `GB(…)` line.
    pub fn json(&self) -> Option<&[u8]> {
        self.bytes.strip_prefix(b"\x10GB(")?.strip_suffix(b")\n")
    }
}

/// Gadgetbridge with one Bangle.js: `BangleJSDeviceSupport` and the parts
/// of the app it leans on (see the module docs).
#[derive(Clone, Debug)]
pub struct Gadgetbridge {
    pub prefs: Prefs,
    pub fixes: Fixes,
    /// `System.currentTimeMillis()`. The phone's clock doesn't move by
    /// itself here.
    pub now_ms: i64,
    /// `TimeZone.getDefault().getOffset(now)`, in milliseconds.
    pub zone_offset_ms: i32,
    /// `Weather.getWeatherSpec()`.
    pub weather: Option<WeatherSpec>,

    // The class's fields (`:159-182`).
    pub connected: bool,
    pub mtu_size: usize,
    received_line: String,
    pub realtime_hrm: bool,
    pub realtime_step: bool,
    pub realtime_hrm_interval: i32,
    last_battery_percent: i32,
    is_missed_call: bool,
    pub gps_update_setup: bool,
    /// `MediaManager`'s `bufferMusicSpec` and `bufferMusicStateSpec`.
    music: Option<MusicSpec>,
    music_state: Option<MusicStateSpec>,
    /// `mNotificationReplyAction` and `mNotificationActions`: the last 16.
    reply_actions: Vec<(i32, i64)>,
    notification_actions: Vec<(i32, Vec<i64>)>,
    /// A missed call's notification, due a second after the call ended
    /// (`:1630-1632`): [`Gadgetbridge::run_delayed`] sends it.
    delayed_notification: Option<NotificationSpec>,
    /// `BangleJSActivityTrack`'s statics: the logs still to fetch, and the
    /// last packet's count.
    tracks_list: Vec<Json>,
    last_packet_count: i32,

    // What it keeps elsewhere.
    /// The device's busy task.
    pub busy: Option<BusyTask>,
    /// `lastSyncTimeMillis` in the device's preferences; 0 if never set.
    pub last_sync_ms: i64,
    /// `PREF_BANGLEJS_ACTIVITY_FULL_SYNC_STATUS` (`:852-854`).
    pub full_sync_status: Option<String>,
    /// The device's firmware and hardware versions, as its details show.
    pub firmware: Option<String>,
    pub hardware: Option<String>,
    /// The activity table: `BangleJSActivitySample`s by timestamp.
    pub samples: BTreeMap<i32, Sample>,
    pub events: Vec<Event>,
    sent: Vec<Sent>,
}

impl Default for Gadgetbridge {
    fn default() -> Self {
        Self::new()
    }
}

/// What a line's handler ends in, other than well.
enum Thrown {
    /// A `JSONException`: caught, and shown as malformed JSON (`:537-540`).
    Json(JsonException),
    /// Anything else: not caught.
    Other(String),
}

impl From<JsonException> for Thrown {
    fn from(e: JsonException) -> Self {
        Self::Json(e)
    }
}

const MUSIC_EVENTS: &[&str] = &[
    "UNKNOWN",
    "PLAY",
    "PAUSE",
    "PLAYPAUSE",
    "NEXT",
    "PREVIOUS",
    "VOLUMEUP",
    "VOLUMEDOWN",
    "FORWARD",
    "REWIND",
];
const CALL_EVENTS: &[&str] = &[
    "UNKNOWN", "ACCEPT", "END", "INCOMING", "OUTGOING", "REJECT", "START", "IGNORE",
];
const NOTIFICATION_EVENTS: &[&str] =
    &["UNKNOWN", "DISMISS", "DISMISS_ALL", "OPEN", "MUTE", "REPLY"];

/// `Enum.valueOf(name)`: the constant, or an `IllegalArgumentException`.
/// (`deviceevents/GBDeviceEventMusicControl.java:53-64`,
/// `GBDeviceEventCallControl.java:65-74`,
/// `GBDeviceEventNotificationControl.java:101-108`.)
fn value_of(
    names: &'static [&'static str],
    class: &str,
    name: &str,
) -> Result<&'static str, Thrown> {
    names.iter().find(|n| **n == name).copied().ok_or_else(|| {
        Thrown::Other(format!(
            "IllegalArgumentException: No enum constant {class}.{name}"
        ))
    })
}

impl Gadgetbridge {
    /// A Gadgetbridge that has never seen the watch, not connected, at
    /// 2026-11-10 12:00 UTC in UTC+1.
    pub fn new() -> Self {
        Self {
            prefs: Prefs::default(),
            fixes: Fixes::ALL,
            now_ms: 1_794_312_000_000,
            zone_offset_ms: 3_600_000,
            weather: None,
            connected: false,
            mtu_size: 20,
            received_line: String::new(),
            realtime_hrm: false,
            realtime_step: false,
            realtime_hrm_interval: 10,
            last_battery_percent: -1,
            is_missed_call: false,
            gps_update_setup: false,
            music: None,
            music_state: None,
            reply_actions: Vec::new(),
            notification_actions: Vec::new(),
            delayed_notification: None,
            tracks_list: Vec::new(),
            last_packet_count: -1,
            busy: None,
            last_sync_ms: 0,
            full_sync_status: None,
            firmware: None,
            hardware: None,
            samples: BTreeMap::new(),
            events: Vec::new(),
            sent: Vec::new(),
        }
    }

    /// What's been sent since the last call, in order.
    pub fn take_sent(&mut self) -> Vec<Sent> {
        std::mem::take(&mut self.sent)
    }

    /// What's happened since the last call (see [`Event`]), in order.
    pub fn take_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }

    fn toast(&mut self, level: Level, text: impl Into<String>) {
        self.events.push(Event::Toast {
            level,
            text: text.into(),
        });
    }

    // ==========================================================================
    // Sending
    // ==========================================================================

    /// `uartTx` (`:390-402`): the string's ISO-8859-1 bytes, cut into
    /// writes of the MTU's size (20 until the watch sends more at once).
    ///
    /// Nothing goes out while disconnected. (Gadgetbridge's
    /// `performInitialized` would try to connect first; the rest of the
    /// app doesn't call a device that isn't initialized.)
    fn uart_tx(&mut self, bytes: Vec<u8>) {
        if self.connected {
            self.sent.push(Sent {
                bytes,
                mtu: self.mtu_size,
            });
        }
    }

    /// `uartTxJSON` (`:504-506`).
    fn uart_tx_json(&mut self, object: &Object) {
        let mut line = b"\x10GB(".to_vec();
        line.extend(object.to_wire());
        line.extend_from_slice(b")\n");
        self.uart_tx(line);
    }

    /// `uartTxJSONError` (`:508-519`).
    fn uart_tx_json_error(&mut self, task: &str, message: &str, id: Option<&str>) {
        let mut o = Object::new();
        o.put("t", task).put_str("id", id).put("err", message);
        self.uart_tx_json(&o);
    }

    fn render(&self, text: Option<&str>) -> Option<Json> {
        render_unicode_as_image(text, self.prefs.text_as_bitmaps).map(Json::from)
    }

    /// The connection comes up: `initializeDevice` (`:340-387`). Sets the
    /// time (if that's on) and asks whether the watch wants GPS. The MTU,
    /// the live activity flags, and **any partial line from before** all
    /// carry over; only the battery's smoothing starts afresh.
    pub fn connect(&mut self) {
        self.connected = true;
        if self.prefs.sync_time {
            self.transmit_time();
        }
        self.last_battery_percent = -1;
        // `requestBangleGPSPowerStatus` (`:1257-1266`).
        let mut o = Object::new();
        o.put("t", "is_gps_active");
        self.uart_tx_json(&o);
    }

    /// The connection is lost: the registered receiver's
    /// `ACTION_DEVICE_CHANGED` (`:271-281`) stops location updates.
    pub fn disconnect(&mut self) {
        self.connected = false;
        self.stop_location_update();
    }

    /// `transmitTime` (`:1245-1255`): JavaScript, not a `GB(…)` message.
    fn transmit_time(&mut self) {
        let tz = java::float_to_string(self.zone_offset_ms as f32 / (1000.0 * 60.0 * 60.0));
        let line = format!(
            "\x10setTime({});E.setTimeZone({tz});(s=>s&&(s.timezone={tz},require('Storage').write('setting.json',s)))(require('Storage').readJSON('setting.json',1))\n",
            self.now_ms / 1000,
        );
        self.uart_tx(line.into_bytes());
    }

    /// `onSetTime` (`:1538-1549`): the time, and a request for the
    /// calendar events the watch has. (The request is queued first: it's
    /// its own transaction, and the time's is queued after it.)
    pub fn on_set_time(&mut self) {
        // `forceCalendarSync` (`:2382-2390`).
        let mut o = Object::new();
        o.put("t", "force_calendar_sync_start");
        self.uart_tx_json(&o);
        self.transmit_time();
    }

    /// `onNotification` (`:1468-1522`).
    pub fn on_notification(&mut self, spec: &NotificationSpec) {
        if !self.prefs.send_app_notifications {
            return;
        }
        let mut handles = Vec::new();
        let mut actions = Vec::new();
        let mut can_reply = false;
        for action in &spec.actions {
            match action.kind {
                ActionKind::WearableReply => {
                    limited_add(&mut self.reply_actions, spec.id, action.handle);
                    can_reply = true;
                }
                ActionKind::Simple => {
                    let mut a = Object::new();
                    let title = crop_to_length(action.title.as_deref(), 40);
                    a.put("title", self.render(Some(&title)));
                    actions.push(Json::Object(a));
                    handles.push(action.handle);
                }
                ActionKind::Other => {}
            }
        }
        let src = if spec.sms {
            Some("SMS Message")
        } else {
            spec.source_name.as_deref()
        };
        let cropped = |text: &Option<String>, len| crop_to_length(text.as_deref(), len);
        let mut o = Object::new();
        o.put("t", "notify")
            .put("id", spec.id)
            .put_str("src", src)
            .put("title", self.render(Some(&cropped(&spec.title, 80))))
            .put("subject", self.render(Some(&cropped(&spec.subject, 80))))
            .put("body", self.render(Some(&cropped(&spec.body, 400))))
            .put("sender", self.render(Some(&cropped(&spec.sender, 40))))
            .put_str("tel", spec.phone_number.as_deref());
        if can_reply {
            o.put("reply", true);
        }
        if !handles.is_empty() {
            self.notification_actions.retain(|(id, _)| *id != spec.id);
            limited_add(&mut self.notification_actions, spec.id, handles);
            o.put("actions", actions);
        }
        self.uart_tx_json(&o);
    }

    /// `onDeleteNotification` (`:1525-1535`).
    pub fn on_delete_notification(&mut self, id: i32) {
        self.notification_actions.retain(|(i, _)| *i != id);
        let mut o = Object::new();
        o.put("t", "notify-").put("id", id);
        self.uart_tx_json(&o);
    }

    /// `onSetCannedMessages` (`:1552-1574`).
    pub fn on_set_canned_messages(&mut self, messages: &[String]) {
        let items = messages
            .iter()
            .map(|message| {
                let mut m = Object::new();
                let rendered = render_unicode_as_image(Some(message), self.prefs.text_as_bitmaps)
                    .expect("a message isn't null");
                if rendered != *message {
                    m.put("disp", rendered);
                }
                m.put("text", message.as_str());
                Json::Object(m)
            })
            .collect::<Vec<_>>();
        let mut o = Object::new();
        o.put("t", "canned_responses_sync").put("d", items);
        self.uart_tx_json(&o);
    }

    /// `onSetAlarms` (`:1577-1598`): every alarm that's been set, whenever
    /// one changes.
    pub fn on_set_alarms(&mut self, alarms: &[Alarm]) {
        let items = alarms
            .iter()
            .filter(|alarm| !alarm.unused)
            .map(|alarm| {
                let mut a = Object::new();
                a.put("h", alarm.hour)
                    .put("m", alarm.minute)
                    .put("rep", alarm.repetition)
                    .put("on", alarm.enabled);
                Json::Object(a)
            })
            .collect::<Vec<_>>();
        let mut o = Object::new();
        o.put("t", "alarm").put("d", items);
        self.uart_tx_json(&o);
    }

    /// `onSetCallState` (`:1601-1659`). A call that ends without having
    /// been answered leaves a notification to send a second later, if
    /// that's on (see [`Gadgetbridge::run_delayed`]).
    pub fn on_set_call_state(&mut self, spec: &CallSpec) {
        match spec.command {
            CallSpec::CALL_INCOMING => self.is_missed_call = true,
            CallSpec::CALL_END => {
                if self.is_missed_call {
                    self.is_missed_call = false;
                    if self.prefs.missed_call_notification {
                        // The app's strings (`values/strings.xml`).
                        let mut n = NotificationSpec {
                            source_name: Some("Phone".into()),
                            title: Some("Missed call".into()),
                            subject: Some("Missed call".into()),
                            ..NotificationSpec::default()
                        };
                        match (&spec.name, &spec.number) {
                            (None, None) => n.body = Some("Caller ID suppressed".into()),
                            (None, Some(number)) => {
                                n.sender = Some(number.clone());
                                n.body = Some(number.clone());
                                n.phone_number = Some(number.clone());
                            }
                            (Some(name), number) => {
                                n.sender = Some(name.clone());
                                // Java's concatenation spells a null out.
                                n.body = Some(format!(
                                    "{name}\n{}",
                                    number.as_deref().unwrap_or("null")
                                ));
                                n.phone_number = number.clone();
                            }
                        }
                        self.delayed_notification = Some(n);
                    }
                }
            }
            _ => self.is_missed_call = false,
        }
        let mut o = Object::new();
        o.put("t", "call")
            .put("cmd", CallSpec::command_name(spec.command))
            .put("name", self.render(spec.name.as_deref()))
            .put_str("number", spec.number.as_deref());
        self.uart_tx_json(&o);
    }

    /// What `handler.postDelayed` has waiting (`:1630-1632`): a missed
    /// call's notification.
    pub fn run_delayed(&mut self) {
        if let Some(spec) = self.delayed_notification.take() {
            self.on_notification(&spec);
        }
    }

    /// `onSetMusicState` (`:1662-1680`). The same state twice is sent once
    /// (`util/MediaManager.java:65-72`).
    pub fn on_set_music_state(&mut self, spec: MusicStateSpec) {
        if self.music_state == Some(spec) {
            return;
        }
        self.music_state = Some(spec);
        let state = match spec.state {
            0 => "play",
            1 => "pause",
            2 => "stop",
            _ => "",
        };
        let mut o = Object::new();
        o.put("t", "musicstate")
            .put("state", state)
            .put("position", spec.position)
            .put("shuffle", i32::from(spec.shuffle))
            .put("repeat", i32::from(spec.repeat));
        self.uart_tx_json(&o);
    }

    /// `onSetMusicInfo` (`:1683-1699`). The same track twice is sent once;
    /// a new one resets the remembered state to playing from the start
    /// (`util/MediaManager.java:77-87`), so "playing, at 0" right after a
    /// new track isn't sent.
    pub fn on_set_music_info(&mut self, spec: &MusicSpec) {
        if self.music.as_ref() == Some(spec) {
            return;
        }
        self.music = Some(spec.clone());
        if let Some(state) = &mut self.music_state {
            state.state = 0;
            state.position = 0;
        }
        let mut o = Object::new();
        o.put("t", "musicinfo")
            .put("artist", self.render(spec.artist.as_deref()))
            .put("album", self.render(spec.album.as_deref()))
            .put("track", self.render(spec.track.as_deref()))
            .put("dur", spec.duration)
            .put("c", spec.track_count)
            .put("n", spec.track_nr);
        self.uart_tx_json(&o);
    }

    /// `onSetPhoneVolume` (`:1702-1712`).
    pub fn on_set_phone_volume(&mut self, volume: f32) {
        let mut o = Object::new();
        o.put("t", "audio").put("v", f64::from(volume));
        self.uart_tx_json(&o);
    }

    /// `transmitActivityStatus` (`:1714-1725`).
    fn transmit_activity_status(&mut self) {
        let mut o = Object::new();
        o.put("t", "act")
            .put("hrm", self.realtime_hrm)
            .put("stp", self.realtime_step)
            .put("int", self.realtime_hrm_interval);
        self.uart_tx_json(&o);
    }

    /// `onEnableRealtimeSteps` (`:1728-1732`). **It compares with the heart
    /// rate's flag, not its own**: enabling steps while the heart rate is
    /// already live does nothing, and so does disabling them once the
    /// heart rate is off.
    pub fn on_enable_realtime_steps(&mut self, enable: bool) {
        if enable == self.realtime_hrm {
            return;
        }
        self.realtime_step = enable;
        self.transmit_activity_status();
    }

    /// `onEnableRealtimeHeartRateMeasurement` (`:1798-1802`). The live
    /// activity screen calls this with `true` every pulse
    /// (`activities/charts/LiveActivityFragment.java:369-370`); only a
    /// change is sent, **so not after a reconnect**, where the watch has
    /// stopped and Gadgetbridge still has it as live.
    pub fn on_enable_realtime_heart_rate_measurement(&mut self, enable: bool) {
        if enable == self.realtime_hrm {
            return;
        }
        self.realtime_hrm = enable;
        self.transmit_activity_status();
    }

    /// `onSetHeartRateMeasurementInterval` (`:1840-1843`).
    pub fn on_set_heart_rate_measurement_interval(&mut self, seconds: i32) {
        self.realtime_hrm_interval = seconds;
        self.transmit_activity_status();
    }

    /// The live activity screen opening or closing: steps, then the heart
    /// rate (`activities/charts/LiveActivityFragment.java:373-381`).
    pub fn live_activity(&mut self, open: bool) {
        self.on_enable_realtime_steps(open);
        self.on_enable_realtime_heart_rate_measurement(open);
    }

    /// `onFetchRecordedData(TYPE_ACTIVITY)` (`:1735-1739`): fetch what's
    /// new since the last fetch ended; the first time, the last day.
    pub fn on_fetch_activity(&mut self) {
        // `getLastSuccessfulSyncTime` (`:1787-1795`).
        let since = if self.last_sync_ms != 0 {
            self.last_sync_ms
        } else {
            self.now_ms - 24 * 60 * 60 * 1000
        };
        self.fetch_activity_data(since);
    }

    /// `onSendConfiguration(PREF_BANGLEJS_ACTIVITY_FULL_SYNC_START)`
    /// (`:1180-1188`): the "full sync" button, a fetch from 0.
    pub fn on_full_sync(&mut self) {
        self.fetch_activity_data(0);
    }

    /// `fetchActivityData` (`:1766-1775`).
    fn fetch_activity_data(&mut self, since_ms: i64) {
        let mut o = Object::new();
        o.put("t", "actfetch").put("ts", since_ms);
        self.uart_tx_json(&o);
    }

    /// `onFetchRecordedData(TYPE_GPS_TRACKS)` (`:1740-1742`,
    /// `BangleJSActivityTrack.java:50-67`): ask for the recorder's logs
    /// since the last one fetched.
    pub fn on_fetch_tracks(&mut self, last_synced_id: Option<&str>) {
        self.busy = Some(BusyTask::FetchingSports);
        self.toast(Level::Info, "Start : Fetching sports details");
        let mut o = Object::new();
        o.put("t", "listRecs").put_str("id", last_synced_id);
        self.uart_tx_json(&o);
    }

    /// `onFindDevice` (`:1805-1814`).
    pub fn on_find_device(&mut self, start: bool) {
        let mut o = Object::new();
        o.put("t", "find").put("n", start);
        self.uart_tx_json(&o);
    }

    /// `onSetConstantVibration` (`:1817-1826`).
    pub fn on_set_constant_vibration(&mut self, intensity: i32) {
        let mut o = Object::new();
        o.put("t", "vibrate").put("n", intensity);
        self.uart_tx_json(&o);
    }

    /// `onScreenshotReq` (`:1829-1837`): JavaScript, not a message.
    pub fn on_screenshot_req(&mut self) {
        self.uart_tx(b"\x10g.dump()\n".to_vec());
    }

    /// `onSetGpsLocation` (`:1299-1325`). `course`, `hdop` and `speed` are
    /// `float`s widened to `double`s, so they print with the digits a
    /// `float` never had (`3.9000000953674316`).
    pub fn on_set_gps_location(&mut self, location: &Location) {
        if !self.prefs.use_updated_location {
            return;
        }
        let mut o = Object::new();
        o.put("t", "gps")
            .put("lat", location.latitude)
            .put("lon", location.longitude)
            .put("alt", location.altitude)
            .put("speed", f64::from(location.speed) * 3.6)
            .put(
                "course",
                location.bearing.map(|b| Json::Double(f64::from(b))),
            )
            .put("time", location.time)
            .put("satellites", location.satellites.unwrap_or(0))
            .put("hdop", f64::from(location.accuracy))
            .put("externalSource", true)
            .put_str("gpsSource", location.provider.as_deref());
        // `JSONObject.put(String, double)` refuses NaN and infinities
        // (`:1322-1324`: a toast, and nothing sent).
        if o.members()
            .iter()
            .any(|(_, v)| matches!(v, Json::Double(d) if !d.is_finite()))
        {
            self.toast(Level::Error, "uartTxJSONError: Forbidden numeric value");
            return;
        }
        self.uart_tx_json(&o);
    }

    /// `onSetLoyaltyCards` (`:1858-1903`).
    pub fn on_set_loyalty_cards(&mut self, cards: &[LoyaltyCard]) {
        let items = cards
            .iter()
            .map(|card| {
                let mut c = Object::new();
                c.put("id", card.id)
                    .put(
                        "name",
                        self.render(Some(&crop_to_length(card.name.as_deref(), 40))),
                    )
                    .put("value", card.value.as_str())
                    .put_str("type", card.format.as_deref())
                    .put("expiration", card.expiry.map(|ms| Json::Long(ms / 1000)))
                    .put("color", card.color);
                if let Some(balance) = &card.balance {
                    c.put(
                        "balance",
                        self.render(Some(&crop_to_length(Some(balance), 20))),
                    );
                }
                if let Some(note) = &card.note {
                    c.put("note", self.render(Some(&crop_to_length(Some(note), 200))));
                }
                Json::Object(c)
            })
            .collect::<Vec<_>>();
        let mut o = Object::new();
        o.put("t", "cards").put("d", items);
        self.uart_tx_json(&o);
    }

    /// `onAddCalendarEvent` (`:1906-1940`), for a description that's
    /// already had its HTML and separator lines removed (`:1911-1922`).
    pub fn on_add_calendar_event(&mut self, spec: &CalendarEventSpec) {
        if !self.prefs.sync_calendar {
            return;
        }
        let rendered = |gb: &Self, text: &Option<String>, len| {
            gb.render(Some(&crop_to_length(text.as_deref(), len)))
        };
        let mut o = Object::new();
        o.put("t", "calendar")
            .put("id", spec.id)
            .put("type", spec.kind)
            .put("timestamp", spec.timestamp)
            .put("durationInSeconds", spec.duration_in_seconds)
            .put("title", rendered(self, &spec.title, 40))
            .put("description", rendered(self, &spec.description, 200))
            .put("location", rendered(self, &spec.location, 40))
            .put("calName", crop_to_length(spec.cal_name.as_deref(), 20))
            .put("color", spec.color)
            .put("allDay", spec.all_day);
        self.uart_tx_json(&o);
    }

    /// `onDeleteCalendarEvent` (`:1943-1957`).
    pub fn on_delete_calendar_event(&mut self, id: i64) {
        if !self.prefs.sync_calendar {
            return;
        }
        let mut o = Object::new();
        o.put("t", "calendar-").put("id", id);
        self.uart_tx_json(&o);
    }

    /// `deleteCalendarEvents` (`:1960-1980`): one id as a number, several
    /// as an array.
    pub fn delete_calendar_events(&mut self, ids: &[i64]) {
        if !self.prefs.sync_calendar || ids.is_empty() {
            return;
        }
        let mut o = Object::new();
        o.put("t", "calendar-");
        if let [id] = ids {
            o.put("id", *id);
        } else {
            o.put(
                "id",
                ids.iter().map(|&id| Json::Long(id)).collect::<Vec<_>>(),
            );
        }
        self.uart_tx_json(&o);
    }

    /// `onSendWeather` (`:2166-2173`): the weather changed.
    pub fn on_send_weather(&mut self) {
        if let Some(weather) = self.weather.clone() {
            self.weather_v1(&weather);
        }
    }

    /// `handleWeatherV1` (`:2001-2024`).
    fn weather_v1(&mut self, w: &WeatherSpec) {
        let mut o = Object::new();
        o.put("t", "weather")
            .put("v", 1)
            .put("temp", w.current_temp)
            .put("hi", w.today_max_temp)
            .put("lo", w.today_min_temp)
            .put("hum", w.current_humidity)
            .put("rain", w.precip_probability)
            // `Math.round(float)` is an `int`, and so is a tenth of it.
            .put("uv", java_round(w.uv_index * 10.0) / 10)
            .put("code", w.current_condition_code)
            .put_str("txt", w.current_condition.as_deref())
            .put("wind", f64::from(java_round(w.wind_speed * 100.0)) / 100.0)
            .put("wdir", w.wind_direction)
            .put_str("loc", w.location.as_deref());
        self.uart_tx_json(&o);
    }

    /// `handleWeatherV2` (`:2026-2115`): the current weather, packed and
    /// in base64 **with Android's line feeds left in** (`:2109`; they go
    /// out as `\n` escapes).
    ///
    /// TODO: no forecasts. With `f` set, a phone adds up to 25 hours and 7
    /// days of them (`:2057-2108`); this says there are none of either.
    fn weather_v2(&mut self, w: &WeatherSpec, forecast: bool) {
        let mut d: Vec<u8> = Vec::new();
        let signed = |d: &mut Vec<u8>, v: i32| d.push(v.clamp(-128, 127) as u8);
        let two = |d: &mut Vec<u8>, v: i32| d.extend_from_slice(&v.to_le_bytes()[..2]);
        let four = |d: &mut Vec<u8>, v: i32| d.extend_from_slice(&v.to_le_bytes());
        signed(&mut d, w.current_temp - 273);
        signed(&mut d, w.today_max_temp - 273);
        signed(&mut d, w.today_min_temp - 273);
        d.push(w.current_humidity as u8);
        d.push(w.precip_probability as u8);
        d.push(java_round(w.uv_index * 10.0) as u8);
        d.push(condition_code_mapping(w.current_condition_code) as u8);
        two(&mut d, java_round(w.wind_speed * 100.0));
        two(&mut d, w.wind_direction);
        signed(&mut d, w.dew_point - 273);
        two(&mut d, java_round(w.pressure * 10.0));
        d.push(w.cloud_cover as u8);
        four(&mut d, java_round(w.visibility * 10.0));
        four(&mut d, w.sun_rise);
        four(&mut d, w.sun_set);
        four(&mut d, w.moon_rise);
        four(&mut d, w.moon_set);
        two(&mut d, w.moon_phase);
        signed(&mut d, w.feels_like_temp - 273);
        if forecast {
            d.push(0);
            d.push(0);
        }
        let mut o = Object::new();
        o.put("t", "weather")
            .put("v", 2)
            .put_str("l", w.location.as_deref())
            .put_str("c", w.current_condition.as_deref())
            .put("d", java::base64_default(&d));
        self.uart_tx_json(&o);
    }

    /// `onSetNavigationInfo` (`:2393-2412`). None of its text is rendered
    /// or cropped.
    pub fn on_set_navigation_info(&mut self, spec: &NavigationInfoSpec) {
        let mut o = Object::new();
        o.put("t", "nav")
            .put_str("instr", spec.instruction.as_deref())
            .put_str("distance", spec.distance_to_turn.as_deref());
        if spec.next_action > 0 && (spec.next_action as usize) < NavigationInfoSpec::ACTIONS.len() {
            o.put(
                "action",
                NavigationInfoSpec::ACTIONS[spec.next_action as usize],
            );
        }
        o.put_str("eta", spec.eta.as_deref());
        self.uart_tx_json(&o);
    }

    /// `enableAccelSender` (`:696-710`): Sleep as Android starting or
    /// stopping its tracking.
    pub fn enable_accel_sender(&mut self, enable: bool) {
        let mut o = Object::new();
        o.put("t", "accelsender")
            .put("enable", enable)
            .put("interval", 1000);
        self.uart_tx_json(&o);
    }

    /// The `com.banglejs.uart.tx` intent (`:295-331`): a line from Tasker
    /// and the like, sent as it is if intents are allowed, with a line
    /// feed if it lacks one. Not in ISO-8859-1? Then `?` (`:391`).
    pub fn on_uart_tx_intent(&mut self, line: &str) {
        if !self.prefs.intents {
            return;
        }
        let mut bytes: Vec<u8> = line
            .chars()
            .map(|c| u8::try_from(u32::from(c)).unwrap_or(b'?'))
            .collect();
        if bytes.last() != Some(&b'\n') {
            bytes.push(b'\n');
        }
        self.uart_tx(bytes);
    }

    // ==========================================================================
    // Receiving
    // ==========================================================================

    /// `onCharacteristicChanged` (`:1191-1242`): one notification's bytes
    /// from the watch.
    ///
    /// - A packet longer than any before raises the MTU for what's sent
    ///   (`:1199-1200`).
    /// - XON and XOFF bytes (17 and 19) are flow control, and removed
    ///   (`:1202-1222`).
    /// - Lines end at `\n`, and **the character before it is dropped,
    ///   whatever it is** (`:1231`): a `\r` is expected.
    pub fn receive(&mut self, packet: &[u8]) {
        if self.prefs.allow_high_mtu && packet.len() > self.mtu_size {
            self.mtu_size = packet.len();
        }
        self.received_line.extend(
            packet
                .iter()
                .filter(|&&b| b != 17 && b != 19)
                .map(|&b| char::from(b)),
        );
        while let Some(p) = self.received_line.find('\n') {
            // By character: ISO-8859-1's upper half is two bytes here.
            let mut line: String = self.received_line[..p].into();
            line.pop();
            self.received_line.drain(..=p);
            if let Err(uncaught) = self.handle_uart_rx_line(&line) {
                self.events.push(Event::Uncaught(uncaught));
                return;
            }
        }
    }

    /// Whether part of a line is waiting for its end.
    pub fn has_partial_line(&self) -> bool {
        !self.received_line.is_empty()
    }

    /// `handleUartRxLine` (`:523-549`).
    fn handle_uart_rx_line(&mut self, line: &str) -> Result<(), String> {
        if line.is_empty() {
            return Ok(());
        }
        if line == ">Uncaught ReferenceError: \"GB\" is not defined" {
            self.toast(
                Level::Error,
                "'Android Integration' plugin not installed on Bangle.js",
            );
        } else if line.starts_with('{') {
            let handled = Object::parse(line).map_err(Thrown::Json).and_then(|json| {
                if json.has("t") {
                    self.handle_uart_rx_json(&json)
                } else {
                    self.events.push(Event::NotUnderstood(line.into()));
                    Ok(())
                }
            });
            match handled {
                Ok(()) => {}
                Err(Thrown::Json(JsonException(message))) => {
                    self.events.push(Event::MalformedJson(message));
                }
                Err(Thrown::Other(uncaught)) => return Err(uncaught),
            }
        } else if let Some(base64) = line.strip_prefix("data:image/bmp;base64,") {
            self.events
                .push(Event::Screenshot(java::base64_decode(base64)));
        } else {
            self.events.push(Event::IgnoredLine(line.into()));
        }
        Ok(())
    }

    /// `handleUartRxJSON` (`:551-639`).
    fn handle_uart_rx_json(&mut self, json: &Object) -> Result<(), Thrown> {
        let packet_type = json.get_string("t")?;
        match packet_type.as_str() {
            "info" => {
                let text = format!("Bangle.js: {}", json.get_string("msg")?);
                self.toast(Level::Info, text);
            }
            "warn" => {
                let text = format!("Bangle.js: {}", json.get_string("msg")?);
                self.toast(Level::Warn, text);
            }
            "error" => {
                let text = format!("Bangle.js: {}", json.get_string("msg")?);
                self.toast(Level::Error, text);
            }
            "ver" => {
                let fw = json.has("fw").then(|| json.get_string("fw")).transpose()?;
                let hw = json.has("hw").then(|| json.get_string("hw")).transpose()?;
                // `GBDeviceEventVersionInfo` only sets what it was given.
                if fw.is_some() {
                    self.firmware.clone_from(&fw);
                }
                if hw.is_some() {
                    self.hardware.clone_from(&hw);
                }
                self.events.push(Event::Version { fw, hw });
            }
            "findPhone" => {
                let start = json.has("n") && json.get_boolean("n")?;
                self.events.push(Event::FindPhone(start));
            }
            "music" => {
                let name = json.get_string("n")?.to_uppercase();
                let event = value_of(MUSIC_EVENTS, "GBDeviceEventMusicControl.Event", &name)?;
                self.events.push(Event::MusicControl(event));
            }
            "call" => {
                let name = json.get_string("n")?.to_uppercase();
                let event = value_of(CALL_EVENTS, "GBDeviceEventCallControl.Event", &name)?;
                self.events.push(Event::CallControl(event));
            }
            "status" => self.handle_battery_status(json)?,
            "notify" => self.handle_notification_control(json)?,
            "actfetch" => self.handle_activity_fetch(json)?,
            "act" => self.handle_activity(json),
            "actTrksList" => self.handle_act_trks_list(json)?,
            "actTrk" => self.handle_act_trk(json)?,
            "http" => self.handle_http(json)?,
            "force_calendar_sync" => {
                // `handleCalendarSync` (`:1003-1056`), less its database.
                if self.prefs.sync_calendar {
                    json.get_array("ids")?;
                    self.events.push(Event::CalendarSync);
                }
            }
            "intent" => self.handle_intent(json)?,
            "file" => {
                // `handleFile` (`:1148-1177`).
                let name = json.get_string("n")?.replace('/', "");
                let mode = if json.get_string("m")? == "w" {
                    "write"
                } else {
                    "append"
                };
                let contents = json.get_string("c")?;
                self.events.push(Event::File {
                    name,
                    mode,
                    contents,
                });
            }
            "gps_power" => {
                // `:619-627`.
                if json.get_boolean("status")? {
                    self.setup_gps_update_timer();
                } else {
                    self.stop_location_update();
                }
            }
            "accel" => {
                // `handleAcceleration` (`:740-746`).
                if json.has("accel") {
                    let accel = json.get_object("accel")?;
                    for axis in ["x", "y", "z"] {
                        accel.get_double(axis)?;
                    }
                    self.events.push(Event::Acceleration);
                }
            }
            "weather" => {
                // `handleWeather` (`:1982-1999`): the watch asks again.
                if let Some(weather) = self.weather.clone() {
                    match json.opt_int("v", 1) {
                        1 => self.weather_v1(&weather),
                        2 => self.weather_v2(&weather, json.opt_boolean("f", false)),
                        _ => {}
                    }
                }
            }
            _ => self.events.push(Event::NotUnderstood(packet_type)),
        }
        Ok(())
    }

    /// `setupGPSUpdateTimer` (`:1268-1296`). Marked as set up even when
    /// the setting keeps it from starting anything.
    fn setup_gps_update_timer(&mut self) {
        if self.gps_update_setup {
            return;
        }
        if self.prefs.gps_update {
            self.events.push(Event::LocationUpdates(true));
        } else {
            self.toast(
                Level::Info,
                "Phone gps data update is deactivated in the settings",
            );
        }
        self.gps_update_setup = true;
    }

    /// `stopLocationUpdate` (`:233-239`).
    fn stop_location_update(&mut self) {
        if self.gps_update_setup {
            self.events.push(Event::LocationUpdates(false));
            self.gps_update_setup = false;
        }
    }

    /// `handleBatteryStatus` (`:751-775`). The level only goes down while
    /// not charging, and only up while charging (`:761-768`), since the
    /// connection began.
    fn handle_battery_status(&mut self, json: &Object) -> Result<(), Thrown> {
        let mut state = BatteryState::Unknown;
        if json.has("chg") {
            state = if json.get_int("chg")? == 1 {
                BatteryState::Charging
            } else {
                BatteryState::Normal
            };
        }
        let mut level = None;
        if json.has("bat") {
            let mut b = json.get_int("bat")?.clamp(0, 100);
            if self.last_battery_percent < 0 {
                self.last_battery_percent = b;
            }
            if state == BatteryState::Normal && b > self.last_battery_percent {
                b = self.last_battery_percent;
            }
            if state == BatteryState::Charging && b < self.last_battery_percent {
                b = self.last_battery_percent;
            }
            self.last_battery_percent = b;
            level = Some(b);
        }
        let voltage = json
            .has("volt")
            .then(|| json.get_double("volt"))
            .transpose()?
            .map(|v| v as f32);
        self.events.push(Event::Battery {
            level,
            state,
            voltage,
        });
        Ok(())
    }

    /// `handleNotificationControl` (`:780-836`).
    fn handle_notification_control(&mut self, json: &Object) -> Result<(), Thrown> {
        let response = json.get_string("n")?.to_uppercase();
        if self.prefs.wake_on_open && response == "OPEN" {
            self.events.push(Event::WakePhone);
        }
        if response == "INVOKE_ACTION" {
            if !json.has("id") || !json.has("action") {
                return Ok(());
            }
            let id = json.opt_int("id", 0);
            let index = json.opt_int("action", -1);
            let handles = self
                .notification_actions
                .iter()
                .find(|(i, _)| *i == id)
                .map(|(_, h)| h);
            if let (Ok(index), Some(handles)) = (usize::try_from(index), handles)
                && let Some(&handle) = handles.get(index)
            {
                self.events.push(Event::NotificationControl {
                    event: "REPLY",
                    handle,
                    phone_number: None,
                    reply: None,
                });
            }
            return Ok(());
        }
        let event = value_of(
            NOTIFICATION_EVENTS,
            "GBDeviceEventNotificationControl.Event",
            &response,
        )?;
        let mut handle = 0;
        if json.has("id") {
            handle = i64::from(json.get_int("id")?);
        }
        let phone_number = json
            .has("tel")
            .then(|| json.get_string("tel"))
            .transpose()?;
        let reply = json
            .has("msg")
            .then(|| json.get_string("msg"))
            .transpose()?;
        if event == "REPLY"
            && let Some((_, found)) = self
                .reply_actions
                .iter()
                .find(|(id, _)| *id == handle as i32)
        {
            handle = *found;
        }
        self.events.push(Event::NotificationControl {
            event,
            handle,
            phone_number,
            reply,
        });
        Ok(())
    }

    fn unset_busy_task(&mut self) {
        if self.busy.take().is_none() {
            self.events.push(Event::UnsetBusyWhenIdle);
        }
    }

    /// `handleActivityFetch` (`:838-857`). The end sets where the next
    /// fetch asks from: the phone's clock, less a minute.
    fn handle_activity_fetch(&mut self, json: &Object) -> Result<(), Thrown> {
        let state = json.get_string("state")?;
        match state.as_str() {
            "start" => self.busy = Some(BusyTask::FetchingActivity),
            "end" => {
                self.last_sync_ms = self.now_ms - 1000 * 60;
                self.unset_busy_task();
                if self.fixes.new_data_on_fetch_end {
                    self.events.push(Event::NewData);
                }
            }
            _ => {}
        }
        self.full_sync_status = Some(state);
        Ok(())
    }

    /// `handleActivity` (`:862-907`): an activity sample, stored unless it
    /// says it's live (`"rt":1`), and then passed on only while Gadgetbridge
    /// has live activity on.
    ///
    /// - The timestamp is milliseconds cut to seconds and then to a Java
    ///   `int` (`:864`); missing, it's now.
    /// - `act` is an `ActivityKind`'s name in any case; one that isn't is
    ///   `UNKNOWN`, and none at all is `ACTIVITY` (`:869-878`).
    /// - Nothing here throws: every field has a fallback.
    fn handle_activity(&mut self, json: &Object) {
        let timestamp = (json.opt_long("ts", self.now_ms) / 1000) as i32;
        let hrm = json.opt_int("hrm", 0);
        let steps = json.opt_int("stp", 0);
        let intensity = json.opt_int("mov", NOT_MEASURED);
        let realtime = json.opt_int("rt", 0) == 1;
        let mut raw_kind = kinds::ACTIVITY;
        if json.has("act") {
            let name = json.opt_string("act", "").to_uppercase();
            raw_kind = kinds::KINDS
                .iter()
                .find(|(kind, _)| *kind == name)
                .map_or(kinds::UNKNOWN, |(_, code)| *code);
        }
        if hrm > 0 {
            self.events.push(Event::SleepAsAndroidHeartRate(hrm));
        }
        let sample = Sample {
            timestamp,
            raw_kind,
            heart_rate: hrm,
            steps,
            raw_intensity: intensity,
        };
        if !realtime {
            self.upsert_sample(sample);
        }
        if realtime && (self.realtime_hrm || self.realtime_step) {
            self.events.push(Event::RealtimeSample(sample));
        }
    }

    /// `BangleJSSampleProvider.upsertSample`
    /// (`devices/banglejs/BangleJSSampleProvider.java:91-130`): a sample
    /// with none stored within two minutes of it (inclusive) is inserted;
    /// otherwise its values go into one of those (see [`upsert_target`] and
    /// [`upsert_merge`]), and it isn't stored itself.
    pub fn upsert_sample(&mut self, sample: Sample) {
        // Java's `int` arithmetic wraps; a range that wraps finds nothing.
        let from = sample.timestamp.wrapping_sub(120);
        let to = sample.timestamp.wrapping_add(120);
        let near: Vec<i32> = if from <= to {
            self.samples.range(from..=to).map(|(&t, _)| t).collect()
        } else {
            Vec::new()
        };
        if near.is_empty() {
            self.samples.insert(sample.timestamp, sample);
            return;
        }
        let target = near[upsert_target(&near, sample.timestamp)];
        let stored = self.samples.get_mut(&target).expect("it was just found");
        upsert_merge(stored, &sample, self.fixes);
    }

    /// `BangleJSActivityTrack.signalFetchingEnded`
    /// (`BangleJSActivityTrack.java:257-264`).
    fn signal_fetching_ended(&mut self) {
        self.last_packet_count = -1;
        self.unset_busy_task();
        self.events.push(Event::NewData);
        if !self.fixes.new_data_on_fetch_end {
            self.toast(Level::Info, "End : Fetching sports details");
        }
    }

    /// `BangleJSActivityTrack.compileTrackRequest`
    /// (`BangleJSActivityTrack.java:239-249`).
    fn request_next_track(&mut self) {
        let id = self.tracks_list.remove(0).as_string();
        let mut o = Object::new();
        o.put("t", "fetchRec").put("id", id).put(
            "last",
            if self.tracks_list.is_empty() {
                "true"
            } else {
                "false"
            },
        );
        self.uart_tx_json(&o);
    }

    /// `BangleJSActivityTrack.handleActTrksList`
    /// (`BangleJSActivityTrack.java:70-84`): the recorder's logs to fetch.
    /// An empty list ends the fetch at once.
    fn handle_act_trks_list(&mut self, json: &Object) -> Result<(), Thrown> {
        self.tracks_list = json.get_array("list")?.to_vec();
        if self.tracks_list.is_empty() {
            self.signal_fetching_ended();
        } else {
            self.request_next_track();
        }
        Ok(())
    }

    /// `BangleJSActivityTrack.handleActTrk`
    /// (`BangleJSActivityTrack.java:87-…`): a packet of a log. Only its
    /// sequencing is modelled; the log's contents (a CSV, parsed into a
    /// workout) aren't kept.
    fn handle_act_trk(&mut self, json: &Object) -> Result<(), Thrown> {
        let count = if json.has("cnt") {
            json.get_int("cnt")?
        } else {
            0
        };
        if count != self.last_packet_count + 1 {
            self.signal_fetching_ended();
            let mut stop = Object::new();
            stop.put("t", "fetchRec").put("id", "stop");
            self.uart_tx_json(&stop);
            return Ok(());
        }
        json.get_string("log")?;
        if json.has("lines") {
            json.get_string("lines")?;
            self.last_packet_count = count;
        } else if self.tracks_list.is_empty() {
            self.signal_fetching_ended();
        } else {
            self.last_packet_count = -1;
            self.request_next_track();
        }
        Ok(())
    }

    /// `handleHttp` (`:912-998`): refused unless both the build and the
    /// device's settings allow it; then the request is made, and answered
    /// here with an empty response.
    fn handle_http(&mut self, json: &Object) -> Result<(), Thrown> {
        let id = json.get_string("id").ok();
        if !self.prefs.internet_build {
            self.uart_tx_json_error(
                "http",
                "Internet access not enabled in Gadgetbridge",
                id.as_deref(),
            );
            return Ok(());
        }
        if !self.prefs.internet_access {
            self.uart_tx_json_error(
                "http",
                "Internet access not enabled for this watch",
                id.as_deref(),
            );
            return Ok(());
        }
        let url = json.get_string("url")?;
        let method = if json.has("method") {
            json.get_string("method")?.to_uppercase()
        } else {
            "GET".into()
        };
        self.events.push(Event::Http { url, method });
        let mut o = Object::new();
        o.put("t", "http")
            .put_str("id", id.as_deref())
            .put("resp", "");
        self.uart_tx_json(&o);
        Ok(())
    }

    /// `handleIntent` (`:1061-1133`): refused, **with an answer**, unless
    /// "Allow Intents" is on.
    fn handle_intent(&mut self, json: &Object) -> Result<(), Thrown> {
        if !self.prefs.intents {
            self.uart_tx_json_error(
                "intent",
                "Android Intents not enabled, check Gadgetbridge Device Settings",
                None,
            );
            return Ok(());
        }
        let string = |key: &str| json.has(key).then(|| json.get_string(key)).transpose();
        let target = string("target")?.unwrap_or_else(|| "broadcastreceiver".into());
        let action = string("action")?;
        for key in ["flags", "categories"] {
            if json.has(key) {
                json.get_array(key)?;
            }
        }
        let package = string("package")?;
        let class = string("class")?;
        string("mimetype")?;
        string("data")?;
        let extras = if json.has("extra") {
            json.get_object("extra")?.members().to_vec()
        } else {
            Vec::new()
        };
        if ![
            "broadcastreceiver",
            "activity",
            "service",
            "foregroundservice",
        ]
        .contains(&target.as_str())
        {
            self.toast(
                Level::Info,
                format!("Targeting '{target}' isn't implemented or it doesn't exist."),
            );
        }
        self.events.push(Event::Intent {
            target,
            action,
            package,
            class,
            extras,
        });
        Ok(())
    }
}

/// `LimitedQueue.add` (`util/LimitedQueue.java:35-40`): the oldest goes
/// once there are 16.
fn limited_add<V>(queue: &mut Vec<(i32, V)>, id: i32, value: V) {
    if queue.len() >= 16 {
        queue.remove(0);
    }
    queue.push((id, value));
}

/// `Math.round(float)`: to the nearest `int`, halves up, saturating, NaN 0.
fn java_round(value: f32) -> i32 {
    (value + 0.5).floor() as i32
}

/// `conditionCodeMapping` (`:2133-2163`): an OpenWeatherMap condition code
/// as a byte. A code its group doesn't list is one below the group's base
/// (`indexOf` gives -1).
fn condition_code_mapping(code: i32) -> i32 {
    let index = |codes: &[i32]| {
        codes
            .iter()
            .position(|&c| c == code)
            .map_or(-1, |i| i as i32)
    };
    match code {
        ..300 => index(&[200, 201, 202, 210, 211, 212, 221, 230, 231, 232]),
        300..400 => 32 + index(&[300, 301, 302, 310, 311, 312, 313, 314, 321]),
        400..500 => 64 - 1,
        500..600 => 96 + index(&[500, 501, 502, 503, 504, 511, 520, 521, 522, 531]),
        600..700 => 128 + index(&[600, 601, 602, 611, 612, 613, 615, 616, 620, 621, 622]),
        700..800 => 160 + index(&[701, 711, 721, 731, 741, 751, 761, 762, 771, 781]),
        800..900 => 192 + index(&[800, 801, 802, 803, 804]),
        _ => 255,
    }
}

#[cfg(test)]
#[path = "device_tests.rs"]
mod tests;
