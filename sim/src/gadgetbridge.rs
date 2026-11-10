//! The simulator with a Gadgetbridge at the other end: `gbmock`, a model
//! of its Bangle.js support (see that crate for the commit it's from).
//!
//! `fuzz.rs` writes its `gb` lines by hand, a dozen shapes of them, and
//! nothing reads what the watch answers. Here the mock decides what a
//! phone sends (everything Gadgetbridge can, spelled as it spells it), and
//! takes every line the watch sends back as Gadgetbridge would:
//!
//! - whatever it sends, in any order, the firmware's UI takes without a
//!   panic (overflows and `debug_assert!`s included) or a step that never
//!   ends;
//! - whatever the watch sends, Gadgetbridge takes without a complaint: no
//!   "Malformed JSON" toast, no exception, nothing it can't place;
//! - and what a fetch leaves in its database is a sample a bucket.
//!
//! [`host_one`] is one input of a coverage-guided fuzzer
//! (`fuzz/fuzz_targets/gadgetbridge.rs`): its bytes choose what happens,
//! through `gbmock::generate`. It's a second target beside `steps`, not
//! a change to that one's generator: `steps`' corpus is tied to how its
//! bytes read, and its scripts also run on a dev kit, over a real link.
//!
//! The same inputs also run on a dev kit (`ur-sim --fuzz ELF --peer
//! ADDRESS --corpus DIR --gadgetbridge`, in `fuzz.rs`): [`drive`] is the
//! session, and a [`Runner`] is where its lines go and its answers come
//! from, the simulator alone or a dev kit the simulator follows. On a dev
//! kit, the mock hears what the dev kit itself sent over the air, so a
//! line real Gadgetbridge would refuse is a finding there too, beside the
//! frames and the phone's messages the follower compares.
//!
//! An input isn't a script: the mock answers what the watch says (a
//! button's intent refused, a fetch's end), so the lines it sends depend
//! on the watch's. [`drive`] keeps what it ran, as a script, for a repro.

use std::path::Path;

use gbmock::generate::{Action, Choices};
use gbmock::{Event, Gadgetbridge};

use crate::flash::Flash;
use crate::script::{self, Output};
use crate::watch::{self, Watch};

/// At most this many steps an input: a step is a few milliseconds, and
/// the fuzzer's timeout is for a UI that spins, not a long script.
const STEPS: usize = 300;

/// The longest a step's wait is on the simulator alone: no more than half
/// an hour a step, so that an input's worth of steps stays quick.
const HOST_WAIT_MS: i64 = 30 * 60_000;

/// One input: a watch booted from erased flash, a Gadgetbridge that
/// connects to it, and then whatever `bytes` say, a step at a time: the
/// phone does something, time passes, or the wearer touches the watch.
/// After every step, what the watch sent goes to Gadgetbridge.
///
/// Panics if the simulator does, if a script line fails, or if
/// Gadgetbridge didn't understand the watch.
pub fn host_one(bytes: &[u8], scratch: &Path) {
    let mut simulator = Simulator::boot(scratch, false);
    let run = drive(bytes, &mut simulator);
    if let Some((_, why)) = &run.failure {
        panic!("{why}\n--- the script:\n{}", run.story());
    }
    watch::capture_to_phone(false);
}

/// The session `bytes` make as a dev kit would run it (`--fuzz --corpus
/// DIR --gadgetbridge`: waits cut short, no steps or heart rates, a frame
/// compared each step), on the simulator alone: its script, or why it
/// failed, and the script up to there. What `--dry --gadgetbridge` runs.
pub fn devkit_script(bytes: &[u8], scratch: &Path) -> Result<String, String> {
    let mut simulator = Simulator::boot(scratch, true);
    let run = drive(bytes, &mut simulator);
    watch::capture_to_phone(false);
    match &run.failure {
        None => Ok(run.story()),
        Some((at, why)) => Err(format!(
            "step {at}: {why}\n--- the script:\n{}",
            run.story()
        )),
    }
}

// ==============================================================================
// Where the lines run
// ==============================================================================

/// Where an input's lines run, and what the watch sends back.
pub(crate) trait Runner {
    /// The simulator alone, as the fuzzer runs it: what a dev kit can't be
    /// made to do (a heart rate, steps) goes in as is, and waits are the
    /// phone's. Otherwise (a dev kit) those lines are comments, and waits
    /// are cut short (see [`Runner::wait_ms`]).
    fn alone(&self) -> bool;

    /// How long, here, a wait of `ms` on the phone's clock (already at
    /// most [`HOST_WAIT_MS`]) is.
    fn wait_ms(&self, ms: i64) -> i64;

    /// Runs a line of the script.
    fn run(&mut self, line: &str) -> Result<(), String>;

    /// What the watch sent the phone since the last call: each line's
    /// bytes, as sent, its line end and all.
    fn to_phone(&mut self) -> Result<Vec<Vec<u8>>, String>;

    /// The lines that end step `n`: none on the simulator alone; a frame
    /// compared on a dev kit.
    fn step_end(&mut self, n: usize) -> Vec<String>;

    /// Whether something's been found (a dev kit's frame that differs):
    /// the session stops after the step that found it.
    fn found(&self) -> bool;
}

/// The simulator, booted from erased flash in `scratch`.
pub(crate) struct Simulator {
    watch: Watch,
    output: Output,
    /// As a dev kit's script would have it (`--dry`): see
    /// [`Runner::alone`].
    as_devkit: bool,
}

impl Simulator {
    pub(crate) fn boot(scratch: &Path, as_devkit: bool) -> Self {
        std::fs::create_dir_all(scratch).expect("the scratch directory is writable");
        let path = scratch.join("gadgetbridge-storage.bin");
        std::fs::write(&path, Flash::erased().bytes()).expect("the scratch directory is writable");
        let watch = Watch::boot(Flash::open(&path).expect("it was just written"));
        watch::capture_to_phone(true);
        Self {
            watch,
            output: Output {
                dir: scratch.to_owned(),
                zoom: 1,
            },
            as_devkit,
        }
    }
}

impl Runner for Simulator {
    fn alone(&self) -> bool {
        !self.as_devkit
    }

    fn wait_ms(&self, ms: i64) -> i64 {
        if self.as_devkit {
            ms.min(DEVKIT_WAIT_MS)
        } else {
            ms
        }
    }

    fn run(&mut self, line: &str) -> Result<(), String> {
        // What only a dev kit does is left out, as `fuzz.rs`'s `--dry`
        // does.
        let command = line.split_whitespace().next().unwrap_or_default();
        if self.as_devkit && matches!(command, "compare" | "check-storage") {
            return Ok(());
        }
        script::execute(&mut self.watch, line, &self.output).map(drop)
    }

    fn to_phone(&mut self) -> Result<Vec<Vec<u8>>, String> {
        // Without their line ends, as kept; the firmware's are `\r\n`.
        Ok(watch::take_to_phone()
            .into_iter()
            .map(|line| format!("{line}\r\n").into_bytes())
            .collect())
    }

    fn step_end(&mut self, n: usize) -> Vec<String> {
        if self.as_devkit {
            vec![compare_line(n)]
        } else {
            Vec::new()
        }
    }

    fn found(&self) -> bool {
        false
    }
}

/// Step `n`'s frame compared, on a dev kit: `compare gNNN`, `g` for a
/// Gadgetbridge session's (a `steps` run's are `sNNN`).
pub(crate) fn compare_line(n: usize) -> String {
    format!("compare g{n:03}")
}

/// The longest a step's wait is on a dev kit, where it's real time: long
/// enough for the screen to time out, and short enough for a night's worth
/// of inputs. The phone's clock (the mock's) still moves the whole wait,
/// so the mock does as it would on the simulator; the dev kit's clock
/// falls behind it, until the mock next sets the time.
pub(crate) const DEVKIT_WAIT_MS: i64 = 20_000;

// ==============================================================================
// The session
// ==============================================================================

/// What a session ran: its steps, each its lines (what the watch sent as
/// comments), and what went wrong, if anything: at which step, and why.
pub(crate) struct Run {
    pub steps: Vec<Vec<String>>,
    pub failure: Option<(usize, String)>,
}

impl Run {
    /// Its lines, one after the other: for a finding's message.
    pub(crate) fn story(&self) -> String {
        let lines: Vec<&str> = self.steps.iter().flatten().map(String::as_str).collect();
        lines.join("\n")
    }
}

/// The session `bytes` make, on `runner`: Gadgetbridge connects, and then
/// a step at a time, the phone does something, time passes, or the wearer
/// touches the watch. After every step, what the watch sent goes to
/// Gadgetbridge, which mustn't complain of it.
pub(crate) fn drive(bytes: &[u8], runner: &mut impl Runner) -> Run {
    let mut session = Session {
        gb: Gadgetbridge::new(),
        run: Run {
            steps: Vec::new(),
            failure: None,
        },
    };
    let _ = session.steps(bytes, runner);
    session.run
}

struct Session {
    gb: Gadgetbridge,
    run: Run,
}

/// Stopped: the failure (or the finding) is in the [`Run`].
struct Stop;

impl Session {
    fn steps(&mut self, bytes: &[u8], runner: &mut impl Runner) -> Result<(), Stop> {
        let mut choices = Choices::new(bytes);
        let mut actions = gbmock::generate::connect_sequence(&mut choices);
        actions.reverse();
        for _ in 0..STEPS {
            let action = match actions.pop() {
                Some(action) => action,
                None if choices.is_empty() => break,
                None => match choices.byte() % 8 {
                    0 => {
                        self.run.steps.push(Vec::new());
                        self.wearer(&mut choices, runner)?;
                        self.end_step(runner)?;
                        continue;
                    }
                    // The phone.
                    _ => gbmock::generate::action(&mut choices),
                },
            };
            self.run.steps.push(Vec::new());
            self.phone(&action, runner)?;
            self.end_step(runner)?;
        }
        // On a dev kit, its answers to the last steps are still on the
        // air: a moment for them, and what they are.
        if !runner.alone() {
            self.run.steps.push(Vec::new());
            self.line(runner, "wait 2s".into())?;
            self.exchange(runner)?;
            self.end_step(runner)?;
        }
        Ok(())
    }

    /// What the wearer does: on the dev kit, what can be done to one
    /// (steps and a heart rate only the simulator can be told, so they're
    /// comments there; the choices that made them still count).
    fn wearer(&mut self, choices: &mut Choices<'_>, runner: &mut impl Runner) -> Result<(), Stop> {
        let (x, y) = (choices.below(80), choices.below(80));
        let line = match choices.byte() % 12 {
            0 | 1 => "button".to_owned(),
            2 => "button 2".to_owned(),
            3 | 4 => format!("tap {x},{y}"),
            5 => format!("long {x},{y}"),
            6 => "up".to_owned(),
            7 => "down".to_owned(),
            8 => "left".to_owned(),
            9 => "right".to_owned(),
            10 => format!("steps {}", choices.below(30_000)),
            _ => format!("heart {}", 40 + choices.below(140)),
        };
        let host_only = line.starts_with("steps ") || line.starts_with("heart ");
        if host_only && !runner.alone() {
            self.comment(format!("host only: {line}"));
            return Ok(());
        }
        self.line(runner, line)
    }

    /// What the phone does, and the lines it and the watch then trade.
    fn phone(&mut self, action: &Action, runner: &mut impl Runner) -> Result<(), Stop> {
        match action {
            // Time passes for both: the watch's clock is the phone's,
            // since it connected.
            Action::Wait { ms } => {
                let ms = (*ms).clamp(0, HOST_WAIT_MS);
                self.gb.now_ms += ms;
                let here = runner.wait_ms(ms);
                if here != ms {
                    self.comment(format!("the phone's clock moves {ms} ms"));
                }
                self.line(runner, format!("wait {here}ms"))?;
            }
            Action::Connect => {
                self.line(runner, "connect".into())?;
                self.gb.connect();
            }
            Action::Disconnect => {
                self.line(runner, "disconnect".into())?;
                self.gb.disconnect();
            }
            action => self.gb.perform(action),
        }
        self.exchange(runner)?;
        if self.gb.has_partial_line() {
            return self.fail("half a line left in Gadgetbridge".into());
        }
        // A sample a bucket, each at its bucket's last second (see
        // `gb_buckets`): so none within the two minutes Gadgetbridge
        // merges. (Up to 2038, that is: Gadgetbridge files samples
        // under a Java `int` of seconds, and a phone whose clock is set
        // past it has the watch send what wraps to before 1970. The
        // fuzzer found that in minutes; no phone will for a while.)
        let bucket = crate::gb_buckets::MINUTES * 60;
        let off = (self.gb.samples.keys())
            .filter(|timestamp| **timestamp >= 0)
            .find(|timestamp| i64::from(**timestamp).rem_euclid(bucket) != bucket - 1);
        if let Some(timestamp) = off {
            return self.fail(format!("a sample off its bucket's end: {timestamp}"));
        }
        Ok(())
    }

    /// Gadgetbridge's lines to the watch, and the watch's back, until
    /// neither has more to say (an answer may get one: a button's intent
    /// refused).
    fn exchange(&mut self, runner: &mut impl Runner) -> Result<(), Stop> {
        for _ in 0..4 {
            for sent in self.gb.take_sent() {
                // As a script line: the bytes between the DLE and the
                // line feed, a character each.
                let line: String = sent
                    .bytes
                    .strip_prefix(b"\x10")
                    .unwrap_or(&sent.bytes)
                    .iter()
                    .map(|&b| char::from(b))
                    .collect();
                let line = line.strip_suffix('\n').unwrap_or(&line);
                // A script line is one line, and a blank one is nothing
                // (only Tasker's lines can be either).
                if line.contains(['\n', '\r']) || line.trim().is_empty() {
                    continue;
                }
                self.line(runner, format!("gb {line}"))?;
            }
            let lines = match runner.to_phone() {
                Ok(lines) => lines,
                Err(e) => return self.fail(e),
            };
            if lines.is_empty() {
                break;
            }
            for line in lines {
                let shown = String::from_utf8_lossy(&line);
                let shown = shown.strip_suffix('\n').unwrap_or(&shown);
                let shown = shown.strip_suffix('\r').unwrap_or(shown);
                self.comment(format!("to the phone: {shown}"));
                // In the writes `ble` makes: 20 bytes at worst.
                for packet in line.chunks(20) {
                    self.gb.receive(packet);
                }
            }
            for event in self.gb.take_events() {
                if matches!(
                    event,
                    Event::MalformedJson(_)
                        | Event::Uncaught(_)
                        | Event::IgnoredLine(_)
                        | Event::NotUnderstood(_)
                ) {
                    return self.fail(format!("Gadgetbridge: {event:?}"));
                }
            }
        }
        Ok(())
    }

    /// The lines that end a step, and whether to go on.
    fn end_step(&mut self, runner: &mut impl Runner) -> Result<(), Stop> {
        let n = self.run.steps.len() - 1;
        for line in runner.step_end(n) {
            self.line(runner, line)?;
        }
        match runner.found() {
            true => Err(Stop),
            false => Ok(()),
        }
    }

    fn line(&mut self, runner: &mut impl Runner, line: String) -> Result<(), Stop> {
        let result = runner.run(&line);
        self.step().push(line);
        match result {
            Ok(()) => Ok(()),
            Err(e) => self.fail(e),
        }
    }

    fn comment(&mut self, text: String) {
        self.step().push(format!("# {text}"));
    }

    fn step(&mut self) -> &mut Vec<String> {
        self.run.steps.last_mut().expect("a step under way")
    }

    fn fail<T>(&mut self, why: String) -> Result<T, Stop> {
        let at = self.run.steps.len().saturating_sub(1);
        self.run.failure = Some((at, why));
        Err(Stop)
    }
}
