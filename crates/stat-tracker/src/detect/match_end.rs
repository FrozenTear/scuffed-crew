use image::DynamicImage;

use super::MatchOutcome;

pub fn detect_outcome(img: &DynamicImage) -> MatchOutcome {
    if let Some(outcome) = detect_banner(img) {
        return outcome;
    }
    detect_commendation_screen(img)
}

/// Text-based outcome fallback for the *captured scoreboard frame*.
///
/// The color-flood detectors above only fire on the brief full-screen
/// VICTORY/DEFEAT banner and the blue commendation screen. The poller samples
/// every few seconds and routinely misses that transient banner, so by the time
/// the user presses Tab we're on the post-match scoreboard — which prints the
/// VICTORY / DEFEAT header at top-center but has none of the color flood. This
/// reads that header text directly, and is meant to be called only when
/// `detect_outcome` returns `Unknown` and no outcome was carried over from the
/// poller.
pub fn detect_outcome_text(img: &DynamicImage) -> MatchOutcome {
    let (w, h) = (img.width(), img.height());
    // Top-center band where OW2 renders the result header.
    let x = w * 30 / 100;
    let y = h * 2 / 100;
    let band_w = w * 40 / 100;
    let band_h = h * 22 / 100;
    if band_w == 0 || band_h == 0 || x + band_w > w || y + band_h > h {
        return MatchOutcome::Unknown;
    }
    let region = img.crop_imm(x, y, band_w, band_h);

    match crate::ocr::recognize_region(&region) {
        Ok(text) => {
            let upper = text.to_uppercase();
            if upper.contains("VICTORY") {
                tracing::info!(text = %text.trim(), "outcome read from scoreboard header text");
                MatchOutcome::Victory
            } else if upper.contains("DEFEAT") {
                tracing::info!(text = %text.trim(), "outcome read from scoreboard header text");
                MatchOutcome::Defeat
            } else if upper.contains("DRAW") {
                MatchOutcome::Draw
            } else {
                tracing::debug!(text = %text.trim(), "scoreboard header text did not contain an outcome");
                MatchOutcome::Unknown
            }
        }
        Err(e) => {
            tracing::debug!(error = %e, "scoreboard header OCR failed");
            MatchOutcome::Unknown
        }
    }
}

// Detect the brief VICTORY/DEFEAT full-screen banner (gold or red backdrop).
// OW2 banners saturate >40% of the screen with a very specific color range.
// Previous thresholds (15%, loose color ranges) caused false positives on
// websites with warm/red colors during normal browsing.
fn detect_banner(img: &DynamicImage) -> Option<MatchOutcome> {
    let rgb = img.to_rgb8();
    let (w, h) = rgb.dimensions();

    // Sample the middle horizontal band (30%-70% height) where the banner
    // text and color flood are most consistent, avoiding HUD/taskbar edges.
    let y_start = h * 30 / 100;
    let y_end = h * 70 / 100;
    let mut gold_count = 0u32;
    let mut red_count = 0u32;
    let mut total = 0u32;

    for y in y_start..y_end {
        for x in 0..w {
            let [r, g, b] = rgb.get_pixel(x, y).0;
            total += 1;
            // OW2 victory gold: saturated warm gold, green channel well above blue
            if r > 200 && g > 140 && g < 220 && b < 60 && (r as i32 - b as i32) > 150 {
                gold_count += 1;
            }
            // OW2 defeat red: deep red, very low green and blue
            if r > 180 && g < 60 && b < 60 {
                red_count += 1;
            }
        }
    }

    if total == 0 {
        return None;
    }

    let gold_ratio = gold_count as f32 / total as f32;
    let red_ratio = red_count as f32 / total as f32;

    // OW2 banners flood >40% of the sampled region with the dominant color.
    // 35% threshold with tighter color ranges eliminates web page false positives.
    const THRESHOLD: f32 = 0.35;

    if gold_ratio > THRESHOLD {
        tracing::debug!(gold_ratio, "victory banner detected");
        Some(MatchOutcome::Victory)
    } else if red_ratio > THRESHOLD {
        tracing::debug!(red_ratio, "defeat banner detected");
        Some(MatchOutcome::Defeat)
    } else {
        None
    }
}

// Detect the post-match commendation/voting screen (after Play of the Game).
// OW2 commendation screens have 57-66% blue-dominant pixels — well above any
// normal desktop content. Previous 20% threshold caused false positives on
// websites with blue themes.
fn detect_commendation_screen(img: &DynamicImage) -> MatchOutcome {
    let rgb = img.to_rgb8();
    let (w, h) = rgb.dimensions();

    let mut blue_margin_count = 0u32;
    let mut total = 0u32;

    for pixel in rgb.pixels() {
        let [r, g, b] = pixel.0;
        total += 1;
        // Tighter blue check: blue must strongly dominate both red and green
        if b > 140 && (b as i32 - r as i32) > 60 && (b as i32 - g as i32) > 30 {
            blue_margin_count += 1;
        }
    }

    if total == 0 {
        return MatchOutcome::Unknown;
    }

    let blue_ratio = blue_margin_count as f32 / total as f32;
    // Require at least 45% — real OW2 commendation screens are 57-66%.
    // This eliminates blue-themed websites while still catching all OW2 screens.
    if blue_ratio < 0.45 {
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
