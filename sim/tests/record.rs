//! Recordings (`record FILE.gif` … `stop`), decoded again: the frames a
//! script's GIF has, and how long each shows. In a process of their own,
//! as the other scripts' tests (`scripts.rs`) are.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Runs `script` (the lines given), with `--out` a fresh directory, which
/// it returns.
fn run(name: &str, script: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("record-{name}"));
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
    let log = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{log}");
    dir
}

/// A GIF's frames: each one's delay (centiseconds), and its pixels (RGBA,
/// of the rectangle it covers).
fn frames(path: &Path) -> Vec<(u16, Vec<u8>)> {
    let mut options = gif::DecodeOptions::new();
    options.set_color_output(gif::ColorOutput::RGBA);
    let mut decoder = options
        .read_info(File::open(path).expect("the GIF was written"))
        .expect("the GIF decodes");
    let mut frames = Vec::new();
    while let Some(frame) = decoder.read_next_frame().expect("each frame decodes") {
        frames.push((frame.delay, frame.buffer.to_vec()));
    }
    frames
}

/// The clock face ticks each second, from boot: so a recording that
/// starts between ticks has a frame until the next, one a second, and the
/// last until `stop`, adding up to exactly the time it took.
#[test]
fn the_clock_ticking() {
    // Booted by `time`, its first frame sweeping in; the UI ticks at 1 s,
    // 2 s, 3 s of uptime, so the recording (from 0.3 s or so, after the
    // sweep, to 3 s later) sees three of them.
    let dir = run(
        "ticking",
        "time 2026-11-09 07:29:50 +01:00\nwait 0.2s\nrecord tick.gif\nwait 3s\nstop\n",
    );
    let frames = frames(&dir.join("tick.gif"));
    let delays: Vec<u16> = frames.iter().map(|&(delay, _)| delay).collect();
    assert_eq!(delays.len(), 4, "{delays:?}");
    assert_eq!(delays[1..3], [100, 100], "{delays:?}");
    assert_eq!(delays.iter().sum::<u16>(), 300, "{delays:?}");
    // Only the colon (it blinks) and the seconds change from one tick to
    // the next: the frames after the first hold just the rectangle round
    // them.
    let whole = 240 * 240 * 4;
    assert_eq!(frames[0].1.len(), whole);
    assert!(
        frames[1..]
            .iter()
            .all(|(_, pixels)| pixels.len() < whole / 4)
    );
}

/// Hours of a dark screen are a moment of the GIF: `--still` long. What
/// follows plays at its own pace, and nothing shows for less than the
/// 20 ms browsers honour.
#[test]
fn a_long_wait_is_cut_short() {
    let dir = run(
        "still",
        "time 2026-11-09 07:29:50 +01:00\nwait 30s\nexpect not screen-on\n\
         record still.gif --still 1s\nwait 7h\nbutton\nwait 1.5s\nright\nwait 1s\nstop\n",
    );
    let frames = frames(&dir.join("still.gif"));
    let delays: Vec<u16> = frames.iter().map(|&(delay, _)| delay).collect();
    // The screen off: black, for the one second, and then while the
    // button's frame goes to the panel, before the backlight comes on
    // (the whole clock face: a fifth of a second, as on the watch).
    let woken = delays[0];
    assert!((110..130).contains(&woken), "{delays:?}");
    assert!(frames[0].1.chunks(4).all(|p| p == [0, 0, 0, 255]));
    // Then 1.5 s of clock face (split by a tick), the sweep to Today, its
    // steps each a mark (20 ms) long, and 1 s more. The sweep takes time
    // of its own, as on the watch, on top of the script's waits.
    assert!(delays.iter().all(|&d| d >= 2 && d % 2 == 0), "{delays:?}");
    let sweep = delays.iter().filter(|&&d| d == 2).count() as u16;
    assert!(sweep >= 3, "{delays:?}");
    assert_eq!(
        delays.iter().sum::<u16>(),
        woken + 150 + 2 * sweep + 100,
        "{delays:?}"
    );
}

#[test]
fn the_whole_watch() {
    let dir = run(
        "watch",
        "time 2026-11-09 07:29:50 +01:00\nwait 0.2s\nrecord watch.gif --watch\nwait 1s\n",
    );
    // Never stopped: written when the script ends.
    let path = dir.join("watch.gif");
    let decoder = gif::DecodeOptions::new()
        .read_info(File::open(&path).expect("the GIF was written at the end"))
        .expect("the GIF decodes");
    // The window's size at the default --zoom 4: wider than the screen.
    assert!(decoder.width() > 320 && decoder.height() > decoder.width());
    assert_eq!(frames(&path).len(), 2);
}

/// A notification's buzz shakes the whole watch, a frame at each of the
/// camera's marks while the motor runs and winds down, then lets it be:
/// the same scene recorded panel-only has none of those frames, and both
/// last as long.
#[test]
fn the_whole_watch_shakes() {
    let scene = |look: &str| {
        format!(
            "time 2026-11-09 07:29:50 +01:00\nwait 0.2s\nrecord buzz.gif{look}\n\
             notify 0\nwait 2s\nstop\n"
        )
    };
    let watch = frames(&run("shake-watch", &scene(" --watch")).join("buzz.gif"));
    let screen = frames(&run("shake-screen", &scene("")).join("buzz.gif"));
    let total = |frames: &[(u16, Vec<u8>)]| frames.iter().map(|&(d, _)| d).sum::<u16>();
    assert_eq!(total(&watch), total(&screen));
    // The 60 ms buzz and its wind-down: several marks' worth of shaking,
    // but nothing like a frame a mark for the 2 s.
    let shaken = watch.len() - screen.len();
    assert!(
        (3..=10).contains(&shaken),
        "{} vs {}",
        watch.len(),
        screen.len()
    );
}

/// The README's GIFs: the script runs to the end, and each GIF stays
/// small enough for a README to load without thinking twice.
#[test]
fn the_readme_gifs() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("record-readme");
    let _ = std::fs::remove_dir_all(&dir);
    let output = Command::new(env!("CARGO_BIN_EXE_ur-sim"))
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .arg("--out")
        .arg(&dir)
        .arg("readme-gifs.txt")
        .output()
        .expect("the simulator runs");
    let log = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{log}");
    for name in ["swipe", "notification", "glance", "alarm"] {
        let path = dir.join(format!("{name}.gif"));
        let size = std::fs::metadata(&path).expect("the GIF was written").len();
        assert!(size < 100_000, "{name}.gif is {size} bytes");
        assert!(frames(&path).len() > 1, "{name}.gif doesn't move");
    }
}
