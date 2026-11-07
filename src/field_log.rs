//! The field log: everything we'd see over the debug probe, kept in the SPI
//! flash, for watches without one (a sealed watch on a wrist).
//!
//! defmt already encodes every log message compactly into the RTT buffer
//! that a debug probe reads. In a field build (the `field-log` feature) this
//! module *is* that reader: it drains the buffer once a second and appends
//! the raw defmt frames to a ring of sectors in the SPI flash. A computer
//! later fetches them over Bluetooth (`scripts/fetch-log.py`, through the
//! service in [`crate::ble`]) and decodes them with `defmt-print` and the
//! ELF of the build that wrote them. So the field log costs no formatting
//! code on the watch, and holds exactly what the probe would have shown.
//!
//! Not for builds read through a probe: the probe and this module would
//! each take whatever the other didn't.
//!
//! Layout: [`REGION`] is a ring of 4 KiB sectors, each a 16-byte header
//! (magic, sequence number, image version, data length) followed by whole
//! defmt frames. Every boot starts a new sector; the oldest is erased to
//! make room. The length is written when a sector is closed (NOR flash can
//! still clear bits of the erased header), so a sector without one is either
//! the current one or was cut off by a reset; the reader then trims at the
//! last frame boundary.
//!
//! Also here, because they're about knowing what happened in the field: the
//! reset reason, and where the previous run panicked (see [`panic`]).

use core::cell::Cell;
#[cfg(feature = "field-log")]
use core::sync::atomic::{AtomicU32, Ordering};

use defmt::{info, warn};
use embassy_sync::blocking_mutex::Mutex as BlockingMutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
#[cfg(feature = "field-log")]
use embassy_time::Timer;

#[cfg(feature = "field-log")]
use crate::spi_flash::{self, SpiFlash};
use crate::spi_flash::{FLASH, SECTOR};

/// The top half of the 4 MiB SPI flash: about two days of debug-level logs.
pub const REGION: core::ops::Range<u32> = 0x20_0000..0x40_0000;
pub const SECTORS: u32 = (REGION.end - REGION.start) / SECTOR;

const MAGIC: u32 = u32::from_le_bytes(*b"urL1");
const HEADER: u32 = 16;

/// Where the log is being written: (sector index, bytes used including the
/// header), once the writer has started.
static CURSOR: BlockingMutex<CriticalSectionRawMutex, Cell<Option<(u32, u32)>>> =
    BlockingMutex::new(Cell::new(None));

// ==============================================================================
// Draining defmt's RTT buffer
// ==============================================================================
//
// Only in field builds; the rest of the module (reading the log back, reset
// reasons, panics) is in every build, so a probe-flashed build can still
// serve an earlier field build's log.

#[cfg(feature = "field-log")]
/// defmt-rtt's control block (`_SEGGER_RTT`), as laid out in defmt-rtt 1.3:
/// SEGGER's standard header with one up channel.
#[repr(C)]
struct RttHeader {
    id: [u8; 16],
    max_up_channels: u32,
    max_down_channels: u32,
    up: RttChannel,
}

#[cfg(feature = "field-log")]
#[repr(C)]
struct RttChannel {
    name: *const u8,
    buffer: *mut u8,
    size: u32,
    write: AtomicU32,
    read: AtomicU32,
    flags: AtomicU32,
}

#[cfg(feature = "field-log")]
unsafe extern "C" {
    static _SEGGER_RTT: RttHeader;
}

#[cfg(feature = "field-log")]
fn rtt() -> &'static RttChannel {
    // SAFETY: defmt-rtt defines this static; we only touch the fields a
    // debug probe would (the read cursor and the mode), which are atomics.
    unsafe { &_SEGGER_RTT.up }
}

#[cfg(feature = "field-log")]
/// How many bytes are waiting in the RTT buffer.
fn waiting() -> usize {
    let channel = rtt();
    let size = channel.size as usize;
    let write = channel.write.load(Ordering::Acquire) as usize;
    let read = channel.read.load(Ordering::Relaxed) as usize;
    (write + size - read) % size
}

#[cfg(feature = "field-log")]
/// Copies whatever whole frames are waiting in the RTT buffer to `out`, and
/// marks them read. Returns how many bytes were copied.
fn drain(out: &mut [u8]) -> usize {
    let channel = rtt();
    let size = channel.size as usize;
    let write = channel.write.load(Ordering::Acquire) as usize;
    let read = channel.read.load(Ordering::Relaxed) as usize;
    let waiting = (write + size - read) % size;
    let n = waiting.min(out.len());
    for (i, byte) in out[..n].iter_mut().enumerate() {
        // SAFETY: Bytes between the read and write cursors are written and
        // left alone by defmt-rtt until we move the read cursor past them.
        *byte = unsafe { channel.buffer.add((read + i) % size).read_volatile() };
    }
    // Frames end in a zero byte; anything after the last one is a frame
    // still being written (or one too big for `out`, which we drop rather
    // than stall on).
    let take = match out[..n].iter().rposition(|&b| b == 0) {
        Some(end) => end + 1,
        None if n == out.len() => {
            channel
                .read
                .store(((read + n) % size) as u32, Ordering::Release);
            return 0;
        }
        None => 0,
    };
    channel
        .read
        .store(((read + take) % size) as u32, Ordering::Release);
    take
}

// ==============================================================================
// Writing to flash
// ==============================================================================

fn sector_address(index: u32) -> u32 {
    REGION.start + index * SECTOR
}

#[cfg(feature = "field-log")]
/// Finds the newest sector's sequence number, so this boot continues after
/// it.
async fn newest(flash: &mut SpiFlash) -> Option<(u32, u32)> {
    let mut newest: Option<(u32, u32)> = None;
    for index in 0..SECTORS {
        let mut header = [0u8; 8];
        flash.read(sector_address(index), &mut header).await;
        let [m0, m1, m2, m3, s0, s1, s2, s3] = header;
        if u32::from_le_bytes([m0, m1, m2, m3]) != MAGIC {
            continue;
        }
        let sequence = u32::from_le_bytes([s0, s1, s2, s3]);
        if newest.is_none_or(|(_, newest)| sequence > newest) {
            newest = Some((index, sequence));
        }
    }
    newest
}

#[cfg(feature = "field-log")]
/// Erases sector `index` and starts it with a header.
async fn open(flash: &mut SpiFlash, index: u32, sequence: u32) -> Result<(), spi_flash::Error> {
    let address = sector_address(index);
    flash.erase(address..address + SECTOR).await?;
    let version = crate::firmware::version_word();
    let mut header = [0xFF; HEADER as usize];
    header[0..4].copy_from_slice(&MAGIC.to_le_bytes());
    header[4..8].copy_from_slice(&sequence.to_le_bytes());
    header[8..12].copy_from_slice(&version.to_le_bytes());
    // Bytes 12..16, the length, stay erased until the sector is closed.
    flash.program(address, &header).await
}

#[cfg(feature = "field-log")]
/// Programs `data` at `address`, across page boundaries as needed.
async fn program(
    flash: &mut SpiFlash,
    mut address: u32,
    mut data: &[u8],
) -> Result<(), spi_flash::Error> {
    while !data.is_empty() {
        let room = (spi_flash::PAGE - address % spi_flash::PAGE) as usize;
        let (now, later) = data.split_at(room.min(data.len()));
        flash.program(address, now).await?;
        address += now.len() as u32;
        data = later;
    }
    Ok(())
}

#[cfg(feature = "field-log")]
/// Drains the RTT buffer into the flash, forever. Spawn once, in field
/// builds only.
#[embassy_executor::task]
pub async fn task() {
    // Skip messages that don't fit (rather than truncating them, defmt-rtt's
    // default, which would leave a corrupt frame in the stream). Mode 0 is
    // SEGGER's "no block, skip".
    rtt().flags.store(0, Ordering::Relaxed);

    let (mut sector, mut sequence) = {
        let mut flash = FLASH.lock().await;
        let flash = flash.as_mut().expect("set at boot");
        if let Err(e) = flash.wake().await {
            warn!("field log: no SPI flash: {}", e);
            return;
        }
        let (sector, sequence) = match newest(flash).await {
            Some((index, sequence)) => ((index + 1) % SECTORS, sequence.wrapping_add(1)),
            None => (0, 0),
        };
        if let Err(e) = open(flash, sector, sequence).await {
            warn!("field log: starting a sector failed: {}", e);
            return;
        }
        flash.sleep().await;
        (sector, sequence)
    };
    let mut used = HEADER;
    CURSOR.lock(|c| c.set(Some((sector, used))));
    info!("field log: writing sector {} (#{})", sector, sequence);

    let mut buffer = [0u8; 512];
    let mut last_flush = embassy_time::Instant::now();
    loop {
        Timer::after_secs(1).await;
        // Each flush wakes the SPI flash and programs a (partial) page, which
        // costs about the same for 10 bytes as for 256. At debug level the
        // log grows ~10 bytes a second, so flushing every second was ~10×
        // the energy of flushing full pages. Wait for a page's worth, or 30 s
        // (so a reset loses little), whichever comes first; the 1 KiB RTT
        // buffer holds far more than either.
        if waiting() < spi_flash::PAGE as usize
            && last_flush.elapsed() < embassy_time::Duration::from_secs(30)
        {
            continue;
        }
        // Stay off the flash during an update: the update's page writes,
        // which hold up its Bluetooth traffic, shouldn't wait behind ours
        // (an erase can take 300 ms). What's logged meanwhile is dropped
        // once the RTT buffer fills.
        if crate::dfu::PROGRESS.lock(Cell::get).is_some() {
            continue;
        }
        last_flush = embassy_time::Instant::now();
        let n = drain(&mut buffer);
        if n == 0 {
            continue;
        }
        let mut flash = FLASH.lock().await;
        let flash = flash.as_mut().expect("set at boot");
        if flash.wake().await.is_err() {
            continue;
        }
        // Whole frames only per sector, so every sector decodes on its own.
        for frame in buffer[..n].split_inclusive(|&b| b == 0) {
            if used + frame.len() as u32 > SECTOR {
                let address = sector_address(sector);
                let _ = flash
                    .program(address + 12, &(used - HEADER).to_le_bytes())
                    .await;
                sector = (sector + 1) % SECTORS;
                sequence = sequence.wrapping_add(1);
                if open(flash, sector, sequence).await.is_err() {
                    // Leave it; the reader trims at frame boundaries.
                    continue;
                }
                used = HEADER;
            }
            if program(flash, sector_address(sector) + used, frame)
                .await
                .is_ok()
            {
                used += frame.len() as u32;
            }
        }
        flash.sleep().await;
        CURSOR.lock(|c| c.set(Some((sector, used))));
    }
}

// ==============================================================================
// Reading it back
// ==============================================================================

/// A sector's header, for the fetcher.
pub struct SectorInfo {
    pub sequence: u32,
    /// The image version that wrote it (MCUBoot's `ih_ver` major, minor,
    /// revision, as one little-endian word); see `firmware::version_word`.
    pub version: u32,
    /// Bytes of frames after the header, as far as known: for the sector
    /// being written, what's written so far; for a sector a reset cut off,
    /// all of it (the reader trims).
    pub length: u32,
}

pub async fn sector_info(index: u32) -> Option<SectorInfo> {
    if index >= SECTORS {
        return None;
    }
    let mut flash = FLASH.lock().await;
    let flash = flash.as_mut().expect("set at boot");
    flash.wake().await.ok()?;
    let mut header = [0u8; HEADER as usize];
    flash.read(sector_address(index), &mut header).await;
    flash.sleep().await;
    let word = |i: usize| u32::from_le_bytes(header[i..i + 4].try_into().expect("4 bytes"));
    if word(0) != MAGIC {
        return None;
    }
    let length = match CURSOR.lock(Cell::get) {
        Some((current, used)) if current == index => used - HEADER,
        _ if word(12) == u32::MAX => SECTOR - HEADER,
        _ => word(12).min(SECTOR - HEADER),
    };
    Some(SectorInfo {
        sequence: word(4),
        version: word(8),
        length,
    })
}

/// Reads frame bytes from sector `index`, starting `offset` bytes after its
/// header.
pub async fn read(index: u32, offset: u32, data: &mut [u8]) {
    let mut flash = FLASH.lock().await;
    let flash = flash.as_mut().expect("set at boot");
    if flash.wake().await.is_err() {
        return;
    }
    flash
        .read(sector_address(index) + HEADER + offset, data)
        .await;
    flash.sleep().await;
}

// ==============================================================================
// Reset reasons and panics
// ==============================================================================

/// Logs why the chip last reset, and where the previous run panicked if it
/// did. Call once at boot, after the clock (which logs) is restored.
pub fn report_last_reset() {
    let power = embassy_nrf::pac::POWER;
    let reason = power.resetreas().read().0;
    // The bits accumulate until cleared (writing 1s clears them).
    power
        .resetreas()
        .write_value(embassy_nrf::pac::power::regs::Resetreas(reason));
    // RESETPIN, DOG, SREQ (a soft reset: ours, or the bootloader's), LOCKUP,
    // OFF, LPCOMP, DIF (debug interface), NFC; 0 means power-on.
    info!(
        "reset reason: {=u32:#x} (1 pin, 2 watchdog, 4 soft, 8 lockup; 0 power-on)",
        reason
    );

    // SAFETY: Reserved RAM (see `memory.x`), only touched here (once, before
    // any task runs) and by the panic handler (which never returns).
    let crumb = unsafe { breadcrumb().read_volatile() };
    unsafe { breadcrumb().write_volatile(Breadcrumb::EMPTY) };
    if crumb.version != crate::firmware::version_word() {
        return;
    }
    if crumb.magic == WATCHDOG_MAGIC {
        // `file` and `file_len` hold the hung code's PC and LR; look them up
        // with `llvm-addr2line -fe <the ELF>`.
        warn!(
            "the previous run hung: watchdog at pc {=u32:#010x}, lr {=u32:#010x}",
            crumb.file as u32, crumb.file_len
        );
        return;
    }
    if crumb.magic != BREADCRUMB_MAGIC {
        return;
    }
    // The file name points into our own flash; same image (checked above),
    // so it's still there.
    let in_flash = |address: usize| (0x8100..0x7C000).contains(&address);
    let file = if in_flash(crumb.file as usize) && crumb.file_len < 256 {
        // SAFETY: Same image, so this is the `&'static str` the panic
        // handler saw.
        unsafe {
            core::str::from_utf8_unchecked(core::slice::from_raw_parts(
                crumb.file,
                crumb.file_len as usize,
            ))
        }
    } else {
        "?"
    };
    warn!("the previous run panicked at {}:{}", file, crumb.line);
}

/// Where a panic happened, kept across the reset that follows it.
#[repr(C)]
#[derive(Clone, Copy)]
struct Breadcrumb {
    magic: u32,
    version: u32,
    file: *const u8,
    file_len: u32,
    line: u32,
}

impl Breadcrumb {
    const EMPTY: Self = Self {
        magic: 0,
        version: 0,
        file: core::ptr::null(),
        file_len: 0,
        line: 0,
    };
}

const BREADCRUMB_MAGIC: u32 = u32::from_le_bytes(*b"urP1");
/// A watchdog bite, rather than a panic (see [`catch_watchdog`]).
const WATCHDOG_MAGIC: u32 = u32::from_le_bytes(*b"urW1");
/// Must match the `RETAINED` region in `memory.x`.
const BREADCRUMB_ADDRESS: usize = 0x2000_D440;
const _: () = assert!(size_of::<Breadcrumb>() <= 32);

fn breadcrumb() -> *mut Breadcrumb {
    BREADCRUMB_ADDRESS as *mut Breadcrumb
}

/// Asks the watchdog to interrupt just before it resets the chip (two
/// 32 kHz cycles before), so [`watchdog_bite`] can note where the CPU was
/// stuck. Call once, after the watchdog is set up.
///
/// The interrupt gets priority 1, above all of ours (2), so it can preempt
/// a hung handler of ours too; priority 1 is otherwise unused (Nordic
/// reserves it, and this only runs at death). A hang inside a critical
/// section (which masks it) still goes unrecorded.
pub fn catch_watchdog() {
    use embassy_nrf::interrupt::{self, InterruptExt, Priority};
    embassy_nrf::pac::WDT.intenset().write(|w| w.set_timeout(true));
    interrupt::WDT.set_priority(Priority::P1);
    // SAFETY: `WDT` below is the handler; it only writes the breadcrumb.
    unsafe { interrupt::WDT.enable() };
}

/// The watchdog's interrupt: hands the stack pointer at entry (where the
/// hardware saved the interrupted code's registers) to `watchdog_bite`.
#[unsafe(naked)]
#[unsafe(no_mangle)]
unsafe extern "C" fn WDT() {
    core::arch::naked_asm!("mrs r0, msp", "b {bite}", bite = sym watchdog_bite);
}

/// Records the interrupted code's PC and LR (frame words 6 and 5) for the
/// next boot, then waits for the reset, ~60 µs away.
extern "C" fn watchdog_bite(frame: *const u32) -> ! {
    // SAFETY: `frame` is the exception frame the hardware just pushed.
    let (pc, lr) = unsafe { (frame.add(6).read(), frame.add(5).read()) };
    let crumb = Breadcrumb {
        magic: WATCHDOG_MAGIC,
        version: crate::firmware::version_word(),
        file: pc as *const u8,
        file_len: lr,
        line: 0,
    };
    // SAFETY: See `report_last_reset`.
    unsafe { breadcrumb().write_volatile(crumb) };
    loop {
        cortex_m::asm::nop();
    }
}

/// Our panic handler. Logs the panic (so a probe, or the field log's next
/// boot, sees it), leaves a breadcrumb for [`report_last_reset`], and resets,
/// rather than waiting for the watchdog with the watch frozen.
///
/// The breadcrumb matters for field builds: the panic's own log message is
/// in RAM that the reset clears before the field log can save it.
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    cortex_m::interrupt::disable();
    if let Some(location) = info.location() {
        let crumb = Breadcrumb {
            magic: BREADCRUMB_MAGIC,
            version: crate::firmware::version_word(),
            file: location.file().as_ptr(),
            file_len: location.file().len() as u32,
            line: location.line(),
        };
        // SAFETY: See `report_last_reset`.
        unsafe { breadcrumb().write_volatile(crumb) };
    }
    defmt::error!("{}", defmt::Display2Format(info));
    // With a debugger attached, stop where we are so it can look.
    let dhcsr = cortex_m::peripheral::DCB::PTR;
    // SAFETY: Reading DHCSR is side-effect free.
    if unsafe { (*dhcsr).dhcsr.read() } & 1 != 0 {
        cortex_m::asm::bkpt();
    }
    cortex_m::peripheral::SCB::sys_reset()
}
