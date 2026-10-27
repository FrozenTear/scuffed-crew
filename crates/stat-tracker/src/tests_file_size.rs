//! File-size guard for the three tracker crates (0.6 G1).
//!
//! Every `.rs` file under `src/`, `tests/` and `examples/` of
//! `stat-tracker`, `stat-tracker-core` and `stat-tracker-ui` must stay at
//! [`LIMIT`] lines or under. The files already over it when the guard
//! landed are listed in [`CAPS`] at their line count then, and may only
//! shrink: a listed file over its cap fails, and so does one under it, with
//! the cap to lower it to. Once a listed file is at [`LIMIT`] or under, it
//! leaves the list. So no file grows past its size today, and a new file
//! starts under the limit.
//!
//! Lines are counted as `str::lines` counts them.

use std::path::{Path, PathBuf};

/// The most lines a file not in [`CAPS`] may have.
const LIMIT: usize = 1_500;

/// The crates under `crates/` the guard covers, and the directories in each.
const CRATES: &[&str] = &["stat-tracker", "stat-tracker-core", "stat-tracker-ui"];
const DIRS: &[&str] = &["src", "tests", "examples"];

/// Files over [`LIMIT`], each at its line count when the guard landed or
/// last shrank, by path under `crates/`, sorted. Lower a cap when its file
/// shrinks; never raise one.
const CAPS: &[(&str, usize)] = &[
    ("stat-tracker-ui/src/app.rs", 3512),
    ("stat-tracker-ui/src/instance.rs", 1690),
    ("stat-tracker-ui/src/notes.rs", 1598),
    ("stat-tracker-ui/src/report.rs", 2451),
    ("stat-tracker-ui/src/settings.rs", 3791),
    ("stat-tracker-ui/src/setup_guide.rs", 3063),
    ("stat-tracker-ui/src/uninstall.rs", 2507),
    ("stat-tracker-ui/src/update.rs", 3324),
    ("stat-tracker/src/capture/stream/mod.rs", 2399),
    ("stat-tracker/src/capture/stream/pw/mod.rs", 2612),
    ("stat-tracker/src/detect/hero_portrait.rs", 1511),
    ("stat-tracker/src/detect/match_end.rs", 1755),
    ("stat-tracker/src/detect/own_row.rs", 1880),
    ("stat-tracker/src/detect/own_row_tests.rs", 2446),
    ("stat-tracker/src/detect/tests_match_end.rs", 1994),
    ("stat-tracker/src/main_tests/mod.rs", 2236),
    ("stat-tracker/src/main_tests/shutdown_and_restart.rs", 1502),
    ("stat-tracker/src/ocr/mod.rs", 2755),
    ("stat-tracker/src/ocr/preprocess.rs", 2336),
    ("stat-tracker/src/packs/mod.rs", 6811),
    ("stat-tracker/src/reader_apply.rs", 1597),
    ("stat-tracker/src/report/blank/reader.rs", 1623),
    ("stat-tracker/src/report/scrub.rs", 2292),
    ("stat-tracker/src/report/tests.rs", 1662),
    ("stat-tracker/src/shadow/banner.rs", 2680),
    ("stat-tracker/src/shadow/digits.rs", 2826),
    ("stat-tracker/src/shadow/heroes.rs", 2538),
    ("stat-tracker/src/shadow/reader.rs", 2576),
    ("stat-tracker/src/shadow/result.rs", 4141),
    ("stat-tracker/src/tests_layering.rs", 2071),
];

/// Every `.rs` file the guard covers, by path under `crates/` with `/`
/// separators, and its line count. `target/` and dot directories are skipped.
fn covered_files() -> Vec<(String, usize)> {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates/ dir");
    let mut files = Vec::new();
    for krate in CRATES {
        for dir in DIRS {
            let root = crates.join(krate).join(dir);
            if root.is_dir() {
                walk(&root, &mut files);
            }
        }
    }
    let mut out: Vec<(String, usize)> = files
        .into_iter()
        .map(|path| {
            let text = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
            let rel = path
                .strip_prefix(crates)
                .expect("under crates/")
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            (rel, text.lines().count())
        })
        .collect();
    out.sort();
    out
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = std::fs::read_dir(dir).unwrap_or_else(|e| panic!("read {}: {e}", dir.display()));
    for entry in entries {
        let path = entry.expect("dir entry").path();
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
        if name
            .as_deref()
            .is_some_and(|n| n.starts_with('.') || n == "target")
        {
            continue;
        }
        if path.is_dir() {
            walk(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn tracker_files_stay_within_their_size_limits() {
    let mut problems = Vec::new();
    for pair in CAPS.windows(2) {
        if pair[0].0 >= pair[1].0 {
            problems.push(format!(
                "CAPS: {} and {} are not sorted, or repeat",
                pair[0].0, pair[1].0
            ));
        }
    }
    for (path, cap) in CAPS {
        if *cap <= LIMIT {
            problems.push(format!(
                "CAPS: {path} has cap {cap}, at or under the {LIMIT} limit"
            ));
        }
    }
    let files = covered_files();
    assert!(
        files.len() > 100,
        "only {} files found: the walk is broken",
        files.len()
    );
    for (path, cap) in CAPS {
        match files.iter().find(|(p, _)| p == path) {
            None => problems.push(format!(
                "{path}: listed in CAPS but missing (renamed or deleted): update CAPS"
            )),
            Some((_, lines)) if lines > cap => problems.push(format!(
                "{path}: {lines} lines, over its cap of {cap}: shrink it, do not raise the cap"
            )),
            Some((_, lines)) if *lines <= LIMIT => problems.push(format!(
                "{path}: {lines} lines, at or under the {LIMIT} limit: remove it from CAPS"
            )),
            Some((_, lines)) if lines < cap => problems.push(format!(
                "{path}: {lines} lines, under its cap of {cap}: lower the cap to {lines}"
            )),
            Some(_) => {}
        }
    }
    for (path, lines) in &files {
        if *lines > LIMIT && !CAPS.iter().any(|(p, _)| p == path) {
            problems.push(format!(
                "{path}: {lines} lines, over the {LIMIT} limit: split it (new files start under the limit)"
            ));
        }
    }
    assert!(problems.is_empty(), "\n{}", problems.join("\n"));
}
