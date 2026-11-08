//! Recordings: animated GIFs of what the screen shows, frame for frame, on
//! the simulator's clock (`record FILE.gif` … `stop`, or R in the window).
//!
//! # Where frames come from
//!
//! From the fake panel ([`crate::display`]), as screenshots do: what
//! `Canvas::flush` and `Canvas::transition` pushed, its idle mode (the dim
//! look), and the backlight. A transition's sweep, a marquee's scroll, and
//! the screen going dark all appear as they would on the watch, because
//! they're what the panel got, not a re-rendering of the canvas.
//!
//! The recorder looks every time the simulator's clock is about to move
//! (see [`clock_moving`], called from [`crate::time`]). Whatever changed
//! since it last looked changed at this instant, since the clock hasn't
//! moved in between: so the new frame starts now. Changes within one
//! instant (a whole flush, say) collapse into one frame, as they do on the
//! panel by the time anyone could look. Through a transition the clock
//! moves a millisecond at a time, so each of its steps is caught.
//!
//! The whole watch (`--watch`) also shakes while the motor buzzes, as the
//! window draws it ([`Case::shake`]). That moves without the panel
//! changing, and while the clock jumps a second at a time, so while the
//! motor plays, the recorder also looks at every [`GRID_CS`] of the
//! simulator's clock between the instant it leaves and the one it's moving
//! to (nothing else happens in between): each of the camera's marks
//! catches the shake as it is then.
//!
//! # Timing
//!
//! The clock is virtual, so the timing is exact and the same every run.
//! Two things stand between it and a GIF:
//!
//! - **Still stretches.** A script skips hours (`until 07:30`), and the
//!   screen may sit unchanged for all of them: a 7-hour GIF of a black
//!   screen helps nobody. So a frame stays up at most `--still` (2 s by
//!   default) in the GIF; the rest of its time is cut. Only stillness is
//!   cut: anything that changes faster than that plays at its own pace.
//! - **Rounding.** GIF delays are whole centiseconds, but browsers show a
//!   delay under 2 cs as 10 cs (a holdover from GIFs that asked for "as
//!   fast as you can"), which would play a transition's 10 ms steps at a
//!   tenth of their speed. So the GIF is a camera at 50 frames a second:
//!   every [`GRID_CS`] on its timeline, it shows the screen as it stands.
//!   A frame replaced before the next mark is never seen, and left out.
//!   Each frame's *start* is rounded to a mark, not its length, so the
//!   errors never add up: a frame is never more than 20 ms late, however
//!   long the recording.
//!
//! # Colours
//!
//! A GIF has at most 256 colours. The screen alone has far fewer (each
//! theme's palette roles, their dim-look versions, black), so its GIFs
//! are exact. The whole watch (`--watch`) adds the case's anti-aliased
//! edges, and the panel's colours at each backlight level: the 255 most
//! common colours are kept exact (the flat areas: the desk, case, glass,
//! and the screen's colours), and the rest, all thin edge pixels, become
//! the nearest of those. The 256th is transparency, for frame differencing.
//!
//! # Size
//!
//! Each frame after the first holds only the rectangle that changed, with
//! the pixels that didn't transparent: a clock's tick is a few digits, a
//! transition step a band of blocks.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;

use embassy_time::{Duration, Instant, TICK_HZ};

use crate::brightness::Level;
use crate::case::Case;
use crate::display::{PANEL, SIZE, rgb888};
use crate::motor::Pattern;

/// The GIF's frame grid, in centiseconds: the shortest delay browsers
/// honour (see the module's docs).
const GRID_CS: u64 = 2;

/// How long a still screen is held, unless the script says otherwise.
pub const STILL: Duration = Duration::from_secs(2);

/// The longest `--still` may be: a delay has to fit GIF's 16 bits of
/// centiseconds (655 s).
pub const MAX_STILL: Duration = Duration::from_secs(600);

/// An image pixel that's the same as in the frame before: transparent.
/// Colours are 0xRRGGBB, so this is none of them.
const UNCHANGED: u32 = u32::MAX;

/// What to record.
pub enum Look {
    /// The panel's 240×240 pixels, as `shot FILE.png` saves them; black
    /// with the backlight off.
    Screen,
    /// The whole watch, as `shot FILE.png --watch` and the window draw it,
    /// at this many pixels a canvas pixel.
    Watch { zoom: usize },
}

thread_local! {
    /// The recording in progress, if any. Thread-local, as the panel is:
    /// the simulator's clock, panel, and watch all live on one thread.
    static RECORDING: RefCell<Option<Recording>> = const { RefCell::new(None) };
    /// What the motor is playing, and since when, as the watch last set it
    /// (see [`motor`]), recording or not: a recording may start mid-buzz.
    static MOTOR: Cell<Option<(Pattern, Instant)>> = const { Cell::new(None) };
}

/// The watch's motor now plays `motor` (a pattern, and since when), or
/// nothing: for `--watch` recordings to shake the watch by.
pub fn motor(motor: Option<(Pattern, Instant)>) {
    MOTOR.set(motor);
}

/// Whether a recording is in progress.
pub fn active() -> bool {
    RECORDING.with_borrow(Option::is_some)
}

/// Starts recording to `path`, from what the screen shows now.
pub fn start(path: PathBuf, look: Look, still: Duration) -> Result<(), String> {
    if active() {
        return Err("record: already recording (`stop` first)".into());
    }
    if still < Duration::from_millis(GRID_CS * 10) || still > MAX_STILL {
        return Err(format!(
            "record: --still is {} ms to {} s",
            GRID_CS * 10,
            MAX_STILL.as_secs()
        ));
    }
    let case = match look {
        Look::Screen => None,
        Look::Watch { zoom } => Some(Case::new(zoom)),
    };
    let (width, height) = case.as_ref().map_or((SIZE, SIZE), |c| (c.width, c.height));
    let (seen, image) = look_at(case.as_ref(), width * height, Instant::now());
    RECORDING.set(Some(Recording {
        path,
        case,
        width,
        height,
        seen,
        current: image,
        since: Instant::now(),
        timeline: Timeline::new(still.as_ticks()),
        shown: None,
        frames: Vec::new(),
    }));
    Ok(())
}

/// Ends the recording, and writes it. Says what it wrote.
pub fn stop() -> Result<String, String> {
    let mut recording = RECORDING
        .take()
        .ok_or("stop: not recording (see `record`)")?;
    // Whatever changed since the clock last moved, and the last frame,
    // held until now: at least one mark, so it's seen at all (a GIF that
    // ends the moment something appears would only flash it).
    recording.look(Instant::now());
    let lasted = (Instant::now() - recording.since).as_ticks();
    let delay = recording.timeline.frame(lasted).max(GRID_CS);
    let image = std::mem::take(&mut recording.current);
    recording.show(image, delay);
    recording.write()
}

/// Called just before the clock moves to `to`: catches what the screen
/// shows as of this instant, if it changed, and the watch's shake on the
/// way to `to`.
pub fn clock_moving(to: Instant) {
    RECORDING.with_borrow_mut(|recording| {
        let Some(recording) = recording else {
            return;
        };
        let now = Instant::now();
        recording.look(now);
        let (Some(_), Some((pattern, since))) = (&recording.case, MOTOR.get()) else {
            return;
        };
        // A pattern that plays once has long wound down a second after it
        // ends: no need to look at every mark through the rest of an
        // overnight `until`.
        let until = if pattern.repeat_from().is_some() {
            to
        } else {
            let length: u64 = pattern.steps().iter().map(|&ms| u64::from(ms)).sum();
            to.min(since + Duration::from_millis(length) + Duration::from_secs(1))
        };
        // A whole number of ticks, rounded up: a frame a little longer than
        // the camera's mark, never one it misses.
        let grid = (TICK_HZ * GRID_CS).div_ceil(100);
        let mut at = Instant::from_ticks((now.as_ticks() / grid + 1) * grid);
        while at < until {
            // The panel can't change on the way: only the shake can.
            if shake(recording.case.as_ref(), at) != recording.seen.shake {
                recording.look(at);
            }
            at += Duration::from_ticks(grid);
        }
    });
}

/// What the screen showed when the recorder last looked: enough to tell
/// cheaply whether it changed, before drawing it.
#[derive(PartialEq)]
struct Seen {
    pixels: Vec<u16>,
    idle: bool,
    backlight: Level,
    /// How far the motor shook the watch (`--watch`; 0 otherwise).
    shake: f32,
}

struct Recording {
    path: PathBuf,
    /// For `--watch`: the case to draw round the panel.
    case: Option<Case>,
    width: usize,
    height: usize,
    seen: Seen,
    /// The frame on screen now (0xRRGGBB), since when, and where that is on
    /// the GIF's timeline.
    current: Vec<u32>,
    since: Instant,
    timeline: Timeline,
    /// The last frame put in the GIF, which the next is a difference from.
    shown: Option<Vec<u32>>,
    frames: Vec<Frame>,
}

/// A frame for the GIF: the rectangle that changed, `UNCHANGED` where a
/// pixel in it didn't, and how long it shows.
struct Frame {
    left: usize,
    top: usize,
    width: usize,
    height: usize,
    pixels: Vec<u32>,
    delay_cs: u64,
}

impl Recording {
    /// Looks at the screen as of `now`: if it changed, the frame on screen
    /// until now ends, and the new one starts.
    fn look(&mut self, now: Instant) {
        let changed = PANEL.with_borrow(|panel| {
            panel.pixels != self.seen.pixels
                || panel.idle != self.seen.idle
                || panel.backlight != self.seen.backlight
        }) || shake(self.case.as_ref(), now) != self.seen.shake;
        if !changed {
            return;
        }
        let (seen, image) = look_at(self.case.as_ref(), self.width * self.height, now);
        self.seen = seen;
        let lasted = (now - self.since).as_ticks();
        let delay = self.timeline.frame(lasted);
        let ended = std::mem::replace(&mut self.current, image);
        self.since = now;
        // Replaced before the camera's next mark: never seen.
        if delay > 0 {
            self.show(ended, delay);
        }
    }

    /// Adds `image` to the GIF, for `delay_cs`: only what differs from the
    /// last frame in it.
    fn show(&mut self, image: Vec<u32>, delay_cs: u64) {
        let Some(shown) = &self.shown else {
            self.frames.push(Frame {
                left: 0,
                top: 0,
                width: self.width,
                height: self.height,
                pixels: image.clone(),
                delay_cs,
            });
            self.shown = Some(image);
            return;
        };
        let width = self.width;
        let changed = |i: usize| image[i] != shown[i];
        let rows: Vec<usize> = (0..self.height)
            .filter(|&y| (0..width).any(|x| changed(y * width + x)))
            .collect();
        let (Some(&top), Some(&bottom)) = (rows.first(), rows.last()) else {
            // The same as the last frame after all (it changed, then
            // changed back, between marks): that one just shows longer.
            return self.extend(delay_cs);
        };
        let columns = |y: usize| (0..width).filter(move |&x| changed(y * width + x));
        let left = rows
            .iter()
            .filter_map(|&y| columns(y).next())
            .min()
            .expect("each of these rows has a change");
        let right = rows
            .iter()
            .filter_map(|&y| columns(y).next_back())
            .max()
            .expect("each of these rows has a change");
        let mut pixels = Vec::with_capacity((right - left + 1) * (bottom - top + 1));
        for y in top..=bottom {
            for x in left..=right {
                let i = y * width + x;
                pixels.push(if changed(i) { image[i] } else { UNCHANGED });
            }
        }
        self.frames.push(Frame {
            left,
            top,
            width: right - left + 1,
            height: bottom - top + 1,
            pixels,
            delay_cs,
        });
        self.shown = Some(image);
    }

    /// The last frame shows `delay_cs` longer; or, if its delay would
    /// overflow GIF's 16 bits, a frame that changes nothing does.
    fn extend(&mut self, delay_cs: u64) {
        let last = self.frames.last_mut().expect("the first frame is whole");
        if last.delay_cs + delay_cs <= u64::from(u16::MAX) {
            last.delay_cs += delay_cs;
        } else {
            self.frames.push(Frame {
                left: 0,
                top: 0,
                width: 1,
                height: 1,
                pixels: vec![UNCHANGED],
                delay_cs,
            });
        }
    }

    fn write(self) -> Result<String, String> {
        let fail = |e: &dyn std::fmt::Display| format!("encoding {}: {e}", self.path.display());
        let palette = Palette::new(&self.frames);
        let dimension = |n: usize| u16::try_from(n).expect("the watch at --zoom 12 fits");
        // In memory, then saved whole: the file is the platform's business
        // (the browser downloads it).
        let mut gif = Vec::new();
        let mut encoder = gif::Encoder::new(
            &mut gif,
            dimension(self.width),
            dimension(self.height),
            &palette.bytes(),
        )
        .map_err(|e| fail(&e))?;
        encoder
            .set_repeat(gif::Repeat::Infinite)
            .map_err(|e| fail(&e))?;
        for frame in &self.frames {
            let buffer: Vec<u8> = frame.pixels.iter().map(|&c| palette.index(c)).collect();
            encoder
                .write_frame(&gif::Frame {
                    delay: u16::try_from(frame.delay_cs).expect("held under 655 s"),
                    // Each frame draws over the last: its transparent
                    // pixels leave the last one's showing.
                    dispose: gif::DisposalMethod::Keep,
                    transparent: Some(palette.transparent),
                    left: dimension(frame.left),
                    top: dimension(frame.top),
                    width: dimension(frame.width),
                    height: dimension(frame.height),
                    buffer: buffer.into(),
                    ..gif::Frame::default()
                })
                .map_err(|e| fail(&e))?;
        }
        encoder.into_inner().map_err(|e| fail(&e))?;
        crate::platform::save(&self.path, &gif)?;
        let seconds: u64 = self.frames.iter().map(|f| f.delay_cs).sum();
        let exact = if palette.exact { "" } else { ", quantised" };
        Ok(format!(
            "recorded {}: {} frames, {}.{:02} s, {} colours{exact}",
            self.path.display(),
            self.frames.len(),
            seconds / 100,
            seconds % 100,
            palette.colours.len(),
        ))
    }
}

/// How far the motor shakes `case` at `now`: 0 without one (the panel
/// alone doesn't shake).
fn shake(case: Option<&Case>, now: Instant) -> f32 {
    case.map_or(0.0, |case| {
        case.shake(crate::watch::motor_pulse(MOTOR.get(), now), now)
    })
}

/// What the screen shows as of `now`: the state to compare, and the image
/// (0xRRGGBB, `pixels` long).
fn look_at(case: Option<&Case>, pixels: usize, now: Instant) -> (Seen, Vec<u32>) {
    let seen = PANEL.with_borrow(|panel| Seen {
        pixels: panel.pixels.clone(),
        idle: panel.idle,
        backlight: panel.backlight,
        shake: shake(case, now),
    });
    let image = match case {
        // The whole watch, its panel lit as the window lights it (reading
        // the panel itself), and shaken as the window shakes it. A shaken
        // frame changes the edges of everything on the watch, but only
        // the edges: its flat areas stay the same colour, and so
        // transparent in the GIF.
        Some(case) => {
            let mut image = vec![0; pixels];
            case.draw(&mut image, crate::case::light(seen.backlight), seen.shake);
            image
        }
        // Off, the panel still holds its last frame, but nobody sees it.
        // Otherwise, as screenshots are: the panel's colours, whatever the
        // backlight's level.
        None if seen.backlight == Level::OFF => vec![0; pixels],
        None => seen
            .pixels
            .iter()
            .map(|&pixel| {
                let [r, g, b] = rgb888(pixel, seen.idle);
                u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b)
            })
            .collect(),
    };
    (seen, image)
}

// ==============================================================================
// Timing
// ==============================================================================

/// The GIF's timeline: the simulator's, with still stretches cut short,
/// seen by a camera that shows a frame every [`GRID_CS`].
struct Timeline {
    /// Where the frame on screen started, in ticks on the GIF's timeline.
    at: u64,
    /// The longest a frame is held, in ticks.
    still: u64,
}

impl Timeline {
    fn new(still: u64) -> Self {
        Self { at: 0, still }
    }

    /// The frame on screen has lasted `lasted` ticks, and the next starts:
    /// how long the GIF shows it, in centiseconds (0: not at all, as the
    /// next replaces it before the camera's next mark).
    fn frame(&mut self, lasted: u64) -> u64 {
        let start = mark(self.at);
        self.at += lasted.min(self.still);
        (mark(self.at) - start) * GRID_CS
    }
}

/// The first of the camera's marks at or after `ticks`: when a frame that
/// starts then is first seen.
fn mark(ticks: u64) -> u64 {
    // Ticks to centiseconds is ×100 / TICK_HZ; to marks, a further
    // ÷ GRID_CS. Exact in integers: hours of ticks ×100 fit in a u64.
    (ticks * 100).div_ceil(TICK_HZ * GRID_CS)
}

// ==============================================================================
// Colours
// ==============================================================================

/// The GIF's one palette, for all its frames.
struct Palette {
    /// 0xRRGGBB, the most common first.
    colours: Vec<u32>,
    /// Each colour in the frames, by its index in `colours`.
    index: HashMap<u32, u8>,
    /// The index after the colours: transparency.
    transparent: u8,
    /// Whether every colour is exact.
    exact: bool,
}

impl Palette {
    /// The 255 commonest colours, exactly; any others as the nearest of
    /// those (see the module's docs).
    fn new(frames: &[Frame]) -> Self {
        let mut counts: HashMap<u32, u64> = HashMap::new();
        for frame in frames {
            for &colour in &frame.pixels {
                if colour != UNCHANGED {
                    *counts.entry(colour).or_default() += 1;
                }
            }
        }
        // Commonest first; ties by colour, so the palette is the same
        // every run.
        let mut ranked: Vec<(u32, u64)> = counts.into_iter().collect();
        ranked.sort_unstable_by_key(|&(colour, count)| (std::cmp::Reverse(count), colour));
        let exact = ranked.len() <= 255;
        let colours: Vec<u32> = ranked.iter().take(255).map(|&(c, _)| c).collect();
        let mut index: HashMap<u32, u8> = colours
            .iter()
            .enumerate()
            .map(|(i, &c)| (c, u8::try_from(i).expect("255 at most")))
            .collect();
        for &(colour, _) in ranked.iter().skip(255) {
            let nearest = (0..colours.len())
                .min_by_key(|&i| distance(colour, colours[i]))
                .expect("255 colours come first");
            index.insert(colour, u8::try_from(nearest).expect("255 at most"));
        }
        Self {
            transparent: u8::try_from(colours.len()).expect("255 at most"),
            colours,
            index,
            exact,
        }
    }

    fn index(&self, colour: u32) -> u8 {
        if colour == UNCHANGED {
            return self.transparent;
        }
        self.index[&colour]
    }

    /// As GIF wants it: RGB, and black for the transparent index.
    fn bytes(&self) -> Vec<u8> {
        self.colours
            .iter()
            .chain([&0])
            .flat_map(|&c| [(c >> 16) as u8, (c >> 8) as u8, c as u8])
            .collect()
    }
}

/// How different two colours look, roughly: squared distance in RGB,
/// green counting most and blue least, as the eye weighs them.
fn distance(a: u32, b: u32) -> u32 {
    let channel = |shift: u32| ((a >> shift) & 0xFF).abs_diff((b >> shift) & 0xFF).pow(2);
    3 * channel(16) + 4 * channel(8) + 2 * channel(0)
}

/// A name for the window's recordings: when it started (on the computer's
/// clock, as the watch's may be anywhere), so each is new.
#[cfg(feature = "window")]
pub fn timestamped() -> String {
    let now = chrono::Local::now().format("%Y%m%d-%H%M%S");
    format!("ur-sim-{now}.gif")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(ms: u64) -> u64 {
        Duration::from_millis(ms).as_ticks()
    }

    fn delays(still_ms: u64, lasted_ms: &[u64]) -> Vec<u64> {
        let mut timeline = Timeline::new(ms(still_ms));
        lasted_ms.iter().map(|&l| timeline.frame(ms(l))).collect()
    }

    #[test]
    fn whole_seconds_are_exact() {
        assert_eq!(delays(2000, &[1000, 1000, 1000]), [100, 100, 100]);
    }

    /// A transition's 10 ms steps: about every other one is replaced
    /// before the camera's next mark, and the rest show for 20 ms, so the
    /// sweep takes as long as on the watch. (The first is on a mark, so
    /// it's seen; and 10 ms is 328 ticks, a hair over half a mark, so the
    /// second just reaches the next.)
    #[test]
    fn steps_faster_than_the_grid_are_left_out() {
        assert_eq!(delays(2000, &[10; 6]), [2, 2, 0, 2, 0, 2]);
    }

    /// Thirds of a second, or the 1 ms steps a transition's clock moves
    /// in, aren't whole marks; rounding each start, not each length, keeps
    /// the total within a mark of the truth however many frames there are.
    #[test]
    fn rounding_doesnt_drift() {
        for (lasted, count) in [(ms(333), 300), (ms(1), 10_000), (ms(7), 999)] {
            let mut timeline = Timeline::new(ms(2000));
            let delays: Vec<u64> = (0..count).map(|_| timeline.frame(lasted)).collect();
            assert!(delays.iter().all(|d| d % GRID_CS == 0), "{delays:?}");
            let total = delays.iter().sum::<u64>() as f64;
            let exact = (lasted * count) as f64 * 100.0 / TICK_HZ as f64;
            assert!((total - exact).abs() < GRID_CS as f64, "{total} {exact}");
        }
        let thirds = delays(2000, &[333; 300]);
        assert!(thirds.iter().all(|&d| d == 32 || d == 34), "{thirds:?}");
    }

    #[test]
    fn still_stretches_are_cut_short() {
        // Seven hours of a dark screen, then a second of clock.
        assert_eq!(delays(2000, &[7 * 3_600_000, 1000]), [200, 100]);
        assert_eq!(delays(500, &[7 * 3_600_000, 1000]), [50, 50]);
    }

    #[test]
    fn marks_round_up() {
        assert_eq!(mark(0), 0);
        assert_eq!(mark(1), 1);
        // 20 ms is 655.36 ticks: 655 is just before the first mark.
        assert_eq!(mark(655), 1);
        assert_eq!(mark(656), 2);
    }

    fn frame(pixels: Vec<u32>) -> Frame {
        Frame {
            left: 0,
            top: 0,
            width: pixels.len(),
            height: 1,
            pixels,
            delay_cs: 2,
        }
    }

    #[test]
    fn few_colours_are_exact() {
        let palette = Palette::new(&[frame(vec![0xFF0000, UNCHANGED, 0x00FF00, 0xFF0000])]);
        assert!(palette.exact);
        assert_eq!(palette.colours, [0xFF0000, 0x00FF00]);
        assert_eq!(palette.index(UNCHANGED), 2);
        assert_eq!(palette.bytes().len(), 9);
    }

    /// Past 255, the rarest colours become the nearest common one.
    #[test]
    fn many_colours_keep_the_commonest() {
        // 255 greys, each twice, and a near-white once.
        let mut pixels: Vec<u32> = (0..255).flat_map(|v| [v * 0x010101; 2]).collect();
        pixels.push(0xFEFFFF);
        let palette = Palette::new(&[frame(pixels)]);
        assert!(!palette.exact);
        assert_eq!(palette.colours.len(), 255);
        assert_eq!(palette.index(0xFEFFFF), palette.index(0xFEFEFE));
        assert_eq!(palette.transparent, 255);
    }
}
