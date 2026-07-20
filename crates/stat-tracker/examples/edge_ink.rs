//! Throwaway validation harness for the CG-3 edge-ink suspect threshold.
//!
//! Dumps, for one player row on each drift fixture, the per-cell OCR text and the
//! per-side edge-ink fractions (left,right) measured on the borderless binarized
//! cell — the exact signal `ocr::cell_edge_ink` / `preprocess::has_edge_ink` use.
//! Point it at the read-only fixtures to eyeball where clean centred digits sit
//! versus where the drifted/clipped reads spike:
//!
//!   cargo run -p scuffed-stat-tracker --example edge_ink -- \
//!       crates/stat-tracker/test-data/drift-20260720 6 frozen
//!
//! Args: <img-or-dir> [team_size] [player_name]. A directory runs every *.png.

use stat_tracker::ocr::{self, preprocess};

fn dump_one(path: &std::path::Path, team_size: usize, player_name: Option<&str>) {
    let Ok(img) = image::open(path) else {
        println!("{}: <open failed>", path.display());
        return;
    };
    let rows = ocr::recognize_scoreboard_cells_with_team_size(&img, Some(team_size));
    let scoreboard = preprocess::crop_scoreboard(&img);

    let row_idx = player_name
        .and_then(|n| stat_tracker::parse::find_player_row_by_name(&rows, n))
        .unwrap_or(0);

    println!("\n=== {} (row {row_idx}) ===", path.display());
    let Some(row_img) = preprocess::crop_player_row(&scoreboard, row_idx, team_size) else {
        return;
    };

    // Sweep the DMG column (index 3) horizontally in ~one-digit steps and print
    // total ink + edge-ink at each offset. This isolates the CG-3 signal on REAL
    // pixels independent of OCR quality: the offset where the digits are centred
    // shows high ink with LOW edge fraction, and a window drifted by ~one digit
    // spikes the edge fraction as a neighbour glyph is pulled against the margin.
    println!("  DMG-column offset sweep (offset : total_ink  edgeMax):");
    let mut best: Option<(f64, f64, usize)> = None; // (edge_max, offset, ink)
    for step in -8i32..=8 {
        let offset = step as f64 * 0.004;
        let cols = preprocess::columns_with_offset(offset);
        if let Some(cell) = preprocess::crop_stat_cell(&row_img, 3, &cols) {
            let bin = preprocess::prepare_cell_binary(&cell);
            let ink = bin.pixels().filter(|p| p.0[0] < 128).count();
            let (l, r) = preprocess::edge_ink_fraction(&bin, 2);
            let edge_max = l.max(r);
            let flag = if edge_max > 0.12 { " SUSPECT" } else { "" };
            println!("    {offset:+.3} : {ink:>4}   {edge_max:.3}{flag}");
            if ink > 40 && best.is_none_or(|(be, ..)| edge_max < be) {
                best = Some((edge_max, offset, ink));
            }
        }
    }
    if let Some((edge_max, offset, ink)) = best {
        println!("  -> cleanest inked DMG offset {offset:+.3}: edgeMax={edge_max:.3} (ink {ink})");
    }
}

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: edge_ink <img|dir> [team] [name]");
    let team_size: usize = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(6);
    let name = std::env::args().nth(3);
    let p = std::path::PathBuf::from(&path);

    if p.is_dir() {
        let mut entries: Vec<_> = std::fs::read_dir(&p)
            .expect("read dir")
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "png"))
            .collect();
        entries.sort();
        for e in entries {
            dump_one(&e, team_size, name.as_deref());
        }
    } else {
        dump_one(&p, team_size, name.as_deref());
    }
}
