//! Outcome-detection replay over locally captured frames.
//!
//! Drop screenshots into `tests/fixtures/outcomes/` (repo root, gitignored like
//! the replay fixtures) named by expected outcome:
//!   victory_*.png  defeat_*.png  draw_*.png   — accolade / banner frames
//!   none_*.png                                — frames that must NOT detect an
//!                                               outcome (desktop, mid-game, menus)
//!
//! Run: cargo test -p scuffed-stat-tracker --test outcome_fixtures -- --ignored

use stat_tracker::detect::MatchOutcome;
use stat_tracker::detect::match_end::detect_outcome;

#[test]
#[ignore = "requires local outcome screenshots in tests/fixtures/outcomes/ (not committed)"]
fn outcome_fixture_replay() {
    let dir = format!(
        "{}/../../tests/fixtures/outcomes",
        env!("CARGO_MANIFEST_DIR")
    );
    let entries = std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("fixtures dir {dir}: {e}"));

    let mut checked = 0;
    let mut failures = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("png") {
            continue;
        }
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let expected = if name.starts_with("victory_") {
            MatchOutcome::Victory
        } else if name.starts_with("defeat_") {
            MatchOutcome::Defeat
        } else if name.starts_with("draw_") {
            MatchOutcome::Draw
        } else if name.starts_with("none_") {
            MatchOutcome::Unknown
        } else {
            eprintln!("skipping {name}: no outcome prefix");
            continue;
        };

        let img = image::open(&path).unwrap_or_else(|e| panic!("failed to open {name}: {e}"));
        let got = detect_outcome(&img);
        if got != expected {
            failures.push(format!("{name}: expected {expected:?}, got {got:?}"));
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
