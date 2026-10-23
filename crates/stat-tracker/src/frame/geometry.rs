//! Board geometry shared by both readers: the scoreboard crop, the 16:9
//! game rect, header label groups, the map-name box, the table-present
//! check and player-row crops. No Tesseract here; the Tesseract
//! binarisation stays in `ocr/preprocess.rs`, which re-exports this module
//! so the old `ocr::preprocess::…` paths keep resolving.

use image::{DynamicImage, RgbImage};

/// Scoreboard layout constants for 2560x1440 OW2 fullscreen.
const SCOREBOARD_X_RATIO: f64 = 0.175;
const SCOREBOARD_Y_RATIO: f64 = 0.15;
const SCOREBOARD_W_RATIO: f64 = 0.65;
const SCOREBOARD_H_RATIO: f64 = 0.70;

/// Height of [`crop_scoreboard`] on a 1440p 16:9 frame.
///
/// `1440 * SCOREBOARD_H_RATIO` truncates to 1007. Callers that scale a
/// 1440p pixel distance by board height divide by this, so the reference
/// is not copied as a second literal.
pub const SCOREBOARD_CROP_H_1440: u32 = 1007;

/// 6v6 squeezes the table horizontally: the name plate sits further left.
/// Measured on the 2026-07-16 fixture (12-row board, dumped row_00.png):
/// name text spans ~0.155–0.24 of row width; 0.26+ lands on the E/A/D digits,
/// which is exactly what OCR read before this constant existed.
pub(crate) const NAME_COL_X_6V6: f64 = 0.15;
pub(crate) const NAME_COL_W_6V6: f64 = 0.10;

/// 5v5 name windows `(x, w)`, one per table layout, as fractions of row
/// width. Measured 2026-10-12 on 15 in-match boards (0, 1 and 2 perks) and
/// one centred pre-match board at 2560x1440:
///
/// | layout | names | first thing to the right |
/// |---|---|---|
/// | in-match, no perks | 0.106-0.225 | E digits from about 0.27 |
/// | in-match with perks | 0.097-0.184 | perk icons, 0.214-0.292 |
/// | centred | 0.328-0.415 | E digits from about 0.46 |
///
/// The in-match table sits left of the career panel and moves further left
/// for each perk column; the centred table is the no-perk table moved right
/// by 0.186. Each window covers its layout's names and stops before that
/// layout's digits and perk icons. A window must also clear the ult-charge
/// ring and rank badge on the left: the old single window (0.26-0.38) took in
/// the centred board's ult ring and read the in-match E/A digits, and found
/// the own row on 0 of 18 boards. These three found it on 18 of 18.
pub(crate) const NAME_WINDOWS_5V5: [(f64, f64); 3] = [
    (NAME_COL_X_5V5_INGAME, NAME_COL_W_5V5_INGAME),
    (NAME_COL_X_5V5_PERKS, NAME_COL_W_5V5_PERKS),
    (NAME_COL_X_5V5_CENTRED, NAME_COL_W_5V5_CENTRED),
];
/// Index of the in-match no-perk window in `ocr::preprocess::name_windows`
/// for 5v5. The 5v5 portrait grid fits only this layout (see
/// [`crate::detect::hero_portrait::portrait_seed_layout_known`]).
pub const NAME_WINDOW_5V5_INGAME: usize = 0;
/// Index of the in-match with-perks window in
/// `ocr::preprocess::name_windows` for 5v5.
pub const NAME_WINDOW_5V5_PERKS: usize = 1;
/// Index of the centred pre-match window in
/// `ocr::preprocess::name_windows` for 5v5.
pub const NAME_WINDOW_5V5_CENTRED: usize = 2;
/// In-match, no perks: 0.10-0.25.
const NAME_COL_X_5V5_INGAME: f64 = 0.10;
const NAME_COL_W_5V5_INGAME: f64 = 0.15;
/// In-match with one or two perks: 0.095-0.20.
const NAME_COL_X_5V5_PERKS: f64 = 0.095;
const NAME_COL_W_5V5_PERKS: f64 = 0.105;
/// Centred pre-match table: 0.29-0.43.
const NAME_COL_X_5V5_CENTRED: f64 = 0.29;
const NAME_COL_W_5V5_CENTRED: f64 = 0.14;

/// Row layout: header takes ~2.5% of scoreboard height.
/// Team 1 starts immediately after header. Team 2 starts at ~56.5% (measured from
/// real screenshots — the VS divider gap is larger than initially estimated).
const HEADER_RATIO: f64 = 0.025;
const TEAM2_START_RATIO: f64 = 0.565;

/// Widest a real header label can be, as a board-width ratio. "DMG" measures
/// 0.023 on the 2026-07 fixtures; UI junk that leaks into the header strip
/// (mic icon + highlighted-hero overlay art) merges into a ~0.37-wide pseudo
/// group, so a generous 2x margin over the widest real label separates them
/// cleanly (CG-4).
pub(crate) const HEADER_LABEL_MAX_W: f64 = 0.05;

/// Plausible spacing between adjacent header label centers. Measured gaps on
/// the 2026-07-20/23 fixtures (identical for 5v5 and 6v6): 0.034–0.065.
pub(crate) const HEADER_GAP_MIN: f64 = 0.02;
pub(crate) const HEADER_GAP_MAX: f64 = 0.10;

/// Dark-text label groups found in the scoreboard header strip, as (start, end)
/// pixel columns. A real scoreboard yields one group per stat label (E, A, D,
/// DMG, H, MIT — six, sometimes merged/split by a step or two). Shared by
/// column calibration and the pre-OCR scoreboard preflight: brightness-based,
/// so it still fires on the desaturated endorse-phase board where the
/// saturation row-dip scan goes blind.
///
/// Column calibration (Tesseract) and the preflight use this fixed scan. The
/// cv digit reader uses [`header_label_groups_scaled`] instead.
pub fn header_label_groups(scoreboard: &DynamicImage) -> Vec<(u32, u32)> {
    label_groups(scoreboard, LabelScan::Fixed)
}

/// Header labels for the cv digit reader. Same scan as
/// [`header_label_groups`] with two changes:
///
/// * the dark cut ([`HEADER_DARK_CUT`]) scales with the strip's light
///   background (95th percentile brightness of the stat area, reference
///   [`HEADER_BG_REF`]), never above the fixed cut, so a dimmed board still
///   separates its labels from the strip;
/// * the cluster minimum rounds down instead of to nearest, so a 2 px "H"
///   label on a board narrower than 1664 px is kept.
pub fn header_label_groups_scaled(scoreboard: &DynamicImage) -> Vec<(u32, u32)> {
    label_groups(scoreboard, LabelScan::Scaled)
}

/// [`header_label_groups_scaled`] with the fixed dark cut: what the scaled
/// scan finds when the strip is not treated as dimmed.
pub fn header_label_groups_scaled_undimmed(scoreboard: &DynamicImage) -> Vec<(u32, u32)> {
    label_groups(scoreboard, LabelScan::ScaledFixedCut)
}

/// The scaled scan lowers its dark cut on this board: the header strip's
/// light background is below [`HEADER_BG_REF`] by enough to move the cut.
pub fn header_strip_dimmed(scoreboard: &DynamicImage) -> bool {
    let rgb = header_strip_rgb(scoreboard);
    header_dark_cut(&rgb, LabelScan::Scaled) < HEADER_DARK_CUT
}

/// The thin header strip [`header_label_groups`] scans, as RGB.
fn header_strip_rgb(scoreboard: &DynamicImage) -> image::RgbImage {
    let (w, h) = (scoreboard.width(), scoreboard.height());
    let scan_start = (h as f64 * 0.005) as u32;
    let scan_end = (h as f64 * 0.025).max(15.0) as u32;
    let scan_h = scan_end.saturating_sub(scan_start).max(1);
    scoreboard
        .crop_imm(0, scan_start, w, scan_h.min(h.saturating_sub(scan_start)))
        .to_rgb8()
}

/// Brightness below which a header strip pixel counts as label ink.
pub const HEADER_DARK_CUT: u32 = 150;
/// Light strip background (p95 brightness) of a normal Tab board header.
pub const HEADER_BG_REF: u32 = 219;

#[derive(Clone, Copy, PartialEq, Eq)]
enum LabelScan {
    Fixed,
    Scaled,
    ScaledFixedCut,
}

/// The dark cut for a header strip under `scan`.
fn header_dark_cut(rgb: &image::RgbImage, scan: LabelScan) -> u32 {
    if scan != LabelScan::Scaled {
        return HEADER_DARK_CUT;
    }
    let w = rgb.width();
    let mut v: Vec<u32> = Vec::with_capacity((w - w / 4) as usize * rgb.height() as usize);
    for y in 0..rgb.height() {
        for x in w / 4..w {
            let px = rgb.get_pixel(x, y);
            v.push((px.0[0] as u32 + px.0[1] as u32 + px.0[2] as u32) / 3);
        }
    }
    if v.is_empty() {
        return HEADER_DARK_CUT;
    }
    v.sort_unstable();
    let bg = v[(v.len() - 1) * 95 / 100].min(HEADER_BG_REF);
    (HEADER_DARK_CUT * bg + HEADER_BG_REF / 2) / HEADER_BG_REF
}

fn label_groups(scoreboard: &DynamicImage, scan: LabelScan) -> Vec<(u32, u32)> {
    let (w, h) = (scoreboard.width(), scoreboard.height());

    let scan_start = (h as f64 * 0.005) as u32;
    let scan_end = (h as f64 * 0.025).max(15.0) as u32;
    let scan_h = scan_end.saturating_sub(scan_start).max(1);
    // Only convert the thin header strip to RGB — not the whole scoreboard.
    let header = scoreboard.crop_imm(0, scan_start, w, scan_h.min(h.saturating_sub(scan_start)));
    let rgb = header.to_rgb8();
    let scan_rows = rgb.height().max(1);
    let dark_cut = header_dark_cut(&rgb, scan);

    // Count dark pixels per column in the header area
    let mut col_dark = vec![0u32; w as usize];
    for y in 0..rgb.height() {
        for x in 0..w {
            let px = rgb.get_pixel(x, y);
            let brightness = (px.0[0] as u32 + px.0[1] as u32 + px.0[2] as u32) / 3;
            if brightness < dark_cut {
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
    let min_w = w as f64 * 3.0 / 1664.0;
    let min_w = if scan != LabelScan::Fixed {
        min_w.floor()
    } else {
        min_w.round()
    };
    let min_cluster_w = (min_w as u32).clamp(2, 3);
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
    let (x, y, crop_w, crop_h) = crop_scoreboard_rect(img.width(), img.height());
    img.crop_imm(x, y, crop_w, crop_h)
}

/// The rectangle [`crop_scoreboard`] cuts from a `w` x `h` frame, as
/// `(x, y, width, height)`. Report blanking uses the same rectangle.
///
/// This is the 6v6 crop (1007 px tall at 1440p, 756 px at 1080p). A 5v5
/// board uses [`crop_scoreboard_rect_for`] with team size 5, which keeps
/// this top edge and grows downward only.
pub fn crop_scoreboard_rect(w: u32, h: u32) -> (u32, u32, u32, u32) {
    // OW2's scoreboard renders inside a 16:9 region centered on the frame. On
    // non-16:9 displays (ultrawide, 16:10) the ratios below must apply to that
    // inner region, not the whole frame, or the crop drifts off the scoreboard.
    let (gx, gy, gw, gh) = game_rect_16_9(w, h);
    let x = gx + (gw as f64 * SCOREBOARD_X_RATIO) as u32;
    let y = gy + (gh as f64 * SCOREBOARD_Y_RATIO) as u32;
    let crop_w = ((gw as f64 * SCOREBOARD_W_RATIO) as u32).min(w.saturating_sub(x));
    let crop_h = ((gh as f64 * SCOREBOARD_H_RATIO) as u32).min(h.saturating_sub(y));
    (x, y, crop_w, crop_h)
}

/// Pixels added under a 1440p scoreboard crop so the 5v5 bottom portrait
/// fits. On that crop the square for row 9 starts at y 928 and is 86 px
/// tall, so it ends at y 1014. The unpadded crop is 1007 px and the slot
/// code skips any square that does not fit.
const SCOREBOARD_5V5_BOTTOM_PAD_1440: u32 = 7;

/// Bottom pad for a 5v5 crop, in px.
///
/// [`SCOREBOARD_5V5_BOTTOM_PAD_1440`] scaled by the frame height from
/// 1440p, then clamped to the pixels left under the unpadded crop so the
/// rectangle stays inside the frame.
pub fn scoreboard_5v5_bottom_pad(frame_w: u32, frame_h: u32) -> u32 {
    let (_, y, _, crop_h) = crop_scoreboard_rect(frame_w, frame_h);
    bottom_pad_clamped(frame_h, y, crop_h)
}

/// `SCOREBOARD_5V5_BOTTOM_PAD_1440` scaled from `frame_h`, stopped where
/// the frame ends.
fn bottom_pad_clamped(frame_h: u32, y: u32, crop_h: u32) -> u32 {
    let wanted =
        (SCOREBOARD_5V5_BOTTOM_PAD_1440 as f64 * f64::from(frame_h) / 1440.0).round() as u32;
    let room = frame_h.saturating_sub(y.saturating_add(crop_h));
    wanted.min(room)
}

/// [`crop_scoreboard_rect`] for a known team size.
///
/// Team size 5 grows downward by [`scoreboard_5v5_bottom_pad`]. Team size
/// 6 and any other size keep the unpadded rectangle: same origin, width
/// and height. The top edge does not move. Recognizers that place rows by
/// dividing the crop height ([`crop_player_row`], the digit reader) must
/// keep the unpadded image from [`crop_scoreboard`]. This function only
/// adds pixels below that image.
pub fn crop_scoreboard_rect_for(w: u32, h: u32, team_size: usize) -> (u32, u32, u32, u32) {
    let (x, y, crop_w, crop_h) = crop_scoreboard_rect(w, h);
    if team_size != 5 {
        return (x, y, crop_w, crop_h);
    }
    let pad = bottom_pad_clamped(h, y, crop_h);
    (x, y, crop_w, crop_h.saturating_add(pad))
}

/// [`crop_scoreboard`] plus the 5v5 bottom pad when `team_size` is 5.
/// Other team sizes are the unpadded crop.
pub fn crop_scoreboard_for(img: &DynamicImage, team_size: usize) -> DynamicImage {
    let (x, y, crop_w, crop_h) = crop_scoreboard_rect_for(img.width(), img.height(), team_size);
    img.crop_imm(x, y, crop_w, crop_h)
}

/// `board_w` x `board_h` is this frame's scoreboard crop: the unpadded
/// rectangle, or that rectangle plus the 5v5 bottom pad.
pub fn is_frame_scoreboard_crop(frame_w: u32, frame_h: u32, board_w: u32, board_h: u32) -> bool {
    let (_, _, cw, ch) = crop_scoreboard_rect(frame_w, frame_h);
    let (_, _, _, ch5) = crop_scoreboard_rect_for(frame_w, frame_h, 5);
    board_w == cw && (board_h == ch || board_h == ch5)
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

/// Top-bar map label, as fractions of the 16:9 playfield. Top-right, above
/// the scoreboard (for example "WATCHPOINT: GIBRALTAR").
const MAP_NAME_X_RATIO: f64 = 0.68;
const MAP_NAME_Y_RATIO: f64 = 0.022;
const MAP_NAME_W_RATIO: f64 = 0.27;
const MAP_NAME_H_RATIO: f64 = 0.040;

/// Pixel rectangle of the top-bar map-name label, in full-frame coordinates.
///
/// `None` when that rectangle is empty or does not fit inside the frame. The
/// ratios match [`crop_map_name`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MapNameRect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

pub fn map_name_rect(frame_w: u32, frame_h: u32) -> Option<MapNameRect> {
    let (gx, gy, gw, gh) = game_rect_16_9(frame_w, frame_h);
    if gw == 0 || gh == 0 || frame_w == 0 || frame_h == 0 {
        return None;
    }
    let x = gx + (gw as f64 * MAP_NAME_X_RATIO) as u32;
    let y = gy + (gh as f64 * MAP_NAME_Y_RATIO) as u32;
    let w = (gw as f64 * MAP_NAME_W_RATIO) as u32;
    let h = (gh as f64 * MAP_NAME_H_RATIO) as u32;
    if w == 0 || h == 0 {
        return None;
    }
    let x_end = x.checked_add(w)?;
    let y_end = y.checked_add(h)?;
    if x_end > frame_w || y_end > frame_h {
        return None;
    }
    Some(MapNameRect { x, y, w, h })
}

/// Crop the top-bar map-name label (top-right, e.g. "WATCHPOINT: GIBRALTAR").
///
/// This sits above the scoreboard crop, so scoreboard OCR never sees it. White
/// text on a dark bar. `recognize_map_label` crops this window from the frame.
///
/// The full 0.27-wide window still reaches the match timer (`ILIOS` read as
/// `ILIOS TIM`). A fixed slice off that window also cuts long names
/// (`WATCHPOINT: GRIMSVOTN` came back as `GRiMsVO`). The right edge moves
/// left only when a timer-sized ink run sits past a gap wider than letter
/// and word spacing. No such gap, and the full window stays, so the longest
/// name still fits at 1440p and 1080p.
pub fn crop_map_name(img: &DynamicImage) -> DynamicImage {
    let (w, h) = (img.width(), img.height());
    let (gx, gy, gw, gh) = game_rect_16_9(w, h);
    let (x, y, cw_full, ch) = match map_name_rect(w, h) {
        Some(r) => (r.x, r.y, r.w, r.h),
        None => {
            let x = gx + (gw as f64 * MAP_NAME_X_RATIO) as u32;
            let y = gy + (gh as f64 * MAP_NAME_Y_RATIO) as u32;
            let cw = ((gw as f64 * MAP_NAME_W_RATIO) as u32).min(w.saturating_sub(x));
            let ch = ((gh as f64 * MAP_NAME_H_RATIO) as u32).min(h.saturating_sub(y));
            (x, y, cw, ch)
        }
    };
    let trim = if cw_full == 0 {
        0
    } else {
        map_name_timer_trim(img, x, y, cw_full, ch, gh).min(cw_full.saturating_sub(1))
    };
    img.crop_imm(x, y, cw_full.saturating_sub(trim), ch)
}

/// Full map-label box, with no timer trim and no pixel change.
///
/// The reader still uses [`crop_map_name`]. A saved crop uses this so the
/// PNG is the native rectangle, including the clock when it sits in the box.
pub fn crop_map_label_box(img: &DynamicImage) -> Option<DynamicImage> {
    let rect = map_name_rect(img.width(), img.height())?;
    Some(img.crop_imm(rect.x, rect.y, rect.w, rect.h))
}

/// Whether the map-label box has text worth saving.
///
/// A flat bar saves nothing. The floor is under a mid fade-in (luma near
/// 107), so a faint label still counts. This is not the reader trim, which
/// still treats luma 170 as ink.
pub fn map_label_has_text(img: &DynamicImage) -> bool {
    let Some(rect) = map_name_rect(img.width(), img.height()) else {
        return false;
    };
    // Below a fade-in near 107, above a dark empty bar.
    const SAVE_LUMA: u16 = 100;
    // A flat fill, dark or bright, has almost no spread.
    const FLAT_SPREAD: u16 = 24;
    const MIN_INK: u32 = 8;
    let rgb = img.crop_imm(rect.x, rect.y, rect.w, rect.h).to_rgb8();
    let mut min_l = 255u16;
    let mut max_l = 0u16;
    let mut ink = 0u32;
    for px in rgb.pixels() {
        let luma = (u16::from(px.0[0]) + u16::from(px.0[1]) + u16::from(px.0[2])) / 3;
        min_l = min_l.min(luma);
        max_l = max_l.max(luma);
        if luma >= SAVE_LUMA {
            ink += 1;
        }
    }
    max_l.saturating_sub(min_l) >= FLAT_SPREAD && ink >= MIN_INK
}

/// How much of the map-name window is the match timer, in pixels.
///
/// Ink columns merge across gaps up to about 1.2% of the game height (letter
/// and word spacing). A later run is the timer when that gap is wider and the
/// run is narrower than the name, or when the hole is at least 6% of the
/// window (a short name with the clock far to the right).
fn map_name_timer_trim(img: &DynamicImage, x: u32, y: u32, cw: u32, ch: u32, gh: u32) -> u32 {
    if cw < 8 || ch < 4 || gh == 0 {
        return 0;
    }
    let rgb = img.crop_imm(x, y, cw, ch).to_rgb8();
    let ink: Vec<bool> = (0..cw)
        .map(|col| {
            (0..ch)
                .filter(|&row| {
                    let p = rgb.get_pixel(col, row).0;
                    let luma = (u16::from(p[0]) + u16::from(p[1]) + u16::from(p[2])) / 3;
                    luma >= 170
                })
                .count()
                >= 2
        })
        .collect();
    let merge = ((gh as f64) * 0.012).round().max(3.0) as u32;
    let runs = ink_runs(&ink, merge);
    let Some((last_i, &(last_s, last_e))) = runs.iter().enumerate().next_back() else {
        return 0;
    };
    if last_i == 0 {
        return 0;
    }
    let prev_e = runs[last_i - 1].1;
    let gap = last_s.saturating_sub(prev_e);
    if gap <= merge {
        return 0;
    }
    let timer_w = last_e.saturating_sub(last_s);
    let left_span = prev_e.saturating_sub(runs[0].0);
    let shorter_than_name = left_span > 0 && timer_w.saturating_mul(2) < left_span;
    let wide_hole = gap as f64 >= cw as f64 * 0.06;
    if shorter_than_name || wide_hole {
        cw.saturating_sub(last_s)
    } else {
        0
    }
}

/// Ink runs, bridging holes of at most `merge` columns.
fn ink_runs(ink: &[bool], merge: u32) -> Vec<(u32, u32)> {
    let mut runs = Vec::new();
    let mut start: Option<u32> = None;
    let mut last: Option<u32> = None;
    for (i, on) in ink.iter().copied().enumerate() {
        let i = i as u32;
        if on {
            if start.is_none() {
                start = Some(i);
            }
            last = Some(i);
        } else if let (Some(s), Some(end)) = (start, last)
            && i - end > merge
        {
            runs.push((s, end + 1));
            start = None;
            last = None;
        }
    }
    if let (Some(s), Some(end)) = (start, last) {
        runs.push((s, end + 1));
    }
    runs
}

/// True when `board` (a [`crop_scoreboard`] result) has a Tab table: a bright
/// header bar, two stacks of wide flat rows, and a darker gap between the
/// stacks. Brightness and layout only. Team hue is ignored, so purple, yellow,
/// or any other recolour still counts. A death cam with a killfeed and HUD
/// does not.
pub fn scoreboard_table_present(board: &DynamicImage) -> bool {
    let Some(m) = measure_table(&board.to_rgb8()) else {
        return false;
    };
    // Header: light bar. Each stack: most scanlines are one brightness.
    // The VS gap sits between the stacks and is darker than both.
    let present = m.header_median >= 150.0
        && m.header_bright >= 0.40
        && m.team1_flat >= 0.50
        && m.team2_flat >= 0.50
        && m.gap_luma + 15.0 < m.team1_luma
        && m.gap_luma + 15.0 < m.team2_luma;
    tracing::debug!(
        header_median = m.header_median,
        header_bright = m.header_bright,
        team1_flat = m.team1_flat,
        team2_flat = m.team2_flat,
        team1_luma = m.team1_luma,
        team2_luma = m.team2_luma,
        gap_luma = m.gap_luma,
        table = present,
        "scoreboard table check"
    );
    present
}

struct TableMeasure {
    header_median: f64,
    header_bright: f64,
    team1_flat: f64,
    team2_flat: f64,
    team1_luma: f64,
    team2_luma: f64,
    gap_luma: f64,
}

fn measure_table(rgb: &RgbImage) -> Option<TableMeasure> {
    let (w, h) = rgb.dimensions();
    if w < 32 || h < 32 {
        return None;
    }
    let (header_median, header_bright) = region_header(rgb, 0.28, 0.97, 0.0, 0.030)?;
    let (team1_flat, team1_luma) = region_stack(rgb, 0.12, 0.90, 0.10, 0.46)?;
    let gap_luma = region_median(rgb, 0.12, 0.90, 0.500, 0.555)?;
    let (team2_flat, team2_luma) = region_stack(rgb, 0.12, 0.90, 0.62, 0.90)?;
    Some(TableMeasure {
        header_median,
        header_bright,
        team1_flat,
        team2_flat,
        team1_luma,
        team2_luma,
        gap_luma,
    })
}

fn region_px(rgb: &RgbImage, x0: f64, x1: f64, y0: f64, y1: f64) -> Option<(u32, u32, u32, u32)> {
    let (w, h) = rgb.dimensions();
    let xa = (w as f64 * x0).round() as u32;
    let xb = (w as f64 * x1).round() as u32;
    let ya = (h as f64 * y0).round() as u32;
    let yb = (h as f64 * y1).round() as u32;
    if xb <= xa || yb <= ya || xa >= w || ya >= h {
        return None;
    }
    Some((xa, xb.min(w), ya, yb.min(h)))
}

fn px_luma(px: [u8; 3]) -> u8 {
    ((px[0] as u16 + px[1] as u16 + px[2] as u16) / 3) as u8
}

fn median_u8(values: &mut [u8]) -> u8 {
    values.sort_unstable();
    values[values.len() / 2]
}

fn region_samples(rgb: &RgbImage, x0: u32, x1: u32, y0: u32, y1: u32) -> Vec<u8> {
    let mut values = Vec::with_capacity(((x1 - x0) * (y1 - y0)) as usize);
    for y in y0..y1 {
        for x in x0..x1 {
            values.push(px_luma(rgb.get_pixel(x, y).0));
        }
    }
    values
}

/// Median luma of the header window, and the fraction of pixels at luma >= 140.
fn region_header(rgb: &RgbImage, x0: f64, x1: f64, y0: f64, y1: f64) -> Option<(f64, f64)> {
    let (xa, xb, ya, yb) = region_px(rgb, x0, x1, y0, y1)?;
    let mut values = region_samples(rgb, xa, xb, ya, yb);
    if values.is_empty() {
        return None;
    }
    let bright = values.iter().filter(|&&v| v >= 140).count() as f64 / values.len() as f64;
    Some((median_u8(&mut values) as f64, bright))
}

/// Fraction of scanlines whose pixels sit near that line's median, and the
/// median luma of the whole window. A team row is flat across the table.
/// A death cam is not.
fn region_stack(rgb: &RgbImage, x0: f64, x1: f64, y0: f64, y1: f64) -> Option<(f64, f64)> {
    let (xa, xb, ya, yb) = region_px(rgb, x0, x1, y0, y1)?;
    let width = (xb - xa) as usize;
    if width == 0 || yb <= ya {
        return None;
    }
    let mut flat = 0u32;
    let mut all = Vec::with_capacity(width * (yb - ya) as usize);
    for y in ya..yb {
        let mut line = Vec::with_capacity(width);
        for x in xa..xb {
            line.push(px_luma(rgb.get_pixel(x, y).0));
        }
        let med = {
            let mut sorted = line.clone();
            median_u8(&mut sorted)
        };
        let close = line.iter().filter(|&&v| v.abs_diff(med) <= 36).count();
        if close as f64 / width as f64 >= 0.62 {
            flat += 1;
        }
        all.extend(line);
    }
    let rows = (yb - ya) as f64;
    Some((flat as f64 / rows, median_u8(&mut all) as f64))
}

fn region_median(rgb: &RgbImage, x0: f64, x1: f64, y0: f64, y1: f64) -> Option<f64> {
    let (xa, xb, ya, yb) = region_px(rgb, x0, x1, y0, y1)?;
    let mut values = region_samples(rgb, xa, xb, ya, yb);
    if values.is_empty() {
        return None;
    }
    Some(median_u8(&mut values) as f64)
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

    let row_h = row_slot_height(h, team_size);

    let base_y = if team == 0 { team1_start } else { team2_start };
    let y = base_y + (team_row as u32 * row_h);

    let actual_h = row_h.min(h.saturating_sub(y));
    if actual_h < row_h / 2 {
        return None;
    }

    Some(scoreboard.crop_imm(0, y, w, actual_h))
}

/// Pixel height of one scoreboard row slot.
///
/// The band from the header to team 2, divided into `team_size` rows plus
/// the one-row gap. [`crop_player_row`] crops with this height. Dividing by
/// the crop height is the row pitch the team-size check compares.
pub(crate) fn row_slot_height(crop_h: u32, team_size: usize) -> u32 {
    let team1_start = (crop_h as f64 * HEADER_RATIO) as u32;
    let team2_start = (crop_h as f64 * TEAM2_START_RATIO) as u32;
    (team2_start - team1_start) / (team_size as u32 + 1)
}

#[cfg(test)]
mod map_name_rect_tests {
    use super::{
        HEADER_RATIO, NAME_COL_W_6V6, NAME_COL_X_6V6, NAME_WINDOWS_5V5, SCOREBOARD_H_RATIO,
        SCOREBOARD_W_RATIO, SCOREBOARD_X_RATIO, SCOREBOARD_Y_RATIO, crop_map_label_box,
        game_rect_16_9, map_name_rect,
    };
    use image::{DynamicImage, Rgb, RgbImage};

    fn overlaps(a: (u32, u32, u32, u32), b: (u32, u32, u32, u32)) -> bool {
        let (ax, ay, aw, ah) = a;
        let (bx, by, bw, bh) = b;
        ax < bx.saturating_add(bw)
            && bx < ax.saturating_add(aw)
            && ay < by.saturating_add(bh)
            && by < ay.saturating_add(ah)
    }

    /// Name column inside the scoreboard crop. `x_ratio` / `w_ratio` are the
    /// 5v5 or 6v6 name-cell fractions of that crop.
    fn name_area(frame_w: u32, frame_h: u32, x_ratio: f64, w_ratio: f64) -> (u32, u32, u32, u32) {
        let (gx, gy, gw, gh) = game_rect_16_9(frame_w, frame_h);
        let bx = gx + (gw as f64 * SCOREBOARD_X_RATIO) as u32;
        let by = gy + (gh as f64 * SCOREBOARD_Y_RATIO) as u32;
        let bw = (gw as f64 * SCOREBOARD_W_RATIO) as u32;
        let bh = (gh as f64 * SCOREBOARD_H_RATIO) as u32;
        let x = bx + (bw as f64 * x_ratio) as u32;
        let y = by + (bh as f64 * HEADER_RATIO) as u32;
        let w = (bw as f64 * w_ratio) as u32;
        let h = bh.saturating_sub(y.saturating_sub(by));
        (x, y, w, h)
    }

    #[test]
    fn map_label_stays_inside_the_box_at_1080p_1440p_and_ultrawide() {
        for (w, h, label) in [
            (1920, 1080, "1080p"),
            (2560, 1440, "1440p"),
            (3440, 1440, "21:9"),
        ] {
            let rect = map_name_rect(w, h).unwrap_or_else(|| panic!("{label}: region missing"));
            let (gx, gy, gw, gh) = game_rect_16_9(w, h);
            let top_bar_bottom = gy + (gh as f64 * SCOREBOARD_Y_RATIO) as u32;
            assert!(rect.x >= gx, "{label}: left of the playfield");
            assert!(
                rect.x.saturating_add(rect.w) <= gx.saturating_add(gw),
                "{label}: past the playfield"
            );
            assert!(rect.y >= gy, "{label}: above the playfield");
            assert!(
                rect.y.saturating_add(rect.h) <= top_bar_bottom,
                "{label}: map crop y {}..{} crosses the scoreboard at {top_bar_bottom}",
                rect.y,
                rect.y + rect.h
            );
            let crop_box = (rect.x, rect.y, rect.w, rect.h);
            let names: Vec<(u32, u32, u32, u32)> = NAME_WINDOWS_5V5
                .iter()
                .chain([(NAME_COL_X_6V6, NAME_COL_W_6V6)].iter())
                .map(|&(x, nw)| name_area(w, h, x, nw))
                .collect();
            let name_top = names.iter().map(|n| n.1).min().unwrap();
            assert!(
                rect.y.saturating_add(rect.h) <= name_top,
                "{label}: map crop ends at {} and the name row starts at {name_top}",
                rect.y + rect.h
            );
            for area in &names {
                assert!(
                    !overlaps(crop_box, *area),
                    "{label}: reaches the name window {area:?}"
                );
            }

            let map = [10, 220, 30];
            let name = [240, 20, 20];
            let mut img = RgbImage::from_pixel(w, h, Rgb([0, 0, 0]));
            for y in rect.y..rect.y + rect.h {
                for x in rect.x..rect.x + rect.w {
                    img.put_pixel(x, y, Rgb(map));
                }
            }
            for &(nx, ny, nw, nh) in &names {
                for y in ny..ny.saturating_add(nh).min(h) {
                    for x in nx..nx.saturating_add(nw).min(w) {
                        img.put_pixel(x, y, Rgb(name));
                    }
                }
            }
            let crop = crop_map_label_box(&DynamicImage::ImageRgb8(img))
                .unwrap_or_else(|| panic!("{label}: box missing"))
                .to_rgb8();
            assert!(
                crop.width() <= rect.w && crop.height() <= rect.h,
                "{label}: crop {}x{} is outside the map-label box {}x{}",
                crop.width(),
                crop.height(),
                rect.w,
                rect.h
            );
            assert_eq!(
                (crop.width(), crop.height()),
                (rect.w, rect.h),
                "{label}: a solid label should keep the whole box"
            );
            assert!(
                crop.pixels().all(|px| px.0 == map),
                "{label}: crop left the map-label box or reached a name row"
            );
        }
    }
}

#[cfg(test)]
mod map_and_table_tests {
    use super::{crop_map_name, game_rect_16_9, header_label_groups, scoreboard_table_present};
    use image::{DynamicImage, Rgb, RgbImage};

    fn full_map_rect(w: u32, h: u32) -> (u32, u32, u32, u32) {
        let (gx, gy, gw, gh) = game_rect_16_9(w, h);
        let x = gx + (gw as f64 * 0.68) as u32;
        let y = gy + (gh as f64 * 0.022) as u32;
        let cw = (gw as f64 * 0.27) as u32;
        let ch = (gh as f64 * 0.040) as u32;
        (x, y, cw, ch)
    }

    #[test]
    fn map_name_crop_keeps_the_full_window_when_there_is_no_timer() {
        for (w, h) in [
            (2560u32, 1440u32),
            (1920, 1080),
            (3840, 2160),
            (2560, 1080),
            (3440, 1440),
        ] {
            let (x, y, cw, ch) = full_map_rect(w, h);
            let mut frame = RgbImage::from_pixel(w, h, Rgb([0, 0, 0]));
            frame.put_pixel(x, y, Rgb([255, 0, 0]));
            frame.put_pixel(x + cw - 1, y, Rgb([0, 255, 0]));
            let crop = crop_map_name(&DynamicImage::ImageRgb8(frame)).to_rgb8();
            assert_eq!(crop.dimensions(), (cw, ch), "{w}x{h}");
            assert_eq!(crop.get_pixel(0, 0).0, [255, 0, 0], "{w}x{h} left");
            assert_eq!(crop.get_pixel(cw - 1, 0).0, [0, 255, 0], "{w}x{h} right");
        }
        // Ultrawide uses the 16:9 game width, not the full frame.
        assert_eq!(game_rect_16_9(2560, 1080).2, 1920);
    }

    /// `WATCHPOINT: GRIMSVOTN` drawn into the old 10% trim zone, then a timer
    /// past a wide gap. Returns the frame, the crop x, the last name pixel,
    /// and the timer's first pixel.
    fn draw_long_name_and_timer(w: u32, h: u32) -> (RgbImage, u32, u32, u32) {
        let (x, y, cw, ch) = full_map_rect(w, h);
        let gh = game_rect_16_9(w, h).3;
        let mut img = RgbImage::from_pixel(w, h, Rgb([12, 14, 22]));
        let merge = ((gh as f64) * 0.012).round().max(3.0) as u32;
        let letter_gap = 2u32;
        let word_gap = (merge / 2).max(2);
        let timer_gap = merge + 4;
        let timer_w = 16u32;
        let text = "WATCHPOINT: GRIMSVOTN";
        let n = text.chars().filter(|c| *c != ' ').count() as u32;
        let gaps = (n - 2) * letter_gap + word_gap;
        let name_end = x + cw - timer_gap - timer_w;
        let max_total = name_end.saturating_sub(x + 4);
        let letter_w = max_total.saturating_sub(gaps) / n;
        assert!(letter_w >= 3, "{w}x{h} letter {letter_w}");
        let total = n * letter_w + gaps;
        let mut cursor = name_end - total;
        let y0 = y + ch / 5;
        let y1 = (y + ch * 4 / 5).min(y + ch);
        let mut last_name_x = cursor;
        let chars: Vec<char> = text.chars().collect();
        for (i, glyph) in chars.iter().copied().enumerate() {
            if glyph == ' ' {
                cursor += word_gap;
                continue;
            }
            for py in y0..y1 {
                for px in cursor..cursor + letter_w {
                    img.put_pixel(px, py, Rgb([236, 236, 236]));
                }
            }
            last_name_x = cursor + letter_w - 1;
            cursor += letter_w;
            if chars.get(i + 1).is_some_and(|next| *next != ' ') {
                cursor += letter_gap;
            }
        }
        let timer_x = last_name_x + 1 + timer_gap;
        assert!(timer_x + timer_w <= x + cw, "{w}x{h} timer past the window");
        let old_trim = (69.0_f64 * game_rect_16_9(w, h).2 as f64 / 2560.0).round() as u32;
        assert!(
            last_name_x >= x + cw - old_trim,
            "{w}x{h} name ends at {last_name_x}, old trim starts at {}",
            x + cw - old_trim
        );
        for py in y0..y1 {
            for px in timer_x..timer_x + timer_w {
                img.put_pixel(px.min(w - 1), py, Rgb([236, 236, 236]));
            }
        }
        (img, x, last_name_x, timer_x)
    }

    #[test]
    fn longest_map_name_fits_and_the_timer_is_cut() {
        for (w, h) in [(2560u32, 1440u32), (1920, 1080), (3840, 2160)] {
            let (frame, x, last_name_x, timer_x) = draw_long_name_and_timer(w, h);
            let crop = crop_map_name(&DynamicImage::ImageRgb8(frame));
            let right = x + crop.width();
            assert!(
                right > last_name_x,
                "{w}x{h} clipped WATCHPOINT: GRIMSVOTN at {right}, name ends {last_name_x}"
            );
            assert!(
                right <= timer_x,
                "{w}x{h} timer at {timer_x} still inside the crop ending {right}"
            );
        }
    }

    /// Synthetic Tab table. Rows are flat fills (any team colour) with a
    /// bright header and a dark gap where team 2 starts. No captured pixels.
    fn table_crop(team_a: [u8; 3], team_b: [u8; 3], rows: usize, pitch: f64) -> DynamicImage {
        let (w, h) = (800u32, 480u32);
        let mut img = RgbImage::from_pixel(w, h, Rgb([16, 18, 28]));
        let header_h = (h as f64 * 0.028) as u32;
        for y in 0..header_h {
            for x in 0..w {
                let label = x > w * 3 / 10 && (x / 14) % 6 == 0;
                let v = if label { 24 } else { 228 };
                img.put_pixel(x, y, Rgb([v, v, v]));
            }
        }
        let wf = w as f64;
        let hf = h as f64;
        for team in 0..2 {
            let colour = if team == 0 { team_a } else { team_b };
            let base = if team == 0 { 0.045 } else { 0.575 };
            for row in 0..rows {
                let y0 = ((base + row as f64 * pitch) * hf) as u32;
                let y1 = y0 + (pitch * 0.80 * hf) as u32;
                for y in y0..y1.min(h) {
                    for x in (0.05 * wf) as u32..(0.95 * wf) as u32 {
                        let n = ((x + y) % 7) as i16 - 3;
                        let ch = |c: u8| (c as i16 + n).clamp(0, 255) as u8;
                        img.put_pixel(x, y, Rgb([ch(colour[0]), ch(colour[1]), ch(colour[2])]));
                    }
                    let name_x0 = (0.18 * wf) as u32;
                    let name_x1 = (0.32 * wf) as u32;
                    for x in name_x0..name_x1 {
                        if (x + y) % 3 == 0 {
                            img.put_pixel(x, y, Rgb([240, 240, 240]));
                        }
                    }
                }
            }
        }
        DynamicImage::ImageRgb8(img)
    }

    /// Gameplay stand-in: a varied scene, a bright strip with six dark glyphs
    /// where the header-label counter looks, and a small HUD chip. Not a table.
    fn death_screen_crop() -> DynamicImage {
        let (w, h) = (800u32, 480u32);
        let mut img = RgbImage::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let v = (x.wrapping_mul(13).wrapping_add(y.wrapping_mul(29)) % 180) as u8;
                img.put_pixel(
                    x,
                    y,
                    Rgb([v.saturating_add(30), v / 2 + 20, 40 + (x % 90) as u8]),
                );
            }
        }
        let header_h = (h as f64 * 0.030) as u32;
        for y in 0..header_h {
            for x in 0..w {
                img.put_pixel(x, y, Rgb([210, 210, 214]));
            }
        }
        for i in 0..6 {
            let x0 = (w as f64 * (0.42 + i as f64 * 0.08)) as u32;
            let y0 = (h as f64 * 0.006) as u32;
            let y1 = (h as f64 * 0.024) as u32;
            for y in y0..y1 {
                for x in x0..x0 + 5 {
                    img.put_pixel(x.min(w - 1), y, Rgb([18, 18, 18]));
                }
            }
        }
        for y in h - 28..h {
            for x in 16..160 {
                img.put_pixel(x, y, Rgb([20, 180, 70]));
            }
        }
        DynamicImage::ImageRgb8(img)
    }

    #[test]
    fn recoloured_tables_count_and_a_death_screen_does_not() {
        let purple = [140, 60, 200];
        let yellow = [230, 200, 40];
        let cases = [
            ("blue/red 6", [40, 110, 220], [200, 50, 50], 6usize, 0.075),
            ("purple/yellow 6", purple, yellow, 6, 0.075),
            ("yellow/purple 6", yellow, purple, 6, 0.075),
            ("purple/yellow 5", purple, yellow, 5, 0.087),
            ("grey endorse", [150, 150, 150], [120, 120, 120], 6, 0.075),
        ];
        for (name, a, b, rows, pitch) in cases {
            assert!(
                scoreboard_table_present(&table_crop(a, b, rows, pitch)),
                "{name} should be a table"
            );
        }

        let flat = DynamicImage::ImageRgb8(RgbImage::from_pixel(800, 480, Rgb(purple)));
        assert!(
            !scoreboard_table_present(&flat),
            "a flat team colour with no header or gap is not a table"
        );

        let death = death_screen_crop();
        let groups = header_label_groups(&death).len();
        assert!(
            (3..=10).contains(&groups),
            "the stand-in header glyphs should fool the label counter, got {groups}"
        );
        assert!(
            !scoreboard_table_present(&death),
            "a death screen with killfeed glyphs and a HUD chip is not a table"
        );
    }
}

#[cfg(test)]
mod five_v_five_crop_tests {
    use super::{
        bottom_pad_clamped, crop_scoreboard, crop_scoreboard_for, crop_scoreboard_rect,
        crop_scoreboard_rect_for, row_slot_height,
    };
    use crate::detect::hero_portrait::portrait_rect;
    use image::{DynamicImage, Rgb, RgbImage};

    fn frame(w: u32, h: u32) -> DynamicImage {
        DynamicImage::ImageRgb8(RgbImage::new(w, h))
    }

    /// Paint row 9's portrait square into a full frame and require every
    /// pixel of it to land inside the 5v5 crop. The unpadded crop still
    /// refuses that square.
    fn bottom_portrait_is_inside(frame_w: u32, frame_h: u32) {
        let (x, y, cw, ch) = crop_scoreboard_rect_for(frame_w, frame_h, 5);
        let (ox, oy, ow, oh) = crop_scoreboard_rect(frame_w, frame_h);
        assert_eq!(
            (x, y, cw),
            (ox, oy, ow),
            "{frame_w}x{frame_h} top edge moved"
        );
        assert!(ch > oh, "{frame_w}x{frame_h} crop did not grow");
        assert!(y + ch <= frame_h, "{frame_w}x{frame_h} crop past the frame");

        let rect = portrait_rect((cw, ch), 9, 5).expect("row 9 fits the 5v5 crop");
        assert!(rect.y + rect.h <= ch);
        assert!(rect.y + rect.h > oh, "row 9 already fit the unpadded crop");
        assert!(portrait_rect((ow, oh), 9, 5).is_none());

        // Rows the unpadded crop already held stay on the same squares.
        // The top edge of the crop did not move, so neither do they.
        for row in [0usize, 4, 5, 8] {
            let tall = portrait_rect((cw, ch), row, 5).unwrap();
            let short = portrait_rect((ow, oh), row, 5).unwrap();
            assert_eq!(
                (tall.x, tall.y, tall.w, tall.h),
                (short.x, short.y, short.w, short.h),
                "row {row}"
            );
        }

        let mut painted = RgbImage::from_pixel(frame_w, frame_h, Rgb([0, 0, 0]));
        for py in rect.y..rect.y + rect.h {
            for px in rect.x..rect.x + rect.w {
                painted.put_pixel(x + px, y + py, Rgb([255, 0, 0]));
            }
        }
        let crop = crop_scoreboard_for(&DynamicImage::ImageRgb8(painted), 5).to_rgb8();
        assert_eq!((crop.width(), crop.height()), (cw, ch));
        for py in 0..rect.h {
            for px in 0..rect.w {
                assert_eq!(
                    crop.get_pixel(rect.x + px, rect.y + py).0,
                    [255, 0, 0],
                    "portrait pixel ({px},{py}) missing from the crop"
                );
            }
        }
    }

    #[test]
    fn synthetic_5v5_1440p_bottom_portrait_is_inside_the_crop() {
        bottom_portrait_is_inside(2560, 1440);
        assert_eq!(
            crop_scoreboard_rect_for(2560, 1440, 5),
            (448, 216, 1664, 1014)
        );
        assert_eq!(crop_scoreboard_rect(2560, 1440), (448, 216, 1664, 1007));
    }

    #[test]
    fn synthetic_5v5_1080p_bottom_portrait_is_inside_the_crop() {
        bottom_portrait_is_inside(1920, 1080);
        assert_eq!(
            crop_scoreboard_rect_for(1920, 1080, 5),
            (336, 162, 1248, 761)
        );
        assert_eq!(crop_scoreboard_rect(1920, 1080), (336, 162, 1248, 756));
    }

    #[test]
    fn six_v_six_crop_is_unchanged() {
        for (w, h, rect) in [
            (2560, 1440, (448, 216, 1664, 1007)),
            (1920, 1080, (336, 162, 1248, 756)),
        ] {
            assert_eq!(crop_scoreboard_rect(w, h), rect, "{w}x{h}");
            assert_eq!(crop_scoreboard_rect_for(w, h, 6), rect, "{w}x{h}");
            assert_eq!(crop_scoreboard_rect_for(w, h, 0), rect, "{w}x{h}");
            let img = frame(w, h);
            assert_eq!(crop_scoreboard(&img).height(), rect.3, "{w}x{h}");
            assert_eq!(crop_scoreboard_for(&img, 6).height(), rect.3, "{w}x{h}");
            // Last 6v6 portrait still fits the unpadded crop.
            assert!(portrait_rect((rect.2, rect.3), 11, 6).is_some(), "{w}x{h}");
        }
    }

    #[test]
    fn five_v_five_bottom_pad_clamps_at_the_frame_bottom() {
        // 7 px wanted at 1440p, 3 px of room under the crop. The pad stops
        // on the last frame row.
        let (frame_h, y, crop_h) = (1440u32, 1430u32, 7u32);
        let pad = bottom_pad_clamped(frame_h, y, crop_h);
        assert_eq!(pad, 3);
        assert_eq!(y + crop_h + pad, frame_h);

        // A 16:9 frame has room, and the crop still ends inside it.
        let (x, y, w, h) = crop_scoreboard_rect_for(2560, 1440, 5);
        assert_eq!((x, y, w, h), (448, 216, 1664, 1014));
        assert!(y + h <= 1440);
        let (x, y, w, h) = crop_scoreboard_rect_for(1920, 1080, 5);
        assert_eq!((x, y, w, h), (336, 162, 1248, 761));
        assert!(y + h <= 1080);
    }

    #[test]
    fn row_slots_stay_on_the_unpadded_height() {
        // crop_player_row divides by crop height. The 5v5 image is taller,
        // so stat rows keep the unpadded height (1007 at 1440p, 756 at 1080p).
        let native = frame(2560, 1440);
        let short = crop_scoreboard(&native);
        let tall = crop_scoreboard_for(&native, 5);
        assert_eq!(short.height(), 1007);
        assert_eq!(tall.height(), 1014);
        assert_eq!(row_slot_height(short.height(), 5), 90);
        assert_ne!(
            row_slot_height(tall.height(), 5),
            row_slot_height(short.height(), 5),
            "feeding the padded height into the row grid would move every row"
        );
        let small = frame(1920, 1080);
        assert_eq!(crop_scoreboard(&small).height(), 756);
        assert_eq!(
            row_slot_height(756, 5),
            row_slot_height(crop_scoreboard(&small).height(), 5)
        );
    }
}
