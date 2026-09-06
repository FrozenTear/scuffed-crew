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
    fuzzy_outcome_word_inner(raw, 2)
}

/// Accolade/rank (Tab) path: same VICTORY/DEFEAT window as EndTitle, but DRAW
/// is exact-contains only. Tab has no sat-mass gate and no two-read confirm;
/// lev≤1 still accepts `DRA`/`RAW` (distance 1 from DRAW).
fn fuzzy_outcome_word_accolade(raw: &str) -> MatchOutcome {
    fuzzy_outcome_word_inner(raw, 0)
}

fn fuzzy_outcome_word_inner(raw: &str, draw_max_dist: usize) -> MatchOutcome {
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
        .filter(|(word, outcome)| {
            if letters.contains(word) {
                return true;
            }
            if letters.len() + 1 < word.len() {
                return false;
            }
            let max_dist = if matches!(outcome, MatchOutcome::Draw) {
                draw_max_dist
            } else {
                2
            };
            strsim::levenshtein(&letters, word) <= max_dist
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

/// Cheap Play of the Game / highlight end-reel hint for the poller.
///
/// Not an outcome confirm — banner / two-agreeing word OCR are unchanged.
/// The poller uses this only to leave mid-match slow cadence (~8s) so the
/// short Victory/Defeat window that follows the reel is sampled at full
/// rate. Accolade / MVP lands ~15–20s after POTG (see [`read_result_word`]).
///
/// Three cheap paths, either is enough to wake:
/// 1. Cinematic letterbox (dark top+bottom bars, lit middle) — no OCR.
/// 2. Title-band OCR for PLAY OF THE GAME / HIGHLIGHT INTRO, gated on
///    [`cinematic_title_band`] so mid-fight ticks do not pay Tesseract.
/// 3. Nameplate POTG title card (large orange player name + white title
///    glyphs). Chat / endorsement toasts light the cinematic band, so path
///    2 never runs on a real nameplate frame (`known-potg-003708`). Phrase
///    OCR is still required — the orange+white gate only decides whether
///    to pay Tesseract.
pub fn detect_end_reel(img: &DynamicImage, rgb: &RgbImage) -> bool {
    if end_reel_letterbox(rgb) {
        tracing::info!("end-reel letterbox — POTG / highlight wake");
        return true;
    }
    if read_end_reel_title(img) {
        tracing::info!("end-reel title — POTG / highlight wake");
        return true;
    }
    if read_end_reel_nameplate(img, rgb) {
        tracing::info!("end-reel nameplate — POTG / highlight wake");
        return true;
    }
    false
}

/// OW2 POTG / highlight replay letterboxes the 16:9 playfield. Mid-match HUD
/// lights the top bar, so both-bars-dark plus a lit middle is a cheap
/// discriminator. A fade-to-black frame fails the middle check.
fn end_reel_letterbox(rgb: &RgbImage) -> bool {
    let (w, h) = rgb.dimensions();
    let (gx, gy, gw, gh) = crate::ocr::preprocess::game_rect_16_9(w, h);
    let band = (gh * 70 / 1000).max(1);
    if band * 2 >= gh {
        return false;
    }
    let top_dark = dark_ratio(rgb, gx, gy, gw, band);
    let bot_dark = dark_ratio(rgb, gx, gy + gh - band, gw, band);
    let mid_y = gy + gh * 300 / 1000;
    let mid_h = gh * 400 / 1000;
    let mid_dark = dark_ratio(rgb, gx, mid_y, gw, mid_h);
    top_dark >= 0.72 && bot_dark >= 0.72 && mid_dark < 0.50
}

fn dark_ratio(rgb: &RgbImage, x0: u32, y0: u32, cw: u32, ch: u32) -> f32 {
    let (w, h) = rgb.dimensions();
    let x1 = (x0 + cw).min(w);
    let y1 = (y0 + ch).min(h);
    if x1 <= x0 || y1 <= y0 {
        return 0.0;
    }
    const STRIDE: u32 = 2;
    let mut dark = 0u32;
    let mut total = 0u32;
    for y in (y0..y1).step_by(STRIDE as usize) {
        for x in (x0..x1).step_by(STRIDE as usize) {
            let [r, g, b] = rgb.get_pixel(x, y).0;
            total += 1;
            if r.max(g).max(b) < 36 {
                dark += 1;
            }
        }
    }
    if total == 0 {
        0.0
    } else {
        dark as f32 / total as f32
    }
}

/// Title-card path: dark cinematic band + bright glyphs, then phrase OCR.
fn read_end_reel_title(img: &DynamicImage) -> bool {
    let Some(crop) = end_reel_cinematic_crop(img) else {
        return false;
    };
    if !cinematic_title_band(&crop) {
        return false;
    }
    ocr_end_reel_phrase(
        crate::ocr::preprocess::prepare_title(&crop),
        "end-reel title",
    )
}

/// Wide upper band where a cinematic POTG / highlight overlay sits.
/// High enough to miss the in-world end-title V/D word (y ~34–52%).
fn end_reel_cinematic_crop(img: &DynamicImage) -> Option<DynamicImage> {
    playfield_crop(img, 120, 80, 760, 220)
}

/// Nameplate POTG title card: orange name + white title glyphs, then the
/// same phrase OCR as [`read_end_reel_title`]. Does not require a dark
/// cinematic field — chat, endorsement toasts, and the hero render light
/// the upper band on a real nameplate frame.
fn read_end_reel_nameplate(img: &DynamicImage, rgb: &RgbImage) -> bool {
    if !nameplate_potg_signal(rgb) {
        return false;
    }
    let Some(crop) = end_reel_nameplate_crop(img) else {
        return false;
    };
    if ocr_end_reel_phrase(prepare_nameplate_title(&crop), "end-reel nameplate") {
        return true;
    }
    // Fallback: same Otsu title prep as the cinematic path, in case the
    // left stack sits on a darker field than the toast-lit upper band.
    ocr_end_reel_phrase(
        crate::ocr::preprocess::prepare_title(&crop),
        "end-reel nameplate title-prep",
    )
}

/// Left-half stack: PLAY OF THE GAME above the large orange battletag.
/// Left-biased so the hero render (right two-thirds) stays out of the crop.
/// Tall enough to catch a vertically-centered nameplate; still left of the
/// centered in-world V/D end-title (x ~32–68%).
fn end_reel_nameplate_crop(img: &DynamicImage) -> Option<DynamicImage> {
    playfield_crop(img, 20, 100, 500, 480)
}

fn playfield_crop(
    img: &DynamicImage,
    x_pm: u32,
    y_pm: u32,
    w_pm: u32,
    h_pm: u32,
) -> Option<DynamicImage> {
    let (fw, fh) = (img.width(), img.height());
    let (gx, gy, gw, gh) = crate::ocr::preprocess::game_rect_16_9(fw, fh);
    let x = gx + gw * x_pm / 1000;
    let y = gy + gh * y_pm / 1000;
    let cw = gw * w_pm / 1000;
    let ch = gh * h_pm / 1000;
    if cw == 0 || ch == 0 || x + cw > fw || y + ch > fh {
        return None;
    }
    Some(img.crop_imm(x, y, cw, ch))
}

fn ocr_end_reel_phrase(prepared: GrayImage, context: &'static str) -> bool {
    match crate::ocr::recognize_prepared(&prepared, "6", Some("ABCDEFGHIJKLMNOPQRSTUVWXYZ ")) {
        Ok(text) => {
            let hit = end_reel_title_match(&text);
            if hit {
                tracing::debug!(text = %text.trim(), context, "end-reel title OCR matched");
            } else {
                tracing::trace!(text = %text.trim(), context, "end-reel title OCR did not match");
            }
            hit
        }
        Err(e) => {
            tracing::debug!(error = %e, context, "end-reel title OCR failed");
            false
        }
    }
}

/// Isolate bright desaturated glyphs (PLAY OF THE GAME) on a lit nameplate
/// card. Yellow endorsement toasts and the orange battletag fail the sat
/// cap; mid-tone world fails the value floor. Black-on-white for Tesseract.
fn prepare_nameplate_title(crop: &DynamicImage) -> GrayImage {
    let rgb = crop.to_rgb8();
    let (w, h) = rgb.dimensions();
    let mut ink = GrayImage::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let [r, g, b] = rgb.get_pixel(x, y).0;
            let (_, sat, val) = rgb_to_hsv(r, g, b);
            let v = if val > 200 && sat < 55 { 0 } else { 255 };
            ink.put_pixel(x, y, Luma([v]));
        }
    }
    let scale = (120 / h.max(1)).clamp(1, 4);
    let work = if scale > 1 {
        image::imageops::resize(
            &ink,
            w * scale,
            h * scale,
            image::imageops::FilterType::Lanczos3,
        )
    } else {
        ink
    };
    let (ww, hh) = work.dimensions();
    let mut bordered = GrayImage::from_pixel(ww + 24, hh + 24, Luma([255]));
    image::imageops::replace(&mut bordered, &work, 12, 12);
    bordered
}

/// Cheap nameplate gate: large OW-orange battletag in the left-center plus
/// bright white title glyphs above it. Does not require a dark field.
fn nameplate_potg_signal(rgb: &RgbImage) -> bool {
    let orange = playfield_hit_ratio(rgb, 30, 340, 460, 240, is_potg_orange);
    let white = playfield_hit_ratio(rgb, 30, 140, 460, 260, is_title_white);
    orange >= NAMEPLATE_ORANGE_MIN && white >= NAMEPLATE_WHITE_MIN
}

const NAMEPLATE_ORANGE_MIN: f32 = 0.035;
const NAMEPLATE_WHITE_MIN: f32 = 0.012;

fn is_potg_orange(r: u8, g: u8, b: u8) -> bool {
    let (hue, sat, val) = rgb_to_hsv(r, g, b);
    // POTG battletag orange (~hue 31). Tight enough to drop victory gold
    // (~hue 46) and defeat red (~hue 0).
    hue_near(hue, 30, 12) && sat > 150 && val > 150 && r > g && g > b
}

fn is_title_white(r: u8, g: u8, b: u8) -> bool {
    let (_, sat, val) = rgb_to_hsv(r, g, b);
    val > 205 && sat < 45
}

fn playfield_hit_ratio(
    rgb: &RgbImage,
    x_pm: u32,
    y_pm: u32,
    w_pm: u32,
    h_pm: u32,
    pred: impl Fn(u8, u8, u8) -> bool,
) -> f32 {
    let (w, h) = rgb.dimensions();
    let (gx, gy, gw, gh) = crate::ocr::preprocess::game_rect_16_9(w, h);
    let x0 = gx + gw * x_pm / 1000;
    let y0 = gy + gh * y_pm / 1000;
    let x1 = (x0 + gw * w_pm / 1000).min(w);
    let y1 = (y0 + gh * h_pm / 1000).min(h);
    if x1 <= x0 || y1 <= y0 {
        return 0.0;
    }
    const STRIDE: u32 = 2;
    let mut hits = 0u32;
    let mut total = 0u32;
    for y in (y0..y1).step_by(STRIDE as usize) {
        for x in (x0..x1).step_by(STRIDE as usize) {
            let [r, g, b] = rgb.get_pixel(x, y).0;
            total += 1;
            if pred(r, g, b) {
                hits += 1;
            }
        }
    }
    if total == 0 {
        0.0
    } else {
        hits as f32 / total as f32
    }
}

/// Dark cinematic field plus some bright glyphs — skips mid-fight sky/HUD
/// (bright world, low dark ratio) without a Tesseract call.
fn cinematic_title_band(crop: &DynamicImage) -> bool {
    if !title_crop_has_signal(crop) {
        return false;
    }
    let rgb = crop.to_rgb8();
    let (w, h) = rgb.dimensions();
    if w == 0 || h == 0 {
        return false;
    }
    let mut dark = 0u32;
    let mut total = 0u32;
    for y in (0..h).step_by(4) {
        for x in (0..w).step_by(4) {
            let [r, g, b] = rgb.get_pixel(x, y).0;
            total += 1;
            if r.max(g).max(b) < 50 {
                dark += 1;
            }
        }
    }
    total > 0 && (dark as f32 / total as f32) > 0.40
}

/// Phrase check for POTG / highlight-intro title OCR. Letter-only so spaces
/// and punctuation drop out; short tokens like PLAY/GAME alone do not match.
fn end_reel_title_match(raw: &str) -> bool {
    let letters: String = raw
        .chars()
        .filter(|c| c.is_ascii_alphabetic())
        .map(|c| c.to_ascii_uppercase())
        .collect();
    if letters.is_empty() {
        return false;
    }
    if letters.contains("PLAYOFTHEGAME") || letters.contains("HIGHLIGHTINTRO") {
        return true;
    }
    const TARGETS: &[&str] = &["PLAYOFTHEGAME", "HIGHLIGHTINTRO"];
    TARGETS
        .iter()
        .any(|t| letters.len() + 2 >= t.len() && strsim::levenshtein(&letters, t) <= 3)
}

/// Read the large top-left VICTORY/DEFEAT title off the post-match accolade /
/// MVP screen (shown ~15-20s after Play of the Game).
/// Region: x 0.5-25.5%, y 3.5-9.5% of the 16:9 playfield.
///
/// The right edge was 14% until 2026-08-18 (fleet::tracker-wl C6). On the
/// current title font a 2560x1440 "DEFEAT" already spans x 2.7-12.0%
/// (2026-07-15 endcards frame) and the seven wider glyphs of "VICTORY"
/// project past 14%, so the crop clipped the word to "VICTOR"/"VICTO" and
/// victories went unread while defeats still landed (store 07-31..08-17:
/// V=10 vs D=39; 08-17 was five wins, 5/5 unknown). The map name printed
/// right of the title enters the wider crop, and Tesseract PSM 7 gives up on
/// the mixed line ("" at >= 21.5% on the 05-30 reference, "DEFEATJT" on the
/// 07-15 frame), so the crop is OCR'd through `prepare_title_trimmed`, which
/// cuts the binary back to the tall title glyphs; measured on both fixtures
/// the 25% crop then reads "VICTORY" / "DEFEAT" exactly like the old 14% one.
fn read_result_word(img: &DynamicImage, stability: Option<&mut FrameStability>) -> MatchOutcome {
    ocr_outcome_word(img, 5, 35, 250, 60, "accolade screen", stability)
}

/// Read the result word off the competitive summary (rank update) screen —
/// VICTORY/DEFEAT printed under the big "COMPETITIVE" title, top-left. The
/// background is dark regardless of UI color theme, and the screen stays up
/// 40s+ (the longest-lived outcome signal, surviving even a starved poller).
/// Region measured from a real 16:9 frame: word spans x 4-12.5%, y 16-21%.
/// Right edge widened 16% -> 25% alongside the accolade crop (C6): same
/// title face, same "VICTORY is wider than DEFEAT" clipping risk — journal
/// 08-13..08-17 read DEFEAT off this screen 78 times and VICTORY zero.
fn read_rank_screen_result(
    img: &DynamicImage,
    stability: Option<&mut FrameStability>,
) -> MatchOutcome {
    ocr_outcome_word(img, 10, 145, 240, 80, "rank screen", stability)
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

    // Trimmed: the widened crops (C6) run into the map/time block right of
    // the title; the trim cuts the binary back to the title glyphs.
    let prepared = crate::ocr::preprocess::prepare_title_trimmed(&crop);

    match crate::ocr::recognize_prepared(&prepared, "7", Some("ABCDEFGHIJKLMNOPQRSTUVWXYZ")) {
        Ok(text) => {
            let outcome = fuzzy_outcome_word_accolade(&text);
            if outcome.is_decided() {
                let word = match outcome {
                    MatchOutcome::Victory => "VICTORY",
                    MatchOutcome::Defeat => "DEFEAT",
                    MatchOutcome::Draw => "DRAW",
                    MatchOutcome::Unknown => "UNKNOWN",
                };
                tracing::info!(text = %text.trim(), context, "result word: {word}");
            } else {
                tracing::trace!(ocr_text = %text.trim(), context, "no result word in region");
            }
            outcome
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
    fn accolade_fuzzy_reads_near_victory_and_defeat() {
        assert_eq!(
            fuzzy_outcome_word_accolade("VCTORY!"),
            MatchOutcome::Victory
        );
        assert_eq!(fuzzy_outcome_word_accolade("DEFEATM"), MatchOutcome::Defeat);
        assert_eq!(fuzzy_outcome_word_accolade("DRAW"), MatchOutcome::Draw);
        assert_eq!(fuzzy_outcome_word_accolade("DRW"), MatchOutcome::Unknown);
    }

    #[test]
    fn accolade_fuzzy_rejects_three_letter_draw_garbage() {
        // Tab path has no sat-mass gate and no 2-read confirm. lev≤2 on DRAW
        // would accept DRA/RAW/DAG; keep those Unknown on this path.
        assert_eq!(fuzzy_outcome_word_accolade("DRA"), MatchOutcome::Unknown);
        assert_eq!(fuzzy_outcome_word_accolade("RAW"), MatchOutcome::Unknown);
        assert_eq!(fuzzy_outcome_word_accolade("DAG"), MatchOutcome::Unknown);
        assert_eq!(fuzzy_outcome_word_accolade("HELLO"), MatchOutcome::Unknown);
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

    #[test]
    fn end_reel_title_match_accepts_potg_phrases() {
        assert!(end_reel_title_match("PLAY OF THE GAME"));
        assert!(end_reel_title_match("YOUR PLAY OF THE GAME"));
        assert!(end_reel_title_match("PLAYOFTHEGAME"));
        assert!(end_reel_title_match("HIGHLIGHT INTRO"));
        assert!(end_reel_title_match("P1AY OF THE GAME")); // letters → PAYOFTHEGAME, lev 1
        assert!(end_reel_title_match("PLAY OF THE GARIE"));
        assert!(!end_reel_title_match("VICTORY"));
        assert!(!end_reel_title_match("DEFEAT"));
        assert!(!end_reel_title_match("PLAY"));
        assert!(!end_reel_title_match("GAME"));
        assert!(!end_reel_title_match("HIGHLIGHTS"));
        assert!(!end_reel_title_match("HELLO"));
        assert!(!end_reel_title_match(""));
    }

    fn letterbox_frame(mid: [u8; 3]) -> RgbImage {
        let mut img = RgbImage::from_pixel(640, 360, Rgb(mid));
        // 7% of 360 = 25.2 → paint 30px bars so the 70/1000 band is black.
        for y in 0..30 {
            for x in 0..640 {
                img.put_pixel(x, y, Rgb([8, 8, 8]));
                img.put_pixel(x, 359 - y, Rgb([8, 8, 8]));
            }
        }
        img
    }

    #[test]
    fn end_reel_letterbox_needs_dark_bars_and_lit_middle() {
        let reel = letterbox_frame([80, 90, 100]);
        assert!(end_reel_letterbox(&reel), "letterboxed reel should wake");
        assert!(
            detect_end_reel(&DynamicImage::ImageRgb8(reel.clone()), &reel),
            "letterbox path should wake without title OCR"
        );

        let black = RgbImage::from_pixel(640, 360, Rgb([0, 0, 0]));
        assert!(
            !end_reel_letterbox(&black),
            "fade-to-black is not a reel (middle is dark)"
        );
        assert!(!detect_end_reel(
            &DynamicImage::ImageRgb8(black.clone()),
            &black
        ));

        let gray = RgbImage::from_pixel(640, 360, Rgb([80, 80, 80]));
        assert!(!end_reel_letterbox(&gray));

        let gold = RgbImage::from_pixel(640, 360, Rgb([230, 180, 20]));
        assert!(
            !end_reel_letterbox(&gold),
            "victory banner flood is not a letterbox"
        );
    }

    #[test]
    fn cinematic_title_band_needs_dark_field_and_glyphs() {
        let dark = DynamicImage::ImageRgb8(RgbImage::from_pixel(80, 40, Rgb([10, 10, 10])));
        assert!(
            !cinematic_title_band(&dark),
            "dark with no glyphs should skip OCR"
        );
        let sky = DynamicImage::ImageRgb8(RgbImage::from_pixel(80, 40, Rgb([180, 200, 220])));
        assert!(
            !cinematic_title_band(&sky),
            "bright mid-fight sky should skip OCR"
        );
        let mut card = RgbImage::from_pixel(80, 40, Rgb([12, 12, 12]));
        for y in 10..30 {
            for x in 10..70 {
                if x % 3 == 0 {
                    card.put_pixel(x, y, Rgb([220, 220, 220]));
                }
            }
        }
        assert!(cinematic_title_band(&DynamicImage::ImageRgb8(card)));
    }

    /// Lit indoor frame + toast + chat + orange name + white title glyphs.
    /// Matches the measured miss on known-potg-003708: no letterbox, no
    /// cinematic dark band, but the nameplate stack is present.
    fn nameplate_potg_frame() -> RgbImage {
        let mut img = RgbImage::from_pixel(640, 360, Rgb([110, 95, 85]));
        for y in 0..360 {
            for x in 280..640 {
                img.put_pixel(x, y, Rgb([150, 120, 100]));
            }
        }
        // Yellow endorsement toast in the cinematic band.
        for y in 40..70 {
            for x in 20..280 {
                img.put_pixel(x, y, Rgb([240, 210, 40]));
            }
        }
        // Chat-like bright line at the top-left.
        for y in 8..14 {
            for x in 10..180 {
                img.put_pixel(x, y, Rgb([255, 140, 40]));
            }
        }
        // White title glyphs in the nameplate title band (y ~14–40%).
        // Thick strokes so stride-2 sampling still hits (real italic stems
        // are several pixels wide; a 1-px lattice can miss every sample).
        for y in 90..120 {
            for x in 30..300 {
                if x % 8 < 3 {
                    img.put_pixel(x, y, Rgb([235, 235, 235]));
                }
            }
        }
        // Large orange battletag (y ~34–58%).
        for y in 140..190 {
            for x in 30..320 {
                img.put_pixel(x, y, Rgb([255, 150, 40]));
            }
        }
        img
    }

    #[test]
    fn nameplate_potg_signal_fires_when_cinematic_band_does_not() {
        let frame = nameplate_potg_frame();
        let img = DynamicImage::ImageRgb8(frame.clone());
        assert!(
            !end_reel_letterbox(&frame),
            "nameplate card is not letterboxed"
        );
        let crop = end_reel_cinematic_crop(&img).expect("cinematic crop");
        assert!(
            !cinematic_title_band(&crop),
            "toast + hero light the cinematic band — old gate must miss"
        );
        let orange = playfield_hit_ratio(&frame, 30, 340, 460, 240, is_potg_orange);
        let white = playfield_hit_ratio(&frame, 30, 140, 460, 260, is_title_white);
        assert!(
            nameplate_potg_signal(&frame),
            "orange name + white title must open the nameplate path (orange={orange:.4} white={white:.4})"
        );
    }

    #[test]
    fn nameplate_potg_signal_rejects_combat_and_partial_cues() {
        let sky = RgbImage::from_pixel(640, 360, Rgb([180, 200, 220]));
        assert!(!nameplate_potg_signal(&sky), "mid-fight sky");
        assert!(!detect_end_reel(
            &DynamicImage::ImageRgb8(sky.clone()),
            &sky
        ));

        let mut orange_only = RgbImage::from_pixel(640, 360, Rgb([40, 40, 40]));
        for y in 140..190 {
            for x in 30..320 {
                orange_only.put_pixel(x, y, Rgb([255, 150, 40]));
            }
        }
        assert!(
            !nameplate_potg_signal(&orange_only),
            "orange blob without white title glyphs"
        );

        let mut white_only = RgbImage::from_pixel(640, 360, Rgb([40, 40, 40]));
        for y in 90..120 {
            for x in 30..300 {
                if x % 8 < 3 {
                    white_only.put_pixel(x, y, Rgb([235, 235, 235]));
                }
            }
        }
        assert!(
            !nameplate_potg_signal(&white_only),
            "white HUD without orange name"
        );

        let gold = RgbImage::from_pixel(640, 360, Rgb([230, 180, 20]));
        assert!(
            !nameplate_potg_signal(&gold),
            "victory gold flood is not a POTG nameplate"
        );
    }
}
