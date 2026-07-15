//! Team-size detection replay over locally captured scoreboards.
//!
//! Ground truths are pinned per file below; frames live in the gitignored
//! local fixture dirs, so missing files are skipped (but detecting zero
//! frames fails the run).
//!
//! Run: cargo test -p scuffed-stat-tracker --test team_size_fixtures -- --ignored

use stat_tracker::detect::hero_portrait::detect_team_size;
use stat_tracker::ocr::preprocess::crop_scoreboard;

/// (path relative to the crate dir, expected team size)
const BOARDS: &[(&str, usize)] = &[
    // 0:00 all-zero 6v6 board — the sparse case dip spacing misread as 5v5.
    (
        "test-data/ban-phase/05_scoreboard_empty_shambali_bans.png",
        6,
    ),
    (
        "test-data/ban-phase/06_scoreboard_midgame_wreckingball_panel.png",
        6,
    ),
    ("test-data/ashe_korean_6v6.png", 6),
    ("test-data/replay_scoreboard.png", 6),
    // Dip pitch measured 0.0788 here — within a hair of the 0.079 threshold.
    ("test-data/post_match_summary.png", 6),
    ("test-data/wiki_scoreboard.png", 5),
    ("test-data/Innlimt bilde.png", 6),
    ("test-data/Innlimt bilde (2).png", 6),
];

#[test]
#[ignore = "requires local scoreboard screenshots in test-data/ (not committed)"]
fn team_size_fixture_replay() {
    let mut checked = 0;
    let mut failures = Vec::new();
    for &(rel, expected) in BOARDS {
        let path = format!("{}/{rel}", env!("CARGO_MANIFEST_DIR"));
        let Ok(img) = image::open(&path) else {
            eprintln!("skipping {rel}: not present locally");
            continue;
        };
        let got = detect_team_size(&crop_scoreboard(&img));
        if got != expected {
            failures.push(format!("{rel}: expected {expected}, got {got}"));
        }
        checked += 1;
    }

    assert!(checked > 0, "no team-size fixture frames found");
    assert!(
        failures.is_empty(),
        "{}/{checked} boards misdetected:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
