//! What the rest of Gadgetbridge hands its Bangle.js support to send: the
//! `…Spec` classes in its `model` package, cut down to the fields
//! `BangleJSDeviceSupport` reads. A `String?` there is an `Option<String>`
//! here, since a `null` and an empty string go out differently (see
//! [`crate::json::Object::put`]).

/// `model/NotificationSpec.kt`, as `onNotification` reads it
/// (`BangleJSDeviceSupport.java:1468-1522`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NotificationSpec {
    pub id: i32,
    /// The app's name. Not set for an SMS, which goes out as
    /// `SMS Message` (`:1498-1500`).
    pub source_name: Option<String>,
    /// `type == NotificationType.GENERIC_SMS`.
    pub sms: bool,
    pub title: Option<String>,
    pub subject: Option<String>,
    pub body: Option<String>,
    pub sender: Option<String>,
    pub phone_number: Option<String>,
    pub actions: Vec<NotificationAction>,
}

/// `NotificationSpec.Action`.
#[derive(Clone, Debug, PartialEq)]
pub struct NotificationAction {
    pub kind: ActionKind,
    pub title: Option<String>,
    pub handle: i64,
}

/// `NotificationSpec.Action.TYPE_…`, those `onNotification` tells apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionKind {
    /// `TYPE_WEARABLE_REPLY`: makes the notification one that can be
    /// replied to (`"reply":true`).
    WearableReply,
    /// `TYPE_WEARABLE_SIMPLE` or `TYPE_CUSTOM_SIMPLE`: listed in `actions`.
    Simple,
    /// Any other type: not sent.
    Other,
}

/// `model/CallSpec.kt:37-48`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CallSpec {
    /// One of the `CALL_…` constants below; anything else goes out with an
    /// empty `cmd`.
    pub command: i32,
    pub name: Option<String>,
    pub number: Option<String>,
}

impl CallSpec {
    pub const CALL_UNDEFINED: i32 = 0;
    pub const CALL_ACCEPT: i32 = 1;
    pub const CALL_INCOMING: i32 = 2;
    pub const CALL_OUTGOING: i32 = 3;
    pub const CALL_REJECT: i32 = 4;
    pub const CALL_START: i32 = 5;
    pub const CALL_END: i32 = 6;

    /// The constant's name less `CALL_`, in lower case, as `onSetCallState`
    /// finds it by reflection (`BangleJSDeviceSupport.java:1645-1652`);
    /// empty for a command that isn't one.
    pub fn command_name(command: i32) -> &'static str {
        match command {
            Self::CALL_UNDEFINED => "undefined",
            Self::CALL_ACCEPT => "accept",
            Self::CALL_INCOMING => "incoming",
            Self::CALL_OUTGOING => "outgoing",
            Self::CALL_REJECT => "reject",
            Self::CALL_START => "start",
            Self::CALL_END => "end",
            _ => "",
        }
    }
}

/// `model/MusicSpec.kt`. Equal specs aren't sent twice (see
/// `util/MediaManager.java:77-87`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MusicSpec {
    pub artist: Option<String>,
    pub album: Option<String>,
    pub track: Option<String>,
    /// Seconds.
    pub duration: i32,
    pub track_count: i32,
    pub track_nr: i32,
}

/// `model/MusicStateSpec.kt:24-52`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MusicStateSpec {
    /// `STATE_PLAYING` 0, `STATE_PAUSED` 1, `STATE_STOPPED` 2,
    /// `STATE_UNKNOWN` -1.
    pub state: i8,
    pub position: i32,
    pub play_rate: i32,
    pub shuffle: i8,
    pub repeat: i8,
}

impl Default for MusicStateSpec {
    fn default() -> Self {
        Self {
            state: -1,
            position: -1,
            play_rate: -1,
            shuffle: -1,
            repeat: -1,
        }
    }
}

/// `model/Alarm.java`, as `onSetAlarms` reads it
/// (`BangleJSDeviceSupport.java:1577-1598`). A Bangle.js has ten
/// (`BangleJSCoordinator.java:172-174`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Alarm {
    pub hour: i32,
    pub minute: i32,
    /// A bit a weekday, Monday first (`Alarm.java:32-41`); 0 is once.
    pub repetition: i32,
    pub enabled: bool,
    /// A slot nothing was ever set in: left out of what's sent.
    pub unused: bool,
}

/// An `android.location.Location`, as `onSetGpsLocation` reads it
/// (`BangleJSDeviceSupport.java:1299-1325`).
#[derive(Clone, Debug, PartialEq)]
pub struct Location {
    pub latitude: f64,
    pub longitude: f64,
    pub altitude: f64,
    /// Metres per second.
    pub speed: f32,
    pub bearing: Option<f32>,
    /// UTC milliseconds.
    pub time: i64,
    /// The `satellites` extra; `None` for a location without extras.
    pub satellites: Option<i32>,
    /// `getAccuracy()`, in metres: sent as `hdop`.
    pub accuracy: f32,
    pub provider: Option<String>,
}

/// `model/NavigationInfoSpec.kt`, as `onSetNavigationInfo` reads it
/// (`BangleJSDeviceSupport.java:2393-2412`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NavigationInfoSpec {
    pub instruction: Option<String>,
    pub distance_to_turn: Option<String>,
    /// An index into [`NavigationInfoSpec::ACTIONS`]; 0 and anything out
    /// of range (`ACTION_MERGE`, its newest, is 18) send no action.
    pub next_action: i32,
    pub eta: Option<String>,
}

impl NavigationInfoSpec {
    /// `navActions` (`BangleJSDeviceSupport.java:2400-2403`).
    pub const ACTIONS: [&'static str; 18] = [
        "",
        "continue",
        "left",
        "left_slight",
        "left_sharp",
        "right",
        "right_slight",
        "right_sharp",
        "keep_left",
        "keep_right",
        "uturn_left",
        "uturn_right",
        "offroute",
        "roundabout_right",
        "roundabout_left",
        "roundabout_straight",
        "roundabout_uturn",
        "finish",
    ];
}

/// `model/CalendarEventSpec.kt`, as `onAddCalendarEvent` reads it
/// (`BangleJSDeviceSupport.java:1906-1940`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CalendarEventSpec {
    pub id: i64,
    pub kind: i32,
    /// UTC seconds.
    pub timestamp: i32,
    pub duration_in_seconds: i32,
    pub title: Option<String>,
    pub description: Option<String>,
    pub location: Option<String>,
    pub cal_name: Option<String>,
    pub color: i32,
    pub all_day: bool,
}

/// `model/WeatherSpec.kt`, without its forecasts (see
/// `Gadgetbridge::weather_v2`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WeatherSpec {
    /// Kelvin, as are the other temperatures.
    pub current_temp: i32,
    pub today_max_temp: i32,
    pub today_min_temp: i32,
    pub current_humidity: i32,
    pub precip_probability: i32,
    pub uv_index: f32,
    pub current_condition_code: i32,
    pub current_condition: Option<String>,
    pub wind_speed: f32,
    pub wind_direction: i32,
    pub location: Option<String>,
    pub dew_point: i32,
    pub pressure: f32,
    pub cloud_cover: i32,
    pub visibility: f32,
    pub sun_rise: i32,
    pub sun_set: i32,
    pub moon_rise: i32,
    pub moon_set: i32,
    pub moon_phase: i32,
    pub feels_like_temp: i32,
}

/// A loyalty card, as `onSetLoyaltyCards` sends it
/// (`BangleJSDeviceSupport.java:1858-1903`), already filtered to the
/// barcode formats a Bangle.js shows, and its balance already as text.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LoyaltyCard {
    pub id: i32,
    pub name: Option<String>,
    /// The barcode's contents, or without one the card's number.
    pub value: String,
    pub format: Option<String>,
    /// UTC milliseconds.
    pub expiry: Option<i64>,
    pub color: i32,
    /// `balance + " " + balanceType`, if it has one.
    pub balance: Option<String>,
    pub note: Option<String>,
}
