//! The version the Info screen shows: build.rs's `UR_RELEASE` (the
//! version without its build metadata, `0.9.0-pr.12`) and `UR_BUILD`
//! (the commit, `1a2b3c4`, or empty for a plain `cargo build`).
//!
//! A function, not the constants themselves, so that the simulator (which
//! has a `version` of its own, in place of this module) can show another
//! in its screenshots, which then don't change with each release. Not in
//! `ui::Status`, which the UI loop's future keeps (static RAM) across
//! every await of a frame.

/// The version, and the commit it's built from.
pub fn shown() -> (&'static str, &'static str) {
    (env!("UR_RELEASE"), env!("UR_BUILD"))
}
