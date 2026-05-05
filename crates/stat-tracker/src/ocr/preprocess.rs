use image::{DynamicImage, GrayImage, Luma};

/// Scoreboard layout constants for 2560x1440 OW2 fullscreen.
/// The Tab scoreboard is centered with ~65% width, ~70% height.
const SCOREBOARD_X_RATIO: f64 = 0.175;
const SCOREBOARD_Y_RATIO: f64 = 0.15;
const SCOREBOARD_W_RATIO: f64 = 0.65;
const SCOREBOARD_H_RATIO: f64 = 0.70;

/// Number of player rows visible on the scoreboard (5v5 = 10 rows)
const PLAYER_ROWS: usize = 10;

/// Stat columns: E, A, D, DMG, HLG, MIT
const STAT_COLUMNS: usize = 6;

/// Relative column boundaries within the scoreboard crop (left edge fraction, width fraction).
/// Tuned from OW2 1440p captures. The stat columns occupy the right ~55% of each row.
const STAT_COL_BOUNDARIES: [(f64, f64); STAT_COLUMNS] = [
    (0.52, 0.07), // Elims
    (0.59, 0.07), // Assists
    (0.66, 0.07), // Deaths
    (0.73, 0.09), // Damage
    (0.82, 0.09), // Healing
    (0.91, 0.09), // Mitigation
];

/// Player name column within each row
const NAME_COL_X: f64 = 0.08;
const NAME_COL_W: f64 = 0.20;

/// Row layout: header takes ~8% of scoreboard height, rows split remainder.
/// Team 1 rows occupy top half, team 2 bottom half, with a small gap between.
const HEADER_RATIO: f64 = 0.08;
const TEAM_GAP_RATIO: f64 = 0.04;

pub fn prepare(img: &DynamicImage) -> GrayImage {
    prepare_adaptive(img)
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
/// Uses tighter Sauvola window since cells contain isolated short strings.
pub fn prepare_cell(img: &DynamicImage) -> GrayImage {
    let gray = img.to_luma8();
    let (w, _) = gray.dimensions();

    let work_img = if w < 80 {
        nearest_2x_upscale(&gray)
    } else {
        gray
    };

    let enhanced = local_contrast_enhance(&work_img, 32);
    let binary = sauvola_threshold(&enhanced, 15, 0.15, 128.0);
    morphological_close(&binary, 1)
}

/// Prepare a player name cell — slightly different params since names are wider text.
pub fn prepare_name_cell(img: &DynamicImage) -> GrayImage {
    let gray = img.to_luma8();
    let (w, _) = gray.dimensions();

    let work_img = if w < 200 {
        nearest_2x_upscale(&gray)
    } else {
        gray
    };

    let enhanced = local_contrast_enhance(&work_img, 48);
    let binary = sauvola_threshold(&enhanced, 21, 0.18, 128.0);
    morphological_close(&binary, 1)
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
/// `row_index` is 0-9 (0-4 = team 1, 5-9 = team 2).
pub fn crop_player_row(scoreboard: &DynamicImage, row_index: usize) -> Option<DynamicImage> {
    if row_index >= PLAYER_ROWS {
        return None;
    }

    let (w, h) = (scoreboard.width(), scoreboard.height());
    let header_h = (h as f64 * HEADER_RATIO) as u32;
    let gap_h = (h as f64 * TEAM_GAP_RATIO) as u32;
    let usable_h = h - header_h - gap_h;
    let row_h = usable_h / PLAYER_ROWS as u32;

    let (team, team_row) = if row_index < 5 {
        (0, row_index)
    } else {
        (1, row_index - 5)
    };

    let y = header_h + (team as u32 * (usable_h / 2 + gap_h)) + (team_row as u32 * row_h);

    if y + row_h > h {
        return None;
    }

    Some(scoreboard.crop_imm(0, y, w, row_h))
}

/// Extract a stat cell from a player row.
/// `col_index` is 0-5 (E, A, D, DMG, HLG, MIT).
pub fn crop_stat_cell(row: &DynamicImage, col_index: usize) -> Option<DynamicImage> {
    if col_index >= STAT_COLUMNS {
        return None;
    }

    let (w, h) = (row.width(), row.height());
    let (col_x_ratio, col_w_ratio) = STAT_COL_BOUNDARIES[col_index];

    let x = (w as f64 * col_x_ratio) as u32;
    let cell_w = (w as f64 * col_w_ratio) as u32;

    // Vertically trim to the middle ~70% of the row to avoid border artifacts
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
pub fn extract_row_cells(row: &DynamicImage) -> Vec<DynamicImage> {
    (0..STAT_COLUMNS)
        .filter_map(|col| crop_stat_cell(row, col))
        .collect()
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
    let gray = img.to_luma8();
    let enhanced = local_contrast_enhance(&gray, 64);
    let binary = sauvola_threshold(&enhanced, 25, 0.2, 128.0);
    let final_img = morphological_close(&binary, 1);

    let _ = std::fs::create_dir_all(debug_dir);
    let _ = DynamicImage::ImageLuma8(gray).save(debug_dir.join("01_grayscale.png"));
    let _ = DynamicImage::ImageLuma8(enhanced).save(debug_dir.join("02_contrast_enhanced.png"));
    let _ = DynamicImage::ImageLuma8(binary).save(debug_dir.join("03_sauvola_binary.png"));
    let _ = DynamicImage::ImageLuma8(final_img).save(debug_dir.join("04_morphological_close.png"));
}
