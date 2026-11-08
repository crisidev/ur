//! What differs between the command line and the browser (see
//! [`crate::web`]): the wall clock, where log lines go, and where files
//! go. Everything else is the same code on both.
//!
//! On `wasm32-unknown-unknown`, the standard library has no clock (its
//! `SystemTime::now` panics), stderr goes nowhere, and there's no file
//! system: the page hands over its clock, shows the log, and downloads
//! the files instead, through the functions it gives the module (see
//! `web/ur.js`).

use std::path::Path;

/// The wall clock now, as `time now` sets the watch's: UTC seconds since
/// 1970, and the local time zone's offset from UTC then, in minutes.
#[cfg(not(target_arch = "wasm32"))]
pub fn now() -> (i64, i32) {
    let now = chrono::Local::now();
    (now.timestamp(), now.offset().local_minus_utc() / 60)
}

/// A line for whoever's watching: the terminal's stderr.
#[cfg(not(target_arch = "wasm32"))]
pub fn log(line: &str) {
    eprintln!("{line}");
}

/// Writes a file (a screenshot), making its directory if need be.
#[cfg(not(target_arch = "wasm32"))]
pub fn save(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let fail = |e: std::io::Error| format!("writing {}: {e}", path.display());
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(fail)?;
    }
    std::fs::write(path, bytes).map_err(fail)
}

#[cfg(target_arch = "wasm32")]
pub fn now() -> (i64, i32) {
    let ms = crate::web::now_ms();
    // `div_euclid`: a second starts at its first millisecond, before 1970
    // too (not that a browser's clock is likely to be there).
    let utc = (ms as i64).div_euclid(1000);
    (utc, crate::web::utc_offset_minutes(ms))
}

/// A line for the page's log.
#[cfg(target_arch = "wasm32")]
pub fn log(line: &str) {
    crate::web::print(crate::web::Text::Log, line);
}

/// A download, named after the file: the browser decides where it goes.
#[cfg(target_arch = "wasm32")]
pub fn save(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let name = path
        .file_name()
        .ok_or_else(|| format!("{}: not a file name", path.display()))?;
    crate::web::download(&name.to_string_lossy(), bytes);
    Ok(())
}
