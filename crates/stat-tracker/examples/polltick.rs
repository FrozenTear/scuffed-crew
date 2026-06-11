//! Measure the CPU cost of one poller tick's detection work (detect_outcome +
//! detect_phase) — what runs every poll_interval_secs in the background.
//! The screen *capture* cost is separate (backend-dependent) and not measured here.
//!
//! Usage: cargo run -p scuffed-stat-tracker --example polltick -- <frame.png>

use stat_tracker::detect;

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: polltick <frame.png>")?;
    let img = image::open(&path)?;
    println!("frame: {}x{}", img.width(), img.height());

    // warm up (first OCR call constructs the thread-local engine)
    let _ = detect::match_end::detect_outcome(&img);
    let _ = detect::match_start::detect_phase(&img);

    let n = 20;
    let t = std::time::Instant::now();
    for _ in 0..n {
        let _ = detect::match_end::detect_outcome(&img);
        let _ = detect::match_start::detect_phase(&img);
    }
    let per = t.elapsed().as_secs_f64() / n as f64 * 1000.0;
    println!("detect_outcome + detect_phase: {per:.1} ms/tick (avg of {n})");
    Ok(())
}
