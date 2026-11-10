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

/// What `settings` printed last (from its first line, the day theme, to
/// the last bond), less the times.
fn settings(log: &str) -> Vec<&str> {
    let lines: Vec<_> = log
        .lines()
        .filter_map(|line| line.split_once("] ").map(|(_, rest)| rest))
        .collect();
    let first = lines
        .iter()
        .rposition(|line| line.starts_with("day theme: "))
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
    for what in ["day theme", "night theme", "brightness"] {
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

/// Where a run's THEME tap led: the scheme its `theme: NAME` line names.
fn tapped_to(log: &str) -> &str {
    let (_, rest) = log.rsplit_once("] theme: ").expect("THEME was tapped");
    rest.lines().next().expect("a line")
}

/// Taps THEME on Settings (right of the clock, past the alarms), then
/// lists the settings.
const TAP_THEME: &str = "left\nleft\nwait 1s\ntap 40,20\nwait 1s\nexpect screen-on\nsettings\n";

/// The same, by day in Oslo.
const DAY: &str = "time 2026-06-21 12:00 +02:00\nlocation 59.91 10.75\nwait 2s\n";

/// A watch upgraded from 0.10, which saved each side's theme as an index
/// into retro's schemes (under keys 17 and 18), boots with the same
/// schemes: TERMINAL at night and PARCHMENT by day, the second of each.
/// Once the wearer picks another (whichever comes next, with whatever
/// themes are built in), it's saved by its ID, which boot then goes by.
#[test]
fn themes_saved_as_indices_carry_over() {
    let dir = dir("flash-theme-migration");
    let flash = dir.join("flash.bin");
    let flash = flash.to_str().expect("UTF-8");
    let (ok, log) = run(
        &dir,
        "old.txt",
        "store 17 01\nstore 18 01\n",
        &["--flash", flash],
    );
    assert!(ok, "{log}");

    // At night (the sun's unknown), TERMINAL, as 0.10 had it.
    let night = format!("expect theme terminal\n{TAP_THEME}");
    let (ok, log) = run(&dir, "night.txt", &night, &["--flash", flash]);
    assert!(ok, "{log}");
    let picked = tapped_to(&log);
    assert_ne!(picked, "TERMINAL", "{log}");
    let saved = settings(&log);
    assert!(
        saved
            .iter()
            .any(|s| s.starts_with(&format!("night theme: {picked} ("))),
        "{log}"
    );
    // By day in Oslo, PARCHMENT.
    let day = format!("{DAY}expect theme parchment\n{TAP_THEME}");
    let (ok, log) = run(&dir, "day.txt", &day, &["--flash", flash]);
    assert!(ok, "{log}");
    let picked_by_day = tapped_to(&log).to_owned();
    assert_ne!(picked_by_day, "PARCHMENT", "{log}");

    // Both now saved by ID, which boot goes by, over the old index.
    let (ok, log) = run(&dir, "after.txt", "settings\n", &["--flash", flash]);
    assert!(ok, "{log}");
    let saved = settings(&log);
    assert!(
        saved
            .iter()
            .any(|s| s.starts_with(&format!("night theme: {picked} ("))),
        "{log}"
    );
    let by_day = format!("day theme: {picked_by_day} (");
    assert!(saved.iter().any(|s| s.starts_with(&by_day)), "{log}");
}

/// `palette::id`: a scheme's name's 32-bit FNV-1a hash.
fn id(name: &str) -> u32 {
    let mut hash: u32 = 0x811c_9dc5;
    for byte in name.bytes() {
        hash = (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193);
    }
    hash
}

/// A scheme's ID as it's stored (little-endian), as `store` takes it.
fn id_hex(name: &str) -> String {
    id(name)
        .to_le_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// A watch that saved a scheme this build hasn't, as when a build with a
/// theme is followed by one without it. Its IDs are written straight into
/// storage, as that build would have saved them, of names no scheme has
/// (no such scheme is in the repository, as only retro is): boot can't
/// tell them from any other scheme's not built in. Each side shows its
/// first scheme then (retro's, first in `themes!`); the IDs stay in flash,
/// for that theme to find again if it comes back, until the wearer picks
/// one for the side, which then saves its own; and nothing gets stuck on
/// the way.
#[test]
fn a_saved_scheme_not_built_in_shows_the_first_until_another_is_picked() {
    let dir = dir("flash-unknown-scheme");
    let flash = dir.join("flash.bin");
    let flash = flash.to_str().expect("UTF-8");
    let (night, day) = (id_hex("NO SUCH SCHEME"), id_hex("NOR THIS ONE"));
    let (ok, log) = run(
        &dir,
        "other.txt",
        &format!("store 43 {night}\nstore 42 {day}\n"),
        &["--flash", flash],
    );
    assert!(ok, "{log}");

    // Boots, falls back on both sides, and saves nothing over the IDs.
    let (ok, log) = run(
        &dir,
        "boot.txt",
        "expect theme arcade\nwait 1m\nsettings\n",
        &["--flash", flash],
    );
    assert!(ok, "{log}");
    let saved = settings(&log);
    let unknown = |side: &str, first: &str| {
        let line = format!("{side} theme: {first} (");
        saved
            .iter()
            .any(|s| s.starts_with(&line) && s.ends_with(", not built in)"))
    };
    assert!(unknown("night", "ARCADE"), "{log}");
    assert!(unknown("day", "HANDHELD"), "{log}");

    // At night, THEME steps on from ARCADE, the first; and that, and only
    // that side, is saved by its own ID.
    let (ok, log) = run(&dir, "night.txt", TAP_THEME, &["--flash", flash]);
    assert!(ok, "{log}");
    let picked = tapped_to(&log).to_owned();
    let saved = settings(&log);
    // As `settings` shows an ID: so this file's hash is the firmware's.
    let line = format!("night theme: {picked} ({:#010x})", id(&picked));
    assert!(saved.contains(&line.as_str()), "{log}");
    let still = |s: &&str| s.starts_with("day theme: HANDHELD") && s.ends_with(", not built in)");
    assert!(saved.iter().any(still), "{log}");

    // By day in Oslo, the same, from HANDHELD.
    let day_tap = format!("{DAY}expect theme handheld\n{TAP_THEME}");
    let (ok, log) = run(&dir, "day.txt", &day_tap, &["--flash", flash]);
    assert!(ok, "{log}");
    let picked_by_day = tapped_to(&log).to_owned();
    let saved = settings(&log);
    let line = format!("day theme: {picked_by_day} ({:#010x})", id(&picked_by_day));
    assert!(saved.contains(&line.as_str()), "{log}");
    assert!(
        saved
            .iter()
            .any(|s| s.starts_with(&format!("night theme: {picked} ("))),
        "{log}"
    );

    // And the storage still reads whole, at the next boot.
    let (ok, log) = run(&dir, "after.txt", "settings\n", &["--flash", flash]);
    assert!(ok, "{log}");
    assert_eq!(settings(&log), saved);
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
