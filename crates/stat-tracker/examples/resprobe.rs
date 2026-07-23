//! Dev probe: run the scoreboard cell-OCR pipeline on a saved debug/accepted
//! scoreboard crop at its native scale and at downscaled factors, to compare
//! read quality across display resolutions (e.g. does the 1440p-calibrated
//! pipeline survive a 1080p or 720p display?).
//! Usage: cargo run -p scuffed-stat-tracker --example resprobe -- <crop.png> [factor ...]
//! e.g. factors 0.75 (1440p→1080p) and 0.5 (1440p→720p).

use image::imageops::FilterType;
use stat_tracker::detect::hero_portrait::scan_rows;
use stat_tracker::ocr;

fn run(label: &str, cropped: &image::DynamicImage) {
    let scan = scan_rows(cropped);
    let team_size = scan.team_size();
    println!(
        "--- {label} ({}x{}) team_size={team_size} looks_like_scoreboard={} ---",
        cropped.width(),
        cropped.height(),
        scan.looks_like_scoreboard()
    );
    let rows = ocr::recognize_scoreboard_cells_pre_cropped(cropped, team_size);
    for (i, row) in rows.iter().enumerate() {
        let stats: Vec<String> = row
            .stats
            .iter()
            .map(|c| {
                let mut s = if c.value.is_empty() {
                    "·".to_string()
                } else {
                    c.value.clone()
                };
                if c.suspect {
                    s.push('!');
                }
                s
            })
            .collect();
        let name = row
            .name
            .as_ref()
            .map(|n| n.value.clone())
            .unwrap_or_default();
        println!(
            "row {i:2} [{:12}] {:>7} {:>7} {:>7} {:>8} {:>8} {:>8} (conf {})",
            name, stats[0], stats[1], stats[2], stats[3], stats[4], stats[5], row.mean_confidence
        );
    }
}

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    let mut args = std::env::args().skip(1);
    let path = args
        .next()
        .ok_or("usage: resprobe <crop.png> [factor ...]")?;
    let img = image::open(&path)?;
    run("native", &img);
    for spec in args {
        let f: f64 = spec.parse()?;
        let (w, h) = (
            (img.width() as f64 * f).round() as u32,
            (img.height() as f64 * f).round() as u32,
        );
        let scaled = img.resize_exact(w, h, FilterType::Lanczos3);
        run(&format!("x{spec}"), &scaled);
    }
    Ok(())
}
