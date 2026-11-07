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
//!    Its amplitude must be steady (motion makes bursts), and the
//!    autocorrelation peak must be high (a periodic signal correlates
//!    strongly with itself one period later; filtered noise doesn't).
//!    Finally, [`Consensus`] only reports once several windows, spread over
//!    more than one window's worth of time, agree.
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

/// Samples to discard after a reset, while the filters settle. The highpass
/// is primed with the first sample (see [`Ppg::push`]), so this only has to
/// cover the response to whatever the signal does in its first moments.
const SETTLE: usize = 27;

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
const HIGHPASS_HZ: f32 = 0.5;
const LOWPASS_HZ: f32 = 4.0;

/// How much the signal's amplitude may vary across the window: the ratio
/// of the loudest to the quietest quarter. A pulse's amplitude wanders
/// slowly (with breathing, say, by tens of percent); an arm movement
/// produces a burst many times larger.
const MAX_AMPLITUDE_RATIO: f32 = 3.0;

/// The normalised autocorrelation a window's beat period must reach (1.0
/// is perfectly periodic). Clean synthetic pulses with realistic heart-rate
/// variability reach 0.7–0.95 (fast ones the least, as a few percent of
/// variation in a short period is more samples' worth of smearing);
/// bandpass-filtered noise has peaks around 0.3–0.5, because the filter
/// itself makes noise look a bit periodic.
const MIN_CORRELATION: f32 = 0.6;

/// The first autocorrelation peak within this fraction of the highest one
/// is the period. A clean signal correlates almost as well at two or three
/// periods as at one, and with noise a later peak may come out higher;
/// taking it would halve or third the rate. In simulation, 0.85 still let
/// noisy fast pulses be read at half rate, while 0.6 never picked a peak
/// short of the period (there's no strong peak at half a period: the
/// signal is then roughly inverted).
const OCTAVE_TOLERANCE: f32 = 0.6;

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
    fn new(cutoff_hz: f32, highpass: bool) -> Self {
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
    /// The amplitude jumps around: most likely movement. `ratio` is the
    /// loudest quarter's amplitude over the quietest's (the limit is
    /// [`MAX_AMPLITUDE_RATIO`]).
    Unsteady { ratio: f32 },
    /// Not periodic enough within 40–200 BPM: noise, or a weak pulse.
    /// `quality` is the best correlation found (the limit is
    /// [`MIN_CORRELATION`]), and `bpm` its rate (NaN if there was no peak
    /// at all).
    Aperiodic { quality: f32, bpm: f32 },
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
}

impl Default for Ppg {
    fn default() -> Self {
        Self::new()
    }
}

impl Ppg {
    pub fn new() -> Self {
        Self {
            highpass: [Biquad::new(HIGHPASS_HZ, true); 2],
            lowpass: Biquad::new(LOWPASS_HZ, false),
            offset: None,
            window: [0.0; WINDOW],
            next: 0,
            seen: 0,
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
        let raw = raw as f32;
        let offset = *self.offset.get_or_insert(raw);
        let mut x = raw - offset;
        for section in &mut self.highpass {
            x = section.step(x);
        }
        x = self.lowpass.step(x);

        self.seen = self.seen.saturating_add(1);
        if self.seen <= SETTLE {
            return None;
        }
        self.window[self.next] = x;
        self.next = (self.next + 1) % WINDOW;

        let stored = self.seen - SETTLE;
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

        // Steadiness: compare the energy of the window's four quarters.
        // Quarters are 2 s, so each holds at least one beat at 40 BPM and
        // their energies are comparable.
        const QUARTER: usize = WINDOW / 4;
        let energy = |part: &[f32]| part.iter().map(|v| v * v).sum::<f32>();
        let mut quietest = f32::INFINITY;
        let mut loudest = 0.0f32;
        for quarter in x.as_chunks::<QUARTER>().0 {
            let e = energy(quarter);
            quietest = quietest.min(e);
            loudest = loudest.max(e);
        }
        let rms = libm::sqrtf(energy(&x) / WINDOW as f32);
        if rms < MIN_RMS {
            return Err(Rejection::Flat);
        }
        // Energies, so the amplitude ratio is squared.
        if loudest > quietest * MAX_AMPLITUDE_RATIO * MAX_AMPLITUDE_RATIO {
            let ratio = libm::sqrtf(loudest / quietest.max(f32::MIN_POSITIVE));
            return Err(Rejection::Unsteady { ratio });
        }

        // The normalised autocorrelation at each lag of interest: the
        // correlation coefficient between the window and itself shifted by
        // `lag`. Normalising each lag by the energy of exactly the samples
        // it uses (rather than the whole window's) keeps long lags, which
        // sum over fewer samples, comparable with short ones.
        let mut r = [0.0f32; MAX_LAG + 1];
        for (lag, r) in r.iter_mut().enumerate().skip(MIN_LAG - 1) {
            let (early, late) = (&x[..WINDOW - lag], &x[lag..]);
            let cross: f32 = early.iter().zip(late).map(|(a, b)| a * b).sum();
            let norm = libm::sqrtf(energy(early) * energy(late));
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
        let estimate = Estimate { period, quality };
        if quality < MIN_CORRELATION || !(MIN_BPM..=MAX_BPM).contains(&estimate.bpm()) {
            return Err(Rejection::Aperiodic {
                quality,
                bpm: estimate.bpm(),
            });
        }
        Ok(estimate)
    }
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

/// Collects window estimates and decides when they agree on a heart rate.
#[derive(Default)]
pub struct Consensus {
    /// The latest accepted estimates, oldest first.
    recent: [f32; AGREEING],
    len: usize,
}

impl Consensus {
    pub const fn new() -> Self {
        Self {
            recent: [0.0; AGREEING],
            len: 0,
        }
    }

    /// Adds an accepted estimate. Returns the heart rate (the median of the
    /// latest estimates) if they agree.
    ///
    /// Rejected windows aren't passed here and don't reset the consensus:
    /// a moment of motion between two good stretches doesn't make either
    /// stretch less trustworthy.
    pub fn push(&mut self, bpm: f32) -> Option<u8> {
        if self.len == AGREEING {
            self.recent.copy_within(1.., 0);
            self.len -= 1;
        }
        self.recent[self.len] = bpm;
        self.len += 1;
        if self.len < AGREEING {
            return None;
        }

        let mut sorted = self.recent;
        sorted.sort_unstable_by(f32::total_cmp);
        if sorted[AGREEING - 1] - sorted[0] > AGREEMENT_BPM {
            return None;
        }
        Some(libm::roundf(sorted[AGREEING / 2]) as u8)
    }
}
