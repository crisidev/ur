//! The temperature on the clock face: the watch's own, from the nRF52832's
//! die sensor.
//!
//! The PineTime has no ambient temperature sensor, only the one on the
//! chip, so what this measures is the watch itself. On the wrist that sits
//! close to skin temperature (low 30s °C); off the wrist for a while it
//! settles towards the room's. It's shown as measured, in whole degrees:
//! modelling the wrist to guess the air temperature would only be a
//! confident-looking guess.
//!
//! The sensor (the TEMP peripheral) belongs to MPSL, Nordic's radio
//! scheduler, so `main` reads it through MPSL rather than directly. A reading takes ~50 µs. Its resolution is 0.25 °C, but
//! Nordic only specifies its accuracy to within a few degrees.
//!
//! The watch warms itself a little while it works, but not enough to
//! matter: on the (open-cased) dev kit, idling at 20.5 °C, five minutes
//! with the screen and backlight on raised the reading by a quarter of a
//! degree, gone again a minute after the screen went off. That's within
//! what [`shown`] smooths away, so the sensor is read on a fixed schedule,
//! whether or not the screen is on.
//!
//! TODO: °F for those who'd want it (a setting, or the phone's locale
//! from Gadgetbridge).

/// How often to read the sensor, in seconds. The watch's temperature
/// changes over minutes (its case and the wrist are slow to warm and cool),
/// so once a minute is plenty.
pub const SAMPLE_EVERY: u64 = 60;

/// The whole degrees (°C) to show, given what was shown before and a new
/// reading in quarter degrees (the sensor's unit).
///
/// A reading near the boundary between two degrees (wobbling between,
/// say, 31.25 and 31.75 °C) would otherwise flip the shown value back and
/// forth from minute to minute, so it only changes once the reading is a
/// whole degree away from what's shown, and then to the nearest whole
/// degree. What's shown is thus never more than 0.75 °C off.
pub fn shown(previous: Option<i8>, quarters: i32) -> i8 {
    if let Some(previous) = previous
        && (quarters - 4 * i32::from(previous)).abs() < 4
    {
        return previous;
    }
    // Round half up, also for negative readings (`div_euclid` floors).
    let nearest = (quarters + 2).div_euclid(4);
    // The sensor's range is -40 to 85 °C, so this never clamps in
    // practice.
    nearest.clamp(i8::MIN.into(), i8::MAX.into()) as i8
}
