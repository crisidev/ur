//! The simulator's scripts, run as a user would: each in a process of its
//! own, since the firmware's modules keep their state in statics (the
//! clock, the location, the motor's request), and the mock clock is one
//! per process.
//!
//! `tour.txt` checks itself with `expect`; this checks that it, the
//! README's screenshots, and Gadgetbridge's lines run to the end, and that
//! every screenshot they take is the approved one, pixel for pixel.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn run(script: &str, args: &[&str]) -> (bool, String) {
    let output = sim(&[args, &[script]].concat());
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

fn sim(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ur-sim"))
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .args(args)
        .output()
        .expect("the simulator runs")
}

/// A fresh directory for a test's screenshots.
fn out(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

// ==============================================================================
// Screenshots, against the approved ones
// ==============================================================================
//
// The simulator draws deterministically: a script's screenshots are the
// same on every run, on every computer. So each script's are committed,
// as approved, and every run must draw exactly those, or something
// changed what the watch shows. A change that's meant to is approved by
// running the tests with `UPDATE_SHOTS=1`, which makes the approved
// images what the scripts drew (adding new ones, and deleting those no
// longer taken), for the commit to include: its diff then shows the new
// screens. (CI renders the same scripts for a PR and its base, and posts
// the differences on the PR: see .github/workflows/screenshots.yml.)

/// A script whose screenshots are checked.
struct Shots {
    script: &'static str,
    /// Where its approved screenshots are, from sim/.
    approved: &'static str,
    /// Approved images that aren't the script's to check, or to update.
    unchecked: &'static [&'static str],
}

/// Whether to approve what the scripts draw, rather than check it.
fn updating() -> bool {
    std::env::var_os("UPDATE_SHOTS").is_some_and(|v| !v.is_empty() && v != "0")
}

/// Every PNG directly in `dir`, by name; none if it doesn't exist (a new
/// script's, before its first approval).
fn pngs(dir: &Path) -> BTreeSet<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return BTreeSet::new();
    };
    entries
        .map(|entry| entry.expect("the directory lists").file_name())
        .map(|name| name.into_string().expect("screenshots have UTF-8 names"))
        .filter(|name| name.ends_with(".png"))
        .collect()
}

/// Runs `shots.script`, and checks its screenshots against the approved
/// ones (or, with `UPDATE_SHOTS`, approves them). Returns its log.
fn check_shots(shots: Shots) -> String {
    let name = shots.script.trim_end_matches(".txt");
    let dir = out(&format!("shots/{name}"));
    let rendered = dir.join("rendered");
    let (ok, log) = run(shots.script, &["--out", rendered.to_str().expect("UTF-8")]);
    assert!(ok, "{log}");
    for unchecked in shots.unchecked {
        let _ = std::fs::remove_file(rendered.join(unchecked));
    }
    let approved = Path::new(env!("CARGO_MANIFEST_DIR")).join(shots.approved);
    let checked = |names: BTreeSet<String>| -> BTreeSet<String> {
        names
            .into_iter()
            .filter(|name| !shots.unchecked.contains(&name.as_str()))
            .collect()
    };
    let (old, new) = (checked(pngs(&approved)), checked(pngs(&rendered)));

    if updating() {
        std::fs::create_dir_all(&approved).expect("the approved directory can be created");
        let mut changes = Vec::new();
        for gone in old.difference(&new) {
            std::fs::remove_file(approved.join(gone)).expect("approved images can be deleted");
            changes.push(format!("removed {gone}"));
        }
        for shot in &new {
            let (from, to) = (rendered.join(shot), approved.join(shot));
            // Only what changed, so the rest keep their timestamps.
            if std::fs::read(&from).ok() != std::fs::read(&to).ok() {
                std::fs::copy(&from, &to).expect("approved images can be written");
                let kind = if old.contains(shot) {
                    "changed"
                } else {
                    "added"
                };
                changes.push(format!("{kind} {shot}"));
            }
        }
        // A test that approves passes, and cargo test shows nothing of a
        // passing test's output, so an approval would look exactly like a
        // clean check. It says so, then, on stderr itself: libtest
        // captures only `print!`/`eprint!`, not writes to the handle.
        // (It still passes, so that approving can be scripted.)
        use std::io::Write as _;
        let mut stderr = std::io::stderr().lock();
        let _ = writeln!(
            stderr,
            "\nUPDATE_SHOTS: approving {}'s screenshots in sim/{}, not checking them: {}",
            shots.script,
            shots.approved,
            if changes.is_empty() {
                "none changed".to_string()
            } else {
                changes.join(", ")
            }
        );
        return log;
    }

    // `--diff` compares whole directories: the approved images this checks
    // (not the unchecked) go in one of their own.
    let expected = dir.join("approved");
    std::fs::create_dir_all(&expected).expect("the target dir is writable");
    for shot in &old {
        std::fs::copy(approved.join(shot), expected.join(shot))
            .expect("the target dir is writable");
    }
    let differences = dir.join("differences");
    let diff = sim(&[
        "--diff",
        expected.to_str().expect("UTF-8"),
        rendered.to_str().expect("UTF-8"),
        "--out",
        differences.to_str().expect("UTF-8"),
    ]);
    if diff.status.success() {
        return log;
    }

    // Each difference, by what it means here: a changed screen (and where
    // to see how), a new one, or one no longer taken.
    let report = std::fs::read_to_string(differences.join("report.tsv")).unwrap_or_default();
    let mut message = format!(
        "{}'s screenshots aren't the approved ones (in sim/{}). Each that \
         differs has an image of it before, after, and where (magenta), side by side:\n",
        shots.script, shots.approved
    );
    for line in report.lines() {
        let mut fields = line.split('\t');
        let (Some(kind), Some(shot), differ) = (fields.next(), fields.next(), fields.next()) else {
            continue;
        };
        let at = |dir: &Path| dir.join(shot).display().to_string();
        message += &match kind {
            "changed" => format!(
                "- {shot} differs, in {} pixels: {}\n",
                differ.unwrap_or("?"),
                at(&differences.join("changed"))
            ),
            "added" => format!("- {shot} is new, not approved yet: {}\n", at(&rendered)),
            "removed" => format!("- {shot} is missing: approved, but no longer taken\n"),
            _ => format!("- {line}\n"),
        };
    }
    // Not a difference, then, but the comparison failing.
    if report.is_empty() {
        message += &String::from_utf8_lossy(&diff.stderr);
    }
    panic!(
        "{message}\nIf that's intended, approve them (from sim/), and commit what changes:\n\n    \
         UPDATE_SHOTS=1 cargo test\n"
    );
}

#[test]
fn the_tour() {
    check_shots(Shots {
        script: "tour.txt",
        approved: "shots/tour",
        unchecked: &[],
    });
}

/// The README's own screenshots, so it can't go stale: approving these
/// updates it. Only its PNGs: the GIFs beside them are readme-gifs.txt's
/// (see tests/record.rs), which this neither checks nor touches.
#[test]
fn the_readme_screenshots() {
    check_shots(Shots {
        script: "readme-screenshots.txt",
        approved: "../docs/screenshots",
        // The README's Info screen shows a release's version and commit
        // (see readme-screenshots.txt), which the tests' build never has.
        // The tour checks the screen itself.
        unchecked: &["info.png"],
    });
}

#[test]
fn a_failed_expectation_fails_the_run() {
    let dir = out("failing");
    let script = dir.join("script.txt");
    std::fs::create_dir_all(&dir).expect("the target dir is writable");
    std::fs::write(&script, "button\nexpect screen-on\n").expect("the target dir is writable");
    let (ok, log) = run(script.to_str().expect("UTF-8"), &[]);
    assert!(!ok);
    assert!(log.contains(":2: expected screen-on"), "{log}");
}

#[test]
fn a_held_finger_is_a_long_press_while_still_down() {
    let dir = out("press");
    let script = dir.join("script.txt");
    std::fs::create_dir_all(&dir).expect("the target dir is writable");
    // On the alarms screen, a long press switches an alarm on, and buzzes,
    // once the finger has been still for 0.6 s: before it lifts.
    std::fs::write(
        &script,
        "left\nwait 1s\npress 20,40\nwait 590ms\nexpect not buzzing\n\
         wait 20ms\nexpect buzzing\nlift\n",
    )
    .expect("the target dir is writable");
    let (ok, log) = run(script.to_str().expect("UTF-8"), &[]);
    assert!(ok, "{log}");
    assert!(log.contains("long press at (20,40)"), "{log}");
}

#[test]
fn a_deliberate_tap_wakes_the_screen_at_once() {
    let dir = out("tap-wake");
    let script = dir.join("script.txt");
    std::fs::create_dir_all(&dir).expect("the target dir is writable");
    // A light one (a brush) doesn't; a deliberate one does, at once, even
    // with a phone connected, where a second would be the next track.
    std::fs::write(
        &script,
        "connect\nwait 11s\nexpect not screen-on\ntap 40,40 20\nwait 1s\n\
         expect not screen-on\ntap 40,40\nexpect screen-on\n",
    )
    .expect("the target dir is writable");
    let (ok, log) = run(script.to_str().expect("UTF-8"), &[]);
    assert!(ok, "{log}");
}

/// Home Assistant's key stays on screen, as a QR code and then as text,
/// for as long as it takes to scan or type it: until a tap takes it away.
/// Back on the settings, the screen goes off as usual.
#[test]
fn the_bthome_key_stays_until_tapped_away() {
    let dir = out("bthome-key");
    let script = dir.join("script.txt");
    std::fs::create_dir_all(&dir).expect("the target dir is writable");
    std::fs::write(
        &script,
        "buttons bthome\nleft\nleft\nwait 1s\ntap 40,72\n\
         wait 10m\nexpect screen-on\ntap\n\
         wait 10m\nexpect screen-on\ntap\n\
         wait 11s\nexpect not screen-on\n",
    )
    .expect("the target dir is writable");
    let (ok, log) = run(script.to_str().expect("UTF-8"), &[]);
    assert!(ok, "{log}");
}

/// With both connection slots busy, a press can't go to Home Assistant:
/// it buzzes as one that went nowhere (the tour shows it red, with FAQ
/// B05), even with BUTTONS on BOTH, where the phone still has it. Once a
/// slot is free, presses go again.
#[test]
fn a_press_that_cant_go_to_home_assistant_says_so() {
    let dir = out("unsent");
    let script = dir.join("script.txt");
    std::fs::create_dir_all(&dir).expect("the target dir is writable");
    std::fs::write(
        &script,
        "buttons both\nconnect\nup\nwait 1s\n\
         slots full\ntap 67,20\nwait 400ms\nexpect buzzing\n\
         slots free\nwait 5s\ntap 13,20\nwait 400ms\n",
    )
    .expect("the target dir is writable");
    let (ok, log) = run(script.to_str().expect("UTF-8"), &[]);
    assert!(ok, "{log}");
    let unsent = log
        .find("not to Home Assistant: every connection slot busy")
        .expect("the first press isn't sent");
    let (first, second) = log.split_at(unsent);
    assert!(!first.contains("to Home Assistant:"), "{log}");
    let (busy, free) = second
        .split_once("connection slots free")
        .expect("the slots free up");
    assert!(busy.contains("buzz: turn"), "{log}");
    assert!(busy.contains(r#""button":3"#), "{log}");
    assert!(free.contains("to Home Assistant: button_1 press"), "{log}");
    assert!(free.contains("buzz: notify"), "{log}");
}

/// With a phone connected, a double tap skips without waking the screen
/// (or ending a glance) only if its first tap is light. A deliberate first
/// tap wakes it at once, and the second skips on the clock face.
#[test]
fn a_double_tap_skips_without_waking_only_after_a_light_first_tap() {
    let dir = out("double-tap");
    let script = dir.join("script.txt");
    std::fs::create_dir_all(&dir).expect("the target dir is writable");
    std::fs::write(
        &script,
        "connect\nwait 11s\n\
         tap 40,40 30\nwait 150ms\ntap 40,40 50\nexpect not screen-on\nexpect buzzing\n\
         wait 2s\ntap 40,40 60\nexpect screen-on\nwait 150ms\ntap 40,40 50\n\
         expect screen-on\nexpect not dim\nexpect buzzing\n\
         wait 11s\nglance\ntap 40,40 30\nwait 150ms\ntap 40,40 50\nexpect dim\n\
         expect buzzing\n\
         wait 6s\nglance\ntap 40,40 60\nexpect not dim\nwait 150ms\ntap 40,40 50\n\
         expect not dim\nexpect buzzing\n",
    )
    .expect("the target dir is writable");
    let (ok, log) = run(script.to_str().expect("UTF-8"), &[]);
    assert!(ok, "{log}");
    assert_eq!(log.matches(r#""n":"next""#).count(), 4, "{log}");
}

#[test]
fn a_double_press_ends_the_quiet_night() {
    let dir = out("double-press");
    let script = dir.join("script.txt");
    std::fs::create_dir_all(&dir).expect("the target dir is writable");
    std::fs::write(
        &script,
        "time 2026-11-09 23:00 +01:00\nasleep on\nwait 2m\nexpect quiet-night\n\
         button 2\nexpect not quiet-night\n",
    )
    .expect("the target dir is writable");
    let (ok, log) = run(script.to_str().expect("UTF-8"), &[]);
    assert!(ok, "{log}");
}

/// Gadgetbridge's own lines, through the firmware's parser: each reaches
/// the UI as what it says.
#[test]
fn gadgetbridge_lines() {
    let log = check_shots(Shots {
        script: "gadgetbridge.txt",
        approved: "shots/gadgetbridge",
        unchecked: &[],
    });
    for line in [
        "time set (UTC offset 60 min)",
        r#"to the phone: {"t":"gps_power","status":false}"#,
        "location set: 59.9127, 10.7461",
        "notification: Signal: Åse Nordmann",
        "notification: Signal: Mamma ?",
        "text as bitmaps",
        "music: Ida Moreau: Night Signals",
        "incoming call: Kari Nordmann",
        "call over",
        "2 alarms from the phone",
        "navigation: Left 150 m Storgata",
        "navigation over",
        "find my watch: true",
        "ignoring weather message",
        "not a line the watch understands",
    ] {
        assert!(log.contains(line), "{line:?} missing from:\n{log}");
    }
}
