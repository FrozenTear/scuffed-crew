//! Release pins that read files of this package and its version, and the
//! guard that the version constants built in stat-tracker-core carry it.
//!
//! `README.md`, `dist/bootstrap.sh` and `CHANGELOG.md` sit next to this
//! crate's `Cargo.toml`, and `CARGO_PKG_VERSION` here is the release
//! version (`dist/check-tag-version.sh` checks the tag against it). These
//! tests therefore live in this package, not in a crate below it, where
//! the relative paths and the version would be another crate's.

/// `needle` as a whole word on some line: nothing but whitespace (or
/// the line's start or end) on either side. `TAG=…-v0.5.0` then does
/// not match a line that pins `…-v0.5.0-rc.7` (S5-REL-3).
fn has_whole_word_line(text: &str, needle: &str) -> bool {
    text.lines().any(|line| {
        line.match_indices(needle).any(|(at, _)| {
            let before = line[..at].chars().next_back();
            let after = line[at + needle.len()..].chars().next();
            before.is_none_or(char::is_whitespace) && after.is_none_or(char::is_whitespace)
        })
    })
}

/// A `## {version}` heading line, exactly (trailing spaces aside).
fn has_changelog_section(text: &str, version: &str) -> bool {
    let heading = format!("## {version}");
    text.lines().any(|line| line.trim_end() == heading)
}

#[test]
fn the_pin_matchers_take_whole_lines_only() {
    let pin = "TAG=stat-tracker-v0.5.0";
    assert!(has_whole_word_line("TAG=stat-tracker-v0.5.0\n", pin));
    assert!(has_whole_word_line("#   TAG=stat-tracker-v0.5.0\n", pin));
    assert!(has_whole_word_line(
        "  TAG=stat-tracker-v0.5.0 bash x.sh\n",
        pin
    ));
    assert!(!has_whole_word_line("TAG=stat-tracker-v0.5.0-rc.7\n", pin));
    assert!(!has_whole_word_line(
        "#   TAG=stat-tracker-v0.5.0-rc.7\n",
        pin
    ));
    assert!(!has_whole_word_line("XTAG=stat-tracker-v0.5.0\n", pin));
    assert!(has_changelog_section("# Log\n\n## 0.5.0\n", "0.5.0"));
    assert!(has_changelog_section("## 0.5.0  \n", "0.5.0"));
    assert!(!has_changelog_section("## 0.5.0-rc.7\n", "0.5.0"));
    assert!(!has_changelog_section("### 0.5.0\n", "0.5.0"));
    assert!(!has_changelog_section("text ## 0.5.0\n", "0.5.0"));
}

#[test]
fn readme_and_bootstrap_examples_pin_this_crate_version() {
    let tag = format!("stat-tracker-v{}", env!("CARGO_PKG_VERSION"));
    let readme = include_str!("../README.md");
    let bootstrap = include_str!("../dist/bootstrap.sh");
    assert!(
        has_whole_word_line(readme, &format!("TAG={tag}")),
        "README pin example is stale; want TAG={tag}"
    );
    assert!(
        has_whole_word_line(bootstrap, &format!("TAG={tag}")),
        "bootstrap.sh pin example is stale; want TAG={tag}"
    );
    let changelog = include_str!("../CHANGELOG.md");
    let version = env!("CARGO_PKG_VERSION");
    assert!(
        has_changelog_section(changelog, version),
        "CHANGELOG is missing a section for {version}"
    );
}

/// `APP_VERSION` (report bundles, `perf.log`) and `USER_AGENT` (every site
/// request) are built in stat-tracker-core from its `CARGO_PKG_VERSION`.
/// That must be this package's version, the release version, so a bump of
/// `crates/stat-tracker/Cargo.toml` that misses
/// `crates/stat-tracker-core/Cargo.toml` fails here (PR #452 review, F3).
#[test]
fn the_version_constants_are_this_release() {
    let version = env!("CARGO_PKG_VERSION");
    assert_eq!(
        crate::version::APP_VERSION,
        version,
        "crates/stat-tracker-core/Cargo.toml must carry the release version"
    );
    assert_eq!(
        crate::version::USER_AGENT,
        format!("scuffed-stat-tracker/{version}")
    );
}
