use image::{DynamicImage, GrayImage, Luma, Rgb, RgbImage};

/// Scoreboard layout constants for 2560x1440 OW2 fullscreen.
const SCOREBOARD_X_RATIO: f64 = 0.175;
const SCOREBOARD_Y_RATIO: f64 = 0.15;
const SCOREBOARD_W_RATIO: f64 = 0.65;
const SCOREBOARD_H_RATIO: f64 = 0.70;

/// Stat columns: E, A, D, DMG, HLG, MIT
const STAT_COLUMNS: usize = 6;

/// Fallback column boundaries if dynamic detection fails.
const STAT_COL_BOUNDARIES_FALLBACK: [(f64, f64); STAT_COLUMNS] = [
    (0.465, 0.033), // Elims
    (0.503, 0.030), // Assists (no overlap with E)
    (0.538, 0.029), // Deaths
    (0.575, 0.070), // Damage
    (0.650, 0.070), // Healing
    (0.725, 0.050), // Mitigation (narrow to exclude UI warning icon)
];

/// Player name column within each row.
/// Layout (left→right): portrait + level/rank badges (0–26%), name text (26–38%), ability icons (38%+).
/// Original value of 0.09 only captured the portrait area, not the name text.
/// Upper bound set to ~38% to exclude the circular hero-ability icons that OCR reads as "Q"/"O".
const NAME_COL_X: f64 = 0.26;
const NAME_COL_W: f64 = 0.12;
/// 6v6 squeezes the table horizontally: the name plate sits further left.
/// Measured on the 2026-07-16 fixture (12-row board, dumped row_00.png):
/// name text spans ~0.155–0.24 of row width; 0.26+ lands on the E/A/D digits,
/// which is exactly what OCR read before this constant existed.
const NAME_COL_X_6V6: f64 = 0.15;
const NAME_COL_W_6V6: f64 = 0.10;

/// Row layout: header takes ~2.5% of scoreboard height.
/// Team 1 starts immediately after header. Team 2 starts at ~56.5% (measured from
/// real screenshots — the VS divider gap is larger than initially estimated).
const HEADER_RATIO: f64 = 0.025;
const TEAM2_START_RATIO: f64 = 0.565;

pub fn prepare(img: &DynamicImage) -> GrayImage {
    prepare_hsv_adaptive(img)
}

/// HSV-masked adaptive pipeline (Phase 1 improvement):
/// 1. HSV color mask — isolate white/near-white text pixels
/// 2. Convert masked result to grayscale
/// 3. Sauvola thresholding
/// 4. Morphological cleanup
pub fn prepare_hsv_adaptive(img: &DynamicImage) -> GrayImage {
    let masked = hsv_white_mask(img);
    let gray = DynamicImage::ImageRgb8(masked).to_luma8();
    let (w, _) = gray.dimensions();

    let work_img = if w < 1280 {
        nearest_2x_upscale(&gray)
    } else {
        gray
    };

    let binary = sauvola_threshold(&work_img, 25, 0.2, 128.0);
    morphological_close(&binary, 1)
}

/// Legacy global threshold method (kept for fallback/comparison)
pub fn prepare_with_threshold(img: &DynamicImage, threshold: u8) -> GrayImage {
    let gray = img.to_luma8();
    let (w, _) = gray.dimensions();

    let work_img = if w < 1280 {
        nearest_2x_upscale(&gray)
    } else {
        gray
    };

    let filtered = median_filter_3x3(&work_img);

    let mut binary = filtered;
    for px in binary.pixels_mut() {
        px.0[0] = if px.0[0] > threshold { 0 } else { 255 };
    }

    binary
}

/// Adaptive preprocessing pipeline:
/// 1. Convert to grayscale
/// 2. Local contrast enhancement (CLAHE-inspired tile-based)
/// 3. Sauvola thresholding (local mean + stddev)
/// 4. Morphological cleanup
pub fn prepare_adaptive(img: &DynamicImage) -> GrayImage {
    let gray = img.to_luma8();
    let (w, _) = gray.dimensions();

    let work_img = if w < 1280 {
        nearest_2x_upscale(&gray)
    } else {
        gray
    };

    let enhanced = local_contrast_enhance(&work_img, 64);
    let binary = sauvola_threshold(&enhanced, 25, 0.2, 128.0);
    morphological_close(&binary, 1)
}

/// Prepare a single cell crop with parameters tuned for numeric stat text.
/// HSV mask isolates white text pixels; we then apply a simple fixed threshold
/// since the mask already did the heavy lifting. Sauvola on these small, mostly-black
/// post-mask images produces poor results (local mean ≈ 0 → garbage thresholds).
pub fn prepare_cell(img: &DynamicImage) -> GrayImage {
    let masked = hsv_white_mask(img);
    let gray = DynamicImage::ImageRgb8(masked).to_luma8();
    let (w, _) = gray.dimensions();

    let work_img = if w < 150 {
        nearest_2x_upscale(&gray)
    } else {
        gray
    };

    let (ww, hh) = work_img.dimensions();
    let mut binary = GrayImage::new(ww, hh);
    for y in 0..hh {
        for x in 0..ww {
            let v = work_img.get_pixel(x, y).0[0];
            binary.put_pixel(x, y, Luma([if v > 30 { 0 } else { 255 }]));
        }
    }

    add_white_border(&binary, 8)
}

/// Prepare a player name cell — HSV mask + simple threshold (same rationale as prepare_cell).
pub fn prepare_name_cell(img: &DynamicImage) -> GrayImage {
    let masked = hsv_white_mask(img);
    let gray = DynamicImage::ImageRgb8(masked).to_luma8();
    let (w, _) = gray.dimensions();

    let work_img = if w < 200 {
        nearest_2x_upscale(&gray)
    } else {
        gray
    };

    let (ww, hh) = work_img.dimensions();
    let mut binary = GrayImage::new(ww, hh);
    for y in 0..hh {
        for x in 0..ww {
            let v = work_img.get_pixel(x, y).0[0];
            binary.put_pixel(x, y, Luma([if v > 30 { 0 } else { 255 }]));
        }
    }

    add_white_border(&binary, 8)
}

/// Prepare a large title-text region (e.g. the post-match VICTORY/DEFEAT
/// header). Unlike the scoreboard cell paths, this is bright, anti-aliased text
/// over a dark/gradient background, so the HSV white-mask + fixed-threshold
/// pipeline erases it. Instead: grayscale, upscale small crops, then Otsu —
/// which adapts to either cyan (VICTORY) or red (DEFEAT) text without a
/// hand-tuned threshold. Text is the brighter cluster, so it becomes black on
/// white for Tesseract.
pub fn prepare_title(img: &DynamicImage) -> GrayImage {
    let gray = img.to_luma8();
    let (w, h) = gray.dimensions();
    // Upscale small crops so the glyphs are tall enough for Tesseract.
    let scale = (120 / h.max(1)).clamp(1, 4);
    let work = if scale > 1 {
        image::imageops::resize(
            &gray,
            w * scale,
            h * scale,
            image::imageops::FilterType::Lanczos3,
        )
    } else {
        gray
    };

    let threshold = otsu_threshold(&work);
    let (ww, hh) = work.dimensions();
    let mut binary = GrayImage::new(ww, hh);
    for y in 0..hh {
        for x in 0..ww {
            let v = work.get_pixel(x, y).0[0];
            binary.put_pixel(x, y, Luma([if v > threshold { 0 } else { 255 }]));
        }
    }

    add_white_border(&binary, 12)
}

/// Otsu's method: pick the gray level that maximizes between-class variance.
fn otsu_threshold(img: &GrayImage) -> u8 {
    let mut hist = [0u32; 256];
    for p in img.pixels() {
        hist[p.0[0] as usize] += 1;
    }
    let total: u32 = img.width() * img.height();
    if total == 0 {
        return 128;
    }
    let sum: f64 = hist
        .iter()
        .enumerate()
        .map(|(i, &c)| i as f64 * c as f64)
        .sum();
    let (mut sum_b, mut w_b, mut max_var, mut threshold) = (0.0f64, 0u32, -1.0f64, 128u8);
    for (t, &count) in hist.iter().enumerate() {
        w_b += count;
        if w_b == 0 {
            continue;
        }
        let w_f = total - w_b;
        if w_f == 0 {
            break;
        }
        sum_b += t as f64 * count as f64;
        let m_b = sum_b / w_b as f64;
        let m_f = (sum - sum_b) / w_f as f64;
        let var = w_b as f64 * w_f as f64 * (m_b - m_f) * (m_b - m_f);
        if var > max_var {
            max_var = var;
            threshold = t as u8;
        }
    }
    threshold
}

/// Column boundaries as (left_edge_fraction, width_fraction) for each of the 6 stat columns.
pub type StatColumns = [(f64, f64); STAT_COLUMNS];

/// Apply a horizontal offset to the fallback column boundaries.
pub fn columns_with_offset(offset: f64) -> StatColumns {
    let mut columns = STAT_COL_BOUNDARIES_FALLBACK;
    for col in &mut columns {
        col.0 += offset;
    }
    columns
}

/// Detect the column offset by finding stat header labels (E, A, D, DMG, H, MIT)
/// in the scoreboard header area. The header has dark text on a bright bar.
///
/// Groups adjacent dark-text clusters into logical labels, then identifies the
/// 6 stat columns by their characteristic spacing pattern (3 narrow E/A/D, then
/// 3 wider DMG/H/MIT). Returns the offset from the fallback E position.
pub fn detect_column_offset(scoreboard: &DynamicImage) -> f64 {
    let w = scoreboard.width();
    let groups = header_label_groups(scoreboard);
    if groups.is_empty() {
        return 0.0;
    }

    // We expect 6 groups for E, A, D, DMG, H, MIT.
    // The first group should be the "E" label.
    let first_center = (groups[0].0 + groups[0].1) as f64 / 2.0 / w as f64;
    let fallback_e_center =
        STAT_COL_BOUNDARIES_FALLBACK[0].0 + STAT_COL_BOUNDARIES_FALLBACK[0].1 / 2.0;
    let offset = first_center - fallback_e_center;

    tracing::debug!(
        groups = groups.len(),
        first_center_ratio = first_center,
        offset,
        "header dark-text column offset"
    );

    offset
}

/// Dark-text label groups found in the scoreboard header strip, as (start, end)
/// pixel columns. A real scoreboard yields one group per stat label (E, A, D,
/// DMG, H, MIT — six, sometimes merged/split by a step or two). Shared by
/// column calibration and the pre-OCR scoreboard preflight: brightness-based,
/// so it still fires on the desaturated endorse-phase board where the
/// saturation row-dip scan goes blind.
pub fn header_label_groups(scoreboard: &DynamicImage) -> Vec<(u32, u32)> {
    let (w, h) = (scoreboard.width(), scoreboard.height());

    let scan_start = (h as f64 * 0.005) as u32;
    let scan_end = (h as f64 * 0.025).max(15.0) as u32;
    let scan_h = scan_end.saturating_sub(scan_start).max(1);
    // Only convert the thin header strip to RGB — not the whole scoreboard.
    let header = scoreboard.crop_imm(0, scan_start, w, scan_h.min(h.saturating_sub(scan_start)));
    let rgb = header.to_rgb8();
    let scan_rows = rgb.height().max(1);

    // Count dark pixels per column in the header area
    let mut col_dark = vec![0u32; w as usize];
    for y in 0..rgb.height() {
        for x in 0..w {
            let px = rgb.get_pixel(x, y);
            let brightness = (px.0[0] as u32 + px.0[1] as u32 + px.0[2] as u32) / 3;
            if brightness < 150 {
                col_dark[x as usize] += 1;
            }
        }
    }

    // Find dark-text clusters
    let threshold = (scan_rows / 4).max(1);
    let mut raw_clusters: Vec<(u32, u32)> = Vec::new();
    let mut in_cluster = false;
    let mut cluster_start = 0u32;

    for (x, &count) in col_dark.iter().enumerate() {
        if count >= threshold {
            if !in_cluster {
                cluster_start = x as u32;
                in_cluster = true;
            }
        } else if in_cluster {
            if (x as u32) - cluster_start >= 3 {
                raw_clusters.push((cluster_start, x as u32));
            }
            in_cluster = false;
        }
    }

    // Filter to stat area (ratio > 0.25) and merge clusters within 15px
    let stat_area_start = (w as f64 * 0.25) as u32;
    let filtered: Vec<(u32, u32)> = raw_clusters
        .iter()
        .filter(|&&(s, _)| s >= stat_area_start)
        .copied()
        .collect();

    if filtered.is_empty() {
        return Vec::new();
    }

    // Merge nearby clusters into logical groups (individual letter clusters → labels)
    let mut groups: Vec<(u32, u32)> = Vec::new();
    let mut g_start = filtered[0].0;
    let mut g_end = filtered[0].1;
    for &(s, e) in &filtered[1..] {
        if s <= g_end + 15 {
            g_end = e;
        } else {
            groups.push((g_start, g_end));
            g_start = s;
            g_end = e;
        }
    }
    groups.push((g_start, g_end));
    groups
}

// --- Scoreboard geometry ---

pub fn crop_scoreboard(img: &DynamicImage) -> DynamicImage {
    let (w, h) = (img.width(), img.height());
    // OW2's scoreboard renders inside a 16:9 region centered on the frame. On
    // non-16:9 displays (ultrawide, 16:10) the ratios below must apply to that
    // inner region, not the whole frame, or the crop drifts off the scoreboard.
    let (gx, gy, gw, gh) = game_rect_16_9(w, h);
    let x = gx + (gw as f64 * SCOREBOARD_X_RATIO) as u32;
    let y = gy + (gh as f64 * SCOREBOARD_Y_RATIO) as u32;
    let crop_w = ((gw as f64 * SCOREBOARD_W_RATIO) as u32).min(w.saturating_sub(x));
    let crop_h = ((gh as f64 * SCOREBOARD_H_RATIO) as u32).min(h.saturating_sub(y));
    img.crop_imm(x, y, crop_w, crop_h)
}

/// Compute the centered 16:9 sub-rectangle of a frame as (x, y, w, h).
///
/// OW2 renders its HUD within a 16:9 area: wider-than-16:9 frames (ultrawide)
/// are pillarboxed (full height, narrower centered width); taller-than-16:9
/// frames (e.g. 16:10) are letterboxed (full width, shorter centered height).
/// For an exact 16:9 frame this returns the whole frame, so 16:9 capture is
/// byte-for-byte unchanged from the previous behavior.
pub fn game_rect_16_9(w: u32, h: u32) -> (u32, u32, u32, u32) {
    const TARGET: f64 = 16.0 / 9.0;
    if h == 0 {
        return (0, 0, w, h);
    }
    let actual = w as f64 / h as f64;
    if (actual - TARGET).abs() < 0.01 {
        (0, 0, w, h)
    } else if actual > TARGET {
        let gw = (h as f64 * TARGET).round() as u32;
        ((w - gw) / 2, 0, gw, h)
    } else {
        let gh = (w as f64 / TARGET).round() as u32;
        (0, (h - gh) / 2, w, gh)
    }
}

/// Crop the top-bar map-name label (top-right, e.g. "WATCHPOINT: GIBRALTAR").
///
/// This sits ABOVE the scoreboard crop, so scoreboard OCR never sees it. White
/// text on a dark bar — pass to `recognize_region`.
pub fn crop_map_name(img: &DynamicImage) -> DynamicImage {
    let (w, h) = (img.width(), img.height());
    let (gx, gy, gw, gh) = game_rect_16_9(w, h);
    let x = gx + (gw as f64 * 0.68) as u32;
    let y = gy + (gh as f64 * 0.022) as u32;
    let cw = ((gw as f64 * 0.27) as u32).min(w.saturating_sub(x));
    let ch = ((gh as f64 * 0.040) as u32).min(h.saturating_sub(y));
    img.crop_imm(x, y, cw, ch)
}

/// Crop the right-side career panel's hero-name title (e.g. "MOIRA").
///
/// This is the player's currently-selected hero, read as plain text — far more
/// reliable than portrait template matching for confusable heroes (e.g. the
/// orange-haired supports Moira / Illari). White text on a dark panel.
pub fn crop_career_hero(img: &DynamicImage) -> DynamicImage {
    let (w, h) = (img.width(), img.height());
    let (gx, gy, gw, gh) = game_rect_16_9(w, h);
    let x = gx + (gw as f64 * 0.57) as u32;
    let y = gy + (gh as f64 * 0.33) as u32;
    let cw = ((gw as f64 * 0.25) as u32).min(w.saturating_sub(x));
    let ch = ((gh as f64 * 0.045) as u32).min(h.saturating_sub(y));
    img.crop_imm(x, y, cw, ch)
}

/// Extract a single player row from the scoreboard crop.
/// `row_index` is 0..(team_size*2-1). `team_size` is 5 or 6.
pub fn crop_player_row(
    scoreboard: &DynamicImage,
    row_index: usize,
    team_size: usize,
) -> Option<DynamicImage> {
    let total_rows = team_size * 2;
    if row_index >= total_rows {
        return None;
    }

    let (w, h) = (scoreboard.width(), scoreboard.height());
    let team1_start = (h as f64 * HEADER_RATIO) as u32;
    let team2_start = (h as f64 * TEAM2_START_RATIO) as u32;

    let (team, team_row) = if row_index < team_size {
        (0, row_index)
    } else {
        (1, row_index - team_size)
    };

    let row_h = (team2_start - team1_start) / (team_size as u32 + 1);

    let base_y = if team == 0 { team1_start } else { team2_start };
    let y = base_y + (team_row as u32 * row_h);

    let actual_h = row_h.min(h.saturating_sub(y));
    if actual_h < row_h / 2 {
        return None;
    }

    Some(scoreboard.crop_imm(0, y, w, actual_h))
}

/// Extract a stat cell from a player row using dynamic column boundaries.
/// `col_index` is 0-5 (E, A, D, DMG, HLG, MIT).
pub fn crop_stat_cell(
    row: &DynamicImage,
    col_index: usize,
    columns: &StatColumns,
) -> Option<DynamicImage> {
    if col_index >= STAT_COLUMNS {
        return None;
    }

    let (w, h) = (row.width(), row.height());
    let (col_x_ratio, col_w_ratio) = columns[col_index];

    let x = (w as f64 * col_x_ratio).max(0.0) as u32;
    let cell_w = (w as f64 * col_w_ratio) as u32;

    let pad_y = (h as f64 * 0.15) as u32;
    let cell_h = h - (pad_y * 2);

    if x + cell_w > w || pad_y + cell_h > h || cell_w == 0 {
        return None;
    }

    Some(row.crop_imm(x, pad_y, cell_w, cell_h))
}

/// Hard-threshold fallback preparation for name cells whose cosmetic
/// nameplates defeat the HSV white mask (gradient plates, tinted glyphs).
/// Grayscale > 200 keeps only the brightest glyph pixels; output is
/// black-text-on-white for Tesseract, upscaled like the primary path.
pub fn prepare_name_cell_hard_threshold(img: &DynamicImage) -> GrayImage {
    let gray = img.to_luma8();
    // Smooth-upscale BEFORE thresholding: at native row height (~77px) the
    // glyphs are too thin to survive a hard binarization.
    let (w, h) = gray.dimensions();
    let up = image::imageops::resize(&gray, w * 4, h * 4, image::imageops::FilterType::CatmullRom);
    let mut bin = up;
    for p in bin.pixels_mut() {
        p.0[0] = if p.0[0] > 200 { 0 } else { 255 };
    }
    bin
}

/// Extract the player name cell from a row. The window depends on the match
/// layout: 6v6 renders a narrower table than 5v5.
pub fn crop_name_cell(row: &DynamicImage, team_size: usize) -> DynamicImage {
    let (name_x, name_w) = if team_size >= 6 {
        (NAME_COL_X_6V6, NAME_COL_W_6V6)
    } else {
        (NAME_COL_X, NAME_COL_W)
    };
    let (w, h) = (row.width(), row.height());
    let x = (w as f64 * name_x) as u32;
    let cell_w = (w as f64 * name_w) as u32;
    let pad_y = (h as f64 * 0.15) as u32;
    let cell_h = h - (pad_y * 2);

    row.crop_imm(x, pad_y, cell_w.min(w - x), cell_h.min(h - pad_y))
}

/// Get all stat cells for a row as individual images.
pub fn extract_row_cells(row: &DynamicImage, columns: &StatColumns) -> Vec<DynamicImage> {
    (0..STAT_COLUMNS)
        .filter_map(|col| crop_stat_cell(row, col, columns))
        .collect()
}

// --- HSV color masking ---

/// HSV white text isolation for OW2 scoreboard.
/// White text has: any H, low S (< ~50/255), high V (> ~160/255).
/// Non-text pixels (game background) are zeroed out.
///
/// Returns an RGB image where only white/near-white pixels are preserved;
/// everything else is black. This dramatically reduces background noise
/// before grayscale conversion and thresholding.
fn hsv_white_mask(img: &DynamicImage) -> RgbImage {
    let rgb = img.to_rgb8();
    let (w, h) = rgb.dimensions();
    let mut output = RgbImage::new(w, h);

    // Tuned thresholds for OW2 scoreboard text at 1440p:
    // - Saturation ceiling: text is white/gray so S is very low
    // - Value floor: text is bright white
    // - Allow slightly dimmer pixels at panel edges (gradient tolerance)
    const SAT_CEIL: u8 = 60;
    const VAL_FLOOR: u8 = 150;
    // Softer threshold for partial alpha text at panel edges
    const VAL_FLOOR_SOFT: u8 = 120;
    const SAT_CEIL_SOFT: u8 = 80;

    for y in 0..h {
        for x in 0..w {
            let px = rgb.get_pixel(x, y);
            let [r, g, b] = px.0;

            let (_, s, v) = rgb_to_hsv(r, g, b);

            // Hard mask: definitely text
            if s <= SAT_CEIL && v >= VAL_FLOOR {
                output.put_pixel(x, y, *px);
            }
            // Soft mask: possible text at lower brightness (semi-transparent areas)
            // Weight the pixel by how close it is to the hard threshold
            else if s <= SAT_CEIL_SOFT && v >= VAL_FLOOR_SOFT {
                let weight = (v - VAL_FLOOR_SOFT) as f32 / (VAL_FLOOR - VAL_FLOOR_SOFT) as f32;
                let weight = weight.clamp(0.0, 1.0);
                let wr = (r as f32 * weight) as u8;
                let wg = (g as f32 * weight) as u8;
                let wb = (b as f32 * weight) as u8;
                output.put_pixel(x, y, Rgb([wr, wg, wb]));
            }
            // Everything else → black (background eliminated)
        }
    }

    output
}

/// Convert RGB (0-255) to HSV. Returns (H: 0-360, S: 0-255, V: 0-255).
fn rgb_to_hsv(r: u8, g: u8, b: u8) -> (u16, u8, u8) {
    let rf = r as f32 / 255.0;
    let gf = g as f32 / 255.0;
    let bf = b as f32 / 255.0;

    let max = rf.max(gf).max(bf);
    let min = rf.min(gf).min(bf);
    let delta = max - min;

    let v = (max * 255.0) as u8;

    if max == 0.0 {
        return (0, 0, v);
    }

    let s = ((delta / max) * 255.0) as u8;

    if delta == 0.0 {
        return (0, s, v);
    }

    let h = if max == rf {
        60.0 * (((gf - bf) / delta) % 6.0)
    } else if max == gf {
        60.0 * ((bf - rf) / delta + 2.0)
    } else {
        60.0 * ((rf - gf) / delta + 4.0)
    };

    let h = if h < 0.0 { h + 360.0 } else { h };

    (h as u16, s, v)
}

// --- Adaptive thresholding core ---

/// Sauvola binarization: threshold = mean * (1 + k * (stddev / R - 1))
/// Text pixels (bright on dark overlay) are inverted: bright → black for Tesseract.
///
/// `window_size` — local region radius (full window = 2*r+1)
/// `k` — sensitivity parameter (0.2 works well for OW2 text)
/// `r_param` — dynamic range normalization (128 for 8-bit images)
fn sauvola_threshold(img: &GrayImage, window_size: u32, k: f64, r_param: f64) -> GrayImage {
    let (w, h) = img.dimensions();
    let mut output = GrayImage::new(w, h);

    // Build integral image and integral of squared values for O(1) local stats
    let len = (w as usize + 1) * (h as usize + 1);
    let mut integral = vec![0i64; len];
    let mut integral_sq = vec![0i64; len];
    let stride = w as usize + 1;

    for y in 0..h as usize {
        let mut row_sum: i64 = 0;
        let mut row_sq_sum: i64 = 0;
        for x in 0..w as usize {
            let val = img.get_pixel(x as u32, y as u32).0[0] as i64;
            row_sum += val;
            row_sq_sum += val * val;
            integral[(y + 1) * stride + (x + 1)] = row_sum + integral[y * stride + (x + 1)];
            integral_sq[(y + 1) * stride + (x + 1)] =
                row_sq_sum + integral_sq[y * stride + (x + 1)];
        }
    }

    let r = window_size;
    for y in 0..h {
        for x in 0..w {
            let x1 = x.saturating_sub(r) as usize;
            let y1 = y.saturating_sub(r) as usize;
            let x2 = ((x + r + 1) as usize).min(w as usize);
            let y2 = ((y + r + 1) as usize).min(h as usize);

            let area = ((x2 - x1) * (y2 - y1)) as f64;
            let sum = integral[y2 * stride + x2]
                - integral[y1 * stride + x2]
                - integral[y2 * stride + x1]
                + integral[y1 * stride + x1];
            let sq_sum = integral_sq[y2 * stride + x2]
                - integral_sq[y1 * stride + x2]
                - integral_sq[y2 * stride + x1]
                + integral_sq[y1 * stride + x1];

            let mean = sum as f64 / area;
            let variance = (sq_sum as f64 / area) - (mean * mean);
            let stddev = variance.max(0.0).sqrt();

            let threshold = mean * (1.0 + k * (stddev / r_param - 1.0));

            let pixel_val = img.get_pixel(x, y).0[0] as f64;
            // Invert: bright text (above threshold) → black (0) for Tesseract
            let out_val = if pixel_val > threshold { 0u8 } else { 255u8 };
            output.put_pixel(x, y, Luma([out_val]));
        }
    }

    output
}

/// Tile-based local contrast enhancement (CLAHE-inspired).
/// Divides image into tiles, computes local min/max, and stretches contrast.
/// Simpler than full CLAHE but effective for the semi-transparent overlay use case.
fn local_contrast_enhance(img: &GrayImage, tile_size: u32) -> GrayImage {
    let (w, h) = img.dimensions();
    let mut output = GrayImage::new(w, h);

    let tiles_x = w.div_ceil(tile_size);
    let tiles_y = h.div_ceil(tile_size);

    // Compute per-tile min/max
    let mut tile_stats: Vec<(u8, u8)> = vec![(255, 0); (tiles_x * tiles_y) as usize];

    for y in 0..h {
        for x in 0..w {
            let tx = (x / tile_size).min(tiles_x - 1);
            let ty = (y / tile_size).min(tiles_y - 1);
            let idx = (ty * tiles_x + tx) as usize;
            let val = img.get_pixel(x, y).0[0];
            tile_stats[idx].0 = tile_stats[idx].0.min(val);
            tile_stats[idx].1 = tile_stats[idx].1.max(val);
        }
    }

    // Apply local contrast stretch with bilinear interpolation between tiles
    for y in 0..h {
        for x in 0..w {
            let tx = (x / tile_size).min(tiles_x - 1);
            let ty = (y / tile_size).min(tiles_y - 1);
            let idx = (ty * tiles_x + tx) as usize;
            let (local_min, local_max) = tile_stats[idx];

            let val = img.get_pixel(x, y).0[0];
            let range = local_max.saturating_sub(local_min) as f64;
            let stretched = if range < 10.0 {
                // Near-uniform tile — just pass through
                val
            } else {
                (val.saturating_sub(local_min) as f64 / range * 255.0).clamp(0.0, 255.0) as u8
            };
            output.put_pixel(x, y, Luma([stretched]));
        }
    }

    output
}

/// Morphological close (dilate then erode) to fill small gaps in text strokes.
fn morphological_close(img: &GrayImage, radius: u32) -> GrayImage {
    let dilated = morphological_op(img, radius, true);
    morphological_op(&dilated, radius, false)
}

/// Generic morphological operation: dilate (max) or erode (min) with square kernel.
fn morphological_op(img: &GrayImage, radius: u32, dilate: bool) -> GrayImage {
    let (w, h) = img.dimensions();
    let mut output = GrayImage::new(w, h);

    for y in 0..h {
        for x in 0..w {
            let mut extremum = if dilate { 0u8 } else { 255u8 };
            for dy in -(radius as i32)..=(radius as i32) {
                for dx in -(radius as i32)..=(radius as i32) {
                    let sx = (x as i32 + dx).clamp(0, w as i32 - 1) as u32;
                    let sy = (y as i32 + dy).clamp(0, h as i32 - 1) as u32;
                    let val = img.get_pixel(sx, sy).0[0];
                    if dilate {
                        extremum = extremum.max(val);
                    } else {
                        extremum = extremum.min(val);
                    }
                }
            }
            output.put_pixel(x, y, Luma([extremum]));
        }
    }

    output
}

fn median_filter_3x3(img: &GrayImage) -> GrayImage {
    let (w, h) = img.dimensions();
    let mut out = GrayImage::new(w, h);

    for y in 0..h {
        for x in 0..w {
            let mut window = [0u8; 9];
            let mut idx = 0;
            for dy in -1i32..=1 {
                for dx in -1i32..=1 {
                    let sx = (x as i32 + dx).clamp(0, w as i32 - 1) as u32;
                    let sy = (y as i32 + dy).clamp(0, h as i32 - 1) as u32;
                    window[idx] = img.get_pixel(sx, sy).0[0];
                    idx += 1;
                }
            }
            window.sort_unstable();
            out.put_pixel(x, y, Luma([window[4]]));
        }
    }

    out
}

/// Add a white border around the image. Tesseract performs better when text
/// doesn't touch the image edge.
fn add_white_border(img: &GrayImage, pad: u32) -> GrayImage {
    let (w, h) = img.dimensions();
    let mut out = GrayImage::from_pixel(w + pad * 2, h + pad * 2, Luma([255]));
    for y in 0..h {
        for x in 0..w {
            out.put_pixel(x + pad, y + pad, *img.get_pixel(x, y));
        }
    }
    out
}

fn nearest_2x_upscale(img: &GrayImage) -> GrayImage {
    let (w, h) = img.dimensions();
    let mut upscaled = GrayImage::new(w * 2, h * 2);
    for y in 0..h {
        for x in 0..w {
            let px = *img.get_pixel(x, y);
            upscaled.put_pixel(x * 2, y * 2, px);
            upscaled.put_pixel(x * 2 + 1, y * 2, px);
            upscaled.put_pixel(x * 2, y * 2 + 1, px);
            upscaled.put_pixel(x * 2 + 1, y * 2 + 1, px);
        }
    }
    upscaled
}

// --- Debug support ---

/// Save intermediate preprocessing stages to debug directory.
pub fn save_debug_stages(img: &DynamicImage, debug_dir: &std::path::Path) {
    let _ = std::fs::create_dir_all(debug_dir);

    // Stage 0: Original input
    let _ = img.save(debug_dir.join("00_original.png"));

    // Stage 1: HSV white mask (new Phase 1 step)
    let masked = hsv_white_mask(img);
    let _ = DynamicImage::ImageRgb8(masked.clone()).save(debug_dir.join("01_hsv_masked.png"));

    // Stage 2: Grayscale of masked image
    let gray = DynamicImage::ImageRgb8(masked).to_luma8();
    let _ = DynamicImage::ImageLuma8(gray.clone()).save(debug_dir.join("02_grayscale.png"));

    // Stage 3: Sauvola binary
    let binary = sauvola_threshold(&gray, 25, 0.2, 128.0);
    let _ = DynamicImage::ImageLuma8(binary.clone()).save(debug_dir.join("03_sauvola_binary.png"));

    // Stage 4: Morphological close (final)
    let final_img = morphological_close(&binary, 1);
    let _ = DynamicImage::ImageLuma8(final_img).save(debug_dir.join("04_final.png"));
}
