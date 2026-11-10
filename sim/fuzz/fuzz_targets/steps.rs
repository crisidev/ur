//! The simulator, driven by `ur_sim::host_one`: each input's bytes are a
//! storage image's settings and a script of steps, as `ur-sim --fuzz`
//! makes from a seed.

#![no_main]

use std::path::PathBuf;
use std::sync::OnceLock;

use libfuzzer_sys::fuzz_target;

/// One scratch directory per process (libFuzzer's `-fork` and `-jobs` run
/// several), for the storage image `host_one` saves settings to: under
/// `UR_FUZZ_SCRATCH`, or this crate's `target/` (which git ignores).
fn scratch() -> &'static PathBuf {
    static SCRATCH: OnceLock<PathBuf> = OnceLock::new();
    SCRATCH.get_or_init(|| {
        let base = std::env::var_os("UR_FUZZ_SCRATCH").map_or_else(
            || PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target"),
            PathBuf::from,
        );
        base.join(format!("scratch-{}", std::process::id()))
    })
}

fuzz_target!(|bytes: &[u8]| {
    ur_sim::host_one(bytes, scratch());
});
