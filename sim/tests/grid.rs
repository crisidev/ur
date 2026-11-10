//! The button grid's timer, through the UI loop (`ui_loop::timer_at`), as
//! the simulator runs it: in a process of its own, killed if it doesn't
//! finish, as a loop that never lets time pass never would.

use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

/// A grid button held, and the button pressed while it is (back to the
/// clock), with the finger held on past when the hold would have become a
/// long press. That long press is moot off the grid, so nothing should be
/// due; it once stayed due, and the UI loop ran again and again, at the
/// same instant, until the finger lifted (the watch's ran flat out; the
/// simulator's, whose time only passes between passes, forever).
#[test]
fn a_hold_left_behind_is_not_due() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("grid-hold");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("the target dir is writable");
    let script = dir.join("script.txt");
    std::fs::write(
        &script,
        "buttons bthome\nwait 1s\nup\nwait 1s\npress 40,20\nwait 1s\n\
         button\nwait 2500ms\nlift\nwait 1s\n",
    )
    .expect("the target dir is writable");
    let mut child = Command::new(env!("CARGO_BIN_EXE_ur-sim"))
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .arg("--out")
        .arg(&dir)
        .arg(&script)
        .spawn()
        .expect("the simulator runs");
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().expect("the simulator can be waited on") {
            break status;
        }
        if started.elapsed() > Duration::from_secs(30) {
            let _ = child.kill();
            panic!("the simulator never finished: its UI loop's timer stayed due");
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    assert!(status.success());
}

/// Runs `script` on the simulator, and returns its log.
fn log_of(name: &str, script: &str) -> String {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("the target dir is writable");
    let path = dir.join("script.txt");
    std::fs::write(&path, script).expect("the target dir is writable");
    let output = Command::new(env!("CARGO_BIN_EXE_ur-sim"))
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .arg("--out")
        .arg(&dir)
        .arg(&path)
        .output()
        .expect("the simulator runs");
    let log = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(output.status.success(), "the script failed:\n{log}");
    log
}

/// What Home Assistant heard from the grid, in order (`button_2
/// hold_press`, say).
fn heard(log: &str) -> Vec<&str> {
    log.lines()
        .filter_map(|line| line.split_once("to Home Assistant: button_"))
        .filter_map(|(_, rest)| rest.split_once(" (counter"))
        .map(|(event, _)| event)
        .collect()
}

/// Holds lifted either side of the moment a touch becomes a long press:
/// a tap just before, and a hold, with its release, from then on. (On the
/// watch, a lift read just after that moment, before the long press's
/// timer, once sent a hold with no release; the simulator, whose timer is
/// never late, can't lift there, so `host-tests/gesture` covers that.)
#[test]
fn a_hold_lifted_near_the_long_press_still_ends() {
    let log = log_of(
        "grid-hold-edge",
        "buttons bthome\nwait 1s\nup\nwait 1s\n\
         press 40,20\nwait 590ms\nlift\nwait 2s\n\
         press 40,20\nwait 600ms\nlift\nwait 2s\n\
         press 40,20\nwait 610ms\nlift\nwait 2s\n",
    );
    assert_eq!(
        heard(&log),
        [
            "2 press",
            "2 hold_press",
            "2 hold_release",
            "2 hold_press",
            "2 hold_release",
        ],
        "{log}"
    );
}

/// The quiet night, with the screen woken by the button, on the button
/// grid: the start of each script below.
const QUIET_GRID: &str = "time 2026-11-10 23:30\nbuttons bthome\nuntil 23:40\nasleep on\n\
     wait 2m\nexpect quiet-night\nbutton\nwait 1s\nup\nwait 1s\n";

/// Whether `event` reached Home Assistant after the log's `marker` line
/// (an `echo`), rather than before it.
fn heard_after(log: &str, marker: &str, event: &str) -> bool {
    let at = |needle: &str| log.lines().position(|line| line.ends_with(needle));
    let marker = at(marker).expect("the marker is in the log");
    let heard = log
        .lines()
        .position(|line| line.contains(&format!("to Home Assistant: button_{event} ")))
        .expect("the event is in the log");
    heard > marker
}

/// A hold in the quiet night, kept on past the screen's timeout: the
/// screen stays on until the lift, the release goes then, and the screen
/// goes off its usual timeout after. The screen used to go off 10 s into
/// the hold, the touch controller to sleep with it, and the release went
/// then, with the finger still down (and before that, never).
#[test]
fn a_quiet_night_hold_keeps_the_screen_on_until_the_lift() {
    let log = log_of(
        "grid-hold-night",
        &format!(
            "{QUIET_GRID}press 40,20\nwait 20s\nexpect screen-on\nexpect quiet-night\n\
             echo lifting\nlift\nwait 9s\nexpect screen-on\nwait 2s\n\
             expect not screen-on\nexpect quiet-night\n"
        ),
    );
    assert_eq!(
        heard(&log),
        ["2 hold_press", "2 long_press", "2 hold_release"],
        "{log}"
    );
    assert!(heard_after(&log, "lifting", "2 hold_release"), "{log}");
}

/// A hold kept on past `gesture::HOLD_LIMIT` (a minute), in the quiet
/// night: not a finger, most likely (bedding resting on the screen), so
/// the screen goes off then, as if the hold had ended, and its release
/// goes as the touch controller sleeps, before the lift.
#[test]
fn a_hold_past_the_limit_still_ends() {
    let log = log_of(
        "grid-hold-limit",
        &format!(
            "{QUIET_GRID}press 40,20\nwait 55s\nexpect screen-on\nwait 10s\n\
             expect not screen-on\necho lifting\nlift\nwait 1s\n"
        ),
    );
    assert_eq!(
        heard(&log),
        ["2 hold_press", "2 long_press", "2 hold_release"],
        "{log}"
    );
    assert!(!heard_after(&log, "lifting", "2 hold_release"), "{log}");
}

/// A hold by day, past the screen's timeout: the screen stays on until
/// the lift too (it used to go off 10 s in, the touch going on unseen).
#[test]
fn a_hold_by_day_keeps_the_screen_on_until_the_lift() {
    let log = log_of(
        "grid-hold-day",
        "time 2026-11-10 12:00\nbuttons bthome\nwait 1s\nup\nwait 1s\n\
         press 40,20\nwait 20s\nexpect screen-on\necho lifting\nlift\nwait 9s\n\
         expect screen-on\nwait 2s\nexpect not screen-on\n",
    );
    assert_eq!(
        heard(&log),
        ["2 hold_press", "2 long_press", "2 hold_release"],
        "{log}"
    );
    assert!(heard_after(&log, "lifting", "2 hold_release"), "{log}");
}

/// In the quiet night, a touch held on a glance only wakes the screen
/// (bedding, against a wrist that turned in sleep, as likely as a
/// finger): it doesn't hold the screen on, which goes off as after any
/// wake, the touch controller with it.
#[test]
fn a_touch_that_woke_the_screen_does_not_hold_it() {
    let log = log_of(
        "grid-hold-glance",
        "time 2026-11-10 23:30\nuntil 23:40\nasleep on\nwait 2m\nexpect quiet-night\n\
         glance\nwait 1s\npress 40,40\nwait 5s\nexpect screen-on\n\
         wait 7s\nexpect not screen-on\nlift\n",
    );
    assert!(log.contains("input: long press"), "{log}");
}
