//! Fuzzing the simulator against a dev kit: `ur-sim --fuzz ELF`.
//!
//! `--devkit` checks the simulator against a dev kit along a script someone
//! wrote. This writes the scripts: random ones, from a seed, of what a dev
//! kit can be made to do (touches, the button, waits, test notifications,
//! the battery, and, with this computer as its phone,
//! Gadgetbridge's lines, connecting and disconnecting, and a BTHome
//! thermometer), each run starting from random settings in its storage. The
//! follower (`devkit.rs`) runs each, and checks every step: the frame, what
//! the dev kit's UI got from the phone and sent it, and, now and then, what
//! it saved and boots with.
//!
//! A run that finds a difference is shrunk: cut after the step that showed
//! it, then steps taken out while it still shows (delta debugging), each
//! try a fresh run from the same storage. What's left, a script and a
//! storage image, is the repro, under `--out`, for `--devkit --storage`.
//!
//! The dev kit's storage is read first, kept as `original-storage.bin`,
//! and written back at the end. The runs start from it without its bonds
//! with anything but this computer, so the phone it's paired with stays
//! out of them.
//!
//! `--dry` writes the scripts and runs them on the simulator alone, without
//! a dev kit: a check of the generator (every line one the simulator
//! takes), not of the simulator.
//!
//! Not checked yet (TODO): the simulator's own models where the follower
//! replays the dev kit's answer instead. A finger's path (`press` …
//! `lift`) through the simulator's gesture recognition (`gesture::Tracking`)
//! against the gesture the dev kit logged; buzzes (`motor::take_request`)
//! against the dev kit's motor, which `compare` doesn't export; and the
//! button grid's BTHome adverts on the air, which this computer could scan
//! for (the presses the dev kit's UI hands its radio are checked). Nor
//! the grid's presses with both Bluetooth connections busy (BTHOME NOT
//! SENT), which takes a second central beside this computer.

use std::fmt::Write as _;
use std::path::Path;

use crate::devkit::{self, Finding, Setup};
use crate::flash::Flash;
use crate::storage::{self, key};
use crate::{alarm, theme};

/// What `--fuzz` was asked for.
pub struct Fuzz {
    pub seed: u64,
    pub runs: u64,
    pub steps: usize,
    pub dry: bool,
}

pub fn run(elf: &Path, out: &Path, setup: &Setup, fuzz: &Fuzz) -> Result<(), String> {
    std::fs::create_dir_all(out).map_err(|e| format!("creating {}: {e}", out.display()))?;
    if fuzz.dry {
        return dry(out, setup, fuzz);
    }
    let mut bench = devkit::Bench::open(elf, setup.peer.clone())?;
    // The dev kit's own storage, to start from and put back. Kept if it's
    // there already: a fuzz run that stopped half way left the dev kit's
    // storage as its last run made it.
    let original_path = out.join("original-storage.bin");
    let original = match std::fs::read(&original_path) {
        Ok(bytes) => {
            eprintln!(
                "fuzz: starting from {} (an earlier run's), not the dev kit's storage now",
                original_path.display()
            );
            bytes
        }
        Err(_) => {
            let bytes = bench.read_storage()?;
            write(&original_path, &bytes)?;
            bytes
        }
    };
    // What's put back at the end, and what the runs start from: less what
    // the firmware can't read.
    let original = without_stale(&original, &out.join("scratch"))?;
    // The runs start from it less its bonds with anything but this
    // computer: the phone that's paired with the dev kit would otherwise
    // connect when it pleased, and send what it pleased (its media, say).
    let base = only_this_computer(&original, &out.join("scratch"))?;
    let mut found = 0;
    let began = std::time::Instant::now();
    let mut shrinking = std::time::Duration::ZERO;
    let result = (|| {
        for n in 0..fuzz.runs {
            let seed = fuzz.seed.wrapping_add(n);
            let mut rng = Rng::new(seed);
            let storage = random_storage(&base, &mut rng, &out.join("scratch"))?;
            let steps = generate(&mut rng, fuzz.steps, setup.peer.is_some());
            let dir = out.join(format!("run-{seed}"));
            save_repro(&dir, seed, &steps, &storage, &[])?;
            eprintln!("fuzz: run {} of {} (seed {seed})", n + 1, fuzz.runs);
            let (started, spent) = (std::time::Instant::now(), bench.spent());
            let outcome = trial(&mut bench, &storage, &steps, &dir);
            report_time(&format!("seed {seed}"), started, spent, bench.spent());
            let Some((at, kind)) = outcome.first() else {
                continue;
            };
            found += 1;
            for line in outcome.report() {
                eprintln!("fuzz: seed {seed}: {line}");
            }
            eprintln!("fuzz: seed {seed}: step {at}: {kind}; shrinking");
            let (started, spent) = (std::time::Instant::now(), bench.spent());
            // From the last reboot before it, if there was one: what came
            // before it is only in that storage. Failing that, from the
            // run's start.
            let reboot = (outcome.reboots.iter()).rfind(|(after, _)| *after < at);
            let (mut minimal, mut shrunk) = (Vec::new(), None);
            let mut from = &storage;
            if let Some((after, rebooted)) = reboot {
                eprintln!("fuzz: seed {seed}: from the reboot after step {after}");
                let (steps, outcome) =
                    shrink(&mut bench, rebooted, &steps[after + 1..=at], &kind, &dir);
                if outcome.shows(&kind) {
                    (minimal, shrunk, from) = (steps, Some(outcome), rebooted);
                }
            }
            let (minimal, outcome) = match shrunk {
                Some(outcome) => (minimal, outcome),
                None => shrink(&mut bench, &storage, &steps[..=at], &kind, &dir),
            };
            report_time(
                &format!("seed {seed}'s shrinking"),
                started,
                spent,
                bench.spent(),
            );
            shrinking += started.elapsed();
            let repro = out.join(format!("finding-{seed}"));
            save_repro(&repro, seed, &minimal, from, &outcome.report())?;
            eprintln!(
                "fuzz: seed {seed}: {} steps of {} still show it: {}",
                minimal.len(),
                steps.len(),
                repro.display()
            );
        }
        Ok::<(), String>(())
    })();
    eprintln!("fuzz: putting the dev kit's storage back");
    let restored = bench.restore(&original, &out.join("scratch"));
    let spent = bench.spent();
    eprintln!(
        "fuzz: {:.0} s in all ({:.0} s shrinking): {} storage writes, {:.1} s; \
         {} resets, {:.1} s",
        began.elapsed().as_secs_f64(),
        shrinking.as_secs_f64(),
        spent.writes,
        spent.writing.as_secs_f64(),
        spent.resets,
        spent.resetting.as_secs_f64()
    );
    result?;
    restored?;
    if found == 0 {
        eprintln!("fuzz: {} runs, no differences", fuzz.runs);
        Ok(())
    } else {
        Err(format!(
            "{found} of {} runs found differences (see {})",
            fuzz.runs,
            out.display()
        ))
    }
}

// ==============================================================================
// Running and shrinking
// ==============================================================================

/// A step: an input (or a few, for a finger's path), perhaps a wait, and a
/// `compare`.
type Step = Vec<String>;

/// What a run found: at which step, and what (each `Finding`, or the error
/// that stopped it).
#[derive(Default)]
struct Outcome {
    findings: Vec<(usize, Finding)>,
    error: Option<(usize, String)>,
    /// After which steps a `reset` rebooted the dev kit, and with what
    /// storage.
    reboots: Vec<(usize, Vec<u8>)>,
}

impl Outcome {
    /// The first thing found, and its kind (for the shrinking to keep).
    fn first(&self) -> Option<(usize, String)> {
        let finding = self.findings.first().map(|(at, f)| (*at, kind(&f.name)));
        let error = (self.error.as_ref()).map(|(at, error)| (*at, error_kind(error)));
        match (finding, error) {
            (Some(f), Some(e)) => Some(if e.0 < f.0 { e } else { f }),
            (f, e) => f.or(e),
        }
    }

    fn shows(&self, kind_wanted: &str) -> bool {
        self.findings
            .iter()
            .any(|(_, f)| kind(&f.name) == kind_wanted)
            || (self.error.as_ref()).is_some_and(|(_, error)| error_kind(error) == kind_wanted)
    }

    fn report(&self) -> Vec<String> {
        let mut lines: Vec<String> = (self.findings.iter())
            .map(|(at, f)| format!("step {at}: {}: {}", f.name, f.what))
            .collect();
        if let Some((at, error)) = &self.error {
            lines.push(format!("step {at}: stopped: {}", error.replace('\n', ": ")));
        }
        lines
    }
}

/// What kind of error an error (`LINE\nWHY`) is: why, without its numbers,
/// so that the shrinking keeps the same error (the dev kit's events
/// outrunning the follower, say), and not any (a step it needed taken out).
fn error_kind(error: &str) -> String {
    let why = error.rsplit('\n').next().unwrap_or(error);
    let why: String = why.chars().filter(|c| !c.is_ascii_digit()).collect();
    format!("error: {why}")
}

/// What kind of difference a finding is, by its name (see `devkit.rs`): a
/// frame's (named for its step), or the phone's, or the storage's.
fn kind(name: &str) -> String {
    for suffix in ["-phone-in", "-phone-out", "-ha"] {
        if name.ends_with(suffix) {
            return suffix[1..].to_owned();
        }
    }
    match name {
        "reset" | "storage" => name.to_owned(),
        _ => "frame".to_owned(),
    }
}

/// How long something took, and how much of that was the dev kit's
/// storage writes and resets (from `before` to `after`).
fn report_time(
    what: &str,
    started: std::time::Instant,
    before: devkit::Spent,
    after: devkit::Spent,
) {
    eprintln!(
        "fuzz: {what}: {:.1} s: {} storage writes, {:.1} s; {} resets, {:.1} s",
        started.elapsed().as_secs_f64(),
        after.writes - before.writes,
        (after.writing - before.writing).as_secs_f64(),
        after.resets - before.resets,
        (after.resetting - before.resetting).as_secs_f64()
    );
}

/// One run of `steps`, from `storage`; stops at the first error.
fn trial(bench: &mut devkit::Bench, storage: &[u8], steps: &[Step], dir: &Path) -> Outcome {
    let mut outcome = Outcome::default();
    let mut follower = match bench.follow(storage, dir) {
        Ok(follower) => follower,
        Err(e) => {
            outcome.error = Some((0, e));
            return outcome;
        }
    };
    for (at, step) in steps.iter().enumerate() {
        for line in step {
            match follower.execute(line, dir) {
                Ok(findings) => outcome
                    .findings
                    .extend(findings.into_iter().map(|f| (at, f))),
                Err(e) => {
                    outcome.error = Some((at, format!("{line}\n{e}")));
                    return outcome;
                }
            }
        }
        if let Some(storage) = follower.take_rebooted() {
            outcome.reboots.push((at, storage));
        }
        if !outcome.findings.is_empty() {
            break;
        }
    }
    outcome
}

/// At most this many runs to shrink one finding, and for at most this
/// long: each is a reset and a storage write, and the steps in real time.
const SHRINK_TRIES: usize = 30;
const SHRINK_TIME: std::time::Duration = std::time::Duration::from_secs(20 * 60);

/// The fewest of `steps` (whose last showed `kind`) that still show it:
/// delta debugging (Zeller's ddmin) over all but the last, which stays.
fn shrink(
    bench: &mut devkit::Bench,
    storage: &[u8],
    steps: &[Step],
    kind_wanted: &str,
    dir: &Path,
) -> (Vec<Step>, Outcome) {
    let tries = std::cell::Cell::new(0);
    let began = std::time::Instant::now();
    let spent = || tries.get() >= SHRINK_TRIES || began.elapsed() > SHRINK_TIME;
    let attempt = |bench: &mut devkit::Bench, candidate: &[Step]| {
        tries.set(tries.get() + 1);
        eprintln!(
            "fuzz: shrinking: try {}, {} steps",
            tries.get(),
            candidate.len()
        );
        trial(bench, storage, candidate, &dir.join("shrinking"))
    };
    let (last, mut rest) = steps
        .split_last()
        .map(|(l, r)| (l.clone(), r.to_vec()))
        .expect("a step");
    // First, the last few steps alone, then twice as many, and so on: most
    // differences need only what came just before them, and a try costs
    // its steps in real time, so the short ones come cheap. (Bounded by
    // twice the steps of the one that shows.) One that shows has shown it
    // can be shown again.
    let mut best_outcome = None;
    let mut tail = 4;
    while tail < rest.len() && !spent() {
        let mut candidate = rest[rest.len() - tail..].to_vec();
        candidate.push(last.clone());
        let outcome = attempt(bench, &candidate);
        if outcome.shows(kind_wanted) {
            candidate.pop();
            rest = candidate;
            best_outcome = Some(outcome);
            break;
        }
        tail *= 2;
    }
    // Failing that, it must show again from the start, or it's not one to
    // shrink (it may depend on timing): keep the whole thing.
    let mut best_outcome = match best_outcome {
        Some(outcome) => outcome,
        None => {
            let outcome = attempt(bench, steps);
            if !outcome.shows(kind_wanted) {
                eprintln!("fuzz: it didn't show again; keeping the run as it was");
                return (steps.to_vec(), outcome);
            }
            outcome
        }
    };
    // Then delta debugging: halves out, then quarters, and so on.
    let mut chunks = 2;
    'outer: while !rest.is_empty() && !spent() {
        let size = rest.len().div_ceil(chunks);
        for start in (0..rest.len()).step_by(size) {
            if spent() {
                break 'outer;
            }
            let mut candidate: Vec<Step> = rest[..start].to_vec();
            candidate.extend_from_slice(&rest[(start + size).min(rest.len())..]);
            candidate.push(last.clone());
            let outcome = attempt(bench, &candidate);
            if outcome.shows(kind_wanted) {
                candidate.pop();
                rest = candidate;
                best_outcome = outcome;
                chunks = (chunks - 1).max(2);
                continue 'outer;
            }
        }
        if size == 1 {
            break;
        }
        chunks = (chunks * 2).min(rest.len());
    }
    rest.push(last);
    (rest, best_outcome)
}

/// Writes a run's script and storage (and what it found) to `dir`.
fn save_repro(
    dir: &Path,
    seed: u64,
    steps: &[Step],
    storage: &[u8],
    report: &[String],
) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    let mut script = format!(
        "# ur-sim --fuzz, seed {seed}: run with\n\
         #   ur-sim --devkit ELF --storage {0}/storage.bin [--peer ADDRESS] {0}/script.txt\n",
        dir.display()
    );
    for line in report {
        let _ = writeln!(script, "# {line}");
    }
    for (n, step) in steps.iter().enumerate() {
        let _ = writeln!(script, "\n# step {n}");
        for line in step {
            let _ = writeln!(script, "{line}");
        }
    }
    write(&dir.join("script.txt"), script.as_bytes())?;
    write(&dir.join("storage.bin"), storage)
}

fn write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    std::fs::write(path, bytes).map_err(|e| format!("writing {}: {e}", path.display()))
}

// ==============================================================================
// Without a dev kit
// ==============================================================================

/// The scripts on the simulator alone: each line must be one it takes.
fn dry(out: &Path, setup: &Setup, fuzz: &Fuzz) -> Result<(), String> {
    for n in 0..fuzz.runs {
        let seed = fuzz.seed.wrapping_add(n);
        let mut rng = Rng::new(seed);
        let storage = random_storage(Flash::erased().bytes(), &mut rng, &out.join("scratch"))?;
        let steps = generate(&mut rng, fuzz.steps, setup.peer.is_some());
        let dir = out.join(format!("run-{seed}"));
        save_repro(&dir, seed, &steps, &storage, &[])?;
        let flash = Flash::from_bytes(storage).ok_or("a storage image of the wrong size")?;
        let mut watch = crate::watch::Watch::boot(flash);
        let output = crate::script::Output {
            dir: dir.clone(),
            zoom: 1,
        };
        let text: String = steps
            .iter()
            .flatten()
            .map(|line| alone(line) + "\n")
            .collect();
        crate::script::run(&mut watch, &format!("seed {seed}"), &text, &output)?;
    }
    eprintln!(
        "fuzz: {} runs' scripts ran on the simulator alone",
        fuzz.runs
    );
    Ok(())
}

/// A line of a dev kit script as the simulator alone takes it: what only
/// a dev kit does (comparing with it, resetting it, its storage, its test
/// media) is left out, as a comment.
fn alone(line: &str) -> String {
    let command = line.split_whitespace().next().unwrap_or_default();
    match command {
        "compare" | "reset" | "check-storage" | "media" => format!("# {line}"),
        _ => line.to_owned(),
    }
}

// ==============================================================================
// Random storage
// ==============================================================================

/// `original`, with random settings saved over it, through the firmware's
/// own storage, as the watch would have: the rest (the bonds, the BTHome
/// key) as it was.
fn random_storage(original: &[u8], rng: &mut Rng, scratch: &Path) -> Result<Vec<u8>, String> {
    std::fs::create_dir_all(scratch).map_err(|e| format!("creating {}: {e}", scratch.display()))?;
    let path = scratch.join("random-storage.bin");
    write(&path, original)?;
    storage::init(Flash::open(&path)?);
    let mut saves: Vec<(u8, Vec<u8>)> = Vec::new();
    // Mostly what the UI could have saved; now and then what it couldn't
    // (an index past the end, bytes at random), which boot must cope with.
    let byte = |rng: &mut Rng, valid: u64| {
        if rng.chance(0.9) {
            rng.below(valid) as u8
        } else {
            rng.next() as u8
        }
    };
    for (key, valid) in [
        (key::THEME_LIGHT, theme::LIGHT.len() as u64),
        (key::THEME_DARK, theme::DARK.len() as u64),
        (key::BRIGHTNESS, 2),
        (key::BUTTONS, 4),
        (key::GRID, 4),
    ] {
        if rng.chance(0.6) {
            saves.push((key, vec![byte(rng, valid)]));
        }
    }
    if rng.chance(0.6) {
        saves.push((key::ALARMS, random_alarms(rng).to_bytes().to_vec()));
    }
    if rng.chance(0.4) {
        let bytes: Vec<u8> = (0..alarm::Phone::BYTES).map(|_| rng.next() as u8).collect();
        saves.push((key::PHONE_ALARMS, bytes));
    }
    crate::watch::run(
        async {
            for (key, value) in &saves {
                let saved = storage::save(*key, value).await;
                if !saved {
                    eprintln!("fuzz: couldn't save key {key} to the random storage");
                }
            }
        },
        &mut None,
    );
    std::fs::read(&path).map_err(|e| format!("reading {}: {e}", path.display()))
}

/// `original`, less every bond but this computer's (all of them, if
/// `bluetoothctl` can't say what its address is).
fn only_this_computer(original: &[u8], scratch: &Path) -> Result<Vec<u8>, String> {
    let computer = computer_address();
    if computer.is_none() {
        eprintln!("fuzz: this computer's Bluetooth address is unknown; forgetting every bond");
    }
    std::fs::create_dir_all(scratch).map_err(|e| format!("creating {}: {e}", scratch.display()))?;
    let path = scratch.join("base-storage.bin");
    write(&path, original)?;
    storage::init(Flash::open(&path)?);
    crate::watch::run(
        async {
            for slot in key::BONDS {
                // As `ble::encode_bond` stores it: a format byte, the
                // address's kind, then the address, least significant
                // byte first.
                let Some(bond) = storage::load::<BOND_BYTES>(slot).await else {
                    continue;
                };
                let mut address: [u8; 6] = bond[2..8].try_into().expect("six bytes");
                address.reverse();
                if Some(address) == computer {
                    continue;
                }
                let shown: Vec<String> = address.iter().map(|b| format!("{b:02X}")).collect();
                eprintln!(
                    "fuzz: the runs forget the dev kit's bond with {}",
                    shown.join(":")
                );
                if !storage::remove(slot).await {
                    eprintln!("fuzz: couldn't forget the bond in slot {slot}");
                }
            }
        },
        &mut None,
    );
    std::fs::read(&path).map_err(|e| format!("reading {}: {e}", path.display()))
}

/// `original`, less any setting stored shorter than the firmware reads it
/// (a value from a build that used its key for something else: a dev kit's
/// `PHONE_ALARMS` held a 1-byte item, which every boot logged as
/// `BufferTooSmall`). TODO: once `storage::load` removes what it can't
/// read, this is moot.
fn without_stale(original: &[u8], scratch: &Path) -> Result<Vec<u8>, String> {
    std::fs::create_dir_all(scratch).map_err(|e| format!("creating {}: {e}", scratch.display()))?;
    let path = scratch.join("tidied-storage.bin");
    write(&path, original)?;
    storage::init(Flash::open(&path)?);
    crate::watch::run(
        async {
            // A longer item gives its first bytes; a shorter one, nothing.
            // So stale is a byte, but not the whole value. (The one-byte
            // settings can't be too short.)
            for (key, whole) in [
                (
                    key::ALARMS,
                    storage::load::<{ alarm::Settings::BYTES }>(key::ALARMS)
                        .await
                        .is_some(),
                ),
                (
                    key::PHONE_ALARMS,
                    storage::load::<{ alarm::Phone::BYTES }>(key::PHONE_ALARMS)
                        .await
                        .is_some(),
                ),
            ] {
                if !whole && storage::load::<1>(key).await.is_some() {
                    eprintln!("fuzz: dropping key {key} from the storage: too short to read");
                    if !storage::remove(key).await {
                        eprintln!("fuzz: couldn't drop key {key}");
                    }
                }
            }
        },
        &mut None,
    );
    std::fs::read(&path).map_err(|e| format!("reading {}: {e}", path.display()))
}

/// How long a bond is in storage (`ble::BOND_LEN`, private to the
/// firmware's Bluetooth, which the simulator doesn't build).
const BOND_BYTES: usize = 43;

/// This computer's Bluetooth address (its default controller's), most
/// significant byte first, as `bluetoothctl show` says.
fn computer_address() -> Option<[u8; 6]> {
    let output = std::process::Command::new("bluetoothctl")
        .arg("show")
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let address = text
        .lines()
        .find_map(|line| line.strip_prefix("Controller "))?;
    let bytes: Vec<u8> = (address.split_whitespace().next()?.split(':'))
        .map(|byte| u8::from_str_radix(byte, 16).ok())
        .collect::<Option<_>>()?;
    bytes.try_into().ok()
}

fn random_alarms(rng: &mut Rng) -> alarm::Settings {
    let bytes: [u8; alarm::Settings::BYTES] = std::array::from_fn(|_| rng.next() as u8);
    alarm::Settings::from_bytes(bytes)
}

// ==============================================================================
// Random scripts
// ==============================================================================

/// What the generator believes of the dev kit, to say sensible things.
struct Context {
    /// This computer can be its phone.
    phone: bool,
    connected: bool,
    advertising: bool,
    /// A call ringing, to end.
    calling: bool,
    /// The local time last set, in seconds since 1970, and the seconds
    /// of waits since (roughly), for alarms just ahead.
    local: Option<i64>,
    waited: f64,
}

/// `count` random steps; with `phone`, this computer is the dev kit's
/// phone, so they include Bluetooth.
fn generate(rng: &mut Rng, count: usize, phone: bool) -> Vec<Step> {
    let mut context = Context {
        phone,
        connected: false,
        advertising: false,
        calling: false,
        local: None,
        waited: 0.0,
    };
    let mut steps = Vec::new();
    if phone {
        // As Gadgetbridge connects: the time first.
        let mut step = vec!["connect".to_owned()];
        step.push(set_time(rng, &mut context));
        context.connected = true;
        step.push("wait 1s".into());
        step.push("compare s000".into());
        steps.push(step);
    }
    while steps.len() < count {
        let n = steps.len();
        let mut step = action(rng, &mut context);
        // Mostly long enough for a transition to end; now and then not,
        // to catch the frames between.
        if rng.chance(0.7) {
            let ms = rng.range(200, 1500);
            step.push(format!("wait {ms}ms"));
            context.waited += ms as f64 / 1000.0;
        }
        step.push(format!("compare s{n:03}"));
        steps.push(step);
    }
    if let Some(last) = steps.last_mut() {
        last.push("check-storage".into());
    }
    steps
}

/// One random action.
fn action(rng: &mut Rng, context: &mut Context) -> Step {
    let point = |rng: &mut Rng| format!("{},{}", rng.below(80), rng.below(80));
    // (weight, what): the weights sum to whatever they sum to.
    let phone = context.phone;
    let connected = context.connected;
    let choices: &[(u64, u8)] = &[
        (14, 0), // tap
        (12, 1), // swipe
        (4, 2),  // long press
        (6, 3),  // a finger's path
        (9, 4),  // button
        (3, 5),  // double press
        (3, 6),  // glance
        (6, 7),  // a wait of a few seconds
        (4, 8),  // a long wait: the screen times out
        (4, 9),  // test notification
        (2, 10), // test media
        (5, 15), // a grid press
        // No `steps`: they'd go into the dev kit's activity, which
        // Gadgetbridge syncs once the phone's paired again (as
        // devkit-compare.txt). TODO: steps, if the dev kit's activity
        // could be put back afterwards, like its storage.
        (3, 12),                             // battery
        (1, 13),                             // reset
        (1, 14),                             // check storage
        (if phone { 3 } else { 0 }, 20),     // connect or disconnect
        (if connected { 6 } else { 0 }, 21), // notification
        (if connected { 3 } else { 0 }, 22), // call
        (if connected { 3 } else { 0 }, 23), // music
        (if connected { 2 } else { 0 }, 24), // find
        (if connected { 3 } else { 0 }, 25), // time
        (if connected { 2 } else { 0 }, 26), // phone alarms
        (if connected { 2 } else { 0 }, 27), // navigation
        (if connected { 1 } else { 0 }, 28), // weather, location
        (if phone { 2 } else { 0 }, 29),     // thermometer
    ];
    let total: u64 = choices.iter().map(|(w, _)| w).sum();
    let mut pick = rng.below(total);
    let what = choices
        .iter()
        .find(|(weight, _)| {
            if pick < *weight {
                true
            } else {
                pick -= weight;
                false
            }
        })
        .map_or(0, |(_, what)| *what);
    let gb = |line: String| vec![format!("gb {line}")];
    match what {
        0 => {
            let ms = if rng.chance(0.2) {
                format!(" {}", rng.range(20, 255))
            } else {
                String::new()
            };
            vec![format!("tap {}{ms}", point(rng))]
        }
        1 => vec![format!(
            "{} {}",
            rng.pick(&["up", "down", "left", "right"]),
            point(rng)
        )],
        2 => vec![format!("long {}", point(rng))],
        3 => {
            let mut path = vec![format!("press {}", point(rng))];
            for _ in 0..rng.below(3) {
                path.push(format!("wait {}ms", rng.range(50, 400)));
                path.push(format!("press {}", point(rng)));
            }
            let held = rng.range(50, 1500);
            context.waited += held as f64 / 1000.0;
            path.push(format!("wait {held}ms"));
            path.push("lift".into());
            path
        }
        4 => vec!["button".into()],
        5 => vec!["button 2".into()],
        6 => vec!["glance".into()],
        7 => {
            let s = rng.range(2, 8);
            context.waited += s as f64;
            vec![format!("wait {s}s")]
        }
        8 => {
            let s = rng.range(10, 40);
            context.waited += s as f64;
            vec![format!("wait {s}s")]
        }
        9 => {
            let hours = if rng.chance(0.3) {
                format!(" {}", rng.below(30))
            } else {
                String::new()
            };
            vec![format!("notify {}{hours}", rng.below(9))]
        }
        10 => vec![format!(
            "media {}",
            rng.pick(&["playing", "paused", "unknown"])
        )],
        12 => {
            // Never critically low for long: below 3.4 V for a minute, the
            // watch switches itself off.
            let mv = if rng.chance(0.2) {
                0
            } else {
                rng.range(3450, 4250)
            };
            vec![format!("battery {mv}")]
        }
        13 => {
            context.connected = false;
            context.calling = false;
            vec!["reset".into(), "wait 2s".into()]
        }
        14 => vec!["check-storage".into()],
        15 => {
            // The button grid (with BUTTONS on, below the clock: the
            // whole screen is its buttons, so any touch on it is a press).
            // Sometimes from the clock; otherwise wherever the screen is,
            // as earlier steps left it.
            let mut path = Vec::new();
            if rng.chance(0.3) {
                path.push("up".into());
                path.push("wait 500ms".into());
            }
            let at = point(rng);
            match rng.below(3) {
                // A tap, which waits 0.3 s (0.6 s with two layers) for a
                // second.
                0 => path.push(format!("tap {at}")),
                // A double tap, or with two layers, a sequence: the same
                // button again, or another. (Two taps over a dev kit's
                // probe are ~0.3 s apart at best: `double` is both in one
                // input, 0.1 s apart.)
                1 => {
                    if rng.chance(0.5) {
                        path.push(format!("double {at}"));
                    } else {
                        path.push(format!("tap {at}"));
                        if rng.chance(0.5) {
                            path.push(format!("wait {}ms", rng.range(50, 700)));
                        }
                        path.push(format!("tap {}", point(rng)));
                    }
                }
                // Held: a hold, then at 1.5 s a long press, then the
                // release.
                _ => {
                    let held = rng.range(600, 2600);
                    context.waited += held as f64 / 1000.0;
                    path.push(format!("press {at}"));
                    path.push(format!("wait {held}ms"));
                    path.push("lift".into());
                }
            }
            path.push(format!("wait {}ms", rng.range(100, 1200)));
            path
        }
        20 => {
            context.connected = !context.connected;
            if context.connected {
                vec!["connect".into(), set_time(rng, context)]
            } else {
                context.calling = false;
                vec!["disconnect".into()]
            }
        }
        21 => gb(format!(
            r#"GB({{"t":"notify","id":{},"src":"{}","title":"{}","subject":"","body":"{}","sender":"{}","tel":"","reply":true}})"#,
            rng.range(1, 2_000_000_000),
            text(rng, 20),
            text(rng, 50),
            text(rng, 80),
            if rng.chance(0.3) {
                text(rng, 30)
            } else {
                String::new()
            },
        )),
        22 => {
            if context.calling && rng.chance(0.7) {
                context.calling = false;
                gb(format!(
                    r#"GB({{"t":"call","cmd":"{}"}})"#,
                    rng.pick(&["end", "reject", "accept"])
                ))
            } else {
                context.calling = true;
                gb(format!(
                    r#"GB({{"t":"call","cmd":"incoming","name":"{}","number":"+47{}"}})"#,
                    if rng.chance(0.8) {
                        text(rng, 30)
                    } else {
                        String::new()
                    },
                    rng.range(10_000_000, 99_999_999)
                ))
            }
        }
        23 => {
            if rng.chance(0.5) {
                gb(format!(
                    r#"GB({{"t":"musicinfo","artist":"{}","album":"{}","track":"{}","dur":{},"c":1,"n":1}})"#,
                    text(rng, 30),
                    text(rng, 20),
                    text(rng, 30),
                    rng.range(30, 600)
                ))
            } else {
                gb(format!(
                    r#"GB({{"t":"musicstate","state":"{}","position":0,"shuffle":-1,"repeat":-1}})"#,
                    rng.pick(&["play", "pause", "stop"])
                ))
            }
        }
        24 => gb(format!(r#"GB({{"t":"find","n":{}}})"#, rng.chance(0.6))),
        25 => vec![set_time(rng, context)],
        26 => {
            let mut alarms = Vec::new();
            for _ in 0..rng.below(5) {
                // Now and then one just ahead, to ring.
                let (h, m) = match context.local {
                    Some(local) if rng.chance(0.4) => {
                        let at = (local + context.waited as i64 + 60 * rng.range(1, 3) as i64)
                            .rem_euclid(86_400)
                            / 60;
                        (at / 60, at % 60)
                    }
                    _ => (rng.below(24) as i64, rng.below(60) as i64),
                };
                alarms.push(format!(
                    r#"{{"h":{h},"m":{m},"rep":{},"on":{}}}"#,
                    if rng.chance(0.5) { 0 } else { rng.below(128) },
                    rng.chance(0.8)
                ));
            }
            gb(format!(r#"GB({{"t":"alarm","d":[{}]}})"#, alarms.join(",")))
        }
        27 => {
            if rng.chance(0.25) {
                gb(r#"GB({"t":"nav"})"#.into())
            } else {
                gb(format!(
                    r#"GB({{"t":"nav","instr":"{}","distance":"{} m","action":"{}","eta":"{:02}:{:02}"}})"#,
                    text(rng, 30),
                    rng.range(10, 2000),
                    rng.pick(&[
                        "left",
                        "right",
                        "straight",
                        "slight_left",
                        "slight_right",
                        "sharp_left",
                        "sharp_right",
                        "uturn",
                        "roundabout_right",
                        "finish",
                        "continue",
                    ]),
                    rng.below(24),
                    rng.below(60)
                ))
            }
        }
        28 => {
            if rng.chance(0.5) {
                gb(format!(
                    r#"GB({{"t":"weather","temp":{},"hum":{},"txt":"{}","loc":"{}"}})"#,
                    rng.range(250, 310),
                    rng.below(100),
                    text(rng, 15),
                    text(rng, 15)
                ))
            } else {
                gb(format!(
                    r#"GB({{"t":"gps","lat":{:.4},"lon":{:.4},"alt":10.0,"speed":0.0,"time":1794150001234,"satellites":0,"hdop":{:.1},"externalSource":true,"gpsSource":"network"}})"#,
                    rng.range(0, 140_000) as f64 / 1000.0 - 70.0,
                    rng.range(0, 360_000) as f64 / 1000.0 - 180.0,
                    rng.range(10, 500) as f64 / 10.0
                ))
            }
        }
        29 => {
            context.advertising = !context.advertising;
            if context.advertising {
                vec![format!(
                    "bthome-temp {:.2}",
                    rng.range(0, 6000) as f64 / 100.0 - 20.0
                )]
            } else {
                vec!["bthome-temp off".into()]
            }
        }
        _ => unreachable!("only the choices above"),
    }
}

/// Gadgetbridge's `setTime`, at a random moment in October 2026 (with
/// Europe's and America's clocks going back, at its end), in a random
/// time zone.
///
/// In the past, and not far: the dev kit records its activity at the time
/// it's told, and Gadgetbridge syncs that once the phone's paired again.
/// A minute from the future would sit in its history ahead of the real
/// ones; one from the past only goes over the dev kit's own, of a watch
/// on a desk.
fn set_time(rng: &mut Rng, context: &mut Context) -> String {
    // 2026-10-01 to 2026-11-01, UTC.
    let utc = rng.range(1_790_812_800, 1_793_491_200) as i64;
    let zone = *rng.pick(&[-8.0, -5.0, 0.0, 1.0, 2.0, 5.5, 9.0, 12.75]);
    context.local = Some(utc + (zone * 3600.0) as i64);
    context.waited = 0.0;
    format!("gb {}", devkit::set_time_line(utc, zone))
}

/// Random text for the phone to send, as Gadgetbridge escapes it: plain
/// words, mostly, and now and then Latin-1 (`\xNN`), other characters
/// (`\uXXXX`, an emoji as two), a quote, or far more than fits.
fn text(rng: &mut Rng, typical: u64) -> String {
    const WORDS: &[&str] = &[
        "Kari",
        "Ola",
        "Nordmann",
        "Ida",
        "Moreau",
        "Signal",
        "Dinner",
        "on",
        "Sunday",
        "Meeting",
        "moved",
        "to",
        "3pm",
        "Late",
        "Trains",
        "Storgata",
        "OK",
        "#plans",
        "ok?",
        "12:30",
        "Quarterly",
        "planning",
        "-",
        "&",
        "Oslo",
    ];
    const ESCAPES: &[&str] = &[
        r"\xc5",
        r"\xe6",
        r"\xf8",
        r"\xe9",
        r"\xfc",
        r"\u2019",
        r"\u201c",
        r"\u2013",
        r"\u2026",
        r"\u00a0",
        r"\ud83d\ude00",
        r"\u4e2d",
        r#"\""#,
        r"\\",
    ];
    let limit = if rng.chance(0.1) {
        typical * 4
    } else {
        rng.range(0, typical)
    };
    let mut out = String::new();
    while (out.len() as u64) < limit {
        if !out.is_empty() {
            out.push(' ');
        }
        if rng.chance(0.15) {
            out.push_str(rng.pick(ESCAPES));
        } else {
            out.push_str(rng.pick(WORDS));
        }
    }
    out
}

/// A small, seeded pseudo-random generator (SplitMix64): the same seed, the
/// same script, here and on any machine.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// In `0..n` (n > 0).
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    /// In `low..=high`.
    fn range(&mut self, low: u64, high: u64) -> u64 {
        low + self.below(high - low + 1)
    }

    fn chance(&mut self, p: f64) -> bool {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64 <= p
    }

    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len() as u64) as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The same seed makes the same script: a finding's seed is enough to
    /// see it again.
    #[test]
    fn a_seed_makes_the_same_script() {
        let script = |seed| generate(&mut Rng::new(seed), 40, true);
        assert_eq!(script(7), script(7));
        assert_ne!(script(7), script(8));
    }
}
