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
