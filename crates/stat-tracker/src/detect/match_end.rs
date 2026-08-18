use image::{DynamicImage, GrayImage, Luma, RgbImage};

use super::MatchOutcome;
use super::stability::FrameStability;

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
    /// Centered italic VICTORY/DEFEAT on the in-world end-title overlay
    /// (theme-colored; no gold/red banner flood). Added 2026-08-18 after a
    /// magenta-UI frame where every other source missed.
    EndTitle,
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
    let rgb = img.to_rgb8();
    detect_outcome_signal_with_rgb(img, &rgb)
}

/// Outcome detection when the caller already converted the frame to RGB (P6).
pub fn detect_outcome_signal_with_rgb(
    img: &DynamicImage,
    rgb: &RgbImage,
) -> Option<(MatchOutcome, OutcomeSource)> {
    detect_outcome_signal_inner(img, rgb, None)
}

/// Poll-tick outcome detection: identical to
/// [`detect_outcome_signal_with_rgb`] except the word-OCR crops are gated on
/// temporal stability — Tesseract only runs once a crop has held still across
/// consecutive ticks (see [`FrameStability`]). The banner color-flood is never
/// gated: it is cheap, lasts ~3s, and a second tick may never come. Word
/// detection shifts at most one tick later, inside the budget of screens that
/// stay up 15–20s and already need two agreeing reads to confirm.
pub fn detect_outcome_signal_polled(
    img: &DynamicImage,
    rgb: &RgbImage,
    stability: &mut FrameStability,
) -> Option<(MatchOutcome, OutcomeSource)> {
    detect_outcome_signal_inner(img, rgb, Some(stability))
}

fn detect_outcome_signal_inner(
    img: &DynamicImage,
    rgb: &RgbImage,
    mut stability: Option<&mut FrameStability>,
) -> Option<(MatchOutcome, OutcomeSource)> {
    if let Some(outcome) = detect_banner(rgb) {
        return Some((outcome, OutcomeSource::Banner));
    }
    match read_result_word(img, stability.as_deref_mut()) {
        MatchOutcome::Unknown => {}
        outcome => return Some((outcome, OutcomeSource::ResultWord)),
    }
    match read_rank_screen_result(img, stability.as_deref_mut()) {
        MatchOutcome::Unknown => {}
        outcome => return Some((outcome, OutcomeSource::RankScreen)),
    }
    match read_end_title(img, rgb, stability) {
        MatchOutcome::Unknown => None,
        outcome => Some((outcome, OutcomeSource::EndTitle)),
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
    let rgb = img.to_rgb8();
    let (fw, fh) = (img.width(), img.height());
    let (gx, gy, gw, gh) = crate::ocr::preprocess::game_rect_16_9(fw, fh);
    // Top-center band where OW2 renders the result header (1/1000ths of 16:9).
    let x = gx + gw * 300 / 1000;
    let y = gy + gh * 20 / 1000;
    let band_w = gw * 400 / 1000;
    let band_h = gh * 220 / 1000;
    if band_w == 0 || band_h == 0 || x + band_w > fw || y + band_h > fh {
        return read_end_title(img, &rgb, None);
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
                read_end_title(img, &rgb, None)
            }
        }
        Err(e) => {
            tracing::debug!(error = %e, "scoreboard header OCR failed");
            read_end_title(img, &rgb, None)
        }
    }
}

/// Centered italic end-title (theme-colored VICTORY/DEFEAT over the world).
/// Cheap sat-mass / scoreline gate first so mid-fight ticks do not pay OCR.
fn read_end_title(
    img: &DynamicImage,
    rgb: &RgbImage,
    mut stability: Option<&mut FrameStability>,
) -> MatchOutcome {
    let mass = center_title_mass(rgb);
    let scoreline =
        mass < END_TITLE_MASS_MIN && scoreline_looks_present(img, rgb, stability.as_deref_mut());
    if mass < END_TITLE_MASS_MIN && !scoreline {
        return MatchOutcome::Unknown;
    }

    let (fw, fh) = (img.width(), img.height());
    let (gx, gy, gw, gh) = crate::ocr::preprocess::game_rect_16_9(fw, fh);
    // Tighter than the mass window — just the italic word.
    // Calibrated on rejected_preflight_20260817_232203.png (magenta VICTORY!).
    let x = gx + gw * 320 / 1000;
    let y = gy + gh * 340 / 1000;
    let cw = gw * 360 / 1000;
    let ch = gh * 180 / 1000;
    if cw == 0 || ch == 0 || x + cw > fw || y + ch > fh {
        return MatchOutcome::Unknown;
    }
    let crop = img.crop_imm(x, y, cw, ch);

    if let Some(stability) = stability
        && !stability.check("end title", &crop)
    {
        tracing::trace!("end title crop not stable — deferring OCR");
        return MatchOutcome::Unknown;
    }

    let prepared = prepare_end_title(&crop);
    match crate::ocr::recognize_prepared_lang(
        &prepared,
        "7",
        Some("ABCDEFGHIJKLMNOPQRSTUVWXYZ!"),
        "eng",
    ) {
        Ok(text) => {
            let outcome = fuzzy_outcome_word(&text);
            if outcome.is_decided() {
                let word = match outcome {
                    MatchOutcome::Victory => "VICTORY",
                    MatchOutcome::Defeat => "DEFEAT",
                    MatchOutcome::Draw => "DRAW",
                    MatchOutcome::Unknown => "UNKNOWN",
                };
                tracing::info!(text = %text.trim(), context = "end title", "result word: {word}");
            } else {
                tracing::trace!(ocr_text = %text.trim(), "end title OCR did not match an outcome");
            }
            outcome
        }
        Err(e) => {
            tracing::debug!(error = %e, "end title OCR failed");
            MatchOutcome::Unknown
        }
    }
}

const END_TITLE_MASS_MIN: f32 = 0.06;

fn fuzzy_outcome_word(raw: &str) -> MatchOutcome {
    let letters: String = raw
        .chars()
        .filter(|c| c.is_ascii_alphabetic())
        .map(|c| c.to_ascii_uppercase())
        .collect();
    if letters.is_empty() {
        return MatchOutcome::Unknown;
    }
    const TARGETS: &[(&str, MatchOutcome)] = &[
        ("VICTORY", MatchOutcome::Victory),
        ("DEFEAT", MatchOutcome::Defeat),
        ("DRAW", MatchOutcome::Draw),
    ];
    let hits: Vec<MatchOutcome> = TARGETS
        .iter()
        .filter(|(word, _)| {
            letters.contains(word)
                || (letters.len() + 1 >= word.len() && strsim::levenshtein(&letters, word) <= 2)
        })
        .map(|(_, outcome)| *outcome)
        .collect();
    if hits.len() == 1 {
        hits[0]
    } else {
        MatchOutcome::Unknown
    }
}

fn rgb_to_hsv(r: u8, g: u8, b: u8) -> (u16, u8, u8) {
    let r = i32::from(r);
    let g = i32::from(g);
    let b = i32::from(b);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;
    let v = max as u8;
    let s = if max == 0 {
        0
    } else {
        ((delta * 255) / max) as u8
    };
    let h = if delta == 0 {
        0
    } else if max == r {
        let x = ((g - b) * 60) / delta;
        if x < 0 { x + 360 } else { x }
    } else if max == g {
        120 + ((b - r) * 60) / delta
    } else {
        240 + ((r - g) * 60) / delta
    };
    ((h as u16) % 360, s, v)
}

fn hue_near(a: u16, b: u16, window: u16) -> bool {
    let d = (i32::from(a) - i32::from(b)).unsigned_abs() as u16;
    d.min(360 - d) <= window
}

fn center_title_mass(rgb: &RgbImage) -> f32 {
    let (w, h) = rgb.dimensions();
    let (gx, gy, gw, gh) = crate::ocr::preprocess::game_rect_16_9(w, h);
    let x0 = gx + gw * 300 / 1000;
    let y0 = gy + gh * 300 / 1000;
    let x1 = x0 + gw * 400 / 1000;
    let y1 = y0 + gh * 250 / 1000;
    if x1 <= x0 || y1 <= y0 {
        return 0.0;
    }

    const STRIDE: u32 = 2;
    let mut bins = [0u32; 18];
    let mut sat_hits = 0u32;
    let mut total = 0u32;
    for y in (y0..y1.min(h)).step_by(STRIDE as usize) {
        for x in (x0..x1.min(w)).step_by(STRIDE as usize) {
            let [r, g, b] = rgb.get_pixel(x, y).0;
            total += 1;
            let (hue, sat, val) = rgb_to_hsv(r, g, b);
            if sat > 150 && val > 120 {
                sat_hits += 1;
                bins[(hue as usize) / 20] += 1;
            }
        }
    }
    if total == 0 || sat_hits == 0 {
        return 0.0;
    }
    let (dom_bin, _) = bins
        .iter()
        .enumerate()
        .max_by_key(|(_, c)| *c)
        .unwrap_or((0, &0));
    let locked = bins[dom_bin];
    locked as f32 / total as f32
}

fn scoreline_looks_present(
    img: &DynamicImage,
    rgb: &RgbImage,
    stability: Option<&mut FrameStability>,
) -> bool {
    let (w, h) = rgb.dimensions();
    let (gx, gy, gw, gh) = crate::ocr::preprocess::game_rect_16_9(w, h);
    let x0 = gx + gw * 300 / 1000;
    let y0 = gy + gh * 730 / 1000;
    let cw = gw * 400 / 1000;
    let ch = gh * 80 / 1000;
    if cw == 0 || ch == 0 || x0 + cw > w || y0 + ch > h {
        return false;
    }
    let mut white = 0u32;
    let mut total = 0u32;
    for y in (y0..y0 + ch).step_by(2) {
        for x in (x0..x0 + cw).step_by(2) {
            let [r, g, b] = rgb.get_pixel(x, y).0;
            total += 1;
            let (_, sat, val) = rgb_to_hsv(r, g, b);
            if val > 200 && sat < 40 {
                white += 1;
            }
        }
    }
    if total == 0 || (white as f32 / total as f32) < 0.008 {
        return false;
    }
    let crop = img.crop_imm(x0, y0, cw, ch);
    if let Some(stability) = stability
        && !stability.check("scoreline", &crop)
    {
        tracing::trace!("scoreline crop not stable — deferring OCR");
        return false;
    }
    let prepared = crate::ocr::preprocess::prepare_title(&crop);
    match crate::ocr::recognize_prepared(&prepared, "7", Some("ABCDEFGHIJKLMNOPQRSTUVWXYZ ")) {
        Ok(text) => {
            let letters: String = text
                .chars()
                .filter(|c| c.is_ascii_alphabetic())
                .map(|c| c.to_ascii_uppercase())
                .collect();
            letters.contains("FINAL") || letters.contains("SCORE")
        }
        Err(_) => false,
    }
}

fn opponent_ink(r: u8, g: u8, b: u8, dom_hue: u16) -> u8 {
    let r = i16::from(r);
    let g = i16::from(g);
    let b = i16::from(b);
    let v = if hue_near(dom_hue, 300, 40) {
        (r + b) / 2 - g
    } else if hue_near(dom_hue, 50, 40) {
        (r + g) / 2 - b
    } else if hue_near(dom_hue, 0, 25) || hue_near(dom_hue, 360, 25) {
        r - g.max(b)
    } else {
        r.max(g).max(b) - r.min(g).min(b)
    };
    v.clamp(0, 255) as u8
}

fn prepare_end_title(crop: &DynamicImage) -> GrayImage {
    let rgb = crop.to_rgb8();
    let (w, h) = rgb.dimensions();
    let mut hue_bins = [0u32; 18];
    for p in rgb.pixels() {
        let (hue, sat, val) = rgb_to_hsv(p.0[0], p.0[1], p.0[2]);
        if sat > 150 && val > 120 {
            hue_bins[(hue as usize) / 20] += 1;
        }
    }
    let dom_hue = (hue_bins
        .iter()
        .enumerate()
        .max_by_key(|(_, c)| *c)
        .map(|(i, _)| i)
        .unwrap_or(0)
        * 20) as u16;

    // Opponent-color ink for the dominant UI hue (magenta → (r+b)/2−g,
    // gold → (r+g)/2−b, red → r−max(g,b)). CLI tesseract reads the
    // magenta form as "VCTORY!"; hue-locked chroma/binary did not.
    let mut gray = GrayImage::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let [r, g, b] = rgb.get_pixel(x, y).0;
            gray.put_pixel(x, y, Luma([opponent_ink(r, g, b, dom_hue)]));
        }
    }
    let (sw, sh) = gray.dimensions();
    let scale = (120 / sh.max(1)).clamp(1, 4);
    if scale > 1 {
        image::imageops::resize(
            &gray,
            sw * scale,
            sh * scale,
            image::imageops::FilterType::CatmullRom,
        )
    } else {
        gray
    }
}

// Detect the brief VICTORY/DEFEAT full-screen banner (gold or red backdrop).
// OW2 banners saturate >40% of the screen with a very specific color range.
// Previous thresholds (15%, loose color ranges) caused false positives on
// websites with warm/red colors during normal browsing.
//
// Pixel scan uses stride 2 — ratio tests tolerate 1-in-2 sampling.
fn detect_banner(rgb: &RgbImage) -> Option<MatchOutcome> {
    let (w, h) = rgb.dimensions();
    let (gx, gy, gw, gh) = crate::ocr::preprocess::game_rect_16_9(w, h);

    // Sample the middle horizontal band (30%-70% of the 16:9 playfield) where
    // the banner colour flood is most consistent.
    let y_start = gy + gh * 30 / 100;
    let y_end = gy + gh * 70 / 100;
    let x_end = gx + gw;
    let mut gold_count = 0u32;
    let mut red_count = 0u32;
    let mut total = 0u32;
    const STRIDE: u32 = 2;

    for y in (y_start..y_end).step_by(STRIDE as usize) {
        for x in (gx..x_end).step_by(STRIDE as usize) {
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
fn read_result_word(img: &DynamicImage, stability: Option<&mut FrameStability>) -> MatchOutcome {
    ocr_outcome_word(img, 5, 35, 135, 60, "accolade screen", stability)
}

/// Read the result word off the competitive summary (rank update) screen —
/// VICTORY/DEFEAT printed under the big "COMPETITIVE" title, top-left. The
/// background is dark regardless of UI color theme, and the screen stays up
/// 40s+ (the longest-lived outcome signal, surviving even a starved poller).
/// Region measured from a real 16:9 frame: word spans x 4-12.5%, y 16-21%.
fn read_rank_screen_result(
    img: &DynamicImage,
    stability: Option<&mut FrameStability>,
) -> MatchOutcome {
    ocr_outcome_word(img, 10, 145, 150, 80, "rank screen", stability)
}

/// Read the map name printed beside the accolade screen's result word
/// ("DEFEAT  |  COLOSSEO / MATCH TIME: 10:10"). Color-scheme-independent like
/// the result word itself, and the most reliable map source when the in-game
/// top-bar OCR missed all game. Region measured on a real 16:9 frame: map
/// text block spans x 13.5-19%, y 4-8.5%; the crop starts right of the title
/// (a clipped title glyph is harmless — we only search for map names).
pub fn read_accolade_map(img: &DynamicImage) -> Option<String> {
    let (fw, fh) = (img.width(), img.height());
    let (gx, gy, gw, gh) = crate::ocr::preprocess::game_rect_16_9(fw, fh);
    let x = gx + gw * 125 / 1000;
    let y = gy + gh * 35 / 1000;
    let cw = gw * 325 / 1000;
    let ch = gh * 55 / 1000;
    if cw == 0 || ch == 0 || x + cw > fw || y + ch > fh {
        return None;
    }
    let crop = img.crop_imm(x, y, cw, ch);
    let prepared = crate::ocr::preprocess::prepare_title(&crop);
    // PSM 6 (block): the crop holds two short lines (map, match time).
    let text = crate::ocr::recognize_prepared(&prepared, "6", None).ok()?;
    let map = crate::parse::match_map_in_text(&text);
    if let Some(m) = &map {
        tracing::info!(map = %m, raw = %text.trim(), "map read from accolade screen");
    }
    map
}

/// OCR a crop (given in 1/1000ths of the 16:9 playfield via [`game_rect_16_9`])
/// prepared as title text, and map VICTORY/DEFEAT/DRAW to an outcome.
fn ocr_outcome_word(
    img: &DynamicImage,
    x_pm: u32,
    y_pm: u32,
    w_pm: u32,
    h_pm: u32,
    context: &'static str,
    stability: Option<&mut FrameStability>,
) -> MatchOutcome {
    let (fw, fh) = (img.width(), img.height());
    let (gx, gy, gw, gh) = crate::ocr::preprocess::game_rect_16_9(fw, fh);
    let x = gx + gw * x_pm / 1000;
    let y = gy + gh * y_pm / 1000;
    let cw = gw * w_pm / 1000;
    let ch = gh * h_pm / 1000;
    if cw == 0 || ch == 0 || x + cw > fw || y + ch > fh {
        return MatchOutcome::Unknown;
    }
    let crop = img.crop_imm(x, y, cw, ch);

    // P8: most idle ticks are in-game — the title crop is near-black. Skip the
    // Lanczos+Otsu+Tess pipeline when the crop has no bright glyph mass.
    if !title_crop_has_signal(&crop) {
        return MatchOutcome::Unknown;
    }

    // PR-A: brightness cannot distinguish a lit combat frame from a real title
    // (see title_crop_has_signal), so on the poll path additionally require the
    // crop to have held still since the previous tick. Result screens are
    // static for 15-20s+; combat never is. `context` doubles as the history
    // key so the accolade and rank crops track independently.
    if let Some(stability) = stability
        && !stability.check(context, &crop)
    {
        tracing::trace!(context, "title crop lit but not stable — deferring OCR");
        return MatchOutcome::Unknown;
    }

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

/// Cheap pre-gate on a title crop: skip OCR only when the crop is near-black
/// (no glyph can be present). Deliberately NOT a "does this look like a title"
/// test — measured on the outcome fixtures, bright in-game frames light up
/// this region far more than a real DEFEAT title on a custom magenta UI theme
/// does (0.6% of samples at r+g+b>480 vs 15–22% on gameplay frames), so
/// brightness cannot distinguish title from game world; anything non-black
/// must go to the color-independent Otsu+Tesseract path. Max-channel is the
/// theme-independent glyph test: real titles measure ≥13% at >200 (magenta
/// defeat: 23%), while the rank crop is exactly 0% on in-game/transition
/// frames — that skip is the actual per-tick saving.
fn title_crop_has_signal(crop: &DynamicImage) -> bool {
    let rgb = crop.to_rgb8();
    let (w, h) = rgb.dimensions();
    if w == 0 || h == 0 {
        return false;
    }
    let mut lit = 0u32;
    let mut total = 0u32;
    // Sample ~every 4th pixel — enough for a go/no-go decision.
    for y in (0..h).step_by(4) {
        for x in (0..w).step_by(4) {
            let [r, g, b] = rgb.get_pixel(x, y).0;
            total += 1;
            if r.max(g).max(b) > 200 {
                lit += 1;
            }
        }
    }
    // 1% threshold = 13–23× below every measured real title.
    total > 0 && (lit as f32 / total as f32) > 0.01
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

    #[test]
    fn fuzzy_outcome_accepts_near_miss_and_rejects_ambiguity() {
        assert_eq!(fuzzy_outcome_word("VCTORY!"), MatchOutcome::Victory);
        assert_eq!(fuzzy_outcome_word("SVICTORY"), MatchOutcome::Victory);
        assert_eq!(fuzzy_outcome_word("DEFEA"), MatchOutcome::Defeat);
        assert_eq!(fuzzy_outcome_word("DRAW"), MatchOutcome::Draw);
        // VECTOR is uniquely closer to VICTORY than DEFEAT/DRAW at lev≤2.
        assert_eq!(fuzzy_outcome_word("VECTOR"), MatchOutcome::Victory);
        assert_eq!(fuzzy_outcome_word(""), MatchOutcome::Unknown);
        assert_eq!(fuzzy_outcome_word("HELLO"), MatchOutcome::Unknown);
        // Two targets inside the window → Unknown (do not pick a winner).
        assert_eq!(fuzzy_outcome_word("VICTORYDEFEAT"), MatchOutcome::Unknown);
        // 2-letter fragments are lev≤2 from DRAW; must not become a Draw.
        assert_eq!(fuzzy_outcome_word("DR"), MatchOutcome::Unknown);
        assert_eq!(fuzzy_outcome_word("RA"), MatchOutcome::Unknown);
        assert_eq!(fuzzy_outcome_word("AW"), MatchOutcome::Unknown);
    }

    #[test]
    fn center_title_mass_sees_magenta_blob_not_black() {
        let mut img = RgbImage::from_pixel(640, 360, Rgb([10, 10, 10]));
        // Magenta title band: x 30–70%, y 35–50% of the 16:9 frame.
        for y in 126..180 {
            for x in 192..448 {
                img.put_pixel(x, y, Rgb([220, 40, 220]));
            }
        }
        assert!(
            center_title_mass(&img) >= END_TITLE_MASS_MIN,
            "mass={}",
            center_title_mass(&img)
        );
        assert_eq!(
            center_title_mass(&RgbImage::from_pixel(640, 360, Rgb([0, 0, 0]))),
            0.0
        );
    }

    #[test]
    fn gold_flood_still_wins_over_end_title() {
        assert_eq!(
            detect_outcome_signal(&flood([230, 180, 20])),
            Some((MatchOutcome::Victory, OutcomeSource::Banner))
        );
    }
}
