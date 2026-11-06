//! Persistent storage: a small key-value map in the last 16 KB of flash.
//!
//! Holds anything that should survive a reboot: Bluetooth bonds (so the phone
//! doesn't have to pair again) and settings (the theme; alarms later).
//!
//! Two choices worth knowing about:
//!
//! - Writes go through MPSL's flash driver, not the plain NVMC. Erasing a
//!   flash page stalls the CPU for ~85 ms, which would make the radio miss
//!   its timing; MPSL schedules erases and writes into gaps between radio
//!   events instead.
//! - `sequential-storage` appends new values and only erases a page when it
//!   runs out of space, spreading wear across the four pages.
//!
//! The region is reserved in `memory.x`, so firmware can never grow into it,
//! and flashing new firmware only erases the pages the image occupies, so
//! stored data survives reflashing.

use core::ops::Range;

use defmt::warn;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::mutex::Mutex;
use nrf_sdc::mpsl::Flash;
use sequential_storage::cache::{Cache, Uncached};
use sequential_storage::map::{MapConfig, MapStorage};

/// Must match the `STORAGE` region in `memory.x`.
const RANGE: Range<u32> = 0x7C000..0x80000;

/// Every stored item's key. Never reuse a retired number: old data under it
/// may still be in flash.
pub mod key {
    /// Bluetooth bonds, one per slot.
    pub const BONDS: [u8; 2] = [1, 2];
    // 16: retired (a single theme index, before themes followed the sun).
    /// The chosen theme within [`crate::theme::LIGHT`] and `DARK`.
    pub const THEME_LIGHT: u8 = 17;
    pub const THEME_DARK: u8 = 18;
    /// The last sunrise/sunset from the phone; see [`crate::clock::Sun`].
    pub const SUN: u8 = 19;
}

/// Big enough for the largest item (a 43-byte bond) plus its key, rounded up
/// to flash word alignment.
const BUFFER: usize = 64;

type Map = MapStorage<u8, Flash<'static>, Cache<Uncached, Uncached, Uncached, u8>>;

/// The map, made storable in a `static`.
struct Store(Map);

// SAFETY: `Flash` is `!Send` because it borrows MPSL, which must not be used
// from interrupts other than its own. The store is only ever used from tasks
// on the single thread-mode executor (never from an interrupt handler), so
// handing it between those tasks is no different from one task owning it.
unsafe impl Send for Store {}

static STORE: Mutex<CriticalSectionRawMutex, Option<Store>> = Mutex::new(None);

/// Must be called once, before anything loads or saves.
pub fn init(flash: Flash<'static>) {
    let map = MapStorage::new(flash, MapConfig::new(RANGE), Cache::new_uncached());
    *STORE.try_lock().expect("nothing uses storage before init") = Some(Store(map));
}

/// Loads the value stored under `key`, if any. Storage errors (e.g.
/// corruption) are logged and treated as "nothing stored".
pub async fn load<const N: usize>(key: u8) -> Option<[u8; N]> {
    let mut buffer = [0u8; BUFFER];
    let mut store = STORE.lock().await;
    let Store(store) = store.as_mut().expect("storage::init runs at boot");
    match store.fetch_item::<[u8; N]>(&mut buffer, &key).await {
        Ok(value) => value,
        Err(e) => {
            warn!("storage: loading key {} failed: {}", key, e);
            None
        }
    }
}

/// Stores `value` under `key`; returns whether that worked (failures are
/// logged).
pub async fn save<const N: usize>(key: u8, value: &[u8; N]) -> bool {
    let mut buffer = [0u8; BUFFER];
    let mut store = STORE.lock().await;
    let Store(store) = store.as_mut().expect("storage::init runs at boot");
    let result = store.store_item(&mut buffer, &key, value).await;
    if let Err(e) = &result {
        warn!("storage: saving key {} failed: {}", key, e);
    }
    result.is_ok()
}

/// Removes whatever is stored under `key`; returns whether that worked
/// (failures are logged).
pub async fn remove(key: u8) -> bool {
    let mut buffer = [0u8; BUFFER];
    let mut store = STORE.lock().await;
    let Store(store) = store.as_mut().expect("storage::init runs at boot");
    let result = store.remove_item(&mut buffer, &key).await;
    if let Err(e) = &result {
        warn!("storage: removing key {} failed: {}", key, e);
    }
    result.is_ok()
}
