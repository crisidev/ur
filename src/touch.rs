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
//!
//! A long press is the exception: it's reported the moment the finger has
//! been down for [`LONG_PRESS`] without moving, while it's still down, so
//! whatever it does (and its buzz) comes then, and the wearer knows when
//! to let go rather than guessing. That touch is then spent: lifting the
//! finger, or moving it, does nothing more. The timer runs from the press,
//! not from reports, though the chip does keep reporting (every ~10 ms)
//! for as long as a finger is down, however still: the field logs have
//! long presses of nearly 10 s, which [`LIFT_TIMEOUT`] would otherwise
//! have cut short. A finger that moves past the tap tolerance
//! ([`SWIPE_DISTANCE`]) before then is on its way to a swipe, and the
//! long press is off for that touch, even if it comes back.
//!
//! When touches can't do anything (the screen is off in the quiet night;
//! see [`set_asleep`]), the chip goes into deep sleep, as InfiniTime puts
//! it to sleep with the screen: it stops scanning the panel, and sends no
//! interrupts for bedding to set off. Only a reset (P0.10) wakes it, which
//! then needs configuring again, as at boot.

use core::sync::atomic::{AtomicBool, Ordering};

use defmt::{Format, debug, info, warn};
use embassy_futures::select::{Either, Either3, select, select3};
use embassy_nrf::gpio::{Input, Output};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Instant, Timer, with_timeout};
use embedded_hal_async::i2c::I2c;

use crate::canvas;
use crate::i2c::Device;

const ADDRESS: u8 = 0x15;
/// First register of the touch report: gesture, finger count, X, Y.
const REG_REPORT: u8 = 0x01;
/// Interrupt control.
const REG_IRQ_CTL: u8 = 0xFA;
/// Power mode; [`DEEP_SLEEP`] is the value InfiniTime writes.
const REG_POWER: u8 = 0xA5;
const DEEP_SLEEP: u8 = 0x03;
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
/// A stationary touch at least this long is a long press, reported as soon
/// as it's been held this long.
const LONG_PRESS: Duration = Duration::from_millis(600);
/// The panel's size in the chip's coordinates (pixels). Reports beyond it
/// are garbage: the first report after the chip wakes from its own sleep
/// sometimes reads e.g. x = 3584, which started phantom swipes.
const PANEL: i32 = 240;
/// While a finger is down, the chip reports every ~10 ms. If reports stop for
/// this long without a lift report, assume we missed the lift.
const LIFT_TIMEOUT: Duration = Duration::from_millis(250);

/// A simulated finger's reports, from [`crate::debug`] (for testing long
/// presses without a finger): where it is, in panel pixels, or `None` once
/// it's lifted. [`touch_task`] treats each like a report from the chip.
pub static SIMULATED: Signal<CriticalSectionRawMutex, Option<(u8, u8)>> = Signal::new();

/// Whether the chip should be asleep, as last asked (see [`set_asleep`]).
static ASLEEP: AtomicBool = AtomicBool::new(false);
/// A change to [`ASLEEP`], for the task.
static ASLEEP_CHANGED: Signal<CriticalSectionRawMutex, ()> = Signal::new();

/// Puts the chip into deep sleep, or wakes it. The UI loop calls this every
/// pass; only changes do anything.
pub fn set_asleep(asleep: bool) {
    if ASLEEP.swap(asleep, Ordering::Relaxed) != asleep {
        ASLEEP_CHANGED.signal(());
    }
}

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
    /// How long the finger was down, in milliseconds (saturating). Not a
    /// `Duration`: its 8 bytes, 8-aligned, grew every queued input event,
    /// and static RAM has no room to spare.
    pub held_ms: u16,
}

/// The touch in progress: where and when it started, and the latest position.
struct Tracking {
    start: (i32, i32),
    last: (i32, i32),
    since: Instant,
    /// It went beyond the tap tolerance ([`SWIPE_DISTANCE`]) at some point,
    /// so it can't become a long press.
    moved: bool,
    /// It was reported as a long press already, while still down.
    held: bool,
}

impl Tracking {
    fn new(at: (i32, i32)) -> Self {
        Self {
            start: at,
            last: at,
            since: Instant::now(),
            moved: false,
            held: false,
        }
    }

    /// The finger is now at `at`.
    fn moved_to(&mut self, at: (i32, i32)) {
        self.last = at;
        let (dx, dy) = (at.0 - self.start.0, at.1 - self.start.1);
        self.moved |= dx.abs().max(dy.abs()) >= SWIPE_DISTANCE;
    }

    /// When this touch becomes a long press, if it still can.
    fn long_press_at(&self) -> Option<Instant> {
        (!self.moved && !self.held).then_some(self.since + LONG_PRESS)
    }

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
        } else if !self.moved && self.since.elapsed() >= LONG_PRESS {
            // Usually reported while held (see `touch_task`); this is for a
            // lift that beat the timer.
            Gesture::LongPress
        } else {
            Gesture::Tap
        };
        let scale = canvas::SCALE as i32;
        Touch {
            gesture,
            x: self.start.0 / scale,
            y: self.start.1 / scale,
            held_ms: self.since.elapsed().as_millis().min(u64::from(u16::MAX)) as u16,
        }
    }
}

/// Puts the controller in deep sleep for System OFF (see
/// [`crate::system_off`]), through the bus `system_off` holds. Between
/// touches it sleeps by itself (6–10 µA), but then ignores I2C, so it's
/// reset first, as [`touch_task`] does at boot, to catch it listening.
/// Deep sleep ([`DEEP_SLEEP`], a µA or two) only ends at a reset, which the
/// next boot's `touch_task` does. (The quiet night's sleep, in
/// `touch_task`, goes through the task's shared-bus device instead: this
/// runs after the tasks have stopped, on the bus `system_off` took over.)
pub async fn deep_sleep(twim: &mut embassy_nrf::twim::Twim<'static>) {
    // The reset line (P0.10), `touch_task`'s, which won't run again.
    let p0 = embassy_nrf::pac::P0;
    p0.outclr().write(|w| w.set_pin(10, true));
    Timer::after_millis(5).await;
    p0.outset().write(|w| w.set_pin(10, true));
    Timer::after_millis(50).await;
    let sleep = twim.write(ADDRESS, &[REG_POWER, DEEP_SLEEP]);
    if !matches!(
        with_timeout(Duration::from_millis(50), sleep).await,
        Ok(Ok(()))
    ) {
        warn!("touch: deep sleep failed");
    }
}

#[embassy_executor::task]
pub async fn touch_task(
    mut i2c: Device,
    mut interrupt: Input<'static>,
    mut reset: Output<'static>,
) {
    reset_chip(&mut reset).await;
    // The chip only listens for a short while after reset. If we miss that
    // window (e.g. the executor was busy), retry on the next interrupt, when
    // the chip is awake again. Until configured, touches still wake the
    // screen, but lifts go unreported, so gestures are unreliable.
    let mut configured = configure(&mut i2c).await;

    let mut tracking: Option<Tracking> = None;
    loop {
        // While a touch is in progress, also wake up when it's been held
        // long enough to be a long press, and if reports dry up, so a
        // missed lift can't leave us stuck mid-touch. Whether it's the
        // long press that's due.
        let deadline = async {
            let Some(touch) = &tracking else {
                return core::future::pending().await;
            };
            let lift = Instant::now() + LIFT_TIMEOUT;
            match touch.long_press_at() {
                Some(at) if at < lift => {
                    Timer::at(at).await;
                    true
                }
                _ => {
                    Timer::at(lift).await;
                    false
                }
            }
        };
        let report = select(interrupt.wait_for_falling_edge(), SIMULATED.wait());
        let simulated = match select3(report, deadline, ASLEEP_CHANGED.wait()).await {
            Either3::First(Either::First(())) => None,
            Either3::First(Either::Second(finger)) => Some(finger),
            Either3::Second(true) => {
                let touch = tracking.as_mut().expect("only due while tracking");
                touch.held = true;
                let event = touch.classify();
                info!(
                    "touch: {} at ({},{}), held",
                    event.gesture, touch.start.0, touch.start.1
                );
                crate::INPUT.send(crate::InputEvent::Touch(event)).await;
                continue;
            }
            Either3::Second(false) => {
                let touch = tracking.take().expect("only times out while tracking");
                finish(&touch).await;
                continue;
            }
            Either3::Third(()) => {
                if ASLEEP.load(Ordering::Relaxed) {
                    // Whatever was in progress is moot: the screen is off.
                    tracking = None;
                    sleep(&mut i2c, &mut reset).await;
                    // Nothing to do until it's time to wake.
                    while ASLEEP.load(Ordering::Relaxed) {
                        ASLEEP_CHANGED.wait().await;
                    }
                    reset_chip(&mut reset).await;
                    configured = configure(&mut i2c).await;
                    info!("touch: awake");
                }
                continue;
            }
        };

        let (lifted, x, y) = match simulated {
            Some(Some((x, y))) => (false, i32::from(x), i32::from(y)),
            // Lifted where it last was.
            Some(None) => (true, 0, 0),
            None => {
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
                // Top two bits of the X high byte: 0 = press, 1 = lift, 2 =
                // contact.
                let lifted = x_hi >> 6 == 1 || fingers == 0;
                let x = (i32::from(x_hi & 0x0F) << 8) | i32::from(x_lo);
                let y = (i32::from(y_hi & 0x0F) << 8) | i32::from(y_lo);
                debug!("touch: fingers={}", fingers);
                (lifted, x, y)
            }
        };
        debug!("touch: lifted={} x={} y={}", lifted, x, y);

        // A position off the panel is a misread (see `PANEL`): it can still
        // end a touch, but never starts or moves one.
        let on_panel = (0..PANEL).contains(&x) && (0..PANEL).contains(&y);
        if !on_panel {
            debug!("touch: ignoring an off-panel position");
        }

        match (&mut tracking, lifted) {
            (None, false) if !on_panel => {}
            (None, false) => tracking = Some(Tracking::new((x, y))),
            (Some(touch), false) => {
                if on_panel {
                    touch.moved_to((x, y));
                }
            }
            (Some(touch), true) => {
                // Lift reports usually carry the final position, but some
                // report (0, 0); don't let that look like a long swipe.
                if on_panel && (x, y) != (0, 0) {
                    touch.moved_to((x, y));
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

/// Resets the chip, which wakes it (even from deep sleep) and leaves it
/// listening on I2C for a while.
async fn reset_chip(reset: &mut Output<'static>) {
    reset.set_low();
    Timer::after_millis(5).await;
    reset.set_high();
    Timer::after_millis(50).await;
}

/// Puts the chip into deep sleep.
///
/// In its usual standby between touches it ignores I2C (see the module
/// docs), so this resets it first, as InfiniTime does. If the write fails
/// anyway, the chip just stays awake, costing a few µA and stray
/// interrupts, which [`touch_task`] ignores until it's woken; and the I2C
/// controller is reset, as after any failed transfer, so the bus the
/// accelerometer and heart rate sensor share can't be left wedged.
async fn sleep(i2c: &mut Device, reset: &mut Output<'static>) {
    reset_chip(reset).await;
    match i2c.write(ADDRESS, &[REG_POWER, DEEP_SLEEP]).await {
        Ok(()) => info!("touch: asleep"),
        Err(e) => {
            warn!("touch: sleeping failed: {}", e);
            crate::i2c::reset_controller().await;
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
    // A long press was reported while held, and that was the touch's one
    // event, whatever the finger did after.
    if touch.held {
        info!(
            "touch: lifted after {}ms",
            touch.since.elapsed().as_millis()
        );
        return;
    }
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
