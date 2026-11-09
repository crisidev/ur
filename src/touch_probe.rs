//! Touch probe: logs everything the touch controller reports, not just the
//! six bytes [`crate::touch`] uses, to find out whether anything in it tells
//! a fingertip from a bigger contact (a knuckle, a palm, a forearm brushing
//! the screen), which could then be rejected. Only with the `touch-probe`
//! feature, read over a debug probe; `scripts/touch_probe.py` has the
//! protocol for a session and analyses its log.
//!
//! What the chip might have beyond the documented report:
//!
//! - **0x00–0x0F**, read in one go on every report instead of 0x01–0x06.
//!   The Hynitron register declaration stops at 0x06, but the reference
//!   driver lays the report out like FocalTech's (PINE64's wiki has it):
//!   0x00 a work mode, then per touch point six bytes from 0x03: event and
//!   X high, X low, touch ID and Y high, Y low, then **0x07 a "pressure"
//!   (weight) and 0x08 "miscellaneous"** (area, in FocalTech's), and the
//!   second point from 0x09. The wiki found 0x07 and 0x08 always 0 on its
//!   PineTime; InfiniTime has them commented out as unavailable. Both are
//!   the bytes most worth seeing with a knuckle or a palm.
//! - **0xB0–0xB3**, `BPC0` and `BPC1`, 16-bit values the declaration gives
//!   no meaning for (the reference driver calls 0xB0 a face detection
//!   switch, a phone's way of ignoring a cheek, the nearest thing to what
//!   we're after). Read on every report too, in case they move while
//!   touched. If they can't be read, the rest of the boot goes without.
//! - **0xA0–0xBF and 0xE0–0xFF**, once after each configuration (the chip
//!   only listens just after its reset): the chip and firmware IDs (0xA7,
//!   0xA9: which variant this is) and its settings (scan thresholds, auto
//!   sleep and reset times), for reading the reports against.
//!
//! Each report is one `info` line: a sequence number (gaps are lines the
//! RTT buffer dropped), uptime in milliseconds, and the bytes, 0x00 on.
//! A finger down reports every ~10 ms, so this logs ~100 lines a second:
//! a probe build only, never the field log.

use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use defmt::{info, warn};
use embassy_time::Instant;
use embedded_hal_async::i2c::{ErrorType, I2c};

use crate::i2c::Device;
use crate::touch::ADDRESS;

// A sealed watch has no probe, and a day of touches would fill the field
// log with these lines and push out everything else.
#[cfg(feature = "field-log")]
compile_error!("`touch-probe` logs ~100 lines a second while touched: read it over a probe");

/// The wider report block: 16 registers from 0x00.
const REG_BLOCK: u8 = 0x00;
/// `BPC0` and `BPC1`, big-endian, from 0xB0.
const REG_BPC: u8 = 0xB0;

/// How many reports have been logged, for spotting dropped lines. Also
/// the marker `scripts/release.sh` looks for to refuse a probe build, as
/// it does `UR_INPUT`: hence a fixed name.
#[unsafe(no_mangle)]
static UR_TOUCH_PROBE: AtomicU32 = AtomicU32::new(0);

/// Whether reading [`REG_BPC`] failed, so we've stopped trying: a register
/// the chip won't serve mustn't cost every touch a failed transfer.
static NO_BPC: AtomicBool = AtomicBool::new(false);

/// Reads a touch report the wide way and logs it whole, then fills `report`
/// with the six bytes (0x01–0x06) the normal build reads. Fails as that
/// read would: if the main block can't be read.
pub async fn read(
    i2c: &mut Device,
    report: &mut [u8; 6],
) -> Result<(), <Device as ErrorType>::Error> {
    // The main block plus BPC's four, logged as one slice: 16 bytes, or 20.
    let mut bytes = [0u8; 20];
    let (block, bpc) = bytes.split_at_mut(16);
    i2c.write_read(ADDRESS, &[REG_BLOCK], block).await?;
    report.copy_from_slice(&block[1..7]);

    let mut len = 16;
    if !NO_BPC.load(Ordering::Relaxed) {
        match i2c.write_read(ADDRESS, &[REG_BPC], bpc).await {
            Ok(()) => len = 20,
            Err(_) => {
                warn!("touch-probe: reading BPC failed; not trying again");
                NO_BPC.store(true, Ordering::Relaxed);
                // As after any failed transfer: don't leave the shared bus
                // wedged (see `crate::touch::sleep`).
                crate::i2c::reset_controller().await;
            }
        }
    }

    let seq = UR_TOUCH_PROBE.fetch_add(1, Ordering::Relaxed);
    // Truncated to u32: wraps after 49 days, far beyond a session.
    let ms = Instant::now().as_millis() as u32;
    info!(
        "touch-probe: #{=u32} {=u32}ms {=[u8]:02x}",
        seq,
        ms,
        &bytes[..len]
    );
    Ok(())
}

/// Logs the chip's IDs and settings, right after it's been configured (and
/// so still listening). Failures only cost the log line.
pub async fn dump_settings(i2c: &mut Device) {
    for start in [0xA0u8, 0xE0] {
        let mut regs = [0u8; 32];
        match i2c.write_read(ADDRESS, &[start], &mut regs).await {
            Ok(()) => info!("touch-probe: regs {=u8:02x}: {=[u8]:02x}", start, &regs[..]),
            Err(_) => {
                warn!("touch-probe: reading regs {=u8:02x} failed", start);
                crate::i2c::reset_controller().await;
            }
        }
    }
}
