//! Gives the firmware's UI its version, as the firmware's own `build.rs`
//! does: the firmware's (from its Cargo.toml), shown on the Info screen.
//! `UR_VERSION` overrides it as there (say, `0.9.0+1a2b3c4`, for the
//! README's screenshots); otherwise it's `0.9.0-dev`, as a plain `cargo
//! build` of the firmware is, so the simulator draws a dev kit's Info
//! screen as it is (see `devkit`).
//!
//! And with no theme feature (see Cargo.toml), turns on the firmware's
//! default themes, as its Cargo.toml's `default` names them: so that a
//! build without the window (`--no-default-features`, as the tests and
//! CI build it) still draws what the watch does, and no command needs to
//! name the default theme.
//!
//! And checks every theme is listed everywhere it has to be (see
//! `src/themes/manifests.rs`), failing the build with what to add where.

use std::env;
use std::fs;
use std::path::Path;

#[path = "../src/themes/manifests.rs"]
mod manifests;

fn main() {
    let manifest = "../Cargo.toml";
    println!("cargo:rerun-if-changed={manifest}");
    println!("cargo:rerun-if-changed=../src/themes");
    println!("cargo:rerun-if-changed=Cargo.toml");
    let problems = manifests::check(Path::new(".."));
    for problem in &problems {
        println!("cargo::error={problem}");
    }
    if !problems.is_empty() {
        return;
    }
    println!("cargo:rerun-if-env-changed=UR_VERSION");
    let text = fs::read_to_string(manifest).expect("the firmware's Cargo.toml is next door");
    // The first `version = "…"` is `[package]`'s, which comes first.
    let version = text
        .lines()
        .find_map(|line| line.strip_prefix("version = \""))
        .and_then(|rest| rest.strip_suffix('"'))
        .expect("the firmware's Cargo.toml has a version");
    let full = env::var("UR_VERSION").unwrap_or_else(|_| format!("{version}-dev"));
    let (release, build) = full.split_once('+').unwrap_or((&full, ""));
    println!("cargo:rustc-env=UR_VERSION={full}");
    println!("cargo:rustc-env=UR_RELEASE={release}");
    println!("cargo:rustc-env=UR_BUILD={build}");

    // The themes built in, by their features' names, as
    // `UR_THEME_FEATURES` (comma-separated), for the tests to check each
    // one's screenshots: cargo's `CARGO_FEATURE_*` can't tell `-` from `_`.
    let own = fs::read_to_string("Cargo.toml").expect("the simulator's Cargo.toml is here");
    let mut built: Vec<String> = (manifests::features(&own).into_keys())
        .filter(|feature| feature.starts_with("theme-"))
        .filter(|feature| {
            let var = format!("CARGO_FEATURE_{}", feature.to_uppercase().replace('-', "_"));
            env::var_os(var).is_some()
        })
        .collect();

    // `default = ["theme-retro"]`, in the firmware's `[features]`.
    if built.is_empty() {
        let defaults = text
            .lines()
            .find_map(|line| line.strip_prefix("default = ["))
            .and_then(|rest| rest.strip_suffix(']'))
            .expect("the firmware's Cargo.toml has default features");
        for feature in defaults.split(',').map(|f| f.trim().trim_matches('"')) {
            if feature.starts_with("theme") {
                println!("cargo:rustc-cfg=feature=\"{feature}\"");
                built.push(feature.to_owned());
            }
        }
    }
    println!("cargo:rustc-env=UR_THEME_FEATURES={}", built.join(","));
}
