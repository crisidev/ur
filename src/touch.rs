//! CST816S touch controller.
//!
//! The chip pulses its interrupt line (P0.28) when it has a new report.
//! Between touches it sleeps and **ignores I2C entirely** (we measured it
//! NACKing ~0.4 s after reset), so it's only ever read in response to an
//! interrupt, never polled, and must be configured right after reset while
//! it's still awake.
//!
//! The chip has its own gesture recognition, but in practice swipes were hard
//! to trigger with it, and its thresholds aren't adjustable. So we ask it for
//! raw reports throughout each touch and classify the touch ourselves from
//! where it started and ended, and how long it lasted.

use defmt::{Format, debug, info, warn};
use embassy_futures::select::{Either, select};
use embassy_nrf::gpio::{Input, Output};
use embassy_time::{Duration, Instant, Timer};
use embedded_hal_async::i2c::I2c;

use crate::canvas;
use crate::i2c::Device;

const ADDRESS: u8 = 0x15;
/// First register of the touch report: gesture, finger count, X, Y.
const REG_REPORT: u8 = 0x01;
/// Interrupt control.
const REG_IRQ_CTL: u8 = 0xFA;
/// Interrupt on: every periodic report while touched (`EnTouch`, bit 6),
/// touch state changes, i.e. press and lift (`EnChange`, bit 5), and chip
/// gestures (`EnMotion`, bit 4). Without `EnChange`, we never hear about the
/// finger lifting, so we can't tell where a touch ended. This is the value
/// InfiniTime uses.
const IRQ_CTL: u8 = 0b0111_0000;

/// Minimum travel, in panel pixels, for a touch to be a swipe rather than a
/// tap. From logged touches on the dev kit: taps drift at most ~11 px, while
/// lazy swipes can be as short as ~24 px.
const SWIPE_DISTANCE: i32 = 20;
/// A stationary touch at least this long is a long press.
const LONG_PRESS: Duration = Duration::from_millis(600);
/// While a finger is down, the chip reports every ~10 ms. If reports stop for
/// this long without a lift report, assume we missed the lift.
const LIFT_TIMEOUT: Duration = Duration::from_millis(250);

#[derive(Clone, Copy, PartialEq, Eq, Format)]
pub enum Gesture {
    Tap,
    LongPress,
    SwipeUp,
    SwipeDown,
    SwipeLeft,
    SwipeRight,
}

#[derive(Clone, Copy, Format)]
pub struct Touch {
    pub gesture: Gesture,
    /// Where the touch started, in canvas pixels.
    pub x: i32,
    pub y: i32,
}

/// The touch in progress: where and when it started, and the latest position.
struct Tracking {
    start: (i32, i32),
    last: (i32, i32),
    since: Instant,
}

impl Tracking {
    fn classify(&self) -> Touch {
        let dx = self.last.0 - self.start.0;
        let dy = self.last.1 - self.start.1;
        let gesture = if dx.abs().max(dy.abs()) >= SWIPE_DISTANCE {
            // Named for the direction the finger moved.
            match (dx.abs() > dy.abs(), dx < 0, dy < 0) {
                (true, true, _) => Gesture::SwipeLeft,
                (true, false, _) => Gesture::SwipeRight,
                (false, _, true) => Gesture::SwipeUp,
                (false, _, false) => Gesture::SwipeDown,
            }
        } else if self.since.elapsed() >= LONG_PRESS {
            Gesture::LongPress
        } else {
            Gesture::Tap
        };
        let scale = canvas::SCALE as i32;
        Touch {
            gesture,
            x: self.start.0 / scale,
            y: self.start.1 / scale,
        }
    }
}

#[embassy_executor::task]
pub async fn touch_task(
    mut i2c: Device,
    mut interrupt: Input<'static>,
    mut reset: Output<'static>,
) {
    reset.set_low();
    Timer::after_millis(5).await;
    reset.set_high();
    Timer::after_millis(50).await;
    // The chip only listens for a short while after reset. If we miss that
    // window (e.g. the executor was busy), retry on the next interrupt, when
    // the chip is awake again. Until configured, touches still wake the
    // screen, but lifts go unreported, so gestures are unreliable.
    let mut configured = configure(&mut i2c).await;

    let mut tracking: Option<Tracking> = None;
    loop {
        // While a touch is in progress, also wake up if reports dry up, so a
        // missed lift can't leave us stuck mid-touch.
        if tracking.is_some() {
            if let Either::Second(()) = select(
                interrupt.wait_for_falling_edge(),
                Timer::after(LIFT_TIMEOUT),
            )
            .await
            {
                let touch = tracking.take().expect("checked is_some above");
                finish(&touch).await;
                continue;
            }
        } else {
            interrupt.wait_for_falling_edge().await;
        }

        if !configured {
            configured = configure(&mut i2c).await;
        }

        let mut report = [0u8; 6];
        if let Err(e) = i2c.write_read(ADDRESS, &[REG_REPORT], &mut report).await {
            warn!("touch: read failed: {}", e);
            crate::i2c::reset_controller().await;
            continue;
        }
        let [_chip_gesture, fingers, x_hi, x_lo, y_hi, y_lo] = report;
        // Top two bits of the X high byte: 0 = press, 1 = lift, 2 = contact.
        let lifted = x_hi >> 6 == 1 || fingers == 0;
        let x = (i32::from(x_hi & 0x0F) << 8) | i32::from(x_lo);
        let y = (i32::from(y_hi & 0x0F) << 8) | i32::from(y_lo);
        debug!(
            "touch: fingers={} lifted={} x={} y={}",
            fingers, lifted, x, y
        );

        match (&mut tracking, lifted) {
            (None, false) => {
                tracking = Some(Tracking {
                    start: (x, y),
                    last: (x, y),
                    since: Instant::now(),
                });
            }
            (Some(touch), false) => touch.last = (x, y),
            (Some(touch), true) => {
                // Lift reports usually carry the final position, but some
                // report (0, 0); don't let that look like a long swipe.
                if (x, y) != (0, 0) {
                    touch.last = (x, y);
                }
                finish(touch).await;
                tracking = None;
            }
            // A lift without a press we saw. The chip sends one more report
            // ~10 ms after each lift (its own gesture detection firing);
            // treating it as a touch produced a phantom second tap.
            (None, true) => {}
        }
    }
}

/// Sets up the chip's interrupt behaviour; returns whether that worked.
async fn configure(i2c: &mut Device) -> bool {
    match i2c.write(ADDRESS, &[REG_IRQ_CTL, IRQ_CTL]).await {
        Ok(()) => true,
        Err(e) => {
            warn!("touch: configuring interrupts failed: {}", e);
            false
        }
    }
}

async fn finish(touch: &Tracking) {
    let event = touch.classify();
    info!(
        "touch: {} from ({},{}) to ({},{}) in {}ms",
        event.gesture,
        touch.start.0,
        touch.start.1,
        touch.last.0,
        touch.last.1,
        touch.since.elapsed().as_millis()
    );
    crate::INPUT.send(crate::InputEvent::Touch(event)).await;
}
