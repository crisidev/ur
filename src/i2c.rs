//! The shared I2C bus: touch controller (0x15), accelerometer (0x18), and
//! heart-rate sensor (0x44), all on TWIM1 at P0.06 (SDA) / P0.07 (SCL).
//!
//! Every transfer on it must have a timeout, and every failed one ends in
//! [`reset_controller`], which also clocks the bus free if a sensor holds
//! it. A transfer that never ends holds the bus's lock for good, and with
//! it every other driver on the bus: on the dev kit (2026-11-10), a boot's
//! first touch transfer (which then had no timeout) never finished, and
//! the accelerometer and heart-rate sensor waited for the bus for as long
//! as the watch ran.

use core::sync::atomic::{AtomicU16, Ordering};

use defmt::{debug, info, warn};
use embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice;
use embassy_nrf::pac::{gpio, twim};
use embassy_nrf::twim::Twim;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::mutex::Mutex;
use embassy_sync::once_lock::OnceLock;
use embassy_time::{Duration, TimeoutError, Timer, with_timeout};

use crate::backoff;

/// One driver's handle on the shared bus.
pub type Device = I2cDevice<'static, CriticalSectionRawMutex, Twim<'static>>;

/// The bus itself, for [`reset_controller`]. Set once at boot.
pub static BUS: OnceLock<&'static Mutex<CriticalSectionRawMutex, Twim<'static>>> = OnceLock::new();

/// The bus's pins, on P0.
const SDA: usize = 6;
const SCL: usize = 7;

/// A transfer that didn't work.
#[derive(defmt::Format)]
pub enum Failure<E> {
    /// The driver reported an error (e.g. a NACK).
    Failed(E),
    /// It didn't finish in time.
    TimedOut,
}

/// A transfer's outcome under [`with_timeout`], with the two ways it can
/// fail as one error, to log. Whatever fails, the caller should log it,
/// then call [`reset_controller`].
///
/// Not an `async fn` that applies the timeout itself: that would keep the
/// transfer's future in its state twice (as the argument, and inside the
/// timeout's future it was moved into), which cost the touch task ~200
/// bytes of static RAM.
pub fn outcome<T, E>(result: Result<Result<T, E>, TimeoutError>) -> Result<T, Failure<E>> {
    match result {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(e)) => Err(Failure::Failed(e)),
        Err(TimeoutError) => Err(Failure::TimedOut),
    }
}

/// Gets the bus working again after a failed transfer: the I2C controller
/// (TWIM1), and a sensor holding SDA low.
///
/// embassy-nrf's TWIM driver can leave the controller wedged, after which
/// every transfer on the bus hangs (we saw exactly this: one stalled write,
/// then even reading a chip ID timed out). On an error it triggers STOP and
/// returns without waiting for the controller to actually stop, and a
/// cancelled transfer (e.g. by a timeout) isn't stopped at all, so the next
/// transfer is set up on a controller that's still busy. Stopping and
/// power-cycling the peripheral clears that; its pin and frequency settings
/// survive being disabled, so the driver carries on as before.
///
/// While the controller is off, the pins are the GPIO's again, so this
/// also clocks out whatever a sensor was part-way through sending (see
/// [`free_bus`]): a transfer cut short can leave one holding SDA low, and
/// a reset of ours (which doesn't reset the sensors) can too, so doing that
/// only at boot isn't enough. On a healthy bus that costs one pin read.
///
/// Holds the bus lock throughout, so no other driver's transfer is caught
/// in the middle. Waits for the lock for a second at most: every transfer
/// has a timeout far shorter, so if the lock is held that long, something
/// is holding it for good, and waiting for it would stop the caller (a
/// sensor's task) for good too. Better to fail its next transfer, and
/// retry.
pub async fn reset_controller() {
    let bus = BUS.get().await;
    let Ok(_exclusive) = with_timeout(Duration::from_secs(1), bus.lock()).await else {
        warn!("i2c: bus busy for 1 s; not reset");
        return;
    };
    let r = embassy_nrf::pac::TWIM1;
    r.tasks_stop().write_value(1);
    // A stop completes within a byte time (~40 µs at 250 kHz) unless the
    // bus is stuck, in which case disabling the peripheral ends it anyway.
    Timer::after_micros(200).await;
    r.enable()
        .write(|w| w.set_enable(twim::vals::Enable::from_bits(0)));
    free_bus().await;
    r.events_stopped().write_value(0);
    r.events_error().write_value(0);
    r.events_suspended().write_value(0);
    r.events_lastrx().write_value(0);
    r.events_lasttx().write_value(0);
    // ERRORSRC bits clear by writing 1.
    r.errorsrc().write_value(twim::regs::Errorsrc(0x7));
    r.enable()
        .write(|w| w.set_enable(twim::vals::Enable::from_bits(6)));
    // The driver whose transfer failed has just said so: this adds nothing
    // to a field log.
    debug!("i2c: reset the controller after a failed transfer");
}

/// Frees the bus if a sensor is stuck mid-transfer, before the controller
/// first takes the pins.
///
/// The sensors aren't reset when the nRF52 is (by a reflash, watchdog, or
/// panic), so one can be left part-way through sending a byte, holding SDA
/// low indefinitely; every later transfer then fails. We saw exactly this on
/// the dev kit.
pub async fn recover_bus() {
    free_bus().await;
}

/// Consecutive [`free_bus`] calls that couldn't free the bus, to keep a
/// bus that stays stuck from filling the log.
static STUCK: AtomicU16 = AtomicU16::new(0);

/// Clocks SCL until a sensor holding SDA low has shifted out the rest of
/// its byte and lets go, then generates a STOP so every device returns to
/// idle: the standard cure for a stuck I2C bus. Only while the pins are
/// the GPIO's: before the controller is set up, or while it's disabled.
///
/// Through the registers, not embassy's pin types, so that it can run
/// after the controller has taken ownership of the pins (from
/// [`reset_controller`]). It puts both pins' configuration back as it
/// found it.
async fn free_bus() {
    let p0 = embassy_nrf::pac::P0;
    let saved = [p0.pin_cnf(SDA).read(), p0.pin_cnf(SCL).read()];
    // Both lines open-drain (drive low, release high), as on a real I2C
    // bus (the board has external pull-ups), and readable throughout:
    // embassy's output-only mode disconnects the input, so SDA always read
    // as low after this, which made the boot log say "SDA still held low"
    // whenever it ran, freed or not. Released before they become outputs,
    // so neither glitches low.
    p0.outset().write(|w| {
        w.set_pin(SDA, true);
        w.set_pin(SCL, true);
    });
    for pin in [SDA, SCL] {
        p0.pin_cnf(pin).write(|w| {
            w.set_dir(gpio::vals::Dir::Output);
            w.set_input(gpio::vals::Input::Connect);
            w.set_pull(gpio::vals::Pull::Disabled);
            w.set_drive(gpio::vals::Drive::S0d1);
            w.set_sense(gpio::vals::Sense::Disabled);
        });
    }
    let high = |pin: usize| p0.in_().read().pin(pin);
    let set = |pin: usize, level: bool| {
        if level {
            p0.outset().write(|w| w.set_pin(pin, true));
        } else {
            p0.outclr().write(|w| w.set_pin(pin, true));
        }
    };
    // 100 kHz-ish half periods (in practice a 30 µs RTC tick).
    let half_period = || Timer::after_micros(5);
    half_period().await;

    if high(SDA) {
        STUCK.store(0, Ordering::Relaxed);
    } else if !high(SCL) {
        // A sensor stretching the clock for good: nothing clocking can do.
        let stuck = STUCK.fetch_add(1, Ordering::Relaxed).saturating_add(1);
        if backoff::worth_logging(stuck) {
            warn!("i2c: SDA and SCL held low ({=u16}x)", stuck);
        }
    } else {
        // A byte plus its ACK is nine clocks, so nine pulses is always enough.
        let mut pulses = 0u8;
        while !high(SDA) && pulses < 9 {
            set(SCL, false);
            half_period().await;
            set(SCL, true);
            half_period().await;
            pulses += 1;
        }
        // STOP: SDA rises while SCL is high.
        set(SCL, false);
        half_period().await;
        set(SDA, false);
        half_period().await;
        set(SCL, true);
        half_period().await;
        set(SDA, true);
        half_period().await;

        if high(SDA) {
            STUCK.store(0, Ordering::Relaxed);
            info!("i2c: freed SDA after {=u8} clocks", pulses);
        } else {
            let stuck = STUCK.fetch_add(1, Ordering::Relaxed).saturating_add(1);
            if backoff::worth_logging(stuck) {
                warn!("i2c: SDA still held low after 9 clocks ({=u16}x)", stuck);
            }
        }
    }

    p0.pin_cnf(SDA).write_value(saved[0]);
    p0.pin_cnf(SCL).write_value(saved[1]);
}

/// For checking recovery on the dev kit (feature `wedge-i2c`): for the
/// first few seconds after boot, keeps switching the I2C controller off
/// behind every driver's back, so their transfers hang, as a wedged
/// controller's do (and one cut off mid-read can leave a sensor holding
/// SDA). Touch, the accelerometer and the heart-rate sensor all set up in
/// that time, so each has to time out, recover the bus, and retry its
/// setup until it works.
#[cfg(feature = "wedge-i2c")]
#[embassy_executor::task]
pub async fn wedge_task() {
    const WEDGED: Duration = Duration::from_secs(5);
    warn!("i2c: wedging the bus for {=u64} s", WEDGED.as_secs());
    let until = embassy_time::Instant::now() + WEDGED;
    while embassy_time::Instant::now() < until {
        embassy_nrf::pac::TWIM1
            .enable()
            .write(|w| w.set_enable(twim::vals::Enable::from_bits(0)));
        Timer::after_millis(20).await;
    }
    info!("i2c: no longer wedging the bus");
}
