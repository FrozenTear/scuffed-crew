use image::DynamicImage;

use super::MatchOutcome;

pub fn detect_outcome(img: &DynamicImage) -> MatchOutcome {
    if let Some(outcome) = detect_banner(img) {
        return outcome;
    }
    detect_accolade_screen(img)
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

// Detect the post-match accolade / MVP screen (after Play of the Game). This
// screen is on-screen ~15-20s while players endorse, so a few-second poller
// catches it reliably (unlike the ~3s VICTORY/DEFEAT banner).
//
// The screen is a light blue-gray body under a dark navy header — NOT the
// saturated blue the old detector required, which is why it never fired.
// Measured on a real frame: ~80% of pixels are blue-margin-dominant under a
// loose test. The result word ("VICTORY"/"DEFEAT") sits in the top-LEFT corner
// in large cyan/red text — not top-center.
fn detect_accolade_screen(img: &DynamicImage) -> MatchOutcome {
    let rgb = img.to_rgb8();
    let (w, h) = rgb.dimensions();
    if w == 0 || h == 0 {
        return MatchOutcome::Unknown;
    }

    let mut blue_count = 0u32;
    let mut total = 0u32;
    for pixel in rgb.pixels() {
        let [r, g, b] = pixel.0;
        total += 1;
        // Loose blue-margin test: blue merely leans above red and green. Catches
        // both the dark navy header and the light periwinkle body.
        if (b as i32 - r as i32) > 15 && (b as i32 - g as i32) > 5 {
            blue_count += 1;
        }
    }

    let blue_ratio = blue_count as f32 / total as f32;
    // Real accolade frames measure ~80%; 0.6 rejects most desktop/web content
    // while leaving headroom. The result-word OCR below is the real guard.
    if blue_ratio < 0.6 {
        return MatchOutcome::Unknown;
    }

    tracing::debug!(blue_ratio, "post-match accolade screen detected");
    read_result_word(img)
}

/// Read the large top-left VICTORY/DEFEAT title off the accolade screen.
/// Region calibrated against a native 16:9 accolade frame: x 0.5-14%, y 3.5-9.5%.
fn read_result_word(img: &DynamicImage) -> MatchOutcome {
    let (w, h) = (img.width(), img.height());
    let x = w * 5 / 1000;
    let y = h * 35 / 1000;
    let cw = w * 135 / 1000;
    let ch = h * 60 / 1000;
    if cw == 0 || ch == 0 || x + cw > w || y + ch > h {
        return MatchOutcome::Unknown;
    }
    let crop = img.crop_imm(x, y, cw, ch);
    let prepared = crate::ocr::preprocess::prepare_title(&crop);

    match crate::ocr::recognize_prepared(&prepared, "7", Some("ABCDEFGHIJKLMNOPQRSTUVWXYZ")) {
        Ok(text) => {
            let upper = text.to_uppercase();
            if upper.contains("VICTORY") {
                tracing::info!(text = %text.trim(), "accolade screen: VICTORY");
                MatchOutcome::Victory
            } else if upper.contains("DEFEAT") {
                tracing::info!(text = %text.trim(), "accolade screen: DEFEAT");
                MatchOutcome::Defeat
            } else if upper.contains("DRAW") {
                MatchOutcome::Draw
            } else {
                tracing::debug!(ocr_text = %text.trim(), "accolade screen detected but result text unreadable");
                MatchOutcome::Unknown
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "accolade result OCR failed");
            MatchOutcome::Unknown
        }
    }
}
