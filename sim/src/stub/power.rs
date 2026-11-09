//! A stand-in for `src/power.rs`'s on-time counters, which the UI loop
//! keeps for the panel and backlight, and its count of what woke the
//! screen. The hourly report is printed, as the field log would show it,
//! less what only the hardware knows (sleep, the stack, the sensors').
//! The counting is the watch's own (`src/on_time.rs`).

use std::sync::Mutex;

use embassy_time::{Duration, Instant};

use crate::on_time::OnTime;

/// The states the UI loop times (the watch's `Meter` has more).
#[derive(Clone, Copy)]
pub enum Meter {
    Panel,
    Backlight,
}

/// What woke the screen (the watch's `Count` has more), by its names.
#[allow(clippy::enum_variant_names)]
#[derive(Clone, Copy)]
pub enum Count {
    WakeButton,
    WakeTouch,
    WakePhone,
    WakeAttention,
    WakeGlance,
}

/// Time in each [`Meter`]'s state, as the watch keeps it.
static ON_TIME: Mutex<[OnTime; 2]> = Mutex::new([OnTime::new(), OnTime::new()]);
/// Each [`Count`] since the last report.
static COUNTS: Mutex<[u32; 5]> = Mutex::new([0; 5]);

fn millis() -> u32 {
    Instant::now().as_millis() as u32
}

pub fn on(meter: Meter) {
    ON_TIME.lock().expect("never poisoned")[meter as usize].on(millis());
}

pub fn off(meter: Meter) {
    ON_TIME.lock().expect("never poisoned")[meter as usize].off(millis());
}

pub fn count(event: Count) {
    COUNTS.lock().expect("never poisoned")[event as usize] += 1;
}

pub fn report(period: Duration) {
    let now = millis();
    let [panel, backlight] = ON_TIME
        .lock()
        .expect("never poisoned")
        .each_ref()
        .map(|meter| meter.take(now));
    let [button, touch, phone, attention, glance] =
        std::mem::take(&mut *COUNTS.lock().expect("never poisoned"));
    crate::watch::log(format!(
        "power, over {} min: panel {panel} ms, backlight {backlight} ms; wakes: button \
         {button}, touch {touch}, phone {phone}, attention {attention}, glance {glance}",
        period.as_secs() / 60
    ));
}

/// Forgets everything, as switching off does to the watch's RAM.
pub fn forget() {
    *ON_TIME.lock().expect("never poisoned") = [OnTime::new(), OnTime::new()];
    *COUNTS.lock().expect("never poisoned") = [0; 5];
}
