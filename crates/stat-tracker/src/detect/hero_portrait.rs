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
                if path.extension().and_then(|e| e.to_str()) == Some("png")
                    && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
                {
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

        if let Some(idx) = player_row
            && let Some(result) = crops.get(idx).and_then(|crop| self.match_portrait(crop))
        {
            tracing::info!(row = idx, hero = %result.0, confidence = result.1, "matched player portrait via highlighted row");
            return Some((result.0, result.1, idx));
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
/// OW2 highlights the player's row with a slightly lighter/brighter background.
///
/// Crops each row via `crop_player_row` so the geometry matches the OCR pipeline
/// exactly. Within each row image, samples a thin horizontal strip at the row
/// center (x 34–50% = right name field, before stat columns; y center ±15%) and
/// measures the mean of background-range pixels [15, 90], filtering out both deep
/// shadows and bright icon/title-text outliers.
fn detect_player_row_inner(scoreboard: &DynamicImage, team_size: usize) -> Option<usize> {
    let mut brightnesses: Vec<(usize, f64)> = Vec::new();

    for row in 0..team_size {
        let Some(row_img) =
            crate::ocr::preprocess::crop_player_row(scoreboard, row, team_size)
        else {
            continue;
        };

        let (rw, rh) = (row_img.width(), row_img.height());
        if rw < 10 || rh < 4 {
            continue;
        }

        let x0 = rw * 34 / 100;
        let x1 = (rw * 50 / 100).min(rw);
        let cy = rh / 2;
        let half_y = (rh * 15 / 100).max(2);
        let y0 = cy.saturating_sub(half_y);
        let y1 = (cy + half_y).min(rh);

        let rgb = row_img.to_rgb8();
        let mut total = 0u64;
        let mut count = 0u64;
        for y in y0..y1 {
            for x in x0..x1 {
                let [r, g, b] = rgb.get_pixel(x, y).0;
                let br = (r as u32 + g as u32 + b as u32) / 3;
                if br >= 15 && br <= 90 {
                    total += br as u64;
                    count += 1;
                }
            }
        }

        if count < 20 {
            continue;
        }

        brightnesses.push((row, total as f64 / count as f64));
    }

    if brightnesses.is_empty() {
        return None;
    }

    let (max_row, max_brightness) = brightnesses
        .iter()
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        .unwrap();

    let avg: f64 =
        brightnesses.iter().map(|(_, b)| b).sum::<f64>() / brightnesses.len() as f64;

    if *max_brightness > avg * 1.12 {
        tracing::debug!(
            row = max_row,
            brightness = format!("{max_brightness:.1}"),
            avg = format!("{avg:.1}"),
            "detected player row via background brightness"
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
    let portrait_x = w / 100;

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

/// Detect whether the scoreboard shows 5v5 or 6v6 via the team-1 row pitch.
///
/// Counting hero portraits fails when rows are empty ("WAITING FOR PLAYER") or
/// when custom team colors make the colored row bars contiguous with no dark
/// gaps. Instead we measure saturation across the name strip per scanline: every
/// row — occupied or empty — has white text that dips saturation once per row,
/// so the dip-to-dip pitch reveals the row count regardless of empty slots or
/// team colors. 6v6 rows are tighter (~7.4% of crop height) than 5v5 (~8.3%).
pub fn detect_team_size(scoreboard: &DynamicImage) -> usize {
    let rgb = scoreboard.to_rgb8();
    let (w, h) = rgb.dimensions();
    if w == 0 || h == 0 {
        return 5;
    }

    // Name strip: right of the portrait column, left of the stat columns and the
    // right-side career panel.
    let x0 = (w as f64 * 0.06) as u32;
    let x1 = (w as f64 * 0.55) as u32;
    let denom = (x1 - x0).max(1) as f64;
    let mut sat = vec![0f64; h as usize];
    for y in 0..h {
        let mut sum = 0.0;
        for x in x0..x1 {
            let [r, g, b] = rgb.get_pixel(x, y).0;
            let max = r.max(g).max(b) as f64;
            let min = r.min(g).min(b) as f64;
            if max > 0.0 {
                sum += (max - min) / max;
            }
        }
        sat[y as usize] = sum / denom;
    }

    // Smooth to suppress per-glyph noise.
    let win = (h as f64 * 0.01).max(2.0) as usize;
    let smooth: Vec<f64> = (0..sat.len())
        .map(|i| {
            let lo = i.saturating_sub(win);
            let hi = (i + win + 1).min(sat.len());
            sat[lo..hi].iter().sum::<f64>() / (hi - lo) as f64
        })
        .collect();

    // Row-center dips within team 1 (above the VS divider, ~bottom of team 1).
    let y_lo = (h as f64 * 0.04) as usize;
    let y_hi = ((h as f64 * 0.45) as usize).min(smooth.len().saturating_sub(1));
    let sep = (h as f64 * 0.035).max(2.0) as usize;
    let mut dips: Vec<usize> = Vec::new();
    for y in (y_lo + 1)..y_hi {
        let lo = y.saturating_sub(sep);
        let hi = (y + sep).min(smooth.len() - 1);
        let is_min = (lo..=hi).all(|j| smooth[y] <= smooth[j]);
        let region_max = (lo..=hi).map(|j| smooth[j]).fold(0.0_f64, f64::max);
        if is_min && region_max - smooth[y] > 0.04 && dips.last().is_none_or(|&d| y - d >= sep) {
            dips.push(y);
        }
    }

    let mut pitches: Vec<f64> = dips.windows(2).map(|p| (p[1] - p[0]) as f64).collect();
    if pitches.is_empty() {
        tracing::debug!("team size: no row dips found, defaulting to 5");
        return 5;
    }
    pitches.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let median_pitch = pitches[pitches.len() / 2] / h as f64;

    // Threshold sits between the measured 5v5 (~8.3%) and 6v6 (~7.4%) pitches.
    let team_size = if median_pitch < 0.079 { 6 } else { 5 };
    tracing::debug!(
        median_pitch,
        dip_count = dips.len(),
        team_size,
        "team size via row pitch"
    );
    team_size
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
