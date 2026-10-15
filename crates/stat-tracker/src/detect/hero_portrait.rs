use std::collections::HashMap;
use std::path::{Path, PathBuf};

use image::{DynamicImage, RgbImage, imageops::FilterType};

const PORTRAIT_SIZE: u32 = 32;

pub struct PortraitMatcher {
    references: HashMap<String, RgbImage>,
}

/// How [`load_portrait_pngs`] treats a directory.
#[derive(Clone, Copy)]
enum LoadMode {
    /// Every hero png. A later file with the same stem replaces the earlier one.
    Replace,
    /// A stem that is not loaded yet. A provisional stand-in is skipped.
    FillGaps,
}

impl PortraitMatcher {
    /// Load hero pngs from `dir` and nothing else.
    ///
    /// Tests and tools that already chose a directory use this. It does not
    /// unpack anything and it does not read a reader pack. Placeholder,
    /// skull, and empty icons are not heroes.
    pub fn load(dir: &Path) -> Self {
        let mut references = HashMap::new();
        load_portrait_pngs(&mut references, dir, LoadMode::Replace);
        tracing::info!(count = references.len(), "loaded hero portrait references");
        Self { references }
    }

    /// Load references from the hero packs installed under `data_dir`.
    ///
    /// Those are the same folders [`crate::shadow::heroes::HeroSet`] reads
    /// after pack sync: `templates/heroes` (heroes-v1), `templates/heroes-v2`,
    /// and `templates/heroes-v2-1080`. The 1440p set is loaded first. The
    /// 1080p set fills stems it does not already have. A crop saved under
    /// [`portraits_dir`] fills a stem the pack does not have, and only when
    /// a pack loaded.
    ///
    /// No pack means an empty matcher. [`Self::match_portrait`] then returns
    /// no hero. Files left in [`portraits_dir`] by an older build are not a
    /// substitute, so a missing pack is never a guess.
    pub fn load_installed(data_dir: &Path) -> Self {
        let local = portraits_dir(data_dir);
        if let Err(e) = invalidate_stale_seeds(&local) {
            tracing::warn!(error = %e, "could not move stale portrait seeds aside; will retry next start");
        }
        let Some(set) = crate::shadow::heroes::HeroSet::load_data_dir(data_dir) else {
            tracing::info!("no hero pack installed; portrait matcher will not name a hero");
            return Self {
                references: HashMap::new(),
            };
        };
        let mut references = HashMap::new();
        let mut dirs = Vec::new();
        if let Some(pack) = set.main_pack() {
            dirs.push(hero_pack_dir(data_dir, pack));
        }
        if let Some(pack) = set.pack_1080() {
            let dir = hero_pack_dir(data_dir, pack);
            if dirs.iter().all(|have| have != &dir) {
                dirs.push(dir);
            }
        }
        for dir in &dirs {
            let mode = if references.is_empty() {
                LoadMode::Replace
            } else {
                LoadMode::FillGaps
            };
            load_portrait_pngs(&mut references, dir, mode);
        }
        if !references.is_empty() {
            load_portrait_pngs(&mut references, &local, LoadMode::FillGaps);
        }
        tracing::info!(
            count = references.len(),
            "loaded hero portrait references from the reader pack"
        );
        Self { references }
    }

    pub fn is_empty(&self) -> bool {
        self.references.is_empty()
    }

    #[cfg(test)]
    fn contains(&self, stem: &str) -> bool {
        self.references.contains_key(stem)
    }

    #[cfg(test)]
    fn from_references(references: HashMap<String, RgbImage>) -> Self {
        Self { references }
    }

    pub fn match_portrait(&self, crop: &DynamicImage) -> Option<(String, f64)> {
        // No references (no hero pack, or none of its pngs loaded): name
        // nothing. The closest of an empty set is not a hero.
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

        // Same floor as a portrait switch. A live Tab guessed Sierra or Tracer
        // at 0.77, and that guess overrode the new reader. Below 0.85 is not a hero.
        if !portrait_confidence_is_trusted(confidence) {
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

/// The player's row: the row of a trusted portrait match, else the
/// highlighted row on its own. A low portrait score (an ult ring over the
/// portrait reads 0.77) must not drop the row, or every later Tab is
/// "player row not found" and the slow full-board read runs each time.
pub fn resolve_player_row(
    portrait_match: Option<usize>,
    scoreboard: &DynamicImage,
    team_size: usize,
) -> Option<usize> {
    portrait_match.or_else(|| detect_player_row_inner(scoreboard, team_size))
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
    highlighted_row(&team1_row_brightness(scoreboard, team_size))
}

/// The highlighted row ([`resolve_player_row`]'s own rule) and how far its
/// background leads the next brightest team 1 row's (`brightest / next`).
/// The own-row seeding gate ([`crate::detect::own_row`]) needs the lead: a
/// cursor hover or a party tint lights a second row. A board with one
/// measurable row has no lead (`f64::INFINITY`).
pub fn player_row_highlight(scoreboard: &DynamicImage, team_size: usize) -> Option<(usize, f64)> {
    let brightnesses = team1_row_brightness(scoreboard, team_size);
    let row = highlighted_row(&brightnesses)?;
    let top = brightnesses.iter().find(|(r, _)| *r == row)?.1;
    let next = brightnesses
        .iter()
        .filter(|(r, _)| *r != row)
        .map(|(_, b)| *b)
        .fold(f64::NEG_INFINITY, f64::max);
    let lead = if next > 0.0 {
        top / next
    } else {
        f64::INFINITY
    };
    Some((row, lead))
}

/// Mean background brightness of each team 1 row that has enough background
/// pixels to measure, as `(row, brightness)`.
fn team1_row_brightness(scoreboard: &DynamicImage, team_size: usize) -> Vec<(usize, f64)> {
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
    brightnesses
}

/// The row whose background is more than 12% brighter than the rows' mean.
fn highlighted_row(brightnesses: &[(usize, f64)]) -> Option<usize> {
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

/// 2560x1440 scoreboard crop width (`crop_scoreboard` on a 16:9 frame).
///
/// The row grid below was measured on that crop (1664x1007). Crop width
/// scales with the frame (1664 at 1440p, 1248 at 1080p). Crop height does
/// not: `1440 * 0.70` truncates to 1007, while `1080 * 0.70` is 756. A
/// height pitch (`h * 58 / 1000`, then times the row index) drops the
/// leftover fraction on every row, so the last 1080p portrait sits 5px
/// (5v5) or 6px (6v6) above the same row on the 1440p grid. Scaling the
/// reference rows by width keeps 1440p pixel-identical and lands 1080p on
/// that grid.
const PORTRAIT_REF_W: u32 = 1664;
/// 6v6: `1007 * 12 / 100` on the 1440p crop.
const PORTRAIT_REF_START_Y: u32 = 120;
/// 6v6: `1007 * 58 / 1000`.
const PORTRAIT_REF_PITCH_6: u32 = 58;

/// 5v5 row 0 top on the 1440p crop. Measured 2026-10-12 on 15 in-match
/// boards (0, 1 and 2 perks) and one centred pre-match board: row 0 starts at
/// y 34.1, and the layouts agree within 0.1 px. The old grid (y 120, pitch
/// 70) put "row 0" on row 1's portrait.
const PORTRAIT_REF_START_Y_5: u32 = 34;
/// 5v5 row pitch, measured 88.07 (team 1) and 88.06 (team 2).
const PORTRAIT_REF_PITCH_5: u32 = 88;
/// 5v5 team 2 row 0 top, measured 576.0 to 576.6. The team gap is not a
/// whole pitch (576 - 34 = 6.15 pitches), so team 2 gets its own origin.
const PORTRAIT_REF_TEAM2_Y_5: u32 = 576;
/// 5v5 portrait square, measured 85.5 px wide. The shared `w * 6 / 100`
/// square (99 px) would run 13 px into the next row at the 88 px pitch.
const PORTRAIT_REF_SIDE_5: u32 = 86;

fn scale_portrait_px(ref_px: u32, board_w: u32) -> u32 {
    let r = PORTRAIT_REF_W as u64;
    (((ref_px as u64 * board_w as u64) + r / 2) / r) as u32
}

/// Shared portrait geometry for 5v5 / 6v6 scoreboard crops.
///
/// OW2 Tab scoreboard layout (after `crop_scoreboard`):
/// - 5 or 6 player rows per team
/// - Hero portrait at the left edge of each row
/// - 6v6: rows start ~12% from top, portrait ~6% of width, square, and a
///   one-row team gap between team 1 and team 2
/// - 5v5: the measured grid above (row 0 at y 34, pitch 88, team 2 at 576,
///   86 px square on the 1440p crop)
///
/// The x origin is shared. It fits the 5v5 in-match table with no perks; the
/// table shifts left by 28 px (1 perk) or 66 px (2 perks), which this grid
/// does not follow.
///
/// Returns None unless the full square fits in the crop. A cut-off
/// rectangle can be saved as a hero reference, so a short board is
/// refused instead of clipped.
/// Stat and digit cells use `crop_player_row`, not this grid.
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

    let side = if team_size == 6 {
        w * 6 / 100
    } else {
        scale_portrait_px(PORTRAIT_REF_SIDE_5, w)
    };
    let portrait_x = w / 100;
    if side == 0 || portrait_x.saturating_add(side) > w {
        return None;
    }

    let team = (row_idx / team_size) as u32;
    let row_in_team = (row_idx % team_size) as u32;
    let ref_y = if team_size == 6 {
        // 6v6: ~5.8% of the 1440p crop per row; the team gap is one extra
        // row of that pitch.
        let steps = if team == 0 {
            row_in_team
        } else {
            team_size as u32 + 1 + row_in_team
        };
        PORTRAIT_REF_START_Y + steps * PORTRAIT_REF_PITCH_6
    } else {
        let origin = if team == 0 {
            PORTRAIT_REF_START_Y_5
        } else {
            PORTRAIT_REF_TEAM2_Y_5
        };
        origin + row_in_team * PORTRAIT_REF_PITCH_5
    };
    let y = scale_portrait_px(ref_y, w);
    if y.saturating_add(side) > h {
        return None;
    }

    Some(PortraitRect {
        x: portrait_x,
        y,
        w: side,
        h: side,
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
    /// 5-6 rows of white text in the team-1 band at a known pitch (about 7.5%
    /// to 7.9% for 6v6, about 8.3% to 9.0% for 5v5), so at least three dips at a plausible pitch
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
    /// measurable. See [`RowScan::checked_team_size`]. When that returns
    /// `None` (the pitches contradict, or a pitch sits in the no-guess
    /// band), this asks the same check for the spectral pitch, else the
    /// dip pitch, so offline callers still get a size. A pitch that check
    /// will not classify leaves the documented default of 5. The capture
    /// path rejects that frame instead.
    pub fn team_size(&self) -> usize {
        if let Some(size) = self.checked_team_size() {
            return size;
        }
        let Some(pitch) = self.spectral_pitch.or(self.median_pitch) else {
            return 5;
        };
        Self {
            dip_count: self.dip_count,
            median_pitch: None,
            spectral_pitch: Some(pitch),
        }
        .checked_team_size()
        .unwrap_or(5)
    }

    /// 5v5 or 6v6 only when the row pitch says so without contradiction
    /// and without landing in the no-guess band.
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
    /// The 0.080/0.083 band is provisional, pending native 1440p 5v5
    /// boards. Highest real 6v6 so far: dip 0.07937 (four 1080p boards)
    /// and spectral 0.07672. At 1440p the 6v6 dip runs 0.0725 to 0.0775.
    /// Weakest real 5v5 PC board so far: spectral 0.0847, dip 0.0866.
    /// `wiki_scoreboard.png` at 414p measured 0.08304 on both pitches.
    ///
    /// - No pitch measured at all: 5, the documented default, as before.
    /// - A plausible pitch strictly below [`TEAM_SIZE_6V6_PITCH_MAX`] is 6.
    /// - A plausible pitch strictly above [`TEAM_SIZE_5V5_PITCH_MIN`] is 5.
    /// - A plausible pitch on the closed interval between those two
    ///   constants is not a size. If either measured pitch lands there,
    ///   the size is unknown.
    /// - Both pitches plausible and outside the band: they must agree, or
    ///   the size is unknown.
    /// - One pitch outside the band, the other missing or implausible:
    ///   that one decides.
    /// - Pitches measured but none plausible: unknown.
    pub fn checked_team_size(&self) -> Option<usize> {
        if self.spectral_pitch.is_none() && self.median_pitch.is_none() {
            return Some(5);
        }
        // `Some(None)` is a plausible pitch inside the no-guess band.
        // `None` is missing or outside [`ROW_PITCH_PLAUSIBLE`].
        let vote = |p: Option<f64>| -> Option<Option<usize>> {
            let p = p.filter(|p| ROW_PITCH_PLAUSIBLE.contains(p))?;
            let side = if p < TEAM_SIZE_6V6_PITCH_MAX {
                Some(6)
            } else if p > TEAM_SIZE_5V5_PITCH_MIN {
                Some(5)
            } else {
                None
            };
            Some(side)
        };
        match (vote(self.spectral_pitch), vote(self.median_pitch)) {
            (Some(Some(a)), Some(Some(b))) if a == b => Some(a),
            (Some(Some(_)), Some(Some(_))) => None,
            (Some(None), _) | (_, Some(None)) => None,
            (Some(Some(a)), None) | (None, Some(Some(a))) => Some(a),
            (None, None) => None,
        }
    }
}

/// Row pitch strictly below this (fraction of crop height) is 6v6.
///
/// Used only by [`RowScan::checked_team_size`]. The 0.080/0.083 band is
/// provisional, pending native 1440p 5v5 boards. The closed interval from
/// here through [`TEAM_SIZE_5V5_PITCH_MIN`] is a no-guess band.
const TEAM_SIZE_6V6_PITCH_MAX: f64 = 0.080;

/// Row pitch strictly above this (fraction of crop height) is 5v5.
///
/// Used only by [`RowScan::checked_team_size`]. See
/// [`TEAM_SIZE_6V6_PITCH_MAX`].
const TEAM_SIZE_5V5_PITCH_MIN: f64 = 0.083;

/// Row pitches that can be a real 6v6 (about 0.0725 to 0.0794) or 5v5
/// (about 0.083 to 0.090) scoreboard, with room on each side. 0.101 and
/// 0.102, the two misleading pitches measured so far, fall outside.
const ROW_PITCH_PLAUSIBLE: std::ops::RangeInclusive<f64> = 0.062..=0.095;

/// What the capture preflight decided, before any OCR.
///
/// A frame the row dips or the header labels accept, and whose pitch settles,
/// is a board. The table check does not override that. It runs only when the
/// pitch fails or the two pitches disagree, and then only picks the log line:
/// no table is [`ScoreboardPreflight::NotAScoreboard`], a table is
/// [`ScoreboardPreflight::TeamSizeUncertain`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScoreboardPreflight {
    /// Neither row dips nor header labels, or the pitch failed and there is
    /// no table.
    NotAScoreboard,
    /// Row dips or header labels are present, the pitch does not settle 5v5
    /// or 6v6, and a table is present.
    TeamSizeUncertain,
    /// Row pitch settled on this team size (5 or 6).
    Ready(usize),
}

/// Same accept path as before the table detector: row dips or header labels,
/// then a settled pitch. `table_present` (from
/// [`crate::ocr::preprocess::scoreboard_table_present`], layout and brightness
/// only) is consulted only when [`RowScan::checked_team_size`] is `None`.
/// `header_labels` is the count from
/// [`crate::ocr::preprocess::header_label_groups`].
pub fn classify_scoreboard_preflight(
    scan: &RowScan,
    header_labels: usize,
    table_present: bool,
) -> ScoreboardPreflight {
    let rows = scan.looks_like_scoreboard();
    let header = (3..=10).contains(&header_labels);
    if !rows && !header {
        return ScoreboardPreflight::NotAScoreboard;
    }
    match scan.checked_team_size() {
        Some(n) => ScoreboardPreflight::Ready(n),
        None if table_present => ScoreboardPreflight::TeamSizeUncertain,
        None => ScoreboardPreflight::NotAScoreboard,
    }
}

/// Run the capture preflight on one scoreboard crop.
pub fn preflight_scoreboard(board: &DynamicImage) -> (ScoreboardPreflight, RowScan) {
    let scan = scan_rows(board);
    let labels = crate::ocr::preprocess::header_label_groups(board).len();
    let table = crate::ocr::preprocess::scoreboard_table_present(board);
    (classify_scoreboard_preflight(&scan, labels, table), scan)
}

/// Detect whether the scoreboard shows 5v5 or 6v6 via the team-1 row pitch.
///
/// Counting hero portraits fails when rows are empty ("WAITING FOR PLAYER") or
/// when custom team colors make the colored row bars contiguous with no dark
/// gaps. Instead we measure saturation across the name strip per scanline: every
/// row — occupied or empty — has white text that dips saturation once per row,
/// so the dip-to-dip pitch reveals the row count regardless of empty slots or
/// team colors. 6v6 rows are tighter (about 7.5% to 7.9% of crop height)
/// than 5v5 (about 8.3% to 9.0%). [`RowScan::checked_team_size`] returns
/// no size for the no-guess band between those layouts. This returns the
/// offline size from [`RowScan::team_size`].
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
/// 6v6 (about 7.5% to 7.9%) to 5v5 (about 8.3% to 9.0%) row pitches with
/// margin. The peak must be interior: a maximum at the range edge means the
/// spectrum is just decaying (no row periodicity), and it must be strong
/// enough relative to band variance.
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

/// At 0.85 the portrait is trusted. Just below that it is not.
fn portrait_confidence_is_trusted(confidence: f64) -> bool {
    confidence >= crate::hero_auth::PORTRAIT_SWITCH_MIN_CONF
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

/// Where a captured reference for `hero_name` lives: `<portrait_stem>.png`
/// under the local portraits directory. The matcher keys a pack file by
/// its own stem (`ana`, `wrecking-ball`).
pub fn portrait_reference_path(portraits_dir: &Path, hero_name: &str) -> PathBuf {
    portraits_dir.join(format!("{}.png", portrait_stem(hero_name)))
}

/// File stem for `hero_name`: lowercase ASCII, accents folded, punctuation
/// dropped, a run of spaces or underscores between two words becomes one `_`.
/// A space next to punctuation goes with it.
///
/// "D.Va" -> `dva`, "Soldier: 76" -> `soldier76`, "Torbjörn" -> `torbjorn`,
/// "Lúcio" -> `lucio`, "Wrecking Ball" -> `wrecking_ball`. A stem maps to
/// itself, so `portrait_stem(stem) != stem` marks a file no hero name can
/// produce. A letter with no ASCII fold is kept lowercased, so a name never
/// maps to an empty stem.
pub fn portrait_stem(hero_name: &str) -> String {
    let mut stem = String::with_capacity(hero_name.len());
    let mut space = false;
    let mut punct = false;
    for c in hero_name.chars().flat_map(char::to_lowercase) {
        if c.is_whitespace() || c == '_' {
            space = true;
            continue;
        }
        let folded = ascii_fold(c);
        if folded.is_empty() {
            punct = true;
            continue;
        }
        if space && !punct && !stem.is_empty() {
            stem.push('_');
        }
        space = false;
        punct = false;
        stem.push_str(&folded);
    }
    stem
}

/// `c` (already lowercase) as ASCII: itself when ASCII alphanumeric, its base
/// letter(s) when a folded Latin letter, itself when any other letter or
/// digit, and empty for punctuation and symbols.
fn ascii_fold(c: char) -> String {
    let base = match c {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' => "a",
        'æ' => "ae",
        'ç' | 'ć' | 'č' => "c",
        'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ę' => "e",
        'ì' | 'í' | 'î' | 'ï' | 'ī' => "i",
        'ł' => "l",
        'ñ' | 'ń' => "n",
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' => "o",
        'œ' => "oe",
        'ś' | 'š' => "s",
        'ß' => "ss",
        'ù' | 'ú' | 'û' | 'ü' | 'ū' => "u",
        'ý' | 'ÿ' => "y",
        'ź' | 'ż' | 'ž' => "z",
        'þ' => "th",
        'ð' => "d",
        _ if c.is_alphanumeric() => return c.to_string(),
        _ => "",
    };
    base.to_string()
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
/// the row was not found), a portrait grid known to fit the board layout
/// ([`portrait_seed_layout_known`]), a slot that is empty or still a
/// provisional stand-in, and either the career panel or `--collect-portraits`.
pub fn should_save_portrait_crop(
    career_panel: bool,
    collect_portraits: bool,
    slot_replaceable: bool,
    player_row_known: bool,
    layout_known: bool,
) -> bool {
    player_row_known && layout_known && slot_replaceable && (collect_portraits || career_panel)
}

/// Whether [`portrait_rect`] is known to fit this board, so its crop may seed
/// a hero reference.
///
/// 6v6 has one measured layout: always. 5v5 only when the own name matched in
/// the in-match no-perk window
/// ([`crate::ocr::preprocess::NAME_WINDOW_5V5_INGAME`]) and in no other
/// window. The 5v5 grid's x fits only that layout: one or two perks shift the
/// table 28 or 66 px left, so the square is cut or misplaced, and the centred
/// table puts the portrait near x 323. A saved crop is never overwritten, so a
/// misplaced one would stick. A row found only by the highlight says nothing
/// about the layout and never seeds on 5v5.
///
/// Name windows 0 (no perks, x 0.10-0.25) and 1 (perks, x 0.095-0.20)
/// overlap, so a short name on a no-perk board matches both, `sole_window` is
/// false, and that board never seeds. This fails safe: no seed is written and
/// the career panel stays the hero authority. The one wrong-seed path left is
/// a perk board where window 1's read fails and window 0's succeeds.
pub fn portrait_seed_layout_known(
    team_size: usize,
    name_match: Option<&crate::parse::NameMatch>,
) -> bool {
    if team_size >= 6 {
        return true;
    }
    name_match.is_some_and(|m| {
        m.window == crate::ocr::preprocess::NAME_WINDOW_5V5_INGAME && m.sole_window
    })
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
/// Doctrine's old stand-in. The bytes are not shipped. The digest stays so
/// an install that still has that file treats the slot as empty and a
/// capture can replace it.
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

/// Version of the portrait grid that captured seeds are cut with.
///
/// - 1: alpha.5 and earlier. The 5v5 grid put "row 0" at y 120 (row 1's
///   portrait), so a 5v5 seed could hold a teammate's hero.
/// - 2: the measured 5v5 grid, and 5v5 seeds only from the no-perk in-match
///   layout ([`portrait_seed_layout_known`]).
///
/// Bump it whenever [`portrait_rect`] moves for a layout that may seed.
pub const SEED_GEOMETRY_VERSION: u32 = 2;

/// Marker in the portraits directory holding the [`SEED_GEOMETRY_VERSION`]
/// its captured seeds were cut with. Absent means version 1.
const SEED_GEOMETRY_MARKER: &str = ".seed-geometry";

/// Move portrait files the matcher must no longer load out of the reference
/// set. Nothing is deleted. Returns how many files moved.
///
/// - Once per [`SEED_GEOMETRY_VERSION`], while the marker is older: every
///   `*.png` that is not a provisional stand-in moves to
///   `invalidated-<old version>/`. Nothing is shipped in the binary to
///   compare against, so an older unpacked reference moves with the seeds.
///   The next qualifying capture can seed the slot again.
/// - On every call: a `*.png` whose stem is not a [`portrait_stem`] (alpha.5
///   wrote `soldier:_76.png` and `torbjörn.png`) moves to
///   `invalidated-<marker version>/`. No hero name maps to it any more.
///
/// A png that cannot be read is skipped with one WARN and stays where it is.
/// It does not stop the other moves or the marker. The marker is written
/// once every move succeeded.
pub fn invalidate_stale_seeds(portraits_dir: &Path) -> std::io::Result<usize> {
    invalidate_stale_seeds_with(portraits_dir, &[])
}

fn invalidate_stale_seeds_with(
    portraits_dir: &Path,
    bundled: &[(&str, &[u8])],
) -> std::io::Result<usize> {
    invalidate_stale_seeds_reading(portraits_dir, bundled, |p| std::fs::read(p))
}

fn invalidate_stale_seeds_reading(
    portraits_dir: &Path,
    bundled: &[(&str, &[u8])],
    read: impl Fn(&Path) -> std::io::Result<Vec<u8>>,
) -> std::io::Result<usize> {
    std::fs::create_dir_all(portraits_dir)?;
    let marker = portraits_dir.join(SEED_GEOMETRY_MARKER);
    let on_disk: u32 = std::fs::read_to_string(&marker)
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(1);
    let stale_grid = on_disk < SEED_GEOMETRY_VERSION;

    let dest = portraits_dir.join(format!("invalidated-{on_disk}"));
    let mut moved = 0usize;
    for entry in std::fs::read_dir(portraits_dir)? {
        let path = entry?.path();
        if !path.is_file() || path.extension().and_then(|e| e.to_str()) != Some("png") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        if stem.starts_with('.') {
            continue;
        }
        let bad_stem = portrait_stem(stem) != stem;
        if !bad_stem {
            if !stale_grid {
                continue;
            }
            let bytes = match read(&path) {
                Ok(bytes) => bytes,
                Err(e) => {
                    tracing::warn!(
                        path = %path.display(),
                        error = %e,
                        "could not read portrait file during the seed migration; left in place"
                    );
                    continue;
                }
            };
            let is_bundled = bundled
                .iter()
                .any(|(name, data)| *name == stem && *data == bytes.as_slice());
            if is_bundled || bytes_match_provisional(stem, &bytes) {
                continue;
            }
        }
        std::fs::create_dir_all(&dest)?;
        let Some(file_name) = path.file_name() else {
            continue;
        };
        let mut target = dest.join(file_name);
        let mut n = 1;
        while target.exists() {
            target = dest.join(format!("{stem}.{n}.png"));
            n += 1;
        }
        std::fs::rename(&path, &target)?;
        moved += 1;
    }

    if stale_grid {
        write_bytes_atomic(&marker, format!("{SEED_GEOMETRY_VERSION}\n").as_bytes())?;
    }
    if moved > 0 {
        tracing::info!(
            moved,
            from_version = on_disk,
            to_version = SEED_GEOMETRY_VERSION,
            dir = %dest.display(),
            "moved portrait seeds cut with an older portrait grid or under a hero-name stem no build writes aside; they re-seed from new captures"
        );
    }
    Ok(moved)
}

fn hero_pack_dir(data_dir: &Path, pack: crate::shadow::heroes::HeroPack) -> PathBuf {
    use crate::shadow::heroes::{
        HeroPack, HeroTemplates, TEMPLATE_SUBDIR_V2, TEMPLATE_SUBDIR_V2_1080,
    };
    match pack {
        HeroPack::V1 => HeroTemplates::dir_in(data_dir),
        HeroPack::V2 => data_dir.join(TEMPLATE_SUBDIR_V2),
        HeroPack::V2At1080 => data_dir.join(TEMPLATE_SUBDIR_V2_1080),
    }
}

/// Read top-level `<stem>.png` files into `references`. `special/` is not
/// walked: those icons are placeholders, skulls, and empty slots.
fn load_portrait_pngs(references: &mut HashMap<String, RgbImage>, dir: &Path, mode: LoadMode) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("png") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        if stem.starts_with('.') || scuffed_types::is_placeholder_hero(stem) {
            continue;
        }
        if matches!(mode, LoadMode::FillGaps)
            && (references.contains_key(stem) || portrait_slot_is_replaceable(&path))
        {
            continue;
        }
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

    /// `crop_scoreboard` on a synthetic 16:9 frame. No game pixels: only the
    /// rectangle the crop and the portrait grid agree on.
    fn board_dims(frame_w: u32, frame_h: u32) -> (u32, u32) {
        let img = image::DynamicImage::ImageRgb8(image::RgbImage::new(frame_w, frame_h));
        let board = crate::ocr::preprocess::crop_scoreboard(&img);
        (board.width(), board.height())
    }

    #[test]
    fn last_1080_portrait_matches_1440_scaled_by_width() {
        // 2560x1440 -> 1664x1007. 1920x1080 -> 1248x756. Height is not 0.75x
        // (1007 * 3/4 = 755.25) because `1440 * 0.70` truncates to 1007.
        let d1440 = board_dims(2560, 1440);
        let d1080 = board_dims(1920, 1080);
        assert_eq!(d1440, (1664, 1007));
        assert_eq!(d1080, (1248, 756));

        // Reference grid, pinned so a 1080 fix cannot slide 1440.
        assert_eq!(portrait_rect(d1440, 0, 6).unwrap().y, 120);
        assert_eq!(portrait_rect(d1440, 11, 6).unwrap().y, 816);
        assert_eq!(portrait_rect(d1440, 0, 5).unwrap().y, 34);
        assert_eq!(portrait_rect(d1440, 5, 5).unwrap().y, 576);
        assert_eq!(portrait_rect(d1440, 8, 5).unwrap().y, 840);

        // Truncating the height pitch left this 6px high.
        assert_eq!(portrait_rect(d1080, 11, 6).unwrap().y, 612);
        assert_eq!(portrait_rect(d1080, 8, 5).unwrap().y, 630);

        // The 5v5 last row runs to y 1014.85 and the crop stops at 1007, so
        // its square is cut at both resolutions and refused.
        assert_eq!(portrait_rect(d1440, 9, 5), None);
        assert_eq!(portrait_rect(d1080, 9, 5), None);

        for (team, rows, full_1440, full_1080) in [
            (6usize, 12usize, d1440.0 * 6 / 100, d1080.0 * 6 / 100),
            (5, 9, 86, 65),
        ] {
            for row in 0..rows {
                let src = portrait_rect(d1440, row, team).unwrap();
                let dst = portrait_rect(d1080, row, team).unwrap();
                let expect_y = ((src.y as u64 * d1080.0 as u64) + 1664 / 2) / 1664;
                assert_eq!(
                    dst.y, expect_y as u32,
                    "team {team} row {row} must scale with board width"
                );
                assert_eq!(
                    (src.w, src.h),
                    (full_1440, full_1440),
                    "team {team} row {row} 1440 portrait is the full square"
                );
                assert_eq!(
                    (dst.w, dst.h),
                    (full_1080, full_1080),
                    "team {team} row {row} 1080 portrait is the full square"
                );
                assert!(src.y + src.h <= d1440.1);
                assert!(dst.y + dst.h <= d1080.1);
            }
        }
    }

    /// 5v5 row bands on the 1440p crop, measured 2026-10-12 on 15 in-match
    /// boards and one centred board (row top, exclusive bottom). Hardcoded on
    /// purpose: deriving them from the constants would test nothing.
    const ROWS_5V5_1440: [(f64, f64); 10] = [
        (34.1, 119.9),
        (122.2, 208.0),
        (210.3, 296.1),
        (298.3, 384.1),
        (386.4, 472.9),
        (576.0, 661.7),
        (664.1, 749.8),
        (752.1, 837.9),
        (840.2, 925.9),
        (928.3, 1014.3),
    ];

    #[test]
    fn five_v_five_portraits_sit_inside_the_measured_rows() {
        let dims = board_dims(2560, 1440);
        for (row, (top, bottom)) in ROWS_5V5_1440.iter().enumerate() {
            let Some(r) = portrait_rect(dims, row, 5) else {
                assert!(*bottom > dims.1 as f64, "row {row} fits but has no square");
                continue;
            };
            let (y0, y1) = (r.y as f64, (r.y + r.h) as f64);
            assert!(
                (y0 - top).abs() <= 2.0,
                "row {row}: square starts at {y0}, the row at {top}"
            );
            assert!(
                y1 <= bottom + 1.0,
                "row {row}: square ends at {y1}, past the row bottom {bottom}"
            );
        }
    }

    #[test]
    fn short_board_returns_none() {
        // 1440-wide crop. 6v6 row 0 starts at y=120 and the square is 99px,
        // so anything shorter than 219px would cut it. That used to come
        // back as 99x30. Callers save this rect as a hero reference.
        assert!(portrait_rect((1664, 150), 0, 6).is_none());
        assert!(portrait_rect((1664, 218), 0, 6).is_none());
        assert!(portrait_rect((1664, 219), 0, 6).is_some());
        // 5v5 row 0 starts at y=34 with an 86px square: 120px is the floor.
        assert!(portrait_rect((1664, 100), 0, 5).is_none());
        assert!(portrait_rect((1664, 119), 0, 5).is_none());
        assert!(portrait_rect((1664, 120), 0, 5).is_some());
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

    /// alpha.5 kept ':' and 'ö', so Soldier: 76 and Torbjörn seeded a second
    /// reference beside the bundled `soldier76` and `torbjorn` (#240).
    #[test]
    fn stem_folds_accents_and_drops_punctuation() {
        let dir = Path::new("/tmp/portraits");
        for (hero, stem) in [
            ("Soldier: 76", "soldier76"),
            ("Torbjörn", "torbjorn"),
            ("D.Va", "dva"),
            ("Lúcio", "lucio"),
            ("Jetpack Cat", "jetpack_cat"),
            ("Junker  Queen", "junker_queen"),
        ] {
            assert_eq!(super::portrait_stem(hero), stem, "{hero}");
            assert_eq!(
                portrait_reference_path(dir, hero),
                dir.join(format!("{stem}.png")),
                "{hero}"
            );
        }
    }

    /// Pack file stems stay the matcher's key. `portrait_stem` folds a
    /// display name; it is not applied to `wrecking-ball`.
    #[test]
    fn pack_file_stems_are_not_rewritten() {
        assert_eq!(super::portrait_stem("Wrecking Ball"), "wrecking_ball");
        assert_ne!(super::portrait_stem("wrecking-ball"), "wrecking-ball");
        assert_eq!(
            scuffed_types::canonical_hero("wrecking-ball"),
            "Wrecking Ball"
        );
    }
}

#[cfg(test)]
mod provisional_portrait_tests {
    use super::{
        digest_matches_provisional, portrait_slot_is_replaceable, save_captured_portrait,
        should_save_portrait_crop,
    };
    use image::{DynamicImage, Rgba, RgbaImage};

    fn solid(color: Rgba<u8>) -> DynamicImage {
        DynamicImage::ImageRgba8(RgbaImage::from_pixel(8, 8, color))
    }

    /// The digest of the old Doctrine stand-in stays listed. Dropping it
    /// would freeze that file on disk as if it were a real crop.
    #[test]
    fn doctrine_stand_in_digest_stays_listed() {
        assert!(super::PROVISIONAL_PORTRAITS.iter().any(|(name, hash)| {
            *name == "doctrine"
                && *hash == "6dd3449514c8b0a657c5ca07f2f89e97820ce76a608684029e36300cc7a862cb"
        }));
    }

    #[test]
    fn a_missing_slot_is_replaced_and_a_user_file_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        assert!(portrait_slot_is_replaceable(
            &dir.path().join("doctrine.png")
        ));
        let saved =
            save_captured_portrait(dir.path(), "Doctrine", &solid(Rgba([0, 255, 0, 255]))).unwrap();
        assert!(saved.is_some());
        let path = dir.path().join("doctrine.png");
        let before = std::fs::read(&path).unwrap();
        assert!(!portrait_slot_is_replaceable(&path));
        let again =
            save_captured_portrait(dir.path(), "Doctrine", &solid(Rgba([0, 0, 255, 255]))).unwrap();
        assert!(again.is_none());
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }

    #[test]
    fn capture_leaves_a_non_provisional_file() {
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
        assert!(should_save_portrait_crop(true, false, true, true, true));
        // Row not identified: do not crop row 0, even with --collect-portraits.
        assert!(!should_save_portrait_crop(true, true, true, false, true));
        // Portrait/held/text guess, collection off.
        assert!(!should_save_portrait_crop(false, false, true, true, true));
        // Layout not known to fit the portrait grid (5v5 perks / centred).
        assert!(!should_save_portrait_crop(true, true, true, true, false));

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("doctrine.png");
        solid(Rgba([255, 0, 0, 255]))
            .save(&path)
            .expect("write real portrait");
        let replaceable = portrait_slot_is_replaceable(&path);
        assert!(!replaceable);
        assert!(!should_save_portrait_crop(
            true,
            true,
            replaceable,
            true,
            true
        ));
    }
}

#[cfg(test)]
mod seed_layout_tests {
    use super::{portrait_seed_layout_known, should_save_portrait_crop};
    use crate::ocr::preprocess::{
        NAME_WINDOW_5V5_CENTRED, NAME_WINDOW_5V5_INGAME, NAME_WINDOW_5V5_PERKS,
    };
    use crate::parse::NameMatch;

    fn m(window: usize, sole_window: bool) -> NameMatch {
        NameMatch {
            row: 0,
            window,
            sole_window,
        }
    }

    #[test]
    fn five_v_five_seeds_only_from_the_no_perk_in_match_window() {
        assert!(portrait_seed_layout_known(
            5,
            Some(&m(NAME_WINDOW_5V5_INGAME, true))
        ));
        // Perk and centred boards: the grid's x does not fit them.
        assert!(!portrait_seed_layout_known(
            5,
            Some(&m(NAME_WINDOW_5V5_PERKS, true))
        ));
        assert!(!portrait_seed_layout_known(
            5,
            Some(&m(NAME_WINDOW_5V5_CENTRED, true))
        ));
        // The no-perk window matched, but so did another: layout unknown.
        assert!(!portrait_seed_layout_known(
            5,
            Some(&m(NAME_WINDOW_5V5_INGAME, false))
        ));
        // Row from the highlight alone: layout unknown.
        assert!(!portrait_seed_layout_known(5, None));
    }

    #[test]
    fn six_v_six_seeding_is_unchanged() {
        assert!(portrait_seed_layout_known(6, None));
        assert!(portrait_seed_layout_known(6, Some(&m(0, true))));
    }

    #[test]
    fn a_perk_board_never_seeds_even_with_the_career_panel() {
        let layout = portrait_seed_layout_known(5, Some(&m(NAME_WINDOW_5V5_PERKS, true)));
        assert!(!should_save_portrait_crop(true, true, true, true, layout));
        let layout = portrait_seed_layout_known(5, Some(&m(NAME_WINDOW_5V5_INGAME, true)));
        assert!(should_save_portrait_crop(true, false, true, true, layout));
    }
}

#[cfg(test)]
mod stale_seed_tests {
    use super::{
        PortraitMatcher, SEED_GEOMETRY_MARKER, SEED_GEOMETRY_VERSION, invalidate_stale_seeds,
        invalidate_stale_seeds_reading, invalidate_stale_seeds_with,
    };

    fn marker(dir: &std::path::Path) -> String {
        std::fs::read_to_string(dir.join(SEED_GEOMETRY_MARKER)).unwrap()
    }

    #[test]
    fn fresh_dir_moves_nothing_and_writes_the_marker() {
        let dir = tempfile::tempdir().unwrap();
        let portraits = dir.path().join("portraits");
        assert_eq!(invalidate_stale_seeds(&portraits).unwrap(), 0);
        assert_eq!(marker(&portraits).trim(), SEED_GEOMETRY_VERSION.to_string());
    }

    #[test]
    fn a_stale_grid_moves_ordinary_pngs_because_nothing_is_shipped() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        std::fs::write(p.join("ana.png"), b"old unpack").unwrap();
        std::fs::write(p.join("notes.txt"), b"keep").unwrap();
        assert_eq!(invalidate_stale_seeds(p).unwrap(), 1);
        assert!(!p.join("ana.png").exists());
        assert_eq!(
            std::fs::read(p.join("invalidated-1/ana.png")).unwrap(),
            b"old unpack"
        );
        assert!(p.join("notes.txt").exists());
        assert_eq!(marker(p).trim(), SEED_GEOMETRY_VERSION.to_string());
        // Once per version: a file written after the marker stays.
        std::fs::write(p.join("dmon.png"), b"new seed").unwrap();
        assert_eq!(invalidate_stale_seeds(p).unwrap(), 0);
        assert_eq!(std::fs::read(p.join("dmon.png")).unwrap(), b"new seed");
    }

    #[test]
    fn listed_reference_bytes_stay_while_seeds_move() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        let known: &[(&str, &[u8])] = &[("ana", b"ref ana")];
        std::fs::write(p.join("ana.png"), b"ref ana").unwrap();
        std::fs::write(p.join("dmon.png"), b"seed one").unwrap();
        std::fs::write(p.join("mercy.png"), b"seed two").unwrap();
        assert_eq!(invalidate_stale_seeds_with(p, known).unwrap(), 2);
        assert_eq!(std::fs::read(p.join("ana.png")).unwrap(), b"ref ana");
        assert_eq!(
            std::fs::read(p.join("invalidated-1/dmon.png")).unwrap(),
            b"seed one"
        );
        assert_eq!(
            std::fs::read(p.join("invalidated-1/mercy.png")).unwrap(),
            b"seed two"
        );
    }

    #[test]
    fn an_older_marker_also_invalidates_and_names_its_version() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        std::fs::write(p.join(SEED_GEOMETRY_MARKER), "1\n").unwrap();
        std::fs::write(p.join("ana.png"), b"crop").unwrap();
        let fake: &[(&str, &[u8])] = &[("ana", b"bundled ana")];
        assert_eq!(invalidate_stale_seeds_with(p, fake).unwrap(), 1);
        assert!(p.join("invalidated-1/ana.png").exists());
        // A second stale file of the same name never overwrites the first.
        std::fs::write(p.join(SEED_GEOMETRY_MARKER), "1\n").unwrap();
        std::fs::write(p.join("ana.png"), b"crop two").unwrap();
        assert_eq!(invalidate_stale_seeds_with(p, fake).unwrap(), 1);
        assert_eq!(
            std::fs::read(p.join("invalidated-1/ana.png")).unwrap(),
            b"crop"
        );
        assert_eq!(
            std::fs::read(p.join("invalidated-1/ana.1.png")).unwrap(),
            b"crop two"
        );
    }

    #[test]
    fn loading_does_not_invent_a_reference() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        std::fs::write(p.join("mercy.png"), b"not a png").unwrap();
        let matcher = PortraitMatcher::load(p);
        assert!(matcher.is_empty());
        assert_eq!(std::fs::read(p.join("mercy.png")).unwrap(), b"not a png");
    }

    /// alpha.5 bad stems (#240) move. A stem whose bytes are listed as a
    /// known reference stays; the public migration lists none.
    #[test]
    fn bad_stem_seeds_move_with_the_grid_migration() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        std::fs::write(p.join("soldier76.png"), b"listed").unwrap();
        std::fs::write(p.join("soldier:_76.png"), b"seed").unwrap();
        std::fs::write(p.join("torbjörn.png"), b"seed").unwrap();
        let known: &[(&str, &[u8])] = &[("soldier76", b"listed")];
        assert_eq!(invalidate_stale_seeds_with(p, known).unwrap(), 2);
        assert!(p.join("invalidated-1/soldier:_76.png").exists());
        assert!(p.join("invalidated-1/torbjörn.png").exists());
        assert_eq!(std::fs::read(p.join("soldier76.png")).unwrap(), b"listed");
        assert_eq!(marker(p).trim(), SEED_GEOMETRY_VERSION.to_string());
    }

    /// A bad stem written after the marker is current (an older daemon still
    /// running) still moves aside on the next start; normal seeds stay.
    #[test]
    fn bad_stem_seeds_move_even_when_the_marker_is_current() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        std::fs::write(
            p.join(SEED_GEOMETRY_MARKER),
            format!("{SEED_GEOMETRY_VERSION}\n"),
        )
        .unwrap();
        std::fs::write(p.join("dmon.png"), b"v2 seed").unwrap();
        std::fs::write(p.join("torbjörn.png"), b"v2 bad stem").unwrap();
        assert_eq!(invalidate_stale_seeds(p).unwrap(), 1);
        assert_eq!(std::fs::read(p.join("dmon.png")).unwrap(), b"v2 seed");
        assert!(!p.join("torbjörn.png").exists());
        let dest = format!("invalidated-{SEED_GEOMETRY_VERSION}/torbjörn.png");
        assert_eq!(std::fs::read(p.join(dest)).unwrap(), b"v2 bad stem");
        // Nothing left to move.
        assert_eq!(invalidate_stale_seeds(p).unwrap(), 0);
    }

    /// One unreadable png (#240 N1): one WARN, the file stays, the other
    /// seeds still move, and the marker is written so it does not repeat.
    #[test]
    fn an_unreadable_png_warns_once_and_the_marker_is_still_written() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        std::fs::write(p.join("ana.png"), b"unreadable").unwrap();
        std::fs::write(p.join("mercy.png"), b"seed").unwrap();
        let (moved, log) = with_warn_log(|| {
            invalidate_stale_seeds_reading(p, &[], |path| {
                if path.file_name().is_some_and(|n| n == "ana.png") {
                    Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied))
                } else {
                    std::fs::read(path)
                }
            })
        });
        assert_eq!(moved.unwrap(), 1);
        assert_eq!(log.matches("WARN").count(), 1, "{log}");
        assert!(log.contains("ana.png"), "{log}");
        assert!(p.join("ana.png").exists());
        assert!(p.join("invalidated-1/mercy.png").exists());
        assert_eq!(marker(p).trim(), SEED_GEOMETRY_VERSION.to_string());
    }

    #[derive(Clone, Default)]
    struct Buf(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl std::io::Write for Buf {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Buf {
        type Writer = Buf;
        fn make_writer(&'a self) -> Buf {
            self.clone()
        }
    }

    fn with_warn_log<T>(f: impl FnOnce() -> T) -> (T, String) {
        let buf = Buf::default();
        let sub = tracing_subscriber::fmt()
            .with_writer(buf.clone())
            .with_ansi(false)
            .with_max_level(tracing::Level::WARN)
            .finish();
        let out = tracing::subscriber::with_default(sub, f);
        let text = String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();
        (out, text)
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
            scan(5, Some(0.084), Some(0.0845)).checked_team_size(),
            Some(5)
        );
        assert_eq!(scan(5, Some(0.084), None).checked_team_size(), Some(5));
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
    fn death_screen_pitch_without_a_table_is_not_a_scoreboard() {
        // Death screen with killfeed and HUD. The row scan logged
        // dip_count=2, dip_pitch=0.0586, spectral_pitch=0.0963. Header
        // labels in the 3..=10 band used to skip the "not a scoreboard"
        // path and reject it as an unsettled 5v5-vs-6v6 pitch.
        let death = scan(2, Some(0.0586), Some(0.0963));
        assert!(!death.looks_like_scoreboard());
        assert_eq!(death.checked_team_size(), None);
        assert_eq!(
            super::classify_scoreboard_preflight(&death, 6, false),
            super::ScoreboardPreflight::NotAScoreboard
        );
    }

    #[test]
    fn death_screen_pitch_on_a_table_is_team_size_unsure() {
        let death = scan(2, Some(0.0586), Some(0.0963));
        assert_eq!(
            super::classify_scoreboard_preflight(&death, 6, true),
            super::ScoreboardPreflight::TeamSizeUncertain
        );
    }

    #[test]
    fn settled_pitch_stays_a_board_when_the_table_detector_misses() {
        // A settled pitch is a board even when the table detector misses
        // (empty, grey, and low-contrast boards). The table check does not
        // block a frame main would accept.
        let board = scan(6, Some(0.074), Some(0.0742));
        assert_eq!(
            super::classify_scoreboard_preflight(&board, 0, false),
            super::ScoreboardPreflight::Ready(6)
        );
        assert_eq!(
            super::classify_scoreboard_preflight(&board, 6, true),
            super::ScoreboardPreflight::Ready(6)
        );
        // Header labels and no measurable pitch: main's default size is 5.
        // A missed table must not turn that into a reject.
        let grey = scan(0, None, None);
        assert!(!grey.looks_like_scoreboard());
        assert_eq!(
            super::classify_scoreboard_preflight(&grey, 6, false),
            super::ScoreboardPreflight::Ready(5)
        );
        // Neither signal: not a board, table or not.
        assert_eq!(
            super::classify_scoreboard_preflight(&grey, 0, true),
            super::ScoreboardPreflight::NotAScoreboard
        );
    }

    /// Rows of `row` colour with a near-white name line and no stat digits.
    fn row_board(row: [u8; 3], text: [u8; 3], bg: [u8; 3]) -> image::DynamicImage {
        let (w, h) = (800u32, 480u32);
        let mut img = image::RgbImage::from_pixel(w, h, image::Rgb(bg));
        let y0s = [40u32, 76, 112, 148, 184];
        for y_mid in y0s {
            for y in y_mid.saturating_sub(8)..y_mid + 8 {
                for x in (0.06 * w as f64) as u32..(0.55 * w as f64) as u32 {
                    let px = if (y_mid..y_mid + 3).contains(&y) {
                        text
                    } else {
                        row
                    };
                    img.put_pixel(x, y, image::Rgb(px));
                }
            }
        }
        image::DynamicImage::ImageRgb8(img)
    }

    #[test]
    fn empty_grey_and_low_contrast_boards_still_count() {
        use super::ScoreboardPreflight;
        use crate::ocr::preprocess::scoreboard_table_present;

        // Empty ban-style board: row bars and name ink, no stat digits.
        // The table detector misses it. The pitch still settles 6v6.
        let empty = row_board([40, 110, 220], [240, 240, 240], [16, 18, 28]);
        assert!(
            !scoreboard_table_present(&empty),
            "empty rows should miss the table detector"
        );
        let (pre, scan) = super::preflight_scoreboard(&empty);
        assert_eq!(pre, ScoreboardPreflight::Ready(6), "{scan:?}");

        // Desaturated grey board: header labels, no saturation dips.
        // Main accepts this as size 5. A flat grey body has no darker gap,
        // so the table detector says no.
        let (w, h) = (800u32, 480u32);
        let mut grey = image::RgbImage::from_pixel(w, h, image::Rgb([150, 150, 150]));
        for i in 0..6 {
            let x0 = (w as f64 * (0.42 + i as f64 * 0.08)) as u32;
            for y in (h as f64 * 0.006) as u32..(h as f64 * 0.022) as u32 {
                for x in x0..x0 + 6 {
                    grey.put_pixel(x, y, image::Rgb([16, 16, 16]));
                }
            }
        }
        let grey = image::DynamicImage::ImageRgb8(grey);
        assert!(!scoreboard_table_present(&grey));
        let (pre, scan) = super::preflight_scoreboard(&grey);
        assert_eq!(pre, ScoreboardPreflight::Ready(5), "{scan:?}");

        // Low-contrast replay: muted rows, no bright header, no stat digits.
        let replay = row_board([96, 84, 84], [148, 146, 144], [60, 62, 64]);
        assert!(!scoreboard_table_present(&replay));
        let (pre, scan) = super::preflight_scoreboard(&replay);
        assert_eq!(pre, ScoreboardPreflight::Ready(6), "{scan:?}");
    }

    #[test]
    fn only_implausible_pitches_are_unknown() {
        let s = scan(3, Some(0.11), Some(0.05));
        assert_eq!(s.checked_team_size(), None);
        // 0.05 is not a row pitch, so the offline fallback stays at the
        // documented default of 5.
        assert_eq!(s.team_size(), 5);
        assert_eq!(scan(2, Some(0.12), None).checked_team_size(), None);
    }

    /// Slot height over crop height for one team size, from the same integer
    /// layout `crop_player_row` uses.
    fn layout_pitch(frame_w: u32, frame_h: u32, team_size: usize) -> (u32, u32, f64) {
        let img = image::DynamicImage::ImageRgb8(image::RgbImage::new(frame_w, frame_h));
        let board = crate::ocr::preprocess::crop_scoreboard(&img);
        let slot = crate::ocr::preprocess::row_slot_height(board.height(), team_size);
        let pitch = slot as f64 / board.height() as f64;
        (board.height(), slot, pitch)
    }

    #[test]
    fn pitch_split_sits_between_layout_five_and_six() {
        let (h1080, six_px_1080, six_1080) = layout_pitch(1920, 1080, 6);
        let (_, five_px_1080, five_1080) = layout_pitch(1920, 1080, 5);
        let (h1440, six_px_1440, six_1440) = layout_pitch(2560, 1440, 6);
        let (_, five_px_1440, five_1440) = layout_pitch(2560, 1440, 5);
        let (h4k, six_px_4k, six_4k) = layout_pitch(3840, 2160, 6);
        let (_, five_px_4k, five_4k) = layout_pitch(3840, 2160, 5);

        assert_eq!((h1080, six_px_1080, five_px_1080), (756, 58, 68));
        assert_eq!((h1440, six_px_1440, five_px_1440), (1007, 77, 90));
        assert_eq!((h4k, six_px_4k, five_px_4k), (1512, 116, 136));

        // Layout slots stay on their own side of the no-guess band.
        // 1080p and 1440p 5v5 here are the crop math, not measured boards.

        // Real 5v5 slot at 1080p (68/756) and 1440p (90/1007), both pitches
        // agreeing, the way the capture check reads a board.
        let five_1080_board = scan(5, Some(five_1080), Some(five_1080));
        assert_eq!(five_1080_board.checked_team_size(), Some(5));
        assert_eq!(five_1080_board.team_size(), 5);
        let five_1440_board = scan(5, Some(five_1440), Some(five_1440));
        assert_eq!(five_1440_board.checked_team_size(), Some(5));
        assert_eq!(five_1440_board.team_size(), 5);
        let five_4k_board = scan(5, Some(five_4k), Some(five_4k));
        assert_eq!(five_4k_board.checked_team_size(), Some(5));
        assert_eq!(
            scan(6, Some(six_1080), Some(six_1080)).checked_team_size(),
            Some(6)
        );
        assert_eq!(
            scan(6, Some(six_1440), Some(six_1440)).checked_team_size(),
            Some(6)
        );
        assert_eq!(
            scan(6, Some(six_4k), Some(six_4k)).checked_team_size(),
            Some(6)
        );

        // Measured 1080p 6v6. The old 0.079 split put 0.0794 on the 5v5 side.
        let measured = scan(6, Some(0.0794), Some(0.0794));
        assert_eq!(measured.checked_team_size(), Some(6));
        assert_eq!(measured.team_size(), 6);
        let straddle = scan(6, Some(0.0788), Some(0.0794));
        assert_eq!(straddle.checked_team_size(), Some(6));
        assert_eq!(straddle.team_size(), 6);
    }

    #[test]
    fn measured_pitch_band_does_not_guess() {
        // The 0.080/0.083 band is provisional, pending native 1440p 5v5
        // boards. Highest real 6v6: dip 0.07937 (four 1080p boards),
        // spectral 0.07672. At 1440p the 6v6 dip runs 0.0725 to 0.0775.
        assert_eq!(
            scan(6, Some(0.07937), Some(0.07937)).checked_team_size(),
            Some(6)
        );
        // Weakest real 5v5 PC board: spectral 0.0847, dip 0.0866.
        // Synthetic pitches only. No scoreboard image.
        assert_eq!(
            scan(5, Some(0.0866), Some(0.0847)).checked_team_size(),
            Some(5)
        );
        assert_eq!(
            scan(6, Some(0.07672), Some(0.07672)).checked_team_size(),
            Some(6)
        );
        assert_eq!(
            scan(6, Some(0.0725), Some(0.0725)).checked_team_size(),
            Some(6)
        );
        assert_eq!(
            scan(6, Some(0.0775), Some(0.0775)).checked_team_size(),
            Some(6)
        );

        // Closed band: both edges and the middle are unknown, including
        // when only one pitch was measured.
        for pitch in [0.080, 0.0815, 0.083] {
            assert_eq!(
                scan(5, Some(pitch), Some(pitch)).checked_team_size(),
                None,
                "{pitch}"
            );
            assert_eq!(
                scan(5, Some(pitch), None).checked_team_size(),
                None,
                "{pitch}"
            );
            assert_eq!(
                scan(5, None, Some(pitch)).checked_team_size(),
                None,
                "{pitch}"
            );
        }
        // One pitch on the band and one just outside still does not guess.
        assert_eq!(scan(5, Some(0.083), Some(0.0835)).checked_team_size(), None);

        // The only real 5v5 Tab board so far, wiki_scoreboard.png at 414p.
        assert_eq!(
            scan(5, Some(0.08304), Some(0.08304)).checked_team_size(),
            Some(5)
        );
    }

    #[test]
    fn wiki_scoreboard_png_is_five_when_present() {
        let roots = [
            format!(
                "{}/test-data/wiki_scoreboard.png",
                env!("CARGO_MANIFEST_DIR")
            ),
            format!(
                "{}/testdata/wiki_scoreboard.png",
                env!("CARGO_MANIFEST_DIR")
            ),
        ];
        let Some(path) = roots.iter().find(|p| std::path::Path::new(p).is_file()) else {
            return;
        };
        let img = image::open(path).unwrap_or_else(|err| panic!("open {path}: {err}"));
        let scan = super::scan_rows(&crate::ocr::preprocess::crop_scoreboard(&img));
        assert_eq!(scan.checked_team_size(), Some(5), "{path}: {scan:?}");
    }
}

#[cfg(test)]
mod portrait_floor_tests {
    use super::{PortraitMatcher, portrait_confidence_is_trusted};
    use image::{DynamicImage, Rgb, RgbImage};
    use std::collections::HashMap;

    fn solid(value: u8) -> DynamicImage {
        DynamicImage::ImageRgb8(RgbImage::from_pixel(32, 32, Rgb([value, value, value])))
    }

    #[test]
    fn portrait_matcher_rejects_a_low_confidence_guess() {
        let mut references = HashMap::new();
        references.insert("sierra".into(), solid(128).to_rgb8());
        let matcher = PortraitMatcher::from_references(references);
        let weak = solid(128 + 59);
        let confidence = 1.0 - (59.0 / 255.0);
        assert!(
            (0.70..0.85).contains(&confidence),
            "synthetic pair should sit near 0.77, got {confidence}"
        );
        assert!(
            matcher.match_portrait(&weak).is_none(),
            "0.77 must not override the new reader"
        );
        let trusted = matcher
            .match_portrait(&solid(128 + 30))
            .expect("about 0.88 is a portrait");
        assert_eq!(trusted.0, "sierra");
        assert!(trusted.1 >= 0.85, "{}", trusted.1);
        let same = matcher.match_portrait(&solid(128)).expect("identical crop");
        assert_eq!(same.0, "sierra");
        assert!(same.1 > 0.99, "{}", same.1);
    }

    #[test]
    fn portrait_matcher_accepts_the_floor_and_rejects_just_below() {
        assert!(portrait_confidence_is_trusted(0.85));
        assert!(!portrait_confidence_is_trusted(0.85_f64.next_down()));
        let mut references = HashMap::new();
        references.insert("sierra".into(), solid(128).to_rgb8());
        let matcher = PortraitMatcher::from_references(references);
        // A flat image's confidence is 1 - delta/255. Delta 38 is the first
        // step at or above 0.85. Delta 39 is the step just below.
        let at_floor = 1.0 - (38.0 / 255.0);
        let just_below = 1.0 - (39.0 / 255.0);
        assert!(at_floor >= 0.85, "{at_floor}");
        assert!(just_below < 0.85, "{just_below}");
        let accepted = matcher
            .match_portrait(&solid(128 + 38))
            .expect("the 0.85 step is a portrait");
        assert_eq!(accepted.0, "sierra");
        assert!((accepted.1 - at_floor).abs() < 1e-9, "{}", accepted.1);
        assert!(matcher.match_portrait(&solid(128 + 39)).is_none());
    }
}

#[cfg(test)]
mod player_row_fallback_tests {
    use super::{PortraitMatcher, resolve_player_row};
    use image::{DynamicImage, Rgb, RgbImage};
    use std::collections::HashMap;

    /// A 5v5 board, flat dark grey, with one team-1 row slot lighter, as the
    /// game draws the player's own row.
    fn board_with_highlight(row: usize) -> DynamicImage {
        let (w, h, team_size) = (1600u32, 900u32, 5usize);
        let mut img = RgbImage::from_pixel(w, h, Rgb([40, 40, 40]));
        let slot = crate::ocr::preprocess::row_slot_height(h, team_size);
        let top = (h as f64 * 0.025) as u32 + row as u32 * slot;
        for y in top..top + slot {
            for x in 0..w {
                img.put_pixel(x, y, Rgb([62, 62, 62]));
            }
        }
        DynamicImage::ImageRgb8(img)
    }

    /// alpha.7 O9: the own-row seeding gate needs how far the highlighted
    /// row leads the next brightest one. The row is `resolve_player_row`'s.
    #[test]
    fn the_highlight_reports_its_lead_over_the_next_brightest_row() {
        let board = board_with_highlight(2);
        let (row, lead) = super::player_row_highlight(&board, 5).expect("highlight");
        assert_eq!(Some(row), resolve_player_row(None, &board, 5));
        assert_eq!(row, 2);
        assert!((lead - 62.0 / 40.0).abs() < 0.01, "lead {lead}");
        // A second lit row (a cursor hover): the same row, a small lead.
        let mut two = board.to_rgb8();
        let slot = crate::ocr::preprocess::row_slot_height(900, 5);
        let top = (900.0 * 0.025) as u32 + 4 * slot;
        for y in top..top + slot {
            for x in 0..1600 {
                two.put_pixel(x, y, Rgb([58, 58, 58]));
            }
        }
        let two = DynamicImage::ImageRgb8(two);
        let (row, lead) = super::player_row_highlight(&two, 5).expect("highlight");
        assert_eq!(Some(row), resolve_player_row(None, &two, 5));
        assert_eq!(row, 2);
        assert!((lead - 62.0 / 58.0).abs() < 0.01, "lead {lead}");
        assert!(lead < crate::detect::own_row::SEED_HIGHLIGHT_LEAD);
        // No lit row: no highlight, as before.
        let flat = DynamicImage::ImageRgb8(RgbImage::from_pixel(1600, 900, Rgb([40, 40, 40])));
        assert_eq!(super::player_row_highlight(&flat, 5), None);
        assert_eq!(resolve_player_row(None, &flat, 5), None);
    }

    #[test]
    fn highlighted_row_survives_a_portrait_below_the_threshold() {
        let board = board_with_highlight(2);
        // The only reference is pure white, so every portrait crop scores far
        // below the trust floor and the portrait path returns nothing.
        let matcher = PortraitMatcher::from_references(HashMap::from([(
            "white".to_string(),
            RgbImage::from_pixel(
                super::PORTRAIT_SIZE,
                super::PORTRAIT_SIZE,
                Rgb([255, 255, 255]),
            ),
        )]));
        let portrait = matcher.match_player_hero_with_team_size(&board, 5);
        assert!(
            portrait.is_none(),
            "portrait should be untrusted: {portrait:?}"
        );

        let row = resolve_player_row(portrait.map(|(_, _, idx)| idx), &board, 5);
        assert_eq!(row, Some(2));
    }

    #[test]
    fn a_trusted_portrait_row_wins_over_the_highlight() {
        let board = board_with_highlight(2);
        assert_eq!(resolve_player_row(Some(4), &board, 5), Some(4));
    }

    #[test]
    fn no_highlight_and_no_portrait_is_no_row() {
        let flat = DynamicImage::ImageRgb8(RgbImage::from_pixel(1600, 900, Rgb([40, 40, 40])));
        assert_eq!(resolve_player_row(None, &flat, 5), None);
    }
}

#[cfg(test)]
mod installed_pack_tests {
    use super::PortraitMatcher;
    use crate::shadow::heroes::test_fixtures::portrait;
    use image::DynamicImage;

    fn save(dir: &std::path::Path, name: &str, id: u32) {
        std::fs::create_dir_all(dir).unwrap();
        portrait(id, 64, Some([20, 20, 30]))
            .save(dir.join(name))
            .unwrap();
    }

    #[test]
    fn a_missing_pack_names_no_hero_even_when_local_pngs_exist() {
        let dir = tempfile::tempdir().unwrap();
        save(&dir.path().join("portraits"), "ana.png", 1);
        let matcher = PortraitMatcher::load_installed(dir.path());
        assert!(matcher.is_empty());
        let crop = DynamicImage::ImageRgba8(portrait(1, 64, Some([20, 20, 30])));
        assert!(matcher.match_portrait(&crop).is_none());
        assert!(matcher.match_player_hero(&crop).is_none());
    }

    #[test]
    fn the_installed_pack_is_the_reference_and_specials_are_not_heroes() {
        let dir = tempfile::tempdir().unwrap();
        let heroes = dir.path().join("templates/heroes");
        save(&heroes, "ana.png", 7);
        save(&heroes, "placeholder.png", 9);
        save(&heroes.join("special"), "skull_01.png", 3);
        let matcher = PortraitMatcher::load_installed(dir.path());
        assert!(matcher.contains("ana"));
        assert!(!matcher.contains("placeholder"));
        assert!(!matcher.contains("skull_01"));
        let crop = DynamicImage::ImageRgba8(portrait(7, 64, Some([20, 20, 30])));
        let (name, conf) = matcher.match_portrait(&crop).expect("pack ana");
        assert_eq!(name, "ana");
        assert!(conf >= 0.85, "{conf}");
    }

    #[test]
    fn a_local_crop_fills_a_stem_the_pack_lacks_and_does_not_replace_one() {
        let dir = tempfile::tempdir().unwrap();
        let heroes = dir.path().join("templates/heroes");
        save(&heroes, "ana.png", 7);
        let portraits = dir.path().join("portraits");
        save(&portraits, "ana.png", 99);
        save(&portraits, "mercy.png", 4);
        // A current geometry marker: without it these files are stale seeds
        // and load moves them aside before the fill.
        std::fs::write(
            portraits.join(super::SEED_GEOMETRY_MARKER),
            format!("{}\n", super::SEED_GEOMETRY_VERSION),
        )
        .unwrap();
        let matcher = PortraitMatcher::load_installed(dir.path());
        assert!(matcher.contains("ana"));
        assert!(matcher.contains("mercy"));
        let pack_ana = DynamicImage::ImageRgba8(portrait(7, 64, Some([20, 20, 30])));
        let (name, _) = matcher.match_portrait(&pack_ana).expect("pack ana wins");
        assert_eq!(name, "ana");
        let local_ana = DynamicImage::ImageRgba8(portrait(99, 64, Some([20, 20, 30])));
        assert!(
            matcher.match_portrait(&local_ana).is_none(),
            "the local ana crop must not override the pack"
        );
    }

    #[test]
    fn the_1080_pack_fills_only_stems_the_main_pack_lacks() {
        let dir = tempfile::tempdir().unwrap();
        save(&dir.path().join("templates/heroes-v2"), "ana.png", 7);
        save(&dir.path().join("templates/heroes-v2-1080"), "ana.png", 99);
        save(&dir.path().join("templates/heroes-v2-1080"), "mercy.png", 4);
        // v2 is used only when it covers v1. With no v1, a v2 set that loads
        // is used as it is.
        let matcher = PortraitMatcher::load_installed(dir.path());
        assert!(matcher.contains("ana"));
        assert!(matcher.contains("mercy"));
        let main = DynamicImage::ImageRgba8(portrait(7, 64, Some([20, 20, 30])));
        assert_eq!(matcher.match_portrait(&main).unwrap().0, "ana");
    }
}

/// Pack parity check for the portrait move. Ignored in CI because the hero
/// pack and real boards are not in the repo. Run locally with:
///
/// ```text
/// SCUFFED_HERO_PACK_DIR=<extracted heroes-v1 pack>
/// SCUFFED_OLD_PORTRAITS_DIR=<portraits/ from the commit before the move>
/// SCUFFED_PROOF_BOARDS_DIR=<scoreboard PNGs>
/// cargo test -p scuffed-stat-tracker pack_parity -- --ignored --nocapture
/// ```
#[cfg(test)]
mod pack_parity_tests {
    use super::*;

    /// Pack files use `soldier-76`, the old bundle used `soldier76`.
    fn norm(stem: &str) -> String {
        stem.chars().filter(|c| c.is_ascii_alphanumeric()).collect()
    }

    fn env_dir(key: &str) -> PathBuf {
        PathBuf::from(std::env::var(key).unwrap_or_else(|_| panic!("set {key}")))
    }

    fn installed_matcher() -> PortraitMatcher {
        let pack = env_dir("SCUFFED_HERO_PACK_DIR");
        let data = tempfile::tempdir().unwrap();
        let dest = crate::shadow::heroes::HeroTemplates::dir_in(data.path());
        std::fs::create_dir_all(&dest).unwrap();
        copy_tree(&pack, &dest);
        let matcher = PortraitMatcher::load_installed(data.path());
        assert!(!matcher.is_empty(), "pack did not load");
        matcher
    }

    fn copy_tree(from: &Path, to: &Path) {
        for e in std::fs::read_dir(from).unwrap().flatten() {
            let p = e.path();
            let t = to.join(e.file_name());
            if p.is_dir() {
                std::fs::create_dir_all(&t).unwrap();
                copy_tree(&p, &t);
            } else {
                std::fs::copy(&p, &t).unwrap();
            }
        }
    }

    #[test]
    #[ignore = "needs the hero pack and old portraits on disk"]
    fn pack_parity_every_moved_portrait_loads_from_the_pack() {
        let old = PortraitMatcher::load(&env_dir("SCUFFED_OLD_PORTRAITS_DIR"));
        let new = installed_matcher();
        let have: std::collections::HashSet<String> =
            new.references.keys().map(|k| norm(k)).collect();
        let mut missing: Vec<_> = old
            .references
            .keys()
            .map(|k| norm(k))
            .filter(|k| !have.contains(k))
            .collect();
        missing.sort();
        println!(
            "old {} stems, pack {} stems",
            old.references.len(),
            have.len()
        );
        assert_eq!(old.references.len(), 52);
        assert!(missing.is_empty(), "not in pack: {missing:?}");
    }

    /// Nearest reference per portrait slot, before the trust threshold, so
    /// the comparison does not pass just because both sides return nothing.
    fn nearest_per_slot(m: &PortraitMatcher, board: &DynamicImage) -> Vec<String> {
        extract_portrait_crops(board)
            .iter()
            .map(|crop| {
                let c = crop
                    .resize_exact(PORTRAIT_SIZE, PORTRAIT_SIZE, FilterType::Lanczos3)
                    .to_rgb8();
                m.references
                    .iter()
                    .map(|(k, r)| (norm(k), mean_absolute_difference(&c, r)))
                    .min_by(|x, y| x.1.total_cmp(&y.1))
                    .map(|(k, _)| k)
                    .unwrap_or_default()
            })
            .collect()
    }

    #[test]
    #[ignore = "needs the hero pack, old portraits and proof boards on disk"]
    fn pack_parity_proof_boards_read_the_same() {
        let old = PortraitMatcher::load(&env_dir("SCUFFED_OLD_PORTRAITS_DIR"));
        let new = installed_matcher();
        let mut boards: Vec<_> = std::fs::read_dir(env_dir("SCUFFED_PROOF_BOARDS_DIR"))
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("png"))
            .collect();
        boards.sort();
        assert!(!boards.is_empty());
        let mut diffs = Vec::new();
        for b in &boards {
            let img = image::open(b).unwrap();
            // What callers see: trusted matches and the player hero.
            let trusted = |m: &PortraitMatcher| {
                let all: Vec<String> = m
                    .match_all_portraits(&img)
                    .iter()
                    .map(|(h, _)| norm(h))
                    .collect();
                let me = m.match_player_hero(&img).map(|(h, _, r)| (norm(&h), r));
                (all, me)
            };
            let (a, n) = (trusted(&old), trusted(&new));
            // Below the trust threshold, for information only.
            let (na, nn) = (nearest_per_slot(&old, &img), nearest_per_slot(&new, &img));
            let same_nearest = na.iter().zip(&nn).filter(|(x, y)| x == y).count();
            println!(
                "{}: trusted old {a:?} pack {n:?}; untrusted nearest agree {same_nearest}/{}",
                b.file_name().unwrap().to_string_lossy(),
                na.len()
            );
            if a != n {
                diffs.push(b.clone());
            }
        }
        println!("{} boards, {} differ", boards.len(), diffs.len());
        assert!(diffs.is_empty(), "differ: {diffs:?}");
    }
}
