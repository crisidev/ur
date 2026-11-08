//! Gives the firmware's UI its version, as the firmware's own `build.rs`
//! does: the firmware's (from its Cargo.toml), shown on the Info screen.
//! `UR_VERSION` overrides it as there (say, `0.9.0+1a2b3c4`, for the
//! README's screenshots); otherwise it's `0.9.0-sim`.

use std::env;
use std::fs;

fn main() {
    let manifest = "../Cargo.toml";
    println!("cargo:rerun-if-changed={manifest}");
    println!("cargo:rerun-if-env-changed=UR_VERSION");
    let text = fs::read_to_string(manifest).expect("the firmware's Cargo.toml is next door");
    // The first `version = "…"` is `[package]`'s, which comes first.
    let version = text
        .lines()
        .find_map(|line| line.strip_prefix("version = \""))
        .and_then(|rest| rest.strip_suffix('"'))
        .expect("the firmware's Cargo.toml has a version");
    let full = env::var("UR_VERSION").unwrap_or_else(|_| format!("{version}-sim"));
    let (release, build) = full.split_once('+').unwrap_or((&full, ""));
    println!("cargo:rustc-env=UR_VERSION={full}");
    println!("cargo:rustc-env=UR_RELEASE={release}");
    println!("cargo:rustc-env=UR_BUILD={build}");
}
