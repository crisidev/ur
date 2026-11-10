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

/// A hold in the quiet night, kept on past the screen's timeout: the
/// screen goes off, and the touch controller to sleep, so the lift is
/// never seen. The release goes then; it once never did, and whatever
/// the hold set going (a light dimming) ran on.
#[test]
fn a_hold_past_the_quiet_night_timeout_ends() {
    let log = log_of(
        "grid-hold-night",
        "time 2026-11-10 23:30\nbuttons bthome\nuntil 23:40\nasleep on\nwait 2m\n\
         expect quiet-night\nbutton\nwait 1s\nup\nwait 1s\n\
         press 40,20\nwait 12s\nexpect not screen-on\nlift\nwait 1s\n",
    );
    assert_eq!(
        heard(&log),
        ["2 hold_press", "2 long_press", "2 hold_release"],
        "{log}"
    );
}
