use image::DynamicImage;

use super::MatchOutcome;

pub fn detect_outcome(img: &DynamicImage) -> MatchOutcome {
    if let Some(outcome) = detect_banner(img) {
        return outcome;
    }
    detect_commendation_screen(img)
}

// Detect the brief VICTORY/DEFEAT full-screen banner (gold or red backdrop)
fn detect_banner(img: &DynamicImage) -> Option<MatchOutcome> {
    let rgb = img.to_rgb8();
    let (w, h) = rgb.dimensions();
    let sample_height = h / 5;
    let mut gold_count = 0u32;
    let mut red_count = 0u32;
    let mut total = 0u32;

    for pixel in rgb.pixels().take((sample_height * w) as usize) {
        let [r, g, b] = pixel.0;
        total += 1;
        if r > 180 && g > 120 && b < 80 {
            gold_count += 1;
        }
        if r > 180 && g < 100 && b < 100 {
            red_count += 1;
        }
    }

    if total == 0 {
        return None;
    }

    let gold_ratio = gold_count as f32 / total as f32;
    let red_ratio = red_count as f32 / total as f32;

    const THRESHOLD: f32 = 0.15;

    if gold_ratio > THRESHOLD {
        Some(MatchOutcome::Victory)
    } else if red_ratio > THRESHOLD {
        Some(MatchOutcome::Defeat)
    } else {
        None
    }
}

// Detect the post-match commendation/voting screen (after Play of the Game).
// Both VICTORY and DEFEAT versions have a dominant blue gradient background
// (57-66% blue-margin pixels vs <0.4% on all other screens).
fn detect_commendation_screen(img: &DynamicImage) -> MatchOutcome {
    let rgb = img.to_rgb8();
    let (w, h) = rgb.dimensions();

    let mut blue_margin_count = 0u32;
    let mut total = 0u32;

    for pixel in rgb.pixels() {
        let [r, g, b] = pixel.0;
        total += 1;
        if b > 130 && (b as i32 - r as i32) > 40 && (b as i32 - g as i32) > 20 {
            blue_margin_count += 1;
        }
    }

    if total == 0 {
        return MatchOutcome::Unknown;
    }

    let blue_ratio = blue_margin_count as f32 / total as f32;
    if blue_ratio < 0.20 {
        return MatchOutcome::Unknown;
    }

    tracing::debug!(blue_ratio, "post-match commendation screen detected");

    // OCR the top-left corner to distinguish VICTORY from DEFEAT
    let corner_w = w * 15 / 100;
    let corner_h = h * 8 / 100;
    let corner = img.crop_imm(0, 0, corner_w, corner_h);

    match crate::ocr::recognize_region(&corner) {
        Ok(text) => {
            let upper = text.to_uppercase();
            if upper.contains("VICTORY") {
                MatchOutcome::Victory
            } else if upper.contains("DEFEAT") {
                MatchOutcome::Defeat
            } else {
                tracing::debug!(ocr_text = %text.trim(), "post-match screen detected but could not read outcome text");
                MatchOutcome::Unknown
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "post-match corner OCR failed");
            MatchOutcome::Unknown
        }
    }
}
