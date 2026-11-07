//! Steps and movement, from the BMA421 accelerometer.
//!
//! Two things come out of the accelerometer:
//!
//! - **Steps**, counted by the chip itself. The BMA421 has a small
//!   "feature engine" (a microcontroller programmed by a 6 KB blob Bosch
//!   publishes) that runs a step counter at very low power, so we never see
//!   the raw motion behind it. We read its running count and keep today's
//!   total.
//! - **Movement**: how much the wrist moved each minute, which is what sleep
//!   tracking runs on. The chip queues accelerometer samples in its FIFO at
//!   12.5 Hz; every few seconds we drain it and add up how far the
//!   acceleration *vector* moved between samples. Using the vector (not just
//!   its length) matters: rolling over in bed changes the direction of
//!   gravity but not its strength.
//!
//! Both are kept per minute for the last day ([`History`]), which is what
//! Gadgetbridge fetches as activity samples, and today's step total is
//! shown on the clock face. The history also holds each minute's heart
//! rate, recorded by [`crate::heart`], and the heart-rate measurement asks
//! [`movement`] whether the wrist is still enough to trust.

use core::cell::{Cell, RefCell};
use core::sync::atomic::{AtomicBool, AtomicU16, Ordering};

use defmt::{info, warn};
use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_time::{Duration, Timer, with_timeout};
use embedded_hal_async::i2c::I2c;

use crate::i2c::Device;
use crate::{clock, sleep};

// ==============================================================================
// The BMA421
// ==============================================================================
//
// Register addresses and sequences follow Bosch's BMA4 SensorAPI (bma4.c,
// bma423.c), as used by InfiniTime for this chip.

const ADDRESS: u8 = 0x18;
const REG_CHIP_ID: u8 = 0x00;
/// The current acceleration, x/y/z (LSB first), 12 bits left-justified.
const REG_ACC_DATA: u8 = 0x12;
const REG_STEP_COUNT: u8 = 0x1E;
const REG_FIFO_LENGTH: u8 = 0x24;
const REG_FIFO_DATA: u8 = 0x26;
const REG_INTERNAL_STATUS: u8 = 0x2A;
const REG_ACC_CONF: u8 = 0x40;
const REG_ACC_RANGE: u8 = 0x41;
const REG_FIFO_DOWNSAMPLING: u8 = 0x45;
const REG_FIFO_CONFIG_1: u8 = 0x49;
const REG_INIT_CTRL: u8 = 0x59;
const REG_ASIC_LSB: u8 = 0x5B;
const REG_ASIC_MSB: u8 = 0x5C;
const REG_FEATURES: u8 = 0x5E;
const REG_POWER_CONF: u8 = 0x7C;
const REG_POWER_CTRL: u8 = 0x7D;
const REG_CMD: u8 = 0x7E;
const CMD_FIFO_FLUSH: u8 = 0xB0;

const CHIP_ID: u8 = 0x11;
/// Far longer than any transfer here takes (the longest, 252 bytes of FIFO
/// at 400 kHz, takes ~6 ms).
const I2C_TIMEOUT: Duration = Duration::from_millis(100);
/// Feature-engine configuration: Bosch's BMA423 config file, which the
/// BMA421 also runs. See `bma423_config.LICENSE` (BSD-3-Clause).
static CONFIG: &[u8; 6144] = include_bytes!("bma423_config.bin");
/// The feature-configuration window behind `REG_FEATURES`.
const FEATURES_LEN: usize = 70;
/// Within it: the step counter's settings (offset 0x3A) high byte, whose
/// bit 4 enables counting.
const STEP_COUNTER_ENABLE: (usize, u8) = (0x3A + 1, 0x10);
/// Upload granularity: the config goes over I2C in pieces this size, each
/// preceded by its position (in 16-bit words). Even, and small enough to
/// stage in RAM (the TWIM can't send from flash).
const CHUNK: usize = 32;

struct Bma421 {
    i2c: Device,
}

impl Bma421 {
    async fn write(&mut self, register: u8, value: u8) -> Result<(), ()> {
        self.write_bytes(&[register, value]).await
    }

    /// Writes `[register, data…]`. Every transfer has a timeout: a wedged
    /// bus must not wedge this task (or, via the shared bus, touch).
    async fn write_bytes(&mut self, bytes: &[u8]) -> Result<(), ()> {
        match with_timeout(I2C_TIMEOUT, self.i2c.write(ADDRESS, bytes)).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => {
                warn!("activity: write to {=u8:#04x} failed: {}", bytes[0], e);
                crate::i2c::reset_controller().await;
                Err(())
            }
            Err(_) => {
                warn!("activity: write to {=u8:#04x} timed out", bytes[0]);
                crate::i2c::reset_controller().await;
                Err(())
            }
        }
    }

    async fn read(&mut self, register: u8, buffer: &mut [u8]) -> Result<(), ()> {
        match with_timeout(
            I2C_TIMEOUT,
            self.i2c.write_read(ADDRESS, &[register], buffer),
        )
        .await
        {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => {
                warn!("activity: read of {=u8:#04x} failed: {}", register, e);
                crate::i2c::reset_controller().await;
                Err(())
            }
            Err(_) => {
                warn!("activity: read of {=u8:#04x} timed out", register);
                crate::i2c::reset_controller().await;
                Err(())
            }
        }
    }

    /// Brings the chip from any state (including whatever InfiniTime left
    /// behind) to counting steps and filling the FIFO.
    async fn init(&mut self) -> Result<(), ()> {
        let mut id = [0];
        self.read(REG_CHIP_ID, &mut id).await?;
        if id[0] != CHIP_ID {
            warn!(
                "activity: unexpected accelerometer chip ID {=u8:#04x}",
                id[0]
            );
            return Err(());
        }

        // Soft reset, so every boot starts from the same state. (It also
        // zeroes the step counter; today's total survives in RAM instead.)
        self.write(REG_CMD, 0xB6).await?;
        Timer::after_millis(5).await;

        // Upload the feature engine's program: advanced power save off
        // (the chip can't take writes this fast otherwise), loading off,
        // stream the blob, loading on.
        self.write(REG_POWER_CONF, 0x00).await?;
        // The datasheet asks for 450 µs after a write in power-save mode; right
        // after a soft reset, the next write still stalled at that gap.
        Timer::after_millis(2).await;
        self.write(REG_INIT_CTRL, 0x00).await?;
        let mut staging = [0u8; CHUNK + 1];
        for (i, chunk) in CONFIG.chunks(CHUNK).enumerate() {
            let word = i * CHUNK / 2;
            self.write(REG_ASIC_LSB, (word & 0x0F) as u8).await?;
            self.write(REG_ASIC_MSB, (word >> 4) as u8).await?;
            staging[0] = REG_FEATURES;
            staging[1..=chunk.len()].copy_from_slice(chunk);
            self.write_bytes(&staging[..=chunk.len()]).await?;
        }
        self.write(REG_INIT_CTRL, 0x01).await?;
        Timer::after_millis(150).await;
        let mut status = [0];
        self.read(REG_INTERNAL_STATUS, &mut status).await?;
        if status[0] & 0x1F != 0x01 {
            warn!(
                "activity: feature engine didn't start (status {=u8:#04x})",
                status[0]
            );
            return Err(());
        }

        // Enable the step counter. The feature window is read and written
        // whole; right after initialisation, the address registers point at
        // its start (as Bosch's driver assumes for full-length accesses).
        let mut features = [0u8; FEATURES_LEN + 1];
        self.read(REG_FEATURES, &mut features[1..]).await?;
        features[0] = REG_FEATURES;
        features[1 + STEP_COUNTER_ENABLE.0] |= STEP_COUNTER_ENABLE.1;
        self.write_bytes(&features).await?;

        // Accelerometer: 100 Hz (what the step counter wants), ±2 g,
        // averaging 4 samples, low-power averaging mode, as InfiniTime does.
        self.write(REG_ACC_CONF, 0x28).await?;
        self.write(REG_ACC_RANGE, 0x00).await?;
        // FIFO: filtered accelerometer data only, downsampled 8× to 12.5 Hz
        // (87.5 bytes/s with headers; the 1 KB FIFO holds 11 s).
        self.write(REG_FIFO_DOWNSAMPLING, 0x80 | (3 << 4)).await?;
        // Accelerometer frames, with headers (see `drain_fifo`).
        self.write(REG_FIFO_CONFIG_1, 0x50).await?;
        self.write(REG_POWER_CTRL, 0x04).await?;
        // Advanced power save back on (with FIFO self-wake-up), now that
        // configuration is done.
        self.write(REG_POWER_CONF, 0x03).await?;
        Ok(())
    }

    /// Steps since the last reset (i.e. since boot).
    async fn steps(&mut self) -> Result<u32, ()> {
        let mut bytes = [0u8; 4];
        self.read(REG_STEP_COUNT, &mut bytes).await?;
        Ok(u32::from_le_bytes(bytes))
    }

    /// Reads queued samples, calling `on_sample` with each (x, y, z) in
    /// 12-bit counts (1024 per g).
    ///
    /// One I2C transfer only, so at most 36 frames (252 bytes: the nRF52832's
    /// TWIM moves at most 255 bytes per transfer); anything beyond stays
    /// queued for next time. A second back-to-back burst doesn't work: with
    /// advanced power save on, the chip returns filler (`e0 e0 …`) without
    /// removing frames from the FIFO, which looked like data arriving at
    /// twice the configured rate.
    ///
    /// The chip's advanced power save corrupts FIFO reads: bytes go missing
    /// (no I2C error is reported). Headerless, every later frame then came
    /// out shifted, as nonsense (axes rotating, impossible values), and the
    /// movement measure (which sleep tracking and the heart rate motion
    /// veto rely on) was wrong from the first glitch until the next reset.
    /// So power save is suspended around the read (see below), and frames
    /// carry headers anyway: a bad header means a broken batch, which is
    /// dropped and the FIFO flushed, losing a few seconds of samples
    /// instead of everything after.
    async fn drain_fifo(&mut self, mut on_sample: impl FnMut([i16; 3])) -> Result<(), ()> {
        let mut length = [0u8; 2];
        self.read(REG_FIFO_LENGTH, &mut length).await?;
        let queued = usize::from(u16::from_le_bytes(length) & 0x3FFF);
        let mut buffer = [0u8; 252];
        let take = queued.min(buffer.len());
        if take == 0 {
            return Ok(());
        }
        // Advanced power save off for the read, and the 450 µs the chip
        // then needs before the next access. With it on, the FIFO stream
        // came out corrupted (missing frame headers) in most reads of a
        // still watch on the desk; with it off, none were, over minutes.
        self.write(REG_POWER_CONF, 0x00).await?;
        Timer::after_millis(1).await;
        let read = self.read(REG_FIFO_DATA, &mut buffer[..take]).await;
        Timer::after_micros(500).await;
        self.write(REG_POWER_CONF, 0x03).await?;
        read?;

        // Parse first, deliver after, so a broken batch delivers nothing.
        let mut frames = heapless::Vec::<[i16; 3], 36>::new();
        let mut i = 0;
        let broken = loop {
            let Some(&header) = buffer[..take].get(i) else {
                break false;
            };
            match header & 0xFC {
                // Accelerometer data.
                0x84 => {
                    let Some(data) = buffer[..take].get(i + 1..i + 7) else {
                        // A frame cut off by the end of our read: it stays
                        // in the FIFO only if we haven't read it, and we
                        // have, so this batch's tail is lost; resync.
                        break true;
                    };
                    let axis = |a: usize| i16::from_le_bytes([data[a], data[a + 1]]) >> 4;
                    let _ = frames.push([axis(0), axis(2), axis(4)]);
                    i += 7;
                }
                // Skip (frames dropped by overflow), configuration change,
                // or "dropped" frames: one parameter byte.
                0x40 | 0x48 | 0x50 => i += 2,
                // Sensor time: three bytes.
                0x44 => i += 4,
                // Empty FIFO.
                0x80 => break false,
                _ => break true,
            }
        };
        if broken {
            warn!(
                "activity: FIFO out of step (header {=u8:#04x} at {}); flushing",
                buffer[i.min(take - 1)],
                i
            );
            self.write(REG_CMD, CMD_FIFO_FLUSH).await?;
            return Ok(());
        }
        for frame in frames {
            on_sample(frame);
        }
        Ok(())
    }
}

// ==============================================================================
// Shared state
// ==============================================================================

/// Per-minute samples for the last 24 hours.
const HISTORY_MINUTES: usize = 24 * 60;

#[derive(Clone, Copy, Default)]
pub struct Sample {
    pub steps: u16,
    /// Average change of the acceleration vector between samples, in
    /// Bangle.js units (8192 per g), since that's what Gadgetbridge expects.
    pub movement: u16,
    /// The heart rate measured during the minute, or 0 if none was (as in
    /// Gadgetbridge, which treats 0 as "not measured").
    pub heart_rate: u8,
    /// Packed into the byte the struct would otherwise pad with (24 hours
    /// of samples cost the same 8.6 KB either way): the minute's
    /// [`sleep::Kind`] (bits 0–3), whether the watch was worn (bits 4–5: 0
    /// unknown, 1 yes, 2 no), and whether the minute was recorded at all
    /// (bit 6).
    flags: u8,
}

impl Sample {
    const SEEN: u8 = 0x40;

    pub fn seen(&self) -> bool {
        self.flags & Self::SEEN != 0
    }

    pub fn sleep(&self) -> sleep::Kind {
        sleep::Kind::from_bits(self.flags & 0x0F)
    }

    fn worn(&self) -> Option<bool> {
        match (self.flags >> 4) & 0x3 {
            1 => Some(true),
            2 => Some(false),
            _ => None,
        }
    }

    fn set_seen(&mut self, worn: Option<bool>) {
        let worn = match worn {
            None => 0,
            Some(true) => 1,
            Some(false) => 2,
        };
        self.flags = (self.flags & 0x0F) | Self::SEEN | (worn << 4);
    }

    fn set_sleep(&mut self, kind: sleep::Kind) {
        self.flags = (self.flags & 0xF0) | kind as u8;
    }

    fn evidence(&self) -> sleep::Minute {
        sleep::Minute {
            seen: self.seen(),
            movement: self.movement,
            steps: self.steps,
            worn: self.worn(),
        }
    }
}

const _: () = assert!(size_of::<Sample>() == 6, "flags must fit in the padding");

/// A ring of per-minute samples, indexed by UTC minute. Minutes the watch
/// didn't see (off, or before the phone set the clock) stay zero.
pub struct History {
    samples: [Sample; HISTORY_MINUTES],
    /// The newest minute with a sample (UTC minutes since 1970).
    newest: Option<i64>,
}

impl History {
    const fn new() -> Self {
        Self {
            samples: [Sample {
                steps: 0,
                movement: 0,
                heart_rate: 0,
                flags: 0,
            }; HISTORY_MINUTES],
            newest: None,
        }
    }

    fn slot(minute: i64) -> usize {
        minute.rem_euclid(HISTORY_MINUTES as i64) as usize
    }

    /// The sample for `minute`, clearing any skipped minutes (and stale
    /// samples from a day ago) on the way.
    fn at(&mut self, minute: i64) -> &mut Sample {
        let newest = self.newest.unwrap_or(minute - 1);
        if minute > newest {
            let first_new = (newest + 1).max(minute - HISTORY_MINUTES as i64 + 1);
            for m in first_new..=minute {
                self.samples[Self::slot(m)] = Sample::default();
            }
            self.newest = Some(minute);
        }
        &mut self.samples[Self::slot(minute)]
    }

    /// Samples from `since` (UTC minute) on, oldest first, skipping minutes
    /// without data.
    pub fn since(&self, since: i64) -> impl Iterator<Item = (i64, Sample)> + '_ {
        let newest = self.newest.unwrap_or(i64::MIN);
        let oldest = newest.saturating_sub(HISTORY_MINUTES as i64 - 1).max(since);
        (oldest..=newest)
            .map(|m| (m, self.samples[Self::slot(m)]))
            .filter(|(_, s)| s.seen() || s.heart_rate > 0)
    }

    /// Re-classifies the recent past's sleep (see [`sleep`]): the last
    /// [`sleep::WINDOW`] minutes, with [`sleep::CONTEXT`] more before them
    /// for context.
    fn classify_sleep(&mut self) {
        const SPAN: usize = sleep::WINDOW + sleep::CONTEXT;
        let Some(newest) = self.newest else {
            return;
        };
        let start = newest - SPAN as i64 + 1;
        let before = if newest - (start - 1) < HISTORY_MINUTES as i64 {
            self.samples[Self::slot(start - 1)].sleep()
        } else {
            sleep::Kind::Unknown
        };
        let mut evidence = [sleep::Minute::default(); SPAN];
        for (i, minute) in evidence.iter_mut().enumerate() {
            *minute = self.samples[Self::slot(start + i as i64)].evidence();
        }
        let mut kinds = [sleep::Kind::Unknown; SPAN];
        sleep::classify(before, &evidence, &mut kinds);
        for (i, kind) in kinds.iter().enumerate().skip(sleep::CONTEXT) {
            self.samples[Self::slot(start + i as i64)].set_sleep(*kind);
        }
    }

    /// Whether the newest minute is classified as asleep (light or deep).
    fn asleep_now(&self) -> bool {
        self.newest.is_some_and(|minute| {
            matches!(
                self.samples[Self::slot(minute)].sleep(),
                sleep::Kind::Light | sleep::Kind::Deep
            )
        })
    }

    /// The most recent sleep in the history (see [`sleep::last_sleep`]).
    fn last_sleep(&self) -> Option<Sleep> {
        let newest = self.newest?;
        let oldest = newest - HISTORY_MINUTES as i64 + 1;
        let mut kinds = [sleep::Kind::Unknown; HISTORY_MINUTES];
        for (i, kind) in kinds.iter_mut().enumerate() {
            *kind = self.samples[Self::slot(oldest + i as i64)].sleep();
        }
        let (start, end, asleep) = sleep::last_sleep(&kinds)?;
        Some(Sleep {
            start_utc_minute: oldest + start as i64,
            end_utc_minute: oldest + end as i64,
            asleep_minutes: asleep as u16,
        })
    }
}

/// A night's (or nap's) sleep.
#[derive(Clone, Copy, PartialEq, Eq, defmt::Format)]
pub struct Sleep {
    pub start_utc_minute: i64,
    /// The minute after the last asleep one.
    pub end_utc_minute: i64,
    /// Minutes actually asleep (light or deep) in between.
    pub asleep_minutes: u16,
}

static LAST_SLEEP: Mutex<CriticalSectionRawMutex, Cell<Option<Sleep>>> =
    Mutex::new(Cell::new(None));

/// Whether the wearer is asleep, as of the last minute's classification.
/// Falling asleep is recognised once [`sleep::ONSET`] minutes of stillness
/// have passed, waking after [`sleep::WAKE_RUN`] minutes of movement (or
/// at once on getting up and walking), so this lags both by that much.
pub fn asleep() -> bool {
    ASLEEP.load(Ordering::Relaxed)
}

static ASLEEP: AtomicBool = AtomicBool::new(false);

/// The most recent sleep in the last 24 hours, for the UI; updated every
/// minute.
pub fn last_sleep() -> Option<Sleep> {
    LAST_SLEEP.lock(Cell::get)
}

struct Today {
    /// Local date, as days since 1970.
    day: i64,
    steps: u32,
}

pub static HISTORY: Mutex<CriticalSectionRawMutex, RefCell<History>> =
    Mutex::new(RefCell::new(History::new()));
static TODAY: Mutex<CriticalSectionRawMutex, RefCell<Option<Today>>> =
    Mutex::new(RefCell::new(None));

/// Records a heart-rate reading in the current minute's sample (if the
/// time is known). A later reading in the same minute replaces it.
///
/// TODO: average the readings within a minute, for the continuous
/// measurement while the clock face is showing.
pub fn record_heart_rate(bpm: u8) {
    let Some(utc_minute) = clock::now_utc_seconds().map(|s| s.div_euclid(60)) else {
        return;
    };
    HISTORY.lock(|history| history.borrow_mut().at(utc_minute).heart_rate = bpm);
}

/// How much the wrist moved over the last few seconds (one poll of the
/// FIFO), in the same units as [`Sample::movement`].
pub fn movement() -> u16 {
    MOVEMENT.load(Ordering::Relaxed)
}

static MOVEMENT: AtomicU16 = AtomicU16::new(0);

/// Set once the accelerometer is configured; nothing else may talk to it
/// before (setup soft-resets it and uploads its program).
static READY: AtomicBool = AtomicBool::new(false);

// ==============================================================================
// Glances
// ==============================================================================

/// How often to look at the wrist's pose while the screen is off: fast
/// enough that the time is up within ~0.5 s of turning the wrist.
const GLANCE_POLL: Duration = Duration::from_millis(200);

/// Watches for the wrist turning to look at the watch (see
/// [`crate::glance`]) while the screen is off, and asks the UI for a
/// glance.
///
/// Reads the current acceleration (6 bytes) rather than the FIFO: one
/// short transfer, no power save dance. A misread in power save shows up
/// as an implausible magnitude, which the detector ignores.
#[embassy_executor::task]
pub async fn glance_task(i2c: Device) {
    let mut bma = Bma421 { i2c };
    let mut detector = crate::glance::Detector::default();
    let mut screen_was_on = true;
    loop {
        Timer::after(GLANCE_POLL).await;
        if !READY.load(Ordering::Relaxed) {
            continue;
        }
        let screen_on = crate::glance::screen_on();
        if screen_on != screen_was_on {
            detector.reset();
            screen_was_on = screen_on;
        }
        // Asleep, turning over in bed shouldn't light the screen (and the
        // accelerometer needn't be read five times a second all night). The
        // button and a tap still wake it. On waking, glances return once the
        // classifier has seen it: after a few minutes of movement, or at
        // once on getting up and walking.
        if screen_on || asleep() {
            detector.reset();
            continue;
        }
        let mut data = [0u8; 6];
        if bma.read(REG_ACC_DATA, &mut data).await.is_err() {
            continue;
        }
        let axis = |i: usize| i16::from_le_bytes([data[i], data[i + 1]]) >> 4;
        let sample = crate::glance::from_sensor([axis(0), axis(2), axis(4)]);
        if detector.push(sample) {
            let _ = crate::INPUT.try_send(crate::InputEvent::Glance);
        }
    }
}

/// Today's steps, for the clock face.
pub fn steps_today() -> Option<u32> {
    let today = clock::now()?.days_since_epoch();
    TODAY.lock(|t| {
        t.borrow()
            .as_ref()
            .filter(|t| t.day == today)
            .map(|t| t.steps)
    })
}

// ==============================================================================
// The task
// ==============================================================================

/// How often to drain the FIFO and read the step counter. At 12.5 Hz × 6
/// bytes, 3 s fills 225 bytes, inside one 252-byte read (see `drain_fifo`).
const POLL_SECONDS: u64 = 3;

#[embassy_executor::task]
pub async fn activity_task(i2c: Device) {
    restore_today();
    // Let the touch controller be configured first: it only listens for a
    // moment after its reset, and our config upload holds the shared bus
    // for about a third of a second.
    Timer::after_millis(200).await;
    let mut bma = Bma421 { i2c };
    // Setup occasionally fails partway (a write to the feature window ends
    // early, or a transfer stalls), maybe two times in ten; a fresh attempt
    // from soft reset has always worked.
    // TODO: find the cause; it may be the nRF52832's TWIM errata, or the
    // touch controller on the same bus.
    let mut attempt = 0;
    while bma.init().await.is_err() {
        attempt += 1;
        if attempt == 5 {
            warn!("activity: accelerometer setup failed; no steps or movement");
            return;
        }
        warn!("activity: accelerometer setup failed; retrying");
        Timer::after_millis(100).await;
    }
    info!("activity: accelerometer running");
    READY.store(true, Ordering::Relaxed);

    let mut last_count = 0u32;
    // The previous sample, to measure movement against.
    let mut previous: Option<[i16; 3]> = None;
    // Movement accumulated within the current minute.
    let (mut minute, mut movement_sum, mut movement_samples) = (None::<i64>, 0u64, 0u32);

    loop {
        Timer::after_secs(POLL_SECONDS).await;

        let mut moved = 0u64;
        let mut samples = 0u32;
        let drained = bma
            .drain_fifo(|now| {
                if let Some(before) = previous {
                    let d = |i: usize| f32::from(now[i]) - f32::from(before[i]);
                    moved += libm::sqrtf(d(0) * d(0) + d(1) * d(1) + d(2) * d(2)) as u64;
                    samples += 1;
                }
                previous = Some(now);
            })
            .await;
        let count = bma.steps().await;
        let (Ok(()), Ok(count)) = (drained, count) else {
            warn!("activity: reading the accelerometer failed");
            continue;
        };
        let new_steps = count.wrapping_sub(last_count);
        last_count = count;
        // The last sample too: which way up the watch is (1024 per g, in
        // the sensor's own axes), for orientation questions such as how
        // it's mounted, or sleep position.
        let [x, y, z] = previous.unwrap_or_default();
        defmt::debug!(
            "activity: {} samples, movement {}, {} new steps (counter {}), last ({}, {}, {})",
            samples,
            moved * 8 / u64::from(samples.max(1)),
            new_steps,
            count,
            x,
            y,
            z
        );

        add_steps_today(new_steps);
        // Counts (1024 per g) to Bangle.js units (8192 per g).
        let recent = (moved * 8 / u64::from(samples.max(1))).min(u64::from(u16::MAX)) as u16;
        MOVEMENT.store(recent, Ordering::Relaxed);

        // Per-minute history needs wall-clock time.
        let Some(utc_minute) = clock::now_utc_seconds().map(|s| s.div_euclid(60)) else {
            continue;
        };
        let new_minute = minute != Some(utc_minute);
        if new_minute {
            (minute, movement_sum, movement_samples) = (Some(utc_minute), 0, 0);
        }
        movement_sum += moved;
        movement_samples += samples;
        // Counts (1024 per g) to Bangle.js units (8192 per g).
        let movement =
            (movement_sum * 8 / u64::from(movement_samples.max(1))).min(u64::from(u16::MAX)) as u16;
        HISTORY.lock(|history| {
            let mut history = history.borrow_mut();
            let sample = history.at(utc_minute);
            sample.steps = sample
                .steps
                .saturating_add(new_steps.min(u32::from(u16::MAX)) as u16);
            sample.movement = movement;
            sample.set_seen(crate::heart::worn());
        });
        // Once a minute: revise the recent past's sleep, and the summary.
        if new_minute {
            let (summary, changed, asleep) = HISTORY.lock(|history| {
                let mut history = history.borrow_mut();
                history.classify_sleep();
                let summary = history.last_sleep();
                (summary, summary != LAST_SLEEP.lock(Cell::get), history.asleep_now())
            });
            if asleep != ASLEEP.swap(asleep, Ordering::Relaxed) {
                info!("sleep: {}", if asleep { "asleep now" } else { "awake now" });
            }
            if changed && let Some(sleep) = summary {
                info!(
                    "sleep: {} min asleep, minutes {}..{} (UTC)",
                    sleep.asleep_minutes, sleep.start_utc_minute, sleep.end_utc_minute
                );
            }
            LAST_SLEEP.lock(|s| s.set(summary));
        }
    }
}

/// `Today::day` before the clock is known.
const UNKNOWN_DAY: i64 = i64::MIN;

/// Adds steps to today's total, starting a new day at local midnight.
fn add_steps_today(steps: u32) {
    let day = clock::now().map(|now| now.days_since_epoch());
    TODAY.lock(|today| {
        let mut today = today.borrow_mut();
        match (&mut *today, day) {
            // The date just became known: steps so far were today's.
            (Some(t), Some(day)) if t.day == UNKNOWN_DAY => {
                t.day = day;
                t.steps += steps;
            }
            (Some(t), Some(day)) if t.day != day => *t = Today { day, steps },
            (Some(t), _) => t.steps += steps,
            // Steps before the clock is known count towards whatever day
            // it turns out to be.
            (None, day) => {
                *today = Some(Today {
                    day: day.unwrap_or(UNKNOWN_DAY),
                    steps,
                })
            }
        }
        if let Some(t) = &*today {
            save_today(t);
        }
    });
}

// ==============================================================================
// Surviving resets
// ==============================================================================
//
// The step counter restarts whenever we reset the chip (every boot), so
// today's total is kept in reset-surviving RAM, like the clock (see
// `clock`): the first 32 bytes of the retained RAM in `memory.x`.

#[repr(C)]
struct Retained {
    magic: u32,
    steps: u32,
    day: i64,
    check: i64,
}

const RETAINED_MAGIC: u32 = 0x7572_5331; // "urS1"
/// Must match the `RETAINED` region in `memory.x`.
const RETAINED_ADDRESS: usize = 0x2000_D400;
const _: () = assert!(size_of::<Retained>() <= 32);

fn check(today: &Today) -> i64 {
    !(today.day ^ (i64::from(today.steps) << 24))
}

fn save_today(today: &Today) {
    let value = Retained {
        magic: RETAINED_MAGIC,
        steps: today.steps,
        day: today.day,
        check: check(today),
    };
    // SAFETY: Reserved, aligned RAM (see `memory.x`), only touched from
    // this task (and `restore_today` before the loop).
    unsafe { (RETAINED_ADDRESS as *mut Retained).write_volatile(value) };
}

fn restore_today() {
    // SAFETY: As in `save_today`; any bit pattern is a valid `Retained`,
    // and the magic and check fields say whether it's real.
    let saved = unsafe { (RETAINED_ADDRESS as *const Retained).read_volatile() };
    let today = Today {
        day: saved.day,
        steps: saved.steps,
    };
    if saved.magic == RETAINED_MAGIC && saved.check == check(&today) {
        info!("activity: resumed with {} steps today", today.steps);
        TODAY.lock(|t| *t.borrow_mut() = Some(today));
    }
}
