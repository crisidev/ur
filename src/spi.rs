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

#[cfg(feature = "game")]
pub use chained::{Chained, write_chained};

/// Long writes without a task wake-up a piece (see [`write_chained`]).
#[cfg(feature = "game")]
mod chained {
    use core::future::poll_fn;
    use core::sync::atomic::{AtomicBool, AtomicU32, Ordering, compiler_fence};
    use core::task::Poll;

    use embassy_nrf::interrupt::typelevel::{Handler, SPI2};
    use embassy_nrf::pac::SPIM2;
    use embassy_nrf::spim::Spim;
    use embassy_sync::waitqueue::AtomicWaker;

    /// The most EasyDMA moves at a time on the nRF52832 (MAXCNT's 8 bits).
    const PIECE: u32 = 255;

    /// Whether a chained write is under way, which [`Chained`] then sees
    /// through; where its next piece starts, and the bytes after that.
    static ACTIVE: AtomicBool = AtomicBool::new(false);
    static NEXT: AtomicU32 = AtomicU32::new(0);
    static LEFT: AtomicU32 = AtomicU32::new(0);
    static WAKER: AtomicWaker = AtomicWaker::new();

    /// SPIM2's interrupt, ahead of embassy-nrf's own handler (see
    /// `main`'s `Irqs`): while a chained write is under way, each piece's
    /// end starts the next, right here, and the last wakes the writer.
    /// Otherwise it leaves everything to embassy-nrf's.
    pub struct Chained;

    impl Handler<SPI2> for Chained {
        unsafe fn on_interrupt() {
            if !ACTIVE.load(Ordering::Acquire) || SPIM2.events_end().read() == 0 {
                return;
            }
            // Ours, so embassy-nrf's handler, next, sees nothing to do.
            SPIM2.events_end().write_value(0);
            SPIM2.events_started().write_value(0);
            let left = LEFT.load(Ordering::Relaxed);
            if left == 0 {
                SPIM2.intenclr().write(|w| w.set_end(true));
                ACTIVE.store(false, Ordering::Release);
                WAKER.wake();
                return;
            }
            let piece = left.min(PIECE);
            let next = NEXT.load(Ordering::Relaxed);
            NEXT.store(next + piece, Ordering::Relaxed);
            LEFT.store(left - piece, Ordering::Relaxed);
            SPIM2.dma().tx().ptr().write_value(next);
            SPIM2
                .dma()
                .tx()
                .maxcnt()
                .write(|w| w.set_maxcnt(piece as _));
            // Started by a CPU that's awake, as embassy-nrf starts all but
            // a transfer's first piece (see `write_chained`).
            SPIM2.tasks_start().write_value(1);
        }
    }

    /// Sends `data`, which must be in RAM, as [`super::write`] does, but
    /// each piece of it EasyDMA can take started by SPIM's interrupt
    /// ([`Chained`]) the moment the last ends, rather than by the task
    /// after a wake-up: on the watch, those cost about 70 µs a piece,
    /// 255 bytes' worth of the bus. Only for the game's long writes; the
    /// caller holds the bus (`bus` says so).
    pub async fn write_chained(bus: &mut Spim<'static>, data: &[u8]) {
        // Stops a write cut short (its future dropped), before the bus's
        // lock goes to anyone else.
        struct Stop;
        impl Drop for Stop {
            fn drop(&mut self) {
                if ACTIVE.swap(false, Ordering::AcqRel) {
                    SPIM2.intenclr().write(|w| w.set_end(true));
                    SPIM2.events_stopped().write_value(0);
                    SPIM2.tasks_stop().write_value(1);
                    while SPIM2.events_stopped().read() == 0 {}
                    SPIM2.events_stopped().write_value(0);
                    SPIM2.events_end().write_value(0);
                }
            }
        }

        let _ = bus;
        if data.is_empty() {
            return;
        }
        let start = data.as_ptr() as u32;
        // RAM, where EasyDMA reads: 0x2000_0000 up.
        assert!(start >= 0x2000_0000, "EasyDMA reads only RAM");
        let first = (data.len() as u32).min(PIECE);
        NEXT.store(start + first, Ordering::Relaxed);
        LEFT.store(data.len() as u32 - first, Ordering::Relaxed);
        compiler_fence(Ordering::SeqCst);

        // As embassy-nrf starts a transfer, for the nRF52832's anomaly
        // 109 (DMA's first access may be corrupted): a transfer of
        // nothing, then, once it's started, the real one. Waited for here
        // rather than by interrupt: it takes no time.
        SPIM2.dma().rx().maxcnt().write(|w| w.set_maxcnt(0));
        SPIM2.dma().tx().ptr().write_value(start);
        SPIM2.dma().tx().maxcnt().write(|w| w.set_maxcnt(0));
        SPIM2.events_started().write_value(0);
        SPIM2.events_end().write_value(0);
        SPIM2.tasks_start().write_value(1);
        while SPIM2.events_started().read() == 0 {}
        // The empty transfer's own end, if it has one, before it's
        // cleared: embassy-nrf gets here only after an interrupt and a
        // wake-up, a good deal later than this.
        cortex_m::asm::delay(320);
        SPIM2.events_started().write_value(0);
        SPIM2.events_end().write_value(0);
        SPIM2
            .dma()
            .tx()
            .maxcnt()
            .write(|w| w.set_maxcnt(first as _));

        let stop = Stop;
        ACTIVE.store(true, Ordering::Release);
        SPIM2.intenset().write(|w| w.set_end(true));
        SPIM2.tasks_start().write_value(1);
        poll_fn(|cx| {
            WAKER.register(cx.waker());
            if ACTIVE.load(Ordering::Acquire) {
                Poll::Pending
            } else {
                Poll::Ready(())
            }
        })
        .await;
        core::mem::forget(stop);
        compiler_fence(Ordering::SeqCst);
    }
}
