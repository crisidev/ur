//! The shared I2C bus: touch controller (0x15), accelerometer (0x18), and
//! heart-rate sensor (0x44), all on TWIM1 at P0.06 (SDA) / P0.07 (SCL).

use defmt::{info, warn};
use embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice;
use embassy_nrf::Peri;
use embassy_nrf::gpio::{AnyPin, Flex, Level, Output, OutputDrive, Pull};
use embassy_nrf::pac::twim;
use embassy_nrf::twim::Twim;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::mutex::Mutex;
use embassy_sync::once_lock::OnceLock;
use embassy_time::Timer;

/// One driver's handle on the shared bus.
pub type Device = I2cDevice<'static, CriticalSectionRawMutex, Twim<'static>>;

/// The bus itself, for [`reset_controller`]. Set once at boot.
pub static BUS: OnceLock<&'static Mutex<CriticalSectionRawMutex, Twim<'static>>> = OnceLock::new();

/// Gets the I2C controller (TWIM1) working again after a failed transfer.
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
/// Holds the bus lock throughout, so no other driver's transfer is caught
/// in the middle.
pub async fn reset_controller() {
    let bus = BUS.get().await;
    let _exclusive = bus.lock().await;
    let r = embassy_nrf::pac::TWIM1;
    r.tasks_stop().write_value(1);
    // A stop completes within a byte time (~40 µs at 250 kHz) unless the
    // bus is stuck, in which case disabling the peripheral ends it anyway.
    Timer::after_micros(200).await;
    r.enable()
        .write(|w| w.set_enable(twim::vals::Enable::from_bits(0)));
    r.events_stopped().write_value(0);
    r.events_error().write_value(0);
    r.events_suspended().write_value(0);
    r.events_lastrx().write_value(0);
    r.events_lasttx().write_value(0);
    // ERRORSRC bits clear by writing 1.
    r.errorsrc().write_value(twim::regs::Errorsrc(0x7));
    r.enable()
        .write(|w| w.set_enable(twim::vals::Enable::from_bits(6)));
    warn!("i2c: reset the controller after a failed transfer");
}

/// Frees the bus if a sensor is stuck mid-transfer.
///
/// The sensors aren't reset when the nRF52 is (by a reflash, watchdog, or
/// panic), so one can be left part-way through sending a byte, holding SDA
/// low indefinitely; every later transfer then fails. We saw exactly this on
/// the dev kit. The standard cure: clock SCL until the device has shifted out
/// the rest of its byte and releases SDA, then generate a STOP so every device
/// returns to idle.
pub async fn recover_bus(sda: Peri<'_, AnyPin>, scl: Peri<'_, AnyPin>) {
    // Both lines open-drain (drive low, release high), as on a real I2C bus;
    // the board has external pull-ups.
    let mut sda = Flex::new(sda);
    sda.set_as_input(Pull::None);
    let mut scl = Output::new(scl, Level::High, OutputDrive::Standard0Disconnect1);
    // 100 kHz-ish half periods.
    let half_period = || Timer::after_micros(5);

    if sda.is_high() {
        return;
    }

    // A byte plus its ACK is nine clocks, so nine pulses is always enough.
    let mut pulses = 0;
    while sda.is_low() && pulses < 9 {
        scl.set_low();
        half_period().await;
        scl.set_high();
        half_period().await;
        pulses += 1;
    }

    // STOP: SDA rises while SCL is high.
    scl.set_low();
    sda.set_low();
    sda.set_as_output(OutputDrive::Standard0Disconnect1);
    half_period().await;
    scl.set_high();
    half_period().await;
    sda.set_high();
    half_period().await;

    if sda.is_high() {
        info!("i2c: recovered stuck bus after {} clock pulses", pulses);
    } else {
        warn!("i2c: SDA still held low after recovery");
    }
}
