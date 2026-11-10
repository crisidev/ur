//! The simulator against `gbmock`, a model of Gadgetbridge (see
//! `src/gadgetbridge.rs`): a few hundred sessions, each from a seed's
//! worth of bytes. The fuzzer (`fuzz/fuzz_targets/gadgetbridge.rs`) runs
//! the same thing for as long as it's left to.

use std::path::Path;

/// One test, as the simulator is one watch a process: the sessions run
/// one after the other.
#[test]
fn sessions_with_gadgetbridge() {
    let scratch = Path::new(env!("CARGO_TARGET_TMPDIR")).join("gadgetbridge");
    // SplitMix64: the same sessions on every machine.
    let mut state = 0x5EED_u64;
    let mut next = move || {
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    };
    for _ in 0..200 {
        let len = 16 + (next() % 600) as usize;
        let bytes: Vec<u8> = (0..len).map(|_| next() as u8).collect();
        ur_sim::gadgetbridge::host_one(&bytes, &scratch);
        // The same session as a dev kit runs it (`--fuzz --corpus DIR
        // --gadgetbridge`): it still passes, and its script is one a dev
        // kit can be made to run, in a sensible time.
        let script = ur_sim::gadgetbridge::devkit_script(&bytes, &scratch)
            .unwrap_or_else(|e| panic!("as a dev kit would run it: {e}"));
        assert!(script.contains("compare g000"), "{script}");
        for line in script.lines() {
            let mut words = line.split_whitespace();
            match words.next() {
                Some("steps" | "heart") => panic!("only the simulator takes {line:?}"),
                Some("wait") => {
                    let ms: u64 = (words.next().and_then(|w| w.strip_suffix("ms")))
                        .map_or(0, |ms| ms.parse().expect("a whole number of ms"));
                    assert!(ms <= 20_000, "a dev kit waits in real time: {line}");
                }
                _ => {}
            }
        }
    }
}
