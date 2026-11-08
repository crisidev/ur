//! A stand-in for `src/activity.rs`, which reads the accelerometer: just
//! what the UI asks of it, from state the simulator sets (steps walked,
//! heart rates, last night's sleep, whether the wearer is asleep).
//!
//! The per-minute history is kept the way the real one is (UTC minutes,
//! steps and a heart rate each), so the Today screen adds it up through the
//! firmware's own [`crate::day::Day`].

use std::collections::BTreeMap;
use std::sync::Mutex;

use crate::clock;

/// A night's (or nap's) sleep: a copy of `activity::Sleep`, which the UI
/// shows on the Last night screen.
#[derive(Clone, Copy, PartialEq, Eq, defmt::Format)]
pub struct Sleep {
    pub start_utc_minute: i64,
    /// The minute after the last asleep one.
    pub end_utc_minute: i64,
    pub asleep_minutes: u16,
    pub deep_minutes: u16,
    pub rem_minutes: u16,
}

#[derive(Default)]
struct State {
    /// UTC minute → (steps, heart rate or 0).
    history: BTreeMap<i64, (u16, u8)>,
    /// Today's steps: (local day since 1970, steps).
    today: Option<(i64, u32)>,
    last_sleep: Option<Sleep>,
    asleep: bool,
    movement: u16,
    /// A dev kit's day, which `today` shows instead (see `devkit`).
    today_from_devkit: Option<Option<crate::day::Day>>,
}

static STATE: Mutex<State> = Mutex::new(State {
    history: BTreeMap::new(),
    today: None,
    last_sleep: None,
    asleep: false,
    movement: 0,
    today_from_devkit: None,
});

fn state() -> std::sync::MutexGuard<'static, State> {
    STATE.lock().expect("nothing panics while holding it")
}

pub fn asleep() -> bool {
    state().asleep
}

/// Whether the accelerometer finds the watch perfectly still: never here,
/// as the simulated wearer has it on.
///
/// TODO: a command for a watch left lying on a desk, which the
/// temperature's wear model notices (see `temperature::Wear`).
pub fn still() -> bool {
    false
}

pub fn movement() -> u16 {
    state().movement
}

pub fn last_sleep() -> Option<Sleep> {
    state().last_sleep
}

/// Today's steps, once the date is known: none yet is 0, as on the watch,
/// whose step counter's every poll counts towards today.
pub fn steps_today() -> Option<u32> {
    let today = clock::now()?.days_since_epoch();
    Some(
        state()
            .today
            .filter(|&(day, _)| day == today)
            .map_or(0, |(_, steps)| steps),
    )
}

/// As `activity::today`: the history since local midnight, hour by hour.
pub fn today() -> Option<crate::day::Day> {
    if let Some(day) = state().today_from_devkit {
        return day;
    }
    let utc_minute = clock::now_utc_seconds()?.div_euclid(60);
    let offset = i64::from(clock::utc_offset_minutes()?);
    let midnight = utc_minute - (utc_minute + offset).rem_euclid(24 * 60);
    let mut day = crate::day::Day::default();
    for (&minute, &(steps, heart_rate)) in state().history.range(midnight..=utc_minute) {
        day.add((minute - midnight) as usize, steps, heart_rate);
    }
    Some(day)
}

// ==============================================================================
// What the simulator sets
// ==============================================================================

/// Counts `steps` as walked now, as `activity::walk` does (in today's total
/// and this minute's history).
pub fn walk(steps: u32) {
    let day = clock::now().map(|now| now.days_since_epoch());
    let minute = clock::now_utc_seconds().map(|s| s.div_euclid(60));
    let mut state = state();
    state.today = match (state.today, day) {
        (Some((d, total)), Some(day)) if d == day => Some((day, total + steps)),
        (_, Some(day)) => Some((day, steps)),
        // Before the time is known, nothing can be filed under a day.
        (today, None) => today,
    };
    if let Some(minute) = minute {
        let sample = state.history.entry(minute).or_default();
        sample.0 = sample
            .0
            .saturating_add(steps.min(u32::from(u16::MAX)) as u16);
    }
}

/// Adds steps to today's total only, for steps already in the history.
pub fn add_to_today(steps: u32) {
    let Some(day) = clock::now().map(|now| now.days_since_epoch()) else {
        return;
    };
    let mut state = state();
    state.today = match state.today {
        Some((d, total)) if d == day => Some((day, total + steps)),
        _ => Some((day, steps)),
    };
}

/// As `activity::record_heart_rate`: a reading in the current minute's
/// sample, if the time is known.
pub fn record_heart_rate(bpm: u8) {
    if let Some(second) = clock::now_utc_seconds() {
        record_heart_rate_at(second.div_euclid(60), bpm);
    }
}

/// Records a heart rate reading in a UTC minute's sample.
pub fn record_heart_rate_at(utc_minute: i64, bpm: u8) {
    state().history.entry(utc_minute).or_default().1 = bpm;
}

/// Records steps in a UTC minute's sample, without touching today's total
/// (for filling in a past day).
pub fn record_steps_at(utc_minute: i64, steps: u16) {
    let mut state = state();
    let sample = state.history.entry(utc_minute).or_default();
    sample.0 = sample.0.saturating_add(steps);
}

/// Shows a dev kit's day from now on, instead of the history here.
#[cfg(not(target_arch = "wasm32"))]
pub fn show_devkit_day(day: Option<crate::day::Day>) {
    state().today_from_devkit = Some(day);
}

pub fn set_last_sleep(sleep: Option<Sleep>) {
    state().last_sleep = sleep;
}

pub fn set_asleep(asleep: bool) {
    state().asleep = asleep;
}

pub fn set_movement(movement: u16) {
    state().movement = movement;
}
