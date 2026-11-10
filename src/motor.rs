//! The vibration motor on P0.16 (active low).
//!
//! Other code asks for a [`Pattern`] via [`buzz`]; a newer request always
//! replaces whatever is playing, so e.g. an incoming call can cut short a
//! notification buzz, and [`Pattern::STOP`] silences anything.
//!
//! The motor is either on or off: it hangs off a plain GPIO, through a
//! transistor. How strong a buzz feels comes from how long it's on (it
//! takes tens of milliseconds to spin up, so a short pulse is a faint
//! tick) and how often. InfiniTime drives it the same way. (The PWM
//! peripheral could run it at part duty, as wasp-os does, but pulse
//! lengths alone give a gentle start, without tying up a peripheral.)

#[cfg(target_os = "none")]
use embassy_futures::select::{Either, select};
#[cfg(target_os = "none")]
use embassy_nrf::gpio::Output;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embassy_time::Duration;
#[cfg(target_os = "none")]
use embassy_time::Timer;

#[derive(Clone, Copy)]
#[cfg_attr(not(target_os = "none"), derive(PartialEq, Eq))]
pub struct Pattern {
    /// Alternating on/off durations in milliseconds, starting with "on".
    steps: &'static [u16],
    /// Where to loop back to once the steps run out, until replaced (calls,
    /// alarms): an even index, so an "on". `None` plays once.
    repeat_from: Option<u8>,
}

impl Pattern {
    /// A pattern, checked: every pattern below is a constant made by
    /// this, so one that `motor_task` would play wrongly fails the build
    /// rather than the wearer.
    const fn new(steps: &'static [u16], repeat_from: Option<u8>) -> Self {
        if let Some(start) = repeat_from {
            let start = start as usize;
            // The loop starts on an "on" that's there, and has an "off"
            // for every "on": looping back from an "on" to an "on" would
            // run the two together, with the motor on for both.
            assert!(start.is_multiple_of(2) && start < steps.len());
            assert!((steps.len() - start).is_multiple_of(2));
            // And it takes time: one of no length would have `motor_task`
            // go round it without ever waiting, and no other task would
            // run again (until the watchdog's reset).
            let mut ms = 0;
            let mut i = start;
            while i < steps.len() {
                ms += steps[i] as u32;
                i += 1;
            }
            assert!(ms > 0);
        }
        Self { steps, repeat_from }
    }

    pub const STOP: Self = Self::new(&[], None);
    /// A notification: noticeable, but brief.
    pub const NOTIFY: Self = Self::new(&[60], None);
    /// The next turn is near (see [`crate::route`]): two taps, so it isn't
    /// taken for a notification.
    pub const TURN: Self = Self::new(&[60, 120, 60], None);
    /// Incoming call: rings until stopped.
    pub const RING: Self = Self::new(&[400, 600], Some(0));
    /// Timers: two quick pulses and a pause, so it's not mistaken for a
    /// call; until stopped. At full strength from the start: a timer is
    /// set by someone awake, who wants to know it's done (the tea, the
    /// oven), perhaps across a noisy kitchen.
    pub const TIMER: Self = Self::new(&FULL, Some(0));
    /// Alarms (snoozes too): from faint, sparse ticks to [`Pattern::TIMER`]'s
    /// full strength over [`ALARM_RAMP`], then that until stopped. Being
    /// shaken awake is unpleasant, and a gentle start is often enough to
    /// wake from light sleep.
    pub const ALARM: Self = Self::new(&ALARM_STEPS, Some(RAMP_LEN as u8));
    /// "Find my watch": as loud as the motor gets, long pulses with short
    /// breaks, to rattle whatever the watch lies on, for someone looking
    /// for it rather than wearing it. (At 80 % on, it costs a few mAh over
    /// [`crate::ui`]'s two minutes of it, a percent or two of the battery.)
    pub const FIND: Self = Self::new(&[800, 200], Some(0));
}

/// Two quick pulses and a pause.
const FULL: [u16; 4] = [200, 120, 200, 800];

/// The alarm's build-up, in five phases of about 12 s, each a short
/// on/off pattern repeated. Pulses grow longer and the pauses shorter;
/// the last phases are already the full pattern's pairs, a little
/// shorter. About a minute in all.
///
/// TODO: tune on the wrist. The first ticks are meant to be only just
/// felt; 30 ms may turn out too faint for this motor, or not faint enough.
const RAMP: [(&[u16], usize); 5] = [
    // 12 s: a faint tick every 3 s.
    (&[30, 2970], 4),
    // 12.5 s: a firmer tick every 2.5 s.
    (&[50, 2450], 5),
    // 12 s: short buzzes every 2 s.
    (&[80, 1920], 6),
    // 11.9 s: pairs, still short and spaced out.
    (&[120, 150, 120, 1100], 8),
    // 10.8 s: pairs almost at full strength.
    (&[160, 130, 160, 900], 8),
];

/// How many steps the ramp takes.
const RAMP_LEN: usize = {
    let mut len = 0;
    let mut i = 0;
    while i < RAMP.len() {
        len += RAMP[i].0.len() * RAMP[i].1;
        i += 1;
    }
    len
};

/// The ramp's steps, then [`FULL`]'s to loop on.
const ALARM_STEPS: [u16; RAMP_LEN + FULL.len()] = {
    let mut steps = [0; RAMP_LEN + FULL.len()];
    let mut at = 0;
    let mut phase = 0;
    while phase < RAMP.len() {
        let (pattern, times) = RAMP[phase];
        let mut time = 0;
        while time < times {
            let mut i = 0;
            while i < pattern.len() {
                steps[at] = pattern[i];
                at += 1;
                i += 1;
            }
            time += 1;
        }
        phase += 1;
    }
    let mut i = 0;
    while i < FULL.len() {
        steps[at] = FULL[i];
        at += 1;
        i += 1;
    }
    steps
};

// The loop must start on an "on", at an index `repeat_from` can hold.
const _: () = assert!(RAMP_LEN.is_multiple_of(2) && RAMP_LEN <= u8::MAX as usize);

/// How long an alarm takes to build up to full strength (about a
/// minute), for anything else that should build up alongside it (the
/// backlight, say).
#[allow(dead_code, reason = "for the backlight, which doesn't build up yet")]
pub const ALARM_RAMP: Duration = {
    let mut ms = 0;
    let mut i = 0;
    while i < RAMP_LEN {
        ms += ALARM_STEPS[i] as u64;
        i += 1;
    }
    Duration::from_millis(ms)
};

static REQUEST: Signal<CriticalSectionRawMutex, Pattern> = Signal::new();

pub fn buzz(pattern: Pattern) {
    REQUEST.signal(pattern);
}

// The simulator (`sim/`) has no motor: it plays requests back on screen
// instead of through `motor_task`, which needs the hardware.

/// The pattern last asked for, if there's been one since the last call.
#[cfg(not(target_os = "none"))]
pub fn take_request() -> Option<Pattern> {
    REQUEST.try_take()
}

#[cfg(not(target_os = "none"))]
impl Pattern {
    /// Its on/off durations in milliseconds, starting with "on".
    pub fn steps(&self) -> &'static [u16] {
        self.steps
    }

    /// Where it loops back to once the steps run out, if it repeats.
    pub fn repeat_from(&self) -> Option<usize> {
        self.repeat_from.map(usize::from)
    }
}

#[cfg(target_os = "none")]
#[embassy_executor::task]
pub async fn motor_task(mut motor: Output<'static>) {
    let mut pattern = REQUEST.wait().await;
    loop {
        // Play `pattern`; if a new request arrives mid-way, switch to it.
        let mut from = 0;
        let interrupted = 'play: loop {
            for (i, &ms) in pattern.steps.iter().enumerate().skip(from) {
                if i % 2 == 0 {
                    motor.set_low();
                } else {
                    motor.set_high();
                }
                if let Either::Second(next) =
                    select(Timer::after_millis(ms.into()), REQUEST.wait()).await
                {
                    break 'play Some(next);
                }
            }
            match pattern.repeat_from {
                Some(start) if usize::from(start) < pattern.steps.len() => {
                    from = usize::from(start)
                }
                _ => break None,
            }
        };
        motor.set_high();
        pattern = match interrupted {
            Some(next) => next,
            None => REQUEST.wait().await,
        };
    }
}
