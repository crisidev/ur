//! That every theme is listed everywhere it has to be, checked as the
//! firmware and the simulator are built: not a module of the firmware,
//! but of both build scripts (`build.rs`, `sim/build.rs`), which include
//! it.
//!
//! For each theme's directory, `src/themes/<name>/`:
//!
//! - its line in `src/themes/mod.rs`'s `themes!`, `<name>: <Type> if
//!   "theme-<name>";` (the compiler checks the rest: the `pub mod` line
//!   and the type, which the line names);
//! - its feature, `theme-<name>`, in the firmware's Cargo.toml and the
//!   simulator's, and in both their `themes-all`.
//!
//! And the other way: a `themes!` line or a `theme-*` feature with no
//! directory. Without these checks, a theme left out of `themes-all`, or
//! out of the simulator's features, would build and quietly never be
//! drawn, checked, or published; one left out of `themes!` the same, its
//! code compiled for nothing.
//!
//! A theme draws every screen itself: no `theme-*` feature may turn on
//! another (`themes-all` aside), as it would to use another theme's
//! code. (Without the feature, the other theme's code isn't there, and
//! the build of the theme alone, as its package is built, fails.) More
//! schemes of a theme are part of it, under its own feature: a module
//! of its (retro's in `src/themes/retro/schemes/`), not a feature.
//!
//! The manifests are read line by line, as `[features]` has them here: a
//! feature on one line, `name = ["a", "b"]`.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

/// Every problem with how the themes are listed, under `root` (the
/// repository), each a sentence saying what to add where; none if they're
/// all in place.
pub fn check(root: &Path) -> Vec<String> {
    let read = |path: &str| {
        fs::read_to_string(root.join(path)).unwrap_or_else(|e| panic!("reading {path}: {e}"))
    };
    let mut problems = Vec::new();

    let mut directories: Vec<String> = fs::read_dir(root.join("src/themes"))
        .expect("src/themes lists")
        .map(|entry| entry.expect("src/themes lists").path())
        .filter(|path| path.join("mod.rs").is_file())
        .map(|path| {
            path.file_name()
                .expect("a directory")
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    directories.sort();

    // `themes! {$` … `}`: `retro: Retro if "theme-retro";`, a line each.
    let listed: BTreeMap<String, String> = {
        let source = read("src/themes/mod.rs");
        let start = source
            .find("\nthemes! {$")
            .expect("src/themes/mod.rs invokes `themes!`");
        let body = &source[start..];
        let body = &body[..body.find("\n}").expect("`themes!` ends")];
        body.lines()
            .skip(1)
            .filter_map(|line| {
                let (module, rest) = line.trim().split_once(':')?;
                let feature = rest.split('"').nth(1)?;
                Some((module.trim().to_owned(), feature.to_owned()))
            })
            .collect()
    };

    let manifests = [
        ("Cargo.toml", "the firmware's"),
        ("sim/Cargo.toml", "the simulator's"),
    ];
    let features: Vec<BTreeMap<String, Vec<String>>> = manifests
        .iter()
        .map(|(path, _)| features(&read(path)))
        .collect();

    for name in &directories {
        let feature = format!("theme-{name}");
        match listed.get(name) {
            None => problems.push(format!(
                "src/themes/{name}/ isn't in `themes!` (src/themes/mod.rs): add `{name}: \
                 <its type> if \"{feature}\";`"
            )),
            Some(listed) if *listed != feature => problems.push(format!(
                "`themes!` (src/themes/mod.rs) has {name} under \"{listed}\": its feature \
                 is \"{feature}\""
            )),
            Some(_) => {}
        }
        for ((path, whose), features) in manifests.iter().zip(&features) {
            if !features.contains_key(&feature) {
                problems.push(format!(
                    "src/themes/{name}/ has no feature in {whose} {path}: add `{feature} = []` \
                     under [features]"
                ));
            }
            if !features
                .get("themes-all")
                .is_some_and(|all| all.contains(&feature))
            {
                problems.push(format!(
                    "{whose} {path} leaves {feature} out of `themes-all`: add \"{feature}\" \
                     to it"
                ));
            }
        }
    }
    for name in listed.keys().filter(|name| !directories.contains(name)) {
        problems.push(format!(
            "`themes!` (src/themes/mod.rs) lists {name}, but there's no src/themes/{name}/mod.rs"
        ));
    }
    for ((path, whose), features) in manifests.iter().zip(&features) {
        for (feature, enables) in features {
            let Some(name) = feature.strip_prefix("theme-") else {
                continue;
            };
            if !directories.iter().any(|d| d == name) {
                problems.push(format!(
                    "{whose} {path} has {feature}, but there's no src/themes/{name}/mod.rs"
                ));
            }
            for other in enables.iter().filter(|f| f.starts_with("theme-")) {
                let theme = other.trim_start_matches("theme-");
                problems.push(format!(
                    "{whose} {path}: {feature} turns on {other}, but a theme draws every screen \
                     itself, without another theme's code: start from a copy of Retro's instead \
                     (see AUTHORING-THEMES.md); or, for more colour schemes of {theme}, add them \
                     to {theme}, under {other} (they need no feature of their own)"
                ));
            }
        }
    }
    problems
}

/// A manifest's `[features]`: each name, with what it turns on.
pub fn features(manifest: &str) -> BTreeMap<String, Vec<String>> {
    let mut features = BTreeMap::new();
    let mut in_features = false;
    for line in manifest.lines().map(str::trim) {
        if line.starts_with('[') {
            in_features = line == "[features]";
            continue;
        }
        if line.starts_with('#') {
            continue;
        }
        let Some((name, value)) = line.split_once('=').filter(|_| in_features) else {
            continue;
        };
        let value = value.trim().trim_start_matches('[').trim_end_matches(']');
        let enables = value
            .split(',')
            .map(|f| f.trim().trim_matches('"').to_owned())
            .filter(|f| !f.is_empty())
            .collect();
        features.insert(name.trim().to_owned(), enables);
    }
    features
}
