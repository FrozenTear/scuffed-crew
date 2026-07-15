//! Dev probe: print the team-1 row-scan signal (dip count + median pitch)
//! behind team-size detection and the scoreboard preflight.
//! Usage: cargo run -p scuffed-stat-tracker --example rowscan -- <img.png>...

use stat_tracker::detect::hero_portrait::scan_rows;
use stat_tracker::ocr::preprocess::crop_scoreboard;

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let paths: Vec<String> = std::env::args().skip(1).collect();
    if paths.is_empty() {
        return Err("usage: rowscan <img.png>...".into());
    }

    for path in &paths {
        let img = image::open(path)?;
        let board = crop_scoreboard(&img);
        let scan = scan_rows(&board);
        let name = path.rsplit('/').next().unwrap_or(path);
        println!(
            "{name} crop={}x{} dips={} dip_pitch={:?} spectral_pitch={:?} -> team_size={} looks_like_scoreboard={}",
            board.width(),
            board.height(),
            scan.dip_count,
            scan.median_pitch,
            scan.spectral_pitch,
            scan.team_size(),
            scan.looks_like_scoreboard(),
        );
    }
    Ok(())
}
