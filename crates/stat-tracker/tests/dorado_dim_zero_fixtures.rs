//! Dim-zero band check on the 2026-10-08 Dorado captures.
//!
//! The dim-zero check is a neutral-and-below-white rule (value 96..=220,
//! saturation at most 40), not one grey level. This replay
//! is how a local copy of those frames confirms a real dim zero still reads
//! as 0 and a shifted assists/deaths pair does not come back.
//!
//! Frames stay gitignored under `test-data/dorado-20261008/` (copyrighted
//! game captures). Drop the full Tab screenshots there.
//!
//! Run: cargo test -p scuffed-stat-tracker --test dorado_dim_zero_fixtures -- --ignored

use stat_tracker::ocr;
use stat_tracker::parse;

#[test]
#[ignore = "needs local fixture frames"]
fn dorado_frames_read_dim_zeros_inside_the_kill_ceilings() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("test-data/dorado-20261008");
    let entries = std::fs::read_dir(&dir)
        .unwrap_or_else(|_| panic!("no fixture frames in {}: nothing was tested", dir.display()));
    let mut checked = 0usize;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("png") {
            continue;
        }
        let img = image::open(&path).unwrap_or_else(|err| panic!("open {}: {err}", path.display()));
        let rows = ocr::recognize_scoreboard_cells(&img);
        let Some(idx) = parse::find_player_row_by_name(&rows, "FROZEN") else {
            eprintln!("skip (no player row): {}", path.display());
            continue;
        };
        let row = &rows[idx];
        let cells: Vec<&str> = row
            .stats
            .iter()
            .take(6)
            .map(|cell| cell.value.as_str())
            .collect();
        assert!(
            cells.len() == 6 && cells.iter().all(|value| !value.is_empty()),
            "{} player row has an empty cell: {cells:?}",
            path.display()
        );
        let numbers: Vec<u32> = cells
            .iter()
            .map(|value| {
                value
                    .chars()
                    .filter(|c| c.is_ascii_digit())
                    .collect::<String>()
                    .parse()
                    .unwrap_or_else(|_| panic!("{} cell {value} is not a number", path.display()))
            })
            .collect();
        assert!(
            numbers[0] <= 99 && numbers[1] <= 99 && numbers[2] <= 50,
            "{} kill columns out of range: {numbers:?}",
            path.display()
        );
        for (col, value) in cells.iter().enumerate() {
            if numbers[col] == 0 {
                assert_eq!(
                    *value,
                    "0",
                    "{} col {col} parsed as 0 but the cell text is {value:?}",
                    path.display()
                );
            }
        }
        // The early board on this night is E2 A0 D0 DMG1105 H259 MIT450.
        // A dim zero read as 8 still sits inside the ceilings, so those
        // two zero cells have to be the string "0".
        if numbers[3] == 1105 && numbers[4] == 259 {
            assert_eq!(cells[0], "2", "{}", path.display());
            assert_eq!(cells[1], "0", "assists zero {}", path.display());
            assert_eq!(cells[2], "0", "deaths zero {}", path.display());
            assert_eq!(cells[5], "450", "{}", path.display());
        }
        if numbers[3] == 3993 && numbers[4] == 989 && numbers[5] == 1583 {
            assert_eq!((numbers[0], numbers[1], numbers[2]), (8, 1, 2));
        }
        checked += 1;
    }
    assert!(checked > 0, "no fixture frames found: nothing was tested");
}
