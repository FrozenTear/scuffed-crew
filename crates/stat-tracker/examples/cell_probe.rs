//! Scratch: OCR a single saved cell crop at several upscales.
//! Usage: cargo run -p scuffed-stat-tracker --example cell_probe -- <cell.png>...
use image::imageops::FilterType;
fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    for path in std::env::args().skip(1) {
        let img = image::open(&path)?;
        for scale in [1u32, 2, 3] {
            let s = if scale == 1 {
                img.clone()
            } else {
                img.resize_exact(
                    img.width() * scale,
                    img.height() * scale,
                    FilterType::Lanczos3,
                )
            };
            let r = stat_tracker::ocr::recognize_cell(&s)?;
            println!(
                "{path} x{scale}: value={:?} conf={} suspect={}",
                r.value, r.confidence, r.suspect
            );
        }
    }
    Ok(())
}
