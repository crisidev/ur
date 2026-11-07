//! Heart rate, from the HRS3300 optical (PPG) sensor.
//!
//! The sensor's green LED is the single most power-hungry thing on the
//! watch short of the backlight and radio (12.5–40 mA while lit), so it
//! isn't run continuously. Instead, a measurement *session* runs:
//!
//! - every [`INTERVAL`], for up to [`SESSION_LIMIT`] or until it has a
//!   reading (typically ~13 s), and
//! - while the clock face is showing (see [`set_watching`]), continuously,
//!   since then someone is looking at the number.
//!
//! Between sessions the sensor is fully off: no LED, no oscillator.
//!
//! The signal processing lives in [`crate::ppg`], which knows nothing about
//! the hardware; this module drives the sensor, keeps its signal in range
//! ([`LEVELS`]), vetoes measurements while the wrist moves (using
//! [`activity::movement`]), and publishes readings for the clock face
//! ([`current`]) and the per-minute history Gadgetbridge fetches.

use core::cell::Cell;
use core::sync::atomic::{AtomicBool, Ordering};

use defmt::{debug, info, warn};
use embassy_futures::select::select;
use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Instant, Ticker, Timer, with_timeout};
use embedded_hal_async::i2c::I2c;

use crate::activity;
use crate::i2c::Device;
use crate::ppg::{self, Consensus, Conversions, Ppg};

// ==============================================================================
// Scheduling
// ==============================================================================

/// How often to measure in the background.
const INTERVAL: Duration = Duration::from_secs(10 * 60);

/// How long a background session may try before giving up. A clean
/// signal gives a reading after ~13 s (1 s settling, 8 s window, then two
/// more estimates 2 s apart that must agree); this leaves room for a few
/// rejected windows, or the gain adjusting.
const SESSION_LIMIT: Duration = Duration::from_secs(30);

/// How long a reading is shown on the clock face. Past this (a few missed
/// background sessions), "--" is more honest than an old number.
const STALE_AFTER: Duration = Duration::from_secs(30 * 60);

/// Wrist movement (see [`activity::movement`]) above which the PPG can't be
/// trusted: moving the sensor against the skin changes the light far more
/// than the pulse does.
///
/// Measured on the dev kit: ~25 lying untouched, 300–700 with a fingertip
/// resting on the sensor (which still gave a clean, steady 57 BPM), and
/// thousands when handled. 300, the first guess, vetoed that clean signal;
/// 1000 (~0.12 g average change between samples) still rejects handling.
///
/// TODO: tune on the wrist, from field logs.
const MOTION_LIMIT: u16 = 1000;

/// How long after movement windows stay vetoed: one analysis window, plus
/// the accelerometer's poll interval (it reports movement a few seconds
/// late).
const MOTION_HOLD: Duration =
    Duration::from_millis((ppg::WINDOW as u64 * ppg::NOMINAL_PERIOD_US as u64) / 1000 + 3000);

/// Whether the clock face is showing; set by the UI loop.
static WATCHING: AtomicBool = AtomicBool::new(false);
/// Raised when [`WATCHING`] becomes true, to start a session right away.
static WATCH_STARTED: Signal<CriticalSectionRawMutex, ()> = Signal::new();

/// Whether the last session saw skin; for sleep tracking, which must tell
/// a still wrist from a watch on the nightstand.
static WORN: Mutex<CriticalSectionRawMutex, Cell<Option<bool>>> = Mutex::new(Cell::new(None));

/// Whether the watch is on a wrist, as of the last measurement session
/// (`None` before the first). Up to [`INTERVAL`] stale.
pub fn worn() -> Option<bool> {
    WORN.lock(Cell::get)
}

/// The latest reading and when it was taken.
static LATEST: Mutex<CriticalSectionRawMutex, Cell<Option<(u8, Instant)>>> =
    Mutex::new(Cell::new(None));

/// Tells the heart-rate task whether someone's looking at the clock face.
/// Cheap; the UI loop calls it every pass.
pub fn set_watching(watching: bool) {
    let was = WATCHING.swap(watching, Ordering::Relaxed);
    if watching && !was {
        WATCH_STARTED.signal(());
    }
}

fn watching() -> bool {
    WATCHING.load(Ordering::Relaxed)
}

/// The latest heart rate and when it was measured, however old.
pub fn latest() -> Option<(u8, Instant)> {
    LATEST.lock(Cell::get)
}

/// The latest heart rate, if it's recent enough to show.
pub fn current(now: Instant) -> Option<u8> {
    latest()
        .filter(|&(_, at)| now.saturating_duration_since(at) <= STALE_AFTER)
        .map(|(bpm, _)| bpm)
}

fn publish(bpm: u8) {
    LATEST.lock(|latest| latest.set(Some((bpm, Instant::now()))));
    activity::record_heart_rate(bpm);
}

// ==============================================================================
// The HRS3300
// ==============================================================================
//
// Register facts are from the HRS3300 datasheet (TianYiHeXin), and from how
// InfiniTime's and wasp-os's drivers configure it on this watch, which
// fills in what the datasheet leaves vague.

const ADDRESS: u8 = 0x44;
const REG_ID: u8 = 0x00;
/// HEN (bit 7: measure), HWT (bits 6:4: wait between conversions), and
/// PDRIVE[1] (bit 3: LED current, high bit).
const REG_ENABLE: u8 = 0x01;
/// The results: 0x08..=0x0F hold both channels, bits scattered across
/// registers (see `Hrs3300::sample`).
const REG_DATA: u8 = 0x08;
/// PDRIVE[0] (bit 6: LED current, low bit) and PON (bit 5: oscillator on).
const REG_PDRIVER: u8 = 0x0C;
/// ADC resolution: HRS channel in bits 7:4, ALS in bits 3:0.
const REG_RES: u8 = 0x16;
/// HRS gain, in bits 4:2.
const REG_HGAIN: u8 = 0x17;

const CHIP_ID: u8 = 0x21;

const ENABLE_HEN: u8 = 0x80;
/// Wait 12.5 ms between conversions (HWT = 0b110). With 25 ms conversions
/// that's the 37.5 ms period [`ppg::NOMINAL_PERIOD_US`] assumes, ~27 Hz:
/// fine resolution of the beat period (180 BPM is 9 samples), with the LED
/// off a third of the time. (Whether the LED is off during the wait isn't
/// documented.)
const ENABLE_HWT_12_5_MS: u8 = 0x60;
const ENABLE_PDRIVE1: u8 = 0x08;
const PDRIVER_PDRIVE0: u8 = 0x40;
const PDRIVER_PON: u8 = 0x20;
/// The datasheet says these bits are reserved but also recommends 0x8 in
/// them. InfiniTime found that 0xF gives a steady LED drive through each
/// conversion where 0x8 modulates it, so we follow that.
const PDRIVER_LOW_BITS: u8 = 0x0F;
/// 14 bits for both channels: 25 ms per HRS conversion, the datasheet's
/// characterised setting. (InfiniTime uses 15 bits, which takes about twice
/// as long; 14 is plenty for a pulse that's ~1% of the level.)
const RES_14_BIT: u8 = 0x66;
const FULL_SCALE: u32 = (1 << 14) - 1;

/// Far longer than any transfer here takes (8 bytes at 250 kHz: ~0.4 ms).
const I2C_TIMEOUT: Duration = Duration::from_millis(50);

/// How often to look for a new conversion: several times per 37.5 ms
/// period, so each is dated to within a few ms (see [`Conversions`]).
const POLL: Duration = Duration::from_millis(10);

/// One LED current and gain setting.
#[derive(Clone, Copy, defmt::Format)]
struct Level {
    /// PDRIVE: 0..=3 for 12.5, 20, 30, 40 mA.
    drive: u8,
    /// HGAIN: 0..=3 for ×1, ×2, ×4, ×8.
    gain: u8,
}

/// Signal levels, weakest first: [`measure`] steps through these to keep
/// the reflected light well within the ADC's range. Gain goes up first,
/// since it costs no power, while LED current is most of the sensor's
/// draw. The bottom is what InfiniTime runs at fixed (12.5 mA, ×1), so it's
/// known to work on most wrists; the remembered level carries over between
/// sessions.
///
/// TODO: gain ×64 (HGAIN 0b100) exists too; try it if ×8 at 40 mA is ever
/// not enough.
const LEVELS: [Level; 7] = [
    Level { drive: 0, gain: 0 },
    Level { drive: 0, gain: 1 },
    Level { drive: 0, gain: 2 },
    Level { drive: 1, gain: 2 },
    Level { drive: 2, gain: 2 },
    Level { drive: 3, gain: 2 },
    Level { drive: 3, gain: 3 },
];

/// Any sample in a second above this, and the ADC is (nearly) clipping:
/// step down.
const SATURATED: u32 = FULL_SCALE * 9 / 10;
/// A second's average below this, and the pulse is down to a few counts:
/// step up. A step at most doubles the signal, so this can't bounce
/// straight back into `SATURATED`.
const WEAK: u32 = FULL_SCALE / 8;
/// Below this even at the top level, almost no light is coming back: the
/// watch isn't being worn.
///
/// TODO: check against real off-wrist readings, which depend on ambient
/// light reaching the sensor.
const NO_CONTACT: u32 = FULL_SCALE / 64;
/// Samples per signal-level check: about a second.
const LEVEL_CHECK_SAMPLES: u32 = 27;

struct Hrs3300 {
    i2c: Device,
}

impl Hrs3300 {
    /// Writes one register. Every transfer has a timeout: a wedged bus must
    /// not wedge this task (or, via the shared bus, touch and steps).
    async fn write(&mut self, register: u8, value: u8) -> Result<(), ()> {
        match with_timeout(I2C_TIMEOUT, self.i2c.write(ADDRESS, &[register, value])).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => {
                warn!("heart: write to {=u8:#04x} failed: {}", register, e);
                crate::i2c::reset_controller().await;
                Err(())
            }
            Err(_) => {
                warn!("heart: write to {=u8:#04x} timed out", register);
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
                warn!("heart: read of {=u8:#04x} failed: {}", register, e);
                crate::i2c::reset_controller().await;
                Err(())
            }
            Err(_) => {
                warn!("heart: read of {=u8:#04x} timed out", register);
                crate::i2c::reset_controller().await;
                Err(())
            }
        }
    }

    /// Checks the sensor is there, and turns it off (whatever the previous
    /// firmware, e.g. InfiniTime, left it doing).
    async fn init(&mut self) -> Result<(), ()> {
        let mut id = [0];
        self.read(REG_ID, &mut id).await?;
        if id[0] != CHIP_ID {
            // Only the datasheet vouches for this value, and the sensor
            // answered at its address, so carry on regardless.
            warn!("heart: unexpected sensor ID {=u8:#04x}", id[0]);
        }
        self.off().await
    }

    /// Starts measuring at `level` (or switches to it).
    async fn on(&mut self, level: Level) -> Result<(), ()> {
        self.write(REG_RES, RES_14_BIT).await?;
        self.write(REG_HGAIN, level.gain << 2).await?;
        // PDRIVE's two bits live in different registers.
        let pdrive0 = (level.drive & 1) * PDRIVER_PDRIVE0;
        let pdrive1 = (level.drive >> 1 & 1) * ENABLE_PDRIVE1;
        self.write(REG_PDRIVER, PDRIVER_PON | pdrive0 | PDRIVER_LOW_BITS)
            .await?;
        self.write(REG_ENABLE, ENABLE_HEN | ENABLE_HWT_12_5_MS | pdrive1)
            .await
    }

    /// Stops measuring, and stops the oscillator too (PON off), as
    /// InfiniTime does, for the lowest idle current.
    async fn off(&mut self) -> Result<(), ()> {
        self.write(REG_ENABLE, ENABLE_HWT_12_5_MS).await?;
        self.write(REG_PDRIVER, 0).await
    }

    /// The latest conversion: (HRS, ALS) channel values.
    ///
    /// One burst read across all eight result registers, since the bits of
    /// each value are scattered among them (the datasheet's tables 4 and 5).
    /// The sensor may update between bytes of the burst; [`Conversions`]
    /// copes with the occasional torn value that results.
    async fn sample(&mut self) -> Result<(u32, u32), ()> {
        let mut r = [0u8; 8];
        self.read(REG_DATA, &mut r).await?;
        let at = |register: u8| u32::from(r[usize::from(register - REG_DATA)]);
        // HRS (CH0): 0x09 holds bits 15:8, 0x0A bits 7:4 (in its low
        // nibble), 0x0F bits 3:0 (low nibble) and 17:16 (bits 5:4).
        let hrs =
            at(0x09) << 8 | (at(0x0A) & 0x0F) << 4 | (at(0x0F) & 0x0F) | (at(0x0F) & 0x30) << 12;
        // ALS (CH1): 0x0D holds bits 17:11, 0x08 bits 10:3, 0x0E bits 2:0.
        let als = (at(0x0D) & 0x7F) << 11 | at(0x08) << 3 | (at(0x0E) & 0x07);
        Ok((hrs, als))
    }
}

// ==============================================================================
// The task
// ==============================================================================

#[embassy_executor::task]
pub async fn heart_task(i2c: Device) {
    // Stay off the bus while it's busiest: touch configures itself right
    // at boot, and the accelerometer's setup (from 200 ms) holds the bus
    // for about a third of a second.
    Timer::after_millis(1000).await;
    let mut hrs = Hrs3300 { i2c };
    let mut attempt = 0;
    while hrs.init().await.is_err() {
        attempt += 1;
        if attempt == 5 {
            warn!("heart: sensor setup failed; no heart rate");
            return;
        }
        warn!("heart: sensor setup failed; retrying");
        Timer::after_millis(100).await;
    }
    info!("heart: sensor ready");

    // Remembered across sessions: the wrist (and the sensor's oscillator)
    // don't change much in ten minutes.
    let mut level = 0;
    let mut conversions = Conversions::new();
    // Measure right away, so the clock face has a number soon after boot.
    let mut next_due = Instant::now();

    loop {
        // Wait for the next background session, or for someone to look at
        // the clock face. (A signal left over from during the last session
        // is stale: `watching()` says whether it still holds.)
        WATCH_STARTED.reset();
        if !watching() && Instant::now() < next_due {
            select(Timer::at(next_due), WATCH_STARTED.wait()).await;
        }
        if !watching() && Instant::now() < next_due {
            continue;
        }

        let started = Instant::now();
        let outcome = measure(&mut hrs, &mut level, &mut conversions).await;
        if hrs.off().await.is_err() {
            // TODO: retry, or the LED stays lit until the next session.
            warn!("heart: couldn't turn the sensor off");
        }
        let elapsed = started.elapsed().as_secs();
        match outcome {
            Ok(Ended::Measured) => {}
            Ok(Ended::TimedOut) => info!("heart: no reading after {} s", elapsed),
            Ok(Ended::NoContact) => info!("heart: not on a wrist (after {} s)", elapsed),
            Ok(Ended::TooBright) => info!("heart: too much light (after {} s)", elapsed),
            Err(()) => warn!("heart: lost the sensor mid-session"),
        }
        // A pulse means skin. No light coming back, or ambient light
        // flooding in, means the sensor isn't against anything. Light
        // without a pulse proves nothing: a desk reflects the LED too (the
        // dev kit lying on one times out exactly like a busy wrist).
        let worn = match outcome {
            Ok(Ended::Measured) => Some(true),
            Ok(Ended::NoContact | Ended::TooBright) => Some(false),
            Ok(Ended::TimedOut) | Err(()) => None,
        };
        WORN.lock(|w| w.set(worn));
        // A reading resets the background schedule; so does a failed
        // background session (retrying right away would most likely fail
        // the same way: the watch is off the wrist, or the wrist is busy).
        // TODO: retry a failed background session sooner than `INTERVAL`.
        if matches!(outcome, Ok(Ended::Measured)) || Instant::now() >= next_due {
            next_due = Instant::now() + INTERVAL;
        }

        // A session only ends while someone's watching if there's no
        // signal to be had; don't start another until they look away.
        while watching() {
            Timer::after_secs(1).await;
        }
    }
}

/// Why a session ended.
enum Ended {
    /// We have a reading (and nobody's watching for more).
    Measured,
    TimedOut,
    /// Almost no light comes back even at the highest level.
    NoContact,
    /// The ADC clips even at the lowest level: ambient light is getting in.
    TooBright,
}

/// One measurement session: turn the sensor on, sample until we have a
/// reading (and keep going while someone's watching), adjusting the level
/// as needed.
async fn measure(
    hrs: &mut Hrs3300,
    level: &mut usize,
    conversions: &mut Conversions,
) -> Result<Ended, ()> {
    hrs.on(LEVELS[*level]).await?;
    conversions.restart();
    let mut ppg = Ppg::new();
    let mut consensus = Consensus::new();
    let started = Instant::now();
    let mut measured = false;
    let mut moved_at: Option<Instant> = None;
    // This second's signal, for the level check.
    let (mut peak, mut sum, mut count) = (0u32, 0u32, 0u32);

    let mut ticker = Ticker::every(POLL);
    loop {
        ticker.next().await;
        let now = Instant::now();
        if !watching() && (measured || now - started >= SESSION_LIMIT) {
            return Ok(if measured {
                Ended::Measured
            } else {
                Ended::TimedOut
            });
        }
        if activity::movement() > MOTION_LIMIT {
            moved_at = Some(now);
        }

        let (raw, ambient) = hrs.sample().await?;
        let mut verdict = None;
        conversions.poll(raw, now.as_micros(), |sample| {
            peak = peak.max(sample);
            sum += sample;
            count += 1;
            if let Some(v) = ppg.push(sample) {
                verdict = Some(v);
            }
        });

        // Once a second: is the signal within range? If not, change level
        // and start over (samples at different levels aren't comparable).
        if count >= LEVEL_CHECK_SAMPLES {
            let mean = sum / count;
            debug!(
                "heart: level {}, mean {}, peak {}, ambient {}, period {} µs",
                LEVELS[*level],
                mean,
                peak,
                ambient,
                conversions.period_us()
            );
            let before = *level;
            if peak >= SATURATED {
                if *level == 0 {
                    return Ok(Ended::TooBright);
                }
                *level -= 1;
            } else if mean < WEAK && *level + 1 < LEVELS.len() {
                *level += 1;
            } else if mean < NO_CONTACT {
                return Ok(Ended::NoContact);
            }
            (peak, sum, count) = (0, 0, 0);
            if *level != before {
                debug!("heart: switching to level {}", LEVELS[*level]);
                hrs.on(LEVELS[*level]).await?;
                conversions.restart();
                ppg.reset();
                continue;
            }
        }

        let Some(verdict) = verdict else {
            continue;
        };
        let estimate = match verdict {
            Ok(estimate) => estimate,
            Err(why) => {
                debug!("heart: window rejected: {}", why);
                continue;
            }
        };
        let bpm = estimate.bpm_at(conversions.period_us());
        if moved_at.is_some_and(|at| now - at < MOTION_HOLD) {
            debug!("heart: window rejected: wrist moving ({} BPM)", bpm);
            continue;
        }
        debug!("heart: window {} BPM (quality {})", bpm, estimate.quality);
        if let Some(bpm) = consensus.push(bpm) {
            if !measured {
                info!(
                    "heart: {} BPM after {} s (level {}, conversion period {} µs)",
                    bpm,
                    started.elapsed().as_secs(),
                    LEVELS[*level],
                    conversions.period_us()
                );
            } else {
                debug!("heart: {} BPM", bpm);
            }
            publish(bpm);
            measured = true;
        }
    }
}
