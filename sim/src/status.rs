//! The status line, for the window's title and the page's: what
//! InfiniSim shows in a window of its own, and the watch's face doesn't
//! always. The time, the screen and its backlight (1 to 7,
//! [`brightness::BY_CURRENT`]'s levels), the phone, the battery and the
//! charger, and the simulator's own clock: how long since boot, and how
//! fast it runs. Text beside the watch rather than drawn on it, as the
//! simulator draws nothing on the face that the watch wouldn't.

use embassy_time::Instant;

use crate::brightness;
use crate::clock;
use crate::watch::Watch;

/// The status line, with the clock running `speed` times as fast as the
/// wall clock.
pub fn line(watch: &Watch, speed: f64) -> String {
    let time = clock::now().map_or("--:--".to_owned(), |t| {
        format!("{:02}:{:02}", t.hour, t.minute)
    });
    let level = brightness::BY_CURRENT
        .iter()
        .position(|&l| l == watch.backlight)
        .map(|rank| rank + 1);
    let screen = match (watch.off, level) {
        (Some(_), _) => "switched off".to_owned(),
        (None, None) => "screen off".to_owned(),
        (None, Some(level)) if watch.app.dim() => format!("dim, light {level}/7"),
        (None, Some(level)) => format!("light {level}/7"),
    };
    let night = if watch.app.quiet_night() {
        " · quiet night"
    } else {
        ""
    };
    let phone = if watch.connected { "phone" } else { "no phone" };
    let battery = watch.battery();
    let power = match (battery.power_present, battery.charging) {
        (_, true) => ", charging",
        (true, false) => ", charged",
        (false, false) => "",
    };
    let up = Instant::now().as_secs();
    let pace = if speed == 1.0 {
        String::new()
    } else {
        format!(" at {speed}×")
    };
    format!(
        "{time} · {screen}{night} · {phone} · {}%{power} · up {}:{:02}:{:02}{pace}",
        battery.percent(),
        up / 3600,
        up / 60 % 60,
        up % 60
    )
}
