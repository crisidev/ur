//! The internal flash, erased and written in MPSL timeslots.
//!
//! While the NVMC erases a page or writes a word, the CPU stalls (it runs
//! from that flash), so the radio would miss its events: every erase and
//! write must happen in time MPSL (Nordic's radio scheduler) hands out as a
//! *timeslot*. MPSL signals a slot's start to a callback, at its highest
//! interrupt priority, and the callback erases or writes until the slot's
//! time is up. Running past the end is fatal: MPSL signals `OVERSTAYED`,
//! then asserts, and the watch resets.
//!
//! nrf-mpsl has such a driver (`nrf_mpsl::Flash`), which this replaces,
//! because on the nRF52832 it can reset or hang the watch:
//!
//! - Its state sits behind a lock that masks the TIMER0 interrupt, on the
//!   premise that MPSL signals a slot's start from there. MPSL also does
//!   from RTC0's interrupt; when that lands while the driver holds the lock
//!   (it does whenever its future is polled), the callback finds the state
//!   borrowed and panics. On a dev kit saving back to back, within ~70
//!   saves. Here, the callback and the driver never use the state at the
//!   same time instead (see [`SLOT`]), so there's nothing to lock.
//! - It books 85 ms (+ 1 ms) for a page erase: the nRF52840's figure. The
//!   nRF52832's product specification allows up to 89.7 ms (see
//!   [`ERASE_PAGE_US`]).
//!
//! With nrf-mpsl's driver, a save on the dev kit also once never finished.
//! That was first blamed on its waking the future from the slot's
//! interrupt, but embassy-executor's run queue is lock-free on this core,
//! so a wake from any priority is safe; the hang is unexplained. Here,
//! MPSL's signal that the session went idle, which comes in thread mode,
//! wakes the future, which is safe either way.
//!
//! A debugger can still make a slot overrun: probe-rs halts the core for
//! ~20 ms each time a session (an `esprobe` command) starts and ends, and
//! a halt in a slot outlasts it. The steps can't budget for that: the
//! halt can come before the callback runs (MPSL resets TIMER0 as it
//! signals the start, so a late start looks fresh) or after it returns.
//! (On the dev kit, a 2-word write took 146 µs by TIMER0, and 152 µs in
//! real time, with a probe attached, and still overstayed its 8.5 ms
//! slot.) A watch without a
//! probe never halts, but a dev kit driven through one (`ur-sim --devkit`,
//! `screenshot.py --input`) did, every few taps: an injected input's save
//! started while the `core write` that injected it was still attached,
//! and its detach halted a slot. So erases and writes first wait for a
//! probe's session to end (see [`probe_detached`]).
//!
//! TODO: go back to nrf-mpsl's driver once a release has the lock fix
//! (on its `main` since alexmoon/nrf-sdc#124; release requested in
//! alexmoon/nrf-sdc#92) and the nRF52832's erase time
//! (alexmoon/nrf-sdc#126), keeping the wait for a probe around it.

use core::cell::{RefCell, UnsafeCell};
use core::future::poll_fn;
use core::sync::atomic::{AtomicI32, AtomicU32, Ordering, compiler_fence};
use core::task::Poll;

use defmt::{info, warn};
use embassy_nrf::Peri;
use embassy_nrf::pac;
use embassy_nrf::pac::nvmc::vals::Wen;
use embassy_nrf::peripherals::NVMC;
use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::waitqueue::WakerRegistration;
use embassy_time::{Duration, Instant};
use embedded_storage_async::nor_flash::{
    ErrorType, MultiwriteNorFlash, NorFlash, NorFlashError, NorFlashErrorKind, ReadNorFlash,
};
use nrf_sdc::mpsl::{self, MultiprotocolServiceLayer, RetVal, raw};

// ==============================================================================
// Timing
// ==============================================================================

const PAGE: u32 = 4096;
const SIZE: u32 = 0x80000;

/// The longest a page erase takes (nRF52832 product specification,
/// NVMC, tERASEPAGE). There's no partial erase on this chip, so a slot
/// must fit a whole one. The dev kit erases a page in 2.04 ms (the spec's
/// typical 2.05), but the slot is for the worst case: an overrun resets
/// the watch, while booking a long slot only makes it harder to place
/// between the radio's events, about once every few hundred saves.
const ERASE_PAGE_US: u32 = 89_700;
/// The longest a word write takes (tWRITE).
const WRITE_WORD_US: u32 = 338;
/// How long a write's slot is; as many words as fit go in each. Short, so
/// MPSL can fit it between the radio's events.
const WRITE_SLOT_US: u32 = 7_500;
/// What a slot needs beyond the flash's own time: MPSL calls us a little
/// after the slot starts, and we need a moment before and after the erase or
/// the writes.
const SLACK_US: u32 = 1_000;
/// How long a normal-priority request may wait for its slot. If it can't
/// get one, we ask again at high priority, waiting as long as it takes.
const TIMEOUT_US: u32 = 30_000;

/// Slots don't need the crystal running: the flash doesn't use the radio.
/// (Erases took the same 2.04 ms on the dev kit with the crystal
/// guaranteed as without.)
const HFCLK: u8 = raw::MPSL_TIMESLOT_HFCLK_CFG_NO_GUARANTEE as u8;

// ==============================================================================
// The driver
// ==============================================================================

/// Why an erase or write failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, defmt::Format)]
pub enum Error {
    /// MPSL refused a timeslot request or session.
    Mpsl(mpsl::Error),
    OutOfBounds,
    Unaligned,
}

impl From<mpsl::Error> for Error {
    fn from(e: mpsl::Error) -> Self {
        Self::Mpsl(e)
    }
}

impl NorFlashError for Error {
    fn kind(&self) -> NorFlashErrorKind {
        match self {
            Self::Mpsl(_) => NorFlashErrorKind::Other,
            Self::OutOfBounds => NorFlashErrorKind::OutOfBounds,
            Self::Unaligned => NorFlashErrorKind::NotAligned,
        }
    }
}

/// The internal flash. Reads are plain memory reads; erases and writes wait
/// for timeslots.
pub struct Flash {
    // Held, not used: timeslots need MPSL (initialised `with_timeslots`),
    // and nothing else may drive the NVMC.
    _mpsl: &'static MultiprotocolServiceLayer<'static>,
    _nvmc: Peri<'static, NVMC>,
}

impl Flash {
    pub fn new(
        mpsl: &'static MultiprotocolServiceLayer<'static>,
        nvmc: Peri<'static, NVMC>,
    ) -> Self {
        Self {
            _mpsl: mpsl,
            _nvmc: nvmc,
        }
    }

    /// Runs `op` in as many timeslots of `slot_us` as it takes; returns its
    /// longest step.
    async fn run(&mut self, op: Op, slot_us: u32) -> Result<Step, Error> {
        if op.is_done() {
            return Ok(Step::default());
        }
        probe_detached().await;
        // A cancelled operation's session closes in the background; wait
        // for it (see `Session`), as MPSL has room for only one.
        poll_fn(|cx| {
            WAITING.lock(|waiting| {
                let mut waiting = waiting.borrow_mut();
                waiting.waker.register(cx.waker());
                if waiting.session_open {
                    Poll::Pending
                } else {
                    Poll::Ready(())
                }
            })
        })
        .await;

        let mut id = 0;
        // SAFETY: The callback is valid for the program's lifetime, and
        // `id` outlives the call.
        RetVal::from(unsafe { raw::mpsl_timeslot_session_open(Some(on_signal), &mut id) })
            .to_result()?;
        WAITING.lock(|waiting| waiting.borrow_mut().session_open = true);
        let _session = Session(id);

        // SAFETY: The session has no request yet, so no slot can start
        // (see `SLOT`).
        let slot = unsafe { &mut *SLOT.0.get() };
        slot.op = op;
        slot.step = Step::default();
        slot.request.request_type = raw::MPSL_TIMESLOT_REQ_TYPE_EARLIEST as u8;
        slot.request.params.earliest = raw::mpsl_timeslot_request_earliest_t {
            hfclk: HFCLK,
            priority: raw::MPSL_TIMESLOT_PRIORITY_NORMAL as u8,
            length_us: slot_us + SLACK_US,
            timeout_us: TIMEOUT_US,
        };
        STATUS.store(RUNNING, Ordering::Relaxed);
        compiler_fence(Ordering::SeqCst);
        // SAFETY: The request lives in a static, which MPSL may read until
        // the session closes.
        RetVal::from(unsafe { raw::mpsl_timeslot_request(id, &slot.request) }).to_result()?;

        poll_fn(|cx| {
            WAITING.lock(|waiting| waiting.borrow_mut().waker.register(cx.waker()));
            match STATUS.load(Ordering::Acquire) {
                RUNNING => Poll::Pending,
                DONE => Poll::Ready(Ok(())),
                code => Poll::Ready(RetVal::new(code).to_result().map(drop)),
            }
        })
        .await?;
        // SAFETY: Done: the last slot ended, and no other was requested.
        Ok(unsafe { (*SLOT.0.get()).step })
    }
}

/// An open timeslot session, closed when dropped: when its operation is
/// done, or if the future running it is dropped part-way. Closing cancels
/// any slot requested and not started (one can't be running: we're not in
/// its interrupt), so the operation's pointers aren't used again.
struct Session(u8);

impl Drop for Session {
    fn drop(&mut self) {
        // SAFETY: The session was opened and isn't closed yet. Failing
        // would mean it's already closed, which is just as good.
        let _ = unsafe { raw::mpsl_timeslot_session_close(self.0) };
    }
}

impl ErrorType for Flash {
    type Error = Error;
}

impl ReadNorFlash for Flash {
    const READ_SIZE: usize = 1;

    async fn read(&mut self, offset: u32, bytes: &mut [u8]) -> Result<(), Error> {
        if offset as usize + bytes.len() > SIZE as usize {
            return Err(Error::OutOfBounds);
        }
        // SAFETY: In bounds of the flash, which is always readable (reads
        // during an erase or write just stall).
        let flash = unsafe { core::slice::from_raw_parts(offset as *const u8, bytes.len()) };
        bytes.copy_from_slice(flash);
        Ok(())
    }

    fn capacity(&self) -> usize {
        SIZE as usize
    }
}

impl NorFlash for Flash {
    const WRITE_SIZE: usize = 4;
    const ERASE_SIZE: usize = PAGE as usize;

    async fn erase(&mut self, from: u32, to: u32) -> Result<(), Error> {
        // Only the settings map erases internal flash, and only its own
        // pages: anything below them is the bootloader or the running
        // firmware, and erasing that leaves a sealed watch with nothing
        // to boot. A bug elsewhere must not be able to.
        if from > to || to > SIZE || from < crate::storage::RANGE.start {
            return Err(Error::OutOfBounds);
        }
        if !from.is_multiple_of(PAGE) || !to.is_multiple_of(PAGE) {
            return Err(Error::Unaligned);
        }
        let op = Op::Erase { address: from, to };
        let step = self.run(op, ERASE_PAGE_US).await?;
        // Erases are rare (one per few hundred saves), so worth a line:
        // a field log shows how close they come to their slot.
        info!(
            "flash: erased {=u32:#x}..{=u32:#x}, longest page {} us ({} by TIMER0)",
            from,
            to,
            step.micros(),
            step.timer0_us
        );
        #[cfg(feature = "flash-stress")]
        crate::flash_stress::erased(step.micros(), step.timer0_us);
        Ok(())
    }

    async fn write(&mut self, offset: u32, bytes: &[u8]) -> Result<(), Error> {
        if offset as usize + bytes.len() > SIZE as usize {
            return Err(Error::OutOfBounds);
        }
        if !offset.is_multiple_of(4) || !bytes.len().is_multiple_of(4) {
            return Err(Error::Unaligned);
        }
        let op = Op::Write {
            dest: offset as *mut u32,
            src: bytes.as_ptr().cast(),
            words: (bytes.len() / 4) as u32,
        };
        let step = self.run(op, WRITE_SLOT_US).await?;
        if step.micros() > WRITE_SLOT_US {
            warn!("flash: a write step took {} us", step.micros());
        }
        Ok(())
    }
}

// The flash can write a word twice between erases, as long as the second
// write only clears bits (which is all `sequential-storage` asks of it).
impl MultiwriteNorFlash for Flash {}

// ==============================================================================
// Debug probes
// ==============================================================================

/// How long an erase or write waits for a probe's session to end, in ms.
///
/// The dev kit takes an injected input within 100 ms of its `esprobe core
/// write` landing, and THEME's save then waited for that session to
/// detach on a third of the taps, for up to 55 ms. A session that stays
/// longer (`esprobe rtt`, as `cargo run` keeps open) doesn't halt the core
/// until it ends, so there's no point waiting for it any longer.
const PROBE_WAIT_MS: u32 = 1_000;

/// When an erase or write first found the probe attached now (ms since
/// boot), or 0 if it isn't. Once that's [`PROBE_WAIT_MS`] ago, the probe
/// is staying, and erases and writes go ahead without waiting.
///
/// A static, not the future's own state, so the wait costs no RAM in the
/// many futures a save is nested in (it was 112 B with a timer in each).
static PROBE_SEEN_MS: AtomicU32 = AtomicU32::new(0);

/// Waits, up to [`PROBE_WAIT_MS`], for a debug probe's session to end (see
/// the module docs): probe-rs sets `C_DEBUGEN` as it attaches and clears
/// it as it detaches, and a halt only comes with either. A sealed watch
/// never has it set, so never waits.
async fn probe_detached() {
    poll_fn(|cx| {
        if !probe_attached() {
            PROBE_SEEN_MS.store(0, Ordering::Relaxed);
            return Poll::Ready(());
        }
        let now = Instant::now();
        // Never 0, which means "not attached".
        let now_ms = (now.as_millis() as u32).max(1);
        let seen = match PROBE_SEEN_MS.load(Ordering::Relaxed) {
            0 => {
                PROBE_SEEN_MS.store(now_ms, Ordering::Relaxed);
                now_ms
            }
            seen => seen,
        };
        if now_ms.wrapping_sub(seen) >= PROBE_WAIT_MS {
            return Poll::Ready(());
        }
        // Look again in 10 ms.
        embassy_time_driver::schedule_wake(
            (now + Duration::from_millis(10)).as_ticks(),
            cx.waker(),
        );
        Poll::Pending
    })
    .await
}

/// Whether a debug probe has a session open (`C_DEBUGEN`).
fn probe_attached() -> bool {
    // SAFETY: Reading DHCSR is side-effect free.
    unsafe { (*cortex_m::peripheral::DCB::PTR).dhcsr.read() & 1 != 0 }
}

// ==============================================================================
// Inside the timeslots
// ==============================================================================

/// What's left of the operation in progress.
enum Op {
    None,
    Erase {
        /// The next page to erase.
        address: u32,
        /// Where to stop (exclusive).
        to: u32,
    },
    Write {
        dest: *mut u32,
        src: *const u32,
        words: u32,
    },
}

impl Op {
    fn is_done(&self) -> bool {
        match *self {
            Self::None => true,
            Self::Erase { address, to } => address >= to,
            Self::Write { words, .. } => words == 0,
        }
    }

    /// Does as much as one slot allows: one page, or as many words as fit.
    /// Returns whether the operation is done.
    ///
    /// Always does at least one page or word: each slot is booked long
    /// enough for that, so the operation can't stall.
    fn step(&mut self) -> bool {
        let nvmc = pac::NVMC;
        let ready = || while !nvmc.ready().read().ready() {};
        match self {
            Self::None => {}
            Self::Erase { address, .. } => {
                nvmc.config().write(|w| w.set_wen(Wen::Een));
                ready();
                nvmc.erasepage().write_value(*address);
                ready();
                *address += PAGE;
            }
            Self::Write { dest, src, words } => {
                nvmc.config().write(|w| w.set_wen(Wen::Wen));
                ready();
                loop {
                    // SAFETY: `write` checked `dest` is in the flash and
                    // aligned, and `src` points into the caller's buffer,
                    // which outlives the operation (dropping the future
                    // closes the session first).
                    unsafe {
                        let word = src.read_unaligned();
                        dest.write_volatile(word);
                        ready();
                        *src = src.add(1);
                        *dest = dest.add(1);
                    }
                    *words -= 1;
                    // TIMER0 counts the slot from its start, in µs.
                    if *words == 0 || timer0_us() + WRITE_WORD_US >= WRITE_SLOT_US {
                        break;
                    }
                }
            }
        }
        // Back to read-only, which also re-enables the instruction cache.
        nvmc.config().write(|w| w.set_wen(Wen::Ren));
        ready();
        self.is_done()
    }
}

/// How long a step took, measured two ways.
#[derive(Clone, Copy, Default)]
struct Step {
    /// By RTC1 (embassy's time driver, on the 32.768 kHz crystal), in its
    /// ticks: real time, whatever clocks the NVMC.
    rtc_ticks: u32,
    /// TIMER0 at the step's end: how far into the slot MPSL's timer has
    /// counted (it runs on the high-frequency clock, as the NVMC does).
    timer0_us: u32,
}

impl Step {
    fn micros(self) -> u32 {
        self.rtc_ticks * 15_625 / 512
    }
}

/// Microseconds since the slot started. Only valid inside a slot, where
/// MPSL has started TIMER0 for us.
fn timer0_us() -> u32 {
    let timer = pac::TIMER0;
    timer.tasks_capture(0).write_value(1);
    timer.cc(0).read()
}

fn rtc_ticks() -> u32 {
    pac::RTC1.counter().read().counter()
}

/// MPSL's timeslot callback: at a slot's start, in MPSL's highest-priority
/// interrupts (TIMER0's or RTC0's; nothing of ours can preempt them), and
/// for the rest in `mpsl_task`, in thread mode.
unsafe extern "C" fn on_signal(
    session: u8,
    signal: u32,
) -> *mut raw::mpsl_timeslot_signal_return_param_t {
    match signal {
        raw::MPSL_TIMESLOT_SIGNAL_START => {
            // SAFETY: In a slot, the callback owns `SLOT`.
            let slot = unsafe { &mut *SLOT.0.get() };
            let start = rtc_ticks();
            let done = slot.op.step();
            let step = Step {
                // RTC1 is 24 bits wide.
                rtc_ticks: rtc_ticks().wrapping_sub(start) & 0xFF_FFFF,
                timer0_us: timer0_us(),
            };
            if step.rtc_ticks > slot.step.rtc_ticks {
                slot.step = step;
            }
            if done {
                slot.action.callback_action = raw::MPSL_TIMESLOT_SIGNAL_ACTION_END as u8;
                // The driver wakes on the session going idle, which MPSL
                // signals from thread mode: waking it from here could race
                // the executor (see `WAITING`).
                STATUS.store(DONE, Ordering::Release);
            } else {
                // The next slot, at normal priority again if the last one
                // needed high.
                slot.action.callback_action = raw::MPSL_TIMESLOT_SIGNAL_ACTION_REQUEST as u8;
                slot.request.params.earliest.priority = raw::MPSL_TIMESLOT_PRIORITY_NORMAL as u8;
                slot.request.params.earliest.timeout_us = TIMEOUT_US;
                slot.action.params.request.p_next = &mut slot.request;
            }
            &mut slot.action
        }
        // The radio needed the time we asked for: ask again, at high
        // priority, and without a deadline.
        raw::MPSL_TIMESLOT_SIGNAL_BLOCKED | raw::MPSL_TIMESLOT_SIGNAL_CANCELLED => {
            // SAFETY: Nothing is requested, so no slot can start.
            let slot = unsafe { &mut *SLOT.0.get() };
            slot.request.params.earliest.priority = raw::MPSL_TIMESLOT_PRIORITY_HIGH as u8;
            slot.request.params.earliest.timeout_us = raw::MPSL_TIMESLOT_EARLIEST_TIMEOUT_MAX_US;
            // SAFETY: The request lives in a static.
            let ret = unsafe { raw::mpsl_timeslot_request(session, &slot.request) };
            if ret < 0 {
                STATUS.store(ret, Ordering::Release);
                WAITING.lock(|waiting| waiting.borrow_mut().waker.wake());
            }
            core::ptr::null_mut()
        }
        raw::MPSL_TIMESLOT_SIGNAL_SESSION_IDLE => {
            WAITING.lock(|waiting| waiting.borrow_mut().waker.wake());
            core::ptr::null_mut()
        }
        raw::MPSL_TIMESLOT_SIGNAL_SESSION_CLOSED => {
            WAITING.lock(|waiting| {
                let mut waiting = waiting.borrow_mut();
                waiting.session_open = false;
                waiting.waker.wake();
            });
            core::ptr::null_mut()
        }
        // MPSL asserts right after this, whatever we return; this just
        // says why. A probe's halt is the one cause seen so far (see the
        // module docs), so say if one could have halted the core.
        raw::MPSL_TIMESLOT_SIGNAL_OVERSTAYED => {
            if probe_attached() {
                panic!("flash: overstayed a timeslot, with a probe attached")
            }
            panic!("flash: overstayed a timeslot")
        }
        _ => core::ptr::null_mut(),
    }
}

// ==============================================================================
// Shared state
// ==============================================================================

/// The operation, and what MPSL reads to schedule its slots.
///
/// Shared between the driver and the slot-start callback without a lock,
/// by taking turns: the driver only touches it while no slot can start
/// (before requesting the first, after the last has ended, or when a
/// request was blocked and not yet renewed), and the callback only in a
/// slot. A lock couldn't do better: the callback runs at MPSL's highest
/// priority, from more than one interrupt, which our critical sections
/// (MPSL's, which leave its radio interrupts running) don't mask.
struct Slot {
    op: Op,
    /// The longest step of the operation, so far.
    step: Step,
    /// The request for the next slot, which MPSL reads from here.
    request: raw::mpsl_timeslot_request_t,
    /// What the callback returns to MPSL.
    action: raw::mpsl_timeslot_signal_return_param_t,
}

struct SlotCell(UnsafeCell<Slot>);

// SAFETY: See `Slot`: the driver and the callback take turns.
unsafe impl Sync for SlotCell {}

static SLOT: SlotCell = SlotCell(UnsafeCell::new(Slot {
    op: Op::None,
    step: Step {
        rtc_ticks: 0,
        timer0_us: 0,
    },
    request: raw::mpsl_timeslot_request_t {
        request_type: raw::MPSL_TIMESLOT_REQ_TYPE_EARLIEST as u8,
        params: raw::mpsl_timeslot_request_t__bindgen_ty_1 {
            earliest: raw::mpsl_timeslot_request_earliest_t {
                hfclk: HFCLK,
                priority: raw::MPSL_TIMESLOT_PRIORITY_NORMAL as u8,
                length_us: 0,
                timeout_us: TIMEOUT_US,
            },
        },
    },
    action: raw::mpsl_timeslot_signal_return_param_t {
        callback_action: 0,
        params: raw::mpsl_timeslot_signal_return_param_t__bindgen_ty_1 {
            request: raw::mpsl_timeslot_signal_return_param_t__bindgen_ty_1__bindgen_ty_1 {
                p_next: core::ptr::null_mut(),
            },
        },
    },
}));

/// The operation's outcome: [`RUNNING`], [`DONE`], or MPSL's (negative)
/// error code.
static STATUS: AtomicI32 = AtomicI32::new(DONE);
const RUNNING: i32 = 1;
const DONE: i32 = 0;

/// What only thread mode touches: the driver, and MPSL's signals other
/// than a slot's start (which `mpsl_task` runs).
struct Waiting {
    waker: WakerRegistration,
    /// Whether a session is open, until MPSL confirms it closed.
    session_open: bool,
}

static WAITING: Mutex<CriticalSectionRawMutex, RefCell<Waiting>> =
    Mutex::new(RefCell::new(Waiting {
        waker: WakerRegistration::new(),
        session_open: false,
    }));
