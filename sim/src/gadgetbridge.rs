//! The simulator with a Gadgetbridge at the other end: `gbmock`, a model
//! of its Bangle.js support (see that crate for the commit it's from).
//!
//! `fuzz.rs` writes its `gb` lines by hand, a dozen shapes of them, and
//! nothing reads what the watch answers. Here the mock decides what a
//! phone sends (everything Gadgetbridge can, spelled as it spells it), and
//! takes every line the watch sends back as Gadgetbridge would:
//!
//! - whatever it sends, in any order, the firmware's UI takes without a
//!   panic (overflows and `debug_assert!`s included) or a step that never
//!   ends;
//! - whatever the watch sends, Gadgetbridge takes without a complaint: no
//!   "Malformed JSON" toast, no exception, nothing it can't place;
//! - and what a fetch leaves in its database is a sample a bucket.
//!
//! [`host_one`] is one input of a coverage-guided fuzzer
//! (`fuzz/fuzz_targets/gadgetbridge.rs`): its bytes choose what happens,
//! through `gbmock::generate`. It's a second target beside `steps`, not
//! a change to that one's generator: `steps`' corpus is tied to how its
//! bytes read, and its scripts also run on a dev kit, over a real link.

use std::path::Path;

use gbmock::generate::{Action, Choices};
use gbmock::{Event, Gadgetbridge};

use crate::flash::Flash;
use crate::script::{self, Output};
use crate::watch::{self, Watch};

/// At most this many steps an input: a step is a few milliseconds, and
/// the fuzzer's timeout is for a UI that spins, not a long script.
const STEPS: usize = 300;

/// One input: a watch booted from erased flash, a Gadgetbridge that
/// connects to it, and then whatever `bytes` say, a step at a time: the
/// phone does something, time passes, or the wearer touches the watch.
/// After every step, what the watch sent goes to Gadgetbridge.
///
/// Panics if the simulator does, if a script line fails, or if
/// Gadgetbridge didn't understand the watch.
pub fn host_one(bytes: &[u8], scratch: &Path) {
    std::fs::create_dir_all(scratch).expect("the scratch directory is writable");
    let path = scratch.join("gadgetbridge-storage.bin");
    std::fs::write(&path, Flash::erased().bytes()).expect("the scratch directory is writable");
    let mut watch = Watch::boot(Flash::open(&path).expect("it was just written"));
    let output = Output {
        dir: scratch.to_owned(),
        zoom: 1,
    };
    watch::capture_to_phone(true);
    let mut gb = Gadgetbridge::new();
    let mut choices = Choices::new(bytes);
    // What's been done, for a finding's message.
    let mut story: Vec<String> = Vec::new();

    let run = |watch: &mut Watch, story: &mut Vec<String>, line: String| {
        story.push(line.clone());
        if let Err(e) = script::execute(watch, &line, &output) {
            panic!("{e}\n--- the script:\n{}", story.join("\n"));
        }
    };

    let mut actions = gbmock::generate::connect_sequence(&mut choices);
    actions.reverse();
    for _ in 0..STEPS {
        let action = match actions.pop() {
            Some(action) => action,
            None if choices.is_empty() => break,
            None => match choices.byte() % 8 {
                // The wearer.
                0 => {
                    let (x, y) = (choices.below(80), choices.below(80));
                    let line = match choices.byte() % 12 {
                        0 | 1 => "button".to_owned(),
                        2 => "button 2".to_owned(),
                        3 | 4 => format!("tap {x},{y}"),
                        5 => format!("long {x},{y}"),
                        6 => "up".to_owned(),
                        7 => "down".to_owned(),
                        8 => "left".to_owned(),
                        9 => "right".to_owned(),
                        10 => format!("steps {}", choices.below(30_000)),
                        _ => format!("heart {}", 40 + choices.below(140)),
                    };
                    run(&mut watch, &mut story, line);
                    continue;
                }
                // The phone.
                _ => gbmock::generate::action(&mut choices),
            },
        };
        match &action {
            // Time passes for both: the watch's clock is the phone's,
            // since it connected. No more than half an hour a step, so
            // that an input's worth of steps stays quick.
            Action::Wait { ms } => {
                let ms = (*ms).clamp(0, 30 * 60_000);
                gb.now_ms += ms;
                run(&mut watch, &mut story, format!("wait {ms}ms"));
            }
            Action::Connect => {
                run(&mut watch, &mut story, "connect".into());
                gb.connect();
            }
            Action::Disconnect => {
                run(&mut watch, &mut story, "disconnect".into());
                gb.disconnect();
            }
            action => gb.perform(action),
        }
        // Gadgetbridge's lines to the watch, and the watch's back, until
        // neither has more to say (an answer may get one: a button's
        // intent refused).
        for _ in 0..4 {
            for sent in gb.take_sent() {
                // As a script line: the bytes between the DLE and the
                // line feed, a character each.
                let line: String = sent
                    .bytes
                    .strip_prefix(b"\x10")
                    .unwrap_or(&sent.bytes)
                    .iter()
                    .map(|&b| char::from(b))
                    .collect();
                let line = line.strip_suffix('\n').unwrap_or(&line);
                // A script line is one line, and a blank one is nothing
                // (only Tasker's lines can be either).
                if line.contains(['\n', '\r']) || line.trim().is_empty() {
                    continue;
                }
                run(&mut watch, &mut story, format!("gb {line}"));
            }
            let lines = watch::take_to_phone();
            if lines.is_empty() {
                break;
            }
            for line in lines {
                story.push(format!("# to the phone: {line}"));
                // In the writes `ble` makes: 20 bytes at worst.
                for packet in format!("{line}\r\n").as_bytes().chunks(20) {
                    gb.receive(packet);
                }
            }
            for event in gb.take_events() {
                assert!(
                    !matches!(
                        event,
                        Event::MalformedJson(_)
                            | Event::Uncaught(_)
                            | Event::IgnoredLine(_)
                            | Event::NotUnderstood(_)
                    ),
                    "Gadgetbridge: {event:?}\n--- the script:\n{}",
                    story.join("\n")
                );
            }
        }
        assert!(
            !gb.has_partial_line(),
            "half a line left in Gadgetbridge\n--- the script:\n{}",
            story.join("\n")
        );
        // A sample a bucket, each at its bucket's last second (see
        // `gb_buckets`): so none within the two minutes Gadgetbridge
        // merges. (Up to 2038, that is: Gadgetbridge files samples
        // under a Java `int` of seconds, and a phone whose clock is set
        // past it has the watch send what wraps to before 1970. The
        // fuzzer found that in minutes; no phone will for a while.)
        for timestamp in gb.samples.keys().filter(|timestamp| **timestamp >= 0) {
            assert_eq!(
                i64::from(*timestamp).rem_euclid(crate::gb_buckets::MINUTES * 60),
                crate::gb_buckets::MINUTES * 60 - 1,
                "a sample off its bucket's end\n--- the script:\n{}",
                story.join("\n")
            );
        }
    }
    watch::capture_to_phone(false);
}
