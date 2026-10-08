//! Scuffed Vision baseline findings on PR 158, pinned on name-free fixtures
//! from `test-data/vision-baseline/` (see the README there for every
//! expected value and where each file came from).
//!
//! 1. Bottom-row cells main reads came back empty, and a 0.75x board lost
//!    10 cells to a different column layout.
//! 2. A recovered dim zero carried a flat confidence of 60.
//! 3. The centred post-game table was read as 5v5, which cut every row one
//!    slot off.

use image::DynamicImage;
use stat_tracker::detect::hero_portrait::RowScan;
use stat_tracker::ocr::{self, preprocess};

fn fixture(name: &str) -> DynamicImage {
    let path = format!(
        "{}/test-data/vision-baseline/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    let img = image::open(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    DynamicImage::ImageRgb8(img.to_rgb8())
}

fn digits(s: &str) -> String {
    s.chars().filter(char::is_ascii_digit).collect()
}

#[test]
fn bottom_row_cells_read_like_main() {
    // The bottom row is cut off at the edge of the scoreboard crop, so its
    // cells are 38 px tall at native. Main read these at 64/38. The PR 158
    // cap alone (64/39) left all four empty.
    let cells = [
        ("native_bottom_row_assists_white8_a.png", "8"),
        ("native_bottom_row_assists_white8_b.png", "8"),
        ("native_bottom_row_assists_white8_c.png", "8"),
        ("native_bottom_row_mitigation_white183.png", "183"),
        // 27 px at 0.75x, under the cap's minimum height, so nothing
        // changes there. Kept as a guard on the retry.
        ("s075_bottom_row_assists_white8.png", "8"),
        ("s075_bottom_row_mitigation_white183.png", "183"),
    ];
    let mut failures = Vec::new();
    for (file, expect) in cells {
        let got = ocr::recognize_cell(&fixture(file)).expect("cell OCR");
        if digits(&got.value) != expect {
            failures.push(format!(
                "{file}: expected {expect}, got {:?} (conf {})",
                got.value, got.confidence
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn a_075x_board_keeps_the_cells_main_reads() {
    // Frame 001449 at 0.75x. Counting recovered dim zeros in column
    // calibration pushed its header-anchored layout over the 75% floor, so
    // the offset sweep main runs here never ran, and these 10 cells were
    // lost.
    let board = fixture("s075_tab_board_stats_only.png");
    let rows = ocr::recognize_scoreboard_cells_pre_cropped(&board, 6);
    assert_eq!(rows.len(), 12);
    let expect: [(usize, usize, &str); 10] = [
        (0, 0, "6"),
        (1, 1, "0"),
        (1, 4, "0"),
        (2, 0, "5"),
        (3, 1, "1"),
        (3, 4, "0"),
        (3, 5, "84"),
        (5, 0, "8"),
        (7, 0, "5"),
        (8, 1, "1"),
    ];
    let failures: Vec<String> = expect
        .iter()
        .filter_map(|&(row, col, want)| {
            let got = &rows[row].stats[col];
            (digits(&got.value) != want)
                .then(|| format!("row {row} col {col}: expected {want}, got {:?}", got.value))
        })
        .collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn recovered_dim_zeros_carry_their_geometric_confidence() {
    let cells = [
        "native_purple_deaths_dim0.png",
        "native_purple_healing_dim0.png",
        "native_yellow_mitigation_dim0.png",
        "native_yellow_mitigation_dim0_wide_counter.png",
        "s075_purple_mitigation_dim0.png",
        "s075_yellow_elims_dim0.png",
        "s075_yellow_healing_dim0.png",
    ];
    let mut failures = Vec::new();
    for file in cells {
        let img = fixture(file);
        let Some(hit) = preprocess::dim_zero_glyph(&img, 2) else {
            failures.push(format!("{file}: the geometric check must see the ring"));
            continue;
        };
        let got = ocr::recognize_cell(&img).expect("cell OCR");
        if got.value != "0" || got.suspect {
            failures.push(format!("{file}: expected a clean 0, got {got:?}"));
            continue;
        }
        if got.confidence != hit.confidence {
            failures.push(format!(
                "{file}: confidence {} is not the ring's {} (extent {}%, offset {}%)",
                got.confidence, hit.confidence, hit.hole_extent_pct, hit.centre_offset_pct
            ));
        }
        // A strong ring must not fall under a "conf below 70 is suspect"
        // rule. Every dim zero in the baseline scores 75 or more.
        if got.confidence < 75 {
            failures.push(format!("{file}: confidence {} under 75", got.confidence));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Frame 235921, rows 0-5 purple then 6-11 yellow: E, A, D, DMG, H, MIT.
const POSTGAME: [[u32; 6]; 12] = [
    [12, 3, 3, 5480, 950, 2347],
    [7, 1, 7, 3789, 0, 82],
    [7, 8, 7, 1858, 3387, 110],
    [7, 15, 6, 1545, 5407, 0],
    [15, 0, 5, 7283, 332, 24],
    [9, 0, 5, 4922, 987, 5785],
    [21, 8, 3, 7161, 1540, 6914],
    [15, 1, 4, 4812, 142, 0],
    [16, 5, 5, 5643, 952, 1685],
    [14, 8, 2, 1762, 1605, 123],
    [15, 15, 3, 3448, 6595, 0],
    [6, 5, 4, 2430, 1382, 203],
];

/// Exact cells and exact DMG cells against [`POSTGAME`].
fn postgame_matches(rows: &[ocr::RowOcrResult]) -> (usize, usize) {
    let (mut exact, mut dmg) = (0, 0);
    for (row, want) in rows.iter().zip(POSTGAME.iter()) {
        for (col, &value) in want.iter().enumerate() {
            let hit = row
                .stats
                .get(col)
                .is_some_and(|c| digits(&c.value) == value.to_string());
            if hit {
                exact += 1;
                if col == 3 {
                    dmg += 1;
                }
            }
        }
    }
    (exact, dmg)
}

#[test]
fn the_postgame_table_reads_as_six_rows_a_team() {
    // The real frame's row scans. The dips give the 6v6 pitch, the spectral
    // peak landed on 0.102, which is neither layout. The names are filled
    // on the fixtures, which removes those dips, so the measured scans are
    // what pick the size here.
    let native_scan = RowScan {
        dip_count: 4,
        median_pitch: Some(0.074_478_649_453_823_24),
        spectral_pitch: Some(0.102_284_011_916_583_91),
    };
    let s075_scan = RowScan {
        dip_count: 4,
        median_pitch: Some(0.075_396_825_396_825_39),
        spectral_pitch: Some(0.101_851_851_851_851_85),
    };
    for (file, scan, min_exact, min_dmg) in [
        (
            "native_postgame_table_names_blanked.png",
            native_scan,
            70,
            12,
        ),
        ("s075_postgame_table_names_blanked.png", s075_scan, 55, 10),
    ] {
        let team = scan.team_size();
        assert_eq!(team, 6, "{file}: the measured scan must say 6v6");
        let board = fixture(file);
        let rows = ocr::recognize_scoreboard_cells_pre_cropped(&board, team);
        assert_eq!(rows.len(), 12, "{file}");
        let (exact, dmg) = postgame_matches(&rows);
        assert!(
            exact >= min_exact && dmg >= min_dmg,
            "{file}: {exact}/72 exact (want {min_exact}), DMG {dmg}/12 (want {min_dmg})"
        );

        // Control: read as 5v5, the rows land a slot off and DMG no longer
        // lines up. That is the shifted read the capture path must never
        // store.
        let shifted = ocr::recognize_scoreboard_cells_pre_cropped(&board, 5);
        let (_, dmg5) = postgame_matches(&shifted);
        assert!(dmg5 <= 3, "{file}: 5v5 read matched DMG on {dmg5} rows");
    }
}
