//! Persistent storage: a small key-value map in the last 12 KB of flash.
//!
//! Holds anything that should survive a reboot: Bluetooth bonds (so the phone
//! doesn't have to pair again) and settings (themes, alarms, the
//! phone's alarms).
//!
//! Two choices worth knowing about:
//!
//! - Writes go through MPSL's flash driver, not the plain NVMC. Erasing a
//!   flash page stalls the CPU for ~85 ms, which would make the radio miss
//!   its timing; MPSL schedules erases and writes into gaps between radio
//!   events instead.
//! - `sequential-storage` appends new values and only erases a page when it
//!   runs out of space, spreading wear across the three pages.
//!
//! The region is reserved in `memory.x`, so firmware can never grow into it.
//! Neither updates nor the bootloader touch it (MCUBoot's swap scratch is the
//! page just below), and flashing over SWD only erases the pages the image
//! occupies, so stored data survives both.

use core::ops::Range;

use defmt::warn;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::mutex::Mutex;
use nrf_sdc::mpsl::Flash;
use sequential_storage::cache::{Cache, Uncached};
use sequential_storage::map::{MapConfig, MapStorage};

/// Must match the `STORAGE` region in `memory.x`.
const RANGE: Range<u32> = 0x7D000..0x80000;

/// Every stored item's key. Never reuse a retired number: old data under it
/// may still be in flash.
pub mod key {
    /// Bluetooth bonds, one per slot.
    pub const BONDS: [u8; 2] = [1, 2];
    // 16: retired (a single theme index, before themes followed the sun).
    /// The chosen theme within [`crate::theme::LIGHT`] and `DARK`.
    pub const THEME_LIGHT: u8 = 17;
    pub const THEME_DARK: u8 = 18;
    // 19: retired (sunrise/sunset from weather, before we computed them).
    /// The last location from the phone; see [`crate::geo::Location`].
    pub const LOCATION: u8 = 20;
    // 21, 22: retired (alarm settings with one alarm and one or two
    // timers).
    /// Alarm settings; see [`crate::alarm::Settings`].
    pub const ALARMS: u8 = 23;
    /// The alarms synced from the phone; see [`crate::alarm::Phone`].
    pub const PHONE_ALARMS: u8 = 24;
    /// The brightness setting; see [`crate::brightness::Setting`].
    pub const BRIGHTNESS: u8 = 25;
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

/// Writes one word of flash *outside* the storage region, through the same
/// MPSL flash driver (which the map owns, hence living here). Only for
/// [`crate::firmware::confirm`], which flips a bootloader flag. Returns
/// whether that worked (failures are logged).
pub async fn write_word_elsewhere(address: u32, word: u32) -> bool {
    assert!(
        !RANGE.contains(&address) && address.is_multiple_of(4),
        "a word-aligned address outside the map"
    );
    let mut store = STORE.lock().await;
    // `MapStorage` doesn't lend out its flash, so take the map apart and
    // rebuild it. It's uncached, so nothing is lost by doing so.
    let Store(map) = store.take().expect("storage::init runs at boot");
    let (mut flash, cache) = map.destroy();
    let result = flash.write(address, &word.to_le_bytes()).await;
    *store = Some(Store(MapStorage::new(flash, MapConfig::new(RANGE), cache)));
    if let Err(e) = &result {
        warn!("storage: writing {:#x} failed: {}", address, e);
    }
    result.is_ok()
}
