//! Dev probe: run the pre-OCR scoreboard preflight (row-dip scan) on frames.
//! Usage: cargo run -p scuffed-stat-tracker --example preflight -- <img.png>...

use stat_tracker::detect::hero_portrait::scan_rows;
use stat_tracker::ocr::preprocess::{crop_scoreboard, header_label_groups};

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let paths: Vec<String> = std::env::args().skip(1).collect();
    if paths.is_empty() {
        return Err("usage: preflight <img.png>...".into());
    }

    for path in &paths {
        let img = image::open(path)?;
        let board = crop_scoreboard(&img);
        let scan = scan_rows(&board);
        let groups = header_label_groups(&board);
        let rows_ok = scan.looks_like_scoreboard();
        let header_ok = (3..=10).contains(&groups.len());
        println!(
            "{:60} dips={:2} pitch={} rows_ok={:5} header_groups={:2} header_ok={:5} pass={} team_size={}",
            path,
            scan.dip_count,
            scan.median_pitch
                .map(|p| format!("{p:.4}"))
                .unwrap_or_else(|| "  -   ".into()),
            rows_ok,
            groups.len(),
            header_ok,
            rows_ok || header_ok,
            scan.team_size(),
        );
    }
    Ok(())
}
