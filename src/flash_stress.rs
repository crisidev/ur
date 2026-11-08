//! A stress test of the internal flash's timeslots, only in builds with the
//! `flash-stress` feature: saves to [`storage`] back to back, so its pages
//! fill and erase every few seconds, while `scripts/flash-stress.py` reads
//! the dev kit's RAM over the probe as hard as `ur-sim --devkit` does. An
//! erase that overran its timeslot would reset the watch (see
//! [`crate::nvmc`]); the script watches for that, and for failed saves.
//!
//! What it counts is exported in [`UR_FLASH_STRESS`], for the script to
//! read.

use core::sync::atomic::{AtomicU32, Ordering};

use embassy_time::Timer;

use crate::storage;

/// Counters, in order: saves, failed saves, page erases, the longest and
/// shortest erase (real time, µs), the longest by TIMER0 (how far into
/// its slot an erase ended, by the slot's own clock), and [`RUNNING`] once
/// the task runs, so the script can tell a booting dev kit from a running
/// one.
#[unsafe(no_mangle)]
static UR_FLASH_STRESS: [AtomicU32; 7] = [const { AtomicU32::new(0) }; 7];

const SAVES: usize = 0;
const FAILURES: usize = 1;
const ERASES: usize = 2;
const LONGEST: usize = 3;
const SHORTEST: usize = 4;
const LONGEST_TIMER0: usize = 5;
const MARKER: usize = 6;
const RUNNING: u32 = u32::from_le_bytes(*b"STRS");

/// Called by [`crate::nvmc`] after each erase, with its longest page.
pub fn erased(micros: u32, timer0_us: u32) {
    let counters = &UR_FLASH_STRESS;
    counters[ERASES].fetch_add(1, Ordering::Relaxed);
    counters[LONGEST].fetch_max(micros, Ordering::Relaxed);
    let shortest = counters[SHORTEST].load(Ordering::Relaxed);
    if shortest == 0 || micros < shortest {
        counters[SHORTEST].store(micros, Ordering::Relaxed);
    }
    counters[LONGEST_TIMER0].fetch_max(timer0_us, Ordering::Relaxed);
}

#[embassy_executor::task]
pub async fn stress_task() {
    // Let boot's own loads and saves (bonds, settings) go first.
    Timer::after_secs(5).await;
    defmt::warn!("flash-stress: saving back to back");
    UR_FLASH_STRESS[MARKER].store(RUNNING, Ordering::Relaxed);
    let mut n: u32 = 0;
    loop {
        n = n.wrapping_add(1);
        // A different value each time, as big as an item gets (a bond's
        // size), so the pages fill as fast as real saves could fill them.
        let mut value = [0u8; 48];
        value[..4].copy_from_slice(&n.to_le_bytes());
        let saved = storage::save(storage::key::STRESS, &value).await;
        let counters = &UR_FLASH_STRESS;
        counters[SAVES].fetch_add(1, Ordering::Relaxed);
        if !saved {
            counters[FAILURES].fetch_add(1, Ordering::Relaxed);
        }
        // Room for the UI and the radio between saves, as between taps.
        Timer::after_millis(20).await;
    }
}
