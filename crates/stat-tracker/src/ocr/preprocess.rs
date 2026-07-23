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

/// Luma floor for the hard-threshold nameplate fallback. Cosmetic plates put
/// mid-grey gradients behind the glyphs; 200 keeps the brightest glyph pixels
/// and drops the plate. Measured on the 2026-07-16 fixtures — lowering it
/// re-admits the plate as ink and the name goes back to empty.
const NAME_GLYPH_LUMA_MIN: u8 = 200;

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
    add_cell_border(&prepare_cell_binary(img))
}

/// Add the standard 8-px white OCR border to a binarized cell. Split out so the
/// per-cell path can binarize once (for the edge-ink measurement) and border the
/// same buffer for Tesseract, rather than re-running the pipeline.
pub fn add_cell_border(binary: &GrayImage) -> GrayImage {
    add_white_border(binary, 8)
}

/// Smooth target-height upscale only when **both** height and width look like a
/// downscaled kill-col crop (CG-4 D reject fix).
///
/// Measured on cg4-20260723 fixtures with header-anchored columns
/// (`ANCHOR_NARROW_W` = 0.026):
/// - native: kill 43×55 / last-row kill 43×38 / wide 79×55
/// - 0.75×:  kill 32×42 / last-row kill 32×29 / wide 59×42
///
/// Height alone cannot separate native last-row (h=38, need nearest for
/// zero-diff vs main) from 0.75× kill (h=42, need CatmullRom for A=9).
/// Width can: native kill is 43px; 0.75× kill is 32px.
const CELL_SMOOTH_MAX_H: u32 = 48;
/// Midpoint between 0.75× kill (32) and native kill (43). Margin either side.
const CELL_SMOOTH_MAX_W: u32 = 38;

/// Target height (px) when a short+narrow cell *is* smooth-upscaled.
const CELL_UPSCALE_TARGET_H: u32 = 64;

/// Hard ceiling on the smooth-upscale factor (CG-4 D).
const CELL_UPSCALE_MAX_FACTOR: f64 = 3.0;

/// Binarized cell WITHOUT the OCR white border: foreground ink = 0 (black),
/// background = 255. This is the image [`prepare_cell`] borders for Tesseract;
/// the edge-ink suspect check ([`edge_ink_fraction`]) runs on the *borderless*
/// form because its left/right columns are the real stat-column crop boundary —
/// ink touching them means the window is clipping or bleeding a neighbour glyph
/// (CG-3 offset drift).
pub fn prepare_cell_binary(img: &DynamicImage) -> GrayImage {
    let masked = hsv_white_mask(img);
    let gray = DynamicImage::ImageRgb8(masked).to_luma8();
    let work_img = upscale_cell_for_ocr(&gray);

    let (ww, hh) = work_img.dimensions();
    let mut binary = GrayImage::new(ww, hh);
    for y in 0..hh {
        for x in 0..ww {
            let v = work_img.get_pixel(x, y).0[0];
            binary.put_pixel(x, y, Luma([if v > 30 { 0 } else { 255 }]));
        }
    }
    binary
}

/// CG-4 D cell size path (Claude reject + native zero-diff ACCEPT):
///
/// * **Smooth** only when `h < CELL_SMOOTH_MAX_H && w < CELL_SMOOTH_MAX_W`
///   (0.75× kill-col class) → CatmullRom to [`CELL_UPSCALE_TARGET_H`] ≤3×.
/// * **Else** pre-D main: `w < 150` nearest-2×, else passthrough — covers
///   native tall kills (~54×55) **and** native short last-row (~54×38).
fn upscale_cell_for_ocr(gray: &GrayImage) -> GrayImage {
    let (w, h) = gray.dimensions();
    if w == 0 || h == 0 {
        return gray.clone();
    }
    let smooth = h < CELL_SMOOTH_MAX_H && w < CELL_SMOOTH_MAX_W;
    if smooth {
        let factor = (CELL_UPSCALE_TARGET_H as f64 / h as f64).min(CELL_UPSCALE_MAX_FACTOR);
        if factor >= 1.05 {
            let nw = ((w as f64) * factor).round().max(1.0) as u32;
            let nh = ((h as f64) * factor).round().max(1.0) as u32;
            return image::imageops::resize(
                gray,
                nw,
                nh,
                image::imageops::FilterType::CatmullRom,
            );
        }
    }
    // Pre-D main path.
    if w < 150 {
        nearest_2x_upscale(gray)
    } else {
        gray.clone()
    }
}

/// Whether a cell would take the smooth target-height path (tests).
#[cfg(test)]
fn uses_smooth_upscale(w: u32, h: u32) -> bool {
    h < CELL_SMOOTH_MAX_H && w < CELL_SMOOTH_MAX_W
}

/// Per-side fill fraction of the outermost `edge_cols` pixel columns of a
/// borderless binarized cell ([`prepare_cell_binary`]), returned as
/// `(left, right)`. Each value is `ink_pixels_in_band / band_area`, so it is
/// scale-invariant (cell upscale does not shift the fill fraction). A centred
/// glyph leaves both bands near-empty; a clipped/bled glyph jams a vertical
/// stroke against one edge, spiking that side.
pub fn edge_ink_fraction(binary: &GrayImage, edge_cols: u32) -> (f64, f64) {
    let (w, h) = binary.dimensions();
    if w == 0 || h == 0 || edge_cols == 0 {
        return (0.0, 0.0);
    }
    // Clamp the band so left/right never overlap on a very narrow cell.
    let ec = edge_cols.min(w.div_ceil(2));
    let (mut left, mut right) = (0u64, 0u64);
    for y in 0..h {
        for x in 0..ec {
            if binary.get_pixel(x, y).0[0] < 128 {
                left += 1;
            }
        }
        for x in (w - ec)..w {
            if binary.get_pixel(x, y).0[0] < 128 {
                right += 1;
            }
        }
    }
    let band_area = (ec * h) as f64;
    (left as f64 / band_area, right as f64 / band_area)
}

/// Whether a borderless binarized cell has ink touching either vertical edge
/// beyond `threshold` fill — the "suspect" signal that the stat-column window is
/// clipping or bleeding a glyph. Uses the worse of the two sides (a clip touches
/// only one edge). See [`edge_ink_fraction`]; threshold validated against the
/// 2026-07-20 drift fixtures (see `capture_gate` module docs).
pub fn has_edge_ink(binary: &GrayImage, edge_cols: u32, threshold: f64) -> bool {
    let (l, r) = edge_ink_fraction(binary, edge_cols);
    l.max(r) > threshold
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

/// Widest a real header label can be, as a board-width ratio. "DMG" measures
/// 0.023 on the 2026-07 fixtures; UI junk that leaks into the header strip
/// (mic icon + highlighted-hero overlay art) merges into a ~0.37-wide pseudo
/// group, so a generous 2x margin over the widest real label separates them
/// cleanly (CG-4).
const HEADER_LABEL_MAX_W: f64 = 0.05;

/// Plausible spacing between adjacent header label centers. Measured gaps on
/// the 2026-07-20/23 fixtures (identical for 5v5 and 6v6): 0.034–0.065.
const HEADER_GAP_MIN: f64 = 0.02;
const HEADER_GAP_MAX: f64 = 0.10;

/// Anchored stat-cell window widths, as board-width ratios, centered on the
/// header label. Narrow kill columns (E/A/D) hold 1–2 digits (widest observed
/// span 0.012); wide accumulator columns hold up to 7 chars ("102,208" ≈
/// 0.045). Both leave clear margin to the smallest measured neighbor gap
/// (0.034 narrow / 0.057 wide), so a centered value can never bleed into the
/// next window — the CG-4 defect this replaces: the fallback comb's 0.070-wide
/// windows overlapped MIT's leading digit into HLG ("2,299"+"4" → 22994) while
/// MIT itself was decapitated ("4,351" → ",351" → OCR "1351").
const ANCHOR_NARROW_W: f64 = 0.026;
const ANCHOR_WIDE_W: f64 = 0.048;

/// Build per-column stat windows anchored on the six header label groups
/// (E, A, D, DMG, H, MIT), one window centered under each label.
///
/// The scoreboard renders stat values center-aligned under their header
/// labels (fixture-verified within ±0.004 board-width on both 5v5 and 6v6
/// boards, 2026-07-20 + 2026-07-23). Anchoring each column to its own label —
/// instead of sliding one fallback comb by a single global offset — makes the
/// geometry per-frame and layout-independent: 6v6 compresses the table
/// relative to 5v5 by more than a pure translation, which is exactly why the
/// global-offset sweep left the rightmost column (MIT) clipped (CG-4).
/// Because the anchors come from the frame itself, the result also holds
/// across display resolutions without recalibration.
///
/// Returns `None` when the groups do not look like the six stat labels
/// (wrong count after junk filtering, or implausible spacing) — callers fall
/// back to the offset-sweep calibration.
pub fn columns_from_header_groups(groups: &[(u32, u32)], board_w: u32) -> Option<StatColumns> {
    if board_w == 0 {
        return None;
    }
    let w = board_w as f64;
    let labels: Vec<f64> = groups
        .iter()
        .filter(|(s, e)| (e.saturating_sub(*s)) as f64 / w <= HEADER_LABEL_MAX_W)
        .map(|(s, e)| (*s + *e) as f64 / 2.0 / w)
        .collect();
    if labels.len() < STAT_COLUMNS {
        return None;
    }
    let centers = &labels[..STAT_COLUMNS];
    for pair in centers.windows(2) {
        let gap = pair[1] - pair[0];
        if !(HEADER_GAP_MIN..=HEADER_GAP_MAX).contains(&gap) {
            return None;
        }
    }

    let mut cols = [(0.0f64, 0.0f64); STAT_COLUMNS];
    for (i, &center) in centers.iter().enumerate() {
        let width = if i < 3 {
            ANCHOR_NARROW_W
        } else {
            ANCHOR_WIDE_W
        };
        let mut start = center - width / 2.0;
        let mut end = center + width / 2.0;
        // Never cross the midpoint toward a neighboring label: guarantees the
        // windows stay disjoint even if a label center is slightly off.
        if i > 0 {
            start = start.max((centers[i - 1] + center) / 2.0);
        }
        if i + 1 < STAT_COLUMNS {
            end = end.min((center + centers[i + 1]) / 2.0);
        }
        start = start.max(0.0);
        end = end.min(1.0);
        if end <= start {
            return None;
        }
        cols[i] = (start, end - start);
    }
    Some(cols)
}

/// Detect the column offset by finding stat header labels (E, A, D, DMG, H, MIT)
/// in the scoreboard header area. The header has dark text on a bright bar.
///
/// Groups adjacent dark-text clusters into logical labels, then identifies the
/// 6 stat columns by their characteristic spacing pattern (3 narrow E/A/D, then
/// 3 wider DMG/H/MIT). Returns the offset from the fallback E position.
pub fn detect_column_offset(scoreboard: &DynamicImage) -> f64 {
    offset_from_header_groups(&header_label_groups(scoreboard), scoreboard.width())
}

/// Global fallback-comb offset derived from already-detected header groups —
/// the single-offset half of header detection, kept for the sweep fallback
/// path so `calibrate_columns` can reuse one `header_label_groups` pass for
/// both the per-column anchoring and this offset seed (CG-4).
pub fn offset_from_header_groups(groups: &[(u32, u32)], board_w: u32) -> f64 {
    if groups.is_empty() || board_w == 0 {
        return 0.0;
    }

    // We expect 6 groups for E, A, D, DMG, H, MIT.
    // The first group should be the "E" label.
    let first_center = (groups[0].0 + groups[0].1) as f64 / 2.0 / board_w as f64;
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

    // Find dark-text clusters. Cluster minimum width and the letter→label
    // merge distance below are board-width-RELATIVE (anchored to their
    // long-serving 3px / 15px values at the 1664px reference board): the "H"
    // label is only ~3px at 1664 and an absolute 3px floor deleted it outright
    // on a 1080p board (2.2px), silently degrading per-column anchoring to the
    // sweep, while an absolute 15px merge distance would fragment "DMG" into
    // letters on a 4K board (CG-4 multi-resolution gate).
    // The cluster minimum is pure noise rejection — cap it at the 1664px
    // reference value of 3 so it never outgrows the thinnest real label ("H"
    // is ~3px at 1664, ~4.5px at 4K: a proportionally-scaled 5px floor would
    // delete it there, the mirror image of the 1080p failure above).
    let min_cluster_w = ((w as f64 * 3.0 / 1664.0).round() as u32).clamp(2, 3);
    let merge_dist = ((w as f64 * 15.0 / 1664.0).round() as u32).max(4);
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
            if (x as u32) - cluster_start >= min_cluster_w {
                raw_clusters.push((cluster_start, x as u32));
            }
            in_cluster = false;
        }
    }

    // Filter to stat area (ratio > 0.25) and merge nearby clusters
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
        if s <= g_end + merge_dist {
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

    // Floor-cast (same as main@10a3b3b). Rounding here caused 190+ native
    // field diffs vs pre-D (Claude ACCEPT: 20-frame native zero wrong-value
    // change). Short+narrow smooth upscale covers the 0.75× kill-col path
    // without perturbing native crop geometry.
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
/// Keeps only the brightest glyph pixels (see [`NAME_GLYPH_LUMA_MIN`]); output
/// is black-text-on-white for Tesseract, upscaled like the primary path.
pub fn prepare_name_cell_hard_threshold(img: &DynamicImage) -> GrayImage {
    let gray = img.to_luma8();
    // Smooth-upscale BEFORE thresholding: at native row height (~77px) the
    // glyphs are too thin to survive a hard binarization.
    let (w, h) = gray.dimensions();
    let up = image::imageops::resize(&gray, w * 4, h * 4, image::imageops::FilterType::CatmullRom);
    let mut bin = up;
    for p in bin.pixels_mut() {
        p.0[0] = if p.0[0] > NAME_GLYPH_LUMA_MIN { 0 } else { 255 };
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

#[cfg(test)]
mod edge_ink_tests {
    use super::*;

    // Binarized-cell convention: ink = 0 (black), background = 255.
    fn blank(w: u32, h: u32) -> GrayImage {
        GrayImage::from_pixel(w, h, Luma([255]))
    }

    fn fill(img: &mut GrayImage, x0: u32, x1: u32, y0: u32, y1: u32) {
        for y in y0..y1 {
            for x in x0..x1 {
                img.put_pixel(x, y, Luma([0]));
            }
        }
    }

    #[test]
    fn centered_glyph_has_no_edge_ink() {
        // A digit stroke in the middle columns leaves the 2-px edge bands empty.
        let mut img = blank(40, 40);
        fill(&mut img, 16, 24, 8, 32);
        assert_eq!(edge_ink_fraction(&img, 2), (0.0, 0.0));
        assert!(!has_edge_ink(&img, 2, 0.12), "centered digit must not flag");
    }

    #[test]
    fn glyph_touching_left_edge_flags() {
        // A stroke jammed against the left crop edge (a bled neighbour digit).
        let mut img = blank(40, 40);
        fill(&mut img, 0, 3, 8, 32);
        let (l, r) = edge_ink_fraction(&img, 2);
        assert!(l > 0.5, "left band mostly inked: {l}");
        assert_eq!(r, 0.0);
        assert!(has_edge_ink(&img, 2, 0.12), "left-edge stroke must flag");
    }

    #[test]
    fn glyph_touching_right_edge_flags() {
        let mut img = blank(40, 40);
        fill(&mut img, 37, 40, 8, 32);
        let (l, r) = edge_ink_fraction(&img, 2);
        assert_eq!(l, 0.0);
        assert!(r > 0.5, "right band mostly inked: {r}");
        assert!(has_edge_ink(&img, 2, 0.12));
    }

    #[test]
    fn stroke_just_inside_the_margin_does_not_flag() {
        // Ends at col 3 — the 2-px edge band (cols 0-1) stays clean. This is the
        // clean/clipped discriminator: 0.12 sits well above this margin case.
        let mut img = blank(40, 40);
        fill(&mut img, 3, 12, 8, 32);
        assert!(!has_edge_ink(&img, 2, 0.12));
    }

    #[test]
    fn threshold_sits_in_the_fixture_gap() {
        // Real 2026-07-20 frames: centered DMG edge fill = 0.000, drift spikes
        // >= 0.167. A 1-px incidental touch (~0.0125 fill) models anti-aliasing
        // and must stay BELOW 0.12; a half-height edge stroke (~0.5 fill) models
        // a real bleed and must stay ABOVE it.
        let mut light = blank(40, 40);
        fill(&mut light, 0, 1, 20, 21);
        assert!(
            !has_edge_ink(&light, 2, 0.12),
            "incidental speck must not flag"
        );
        let mut bleed = blank(40, 40);
        fill(&mut bleed, 0, 2, 10, 30);
        assert!(has_edge_ink(&bleed, 2, 0.12), "real bleed must flag");
    }
}

#[cfg(test)]
mod header_anchor_tests {
    use super::*;

    /// The six header label groups measured on the 2026-07-23 King's Row 6v6
    /// fixture (board width 1664) — identical spans on the 2026-07-20 5v5
    /// drift fixtures — plus the junk pseudo-group the mic icon + highlighted
    /// hero overlay art merge into. Centers: E 0.3206, A 0.3549, D 0.3912,
    /// DMG 0.4441, H 0.5093, MIT 0.5661.
    const FIXTURE_W: u32 = 1664;
    const FIXTURE_GROUPS: [(u32, u32); 7] = [
        (530, 537),
        (586, 595),
        (649, 653),
        (720, 758),
        (846, 849),
        (935, 949),
        (1049, 1669), // overlay junk, width 0.37 — must be filtered
    ];

    /// Real ink spans from the same fixture rows the windows must respect:
    /// MIT value "8,619" spans 0.554–0.586; a 6-char healing value centered
    /// under H spans at most ~0.493–0.526.
    const MIT_INK_START: f64 = 0.554;
    const MIT_INK_END: f64 = 0.5865;

    #[test]
    fn anchors_all_six_columns_and_filters_junk() {
        let cols = columns_from_header_groups(&FIXTURE_GROUPS, FIXTURE_W)
            .expect("six labels + junk must anchor");
        let expected_centers = [0.3206, 0.3549, 0.3912, 0.4441, 0.5093, 0.5661];
        for (i, ((start, width), expect)) in cols.iter().zip(expected_centers).enumerate() {
            let center = start + width / 2.0;
            assert!(
                (center - expect).abs() < 0.004,
                "col {i} center {center:.4} != label center {expect:.4}"
            );
        }
    }

    #[test]
    fn windows_are_ordered_and_disjoint() {
        let cols = columns_from_header_groups(&FIXTURE_GROUPS, FIXTURE_W).unwrap();
        for i in 1..cols.len() {
            let prev_end = cols[i - 1].0 + cols[i - 1].1;
            assert!(
                cols[i].0 >= prev_end,
                "window {i} starts at {:.4} before window {} ends at {prev_end:.4}",
                cols[i].0,
                i - 1
            );
        }
    }

    #[test]
    fn mit_window_covers_its_leading_digit_and_hlg_cannot_steal_it() {
        // THE CG-4 regression pin: the 2026-07-22 Numbani corruption was the
        // HLG window annexing MIT's lead digit ("2,299"+"4" → 22994) while the
        // MIT window read the remainder ("4,251" → "251" / "1351"). Anchored
        // windows must put the full MIT ink span inside MIT and none of it
        // inside HLG.
        let cols = columns_from_header_groups(&FIXTURE_GROUPS, FIXTURE_W).unwrap();
        let (hlg_start, hlg_w) = cols[4];
        let (mit_start, mit_w) = cols[5];
        assert!(
            mit_start < MIT_INK_START,
            "MIT window starts {mit_start:.4}, clips leading digit at {MIT_INK_START}"
        );
        assert!(
            mit_start + mit_w > MIT_INK_END,
            "MIT window ends {:.4}, clips trailing digit at {MIT_INK_END}",
            mit_start + mit_w
        );
        assert!(
            hlg_start + hlg_w < MIT_INK_START,
            "HLG window ends {:.4}, would annex MIT's lead digit at {MIT_INK_START}",
            hlg_start + hlg_w
        );
    }

    #[test]
    fn too_few_labels_returns_none() {
        assert!(columns_from_header_groups(&FIXTURE_GROUPS[..5], FIXTURE_W).is_none());
        // Six groups but one is junk-wide → five usable → None.
        let mut with_junk = FIXTURE_GROUPS[..5].to_vec();
        with_junk.push((1049, 1669));
        assert!(columns_from_header_groups(&with_junk, FIXTURE_W).is_none());
    }

    #[test]
    fn implausible_spacing_returns_none() {
        // Two labels nearly on top of each other (gap << HEADER_GAP_MIN).
        let squeezed = [
            (530, 537),
            (540, 547),
            (649, 653),
            (720, 758),
            (846, 849),
            (935, 949),
        ];
        assert!(columns_from_header_groups(&squeezed, FIXTURE_W).is_none());
        // A gap wider than HEADER_GAP_MAX (labels from different UI regions).
        let torn = [
            (530, 537),
            (586, 595),
            (649, 653),
            (720, 758),
            (846, 849),
            (1400, 1420),
        ];
        assert!(columns_from_header_groups(&torn, FIXTURE_W).is_none());
    }

    #[test]
    fn offset_from_groups_matches_first_label_delta() {
        let offset = offset_from_header_groups(&FIXTURE_GROUPS, FIXTURE_W);
        let e_center = (530.0 + 537.0) / 2.0 / FIXTURE_W as f64;
        let fallback_center =
            STAT_COL_BOUNDARIES_FALLBACK[0].0 + STAT_COL_BOUNDARIES_FALLBACK[0].1 / 2.0;
        assert!((offset - (e_center - fallback_center)).abs() < 1e-9);
        assert_eq!(offset_from_header_groups(&[], FIXTURE_W), 0.0);
    }
}

#[cfg(test)]
mod cell_upscale_tests {
    use super::*;

    #[test]
    fn smooth_only_for_short_and_narrow() {
        // Header-anchored dims (cg4-20260723).
        assert!(!uses_smooth_upscale(43, 55)); // native kill
        assert!(!uses_smooth_upscale(43, 38)); // native last-row kill
        assert!(!uses_smooth_upscale(79, 38)); // native last-row wide
        assert!(uses_smooth_upscale(32, 42)); // 0.75× kill
        assert!(uses_smooth_upscale(32, 29)); // 0.75× last-row kill
        assert!(!uses_smooth_upscale(59, 42)); // 0.75× wide (nearest)
    }

    #[test]
    fn smooth_short_narrow_grows_toward_target() {
        let gray = GrayImage::from_pixel(32, 42, Luma([200]));
        let up = upscale_cell_for_ocr(&gray);
        assert_eq!(up.dimensions().1, 64);
        assert_eq!(up.dimensions().0, (32.0_f64 * 64.0 / 42.0).round() as u32);
    }

    #[test]
    fn native_tall_narrow_uses_nearest_2x() {
        let gray = GrayImage::from_pixel(43, 55, Luma([200]));
        assert_eq!(upscale_cell_for_ocr(&gray).dimensions(), (86, 110));
    }

    #[test]
    fn native_short_last_row_uses_nearest_2x_not_smooth() {
        let gray = GrayImage::from_pixel(43, 38, Luma([200]));
        assert_eq!(upscale_cell_for_ocr(&gray).dimensions(), (86, 76));
    }

    #[test]
    fn wide_passthrough() {
        let gray = GrayImage::from_pixel(160, 55, Luma([200]));
        assert_eq!(upscale_cell_for_ocr(&gray).dimensions(), (160, 55));
    }

    #[test]
    fn prepare_cell_binary_runs_on_small_crop() {
        let img = DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            18,
            24,
            image::Rgb([240, 240, 240]),
        ));
        let bin = prepare_cell_binary(&img);
        assert!(bin.pixels().all(|p| p.0[0] == 0 || p.0[0] == 255));
    }
}
