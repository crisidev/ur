//! The simulator's commands: one per line, in script files, typed into the
//! terminal while the window is open, or piped in.
//!
//! They cover what the dev kit's debug inputs do (`scripts/screenshot.py`,
//! `src/debug.rs`), with the same coordinates (canvas pixels, 0–79), plus
//! what a dev kit can't be made to do: the phone's every message, the time
//! and place, sensors, and skipping ahead in time. [`HELP`] lists them.

use std::path::PathBuf;

use embassy_time::Duration;

use crate::activity;
use crate::aranet;
use crate::bthome_sensor;
use crate::clock;
use crate::decimal;
use crate::gb::{self, Incoming, LineBuffer, Outgoing};
use crate::geo;
use crate::gesture::{Gesture, Touch};
use crate::heart_reading;
use crate::route;
use crate::ui::{PhoneEvent, Unpaired};
use crate::watch::{Watch, find_theme, local_second};
use crate::{InputEvent, alarm, debug};

pub const HELP: &str = "\
Inputs (X,Y in canvas pixels, 0-79; 40,40 if left out):
  button [2] | glance        (2: a double press, 0.2 s apart)
  tap [X,Y [MS]] | long [X,Y] | up|down|left|right [X,Y]
                            a recognised gesture (a tap held MS ms, not 80)
  double X,Y                two taps 0.1 s apart (a double tap on the grid)
  press X,Y | lift          a finger down (or moved), and up: through the
                            touch task's recogniser, so holding it still
                            for 0.6 s is a long press, and moving it a swipe
The phone:
  gb LINE                    a line as Gadgetbridge sends it, through the
                             firmware's parser, e.g.
                             gb GB({\"t\":\"notify\",\"src\":\"Signal\",\"title\":\"Kari\"})
                             (the rest of the line as is: no quotes needed,
                             and no # comment)
  connect [silent] | disconnect
                             (with a dev kit, `connect` sends the time, as
                             Gadgetbridge does; `silent`, nothing)
  time YYYY-MM-DD HH:MM[:SS] [+HH:MM] | time now     the phone sets the time
  location LAT LON                                   a GPS fix
  notify APP WHO [AGO]       a notification (AGO: arrived e.g. 3h ago)
  notify N [AGO]             the dev kit's test notification N (0-8)
  call CALLER | call over
  music ARTIST TRACK [playing|paused] | music playing|paused
  find on|off                \"find my watch\"
  phone-alarms [HH:MM[/REP][/off] ...]   REP: Gadgetbridge's weekday bits
                             (Monday 1 ... Sunday 64; 0 or none: once)
  nav ACTION [DISTANCE [STREET]] | nav off    e.g. nav left \"150 m\" Storgata
  unpaired not-paired|pairing-lost|off
  passkey NNNNNN|off | update PERCENT|off|refused
  slots full|free            both Bluetooth connections in use (the phone
                             and a computer, say), so the grid's presses
                             can't go to Home Assistant
The wearer and the watch:
  battery MV [charging|charged|unplugged]
                             (the percentage starts over from it)
  steps N | heart BPM|off    a heart rate measured now (shown for 30 min)
  temperature C              the watch's own, its die (shown, or the
                             wrist's estimate from it, from the next
                             minute's reading, as on the watch)
  aranet PPM C RH% [HPA [BATTERY% [AGO]]] | aranet off
                             a nearby Aranet4's reading, as a scan hears
                             it (AGO: measured e.g. 4m ago); it gives the
                             clock face's temperature, and the Air screen
  bthome-temp C [RH% [BATTERY% [AGO]]] | bthome-temp off
                             a nearby BTHome thermometer's instead (AGO:
                             heard e.g. 4m ago)
  asleep on|off | movement N           what sleep tracking says (smart
                                       alarms, the quiet night)
  last-night HH:MM HH:MM [deep=MIN] [rem=MIN] [awake=MIN]
  demo-day                   fill today's history so far with a typical day
  alarm 1|2 HH:MM [on|off] | smart on|off [HH:MM [HH:MM]] | timer 1|2 MIN
  theme NAME|auto            pick a theme, and its time of day (auto: the sun's)
  buttons off|bthome|app|both
                             the button grid, and where its presses go (as
                             the Settings screen's BUTTONS row)
  grid 6|99                  the button grid's layout: six buttons, or two
                             layers, codes 1-99 (the GRID row)
Time:
  wait DURATION              e.g. 1.5s, 300ms, 10m, 2h (a bare number: seconds)
  until HH:MM                to the next time the clock shows HH:MM (and
                             the UI's once-a-second tick has seen it)
Output:
  shot FILE.png [THEME]      a screenshot (relative to --out); with THEME,
                             in its colours (as screenshot.py --theme)
  shot FILE.png --watch      the whole watch, as the window shows it
  shot FILE.png --panel      as the real panel shows it (its colours, black
                             level and backlight; see src/panel.rs); with
                             --watch too, the whole watch so
  record FILE.gif [--watch] [--still DURATION] | stop
                             an animated GIF (relative to --out) of the
                             screen (or the whole watch) from now until
                             `stop`, on the watch's clock; a still screen
                             is held at most DURATION (2s) in it, so
                             `wait`s and `until`s don't drag
  expect [not] STATE         fail unless: screen-on, dim, quiet-night,
                             buzzing, off (switched off), or clock (the
                             clock face, for someone to read)
  expect steps N             fail unless today's steps are N, on the clock
                             face and the Today screen alike
  settings                   what the flash holds (see --flash), as the
                             firmware decodes it
  echo TEXT | verbose on|off | help | quit
";

/// What the caller should do after a command.
pub enum Next {
    Continue,
    Quit,
}

/// Where screenshots go.
pub struct Output {
    pub dir: PathBuf,
    /// Pixels a canvas pixel, for the whole watch (`shot … --watch`).
    pub zoom: usize,
}

/// Runs one line. Blank lines and `#` comments do nothing.
pub fn execute(watch: &mut Watch, line: &str, output: &Output) -> Result<Next, String> {
    // `gb` takes the rest of the line as it is, unsplit: Gadgetbridge's
    // lines are full of quotes, and a `#` in one is text.
    if let Some(rest) = line.trim_start().strip_prefix("gb ") {
        gadgetbridge(watch, rest.trim())?;
        return Ok(Next::Continue);
    }
    let words = split(line)?;
    let Some((command, args)) = words.split_first() else {
        return Ok(Next::Continue);
    };
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let arg = |i: usize| args.get(i).copied();
    let need = |i: usize, what: &str| arg(i).ok_or_else(|| format!("{command}: needs {what}"));

    match command.as_str() {
        // ======================================================================
        // Inputs
        // ======================================================================
        "button" => match arg(0) {
            None => watch.input(InputEvent::Button),
            // A double press, as the dev kit's `button:2,0`.
            Some("2") => {
                watch.input(InputEvent::Button);
                watch.advance(Duration::from_millis(200));
                watch.input(InputEvent::Button);
            }
            Some(other) => return Err(format!("button: {other}? (2 for a double press)")),
        },
        "glance" => watch.input(InputEvent::Glance),
        "tap" | "long" | "up" | "down" | "left" | "right" => {
            let (x, y) = arg(0).map_or(Ok((40, 40)), point)?;
            let gesture = match command.as_str() {
                "tap" => Gesture::Tap,
                "long" => Gesture::LongPress,
                "up" => Gesture::SwipeUp,
                "down" => Gesture::SwipeDown,
                "left" => Gesture::SwipeLeft,
                _ => Gesture::SwipeRight,
            };
            // As the dev kit's injected gestures: a typical finger's timing,
            // or a tap's as asked (`tap X,Y MS`).
            let held_ms = match (gesture, arg(1)) {
                (Gesture::LongPress, _) => 800,
                (Gesture::Tap, Some(ms)) => number(ms)?,
                _ => 80,
            };
            watch.input(InputEvent::Touch(Touch {
                gesture,
                x,
                y,
                held_ms,
            }));
        }
        // Two taps 0.1 s apart, as the dev kit's input 19 (too quick for
        // two over the probe): a double tap, for the button grid.
        "double" => {
            let (x, y) = point(need(0, "X,Y")?)?;
            let tap = || {
                InputEvent::Touch(Touch {
                    gesture: Gesture::Tap,
                    x,
                    y,
                    held_ms: 80,
                })
            };
            watch.input(tap());
            watch.advance(Duration::from_millis(100));
            watch.input(tap());
        }
        "press" => {
            let (x, y) = point(need(0, "X,Y")?)?;
            // The middle of the canvas pixel, in panel pixels.
            watch.finger_at(x * 3 + 1, y * 3 + 1);
        }
        "lift" => watch.lift(),

        // ======================================================================
        // The phone
        // ======================================================================
        "gb" => return Err("gb: needs a line, as Gadgetbridge sends it".into()),
        "connect" | "disconnect" => {
            // (`connect silent` is for a dev kit, where a phone counts as
            // one once it writes, and `connect` sends the time as
            // Gadgetbridge does. Here, a connected phone is one.)
            if !matches!(arg(0), None | Some("silent"))
                || (command == "disconnect" && arg(0).is_some())
            {
                return Err(format!("{command} {}?", arg(0).unwrap_or_default()));
            }
            watch.connected = command == "connect";
            watch.log(format!("{command}ed"));
            watch.refresh();
        }
        "time" => {
            let (utc, offset) = if arg(0) == Some("now") {
                crate::platform::now()
            } else {
                parse_time(need(0, "a date")?, need(1, "a time")?, arg(2))?
            };
            clock::set(utc, offset as i16);
            watch.log("time set");
            watch.refresh();
        }
        "location" => {
            // In millionths of a degree, read as the watch reads the phone's.
            let number = |s: &str| {
                decimal::parse(s.as_bytes())
                    .map(|d| d.scaled(6))
                    .ok_or(format!("location: {s}: not a number"))
            };
            let (latitude, longitude) = (number(need(0, "LAT")?)?, number(need(1, "LON")?)?);
            let location =
                geo::Location::from_micro(latitude, longitude).ok_or("location: out of range")?;
            geo::set_location(location, true);
            watch.refresh();
        }
        "notify" => {
            // `notify N [AGO]`: the dev kit's test notifications.
            let (app, who, ago) = match need(0, "APP WHO, or N")?.parse::<u8>() {
                Ok(n) => {
                    let (app, who) = test_notification(n);
                    (app.to_owned(), who.to_owned(), arg(1))
                }
                Err(_) => (args[0].to_owned(), need(1, "WHO")?.to_owned(), arg(2)),
            };
            if let Some(ago) = ago {
                debug::backdate_next(duration(ago)?.as_secs() as i64 / 60);
            }
            watch.input(InputEvent::Phone(PhoneEvent::Notification {
                app: app.as_str().into(),
                who: who.as_str().into(),
            }));
        }
        "call" => {
            let event = match need(0, "CALLER, or over")? {
                "over" => PhoneEvent::CallOver,
                caller => PhoneEvent::IncomingCall {
                    caller: truncated(caller),
                },
            };
            watch.input(InputEvent::Phone(event));
        }
        "music" => match need(0, "ARTIST TRACK, or playing|paused")? {
            "playing" | "paused" => {
                let playing = args[0] == "playing";
                watch.input(InputEvent::Phone(PhoneEvent::MusicState { playing }));
            }
            artist => {
                let track = need(1, "TRACK")?;
                watch.input(InputEvent::Phone(PhoneEvent::MusicInfo {
                    artist: truncated(artist),
                    track: truncated(track),
                }));
                if let Some(state) = arg(2) {
                    let playing = on_off(state, "playing", "paused")?;
                    watch.input(InputEvent::Phone(PhoneEvent::MusicState { playing }));
                }
            }
        },
        "find" => {
            let on = on_off(need(0, "on|off")?, "on", "off")?;
            watch.input(InputEvent::Phone(PhoneEvent::Find { on }));
        }
        "phone-alarms" => {
            let mut alarms = alarm::Phone::default();
            for item in &args {
                let mut parts = item.split('/');
                let (hour, minute) = hh_mm(parts.next().expect("split yields one"))?;
                let mut days = 0;
                let mut on = true;
                for part in parts {
                    match part {
                        "off" => on = false,
                        "on" => on = true,
                        bits => days = bits.parse().map_err(|_| format!("phone-alarms: {item}"))?,
                    }
                }
                if !alarms.push(hour.into(), minute.into(), days, on) {
                    return Err("phone-alarms: too many".into());
                }
            }
            watch.input(InputEvent::Phone(PhoneEvent::Alarms(alarms)));
        }
        "nav" => {
            let step = match need(0, "ACTION, or off")? {
                "off" => None,
                action => Some(route::Step {
                    turn: route::Turn::from_action(Some(action)),
                    distance: truncated(arg(1).unwrap_or("")),
                    street: truncated(arg(2).unwrap_or("")),
                }),
            };
            watch.input(InputEvent::Phone(PhoneEvent::Route(step)));
        }
        "unpaired" => {
            let why = match need(0, "not-paired|pairing-lost|off")? {
                "not-paired" => Some(Unpaired::NotPaired),
                "pairing-lost" => Some(Unpaired::PairingLost),
                "off" => None,
                other => return Err(format!("unpaired: {other}?")),
            };
            watch.unpaired = why;
            match why {
                Some(why) => watch.input(InputEvent::Phone(PhoneEvent::Unpaired(why))),
                None => watch.refresh(),
            }
        }
        "passkey" => {
            watch.passkey = match need(0, "NNNNNN|off")? {
                "off" => None,
                key => Some(key.parse().map_err(|_| format!("passkey: {key}?"))?),
            };
            // As `ble`: the passkey needs the wearer to look.
            if watch.passkey.is_some() {
                watch.input(InputEvent::Attention);
            } else {
                watch.refresh();
            }
        }
        "slots" => {
            watch.slots_full = on_off(need(0, "full|free")?, "full", "free")?;
            watch.log(format!("connection slots {}", args[0]));
        }
        "update" => match need(0, "PERCENT|off|refused")? {
            // As `dfu` does when it won't install what arrived.
            "refused" => {
                watch.update_progress = None;
                watch.input(InputEvent::Phone(PhoneEvent::UpdateRefused));
            }
            progress => {
                watch.update_progress = match progress {
                    "off" => None,
                    percent => Some(percent.parse().map_err(|_| format!("update: {percent}?"))?),
                };
                watch.refresh();
            }
        },

        // ======================================================================
        // The wearer and the watch
        // ======================================================================
        "battery" => {
            watch.battery_mv = number(need(0, "MV")?)?;
            match arg(1) {
                None => {}
                Some("charging") => (watch.charging, watch.power_present) = (true, true),
                Some("charged") => (watch.charging, watch.power_present) = (false, true),
                Some("unplugged") => (watch.charging, watch.power_present) = (false, false),
                Some(other) => return Err(format!("battery: {other}?")),
            }
            watch.restart_battery();
            watch.refresh();
        }
        "steps" => {
            activity::walk(number(need(0, "N")?)?);
            watch.refresh();
        }
        "heart" => {
            // As the heart rate task publishes a reading: shown for half
            // an hour, and in this minute's history.
            match need(0, "BPM|off")? {
                "off" => heart_reading::forget(),
                bpm => heart_reading::publish(number(bpm)?),
            }
            watch.refresh();
        }
        "temperature" => {
            watch.temperature = number(need(0, "C")?)?;
            watch.refresh();
        }
        "aranet" => {
            let sighting = match need(0, "PPM C RH%, or off")? {
                "off" => None,
                ppm => Some(aranet_sighting(
                    number(ppm)?,
                    number(need(1, "C")?)?,
                    number(need(2, "RH%")?.trim_end_matches('%'))?,
                    arg(3).map(number).transpose()?.unwrap_or(1013.0),
                    arg(4)
                        .map(|b| number(b.trim_end_matches('%')))
                        .transpose()?
                        .unwrap_or(87),
                    arg(5).map(duration).transpose()?.unwrap_or_default(),
                )?),
            };
            watch.hear_aranet(sighting);
        }
        "bthome-temp" => {
            let sighting = match need(0, "C, or off")? {
                "off" => None,
                celsius => Some(bthome_sighting(
                    number(celsius)?,
                    arg(1)
                        .map(|h| number(h.trim_end_matches('%')))
                        .transpose()?,
                    arg(2)
                        .map(|b| number(b.trim_end_matches('%')))
                        .transpose()?,
                    arg(3).map(duration).transpose()?.unwrap_or_default(),
                )?),
            };
            watch.hear_aranet(sighting);
        }
        "asleep" => {
            activity::set_asleep(on_off(need(0, "on|off")?, "on", "off")?);
            watch.refresh();
        }
        "movement" => {
            activity::set_movement(number(need(0, "N")?)?);
            watch.refresh();
        }
        "last-night" => {
            let sleep = last_night(&args)?;
            activity::set_last_sleep(Some(sleep));
            watch.refresh();
        }
        "demo-day" => {
            demo_day()?;
            watch.refresh();
        }
        "alarm" => {
            let i = index(need(0, "1|2")?, alarm::ALARMS)?;
            let (hour, minute) = hh_mm(need(1, "HH:MM")?)?;
            let on = arg(2).map_or(Ok(true), |s| on_off(s, "on", "off"))?;
            // As if set on the alarms screen, and saved as it would be.
            watch.change_settings(|app| {
                app.alarms.at_time[i] = u16::from(hour) * 60 + u16::from(minute);
                app.alarms.at[i] = on;
            });
        }
        "smart" => {
            let mut alarms = watch.app.alarms;
            alarms.smart = on_off(need(0, "on|off")?, "on", "off")?;
            let minutes = |s: &str| hh_mm(s).map(|(h, m)| u16::from(h) * 60 + u16::from(m));
            if let Some(weekday) = arg(1) {
                alarms.smart_weekday = minutes(weekday)?;
            }
            if let Some(weekend) = arg(2) {
                alarms.smart_weekend = minutes(weekend)?;
            }
            watch.change_settings(|app| app.alarms = alarms);
        }
        "timer" => {
            let i = index(need(0, "1|2")?, alarm::TIMERS)?;
            let minutes = number(need(1, "MIN")?)?;
            watch.change_settings(|app| app.alarms.timers[i] = minutes);
        }
        "theme" => match need(0, "NAME|auto")? {
            "auto" => {
                watch.daytime = None;
                watch.refresh();
            }
            name => watch.choose_theme(name)?,
        },
        "buttons" => {
            use crate::buttons::Setting;
            watch.app.buttons = match need(0, "off|bthome|app|both")? {
                "off" => Setting::Off,
                "bthome" => Setting::BtHome,
                "app" => Setting::App,
                "both" => Setting::Both,
                other => return Err(format!("buttons: {other}? (off, bthome, app, or both)")),
            };
            watch.refresh();
        }
        "grid" => {
            use crate::buttons::Layout;
            watch.app.grid_layout = match need(0, "6|99")? {
                "6" => Layout::Six,
                "99" => Layout::TwoLayers,
                other => return Err(format!("grid: {other}? (6 or 99)")),
            };
            watch.refresh();
        }

        // ======================================================================
        // Time and output
        // ======================================================================
        "wait" => watch.advance(duration(need(0, "DURATION")?)?),
        "until" => {
            let (hour, minute) = hh_mm(need(0, "HH:MM")?)?;
            watch.advance_to_local(hour, minute)?;
        }
        "shot" => {
            let path = output.dir.join(need(0, "FILE")?);
            let flags = [arg(1), arg(2)];
            let flag = |name| flags.contains(&Some(name));
            match (flag("--watch"), flag("--panel")) {
                // The whole watch, as the window shows it.
                (true, mimic) => {
                    crate::shot::save_watch(&path, watch.backlight, output.zoom, mimic)?;
                }
                (false, true) => crate::shot::save_mimicked(&path, watch.backlight)?,
                (false, false) => {
                    watch.screenshot(&path, arg(1).map(find_theme).transpose()?)?;
                }
            }
            watch.log(format!("screenshot: {}", path.display()));
        }
        "record" => {
            let path = output.dir.join(need(0, "FILE.gif")?);
            let mut look = crate::record::Look::Screen;
            let mut still = crate::record::STILL;
            let mut options = args[1..].iter();
            while let Some(&option) = options.next() {
                match option {
                    "--watch" => look = crate::record::Look::Watch { zoom: output.zoom },
                    "--still" => {
                        still = duration(options.next().ok_or("record: --still needs a DURATION")?)?
                    }
                    other => return Err(format!("record: {other}? (--watch, --still DURATION)")),
                }
            }
            crate::record::start(path.clone(), look, still)?;
            watch.log(format!("recording: {}", path.display()));
        }
        "stop" => {
            let recorded = crate::record::stop()?;
            watch.log(recorded);
        }
        "expect" if arg(0) == Some("steps") => {
            // Today's total, as the clock face shows it, and as the
            // Today screen adds it up: they must agree.
            let want: u32 = number(need(1, "N")?)?;
            let count = activity::steps_today();
            let chart = activity::today()
                .map(|day| day.hours.iter().map(|h| u32::from(h.steps)).sum::<u32>());
            if count != Some(want) || chart != Some(want) {
                return Err(format!(
                    "expected {want} steps today (the clock face has {count:?}, Today {chart:?})"
                ));
            }
        }
        "expect" => {
            let (negated, what) = match need(0, "STATE")? {
                "not" => (true, need(1, "STATE")?),
                what => (false, what),
            };
            let actual = match what {
                "screen-on" => watch.backlight != crate::brightness::Level::OFF,
                "dim" => watch.app.dim(),
                "quiet-night" => watch.app.quiet_night(),
                "buzzing" => watch.buzzing(),
                "off" => watch.off.is_some(),
                "clock" => watch.app.showing_clock(),
                other => return Err(format!("expect: {other}? (try `help`)")),
            };
            if actual == negated {
                let not = if negated { "not " } else { "" };
                return Err(format!("expected {not}{what}"));
            }
        }
        "settings" => {
            for line in crate::settings::describe() {
                watch.log(line);
            }
        }
        "echo" => watch.log(args.join(" ")),
        "verbose" => watch.verbose = on_off(need(0, "on|off")?, "on", "off")?,
        "help" => crate::platform::log(HELP.trim_end()),
        "quit" | "exit" => return Ok(Next::Quit),
        other => return Err(format!("unknown command {other:?} (try `help`)")),
    }
    Ok(Next::Continue)
}

/// `gb LINE`: a line as Gadgetbridge writes it to the watch, through the
/// firmware's own line buffer and parser (`src/gb.rs`), and acted on as
/// `src/ble.rs` acts on it: the clock and location set, events to the UI,
/// replies printed. What `ble` would do over the radio (GPS requests, the
/// activity sync) isn't simulated.
fn gadgetbridge(watch: &mut Watch, line: &str) -> Result<(), String> {
    let bytes = gb_wire(line)?;

    // As `ble`: whoever writes to the UART is the phone.
    if !watch.connected {
        watch.connected = true;
        watch.log("connected");
    }
    let mut buffer = LineBuffer::new();
    buffer.push(&bytes, |line| match gb::parse(line) {
        Some(Incoming::SetTime {
            utc_seconds,
            offset_minutes,
        }) => {
            clock::set(utc_seconds, offset_minutes);
            watch.log(format!("time set (UTC offset {offset_minutes} min)"));
            watch.refresh();
        }
        Some(Incoming::Gps {
            latitude_micro,
            longitude_micro,
            accuracy,
        }) => match geo::Location::from_micro(latitude_micro, longitude_micro) {
            Some(location) => {
                geo::set_location(location, aranet::precise(accuracy));
                let degrees = |micro: i64| micro as f64 / 1e6;
                watch.log(format!(
                    "location set: {}, {}",
                    degrees(latitude_micro),
                    degrees(longitude_micro)
                ));
                watch.refresh();
            }
            None => watch.log("location out of range; ignored"),
        },
        // `ble` says whether it's waiting for a fix; the simulator never
        // asks for one.
        Some(Incoming::IsGpsActive) => {
            let _ = crate::ble::OUTBOX.try_send(Outgoing::GpsPower(false));
            watch.refresh();
        }
        Some(Incoming::ActivityFetch { since_ms }) => activity_fetch(watch, since_ms),
        // As `ble` does: the UI loop's passes run the session, and send
        // its reports.
        Some(Incoming::Live {
            heart_rate,
            steps,
            interval,
        }) => {
            let request = crate::live::Request::new(heart_rate, steps, interval);
            crate::live::ask(request);
            if request.wanted() {
                watch.log(format!(
                    "live activity: heart rate {heart_rate}, steps {steps}, every {} s",
                    request.interval
                ));
            } else {
                watch.log("live activity: stop");
            }
            watch.refresh();
        }
        // As `ble::interpret` does: the warning after the message.
        Some(Incoming::Phone {
            event,
            text_as_bitmaps,
        }) => {
            watch.input(InputEvent::Phone(event));
            if text_as_bitmaps {
                watch.input(InputEvent::Phone(PhoneEvent::TextAsBitmaps));
            }
        }
        Some(Incoming::Ignored(kind)) => watch.log(format!("ignoring {kind} message")),
        None => watch.log("not a line the watch understands; ignored"),
    });
    Ok(())
}

/// As `ble::send_activity` does: the history since what Gadgetbridge asked
/// for, less what's revised since, in buckets (see `gb_buckets`), the
/// bucket under way left for later. Straight to the log rather than
/// through `ble::OUTBOX`, which holds a handful: a fetch is hundreds of
/// lines. The simulator's history has steps and heart rates, but no
/// movement or sleep, nor `ble`'s 24-hour limit.
fn activity_fetch(watch: &mut Watch, since_ms: i64) {
    let send = |message: Outgoing| {
        let line = message.to_line();
        watch.log(format!("to the phone: {}", line.trim_end()));
    };
    send(Outgoing::ActivityFetch { done: false });
    let now = clock::now_utc_seconds();
    let since =
        gb::fetch_from(since_ms, now).div_euclid(60_000) - crate::stages::REVISED as i64;
    if let Some(now) = now {
        let mut fetch = crate::gb_buckets::Fetch::new(since, i64::MIN / 2, now.div_euclid(60));
        while let Some(bucket) = fetch.next(activity::history_since(fetch.from()).into_iter().map(
            |(minute, (steps, heart_rate))| {
                let sample = crate::gb_buckets::Minute {
                    steps,
                    movement: 0,
                    heart_rate,
                    seen: true,
                    kind: None,
                };
                (minute, sample)
            },
        )) {
            send(Outgoing::Activity {
                timestamp_ms: bucket.timestamp_ms(),
                steps: bucket.steps,
                movement: bucket.movement,
                heart_rate: bucket.heart_rate,
                kind: bucket.kind,
            });
        }
    }
    // The empty track list too: the simulator doesn't keep `ble`'s pace
    // for it.
    for message in gb::FETCH_END {
        send(message);
    }
}

/// A `gb` line as Gadgetbridge puts it on the wire: ISO-8859-1, between
/// Espruino's "don't echo this" and a newline. Each of the script's
/// characters up to U+00FF is one byte, and Gadgetbridge escapes the rest.
pub fn gb_wire(line: &str) -> Result<Vec<u8>, String> {
    let mut bytes = vec![0x10];
    for c in line.chars() {
        let byte = u8::try_from(u32::from(c)).map_err(|_| {
            format!("gb: {c:?} isn't ISO-8859-1; escape it as Gadgetbridge does (\\uXXXX)")
        })?;
        bytes.push(byte);
    }
    bytes.push(b'\n');
    Ok(bytes)
}

/// Runs a script file, line by line, stopping at the first error.
#[cfg(not(target_arch = "wasm32"))]
pub fn run_file(
    watch: &mut Watch,
    path: &std::path::Path,
    output: &Output,
) -> Result<Next, String> {
    let text =
        std::fs::read_to_string(path).map_err(|e| format!("reading {}: {e}", path.display()))?;
    run(watch, &path.display().to_string(), &text, output)
}

/// Runs a script, line by line, stopping at the first error, which names
/// the script (`name`) and the line.
pub fn run(watch: &mut Watch, name: &str, text: &str, output: &Output) -> Result<Next, String> {
    for (n, line) in text.lines().enumerate() {
        match execute(watch, line, output) {
            Ok(Next::Continue) => {}
            Ok(Next::Quit) => return Ok(Next::Quit),
            Err(e) => return Err(format!("{name}:{}: {e}", n + 1)),
        }
    }
    Ok(Next::Continue)
}

// ==============================================================================
// Parsing
// ==============================================================================

/// Splits a line into words: whitespace between them, double quotes around
/// words with spaces in, and a `#` outside quotes starts a comment.
pub fn split(line: &str) -> Result<Vec<String>, String> {
    let mut words = Vec::new();
    let mut word: Option<String> = None;
    let mut quoted = false;
    for c in line.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                word.get_or_insert_default();
            }
            '#' if !quoted => break,
            c if c.is_whitespace() && !quoted => words.extend(word.take()),
            c => word.get_or_insert_default().push(c),
        }
    }
    if quoted {
        return Err("unterminated quote".into());
    }
    words.extend(word);
    Ok(words)
}

/// A nearby Aranet4's reading, heard now: built as its advertisement's
/// bytes, and read back through the firmware's own parser, so the
/// simulator shows what the watch would make of them. The status colour
/// is left out, so the band follows the default thresholds.
fn aranet_sighting(
    ppm: u16,
    celsius: f32,
    humidity: u8,
    hpa: f32,
    battery: u8,
    ago: Duration,
) -> Result<aranet::Sighting, String> {
    let mut payload = [0u8; 22];
    // Integrations on, firmware 1.4.4.
    payload[..4].copy_from_slice(&[0x20, 4, 4, 1]);
    payload[8..10].copy_from_slice(&ppm.to_le_bytes());
    let twentieths = (celsius * 20.0).round();
    if !(0.0..16384.0).contains(&twentieths) {
        return Err(format!("{celsius} °C: out of an Aranet4's range"));
    }
    payload[10..12].copy_from_slice(&(twentieths as u16).to_le_bytes());
    payload[12..14].copy_from_slice(&((hpa * 10.0).round() as u16).to_le_bytes());
    payload[14] = humidity;
    payload[15] = battery;
    payload[17..19].copy_from_slice(&300u16.to_le_bytes());
    let age = u16::try_from(ago.as_secs()).map_err(|_| "AGO: under 18 h".to_owned())?;
    payload[19..21].copy_from_slice(&age.to_le_bytes());
    let reading = aranet::parse(&payload).map_err(|skip| format!("{skip:?}"))?;
    Ok(aranet::Sighting {
        address: [0x2a, 0x3f, 0x11, 0x22, 0x33, 0xc4],
        rssi: -60,
        reading,
        heard: embassy_time::Instant::now().as_millis(),
    })
}

/// A nearby BTHome thermometer's reading, heard `ago`: built as the
/// service data a LYWSD03MMC on pvvx's firmware sends (packet id,
/// battery, temperature and humidity in hundredths), and read back through
/// the firmware's own parser.
fn bthome_sighting(
    celsius: f32,
    humidity: Option<f32>,
    battery: Option<u8>,
    ago: Duration,
) -> Result<aranet::Sighting, String> {
    let mut data = vec![0x40, 0x00, 0x2A];
    if let Some(battery) = battery {
        data.extend_from_slice(&[0x01, battery]);
    }
    let hundredths = (celsius * 100.0).round();
    if !(-32768.0..32768.0).contains(&hundredths) {
        return Err(format!("{celsius} °C: out of BTHome's range"));
    }
    data.push(0x02);
    data.extend_from_slice(&(hundredths as i16).to_le_bytes());
    if let Some(humidity) = humidity {
        data.push(0x03);
        data.extend_from_slice(&((humidity * 100.0).round() as u16).to_le_bytes());
    }
    let mut reading = bthome_sensor::parse(&data).map_err(|skip| format!("{skip:?}"))?;
    // Heard now, as old as if heard `ago` (the simulator's uptime may be
    // shorter than that).
    reading.age = u16::try_from(ago.as_secs()).map_err(|_| "AGO: under 18 h".to_owned())?;
    Ok(aranet::Sighting {
        address: [0xb2, 0x18, 0x8d, 0x38, 0xc1, 0xa4],
        rssi: -60,
        reading,
        heard: embassy_time::Instant::now().as_millis(),
    })
}

fn number<T: std::str::FromStr>(s: &str) -> Result<T, String> {
    s.parse()
        .map_err(|_| format!("{s:?} isn't a number in range"))
}

fn index(s: &str, count: usize) -> Result<usize, String> {
    match s.parse::<usize>() {
        Ok(n) if (1..=count).contains(&n) => Ok(n - 1),
        _ => Err(format!("{s:?}: not 1 to {count}")),
    }
}

fn on_off(s: &str, on: &str, off: &str) -> Result<bool, String> {
    match s {
        _ if s == on => Ok(true),
        _ if s == off => Ok(false),
        _ => Err(format!("{s:?}: {on} or {off}?")),
    }
}

/// `X,Y` in canvas pixels.
pub fn point(s: &str) -> Result<(i32, i32), String> {
    let (x, y) = s.split_once(',').ok_or_else(|| format!("{s:?}: X,Y?"))?;
    let coordinate = |v: &str| match v.trim().parse::<i32>() {
        Ok(v) if (0..80).contains(&v) => Ok(v),
        _ => Err(format!("{s:?}: coordinates are 0 to 79")),
    };
    Ok((coordinate(x)?, coordinate(y)?))
}

fn hh_mm(s: &str) -> Result<(u8, u8), String> {
    let (h, m) = s.split_once(':').ok_or_else(|| format!("{s:?}: HH:MM?"))?;
    match (h.parse::<u8>(), m.parse::<u8>()) {
        (Ok(h), Ok(m)) if h < 24 && m < 60 => Ok((h, m)),
        _ => Err(format!("{s:?}: HH:MM?")),
    }
}

/// `1.5s`, `300ms`, `10m`, `2h`, or seconds.
pub fn duration(s: &str) -> Result<Duration, String> {
    let (value, unit_ms) = if let Some(v) = s.strip_suffix("ms") {
        (v, 1.0)
    } else if let Some(v) = s.strip_suffix('s') {
        (v, 1e3)
    } else if let Some(v) = s.strip_suffix('m') {
        (v, 60e3)
    } else if let Some(v) = s.strip_suffix('h') {
        (v, 3600e3)
    } else {
        (s, 1e3)
    };
    let value: f64 = value.parse().map_err(|_| format!("{s:?}: a duration?"))?;
    if value < 0.0 {
        return Err(format!("{s:?}: time only goes forwards"));
    }
    Ok(Duration::from_micros((value * unit_ms * 1e3) as u64))
}

/// `2026-11-09 07:30[:00] [+01:00]` → (UTC seconds, offset minutes).
fn parse_time(date: &str, time: &str, offset: Option<&str>) -> Result<(i64, i32), String> {
    let bad = || format!("time: {date} {time}: YYYY-MM-DD HH:MM[:SS]?");
    let mut d = date.split('-').map(str::parse::<i64>);
    let (Some(Ok(year)), Some(Ok(month)), Some(Ok(day)), None) =
        (d.next(), d.next(), d.next(), d.next())
    else {
        return Err(bad());
    };
    let mut t = time.split(':').map(str::parse::<i64>);
    let (Some(Ok(hour)), Some(Ok(minute))) = (t.next(), t.next()) else {
        return Err(bad());
    };
    let second = t.next().unwrap_or(Ok(0)).map_err(|_| bad())?;
    let offset_minutes = match offset {
        None => 0,
        Some(o) => {
            let sign = match o.chars().next() {
                Some('+') => 1,
                Some('-') => -1,
                _ => return Err(format!("time: {o}: +HH:MM?")),
            };
            let (h, m) = hh_mm(&o[1..])?;
            sign * (i32::from(h) * 60 + i32::from(m))
        }
    };
    let days = clock::DateTime {
        year: year as u16,
        month: month as u8,
        day: day as u8,
        hour: 0,
        minute: 0,
        second: 0,
    }
    .days_since_epoch();
    let local = days * 86_400 + hour * 3600 + minute * 60 + second;
    Ok((local - i64::from(offset_minutes) * 60, offset_minutes))
}

/// Copies `s` into a fixed-size string, cutting it short (at a character
/// boundary) if need be, as the phone's messages are.
pub fn truncated<const N: usize>(s: &str) -> heapless::String<N> {
    let mut out = heapless::String::new();
    for c in s.chars() {
        if out.push(c).is_err() {
            break;
        }
    }
    out
}

/// The dev kit's test notifications (`src/debug.rs`, input 9).
pub fn test_notification(n: u8) -> (&'static str, &'static str) {
    match n {
        1 => ("Podcasts", "Night Signals: Ida Moreau"),
        2 => ("Calendar", "Quarterly planning: platform team, 4B"),
        3 => ("Signal", "Nora Lindqvist"),
        4 => ("Signal", "Åse Kristiansen"),
        5 => ("Messages", "Ida"),
        6 => ("Punctuation", "!\"#$%&' ()*+,-./"),
        7 => ("Punctuation", ":;<=>?@ [\\]^_ `{|}~"),
        8 => (
            "Messages",
            "\u{201c}Don\u{2019}t\u{201d} \u{2013} Ida\u{2026}",
        ),
        _ => ("Signal", "Jane Smith"),
    }
}

/// `last-night 23:10 06:45 [deep=M] [rem=M] [awake=M]`: the night that
/// ended most recently at the second time.
fn last_night(args: &[&str]) -> Result<activity::Sleep, String> {
    let [start, end, rest @ ..] = args else {
        return Err("last-night: needs HH:MM HH:MM".into());
    };
    let now = local_second().ok_or("last-night: the time isn't set (see `time`)")?;
    let offset = i64::from(clock::utc_offset_minutes().expect("set with the time"));
    let minute_of = |s: &str| hh_mm(s).map(|(h, m)| i64::from(h) * 60 + i64::from(m));
    let (start, end) = (minute_of(start)?, minute_of(end)?);
    // The latest end at or before now, and the start before it.
    let now_minute = now.div_euclid(60);
    let today = now_minute - now_minute.rem_euclid(1440);
    let mut end_local = today + end;
    if end_local > now_minute {
        end_local -= 1440;
    }
    let mut start_local = end_local - end + start;
    if start_local >= end_local {
        start_local -= 1440;
    }
    let length = end_local - start_local;
    let (mut deep, mut rem, mut awake) = (length / 5, length / 4, 15);
    for option in rest {
        let (key, value) = option
            .split_once('=')
            .ok_or_else(|| format!("last-night: {option}?"))?;
        let value = number(value)?;
        match key {
            "deep" => deep = value,
            "rem" => rem = value,
            "awake" => awake = value,
            _ => return Err(format!("last-night: {key}?")),
        }
    }
    Ok(activity::Sleep {
        start_utc_minute: start_local - offset,
        end_utc_minute: end_local - offset,
        asleep_minutes: (length - awake).clamp(0, i64::from(u16::MAX)) as u16,
        deep_minutes: deep as u16,
        rem_minutes: rem as u16,
    })
}

/// Fills today's history, from midnight to now, with a plausible day:
/// asleep until 7 with a low heart rate, a walk to work and back, an
/// evening run. The same day every time. By 17:42 (the README's clock
/// face), that's about 6,500 steps, so its meter shows six of ten blocks
/// rather than the bar past the goal; the run takes the day to about
/// 16,700.
fn demo_day() -> Result<(), String> {
    let now = local_second().ok_or("demo-day: the time isn't set (see `time`)")?;
    let offset = i64::from(clock::utc_offset_minutes().expect("set with the time"));
    let now_minute = now.div_euclid(60);
    let midnight = now_minute - now_minute.rem_euclid(1440);
    let mut steps_today = 0;
    for local in midnight..now_minute {
        let minute = local - midnight;
        let hour = minute / 60;
        // A little wobble, the same every run.
        let wobble = (minute * 37 % 11) as u16;
        let (steps, heart) = match hour {
            0..7 => (0, (52 + wobble % 5) as u8),
            7 => (5 + wobble, 64 + wobble as u8),
            8 => (30 + wobble * 2, 88 + wobble as u8),
            12 => (8 + wobble, 74 + wobble as u8),
            17 => (30 + wobble * 2, 92 + wobble as u8),
            19 => (140 + wobble * 2, 145 + wobble as u8 * 2),
            9..23 => (1 + wobble / 3, 66 + wobble as u8),
            _ => (0, 58 + wobble as u8 % 4),
        };
        let utc = local - offset;
        activity::record_steps_at(utc, steps);
        // As awake and still: a reading every twenty minutes, at most.
        if hour < 7 || minute % 5 == 0 {
            activity::record_heart_rate_at(utc, heart);
        }
        steps_today += u32::from(steps);
    }
    // Today's total, as the step counter would have it (not `walk`, which
    // would count the history's steps again).
    activity::add_to_today(steps_today);
    Ok(())
}
