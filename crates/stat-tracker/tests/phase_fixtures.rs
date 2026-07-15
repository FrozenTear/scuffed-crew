//! Phase-detection replay over locally captured frames.
//!
//! Drop screenshots into `tests/fixtures/phases/` (repo root, gitignored like
//! the outcome fixtures) named by expected phase:
//!   mapvote_*.png  ban_*.png  select_*.png   — match-start screens
//!   none_*.png                               — frames that must NOT detect a
//!                                              phase (scoreboards, menus,
//!                                              end-cards, transitions)
//!
//! Run: cargo test -p scuffed-stat-tracker --test phase_fixtures -- --ignored

use stat_tracker::detect::GamePhase;
use stat_tracker::detect::match_start::detect_phase;

#[test]
#[ignore = "requires local phase screenshots in tests/fixtures/phases/ (not committed)"]
fn phase_fixture_replay() {
    let dir = format!("{}/../../tests/fixtures/phases", env!("CARGO_MANIFEST_DIR"));
    let entries = std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("fixtures dir {dir}: {e}"));

    let mut checked = 0;
    let mut failures = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("png") {
            continue;
        }
        let name = path.file_name().unwrap().to_string_lossy().to_string();

        let img = image::open(&path).unwrap_or_else(|e| panic!("failed to open {name}: {e}"));
        let got = detect_phase(&img);
        let ok = if name.starts_with("mapvote_") {
            matches!(got, GamePhase::MapVote { .. })
        } else if name.starts_with("ban_") {
            got == GamePhase::HeroBan
        } else if name.starts_with("select_") {
            got == GamePhase::HeroSelect
        } else if name.starts_with("none_") {
            got == GamePhase::Unknown
        } else {
            eprintln!("skipping {name}: no phase prefix");
            continue;
        };

        if !ok {
            failures.push(format!("{name}: got {got:?}"));
        }
        checked += 1;
    }

    assert!(checked > 0, "no fixture frames found in {dir}");
    assert!(
        failures.is_empty(),
        "{}/{checked} frames misdetected:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
