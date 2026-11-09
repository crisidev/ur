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

/// A hold held on to `buttons::LONG_HOLD` (1.5 s) is a long press too,
/// sent while the finger is still down, to both Home Assistant and the
/// phone, then the release on the lift; one lifted sooner has none. And
/// a long press that can't go to Home Assistant shows so, as any press.
#[test]
fn a_hold_held_on_is_a_long_press() {
    let dir = out("long-press");
    let script = dir.join("script.txt");
    std::fs::create_dir_all(&dir).expect("the target dir is writable");
    std::fs::write(
        &script,
        "buttons both\nconnect\nup\nwait 1s\n\
         press 40,60\nwait 1400ms\nexpect not buzzing\nwait 130ms\nexpect buzzing\n\
         wait 1s\nlift\nwait 2s\n\
         echo short\npress 13,20\nwait 1s\nlift\nwait 2s\n\
         echo busy\nslots full\npress 67,20\nwait 1530ms\nexpect buzzing\nlift\n",
    )
    .expect("the target dir is writable");
    let (ok, log) = run(script.to_str().expect("UTF-8"), &[]);
    assert!(ok, "{log}");
    let (long, rest) = log.split_once("short").expect("the script runs on");
    let (short, busy) = rest.split_once("busy").expect("the script runs on");
    for gesture in ["hold_press", "long_press", "hold_release"] {
        assert!(
            long.contains(&format!("button_5 {gesture}")),
            "{gesture} to Home Assistant: {log}"
        );
        assert!(
            long.contains(&format!(r#""button":5,"gesture":"{gesture}""#)),
            "{gesture} to the phone: {log}"
        );
    }
    assert!(long.find("long_press") < long.find("release at"), "{log}");
    assert!(short.contains("button_1 hold_release"), "{log}");
    assert!(!short.contains("long_press"), "{log}");
    // With the slots full, none of the three went to Home Assistant, and
    // each buzzed so; the phone had them all.
    assert_eq!(busy.matches("not to Home Assistant").count(), 3, "{log}");
    assert_eq!(busy.matches("buzz: turn").count(), 3, "{log}");
    assert!(
        busy.contains(r#""button":3,"gesture":"long_press""#),
        "{log}"
    );
}

/// With a phone connected, a double tap plays or pauses without waking
/// the screen (or ending a glance) only if its first tap is light. A
/// deliberate first tap wakes it at once, and the second plays or pauses
/// on the clock face, as it would have with the screen off. The awake
/// clock face's own double tap still skips.
#[test]
fn a_double_tap_plays_or_pauses_without_waking_only_after_a_light_first_tap() {
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
         expect not dim\nexpect buzzing\n\
         echo clock\nwait 2s\ntap 40,40 50\nwait 150ms\ntap 40,40 50\nexpect buzzing\n",
    )
    .expect("the target dir is writable");
    let (ok, log) = run(script.to_str().expect("UTF-8"), &[]);
    assert!(ok, "{log}");
    let (gestures, clock) = log.split_once("] clock\n").expect("the script runs on");
    assert_eq!(gestures.matches(r#""n":"playpause""#).count(), 4, "{log}");
    assert_eq!(gestures.matches(r#""n":"next""#).count(), 0, "{log}");
    assert_eq!(clock.matches(r#""n":"next""#).count(), 1, "{log}");
    assert_eq!(clock.matches(r#""n":"playpause""#).count(), 0, "{log}");
}

/// With a phone connected, a tap, then a touch held (a tap-and-hold),
/// skips with the screen off or on a glance, through the touch task's
/// recogniser: the hold must start within 0.5 s of the tap's lift, and in
/// the middle of the screen (see `wake::centred`), a fifth in from the
/// sides, a quarter from the top and bottom. A long press on its own
/// there does nothing, not even a buzz, and leaves the screen (and a
/// glance) as it was; nor does a pair with either touch nearer the edge,
/// where an arm folded over the watch rests. After a deliberate tap, which
/// wakes the screen, the hold skips on the clock face (unless the tap was
/// at the edge of a glance: then it's the clock face's long press). On the
/// awake clock face, a long press still plays or pauses anywhere, bar for
/// 3 s after a touch woke it (maybe the folded arm): then only in the
/// middle.
#[test]
fn a_tap_and_hold_skips_and_a_lone_long_press_does_nothing() {
    let dir = out("tap-and-hold");
    let script = dir.join("script.txt");
    std::fs::create_dir_all(&dir).expect("the target dir is writable");
    std::fs::write(
        &script,
        "connect\nwait 1s\n\
         echo awake-edge\nlong 12,40\nexpect buzzing\nwait 2s\n\
         press 75,40\nwait 610ms\nexpect buzzing\nlift\nexpect clock\n\
         wait 11s\nexpect not screen-on\n\
         echo off-lone\nlong 40,40\nexpect not buzzing\npress 40,40\nwait 700ms\n\
         expect not buzzing\nlift\nexpect not screen-on\nwait 1s\n\
         echo off\ntap 40,40 30\npress 40,40\nwait 610ms\nexpect buzzing\n\
         expect not screen-on\nlift\nwait 2s\n\
         echo off-late\ntap 40,40 30\nwait 600ms\npress 40,40\nwait 700ms\n\
         expect not buzzing\nlift\nwait 1s\n\
         echo off-edge\ntap 40,40 30\nlong 12,40\nexpect not buzzing\n\
         tap 40,40 30\nlong 40,61\nexpect not buzzing\ntap 40,40 30\nlong 40,19\n\
         expect not buzzing\ntap 40,4 30\nlong 40,40\nexpect not buzzing\n\
         expect not screen-on\n\
         echo glance\nglance\ntap 40,40 30\npress 40,40\nwait 610ms\nexpect buzzing\n\
         expect dim\nlift\nwait 10s\n\
         echo glance-edge\nglance\nlong 40,40\nexpect not buzzing\n\
         tap 40,40 30\nlong 64,40\nexpect not buzzing\ntap 40,40 30\nlong 40,12\n\
         expect not buzzing\ntap 40,77 30\nlong 40,40\nexpect not buzzing\n\
         expect dim\nwait 6s\nexpect not screen-on\n\
         echo tap-woke\ntap 40,40\nexpect screen-on\npress 40,40\nwait 610ms\n\
         expect buzzing\nlift\nwait 11s\nexpect not screen-on\n\
         echo tap-woke-edge\ntap 40,40\nexpect screen-on\nwait 1s\nlong 40,4\n\
         expect not buzzing\nwait 1s\nlong 12,40\nexpect not buzzing\n\
         echo tap-woke-later\nlong 40,40\nexpect buzzing\nwait 1500ms\n\
         long 40,4\nexpect buzzing\nwait 11s\nexpect not screen-on\n\
         echo glance-tap-woke-edge\nglance\ntap 40,4\nexpect screen-on\n\
         long 40,40\nexpect buzzing\nwait 1s\nlong 40,70\nexpect not buzzing\n\
         echo done\n",
    )
    .expect("the target dir is writable");
    let (ok, log) = run(script.to_str().expect("UTF-8"), &[]);
    assert!(ok, "{log}");
    // Each section, from its echo to the next one's: how many times it
    // played or paused, and skipped.
    let sections = [
        ("awake-edge", 2, 0),
        ("off-lone", 0, 0),
        ("off", 0, 1),
        ("off-late", 0, 0),
        ("off-edge", 0, 0),
        ("glance", 0, 1),
        ("glance-edge", 0, 0),
        ("tap-woke", 0, 1),
        ("tap-woke-edge", 0, 0),
        ("tap-woke-later", 2, 0),
        // The tap at the edge can't start a tap-and-hold, so the hold
        // after it is the clock face's long press, in the middle.
        ("glance-tap-woke-edge", 1, 0),
        ("done", 0, 0),
    ];
    let at = |name: &str| {
        log.find(&format!("] {name}\n"))
            .unwrap_or_else(|| panic!("{name} echoes: {log}"))
    };
    for pair in sections.windows(2) {
        let [(name, playpause, next), (following, _, _)] = pair else {
            unreachable!("windows of two")
        };
        let section = &log[at(name)..at(following)];
        let counts = (
            section.matches(r#""n":"playpause""#).count(),
            section.matches(r#""n":"next""#).count(),
        );
        assert_eq!(counts, (*playpause, *next), "{name}: {log}");
    }
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

/// A call (or a notification) over a ringing alarm leaves it ringing:
/// the motor goes back to the alarm once the call is over.
#[test]
fn an_alarm_rings_on_after_a_call() {
    let dir = out("alarm-call");
    let script = dir.join("script.txt");
    std::fs::create_dir_all(&dir).expect("the target dir is writable");
    std::fs::write(
        &script,
        "time 2026-11-09 06:59 +01:00\nalarm 1 07:00 on\nuntil 07:00\nexpect buzzing\n\
         call Kari\nnotify Signal Kari\ncall over\nexpect buzzing\n\
         button\nexpect not buzzing\n",
    )
    .expect("the target dir is writable");
    let (ok, log) = run(script.to_str().expect("UTF-8"), &[]);
    assert!(ok, "{log}");
    let buzzes: Vec<&str> = log
        .lines()
        .filter_map(|line| line.split_once("buzz: ").map(|(_, buzz)| buzz))
        .collect();
    assert_eq!(buzzes, ["alarm", "ring", "alarm", "stop"], "{log}");
}

/// The phone setting the clock back while a timer runs (host fuzzing's
/// find, as it was found): the timer counted local seconds, so 8.7 days
/// back left over 200 hours on a 3-minute timer, and opening it in the
/// editor, with its two digits of hours, panicked. A timer counts uptime
/// now: it opens, and rings on time.
#[test]
fn a_running_timer_survives_the_clock_set_back() {
    let dir = out("timer-clock-back");
    let script = dir.join("script.txt");
    std::fs::create_dir_all(&dir).expect("the target dir is writable");
    std::fs::write(
        &script,
        "connect\n\
         gb setTime(1793254721);E.setTimeZone(-5.0);(s=>s&&(s.timezone=-5.0,require('Storage').write('setting.json',s)))(require('Storage').readJSON('setting.json',1))\n\
         wait 1s\n\
         left 14,16\n\
         # Timer 2 (3 minutes): a long press starts it.\n\
         long 70,70\n\
         gb setTime(1792501729);E.setTimeZone(-8.0);(s=>s&&(s.timezone=-8.0,require('Storage').write('setting.json',s)))(require('Storage').readJSON('setting.json',1))\n\
         wait 1107ms\n\
         # Its editor, counting down.\n\
         tap 64,79\n\
         shot editor.png\n\
         wait 170s\n\
         expect not buzzing\n\
         wait 10s\n\
         expect buzzing\n",
    )
    .expect("the target dir is writable");
    let (ok, log) = run(
        script.to_str().expect("UTF-8"),
        &["--out", dir.to_str().expect("UTF-8")],
    );
    assert!(ok, "{log}");
}

/// A 10-minute timer takes 10 minutes, even across three time zones (it
/// took 3 h 10 min when it counted local time).
#[test]
fn a_timer_ignores_a_new_time_zone() {
    let dir = out("timer-time-zone");
    let script = dir.join("script.txt");
    std::fs::create_dir_all(&dir).expect("the target dir is writable");
    std::fs::write(
        &script,
        "time 2026-11-09 12:00 +01:00\ntimer 1 10\nleft\nlong 20,70\nwait 1m\n\
         time 2026-11-09 09:01 -02:00\nwait 530s\nexpect not buzzing\n\
         wait 20s\nexpect buzzing\n",
    )
    .expect("the target dir is writable");
    let (ok, log) = run(script.to_str().expect("UTF-8"), &[]);
    assert!(ok, "{log}");
}

/// "Find my watch" rings until found (a touch), and gives up after two
/// minutes; under a ringing alarm it waits its turn, and neither it nor
/// a call ending silences the alarm.
#[test]
fn find_my_watch_rings_until_found_and_never_silences_an_alarm() {
    let dir = out("find");
    let script = dir.join("script.txt");
    std::fs::create_dir_all(&dir).expect("the target dir is writable");
    std::fs::write(
        &script,
        "time 2026-11-09 06:58 +01:00\nfind on\nexpect buzzing\nwait 2s\ntap\n\
         expect not buzzing\nexpect clock\n\
         alarm 1 07:00 on\nuntil 07:00\nexpect buzzing\n\
         find on\nfind off\nexpect buzzing\ncall Kari\ncall over\nexpect buzzing\n\
         button\nexpect not buzzing\n\
         find on\nwait 119s\nexpect buzzing\nwait 2s\nexpect not buzzing\n",
    )
    .expect("the target dir is writable");
    let (ok, log) = run(script.to_str().expect("UTF-8"), &[]);
    assert!(ok, "{log}");
    let buzzes: Vec<&str> = log
        .lines()
        .filter_map(|line| line.split_once("buzz: ").map(|(_, buzz)| buzz))
        .collect();
    assert_eq!(
        buzzes,
        [
            "find", "stop", "alarm", "ring", "alarm", "stop", "find", "stop"
        ],
        "{log}"
    );
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
        "live activity: heart rate true, steps true, every 10 s",
        r#"to the phone: {"t":"act","stp":0,"hrm":72,"rt":1}"#,
        "live activity: stop",
    ] {
        assert!(log.contains(line), "{line:?} missing from:\n{log}");
    }
}

/// Gadgetbridge's live activity (`live`): a report every interval, with
/// the steps since the last and this session's latest heart rate, until
/// Gadgetbridge stops it or the phone disconnects, however long that
/// takes (the chart is open on the phone: no time limit).
#[test]
fn live_activity() {
    let dir = out("live");
    let script = dir.join("script.txt");
    std::fs::create_dir_all(&dir).expect("the target dir is writable");
    let start = |interval: u32| {
        format!("gb GB({{\"t\":\"act\",\"hrm\":true,\"stp\":true,\"int\":{interval}}})\n")
    };
    let stop = "gb GB({\"t\":\"act\",\"hrm\":false,\"stp\":false,\"int\":10})\n";
    // The time first, as Gadgetbridge sends it on connecting: today's
    // steps need it.
    let text = [
        "gb setTime(1794150000);E.setTimeZone(1.0)\nheart 60\n".to_string(),
        start(10),
        "wait 5s\nsteps 12\nheart 72\nwait 10s\nsteps 3\nwait 10s\n".into(),
        stop.into(),
        "wait 30s\necho disconnecting\n".into(),
        start(5),
        "wait 6s\ndisconnect\nwait 30s\necho long\n".into(),
        start(60),
        "wait 30m\nwait 30s\necho end\n".into(),
    ]
    .concat();
    std::fs::write(&script, text).expect("the target dir is writable");
    let (ok, log) = run(script.to_str().expect("UTF-8"), &[]);
    assert!(ok, "{log}");
    let reports = |part: &str| {
        part.lines()
            .filter_map(|line| line.split_once("to the phone: "))
            .map(|(_, json)| json.to_owned())
            .collect::<Vec<_>>()
    };
    let (stopping, rest) = log.split_once("live activity: stop").expect("it stops");
    let (stopped, rest) = rest.split_once("disconnecting").expect("it goes on");
    let (disconnecting, rest) = rest.split_once("long").expect("it goes on");
    let (long, _) = rest.split_once("end").expect("it ends");
    // The reading from before the session isn't this session's.
    assert_eq!(
        reports(stopping),
        [
            r#"{"t":"act","stp":12,"hrm":72,"rt":1}"#,
            r#"{"t":"act","stp":3,"hrm":72,"rt":1}"#,
        ],
        "{log}"
    );
    assert_eq!(reports(stopped), [] as [&str; 0], "{log}");
    assert_eq!(
        reports(disconnecting),
        [r#"{"t":"act","stp":0,"hrm":0,"rt":1}"#],
        "{log}"
    );
    // A report a minute for the half hour, with no end in sight.
    assert_eq!(
        reports(long),
        [r#"{"t":"act","stp":0,"hrm":0,"rt":1}"#; 30],
        "{log}"
    );
}

/// Summer time's changes, as Gadgetbridge sends them (the new offset a
/// few seconds after the change): an alarm every day at 02:30 rings once
/// the night the clock goes back, rather than in both 02:00s, and the
/// night it goes forward over 02:30, at the change, rather than not at
/// all.
#[test]
fn summer_time_rings_each_alarm_once() {
    let dir = out("summer-time");
    let script = dir.join("script.txt");
    std::fs::create_dir_all(&dir).expect("the target dir is writable");
    std::fs::write(
        &script,
        "# 2026-10-25: 03:00 CEST is 02:00 CET, at 01:00 UTC.\n\
         time 2026-10-25 01:55 +02:00\nconnect\nphone-alarms 02:30/127\n\
         until 02:30\nexpect buzzing\nbutton\nuntil 03:00\nwait 5s\n\
         gb setTime(1792890005);E.setTimeZone(1.0);\n\
         until 02:30\nexpect not buzzing\nuntil 03:00\nexpect not buzzing\n\
         # 2026-03-29: 02:00 CET is 03:00 CEST, at 01:00 UTC.\n\
         time 2026-03-29 01:55 +01:00\nuntil 02:00\nwait 5s\n\
         gb setTime(1774746005);E.setTimeZone(2.0);\n\
         wait 2s\nexpect buzzing\n",
    )
    .expect("the target dir is writable");
    let (ok, log) = run(script.to_str().expect("UTF-8"), &[]);
    assert!(ok, "{log}");
}

/// Today's steps across a time zone change, as phones count them: on a
/// new date that midnight didn't bring, the steps since the new zone's
/// midnight, which the Today screen shows too (`expect steps` checks
/// both). Flying west across midnight, the evening after landing used to
/// count from 0 as a new day (STEPS 5 under Today's 1,205), and start
/// over again at the real midnight.
#[test]
fn steps_today_follow_a_new_time_zone() {
    let dir = out("steps-time-zone");
    let script = dir.join("script.txt");
    std::fs::create_dir_all(&dir).expect("the target dir is writable");
    std::fs::write(
        &script,
        "# West over midnight: Oslo to London, landing on the 9th again.\n\
         time 2026-11-09 23:40 +01:00\nsteps 1000\nexpect steps 1000\n\
         until 00:10\nsteps 200\nexpect steps 200\n\
         time 2026-11-09 23:12 +00:00\nexpect steps 1200\n\
         steps 5\nexpect steps 1205\n\
         until 00:10\nexpect steps 0\nsteps 7\nexpect steps 7\n\
         # East over midnight: London to Oslo, landing on the 12th.\n\
         time 2026-11-11 22:30 +00:00\nsteps 3000\nuntil 23:40\nsteps 50\n\
         expect steps 3050\n\
         time 2026-11-12 00:41 +01:00\nexpect steps 50\nsteps 8\nexpect steps 58\n\
         # The same date an hour over: as it was.\n\
         time 2026-11-13 15:00 +01:00\nsteps 100\nexpect steps 100\n\
         time 2026-11-13 14:01 +00:00\nexpect steps 100\n",
    )
    .expect("the target dir is writable");
    let (ok, log) = run(script.to_str().expect("UTF-8"), &[]);
    assert!(ok, "{log}");
}
