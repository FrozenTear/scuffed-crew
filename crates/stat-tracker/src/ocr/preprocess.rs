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

/// Only cells shorter than this are smooth-upscaled (CG-4 D REJECT fix).
/// Native 1440p kill cells land ~53–55px — upscaling those invented phantom
/// "9"s from dim "0"/empty (277-cell native regression vs main). Short cells
/// (1080p / 0.75×, lone-digit crops) stay below this floor and still get help.
const CELL_UPSCALE_TRIGGER_H: u32 = 48;

/// Target height (px) when a short cell *is* upscaled. Kept ≥ trigger so short
/// cells grow enough for lone-8 recovery without needing the old always-on path.
const CELL_UPSCALE_TARGET_H: u32 = 64;

/// Hard ceiling on the cell upscale factor (CG-4 D). Beyond this, smooth filters
/// oversmooth thin digits into empty reads.
const CELL_UPSCALE_MAX_FACTOR: f64 = 3.0;

/// Once a crop is at least this tall, the smooth upscale does not go past
/// [`CELL_UPSCALE_FACTOR_CAP`]. A shorter crop is a tiny glyph, not a
/// trimmed stat cell, and still climbs toward [`CELL_UPSCALE_TARGET_H`].
const CELL_UPSCALE_CAP_MIN_H: u32 = 28;

/// 64/39. A native Dorado stat cell is 39px tall and already reads at this
/// factor (PR 158). Targeting 64px after a 3-4px trim raises the factor
/// to about 1.94-2.06, and Tesseract then returns "" or "1" for a "11".
/// Capping here keeps a trimmed crop on the same scale as the cell it was
/// cut from.
const CELL_UPSCALE_FACTOR_CAP: f64 = CELL_UPSCALE_TARGET_H as f64 / 39.0;

/// Two checks for a dim zero stroke. Not a calibrated grey level.
///
/// Neutral grey: R, G, and B roughly equal. Purple and yellow row fills
/// are strongly saturated, and the soft edge of a glyph picks that colour
/// up. On the Dorado cells in PR 158 the zero stroke stays at saturation
/// at most about 33, so the cap is 40 (that measurement plus a small
/// margin). Fill near saturation 240 fails this check.
///
/// Below white: white digits on those cells reach 250-255. The ceiling is
/// 220, clear of that range and above the measured stroke cores (about
/// 171-186). The floor is 96, just above team-fill brightness in the low
/// 90s, which still keeps the neutral stroke.
///
/// The HSV mask already keeps a neutral pixel at about 172 (saturation
/// under 60 and value at least 150), so a native cell often has enough
/// ink for Tesseract and this path does not run. The same stroke still
/// has to pass here when that mask leaves the cell empty.
///
/// The crop is one cell, so there is no brighter digit in the same image
/// to measure against. A threshold relative to the rest of the row would
/// need pixels this function does not receive.
const DIM_ZERO_V_MIN: u8 = 96;
const DIM_ZERO_V_MAX: u8 = 220;
const DIM_ZERO_SAT_MAX: u8 = 40;

/// A hole narrower or shorter than this fraction of the glyph box is a
/// bowl (6, 9) or a closed 4, not the counter of a 0. The Dorado zeros
/// in PR 158 have a counter 4px wide in a 10px box (40%). A realistic 6
/// is about 35% on its short axis, so 38 still rejects that bowl and
/// accepts the real counter with a little room. A "10" is not rejected
/// by this fraction when the "1" sits close to the ring: the ring's own
/// hole is still large. That pair is two ink components, and a zero is
/// one ring.
const DIM_ZERO_HOLE_EXTENT_MIN: u32 = 38;

/// The hole's centroid may sit at most this far from the glyph box centre,
/// in percent of the box on each axis.
const DIM_ZERO_CENTRE_OFFSET_MAX_PCT: u32 = 22;

/// Taken off a dim zero's confidence when its ring reaches the outer edge
/// columns. The ring may be clipped or another glyph bleeding in.
const DIM_ZERO_EDGE_PENALTY: i32 = 15;

const DIM_LABEL_INK: u8 = 1;
const DIM_LABEL_EXTERIOR: u8 = 2;
const DIM_LABEL_HOLE: u8 = 3;

/// A cell the bright mask emptied that is still a dim zero value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DimZeroHit {
    /// Dim-zero ink lies in the outer `edge_cols` of the crop. The ring is
    /// clipped or bleeding, so the capture gate must not treat the 0 as a
    /// clean read.
    ///
    /// One in-band pixel is enough. [`has_edge_ink`] wants 12% of the same
    /// band, about 13px on a 56px cell with a 2px band. A thin ring only
    /// kisses the edge, and that fill fraction would miss it.
    pub touches_edge: bool,
    /// The hole's share of the glyph box on its tighter axis, in percent.
    /// The floor is [`DIM_ZERO_HOLE_EXTENT_MIN`].
    pub hole_extent_pct: u32,
    /// How far the hole's centroid sits from the glyph box centre on its
    /// worse axis, in percent of the box on that axis. The ceiling is
    /// [`DIM_ZERO_CENTRE_OFFSET_MAX_PCT`].
    pub centre_offset_pct: u32,
    /// 0..=100. See [`dim_zero_confidence`].
    pub confidence: i32,
}

/// Hole extent at which the extent term is full. A realistic 6 or 9 bowl is
/// about 35% on its short axis and the floor is 38%, so 44% is clearly a
/// counter and not a bowl. The real zeros in the Scuffed Vision baseline
/// measure 40-55% at 1440p and 42-60% at 0.75x.
const DIM_ZERO_EXTENT_FULL_PCT: u32 = 44;

/// Confidence for a geometric dim zero, on the same 0..=100 scale as a
/// Tesseract word confidence. It says how far the ring cleared the two
/// shape checks that tell a 0 from a 6, 9, or closed 4:
///
/// - 55 is the floor: the ring only just passed both checks (hole extent
///   at the 38% floor, centroid offset at the 22% ceiling).
/// - up to +25 for hole extent, linear from 38% to 44% and full above.
/// - up to +15 for centring, linear from a 22% offset to a perfectly
///   centred hole.
///
/// So a ring at the floor scores 55, a counter of 41% or more with the
/// hole within 5% of centre scores at least 78, and the ceiling is 95.
/// Every dim zero in that baseline scores 75 or more, above a "conf below
/// 70 is suspect" rule. A ring in the outer edge columns is also marked
/// `suspect`, so it loses 15.
pub fn dim_zero_confidence(hole_extent_pct: u32, centre_offset_pct: u32) -> i32 {
    let floor = DIM_ZERO_HOLE_EXTENT_MIN as f32;
    let full = DIM_ZERO_EXTENT_FULL_PCT as f32;
    let extent = ((hole_extent_pct as f32 - floor) / (full - floor)).clamp(0.0, 1.0);
    let max_off = DIM_ZERO_CENTRE_OFFSET_MAX_PCT as f32;
    let centred = (1.0 - centre_offset_pct as f32 / max_off).clamp(0.0, 1.0);
    (55.0 + 25.0 * extent + 15.0 * centred).round() as i32
}

/// Eight-connected ink blobs. Returns early once a second blob is found.
fn ink_component_count(label: &[u8], w: u32, h: u32) -> u32 {
    let mut seen = vec![false; label.len()];
    let mut count = 0u32;
    let mut stack = Vec::new();
    for y in 0..h {
        for x in 0..w {
            let start = (y * w + x) as usize;
            if label[start] != DIM_LABEL_INK || seen[start] {
                continue;
            }
            count += 1;
            if count > 1 {
                return count;
            }
            stack.push((x, y));
            seen[start] = true;
            while let Some((cx, cy)) = stack.pop() {
                for dy in -1i32..=1 {
                    for dx in -1i32..=1 {
                        if dx == 0 && dy == 0 {
                            continue;
                        }
                        let nx = cx as i32 + dx;
                        let ny = cy as i32 + dy;
                        if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 {
                            continue;
                        }
                        let i = (ny as u32 * w + nx as u32) as usize;
                        if label[i] == DIM_LABEL_INK && !seen[i] {
                            seen[i] = true;
                            stack.push((nx as u32, ny as u32));
                        }
                    }
                }
            }
        }
    }
    count
}

/// Whether `img` is a single dim `0`: one gray ring the primary cell mask
/// erases, with one hole that fills the middle of the glyph.
///
/// Called only after that mask found almost no ink. A `6` or `9` also has
/// one hole, but the hole is a bowl: it covers well under half the glyph
/// on one axis. A closed `4` is the same. An `8` has two holes. A `10` is
/// two ink blobs whenever the `1` does not touch the ring, including a
/// gap of one to three pixels, so the hole-size rule is not what rejects
/// it. Those stay unread so a failed cell is still rejected instead of
/// being invented as zero. `edge_cols` is the same vertical band the
/// bright-ink suspect check uses. The bright check wants a fill fraction
/// of that band. This one flags any pixel in it, because a ring against
/// the crop is only a few pixels of ink.
pub fn dim_zero_glyph(img: &DynamicImage, edge_cols: u32) -> Option<DimZeroHit> {
    let owned;
    let rgb = if let Some(rgb) = img.as_rgb8() {
        rgb
    } else {
        owned = img.to_rgb8();
        &owned
    };
    let (w, h) = rgb.dimensions();
    if w < 12 || h < 16 || w > 400 || h > 400 {
        return None;
    }
    let n = (w * h) as usize;
    // 0 empty, 1 ink, 2 exterior, 3 hole. One buffer for the whole pass.
    let mut label = vec![0u8; n];
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (w, h, 0u32, 0u32);
    let mut ink_count = 0u32;
    let mut touches_edge = false;
    let edge = edge_cols.min(w);
    for y in 0..h {
        for x in 0..w {
            let [r, g, b] = rgb.get_pixel(x, y).0;
            let v = r.max(g).max(b);
            let min_c = r.min(g).min(b);
            let sat = if v == 0 {
                0
            } else {
                ((u16::from(v - min_c) * 255) / u16::from(v)) as u8
            };
            // Neutral grey, then clearly darker than white text.
            let neutral = sat <= DIM_ZERO_SAT_MAX;
            let below_white = (DIM_ZERO_V_MIN..=DIM_ZERO_V_MAX).contains(&v);
            if !neutral || !below_white {
                continue;
            }
            label[(y * w + x) as usize] = DIM_LABEL_INK;
            ink_count += 1;
            // One pixel, not the 12% fill `has_edge_ink` uses. A thin ring
            // in this band is a few pixels, and 12% of a 2-column band on
            // a 56px cell is about 13px.
            if x < edge || x + edge >= w {
                touches_edge = true;
            }
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
        }
    }
    // A zero stroke is a thin ring, not a speck and not a filled blob.
    if ink_count < 24 {
        return None;
    }
    let bw = max_x - min_x + 1;
    let bh = max_y - min_y + 1;
    if bw < 6 || bh < 10 {
        return None;
    }
    let aspect = bw as f32 / bh as f32;
    if !(0.35..=1.05).contains(&aspect) {
        return None;
    }
    let bbox_area = bw * bh;
    let fill = ink_count * 100 / bbox_area;
    if !(15..=75).contains(&fill) {
        return None;
    }
    // A zero is one ring. A "10" with a gap, even one pixel, is two blobs.
    // Eight-connected, so a thin ring that only meets on a diagonal stays
    // one component. An empty column between a stroke and a ring does not.
    if ink_component_count(&label, w, h) != 1 {
        return None;
    }

    // Non-ink pixels connected to the crop border are the background. What
    // remains inside the ring is the hole.
    let mut stack = Vec::new();
    {
        let mut seed = |x: u32, y: u32| {
            let i = (y * w + x) as usize;
            if label[i] == 0 {
                label[i] = DIM_LABEL_EXTERIOR;
                stack.push((x, y));
            }
        };
        for x in 0..w {
            seed(x, 0);
            seed(x, h - 1);
        }
        for y in 0..h {
            seed(0, y);
            seed(w - 1, y);
        }
    }
    while let Some((x, y)) = stack.pop() {
        let mut step = |nx: u32, ny: u32| {
            let i = (ny * w + nx) as usize;
            if label[i] == 0 {
                label[i] = DIM_LABEL_EXTERIOR;
                stack.push((nx, ny));
            }
        };
        if x > 0 {
            step(x - 1, y);
        }
        if x + 1 < w {
            step(x + 1, y);
        }
        if y > 0 {
            step(x, y - 1);
        }
        if y + 1 < h {
            step(x, y + 1);
        }
    }

    let mut hole_count = 0u32;
    let mut components = 0u32;
    let mut sum_x = 0u64;
    let mut sum_y = 0u64;
    let (mut hole_min_x, mut hole_min_y, mut hole_max_x, mut hole_max_y) = (w, h, 0u32, 0u32);
    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let start = (y * w + x) as usize;
            if label[start] != 0 {
                continue;
            }
            components += 1;
            stack.clear();
            stack.push((x, y));
            label[start] = DIM_LABEL_HOLE;
            while let Some((cx, cy)) = stack.pop() {
                hole_count += 1;
                sum_x += u64::from(cx);
                sum_y += u64::from(cy);
                hole_min_x = hole_min_x.min(cx);
                hole_min_y = hole_min_y.min(cy);
                hole_max_x = hole_max_x.max(cx);
                hole_max_y = hole_max_y.max(cy);
                let mut visit = |nx: u32, ny: u32| {
                    let i = (ny * w + nx) as usize;
                    if label[i] == 0 {
                        label[i] = DIM_LABEL_HOLE;
                        stack.push((nx, ny));
                    }
                };
                if cx > min_x {
                    visit(cx - 1, cy);
                }
                if cx < max_x {
                    visit(cx + 1, cy);
                }
                if cy > min_y {
                    visit(cx, cy - 1);
                }
                if cy < max_y {
                    visit(cx, cy + 1);
                }
            }
        }
    }
    // One enclosed hole, large enough to be the counter of a zero and not
    // most of the glyph. Two counters (an 8) fail here even when the pair
    // of holes, taken together, is centered and tall.
    if components != 1 || hole_count < 8 || hole_count * 100 / bbox_area > 55 {
        return None;
    }
    let hole_w = hole_max_x - hole_min_x + 1;
    let hole_h = hole_max_y - hole_min_y + 1;
    // The counter of a zero fills the ring. A 6 or 9 bowl and a closed 4
    // are smaller than this on one axis. A "10" is rejected earlier, as
    // two ink components, once the stroke does not touch the ring.
    if hole_w * 100 < bw * DIM_ZERO_HOLE_EXTENT_MIN || hole_h * 100 < bh * DIM_ZERO_HOLE_EXTENT_MIN
    {
        return None;
    }
    let hx = sum_x as f32 / hole_count as f32;
    let hy = sum_y as f32 / hole_count as f32;
    let cx = (min_x + max_x) as f32 / 2.0;
    let cy = (min_y + max_y) as f32 / 2.0;
    let off_x = (hx - cx).abs() * 100.0 / bw as f32;
    let off_y = (hy - cy).abs() * 100.0 / bh as f32;
    let centre_offset_pct = off_x.max(off_y).round() as u32;
    if off_x > DIM_ZERO_CENTRE_OFFSET_MAX_PCT as f32
        || off_y > DIM_ZERO_CENTRE_OFFSET_MAX_PCT as f32
    {
        return None;
    }
    let hole_extent_pct = (hole_w * 100 / bw).min(hole_h * 100 / bh);
    Some(DimZeroHit {
        touches_edge,
        hole_extent_pct,
        centre_offset_pct,
        confidence: dim_zero_confidence(hole_extent_pct, centre_offset_pct)
            - if touches_edge {
                DIM_ZERO_EDGE_PENALTY
            } else {
                0
            },
    })
}

/// Binarized cell WITHOUT the OCR white border: foreground ink = 0 (black),
/// background = 255. This is the image [`prepare_cell`] borders for Tesseract;
/// the edge-ink suspect check ([`edge_ink_fraction`]) runs on the *borderless*
/// form because its left/right columns are the real stat-column crop boundary —
/// ink touching them means the window is clipping or bleeding a neighbour glyph
/// (CG-3 offset drift).
pub fn prepare_cell_binary(img: &DynamicImage) -> GrayImage {
    prepare_cell_binary_at(img, cell_upscale_factor(img.height()))
}

/// [`prepare_cell_binary`] at the factor the cell got before PR 158 capped
/// it, or `None` when the cap does not change this cell's factor.
///
/// The cap only binds on crops 28 to 38 px tall. On a 1440p 6v6 board that
/// is the bottom row alone: [`crop_player_row`] cuts it off at the edge of
/// the scoreboard crop (54 of 77 px), so its cells are 38 px tall and get
/// 64/39 = 1.641 instead of 64/38 = 1.684. Tesseract's read of a small
/// glyph flips with tiny factor changes. On `accepted_20261008_000324` a
/// white 8 reads at 1.60, 1.62, 1.684, 1.70 and 1.80 but is empty at 1.641,
/// 1.66 and 2.0. On `accepted_20261008_001449` a white 183 reads at 1.0,
/// 1.5, 1.684 and 2.0 but is empty at 1.60 to 1.66 (Scuffed Vision
/// baseline, PR 158). The per-cell read retries here when the capped read
/// is empty, so the cap never loses a cell the old factor read.
pub fn prepare_cell_binary_uncapped(img: &DynamicImage) -> Option<GrayImage> {
    let h = img.height();
    let capped = cell_upscale_factor(h);
    let uncapped = uncapped_cell_upscale_factor(h);
    ((uncapped - capped).abs() > 1e-9).then(|| prepare_cell_binary_at(img, uncapped))
}

fn prepare_cell_binary_at(img: &DynamicImage, factor: f64) -> GrayImage {
    let masked = hsv_white_mask(img);
    let gray = DynamicImage::ImageRgb8(masked).to_luma8();
    let work_img = upscale_cell_at(&gray, factor);

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

/// Test helper: [`upscale_cell_at`] at the normal [`cell_upscale_factor`].
#[cfg(test)]
fn upscale_cell_for_ocr(gray: &GrayImage) -> GrayImage {
    upscale_cell_at(gray, cell_upscale_factor(gray.height()))
}

/// CG-4 D: upscale **only** cells with height &lt; [`CELL_UPSCALE_TRIGGER_H`]
/// toward [`CELL_UPSCALE_TARGET_H`] (factor capped at [`CELL_UPSCALE_MAX_FACTOR`],
/// and at [`CELL_UPSCALE_FACTOR_CAP`] once the crop is at least
/// [`CELL_UPSCALE_CAP_MIN_H`]). Native-height cells pass through unchanged so
/// dim low-contrast glyphs are not smooth-warped into phantom digits.
/// CatmullRom (not Lanczos). Lanczos on dim "0" glyphs produced conf-96 "9"
/// phantoms at any factor ≥1.05 (Claude reject). `factor` is normally
/// [`cell_upscale_factor`]; [`prepare_cell_binary_uncapped`] passes the
/// uncapped one.
fn upscale_cell_at(gray: &GrayImage, factor: f64) -> GrayImage {
    let (w, h) = gray.dimensions();
    if w == 0 || h == 0 || h >= CELL_UPSCALE_TRIGGER_H {
        return gray.clone();
    }
    if factor < 1.05 {
        return gray.clone();
    }
    let nw = ((w as f64) * factor).round().max(1.0) as u32;
    let nh = ((h as f64) * factor).round().max(1.0) as u32;
    image::imageops::resize(gray, nw, nh, image::imageops::FilterType::CatmullRom)
}

/// Height to scale factor. 1.0 at or above the trigger (no upscale).
fn cell_upscale_factor(h: u32) -> f64 {
    let toward = uncapped_cell_upscale_factor(h);
    if h >= CELL_UPSCALE_CAP_MIN_H {
        toward.min(CELL_UPSCALE_FACTOR_CAP)
    } else {
        toward
    }
}

/// [`cell_upscale_factor`] without the [`CELL_UPSCALE_FACTOR_CAP`] cap.
fn uncapped_cell_upscale_factor(h: u32) -> f64 {
    if h == 0 || h >= CELL_UPSCALE_TRIGGER_H {
        1.0
    } else {
        (CELL_UPSCALE_TARGET_H as f64 / h as f64).min(CELL_UPSCALE_MAX_FACTOR)
    }
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
/// beyond `threshold` fill. The "suspect" signal that the stat-column window is
/// clipping or bleeding a glyph. Uses the worse of the two sides (a clip touches
/// only one edge). See [`edge_ink_fraction`]; threshold validated against the
/// 2026-07-20 drift fixtures (see `capture_gate` module docs).
///
/// A dim zero does not use this fraction. Any one in-band pixel sets
/// `touches_edge`, because a thin ring in a 2-column band is a few pixels
/// and 12% of that band on a 56px cell is about 13px.
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
/// pipeline erases it. Grayscale is **max(R,G,B)** rather than Rec.601 luma:
/// magenta PMA titles sit at ~luma 90 with a high-Otsu pulled up by white
/// MATCH TIME (`rejected_preflight_20260824_001227`), while max-channel keeps
/// magenta, yellow, and white all as the bright cluster. Then Otsu + invert
/// so text is black on white for Tesseract.
pub fn prepare_title(img: &DynamicImage) -> GrayImage {
    let rgb = img.to_rgb8();
    let (w, h) = rgb.dimensions();
    let mut gray = GrayImage::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let [r, g, b] = rgb.get_pixel(x, y).0;
            gray.put_pixel(x, y, Luma([r.max(g).max(b)]));
        }
    }
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

/// [`prepare_title`] for a crop that may run past the title into smaller
/// neighbouring text (the accolade screen prints the map name and match time
/// right of VICTORY/DEFEAT). Tesseract PSM 7 returns nothing at all when one
/// "line" mixes the tall title glyphs with that two-line block, so after
/// binarizing, cut the image at the end of the tall-glyph run: a column is
/// "tall" when its ink spans at least half the crop height (title stems do,
/// the map/time face does not). Everything right of the last tall column
/// (plus a small pad) is dropped. A crop with no tall column is returned
/// untrimmed. Measured on the 2026-05-30 and 2026-07-15 accolade fixtures:
/// untrimmed 25%-wide crops OCR to "" / "DEFEATJT", trimmed to "VICTORY" /
/// "DEFEAT" (fleet::tracker-wl C6).
pub fn prepare_title_trimmed(img: &DynamicImage) -> GrayImage {
    let bordered = prepare_title(img);
    // prepare_title pads 12px of white; work on the inner image.
    let (bw, bh) = bordered.dimensions();
    if bw <= 24 || bh <= 24 {
        return bordered;
    }
    let inner = image::imageops::crop_imm(&bordered, 12, 12, bw - 24, bh - 24).to_image();
    let trimmed = trim_to_tall_glyphs(&inner);
    add_white_border(&trimmed, 12)
}

/// Cut a black-on-white binary at the end of its tall-glyph run — see
/// [`prepare_title_trimmed`]. Public for the synthetic unit test only.
pub fn trim_to_tall_glyphs(binary: &GrayImage) -> GrayImage {
    let (w, h) = binary.dimensions();
    if w == 0 || h == 0 {
        return binary.clone();
    }
    let min_span = h / 2;
    let mut last_tall: Option<u32> = None;
    for x in 0..w {
        let mut top = None;
        let mut bottom = 0u32;
        for y in 0..h {
            if binary.get_pixel(x, y).0[0] < 128 {
                if top.is_none() {
                    top = Some(y);
                }
                bottom = y;
            }
        }
        if let Some(t) = top
            && bottom - t + 1 >= min_span
        {
            last_tall = Some(x);
        }
    }
    match last_tall {
        // Pad by a glyph-ish width so a trailing thin stem is not clipped.
        Some(x) => {
            let end = (x + h / 4 + 1).min(w);
            image::imageops::crop_imm(binary, 0, 0, end, h).to_image()
        }
        None => binary.clone(),
    }
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

    // CG-4 D MED-2: round (not floor-via-as-u32) so 0.75× / scaled boards don't
    // systematically truncate ~1px off the right of kill-col windows — that
    // jitter produced unflagged wrong reads (A 9→"5") worse than empty.
    let x = (w as f64 * col_x_ratio).max(0.0).round() as u32;
    let cell_w = (w as f64 * col_w_ratio).round().max(1.0) as u32;

    let pad_y = (h as f64 * 0.15).round() as u32;
    let cell_h = h.saturating_sub(pad_y.saturating_mul(2));

    if x >= w || pad_y >= h || cell_w == 0 || cell_h == 0 {
        return None;
    }
    let cell_w = cell_w.min(w - x);
    let cell_h = cell_h.min(h - pad_y);

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
    fn factor_is_one_at_or_above_trigger() {
        // Native ~53–55px cells must never upscale (phantom-9 reject).
        assert!((cell_upscale_factor(CELL_UPSCALE_TRIGGER_H) - 1.0).abs() < 1e-9);
        assert!((cell_upscale_factor(53) - 1.0).abs() < 1e-9);
        assert!((cell_upscale_factor(55) - 1.0).abs() < 1e-9);
        assert!((cell_upscale_factor(80) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn factor_targets_64_only_below_trigger_and_caps_at_3x() {
        // 42px (<48) → 64/42 ≈ 1.524, under the Dorado cap.
        let f42 = cell_upscale_factor(42);
        assert!((f42 - (64.0 / 42.0)).abs() < 1e-9);
        assert!(f42 < CELL_UPSCALE_MAX_FACTOR);

        // 39px is the native Dorado cell. A 31px trim of that cell must
        // not scale harder, or a "11" comes back as "1".
        let native = cell_upscale_factor(39);
        assert!((native - CELL_UPSCALE_FACTOR_CAP).abs() < 1e-9);
        assert!(cell_upscale_factor(31) <= native + 1e-9);
        assert!(cell_upscale_factor(33) <= native + 1e-9);

        // Very short cell would want >3× → capped. The Dorado cap does
        // not apply under 28px.
        assert!((cell_upscale_factor(15) - 3.0).abs() < 1e-9);
        assert!((cell_upscale_factor(10) - 3.0).abs() < 1e-9);
    }

    #[test]
    fn upscale_grows_short_cell_to_about_target_height() {
        // 22px is under the Dorado cap's minimum height, so it still
        // climbs to the 64px target (64/22, under the 3× ceiling).
        let gray = GrayImage::from_pixel(20, 22, Luma([200]));
        let up = upscale_cell_for_ocr(&gray);
        let (w, h) = up.dimensions();
        assert_eq!(h, 64, "22 × 64/22 → 64");
        assert_eq!(w, 58, "width scales with height");
    }

    #[test]
    fn upscale_leaves_native_height_cells_unchanged() {
        // ≥ TRIGGER (48), including pre-D native ~53–55px, pass through.
        for h in [48u32, 53, 55, 60, 70] {
            let gray = GrayImage::from_pixel(40, h, Luma([200]));
            let up = upscale_cell_for_ocr(&gray);
            assert_eq!(up.dimensions(), (40, h), "h={h} must not upscale");
        }
    }

    #[test]
    fn prepare_cell_binary_runs_on_small_crop() {
        // Smoke: tiny RGB crop goes through HSV + target-height upscale + threshold.
        let img = DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            18,
            24,
            image::Rgb([240, 240, 240]),
        ));
        let bin = prepare_cell_binary(&img);
        let (w, h) = bin.dimensions();
        assert!(h >= 24, "upscaled height {h}");
        assert!(w >= 18);
        // Output is binary (only 0 or 255).
        assert!(bin.pixels().all(|p| p.0[0] == 0 || p.0[0] == 255));
    }
}

#[cfg(test)]
mod tall_glyph_trim_tests {
    use super::trim_to_tall_glyphs;
    use image::{GrayImage, Luma};

    /// White canvas with a "title" block of full-height ink on the left and a
    /// short "map name" block of half-height ink further right.
    fn title_then_small_text() -> GrayImage {
        let mut img = GrayImage::from_pixel(400, 80, Luma([255]));
        for y in 4..76 {
            for x in 10..150 {
                img.put_pixel(x, y, Luma([0]));
            }
        }
        for y in 30..50 {
            for x in 200..390 {
                img.put_pixel(x, y, Luma([0]));
            }
        }
        img
    }

    #[test]
    fn cuts_after_the_tall_run_and_keeps_a_pad() {
        let trimmed = trim_to_tall_glyphs(&title_then_small_text());
        // Last tall column is 149; pad = h/4 = 20 → width 170.
        assert_eq!(trimmed.width(), 170);
        assert_eq!(trimmed.height(), 80);
        // Small text is gone.
        assert!((0..trimmed.width()).all(|x| trimmed.get_pixel(x, 40).0[0] == 255 || x < 150));
    }

    #[test]
    fn no_tall_column_leaves_the_image_alone() {
        let mut img = GrayImage::from_pixel(100, 80, Luma([255]));
        for y in 35..45 {
            for x in 0..100 {
                img.put_pixel(x, y, Luma([0]));
            }
        }
        assert_eq!(trim_to_tall_glyphs(&img).width(), 100);
        assert_eq!(trim_to_tall_glyphs(&GrayImage::new(0, 0)).width(), 0);
    }
}

#[cfg(test)]
mod prepare_title_chroma_tests {
    use super::{prepare_title, prepare_title_trimmed};
    use image::{DynamicImage, GrayImage, Rgb, RgbImage};

    /// PMA-like crop: tall magenta title (low Rec.601 luma) plus short white
    /// map/time text. White pixels pull Otsu above magenta-luma, which is how
    /// `rejected_preflight_20260824_001227` lost VICTORY and OCR'd MATCH TIME.
    fn magenta_title_white_map() -> DynamicImage {
        let mut img = RgbImage::from_pixel(200, 40, Rgb([12, 10, 18]));
        for y in 2..38 {
            for x in 4..72 {
                img.put_pixel(x, y, Rgb([200, 40, 210]));
            }
        }
        for y in 14..26 {
            for x in 100..190 {
                img.put_pixel(x, y, Rgb([245, 245, 245]));
            }
        }
        DynamicImage::ImageRgb8(img)
    }

    fn yellow_title_white_map() -> DynamicImage {
        let mut img = RgbImage::from_pixel(200, 40, Rgb([12, 10, 18]));
        for y in 2..38 {
            for x in 4..72 {
                img.put_pixel(x, y, Rgb([230, 180, 20]));
            }
        }
        for y in 14..26 {
            for x in 100..190 {
                img.put_pixel(x, y, Rgb([245, 245, 245]));
            }
        }
        DynamicImage::ImageRgb8(img)
    }

    fn tall_ink_columns(bin: &GrayImage) -> u32 {
        let (w, h) = bin.dimensions();
        if w == 0 || h == 0 {
            return 0;
        }
        let min_span = h / 2;
        let mut n = 0u32;
        for x in 0..w {
            let mut top = None;
            let mut bottom = 0u32;
            for y in 0..h {
                if bin.get_pixel(x, y).0[0] < 128 {
                    if top.is_none() {
                        top = Some(y);
                    }
                    bottom = y;
                }
            }
            if let Some(t) = top
                && bottom - t + 1 >= min_span
            {
                n += 1;
            }
        }
        n
    }

    #[test]
    fn magenta_title_is_ink_despite_white_map_text() {
        let bin = prepare_title(&magenta_title_white_map());
        let n = tall_ink_columns(&bin);
        assert!(
            n >= 30,
            "magenta title dropped by luma Otsu: {n} tall ink columns (need max-channel gray)"
        );
    }

    #[test]
    fn yellow_title_is_still_ink() {
        let bin = prepare_title(&yellow_title_white_map());
        let n = tall_ink_columns(&bin);
        assert!(n >= 30, "yellow DEFEAT title lost ink: {n} tall columns");
    }

    #[test]
    fn magenta_title_trim_drops_the_map_block() {
        let trimmed = prepare_title_trimmed(&magenta_title_white_map());
        // Scale is 120/40=3, title ends at x=72 → 216 plus pad; map starts at 100→300.
        // After 12px border the inner width should stay left of the map block.
        assert!(
            trimmed.width() < 300,
            "trim kept the white map block: width={}",
            trimmed.width()
        );
        assert!(
            tall_ink_columns(&trimmed) >= 30,
            "trimmed image has no magenta title ink"
        );
    }
}

/// Synthetic dim glyphs. Shape tests live in [`dim_zero_tests`].
#[cfg(test)]
pub(crate) mod dim_zero_fixtures {
    use super::*;

    const BG: Rgb<u8> = Rgb([72, 28, 112]);
    /// Gray just under the HSV soft-weight binary cut (value ~122).
    pub(crate) const DIM: Rgb<u8> = Rgb([122, 122, 122]);

    pub(crate) fn cell(w: u32, h: u32) -> RgbImage {
        RgbImage::from_pixel(w, h, BG)
    }

    pub(crate) fn paint_ring(
        img: &mut RgbImage,
        cx: f32,
        cy: f32,
        rx: f32,
        ry: f32,
        thickness: f32,
        color: Rgb<u8>,
    ) {
        let (w, h) = img.dimensions();
        let norm = thickness / rx.min(ry);
        for y in 0..h {
            for x in 0..w {
                let dx = (x as f32 + 0.5 - cx) / rx;
                let dy = (y as f32 + 0.5 - cy) / ry;
                let d = dx.mul_add(dx, dy * dy).sqrt();
                if d <= 1.0 && d >= 1.0 - norm {
                    img.put_pixel(x, y, color);
                }
            }
        }
    }

    pub(crate) fn wrap(img: RgbImage) -> DynamicImage {
        DynamicImage::ImageRgb8(img)
    }

    /// Native-height kill cell whose only glyph is a dim zero. The primary
    /// mask drops it (see the OCR test); this is the shape `dim_zero_glyph`
    /// has to accept.
    pub(crate) fn dim_zero_cell() -> DynamicImage {
        let mut img = cell(48, 56);
        paint_ring(&mut img, 24.0, 28.0, 8.0, 14.0, 3.0, DIM);
        wrap(img)
    }

    /// Same ring, shifted so its left stroke sits in the outer two columns.
    pub(crate) fn dim_zero_touching_left_edge() -> DynamicImage {
        let mut img = cell(48, 56);
        paint_ring(&mut img, 8.0, 28.0, 8.0, 14.0, 3.0, DIM);
        wrap(img)
    }

    pub(crate) fn dim_stroke_cell() -> DynamicImage {
        let mut img = cell(48, 56);
        for y in 12..44 {
            for x in 22..27 {
                img.put_pixel(x, y, DIM);
            }
        }
        wrap(img)
    }

    /// A dim 6 whose bowl is only the lower ~44% of the glyph. The hole
    /// sits too low to be a zero even before the extent check.
    pub(crate) fn dim_six_cell() -> DynamicImage {
        let mut img = cell(48, 56);
        paint_ring(&mut img, 24.0, 40.0, 8.0, 9.0, 2.5, DIM);
        for y in 8..40 {
            for x in 16..20 {
                img.put_pixel(x, y, DIM);
            }
        }
        wrap(img)
    }

    /// A 6 whose bowl is about 57% of the glyph height, so the hole's
    /// center is close enough to the box center to fool a centroid test.
    /// The hole itself is still a bowl, not the counter of a zero.
    pub(crate) fn dim_six_realistic() -> DynamicImage {
        let mut img = cell(48, 56);
        // Glyph roughly y=8..48 (height 40). Bowl is the lower 23px.
        paint_ring(&mut img, 24.0, 36.0, 8.0, 11.0, 3.2, DIM);
        for y in 8..36 {
            for x in 16..20 {
                img.put_pixel(x, y, DIM);
            }
        }
        wrap(img)
    }

    /// Mirror of [`dim_six_realistic`]: bowl high, stem down the right.
    pub(crate) fn dim_nine_realistic() -> DynamicImage {
        let mut img = cell(48, 56);
        paint_ring(&mut img, 24.0, 20.0, 8.0, 11.0, 3.2, DIM);
        for y in 20..48 {
            for x in 28..32 {
                img.put_pixel(x, y, DIM);
            }
        }
        wrap(img)
    }

    /// A closed 4: stem, crossbar, and a left stroke that seals a small
    /// counter. The counter is not half the glyph.
    pub(crate) fn dim_four_closed() -> DynamicImage {
        let mut img = cell(48, 56);
        // Stem runs the full glyph. The closed counter sits on the center
        // so a centroid test accepts it, and it is much shorter than the stem.
        for y in 8..48 {
            for x in 30..34 {
                img.put_pixel(x, y, DIM);
            }
        }
        for y in 18..22 {
            for x in 16..34 {
                img.put_pixel(x, y, DIM);
            }
        }
        for y in 32..36 {
            for x in 16..34 {
                img.put_pixel(x, y, DIM);
            }
        }
        for y in 18..36 {
            for x in 16..20 {
                img.put_pixel(x, y, DIM);
            }
        }
        wrap(img)
    }

    /// A dim "10": a 4px stroke, then a 2px gap, then the zero fixture's ring.
    /// The hole in the ring is large enough to pass the extent rule. The
    /// stroke is a second ink component, which is what rejects it.
    pub(crate) fn dim_ten() -> DynamicImage {
        dim_ten_gap(4, 2)
    }

    /// A `stroke_w` pixel "1" sitting `gap` empty columns left of the zero
    /// fixture's ring (outer ink at x = 16).
    pub(crate) fn dim_ten_gap(stroke_w: u32, gap: u32) -> DynamicImage {
        let mut img = cell(48, 56);
        let ring_left = 16u32;
        let stroke_right = ring_left - gap;
        let stroke_left = stroke_right - stroke_w;
        for y in 14..42 {
            for x in stroke_left..stroke_right {
                img.put_pixel(x, y, DIM);
            }
        }
        paint_ring(&mut img, 24.0, 28.0, 8.0, 14.0, 3.0, DIM);
        wrap(img)
    }

    /// Same ring as [`dim_zero_cell`] with a 4px stroke. The hole still
    /// covers about half the box, so a 60% extent rule would drop it.
    pub(crate) fn dim_bold_zero() -> DynamicImage {
        let mut img = cell(48, 56);
        paint_ring(&mut img, 24.0, 28.0, 8.0, 14.0, 4.0, DIM);
        wrap(img)
    }

    /// Same ring with a 2px stroke. Eight-connected ink must keep this one
    /// blob, or a thin zero splits into unread pieces.
    pub(crate) fn dim_thin_zero() -> DynamicImage {
        let mut img = cell(48, 56);
        paint_ring(&mut img, 24.0, 28.0, 8.0, 14.0, 2.0, DIM);
        wrap(img)
    }

    /// A rectangular frame whose fill is past 75% and whose hole still
    /// spans half the box. Fill is the only check that rejects it.
    pub(crate) fn dim_fill_frame() -> DynamicImage {
        let mut img = cell(48, 56);
        for y in 14..42 {
            for x in 14..32 {
                let in_hole = (18..27).contains(&x) && (20..33).contains(&y);
                if !in_hole {
                    img.put_pixel(x, y, DIM);
                }
            }
        }
        wrap(img)
    }

    /// A thin frame whose hole is more than 55% of the box. Hole size is
    /// the only check that rejects it. A closed ring large enough for the
    /// box floor already has more than 8 hole pixels, so the lower bound
    /// is not separately reachable.
    pub(crate) fn dim_wide_hole() -> DynamicImage {
        let mut img = cell(48, 56);
        for y in 14..42 {
            for x in 16..32 {
                let in_hole = (18..30).contains(&x) && (16..40).contains(&y);
                if !in_hole {
                    img.put_pixel(x, y, DIM);
                }
            }
        }
        wrap(img)
    }

    /// A frame whose hole sits far enough left that the centroid check is
    /// the only one that rejects it.
    pub(crate) fn dim_shifted_hole() -> DynamicImage {
        let mut img = cell(48, 56);
        for y in 12..42 {
            for x in 10..30 {
                let in_hole = (11..20).contains(&x) && (18..36).contains(&y);
                if !in_hole {
                    img.put_pixel(x, y, DIM);
                }
            }
        }
        wrap(img)
    }

    /// A 5x9 frame. The box floor is the only check that rejects it.
    pub(crate) fn dim_tiny_frame() -> DynamicImage {
        let mut img = cell(48, 56);
        for y in 20..29 {
            for x in 20..25 {
                let in_hole = (21..24).contains(&x) && (21..28).contains(&y);
                if !in_hole {
                    img.put_pixel(x, y, DIM);
                }
            }
        }
        wrap(img)
    }

    /// Two stacked counters. The pair is centered; only the one-hole rule
    /// keeps this from being read as zero.
    pub(crate) fn dim_eight() -> DynamicImage {
        let mut img = cell(48, 56);
        paint_ring(&mut img, 24.0, 18.0, 8.0, 8.0, 2.6, DIM);
        paint_ring(&mut img, 24.0, 36.0, 8.0, 8.0, 2.6, DIM);
        wrap(img)
    }

    pub(crate) fn dim_filled_blob() -> DynamicImage {
        let mut img = cell(48, 56);
        for y in 12..44 {
            for x in 16..32 {
                img.put_pixel(x, y, DIM);
            }
        }
        wrap(img)
    }

    /// One ring, wider than it is tall. A single digit is not this shape.
    pub(crate) fn dim_wide_pair() -> DynamicImage {
        let mut img = cell(64, 56);
        paint_ring(&mut img, 32.0, 28.0, 20.0, 12.0, 3.0, DIM);
        wrap(img)
    }

    /// A handful of dim pixels. Not a stroke and not a ring.
    pub(crate) fn dim_speck() -> DynamicImage {
        let mut img = cell(48, 56);
        for y in 26..29 {
            for x in 22..25 {
                img.put_pixel(x, y, DIM);
            }
        }
        wrap(img)
    }
}

#[cfg(test)]
mod dim_zero_tests {
    use super::dim_zero_fixtures::{
        cell, dim_bold_zero, dim_eight, dim_fill_frame, dim_filled_blob, dim_four_closed,
        dim_nine_realistic, dim_shifted_hole, dim_six_cell, dim_six_realistic, dim_speck,
        dim_stroke_cell, dim_ten, dim_ten_gap, dim_thin_zero, dim_tiny_frame, dim_wide_hole,
        dim_wide_pair, dim_zero_cell, dim_zero_touching_left_edge, paint_ring, wrap,
    };
    use super::dim_zero_glyph;
    use image::{DynamicImage, Rgb};

    fn is_zero(img: &DynamicImage) -> bool {
        dim_zero_glyph(img, 2).is_some()
    }

    #[test]
    fn a_dim_ring_is_a_zero_and_a_stroke_is_not() {
        let hit = dim_zero_glyph(&dim_zero_cell(), 2).expect("centered dim ring");
        assert!(!hit.touches_edge);
        assert!(!is_zero(&dim_stroke_cell()), "a dim 1 has no hole");
        assert!(!is_zero(&dim_six_cell()), "a low hole is a 6, not a 0");
        assert!(!is_zero(&DynamicImage::ImageRgb8(cell(48, 56))));
    }

    #[test]
    fn a_ring_in_the_outer_columns_is_a_suspect_zero() {
        let hit = dim_zero_glyph(&dim_zero_touching_left_edge(), 2)
            .expect("a complete ring may sit against the crop edge");
        assert!(hit.touches_edge);
    }

    #[test]
    fn dim_zero_confidence_follows_the_documented_scale() {
        use super::dim_zero_confidence;
        assert_eq!(dim_zero_confidence(38, 22), 55, "the floor");
        assert_eq!(dim_zero_confidence(30, 40), 55, "never below the floor");
        assert_eq!(dim_zero_confidence(44, 0), 95, "the ceiling");
        assert_eq!(dim_zero_confidence(60, 0), 95, "never above the ceiling");
        assert!(dim_zero_confidence(41, 5) >= 78);
        assert!(dim_zero_confidence(42, 4) > dim_zero_confidence(40, 4));
        assert!(dim_zero_confidence(42, 2) > dim_zero_confidence(42, 10));
    }

    #[test]
    fn a_hit_carries_its_geometric_confidence_and_the_edge_penalty() {
        use super::{DIM_ZERO_EDGE_PENALTY, dim_zero_confidence};
        let hit = dim_zero_glyph(&dim_zero_cell(), 2).expect("centered dim ring");
        assert_eq!(
            hit.confidence,
            dim_zero_confidence(hit.hole_extent_pct, hit.centre_offset_pct)
        );
        assert!(hit.confidence >= 70, "a clean ring is not suspect: {hit:?}");
        let edge = dim_zero_glyph(&dim_zero_touching_left_edge(), 2).expect("edge ring");
        assert_eq!(
            edge.confidence,
            dim_zero_confidence(edge.hole_extent_pct, edge.centre_offset_pct)
                - DIM_ZERO_EDGE_PENALTY
        );
    }

    #[test]
    fn a_bright_ring_is_not_the_dim_zero_path() {
        let mut img = cell(48, 56);
        paint_ring(&mut img, 24.0, 28.0, 8.0, 14.0, 3.0, Rgb([236, 236, 236]));
        assert!(
            !is_zero(&wrap(img)),
            "a bright digit stays on the Tesseract path"
        );
    }

    /// Below white, not a single grey. Floor 96, a core like the measured
    /// stroke, a value well above that core, the ceiling, then the first
    /// step past it and a white digit.
    #[test]
    fn a_ring_clearly_darker_than_white_is_a_zero() {
        for (gray, readable) in [
            (90u8, false),
            (96, true),
            (180, true),
            (220, true),
            (221, false),
            (250, false),
        ] {
            let mut img = cell(48, 56);
            paint_ring(
                &mut img,
                24.0,
                28.0,
                8.0,
                14.0,
                3.0,
                Rgb([gray, gray, gray]),
            );
            assert_eq!(
                is_zero(&wrap(img)),
                readable,
                "gray {gray} readable={readable}"
            );
        }
    }

    /// Neutral grey: channels roughly equal. Saturation 40 is the cap
    /// (measured stroke max about 33, plus margin). Just over the cap is
    /// row-fill colour on the edge of a glyph, not the stroke.
    #[test]
    fn a_neutral_grey_ring_is_a_zero_and_a_saturated_ring_is_not() {
        // (172 - 145) * 255 / 172 = 40. The next step is 41.
        let mut at_cap = cell(48, 56);
        paint_ring(
            &mut at_cap,
            24.0,
            28.0,
            8.0,
            14.0,
            3.0,
            Rgb([172, 145, 145]),
        );
        assert!(is_zero(&wrap(at_cap)), "saturation 40 still reads");
        let mut over_cap = cell(48, 56);
        paint_ring(
            &mut over_cap,
            24.0,
            28.0,
            8.0,
            14.0,
            3.0,
            Rgb([172, 144, 144]),
        );
        assert!(!is_zero(&wrap(over_cap)), "saturation 41 is not neutral");
        let mut img = cell(48, 56);
        paint_ring(&mut img, 24.0, 28.0, 8.0, 14.0, 3.0, Rgb([140, 40, 40]));
        assert!(!is_zero(&wrap(img)), "a red ring is row fill");
    }

    #[test]
    fn realistic_dim_digits_are_not_zero() {
        assert!(!is_zero(&dim_six_realistic()), "realistic 6");
        assert!(!is_zero(&dim_nine_realistic()), "realistic 9");
        assert!(!is_zero(&dim_four_closed()), "closed 4");
        assert!(!is_zero(&dim_ten()), "10");
        assert!(!is_zero(&dim_eight()), "8");
    }

    #[test]
    fn shape_checks_reject_a_blob_a_wide_cell_and_a_speck() {
        assert!(!is_zero(&dim_filled_blob()), "filled blob");
        assert!(!is_zero(&dim_wide_pair()), "two-digit cell");
        assert!(!is_zero(&dim_speck()), "speck");
    }

    #[test]
    fn a_bold_zero_still_reads_and_each_shape_check_has_its_own_reject() {
        // 4px stroke on a 16px-wide ring. Hole extent is about half the
        // box, so raising the 38% rule to 60% drops this zero.
        assert!(is_zero(&dim_bold_zero()), "bold zero");
        assert!(is_zero(&dim_thin_zero()), "thin zero stays one component");
        assert!(!is_zero(&dim_fill_frame()), "fill");
        assert!(!is_zero(&dim_wide_hole()), "hole size");
        assert!(!is_zero(&dim_shifted_hole()), "centroid");
        assert!(!is_zero(&dim_tiny_frame()), "box floor");
    }

    #[test]
    fn a_close_ten_is_not_a_zero() {
        // The extent rule only rejects a "10" once the stroke is far
        // enough to widen the box. A gap of 1 to 3px still has a full
        // ring, and it must stay unread because it is two components.
        for gap in 1..=4 {
            assert!(!is_zero(&dim_ten_gap(4, gap)), "4px stroke at gap {gap}");
            assert!(!is_zero(&dim_ten_gap(3, gap)), "3px stroke at gap {gap}");
        }
        assert!(!is_zero(&dim_ten()), "fixture gap");
    }
}

#[cfg(test)]
mod uncapped_retry_tests {
    use super::{prepare_cell_binary, prepare_cell_binary_uncapped};
    use image::{DynamicImage, RgbImage};

    fn blank(h: u32) -> DynamicImage {
        DynamicImage::ImageRgb8(RgbImage::new(40, h))
    }

    #[test]
    fn only_capped_heights_get_an_uncapped_retry() {
        // 38 px is the truncated bottom row at native: capped to 64/39.
        let retry = prepare_cell_binary_uncapped(&blank(38)).expect("38 px is capped");
        let capped = prepare_cell_binary(&blank(38));
        assert_eq!(retry.height(), 64, "64/38 of 38 px");
        assert_eq!(capped.height(), 62, "64/39 of 38 px");
        for h in [20, 27, 39, 44, 48, 60] {
            assert!(prepare_cell_binary_uncapped(&blank(h)).is_none(), "h={h}");
        }
    }
}
