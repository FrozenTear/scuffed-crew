//! Player-row resolution replay over locally captured 6v6 scoreboards.
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
