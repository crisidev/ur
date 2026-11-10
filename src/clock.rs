//! Wall-clock time.
//!
//! The watch has no battery-backed RTC, so after every boot it knows only
//! time-since-boot until the phone tells it the time (Gadgetbridge sends UTC
//! and the UTC offset on every connect and when the phone's time zone
//! changes; see [`crate::gb`]). We then remember "at uptime X, it was UTC
//! time Y, in offset Z" and derive the time from the RTC-driven uptime from
//! then on. Between syncs, accuracy is that of the 32.768 kHz crystal
//! (±20 ppm, i.e. under 2 s/day).
//!
//! The time also survives resets that keep power on (reflashing, watchdog,
//! panics): [`checkpoint`] saves it once a second in RAM that startup doesn't
//! clear, and [`restore`] picks it up at boot, off by at most about a second
//! (plus however long a debugger halted the core) until the phone next syncs.
//! A drained battery still loses it, and so does System OFF (see
//! [`forget`]).
//!
//! TODO: daylight-saving changes between syncs keep the old offset until the
//! phone next connects (which it normally does within minutes).

use core::cell::Cell;

use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_time::Instant;

pub use crate::calendar::DateTime;

#[derive(Clone, Copy)]
struct Sync {
    /// Uptime when the time was set.
    at: Instant,
    /// Seconds since 1970-01-01T00:00Z at `at`.
    utc_seconds: i64,
    offset_minutes: i16,
}

static SYNC: Mutex<CriticalSectionRawMutex, Cell<Option<Sync>>> = Mutex::new(Cell::new(None));

pub fn set(utc_seconds: i64, offset_minutes: i16) {
    // How far off we were shows crystal drift between syncs, and how much a
    // reset (e.g. a reflash, which halts the core) set us back.
    if let Some(before) = now_utc_seconds() {
        defmt::info!("clock: was off by {} s", before - utc_seconds);
    }
    let sync = Sync {
        at: Instant::now(),
        utc_seconds,
        offset_minutes,
    };
    SYNC.lock(|cell| cell.set(Some(sync)));
    log_now();
}

/// Logs the wall-clock time, so logs (whose timestamps count from boot)
/// can be lined up with other records, such as another watch's. Called on
/// every sync and periodically.
pub fn log_now() {
    if let (Some(utc), Some(local)) = (now_utc_seconds(), now()) {
        defmt::info!(
            "clock: {=i64} UTC = {}-{=u8:02}-{=u8:02} {=u8:02}:{=u8:02}:{=u8:02} local",
            utc,
            local.year,
            local.month,
            local.day,
            local.hour,
            local.minute,
            local.second
        );
    }
}

/// Seconds since 1970-01-01T00:00Z, if the phone has told us the time.
pub fn now_utc_seconds() -> Option<i64> {
    let sync = SYNC.lock(Cell::get)?;
    Some(sync.utc_seconds + (Instant::now() - sync.at).as_secs() as i64)
}

/// The current local time, if the phone has told us yet.
pub fn now() -> Option<DateTime> {
    let offset = utc_offset_minutes()?;
    Some(DateTime::from_epoch_seconds(
        now_utc_seconds()? + i64::from(offset) * 60,
    ))
}

/// The phone's UTC offset at the last sync.
pub fn utc_offset_minutes() -> Option<i16> {
    SYNC.lock(Cell::get).map(|sync| sync.offset_minutes)
}

// ==============================================================================
// Surviving resets
// ==============================================================================

/// The last known time, in the second half of the RAM block `memory.x` keeps
/// out of the linker's hands. Nothing initialises it at startup, so it
/// survives any reset that keeps power on, and its fixed address means even a
/// different firmware build (after reflashing) finds it.
#[repr(C)]
struct Retained {
    magic: u32,
    offset_minutes: i16,
    utc_seconds: i64,
    /// `utc_seconds` and `offset_minutes` mixed and inverted, so random
    /// power-on RAM contents can't pass for a valid time.
    check: i64,
}

impl Retained {
    fn check(utc_seconds: i64, offset_minutes: i16) -> i64 {
        !(utc_seconds ^ (i64::from(offset_minutes) << 48))
    }
}

/// Changed whenever `Retained`'s layout does, so a new build never misreads
/// an old build's checkpoint.
const RETAINED_MAGIC: u32 = 0x7572_4332; // "urC2"

/// Must match the `RETAINED` region in `memory.x`.
const RETAINED_ADDRESS: usize = 0x2000_D420;
const _: () = assert!(size_of::<Retained>() <= 32);

fn retained() -> *mut Retained {
    #[cfg(target_os = "none")]
    return RETAINED_ADDRESS as *mut Retained;
    // On the host (the simulator, which runs the UI loop and its
    // checkpoints), an ordinary static stands in for the retained block.
    #[cfg(not(target_os = "none"))]
    {
        static mut HOST: Retained = Retained {
            magic: 0,
            offset_minutes: 0,
            utc_seconds: 0,
            check: 0,
        };
        &raw mut HOST
    }
}

/// Saves the current time for [`restore`]. Call about once a second; the
/// interval bounds how far behind a restored clock can be.
pub fn checkpoint() {
    let Some(sync) = SYNC.lock(Cell::get) else {
        return;
    };
    let utc_seconds = now_utc_seconds().expect("synced, checked above");
    let value = Retained {
        magic: RETAINED_MAGIC,
        offset_minutes: sync.offset_minutes,
        utc_seconds,
        check: Retained::check(utc_seconds, sync.offset_minutes),
    };
    // SAFETY: The address is reserved RAM (see `memory.x`), suitably
    // aligned, and only ever accessed from the UI loop (checkpoint) and once
    // at boot before any task runs (restore); never concurrently. Volatile so
    // the write isn't optimised away as dead.
    unsafe { retained().write_volatile(value) };
}

/// Drops the checkpoint, so the next boot doesn't resume from it: before
/// System OFF (see [`crate::system_off`]), during which no clock runs, so
/// the time is unknown afterwards, however long the watch was off. A stale
/// time would be worse than none: alarms would ring at the wrong time, and
/// activity would be recorded (and synced) under the wrong minutes.
pub fn forget() {
    // SAFETY: As in `checkpoint`, from the same (UI loop) task.
    unsafe { core::ptr::addr_of_mut!((*retained()).magic).write_volatile(0) };
}

/// Loses the time, as RAM is lost while the watch is off: for the
/// simulator, whose RAM outlives its switching off. (On the watch, the
/// next boot starts without a time anyway.)
#[cfg(not(target_os = "none"))]
pub fn lose() {
    SYNC.lock(|cell| cell.set(None));
}

/// Resumes the clock from the last [`checkpoint`] before a reset, if there
/// was one. Call once, early at boot.
pub fn restore() {
    // SAFETY: See `checkpoint`. Any bit pattern is a valid `Retained`
    // (plain integers), so reading uninitialised-looking RAM is fine; the
    // magic and check fields tell us whether it's real.
    let saved = unsafe { retained().read_volatile() };
    if saved.magic != RETAINED_MAGIC
        || saved.check != Retained::check(saved.utc_seconds, saved.offset_minutes)
    {
        return;
    }
    // The checkpoint was up to a second before the reset, and booting takes
    // a fraction of a second more; splitting the difference keeps the error
    // within about half a second either way.
    set(saved.utc_seconds + 1, saved.offset_minutes);
    defmt::info!("clock: resumed after reset");
}
