//! Scuffed Vision baseline findings on PR 158, pinned on fixtures from
//! `test-data/vision-baseline/` (see the README there). The three full
//! boards are synthetic. The stat cells are name-free crops.
//!
//! 1. Bottom-row cells main reads came back empty, and a 0.75x board lost
//!    10 cells to a different column layout.
//! 2. A recovered dim zero carried a flat confidence of 60.
//! 3. The centred post-game table was read as 5v5, which cut every row one
//!    slot off.

use image::DynamicImage;
use stat_tracker::frame::layout::RowScan;
use stat_tracker::ocr::{self, preprocess};

use crate::tess_engine::{known_variant, tesseract_version};

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
    // Synthetic 1248x756 table. These 10 cells are the column-layout check:
    // a shifted grid drops them. Row 3 column A is 7 because a lone 1 in
    // that slot does not OCR.
    let board = fixture("s075_tab_board_stats_only.png");
    let rows = ocr::recognize_scoreboard_cells_pre_cropped(&board, 6);
    assert_eq!(rows.len(), 12);
    let expect: [(usize, usize, &str); 10] = [
        (0, 0, "6"),
        (1, 1, "0"),
        (1, 4, "0"),
        (2, 0, "5"),
        (3, 1, "7"),
        (3, 4, "0"),
        (3, 5, "84"),
        (5, 0, "8"),
        (7, 0, "5"),
        (8, 1, "1"),
    ];
    let failures: Vec<String> = expect
        .iter()
        .filter_map(|&(row, col, want)| {
            let got = digits(&rows[row].stats[col].value);
            let cell = format!("s075_tab_board_stats_only.png r{row}c{col}");
            (got != want && !known_variant(&cell, &got))
                .then(|| format!("row {row} col {col}: expected {want}, got {got:?}"))
        })
        .collect();
    assert!(
        failures.is_empty(),
        "Tesseract {}:\n{}",
        tesseract_version(),
        failures.join("\n")
    );
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
        let own_read = known_variant(&format!("{file} confidence"), &got.confidence.to_string());
        if got.confidence != hit.confidence && !own_read {
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
    assert!(
        failures.is_empty(),
        "Tesseract {}:\n{}",
        tesseract_version(),
        failures.join("\n")
    );
}

/// Synthetic post-match grid: rows 0-5 then 6-11, columns E, A, D, DMG, H, MIT.
const POSTGAME: [[u32; 6]; 12] = [
    [12, 3, 3, 5480, 950, 2347],
    [7, 1, 7, 3789, 0, 82],
    [7, 8, 7, 1858, 3387, 110],
    [7, 15, 6, 1545, 5407, 0],
    [15, 3, 5, 7283, 332, 24],
    [19, 0, 5, 4922, 987, 5785],
    [21, 8, 3, 7161, 1540, 6914],
    [15, 1, 4, 4812, 142, 0],
    [16, 5, 5, 5643, 952, 1685],
    [14, 8, 2, 1762, 1645, 123],
    [15, 15, 3, 3448, 6595, 0],
    [6, 5, 4, 2430, 1382, 203],
];

/// Exact cells and exact DMG cells against [`POSTGAME`]. With a `fixture`
/// name, a cell the running engine is listed as reading differently (see
/// [`crate::tess_engine`]) counts as exact when it reads exactly that.
fn postgame_matches(rows: &[ocr::RowOcrResult], fixture: Option<&str>) -> (usize, usize) {
    let (mut exact, mut dmg) = (0, 0);
    for (r, (row, want)) in rows.iter().zip(POSTGAME.iter()).enumerate() {
        for (col, &value) in want.iter().enumerate() {
            let hit = row.stats.get(col).is_some_and(|c| {
                let got = digits(&c.value);
                got == value.to_string()
                    || fixture.is_some_and(|f| known_variant(&format!("{f} r{r}c{col}"), &got))
            });
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
    // Row scans measured on the old frame. The synthetic boards have no name
    // dips, so the test injects that scan. The dips give the 6v6 pitch. The
    // spectral peak is neither layout. Template digits are not the game
    // font: the floor is this board's own read. A 5v5 read must not line
    // the DMG column up.
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
            66,
            11,
        ),
        ("s075_postgame_table_names_blanked.png", s075_scan, 49, 9),
    ] {
        let team = scan.team_size();
        assert_eq!(team, 6, "{file}: the measured scan must say 6v6");
        let board = fixture(file);
        let rows = ocr::recognize_scoreboard_cells_pre_cropped(&board, team);
        assert_eq!(rows.len(), 12, "{file}");
        let (exact, dmg) = postgame_matches(&rows, Some(file));
        let misses: Vec<String> = rows
            .iter()
            .zip(POSTGAME.iter())
            .enumerate()
            .flat_map(|(r, (row, want))| {
                want.iter().enumerate().filter_map(move |(c, value)| {
                    let got = row.stats.get(c).map(|cell| digits(&cell.value));
                    (got.as_deref() != Some(value.to_string().as_str()))
                        .then(|| format!("r{r}c{c} want {value} got {got:?}"))
                })
            })
            .collect();
        eprintln!(
            "{file} on Tesseract {}: {exact}/72 exact or listed, DMG {dmg}/12, misses:\n{}",
            tesseract_version(),
            misses.join("\n")
        );
        let shifted = ocr::recognize_scoreboard_cells_pre_cropped(&board, 5);
        let (_, dmg5) = postgame_matches(&shifted, None);
        assert!(
            exact >= min_exact && dmg >= min_dmg,
            "{file} on Tesseract {}: {exact}/72 exact or listed (want {min_exact}), DMG {dmg}/12 (want {min_dmg})\n{}",
            tesseract_version(),
            misses.join("\n")
        );

        // Control: read as 5v5, the rows land a slot off and DMG no longer
        // lines up. That is the shifted read the capture path must never
        // store.
        assert!(dmg5 <= 3, "{file}: 5v5 read matched DMG on {dmg5} rows");
    }
}
