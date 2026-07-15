use image::{DynamicImage, RgbImage};

use super::GamePhase;

/// Detect map-vote / hero-ban / hero-select phase.
///
/// Converts the frame to RGB once; detectors share that buffer (P6).
pub fn detect_phase(img: &DynamicImage) -> GamePhase {
    let rgb = img.to_rgb8();
    detect_phase_with_rgb(img, &rgb)
}

/// Same as [`detect_phase`] when the caller already holds an RGB conversion
/// (e.g. poll tick that also runs banner detection on the same frame).
pub fn detect_phase_with_rgb(img: &DynamicImage, rgb: &RgbImage) -> GamePhase {
    if let Some(phase) = detect_map_vote(img, rgb) {
        return phase;
    }
    if detect_hero_ban(img, rgb) {
        return GamePhase::HeroBan;
    }
    if detect_hero_select(img, rgb) {
        return GamePhase::HeroSelect;
    }
    GamePhase::Unknown
}

/// Pixel-scan stride: ratio tests tolerate 1-in-2 sampling and cut CPU ~4×.
const SCAN_STRIDE: u32 = 2;

/// The raw pixel-gate measurements every phase detector thresholds on.
/// Exposed so `examples/phaseprobe.rs` (and fixture tests) can show HOW FAR
/// a frame is from each gate — the 2026-07-14 session-merge went undiagnosed
/// for a full evening because a failed gate produced no signal at all.
#[derive(Debug, Clone, Copy)]
pub struct GateMetrics {
    /// Dark-navy ratio in the top quarter (map-vote gate, needs ≥0.40).
    pub navy_ratio: f32,
    /// Red-accent ratio in the top sixth (hero-ban gate, needs ≥0.05).
    pub ban_red_ratio: f32,
    /// Dark ratio in the top sixth (hero-ban gate, needs ≥0.30).
    pub ban_dark_ratio: f32,
    /// Dark ratio in the top eighth (hero-select gate, needs ≥0.50).
    pub select_dark_header_ratio: f32,
    /// Color variance across the lower-half sample grid (hero-select gate,
    /// needs ≥2000).
    pub select_grid_variance: f64,
}

/// Measure every phase pixel gate on a frame without running any OCR.
pub fn gate_metrics(rgb: &RgbImage) -> GateMetrics {
    let (red, dark) = ban_ratios(rgb);
    GateMetrics {
        navy_ratio: navy_ratio(rgb),
        ban_red_ratio: red,
        ban_dark_ratio: dark,
        select_dark_header_ratio: select_header_dark_ratio(rgb),
        select_grid_variance: select_grid_variance(rgb),
    }
}

/// Dark-navy background ratio in the top quarter of the frame (map-vote gate).
fn navy_ratio(rgb: &RgbImage) -> f32 {
    let (w, h) = rgb.dimensions();
    let mut navy_count = 0u32;
    let mut total = 0u32;

    // Sample the top quarter and side margins (avoiding the map cards in center)
    for y in (0..(h / 4)).step_by(SCAN_STRIDE as usize) {
        for x in (0..w).step_by(SCAN_STRIDE as usize) {
            let pixel = rgb.get_pixel(x, y);
            let [r, g, b] = pixel.0;
            total += 1;
            if r < 70 && g < 70 && b > 70 && (b as i32 - r as i32) > 30 {
                navy_count += 1;
            }
        }
    }

    if total == 0 {
        return 0.0;
    }
    navy_count as f32 / total as f32
}

fn detect_map_vote(img: &DynamicImage, rgb: &RgbImage) -> Option<GamePhase> {
    let (w, h) = rgb.dimensions();

    // Map vote screen in OW2 has a dark blue/navy background with slight gradient.
    // The center third of the screen contains 3 map preview cards.
    // Top area often shows "MAP VOTE" or timer text.
    // Characteristic: dark navy pixels (R<60, G<60, B>80) dominate the background.
    let navy_ratio = navy_ratio(rgb);
    if navy_ratio < 0.40 {
        return None;
    }

    // Confirm with OCR on the top portion looking for "VOTE" or "MAP"
    let top_region = img.crop_imm(w / 4, 0, w / 2, h / 6);
    match crate::ocr::recognize_region(&top_region) {
        Ok(text) => {
            let upper = text.to_uppercase();
            if upper.contains("VOTE") || upper.contains("MAP") {
                let maps = extract_map_names(&upper);
                tracing::info!(navy_ratio, maps = ?maps, "map vote screen detected");
                Some(GamePhase::MapVote { maps })
            } else {
                tracing::debug!(
                    navy_ratio,
                    text = %upper.chars().take(80).collect::<String>(),
                    "map-vote pixel gate passed but OCR did not confirm"
                );
                None
            }
        }
        Err(_) => None,
    }
}

/// (red-accent, dark) ratios in the top sixth of the frame (hero-ban gates).
fn ban_ratios(rgb: &RgbImage) -> (f32, f32) {
    let (w, h) = rgb.dimensions();
    let header_h = h / 6;
    let mut red_accent = 0u32;
    let mut dark_count = 0u32;
    let mut total = 0u32;

    for y in (0..header_h).step_by(SCAN_STRIDE as usize) {
        for x in (0..w).step_by(SCAN_STRIDE as usize) {
            let pixel = rgb.get_pixel(x, y);
            let [r, g, b] = pixel.0;
            total += 1;
            if r > 150 && g < 80 && b < 80 {
                red_accent += 1;
            }
            if r < 50 && g < 50 && b < 70 {
                dark_count += 1;
            }
        }
    }

    if total == 0 {
        return (0.0, 0.0);
    }
    (
        red_accent as f32 / total as f32,
        dark_count as f32 / total as f32,
    )
}

fn detect_hero_ban(img: &DynamicImage, rgb: &RgbImage) -> bool {
    let (w, h) = rgb.dimensions();

    // Hero ban screen has a distinctive red/orange tint in the header area
    // and shows "BAN" text. The background is darker than normal gameplay.
    let header_h = h / 6;
    let (red_ratio, dark_ratio) = ban_ratios(rgb);

    // Need significant red accent (ban UI) combined with dark background
    if red_ratio < 0.05 || dark_ratio < 0.30 {
        return false;
    }

    // Confirm with OCR
    let top_region = img.crop_imm(w / 4, 0, w / 2, header_h);
    match crate::ocr::recognize_region(&top_region) {
        Ok(text) => {
            let upper = text.to_uppercase();
            if upper.contains("BAN") {
                tracing::info!(red_ratio, "hero ban screen detected");
                true
            } else {
                tracing::debug!(
                    red_ratio,
                    dark_ratio,
                    text = %upper.chars().take(80).collect::<String>(),
                    "hero-ban pixel gate passed but OCR did not confirm"
                );
                false
            }
        }
        Err(_) => false,
    }
}

/// Dark ratio in the top eighth of the frame (hero-select header gate).
fn select_header_dark_ratio(rgb: &RgbImage) -> f32 {
    let (w, h) = rgb.dimensions();
    let header_h = h / 8;
    let mut dark_header = 0u32;
    let mut header_total = 0u32;

    for y in (0..header_h).step_by(SCAN_STRIDE as usize) {
        for x in (0..w).step_by(SCAN_STRIDE as usize) {
            let pixel = rgb.get_pixel(x, y);
            let [r, g, b] = pixel.0;
            header_total += 1;
            if r < 80 && g < 80 && b < 100 {
                dark_header += 1;
            }
        }
    }

    if header_total == 0 {
        return 0.0;
    }
    dark_header as f32 / header_total as f32
}

/// Color variance across a sample grid of the lower half (hero-grid gate).
fn select_grid_variance(rgb: &RgbImage) -> f64 {
    let (w, h) = rgb.dimensions();
    let bottom_start = h / 2;
    let step_x = (w / 50).max(1);
    let step_y = ((h - bottom_start) / 20).max(1);
    let mut colors: Vec<[u8; 3]> = Vec::new();

    for sy in 0..20 {
        for sx in 0..50 {
            let x = sx * step_x;
            let y = bottom_start + sy * step_y;
            if x < w && y < h {
                let pixel = rgb.get_pixel(x, y);
                colors.push(pixel.0);
            }
        }
    }

    if colors.len() < 100 {
        return 0.0;
    }

    // Hero grid should be colorful — high variance across the samples.
    let avg_r = colors.iter().map(|c| c[0] as f64).sum::<f64>() / colors.len() as f64;
    let avg_g = colors.iter().map(|c| c[1] as f64).sum::<f64>() / colors.len() as f64;
    let avg_b = colors.iter().map(|c| c[2] as f64).sum::<f64>() / colors.len() as f64;

    colors
        .iter()
        .map(|c| {
            let dr = c[0] as f64 - avg_r;
            let dg = c[1] as f64 - avg_g;
            let db = c[2] as f64 - avg_b;
            dr * dr + dg * dg + db * db
        })
        .sum::<f64>()
        / colors.len() as f64
}

fn detect_hero_select(img: &DynamicImage, rgb: &RgbImage) -> bool {
    let (w, h) = rgb.dimensions();

    // Hero select screen characteristics:
    // - Top portion shows "CHOOSE YOUR HERO" or "ASSEMBLE YOUR TEAM"
    // - Has a bright, colorful hero grid in the lower 2/3
    // - Top banner area is relatively dark with text
    // - Bottom area has high color variance from hero portraits

    let header_h = h / 8;
    let dark_ratio = select_header_dark_ratio(rgb);
    if dark_ratio < 0.50 {
        return false;
    }

    let variance = select_grid_variance(rgb);
    if variance < 2000.0 {
        return false;
    }

    // Confirm with OCR on header
    let top_region = img.crop_imm(w / 4, 0, w / 2, header_h);
    match crate::ocr::recognize_region(&top_region) {
        Ok(text) => {
            let upper = text.to_uppercase();
            let is_hero_select =
                upper.contains("CHOOSE") || upper.contains("HERO") || upper.contains("ASSEMBLE");
            if is_hero_select {
                tracing::info!(variance, "hero select screen detected");
            } else {
                tracing::debug!(
                    dark_ratio,
                    variance,
                    text = %upper.chars().take(80).collect::<String>(),
                    "hero-select pixel gates passed but OCR did not confirm"
                );
            }
            is_hero_select
        }
        Err(_) => false,
    }
}

const MAP_NAMES: &[&str] = &[
    "CIRCUIT ROYAL",
    "DORADO",
    "HAVANA",
    "JUNKERTOWN",
    "RIALTO",
    "ROUTE 66",
    "SHAMBALI",
    "WATCHPOINT",
    "GIBRALTAR",
    "BLIZZARD WORLD",
    "EICHENWALDE",
    "HOLLYWOOD",
    // Not bare "KING"/"ROW" — they substring-match unrelated text
    // ("WRECKING", "BROWN"); apostrophe loss in OCR is covered by both forms.
    "KING'S ROW",
    "KINGS ROW",
    "MIDTOWN",
    "NUMBANI",
    "PARAISO",
    "NEON JUNCTION",
    "ANTARCTIC",
    "BUSAN",
    "ILIOS",
    "LIJIANG",
    "NEPAL",
    "OASIS",
    "SAMOA",
    "COLOSSEO",
    "ESPERANCA",
    "NEW QUEEN",
    "RUNASAPI",
    "NEW JUNK",
    "SURAVASA",
    "HANAOKA",
    "THRONE",
    "ANUBIS",
    "AATLIS",
];

fn extract_map_names(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    for &name in MAP_NAMES {
        if text.contains(name) {
            found.push(name.to_string());
        }
    }
    found
}
