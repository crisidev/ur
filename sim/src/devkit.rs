//! Checking the simulator against a dev kit, frame for frame:
//! `ur-sim --devkit ELF SCRIPT`.
//!
//! The dev kit runs a build with the `compare` feature, which exports what
//! its UI loop does and renders with (see `src/compare.rs`). The simulator
//! follows it: it starts its UI from the dev kit's boot (the same instant,
//! the same settings from flash), replays every event the dev kit's UI got
//! (ticks and inputs), at the very instants it got
//! them, and for each `compare` in the script renders the frame the dev
//! kit last finished, with the readings it had (time, battery, sensors,
//! today's activity). Same code, same state, same readings: the frames
//! must match, pixel for pixel, and any that don't are reported.
//!
//! The script's inputs go to the dev kit the way `scripts/screenshot.py`
//! injects them, and only those it can inject. The simulator doesn't
//! decide what they do: it replays what the dev kit made of them (a
//! finger's touch, as its touch task recognised it).
//!
//! Each `compare` writes `NAME.png`: the simulator's frame, the dev kit's,
//! and where they differ (magenta), side by side.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration as StdDuration, Instant as StdInstant};

use embassy_time::Instant;

use crate::alarm;
use crate::battery::Battery;
use crate::brightness;
use crate::buttons::Press;
use crate::canvas::{Canvas, HEIGHT, WIDTH};
use crate::clock::{self, DateTime};
use crate::compare::{self, Event, Export, Head, Music, Pass, kind, phone};
use crate::display::{Display, PANEL, SIZE, rgb888};
use crate::flash::{Flash, REGION_BYTES};
use crate::gb::{self, Incoming, LineBuffer};
use crate::gesture::{Gesture, Touch};
use crate::script;
use crate::shot;
use crate::ui::{App, PhoneEvent, Status, ThemeChoice, Unpaired};
use crate::ui_loop::{Stored, UiLoop};
use crate::{InputEvent, activity, debug, storage, time};

const TARGET: [&str; 2] = ["--target", "nRF52832_xxAA"];
/// A music info's artist or track, as `PhoneEvent::MusicInfo` holds it.
type MusicText = heapless::String<{ compare::MUSIC_TEXT }>;
/// How long the dev kit may take to take an input (its input task looks
/// every 100 ms) and log it.
const INPUT_TIMEOUT: StdDuration = StdDuration::from_secs(3);
/// While waiting, how often to read the dev kit's events: well within the
/// [`compare::EVENTS`] it keeps (a tick a second).
const POLL: StdDuration = StdDuration::from_secs(2);

/// What `--devkit` (and `--fuzz`) set up besides the dev kit: this
/// computer as its phone, at `peer` (its Bluetooth address), and the
/// storage it starts from.
pub struct Setup {
    pub peer: Option<String>,
    pub storage: Option<PathBuf>,
}

pub fn run(elf: &Path, scripts: &[PathBuf], out: &Path, setup: &Setup) -> Result<(), String> {
    let devkit = DevKit::open(elf)?;
    let storage = match &setup.storage {
        Some(path) => {
            Some(std::fs::read(path).map_err(|e| format!("reading {}: {e}", path.display()))?)
        }
        None => None,
    };
    let mut rig = Rig::new(setup.peer.clone());
    let mut follower = Follower::start(&devkit, &mut rig, storage.as_deref(), out)?;
    let mut failed = Vec::new();
    for path in scripts {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("reading {}: {e}", path.display()))?;
        for (n, line) in text.lines().enumerate() {
            let at = |e: String| format!("{}:{}: {e}", path.display(), n + 1);
            for Finding { name, what } in follower.execute(line, out).map_err(at)? {
                eprintln!("MISMATCH: {name}: {what}");
                failed.push(name);
            }
        }
    }
    if failed.is_empty() {
        eprintln!("everything matched");
        Ok(())
    } else {
        Err(format!("differ: {}", failed.join(", ")))
    }
}

/// Where the simulator and the dev kit disagreed.
pub struct Finding {
    pub name: String,
    pub what: String,
}

// ==============================================================================
// The dev kit, over the probe
// ==============================================================================

struct DevKit {
    input: u32,
    /// `UR_CANVAS`, which points to the canvas once the firmware's set it
    /// up, and where it points.
    canvas_pointer: u32,
    canvas: std::cell::Cell<Option<u32>>,
    export: u32,
    /// The last theme's palette, by its address: they don't change.
    palette: std::cell::RefCell<Option<(u32, Palette)>>,
    /// What the storage writes and resets have taken so far.
    spent: std::cell::Cell<Spent>,
    /// With a coverage build (`coverage-build.sh`): where its functions'
    /// flags are, and how many.
    flags: Option<(u32, usize)>,
    /// The ELF, and its RTT control block, for [`Self::boot_log`].
    elf: std::path::PathBuf,
    rtt: Option<u32>,
    /// The stack's lowest address (`_stack_end`), for [`Self::stack_left`].
    stack_end: Option<u32>,
}

/// Wall time the dev kit spent other than on a script's steps: writing
/// its storage, and resetting (to the app's first export, past the
/// bootloader's 5 s).
#[derive(Clone, Copy, Default, Debug)]
pub struct Spent {
    pub writes: u32,
    pub writing: StdDuration,
    pub resets: u32,
    pub resetting: StdDuration,
}

impl DevKit {
    fn open(elf: &Path) -> Result<Self, String> {
        let nm = Command::new("llvm-nm")
            .arg(elf)
            .output()
            .map_err(|e| format!("running llvm-nm: {e}"))?;
        let symbols = String::from_utf8_lossy(&nm.stdout);
        let find = |name: &str| {
            symbols
                .lines()
                .find_map(|line| {
                    let mut parts = line.split_whitespace();
                    let (address, _, symbol) = (parts.next()?, parts.next()?, parts.next()?);
                    (symbol == name).then(|| u32::from_str_radix(address, 16).ok())?
                })
                .ok_or_else(|| {
                    format!(
                        "no {name} in {}{}",
                        elf.display(),
                        if name == "UR_COMPARE" || name == "UR_INPUT" {
                            " (build it with `--features compare`)"
                        } else {
                            ""
                        }
                    )
                })
        };
        Ok(Self {
            input: find("UR_INPUT")?,
            canvas_pointer: find("UR_CANVAS")?,
            canvas: std::cell::Cell::new(None),
            export: find("UR_COMPARE")?,
            palette: std::cell::RefCell::new(None),
            spent: std::cell::Cell::default(),
            elf: elf.to_owned(),
            rtt: find("_SEGGER_RTT").ok(),
            stack_end: find("_stack_end").ok(),
            flags: match (find("__sancov_bools_start"), find("__sancov_bools_end")) {
                (Ok(start), Ok(end)) => Some((start, (end - start) as usize)),
                _ => None,
            },
        })
    }

    /// What the dev kit has logged since it booted, as far as it's
    /// explained a restart: why it reset, and what the previous run's
    /// breadcrumb says (a panic's message, or where the watchdog found it
    /// stuck; see `field_log::report_boot`). The firmware logs those first,
    /// over RTT, and with no host reading, defmt keeps what's there and
    /// drops what doesn't fit, so they're still there to drain a while
    /// after. (Draining is all it costs: nothing else here reads RTT.)
    fn boot_log(&self) -> String {
        let Some(rtt) = self.rtt else {
            return "(no _SEGGER_RTT in the ELF)".into();
        };
        // After a restart, the bootloader takes ~10 s (its logo, then the
        // image's hash) before the app sets up RTT: wait for the control
        // block's "SEGGER RTT", as scripts/run-esprobe.sh does.
        let started = StdInstant::now();
        while started.elapsed() < StdDuration::from_secs(30) {
            if self
                .read_words(rtt, 1)
                .is_ok_and(|words| words[0] == 0x4747_4553)
            {
                break;
            }
            std::thread::sleep(StdDuration::from_secs(1));
        }
        let pipeline = format!(
            "timeout 10 esprobe {} rtt --control-block {rtt:#x} --idle-ms 1500 \
             | defmt-print -e '{}'",
            TARGET.join(" "),
            self.elf.display()
        );
        let output = Command::new("sh").args(["-c", &pipeline]).output();
        let Ok(output) = output else {
            return "(couldn't run esprobe rtt)".into();
        };
        let text = String::from_utf8_lossy(&output.stdout);
        let lines: Vec<&str> = text
            .lines()
            .filter(|line| {
                [
                    "reset reason",
                    "previous run",
                    "panic",
                    "watchdog",
                    "fault",
                    "backtrace",
                    "0x0",
                ]
                .iter()
                .any(|word| line.contains(word))
            })
            .collect();
        if lines.is_empty() {
            format!(
                "(nothing about a restart in its log: {} lines)",
                text.lines().count()
            )
        } else {
            lines.join("\n")
        }
    }

    /// How close the stack has come to its end since boot (or since the
    /// firmware last repainted it; see `power::stack_peak`): the bytes
    /// between its guard and the deepest word it has scrubbed of
    /// cortex-m-rt's paint. Only the bottom [`STACK_LOOK`] bytes are read
    /// (`fast-dump`, without halting the core), so more than that reads as
    /// that; `None` without `_stack_end` in the ELF.
    ///
    /// An overflow runs into `field_log::guard_stack`'s 32 bytes, which
    /// fault, and the watch restarts: a step that leaves little room is
    /// worth knowing of before one goes a little deeper (as main's compare
    /// build did on Settings, THEME, at 9c65743).
    fn stack_left(&self) -> Result<Option<u32>, String> {
        let Some(end) = self.stack_end else {
            return Ok(None);
        };
        // Above the guard, which faults on any access.
        let bottom = end + 32;
        let words = self.read_words(bottom, STACK_LOOK as usize / 4)?;
        let untouched = words
            .iter()
            .position(|&word| word != STACK_PAINT)
            .unwrap_or(words.len());
        Ok(Some(untouched as u32 * 4))
    }

    /// Which of a coverage build's functions have run since it booted
    /// (`None` without one): a flag is 1 once its function has run, and
    /// the stack's paint (0xCC) until then (see `src/coverage.rs`).
    fn coverage(&self) -> Result<Option<Vec<bool>>, String> {
        let Some((start, count)) = self.flags else {
            return Ok(None);
        };
        Ok(Some(
            self.read_bytes(start, count)?
                .into_iter()
                .map(|flag| flag == 1)
                .collect(),
        ))
    }

    /// Runs an esprobe command, a few times over if the probe's bridge
    /// times out now and then (it does), or its USB serial device drops
    /// out for a second (it has). Only for what's as good done twice:
    /// reads, and writes of what's there already after the first (not the
    /// dev kit's input word, which it clears as it takes it: see
    /// [`Self::inject`]). Returns what it printed.
    fn esprobe(args: &[&str]) -> Result<Vec<u8>, String> {
        Self::esprobe_tries(args, 5)
    }

    fn esprobe_tries(args: &[&str], most: u32) -> Result<Vec<u8>, String> {
        let mut tries = 0;
        loop {
            let started = StdInstant::now();
            let output = Command::new("esprobe")
                .args(TARGET)
                .args(args)
                .output()
                .map_err(|e| format!("running esprobe: {e}"))?;
            if started.elapsed() > StdDuration::from_secs(5) {
                eprintln!("  (the probe took {:.0?})", started.elapsed());
            }
            if output.status.success() {
                return Ok(output.stdout);
            }
            tries += 1;
            if tries == most {
                return Err(format!(
                    "esprobe {}: {}",
                    args.join(" "),
                    String::from_utf8_lossy(&output.stderr).trim()
                ));
            }
            eprintln!(
                "  (esprobe {} failed; again: {})",
                args.first().unwrap_or(&""),
                String::from_utf8_lossy(&output.stderr).trim()
            );
            // 0.2, 0.4, 0.8, 1.6 s: a USB device takes a moment to return.
            std::thread::sleep(StdDuration::from_millis(200 << (tries - 1)));
        }
    }

    /// Reads `count` words of the dev kit's memory, from `address` on.
    ///
    /// With `esprobe fast-dump`, over the bridge's bulk path, which never
    /// halts the core. `esprobe core read` is a probe-rs session, which
    /// halts it for ~20 ms as it attaches and again as it detaches; this
    /// reads several times a second, and a halt that lands in a flash
    /// timeslot overruns it, which makes MPSL reset the dev kit (see
    /// CONTRIBUTING.md). The core runs on as this reads, so what's read
    /// isn't a snapshot: see [`DevKit::frame`] and [`DevKit::music`].
    fn read_words(&self, address: u32, count: usize) -> Result<Vec<u32>, String> {
        // To a file: fast-dump prints a summary on stdout. (And it reads
        // whole words only, rounding a size down to them.)
        let dump = std::env::temp_dir().join(format!("ur-sim-devkit-{}.bin", std::process::id()));
        Self::esprobe(&[
            "fast-dump",
            "--address",
            &address.to_string(),
            "--size",
            &(4 * count).to_string(),
            dump.to_str()
                .ok_or("the temporary directory's path isn't UTF-8")?,
        ])?;
        let bytes = std::fs::read(&dump).map_err(|e| format!("reading {}: {e}", dump.display()))?;
        let _ = std::fs::remove_file(&dump);
        if bytes.len() != 4 * count {
            return Err(format!(
                "short read at {address:#x}: {} bytes of {}",
                bytes.len(),
                4 * count
            ));
        }
        Ok(bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|&word| u32::from_le_bytes(word))
            .collect())
    }

    fn read_bytes(&self, address: u32, len: usize) -> Result<Vec<u8>, String> {
        let words = self.read_words(address, len.div_ceil(4))?;
        let mut bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
        bytes.truncate(len);
        Ok(bytes)
    }

    fn write_word(&self, address: u32, value: u32) -> Result<(), String> {
        Self::esprobe(&[
            "core",
            "write",
            &format!("{address:#x}"),
            &format!("{value:#x}"),
        ])
        .map(drop)
    }

    /// The storage region (`storage::RANGE`), as on the chip.
    fn read_storage(&self) -> Result<Vec<u8>, String> {
        self.read_bytes(storage::RANGE.start, REGION_BYTES)
    }

    /// Writes `image` over the storage region, unless it's there already,
    /// and checks it took. The dev kit needs a reset after (`esprobe
    /// flash` resets it, but not as `reset` does).
    ///
    /// As an ELF of that region alone, through `esprobe flash`, which
    /// erases and writes only its pages: 0.7 s. (`esprobe program`, with a
    /// raw image, first backs the whole 512 KB flash up: 15 s, for every
    /// run, and every try while shrinking.)
    fn write_storage(&self, image: &[u8], scratch: &Path) -> Result<(), String> {
        if image.len() != REGION_BYTES {
            return Err(format!(
                "a storage image is {REGION_BYTES} bytes, not {}",
                image.len()
            ));
        }
        // A run that saved nothing (or a try while shrinking that didn't)
        // left it as it was.
        if self.read_storage()? == image {
            return Ok(());
        }
        let started = StdInstant::now();
        std::fs::create_dir_all(scratch)
            .map_err(|e| format!("creating {}: {e}", scratch.display()))?;
        let elf = scratch.join("storage.elf");
        std::fs::write(&elf, storage_elf(image))
            .map_err(|e| format!("writing {}: {e}", elf.display()))?;
        let path = elf.to_str().ok_or("a path isn't UTF-8")?;
        Self::esprobe(&["flash", path])?;
        if self.read_storage()? != image {
            return Err("the dev kit's storage isn't what was written".into());
        }
        let mut spent = self.spent.get();
        spent.writes += 1;
        spent.writing += started.elapsed();
        self.spent.set(spent);
        Ok(())
    }

    /// Everything exported but the events.
    fn head(&self) -> Result<Head, String> {
        let bytes = self.read_bytes(self.export, size_of::<Head>())?;
        // SAFETY: `Head` is `repr(C)` integers and arrays of them, laid out
        // alike on both targets (see the test below), for which any bytes
        // are valid.
        Ok(unsafe { bytes.as_ptr().cast::<Head>().read_unaligned() })
    }

    /// Events `from` up to `to` (counted from boot), which the ring must
    /// still hold.
    fn events(&self, from: u32, to: u32) -> Result<Vec<Event>, String> {
        let ring = self.export + std::mem::offset_of!(Export, events) as u32;
        let size = size_of::<Event>();
        let mut events = Vec::new();
        let mut next = from;
        // In at most two reads: up to the ring's end, then from its start.
        while next != to {
            let slot = next as usize % compare::EVENTS;
            let count = (to.wrapping_sub(next) as usize).min(compare::EVENTS - slot);
            let bytes = self.read_bytes(ring + (slot * size) as u32, count * size)?;
            for chunk in bytes.chunks_exact(size) {
                // SAFETY: As for `head`.
                events.push(unsafe { chunk.as_ptr().cast::<Event>().read_unaligned() });
            }
            next = next.wrapping_add(count as u32);
        }
        Ok(events)
    }

    /// The text of music info number `number` (see `compare::Music`).
    fn music(&self, number: u16) -> Result<(MusicText, MusicText), String> {
        let address = self.export + std::mem::offset_of!(Export, music) as u32;
        let bytes = self.read_bytes(address, size_of::<Music>())?;
        // SAFETY: As for `head`.
        let music = unsafe { bytes.as_ptr().cast::<Music>().read_unaligned() };
        // The number again (the first word's low half), after the text: the
        // dev kit runs on as this reads, and if another music info came in
        // meanwhile, the text may be half of each. Its number changes
        // first (see `compare::publish`), so this sees that.
        let again = self.read_words(address, 1)?[0] as u16;
        if music.number != number || again != number {
            return Err(
                "the dev kit's music changed again before this read what it \
                        was; can't tell what it showed"
                    .into(),
            );
        }
        let text = |bytes: &[u8], len: u8| -> Result<MusicText, String> {
            let text = bytes
                .get(..usize::from(len))
                .and_then(|bytes| std::str::from_utf8(bytes).ok())
                .ok_or("the dev kit's music text is garbled")?;
            MusicText::try_from(text).map_err(|_| "the dev kit's music text is too long".into())
        };
        Ok((
            text(&music.artist, music.artist_len)?,
            text(&music.track, music.track_len)?,
        ))
    }

    /// A theme's colours, as the panel gets them.
    fn palette(&self, theme: u32) -> Result<Palette, String> {
        if let Some((at, palette)) = &*self.palette.borrow()
            && *at == theme
        {
            return Ok(palette.clone());
        }
        // `Palette` is `repr(C)`: the name (`&str`: pointer, length), then
        // a big-endian RGB565 colour per role, then the half-toned roles
        // (a bit each). (The dev kit's firmware has to be this tree's, for
        // the count to match.)
        let count = crate::palette::Color::COUNT;
        let bytes = self.read_bytes(theme + 8, 2 * count + 2)?;
        let palette = Palette {
            colors: bytes[..2 * count]
                .chunks(2)
                .map(|c| u16::from_be_bytes([c[0], c[1]]))
                .collect(),
            halftone: u16::from_le_bytes([bytes[2 * count], bytes[2 * count + 1]]),
        };
        *self.palette.borrow_mut() = Some((theme, palette.clone()));
        Ok(palette)
    }

    /// Restarts the dev kit, so its UI starts afresh, as the simulator's
    /// does. The export is cleared first, so its boot isn't mistaken for
    /// the last one's.
    fn reset(&self) -> Result<Head, String> {
        let begun = StdInstant::now();
        Self::esprobe(&["core", "reset-halt"])?;
        let magic = self.export + std::mem::offset_of!(Head, magic) as u32;
        self.write_word(magic, 0)?;
        Self::esprobe(&["core", "run"])?;
        let started = StdInstant::now();
        loop {
            let head = self.head()?;
            if head.magic == compare::MAGIC {
                let mut spent = self.spent.get();
                spent.resets += 1;
                spent.resetting += begun.elapsed();
                self.spent.set(spent);
                return Ok(head);
            }
            if started.elapsed() > StdDuration::from_secs(15) {
                return Err("the dev kit didn't boot (or isn't a `compare` build)".into());
            }
            std::thread::sleep(StdDuration::from_millis(300));
        }
    }

    /// Injects an input, as `scripts/screenshot.py` does (see
    /// `src/debug.rs`), once the last one's been taken.
    fn inject(&self, kind: u8, x: u8, y: u8, extra: u8) -> Result<(), String> {
        self.wait_for_input_taken()?;
        let word = u32::from_le_bytes([kind, x, y, extra]);
        let logged = self.head()?.written;
        let write = [
            "core",
            "write",
            &format!("{:#x}", self.input),
            &format!("{word:#x}"),
        ];
        for _ in 0..5 {
            // Once at a time: the bridge can report a failure after the
            // core has the word, and the dev kit takes it (within its
            // 100 ms poll) and clears it, so a blind retry is a second
            // input. (One went in twice, a second apart: seed 4010.)
            if Self::esprobe_tries(&write, 1).is_ok() {
                return self.wait_for_input_taken();
            }
            // Long enough for the dev kit to take it and log it, even
            // behind a whole-screen redraw (~0.5 s).
            std::thread::sleep(StdDuration::from_millis(600));
            // Still there (not taken yet), or taken (and logged: every input
            // is, bar a finger's press or move, which is as good twice).
            if self.read_words(self.input, 1)?[0] != 0 || self.head()?.written != logged {
                eprintln!("  (esprobe failed writing the input, but it got there)");
                return self.wait_for_input_taken();
            }
            eprintln!("  (esprobe failed writing the input; again)");
        }
        Err("esprobe: writing the dev kit's input failed five times".into())
    }

    fn wait_for_input_taken(&self) -> Result<(), String> {
        let started = StdInstant::now();
        while self.read_words(self.input, 1)?[0] != 0 {
            if started.elapsed() > INPUT_TIMEOUT {
                return Err("the dev kit didn't take the input".into());
            }
            std::thread::sleep(StdDuration::from_millis(50));
        }
        Ok(())
    }

    /// The first frame the dev kit finishes after every event it has
    /// logged so far (an event's frame comes a moment after it): what it
    /// was drawn with, its colour roles, and the theme's palette (RGB565,
    /// as the panel gets it). `None` if it draws none for a while, as with
    /// the screen off.
    fn frame(&self) -> Result<Option<Frame>, String> {
        let deadline = StdInstant::now() + StdDuration::from_secs(6);
        while StdInstant::now() < deadline {
            let before = self.head()?;
            let newest = match before.written.checked_sub(1) {
                Some(last) => self
                    .events(last, before.written)?
                    .first()
                    .map(|event| event.at.max(event.ticked)),
                None => None,
            };
            // A pass can read the same instant as the event before it: the
            // clock ticks every ~30 µs, and drawing takes milliseconds, so
            // an event can't follow a pass that closely.
            if before.started != before.finished || newest.is_some_and(|at| before.pass.now < at) {
                std::thread::sleep(StdDuration::from_millis(150));
                continue;
            }
            let canvas = match self.canvas.get() {
                Some(canvas) => canvas,
                None => {
                    let canvas = self.read_words(self.canvas_pointer, 1)?[0];
                    self.canvas.set(Some(canvas));
                    canvas
                }
            };
            // `Canvas` is `repr(C)`: back, front, `front_valid`, then (at
            // the next 4-byte boundary) the theme pointer.
            let bytes = WIDTH * HEIGHT / 2;
            let words = self.read_words(canvas + bytes as u32, bytes / 4 + 2)?;
            let theme = words[bytes / 4 + 1];
            let front: Vec<u8> = words[..bytes / 4]
                .iter()
                .flat_map(|w| w.to_le_bytes())
                .collect();
            // A seqlock: no pass started between the two reads of
            // `started`, so what was read in between (the canvas, and
            // `before`'s pass, which comes after `started`) is one pass's.
            // Not `after`'s pass: the dev kit runs on as this reads, so a
            // pass may have started once its `started` was read.
            let after = self.head()?;
            if after.started == before.started && after.finished == before.finished {
                let palette = self.palette(theme)?;
                return Ok(Some(Frame {
                    head: before,
                    front,
                    palette,
                }));
            }
        }
        Ok(None)
    }
}

/// A whole frame from the dev kit: what it was drawn with, its colour
/// roles (two to a byte), and its theme's palette.
struct Frame {
    head: Head,
    front: Vec<u8>,
    palette: Palette,
}

/// A theme's colours (RGB565, a role each), and which roles it half-tones
/// (a bit each; see `Palette::columns`).
#[derive(Clone)]
struct Palette {
    colors: Vec<u16>,
    halftone: u16,
}

// ==============================================================================
// The simulator, following
// ==============================================================================

/// What the phone said, for a phone event the dev kit logs without its
/// content: the simulator knows what it injected (a test notification, or
/// what a Gadgetbridge line it sent says), so it expects it, in order.
struct Sent {
    event: PhoneEvent,
    /// For a test notification (see `debug::take_backdate_minutes`).
    backdate_minutes: i64,
    /// When it was sent, for how long it took.
    at: StdInstant,
    /// When the dev kit should have had it by: past this, it never will.
    due: StdInstant,
}

/// cortex-m-rt's stack paint (`paint-stack`), which the firmware keeps.
const STACK_PAINT: u32 = 0xCCCC_CCCC;

/// How much of the stack's bottom [`DevKit::stack_left`] reads.
const STACK_LOOK: u32 = 2048;

/// How long the dev kit may take to log what the phone sent, or to send
/// the phone what its UI asked: past this, it never will.
const PHONE_GRACE: StdDuration = StdDuration::from_secs(3);

/// How fast a Gadgetbridge line reaches the dev kit's UI at worst, in
/// bytes a second, on top of [`PHONE_GRACE`]. Once the link has idled, the
/// watch's radio listens about once a second (see `slow_down` in
/// `src/ble.rs`) and takes one link-layer packet (27 bytes) each time:
/// about 28 B/s measured, so a 335-byte notification took 12 s.
const PHONE_IN_RATE: f64 = 20.0;

pub struct Follower<'a> {
    devkit: &'a DevKit,
    rig: &'a mut Rig,
    app: App,
    canvas: Box<Canvas>,
    display: Display,
    /// Events replayed so far.
    replayed: u32,
    /// As the dev kit's UI loop last told its App (every pass, and awake
    /// passes), as its `STATE` events say.
    connected: bool,
    air: bool,
    daytime: bool,
    sent: VecDeque<Sent>,
    /// What the simulator's UI asked to send the phone (its lines), while
    /// the dev kit's was connected, since the last check.
    outbox: Vec<String>,
    /// The button grid's presses the simulator's UI handed BTHome, which
    /// the dev kit's hasn't yet (as its `HA_PRESS` events say), and when.
    ha_presses: VecDeque<(Press, StdInstant)>,
    /// Where the two differed, since the last check.
    ha_differences: Vec<String>,
    /// The grid button the dev kit's UI last sent a hold for, until its
    /// release: see [`Self::check_held`].
    held: Option<u8>,
    /// The storage a `reset` rebooted the dev kit with, since the last
    /// look.
    rebooted: Option<Vec<u8>>,
}

impl<'a> Follower<'a> {
    /// Resets the dev kit (onto `storage` first, if given), and follows
    /// it from its boot.
    fn start(
        devkit: &'a DevKit,
        rig: &'a mut Rig,
        storage: Option<&[u8]>,
        scratch: &Path,
    ) -> Result<Self, String> {
        rig.tidy()?;
        if let Some(image) = storage {
            eprintln!("writing the dev kit's storage");
            devkit.write_storage(image, scratch)?;
        }
        eprintln!("resetting the dev kit");
        let boot = devkit.reset()?.boot;
        let mut follower = Self {
            devkit,
            rig,
            app: boot_app(&boot),
            canvas: Box::new(Canvas::new()),
            display: Display,
            replayed: 0,
            connected: false,
            air: false,
            daytime: false,
            sent: VecDeque::new(),
            outbox: Vec::new(),
            ha_presses: VecDeque::new(),
            ha_differences: Vec::new(),
            held: None,
            rebooted: None,
        };
        follower.follow(&boot)?;
        Ok(follower)
    }

    /// Follows the dev kit from the boot it just made: its UI is new, its
    /// events counted afresh.
    fn follow(&mut self, boot: &compare::Boot) -> Result<(), String> {
        time::set(Instant::from_ticks(boot.at));
        self.app = boot_app(boot);
        self.replayed = 0;
        (self.connected, self.air, self.daytime) = (false, false, false);
        self.sent.clear();
        self.outbox.clear();
        self.ha_presses.clear();
        self.ha_differences.clear();
        if let Some(peer) = &mut self.rig.peer {
            peer.poll();
            peer.received.clear();
            peer.heard.clear();
        }
        eprintln!(
            "following the dev kit from its boot at {} s of uptime",
            boot.at / 32_768
        );
        self.catch_up(None)
    }

    /// The storage the last `reset` rebooted the dev kit with, if one has
    /// since the last call: from there, the dev kit and the simulator know
    /// no more than a run's start does (bar the clock, and a thermometer
    /// left advertising), so what follows can be tried from there alone.
    pub fn take_rebooted(&mut self) -> Option<Vec<u8>> {
        self.rebooted.take()
    }

    pub fn execute(&mut self, line: &str, out: &Path) -> Result<Vec<Finding>, String> {
        // As in a script for the simulator alone: the rest of the line,
        // unsplit.
        if let Some(rest) = line.trim_start().strip_prefix("gb ") {
            self.gadgetbridge(rest.trim())?;
            return Ok(Vec::new());
        }
        let words = script::split(line)?;
        let Some((command, args)) = words.split_first() else {
            return Ok(Vec::new());
        };
        let arg = |i: usize| args.get(i).map(String::as_str);
        let point = |i: usize| arg(i).map_or(Ok((40, 40)), script::point);
        // The dev kit's input kinds (see `src/debug.rs`).
        match command.as_str() {
            "button" => match arg(0) {
                None => self.inject(1, 0, 0, 0)?,
                Some("2") => self.inject(1, 2, 0, 0)?,
                Some(other) => return Err(format!("button {other}?")),
            },
            "glance" => self.inject(8, 0, 0, 0)?,
            "tap" | "long" | "up" | "down" | "left" | "right" => {
                let kind = match command.as_str() {
                    "tap" => 2,
                    "long" => 3,
                    "up" => 4,
                    "down" => 5,
                    "left" => 6,
                    _ => 7,
                };
                let (x, y) = point(0)?;
                let ms = match arg(1) {
                    Some(ms) if kind == 2 => ms
                        .parse::<u8>()
                        .map_err(|_| format!("tap: {ms} ms? (at most 255 on a dev kit)"))?,
                    Some(_) => return Err(format!("{command}: too many arguments")),
                    None => 0,
                };
                self.inject(kind, x as u8, y as u8, ms)?;
            }
            "double" => {
                let (x, y) = script::point(arg(0).ok_or("double: needs X,Y")?)?;
                self.inject(19, x as u8, y as u8, 0)?;
            }
            "press" => {
                let (x, y) = script::point(arg(0).ok_or("press: needs X,Y")?)?;
                self.inject(16, x as u8, y as u8, 0)?;
            }
            "lift" => self.inject(17, 0, 0, 0)?,
            "notify" => {
                let n: u8 =
                    arg(0).map_or(Ok(0), |n| n.parse().map_err(|_| format!("notify {n}?")))?;
                let hours: u8 = arg(1).map_or(Ok(0), |h| {
                    h.trim_end_matches('h')
                        .parse()
                        .map_err(|_| format!("notify: {h} hours ago?"))
                })?;
                let (app, who) = script::test_notification(n);
                self.settle_sent()?;
                self.sent.push_back(Sent {
                    event: PhoneEvent::Notification {
                        app: app.into(),
                        who: who.into(),
                    },
                    backdate_minutes: i64::from(hours) * 60,
                    due: StdInstant::now() + PHONE_GRACE,
                    at: StdInstant::now(),
                });
                self.inject(9, n, hours, 0)?;
            }
            "media" => {
                let state = match arg(0) {
                    None | Some("paused") => 0,
                    Some("playing") => 1,
                    Some("unknown") => 2,
                    Some(other) => {
                        return Err(format!("media {other}? (playing, paused, unknown)"));
                    }
                };
                self.inject(14, state, 0, 0)?;
            }
            "find" => {
                let on = match arg(0) {
                    Some("on") => 1,
                    Some("off") => 0,
                    other => return Err(format!("find {other:?}? (on or off)")),
                };
                self.inject(20, on, 0, 0)?;
            }
            // The quiet night, which a dev kit on a desk never sleeps
            // into: its sleep tracking told the wearer is asleep (or
            // left to say so itself again).
            "asleep" => {
                let on = match arg(0) {
                    Some("on") => 1,
                    Some("off") => 0,
                    other => return Err(format!("asleep {other:?}? (on or off)")),
                };
                self.inject(21, on, 0, 0)?;
            }
            "steps" => {
                let steps: u16 = arg(0)
                    .ok_or("steps: needs N")?
                    .parse()
                    .map_err(|_| "steps: up to 65535 on a dev kit")?;
                let [x, y] = steps.to_le_bytes();
                self.inject(15, x, y, 0)?;
            }
            "battery" => {
                let mv: u16 = arg(0)
                    .ok_or("battery: needs MV")?
                    .parse()
                    .map_err(|_| "battery: MV?")?;
                let [x, y] = mv.to_le_bytes();
                self.inject(13, x, y, 0)?;
            }
            "wait" => {
                let wait = script::duration(arg(0).ok_or("wait: needs a duration")?)?;
                self.idle(StdDuration::from_micros(wait.as_micros()))?;
            }
            "compare" => {
                let name = arg(0).ok_or("compare: needs a NAME")?;
                let mut findings = Vec::new();
                let pixels = self.compare(name, out)?;
                if pixels > 0 {
                    findings.push(Finding {
                        name: name.to_owned(),
                        what: format!("{pixels} pixels differ"),
                    });
                }
                findings.extend(self.check_phone(name));
                findings.extend(self.check_ha(name));
                return Ok(findings);
            }
            "connect" => {
                self.rig.connect()?;
                self.catch_up(None)?;
                // As Gadgetbridge does on every connection: the time first.
                // The watch takes a central for its phone only once it
                // writes (`ble::become_phone`), so without this a script's
                // phone isn't one (media gestures, calls) until its first
                // `gb` line. `connect silent` leaves it out, for a central
                // that never writes.
                match arg(0) {
                    None => self.gadgetbridge(&real_time_line()?.0)?,
                    Some("silent") => {}
                    Some(other) => return Err(format!("connect {other}? (or `connect silent`)")),
                }
            }
            "disconnect" => {
                self.rig.peer()?.request("disconnect")?;
                self.catch_up(None)?;
            }
            "bthome-temp" => {
                self.rig.advertiser = None;
                match arg(0).ok_or("bthome-temp: needs C, or off")? {
                    "off" => {}
                    celsius => {
                        let celsius: f64 = celsius
                            .parse()
                            .map_err(|_| format!("bthome-temp {celsius}? (C, or off)"))?;
                        let hundredths = (celsius * 100.0).round();
                        if !(f64::from(i16::MIN)..=f64::from(i16::MAX)).contains(&hundredths) {
                            return Err(format!("bthome-temp {celsius}: out of BTHome's range"));
                        }
                        self.rig.advertiser = Some(Advertiser::start(hundredths as i16)?);
                    }
                }
            }
            // Restarts the dev kit (as a watchdog or a crash would), and
            // checks it boots with the settings its UI had: what it saved.
            "reset" => {
                let expected = self.settle()?;
                // What it boots with: a start as good as the run's own
                // (see `take_rebooted`).
                self.rebooted = Some(self.devkit.read_storage()?);
                // A hold this reset cut short has no release to come.
                self.held = None;
                let boot = self.devkit.reset()?.boot;
                let booted = Persisted::of(&boot_app(&boot));
                self.follow(&boot)?;
                return Ok(Persisted::differ("reset", &booted, &expected)
                    .into_iter()
                    .collect());
            }
            // Checks the dev kit's storage holds what its UI has: what
            // it would boot with.
            "check-storage" => {
                let expected = self.settle()?;
                let image = self.devkit.read_storage()?;
                let stored = Persisted::of(&booted_from(image)?);
                return Ok(Persisted::differ("storage", &stored, &expected)
                    .into_iter()
                    .collect());
            }
            // What the dev kit's UI is doing, as the simulator's, having
            // replayed the same, says.
            "expect" => {
                let (negated, what) = match arg(0) {
                    Some("not") => (true, arg(1)),
                    what => (false, what),
                };
                self.catch_up(None)?;
                let actual = match what {
                    Some("screen-on") => self.app.awake,
                    Some("dim") => self.app.dim(),
                    Some("quiet-night") => self.app.quiet_night(),
                    other => {
                        return Err(format!(
                            "expect {other:?}? (screen-on, dim, or quiet-night, with a dev kit)"
                        ));
                    }
                };
                if actual == negated {
                    let not = if negated { "not " } else { "" };
                    return Err(format!("expected {not}{}", what.unwrap_or_default()));
                }
            }
            "echo" => eprintln!("{}", args.join(" ")),
            other => {
                return Err(format!(
                    "{other}: not something a dev kit can be made to do (see `ur-sim --help`)"
                ));
            }
        }
        Ok(Vec::new())
    }

    /// Follows the dev kit for `wait`, doing nothing else.
    fn idle(&mut self, wait: StdDuration) -> Result<(), String> {
        let until = StdInstant::now() + wait;
        while let Some(left) = until.checked_duration_since(StdInstant::now()) {
            std::thread::sleep(left.min(POLL));
            self.catch_up(None)?;
        }
        Ok(())
    }

    /// The lines the dev kit sent this computer since the last call (or
    /// since it booted), as it sent them, for a model of Gadgetbridge to
    /// hear (`gadgetbridge.rs`). Gives the dev kit `settle` first, for the
    /// answers to what was just sent to come over the air. These are
    /// every line, not only the UI's that `compare` checks: the time, the
    /// battery, an activity fetch's.
    pub fn take_heard(&mut self, settle: StdDuration) -> Result<Vec<Vec<u8>>, String> {
        self.idle(settle)?;
        let Some(peer) = &mut self.rig.peer else {
            return Ok(Vec::new());
        };
        peer.poll();
        Ok(std::mem::take(&mut peer.heard))
    }

    /// `gb LINE`: sends the line, as Gadgetbridge would, from this
    /// computer; expects what it makes the UI do, as `ble::interpret`
    /// passes it on.
    fn gadgetbridge(&mut self, line: &str) -> Result<(), String> {
        let bytes = script::gb_wire(line)?;
        let mut expected = Vec::new();
        LineBuffer::new().push(&bytes, |line| {
            if let Some(Incoming::Phone {
                event,
                text_as_bitmaps,
            }) = gb::parse(line)
            {
                expected.push(event);
                if text_as_bitmaps {
                    expected.push(PhoneEvent::TextAsBitmaps);
                }
            }
        });
        let at = StdInstant::now();
        // (Bytes are at most a few KB: exactly an `f64`.)
        #[allow(clippy::cast_precision_loss)]
        let due = at + PHONE_GRACE + StdDuration::from_secs_f64(bytes.len() as f64 / PHONE_IN_RATE);
        self.sent.extend(expected.into_iter().map(|event| Sent {
            event,
            backdate_minutes: 0,
            at,
            due,
        }));
        let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        // Following the dev kit meanwhile: a long line takes seconds over
        // the air, while its UI may log ten events a second (scrolling
        // text), more than its export keeps.
        let started = StdInstant::now();
        self.rig.peer()?.begin(&format!("send {hex}"))?;
        loop {
            self.catch_up(None)?;
            if self
                .rig
                .peer()?
                .answer(StdDuration::from_millis(500))?
                .is_some()
            {
                break;
            }
        }
        let took = started.elapsed();
        if took > StdDuration::from_secs(1) {
            eprintln!("  phone: {} bytes took {took:.1?} to send", bytes.len());
        }
        self.catch_up(None)
    }

    /// Waits for the dev kit to save what its UI changed (it saves after
    /// each event, but flash takes its turn with the radio), and returns
    /// what the simulator's UI says it should have.
    fn settle(&mut self) -> Result<Persisted, String> {
        std::thread::sleep(StdDuration::from_secs(2));
        self.catch_up(None)?;
        Ok(Persisted::of(&self.app))
    }

    /// Checks the button grid's presses for Home Assistant, since the last
    /// check: the dev kit's UI's against the simulator's.
    fn check_ha(&mut self, name: &str) -> Vec<Finding> {
        // The dev kit's come once its radio's done with each (a burst of
        // adverts), so a moment after the simulator's.
        for (press, at) in &self.ha_presses {
            if at.elapsed() > PHONE_GRACE {
                self.ha_differences.push(format!(
                    "the simulator's UI pressed {} {:?}; the dev kit's nothing",
                    press.code, press.gesture
                ));
            }
        }
        (self.ha_presses).retain(|(_, at)| at.elapsed() <= PHONE_GRACE);
        if self.ha_differences.is_empty() {
            return Vec::new();
        }
        vec![Finding {
            name: format!("{name}-ha"),
            what: std::mem::take(&mut self.ha_differences).join("; "),
        }]
    }

    /// Waits until the dev kit has logged every line sent to it (or they're
    /// overdue, for [`Self::check_phone`] to report). A test notification
    /// injected over the probe arrives at once, so it can overtake a
    /// Gadgetbridge line still on its way over the air (seconds, on an
    /// idle link). The dev kit would then log them in an order this can't
    /// tell from a wrong one: it matches what it sent by kind, in order.
    fn settle_sent(&mut self) -> Result<(), String> {
        while let Some(due) = self.sent.iter().map(|sent| sent.due).max() {
            let Some(left) = due.checked_duration_since(StdInstant::now()) else {
                break;
            };
            std::thread::sleep(left.min(POLL));
            self.catch_up(None)?;
        }
        Ok(())
    }

    /// Checks what the dev kit sent the phone (this computer) against what
    /// the simulator's UI asked to send, since the last check; and that the
    /// dev kit got everything sent to it.
    fn check_phone(&mut self, name: &str) -> Vec<Finding> {
        let mut findings = Vec::new();
        let late: Vec<_> = self
            .sent
            .iter()
            .filter(|sent| sent.due < StdInstant::now())
            .map(|sent| describe(&InputEvent::Phone(sent.event.clone())))
            .collect();
        if !late.is_empty() {
            self.sent.retain(|sent| sent.due >= StdInstant::now());
            findings.push(Finding {
                name: format!("{name}-phone-in"),
                what: format!("the dev kit's UI never got {}", late.join(", ")),
            });
        }
        let Some(peer) = &mut self.rig.peer else {
            self.outbox.clear();
            return findings;
        };
        // The UI's own messages: the rest (the time, the battery, activity)
        // the dev kit's Bluetooth task sends of its own accord.
        let ui = |line: &String| {
            [r#"{"t":"call""#, r#"{"t":"music""#, r#"{"t":"intent""#]
                .iter()
                .any(|prefix| line.starts_with(prefix))
        };
        let deadline = StdInstant::now() + PHONE_GRACE;
        loop {
            peer.poll();
            if peer.received.iter().filter(|line| ui(line)).count() >= self.outbox.len()
                || StdInstant::now() > deadline
            {
                break;
            }
            std::thread::sleep(StdDuration::from_millis(100));
        }
        let theirs: Vec<String> = peer.received.drain(..).filter(|line| ui(line)).collect();
        let ours = std::mem::take(&mut self.outbox);
        if theirs != ours {
            findings.push(Finding {
                name: format!("{name}-phone-out"),
                what: format!("the dev kit sent the phone {theirs:?}; the simulator's UI {ours:?}"),
            });
        }
        findings
    }

    fn inject(&mut self, kind: u8, x: u8, y: u8, extra: u8) -> Result<(), String> {
        // A tap (or a finger, which may make one) on the storage mode
        // prompt would switch the dev kit off: a button cancels it instead,
        // as anything but a tap does. (By the simulator's UI, which has
        // seen what the dev kit's has.)
        self.catch_up(None)?;
        let (kind, x, y, extra) = if matches!(kind, 2 | 16 | 19) && self.app.offering_storage_mode()
        {
            eprintln!("  (a button, not the tap, on the storage mode prompt)");
            (1, 0, 0, 0)
        } else {
            (kind, x, y, extra)
        };
        self.devkit.inject(kind, x, y, extra)?;
        self.catch_up(None)
    }

    /// The stack's room left at its deepest so far: see
    /// [`DevKit::stack_left`].
    pub fn stack_left(&self) -> Result<Option<u32>, String> {
        self.devkit.stack_left()
    }

    /// Why the dev kit restarted, if it has: see [`DevKit::boot_log`].
    pub fn boot_log(&self) -> String {
        self.devkit.boot_log()
    }

    /// Checks the dev kit's own grid presses against what Home Assistant
    /// is promised (see `buttons`): a hold always ends in its release,
    /// before any other press. The simulator can't tell: it replays the
    /// gestures the dev kit's touch task made, so a release that task
    /// never made is missing from both (as when a lift beats the long
    /// press timer, or the quiet night puts the touch controller to sleep
    /// mid-hold).
    fn check_held(&mut self, press: Press) {
        use crate::buttons::Gesture;
        match (self.held, press.gesture) {
            (None, Gesture::Release) => self.ha_differences.push(format!(
                "the dev kit's UI released {} with no hold before it",
                press.code
            )),
            (Some(held), Gesture::Release) if held != press.code => self.ha_differences.push(
                format!("the dev kit's UI held {held} but released {}", press.code),
            ),
            (Some(held), Gesture::Hold | Gesture::Tap | Gesture::DoubleTap) => {
                self.ha_differences.push(format!(
                    "the dev kit's UI held {held}, and then pressed {} {:?} with no \
                     release between",
                    press.code, press.gesture
                ));
            }
            _ => {}
        }
        self.held = match press.gesture {
            Gesture::Hold => Some(press.code),
            Gesture::Release => None,
            _ => self.held,
        };
    }

    /// Which of a coverage build's functions have run since the dev kit
    /// booted (`None` without one).
    pub fn coverage(&self) -> Result<Option<Vec<bool>>, String> {
        self.devkit.coverage()
    }

    /// Replays the dev kit's events since the last time, up to (not
    /// including) `before`, if given.
    fn catch_up(&mut self, before: Option<u64>) -> Result<(), String> {
        let head = self.devkit.head()?;
        // Not yet (or no longer) the app's export: the dev kit restarted,
        // and is in its bootloader, or booting. (Its counter there is
        // whatever the bootloader left in RAM.)
        if head.magic != compare::MAGIC {
            return Err(
                "the dev kit restarted: its export is gone (in its bootloader, or booting); \
                        see its log"
                    .into(),
            );
        }
        let written = head.written;
        let behind = written.wrapping_sub(self.replayed) as usize;
        if behind > compare::EVENTS {
            return Err(format!(
                "fell behind the dev kit's events ({behind} since the last look; it keeps {}), \
                 or it restarted (see its log)",
                compare::EVENTS
            ));
        }
        for event in self.devkit.events(self.replayed, written)? {
            // (At the same instant as the pass: before it; see `frame`.)
            if before.is_some_and(|before| event.at > before) {
                break;
            }
            self.replay(&event)?;
            self.replayed = self.replayed.wrapping_add(1);
        }
        Ok(())
    }

    fn replay(&mut self, event: &Event) -> Result<(), String> {
        let at = Instant::from_ticks(event.at);
        time::set(at);
        // The flags as the dev kit's last pass left them, as its UI had
        // them for this event. Not for a change of them, though: the pass
        // that logged it applied the new flags straight away, and the old
        // ones applied first could undo what came before (a disconnect
        // forgets the music, so a `musicstate` handled just before the
        // pass that saw the phone connect would be lost here only).
        if event.kind != kind::STATE {
            self.app.set_connected(self.connected);
            self.app.set_air(self.air);
            self.app.set_daytime(self.daytime);
        }
        let input = match event.kind {
            kind::TICK => {
                let second =
                    (event.local_second != compare::NONE_I64).then_some(event.local_second);
                let flag = |f| event.flags & f != 0;
                self.app.check_alarms(
                    at,
                    second,
                    flag(compare::flag::STIRRING),
                    flag(compare::flag::ASLEEP),
                );
                let ticked = Instant::from_ticks(event.ticked);
                time::set(ticked);
                self.app.on_tick(
                    ticked,
                    flag(compare::flag::WATCHING),
                    flag(compare::flag::FINGER_DOWN),
                );
                None
            }
            kind::BUTTON => Some(InputEvent::Button),
            kind::ATTENTION => Some(InputEvent::Attention),
            kind::GLANCE => Some(InputEvent::Glance),
            kind::TOUCH => Some(InputEvent::Touch(Touch {
                gesture: compare::gesture_from_code(event.gesture)
                    .ok_or("the dev kit logged an unknown gesture")?,
                x: i32::from(event.x),
                y: i32::from(event.y),
                held_ms: event.held_ms,
            })),
            kind::PHONE => Some(InputEvent::Phone(self.phone(event)?)),
            kind::STATE => {
                let flag = |f| event.flags & f != 0;
                let (connected, air, daytime) = (
                    flag(compare::state::CONNECTED),
                    flag(compare::state::AIR),
                    flag(compare::state::DAYTIME),
                );
                for (what, was, is) in [
                    ("a phone connected", self.connected, connected),
                    ("a room sensor heard", self.air, air),
                    ("daytime", self.daytime, daytime),
                ] {
                    if was != is {
                        let no = if is { "" } else { "no longer " };
                        eprintln!("  dev kit: {no}{what} at {:.3} s", seconds(at));
                    }
                }
                (self.connected, self.air, self.daytime) = (connected, air, daytime);
                self.app.set_connected(connected);
                self.app.set_air(air);
                self.app.set_daytime(daytime);
                None
            }
            kind::HA_PRESS => {
                let press = Press {
                    code: event.x,
                    gesture: compare::ha_gesture_from_code(event.gesture)
                        .ok_or("the dev kit logged an unknown grid gesture")?,
                };
                let sent = event.flags != 0;
                self.check_held(press);
                eprintln!(
                    "  dev kit: a grid press for Home Assistant, {} {:?}{} at {:.3} s",
                    press.code,
                    press.gesture,
                    if sent { "" } else { ", not sent" },
                    seconds(at)
                );
                match self.ha_presses.pop_front() {
                    Some((ours, _)) if ours == press => {}
                    ours => self.ha_differences.push(format!(
                        "the dev kit's UI pressed {} {:?}; the simulator's {}",
                        press.code,
                        press.gesture,
                        ours.map_or("nothing".to_owned(), |(ours, _)| format!(
                            "{} {:?}",
                            ours.code, ours.gesture
                        ))
                    )),
                }
                // Whether it went is the radio's say, as the simulator's
                // `slots full` is.
                if !sent {
                    self.app.ha_unsent(press, at);
                }
                None
            }
            kind::TIMER => {
                let finger_down = event.flags & compare::flag::FINGER_DOWN != 0;
                self.app.on_timer(at, finger_down);
                None
            }
            other => return Err(format!("the dev kit logged an unknown event ({other})")),
        };
        if let Some(input) = input {
            eprintln!("  dev kit: {} at {:.3} s", describe(&input), seconds(at));
            self.app.on_input(input, at);
        }
        // What the events set off on the dev kit (buzzes, messages for the
        // phone) isn't drawn: drop it here, so it doesn't pile up.
        while crate::motor::take_request().is_some() {}
        // What it sends the phone is checked (see `check_phone`): the dev
        // kit's goes nowhere unless connected.
        while let Ok(message) = crate::ble::OUTBOX.try_receive() {
            if self.connected {
                self.outbox.push(message.to_line().trim_end().to_owned());
            }
        }
        while let Some(press) = self.app.take_ha_press() {
            self.ha_presses.push_back((press, StdInstant::now()));
        }
        Ok(())
    }

    /// What the phone sent, by the kind the dev kit logged: what this sent
    /// it, in order.
    fn phone(&mut self, event: &Event) -> Result<PhoneEvent, String> {
        let next = self
            .sent
            .front()
            .map(|sent| compare::phone_code(&sent.event));
        if next == Some(event.phone) {
            let sent = self.sent.pop_front().expect("there's a next");
            // From starting to send to seeing it here (to within a look, of
            // 0.5 s while sending): how long the line took over the air.
            let took = sent.at.elapsed();
            if took > StdDuration::from_secs(1) {
                eprintln!("  phone: the dev kit had it {took:.1?} after it was sent");
            }
            return Ok(match sent.event {
                PhoneEvent::Notification { app, who } => {
                    debug::backdate_next(sent.backdate_minutes);
                    PhoneEvent::Notification { app, who }
                }
                PhoneEvent::MusicInfo { artist, track } => {
                    // The text as the dev kit got it, too: the same.
                    let got = self.devkit.music(event.held_ms)?;
                    if got != (artist.clone(), track.clone()) {
                        return Err(format!(
                            "the dev kit's music info says {got:?}, not \
                             ({artist:?}, {track:?}) as sent"
                        ));
                    }
                    PhoneEvent::MusicInfo { artist, track }
                }
                other => other,
            });
        }
        Ok(match event.phone {
            // The `media` input's (see `src/debug.rs`), or a connected
            // phone's own: what the dev kit got, either way.
            phone::MUSIC_INFO => {
                let (artist, track) = self.devkit.music(event.held_ms)?;
                PhoneEvent::MusicInfo { artist, track }
            }
            phone::MUSIC_STATE_PAUSED => PhoneEvent::MusicState { playing: false },
            phone::MUSIC_STATE_PLAYING => PhoneEvent::MusicState { playing: true },
            phone::TEXT_AS_BITMAPS => PhoneEvent::TextAsBitmaps,
            // The `find` input's, or a connected phone's: all there is.
            phone::FIND => PhoneEvent::Find { on: true },
            phone::FIND_OFF => PhoneEvent::Find { on: false },
            other => {
                let expected = match &self.sent.front() {
                    Some(sent) => format!(
                        ", expecting {}",
                        describe(&InputEvent::Phone(sent.event.clone()))
                    ),
                    None => String::new(),
                };
                return Err(format!(
                    "the dev kit got a message from the phone (kind {other}) this can't \
                     reproduce{expected} (is a phone connected to it?)"
                ));
            }
        })
    }

    /// Compares the dev kit's latest whole frame with the simulator's
    /// drawing of it; writes them side by side to `out/NAME.png`. Returns
    /// how many canvas pixels differ.
    fn compare(&mut self, name: &str, out: &Path) -> Result<usize, String> {
        // With the screen off, the panel keeps the last frame it showed,
        // from before what's happened since: nothing to compare. Whether
        // it's off is the simulator's UI's call, having seen the same.
        let off = |follower: &Self| {
            (!follower.app.awake).then(|| {
                eprintln!("compare {name}: skipped: the screen is off");
                0
            })
        };
        self.catch_up(None)?;
        if let Some(skipped) = off(self) {
            return Ok(skipped);
        }
        let Some(Frame {
            head,
            front,
            palette,
        }) = self.devkit.frame()?
        else {
            self.catch_up(None)?;
            return off(self).ok_or_else(|| {
                format!(
                    "compare {name}: the dev kit drew nothing after its last event, \
                     though the simulator's screen is on"
                )
            });
        };
        let pass = head.pass;
        // Everything before the frame, and nothing after.
        self.catch_up(Some(pass.now))?;
        self.render(&pass);
        let devkit: Vec<u16> = (0..WIDTH * HEIGHT)
            .map(|i| {
                let mut role = if i % 2 == 0 {
                    front[i / 2] & 0x0F
                } else {
                    front[i / 2] >> 4
                };
                // As the simulator's panel shows it at the pixel compared
                // below, the middle of the canvas pixel's three columns.
                if palette.halftone >> role & 1 != 0 && (i % WIDTH * 3 + 1) % 2 == 1 {
                    role = 0;
                }
                (palette.colors)
                    .get(usize::from(role))
                    .copied()
                    .unwrap_or(0xF81F)
            })
            .collect();
        let sim: Vec<u16> = PANEL.with_borrow(|panel| {
            (0..WIDTH * HEIGHT)
                .map(|i| {
                    let (x, y) = (i % WIDTH * 3, i / WIDTH * 3);
                    panel.pixels[(y + 1) * SIZE + x + 1]
                })
                .collect()
        });
        // The HA key's QR code is drawn over the panel, beneath the canvas
        // the dev kit exports (see `Canvas::qr`): that square goes
        // uncompared, bar its key, which the simulator drew from the
        // export (`render`). The canvas around it is compared as ever.
        let qr = self.canvas.qr_area();
        let hidden = |i: usize| {
            qr.as_ref().is_some_and(|area| {
                area.contains(&(i % WIDTH * 3 + 1)) && area.contains(&(i / WIDTH * 3 + 1))
            })
        };
        let differ = (0..WIDTH * HEIGHT)
            .filter(|&i| sim[i] != devkit[i] && !hidden(i))
            .count();
        report_push(name, &head.push);
        let path = out.join(format!("{name}.png"));
        side_by_side(&path, &sim, &devkit)?;
        let time = if pass.has_time != 0 {
            format!("{:02}:{:02}:{:02}", pass.hour, pass.minute, pass.second)
        } else {
            "no time".into()
        };
        eprintln!(
            "compare {name}: the frame at {:.3} s ({time}): {}{} -> {}",
            seconds(Instant::from_ticks(pass.now)),
            if differ == 0 {
                "identical".to_owned()
            } else {
                format!("{differ} pixels differ")
            },
            if qr.is_some() {
                " around the QR code (which the dev kit doesn't export)"
            } else {
                ""
            },
            path.display()
        );
        Ok(differ)
    }

    /// Draws the frame the dev kit drew at `pass.now`, with its readings.
    fn render(&mut self, pass: &Pass) {
        let now = Instant::from_ticks(pass.now);
        time::set(now);
        self.connected = pass.connected != 0;
        self.daytime = pass.daytime != 0;
        self.app.set_connected(self.connected);
        self.app.set_daytime(self.daytime);
        if pass.has_offset != 0 && pass.local_second != compare::NONE_I64 {
            let offset = pass.offset_minutes;
            clock::set(pass.local_second - i64::from(offset) * 60, offset);
        }
        activity::show_devkit_day(pass.day());
        let battery = Battery::measured(
            pass.battery_mv,
            pass.battery_percent,
            pass.charging != 0,
            pass.power_present != 0,
        );
        let status = Status {
            uptime_secs: pass.uptime_secs,
            local_time: (pass.has_time != 0).then_some(DateTime {
                year: pass.year,
                month: pass.month,
                day: pass.day,
                hour: pass.hour,
                minute: pass.minute,
                second: pass.second,
            }),
            connected: self.connected,
            unpaired: match pass.unpaired {
                1 => Some(Unpaired::NotPaired),
                2 => Some(Unpaired::PairingLost),
                _ => None,
            },
            passkey: (pass.passkey != compare::NONE_U32).then_some(pass.passkey),
            update_progress: (pass.update_progress != 255).then_some(pass.update_progress),
            steps: (pass.steps != compare::NONE_U32).then_some(pass.steps),
            heart_rate: (pass.heart_rate != 0).then_some(pass.heart_rate),
            temperature: (pass.has_temperature != 0).then_some(pass.temperature),
            temperature_from_sensor: pass.has_temperature == 2,
            air: pass.air.sighting(),
            sleep: (pass.has_sleep != 0).then_some(activity::Sleep {
                start_utc_minute: pass.sleep_start_utc_minute,
                end_utc_minute: pass.sleep_end_utc_minute,
                asleep_minutes: pass.sleep_asleep_minutes,
                deep_minutes: pass.sleep_deep_minutes,
                rem_minutes: pass.sleep_rem_minutes,
            }),
            local_second: (pass.local_second != compare::NONE_I64).then_some(pass.local_second),
            battery: &battery,
            bthome: (pass.has_bthome != 0).then_some(crate::bthome::Identity {
                key: pass.bthome_key,
                address: pass.bthome_address,
            }),
        };
        self.canvas.set_palette(self.app.palette());
        self.app.render(&mut self.canvas, &status, now);
        let Self {
            canvas, display, ..
        } = self;
        crate::watch::run(canvas.flush(display), &mut None);
    }
}

/// `image` as an ELF that `esprobe flash` writes to the storage region:
/// one loadable section (`.storage`, as probe-rs wants a section as well
/// as the segment), at [`storage::RANGE`]'s start.
fn storage_elf(image: &[u8]) -> Vec<u8> {
    const HEADER: u32 = 52;
    const SEGMENT: u32 = 32;
    const SECTION: u32 = 40;
    let names = b"\0.storage\0.shstrtab\0";
    let address = storage::RANGE.start;
    let data_at = HEADER + SEGMENT;
    let names_at = data_at + image.len() as u32;
    let sections_at = (names_at + names.len() as u32).next_multiple_of(4);
    let words = |words: &[u32]| {
        words
            .iter()
            .flat_map(|w| w.to_le_bytes())
            .collect::<Vec<_>>()
    };
    let halves = |halves: &[u16]| {
        halves
            .iter()
            .flat_map(|h| h.to_le_bytes())
            .collect::<Vec<_>>()
    };
    let mut elf = b"\x7fELF\x01\x01\x01\0\0\0\0\0\0\0\0\0".to_vec();
    // An executable, for ARM, version 1, entering at the data (unused),
    // with its segment table, sections, and flags (EABI 5, hard float).
    elf.extend(halves(&[2, 40]));
    elf.extend(words(&[1, address, HEADER, sections_at, 0x0500_0400]));
    elf.extend(halves(&[
        HEADER as u16,
        SEGMENT as u16,
        1,
        SECTION as u16,
        3,
        2,
    ]));
    // The segment: loaded, at `address`, read and execute.
    let size = image.len() as u32;
    elf.extend(words(&[1, data_at, address, address, size, size, 5, 4]));
    elf.extend(image);
    elf.extend(names);
    elf.resize(sections_at as usize, 0);
    elf.extend([0; SECTION as usize]);
    // `.storage`: bits, allocated and executable; then the names.
    elf.extend(words(&[1, 1, 6, address, data_at, size, 0, 0, 4, 0]));
    elf.extend(words(&[
        10,
        3,
        0,
        0,
        names_at,
        names.len() as u32,
        0,
        0,
        1,
        0,
    ]));
    elf
}

/// How long the dev kit took to push its frame to the panel, if it pushed
/// anything, and how long the simulator's panel says that push takes (see
/// `display::cost`). The simulator's own push of the frame isn't
/// comparable: it follows only the frames it compares, so it pushes what
/// changed since the last of those.
///
/// The dev kit's time runs from the first address window to the frame's
/// being finished, so a transition's includes its frame timer's waits,
/// which the model's sum of costs doesn't.
fn report_push(name: &str, push: &compare::Push) {
    if push.windows == 0 {
        return;
    }
    let took = push.finished.saturating_sub(push.started) as f64 / 32.768;
    let model = crate::display::cost::push(
        u64::from(push.windows),
        u64::from(push.transfers),
        u64::from(push.bytes),
    ) as f64
        / 1e6;
    eprintln!(
        "compare {name}: the dev kit pushed {} windows, {} transfers ({} chunks), {} bytes \
         in {took:.1} ms; the simulator's panel takes {model:.1} ms",
        push.windows, push.transfers, push.chunks, push.bytes
    );
}

fn seconds(at: Instant) -> f64 {
    at.as_ticks() as f64 / 32_768.0
}

fn describe(event: &InputEvent) -> String {
    match event {
        InputEvent::Button => "button".into(),
        InputEvent::Glance => "glance".into(),
        InputEvent::Attention => "attention".into(),
        InputEvent::Touch(touch) => format!(
            "{} at ({},{}), {} ms",
            match touch.gesture {
                Gesture::Tap => "tap",
                Gesture::LongPress => "long press",
                Gesture::SwipeUp => "swipe up",
                Gesture::SwipeDown => "swipe down",
                Gesture::SwipeLeft => "swipe left",
                Gesture::SwipeRight => "swipe right",
                Gesture::Release => "release",
            },
            touch.x,
            touch.y,
            touch.held_ms
        ),
        InputEvent::Phone(message) => format!(
            "a message from the phone ({})",
            match message {
                PhoneEvent::Notification { .. } => "a notification",
                PhoneEvent::IncomingCall { .. } => "a call",
                PhoneEvent::CallOver => "the call over",
                PhoneEvent::MusicInfo { .. } => "music info",
                PhoneEvent::MusicState { .. } => "music state",
                PhoneEvent::Find { .. } => "find",
                PhoneEvent::Unpaired(_) => "unpaired",
                PhoneEvent::Alarms(_) => "alarms",
                PhoneEvent::Route(_) => "a route",
                PhoneEvent::TextAsBitmaps => "text as bitmaps",
                PhoneEvent::IntentsOff => "intents off",
                PhoneEvent::UpdateRefused => "update refused",
            }
        ),
    }
}

/// Writes the simulator's frame, the dev kit's, and their differences side
/// by side (see [`shot::save_difference`]), each canvas pixel drawn as the
/// panel's block of pixels.
fn side_by_side(path: &Path, sim: &[u16], devkit: &[u16]) -> Result<(), String> {
    // Both are the canvas's own pixels (the dev kit exports its canvas, not
    // the panel), so each fills a square of the panel.
    let scale = SIZE / WIDTH;
    let image = |canvas: &[u16]| shot::Image {
        width: WIDTH * scale,
        height: HEIGHT * scale,
        rgb: (0..HEIGHT * scale)
            .flat_map(|y| (0..WIDTH * scale).map(move |x| (y / scale) * WIDTH + x / scale))
            .flat_map(|i| rgb888(canvas[i], false))
            .collect(),
    };
    shot::save_difference(path, &image(sim), &image(devkit)).map(|_| ())
}

// ==============================================================================
// Settings: what the dev kit boots with
// ==============================================================================

/// The UI the dev kit's boot made, from the settings it exported.
fn boot_app(boot: &compare::Boot) -> App {
    let themes = ThemeChoice {
        light: boot.theme_light,
        dark: boot.theme_dark,
    };
    let mut app = App::new(
        Instant::from_ticks(boot.at),
        themes,
        alarm::Settings::from_bytes(boot.alarms),
    );
    app.phone_alarms = alarm::Phone::from_bytes(boot.phone_alarms);
    app.brightness = brightness::Setting::from_byte(boot.brightness);
    app.buttons = crate::buttons::Setting::from_byte(boot.buttons);
    app.grid_layout = crate::buttons::Layout::from_byte(boot.grid);
    app
}

/// The UI a watch with this storage would boot with, through the
/// firmware's own loading (`ui_loop::Stored`).
pub(crate) fn booted_from(image: Vec<u8>) -> Result<App, String> {
    let flash = Flash::from_bytes(image).ok_or("the storage read short")?;
    storage::init(flash);
    Ok(UiLoop::boot(crate::watch::run(Stored::load(), &mut None)).0)
}

/// The settings the watch keeps in flash, as bytes, to compare.
#[derive(PartialEq, Debug)]
pub(crate) struct Persisted {
    light_theme: u8,
    dark_theme: u8,
    brightness: u8,
    buttons: u8,
    grid: u8,
    alarms: [u8; alarm::Settings::BYTES],
    phone_alarms: [u8; alarm::Phone::BYTES],
}

impl Persisted {
    pub(crate) fn of(app: &App) -> Self {
        Self {
            light_theme: app.themes.light,
            dark_theme: app.themes.dark,
            brightness: app.brightness.to_byte(),
            buttons: app.buttons.to_byte(),
            grid: app.grid_layout.to_byte(),
            alarms: app.alarms.to_bytes(),
            phone_alarms: app.phone_alarms.to_bytes(),
        }
    }

    /// What differs between the dev kit's (`got`) and what the simulator's
    /// UI says it should be.
    pub(crate) fn differ(name: &str, got: &Self, expected: &Self) -> Option<Finding> {
        let mut what = Vec::new();
        let mut field = |field: &str, got: &dyn std::fmt::Debug, expected: &dyn std::fmt::Debug| {
            let (got, expected) = (format!("{got:?}"), format!("{expected:?}"));
            if got != expected {
                what.push(format!("{field} {got} (the simulator's UI: {expected})"));
            }
        };
        field("light theme", &got.light_theme, &expected.light_theme);
        field("dark theme", &got.dark_theme, &expected.dark_theme);
        field("brightness", &got.brightness, &expected.brightness);
        field("buttons", &got.buttons, &expected.buttons);
        field("grid", &got.grid, &expected.grid);
        field("alarms", &got.alarms, &expected.alarms);
        field("phone alarms", &got.phone_alarms, &expected.phone_alarms);
        (!what.is_empty()).then(|| Finding {
            name: name.to_owned(),
            what: format!("the dev kit's {}", what.join("; ")),
        })
    }
}

/// The dev kit, and what stands around it, for run after run (`--fuzz`).
pub struct Bench {
    devkit: DevKit,
    rig: Rig,
}

impl Bench {
    pub fn open(elf: &Path, peer: Option<String>) -> Result<Self, String> {
        Ok(Self {
            devkit: DevKit::open(elf)?,
            rig: Rig::new(peer),
        })
    }

    pub fn read_storage(&self) -> Result<Vec<u8>, String> {
        self.devkit.read_storage()
    }

    /// What its storage writes and resets have taken so far.
    pub fn spent(&self) -> Spent {
        self.devkit.spent.get()
    }

    /// A run: the dev kit reset onto `storage`, and followed.
    /// Each of a coverage build's flags' functions, by name and source
    /// line, in the flags' order: through the ELF's PC table (which
    /// `coverage.x` keeps out of flash) and `llvm-addr2line`. `None` without
    /// a coverage build.
    pub fn functions(&self, elf: &Path, scratch: &Path) -> Result<Option<Vec<String>>, String> {
        if self.devkit.flags.is_none() {
            return Ok(None);
        }
        std::fs::create_dir_all(scratch)
            .map_err(|e| format!("creating {}: {e}", scratch.display()))?;
        let table = scratch.join("sancov-pcs.bin");
        let dumped = Command::new("llvm-objcopy")
            .arg(format!("--dump-section=.sancov_pcs={}", table.display()))
            .arg(elf)
            .arg(scratch.join("sancov-objcopy.elf"))
            .status()
            .map_err(|e| format!("running llvm-objcopy: {e}"))?;
        if !dumped.success() {
            return Err("llvm-objcopy couldn't dump the ELF's .sancov_pcs".into());
        }
        let table = std::fs::read(&table).map_err(|e| format!("reading the PC table: {e}"))?;
        // Each flag's entry: its function's address (Thumb: odd), and
        // flags (1: a function's entry, which they all are here).
        let pcs: Vec<String> = table
            .as_chunks::<8>()
            .0
            .iter()
            .map(|entry| {
                let pc = u32::from_le_bytes([entry[0], entry[1], entry[2], entry[3]]);
                format!("{:#x}", pc & !1)
            })
            .collect();
        let output = Command::new("llvm-addr2line")
            .args(["-f", "-C", "-e"])
            .arg(elf)
            .args(&pcs)
            .output()
            .map_err(|e| format!("running llvm-addr2line: {e}"))?;
        let text = String::from_utf8_lossy(&output.stdout);
        let lines: Vec<&str> = text.lines().collect();
        // Two lines each: the function, then file:line.
        Ok(Some(
            lines
                .chunks(2)
                .map(|pair| {
                    let file = pair
                        .get(1)
                        .map_or("?", |f| f.rsplit_once("/src/").map_or(f, |(_, rest)| rest));
                    format!("{} ({file})", pair[0])
                })
                .collect(),
        ))
    }

    pub fn follow(&mut self, storage: &[u8], scratch: &Path) -> Result<Follower<'_>, String> {
        Follower::start(&self.devkit, &mut self.rig, Some(storage), scratch)
    }

    /// Puts the dev kit's storage back as it was, and resets it; then,
    /// with this computer as its phone, sets its clock to the real time,
    /// as the runs set it to others (and it'd record its activity at
    /// theirs until its phone next connects).
    pub fn restore(&mut self, original: &[u8], scratch: &Path) -> Result<(), String> {
        self.rig.tidy()?;
        self.devkit.write_storage(original, scratch)?;
        self.devkit.reset()?;
        if self.rig.address.is_some() {
            // Best effort: the storage, which matters more, is back.
            if let Err(e) = self.real_time() {
                eprintln!("fuzz: couldn't set the dev kit's clock to the real time: {e}");
            }
        }
        Ok(())
    }

    fn real_time(&mut self) -> Result<(), String> {
        let (line, utc, zone) = real_time_line()?;
        let hex: String = (script::gb_wire(&line)?.iter())
            .map(|b| format!("{b:02x}"))
            .collect();
        self.rig.connect()?;
        let peer = self.rig.peer()?;
        peer.request(&format!("send {hex}"))?;
        peer.request("disconnect")?;
        eprintln!("fuzz: the dev kit's clock is at the real time again ({utc}, UTC{zone})");
        Ok(())
    }
}

/// Gadgetbridge's line setting the clock to this computer's: the line, the
/// time (UTC seconds), and the offset (`+0100` or the like).
fn real_time_line() -> Result<(String, u64, String), String> {
    let utc = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_secs();
    let output = Command::new("date")
        .arg("+%z")
        .output()
        .map_err(|e| format!("running date: {e}"))?;
    let zone = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    let minutes = |digits: &str| digits.parse::<i64>().map_err(|e| format!("{zone}: {e}"));
    let (sign, digits) = zone.split_at_checked(1).ok_or("date +%z said nothing")?;
    let (hours, rest) = digits.split_at_checked(2).ok_or("date +%z's hours")?;
    let offset = (minutes(hours)? * 60 + minutes(rest)?) * if sign == "-" { -1 } else { 1 };
    let line = set_time_line(utc as i64, offset as f64 / 60.0);
    Ok((line, utc, zone))
}

/// Gadgetbridge's line setting the clock to `utc` (seconds), in a time
/// zone `zone` hours from UTC.
pub fn set_time_line(utc: i64, zone: f64) -> String {
    format!(
        "setTime({utc});E.setTimeZone({zone:.1});(s=>s&&(s.timezone={zone:.1},\
         require('Storage').write('setting.json',s)))(require('Storage').readJSON('setting.json',1))"
    )
}

// ==============================================================================
// This computer, as the dev kit's phone
// ==============================================================================

/// What stands around the dev kit: this computer as its phone, and as a
/// BTHome thermometer. Kept from one run to the next (`--fuzz`).
pub struct Rig {
    address: Option<String>,
    peer: Option<Peer>,
    advertiser: Option<Advertiser>,
}

impl Rig {
    pub fn new(address: Option<String>) -> Self {
        Self {
            address,
            peer: None,
            advertiser: None,
        }
    }

    /// The phone, started at its first use.
    fn peer(&mut self) -> Result<&mut Peer, String> {
        if self.peer.is_none() {
            let address = self.address.as_deref().ok_or(
                "this needs this computer as the dev kit's phone: pair them, and pass \
                 --peer ADDRESS (the dev kit's Bluetooth address)",
            )?;
            self.peer = Some(Peer::start(address)?);
        }
        Ok(self.peer.as_mut().expect("started above"))
    }

    /// Connects the phone, in a process of its own if its last link went
    /// down: a second connection in the same process hears every
    /// notification twice, which reads as the dev kit sending everything
    /// twice.
    fn connect(&mut self) -> Result<(), String> {
        let mut heard = Vec::new();
        if let Some(peer) = &mut self.peer {
            peer.poll();
            if peer.dropped {
                // What it heard before the link went down is still to be
                // taken (see `Follower::take_heard`).
                heard = std::mem::take(&mut peer.heard);
                self.peer = None;
            }
        }
        let peer = self.peer()?;
        heard.append(&mut peer.heard);
        peer.heard = heard;
        peer.request("connect").map(drop)
    }

    /// Back as a run starts: no thermometer, no phone connected.
    fn tidy(&mut self) -> Result<(), String> {
        self.advertiser = None;
        if let Some(peer) = &mut self.peer {
            peer.request("disconnect")?;
        }
        Ok(())
    }
}

/// `scripts/gb-peer.py`, Gadgetbridge's stand-in, as a child process (see
/// its docs for what it's told and says).
struct Peer {
    child: Child,
    stdin: ChildStdin,
    lines: mpsc::Receiver<String>,
    /// What the watch sent, not yet a whole line.
    partial: Vec<u8>,
    /// The lines the watch sent, since they were last taken.
    received: Vec<String>,
    /// The same lines, as the watch sent them (its bytes, line feed and
    /// all), for a model of Gadgetbridge to hear (see
    /// [`Follower::take_heard`]): apart, as `received` is taken at every
    /// `compare`, and lossily (as UTF-8).
    heard: Vec<Vec<u8>>,
    /// The link went down since the process started (see
    /// `scripts/gb-peer.py`'s `dropped`).
    dropped: bool,
    /// What it was last asked (its verb), until it's answered, and by
    /// when.
    asked: Option<(String, StdInstant)>,
}

impl Peer {
    fn start(address: &str) -> Result<Self, String> {
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/gb-peer.py");
        let mut child = Command::new("uv")
            .args(["run", "--quiet", "--with", "bleak", "python"])
            .arg(&script)
            .arg(address)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .map_err(|e| format!("running {} (with uv): {e}", script.display()))?;
        let stdin = child.stdin.take().expect("piped");
        let stdout = child.stdout.take().expect("piped");
        let (send, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if send.send(line).is_err() {
                    break;
                }
            }
        });
        Ok(Self {
            child,
            stdin,
            lines,
            partial: Vec::new(),
            received: Vec::new(),
            heard: Vec::new(),
            dropped: false,
            asked: None,
        })
    }

    /// Tells it `command`, and waits for its answer, taking in what the
    /// watch sends meanwhile.
    fn request(&mut self, command: &str) -> Result<(), String> {
        self.begin(command)?;
        loop {
            if self.answer(StdDuration::from_secs(1))?.is_some() {
                return Ok(());
            }
        }
    }

    /// Tells it `command`, for [`Peer::answer`] to wait for its answer.
    fn begin(&mut self, command: &str) -> Result<(), String> {
        writeln!(self.stdin, "{command}").map_err(|e| format!("the phone's stand-in: {e}"))?;
        let verb = command.split(' ').next().unwrap_or_default();
        // Connecting may scan for ten seconds, then take thirty.
        self.asked = Some((
            verb.to_owned(),
            StdInstant::now() + StdDuration::from_secs(60),
        ));
        Ok(())
    }

    /// Waits up to `wait` for the answer to what [`Peer::begin`] asked:
    /// `Some` once it's done.
    fn answer(&mut self, wait: StdDuration) -> Result<Option<()>, String> {
        let (verb, deadline) = self.asked.clone().ok_or("nothing was asked")?;
        let until = (StdInstant::now() + wait).min(deadline);
        loop {
            let left = until.saturating_duration_since(StdInstant::now());
            let line = match self.lines.recv_timeout(left) {
                Ok(line) => line,
                Err(_) if StdInstant::now() < deadline => return Ok(None),
                Err(_) => return Err(format!("the phone's stand-in didn't answer {verb}")),
            };
            if line == "ok" {
                self.asked = None;
                return Ok(Some(()));
            }
            if let Some(why) = line.strip_prefix("error ") {
                self.asked = None;
                return Err(format!("the phone's stand-in, at {verb}: {why}"));
            }
            self.take(&line);
        }
    }

    /// Takes in what it said since.
    fn poll(&mut self) {
        while let Ok(line) = self.lines.try_recv() {
            self.take(&line);
        }
    }

    fn take(&mut self, line: &str) {
        if let Some(hex) = line.strip_prefix("rx ") {
            let bytes = (0..hex.len() / 2)
                .filter_map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).ok());
            self.partial.extend(bytes);
            while let Some(end) = self.partial.iter().position(|&b| b == b'\n') {
                let bytes: Vec<u8> = self.partial.drain(..=end).collect();
                let line = String::from_utf8_lossy(&bytes).trim_end().to_owned();
                eprintln!("  phone: got {line}");
                self.received.push(line);
                self.heard.push(bytes);
            }
        } else if line == "dropped" {
            eprintln!("  phone: the dev kit disconnected");
            self.dropped = true;
        } else {
            eprintln!("  phone: {line}");
        }
    }
}

impl Drop for Peer {
    fn drop(&mut self) {
        // Disconnects as it goes; if it won't, it's stopped.
        let _ = writeln!(self.stdin, "quit");
        let deadline = StdInstant::now() + StdDuration::from_secs(10);
        while StdInstant::now() < deadline {
            if let Ok(Some(_)) = self.child.try_wait() {
                return;
            }
            std::thread::sleep(StdDuration::from_millis(100));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// `scripts/bthome-advertise.sh`: this computer advertising as a BTHome
/// thermometer, until dropped.
struct Advertiser(Child);

impl Advertiser {
    fn start(hundredths: i16) -> Result<Self, String> {
        use std::os::unix::process::CommandExt;
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/bthome-advertise.sh");
        Command::new("bash")
            .arg(&script)
            .arg(hundredths.to_string())
            .stdout(Stdio::null())
            // Its own process group, so it and its `bluetoothctl` stop
            // together.
            .process_group(0)
            .spawn()
            .map(Self)
            .map_err(|e| format!("running {}: {e}", script.display()))
    }
}

impl Drop for Advertiser {
    fn drop(&mut self) {
        let group = format!("-{}", self.0.id());
        let _ = Command::new("kill").args(["-TERM", "--", &group]).status();
        let _ = self.0.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The follower reads the dev kit's `Export` bytes straight into its
    /// own: the layout must be the watch's. (On the watch: `llvm-nm -S`
    /// on a `compare` build gives `UR_COMPARE`'s size.)
    #[test]
    fn the_export_is_laid_out_as_on_the_watch() {
        assert_eq!(size_of::<Export>(), 0x990);
        assert_eq!(size_of::<Event>(), 32);
        assert_eq!(size_of::<Music>(), 84);
        let music = std::mem::offset_of!(Export, music);
        assert_eq!(
            std::mem::offset_of!(Export, events),
            music - compare::EVENTS * size_of::<Event>()
        );
        assert_eq!(music + size_of::<Music>(), 0x990 - 4, "and padding");
    }
}
