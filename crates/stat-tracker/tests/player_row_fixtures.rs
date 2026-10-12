//! Player-row resolution replay over locally captured 6v6 scoreboards, and
//! over private 5v5 boards named by an env var (see
//! [`player_row_resolves_by_name_in_5v5`]).
//!
//! Regression for 2026-07-16 "games didn't register": the fixed 5v5 name
//! window read stat digits in the 6v6 layout, and cosmetic nameplates
//! defeated the HSV white mask — every capture rejected as noplayerrow.
//!
//! Frames live in the gitignored local fixture dir; missing files are
//! skipped (but resolving zero frames fails the run).
//!
//! Run: cargo test -p scuffed-stat-tracker --test player_row_fixtures -- --ignored

use stat_tracker::ocr;
use stat_tracker::ocr::preprocess::crop_scoreboard;
use stat_tracker::parse;

/// (path relative to the crate dir, player name, expected row index)
const BOARDS: &[(&str, &str, usize)] = &[
    // Clean 6v6 magenta-palette board; cosmetic nameplate needs the
    // hard-threshold fallback ("M- FrozeN").
    (
        "test-data/6v6-nameplate/push_esperanca_frozen_row0.png",
        "Frozen",
        0,
    ),
    // Same session, blurrier read ("frozey") — exercises the fuzzy matcher.
    ("test-data/6v6-nameplate/fuzzy_frozey_row0.png", "Frozen", 0),
];

#[test]
#[ignore = "needs local fixture frames"]
fn player_row_resolves_by_name_in_6v6() {
    let mut checked = 0usize;
    for (path, name, expected_row) in BOARDS {
        let Ok(img) = image::open(path) else {
            eprintln!("skip (missing): {path}");
            continue;
        };
        let board = crop_scoreboard(&img);
        let rows = ocr::recognize_scoreboard_cells_pre_cropped(&board, 6);
        let row = parse::find_player_row_by_name(&rows, name);
        assert_eq!(
            row,
            Some(*expected_row),
            "player row mismatch for {path}: rows read as {:?}",
            rows.iter()
                .map(|r| r.name.as_ref().map(|c| c.value.clone()))
                .collect::<Vec<_>>()
        );
        checked += 1;
    }
    assert!(checked > 0, "no fixture frames found — nothing was tested");
}

/// Directory of private 5v5 boards: full 2560x1440 frames or 1664x1007
/// scoreboard crops. They show player names, so they never enter the repo.
/// Put only Tab scoreboards in it: the pre-flight alone lets some gameplay
/// frames through, and the capture loop rejects those elsewhere.
const FRAMES_5V5_ENV: &str = "SCUFFED_5V5_FRAMES";
/// The configured player name for those boards (never hardcoded here).
const NAME_ENV: &str = "SCUFFED_PLAYER_NAME";
/// The own row on those boards. Defaults to 0 (team 1, top row).
const ROW_ENV: &str = "SCUFFED_5V5_OWN_ROW";

/// 2026-10-12 "own row not found" on 5v5: the single name window fitted only
/// the centred pre-match table, and on the in-match board it read the E/A
/// digits or the perk icons on every row.
///
/// Run: SCUFFED_5V5_FRAMES=<dir> SCUFFED_PLAYER_NAME=<name> \
///   cargo test -p scuffed-stat-tracker --test player_row_fixtures -- --ignored
#[test]
#[ignore = "needs private 5v5 frames (SCUFFED_5V5_FRAMES, SCUFFED_PLAYER_NAME)"]
fn player_row_resolves_by_name_in_5v5() {
    use stat_tracker::detect::hero_portrait::{ScoreboardPreflight, preflight_scoreboard};

    let dir = std::env::var(FRAMES_5V5_ENV)
        .unwrap_or_else(|_| panic!("set {FRAMES_5V5_ENV} to a directory of 5v5 boards"));
    let name = std::env::var(NAME_ENV)
        .unwrap_or_else(|_| panic!("set {NAME_ENV} to the player name on those boards"));
    let expected: usize = std::env::var(ROW_ENV)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);

    let mut paths: Vec<_> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{dir}: {e}"))
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("png"))
        .collect();
    paths.sort();

    let mut checked = 0usize;
    let mut missed = Vec::new();
    for path in &paths {
        let Ok(img) = image::open(path) else {
            continue;
        };
        let board = if (img.width(), img.height()) == (1664, 1007) {
            img
        } else {
            crop_scoreboard(&img)
        };
        // Frames the tracker would not read as a 5v5 board (menus, the
        // pre-flight rejects) are not part of this check.
        if !matches!(
            preflight_scoreboard(&board).0,
            ScoreboardPreflight::Ready(5)
        ) {
            eprintln!("skip (not a 5v5 board): {}", path.display());
            continue;
        }
        let rows = ocr::recognize_scoreboard_cells_pre_cropped(&board, 5);
        let row = parse::find_player_row_by_name(&rows, &name);
        if row != Some(expected) {
            // File name only: the reads would print player names.
            missed.push(format!(
                "{} -> {row:?}",
                path.file_name().unwrap().to_string_lossy()
            ));
        }
        checked += 1;
    }
    assert!(checked > 0, "no 5v5 boards found in {dir}");
    assert!(
        missed.is_empty(),
        "own row {expected} not found by name on {} of {checked} boards: {missed:?}",
        missed.len()
    );
}
