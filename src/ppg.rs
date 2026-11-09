//! Heart rate from a PPG (photoplethysmogram): pure signal processing.
//!
//! A PPG sensor shines a green LED into the skin and measures how much
//! light comes back. Blood absorbs green light, so each heartbeat's pulse
//! of blood shows up as a small dip in the reflected light, riding on a
//! large constant level (skin, bone, venous blood) that also drifts with
//! breathing, posture, and how tightly the strap sits. The dip is typically
//! around 1% of the level, and anything that moves the sensor against the
//! skin swamps it.
//!
//! Turning that into a heart rate takes four steps:
//!
//! 0. **Sampling** ([`Conversions`]): the sensor converts on its own
//!    oscillator, and its samples must be taken at *its* pace, not ours;
//!    see there for why.
//! 1. **Filtering** ([`Ppg::push`]): a 0.5–4 Hz bandpass (30–240 BPM, with
//!    margin either side of the 40–200 BPM we report) removes the constant
//!    level, the drift, and breathing below the band, and sensor noise
//!    above it.
//! 2. **Period estimation** (`Ppg::estimate`): the autocorrelation of the
//!    last 8 s of filtered signal peaks at lags that are a whole number of
//!    beats. The strongest early peak is the beat period. Autocorrelation
//!    uses every sample of every beat, which makes it far more robust to
//!    noise than finding individual pulse peaks, and it doesn't care about
//!    the pulse's shape (which varies a lot between people and sites).
//! 3. **Rejection**: a window only yields a rate if it looks like a pulse.
//!    Motion makes bursts: seconds much louder than the rest are masked
//!    out, and what's left must be steady and most of the window. The
//!    autocorrelation peak must be high (a periodic signal correlates
//!    strongly with itself one period later; filtered noise doesn't). A
//!    slow rate must also come from a steady light level: a shifting
//!    sensor's filtered steps look like a slow pulse (see
//!    [`MAX_SLOW_DRIFT`]). Finally, [`Consensus`] only reports once several
//!    windows, spread over more than one window's worth of time, agree;
//!    on a calm wrist, two in a row that agree closely are enough, even
//!    if neither is quite periodic enough on its own
//!    ([`Rejection::Faint`]).
//!
//! Nothing here touches hardware: it takes raw counts and timestamps, and
//! makes no assumptions about the counts' scale, so it can be exercised on
//! the host against synthetic signals.

// ==============================================================================
// Parameters
// ==============================================================================

/// The sensor's nominal time per sample: a 25 ms conversion (at 14 bits)
/// plus a 12.5 ms wait, from the HRS3300 datasheet. The real period differs
/// by however far the sensor's oscillator is off; [`Conversions`] measures
/// it, and [`Estimate::bpm_at`] corrects for it.
pub const NOMINAL_PERIOD_US: u32 = 37_500;

/// The sample rate everything below is designed for (~26.7 Hz).
pub const SAMPLE_HZ: f32 = 1e6 / NOMINAL_PERIOD_US as f32;

/// Samples per analysis window: 8 s, so even at 40 BPM it holds five beats,
/// and the autocorrelation at the longest lag still sums over 6 s. A
/// multiple of four, for the steadiness check's quarters.
pub const WINDOW: usize = 212;

/// Conversions to throw away after the sensor starts (or restarts at a new
/// level): the first comes out 3-5% darker than the rest, presumably while
/// the LED warms up. That's a step of 70-160 counts, ten or twenty times
/// the pulse, and primed with it, the highpass rang for seconds: in field
/// logs, the first two windows of a session were dominated by the
/// transient, and a desk-bound session, which gives up at 12 s, has only
/// those two.
const WARMUP: usize = 1;

/// Samples to discard after warming up, while the filters settle. The
/// highpass is primed with the first sample it gets (see [`Ppg::push`]),
/// so this only has to cover the response to whatever the signal does in
/// its first moments: half a second. (A second, which this was while the
/// warm-up sample went through the filters, cost every session a second's
/// LED before its first window, for no gain in field-log replays.)
const SETTLE: usize = 13;

/// Samples between estimates: 2 s. Consecutive windows overlap by 6 s, so
/// estimates closer together than this would mostly repeat each other, and
/// [`Consensus`] wants agreement across *different* data.
pub const ESTIMATE_EVERY: usize = 53;

/// The range of rates we report. The autocorrelation is searched over
/// exactly the lags between these.
const MIN_BPM: f32 = 40.0;
const MAX_BPM: f32 = 200.0;
const MIN_LAG: usize = (60.0 * SAMPLE_HZ / MAX_BPM) as usize;
/// One past the longest lag of interest, so a peak there can be confirmed
/// as a peak (its right-hand neighbour must be lower).
const MAX_LAG: usize = (60.0 * SAMPLE_HZ / MIN_BPM) as usize + 1;

/// The filters' corner frequencies. Below 0.5 Hz is breathing (0.1–0.4 Hz)
/// and drift; above 4 Hz is noise and the pulse's higher harmonics, which
/// only sharpen its shape and don't help find the period. (These hold at
/// the nominal sample rate; the sensor's real rate moves them by a few
/// percent, which doesn't matter.)
// Only the test designs from these; the filters hold its results.
#[cfg_attr(not(test), allow(dead_code))]
const HIGHPASS_HZ: f32 = 0.5;
#[cfg_attr(not(test), allow(dead_code))]
const LOWPASS_HZ: f32 = 4.0;

/// `Biquad::design(HIGHPASS_HZ, true)` and `(LOWPASS_HZ, false)`, as the
/// test below checks.
const HIGHPASS: Biquad =
    Biquad::from_bits([0x3f6b8974, 0xbfeb8974, 0x3f6b8974, 0xbfeab7c0, 0x3f58b651]);
const LOWPASS: Biquad =
    Biquad::from_bits([0x3e0640c4, 0x3e8640c4, 0x3e0640c4, 0xbf3f6f1c, 0x3e8b5fc1]);

/// Parts the window is split into to find motion bursts: each ~1 s, so a
/// burst costs only the second or two it lasts, yet holds most of a beat
/// even at 40 BPM, so a pulse's parts have comparable energies.
const PARTS: usize = 8;

/// How much louder than the window's median part (in amplitude) a part
/// may be before it's masked out as motion. A pulse's amplitude wanders
/// slowly, with breathing say, by tens of percent; moving the arm or
/// fingers makes bursts many times the pulse.
///
/// This used to reject the whole window, judged by its loudest quarter
/// against its quietest (at 4.5x). At a desk, though, the hand makes a
/// burst every few seconds, and in field logs of desk days 45% of
/// windows were rejected that way, against 18% asleep: hardly any 8 s
/// window was free of one. Most of those windows had nothing to recover
/// (only 7% had their best peak at a Garmin's rate, and letting through
/// ratios up to 8 gained one window in 800), but masking the burst and
/// measuring the rest accepted half as many windows again (71 to 110 of
/// ~830), and about as accurately (80% within 5 BPM of the Garmin, from
/// 83%).
const MAX_PART_AMPLITUDE: f32 = 2.0;

/// How much the parts left after masking may vary: the ratio of the
/// loudest one's amplitude to the quietest's. Masking only takes out
/// bursts that stand out from the median part, so it can't help a window
/// that's mostly motion: then the median is motion, and only the
/// remaining pulse stands out, as the quietest.
///
/// On a real wrist (field log, sitting) the pulse's own ratio between 2 s
/// quarters ran 3.1–4.0; arm movement read 5–10. Between 1 s parts a
/// pulse varies a little more, but replaying field logs, comparing parts
/// rather than quarters lost only 3 of 476 accepted windows.
const MAX_AMPLITUDE_RATIO: f32 = 4.5;

/// Parts that must be left after masking. Five of eight (5.3 s) still
/// hold four beats at 40 BPM, and every lag up to the longest still has
/// pairs of unmasked samples to correlate; with fewer, too many windows
/// that were mostly motion got through.
const MIN_QUIET_PARTS: usize = 5;

/// The normalised autocorrelation a window's beat period must reach (1.0
/// is perfectly periodic). Clean synthetic pulses with realistic heart-rate
/// variability reach 0.7–0.95 (fast ones the least, as a few percent of
/// variation in a short period is more samples' worth of smearing);
/// bandpass-filtered noise has peaks around 0.3–0.5, because the filter
/// itself makes noise look a bit periodic. A real wrist window at 70 BPM
/// reached 0.595 (and the Garmin agreed), so slightly under 0.6; noise is
/// still kept out by needing three windows to agree (`Consensus`).
const MIN_CORRELATION: f32 = 0.55;

/// [`MIN_CORRELATION`] for a window with motion masked out: what's left
/// of it is shorter, and what came just before or after a burst may still
/// carry some of it. In field-log replays, masked windows at 0.55-0.6
/// were within 5 BPM of a Garmin only half the time (6 of 11), against
/// 72% above 0.6. That's few windows to go by: this errs on the side of
/// caution.
const MIN_MASKED_CORRELATION: f32 = 0.6;

/// The normalised autocorrelation a window needs to be [`Rejection::Faint`]
/// rather than [`Rejection::Aperiodic`]: short of the bars above, but
/// enough to count towards a reading on a calm wrist, if the next window
/// agrees (see [`Consensus::push_faint`]).
///
/// A still wrist that isn't asleep (lying down awake, cold hands) can
/// have a pulse of only a few counts, beside the slower swings of
/// breathing and blood flow, and its windows land at the right rate with
/// quality 0.3-0.5, again and again. In a field log (2026-11-09, lying
/// still for an hour, a Garmin reading 62-67), three 30 s sessions gave no
/// reading, though one had eight of its eleven windows at 57-64 BPM, of
/// quality 0.18-0.48.
///
/// Replaying every recorded session (252, from a night, two days, an
/// evening and an afternoon, against a Garmin), calm pairs made 118
/// readings where three accepted windows made 106, in 12 s rather than
/// 13 on average (a
/// night's LED time fell 15%). One of the changed readings went wrong (65
/// BPM against 50, from two good windows during a rise the Garmin didn't
/// see), and one went right, so 5 are more than 8 BPM off either way.
/// Floors of 0.35 and 0.4 made the same 118 readings; 0.5 made 116, one
/// more of them wrong, and lost the hour above.
///
/// Bandpass-filtered noise reaches 0.3-0.5 too, and as consecutive windows
/// share 6 s of signal, its peak can hold still for both: in synthetic
/// white noise on a calm wrist, 2-4 of 200 runs of 300 s made a reading,
/// about one per 500-1000 sessions without a pulse to be seen (see
/// host-tests/ppg). A three-window agreement makes none.
const MIN_FAINT_CORRELATION: f32 = 0.45;

/// The first autocorrelation peak within this fraction of the highest one
/// is the period. A clean signal correlates almost as well at two or three
/// periods as at one, and with noise a later peak may come out higher;
/// taking it would halve or third the rate. In simulation, 0.85 still let
/// noisy fast pulses be read at half rate, while 0.6 never picked a peak
/// short of the period (there's no strong peak at half a period: the
/// signal is then roughly inverted).
const OCTAVE_TOLERANCE: f32 = 0.6;

/// Windows slower than this (BPM) must have a steady light level (see
/// [`MAX_SLOW_DRIFT`]), and a wrist that kept still (see
/// [`slow_while_busy`]).
const SLOW_BPM: f32 = 50.0;

/// How far the raw light level may move across a window slower than
/// [`SLOW_BPM`]: the spread of its quarters' mean levels, as a fraction of
/// the lowest (see [`Ppg::drift`]).
///
/// When the sensor shifts against the skin, the level steps or ramps by
/// tens of percent, and the highpass turns that into a slow swing just
/// above its corner, which can correlate with itself well enough to pass
/// for a pulse of 40–45 BPM. In a field log, a desk-bound wrist read 42
/// BPM (a Garmin said 67) from three windows of quality 0.57–0.62 while the
/// level climbed 14–24% across each. Over 140 accepted windows from field
/// logs, genuine ones under 50 BPM (asleep, down to 45) moved at most 4%;
/// faster ones, at a desk, up to 15% while still matching the Garmin, which
/// is why faster windows are left alone.
const MAX_SLOW_DRIFT: f32 = 0.10;

/// Below this RMS (in raw counts), the window is flat: no light, a
/// saturated or stuck sensor, or ADC noise alone.
const MIN_RMS: f32 = 0.5;

// ==============================================================================
// Filtering
// ==============================================================================

/// A second-order IIR section, in transposed direct form II (which keeps
/// the state small and well-conditioned in `f32`).
///
/// Coefficients are from Robert Bristow-Johnson's "Audio EQ Cookbook", with
/// Q = 1/√2 (Butterworth: maximally flat in the passband, no ringing
/// peak to exaggerate one heart rate over another).
#[derive(Clone, Copy)]
struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

impl Biquad {
    /// The cookbook's section for `cutoff_hz`. Only for checking
    /// [`HIGHPASS`] and [`LOWPASS`], which hold its results: on the watch,
    /// `libm`'s `sinf` and `cosf` compute in `f64`, whose soft-float
    /// routines (the M4F's FPU is single precision) cost ~3 KB of flash.
    #[cfg(test)]
    fn design(cutoff_hz: f32, highpass: bool) -> Self {
        let w0 = 2.0 * core::f32::consts::PI * cutoff_hz / SAMPLE_HZ;
        let cos = libm::cosf(w0);
        let alpha = libm::sinf(w0) / (2.0 * core::f32::consts::FRAC_1_SQRT_2);
        let a0 = 1.0 + alpha;
        let (b0, b1) = if highpass {
            ((1.0 + cos) / 2.0, -(1.0 + cos))
        } else {
            ((1.0 - cos) / 2.0, 1.0 - cos)
        };
        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b0 / a0,
            a1: -2.0 * cos / a0,
            a2: (1.0 - alpha) / a0,
            z1: 0.0,
            z2: 0.0,
        }
    }

    /// A section with coefficients `[b0, b1, b2, a1, a2]`, as their bits
    /// (exact, unlike decimal literals).
    const fn from_bits(bits: [u32; 5]) -> Self {
        let [b0, b1, b2, a1, a2] = bits;
        Self {
            b0: f32::from_bits(b0),
            b1: f32::from_bits(b1),
            b2: f32::from_bits(b2),
            a1: f32::from_bits(a1),
            a2: f32::from_bits(a2),
            z1: 0.0,
            z2: 0.0,
        }
    }

    fn step(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }
}

// ==============================================================================
// The estimator
// ==============================================================================

/// One window's heart rate.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(target_os = "none", derive(defmt::Format))]
pub struct Estimate {
    /// The beat period, in samples.
    pub period: f32,
    /// The autocorrelation at the beat period: how periodic the window was.
    pub quality: f32,
    /// How far the raw light level moved across the window (see
    /// [`Ppg::drift`]).
    pub drift: f32,
}

impl Estimate {
    /// The heart rate if samples came exactly [`NOMINAL_PERIOD_US`] apart.
    pub fn bpm(&self) -> f32 {
        self.bpm_at(NOMINAL_PERIOD_US as f32)
    }

    /// The heart rate given the measured time between samples.
    pub fn bpm_at(&self, sample_period_us: f32) -> f32 {
        60e6 / (self.period * sample_period_us)
    }
}

/// Why a window didn't yield a heart rate, with the numbers behind the
/// verdict (logged, to tune the thresholds from field data).
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(target_os = "none", derive(defmt::Format))]
pub enum Rejection {
    /// No signal at all.
    Flat,
    /// The amplitude jumps around, through more of the window than can be
    /// masked out: most likely movement. `ratio` is the loudest part's
    /// amplitude over the quietest's (see [`MAX_PART_AMPLITUDE`] and
    /// [`MAX_AMPLITUDE_RATIO`]).
    Unsteady { ratio: f32 },
    /// Not periodic within 40–200 BPM: noise, or a pulse too weak to tell
    /// from it. `quality` is the best correlation found (under
    /// [`MIN_FAINT_CORRELATION`], or its rate out of range), and `bpm` its
    /// rate (NaN if there was no peak at all).
    Aperiodic { quality: f32, bpm: f32 },
    /// Slow, while the light level shifted: most likely the sensor moving
    /// on the skin (see [`MAX_SLOW_DRIFT`]).
    Drifting { drift: f32, bpm: f32 },
    /// Periodic within 40–200 BPM, but not enough to stand on its own (at
    /// least [`MIN_FAINT_CORRELATION`], short of [`MIN_CORRELATION`] or
    /// [`MIN_MASKED_CORRELATION`]): a weak pulse, or noise. It may still
    /// count towards a reading on a calm wrist (see
    /// [`Consensus::push_faint`]).
    Faint(Estimate),
}

/// Filters raw samples and estimates the heart rate from them every
/// [`ESTIMATE_EVERY`] samples.
pub struct Ppg {
    /// A fourth-order highpass (two sections): breathing sits only an
    /// octave or so below the slowest heart rates, and with second order
    /// alone it came through at a quarter of its size.
    highpass: [Biquad; 2],
    lowpass: Biquad,
    /// The first raw sample, subtracted from every sample. Starting the
    /// highpass at zero instead of at a large constant level avoids a step
    /// response that would take seconds to die away.
    offset: Option<f32>,
    /// The filtered signal, as a ring; `next` is the oldest sample.
    window: [f32; WINDOW],
    next: usize,
    /// Samples since the last reset, saturating.
    seen: usize,
    /// The mean raw level of each [`ESTIMATE_EVERY`]-sample block of the
    /// window, as a ring (`block_next` is the oldest), and the sum of the
    /// block in progress: the window's four quarters, for [`Ppg::drift`].
    blocks: [f32; BLOCKS],
    block_next: usize,
    block_sum: f32,
}

/// Blocks of [`ESTIMATE_EVERY`] samples per window.
const BLOCKS: usize = WINDOW / ESTIMATE_EVERY;
const _: () = assert!(WINDOW == BLOCKS * ESTIMATE_EVERY);

impl Default for Ppg {
    fn default() -> Self {
        Self::new()
    }
}

impl Ppg {
    pub fn new() -> Self {
        Self {
            highpass: [HIGHPASS; 2],
            lowpass: LOWPASS,
            offset: None,
            window: [0.0; WINDOW],
            next: 0,
            seen: 0,
            blocks: [0.0; BLOCKS],
            block_next: 0,
            block_sum: 0.0,
        }
    }

    /// Starts over, e.g. after the sensor's gain changed (which makes the
    /// samples so far incomparable with those to come).
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// Adds one raw sample. Every [`ESTIMATE_EVERY`] samples once the
    /// window is full, returns that window's verdict.
    pub fn push(&mut self, raw: u32) -> Option<Result<Estimate, Rejection>> {
        self.seen = self.seen.saturating_add(1);
        if self.seen <= WARMUP {
            return None;
        }
        let raw = raw as f32;
        let offset = *self.offset.get_or_insert(raw);
        let mut x = raw - offset;
        for section in &mut self.highpass {
            x = section.step(x);
        }
        x = self.lowpass.step(x);

        if self.seen <= WARMUP + SETTLE {
            return None;
        }
        self.window[self.next] = x;
        self.next = (self.next + 1) % WINDOW;

        let stored = self.seen - WARMUP - SETTLE;
        self.block_sum += raw;
        if stored.is_multiple_of(ESTIMATE_EVERY) {
            self.blocks[self.block_next] = self.block_sum / ESTIMATE_EVERY as f32;
            self.block_next = (self.block_next + 1) % BLOCKS;
            self.block_sum = 0.0;
        }
        if stored >= WINDOW && (stored - WINDOW).is_multiple_of(ESTIMATE_EVERY) {
            Some(self.estimate())
        } else {
            None
        }
    }

    /// Estimates the heart rate from the current window.
    fn estimate(&self) -> Result<Estimate, Rejection> {
        // Unroll the ring, oldest first, and remove what little DC the
        // highpass left (it matters for the correlation's normalisation).
        let mut x = [0.0f32; WINDOW];
        for (i, sample) in x.iter_mut().enumerate() {
            *sample = self.window[(self.next + i) % WINDOW];
        }
        let mean = x.iter().sum::<f32>() / WINDOW as f32;
        for sample in &mut x {
            *sample -= mean;
        }

        let energy = |part: &[f32]| part.iter().map(|v| v * v).sum::<f32>();
        let rms = libm::sqrtf(energy(&x) / WINDOW as f32);
        if rms < MIN_RMS {
            return Err(Rejection::Flat);
        }

        // Steadiness: find the parts much louder than the window's median
        // part, and mask them out (see `MAX_PART_AMPLITUDE`). The median,
        // not the mean or the quietest: a burst inflates the mean, and a
        // pulse's quietest second can be far quieter than its typical one.
        let part_of = |i: usize| i * PARTS / WINDOW;
        let mut loudness = [0.0f32; PARTS];
        let mut lengths = [0usize; PARTS];
        for (i, v) in x.iter().enumerate() {
            loudness[part_of(i)] += v * v;
            lengths[part_of(i)] += 1;
        }
        for (e, n) in loudness.iter_mut().zip(lengths) {
            *e /= n as f32;
        }
        let mut sorted = loudness;
        sorted.sort_unstable_by(f32::total_cmp);
        let median = (sorted[PARTS / 2 - 1] + sorted[PARTS / 2]) / 2.0;
        let limit = median * MAX_PART_AMPLITUDE * MAX_PART_AMPLITUDE;
        let quiet = loudness.map(|e| e <= limit);
        let quiet_parts = quiet.iter().filter(|&&q| q).count();
        // The loudest part left: `sorted` ascends, and the quiet parts
        // are its first ones.
        let loudest_left = sorted[quiet_parts.max(1) - 1];
        let ratio_squared = MAX_AMPLITUDE_RATIO * MAX_AMPLITUDE_RATIO;
        if quiet_parts < MIN_QUIET_PARTS || loudest_left > sorted[0] * ratio_squared {
            // Energies, so the amplitude ratio is the square root.
            let ratio = libm::sqrtf(sorted[PARTS - 1] / sorted[0].max(f32::MIN_POSITIVE));
            return Err(Rejection::Unsteady { ratio });
        }
        let masked = quiet_parts < PARTS;
        if masked {
            // Masked samples become zero, so they drop out of every
            // product below; the rest are re-centred on their own mean.
            let mut sum = 0.0;
            for (i, v) in x.iter().enumerate() {
                if quiet[part_of(i)] {
                    sum += v;
                }
            }
            let kept = (0..PARTS)
                .filter(|&p| quiet[p])
                .map(|p| lengths[p])
                .sum::<usize>();
            let mean = sum / kept as f32;
            for (i, v) in x.iter_mut().enumerate() {
                *v = if quiet[part_of(i)] { *v - mean } else { 0.0 };
            }
        }

        // The normalised autocorrelation at each lag of interest: the
        // correlation coefficient between the window and itself shifted by
        // `lag`. Normalising each lag by the energy of exactly the samples
        // it uses (rather than the whole window's) keeps long lags, which
        // sum over fewer samples, comparable with short ones. With parts
        // masked, a sample is only used where its partner, `lag` away, is
        // unmasked too.
        let mut r = [0.0f32; MAX_LAG + 1];
        for (lag, r) in r.iter_mut().enumerate().skip(MIN_LAG - 1) {
            let (early, late) = (&x[..WINDOW - lag], &x[lag..]);
            let cross: f32 = early.iter().zip(late).map(|(a, b)| a * b).sum();
            let norm = if masked {
                let (mut e, mut l) = (0.0, 0.0);
                for (i, (a, b)) in early.iter().zip(late).enumerate() {
                    if quiet[part_of(i)] && quiet[part_of(i + lag)] {
                        e += a * a;
                        l += b * b;
                    }
                }
                libm::sqrtf(e * l)
            } else {
                libm::sqrtf(energy(early) * energy(late))
            };
            *r = if norm > 0.0 { cross / norm } else { 0.0 };
        }

        // Local maxima within the lag range, located between whole lags.
        //
        // Lags are whole samples, which at high rates is coarse (180 BPM is
        // 8.3 samples, so ±0.5 samples is ±11 BPM). Fitting a parabola
        // through each peak and its neighbours finds where the true peak
        // lies between them, and how high it reaches. The height matters as
        // much as the position: a sharp peak that falls between two lags
        // is sampled well below its top, and compared at whole lags, a
        // multiple of the period that happens to land on a whole lag can
        // look higher than the period itself.
        let peak = |lag: usize| {
            let (left, centre, right) = (r[lag - 1], r[lag], r[lag + 1]);
            if !(centre >= left && centre > right) {
                return None;
            }
            let curvature = left - 2.0 * centre + right;
            let shift = (0.5 * (left - right) / curvature).clamp(-0.5, 0.5);
            let height = centre - 0.25 * (left - right) * shift;
            Some((lag as f32 + shift, height.min(1.0)))
        };
        let lags = MIN_LAG..MAX_LAG;
        let best = lags
            .clone()
            .filter_map(peak)
            .map(|(_, height)| height)
            .fold(f32::NEG_INFINITY, f32::max);
        // The period is the first peak that's nearly as high as the highest
        // (see `OCTAVE_TOLERANCE`).
        let Some((period, quality)) = lags
            .filter_map(peak)
            .find(|&(_, height)| height >= best * OCTAVE_TOLERANCE)
        else {
            return Err(Rejection::Aperiodic {
                quality: best,
                bpm: f32::NAN,
            });
        };
        let estimate = Estimate {
            period,
            quality,
            drift: self.drift(),
        };
        let bpm = estimate.bpm();
        if quality < MIN_FAINT_CORRELATION || !(MIN_BPM..=MAX_BPM).contains(&bpm) {
            return Err(Rejection::Aperiodic { quality, bpm });
        }
        if bpm < SLOW_BPM && estimate.drift > MAX_SLOW_DRIFT {
            return Err(Rejection::Drifting {
                drift: estimate.drift,
                bpm,
            });
        }
        let min_quality = if masked {
            MIN_MASKED_CORRELATION
        } else {
            MIN_CORRELATION
        };
        if quality < min_quality {
            return Err(Rejection::Faint(estimate));
        }
        Ok(estimate)
    }
}

impl Ppg {
    /// How far the raw light level moved across the window: the spread of
    /// its quarters' mean levels, as a fraction of the lowest. Quarters
    /// (2 s) rather than samples, so the pulse and noise average out.
    fn drift(&self) -> f32 {
        let (low, high) = self
            .blocks
            .iter()
            .fold((f32::INFINITY, 0.0f32), |(low, high), &b| {
                (low.min(b), high.max(b))
            });
        if low > 0.0 { (high - low) / low } else { 0.0 }
    }
}

/// Whether a window slower than [`SLOW_BPM`] came while the wrist was
/// busy, awake: at least half of `movement`, the wrist's movement readings
/// through the window, above `calm` (`heart::CALM`), and the sleep
/// classifier not saying `asleep`. Such a window isn't to be trusted.
///
/// A heart rate under 50 is a resting one, and awake, resting means a
/// still wrist. A busy wrist, though, can shift the sensor back and forth
/// slowly enough to correlate like a slow pulse while the light level
/// stays put, which [`MAX_SLOW_DRIFT`] can't catch. In field logs, every
/// accepted window under 50 BPM at a desk was wrong (one session read 45
/// BPM with a Garmin at 71, at movement 450-950), while of 212 asleep,
/// all but 2 came with a calm wrist, and those 2 were right: turning over
/// in bed is a few seconds' movement, after which the pulse is as slow
/// as before.
pub fn slow_while_busy(bpm: f32, movement: &[u16], calm: u16, asleep: bool) -> bool {
    let busy = movement.iter().filter(|&&m| m > calm).count();
    bpm < SLOW_BPM && !asleep && !movement.is_empty() && busy * 2 >= movement.len()
}

// ==============================================================================
// Sampling at the sensor's pace
// ==============================================================================

/// How long rate measurement must run before it's trusted: long enough
/// that the uncertainty in when we noticed a conversion (one poll) is a
/// small fraction of it.
const RATE_SPAN_US: u64 = 2_000_000;

/// Turns frequent polls of the sensor's latest result into its sequence of
/// conversions, one sample each, and measures how far apart they really
/// are.
///
/// Why not simply read the sensor at a fixed rate: it converts on its own
/// oscillator, which may be several percent off. Reading it every 40 ms
/// while it converts every 37.5 ms (say) makes each sample's age, the time
/// since its conversion, sawtooth between nothing and a whole conversion
/// period, at the beat frequency between the two clocks. That beat can land
/// anywhere from 0 to several Hz depending on the oscillator, often right
/// in the heart-rate band, and any slope in the light level (drift,
/// breathing, the pulse itself) times the sawtooth age is a periodic
/// artefact. In simulation the estimator duly found it: drift and
/// breathing alone yielded confident heart rates, and fast pulses were read
/// at half or a third of their rate.
///
/// Instead, the sensor is polled several times per conversion, and each
/// change of value is a new conversion, so the sensor's own clock spaces
/// the samples evenly. Two subtleties:
///
/// - Consecutive conversions can give the same value. A stretch without a
///   change that's longer than a period must hide repeats, so the value is
///   emitted once per period elapsed.
/// - A read may straddle the sensor updating its result, mixing bytes of
///   two conversions. That shows up as a change followed by another much
///   sooner than a period: the first was torn, and is replaced.
///
/// Timing comes from the polls, which the executor may delay (in
/// simulation, by up to 15 ms now and then). A conversion started somewhere
/// between the last poll that showed the old value and the first that
/// shows the new one, so it's dated halfway between them. Dating it at the
/// poll that noticed it instead made a late poll stretch the previous
/// conversion to 1.5 periods, which counted as two.
pub struct Conversions {
    /// The time between conversions: nominal until measured.
    period_us: f32,
    /// The current conversion's value, and when it (probably) started.
    current: Option<(u32, u64)>,
    /// When the previous poll was.
    last_poll_us: u64,
    /// Samples emitted since the last restart.
    emitted: u32,
    /// For measuring the rate: when the first conversion after a restart
    /// started, and `emitted` at that point.
    reference: Option<(u64, u32)>,
}

impl Default for Conversions {
    fn default() -> Self {
        Self::new()
    }
}

impl Conversions {
    pub const fn new() -> Self {
        Self {
            period_us: NOMINAL_PERIOD_US as f32,
            current: None,
            last_poll_us: 0,
            emitted: 0,
            reference: None,
        }
    }

    /// Forgets the current conversion and the rate measurement in progress
    /// (but not the rate measured so far), e.g. after reconfiguring the
    /// sensor, which may restart its conversion cycle.
    pub fn restart(&mut self) {
        *self = Self {
            period_us: self.period_us,
            ..Self::new()
        };
    }

    /// The time between conversions, as measured (or nominal if not yet
    /// measured).
    pub fn period_us(&self) -> f32 {
        self.period_us
    }

    /// Takes one poll of the sensor's latest result at `now_us`, and calls
    /// `emit` with each conversion it completes, oldest first.
    pub fn poll(&mut self, raw: u32, now_us: u64, mut emit: impl FnMut(u32)) {
        let previous_poll = core::mem::replace(&mut self.last_poll_us, now_us);
        let Some((value, since)) = self.current else {
            self.current = Some((raw, now_us));
            return;
        };
        let period = self.period_us;

        if raw == value {
            // A whole period (and then some, for timing slop) with no change:
            // the sensor must have repeated itself.
            if now_us.saturating_sub(since) as f32 >= 1.5 * period {
                emit(value);
                self.emitted += 1;
                self.current = Some((value, since + period as u64));
            }
            return;
        }

        let started = previous_poll + now_us.saturating_sub(previous_poll) / 2;
        let elapsed = started.saturating_sub(since) as f32;
        if elapsed < 0.5 * period {
            // Torn read: what we saw before was half old, half new.
            self.current = Some((raw, since));
            return;
        }

        // A new conversion. The previous value stood for however many
        // periods it was there (usually one).
        let repeats = (libm::roundf(elapsed / period) as u32).max(1);
        for _ in 0..repeats {
            emit(value);
        }
        self.emitted += repeats;
        self.current = Some((raw, started));

        // The rate: conversions emitted since the reference conversion
        // started, over the time since. A measurement far from nominal
        // means we're miscounting (or the sensor isn't doing what the
        // datasheet says), so it's not believed.
        match self.reference {
            None => self.reference = Some((started, self.emitted)),
            Some((start, emitted)) if started - start >= RATE_SPAN_US => {
                let measured = (started - start) as f32 / (self.emitted - emitted) as f32;
                let nominal = NOMINAL_PERIOD_US as f32;
                if (0.75 * nominal..1.33 * nominal).contains(&measured) {
                    self.period_us = measured;
                }
            }
            Some(_) => {}
        }
    }
}

// ==============================================================================
// Consensus
// ==============================================================================

/// How many agreeing estimates make a reading. Estimates are 2 s apart and
/// windows 8 s long, so three span 12 s of signal, half of it seen by only
/// one of them.
const AGREEING: usize = 3;

/// How far apart (max − min, in BPM) agreeing estimates may be. Real heart
/// rate changes by a few BPM over 12 s at rest; a wrong estimate (noise
/// that happened to look periodic, or an octave error) is usually off by
/// far more.
const AGREEMENT_BPM: f32 = 6.0;

/// How far apart (BPM) two windows in a row on a calm wrist may be to make
/// a reading between them (see [`Consensus::push_faint`]). Tighter than
/// [`AGREEMENT_BPM`], as it stands in for a third window. In field-log
/// replays, 2 BPM lost two readings that agreed with the Garmin, and 4 or
/// 6 gained none.
const CALM_AGREEMENT_BPM: f32 = 3.0;

/// The fastest rate a calm pair (see [`Consensus`]) may read. A calm wrist's
/// pulse is a resting one, and the faster the rate, the fewer samples a
/// beat spans, so the coarser each window's estimate: at 170 BPM, two
/// agreeing windows came out up to 5.7 BPM off in synthetic tests (and
/// 9.9 with motion masked out), where three were within 3.1 (4.8). Faster
/// pulses still read the usual way, from three windows.
const MAX_CALM_BPM: f32 = 120.0;

/// Collects window estimates and decides when they agree on a heart rate.
///
/// Two ways to agree:
///
/// - [`AGREEING`] accepted windows within [`AGREEMENT_BPM`], wherever
///   they fall in the session (see [`Consensus::push`]), or
/// - on a calm wrist, two windows in a row within
///   [`CALM_AGREEMENT_BPM`], either of them merely [`Rejection::Faint`]
///   (see [`Consensus::push_faint`]). Calm, because stillness is what
///   lets a weak pulse be seen at all, and what a moving wrist's
///   periodic-looking swings lack. That's also ~2 s sooner, and so 2 s
///   less LED, than waiting for a third window.
pub struct Consensus {
    /// The latest accepted estimates, oldest first.
    recent: [f32; AGREEING],
    len: usize,
    /// The previous window's rate, if it was accepted or faint, on a calm
    /// wrist; NaN if not (or if it was rejected).
    calm: f32,
}

impl Default for Consensus {
    fn default() -> Self {
        Self::new()
    }
}

impl Consensus {
    pub const fn new() -> Self {
        Self {
            recent: [0.0; AGREEING],
            len: 0,
            calm: f32::NAN,
        }
    }

    /// Adds an accepted estimate, `calm` saying whether the wrist was calm
    /// throughout its window. Returns the heart rate (the median of the
    /// latest estimates) if they agree.
    ///
    /// Rejected windows don't reset the [`AGREEING`] estimates: a moment of
    /// motion between two good stretches doesn't make either stretch less
    /// trustworthy. (They do break a calm pair: see [`Consensus::reject`].)
    pub fn push(&mut self, bpm: f32, calm: bool) -> Option<u8> {
        let paired = self.pair(bpm, calm);
        if self.len == AGREEING {
            self.recent.copy_within(1.., 0);
            self.len -= 1;
        }
        self.recent[self.len] = bpm;
        self.len += 1;
        if self.len < AGREEING {
            return paired;
        }

        let mut sorted = self.recent;
        sorted.sort_unstable_by(f32::total_cmp);
        if sorted[AGREEING - 1] - sorted[0] > AGREEMENT_BPM {
            return paired;
        }
        Some(libm::roundf(sorted[AGREEING / 2]) as u8)
    }

    /// Adds a [`Rejection::Faint`] window's rate: it only counts on a calm
    /// wrist, agreeing with the window just before, and if it's slower
    /// than [`SLOW_BPM`], only `asleep`.
    ///
    /// Awake, a pulse that slow is rare, while bandpass-filtered noise
    /// lands there often: of 12 pairs that agreed in 200 runs of 300 s of
    /// two kinds of synthetic noise, 6 were at 45-49 BPM. It cost no
    /// reading in field-log replays.
    pub fn push_faint(&mut self, bpm: f32, calm: bool, asleep: bool) -> Option<u8> {
        self.pair(bpm, calm && (asleep || bpm >= SLOW_BPM))
    }

    /// Notes a rejected window (or one vetoed for movement), which a calm
    /// pair can't span.
    pub fn reject(&mut self) {
        self.calm = f32::NAN;
    }

    /// The calm pair: this window and the one before, if both were calm
    /// (and not too fast, see [`MAX_CALM_BPM`]) and agree, make a reading
    /// (their mean).
    fn pair(&mut self, bpm: f32, calm: bool) -> Option<u8> {
        let calm = calm && bpm <= MAX_CALM_BPM;
        let previous = core::mem::replace(&mut self.calm, if calm { bpm } else { f32::NAN });
        // NaN (no calm window before) never agrees.
        (calm && (bpm - previous).abs() <= CALM_AGREEMENT_BPM)
            .then(|| libm::roundf((bpm + previous) / 2.0) as u8)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_filters_are_the_cookbooks() {
        let bits = |s: Biquad| [s.b0, s.b1, s.b2, s.a1, s.a2].map(f32::to_bits);
        let designed = [
            bits(Biquad::design(HIGHPASS_HZ, true)),
            bits(Biquad::design(LOWPASS_HZ, false)),
        ];
        assert_eq!(
            [bits(HIGHPASS), bits(LOWPASS)],
            designed,
            "{designed:#010x?}"
        );
    }
}
