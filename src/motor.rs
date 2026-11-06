//! The vibration motor on P0.16 (active low).
//!
//! Other code asks for a [`Pattern`] via [`buzz`]; a newer request always
//! replaces whatever is playing, so e.g. an incoming call can cut short a
//! notification buzz, and [`Pattern::STOP`] silences anything.

use embassy_futures::select::{Either, select};
use embassy_nrf::gpio::Output;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embassy_time::Timer;

#[derive(Clone, Copy)]
pub struct Pattern {
    /// Alternating on/off durations in milliseconds, starting with "on".
    steps: &'static [u16],
    /// Loop until replaced (calls, alarms) rather than playing once.
    repeat: bool,
}

impl Pattern {
    #[allow(dead_code)] // Used once calls can be dismissed (milestone 3).
    pub const STOP: Self = Self {
        steps: &[],
        repeat: false,
    };
    /// A notification: noticeable, but brief.
    pub const NOTIFY: Self = Self {
        steps: &[60],
        repeat: false,
    };
    /// Incoming call: rings until stopped.
    #[allow(dead_code)] // Used once notifications land (milestone 3).
    pub const RING: Self = Self {
        steps: &[400, 600],
        repeat: true,
    };
}

static REQUEST: Signal<CriticalSectionRawMutex, Pattern> = Signal::new();

pub fn buzz(pattern: Pattern) {
    REQUEST.signal(pattern);
}

#[embassy_executor::task]
pub async fn motor_task(mut motor: Output<'static>) {
    let mut pattern = REQUEST.wait().await;
    loop {
        // Play `pattern`; if a new request arrives mid-way, switch to it.
        let interrupted = 'play: loop {
            for (i, &ms) in pattern.steps.iter().enumerate() {
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
            if !pattern.repeat || pattern.steps.is_empty() {
                break None;
            }
        };
        motor.set_high();
        pattern = match interrupted {
            Some(next) => next,
            None => REQUEST.wait().await,
        };
    }
}
