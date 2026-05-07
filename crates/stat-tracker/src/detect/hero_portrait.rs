use std::collections::HashMap;
use std::path::{Path, PathBuf};

use image::{DynamicImage, RgbImage, imageops::FilterType};

const PORTRAIT_SIZE: u32 = 32;

include!(concat!(env!("OUT_DIR"), "/bundled_portraits.rs"));

pub struct PortraitMatcher {
    references: HashMap<String, RgbImage>,
}

impl PortraitMatcher {
    pub fn load(portraits_dir: &Path) -> Self {
        // Unpack bundled portraits if the directory is empty or missing
        unpack_bundled(portraits_dir);

        let mut references = HashMap::new();

        if let Ok(entries) = std::fs::read_dir(portraits_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("png") {
                    if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                        match image::open(&path) {
                            Ok(img) => {
                                let resized = img
                                    .resize_exact(PORTRAIT_SIZE, PORTRAIT_SIZE, FilterType::Lanczos3)
                                    .to_rgb8();
                                references.insert(stem.to_string(), resized);
                            }
                            Err(e) => {
                                tracing::warn!(path = %path.display(), error = %e, "failed to load portrait reference");
                            }
                        }
                    }
                }
            }
        }

        tracing::info!(count = references.len(), "loaded hero portrait references");
        Self { references }
    }

    pub fn is_empty(&self) -> bool {
        self.references.is_empty()
    }

    pub fn match_portrait(&self, crop: &DynamicImage) -> Option<(String, f64)> {
        if self.references.is_empty() {
            return None;
        }

        let candidate = crop
            .resize_exact(PORTRAIT_SIZE, PORTRAIT_SIZE, FilterType::Lanczos3)
            .to_rgb8();

        let mut best_name = None;
        let mut best_score = f64::MAX;

        for (name, reference) in &self.references {
            let score = mean_absolute_difference(&candidate, reference);
            if score < best_score {
                best_score = score;
                best_name = Some(name.clone());
            }
        }

        // Confidence: invert MAD to 0-1 range (0 = no match, 1 = perfect)
        // MAD ranges from 0 (identical) to 255 (maximum difference)
        let confidence = 1.0 - (best_score / 255.0);

        // Reject matches below threshold
        if confidence < 0.70 {
            tracing::debug!(best_score, confidence, "portrait match below threshold");
            return None;
        }

        best_name.map(|name| (name, confidence))
    }

    pub fn match_all_portraits(&self, scoreboard: &DynamicImage) -> Vec<(String, f64)> {
        let crops = extract_portrait_crops(scoreboard);
        crops
            .iter()
            .filter_map(|crop| self.match_portrait(crop))
            .collect()
    }

    pub fn match_player_portrait(
        &self,
        scoreboard: &DynamicImage,
        player_row_index: Option<usize>,
    ) -> Option<(String, f64)> {
        let crops = extract_portrait_crops(scoreboard);
        let idx = player_row_index.unwrap_or(0);
        crops.get(idx).and_then(|crop| self.match_portrait(crop))
    }

    pub fn match_player_hero(&self, scoreboard: &DynamicImage) -> Option<(String, f64, usize)> {
        let team_size = detect_team_size(scoreboard);
        let player_row = detect_player_row_inner(scoreboard, team_size);
        let crops = extract_portrait_crops_inner(scoreboard, team_size);

        if let Some(idx) = player_row {
            if let Some(result) = crops.get(idx).and_then(|crop| self.match_portrait(crop)) {
                tracing::info!(row = idx, hero = %result.0, confidence = result.1, "matched player portrait via highlighted row");
                return Some((result.0, result.1, idx));
            }
        }

        // Log all team 1 portrait matches for debugging, but do NOT use them
        // as the result. Picking the highest-confidence teammate portrait caused
        // wrong-hero attribution when the highlighted row wasn't detected.
        tracing::debug!("highlighted row detection failed, falling back to OCR text");
        let mut all_matches: Vec<(usize, String, f64)> = Vec::new();
        for (i, crop) in crops.iter().enumerate().take(team_size) {
            if let Some((name, conf)) = self.match_portrait(crop) {
                all_matches.push((i, name, conf));
            }
        }
        if !all_matches.is_empty() {
            tracing::debug!(
                matches = ?all_matches.iter().map(|(i, n, c)| format!("row {i}: {n} ({c:.2})")).collect::<Vec<_>>(),
                "team 1 portrait matches (debug only, not used without highlighted row)"
            );
        }

        None
    }
}

/// Detect which row in team 1 is the player's own row.
/// OW2 highlights the player's row with a brighter/glowing background.
/// Returns the row index (0-based within team 1).
fn detect_player_row_inner(scoreboard: &DynamicImage, team_size: usize) -> Option<usize> {
    let rgb = scoreboard.to_rgb8();
    let (w, h) = rgb.dimensions();

    let row_height = match team_size {
        6 => h * 58 / 1000,
        _ => h * 7 / 100,
    };
    let start_y = h * 12 / 100;

    // Sample a horizontal strip in the name area (right of portrait, left of stats)
    let sample_x_start = w * 10 / 100;
    let sample_x_end = w * 30 / 100;
    let row_margin = row_height / 4;

    let mut brightnesses: Vec<(usize, f64)> = Vec::new();

    for row in 0..team_size as u32 {
        let y_start = start_y + row * row_height + row_margin;
        let y_end = start_y + (row + 1) * row_height - row_margin;

        let mut total: u64 = 0;
        let mut count: u64 = 0;

        for y in y_start..y_end.min(h) {
            for x in sample_x_start..sample_x_end.min(w) {
                let [r, g, b] = rgb.get_pixel(x, y).0;
                total += r as u64 + g as u64 + b as u64;
                count += 1;
            }
        }

        if count > 0 {
            brightnesses.push((row as usize, total as f64 / (count * 3) as f64));
        }
    }

    if brightnesses.is_empty() {
        return None;
    }

    let (max_row, max_brightness) = brightnesses
        .iter()
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        .unwrap();

    let avg: f64 = brightnesses.iter().map(|(_, b)| b).sum::<f64>() / brightnesses.len() as f64;

    // The highlighted row should be meaningfully brighter than the average
    if *max_brightness > avg * 1.15 {
        tracing::debug!(
            row = max_row,
            brightness = format!("{max_brightness:.1}"),
            avg = format!("{avg:.1}"),
            "detected player row via brightness"
        );
        Some(*max_row)
    } else {
        tracing::debug!(
            max_brightness = format!("{max_brightness:.1}"),
            avg = format!("{avg:.1}"),
            "no clearly highlighted row detected"
        );
        None
    }
}

/// Extract hero portrait crops from a scoreboard screenshot.
/// OW2 Tab scoreboard layout (after crop_scoreboard):
/// - 5 or 6 player rows per team (5v5 or 6v6)
/// - Hero portrait is at the left edge of each row
/// - Rows start ~12% from top
/// - Portrait occupies roughly leftmost 5-6% of width, square
fn extract_portrait_crops(scoreboard: &DynamicImage) -> Vec<DynamicImage> {
    extract_portrait_crops_inner(scoreboard, detect_team_size(scoreboard))
}

fn extract_portrait_crops_inner(scoreboard: &DynamicImage, team_size: usize) -> Vec<DynamicImage> {
    let (w, h) = (scoreboard.width(), scoreboard.height());
    let portrait_w = w * 6 / 100;
    let portrait_h = portrait_w; // square
    let portrait_x = w * 1 / 100;

    // Adjust row height based on team size
    // 5v5: rows take ~7% of height each, 6v6: rows take ~5.8% each
    let row_height = match team_size {
        6 => h * 58 / 1000,
        _ => h * 7 / 100,
    };
    let start_y = h * 12 / 100;

    // Gap between teams scales with team size
    let team2_offset = row_height * (team_size as u32 + 1);

    let mut crops = Vec::with_capacity(team_size * 2);

    for team in 0..2u32 {
        let base_y = start_y + if team == 0 { 0 } else { team2_offset };
        for row in 0..team_size as u32 {
            let y = base_y + row * row_height;
            if y + portrait_h <= h && portrait_x + portrait_w <= w {
                let crop = scoreboard.crop_imm(portrait_x, y, portrait_w, portrait_h);
                crops.push(crop);
            }
        }
    }

    crops
}

/// Detect whether the scoreboard shows 5v5 or 6v6 by counting distinct
/// portrait-sized bright regions in the left column.
pub fn detect_team_size(scoreboard: &DynamicImage) -> usize {
    let rgb = scoreboard.to_rgb8();
    let (w, h) = rgb.dimensions();
    let scan_x = w * 3 / 100; // center of portrait column
    let start_y = h * 10 / 100;
    let end_y = h * 55 / 100; // first team only occupies top half

    // Scan vertically and count transitions from dark to non-dark
    let mut in_row = false;
    let mut row_count = 0;
    let row_min_height = h * 3 / 100; // minimum height to count as a row
    let mut row_pixels = 0u32;

    for y in start_y..end_y {
        let pixel = rgb.get_pixel(scan_x, y);
        let [r, g, b] = pixel.0;
        let brightness = (r as u32 + g as u32 + b as u32) / 3;

        if brightness > 40 {
            if !in_row {
                in_row = true;
                row_pixels = 0;
            }
            row_pixels += 1;
        } else {
            if in_row && row_pixels >= row_min_height {
                row_count += 1;
            }
            in_row = false;
        }
    }
    if in_row && row_pixels >= row_min_height {
        row_count += 1;
    }

    if row_count >= 6 {
        6
    } else {
        5
    }
}

fn mean_absolute_difference(a: &RgbImage, b: &RgbImage) -> f64 {
    let (w, h) = a.dimensions();
    assert_eq!((w, h), b.dimensions());

    let mut total_diff: u64 = 0;
    let pixel_count = (w * h) as u64;

    for (pa, pb) in a.pixels().zip(b.pixels()) {
        let dr = (pa.0[0] as i32 - pb.0[0] as i32).unsigned_abs() as u64;
        let dg = (pa.0[1] as i32 - pb.0[1] as i32).unsigned_abs() as u64;
        let db = (pa.0[2] as i32 - pb.0[2] as i32).unsigned_abs() as u64;
        total_diff += dr + dg + db;
    }

    total_diff as f64 / (pixel_count * 3) as f64
}

/// Save a portrait crop as a reference image.
/// The filename is the hero name (lowercase, no spaces).
pub fn save_portrait_reference(
    portraits_dir: &Path,
    hero_name: &str,
    crop: &DynamicImage,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    std::fs::create_dir_all(portraits_dir)?;
    let filename = hero_name.to_lowercase().replace(' ', "_").replace('.', "");
    let path = portraits_dir.join(format!("{filename}.png"));
    let resized = crop.resize_exact(PORTRAIT_SIZE, PORTRAIT_SIZE, FilterType::Lanczos3);
    resized.save(&path)?;
    tracing::info!(hero = hero_name, path = %path.display(), "saved portrait reference");
    Ok(path)
}

/// Get the portraits directory path within the data dir.
pub fn portraits_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("portraits")
}

/// Unpack bundled portraits to the data directory if they don't already exist.
/// User-provided portraits in the directory take precedence (not overwritten).
fn unpack_bundled(portraits_dir: &Path) {
    let bundled = bundled_portraits();
    if bundled.is_empty() {
        return;
    }

    if let Err(e) = std::fs::create_dir_all(portraits_dir) {
        tracing::warn!(error = %e, "failed to create portraits directory");
        return;
    }

    let mut unpacked = 0;
    for (name, data) in bundled {
        let path = portraits_dir.join(format!("{name}.png"));
        if !path.exists() {
            if let Err(e) = std::fs::write(&path, data) {
                tracing::warn!(hero = name, error = %e, "failed to unpack bundled portrait");
            } else {
                unpacked += 1;
            }
        }
    }

    if unpacked > 0 {
        tracing::info!(count = unpacked, "unpacked bundled portrait references");
    }
}
