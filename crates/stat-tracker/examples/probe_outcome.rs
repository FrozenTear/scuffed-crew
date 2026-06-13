//! Throwaway probe: run the REAL production outcome detectors on a frame.
//! Usage: cargo run -p scuffed-stat-tracker --example probe_outcome -- <img.png>

use stat_tracker::detect::match_end::{detect_outcome, detect_outcome_text};
use stat_tracker::ocr;

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: probe_outcome <img>")?;
    let img = image::open(&path)?;
    let (w, h) = (img.width(), img.height());

    // Replicate the banner color gates (informational; the accolade blue-ratio
    // gate was removed — the result-word OCR now runs unconditionally).
    let rgb = img.to_rgb8();
    let (mut gold, mut red, mut band_total) = (0u32, 0u32, 0u32);
    let (ys, ye) = (h * 30 / 100, h * 70 / 100);
    for yy in ys..ye {
        for xx in 0..w {
            let [r, g, b] = rgb.get_pixel(xx, yy).0;
            band_total += 1;
            if r > 200 && g > 140 && g < 220 && b < 60 && (r as i32 - b as i32) > 150 {
                gold += 1;
            }
            if r > 180 && g < 60 && b < 60 {
                red += 1;
            }
        }
    }
    let (mut blue, mut full) = (0u32, 0u32);
    for p in rgb.pixels() {
        let [r, g, b] = p.0;
        full += 1;
        if (b as i32 - r as i32) > 15 && (b as i32 - g as i32) > 5 {
            blue += 1;
        }
    }
    println!(
        "banner gold_ratio={:.3} red_ratio={:.3} (threshold 0.35)",
        gold as f32 / band_total as f32,
        red as f32 / band_total as f32
    );
    println!(
        "accolade blue_ratio={:.3} (informational — no longer gates detection)",
        blue as f32 / full as f32
    );
    println!("detect_outcome()      => {:?}", detect_outcome(&img));
    println!("detect_outcome_text() => {:?}", detect_outcome_text(&img));
    println!(
        "read_accolade_map()   => {:?}",
        stat_tracker::detect::match_end::read_accolade_map(&img)
    );

    // Reproduce read_result_word's exact crop + prepare_title path.
    let x = w * 5 / 1000;
    let y = h * 35 / 1000;
    let cw = w * 135 / 1000;
    let ch = h * 60 / 1000;
    let crop = img.crop_imm(x, y, cw, ch);
    let prepared = ocr::preprocess::prepare_title(&crop);
    prepared.save("/tmp/probe_title_bin.png").ok();
    let txt = ocr::recognize_prepared(&prepared, "7", Some("ABCDEFGHIJKLMNOPQRSTUVWXYZ"))?;
    println!(
        "prepare_title+OCR     => {:?}  (saved /tmp/probe_title_bin.png)",
        txt.trim()
    );
    Ok(())
}
