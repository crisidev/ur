//! The simulator against a model of Gadgetbridge, driven by
//! `ur_sim::gadgetbridge::host_one`: each input's bytes choose what the
//! phone does, what the wearer does, and how time passes; everything the
//! watch sends back goes to the model, which has to understand it.
//!
//! ```sh
//! cd sim && cargo +nightly fuzz run -s none gadgetbridge -- -fork=8 -timeout=20
//! ```

#![no_main]

use std::path::PathBuf;
use std::sync::OnceLock;

use libfuzzer_sys::fuzz_target;

/// One scratch directory per process, as `steps` has (see there).
fn scratch() -> &'static PathBuf {
    static SCRATCH: OnceLock<PathBuf> = OnceLock::new();
    SCRATCH.get_or_init(|| {
        let base = std::env::var_os("UR_FUZZ_SCRATCH").map_or_else(
            || PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target"),
            PathBuf::from,
        );
        base.join(format!("scratch-gb-{}", std::process::id()))
    })
}

fuzz_target!(|bytes: &[u8]| {
    ur_sim::gadgetbridge::host_one(bytes, scratch());
});
