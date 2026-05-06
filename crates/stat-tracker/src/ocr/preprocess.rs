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
    (0.465, 0.033),  // Elims
    (0.503, 0.030),  // Assists (no overlap with E)
    (0.538, 0.030),  // Deaths (no overlap with A)
    (0.575, 0.070),  // Damage
    (0.650, 0.070),  // Healing
    (0.725, 0.055),  // Mitigation (narrower to exclude UI warning icon)
];

/// Player name column within each row
const NAME_COL_X: f64 = 0.09;
const NAME_COL_W: f64 = 0.22;

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

    let work_img = if w < 100 {
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

/// Column boundaries as (left_edge_fraction, width_fraction) for each of the 6 stat columns.
pub type StatColumns = [(f64, f64); STAT_COLUMNS];

/// Return stat column boundaries. Uses fixed positions calibrated for the
/// most common OW2 scoreboard layout at 2560x1440.
pub fn detect_stat_columns(_scoreboard: &DynamicImage) -> StatColumns {
    STAT_COL_BOUNDARIES_FALLBACK
}

// --- Scoreboard geometry ---

pub fn crop_scoreboard(img: &DynamicImage) -> DynamicImage {
    let (w, h) = (img.width(), img.height());
    let x = (w as f64 * SCOREBOARD_X_RATIO) as u32;
    let y = (h as f64 * SCOREBOARD_Y_RATIO) as u32;
    let crop_w = (w as f64 * SCOREBOARD_W_RATIO) as u32;
    let crop_h = (h as f64 * SCOREBOARD_H_RATIO) as u32;
    img.crop_imm(x, y, crop_w, crop_h)
}

/// Extract a single player row from the scoreboard crop.
/// `row_index` is 0..(team_size*2-1). `team_size` is 5 or 6.
pub fn crop_player_row(scoreboard: &DynamicImage, row_index: usize, team_size: usize) -> Option<DynamicImage> {
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
pub fn crop_stat_cell(row: &DynamicImage, col_index: usize, columns: &StatColumns) -> Option<DynamicImage> {
    if col_index >= STAT_COLUMNS {
        return None;
    }

    let (w, h) = (row.width(), row.height());
    let (col_x_ratio, col_w_ratio) = columns[col_index];

    let x = (w as f64 * col_x_ratio).max(0.0) as u32;
    let cell_w = (w as f64 * col_w_ratio) as u32;

    let pad_y = (h as f64 * 0.15) as u32;
    let cell_h = h - (pad_y * 2);

    if x + cell_w > w || pad_y + cell_h > h {
        return None;
    }

    Some(row.crop_imm(x, pad_y, cell_w, cell_h))
}

/// Extract the player name cell from a row.
pub fn crop_name_cell(row: &DynamicImage) -> DynamicImage {
    let (w, h) = (row.width(), row.height());
    let x = (w as f64 * NAME_COL_X) as u32;
    let cell_w = (w as f64 * NAME_COL_W) as u32;
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
            integral_sq[(y + 1) * stride + (x + 1)] = row_sq_sum + integral_sq[y * stride + (x + 1)];
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
            let sum = integral[y2 * stride + x2] - integral[y1 * stride + x2]
                - integral[y2 * stride + x1] + integral[y1 * stride + x1];
            let sq_sum = integral_sq[y2 * stride + x2] - integral_sq[y1 * stride + x2]
                - integral_sq[y2 * stride + x1] + integral_sq[y1 * stride + x1];

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

    let tiles_x = (w + tile_size - 1) / tile_size;
    let tiles_y = (h + tile_size - 1) / tile_size;

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
                let norm = (val.saturating_sub(local_min) as f64 / range * 255.0)
                    .clamp(0.0, 255.0) as u8;
                norm
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
