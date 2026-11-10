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
//! What each touch was is [`crate::gesture`]'s call. A long press is
//! the exception to waiting for the lift: it's reported the moment the
//! finger has been down for [`LONG_PRESS`] without moving, while it's
//! still down. The timer runs from the press, not from reports, though
//! the chip does keep reporting (every ~10 ms) for as long as a finger is
//! down, however still: the field logs have long presses of nearly 10 s,
//! which [`LIFT_TIMEOUT`] would otherwise have cut short.
//!
//! Nearly 10 s, and no longer, perhaps because the chip resets itself
//! after a long press of its `LongPressTime`, 10 s by default (per
//! Hynitron's register declaration), which ends the touch: reports stop,
//! and [`LIFT_TIMEOUT`] sends its release. So [`configure`] raises that
//! to [`HOLD_LIMIT`], for a grid hold's release to come at the lift.
//! That seems to take: on the dev kit (2026-11-10, a real finger), holds
//! of 13.3 s, 42 s and 54 s all kept the screen on and released at the
//! lift, none at 10 s.
//!
//! But the chip doesn't keep a long, still finger for ever on its own:
//! the 42 s and 54 s holds were meant to last ~70 s, and it reported a
//! lift with the finger still down, likely its baseline tracking taking
//! the finger in (a cold one, that day). And one 9.2 s hold "lifted",
//! then pressed again 20 ms later, with no lift the wearer remembers: a
//! second `hold_press` mid-hold. So [`HOLD_LIMIT`] wasn't reached there.
//! TODO: find what drops the finger (`AutoReset`, 0xFB? the baseline's
//! update rate?), and whether a re-press that soon after a lift, on the
//! same spot, should continue the hold rather than start another.
//!
//! [`LONG_PRESS`]: crate::gesture::LONG_PRESS
//! [`HOLD_LIMIT`]: crate::gesture::HOLD_LIMIT
//!
//! When touches can't do anything (the screen is off in the quiet night;
//! see [`set_asleep`]), the chip goes into deep sleep, as InfiniTime puts
//! it to sleep with the screen: it stops scanning the panel, and sends no
//! interrupts for bedding to set off. Only a reset (P0.10) wakes it, which
//! then needs configuring again, as at boot. A touch in progress then is
//! dropped, bar a long press's release, which goes then rather than at a
//! lift that will never be seen (see [`crate::gesture::Tracking::lost`]).
//! The screen, and so the chip, stays on for a long press begun on the
//! awake screen until its release, but only up to [`HOLD_LIMIT`] (see
//! [`crate::gesture::screen_timeout`]): a sleeve or bedding resting on
//! it mustn't keep it lit all night. Past that, in the quiet night, the
//! screen goes off, the chip to sleep, and the release goes then.

use core::sync::atomic::{AtomicBool, Ordering};

use defmt::{debug, info, warn};
use embassy_futures::select::{Either, Either3, select, select3};
use embassy_nrf::gpio::{Input, Output};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Instant, Timer, with_timeout};
use embedded_hal_async::i2c::I2c;

use crate::gesture::{HOLD_LIMIT, Tracking};
use crate::i2c::Device;

pub use crate::gesture::{Gesture, Touch};

pub(crate) const ADDRESS: u8 = 0x15;
/// First register of the touch report: gesture, finger count, X, Y.
const REG_REPORT: u8 = 0x01;
/// Interrupt control.
const REG_IRQ_CTL: u8 = 0xFA;
/// How long, in seconds, a touch may last before the chip resets itself
/// (`LongPressTime`; 0 would never). See the module docs.
const REG_LONG_PRESS_TIME: u8 = 0xFC;
/// Power mode; [`DEEP_SLEEP`] is the value InfiniTime writes.
const REG_POWER: u8 = 0xA5;
const DEEP_SLEEP: u8 = 0x03;
/// Interrupt on: every periodic report while touched (`EnTouch`, bit 6),
/// touch state changes, i.e. press and lift (`EnChange`, bit 5), and chip
/// gestures (`EnMotion`, bit 4). Without `EnChange`, we never hear about the
/// finger lifting, so we can't tell where a touch ended. This is the value
/// InfiniTime uses.
const IRQ_CTL: u8 = 0b0111_0000;

/// The panel's size in the chip's coordinates (pixels). Reports beyond it
/// are garbage: the first report after the chip wakes from its own sleep
/// sometimes reads e.g. x = 3584, which started phantom swipes.
const PANEL: i32 = 240;
/// While a finger is down, the chip reports every ~10 ms. If reports stop for
/// this long without a lift report, assume we missed the lift.
const LIFT_TIMEOUT: Duration = Duration::from_millis(250);
/// Far longer than any transfer here takes (a 6-byte report at 250 kHz:
/// ~0.4 ms). Every transfer has one: one that never ended once held the
/// shared bus, and with it the accelerometer and heart-rate sensor, for as
/// long as the watch ran (see [`crate::i2c`]).
const I2C_TIMEOUT: Duration = Duration::from_millis(50);

/// A simulated finger's reports, from [`crate::debug`] (for testing long
/// presses without a finger; only with `debug-input`): where it is, in
/// panel pixels, or `None` once it's lifted. [`touch_task`] treats each
/// like a report from the chip.
#[cfg(feature = "debug-input")]
pub static SIMULATED: Signal<CriticalSectionRawMutex, Option<(u8, u8)>> = Signal::new();

/// Whether a finger is down, for the button grid: a wait for a second
/// button can't run out while one is (see [`crate::buttons::Grid::expire`]).
static DOWN: AtomicBool = AtomicBool::new(false);

/// Whether a finger is on the screen right now (a touch begun, and not yet
/// over).
pub fn finger_down() -> bool {
    DOWN.load(Ordering::Relaxed)
}

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
    // Tried a few times, with the bus recovered between tries (see
    // `i2c::write_held`): all within the ~0.4 s the chip listens after
    // its reset.
    if !crate::i2c::write_held(twim, ADDRESS, &[REG_POWER, DEEP_SLEEP], I2C_TIMEOUT).await {
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
        // Every change to `tracking` comes back here before awaiting.
        DOWN.store(tracking.is_some(), Ordering::Relaxed);
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
        #[cfg(feature = "debug-input")]
        let simulated = SIMULATED.wait();
        // Without `debug-input`, no finger but a real one.
        #[cfg(not(feature = "debug-input"))]
        let simulated = core::future::pending::<Option<(u8, u8)>>();
        let report = select(interrupt.wait_for_falling_edge(), simulated);
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
                    // Bar a long press's release, as its lift won't be
                    // seen now: a button grid hold held past the screen's
                    // timeout ends here, rather than never.
                    if let Some(release) = tracking.take().and_then(|touch| touch.lost()) {
                        info!("touch: cut short after {}ms", release.held_ms);
                        crate::INPUT.send(crate::InputEvent::Touch(release)).await;
                    }
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
                let read = with_timeout(
                    I2C_TIMEOUT,
                    i2c.write_read(ADDRESS, &[REG_REPORT], &mut report),
                )
                .await;
                if let Err(e) = crate::i2c::outcome(read) {
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
    let write = with_timeout(I2C_TIMEOUT, i2c.write(ADDRESS, &[REG_POWER, DEEP_SLEEP])).await;
    match crate::i2c::outcome(write) {
        Ok(()) => info!("touch: asleep"),
        Err(e) => {
            warn!("touch: sleeping failed: {}", e);
            crate::i2c::reset_controller().await;
        }
    }
}

/// Sets up the chip's interrupt behaviour; returns whether that worked.
async fn configure(i2c: &mut Device) -> bool {
    let write = with_timeout(I2C_TIMEOUT, i2c.write(ADDRESS, &[REG_IRQ_CTL, IRQ_CTL])).await;
    match crate::i2c::outcome(write) {
        Ok(()) => {
            // Not "never": a stuck contact (water, say) should still end,
            // as the chip's reset does. If this fails, holds end at the
            // chip's default instead, as they did, with their release.
            // With a timeout and a reset like every transfer here: one
            // that never ended would hold the shared bus for good.
            let limit = HOLD_LIMIT.as_secs() as u8;
            let write = with_timeout(
                I2C_TIMEOUT,
                i2c.write(ADDRESS, &[REG_LONG_PRESS_TIME, limit]),
            )
            .await;
            if let Err(e) = crate::i2c::outcome(write) {
                warn!("touch: setting the long press time failed: {}", e);
                crate::i2c::reset_controller().await;
            }
            true
        }
        Err(e) => {
            warn!("touch: configuring interrupts failed: {}", e);
            // As after any failed transfer: a timed-out one leaves the
            // controller busy, and the bus maybe held.
            crate::i2c::reset_controller().await;
            false
        }
    }
}

async fn finish(touch: &Tracking) {
    // Over, as of now: the UI may act on the event before the loop comes
    // round to say so.
    DOWN.store(false, Ordering::Relaxed);
    let (event, release) = touch.lifted();
    // A long press was reported while held, and that was the touch's one
    // event, whatever the finger did after: its lift is only a release.
    if touch.held {
        info!("touch: lifted after {}ms", event.held_ms);
    } else {
        info!(
            "touch: {} from ({},{}) to ({},{}) in {}ms",
            event.gesture, touch.start.0, touch.start.1, touch.last.0, touch.last.1, event.held_ms
        );
    }
    crate::INPUT.send(crate::InputEvent::Touch(event)).await;
    // A long press only seen now, its lift having beaten the timer: its
    // release follows at once (see `Tracking::lifted`).
    if let Some(release) = release {
        crate::INPUT.send(crate::InputEvent::Touch(release)).await;
    }
}
