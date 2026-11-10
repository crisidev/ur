//! The watch's flash in the simulator: settings saved by the firmware's
//! own UI loop and storage, kept in a file between runs (`--flash`), loaded
//! at boot, and after switching off and back on, as on the watch.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Runs `script` (written to a file of its own) with `args`; returns
/// whether it succeeded, and what it printed.
fn run(dir: &Path, name: &str, script: &str, args: &[&str]) -> (bool, String) {
    let path = dir.join(name);
    std::fs::write(&path, script).expect("the target dir is writable");
    let output = Command::new(env!("CARGO_BIN_EXE_ur-sim"))
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .args(args)
        .arg(&path)
        .output()
        .expect("the simulator runs");
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// A fresh directory for a test's files.
fn dir(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("the target dir is writable");
    dir
}

/// What `settings` printed last (from its first line, the light theme, to
/// the last bond), less the times.
fn settings(log: &str) -> Vec<&str> {
    let lines: Vec<_> = log
        .lines()
        .filter_map(|line| line.split_once("] ").map(|(_, rest)| rest))
        .collect();
    let first = lines
        .iter()
        .rposition(|line| line.starts_with("light theme: "))
        .expect("`settings` ran");
    let last = first
        + lines[first..]
            .iter()
            .position(|line| line.starts_with("bond 2: "))
            .expect("`settings` ran to the end");
    lines[first..=last].to_vec()
}

#[test]
fn settings_survive_between_runs() {
    let dir = dir("flash-between-runs");
    let flash = dir.join("flash.bin");
    let flash = flash.to_str().expect("UTF-8");
    // On the settings screen (right of the clock, past the alarms), a tap
    // on the theme and one on the brightness, saved by the UI loop as on
    // the watch; and an alarm, as the script sets it.
    let (ok, log) = run(
        &dir,
        "first.txt",
        "alarm 1 06:45 on\nleft\nleft\nwait 1s\ntap 40,20\nwait 1s\ntap 40,37\nwait 1s\nsettings\n",
        &["--flash", flash],
    );
    assert!(ok, "{log}");
    let saved = settings(&log);
    assert!(
        saved.iter().any(|s| s.starts_with("alarms: 06:45 on")),
        "{log}"
    );
    for what in ["light theme", "dark theme", "brightness"] {
        let stored = saved
            .iter()
            .any(|s| s.starts_with(what) && !s.ends_with("not stored"));
        // Only the group in use has its theme switched.
        assert!(stored || what.contains("theme"), "{what}: {log}");
    }
    assert!(
        saved
            .iter()
            .any(|s| s.contains("theme: ") && !s.ends_with("not stored")),
        "{log}"
    );

    let (ok, log) = run(&dir, "second.txt", "settings\n", &["--flash", flash]);
    assert!(ok, "{log}");
    assert_eq!(settings(&log), saved);

    // Without `--flash`, every run starts erased.
    let (ok, log) = run(&dir, "third.txt", "settings\n", &[]);
    assert!(ok, "{log}");
    assert!(log.contains("alarms: not stored"), "{log}");
}

#[test]
fn switching_off_and_on_boots_from_flash() {
    let dir = dir("flash-reboot");
    // A critically low battery switches the watch off, and the charger
    // wakes it: a new boot, which loads the alarm from flash, and knows the
    // time again only because the phone's connected, and sets it.
    let script = "\
time 2026-11-09 07:29:50 +01:00
connect
alarm 1 07:45 on
battery 3300
wait 90s
expect off
battery 3500 charging
wait 5s
expect not off
until 07:45
expect buzzing
";
    let (ok, log) = run(&dir, "reboot.txt", script, &[]);
    assert!(ok, "{log}");
    assert!(log.contains("booted"), "{log}");
    assert!(
        log.contains("the phone reconnected, and set the time"),
        "{log}"
    );
}

#[test]
fn a_flash_file_of_the_wrong_size_is_refused() {
    let dir = dir("flash-wrong-size");
    let flash = dir.join("flash.bin");
    std::fs::write(&flash, [0xFF; 100]).expect("the target dir is writable");
    let (ok, log) = run(
        &dir,
        "script.txt",
        "settings\n",
        &["--flash", flash.to_str().expect("UTF-8")],
    );
    assert!(!ok);
    assert!(log.contains("the storage region is 12288"), "{log}");
}

/// A one-off alarm from the phone ringing at the same minute as one of the
/// watch's own (the host fuzzer's persistence check found this): both turn
/// themselves off as they ring together, but only the watch's was saved,
/// as the phone's were saved only when one of theirs was the alarm that
/// rang (the watch's, listed first, is). Booted again before the phone
/// next syncs, the watch had the phone's alarm on again, and it rang
/// again the next morning.
#[test]
fn a_phone_alarm_rung_with_the_watchs_is_saved() {
    let dir = dir("flash-phone-alarm-rang");
    let script = "\
time 2026-11-09 06:59:30 +01:00
connect
gb GB({\"t\":\"alarm\",\"d\":[{\"h\":7,\"m\":0,\"rep\":0,\"on\":true}]})
alarm 1 07:00 on
until 07:00
expect buzzing
wait 1s
settings
# Switched off (a flat battery) and on again (the charger): it boots from
# flash, and the phone sets the time, but sends no alarms.
battery 3300
wait 5m
expect off
battery 3500 charging
wait 5s
expect not off
until 07:00
expect not buzzing
";
    let (ok, log) = run(&dir, "rang.txt", script, &[]);
    assert!(ok, "{log}");
    let saved = settings(&log);
    assert!(
        saved.iter().any(|s| s.starts_with("alarms: 07:00 off")),
        "{log}"
    );
    assert!(
        saved.contains(&"phone alarms: 07:00 Rang (days 0000000)"),
        "{log}"
    );
    assert!(log.contains("booted"), "{log}");
}
