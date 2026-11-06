//! Makes `memory.x` visible to the linker, which `link.x` from cortex-m-rt
//! includes by name.

use std::{env, fs, path::PathBuf};

fn main() {
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("cargo always sets OUT_DIR"));
    fs::copy("memory.x", out.join("memory.x")).expect("memory.x exists in the crate root");
    println!("cargo:rustc-link-search={}", out.display());
    println!("cargo:rerun-if-changed=memory.x");
}
