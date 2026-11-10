//! Persistent storage: a small key-value map in the last 12 KB of flash.
//!
//! Holds anything that should survive a reboot: Bluetooth bonds (so the phone
//! doesn't have to pair again) and settings (themes, alarms, the
//! phone's alarms).
//!
//! Two choices worth knowing about:
//!
//! - Writes go through MPSL timeslots ([`crate::nvmc`]), not the plain
//!   NVMC. Erasing a flash page stalls the CPU for ~85 ms, which would make
//!   the radio miss its timing; MPSL schedules erases and writes into gaps
//!   between radio events instead.
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
use embedded_storage_async::nor_flash::NorFlash;
use sequential_storage::cache::{Cache, Uncached};
use sequential_storage::map::{MapConfig, MapStorage};

/// The internal flash, through MPSL timeslots. In the simulator (`sim/`), which
/// runs this module too, a model of it, with the same pages and the same
/// addresses.
#[cfg(target_os = "none")]
pub type Flash = crate::nvmc::Flash;
#[cfg(not(target_os = "none"))]
pub type Flash = crate::flash::Flash;

/// Must match the `STORAGE` region in `memory.x`.
pub const RANGE: Range<u32> = 0x7D000..0x80000;

/// Every stored item's key. Never reuse a retired number: old data under it
/// may still be in flash.
pub mod key {
    /// Bluetooth bonds, one per slot.
    pub const BONDS: [u8; 2] = [1, 2];
    // 16: retired (a single theme index, before themes followed the sun).
    /// Retired, but read to migrate from: the chosen scheme for daylight
    /// and for darkness as an index into retro's, as 0.10 and earlier
    /// saved it (see `themes::migrate`).
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
    /// The button grid's setting; see [`crate::buttons::Setting`].
    pub const BUTTONS: u8 = 30;
    /// Home Assistant's key for the grid's presses; see [`crate::bthome`].
    pub const BTHOME_KEY: u8 = 31;
    /// How far the BTHome counter is reserved (see `bthome::send`).
    pub const BTHOME_COUNTER: u8 = 32;
    /// How to estimate the room's temperature on the wrist; see
    /// [`crate::temperature::Calibration`]. (Not 33–39: kept clear for
    /// what other branches add.)
    pub const CALIBRATION: u8 = 40;
    /// The button grid's layout; see [`crate::buttons::Layout`].
    pub const GRID: u8 = 41;
    /// The chosen colour scheme for daylight and for darkness, by its ID
    /// ([`crate::palette::id`], little-endian), not an index, as which
    /// schemes there are varies from build to build (see
    /// [`crate::themes`]).
    pub const THEME_DAY: u8 = 42;
    pub const THEME_NIGHT: u8 = 43;
    /// Settings' FORCE: which side's schemes the theme shows (see
    /// [`crate::ui::Force`]).
    pub const FORCE: u8 = 44;
    /// Written only by builds with the `flash-stress` feature (see
    /// `flash_stress.rs`); a dev kit that ran one may hold it.
    #[allow(dead_code)] // Used only by those builds.
    pub const STRESS: u8 = 99;
}

/// Big enough for the largest item (a 43-byte bond) plus its key, rounded up
/// to flash word alignment.
const BUFFER: usize = 64;

// The map's pages are the flash's (4 KiB), and it needs two at least: one
// to move the live items to while it erases another.
const _: () = assert!(
    RANGE.start.is_multiple_of(4096)
        && RANGE.end.is_multiple_of(4096)
        && RANGE.end - RANGE.start >= 2 * 4096
);

type Map = MapStorage<u8, Flash, Cache<Uncached, Uncached, Uncached, u8>>;

/// The map, made storable in a `static`.
struct Store(Map);

// SAFETY: `Flash` is `!Send` because it borrows MPSL, which must not be used
// from interrupts other than its own. The store is only ever used from tasks
// on the single thread-mode executor (never from an interrupt handler), so
// handing it between those tasks is no different from one task owning it.
unsafe impl Send for Store {}

static STORE: Mutex<CriticalSectionRawMutex, Option<Store>> = Mutex::new(None);

/// Must be called once, before anything loads or saves.
pub fn init(flash: Flash) {
    let map = MapStorage::new(flash, MapConfig::new(RANGE), Cache::new_uncached());
    *STORE.try_lock().expect("nothing uses storage before init") = Some(Store(map));
}

/// Loads the value stored under `key`, if any. Storage errors (e.g.
/// corruption) are logged and treated as "nothing stored"; an item too
/// short for its key is removed (see [`load_into`]).
pub async fn load<const N: usize>(key: u8) -> Option<[u8; N]> {
    let mut value = [0; N];
    load_into(key, &mut value).await.then_some(value)
}

/// [`load`], into `value`; returns whether anything was stored.
///
/// Not generic over the value's size, unlike `load`: a `fetch_item` per
/// size was a copy of the map's lookup per size, ~0.7 KB each. As for the
/// `[u8; N]` that took, a longer item gives its first `value.len()`
/// bytes, and a shorter one is removed.
///
/// Removed, rather than left to fail every boot: no firmware of ours
/// reads it, as an item's layout only ever changes under a new key (see
/// [`key`]), so a shorter one is a leftover, of a test build say (a dev
/// kit had a 1-byte item under [`key::PHONE_ALARMS`], which takes 41).
/// Left, it read as nothing anyway, and logged a warning at every boot
/// until the setting was next saved. Only that case: any other error
/// (reading the flash, or a corrupt page) isn't this item's fault, and
/// removing it then could lose a good value.
async fn load_into(key: u8, value: &mut [u8]) -> bool {
    // The map reads the key and the value into `buffer`: a value that
    // doesn't fit beside its key could never be loaded (nor saved; see
    // `save`), and the setting would read as "nothing stored" for good.
    debug_assert!(
        value.len() < BUFFER,
        "an item fits the buffer, with its key"
    );
    let mut buffer = [0u8; BUFFER];
    let mut store = STORE.lock().await;
    let Store(store) = store.as_mut().expect("storage::init runs at boot");
    let short = match store.fetch_item::<&[u8]>(&mut buffer, &key).await {
        Ok(Some(stored)) if stored.len() >= value.len() => {
            value.copy_from_slice(&stored[..value.len()]);
            return true;
        }
        Ok(Some(stored)) => stored.len(),
        Ok(None) => return false,
        Err(e) => {
            warn!("storage: loading key {} failed: {}", key, e);
            return false;
        }
    };
    match store.remove_item(&mut buffer, &key).await {
        Ok(()) => warn!(
            "storage: key {} held {} bytes, not {}; removed",
            key,
            short,
            value.len()
        ),
        Err(e) => warn!(
            "storage: key {} held {} bytes, not {}; removing failed: {}",
            key,
            short,
            value.len(),
            e
        ),
    }
    false
}

/// Stores `value` under `key`; returns whether that worked (failures are
/// logged). A slice, not an array, so there's one copy of this whatever
/// the size.
pub async fn save(key: u8, value: &[u8]) -> bool {
    // The map writes the key and the value from `buffer`: a value that
    // doesn't fit beside its key is refused, every time, with only a
    // warning in the log to say the setting isn't kept. Every item is of
    // a fixed size, so one save in a test (or the simulator) checks it.
    debug_assert!(
        value.len() < BUFFER,
        "an item fits the buffer, with its key"
    );
    let mut buffer = [0u8; BUFFER];
    let mut store = STORE.lock().await;
    let Store(store) = store.as_mut().expect("storage::init runs at boot");
    let result = store.store_item(&mut buffer, &key, &value).await;
    if let Err(e) = &result {
        warn!("storage: saving key {} failed: {}", key, e);
    }
    // What was saved is what a load now gives: the one thing the map is
    // for. Checked where debug assertions are on (the simulator, and its
    // fuzzer, over a model of the flash that only clears bits, as the
    // chip does), not on the watch, where it would be a second pass over
    // the flash for every save.
    #[cfg(debug_assertions)]
    if result.is_ok() {
        let mut buffer = [0u8; BUFFER];
        let read_back = store.fetch_item::<&[u8]>(&mut buffer, &key).await;
        debug_assert!(
            matches!(read_back, Ok(Some(stored)) if stored == value),
            "a saved item reads back as it was saved"
        );
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
    let result = NorFlash::write(&mut flash, address, &word.to_le_bytes()).await;
    *store = Some(Store(MapStorage::new(flash, MapConfig::new(RANGE), cache)));
    if let Err(e) = &result {
        warn!("storage: writing {:#x} failed: {}", address, e);
    }
    result.is_ok()
}
