//! Where the battery goes: the executor's sleep loop (with the nRF52832's
//! FPU erratum worked around), and on-time counters for everything that
//! draws real current, logged once an hour.
//!
//! The counters don't measure current; they measure *time* in each
//! power-hungry state. Multiplied by each state's current (the table in
//! `docs/power-plan.md`), they give an energy budget per hour that can be
//! checked against the battery voltage's slope, and they show which knob
//! to turn next.
//!
//! The same report carries the hour's deepest stack use (see
//! [`stack_peak`]), since an overflow crashes the watch.

use core::sync::atomic::{AtomicU32, Ordering};

use defmt::info;
use embassy_time::Instant;

// ==============================================================================
// Sleeping
// ==============================================================================

/// RTC ticks (32768 Hz) spent in WFE since the last report. ISRs that run
/// while we're "asleep" (the radio's, mostly) count as asleep; this is the
/// thread-mode duty cycle.
static ASLEEP_TICKS: AtomicU32 = AtomicU32::new(0);
/// How often WFE returned since the last report. Thousands per hour is
/// normal (timers, radio events); millions means something keeps waking the
/// CPU, e.g. a pending interrupt nobody services.
static WAKEUPS: AtomicU32 = AtomicU32::new(0);

/// Sleeps until the next event. Call only from the thread-mode executor
/// loop, after polling.
pub fn sleep() {
    // Nordic anomaly 87: once the FPU has raised an exception flag (and
    // "inexact" is raised by almost any arithmetic), its interrupt stays
    // pending and can wake the CPU from WFE straight away. There's no
    // interrupt enable at the peripheral, so clear the flags and the pending
    // bit before every sleep, as the erratum's workaround does.
    //
    // SAFETY: Only the cumulative exception flags (IOC, DZC, OFC, UFC, IXC,
    // IDC) are cleared; rounding mode and the other control bits are kept.
    unsafe {
        let mut fpscr: u32;
        core::arch::asm!("vmrs {}, fpscr", out(reg) fpscr, options(nomem, nostack));
        fpscr &= !0x9F;
        core::arch::asm!("vmsr fpscr, {}", in(reg) fpscr, options(nomem, nostack));
    }
    cortex_m::peripheral::NVIC::unpend(embassy_nrf::pac::Interrupt::FPU);

    let before = Instant::now().as_ticks();
    cortex_m::asm::wfe();
    let slept = Instant::now().as_ticks().wrapping_sub(before);
    ASLEEP_TICKS.fetch_add(slept as u32, Ordering::Relaxed);
    WAKEUPS.fetch_add(1, Ordering::Relaxed);
}

// ==============================================================================
// On-time counters
// ==============================================================================

/// Each power-hungry state we track, in milliseconds per report.
#[derive(Clone, Copy, defmt::Format)]
pub enum Meter {
    /// Panel out of sleep (ST7789 ~5.6 mA).
    Panel,
    /// Backlight on, at any level (1.8–19.6 mA; see [`crate::brightness`]).
    ///
    /// TODO: split by level, if the hourly line is to estimate its current.
    Backlight,
    /// HRS3300 measuring, by LED drive: 12.5, 20, 30, 40 mA settings.
    HeartLed0,
    HeartLed1,
    HeartLed2,
    HeartLed3,
    /// A phone (or other client) connected; see the logged connection
    /// interval for what that costs.
    Connected,
    /// SPI flash out of deep power-down.
    SpiFlash,
    /// The wrist still, so the accelerometer isn't polled at all (see
    /// `activity::still`); the rest of the time, it's read every 0.2 s
    /// (screen off, awake) or every second.
    Still,
    /// Listening for Aranet4s (radio receiving, ~5.5 mA; see
    /// [`crate::aranet`]).
    Scanning,
}

const METERS: usize = 10;
static MILLIS: [AtomicU32; METERS] = [const { AtomicU32::new(0) }; METERS];

/// Adds time spent in `meter`'s state.
pub fn add(meter: Meter, since: Instant) {
    let ms = since.elapsed().as_millis() as u32;
    MILLIS[meter as usize].fetch_add(ms, Ordering::Relaxed);
}

/// Event counts per report.
#[derive(Clone, Copy, defmt::Format)]
pub enum Count {
    WakeButton,
    WakeTouch,
    WakePhone,
    WakeAttention,
    WakeGlance,
    HeartSessions,
    Disconnects,
    /// Scans for Aranet4s.
    Scans,
}

const COUNTS: usize = 8;
static EVENTS: [AtomicU32; COUNTS] = [const { AtomicU32::new(0) }; COUNTS];

pub fn count(event: Count) {
    EVENTS[event as usize].fetch_add(1, Ordering::Relaxed);
}

/// Logs and resets everything. Call about once an hour (from the UI loop),
/// right before the battery line, so the two can be lined up.
pub fn report(period: embassy_time::Duration) {
    let asleep = ASLEEP_TICKS.swap(0, Ordering::Relaxed) as u64;
    let total = period.as_ticks().max(1);
    let take = |a: &AtomicU32| a.swap(0, Ordering::Relaxed);
    info!(
        "power: awake {=u64}‰, {=u32} wakeups; ms: panel {=u32}, backlight {=u32}, \
         led {=u32}/{=u32}/{=u32}/{=u32}, connected {=u32}, flash {=u32}, still {=u32}, scanning {=u32}; \
         wakes: button {=u32}, touch {=u32}, phone {=u32}, attention {=u32}, glance {=u32}; \
         hr sessions {=u32}, disconnects {=u32}, scans {=u32}; stack {=u32} B",
        1000 - asleep.min(total) * 1000 / total,
        WAKEUPS.swap(0, Ordering::Relaxed),
        take(&MILLIS[Meter::Panel as usize]),
        take(&MILLIS[Meter::Backlight as usize]),
        take(&MILLIS[Meter::HeartLed0 as usize]),
        take(&MILLIS[Meter::HeartLed1 as usize]),
        take(&MILLIS[Meter::HeartLed2 as usize]),
        take(&MILLIS[Meter::HeartLed3 as usize]),
        take(&MILLIS[Meter::Connected as usize]),
        take(&MILLIS[Meter::SpiFlash as usize]),
        take(&MILLIS[Meter::Still as usize]),
        take(&MILLIS[Meter::Scanning as usize]),
        take(&EVENTS[Count::WakeButton as usize]),
        take(&EVENTS[Count::WakeTouch as usize]),
        take(&EVENTS[Count::WakePhone as usize]),
        take(&EVENTS[Count::WakeAttention as usize]),
        take(&EVENTS[Count::WakeGlance as usize]),
        take(&EVENTS[Count::HeartSessions as usize]),
        take(&EVENTS[Count::Disconnects as usize]),
        take(&EVENTS[Count::Scans as usize]),
        stack_peak(),
    );
}

// ==============================================================================
// Stack depth
// ==============================================================================

/// The most stack used since the last call (or since boot), in bytes.
///
/// cortex-m-rt paints the whole stack with a known word at boot
/// (`paint-stack`), and whatever the stack has reached since has scrubbed
/// the paint off, so the lowest scrubbed word is the deepest it has been.
/// That word is found by scanning up from the bottom; then the scrubbed part
/// below where the stack is now is repainted, so the next call reports the
/// next period's peak rather than the boot's (building the Bluetooth host
/// takes one of the deepest paths there are, so a peak since boot would
/// hide everything after it).
///
/// It's a lower bound: a frame that reserves space it never writes leaves
/// paint there, and if that's the deepest frame, the scan stops above it.
/// It's ~2700 words read and fewer written, tens of µs, once an hour, and
/// no RAM.
fn stack_peak() -> u32 {
    unsafe extern "C" {
        static _stack_start: u32;
    }
    let top = (&raw const _stack_start) as u32;
    // Above the guard (see `field_log::guard_stack`), which faults on any
    // access.
    let bottom = crate::field_log::STACK_END + crate::field_log::STACK_GUARD;
    let sp = cortex_m::register::msp::read();
    let word = |address: u32| address as *mut u32;
    let mut deepest = bottom;
    // SAFETY: Reads the stack between its guard and where it is now: our
    // RAM, and unused (painted or dead) below `sp`.
    while deepest < sp && unsafe { word(deepest).read_volatile() } == cortex_m_rt::STACK_PAINT_VALUE
    {
        deepest += 4;
    }
    // Below `sp` is free: code only ever uses the stack above it (an
    // interrupt that preempts us here pushes below it, but is gone before
    // we carry on). So it's ours to repaint, from thread mode.
    for address in (deepest..sp).step_by(4) {
        // SAFETY: As above.
        unsafe { word(address).write_volatile(cortex_m_rt::STACK_PAINT_VALUE) };
    }
    top - deepest
}
