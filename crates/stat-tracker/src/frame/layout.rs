//! Board layout: where the rows are and how big the teams are. The
//! team-1 row scan (`scan_rows`, `RowScan`, `detect_team_size`), the
//! capture preflight (`ScoreboardPreflight`, `preflight_scoreboard`), the
//! highlighted own row (`player_row_highlight`, `resolve_player_row`) and
//! the hero portrait grid (`portrait_rect`). Pixels only: no reader, no
//! Tesseract. The portrait matcher stays in `detect/hero_portrait.rs`,
//! which re-exports these at their old paths.

use image::DynamicImage;

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
pub(crate) fn detect_player_row_inner(
    scoreboard: &DynamicImage,
    team_size: usize,
) -> Option<usize> {
    highlighted_row(&team1_row_brightness(scoreboard, team_size))
}

/// The highlighted row ([`resolve_player_row`]'s own rule) and how far its
/// background leads the next brightest team 1 row's (`brightest / next`).
/// The own-row seeding gate (`detect::own_row`) needs the lead: a
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
        let Some(row_img) = crate::frame::geometry::crop_player_row(scoreboard, row, team_size)
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
/// [`crate::frame::geometry::scoreboard_table_present`], layout and brightness
/// only) is consulted only when [`RowScan::checked_team_size`] is `None`.
/// `header_labels` is the count from
/// [`crate::frame::geometry::header_label_groups`].
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
    let labels = crate::frame::geometry::header_label_groups(board).len();
    let table = crate::frame::geometry::scoreboard_table_present(board);
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
        let board = crate::frame::geometry::crop_scoreboard(&img);
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
        use crate::frame::geometry::scoreboard_table_present;

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
        let board = crate::frame::geometry::crop_scoreboard(&img);
        let slot = crate::frame::geometry::row_slot_height(board.height(), team_size);
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
        let scan = super::scan_rows(&crate::frame::geometry::crop_scoreboard(&img));
        assert_eq!(scan.checked_team_size(), Some(5), "{path}: {scan:?}");
    }
}
