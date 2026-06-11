use image::DynamicImage;

use super::MatchOutcome;

/// Which detector produced an outcome. The banner color-flood is specific
/// enough to act on from a single frame; the word-OCR sources (accolade
/// screen, competitive rank screen) are cheap but weaker evidence, so the
/// poller requires two agreeing word reads inside a confirmation window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutcomeSource {
    Banner,
    /// Top-left result word on the post-match accolade screen.
    ResultWord,
    /// Result word under the "COMPETITIVE" title on the rank-update screen.
    RankScreen,
}

/// One-shot outcome read for a single frame (Tab captures, dev tools).
pub fn detect_outcome(img: &DynamicImage) -> MatchOutcome {
    detect_outcome_signal(img)
        .map(|(outcome, _)| outcome)
        .unwrap_or(MatchOutcome::Unknown)
}

/// Outcome detection with its evidence source, for the poller.
///
/// Order: the full-screen VICTORY/DEFEAT banner color-flood first (fast, very
/// specific), then OCR of the accolade screen's top-left result word. The word
/// OCR runs unconditionally — it used to be gated behind a "60% of pixels lean
/// blue" accolade-screen check, but custom UI color schemes (e.g. magenta)
/// break any assumption about the screen's dominant color, and the full-frame
/// pixel scan cost more than the small-crop OCR it was guarding. The Otsu-based
/// `read_result_word` is color-scheme-independent.
pub fn detect_outcome_signal(img: &DynamicImage) -> Option<(MatchOutcome, OutcomeSource)> {
    if let Some(outcome) = detect_banner(img) {
        return Some((outcome, OutcomeSource::Banner));
    }
    match read_result_word(img) {
        MatchOutcome::Unknown => {}
        outcome => return Some((outcome, OutcomeSource::ResultWord)),
    }
    match read_rank_screen_result(img) {
        MatchOutcome::Unknown => None,
        outcome => Some((outcome, OutcomeSource::RankScreen)),
    }
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

/// Read the large top-left VICTORY/DEFEAT title off the post-match accolade /
/// MVP screen (shown ~15-20s after Play of the Game).
/// Region calibrated against a native 16:9 accolade frame: x 0.5-14%, y 3.5-9.5%.
/// Validated on a custom magenta UI theme (2026-06-11 defeat frame).
fn read_result_word(img: &DynamicImage) -> MatchOutcome {
    ocr_outcome_word(img, 5, 35, 135, 60, "accolade screen")
}

/// Read the result word off the competitive summary (rank update) screen —
/// VICTORY/DEFEAT printed under the big "COMPETITIVE" title, top-left. The
/// background is dark regardless of UI color theme, and the screen stays up
/// 40s+ (the longest-lived outcome signal, surviving even a starved poller).
/// Region measured from a real 16:9 frame: word spans x 4-12.5%, y 16-21%.
fn read_rank_screen_result(img: &DynamicImage) -> MatchOutcome {
    ocr_outcome_word(img, 10, 145, 150, 80, "rank screen")
}

/// OCR a crop (given in 1/1000ths of the 16:9 frame) prepared as title text,
/// and map VICTORY/DEFEAT/DRAW to an outcome.
fn ocr_outcome_word(
    img: &DynamicImage,
    x_pm: u32,
    y_pm: u32,
    w_pm: u32,
    h_pm: u32,
    context: &str,
) -> MatchOutcome {
    let (w, h) = (img.width(), img.height());
    let x = w * x_pm / 1000;
    let y = h * y_pm / 1000;
    let cw = w * w_pm / 1000;
    let ch = h * h_pm / 1000;
    if cw == 0 || ch == 0 || x + cw > w || y + ch > h {
        return MatchOutcome::Unknown;
    }
    let crop = img.crop_imm(x, y, cw, ch);
    let prepared = crate::ocr::preprocess::prepare_title(&crop);

    match crate::ocr::recognize_prepared(&prepared, "7", Some("ABCDEFGHIJKLMNOPQRSTUVWXYZ")) {
        Ok(text) => {
            let upper = text.to_uppercase();
            if upper.contains("VICTORY") {
                tracing::info!(text = %text.trim(), context, "result word: VICTORY");
                MatchOutcome::Victory
            } else if upper.contains("DEFEAT") {
                tracing::info!(text = %text.trim(), context, "result word: DEFEAT");
                MatchOutcome::Defeat
            } else if upper.contains("DRAW") {
                MatchOutcome::Draw
            } else {
                tracing::trace!(ocr_text = %text.trim(), context, "no result word in region");
                MatchOutcome::Unknown
            }
        }
        Err(e) => {
            // Runs every poll tick now — a broken Tesseract setup would make a
            // warn here fire every few seconds; captures fail loudly anyway.
            tracing::debug!(error = %e, context, "result word OCR failed");
            MatchOutcome::Unknown
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgb, RgbImage};

    fn flood(color: [u8; 3]) -> DynamicImage {
        DynamicImage::ImageRgb8(RgbImage::from_pixel(640, 360, Rgb(color)))
    }

    #[test]
    fn gold_flood_is_victory_banner() {
        assert_eq!(
            detect_outcome_signal(&flood([230, 180, 20])),
            Some((MatchOutcome::Victory, OutcomeSource::Banner))
        );
    }

    #[test]
    fn red_flood_is_defeat_banner() {
        assert_eq!(
            detect_outcome_signal(&flood([200, 30, 30])),
            Some((MatchOutcome::Defeat, OutcomeSource::Banner))
        );
    }

    #[test]
    fn black_frame_is_no_signal() {
        assert_eq!(detect_outcome_signal(&flood([0, 0, 0])), None);
        assert_eq!(detect_outcome(&flood([0, 0, 0])), MatchOutcome::Unknown);
    }
}
