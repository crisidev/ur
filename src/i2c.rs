//! The shared I2C bus: touch controller (0x15), accelerometer (0x18), and
//! heart-rate sensor (0x44), all on TWIM1 at P0.06 (SDA) / P0.07 (SCL).

use defmt::{info, warn};
use embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice;
use embassy_nrf::Peri;
use embassy_nrf::gpio::{AnyPin, Flex, Level, Output, OutputDrive, Pull};
use embassy_nrf::twim::Twim;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_time::Timer;

/// One driver's handle on the shared bus.
pub type Device = I2cDevice<'static, CriticalSectionRawMutex, Twim<'static>>;

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
