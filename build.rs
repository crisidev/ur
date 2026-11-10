//! Makes `memory.x` visible to the linker, which `link.x` from cortex-m-rt
//! includes by name, passes the linker scripts, wraps two of Nordic's functions (see below), and gives
//! the firmware its full version.
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

    // The linker scripts: `link.x` comes from cortex-m-rt (and includes our
    // `memory.x`); `defmt.x` places the defmt string table where
    // `defmt-print` expects to find it. `--nmagic` avoids page-aligning
    // sections, which would waste flash. These live here rather than in
    // `.cargo/config.toml`'s `rustflags` because cargo merges the
    // `rustflags` of every config file up the directory tree: a git worktree
    // inside the repository (e.g. under `.claude/worktrees/`) would pass
    // each script twice, and the link would fail on duplicate sections.
    println!("cargo:rustc-link-arg-bins=-Tlink.x");
    println!("cargo:rustc-link-arg-bins=-Tdefmt.x");
    println!("cargo:rustc-link-arg-bins=--nmagic");
    // Identical code folding: functions that compile to the same bytes (a
    // generic at two types of the same size, `index` and `index_mut`,
    // empty functions, LLVM's outlined fragments, …) are kept once,
    // ~1.7 KB less flash. `all` rather than `safe`, which folds nothing
    // here (rustc marks nearly every function's address as significant).
    // What it gives up is distinct addresses for folded functions, which
    // Rust doesn't promise (`fn_addr_eq`'s docs), and nothing here
    // compares; but a backtrace or a debugger may name one of the folded
    // functions for another.
    println!("cargo:rustc-link-arg-bins=--icf=all");

    // Route nrf-sdc's and nrf-mpsl's calls to `sdc_init` and `mpsl_init`
    // through our `__wrap_sdc_init` and `__wrap_mpsl_init`, which register
    // our fault handlers instead of theirs; see `src/field_log.rs` ("Nordic's
    // fault handlers"). Without these, the link fails (nothing defines
    // `__real_sdc_init`) rather than silently keeping their handlers.
    println!("cargo:rustc-link-arg-bins=--wrap=sdc_init");
    println!("cargo:rustc-link-arg-bins=--wrap=mpsl_init");

    // The `coverage` feature's sections (see `coverage.x`), and only with
    // it: a release build links exactly as without this.
    println!("cargo:rerun-if-changed=coverage.x");
    if env::var_os("CARGO_FEATURE_COVERAGE").is_some() {
        fs::copy("coverage.x", out.join("coverage.x"))
            .expect("coverage.x exists in the crate root");
        println!("cargo:rustc-link-arg-bins=-Tcoverage.x");
    }

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
