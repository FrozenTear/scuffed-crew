//! Every file this crate compiles in with `include_bytes!` or
//! `include_str!` must be tracked by git. The root `.gitignore` ignores
//! `*.png` (with an exception for `crates/stat-tracker-ui/assets/`), so an
//! icon placed elsewhere, or one never added, can build in a working tree
//! and be missing from every clean checkout (CI, a release, the AUR
//! build). This test fails in the working tree instead. Outside a git
//! checkout (a source tarball) it has nothing to check and passes.

use std::path::{Path, PathBuf};
use std::process::Command;

/// The literal path argument of every `include_bytes!` and `include_str!`
/// call in `source`; a call may break the line after its parenthesis.
fn include_paths(source: &str) -> Vec<String> {
    let mut paths = Vec::new();
    // Split so this file's own source holds no call to find.
    for mac in [concat!("include_bytes", "!("), concat!("include_str", "!(")] {
        let mut rest = source;
        while let Some(at) = rest.find(mac) {
            rest = &rest[at + mac.len()..];
            let arg = rest.trim_start();
            if let Some(quoted) = arg.strip_prefix('"')
                && let Some(end) = quoted.find('"')
            {
                paths.push(quoted[..end].to_string());
            }
        }
    }
    paths
}

fn tracked(repo_file: &Path) -> Option<bool> {
    let dir = repo_file.parent()?;
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["ls-files", "--error-unmatch", "--"])
        .arg(repo_file.file_name()?)
        .output()
        .ok()?;
    Some(out.status.success())
}

fn in_git_checkout(dir: &Path) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["rev-parse", "--is-inside-work-tree"])
        .output()
        .is_ok_and(|out| out.status.success())
}

#[test]
fn the_include_parser_finds_both_macros() {
    let source = "a(include_bytes!(\"../x.png\"));\nlet t = include_str!(\n    \"y.md\",\n);";
    assert_eq!(include_paths(source), ["../x.png", "y.md"]);
}

#[test]
fn every_included_file_is_tracked_by_git() {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    if !in_git_checkout(&src) {
        return;
    }
    let mut untracked = Vec::new();
    for entry in std::fs::read_dir(&src).expect("src") {
        let file = entry.expect("entry").path();
        if file.extension().is_none_or(|ext| ext != "rs") {
            continue;
        }
        let source = std::fs::read_to_string(&file).expect("source");
        for rel in include_paths(&source) {
            let included = src.join(&rel);
            if tracked(&included) != Some(true) {
                untracked.push(format!("{} -> {rel}", file.display()));
            }
        }
    }
    assert!(
        untracked.is_empty(),
        "compiled in but not tracked by git (git add it):\n{}",
        untracked.join("\n")
    );
}
