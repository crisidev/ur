//! A stand-in for `src/power.rs`'s on-time counters, which the UI loop
//! keeps for the panel and backlight, and its count of what woke the
//! screen. The hourly report is printed, as the field log would show it,
//! less what only the hardware knows (sleep, the stack, the sensors').

use std::sync::Mutex;

use embassy_time::{Duration, Instant};

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

/// Milliseconds in each [`Meter`], and each [`Count`], since the last
/// report.
static TOTALS: Mutex<([u64; 2], [u32; 5])> = Mutex::new(([0; 2], [0; 5]));

pub fn add(meter: Meter, since: Instant) {
    let ms = since.elapsed().as_millis();
    TOTALS.lock().expect("never poisoned").0[meter as usize] += ms;
}

pub fn count(event: Count) {
    TOTALS.lock().expect("never poisoned").1[event as usize] += 1;
}

pub fn report(period: Duration) {
    let ([panel, backlight], [button, touch, phone, attention, glance]) =
        std::mem::take(&mut *TOTALS.lock().expect("never poisoned"));
    crate::watch::log(format!(
        "power, over {} min: panel {panel} ms, backlight {backlight} ms; wakes: button \
         {button}, touch {touch}, phone {phone}, attention {attention}, glance {glance}",
        period.as_secs() / 60
    ));
}
