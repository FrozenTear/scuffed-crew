//! One-off calibration tool for the post-match accolade screen.
//! Measures header/body brightness + blue-margin zones and OCRs candidate
//! top-left result-text crops to locate VICTORY/DEFEAT.
//!
//! Usage: cargo run -p scuffed-stat-tracker --example accolade -- <image.png>

use stat_tracker::ocr;

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: accolade <image.png>")?;
    let img = image::open(&path)?;
    let (w, h) = (img.width(), img.height());
    println!("image: {w}x{h}");

    let rgb = img.to_rgb8();

    // Zone brightness + blue-margin stats.
    let zone = |x0: f64, y0: f64, x1: f64, y1: f64, label: &str| {
        let (xs, ys) = ((w as f64 * x0) as u32, (h as f64 * y0) as u32);
        let (xe, ye) = ((w as f64 * x1) as u32, (h as f64 * y1) as u32);
        let (mut sum_br, mut blue, mut n) = (0u64, 0u64, 0u64);
        for y in ys..ye {
            for x in xs..xe {
                let [r, g, b] = rgb.get_pixel(x, y).0;
                sum_br += (r as u64 + g as u64 + b as u64) / 3;
                if b as i32 - r as i32 > 15 && b as i32 - g as i32 > 5 {
                    blue += 1;
                }
                n += 1;
            }
        }
        println!(
            "  {label:<16} mean_brightness={:.0}  blue_margin={:.1}%",
            sum_br as f64 / n.max(1) as f64,
            blue as f64 / n.max(1) as f64 * 100.0
        );
    };
    println!("zones:");
    zone(0.0, 0.0, 1.0, 0.11, "top header");
    zone(0.0, 0.12, 1.0, 0.95, "body");
    zone(0.0, 0.0, 0.20, 0.12, "result corner");

    // OCR candidate crops for the result word — tight on VICTORY only.
    let crops: &[(f64, f64, f64, f64)] = &[
        (0.005, 0.035, 0.14, 0.095), // tight VICTORY box
        (0.0, 0.03, 0.15, 0.105),
        (0.0, 0.0, 0.16, 0.11),
    ];
    println!("\nresult-region OCR candidates (recognize_region):");
    for (i, &(x0, y0, x1, y1)) in crops.iter().enumerate() {
        let (xs, ys) = ((w as f64 * x0) as u32, (h as f64 * y0) as u32);
        let (cw, ch) = ((w as f64 * (x1 - x0)) as u32, (h as f64 * (y1 - y0)) as u32);
        let crop = img.crop_imm(xs, ys, cw, ch);
        let _ = crop.save(format!("/tmp/accolade_crop_{i}.png"));
        let text = ocr::recognize_region(&crop).unwrap_or_default();
        println!(
            "  [{i}] x{:.1}%-{:.0}% y{:.1}%-{:.1}%  -> {:?}  (saved /tmp/accolade_crop_{i}.png)",
            x0 * 100.0,
            x1 * 100.0,
            y0 * 100.0,
            y1 * 100.0,
            text.trim()
        );
    }

    // Try a simple high-contrast binarization of the tight box, OCR raw.
    {
        let (x0, y0, x1, y1) = crops[0];
        let (xs, ys) = ((w as f64 * x0) as u32, (h as f64 * y0) as u32);
        let (cw, ch) = ((w as f64 * (x1 - x0)) as u32, (h as f64 * (y1 - y0)) as u32);
        let crop = img.crop_imm(xs, ys, cw, ch).to_luma8();
        // Upscale 3x + threshold at 110 (cyan text ~ bright on dark header).
        let up =
            image::imageops::resize(&crop, cw * 3, ch * 3, image::imageops::FilterType::Lanczos3);
        let bin = image::GrayImage::from_fn(up.width(), up.height(), |x, y| {
            if up.get_pixel(x, y).0[0] > 110 {
                image::Luma([0u8])
            } else {
                image::Luma([255u8])
            }
        });
        bin.save("/tmp/accolade_bin.png").ok();
        println!("  saved /tmp/accolade_bin.png (3x upscaled, inverted threshold@110)");
    }

    Ok(())
}
