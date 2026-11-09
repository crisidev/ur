//! `ur-sim --fuzz --dry`: the dev kit fuzzer's scripts, run on the
//! simulator alone. A check of the generator (every line it writes is one
//! the simulator takes, bar what only a dev kit does), not of the
//! simulator; that needs a dev kit.

use std::path::Path;
use std::process::Command;

#[test]
fn every_generated_line_runs_on_the_simulator() {
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("fuzz-dry");
    let _ = std::fs::remove_dir_all(&out);
    let output = Command::new(env!("CARGO_BIN_EXE_ur-sim"))
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        // With a phone (an address that's never connected to: `--dry`
        // doesn't), so the scripts have Bluetooth in.
        .args([
            "--fuzz",
            "unused.elf",
            "--dry",
            "--peer",
            "00:00:00:00:00:00",
        ])
        .args(["--seed", "1", "--runs", "12", "--steps", "60", "--out"])
        .arg(&out)
        .output()
        .expect("the simulator runs");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    // Each run left its script, for a look.
    assert!(out.join("run-1/script.txt").exists());
}
