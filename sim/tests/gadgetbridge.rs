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
    }
}
