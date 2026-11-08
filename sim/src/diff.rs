//! Comparing two sets of screenshots: `ur-sim --diff BEFORE AFTER`.
//!
//! The simulator draws deterministically, so a script's screenshots are
//! the same on every run and every computer, and any pixel that changes is
//! a change to what the watch shows. This finds them: every PNG under
//! `BEFORE` and `AFTER` (by its path below each), pixel for pixel. The
//! tests run it against the approved screenshots (see `tests/scripts.rs`),
//! and CI between a PR and its base, for the PR's comment (see
//! `.github/workflows/screenshots.yml`).
//!
//! Under `--out`, it writes what a reviewer needs to see each difference:
//!
//! - `changed/PATH`: before, after, and where they differ, side by side
//!   (see [`shot::save_difference`]);
//! - `added/PATH`: a screenshot only `AFTER` has;
//! - `removed/PATH`: one only `BEFORE` has;
//! - `report.tsv`: a line for each, `changed`, `added` or `removed`, the
//!   path, and how many pixels differ (`changed` only).
//!
//! It prints the same, and fails if anything differs.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use crate::shot;

/// The subdirectories of `--out`, and what the report calls each.
const CHANGED: &str = "changed";
const ADDED: &str = "added";
const REMOVED: &str = "removed";
const REPORT: &str = "report.tsv";

/// Compares the screenshots, writes the differences under `out`, and
/// returns whether there were none.
pub fn run(before: &Path, after: &Path, out: &Path) -> Result<bool, String> {
    // What a previous run left would read as this run's differences.
    for dir in [CHANGED, ADDED, REMOVED] {
        let dir = out.join(dir);
        if dir.exists() {
            std::fs::remove_dir_all(&dir)
                .map_err(|e| format!("removing the old {}: {e}", dir.display()))?;
        }
    }

    let (old, new) = (pngs(before)?, pngs(after)?);
    let mut report = String::new();
    let mut differences = 0;
    for path in old.union(&new) {
        let shown = path.display();
        let line = match (old.contains(path), new.contains(path)) {
            (true, true) => {
                let (from, to) = (before.join(path), after.join(path));
                // Identical files are identical pictures, without decoding
                // them; most are.
                if read(&from)? == read(&to)? {
                    continue;
                }
                let (from, to) = (shot::read(&from)?, shot::read(&to)?);
                if from == to {
                    // The same pixels, encoded differently.
                    continue;
                }
                let difference = out.join(CHANGED).join(path);
                let differ = shot::save_difference(&difference, &from, &to)?;
                let sizes = if (from.width, from.height) == (to.width, to.height) {
                    format!("{} of {}", differ, to.width * to.height)
                } else {
                    format!(
                        "{differ} ({}×{}, then {}×{})",
                        from.width, from.height, to.width, to.height
                    )
                };
                writeln!(report, "{CHANGED}\t{shown}\t{differ}").expect("a String grows");
                format!(
                    "changed: {shown}: {sizes} pixels differ -> {}",
                    difference.display()
                )
            }
            (false, true) => {
                let copy = out.join(ADDED).join(path);
                copy_to(&after.join(path), &copy)?;
                writeln!(report, "{ADDED}\t{shown}\t").expect("a String grows");
                format!(
                    "added: {shown} (only in {}) -> {}",
                    after.display(),
                    copy.display()
                )
            }
            (true, false) => {
                let copy = out.join(REMOVED).join(path);
                copy_to(&before.join(path), &copy)?;
                writeln!(report, "{REMOVED}\t{shown}\t").expect("a String grows");
                format!(
                    "removed: {shown} (only in {}) -> {}",
                    before.display(),
                    copy.display()
                )
            }
            (false, false) => unreachable!("the union has only what either has"),
        };
        println!("{line}");
        differences += 1;
    }

    std::fs::create_dir_all(out).map_err(|e| format!("creating {}: {e}", out.display()))?;
    let path = out.join(REPORT);
    std::fs::write(&path, &report).map_err(|e| format!("writing {}: {e}", path.display()))?;
    if differences == 0 {
        println!(
            "{} and {}: the same {} screenshots",
            before.display(),
            after.display(),
            new.len()
        );
    }
    Ok(differences == 0)
}

/// Every PNG under `root`, by its path below it.
fn pngs(root: &Path) -> Result<BTreeSet<PathBuf>, String> {
    let mut found = BTreeSet::new();
    let mut dirs = vec![PathBuf::new()];
    while let Some(dir) = dirs.pop() {
        let entries = std::fs::read_dir(root.join(&dir))
            .map_err(|e| format!("listing {}: {e}", root.join(&dir).display()))?;
        for entry in entries {
            let entry = entry.map_err(|e| format!("listing {}: {e}", root.display()))?;
            let path = dir.join(entry.file_name());
            let kind = entry
                .file_type()
                .map_err(|e| format!("looking at {}: {e}", root.join(&path).display()))?;
            if kind.is_dir() {
                dirs.push(path);
            } else if path.extension().is_some_and(|x| x == "png") {
                found.insert(path);
            }
        }
    }
    Ok(found)
}

fn read(path: &Path) -> Result<Vec<u8>, String> {
    std::fs::read(path).map_err(|e| format!("reading {}: {e}", path.display()))
}

fn copy_to(from: &Path, to: &Path) -> Result<(), String> {
    let fail = |e: std::io::Error| format!("copying {} to {}: {e}", from.display(), to.display());
    if let Some(dir) = to.parent() {
        std::fs::create_dir_all(dir).map_err(fail)?;
    }
    std::fs::copy(from, to).map(|_| ()).map_err(fail)
}
