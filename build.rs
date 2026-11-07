//! Makes `memory.x` visible to the linker, which `link.x` from cortex-m-rt
//! includes by name, and gives the firmware its full version.
//!
//! The version is Cargo.toml's, extended by `scripts/release.sh` (through
//! `UR_VERSION`) with what kind of build it is and the commit it's from:
//! `0.9.0+1a2b3c4` for a release, `0.9.0-pr.12+1a2b3c4` for a pull request,
//! `0.9.0-snap.345+1a2b3c4` for a commit on main. A plain `cargo build` is
//! `0.9.0-dev`. The firmware shows it on the Info screen and logs it at boot.
//! The bootloader's image header only holds the numbers (see
//! `scripts/package.py`).

use std::{env, fs, path::PathBuf};

fn main() {
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("cargo always sets OUT_DIR"));
    fs::copy("memory.x", out.join("memory.x")).expect("memory.x exists in the crate root");
    println!("cargo:rustc-link-search={}", out.display());
    println!("cargo:rerun-if-changed=memory.x");

    let cargo = env::var("CARGO_PKG_VERSION").expect("cargo always sets CARGO_PKG_VERSION");
    println!("cargo:rerun-if-env-changed=UR_VERSION");
    let full = env::var("UR_VERSION").unwrap_or_else(|_| format!("{cargo}-dev"));
    // Cargo.toml is the source of truth: the extension may only add to it.
    let extends = full
        .strip_prefix(&cargo)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with(['-', '+', '.']));
    assert!(
        extends,
        "UR_VERSION ({full}) must extend Cargo.toml's version ({cargo})"
    );
    let (release, build) = full.split_once('+').unwrap_or((&full, ""));
    // `UR_VERSION`: everything; `UR_RELEASE`: without the build metadata
    // (the commit); `UR_BUILD`: just that, or empty.
    println!("cargo:rustc-env=UR_VERSION={full}");
    println!("cargo:rustc-env=UR_RELEASE={release}");
    println!("cargo:rustc-env=UR_BUILD={build}");
}
