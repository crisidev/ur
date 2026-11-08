//! The shared SPI bus (SPIM2: SCK P0.02, MOSI P0.03, MISO P0.04): the LCD
//! (CS P0.25) and the SPI flash (CS P0.05).
//!
//! Each driver drives its own chip-select, and must hold the bus lock for as
//! long as its chip is selected: anything sent while two chips are selected
//! reaches both.

use embassy_nrf::spim::Spim;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::mutex::{Mutex, MutexGuard};
use embassy_sync::once_lock::OnceLock;

pub type Bus = Mutex<CriticalSectionRawMutex, Spim<'static>>;
pub type Guard = MutexGuard<'static, CriticalSectionRawMutex, Spim<'static>>;

/// Set once at boot.
pub static BUS: OnceLock<&'static Bus> = OnceLock::new();

/// SPIM transfers only fail for buffers outside RAM, and embassy-nrf copies
/// those into RAM first.
pub const WRITE_CANNOT_FAIL: &str = "SPIM only fails on non-RAM buffers, which embassy copies";

/// Sends `data`, from RAM or flash, discarding what comes back.
///
/// Instead of `Spim::write`, because of what that costs in RAM: it handles
/// data in flash (which DMA can't read) by copying it to a 255-byte buffer
/// that lives in the calling future, so every task that ever touched the
/// bus asynchronously carried those 255 bytes (and more), in its static
/// task storage, forever. Almost everything we send is in RAM already;
/// the rest (constant commands) is a few bytes, bounced through a small
/// buffer instead.
pub async fn write(bus: &mut Spim<'static>, data: &[u8]) {
    if bus.write_from_ram(data).await.is_ok() {
        return;
    }
    // Pieces of one transfer can go out separately: the chip only sees
    // the bytes between its chip-select edges, which the caller owns.
    let mut bounce = [0u8; 8];
    for piece in data.chunks(bounce.len()) {
        let bounce = &mut bounce[..piece.len()];
        bounce.copy_from_slice(piece);
        bus.write_from_ram(bounce)
            .await
            .expect("the bounce buffer is in RAM");
    }
}

/// Reads `data.len()` bytes, sending the over-read character meanwhile, as
/// `Spim::read` does. Like [`write`], to avoid `Spim::read`'s copy buffer
/// (which it carries even though it has nothing to send).
pub async fn read(bus: &mut Spim<'static>, data: &mut [u8]) {
    bus.transfer_from_ram(data, &[])
        .await
        .expect("an empty write is trivially in RAM");
}
