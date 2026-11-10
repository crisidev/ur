//! `settings`: what the watch's flash holds, read through the firmware's
//! own storage (`src/storage.rs`) and decoded with its own decoders, as it
//! would load them at boot. With `--flash` and a dump of a watch's flash
//! (say, one running an older release), this shows what that release
//! saved, and the window then boots with it.

use crate::storage::{self, key};
use crate::themes::{self, Theme as _};
use crate::watch::run;
use crate::{alarm, brightness, buttons, geo};

/// The stored settings, a line each; what isn't stored says so (the UI
/// then starts with its default).
pub fn describe() -> Vec<String> {
    run(read(), &mut None)
}

async fn read() -> Vec<String> {
    let mut lines = Vec::new();
    let none = || "not stored".to_owned();

    let themes = [
        ("day theme", key::THEME_DAY, key::THEME_LIGHT, true),
        ("night theme", key::THEME_NIGHT, key::THEME_DARK, false),
    ];
    for (what, key, old, day) in themes {
        // By its ID, as boot finds it among this build's schemes (the
        // first, for one not built in).
        let value = storage::load::<4>(key).await.map_or_else(none, |id| {
            let id = u32::from_le_bytes(id);
            let theme = themes::get(day, themes::find(day, id));
            let known = themes::schemes(day).iter().any(|t| t.palette().id == id);
            let unknown = if known { "" } else { ", not built in" };
            format!("{} ({id:#010x}{unknown})", theme.palette().name)
        });
        lines.push(format!("{what}: {value}"));
        // What 0.10 and earlier saved, which boot migrates from while
        // there's no ID (see `themes::migrate`).
        if let Some([index]) = storage::load::<1>(old).await {
            let id = themes::migrate(day, index);
            let theme = themes::get(day, themes::find(day, id));
            let name = theme.palette().name;
            lines.push(format!("{what}, as 0.10 saved it: index {index} ({name})"));
        }
    }

    let value = storage::load::<1>(key::FORCE)
        .await
        .map_or_else(none, |[byte]| {
            crate::ui::Force::from_byte(byte).name().to_owned()
        });
    lines.push(format!("force: {value}"));

    let value = storage::load::<1>(key::BRIGHTNESS)
        .await
        .map_or_else(none, |[byte]| {
            brightness::Setting::from_byte(byte).name().to_owned()
        });
    lines.push(format!("brightness: {value}"));

    let value = storage::load::<1>(key::BUTTONS)
        .await
        .map_or_else(none, |[byte]| {
            buttons::Setting::from_byte(byte).name().to_owned()
        });
    lines.push(format!("button grid: {value}"));
    let value = storage::load::<1>(key::GRID)
        .await
        .map_or_else(none, |[byte]| {
            buttons::Layout::from_byte(byte).name().to_owned()
        });
    lines.push(format!("grid layout: {value}"));

    let value = storage::load(key::ALARMS).await.map_or_else(none, |bytes| {
        let alarms = alarm::Settings::from_bytes(bytes);
        let hhmm = |minutes: u16| format!("{:02}:{:02}", minutes / 60, minutes % 60);
        let on = |on: bool| if on { "on" } else { "off" };
        let at: Vec<_> = (alarms.at.iter().zip(alarms.at_time))
            .map(|(&switch, time)| format!("{} {}", hhmm(time), on(switch)))
            .collect();
        let timers: Vec<_> = alarms.timers.iter().map(|m| format!("{m} min")).collect();
        format!(
            "{}; smart wake {} (weekdays {}, weekends {}); timers {}",
            at.join(", "),
            on(alarms.smart),
            hhmm(alarms.smart_weekday),
            hhmm(alarms.smart_weekend),
            timers.join(", ")
        )
    });
    lines.push(format!("alarms: {value}"));

    let value = storage::load(key::PHONE_ALARMS)
        .await
        .map_or_else(none, |bytes| {
            let phone = alarm::Phone::from_bytes(bytes);
            let alarms: Vec<_> = phone
                .alarms()
                .iter()
                .map(|a| {
                    format!(
                        "{:02}:{:02} {:?} (days {:07b})",
                        a.time / 60,
                        a.time % 60,
                        a.switch,
                        a.days
                    )
                })
                .collect();
            if alarms.is_empty() {
                "none".into()
            } else {
                alarms.join(", ")
            }
        });
    lines.push(format!("phone alarms: {value}"));

    let value = storage::load(key::LOCATION)
        .await
        .map_or_else(none, |bytes| {
            let location = geo::Location::from_bytes(bytes);
            format!(
                "{:.4}, {:.4}",
                f64::from(location.latitude_micro) / 1e6,
                f64::from(location.longitude_micro) / 1e6
            )
        });
    lines.push(format!("location: {value}"));

    // Bonds are `ble`'s to decode (a bond is its `BOND_LEN`, 43 bytes),
    // which the simulator doesn't run.
    for (i, slot) in key::BONDS.into_iter().enumerate() {
        let stored = storage::load::<43>(slot).await.is_some();
        let value = if stored { "stored" } else { "none" };
        lines.push(format!("bond {}: {value}", i + 1));
    }
    lines
}
