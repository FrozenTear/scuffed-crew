//! Dev probe: run the production phase + outcome detectors on poll frames.
//! Usage: cargo run -p scuffed-stat-tracker --example phaseprobe -- <img.png>...

use stat_tracker::detect;

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let paths: Vec<String> = std::env::args().skip(1).collect();
    if paths.is_empty() {
        return Err("usage: phaseprobe <img.png>...".into());
    }

    for path in &paths {
        let img = image::open(path)?;
        let rgb = img.to_rgb8();
        let phase = detect::match_start::detect_phase_with_rgb(&img, &rgb);
        let signal = detect::match_end::detect_outcome_signal_with_rgb(&img, &rgb);
        let m = detect::match_start::gate_metrics(&rgb);
        let name = path.rsplit('/').next().unwrap_or(path);
        // Gate thresholds: vote navy≥0.40, ban red≥0.003+dark≥0.30,
        // select dark≥0.50+var≥2000 (each then needs OCR confirmation).
        println!(
            "{name} phase={phase:?} outcome={signal:?} | navy={:.3} ban_red={:.3} ban_dark={:.3} sel_dark={:.3} sel_var={:.0}",
            m.navy_ratio,
            m.ban_red_ratio,
            m.ban_dark_ratio,
            m.select_dark_header_ratio,
            m.select_grid_variance,
        );
    }
    Ok(())
}
