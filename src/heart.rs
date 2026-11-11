//! Heart rate, from the HRS3300 optical (PPG) sensor.
//!
//! The sensor's green LED is the single most power-hungry thing on the
//! watch short of the backlight and radio (12.5–40 mA while lit), so it
//! isn't run continuously. Instead, a measurement *session* runs:
//!
//! - in the background, when [`crate::heart_schedule`] says one is due
//!   (every 2–20 minutes, depending on movement and recent readings), once
//!   the wrist has been still for a few seconds after that (see [`CALM`]),
//!   or two minutes on if it isn't, or at once if a reading is overdue
//!   ([`heart_schedule::Schedule::start`]), for up to [`SESSION_LIMIT`] or
//!   until it has a reading (typically ~11 s), carrying on through
//!   movement (see `heart_schedule`'s "Trying through movement"); if it
//!   runs out on a calm wrist with a pulse too faint to read, it may go on
//!   for another 15 s with a brighter LED, once or twice
//!   ([`heart_schedule::brighter`]); awake, every other
//!   round of tries (a scheduled session and its retries) starts with a
//!   brighter LED (see [`Arm`]); both are an experiment, as is, in the
//!   logging build only, a busy wrist's session going on to 90 s on half
//!   the awake rounds (see `heart_schedule::Length`, with the
//!   accelerometer's samples logged through every awake session, see
//!   `trace_accel`), and
//! - while the clock face is showing (see
//!   [`crate::heart_reading::set_watching`]), continuously,
//!   since then someone is looking at the number, and
//! - while Gadgetbridge's live activity asks for it (see [`crate::live`]
//!   and [`crate::heart_reading::set_live`]), continuously too, for as long
//!   as it asks.
//!
//! Between sessions the sensor is fully off: no LED, no oscillator. (Should
//! switching it off fail, that's tried again every second until it works.)
//!
//! The signal processing lives in [`crate::ppg`], which knows nothing about
//! the hardware; this module drives the sensor, keeps its signal in range
//! ([`LEVELS`]), vetoes measurements while the wrist moves (using
//! [`activity::movement`]), and publishes readings for the clock face
//! and the per-minute history Gadgetbridge fetches, through
//! [`crate::heart_reading`].

#[cfg(feature = "field-log")]
use core::cell::Cell;
#[cfg(feature = "field-log")]
use core::sync::atomic::{AtomicU8, Ordering};

use defmt::{debug, info, trace, warn};
use embassy_futures::select::select;
use embassy_time::{Duration, Instant, Ticker, Timer, with_timeout};
use embedded_hal_async::i2c::I2c;

use crate::activity;
// What the rest of the firmware reads and sets lives there, free of
// hardware, so the simulator runs it too.
use crate::heart_reading::{WAKE, publish, watching};
pub use crate::heart_reading::{wake, worn};
use crate::heart_schedule::{self, Arm, Before, Outcome, Schedule, Start};
#[cfg(feature = "field-log")]
use crate::heart_schedule::{Length, Lengths};
use crate::i2c::Device;
use crate::ppg::{Consensus, Conversions, Ppg, Rejection};
#[cfg(feature = "field-log")]
use embassy_sync::blocking_mutex::Mutex;
#[cfg(feature = "field-log")]
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;

// ==============================================================================
// Scheduling
// ==============================================================================

/// How long a background session may try before giving up. A clean
/// signal on a calm wrist gives a reading after ~10.5 s (half a second
/// warming up and settling, an 8 s window, then another 2 s later that
/// agrees with it; see `ppg::Consensus`); this leaves room for a few
/// rejected windows, or the gain adjusting.
///
/// Not longer: in field logs (195 readings), 88% came by 14 s, and of the
/// sessions still going at 20 s, fewer than 2% a second read after that.
/// A still wrist's signal comes and goes over minutes rather than
/// seconds, so the LED does more good in another session soon after (see
/// [`heart_schedule::CALM_RETRY_INTERVAL`]) than in a longer one.
///
/// A background session runs all of it however the wrist moves, unless
/// it reads, or finds no skin: a busy wrist may settle within it, and its
/// samples under movement, in a field log, are the data a motion-tolerant
/// estimator needs (see `heart_schedule`'s "Trying through movement").
const SESSION_LIMIT: Duration = Duration::from_secs(30);

/// Wrist movement (see [`activity::movement`]) above which the PPG can't be
/// trusted: moving the sensor against the skin changes the light far more
/// than the pulse does.
///
/// With the accelerometer read correctly (0.8.0 on): ~30 lying still,
/// 2000–7000 when handled. Earlier field logs, on a corrupted movement
/// measure, vetoed 98% of on-wrist time, including every window that
/// passed the signal checks, though those matched a Garmin worn alongside
/// to a median +0.4 BPM.
///
/// Also what [`activity::active_polls`] counts as vigorous movement, for
/// the schedule.
///
/// TODO: tune from a 0.8.x field log with walking in it.
pub const MOTION_LIMIT: u16 = 1500;

/// Wrist movement (see [`activity::movement`]) at or below which a
/// background session may start, once it has held for
/// [`heart_schedule::START_CALM_POLLS`] polls. A due session waits for
/// that for a while, as a busy wrist rarely reads, but not indefinitely
/// (see [`heart_schedule::CALM_WAIT`]). Above it, the wrist counts as busy
/// for [`heart_schedule::calm_throughout`].
///
/// On the wrist the pulse is tiny (±5 counts on ~3000), and sitting at a
/// desk, a hand's small movements make bursts ten times that every few
/// seconds, so no window gets the ~12 s of calm a reading needs. In a
/// field log compared against a Garmin (0.8.13, 14 sessions), all four
/// readings came once movement had dropped to 30–130; sessions spent at
/// 300–2500 throughout never read. Two sessions at 75–240 failed too:
/// calm is necessary, not sufficient.
pub const CALM: u16 = 150;

/// How often to look again, while waiting for a background session to
/// fall due, at whether movement has brought it forward. Precision isn't
/// needed (the session waits for a calm wrist anyway), and waking for it
/// is cheap but not free.
const RECHECK_SECS: u32 = 30;

/// Seconds of movement readings kept (one per level check, i.e. a second),
/// to judge a whole window: an analysis window is ~8 s.
const MOTION_HISTORY: usize = 8;

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
/// These only follow the light level, which on a wrist sits at the second
/// (12.5 mA ×2: 1,600-4,000) whatever the pulse. A background session
/// may raise the drive alone, keeping the gain, once its usual 30 s have
/// found only a faint pulse (see [`heart_schedule::brighter`]).
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
///
/// On the wrist, 12.5 mA ×2 reads 2000–3500, and every window that passed
/// the signal checks in a day's field log was at that level. At
/// `FULL_SCALE / 8` (2047), contact wobble kept bumping sessions to ×4,
/// which restarts the analysis and never yielded a window.
const WEAK: u32 = FULL_SCALE / 16;
/// At the lowest current (×1), a mean below this is no skin: off the wrist
/// read 73–223, on it 1076–1631. Scaled by the gain at higher gains.
///
/// TODO: check in daylight; these were indoors and on a charger.
const NO_SKIN: u32 = FULL_SCALE / 32;
/// Below this even at the top level, almost no light is coming back: the
/// watch isn't being worn.
///
/// TODO: check against real off-wrist readings, which depend on ambient
/// light reaching the sensor.
const NO_CONTACT: u32 = FULL_SCALE / 64;
/// Samples per signal-level check: about a second.
const LEVEL_CHECK_SAMPLES: u32 = 27;

// ==============================================================================
// What the numbers above promise each other
// ==============================================================================
//
// Checked when compiling, so they cost the watch nothing. This task's logic
// is tangled up with the sensor and the clock, where no host test or proof
// reaches it and a release build has no debug assertions, so what can be
// said of its constants is said here. (What it computes is checked in
// `heart_schedule` and `ppg`, which the host tests run.)

// `LEVELS` is weakest first, a step at a time: each raises either the gain
// by one setting (twice the signal) or the drive by one (at most twice the
// current, see `WEAK`), and both stay settings: two bits each in the
// registers `Hrs3300::on` packs them into, and `DRIVE_CURRENT`'s index.
const _: () = {
    let mut i = 0;
    while i < LEVELS.len() {
        let level = LEVELS[i];
        assert!(level.drive <= 3 && level.gain <= 3);
        if i > 0 {
            let weaker = LEVELS[i - 1];
            let gain_up = level.drive == weaker.drive && level.gain == weaker.gain + 1;
            let drive_up = level.gain == weaker.gain && level.drive == weaker.drive + 1;
            assert!(gain_up || drive_up);
            let current = heart_schedule::DRIVE_CURRENT;
            assert!(current[level.drive as usize] <= 2 * current[weaker.drive as usize]);
        }
        i += 1;
    }
    // Sessions start at the second level, and the lowest drive is where
    // "no skin" is told from skin.
    assert!(LEVELS.len() > 1 && LEVELS[0].drive == 0 && LEVELS[1].drive == 0);
};
// Stepping up from a weak signal can't land in saturation, nor the other
// way round, or the level would bounce between two for the whole session.
const _: () = assert!(NO_CONTACT < NO_SKIN && NO_SKIN < WEAK && 2 * WEAK < SATURATED);
// A brighter phase's predicted peak stays clear of where the level check
// steps back down.
const _: () = assert!(heart_schedule::BRIGHTER_PEAK_LIMIT < SATURATED);
// The budget's reserve is a whole session (see `SESSION_RESERVE`), and a
// brighter phase is shorter than one.
const _: () = assert!(SESSION_LIMIT.as_secs() == heart_schedule::SESSION_RESERVE as u64);
const _: () = assert!(heart_schedule::BRIGHTER_FOR as u64 <= SESSION_LIMIT.as_secs());
// A level check is a second of samples at the sensor's nominal rate, with
// room in `measure`'s `second` for the few more a late poll completes; and
// a second's sum of 18-bit samples fits its `u32`.
const _: () = assert!(LEVEL_CHECK_SAMPLES == 1_000_000u32.div_ceil(crate::ppg::NOMINAL_PERIOD_US));
const _: () = assert!(LEVEL_CHECK_SAMPLES as usize + 8 <= SECOND_SAMPLES);
// The movement kept, a reading a second, spans one analysis window.
const _: () = assert!(
    MOTION_HISTORY as u64
        == (crate::ppg::WINDOW as u64 * crate::ppg::NOMINAL_PERIOD_US as u64).div_ceil(1_000_000)
);
// The sensor is polled several times a conversion (see `Conversions`).
const _: () = assert!(POLL.as_micros() * 3 <= crate::ppg::NOMINAL_PERIOD_US as u64);
// The calm a session waits for is stricter than what vetoes a window.
const _: () = assert!(CALM < MOTION_LIMIT);

/// Room for a second's samples, for the field log (see `measure`).
const SECOND_SAMPLES: usize = 40;

struct Hrs3300 {
    i2c: Device,
    /// Whether to keep transfer failures out of the log: while setup is
    /// retried slowly, only some attempts log (see [`crate::backoff`]).
    quiet: bool,
}

impl Hrs3300 {
    /// Writes one register. Every transfer has a timeout: a wedged bus must
    /// not wedge this task (or, via the shared bus, touch and steps).
    async fn write(&mut self, register: u8, value: u8) -> Result<(), ()> {
        match with_timeout(I2C_TIMEOUT, self.i2c.write(ADDRESS, &[register, value])).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => {
                if !self.quiet {
                    warn!("heart: write to {=u8:#04x} failed: {}", register, e);
                }
                crate::i2c::reset_controller().await;
                Err(())
            }
            Err(_) => {
                if !self.quiet {
                    warn!("heart: write to {=u8:#04x} timed out", register);
                }
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
                if !self.quiet {
                    warn!("heart: read of {=u8:#04x} failed: {}", register, e);
                }
                crate::i2c::reset_controller().await;
                Err(())
            }
            Err(_) => {
                if !self.quiet {
                    warn!("heart: read of {=u8:#04x} timed out", register);
                }
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

    /// [`Hrs3300::off`], tried up to `tries` times; whether it worked.
    /// Every failed transfer resets the bus controller (see
    /// [`Hrs3300::write`]), so the next try may well work. Only the first
    /// try logs its failure.
    async fn off_tries(&mut self, tries: u8) -> bool {
        let quiet = self.quiet;
        let mut off = false;
        for _ in 0..tries {
            off = self.off().await.is_ok();
            if off {
                break;
            }
            self.quiet = true;
        }
        self.quiet = quiet;
        off
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

/// Switches the sensor off for System OFF (see [`crate::system_off`]),
/// through the bus `system_off` holds, whatever the heart-rate task was
/// doing: the sensor keeps its state while the nRF52 is off, so a session
/// cut short would otherwise leave the LED lit until the battery is flat.
/// Each write is tried a few times, with the bus recovered between tries
/// (see [`crate::i2c::write_held`]).
pub async fn switch_off(twim: &mut embassy_nrf::twim::Twim<'static>) {
    for write in [[REG_ENABLE, ENABLE_HWT_12_5_MS], [REG_PDRIVER, 0]] {
        if !crate::i2c::write_held(twim, ADDRESS, &write, I2C_TIMEOUT).await {
            warn!("heart: switching the sensor off failed");
        }
    }
}

#[embassy_executor::task]
pub async fn heart_task(i2c: Device) {
    // Stay off the bus while it's busiest: touch configures itself right
    // at boot, and the accelerometer's setup (from 200 ms) holds the bus
    // for about a third of a second.
    Timer::after_millis(1000).await;
    let mut hrs = Hrs3300 { i2c, quiet: false };
    // For as long as it takes, ever more slowly: giving up would leave a
    // sealed watch without heart rate until its next reboot (see
    // `crate::backoff`).
    let mut backoff = crate::backoff::Backoff::new();
    loop {
        hrs.quiet = !backoff.loud();
        if hrs.init().await.is_ok() {
            break;
        }
        let retry = backoff.failed();
        if retry.log {
            warn!(
                "heart: sensor setup failed ({=u16}x); retrying in {=u32} ms",
                retry.failures, retry.wait_ms
            );
        }
        Timer::after_millis(u64::from(retry.wait_ms)).await;
    }
    hrs.quiet = false;
    info!("heart: sensor ready");

    // The level the last *successful* session ended at: the wrist (and the
    // sensor's oscillator) don't change much between sessions. A failed
    // session (off the wrist, typically) climbs to the top level, 40 mA, and
    // starting every later session there would triple the LED's cost.
    // Starting at ×2 (12.5 mA): where on-wrist readings settled anyway, so
    // the first second isn't spent climbing to it.
    let mut good_level = 1;
    // The light on skin: the last second's peak of the last session that
    // didn't end off the wrist, scaled to 12.5 mA ×1 (0: none yet), to
    // tell whether a bright arm's start would saturate. Unknown, it starts
    // bright anyway, and steps back down if it saturates.
    let mut skin_peak = 0u32;
    // When to measure in the background. Its first session is due at once,
    // so the clock face has a number soon after boot.
    let mut schedule = Schedule::new();
    let mut conversions = Conversions::new();
    // Whether the sensor may still be on, its LED lit (12.5–40 mA):
    // switching it off after the last session failed, likeliest after one
    // that lost the sensor mid-session. Left so, it would stay lit until
    // the next session, up to an hour away when sessions keep failing, or
    // for as long as someone watched. So it's tried again every second
    // until it works (see `retry_off`): a failed try is cheap, at most a
    // timed-out transfer and a controller reset.
    let mut maybe_lit = false;
    // Which awake rounds go on longer on a busy wrist (see `Length`).
    #[cfg(feature = "field-log")]
    let mut lengths = Lengths::new();

    loop {
        // Wait until a background session is due and the wrist is still,
        // or until someone looks at the clock face. Movement can bring the
        // session forward while we wait, so the due time is looked at
        // again every `RECHECK_SECS` while the wrist moves (while it's
        // still, nothing can, and `activity` wakes us when it starts
        // moving); once due, every 3 s, as movement is only updated with
        // each read of the accelerometer, and at once when it sees the
        // wrist go still. (A wake left over from during the last session is
        // stale: everything is looked at afresh.) A due session waits for
        // calm only so long, though (see `heart_schedule::Schedule::start`),
        // counted from when it was first found due.
        let mut due_since = None;
        let due = loop {
            WAKE.reset();
            if maybe_lit {
                maybe_lit = !retry_off(&mut hrs).await;
            }
            if watching() {
                break None;
            }
            let (at, why) = schedule.due(activity::active_polls(), activity::asleep());
            let now = uptime();
            let wait = if now < at {
                due_since = None;
                if activity::still() {
                    at - now
                } else {
                    (at - now).min(RECHECK_SECS)
                }
            } else {
                let since = *due_since.get_or_insert(now);
                match schedule.start(now, since, activity::calm_polls()) {
                    // A calm start reads far more often, and a moment's
                    // calm at a desk rarely lasts long enough.
                    Start::Wait => 3,
                    Start::Calm => break Some(why),
                    Start::Busy => {
                        // So a field log can tell these sessions apart.
                        info!("heart: busy start ({=u8} s)", (now - since).min(255) as u8);
                        break Some(why);
                    }
                }
            };
            // Back within a second while the sensor may be on, to try
            // switching it off again.
            let wait = if maybe_lit { 1 } else { wait };
            select(Timer::after_secs(u64::from(wait)), WAKE.wait()).await;
        };
        let background = due.is_some();
        let asleep = activity::asleep();
        let mut level = good_level;
        let base = LEVELS[level].drive;
        let budget = schedule.budget(uptime());
        // The round of tries and its arm (a watched session is in neither),
        // and the drive it starts at.
        let (round, arm) = match due {
            Some(_) => schedule.begin(asleep),
            None => (0, Arm::Dim),
        };
        let drive = match arm {
            Arm::Bright => {
                let peak = (skin_peak << LEVELS[level].gain)
                    * heart_schedule::DRIVE_CURRENT[usize::from(base)]
                    / heart_schedule::DRIVE_CURRENT[0];
                heart_schedule::bright_start(base, peak, budget).unwrap_or(base)
            }
            Arm::Dim | Arm::Asleep => base,
        };
        // And, in the logging build, its length (see `Length`).
        let field = FieldArms {
            #[cfg(feature = "field-log")]
            length: match due {
                Some(_) => lengths.of(round, arm),
                None => Length::Usual,
            },
        };
        if let Some(why) = due {
            let (awake, asleep) = schedule.resting();
            #[cfg(not(feature = "field-log"))]
            info!(
                "heart: due ({}); budget {} s, resting {}/{}; round {} {} drive {}",
                why, budget, awake, asleep, round, arm, drive
            );
            #[cfg(feature = "field-log")]
            info!(
                "heart: due ({}); budget {} s, resting {}/{}; round {} {} drive {} {}",
                why, budget, awake, asleep, round, arm, drive, field.length
            );
        }
        // Movement from here on counts towards the next session.
        let before = Before {
            active_polls: activity::take_active_polls(),
            stepless_polls: activity::stepless_polls(),
            asleep,
        };

        let started = Instant::now();
        crate::power::count(crate::power::Count::HeartSessions);
        // Asleep, the wrist's movement is turning over, which isn't what
        // the experiment is after: no samples (~50 KB of log a night).
        #[cfg(feature = "field-log")]
        if !asleep {
            ACCEL.store(ACCEL_SESSION, Ordering::Relaxed);
        }
        // What the session spends of the budget, at 12.5 mA's rate (see
        // `heart_schedule::budget_seconds`): a brighter phase costs more.
        let mut spent = 0;
        let mut peak = 0;
        let outcome = measure(
            &mut hrs,
            &mut level,
            drive,
            &mut conversions,
            background.then_some(budget),
            &mut spent,
            &mut peak,
            field,
        )
        .await;
        // The accelerometer's next samples still overlap the session.
        #[cfg(feature = "field-log")]
        if !asleep {
            ACCEL.store(ACCEL_TAIL, Ordering::Relaxed);
        }
        // A session that recorded on after its reading (see `Recording`)
        // ended, as far as anything but the budget goes, at that reading,
        // at the level and light it had then, however it ended.
        #[cfg(feature = "field-log")]
        let recorded = RECORDING.lock(Cell::take);
        #[cfg(feature = "field-log")]
        let ended_early = recorded.is_some() && !matches!(outcome, Ok(Ended::Measured(_)));
        #[cfg(feature = "field-log")]
        let outcome = match recorded {
            Some(reading) => {
                level = usize::from(reading.level);
                peak = reading.skin_peak;
                Ok(Ended::Measured(reading.bpm))
            }
            None => outcome,
        };
        if matches!(outcome, Ok(Ended::Measured(_))) {
            good_level = level;
        }
        if !matches!(outcome, Ok(Ended::NoContact | Ended::TooBright) | Err(())) && peak > 0 {
            skin_peak = peak;
        }
        // A few tries at once, then every second (see `maybe_lit`). Only
        // the first try's failed transfer is logged, then this line, and
        // one more when it works, however many tries that takes.
        //
        // The power meter counts the LED off from here regardless (see
        // `LedMeter`): it's a fault's cost, not a session's.
        maybe_lit = !hrs.off_tries(3).await;
        if maybe_lit {
            warn!("heart: couldn't turn the sensor off; retrying every second");
        }
        // Rounded up: the budget should err on the side of caution.
        let led_seconds = started.elapsed().as_millis().div_ceil(1000) as u32;
        match outcome {
            Ok(Ended::Measured(_)) => {}
            Ok(Ended::TimedOut { calm: false }) => {
                info!("heart: no reading after {} s", led_seconds)
            }
            Ok(Ended::TimedOut { calm: true }) => {
                info!("heart: no reading after {} s; wrist calm", led_seconds)
            }
            Ok(Ended::NoContact) => info!("heart: not on a wrist (after {} s)", led_seconds),
            Ok(Ended::TooBright) => info!("heart: too much light (after {} s)", led_seconds),
            Err(()) => warn!("heart: lost the sensor mid-session"),
        }
        #[cfg(feature = "field-log")]
        if recorded.is_some() {
            info!(
                "heart: recorded to {} s{=str}",
                led_seconds,
                if ended_early { "; ended early" } else { "" }
            );
        }
        // A pulse means skin. No light coming back, or ambient light
        // flooding in, means the sensor isn't against anything. Light
        // without a pulse proves nothing: a desk reflects the LED too (the
        // dev kit lying on one times out exactly like a busy wrist).
        let (worn, outcome) = match outcome {
            Ok(Ended::Measured(bpm)) => (Some(true), Outcome::Reading(bpm)),
            Ok(Ended::NoContact | Ended::TooBright) => (Some(false), Outcome::OffWrist),
            Ok(Ended::TimedOut { calm: true }) => (None, Outcome::CalmNoReading),
            Ok(Ended::TimedOut { calm: false }) | Err(()) => (None, Outcome::NoReading),
        };
        crate::heart_reading::set_worn(worn);
        // At the lowest drive, `spent` is `led_seconds`, short of the
        // moments around the LED's switching on and off.
        let spent = spent.max(led_seconds);
        #[cfg(not(feature = "field-log"))]
        schedule.ended(uptime(), spent, outcome, background, before);
        #[cfg(feature = "field-log")]
        schedule.ended(
            recorded.map_or_else(uptime, |reading| reading.at),
            spent,
            outcome,
            background,
            before,
        );

        // A session only ends while someone's watching if there's no
        // signal to be had (off the wrist, typically); don't start another
        // until they look away, or Gadgetbridge stops asking.
        //
        // Deliberately no retrying while Gadgetbridge asks, in case the
        // watch goes on a wrist meanwhile: not measuring off the wrist is
        // what's wanted, and reopening the chart tries again.
        while watching() {
            Timer::after_secs(1).await;
            if maybe_lit {
                maybe_lit = !retry_off(&mut hrs).await;
            }
        }
    }
}

/// One more, quiet, try at switching off a sensor that may still be on
/// (see `maybe_lit` in [`heart_task`]); whether it worked, which is
/// logged.
async fn retry_off(hrs: &mut Hrs3300) -> bool {
    hrs.quiet = true;
    let off = hrs.off().await.is_ok();
    hrs.quiet = false;
    if off {
        info!("heart: sensor off at last");
    }
    off
}

/// Seconds since boot, the schedule's clock.
fn uptime() -> u32 {
    Instant::now().as_secs() as u32
}

/// Accounts the LED's on-time to its drive current, for [`crate::power`]
/// and the schedule's budget (`spent`, see
/// [`heart_schedule::budget_seconds`]): on from creation until dropped.
struct LedMeter<'a> {
    drive: u8,
    /// When the LED went on at `drive`.
    since: Instant,
    /// The budget spent at earlier drives.
    spent: &'a mut u32,
}

impl<'a> LedMeter<'a> {
    fn new(drive: u8, spent: &'a mut u32) -> Self {
        crate::power::on(Self::meter(drive));
        Self {
            drive,
            since: Instant::now(),
            spent,
        }
    }

    fn switch(&mut self, drive: u8) {
        crate::power::off(Self::meter(self.drive));
        crate::power::on(Self::meter(drive));
        *self.spent = self.spent();
        self.drive = drive;
        self.since = Instant::now();
    }

    /// The budget spent so far.
    fn spent(&self) -> u32 {
        let millis = self.since.elapsed().as_millis() as u32;
        *self.spent + heart_schedule::budget_seconds(self.drive, millis)
    }

    fn meter(drive: u8) -> crate::power::Meter {
        use crate::power::Meter;
        [
            Meter::HeartLed0,
            Meter::HeartLed1,
            Meter::HeartLed2,
            Meter::HeartLed3,
        ][usize::from(drive & 3)]
    }
}

impl Drop for LedMeter<'_> {
    fn drop(&mut self) {
        crate::power::off(Self::meter(self.drive));
        *self.spent = self.spent();
    }
}

/// The experiments a session is in that only the logging build runs: its
/// `heart_schedule::Length`. Empty in the plain build, whose sessions are
/// as they were.
#[derive(Clone, Copy)]
struct FieldArms {
    #[cfg(feature = "field-log")]
    length: Length,
}

/// The logging build's longer arm: a session that read before 90 s on a
/// busy wrist, recording on (see `heart_schedule::Length`). Its reading,
/// the uptime it came at, and the level and skin peak the session had
/// then: as far as anything but the budget goes, the session ended there.
#[cfg(feature = "field-log")]
#[derive(Clone, Copy)]
struct Recording {
    bpm: u8,
    at: u32,
    level: u8,
    skin_peak: u32,
}

/// The session recording on, if it is; taken when it ends. A static
/// rather than the heart task's state, as `read` needs it too, and so the
/// plain build's task is as it was.
#[cfg(feature = "field-log")]
static RECORDING: Mutex<CriticalSectionRawMutex, Cell<Option<Recording>>> =
    Mutex::new(Cell::new(None));

/// How long a session on the longer arm may run: 90 s.
#[cfg(feature = "field-log")]
const LONGER_LIMIT: Duration =
    Duration::from_secs(SESSION_LIMIT.as_secs() + heart_schedule::LONGER_FOR as u64);

/// Whether a session that has read is recording on, until `limit` (see
/// [`Recording`]); never in the plain build.
fn recording_on(elapsed: Duration, limit: Duration) -> bool {
    #[cfg(feature = "field-log")]
    return elapsed < limit && RECORDING.lock(Cell::get).is_some();
    #[cfg(not(feature = "field-log"))]
    {
        let _ = (elapsed, limit);
        false
    }
}

/// Why a session ended.
enum Ended {
    /// We have a reading (and nobody's watching for more): the last, in
    /// BPM.
    Measured(u8),
    /// No reading within [`SESSION_LIMIT`] (and any brighter phases); `calm`
    /// if the wrist was calm through most of it (see
    /// [`heart_schedule::calm_throughout`]).
    TimedOut { calm: bool },
    /// Almost no light comes back even at the highest level.
    NoContact,
    /// The ADC clips even at the lowest level: ambient light is getting in.
    TooBright,
}

/// One measurement session: turn the sensor on, sample until we have a
/// reading (and keep going while someone's watching), adjusting the level
/// as needed. It starts at `drive`, the level's or a bright arm's (see
/// [`Arm`]). `budget`: the schedule's LED budget at the start, if the
/// schedule started it (rather than someone looking at the clock face).
/// `spent` is set to what the session cost the budget, and `skin_peak` to
/// its last second's peak, scaled to 12.5 mA ×1. `field`: the logging
/// build's experiments it's in (see [`FieldArms`]).
// The eighth argument is that, empty in the plain build.
#[allow(clippy::too_many_arguments)]
async fn measure(
    hrs: &mut Hrs3300,
    level: &mut usize,
    drive: u8,
    conversions: &mut Conversions,
    budget: Option<u32>,
    spent: &mut u32,
    skin_peak: &mut u32,
    #[cfg_attr(not(feature = "field-log"), allow(unused_variables))] field: FieldArms,
) -> Result<Ended, ()> {
    // The LED's drive: `LEVELS[*level]`'s, unless the session started
    // brighter or a phase of it has gone brighter (see
    // `heart_schedule::brighter`), keeping the gain.
    let mut drive = drive;
    let at = |level: usize, drive: u8| Level {
        drive,
        gain: LEVELS[level].gain,
    };
    hrs.on(at(*level, drive)).await?;
    // Ends the LED's on-time when the session returns, however it returns.
    let mut led = LedMeter::new(drive, spent);
    // When the session's current phase runs out (its usual limit, or a
    // brighter phase's), and the brighter phases so far: a bright arm's
    // start counts as one.
    let (mut limit, mut steps) = (SESSION_LIMIT, u8::from(drive != LEVELS[*level].drive));
    // Whether the latest window was faint or aperiodic on a calm wrist,
    // and the last whole second's mean and peak: what a brighter phase
    // goes by.
    let (mut last_failed, mut last_mean, mut last_peak) = (false, 0u32, 0u32);
    conversions.restart();
    let mut ppg = Ppg::new();
    let mut consensus = Consensus::new();
    let started = Instant::now();
    // The latest reading, once there is one.
    let mut measured = None;
    // For judging whether the wrist was calm through the session: the level
    // checks so far, and how many found it busy.
    let (mut checks, mut busy) = (0u8, 0u8);
    // This window's movement, a reading a second (the accelerometer
    // reports every 2 s, so about four distinct values).
    let mut movement = heapless::HistoryBuf::<u16, MOTION_HISTORY>::new();
    // This second's signal, for the level check.
    let (mut peak, mut sum, mut count) = (0u32, 0u32, 0u32);
    // And the samples themselves, for the field log at `trace`: replaying
    // real wrists through `ppg` (host-tests/ppg) is how its limits get
    // tuned. About 60 bytes a second while measuring. (Room for a few more
    // than `LEVEL_CHECK_SAMPLES`, as one poll can complete several.)
    let mut second = heapless::Vec::<u16, SECOND_SAMPLES>::new();

    let mut ticker = Ticker::every(POLL);
    loop {
        ticker.next().await;
        let now = Instant::now();
        let elapsed = now - started;
        // A phase that ran out without a reading on a calm wrist may go on
        // brighter (see `heart_schedule::brighter`).
        let brighter = budget
            .filter(|_| !watching() && measured.is_none() && elapsed >= limit)
            .filter(|_| last_failed && heart_schedule::calm_throughout(busy, checks))
            .and_then(|budget| {
                let spare = budget.saturating_sub(led.spent());
                heart_schedule::brighter(drive, steps, last_peak, spare)
            });
        if let Some(next) = brighter {
            // The part-second at the dimmer drive, for the replay.
            trace!(
                "heart: samples {=[?]} movement {}",
                second.as_slice(),
                activity::movement()
            );
            info!(
                "heart: brighter after {} s: drive {}, from mean {}, pulse {}",
                elapsed.as_secs(),
                next,
                last_mean,
                last_peak.saturating_sub(last_mean)
            );
            drive = next;
            steps += 1;
            limit = elapsed + Duration::from_secs(u64::from(heart_schedule::BRIGHTER_FOR));
            hrs.on(at(*level, drive)).await?;
            led.switch(drive);
            conversions.restart();
            ppg.reset();
            consensus.reject();
            (peak, sum, count, last_failed) = (0, 0, 0, false);
            second.clear();
            continue;
        }
        // The logging build's longer arm (see `Length`): a background
        // session that has run its usual time on a busy wrist without a
        // reading goes on, at the same drive, with its windows and their
        // consensus carrying on, if the budget covers that and still keeps
        // a session's reserve. Busy, not calm throughout, so never where a
        // brighter phase could start instead; and none after it, so the
        // arms differ in the extra minute alone.
        #[cfg(feature = "field-log")]
        if field.length == Length::Longer
            && limit == SESSION_LIMIT
            && elapsed >= limit
            && measured.is_none()
            && !watching()
            && !heart_schedule::calm_throughout(busy, checks)
            && let Some(budget) = budget
            && heart_schedule::longer(
                drive,
                heart_schedule::LONGER_FOR,
                budget.saturating_sub(led.spent()),
            )
        {
            info!(
                "heart: longer after {} s (busy {}/{})",
                elapsed.as_secs(),
                busy,
                checks
            );
            limit = LONGER_LIMIT;
            steps = heart_schedule::BRIGHTER_STEPS;
        }
        // And one that read before then, on a busy wrist, records on to
        // 90 s (see `Recording`), on the same terms.
        #[cfg(feature = "field-log")]
        if field.length == Length::Longer
            && let Some(bpm) = measured
            && RECORDING.lock(Cell::get).is_none()
            && (limit == SESSION_LIMIT || limit == LONGER_LIMIT)
            && elapsed < LONGER_LIMIT
            && !watching()
            && !heart_schedule::calm_throughout(busy, checks)
            && let Some(budget) = budget
            && heart_schedule::longer(
                drive,
                (LONGER_LIMIT - elapsed).as_secs() as u32 + 1,
                budget.saturating_sub(led.spent()),
            )
        {
            info!(
                "heart: recording on after a reading at {} s (busy {}/{})",
                elapsed.as_secs(),
                busy,
                checks
            );
            RECORDING.lock(|recording| {
                recording.set(Some(Recording {
                    bpm,
                    at: uptime(),
                    level: *level as u8,
                    skin_peak: *skin_peak,
                }))
            });
            limit = LONGER_LIMIT;
            steps = heart_schedule::BRIGHTER_STEPS;
        }
        // Someone's watching, or a session that read is recording on.
        let ended = if watching() || recording_on(elapsed, limit) {
            None
        } else if let Some(bpm) = measured {
            Some(Ended::Measured(bpm))
        } else if elapsed >= limit {
            Some(Ended::TimedOut {
                calm: heart_schedule::calm_throughout(busy, checks),
            })
        } else {
            None
        };
        if let Some(ended) = ended {
            // The last part-second too, which holds the end of the last
            // window: without it, a replay is a window short.
            trace!(
                "heart: samples {=[?]} movement {}",
                second.as_slice(),
                activity::movement()
            );
            return Ok(ended);
        }

        let (raw, ambient) = hrs.sample().await?;
        let mut verdict = None;
        conversions.poll(raw, now.as_micros(), |sample| {
            peak = peak.max(sample);
            sum += sample;
            count += 1;
            // Full only if conversions stalled and then caught up, which
            // the replay can live without.
            let _ = second.push(sample as u16);
            if let Some(v) = ppg.push(sample) {
                verdict = Some(v);
            }
        });

        // Once a second: is the signal within range? If not, change level
        // and start over (samples at different levels aren't comparable).
        if count >= LEVEL_CHECK_SAMPLES {
            let moved = activity::movement();
            movement.write(moved);
            checks = checks.saturating_add(1);
            if moved > CALM {
                busy = busy.saturating_add(1);
            }
            let mean = sum / count;
            // A dev kit can be told that its desk is skin (see
            // `debug::skin_forced`): as far as the level goes, the light
            // is then always enough, neither stepping up nor ending the
            // session for no skin. Half the ADC's range is above `WEAK`
            // and every no-skin threshold, scaled to any drive at the
            // gains it applies to (at most 6,540, at ×4 and 40 mA). (The
            // saturation check stays: it goes by the peak.)
            #[cfg(feature = "debug-input")]
            let mean = if crate::debug::skin_forced() {
                mean.max(FULL_SCALE / 2)
            } else {
                mean
            };
            debug!(
                "heart: level {}, mean {}, peak {}, ambient {}, period {} µs",
                at(*level, drive),
                mean,
                peak,
                ambient,
                conversions.period_us()
            );
            trace!(
                "heart: samples {=[?]} movement {}",
                second.as_slice(),
                activity::movement()
            );
            second.clear();
            let before = (*level, drive);
            if peak >= SATURATED {
                if drive != LEVELS[*level].drive {
                    // Too bright after all: back to the session's level,
                    // for good.
                    drive = LEVELS[*level].drive;
                    steps = heart_schedule::BRIGHTER_STEPS;
                } else if *level == 0 {
                    return Ok(Ended::TooBright);
                } else {
                    *level -= 1;
                }
            } else if LEVELS[*level].drive == 0
                && mean * heart_schedule::DRIVE_CURRENT[0]
                    < (NO_SKIN << LEVELS[*level].gain)
                        * heart_schedule::DRIVE_CURRENT[usize::from(drive)]
            {
                // Off the wrist: little light comes back at the lowest
                // current (scaled to a brighter one); skin reflects
                // several times more.
                return Ok(Ended::NoContact);
            } else if mean < WEAK && *level + 1 < LEVELS.len() {
                *level += 1;
            } else if mean < NO_CONTACT {
                return Ok(Ended::NoContact);
            }
            if *level != before.0 {
                // Any brighter drive was the old level's.
                drive = LEVELS[*level].drive;
            }
            (last_mean, last_peak) = (mean, peak);
            *skin_peak = (peak >> LEVELS[before.0].gain) * heart_schedule::DRIVE_CURRENT[0]
                / heart_schedule::DRIVE_CURRENT[usize::from(before.1)];
            (peak, sum, count) = (0, 0, 0);
            if (*level, drive) != before {
                debug!("heart: switching to level {}", at(*level, drive));
                hrs.on(at(*level, drive)).await?;
                led.switch(drive);
                conversions.restart();
                ppg.reset();
                // Windows either side aren't in a row, to pair.
                consensus.reject();
                last_failed = false;
                continue;
            }
        }

        let Some(verdict) = verdict else {
            continue;
        };
        // A faint window (see `Rejection::Faint`) only counts on a calm
        // wrist, alongside the next; the movement vetoes below can't apply
        // to one, as calm is stricter than either.
        let calm = movement.iter().all(|&m| m <= CALM);
        last_failed = calm
            && matches!(
                verdict,
                Err(Rejection::Faint(_) | Rejection::Aperiodic { .. })
            );
        let (estimate, faint) = match verdict {
            Ok(estimate) => (estimate, false),
            Err(Rejection::Faint(estimate)) => (estimate, true),
            Err(why) => {
                debug!("heart: window rejected: {}", why);
                consensus.reject();
                continue;
            }
        };
        let bpm = estimate.bpm_at(conversions.period_us());
        if faint {
            debug!(
                "heart: window faint: {} BPM (quality {}, calm {})",
                bpm, estimate.quality, calm
            );
            if let Some(bpm) = consensus.push_faint(bpm, calm, activity::asleep()) {
                measured = Some(read(bpm, measured, started, at(*level, drive), conversions));
            }
            continue;
        }
        // Only sustained movement vetoes a window: most of its readings
        // high, not one poll's spike (which used to veto the next 11 s).
        let moving = movement.iter().filter(|&&m| m > MOTION_LIMIT).count();
        if moving * 2 > movement.len() {
            debug!(
                "heart: window rejected: wrist moving ({} BPM, {}/{})",
                bpm,
                moving,
                movement.len()
            );
            consensus.reject();
            continue;
        }
        // Nor a slow one while the wrist was merely busy, awake: that's
        // more likely the sensor shifting than a resting pulse.
        if crate::ppg::slow_while_busy(bpm, movement.as_slice(), CALM, activity::asleep()) {
            debug!("heart: window rejected: slow while busy ({} BPM)", bpm);
            consensus.reject();
            continue;
        }
        debug!(
            "heart: window {} BPM (quality {}, drift {})",
            bpm, estimate.quality, estimate.drift
        );
        if let Some(bpm) = consensus.push(bpm, calm) {
            measured = Some(read(bpm, measured, started, at(*level, drive), conversions));
        }
    }
}

/// Publishes a reading, logging the session's first; returns it, as the
/// session's latest.
fn read(
    bpm: u8,
    measured: Option<u8>,
    started: Instant,
    level: Level,
    conversions: &Conversions,
) -> u8 {
    // Recording on after the session's reading: logged, never published,
    // and the session's reading stays its first (see `Recording`). Unless
    // someone has looked at the clock face meanwhile, who's shown it.
    #[cfg(feature = "field-log")]
    if !watching()
        && let Some(first) = RECORDING.lock(Cell::get)
    {
        debug!(
            "heart: record-only {} BPM after {} s",
            bpm,
            started.elapsed().as_secs()
        );
        return first.bpm;
    }
    if measured.is_none() {
        info!(
            "heart: {} BPM after {} s (level {}, conversion period {} µs)",
            bpm,
            started.elapsed().as_secs(),
            level,
            conversions.period_us()
        );
    } else {
        debug!("heart: {} BPM", bpm);
    }
    publish(bpm);
    bpm
}

// ==============================================================================
// The accelerometer through each session (the logging build)
// ==============================================================================

/// Whether the accelerometer's samples are wanted in the log: not between
/// sessions, through a session, or once more after one (whose last
/// samples are only read then).
#[cfg(feature = "field-log")]
static ACCEL: AtomicU8 = AtomicU8::new(ACCEL_IDLE);
#[cfg(feature = "field-log")]
const ACCEL_IDLE: u8 = 0;
#[cfg(feature = "field-log")]
const ACCEL_SESSION: u8 = 1;
#[cfg(feature = "field-log")]
const ACCEL_TAIL: u8 = 2;

/// Logs the accelerometer's samples from one of `activity`'s polls (every
/// 2 s), if they overlap an awake heart rate session: for replaying the PPG
/// against the wrist's movement offline, to find a busy wrist's rhythm
/// (steps, the arm's swing) and test rejecting what the PPG reads at it
/// (see [`Length`] for why). Here rather than in `activity` for the log's
/// `ur::heart=trace` (`scripts/release.sh`); only in the logging build.
///
/// `accel: [x, y, z, x, y, z, …]`: the poll's samples, oldest first, at
/// the FIFO's rate (nominally 12.5 Hz, but by the accelerometer's own
/// clock: 11.94 Hz on the sealed watch, so a replay should measure it from
/// the lines' times and lengths), the newest a moment before the line's
/// time, in 1/64 g (±2 g, as the chip measures, is ±128, so each axis
/// fits a byte), the same as a capture build's `activity: samples` (see
/// `activity::trace_samples`). That's ~93 bytes a poll (rzCOBS-encoded,
/// measured on a capture log's samples), ~46 a second of session, against
/// the ~100 of the PPG's own lines; ~24 samples a line, or fewer when a
/// read failed or the FIFO was flushed. The movement's fundamental (a
/// walk's ~1.8 Hz of steps, or ~0.9 Hz of arm swing) is well below the
/// ~6 Hz this resolves.
#[cfg(feature = "field-log")]
pub fn trace_accel(frames: &[[i16; 3]]) {
    let wanted = match ACCEL.load(Ordering::Relaxed) {
        ACCEL_IDLE => false,
        ACCEL_SESSION => true,
        // The poll after a session: the last it overlaps. (Unless a new
        // session has started meanwhile, which stays wanted.)
        _ => {
            let _ = ACCEL.compare_exchange(
                ACCEL_TAIL,
                ACCEL_IDLE,
                Ordering::Relaxed,
                Ordering::Relaxed,
            );
            true
        }
    };
    if !wanted {
        return;
    }
    // A FIFO read holds at most 36 frames (`activity::drain_fifo`).
    let mut raw = [0i8; 3 * 36];
    let samples = frames.len().min(36);
    for (out, &axis) in raw.iter_mut().zip(frames[..samples].as_flattened()) {
        *out = (axis >> 4).clamp(-128, 127) as i8;
    }
    trace!("accel: {=[?]}", raw[..3 * samples]);
}
