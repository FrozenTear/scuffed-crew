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
        self.match_player_hero_with_team_size(scoreboard, team_size)
    }

    /// Like [`Self::match_player_hero`] when the caller already detected team size
    /// (avoids a second full scoreboard saturation scan — P7).
    pub fn match_player_hero_with_team_size(
        &self,
        scoreboard: &DynamicImage,
        team_size: usize,
    ) -> Option<(String, f64, usize)> {
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
        let Some(row_img) = crate::ocr::preprocess::crop_player_row(scoreboard, row, team_size)
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
                if (15..=90).contains(&br) {
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

    let avg: f64 = brightnesses.iter().map(|(_, b)| b).sum::<f64>() / brightnesses.len() as f64;

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

/// Pixel rectangle of the hero portrait for a scoreboard row.
///
/// Coordinates are relative to a scoreboard crop (`crop_scoreboard` output).
/// `row_idx` is 0-based across both teams (0..team_size*2). Canonical layout
/// used by portrait matching; call sites that collect portrait references must
/// use this instead of inlined 5v5-only geometry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortraitRect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

/// Shared portrait geometry for 5v5 / 6v6 scoreboard crops.
///
/// OW2 Tab scoreboard layout (after `crop_scoreboard`):
/// - 5 or 6 player rows per team
/// - Hero portrait at the left edge of each row
/// - Rows start ~12% from top
/// - Portrait ~leftmost 5–6% of width, square
/// - One-row team gap between team 1 and team 2
pub fn portrait_rect(
    scoreboard_dims: (u32, u32),
    row_idx: usize,
    team_size: usize,
) -> Option<PortraitRect> {
    let (w, h) = scoreboard_dims;
    if w == 0 || h == 0 || team_size == 0 {
        return None;
    }
    let total_rows = team_size * 2;
    if row_idx >= total_rows {
        return None;
    }

    let portrait_w = w * 6 / 100;
    let portrait_h = portrait_w; // square
    let portrait_x = w / 100;

    // 5v5: rows take ~7% of height each; 6v6: ~5.8% each
    let row_height = match team_size {
        6 => h * 58 / 1000,
        _ => h * 7 / 100,
    };
    let start_y = h * 12 / 100;
    // Gap between teams scales with team size (one extra row-height of padding)
    let team2_offset = row_height * (team_size as u32 + 1);

    let team = (row_idx / team_size) as u32;
    let row_in_team = (row_idx % team_size) as u32;
    let base_y = start_y + if team == 0 { 0 } else { team2_offset };
    let y = base_y + row_in_team * row_height;

    if y + portrait_h > h || portrait_x + portrait_w > w {
        return None;
    }

    Some(PortraitRect {
        x: portrait_x,
        y,
        w: portrait_w,
        h: portrait_h,
    })
}

/// Extract hero portrait crops from a scoreboard screenshot.
fn extract_portrait_crops(scoreboard: &DynamicImage) -> Vec<DynamicImage> {
    extract_portrait_crops_inner(scoreboard, detect_team_size(scoreboard))
}

fn extract_portrait_crops_inner(scoreboard: &DynamicImage, team_size: usize) -> Vec<DynamicImage> {
    let dims = (scoreboard.width(), scoreboard.height());
    let mut crops = Vec::with_capacity(team_size * 2);

    for row_idx in 0..(team_size * 2) {
        if let Some(r) = portrait_rect(dims, row_idx, team_size) {
            crops.push(scoreboard.crop_imm(r.x, r.y, r.w, r.h));
        }
    }

    crops
}

/// Row-structure signal measured from the team-1 name strip: saturation dips
/// mark row centers regardless of occupancy or team colors. Computed once per
/// capture and shared by team-size detection and the pre-OCR scoreboard
/// preflight.
#[derive(Debug, Clone, Copy)]
pub struct RowScan {
    /// Number of row-center saturation dips found in the team-1 band.
    pub dip_count: usize,
    /// Median dip-to-dip pitch as a fraction of crop height (None with <2 dips).
    pub median_pitch: Option<f64>,
    /// Row pitch from the saturation profile's DFT peak, as a fraction of
    /// crop height. More robust than dip spacing on sparse boards (a 0:00
    /// all-zero scoreboard renders too little white text for every row to
    /// produce its own dip, but the phase-coherent periodicity survives —
    /// plain autocorrelation does NOT, the broad valley swamps it). None when
    /// the spectrum has no interior peak strong enough to trust.
    pub spectral_pitch: Option<f64>,
}

impl RowScan {
    /// Cheap non-OCR scoreboard preflight: a real scoreboard always renders
    /// 5-6 rows of white text in the team-1 band at a known pitch (~7.4%
    /// for 6v6, ~8.3% for 5v5), so at least three dips at a plausible pitch
    /// must be present. Menus, transitions, black frames, and gameplay
    /// scenes lack this periodic structure and are rejected before any
    /// Tesseract or portrait-template work runs.
    pub fn looks_like_scoreboard(&self) -> bool {
        self.dip_count >= 3
            && self
                .median_pitch
                .is_some_and(|p| (0.05..=0.12).contains(&p))
    }

    /// 5v5 or 6v6 from the row pitch; defaults to 5 when no pitch was
    /// measurable. Threshold sits between the measured 5v5 (~8.3%) and
    /// 6v6 (~7.4%) pitches. See [`RowScan::checked_team_size`] for which
    /// pitch is used. When that returns `None` (the pitches contradict each
    /// other), this falls back to the old rule (spectral pitch, else dip
    /// pitch) so offline callers still get a size. The capture path rejects
    /// that frame instead.
    pub fn team_size(&self) -> usize {
        self.checked_team_size().unwrap_or_else(|| {
            match self.spectral_pitch.or(self.median_pitch) {
                Some(p) if p < TEAM_SIZE_PITCH_SPLIT => 6,
                _ => 5,
            }
        })
    }

    /// 5v5 or 6v6 only when the row pitch says so without contradiction.
    ///
    /// A pitch outside [`ROW_PITCH_PLAUSIBLE`] matches neither layout and
    /// is ignored. Dip spacing overshoots when a sparse row fails to make
    /// its own dip (0.101 on a 0:00 all-zero 6v6 board). The spectral peak
    /// can lock onto something that is not the row pitch: on the centred
    /// post-game table (`accepted_20261007_235921`, placeholder portraits,
    /// titles under some names) it measured 0.102 while the dips measured
    /// 0.0745. Preferring that spectral value gave 5, every row of that
    /// 6v6 table was cut one slot off, and 60 of 72 cells were wrong, 20 of
    /// them at confidence 90 or more (Scuffed Vision baseline, PR 158).
    ///
    /// - No pitch measured at all: 5, the documented default, as before.
    /// - Both pitches plausible: they must agree on the layout, or the size
    ///   is unknown.
    /// - One plausible: that one decides.
    /// - Pitches measured but none plausible: unknown.
    pub fn checked_team_size(&self) -> Option<usize> {
        if self.spectral_pitch.is_none() && self.median_pitch.is_none() {
            return Some(5);
        }
        let plausible = |p: Option<f64>| p.filter(|p| ROW_PITCH_PLAUSIBLE.contains(p));
        let layout = |p: f64| if p < TEAM_SIZE_PITCH_SPLIT { 6 } else { 5 };
        match (
            plausible(self.spectral_pitch).map(layout),
            plausible(self.median_pitch).map(layout),
        ) {
            (Some(a), Some(b)) if a == b => Some(a),
            (Some(_), Some(_)) => None,
            (Some(a), None) | (None, Some(a)) => Some(a),
            (None, None) => None,
        }
    }
}

/// Row pitch (fraction of crop height) between the two layouts: 6v6 below,
/// 5v5 at or above.
const TEAM_SIZE_PITCH_SPLIT: f64 = 0.079;

/// Row pitches that can be a real 6v6 (about 0.074) or 5v5 (about 0.083)
/// scoreboard, with about 15% of margin on each side. 0.101 and 0.102, the
/// two misleading pitches measured so far, fall outside.
const ROW_PITCH_PLAUSIBLE: std::ops::RangeInclusive<f64> = 0.062..=0.095;

/// Detect whether the scoreboard shows 5v5 or 6v6 via the team-1 row pitch.
///
/// Counting hero portraits fails when rows are empty ("WAITING FOR PLAYER") or
/// when custom team colors make the colored row bars contiguous with no dark
/// gaps. Instead we measure saturation across the name strip per scanline: every
/// row — occupied or empty — has white text that dips saturation once per row,
/// so the dip-to-dip pitch reveals the row count regardless of empty slots or
/// team colors. 6v6 rows are tighter (~7.4% of crop height) than 5v5 (~8.3%).
pub fn detect_team_size(scoreboard: &DynamicImage) -> usize {
    scan_rows(scoreboard).team_size()
}

/// Measure the team-1 row-dip structure (see [`detect_team_size`] for the
/// method). Callers that need both the preflight verdict and the team size
/// should call this once and derive both from the returned [`RowScan`].
pub fn scan_rows(scoreboard: &DynamicImage) -> RowScan {
    let rgb = scoreboard.to_rgb8();
    let (w, h) = rgb.dimensions();
    if w == 0 || h == 0 {
        return RowScan {
            dip_count: 0,
            median_pitch: None,
            spectral_pitch: None,
        };
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

    let spectral_pitch = spectral_pitch(&smooth[y_lo..=y_hi], h);

    let mut pitches: Vec<f64> = dips.windows(2).map(|p| (p[1] - p[0]) as f64).collect();
    if pitches.is_empty() {
        tracing::debug!(dip_count = dips.len(), "row scan: no row pitch measurable");
        return RowScan {
            dip_count: dips.len(),
            median_pitch: None,
            spectral_pitch,
        };
    }
    pitches.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let median_pitch = pitches[pitches.len() / 2] / h as f64;

    tracing::debug!(
        median_pitch,
        spectral_pitch,
        dip_count = dips.len(),
        "row scan via saturation dips"
    );
    RowScan {
        dip_count: dips.len(),
        median_pitch: Some(median_pitch),
        spectral_pitch,
    }
}

/// Minimum DFT peak magnitude relative to the band's standard deviation. A
/// pure sine scores ~0.35 on this scale; the sparse 0:00 all-zero board
/// measured 0.21, well-populated boards higher still.
const SPECTRAL_MIN_STRENGTH: f64 = 0.12;

/// Row pitch (fraction of crop height) from the strongest DFT component of
/// the smoothed team-1 saturation profile, scanning periods that span the
/// 6v6 (~7.4%) to 5v5 (~8.3%) row pitches with margin. The peak must be
/// interior — a maximum at the range edge means the spectrum is just decaying
/// (no row periodicity) — and strong enough relative to band variance.
fn spectral_pitch(band: &[f64], crop_h: u32) -> Option<f64> {
    let n = band.len();
    let p_lo = ((crop_h as f64 * 0.055) as usize).max(4);
    let p_hi = ((crop_h as f64 * 0.11) as usize).min(n / 2);
    if p_hi <= p_lo + 2 {
        return None;
    }

    let mean = band.iter().sum::<f64>() / n as f64;
    let centered: Vec<f64> = band.iter().map(|v| v - mean).collect();
    let variance = centered.iter().map(|v| v * v).sum::<f64>() / n as f64;
    if variance < 1e-9 {
        return None;
    }

    let mags: Vec<f64> = (p_lo..=p_hi)
        .map(|period| {
            let omega = std::f64::consts::TAU / period as f64;
            let (mut re, mut im) = (0.0f64, 0.0f64);
            for (i, v) in centered.iter().enumerate() {
                re += v * (omega * i as f64).cos();
                im += v * (omega * i as f64).sin();
            }
            (re * re + im * im).sqrt() / n as f64
        })
        .collect();

    let (best_idx, best_mag) = mags
        .iter()
        .copied()
        .enumerate()
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))?;

    let strength = best_mag / variance.sqrt();
    // Edge maximum = monotone spectrum, not a row-pitch peak.
    if best_idx == 0 || best_idx == mags.len() - 1 || strength < SPECTRAL_MIN_STRENGTH {
        tracing::debug!(
            period = p_lo + best_idx,
            strength,
            edge = best_idx == 0 || best_idx == mags.len() - 1,
            "spectral pitch rejected"
        );
        return None;
    }
    Some((p_lo + best_idx) as f64 / crop_h as f64)
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

/// Where the reference image for `hero_name` lives: the hero name lowercased,
/// spaces to underscores, dots dropped ("D.Mon" -> dmon.png, "Wrecking Ball"
/// -> wrecking_ball.png) — the same stem `PortraitMatcher::load` keys on.
pub fn portrait_reference_path(portraits_dir: &Path, hero_name: &str) -> PathBuf {
    let filename = hero_name.to_lowercase().replace(' ', "_").replace('.', "");
    portraits_dir.join(format!("{filename}.png"))
}

/// Save a portrait crop as a reference image.
/// The filename is the hero name (lowercase, no spaces).
///
/// Always overwrites. Callers that must keep a real crop or a user-supplied
/// file use [`save_captured_portrait`].
pub fn save_portrait_reference(
    portraits_dir: &Path,
    hero_name: &str,
    crop: &DynamicImage,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    std::fs::create_dir_all(portraits_dir)?;
    let path = portrait_reference_path(portraits_dir, hero_name);
    let resized = crop.resize_exact(PORTRAIT_SIZE, PORTRAIT_SIZE, FilterType::Lanczos3);
    let bytes = png_bytes(&resized)?;
    write_bytes_atomic(&path, &bytes)?;
    tracing::info!(hero = hero_name, path = %path.display(), "saved portrait reference");
    Ok(path)
}

/// Write `bytes` via a temp file in the same directory, then rename.
/// Rename on one filesystem is atomic, so a crash cannot leave a half-written
/// portrait in the slot the matcher will load.
fn write_bytes_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let file_name = path.file_name().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "portrait path has no file name",
        )
    })?;
    let tmp_path = path.with_file_name(format!(
        ".{}.{}.tmp",
        file_name.to_string_lossy(),
        std::process::id()
    ));
    if let Err(e) = std::fs::write(&tmp_path, bytes) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(e);
    }
    if let Err(e) = std::fs::rename(&tmp_path, path) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(e);
    }
    Ok(())
}

fn png_bytes(img: &DynamicImage) -> Result<Vec<u8>, image::ImageError> {
    let mut buf = std::io::Cursor::new(Vec::new());
    img.write_to(&mut buf, image::ImageFormat::Png)?;
    Ok(buf.into_inner())
}

/// Whether this capture may write a portrait reference.
///
/// A save needs an identified player row (row 0 belongs to someone else when
/// the row was not found), a slot that is empty or still a provisional
/// stand-in, and either the career panel or `--collect-portraits`.
pub fn should_save_portrait_crop(
    career_panel: bool,
    collect_portraits: bool,
    slot_replaceable: bool,
    player_row_known: bool,
) -> bool {
    player_row_known && slot_replaceable && (collect_portraits || career_panel)
}

/// Write `crop` only when the slot is empty or still holds a provisional
/// stand-in. Returns `Ok(None)` when an existing file has any other bytes.
pub fn save_captured_portrait(
    portraits_dir: &Path,
    hero_name: &str,
    crop: &DynamicImage,
) -> Result<Option<PathBuf>, Box<dyn std::error::Error>> {
    let path = portrait_reference_path(portraits_dir, hero_name);
    if !portrait_slot_is_replaceable(&path) {
        tracing::debug!(
            hero = hero_name,
            path = %path.display(),
            "portrait reference kept (bytes differ from the provisional stand-in)"
        );
        return Ok(None);
    }
    save_portrait_reference(portraits_dir, hero_name, crop).map(Some)
}

/// Get the portraits directory path within the data dir.
pub fn portraits_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("portraits")
}

/// Bundled stand-in portraits the tracker may replace.
///
/// Each entry is `(file stem, sha256 hex of a bundled PNG)`. A stem may
/// appear more than once. Never remove a hash that has shipped: an older
/// install still has those bytes on disk, and dropping the digest would make
/// the tracker treat that stand-in as a real crop and keep it forever.
/// Append the new digest instead.
///
/// An on-disk file whose bytes hash to any listed digest for its stem is
/// treated as missing, so the first real in-game crop overwrites it. Any
/// other bytes — a real crop, or a file the user placed there — are left
/// alone.
///
/// Doctrine's stand-in is Blizzard Entertainment artwork, sourced via the
/// Overwatch wiki. `doctrine_provisional_hash_matches_bundled_png` checks
/// this digest against the bytes `build.rs` embeds.
const PROVISIONAL_PORTRAITS: &[(&str, &str)] = &[(
    "doctrine",
    "6dd3449514c8b0a657c5ca07f2f89e97820ce76a608684029e36300cc7a862cb",
)];

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest;
    let digest = sha2::Sha256::digest(bytes);
    format!("{digest:x}")
}

fn digest_matches_provisional(table: &[(&str, &str)], stem: &str, digest: &str) -> bool {
    table
        .iter()
        .any(|(name, hash)| *name == stem && *hash == digest)
}

fn bytes_match_provisional(stem: &str, bytes: &[u8]) -> bool {
    digest_matches_provisional(PROVISIONAL_PORTRAITS, stem, &sha256_hex(bytes))
}

/// True when `path` is absent, or its bytes are a known provisional stand-in.
/// A file with any other bytes is not replaceable.
pub fn portrait_slot_is_replaceable(path: &Path) -> bool {
    if !path.exists() {
        return true;
    }
    let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
        return false;
    };
    match std::fs::read(path) {
        Ok(bytes) => bytes_match_provisional(stem, &bytes),
        Err(_) => false,
    }
}

/// Unpack bundled portraits into the data directory.
///
/// Missing files are written. A file whose bytes match a
/// [`PROVISIONAL_PORTRAITS`] digest is treated as missing (rewriting the same
/// stand-in is a no-op). A file with any other bytes is left alone.
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
        if !portrait_slot_is_replaceable(&path) {
            continue;
        }
        if std::fs::read(&path).ok().as_deref() == Some(data) {
            continue;
        }
        if let Err(e) = write_bytes_atomic(&path, data) {
            tracing::warn!(hero = name, error = %e, "failed to unpack bundled portrait");
        } else {
            unpacked += 1;
        }
    }

    if unpacked > 0 {
        tracing::info!(count = unpacked, "unpacked bundled portrait references");
    }
}

#[cfg(test)]
mod portrait_rect_tests {
    use super::portrait_rect;

    #[test]
    fn five_v_five_rows_are_spaced() {
        let dims = (1000u32, 1000u32);
        let r0 = portrait_rect(dims, 0, 5).expect("row 0");
        let r1 = portrait_rect(dims, 1, 5).expect("row 1");
        let r5 = portrait_rect(dims, 5, 5).expect("team2 row 0");
        assert_eq!(r0.w, r0.h);
        assert!(r1.y > r0.y);
        // Team 2 starts after team1 rows + gap (team_size+1 row heights)
        assert!(r5.y > r1.y);
        assert_eq!(portrait_rect(dims, 10, 5), None);
    }

    #[test]
    fn six_v_six_uses_tighter_row_height() {
        let dims = (1000u32, 1000u32);
        let five = portrait_rect(dims, 1, 5).unwrap().y - portrait_rect(dims, 0, 5).unwrap().y;
        let six = portrait_rect(dims, 1, 6).unwrap().y - portrait_rect(dims, 0, 6).unwrap().y;
        assert!(
            six < five,
            "6v6 rows should be tighter than 5v5 ({six} vs {five})"
        );
        // Last row of team 2 exists for 6v6
        assert!(portrait_rect(dims, 11, 6).is_some());
        assert!(portrait_rect(dims, 12, 6).is_none());
    }

    #[test]
    fn rejects_empty_dims() {
        assert!(portrait_rect((0, 1000), 0, 5).is_none());
        assert!(portrait_rect((1000, 0), 0, 5).is_none());
        assert!(portrait_rect((1000, 1000), 0, 0).is_none());
    }
}

#[cfg(test)]
mod reference_path_tests {
    use super::portrait_reference_path;
    use std::path::Path;

    /// The on-disk stem is what `PortraitMatcher::load` keys on, so a
    /// dotted/spaced hero name must map to exactly one predictable file
    /// (WL-5 review nit).
    #[test]
    fn stem_drops_dots_and_underscores_spaces() {
        let dir = Path::new("/tmp/portraits");
        assert_eq!(portrait_reference_path(dir, "D.Mon"), dir.join("dmon.png"));
        assert_eq!(portrait_reference_path(dir, "D.Va"), dir.join("dva.png"));
        assert_eq!(
            portrait_reference_path(dir, "Wrecking Ball"),
            dir.join("wrecking_ball.png")
        );
        assert_eq!(
            portrait_reference_path(dir, "Doctrine"),
            dir.join("doctrine.png")
        );
    }
}

#[cfg(test)]
mod bundled_portrait_tests {
    use super::bundled_portraits;

    /// `build.rs` bundles every `portraits/*.png` under its file stem.
    /// Doctrine's stem is `doctrine` (`portrait_reference_path("Doctrine")`).
    #[test]
    fn bundled_set_includes_doctrine() {
        let bytes = bundled_portraits()
            .iter()
            .find(|(name, _)| *name == "doctrine")
            .map(|(_, bytes)| *bytes)
            .expect("bundled portraits include doctrine");
        assert!(!bytes.is_empty());
        let img = image::load_from_memory(bytes).expect("doctrine.png decodes");
        assert_eq!((img.width(), img.height()), (32, 32));
        assert!(matches!(img, image::DynamicImage::ImageRgba8(_)));
    }

    /// The digest in [`PROVISIONAL_PORTRAITS`] is the sha256 of the PNG
    /// `build.rs` embeds, so a replaced stand-in still matches this const.
    #[test]
    fn doctrine_provisional_hash_matches_bundled_png() {
        let bytes = bundled_portraits()
            .iter()
            .find(|(name, _)| *name == "doctrine")
            .map(|(_, bytes)| *bytes)
            .expect("bundled portraits include doctrine");
        assert!(super::digest_matches_provisional(
            super::PROVISIONAL_PORTRAITS,
            "doctrine",
            &super::sha256_hex(bytes),
        ));
    }
}

#[cfg(test)]
mod provisional_portrait_tests {
    use super::{
        bundled_portraits, digest_matches_provisional, portrait_slot_is_replaceable,
        save_captured_portrait, should_save_portrait_crop, unpack_bundled,
    };
    use image::{DynamicImage, Rgba, RgbaImage};

    fn solid(color: Rgba<u8>) -> DynamicImage {
        DynamicImage::ImageRgba8(RgbaImage::from_pixel(8, 8, color))
    }

    #[test]
    fn capture_replaces_provisional_stand_in() {
        let dir = tempfile::tempdir().unwrap();
        let bundled = bundled_portraits()
            .iter()
            .find(|(name, _)| *name == "doctrine")
            .map(|(_, bytes)| *bytes)
            .unwrap();
        let path = dir.path().join("doctrine.png");
        std::fs::write(&path, bundled).unwrap();
        assert!(portrait_slot_is_replaceable(&path));

        let saved =
            save_captured_portrait(dir.path(), "Doctrine", &solid(Rgba([0, 255, 0, 255]))).unwrap();
        assert!(saved.is_some());
        let after = std::fs::read(&path).unwrap();
        assert_ne!(after, bundled);
        assert!(!portrait_slot_is_replaceable(&path));
    }

    #[test]
    fn capture_and_unpack_leave_non_provisional_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("doctrine.png");
        solid(Rgba([255, 0, 0, 255]))
            .save(&path)
            .expect("write user portrait");
        let before = std::fs::read(&path).unwrap();
        assert!(!portrait_slot_is_replaceable(&path));

        let saved =
            save_captured_portrait(dir.path(), "Doctrine", &solid(Rgba([0, 0, 255, 255]))).unwrap();
        assert!(saved.is_none());
        assert_eq!(std::fs::read(&path).unwrap(), before);

        unpack_bundled(dir.path());
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }

    #[test]
    fn unpack_writes_missing_files() {
        let dir = tempfile::tempdir().unwrap();
        unpack_bundled(dir.path());
        let bundled = bundled_portraits()
            .iter()
            .find(|(name, _)| *name == "doctrine")
            .map(|(_, bytes)| *bytes)
            .unwrap();
        let path = dir.path().join("doctrine.png");
        assert_eq!(std::fs::read(&path).unwrap(), bundled);
        assert!(portrait_slot_is_replaceable(&path));
        assert!(dir.path().read_dir().unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".tmp")
        }));
    }

    /// A stem may list several shipped digests. `.any` keeps every one of
    /// them replaceable; stopping at the first row would freeze the older file.
    #[test]
    fn a_stem_matches_every_listed_hash() {
        let table = &[("doctrine", "aaa"), ("doctrine", "bbb"), ("ana", "ccc")];
        assert!(digest_matches_provisional(table, "doctrine", "aaa"));
        assert!(digest_matches_provisional(table, "doctrine", "bbb"));
        assert!(!digest_matches_provisional(table, "doctrine", "ccc"));
        assert!(!digest_matches_provisional(table, "ana", "aaa"));
    }

    #[test]
    fn capture_decision_needs_a_known_row_and_a_replaceable_slot() {
        // Career panel, empty-or-stand-in slot, identified row.
        assert!(should_save_portrait_crop(true, false, true, true));
        // Row not identified: do not crop row 0, even with --collect-portraits.
        assert!(!should_save_portrait_crop(true, true, true, false));
        // Portrait/held/text guess, collection off.
        assert!(!should_save_portrait_crop(false, false, true, true));

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("doctrine.png");
        solid(Rgba([255, 0, 0, 255]))
            .save(&path)
            .expect("write real portrait");
        let replaceable = portrait_slot_is_replaceable(&path);
        assert!(!replaceable);
        assert!(!should_save_portrait_crop(true, true, replaceable, true));
    }
}

#[cfg(test)]
mod team_size_tests {
    use super::RowScan;

    fn scan(dips: usize, median: Option<f64>, spectral: Option<f64>) -> RowScan {
        RowScan {
            dip_count: dips,
            median_pitch: median,
            spectral_pitch: spectral,
        }
    }

    #[test]
    fn postgame_table_with_a_stray_spectral_peak_is_six() {
        // Measured on accepted_20261007_235921 (Scuffed Vision, PR 158) at
        // native and 0.75x. The old rule preferred the 0.102 spectral peak
        // and returned 5, shifting every row of the 6v6 table.
        for s in [
            scan(4, Some(0.07447864945382324), Some(0.10228401191658391)),
            scan(4, Some(0.07539682539682539), Some(0.10185185185185185)),
        ] {
            assert_eq!(s.checked_team_size(), Some(6), "{s:?}");
            assert_eq!(s.team_size(), 6, "{s:?}");
        }
    }

    #[test]
    fn sparse_board_with_overshooting_dips_is_still_six() {
        // 0:00 all-zero 6v6 board: a missing dip stretches the dip pitch to
        // 0.101 while the spectral peak keeps the real 0.0745 row pitch.
        let s = scan(3, Some(0.101), Some(0.0745));
        assert_eq!(s.checked_team_size(), Some(6));
        assert_eq!(s.team_size(), 6);
    }

    #[test]
    fn typical_boards_keep_their_size() {
        assert_eq!(
            scan(6, Some(0.074), Some(0.0742)).checked_team_size(),
            Some(6)
        );
        assert_eq!(
            scan(5, Some(0.083), Some(0.0835)).checked_team_size(),
            Some(5)
        );
        assert_eq!(scan(5, Some(0.083), None).checked_team_size(), Some(5));
        assert_eq!(scan(6, None, Some(0.074)).checked_team_size(), Some(6));
    }

    #[test]
    fn nothing_measured_keeps_the_documented_default() {
        let s = scan(0, None, None);
        assert_eq!(s.checked_team_size(), Some(5));
        assert_eq!(s.team_size(), 5);
    }

    #[test]
    fn contradicting_plausible_pitches_are_unknown() {
        // Both pitches fit a layout but not the same one: capture rejects,
        // offline callers fall back to the spectral pitch as before.
        let s = scan(5, Some(0.084), Some(0.074));
        assert_eq!(s.checked_team_size(), None);
        assert_eq!(s.team_size(), 6);
        let s = scan(5, Some(0.074), Some(0.084));
        assert_eq!(s.checked_team_size(), None);
        assert_eq!(s.team_size(), 5);
    }

    #[test]
    fn only_implausible_pitches_are_unknown() {
        let s = scan(3, Some(0.11), Some(0.05));
        assert_eq!(s.checked_team_size(), None);
        assert_eq!(s.team_size(), 6);
        assert_eq!(scan(2, Some(0.12), None).checked_team_size(), None);
    }
}
