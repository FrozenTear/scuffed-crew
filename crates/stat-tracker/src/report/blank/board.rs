//! Board geometry for the scoreboard blanker, measured on every board.
//!
//! Lengths are in 1664 px crop units (the scoreboard crop of a 2560x1440
//! frame) times `s = width / 1664`. The table edges come from the light header
//! strip, the stat columns hang off its right edge and must match the header
//! labels, and the rows are fitted to the 2 px separators between them. A
//! measurement that is missing or out of range fails the whole board; a row
//! whose separators do not show keeps nothing.
//!
//! Measured on the 34 real scoreboards of 2026-10-11/12 (four in-match Tab
//! layouts and the centred post-match table) at 1664 px and on their
//! 1248 px LANCZOS copies, see `docs/report-bundle.md` section 4.
//!
//! 5v5 Tab boards draw the same table `K5` larger, so every 6v6 length holds
//! at `s * K5`. The crop cuts the table's left edge off, so the 5v5 checks
//! anchor on the header strip's right edge instead (measured on 20 real
//! 1440p 5v5 boards of 2026-10-15, two games, four perk layouts).

use image::RgbImage;

use super::NAME_COL;

pub(super) const REF_W: f64 = 1664.0;
const REF_H: f64 = 1007.0;

/// Digit column centres of E A D DMG H MIT: `R - OFF * s`.
pub(super) const OFF: [f64; 6] = [511.5, 454.0, 396.5, 305.0, 201.0, 96.5];
/// Header label centres: `R - LAB_OFF * s`, each within `LAB_TOL * s`.
const LAB_OFF: [f64; 6] = [510.5, 453.5, 394.5, 304.75, 197.0, 101.0];
const LAB_TOL: f64 = 6.0;
/// `(L + R) / 2` of the in-match Tab table (beside the hero card); the
/// post-match table is centred in the crop instead.
const CENTRE: f64 = 522.0;
const CENTRE_TOL: f64 = 2.5;
/// `R - L` of the measured layouts: no perks, two perk or badge variants, and
/// the wide perk layout whose role bar starts at x 0.
const WIDTHS: [f64; 4] = [930.0, 978.0, 994.0, 1044.0];
const WIDTH_TOL: f64 = 3.0;
/// Header strip bottom row (measured 29).
const HDR_RANGE: (f64, f64) = (26.0, 32.0);
/// Anti-aliased rows at the bottom of the header strip (1664 px wide crop).
const EDGE_ROWS: f64 = 1.0;
/// Widest dark speck inside a header row that is bridged (1664 px crop).
const SPECK: f64 = 3.0;
/// Role bar columns sampled for the separator profile, from `L`.
const BAR: (f64, f64) = (5.0, 27.0);
/// Role bar width.
pub(super) const BAR_FULL: f64 = 32.0;
/// Row pitch search (6v6 measured 75.0 to 75.25).
const ROW_P: (f64, f64) = (74.5, 75.75);
/// Separator line width.
const SEP_W: f64 = 2.0;
/// Team 2's first row top (measured 566).
const T2_RANGE_6: (f64, f64) = (556.0, 576.0);
/// A separator shows when its stat-area contrast reaches `SEP_MIN` (real
/// >= 0.22) or its role-bar contrast reaches `SEP_BAR`.
const SEP_MIN: f64 = 0.12;
const SEP_BAR: f64 = 0.30;
/// Internal separators that must show before a team's grid is trusted.
const SEP_TRUST: usize = 3;
/// Crop aspect tolerance, and the scale range (1080p crops are 0.75, 2160p 1.5).
const ASPECT_TOL: f64 = 0.01;
const SCALE: (f64, f64) = (0.70, 1.60);

/// 5v5 table scale vs 6v6: row pitch 88 against 75 (labels, digits and the
/// header height agree within 1%).
const K5: f64 = 88.0 / 75.0;
/// 5v5 header strip right edge `R` in crop units (the strip starts at x 0,
/// cut by the crop), one per perk layout.
const R5: [f64; 4] = [1068.0, 1096.0, 1105.0, 1134.0];
/// 5v5 header label centres `R - LAB5 * s` (crop units; H measured 231.5
/// to 239.5, MIT 114 to 119).
const LAB5: [f64; 6] = [599.0, 532.0, 464.5, 357.5, 235.5, 116.5];
/// Team 2's first row top in a 5v5 crop (measured 577).
const T2_RANGE_5: (f64, f64) = (567.0, 587.0);
/// `L + R` of a 5v5 table in crop units. The table is centred like 6v6, so
/// its left edge `L` (outside the crop) moves opposite to `R`: the role bar
/// ends at `L + 32 s`, measured at x 13 with `R` 1068. The name column is
/// [`NAME_COL`] from that `L`.
const LR5: f64 = 1043.5;

/// Why a board, or part of it, keeps nothing.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Fail(pub(super) String);

impl Fail {
    fn new(why: impl Into<String>) -> Self {
        Fail(why.into())
    }
}

/// Python `round()` (ties to even), as in the prototype and the reader.
pub(super) fn pyround(x: f64) -> i64 {
    x.round_ties_even() as i64
}

/// One player row of the fitted grid. `bot` is exclusive and sits above the
/// separator under the row.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Row {
    pub(super) team: usize,
    pub(super) idx: usize,
    pub(super) top: u32,
    pub(super) bot: u32,
    /// Both separators of the row show and the team grid is trusted.
    pub(super) verified: bool,
}

/// The measured table.
#[derive(Debug, Clone)]
pub(super) struct Board {
    pub(super) s: f64,
    /// Table edges from the header strip, `[l, r)`.
    pub(super) l: u32,
    /// Read by the port check and kept for debugging.
    #[allow(dead_code)]
    pub(super) r: u32,
    /// First row under the header strip.
    #[allow(dead_code)]
    pub(super) hb: u32,
    /// Digit column centres `R - OFF * s` and their windows (midpoints).
    pub(super) cols: [f64; 6],
    pub(super) wins: [(usize, usize); 6],
    /// Rows of both teams that the crop holds, verified or not.
    pub(super) rows: Vec<Row>,
    /// Name column `[x0, x1)` that the name-column fallback fills.
    pub(super) name: (u32, u32),
}

/// Measure a scoreboard crop. 6v6 and 5v5 have measured geometry; other
/// team sizes fail.
pub(super) fn measure(img: &RgbImage, team: usize) -> Result<Board, Fail> {
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 {
        return Err(Fail::new("empty image"));
    }
    let s = f64::from(w) / REF_W;
    if ((f64::from(w) / f64::from(h)) / (REF_W / REF_H) - 1.0).abs() > ASPECT_TOL {
        return Err(Fail(format!("not a scoreboard crop aspect ({w}x{h})")));
    }
    if !(SCALE.0..=SCALE.1).contains(&s) {
        return Err(Fail(format!("crop size {w}x{h} out of range")));
    }
    let crop_s = s;
    let (s, labels, lab_tol) = match team {
        6 => (s, LAB_OFF.map(|o| o * s), LAB_TOL * s),
        5 => (s * K5, LAB5.map(|o| o * s), LAB_TOL * s),
        _ => return Err(Fail(format!("team size {team} has no measured geometry"))),
    };
    let (l, r, hb) = measure_header(img, s, team, crop_s)?;
    check_labels(img, l, r, &labels, lab_tol)?;
    let rows = measure_rows(img, l, r, hb, s, team, crop_s)?;
    let left = if team == 5 {
        LR5 * crop_s - f64::from(r)
    } else {
        f64::from(l)
    };
    let name = (left + NAME_COL.0 * s, left + NAME_COL.1 * s);
    let name = (
        name.0.floor().max(0.0) as u32,
        (name.1.ceil() as u32).min(w),
    );
    let cols: [f64; 6] = std::array::from_fn(|k| f64::from(r) - OFF[k] * s);
    let wins = std::array::from_fn(|k| {
        let lo = if k > 0 {
            (cols[k - 1] + cols[k]) / 2.0
        } else {
            cols[0] - (cols[1] - cols[0]) / 2.0
        };
        let hi = if k < 5 {
            (cols[k] + cols[k + 1]) / 2.0
        } else {
            cols[5] + (cols[5] - cols[4]) / 2.0
        };
        (
            pyround(lo).max(0) as usize,
            (pyround(hi).max(0) as usize).min(w as usize),
        )
    });
    Ok(Board {
        s,
        l,
        r,
        hb,
        cols,
        wins,
        rows,
        name,
    })
}

/// Light header pixel: every channel >= 150 and channel spread <= 40.
fn header_light(p: [u8; 3]) -> bool {
    let mn = p[0].min(p[1]).min(p[2]);
    let mx = p[0].max(p[1]).max(p[2]);
    mn >= 150 && mx - mn <= 40
}

/// Fill interior gaps of at most `max` px between light pixels. Real
/// captures can carry a few dark specks (cursor, compression) inside the
/// strip; without this one 2 px speck splits the row's run below the minimum
/// width. The run edges never move, so `L` and `R` stay exact.
fn close_gaps(row: &[bool], max: usize) -> Vec<bool> {
    let mut out = row.to_vec();
    let mut last: Option<usize> = None;
    for (x, &v) in row.iter().enumerate() {
        if v {
            if let Some(p) = last
                && x - p - 1 <= max
            {
                out[p + 1..x].iter_mut().for_each(|o| *o = true);
            }
            last = Some(x);
        }
    }
    out
}

/// Longest run of `true`, as `[start, end)`; `(0, 0)` when there is none.
fn longest_run(row: &[bool]) -> (usize, usize) {
    let mut best = (0, 0);
    let mut start = None;
    for (x, &v) in row.iter().chain(std::iter::once(&false)).enumerate() {
        match (v, start) {
            (true, None) => start = Some(x),
            (false, Some(st)) => {
                if x - st > best.1 - best.0 {
                    best = (st, x);
                }
                start = None;
            }
            _ => {}
        }
    }
    best
}

/// Table edges `L`, `R` and the header strip bottom from the light header
/// strip. Only the known layouts pass: for 6v6 the in-match table centred at
/// 522 s or the post-match table centred in the crop, in one of [`WIDTHS`];
/// for 5v5 a strip from x 0 to one of [`R5`] (`crop_s` scale).
fn measure_header(
    img: &RgbImage,
    s: f64,
    team: usize,
    crop_s: f64,
) -> Result<(u32, u32, u32), Fail> {
    let (w, h) = img.dimensions();
    let lim = (h as usize).min(pyround(36.0 * s).max(0) as usize);
    let light: Vec<Vec<bool>> = (0..lim)
        .map(|y| {
            (0..w)
                .map(|x| header_light(img.get_pixel(x, y as u32).0))
                .collect()
        })
        .collect();
    let first = (pyround(30.0 * s).max(0) as usize).min(lim);
    let speck = pyround(SPECK * s).max(1) as usize;
    let clean: Vec<(usize, usize)> = light[..first]
        .iter()
        .map(|row| longest_run(&close_gaps(row, speck)))
        .filter(|&(st, en)| (en - st) as f64 >= 850.0 * s)
        .collect();
    if (clean.len() as f64) < 8.0 * s {
        let longest = light[..first]
            .iter()
            .map(|row| {
                let (start, end) = longest_run(&close_gaps(row, speck));
                end.saturating_sub(start)
            })
            .max()
            .unwrap_or(0);
        let wide = (850.0 * s).ceil() as usize;
        return Err(Fail::new(format!(
            "header strip not found: {} rows at least {wide}px wide, need {:.0}, longest run {longest}px",
            clean.len(),
            8.0 * s
        )));
    }
    let mut sts: Vec<usize> = clean.iter().map(|c| c.0).collect();
    let mut ens: Vec<usize> = clean.iter().map(|c| c.1).collect();
    sts.sort_unstable();
    ens.sort_unstable();
    let (l, r) = (sts[sts.len() / 2], ens[ens.len() / 2]);
    let agree = clean
        .iter()
        .filter(|&&(st, en)| st.abs_diff(l) <= 1 && en.abs_diff(r) <= 1)
        .count();
    if (agree as f64) < 0.6 * clean.len() as f64 {
        return Err(Fail::new("header edges ragged"));
    }
    let share = |y: usize| light[y][l..r].iter().filter(|&&v| v).count() as f64 / (r - l) as f64;
    let hb = (0..lim)
        .find(|&y| share(y) < 0.5)
        .ok_or_else(|| Fail::new("header strip interrupted"))?;
    // The strip's bottom edge is anti-aliased on real captures: the last
    // light row blends with the team bar under it, so only part of it passes
    // the grey test. Allow that many half-lit edge rows right above `hb`;
    // every row above them must still be solid.
    let edge = pyround(EDGE_ROWS * s).max(1) as usize;
    let solid = hb.saturating_sub(edge);
    if !(0..solid).all(|y| share(y) >= 0.85) {
        return Err(Fail::new("header strip interrupted"));
    }
    if !(HDR_RANGE.0 * s..=HDR_RANGE.1 * s).contains(&(hb as f64)) {
        return Err(Fail(format!("header height {hb}")));
    }
    if r + 1 > w as usize {
        return Err(Fail::new("table right edge outside the crop"));
    }
    if team == 5 {
        let wtol = (WIDTH_TOL * crop_s).max(2.0);
        if l != 0 || !R5.iter().any(|fr| (r as f64 - fr * crop_s).abs() <= wtol) {
            return Err(Fail(format!(
                "5v5 header {l} to {r} matches no known layout"
            )));
        }
        return Ok((l as u32, r as u32, hb as u32));
    }
    let ctr = (l + r) as f64 / 2.0;
    let tol = (CENTRE_TOL * s).max(2.0);
    if (ctr - CENTRE * s).abs() > tol && (ctr - f64::from(w) / 2.0).abs() > tol {
        return Err(Fail(format!(
            "table centre {ctr:.1} matches no known layout"
        )));
    }
    let wtol = (WIDTH_TOL * s).max(2.0);
    if !WIDTHS
        .iter()
        .any(|fw| ((r - l) as f64 - fw * s).abs() <= wtol)
    {
        return Err(Fail(format!(
            "table width {} matches no known layout",
            r - l
        )));
    }
    Ok((l as u32, r as u32, hb as u32))
}

/// Header label centres: [`crate::ocr::preprocess::header_label_groups`]
/// with the reader's width filter (at most 5% of the board width).
fn header_label_centres(img: &RgbImage) -> Vec<f64> {
    let w = f64::from(img.width());
    let board = image::DynamicImage::ImageRgb8(img.clone());
    crate::ocr::preprocess::header_label_groups(&board)
        .into_iter()
        .filter(|&(st, en)| f64::from(en - st) / w <= 0.05)
        .map(|(st, en)| f64::from(st + en) / 2.0)
        .collect()
}

/// The six stat labels sit at their offsets `lab` (px) from `R`, each within
/// `tol` px: 5 of 6 must be found (the one-letter H can drop out of the
/// label scan at some scales) and no other label may sit between E and MIT.
fn check_labels(img: &RgbImage, l: u32, r: u32, lab: &[f64; 6], tol: f64) -> Result<(), Fail> {
    let s = tol / LAB_TOL;
    let (l, r) = (f64::from(l), f64::from(r));
    let mut hit = [false; 6];
    for x in header_label_centres(img)
        .into_iter()
        .filter(|&x| l <= x && x < r)
    {
        let (k, d) = lab.iter().map(|o| ((r - x) - o).abs()).enumerate().fold(
            (0, f64::INFINITY),
            |b, (k, d)| if d < b.1 { (k, d) } else { b },
        );
        if d <= tol {
            hit[k] = true;
        } else if r - lab[0] - 30.0 * s < x && x < r - lab[5] + 30.0 * s {
            return Err(Fail(format!("unexpected header label at {x:.0}")));
        }
    }
    let n = hit.iter().filter(|&&v| v).count();
    if n < 5 {
        return Err(Fail(format!("{n} of 6 header labels at their columns")));
    }
    Ok(())
}

/// Median as numpy computes it (the mean of the two middle values).
pub(super) fn median(v: &mut [f64]) -> f64 {
    v.sort_by(f64::total_cmp);
    let n = v.len();
    if n == 0 {
        return f64::NAN;
    }
    if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    }
}

/// Contrast of a row boundary at `y` in a brightness profile: a 2 px line
/// darker than both neighbours (a separator) or brighter than both (the own
/// row's highlight border). `None` when the neighbours fall off the profile.
fn sep_contrast(p: &[f64], y: i64, s: f64) -> Option<f64> {
    let o = pyround(2.5 * s).max(2);
    if y - o < 0 || y + 1 + o >= p.len() as i64 {
        return None;
    }
    let y = y as usize;
    let o = o as usize;
    let (lo, hi) = if s >= 0.9 {
        (p[y].min(p[y + 1]), p[y].max(p[y + 1]))
    } else {
        (p[y], p[y])
    };
    let nlo = p[y - o].min(p[y + 1 + o]);
    let nhi = p[y - o].max(p[y + 1 + o]);
    let dark = (nlo - lo) / nlo.max(1.0);
    let bright = (hi - nhi) / hi.max(1.0);
    Some(dark.max(bright))
}

/// A team's fitted grid: first row top `t`, pitch `p`, and the (role bar,
/// stat area) contrast of each internal separator.
struct Grid {
    t: f64,
    p: f64,
    seps: Vec<(f64, f64)>,
}

/// Best `(T, P)` grid for one team: rows at `T + i * P`, scored on the
/// internal separators of both profiles (stat area counts twice).
fn fit_team(pb: &[f64], ps: &[f64], s: f64, t_lo: f64, t_hi: f64, n: usize) -> Option<Grid> {
    let mut best: Option<(f64, Grid)> = None;
    let tstep = if s >= 0.9 { 1.0 } else { 0.5 * s };
    let mut t = t_lo;
    while t <= t_hi + 1e-9 {
        let mut p = ROW_P.0 * s;
        while p <= ROW_P.1 * s + 1e-9 {
            let sc: Option<Vec<(f64, f64)>> = (1..n)
                .map(|i| {
                    let y = pyround(t + i as f64 * p - SEP_W * s);
                    Some((sep_contrast(pb, y, s)?, sep_contrast(ps, y, s)?))
                })
                .collect();
            if let Some(sc) = sc.filter(|v| !v.is_empty()) {
                let tot: f64 = sc
                    .iter()
                    .map(|&(cb, cs)| cb.max(0.0) + 2.0 * cs.max(0.0))
                    .sum();
                if best.as_ref().is_none_or(|b| tot > b.0 + 1e-9) {
                    best = Some((tot, Grid { t, p, seps: sc }));
                }
            }
            p += 0.25 * s;
        }
        t += tstep;
    }
    best.map(|(_, g)| g)
}

/// Row grid per team, fitted to the row separators.
///
/// A team's grid is trusted when at least [`SEP_TRUST`] of its internal
/// separators show. A row is verified only when the separators on both of
/// its sides show (team edges count once the grid is trusted); a row the
/// crop clips to under 60% of the pitch is left out. A 5v5 crop holds no
/// role bar (the table's left edge is cut off), so its bar profile is the
/// stat area's.
fn measure_rows(
    img: &RgbImage,
    l: u32,
    r: u32,
    hb: u32,
    s: f64,
    team: usize,
    crop_s: f64,
) -> Result<Vec<Row>, Fail> {
    let (w, h) = img.dimensions();
    let mx = |x: u32, y: u32| {
        let p = img.get_pixel(x, y).0;
        f64::from(p[0].max(p[1]).max(p[2]))
    };
    // `L + round(offset)`, not `round(L + offset)`: ties round to even.
    let xb: Vec<u32> = (i64::from(l) + pyround(BAR.0 * s)..i64::from(l) + pyround(BAR.1 * s))
        .filter(|&x| x >= 0 && x < i64::from(w))
        .map(|x| x as u32)
        .collect();
    let step = pyround(2.0 * s).max(1) as usize;
    let xs: Vec<u32> = (pyround(f64::from(r) - 560.0 * s)..pyround(f64::from(r) - 60.0 * s))
        .step_by(step)
        .filter(|&x| x >= 0 && x < i64::from(w))
        .map(|x| x as u32)
        .collect();
    if (xb.is_empty() && team != 5) || xs.is_empty() {
        return Err(Fail::new("no profile columns"));
    }
    let profile = |cols: &[u32]| -> Vec<f64> {
        (0..h)
            .map(|y| median(&mut cols.iter().map(|&x| mx(x, y)).collect::<Vec<_>>()))
            .collect()
    };
    let ps = profile(&xs);
    let pb = if team == 5 { ps.clone() } else { profile(&xb) };
    let t2 = if team == 5 {
        (T2_RANGE_5.0 * crop_s, T2_RANGE_5.1 * crop_s)
    } else {
        (T2_RANGE_6.0 * s, T2_RANGE_6.1 * s)
    };
    let hb = f64::from(hb);
    let mut rows = Vec::new();
    let mut trusted = [false; 2];
    for (ti, (lo, hi)) in [(hb - 2.0 * s, hb + 2.0 * s), t2].into_iter().enumerate() {
        let Some(Grid { t, p, seps: sc }) = fit_team(&pb, &ps, s, lo, hi, team) else {
            continue;
        };
        let shown: Vec<bool> = sc
            .iter()
            .map(|&(cb, cs)| cs >= SEP_MIN || cb >= SEP_BAR)
            .collect();
        let ok = shown.iter().filter(|&&v| v).count() >= SEP_TRUST.min(team - 1);
        trusted[ti] = ok;
        for i in 0..team {
            let top = pyround(t + i as f64 * p).max(0);
            let bot = pyround(t + (i + 1) as f64 * p - SEP_W * s).min(i64::from(h));
            if ((bot - top) as f64) < 0.6 * p {
                continue;
            }
            let above = i == 0 || shown[i - 1];
            let below = i == team - 1 || shown[i];
            rows.push(Row {
                team: ti,
                idx: i,
                top: top as u32,
                bot: bot as u32,
                verified: ok && above && below,
            });
        }
    }
    if !trusted.iter().any(|&t| t) {
        return Err(Fail::new("no team grid trusted"));
    }
    let last1 = rows.iter().filter(|r| r.team == 0).map(|r| r.bot).max();
    let first2 = rows.iter().filter(|r| r.team == 1).map(|r| r.top).min();
    if let (Some(b1), Some(t2)) = (last1, first2)
        && f64::from(t2) < f64::from(b1) + 20.0 * s
    {
        return Err(Fail::new("teams overlap"));
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use image::Rgb;

    use super::super::synth;
    use super::*;

    /// Real captures blend the header's last row with the team bar under it
    /// and can carry small dark specks inside the strip. Both used to make
    /// every real board fail with "header strip interrupted" or "header
    /// strip not found". Synthetic pixels only.
    #[test]
    fn an_anti_aliased_header_edge_and_specks_still_measure() {
        let mut img = synth::board(synth::Layout::L4);
        let (l, r) = synth::Layout::L4.edges();
        // the last header row is half grey, half a blend with the bar colour
        for x in l..r {
            if x % 5 < 2 {
                img.put_pixel(x, 28, Rgb([20, 200, 230]));
            }
        }
        // 2 px dark specks in every row of the lower header half
        for y in 17..28 {
            img.put_pixel(150 + y, y, Rgb([5, 5, 5]));
            img.put_pixel(151 + y, y, Rgb([5, 5, 5]));
        }
        let b = measure(&img, 6).expect("the board measures");
        assert_eq!((b.l, b.r, b.hb), (l, r, 29));
    }

    /// Only one edge row is forgiven: a strip broken higher up still fails.
    #[test]
    fn a_header_broken_above_its_edge_row_fails() {
        let mut img = synth::board(synth::Layout::L4);
        let (l, r) = synth::Layout::L4.edges();
        for y in 26..28 {
            for x in l..r {
                if x % 5 < 2 {
                    img.put_pixel(x, y, Rgb([20, 200, 230]));
                }
            }
        }
        assert!(measure(&img, 6).is_err());
    }

    #[test]
    fn close_gaps_keeps_run_edges_and_skips_wide_gaps() {
        let row = [
            false, true, false, false, true, false, false, false, false, true, false,
        ];
        let out = close_gaps(&row, 2);
        assert_eq!(
            out,
            [
                false, true, true, true, true, false, false, false, false, true, false
            ]
        );
    }
}
