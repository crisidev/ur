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
