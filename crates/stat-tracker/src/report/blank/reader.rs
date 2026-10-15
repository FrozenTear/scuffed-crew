//! Copy of the cv-v5 digit reader for the scoreboard blanker.
//!
//! Source: `shadow/digits.rs` at a9c01c0 (lines 146-1507), the reader that
//! sets the saved stats. The blanker keeps a digit only where this copy reads
//! it as a well-formed number, so it owns the copy: a later change to the
//! reader cannot widen what a report keeps without a change here, and the
//! reader team's file stays untouched.
//!
//! Changes from the source, nothing else:
//! * items are `pub(super)` where the blanker needs them;
//! * `column_centres` returns `None` instead of a `ShadowError`;
//! * [`read_cell`] also returns the chosen pieces with their classes and
//!   scores ([`Reading::pieces`]), which the blanker's glyph gates check;
//! * the reader's test-only items and `read_board` are left out.
//!
//! Items the blanker does not call stay, so the file diffs line for line
//! against digits.rs; hence the module-level `dead_code` allow.
#![allow(dead_code)]

use std::sync::OnceLock;

use image::DynamicImage;

use crate::ocr::preprocess;

/// Flag a cell when its calibrated confidence is below this.
///
/// Raw margins are not comparable across digits: a correct '3' (runner-up
/// '8') typically clears it by ~0.12 and can drop to 0.05, while a lone '1'
/// sits around 0.43. The old raw cut of 0.06 flagged right 3s and 8s; on the
/// labelled and stressed sets the worst wrong read it caught scores 0.29
/// after calibration, so 0.35 keeps every such error flagged with headroom.
pub(super) const SUSPECT_CONF: f32 = 0.35;
/// Partition margins (best reading minus best different reading) are scaled
/// by this so `2 * pm / PM_REF` sits on the same 1.0-is-typical scale.
pub(super) const PM_REF: f64 = 0.2;
/// Flag a reading whose ink (binarised pixels over all runs) is below this
/// many `dh * dh` per digit read. A correct lone '1', the thinnest reading,
/// measures at least 0.17 on every labelled and stressed set; a dim 0 that
/// lost most of its ring to compression and reads as '1' or '11' sits at
/// 0.10 to 0.14. The gap is thin (wiped 0 up to 0.14, floor 0.15, thinnest
/// right '1' 0.17) and was fitted only on those sets, so recheck it when new
/// labelled captures or capture paths are added.
pub(super) const MIN_INK_PER_GLYPH: f64 = 0.15;
/// Pair check scale: the median pair-check score of correctly read 4s (vs 6)
/// and 0s (vs 3) on the labelled Tab sets is 0.94 to 0.96 at both sizes (and
/// moves by under 0.003 with any one map held out), so `score / PAIR_TYP` is
/// on the same 1.0-is-typical scale as the glyph margin.
pub(super) const PAIR_TYP: f64 = 0.95;
/// A pair check only runs on a glyph whose own calibrated margin is at least
/// this: the top read must still lead clearly; the check cannot rescue a
/// coin flip.
pub(super) const PAIR_FLOOR: f64 = 0.25;
/// Pixels where the two templates differ by at least this share of their
/// largest difference make up the region a pair check looks at.
pub(super) const PAIR_MASK: f64 = 0.5;
/// Zeros are drawn in dim grey. A cell is dim when its brightness peak is
/// below this share of the board's bright cells (90th percentile of cell
/// peaks). Real dim zeros sit at 0.66.
pub(super) const DIM_RATIO: f64 = 0.8;

pub(super) const HEADER_RATIO: f64 = 0.025;
pub(super) const TEAM2_START_RATIO: f64 = 0.565;

/// Scoreboard heights the templates were built at (crop of 2560x1440 and 1920x1080).
pub(super) const REF_H_1440: u32 = 1007;
pub(super) const REF_H_1080: u32 = 756;
/// Below this the glyphs are too small to match reliably.
pub(super) const MIN_BOARD_H: u32 = 400;

pub(super) const NCLS: usize = 11;
pub(super) const COMMA: usize = 10;
pub(super) const CH: usize = 26;
pub(super) const CW: usize = 22;
pub(super) const CANVAS: usize = CH * CW;
/// Ink threshold for segmentation.
pub(super) const INK_THR: f32 = 0.45;
/// Max gap inside one number, in text heights ('11' is wide-set).
pub(super) const GAP_K: f64 = 0.9;
/// Gaps wider than [`GAP_K`] but at most this many text heights are treated
/// as a thousands separator: the groups join and a comma is inserted at the
/// bridge. Fitted on the web hand-check MIT 1,148 cell (gap 10 px, hmax 11).
pub(super) const SEP_GAP_K: f64 = 1.35;
/// Runs wider than this (in text heights) hold 2+ touching glyphs.
pub(super) const WIDE_RUN: f64 = 0.95;
pub(super) const BEAM: usize = 64;
pub(super) const KRUN: usize = 12;
/// Widest single glyph, in text heights.
pub(super) const MAX_GLYPH: f64 = 1.10;
/// Objective penalty per extra piece.
pub(super) const SPLIT_PEN: f64 = 0.04;
/// Canvas covers [top, top + DESC * dh) so the comma tail fits.
pub(super) const DESC: f64 = 1.35;
/// Slopes (px per row, negative leans like '/') of the extra cut lines tried
/// inside runs of touching glyphs. The diagonals of 7, 4 and 2 measure about
/// -0.65 at 1080p.
pub(super) const SLANTS: [f64; 2] = [-0.35, -0.7];
/// A slanted cut is kept only where it crosses at most this many ink pixels
/// (and fewer than the vertical cut through the same point).
pub(super) const SLANT_MAX_INK: usize = 1;
/// A slanted piece's column extent covers pixels with at least this much ink,
/// so a glyph keeps its anti-aliased edge like an isolated glyph does.
pub(super) const SLANT_EXTENT_INK: f32 = 0.15;
/// A cell whose own text height is this many px off its row's median takes
/// the row's text band instead (see [`row_text_band`]).
pub(super) const BAND_TOL: usize = 2;
/// Cells that must segment before a row consensus is trusted (a row has 6
/// stat cells; with fewer voters two shrunk cells can carry the median).
pub(super) const MIN_BAND_CELLS: usize = 4;

/// Python-style `round()` (ties to even), used wherever the prototype rounds.
pub(super) fn pyround(x: f64) -> i64 {
    x.round_ties_even() as i64
}

// ---------------------------------------------------------------- templates

pub(super) struct Templates {
    pub(super) t: Vec<[f32; CANVAS]>,
    pub(super) ar_lo: [f64; NCLS],
    pub(super) ar_hi: [f64; NCLS],
    /// Median glyph margin of correct reads per class (calibration scale).
    pub(super) typ: [f64; NCLS],
}

pub(super) const TPL_1440_PNG: &[u8] =
    include_bytes!("../../../assets/shadow-digits/digits_1440.png");
pub(super) const TPL_1080_PNG: &[u8] =
    include_bytes!("../../../assets/shadow-digits/digits_1080.png");

// Width gates (glyph width / text height) per class 0-9 and ',', from the
// same template build as the PNGs (mean +- 3 sd + 0.05).
pub(super) const AR_1440: ([f64; NCLS], [f64; NCLS]) = (
    [
        0.6203273556426669,
        0.1889027311771499,
        0.5631035715139091,
        0.5398853320592452,
        0.6224773293202185,
        0.5433116505660587,
        0.627875755680634,
        0.6206880574272476,
        0.5397347603536529,
        0.6229497058957643,
        0.09088588693684237,
    ],
    [
        0.9430360860590536,
        0.5169092747993298,
        0.9068670519220611,
        0.8198853320592452,
        0.9915863073915018,
        0.8233116505660587,
        0.9078757556806341,
        0.9089654951270686,
        0.8197347603536529,
        0.9029497058957643,
        0.3708858869368424,
    ],
);
pub(super) const AR_1080: ([f64; NCLS], [f64; NCLS]) = (
    [
        0.5354627551785398,
        0.1488864871283631,
        0.5115792255872451,
        0.45885760516669244,
        0.5614913920983633,
        0.4361674000280148,
        0.525508415749264,
        0.5432641600839146,
        0.4354083992183799,
        0.5231755267856124,
        0.04069201805657961,
    ],
    [
        1.0302281481894173,
        0.5901592117963682,
        0.9693204051613282,
        0.916986156683895,
        1.0684266406885221,
        0.9265699566179713,
        1.0263344682970097,
        0.9474721819588406,
        0.9177098803515125,
        1.0324979185430054,
        0.4252125696587077,
    ],
);

// Median best-minus-second-best margin of correctly read glyphs per class
// 0-9 and ',', measured with these templates on the labelled Tab sets:
// 1440p (20 boards; per-match hold-out moves no flag decision) and 1080p
// (the same 20 boards downscaled plus 15 real 1920x1080 captures).
pub(super) const TYP_1440: [f64; NCLS] = [
    0.1931, 0.4281, 0.2000, 0.1210, 0.3097, 0.1475, 0.1769, 0.4278, 0.1161, 0.2369, 0.7227,
];
pub(super) const TYP_1080: [f64; NCLS] = [
    0.1795, 0.3726, 0.1666, 0.1123, 0.2683, 0.1376, 0.1596, 0.3788, 0.1116, 0.2110, 0.7110,
];

pub(super) fn load_templates(
    png: &[u8],
    ar: ([f64; NCLS], [f64; NCLS]),
    typ: [f64; NCLS],
) -> Templates {
    let img = image::load_from_memory(png)
        .expect("embedded shadow digit templates decode")
        .to_luma8();
    assert_eq!(
        (img.width() as usize, img.height() as usize),
        (CW * NCLS, CH)
    );
    let mut t = Vec::with_capacity(NCLS);
    for c in 0..NCLS {
        let mut v = [0f32; CANVAS];
        for y in 0..CH {
            for x in 0..CW {
                v[y * CW + x] = img.get_pixel((c * CW + x) as u32, y as u32).0[0] as f32 / 255.0;
            }
        }
        normalise(&mut v);
        t.push(v);
    }
    Templates {
        t,
        ar_lo: ar.0,
        ar_hi: ar.1,
        typ,
    }
}

pub(super) fn templates_for(board_h: u32) -> &'static Templates {
    static T1440: OnceLock<Templates> = OnceLock::new();
    static T1080: OnceLock<Templates> = OnceLock::new();
    if board_h.abs_diff(REF_H_1440) <= board_h.abs_diff(REF_H_1080) {
        T1440.get_or_init(|| load_templates(TPL_1440_PNG, AR_1440, TYP_1440))
    } else {
        T1080.get_or_init(|| load_templates(TPL_1080_PNG, AR_1080, TYP_1080))
    }
}

/// Zero-mean, unit-norm in place.
pub(super) fn normalise(v: &mut [f32]) {
    let mean = v.iter().map(|&x| x as f64).sum::<f64>() / v.len() as f64;
    let mut nn = 0f64;
    for x in v.iter_mut() {
        *x = (*x as f64 - mean) as f32;
        nn += (*x as f64) * (*x as f64);
    }
    let norm = nn.sqrt().max(1e-9);
    for x in v.iter_mut() {
        *x = (*x as f64 / norm) as f32;
    }
}

// ---------------------------------------------------------------- planes

/// Row-major float image.
#[derive(Clone)]
pub(super) struct Plane {
    pub(super) w: usize,
    pub(super) h: usize,
    pub(super) d: Vec<f32>,
}

impl Plane {
    pub(super) fn zeros(w: usize, h: usize) -> Self {
        Plane {
            w,
            h,
            d: vec![0.0; w * h],
        }
    }
    #[inline]
    pub(super) fn at(&self, x: usize, y: usize) -> f32 {
        self.d[y * self.w + x]
    }
}

/// Neutral (white/grey) brightness: min channel, zeroed where saturated.
pub(super) fn neutral_ink(img: &DynamicImage) -> Plane {
    let rgb = img.to_rgb8();
    let (w, h) = (rgb.width() as usize, rgb.height() as usize);
    let mut p = Plane::zeros(w, h);
    for (i, px) in rgb.pixels().enumerate() {
        let [r, g, b] = px.0.map(|c| c as f32);
        let mn = r.min(g).min(b);
        let mx = r.max(g).max(b);
        let sat = (mx - mn) / mx.max(1.0);
        p.d[i] = if sat < 0.30 { mn } else { 0.0 };
    }
    p
}

// ---------------------------------------------------------------- geometry

/// Header label centres (px) of E A D DMG H MIT, filtered like columns_from_header_groups.
pub(super) fn column_centres(board: &DynamicImage) -> Option<[f64; 6]> {
    let w = board.width() as f64;
    let labels: Vec<f64> = preprocess::header_label_groups(board)
        .into_iter()
        .filter(|&(s, e)| (e - s) as f64 / w <= 0.05)
        .map(|(s, e)| (s + e) as f64 / 2.0)
        .collect();
    if labels.len() < 6 {
        return None;
    }
    let mut c = [0f64; 6];
    c.copy_from_slice(&labels[..6]);
    if !c
        .windows(2)
        .all(|p| (0.02..=0.10).contains(&((p[1] - p[0]) / w)))
    {
        return None;
    }
    Some(c)
}

pub(super) fn column_windows(c: &[f64; 6], w: usize) -> [(usize, usize); 6] {
    let mut out = [(0, 0); 6];
    for i in 0..6 {
        let lo = if i > 0 {
            (c[i - 1] + c[i]) / 2.0
        } else {
            c[0] - (c[1] - c[0]) / 2.0
        };
        let hi = if i < 5 {
            (c[i] + c[i + 1]) / 2.0
        } else {
            c[5] + (c[5] - c[4]) / 2.0
        };
        out[i] = (
            pyround(lo.max(0.0)) as usize,
            pyround(hi.min(w as f64)) as usize,
        );
    }
    out
}

/// Tracker row grid as seeds, snapped to text bands of the neutral-ink projection.
pub(super) fn row_bands(
    n: &Plane,
    windows: &[(usize, usize); 6],
    team: usize,
) -> Vec<((usize, usize), bool)> {
    let h = n.h;
    let t1 = (h as f64 * HEADER_RATIO) as usize;
    let t2 = (h as f64 * TEAM2_START_RATIO) as usize;
    let rh = (t2 - t1) / (team + 1);
    let proj: Vec<u32> = (0..h)
        .map(|y| {
            windows
                .iter()
                .flat_map(|&(a, b)| a..b)
                .filter(|&x| n.at(x, y) > 100.0)
                .count() as u32
        })
        .collect();
    let hdr_end = (h as f64 * 0.025).max(15.0) as usize + 2;
    let mut bands = Vec::new();
    let mut start = None;
    for (y, &p) in proj.iter().enumerate().skip(hdr_end) {
        match (p > 0, start) {
            (true, None) => start = Some(y),
            (false, Some(s)) => {
                bands.push((s, y));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        bands.push((s, h));
    }
    let min_h = 3.max(pyround(h as f64 * 6.0 / 1007.0) as usize);
    let max_h = pyround(h as f64 * 40.0 / 1007.0) as usize;
    bands.retain(|&(a, b)| (min_h..=max_h).contains(&(b - a)));
    let mut rows = Vec::with_capacity(2 * team);
    for r in 0..2 * team {
        let base = if r < team { t1 } else { t2 };
        let y0 = base + (r % team) * rh;
        let ctr = y0 as f64 + rh as f64 / 2.0;
        let dist = |b: &(usize, usize)| ((b.0 + b.1) as f64 / 2.0 - ctr).abs();
        let mut best: Option<(usize, usize)> = None;
        for b in bands.iter().filter(|b| dist(b) < rh as f64 * 0.6) {
            if best.is_none_or(|bb| dist(b) < dist(&bb)) {
                best = Some(*b);
            }
        }
        // `true`: no text band near the seed, the tracker crop is used
        rows.push(match best {
            Some(b) => (b, false),
            None => {
                // fall back to the tracker crop with its 15% padding
                let pad = pyround(rh as f64 * 0.15) as usize;
                ((y0 + pad, (y0 + rh - pad).min(h)), true)
            }
        });
    }
    rows
}

// ---------------------------------------------------------------- resampling

/// Pillow `precompute_coeffs` for the bilinear (triangle) filter.
pub(super) fn pil_coeffs(in_size: usize, out_size: usize) -> Vec<(usize, Vec<f64>)> {
    let scale = in_size as f64 / out_size as f64;
    let filterscale = scale.max(1.0);
    let support = filterscale;
    let ss = 1.0 / filterscale;
    (0..out_size)
        .map(|xx| {
            let center = (xx as f64 + 0.5) * scale;
            let xmin = ((center - support + 0.5) as i64).max(0) as usize;
            let xmax = ((center + support + 0.5) as i64).min(in_size as i64) as usize;
            let mut k: Vec<f64> = (xmin..xmax)
                .map(|x| {
                    let t = ((x as f64 - center + 0.5) * ss).abs();
                    if t < 1.0 { 1.0 - t } else { 0.0 }
                })
                .collect();
            let ww: f64 = k.iter().sum();
            if ww != 0.0 {
                k.iter_mut().for_each(|v| *v /= ww);
            }
            (xmin, k)
        })
        .collect()
}

/// Pillow `Image.resize((nw, nh), BILINEAR)` on a mode-F image: horizontal pass, then vertical.
pub(super) fn pil_resize(src: &Plane, nw: usize, nh: usize) -> Plane {
    let hc = pil_coeffs(src.w, nw);
    let mut mid = Plane::zeros(nw, src.h);
    for y in 0..src.h {
        for (x, (x0, k)) in hc.iter().enumerate() {
            let s: f64 = k
                .iter()
                .enumerate()
                .map(|(i, &kv)| src.at(x0 + i, y) as f64 * kv)
                .sum();
            mid.d[y * nw + x] = s as f32;
        }
    }
    let vc = pil_coeffs(src.h, nh);
    let mut out = Plane::zeros(nw, nh);
    for (y, (y0, k)) in vc.iter().enumerate() {
        for x in 0..nw {
            let s: f64 = k
                .iter()
                .enumerate()
                .map(|(i, &kv)| mid.at(x, y0 + i) as f64 * kv)
                .sum();
            out.d[y * nw + x] = s as f32;
        }
    }
    out
}

// ---------------------------------------------------------------- segmentation

pub(super) struct Segment {
    pub(super) ink: Plane,
    pub(super) b: Vec<bool>,
    pub(super) top: usize,
    pub(super) dh: usize,
    pub(super) runs: Vec<(usize, usize)>,
    pub(super) rival: f64,
    /// Brightness peak of the cell before normalising (99.5th percentile).
    pub(super) peak: f64,
    /// Binarised ink exists in the cell to the left of the picked group.
    pub(super) ink_left: bool,
    /// `sep_after[i]` is set when runs[i] and runs[i+1] were joined across a
    /// thousands-separator gap (a comma is inserted there when reading).
    pub(super) sep_after: Vec<bool>,
    /// Ink groups in the cell window (the picked one included).
    pub(super) groups: usize,
}

pub(super) fn percentile_sorted(v: &[f32], q: f64) -> f64 {
    let idx = q / 100.0 * (v.len() - 1) as f64;
    let lo = idx.floor() as usize;
    let hi = idx.ceil() as usize;
    let f = idx - lo as f64;
    v[lo] as f64 + (v[hi] as f64 - v[lo] as f64) * f
}

/// Normalised cell ink map and its raw brightness peak; `None` when the cell
/// holds no contrast.
pub(super) fn cell_ink(
    n: &Plane,
    band: (usize, usize),
    win: (usize, usize),
    pad: usize,
) -> Option<(Plane, f64)> {
    let y0 = band.0.saturating_sub(pad);
    let y1 = (band.1 + pad).min(n.h);
    let (x0, x1) = (win.0.min(n.w), win.1.min(n.w));
    if y1 <= y0 || x1 <= x0 {
        return None;
    }
    let (w, h) = (x1 - x0, y1 - y0);
    let mut c = Plane::zeros(w, h);
    for y in 0..h {
        for x in 0..w {
            c.d[y * w + x] = n.at(x0 + x, y0 + y);
        }
    }
    let mut sorted = c.d.clone();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let bg = percentile_sorted(&sorted, 50.0);
    let peak = percentile_sorted(&sorted, 99.5);
    if peak - bg < 40.0 {
        return None;
    }
    for v in c.d.iter_mut() {
        *v = ((*v as f64 - bg) / (peak - bg)).clamp(0.0, 1.0) as f32;
    }
    Some((c, peak))
}

pub(super) fn runs_of(cols: &[bool]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = None;
    for (x, &v) in cols.iter().chain(std::iter::once(&false)).enumerate() {
        match (v, start) {
            (true, None) => start = Some(x),
            (false, Some(s)) => {
                out.push((s, x));
                start = None;
            }
            _ => {}
        }
    }
    out
}

/// Pick the centred ink group, find the digit band, and list projection runs.
pub(super) fn segment(ink: Plane, peak: f64) -> Option<Segment> {
    let (w, h) = (ink.w, ink.h);
    let b: Vec<bool> = ink.d.iter().map(|&v| v > INK_THR).collect();
    let col_any: Vec<bool> = (0..w).map(|x| (0..h).any(|y| b[y * w + x])).collect();
    let runs = runs_of(&col_any);
    if runs.is_empty() {
        return None;
    }
    let run_h = |&(s, e): &(usize, usize)| {
        let rows: Vec<usize> = (0..h).filter(|&y| (s..e).any(|x| b[y * w + x])).collect();
        rows.last().map_or(0, |l| l - rows[0] + 1)
    };
    let hmax = runs.iter().map(run_h).max().unwrap_or(0);
    let gap_max = (GAP_K * hmax as f64).max(2.0);
    let mut groups: Vec<Vec<usize>> = vec![vec![0]];
    // Original run-index pairs joined only because of a thousands separator.
    let mut sep_bridges: Vec<(usize, usize)> = Vec::new();
    for i in 1..runs.len() {
        let last = *groups.last().unwrap().last().unwrap();
        let gap = (runs[i].0 - runs[last].1) as f64;
        if gap <= gap_max {
            groups.last_mut().unwrap().push(i);
        } else if is_separator_gap(gap, hmax, &b, w, h, runs[last].1, runs[i].0) {
            groups.last_mut().unwrap().push(i);
            sep_bridges.push((last, i));
        } else {
            groups.push(vec![i]);
        }
    }
    let gscore = |g: &Vec<usize>| {
        let (s, e) = (runs[g[0]].0, runs[*g.last().unwrap()].1);
        let mass: f64 = (0..h)
            .map(|y| (s..e).map(|x| ink.at(x, y) as f64).sum::<f64>())
            .sum();
        mass / (1.0 + ((s + e) as f64 / 2.0 - w as f64 / 2.0).abs() / (0.25 * w as f64))
    };
    let mut gsc: Vec<(f64, &Vec<usize>)> = groups.iter().map(|g| (gscore(g), g)).collect();
    gsc.sort_by(|a, b| b.0.total_cmp(&a.0));
    let g = gsc[0].1;
    let (gx0, gx1) = (runs[g[0]].0, runs[*g.last().unwrap()].1);
    let rs: Vec<u32> = (0..h)
        .map(|y| (gx0..gx1).filter(|&x| b[y * w + x]).count() as u32)
        .collect();
    let rmax = *rs.iter().max().unwrap_or(&0) as f64;
    let rows: Vec<usize> = (0..h).filter(|&y| rs[y] as f64 >= 0.25 * rmax).collect();
    let top = *rows.first()?;
    let bot = *rows.last()? + 1;
    let dh = bot - top;
    if dh < 4 {
        return None;
    }
    let rival = if gsc.len() > 1 {
        gsc[1].0 / gsc[0].0
    } else {
        0.0
    };
    let ink_left = (0..gx0).any(|x| (0..h).any(|y| b[y * w + x]));
    let picked: Vec<(usize, usize)> = g.iter().map(|&i| runs[i]).collect();
    let mut sep_after = vec![false; picked.len().saturating_sub(1)];
    for &(a, b_idx) in &sep_bridges {
        if let (Some(ia), Some(ib)) = (
            g.iter().position(|&x| x == a),
            g.iter().position(|&x| x == b_idx),
        ) && ib == ia + 1
        {
            sep_after[ia] = true;
        }
    }
    Some(Segment {
        ink,
        b,
        top,
        dh,
        runs: picked,
        rival,
        peak,
        ink_left,
        sep_after,
        groups: groups.len(),
    })
}

/// A gap wider than [`GAP_K`] that should still keep digit groups together.
///
/// True when the gap width is in the thousands-separator range
/// (`gap_max`, [`SEP_GAP_K`] * hmax], or when the gap's binarised ink is
/// bottom-heavy like a comma (more ink in the lower half than the upper).
pub(super) fn is_separator_gap(
    gap: f64,
    hmax: usize,
    b: &[bool],
    w: usize,
    h: usize,
    x0: usize,
    x1: usize,
) -> bool {
    let sep_hi = (SEP_GAP_K * hmax as f64).max(2.0);
    if gap <= sep_hi {
        return true;
    }
    if x1 <= x0 || h < 2 {
        return false;
    }
    let mid = h / 2;
    let mut top = 0usize;
    let mut bot = 0usize;
    for y in 0..h {
        let row = y * w;
        let n = (x0..x1).filter(|&x| b[row + x]).count();
        if y < mid {
            top += n;
        } else {
            bot += n;
        }
    }
    // Comma: some ink, mostly in the lower half, gap not enormous.
    bot > 0 && bot >= top.saturating_mul(2) && gap <= (2.0 * hmax as f64).max(4.0)
}

/// Rows [top, top + DESC*dh) scaled so the strip is CH rows tall; same factor horizontally.
pub(super) fn scaled_strip(ink: &Plane, top: usize, dh: usize) -> (Plane, f64) {
    let y1 = ink.h.min(top + pyround(DESC * dh as f64) as usize);
    let gh = y1 - top;
    let mut g = Plane::zeros(ink.w, gh);
    g.d.copy_from_slice(&ink.d[top * ink.w..y1 * ink.w]);
    let sc = CH as f64 / (DESC * dh as f64);
    let nh = (pyround(gh as f64 * sc) as usize).max(1);
    let nw = (pyround(g.w as f64 * sc) as usize).max(1);
    let a = pil_resize(&g, nw, nh);
    let mut out = Plane::zeros(nw, CH);
    let rows = nh.min(CH);
    out.d[..rows * nw].copy_from_slice(&a.d[..rows * nw]);
    (out, sc)
}

pub(super) fn piece_canvas(strip: &Plane, sc: f64, x0: usize, x1: usize) -> [f32; CANVAS] {
    let xa = (pyround(x0 as f64 * sc) as usize).min(strip.w);
    let xb = ((pyround(x1 as f64 * sc) as usize).max(xa + 1)).min(strip.w);
    let mut ga = xa;
    let mut w = xb.saturating_sub(xa);
    if w > CW {
        ga += (w - CW) / 2;
        w = CW;
    }
    let off = (CW - w) / 2;
    let mut out = [0f32; CANVAS];
    for y in 0..CH {
        for x in 0..w {
            out[y * CW + off + x] = strip.at(ga + x, y);
        }
    }
    out
}

/// Interior cut positions in run [s,e): low-ink valley columns and their right neighbours.
pub(super) fn candidate_cuts(colsum: &[u32], s: usize, e: usize, dh: usize) -> Vec<usize> {
    let cs = &colsum[s..e];
    let lim = (0.40 * dh as f64).max(2.0);
    let mut cuts = std::collections::BTreeSet::new();
    for i in 1..(e - s).saturating_sub(1) {
        if cs[i] as f64 <= lim && cs[i] <= cs[i - 1] && cs[i] <= cs[i + 1] {
            cuts.insert(s + i);
            cuts.insert(s + i + 1);
        }
    }
    cuts.into_iter()
        .filter(|&c| s + 2 <= c && c + 2 <= e)
        .collect()
}

pub(super) fn partitions(s: usize, e: usize, cuts: &[usize]) -> Vec<Vec<(usize, usize)>> {
    const MAX_PIECES: usize = 4;
    const MIN_W: usize = 2;
    fn rec(
        start: usize,
        e: usize,
        cuts: &[usize],
        acc: &mut Vec<(usize, usize)>,
        out: &mut Vec<Vec<(usize, usize)>>,
    ) {
        if acc.len() >= MAX_PIECES {
            return;
        }
        for &c in cuts.iter().chain(std::iter::once(&e)) {
            if c <= start || c - start < MIN_W {
                continue;
            }
            acc.push((start, c));
            if c == e {
                out.push(acc.clone());
            } else {
                rec(c, e, cuts, acc, out);
            }
            acc.pop();
        }
    }
    let mut out = Vec::new();
    rec(s, e, cuts, &mut Vec::new(), &mut out);
    if out.is_empty() {
        out.push(vec![(s, e)]);
    }
    out
}

pub(super) enum RunKind {
    /// Explicit partitions, as lists of piece ids.
    Enum(Vec<Vec<usize>>),
    /// Touching glyphs: for each end cut (ascending [`cut_key`]), the pieces
    /// (id, start cut key) ending there.
    Dp(Vec<(usize, Vec<(usize, usize)>)>),
}

/// DP node key of a cut line through column boundary `x` (at the text band's
/// middle row): `kind` 0 is vertical, `k > 0` uses slope `SLANTS[k - 1]`.
/// Keys sort by `x` first.
pub(super) fn cut_key(x: usize, kind: usize) -> usize {
    x * (SLANTS.len() + 1) + kind
}

pub(super) struct Run {
    pub(super) s: usize,
    pub(super) e: usize,
    pub(super) kind: RunKind,
}

pub(super) struct Cell {
    pub(super) runs: Vec<Run>,
    /// Piece column ranges `[x0, x1)` in cell pixels, by piece id.
    pub(super) pieces: Vec<(usize, usize)>,
    /// Scaled strip and scale the canvases were cut from.
    pub(super) strip: Plane,
    pub(super) sc: f64,
    /// Binarised ink pixels per cell column.
    pub(super) colsum: Vec<u32>,
    /// Piece `p` was cut along a slanted line (masked; `pieces[p]` is its ink extent).
    pub(super) slanted: Vec<bool>,
    /// Normalised piece canvases, `CANVAS` floats per piece.
    pub(super) v: Vec<f32>,
    pub(super) mass: Vec<f64>,
    pub(super) ar: Vec<f64>,
    pub(super) total_mass: f64,
    /// Text height the canvases were scaled for (after the row consensus).
    pub(super) dh: usize,
    pub(super) rival: f64,
    pub(super) peak: f64,
    pub(super) ink_left: bool,
    /// Run index of each piece (for inserting commas at separator bridges).
    pub(super) piece_run: Vec<usize>,
    /// Copied from [`Segment::sep_after`].
    pub(super) sep_after: Vec<bool>,
    /// Copied from [`Segment::groups`].
    pub(super) groups: usize,
    /// Text band top (after the row consensus), in cell rows.
    pub(super) top: usize,
    /// The cell's normalised ink and its binarised pixels (> [`INK_THR`]),
    /// cell pixels (blanker addition).
    pub(super) ink: Plane,
    pub(super) b: Vec<bool>,
}

/// Ink map and segmentation for one cell.
pub(super) fn segment_cell(
    n: &Plane,
    band: (usize, usize),
    win: (usize, usize),
    pad: usize,
) -> Option<Segment> {
    let (ink, peak) = cell_ink(n, band, win, pad)?;
    segment(ink, peak)
}

/// Row consensus text band `(top, dh)` from the cells' own estimates.
///
/// `segment` finds the text band from rows holding at least 25% of the
/// widest row's ink. A lone '4' is the weak case: its crossbar row is ~10 px
/// wide at 1440p and the stem below it 2 to 3 px, so depending on sub-pixel
/// phase the stem rows fall under the cut and `dh` shrinks from 13 to 9 or 10.
/// The 10 px wide glyph then looks wider than `WIDE_RUN * dh`, is split as
/// touching glyphs, and reads "16" or "311". All cells of a row share one
/// font size and baseline, so the lower median of `dh` over the row (and the
/// median `top` of the cells within `BAND_TOL` of it) is a safe reference.
///
/// [`apply_row_band`] uses it in both directions: a cell `BAND_TOL` or more
/// shorter or taller than the row gets the row's band. Taller cells are kept
/// in on purpose; on live boards that correction only raised confidence.
/// With fewer than `MIN_BAND_CELLS` segmented cells there is no consensus and
/// every cell keeps its own measurement.
pub(super) fn row_text_band(segs: &[Option<Segment>]) -> Option<(usize, usize)> {
    let mut dhs: Vec<usize> = segs.iter().flatten().map(|s| s.dh).collect();
    if dhs.len() < MIN_BAND_CELLS {
        return None;
    }
    dhs.sort_unstable();
    let med = dhs[(dhs.len() - 1) / 2];
    let mut tops: Vec<usize> = segs
        .iter()
        .flatten()
        .filter(|s| s.dh.abs_diff(med) < BAND_TOL)
        .map(|s| s.top)
        .collect();
    tops.sort_unstable();
    Some((tops[(tops.len() - 1) / 2], med))
}

/// Replace a cell's band by the row's when its own height is off by `BAND_TOL` or more.
pub(super) fn apply_row_band(seg: &mut Segment, row: Option<(usize, usize)>) {
    if let Some((top, dh)) = row
        && seg.dh.abs_diff(dh) >= BAND_TOL
        && top < seg.ink.h
    {
        seg.top = top;
        seg.dh = dh;
    }
}

/// Template-independent work for one cell: all candidate piece canvases.
pub(super) fn prepare_cell(seg: Segment) -> Option<Cell> {
    let dh = seg.dh;
    let (strip, sc) = scaled_strip(&seg.ink, seg.top, dh);
    let w = seg.ink.w;
    let colsum: Vec<u32> = (0..w)
        .map(|x| (0..seg.ink.h).filter(|&y| seg.b[y * w + x]).count() as u32)
        .collect();
    let mut pieces: Vec<(usize, usize)> = Vec::new();
    let mut piece_run: Vec<usize> = Vec::new();
    let mut index = std::collections::HashMap::new();
    let mut pid = |x0: usize, x1: usize, run_i: usize, piece_run: &mut Vec<usize>| {
        *index.entry((x0, x1)).or_insert_with(|| {
            pieces.push((x0, x1));
            piece_run.push(run_i);
            pieces.len() - 1
        })
    };
    let mut runs = Vec::new();
    for (run_i, &(rs, re)) in seg.runs.iter().enumerate() {
        if (re - rs) as f64 > WIDE_RUN * dh as f64 {
            let maxw = (MAX_GLYPH * dh as f64).ceil() as usize;
            let mut ends = Vec::new();
            for x1 in rs + 2..=re {
                if x1 != re && x1 + 2 > re {
                    continue;
                }
                let starts: Vec<(usize, usize)> = (rs.max(x1.saturating_sub(maxw))
                    ..x1.saturating_sub(1))
                    .filter(|&x0| x0 == rs || x0 >= rs + 2)
                    .map(|x0| (pid(x0, x1, run_i, &mut piece_run), cut_key(x0, 0)))
                    .collect();
                ends.push((cut_key(x1, 0), starts));
            }
            runs.push(Run {
                s: rs,
                e: re,
                kind: RunKind::Dp(ends),
            });
        } else {
            let cuts = candidate_cuts(&colsum, rs, re, dh);
            let parts = partitions(rs, re, &cuts)
                .into_iter()
                .map(|p| {
                    p.into_iter()
                        .map(|(a, b)| pid(a, b, run_i, &mut piece_run))
                        .collect()
                })
                .collect();
            runs.push(Run {
                s: rs,
                e: re,
                kind: RunKind::Enum(parts),
            });
        }
    }
    let mut v = Vec::with_capacity(pieces.len() * CANVAS);
    for &(x0, x1) in &pieces {
        let mut c = piece_canvas(&strip, sc, x0, x1);
        normalise(&mut c);
        v.extend_from_slice(&c);
    }
    let mut cb = vec![0u64; w + 1];
    for x in 0..w {
        cb[x + 1] = cb[x] + colsum[x] as u64;
    }
    let mass: Vec<f64> = pieces
        .iter()
        .map(|&(a, b)| (cb[b] - cb[a]) as f64)
        .collect();
    let ar: Vec<f64> = pieces
        .iter()
        .map(|&(a, b)| (b - a) as f64 / dh as f64)
        .collect();
    let total_mass = seg.runs.iter().map(|&(a, e)| (cb[e] - cb[a]) as f64).sum();
    let mut set = PieceSet {
        slanted: vec![false; pieces.len()],
        pieces,
        piece_run,
        v,
        mass,
        ar,
    };
    for (run_i, run) in runs.iter_mut().enumerate() {
        add_slanted_pieces(&seg, run, &mut set, run_i);
    }
    let PieceSet {
        pieces,
        piece_run,
        slanted,
        v,
        mass,
        ar,
    } = set;
    Some(Cell {
        runs,
        pieces,
        slanted,
        strip,
        sc,
        colsum,
        v,
        mass,
        ar,
        total_mass,
        dh,
        rival: seg.rival,
        peak: seg.peak,
        ink_left: seg.ink_left,
        piece_run,
        sep_after: seg.sep_after,
        groups: seg.groups,
        top: seg.top,
        ink: seg.ink,
        b: seg.b,
    })
}

/// Piece list under construction: ranges, canvases, ink and width ratios.
pub(super) struct PieceSet {
    pub(super) pieces: Vec<(usize, usize)>,
    pub(super) piece_run: Vec<usize>,
    pub(super) slanted: Vec<bool>,
    pub(super) v: Vec<f32>,
    pub(super) mass: Vec<f64>,
    pub(super) ar: Vec<f64>,
}

/// Add slanted cuts to a touching-glyph run (`RunKind::Dp`).
///
/// A cut through `x` with slope `m` runs along `x + m * (y + 0.5 - mid)`;
/// pixel `(px, y)` is left of it when `px + 0.5` is. Only cuts that cross at
/// most `SLANT_MAX_INK` ink pixels, and fewer than the vertical cut through
/// the same point, are added. Every piece between two cuts where at least one
/// is slanted holds only the ink between the lines; its canvas is cut from a
/// strip of that masked ink over its own ink extent, like an isolated glyph.
/// A masked piece that holds the same ink as a vertical piece reuses it.
pub(super) fn add_slanted_pieces(seg: &Segment, run: &mut Run, set: &mut PieceSet, run_i: usize) {
    let RunKind::Dp(ends) = &mut run.kind else {
        return;
    };
    let (rs, re, dh, w) = (run.s, run.e, seg.dh, seg.ink.w);
    let maxw = (MAX_GLYPH * dh as f64).ceil() as usize;
    let mid = seg.top as f64 + dh as f64 / 2.0;
    let line = |x: usize, kind: usize, y: usize| {
        let m = if kind == 0 { 0.0 } else { SLANTS[kind - 1] };
        x as f64 + m * (y as f64 + 0.5 - mid)
    };
    let rows: Vec<usize> = (0..seg.ink.h)
        .filter(|&y| (rs..re).any(|x| seg.b[y * w + x]))
        .collect();
    let crossed = |x: usize, kind: usize| {
        rows.iter()
            .filter(|&&y| {
                let c = line(x, kind, y).floor();
                c >= rs as f64 && c < re as f64 && seg.b[y * w + c as usize]
            })
            .count()
    };
    let mut cuts: Vec<(usize, usize)> = vec![(rs, 0), (re, 0)];
    cuts.extend((rs + 2..=re.saturating_sub(2)).map(|x| (x, 0)));
    let mut any = false;
    for x in rs + 2..re.saturating_sub(1) {
        let vertical = crossed(x, 0);
        for kind in 1..=SLANTS.len() {
            let c = crossed(x, kind);
            if c < vertical && c <= SLANT_MAX_INK {
                cuts.push((x, kind));
                any = true;
            }
        }
    }
    if !any {
        return;
    }
    let mut seen: std::collections::HashMap<Vec<u32>, usize> = std::collections::HashMap::new();
    let mut extra: std::collections::BTreeMap<usize, Vec<(usize, usize)>> = Default::default();
    for &(xb, kb) in &cuts {
        for &(xa, ka) in &cuts {
            if (ka == 0 && kb == 0) || xa >= xb || xb == rs || xa == re {
                continue;
            }
            let narrowest = rows
                .iter()
                .map(|&y| line(xb, kb, y) - line(xa, ka, y))
                .fold(f64::INFINITY, f64::min);
            if narrowest < 2.0 {
                continue;
            }
            let inside = |x: usize, y: usize| {
                let px = x as f64 + 0.5;
                px >= line(xa, ka, y) && px < line(xb, kb, y)
            };
            let mut masked = Plane::zeros(w, seg.ink.h);
            let (mut e0, mut e1, mut ink) = (usize::MAX, 0, 0usize);
            for y in 0..seg.ink.h {
                for x in rs..re {
                    if !inside(x, y) {
                        continue;
                    }
                    let v = seg.ink.at(x, y);
                    masked.d[y * w + x] = v;
                    ink += seg.b[y * w + x] as usize;
                    if v > SLANT_EXTENT_INK {
                        e0 = e0.min(x);
                        e1 = e1.max(x + 1);
                    }
                }
            }
            if ink == 0 || e1 <= e0 + 1 || e1 - e0 > maxw {
                continue;
            }
            // same ink as the vertical piece over its extent: reuse that piece
            let uncut =
                (e0..e1).all(|x| (0..seg.ink.h).all(|y| inside(x, y) || seg.ink.at(x, y) <= 0.0));
            let vertical = uncut
                .then(|| {
                    set.pieces
                        .iter()
                        .zip(&set.slanted)
                        .position(|(&p, &sl)| !sl && p == (e0, e1))
                })
                .flatten();
            let sig: Vec<u32> = (0..seg.ink.h)
                .flat_map(|y| (e0..e1).map(move |x| (x, y)))
                .map(|(x, y)| masked.d[y * w + x].to_bits())
                .chain([e0 as u32, e1 as u32])
                .collect();
            let id = match vertical.or_else(|| seen.get(&sig).copied()) {
                Some(id) => id,
                None => {
                    let (strip, sc) = scaled_strip(&masked, seg.top, dh);
                    let mut c = piece_canvas(&strip, sc, e0, e1);
                    normalise(&mut c);
                    set.v.extend_from_slice(&c);
                    set.pieces.push((e0, e1));
                    set.piece_run.push(run_i);
                    set.slanted.push(true);
                    set.mass.push(ink as f64);
                    set.ar.push((e1 - e0) as f64 / dh as f64);
                    seen.insert(sig, set.pieces.len() - 1);
                    set.pieces.len() - 1
                }
            };
            extra
                .entry(cut_key(xb, kb))
                .or_default()
                .push((id, cut_key(xa, ka)));
        }
    }
    for (key, starts) in extra {
        match ends.iter_mut().find(|e| e.0 == key) {
            Some(e) => e.1.extend(starts),
            None => ends.push((key, starts)),
        }
    }
    ends.sort_by_key(|e| e.0);
}

pub(super) type Scored = (f64, Vec<usize>);

/// True when `ink` binarised pixels are too few for `digits` glyphs of text
/// height `dh` (see `MIN_INK_PER_GLYPH`).
pub(super) fn too_little_ink(ink: f64, digits: usize, dh: usize) -> bool {
    ink < MIN_INK_PER_GLYPH * (digits * dh * dh) as f64
}

pub(super) fn sort_desc(v: &mut [Scored]) {
    v.sort_by(|a, b| b.0.total_cmp(&a.0));
}

/// Top-K partitions of a run, scored with `piece_score` (already includes -SPLIT_PEN).
pub(super) fn kbest_run(run: &Run, piece_score: &[f64], k: usize) -> Vec<Scored> {
    match &run.kind {
        RunKind::Enum(parts) => {
            let mut sc: Vec<Scored> = parts
                .iter()
                .map(|pp| (pp.iter().map(|&p| piece_score[p]).sum(), pp.clone()))
                .collect();
            sort_desc(&mut sc);
            sc.truncate(k);
            sc
        }
        RunKind::Dp(ends) => {
            let mut best: std::collections::HashMap<usize, Vec<Scored>> =
                std::collections::HashMap::new();
            best.insert(cut_key(run.s, 0), vec![(0.0, Vec::new())]);
            for (x1, starts) in ends {
                let mut cand: Vec<Scored> = Vec::new();
                for &(p, x0) in starts {
                    if let Some(prev) = best.get(&x0) {
                        for (sc0, acc) in prev {
                            let mut a = acc.clone();
                            a.push(p);
                            cand.push((sc0 + piece_score[p], a));
                        }
                    }
                }
                if !cand.is_empty() {
                    sort_desc(&mut cand);
                    // slanted and vertical cuts can reach the same pieces
                    let mut seen = std::collections::HashSet::new();
                    cand.retain(|c| seen.insert(c.1.clone()));
                    cand.truncate(k);
                    best.insert(*x1, cand);
                }
            }
            best.remove(&cut_key(run.e, 0)).unwrap_or_default()
        }
    }
}

pub(super) fn thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// Kill columns (E/A/D) are plain digits; DMG/H/MIT use thousands groups.
pub(super) fn plain_field(k: usize) -> bool {
    k < 3
}

pub(super) fn grammar_ok(t: &str, k: usize) -> bool {
    let d: String = t.chars().filter(|&c| c != ',').collect();
    if d.is_empty() || (d.len() > 1 && d.starts_with('0')) {
        return false;
    }
    if plain_field(k) {
        return !t.contains(',') && d.len() <= 3;
    }
    d.len() <= 6 && d.parse::<u64>().is_ok_and(|n| thousands(n) == t)
}

/// Canvas of piece `p` with edge columns holding at most one ink pixel
/// dropped. Anti-aliasing can leave one stray pixel beside a glyph (a dim 0's
/// right flank at 1080p); it widens the piece by a column and shifts the
/// glyph off centre. Only the pair checks use this; matching keeps the
/// canvases the templates were built from.
pub(super) fn recentred_canvas(cell: &Cell, p: usize) -> [f32; CANVAS] {
    if cell.slanted[p] {
        // already cut to its own ink extent
        let mut c = [0f32; CANVAS];
        c.copy_from_slice(&cell.v[p * CANVAS..(p + 1) * CANVAS]);
        return c;
    }
    let (mut x0, mut x1) = cell.pieces[p];
    while x1 - x0 > 2 && cell.colsum[x0] <= 1 {
        x0 += 1;
    }
    while x1 - x0 > 2 && cell.colsum[x1 - 1] <= 1 {
        x1 -= 1;
    }
    let mut c = piece_canvas(&cell.strip, cell.sc, x0, x1);
    normalise(&mut c);
    c
}

/// Correlation of a canvas with `t[a] - t[b]` over only the pixels where the
/// two templates differ most (`PAIR_MASK`). For 4 vs 6 that is the 4's open
/// top-left and stem against the 6's curved top-left and closed bottom loop;
/// for 0 vs 3 the 0's closed left side and hollow centre against the 3's
/// open left and middle bar. Positive favours `a`; it is antisymmetric, so a
/// real 6 scores as negative as a real 4 scores positive.
pub(super) fn pair_check(v: &[f32; CANVAS], tpl: &Templates, a: usize, b: usize) -> f64 {
    let d: Vec<f64> = (0..CANVAS)
        .map(|i| tpl.t[a][i] as f64 - tpl.t[b][i] as f64)
        .collect();
    let lim = PAIR_MASK * d.iter().fold(0f64, |m, x| m.max(x.abs()));
    let idx: Vec<usize> = (0..CANVAS).filter(|&i| d[i].abs() >= lim).collect();
    let n = idx.len().max(1) as f64;
    let vm = idx.iter().map(|&i| v[i] as f64).sum::<f64>() / n;
    let dm = idx.iter().map(|&i| d[i]).sum::<f64>() / n;
    let (mut num, mut vv, mut dd) = (0f64, 0f64, 0f64);
    for &i in &idx {
        let (x, y) = (v[i] as f64 - vm, d[i] - dm);
        num += x * y;
        vv += x * x;
        dd += y * y;
    }
    num / (vv.sqrt() * dd.sqrt()).max(1e-9)
}

/// Calibrated margin from a pair check, for a glyph read as `best` whose
/// shape runner-up is `second`, when a check applies:
/// * a 4 next to a 6, or next to a 1 when the piece is too wide to be a 1
///   (the 4's tall stem alone resembles a 1), is checked against the 6;
/// * a 0 next to a 3 is checked only when the cell is dim, since zeros are
///   drawn in grey; a bright cell read as 0 gets no help.
pub(super) fn pair_margin(
    cell: &Cell,
    tpl: &Templates,
    p: usize,
    best: usize,
    second: usize,
    dim: bool,
) -> Option<f64> {
    let other = match (best, second) {
        (4, 6) => 6,
        (4, 1) if cell.ar[p] > tpl.ar_hi[1] => 6,
        (0, 3) if dim => 3,
        _ => return None,
    };
    Some(pair_check(&recentred_canvas(cell, p), tpl, best, other) / PAIR_TYP)
}

/// Value, margin and whether any sanity flag fired.
pub(super) struct Reading {
    pub(super) value: Option<u32>,
    pub(super) margin: f64,
    pub(super) flagged: bool,
    /// The chosen reading's glyphs, left to right (blanker addition).
    pub(super) pieces: Vec<PieceRead>,
    /// The chosen reading's text, and whether the field grammar accepts it
    /// (blanker addition).
    pub(super) text: String,
    pub(super) grammar: bool,
}

/// One glyph of the chosen reading (blanker addition).
#[derive(Clone, Debug)]
pub(super) struct PieceRead {
    /// Piece id in [`Cell::pieces`].
    pub(super) id: usize,
    /// Cell column range `[x0, x1)`; a slanted piece's ink extent.
    pub(super) x0: usize,
    pub(super) x1: usize,
    /// Template class (0-9, [`COMMA`]).
    pub(super) class: usize,
    /// Width-gated score of `class`, and the best other class's score.
    pub(super) score: f64,
    pub(super) second: f64,
    pub(super) second_class: usize,
    pub(super) slanted: bool,
}

/// `dim`: the cell is much darker than the board's bright cells (see
/// `DIM_RATIO`).
pub(super) fn read_cell(cell: Option<&Cell>, tpl: &Templates, k: usize, dim: bool) -> Reading {
    let Some(cell) = cell else {
        return Reading {
            value: None,
            margin: 0.0,
            flagged: true,
            pieces: Vec::new(),
            text: String::new(),
            grammar: false,
        };
    };
    let np = cell.ar.len();
    let mut allowed = [true; NCLS];
    if plain_field(k) {
        allowed[COMMA] = false;
    }
    let mut best_c = vec![0usize; np];
    let mut best_s = vec![0f64; np];
    let mut second_s = vec![0f64; np];
    let mut second_c = vec![0usize; np];
    for p in 0..np {
        let v = &cell.v[p * CANVAS..(p + 1) * CANVAS];
        let s: [f64; NCLS] = std::array::from_fn(|c| {
            v.iter()
                .zip(tpl.t[c].iter())
                .map(|(&a, &b)| a as f64 * b as f64)
                .sum()
        });
        let ar = cell.ar[p];
        let sw: [f64; NCLS] = std::array::from_fn(|c| {
            if allowed[c] && ar >= tpl.ar_lo[c] && ar <= tpl.ar_hi[c] {
                s[c]
            } else {
                -1.0
            }
        });
        let mut bc = 0;
        for c in 1..NCLS {
            if sw[c] > sw[bc] {
                bc = c;
            }
        }
        best_c[p] = bc;
        best_s[p] = sw[bc];
        // second best: any other allowed class, ignoring the width gate (a shape margin)
        second_s[p] = f64::NEG_INFINITY;
        for c in (0..NCLS).filter(|&c| c != bc) {
            let sc = if allowed[c] { s[c] } else { -1.0 };
            if sc > second_s[p] {
                second_s[p] = sc;
                second_c[p] = c;
            }
        }
    }
    let total_mass = cell.total_mass.max(1e-9);
    let piece_score: Vec<f64> = (0..np)
        .map(|p| best_s[p] * cell.mass[p] / total_mass - SPLIT_PEN)
        .collect();
    let mut beam: Vec<Scored> = vec![(SPLIT_PEN * cell.runs.len() as f64, Vec::new())];
    for run in &cell.runs {
        let opts = kbest_run(run, &piece_score, KRUN);
        if opts.is_empty() {
            continue;
        }
        let mut next = Vec::with_capacity(beam.len() * opts.len());
        for (b0, a0) in &beam {
            for (o0, p0) in &opts {
                let mut a = a0.clone();
                a.extend_from_slice(p0);
                next.push((b0 + o0, a));
            }
        }
        sort_desc(&mut next);
        next.truncate(BEAM);
        beam = next;
    }
    let text = |a: &[usize]| -> String {
        let mut out = String::new();
        let mut prev_run: Option<usize> = None;
        for &p in a {
            let r = cell.piece_run[p];
            if let Some(pr) = prev_run
                && r > pr
            {
                for br in pr..r {
                    if cell.sep_after.get(br) == Some(&true) {
                        out.push(',');
                    }
                }
            }
            if best_c[p] == COMMA {
                out.push(',');
            } else {
                out.push(char::from(b'0' + best_c[p] as u8));
            }
            prev_run = Some(r);
        }
        out
    };
    let good: Vec<&Scored> = beam
        .iter()
        .filter(|(_, a)| grammar_ok(&text(a), k))
        .collect();
    let grammar = !good.is_empty();
    let pool: Vec<&Scored> = if grammar { good } else { beam.iter().collect() };
    let (obj, acc) = (pool[0].0, &pool[0].1);
    let s = text(acc);
    let digits: String = s.chars().filter(|&c| c != ',').collect();
    let alt = pool[1..]
        .iter()
        .find(|(_, a)| text(a).replace(',', "") != digits)
        .map(|(o, _)| *o);
    // glyph margin in units of that digit's typical margin; a glyph below the
    // suspect line can be lifted by its pair check (never lowered)
    let glyph_margin = |p: usize| {
        let raw = (best_s[p] - second_s[p]) / tpl.typ[best_c[p]];
        if (PAIR_FLOOR..SUSPECT_CONF as f64).contains(&raw)
            && let Some(m) = pair_margin(cell, tpl, p, best_c[p], second_c[p], dim)
        {
            raw.max(m)
        } else {
            raw
        }
    };
    let gm = acc
        .iter()
        .map(|&p| glyph_margin(p))
        .fold(f64::INFINITY, f64::min);
    let gm = if acc.is_empty() { 0.0 } else { gm };
    // partition ambiguity: best alternative reading with a different digit string
    let pm = alt.map_or(1.0, |a| obj - a);
    let margin = gm.min(2.0 * pm / PM_REF);
    let pieces = acc
        .iter()
        .map(|&p| PieceRead {
            id: p,
            x0: cell.pieces[p].0,
            x1: cell.pieces[p].1,
            class: best_c[p],
            score: best_s[p],
            second: second_s[p],
            second_class: second_c[p],
            slanted: cell.slanted[p],
        })
        .collect();
    if digits.is_empty() {
        return Reading {
            value: None,
            margin: 0.0,
            flagged: true,
            pieces,
            text: s,
            grammar,
        };
    }
    let parsed = digits.parse::<u64>().ok();
    let nd = digits.len();
    let mut flagged = acc.iter().any(|&p| best_s[p] <= -1.0) || !grammar;
    flagged |= if plain_field(k) { nd > 2 } else { nd > 6 };
    flagged |= nd > 1 && digits.starts_with('0');
    if !plain_field(k) {
        flagged |= parsed.is_none_or(|n| thousands(n) != s);
    }
    flagged |= s.starts_with(',') || s.ends_with(',');
    flagged |= cell.rival > 0.5;
    flagged |= cell.ink_left;
    flagged |= obj < 0.55;
    // Too little ink for the digits read: a dim 0 half wiped out by
    // compression reads as a thin "1" with a clean margin.
    flagged |= too_little_ink(cell.total_mass, nd, cell.dh);
    let value = parsed.and_then(|n| u32::try_from(n).ok());
    Reading {
        flagged: flagged || value.is_none(),
        value,
        margin,
        pieces,
        text: s,
        grammar,
    }
}

/// Brightness of the board's bright cells: the 90th percentile of the
/// segmented cells' peaks (white digits; zeros are drawn darker).
pub(super) fn bright_peak(rows: &[[Option<Segment>; 6]]) -> Option<f64> {
    let mut pk: Vec<f64> = rows.iter().flatten().flatten().map(|s| s.peak).collect();
    if pk.is_empty() {
        return None;
    }
    pk.sort_by(f64::total_cmp);
    Some(pk[(pk.len() - 1) * 9 / 10])
}

// ---------------------------------------------------------------- board read (blanker addition)

/// One stat cell as the reader saw it.
pub(super) struct CellDetail {
    /// Board pixel of the cell plane's (0, 0): `(window start, band top - pad)`.
    pub(super) origin: (usize, usize),
    /// The prepared cell (`None`: no contrast or no ink group).
    pub(super) cell: Option<Cell>,
    pub(super) reading: Reading,
    pub(super) confidence: f32,
    pub(super) suspect: bool,
}

/// Every cell of a board, read exactly like `shadow::digits::read_board`.
pub(super) struct BoardDetail {
    pub(super) centres: [f64; 6],
    pub(super) windows: [(usize, usize); 6],
    /// Text band per row and whether it is the tracker-grid fallback.
    pub(super) bands: Vec<((usize, usize), bool)>,
    pub(super) rows: Vec<[CellDetail; 6]>,
    /// Neutral ink of the whole board (see [`neutral_ink`]).
    pub(super) ink: Plane,
}

/// `read_board` without the time budget, keeping the per-cell detail.
/// `None` when the board is too small or its header labels are not found.
pub(super) fn read_board_detail(board: &DynamicImage, team_size: usize) -> Option<BoardDetail> {
    if !(5..=6).contains(&team_size) || board.height() < MIN_BOARD_H {
        return None;
    }
    let h = board.height();
    let tpl = templates_for(h);
    let centres = column_centres(board)?;
    let n = neutral_ink(board);
    let wins = column_windows(&centres, n.w);
    let bands = row_bands(&n, &wins, team_size);
    let pad = (pyround(4.0 * h as f64 / REF_H_1440 as f64) as usize).max(1);
    let mut seg_rows: Vec<[Option<Segment>; 6]> = bands
        .iter()
        .map(|&(band, _)| std::array::from_fn(|k| segment_cell(&n, band, wins[k], pad)))
        .collect();
    let bright = bright_peak(&seg_rows);
    let mut rows = Vec::with_capacity(seg_rows.len());
    for (segs, &(band, _)) in seg_rows.iter_mut().zip(&bands) {
        let row_band = row_text_band(segs);
        let cells: [CellDetail; 6] = std::array::from_fn(|k| {
            let cell = segs[k].take().and_then(|mut seg| {
                apply_row_band(&mut seg, row_band);
                prepare_cell(seg)
            });
            let dim = cell
                .as_ref()
                .is_some_and(|c| bright.is_some_and(|b| c.peak < DIM_RATIO * b));
            let reading = read_cell(cell.as_ref(), tpl, k, dim);
            let confidence = if reading.margin.is_finite() {
                reading.margin.clamp(0.0, 1.0) as f32
            } else {
                0.0
            };
            let suspect = reading.flagged || reading.value.is_none() || confidence < SUSPECT_CONF;
            CellDetail {
                origin: (wins[k].0.min(n.w), band.0.saturating_sub(pad)),
                cell,
                reading,
                confidence,
                suspect,
            }
        });
        rows.push(cells);
    }
    Some(BoardDetail {
        centres,
        windows: wins,
        bands,
        rows,
        ink: n,
    })
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    /// Synthetic post-match boards. Rows 0-5 then 6-11, columns E A D DMG H MIT.
    /// Same grid as `synth::POSTGAME_VALUES`.
    const POSTGAME: [[u32; 6]; 12] = [
        [12, 3, 3, 5480, 950, 2347],
        [7, 1, 7, 3789, 0, 82],
        [7, 8, 7, 1858, 3387, 110],
        [7, 15, 6, 1545, 5407, 0],
        [15, 3, 5, 7283, 332, 24],
        [19, 0, 5, 4922, 987, 5785],
        [21, 8, 3, 7161, 1540, 6914],
        [15, 1, 4, 4812, 142, 0],
        [16, 5, 5, 5643, 952, 1685],
        [14, 8, 2, 1762, 1645, 123],
        [15, 15, 3, 3448, 6595, 0],
        [6, 5, 4, 2430, 1382, 203],
    ];

    pub(in crate::report::blank) fn fixture(name: &str) -> DynamicImage {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("test-data/vision-baseline")
            .join(name);
        image::open(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
    }

    #[test]
    fn the_copy_reads_both_post_game_fixtures_exactly() {
        for name in [
            "native_postgame_table_names_blanked.png",
            "s075_postgame_table_names_blanked.png",
        ] {
            let d = read_board_detail(&fixture(name), 6).expect(name);
            assert_eq!(d.rows.len(), 12, "{name}");
            for (r, row) in d.rows.iter().enumerate() {
                for (k, c) in row.iter().enumerate() {
                    assert_eq!(
                        c.reading.value,
                        Some(POSTGAME[r][k]),
                        "{name} row {r} col {k}"
                    );
                    assert!(!c.suspect, "{name} row {r} col {k} suspect");
                    let glyphs = c.reading.pieces.iter().filter(|p| p.class != COMMA).count();
                    assert_eq!(
                        glyphs,
                        POSTGAME[r][k].to_string().len(),
                        "{name} row {r} col {k}"
                    );
                }
            }
        }
    }
}
