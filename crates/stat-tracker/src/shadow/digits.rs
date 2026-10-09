//! Template digit matcher for scoreboard stat cells (shadow mode only).
//!
//! Port of the Scuffed Vision prototype matcher. Geometry follows the
//! tracker's own layout:
//! * columns: header label centres from [`preprocess::header_label_groups`],
//!   filtered like `columns_from_header_groups`; each column window runs to
//!   the midpoints between neighbouring labels;
//! * rows: the `crop_player_row` grid (header 0.025, team 2 at 0.565,
//!   `row_h = (t2 - t1) / (team + 1)`) as seeds, each snapped to the nearest
//!   neutral-ink text band from a horizontal projection over the stat columns.
//!
//! Digits: brightness-normalised ink map (per cell `(min(R,G,B) - bg) / (peak - bg)`,
//! masked to low saturation), column-projection segmentation, comma
//! detection, fixed-size canvases and zero-mean normalised correlation
//! against embedded per-resolution templates. Confidence is a margin: best
//! minus second-best correlation (min over glyphs), also bounded by how far
//! the best reading beats the best different reading.
//!
//! Text band: each cell estimates its own (top, height); a cell whose height
//! is 2+ px off its row's median takes the row's band, so a lone thin-stem
//! glyph (a 4 at 1440p) cannot shrink its band and get split into pieces.
//!
//! Resampling reproduces Pillow's float bilinear resize so canvases match the
//! ones the templates were built from.

use std::sync::OnceLock;
use std::time::{Duration, Instant};

use image::DynamicImage;

use crate::ocr::preprocess;

pub const FIELDS: [&str; 6] = ["E", "A", "D", "DMG", "H", "MIT"];

/// Flag a cell when best minus second-best correlation is below this.
pub const SUSPECT_MARGIN: f32 = 0.06;

#[derive(Debug, Clone, PartialEq)]
pub struct CellRead {
    pub value: Option<u32>,
    pub confidence: f32,
    pub suspect: bool,
}

#[derive(Debug, Clone)]
pub struct RowRead {
    pub cells: [CellRead; 6],
}

#[derive(Debug, Clone)]
pub struct BoardRead {
    pub rows: Vec<RowRead>,
    pub elapsed_ms: u32,
}

#[derive(Debug)]
pub enum ShadowError {
    Layout(&'static str),
    OverBudget,
    Unsupported(&'static str),
}

impl std::fmt::Display for ShadowError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ShadowError::Layout(why) => write!(f, "layout: {why}"),
            ShadowError::OverBudget => write!(f, "over budget"),
            ShadowError::Unsupported(why) => write!(f, "unsupported: {why}"),
        }
    }
}

impl std::error::Error for ShadowError {}

// Same ratios as preprocess::crop_player_row.
const HEADER_RATIO: f64 = 0.025;
const TEAM2_START_RATIO: f64 = 0.565;

/// Scoreboard heights the templates were built at (crop of 2560x1440 and 1920x1080).
const REF_H_1440: u32 = 1007;
const REF_H_1080: u32 = 756;
/// Below this the glyphs are too small to match reliably.
const MIN_BOARD_H: u32 = 400;

const NCLS: usize = 11;
const COMMA: usize = 10;
const CH: usize = 26;
const CW: usize = 22;
const CANVAS: usize = CH * CW;
/// Ink threshold for segmentation.
const INK_THR: f32 = 0.45;
/// Max gap inside one number, in text heights ('11' is wide-set).
const GAP_K: f64 = 0.9;
/// Runs wider than this (in text heights) hold 2+ touching glyphs.
const WIDE_RUN: f64 = 0.95;
const BEAM: usize = 64;
const KRUN: usize = 12;
/// Widest single glyph, in text heights.
const MAX_GLYPH: f64 = 1.10;
/// Objective penalty per extra piece.
const SPLIT_PEN: f64 = 0.04;
/// Canvas covers [top, top + DESC * dh) so the comma tail fits.
const DESC: f64 = 1.35;
/// A cell whose own text height is this many px off its row's median takes
/// the row's text band instead (see [`row_text_band`]).
const BAND_TOL: usize = 2;
/// Cells that must segment before a row consensus is trusted (a row has 6
/// stat cells; with fewer voters two shrunk cells can carry the median).
const MIN_BAND_CELLS: usize = 4;

/// Python-style `round()` (ties to even), used wherever the prototype rounds.
fn pyround(x: f64) -> i64 {
    x.round_ties_even() as i64
}

// ---------------------------------------------------------------- templates

struct Templates {
    t: Vec<[f32; CANVAS]>,
    ar_lo: [f64; NCLS],
    ar_hi: [f64; NCLS],
}

const TPL_1440_PNG: &[u8] = include_bytes!("../../assets/shadow-digits/digits_1440.png");
const TPL_1080_PNG: &[u8] = include_bytes!("../../assets/shadow-digits/digits_1080.png");

// Width gates (glyph width / text height) per class 0-9 and ',', from the
// same template build as the PNGs (mean +- 3 sd + 0.05).
const AR_1440: ([f64; NCLS], [f64; NCLS]) = (
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
const AR_1080: ([f64; NCLS], [f64; NCLS]) = (
    [
        0.5503191449372535,
        0.14119802759209582,
        0.5124228076317954,
        0.45866675656073513,
        0.5509440937355133,
        0.4301998649477294,
        0.5170751754054589,
        0.539402375514284,
        0.42419604756409807,
        0.5345078549932906,
        0.04589385469689673,
    ],
    [
        1.0192159688115103,
        0.5820841361505943,
        0.9569391285618241,
        0.8965868666276705,
        1.0663226503604988,
        0.897423980560247,
        1.027188600565634,
        0.9283669204992868,
        0.9048665655376031,
        1.0220619928628643,
        0.4024194267374038,
    ],
);

fn load_templates(png: &[u8], ar: ([f64; NCLS], [f64; NCLS])) -> Templates {
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
    }
}

fn templates_for(board_h: u32) -> &'static Templates {
    static T1440: OnceLock<Templates> = OnceLock::new();
    static T1080: OnceLock<Templates> = OnceLock::new();
    if board_h.abs_diff(REF_H_1440) <= board_h.abs_diff(REF_H_1080) {
        T1440.get_or_init(|| load_templates(TPL_1440_PNG, AR_1440))
    } else {
        T1080.get_or_init(|| load_templates(TPL_1080_PNG, AR_1080))
    }
}

/// Zero-mean, unit-norm in place.
fn normalise(v: &mut [f32]) {
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
struct Plane {
    w: usize,
    h: usize,
    d: Vec<f32>,
}

impl Plane {
    fn zeros(w: usize, h: usize) -> Self {
        Plane {
            w,
            h,
            d: vec![0.0; w * h],
        }
    }
    #[inline]
    fn at(&self, x: usize, y: usize) -> f32 {
        self.d[y * self.w + x]
    }
}

/// Neutral (white/grey) brightness: min channel, zeroed where saturated.
fn neutral_ink(img: &DynamicImage) -> Plane {
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
fn column_centres(board: &DynamicImage) -> Result<[f64; 6], ShadowError> {
    let w = board.width() as f64;
    let labels: Vec<f64> = preprocess::header_label_groups(board)
        .into_iter()
        .filter(|&(s, e)| (e - s) as f64 / w <= 0.05)
        .map(|(s, e)| (s + e) as f64 / 2.0)
        .collect();
    if labels.len() < 6 {
        return Err(ShadowError::Layout("header labels not found"));
    }
    let mut c = [0f64; 6];
    c.copy_from_slice(&labels[..6]);
    if !c
        .windows(2)
        .all(|p| (0.02..=0.10).contains(&((p[1] - p[0]) / w)))
    {
        return Err(ShadowError::Layout("implausible header spacing"));
    }
    Ok(c)
}

fn column_windows(c: &[f64; 6], w: usize) -> [(usize, usize); 6] {
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
fn row_bands(n: &Plane, windows: &[(usize, usize); 6], team: usize) -> Vec<(usize, usize)> {
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
        rows.push(best.unwrap_or_else(|| {
            // fall back to the tracker crop with its 15% padding
            let pad = pyround(rh as f64 * 0.15) as usize;
            (y0 + pad, (y0 + rh - pad).min(h))
        }));
    }
    rows
}

// ---------------------------------------------------------------- resampling

/// Pillow `precompute_coeffs` for the bilinear (triangle) filter.
fn pil_coeffs(in_size: usize, out_size: usize) -> Vec<(usize, Vec<f64>)> {
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
fn pil_resize(src: &Plane, nw: usize, nh: usize) -> Plane {
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

struct Segment {
    ink: Plane,
    b: Vec<bool>,
    top: usize,
    dh: usize,
    runs: Vec<(usize, usize)>,
    rival: f64,
}

fn percentile_sorted(v: &[f32], q: f64) -> f64 {
    let idx = q / 100.0 * (v.len() - 1) as f64;
    let lo = idx.floor() as usize;
    let hi = idx.ceil() as usize;
    let f = idx - lo as f64;
    v[lo] as f64 + (v[hi] as f64 - v[lo] as f64) * f
}

/// Normalised cell ink map; `None` when the cell holds no contrast.
fn cell_ink(n: &Plane, band: (usize, usize), win: (usize, usize), pad: usize) -> Option<Plane> {
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
    Some(c)
}

fn runs_of(cols: &[bool]) -> Vec<(usize, usize)> {
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
fn segment(ink: Plane) -> Option<Segment> {
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
    for i in 1..runs.len() {
        let last = *groups.last().unwrap().last().unwrap();
        if (runs[i].0 - runs[last].1) as f64 <= gap_max {
            groups.last_mut().unwrap().push(i);
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
    let runs = g.iter().map(|&i| runs[i]).collect();
    Some(Segment {
        ink,
        b,
        top,
        dh,
        runs,
        rival,
    })
}

/// Rows [top, top + DESC*dh) scaled so the strip is CH rows tall; same factor horizontally.
fn scaled_strip(ink: &Plane, top: usize, dh: usize) -> (Plane, f64) {
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

fn piece_canvas(strip: &Plane, sc: f64, x0: usize, x1: usize) -> [f32; CANVAS] {
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
fn candidate_cuts(colsum: &[u32], s: usize, e: usize, dh: usize) -> Vec<usize> {
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

fn partitions(s: usize, e: usize, cuts: &[usize]) -> Vec<Vec<(usize, usize)>> {
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

enum RunKind {
    /// Explicit partitions, as lists of piece ids.
    Enum(Vec<Vec<usize>>),
    /// Touching glyphs: for each end x1 (ascending), the pieces (id, x0) ending there.
    Dp(Vec<(usize, Vec<(usize, usize)>)>),
}

struct Run {
    s: usize,
    e: usize,
    kind: RunKind,
}

struct Cell {
    runs: Vec<Run>,
    /// Normalised piece canvases, `CANVAS` floats per piece.
    v: Vec<f32>,
    mass: Vec<f64>,
    ar: Vec<f64>,
    total_mass: f64,
    rival: f64,
}

/// Ink map and segmentation for one cell.
fn segment_cell(
    n: &Plane,
    band: (usize, usize),
    win: (usize, usize),
    pad: usize,
) -> Option<Segment> {
    segment(cell_ink(n, band, win, pad)?)
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
fn row_text_band(segs: &[Option<Segment>]) -> Option<(usize, usize)> {
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
fn apply_row_band(seg: &mut Segment, row: Option<(usize, usize)>) {
    if let Some((top, dh)) = row
        && seg.dh.abs_diff(dh) >= BAND_TOL
        && top < seg.ink.h
    {
        seg.top = top;
        seg.dh = dh;
    }
}

/// Template-independent work for one cell: all candidate piece canvases.
fn prepare_cell(seg: Segment) -> Option<Cell> {
    let dh = seg.dh;
    let (strip, sc) = scaled_strip(&seg.ink, seg.top, dh);
    let w = seg.ink.w;
    let colsum: Vec<u32> = (0..w)
        .map(|x| (0..seg.ink.h).filter(|&y| seg.b[y * w + x]).count() as u32)
        .collect();
    let mut pieces: Vec<(usize, usize)> = Vec::new();
    let mut index = std::collections::HashMap::new();
    let mut pid = |x0: usize, x1: usize| {
        *index.entry((x0, x1)).or_insert_with(|| {
            pieces.push((x0, x1));
            pieces.len() - 1
        })
    };
    let mut runs = Vec::new();
    for &(rs, re) in &seg.runs {
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
                    .map(|x0| (pid(x0, x1), x0))
                    .collect();
                ends.push((x1, starts));
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
                .map(|p| p.into_iter().map(|(a, b)| pid(a, b)).collect())
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
    let mass = pieces
        .iter()
        .map(|&(a, b)| (cb[b] - cb[a]) as f64)
        .collect();
    let ar = pieces
        .iter()
        .map(|&(a, b)| (b - a) as f64 / dh as f64)
        .collect();
    let total_mass = seg.runs.iter().map(|&(a, e)| (cb[e] - cb[a]) as f64).sum();
    Some(Cell {
        runs,
        v,
        mass,
        ar,
        total_mass,
        rival: seg.rival,
    })
}

type Scored = (f64, Vec<usize>);

fn sort_desc(v: &mut [Scored]) {
    v.sort_by(|a, b| b.0.total_cmp(&a.0));
}

/// Top-K partitions of a run, scored with `piece_score` (already includes -SPLIT_PEN).
fn kbest_run(run: &Run, piece_score: &[f64], k: usize) -> Vec<Scored> {
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
            best.insert(run.s, vec![(0.0, Vec::new())]);
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
                    cand.truncate(k);
                    best.insert(*x1, cand);
                }
            }
            best.remove(&run.e).unwrap_or_default()
        }
    }
}

fn thousands(n: u64) -> String {
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
fn plain_field(k: usize) -> bool {
    k < 3
}

fn grammar_ok(t: &str, k: usize) -> bool {
    let d: String = t.chars().filter(|&c| c != ',').collect();
    if d.is_empty() || (d.len() > 1 && d.starts_with('0')) {
        return false;
    }
    if plain_field(k) {
        return !t.contains(',') && d.len() <= 3;
    }
    d.len() <= 6 && d.parse::<u64>().is_ok_and(|n| thousands(n) == t)
}

/// Value, margin and whether any sanity flag fired.
struct Reading {
    value: Option<u32>,
    margin: f64,
    flagged: bool,
}

fn read_cell(cell: Option<&Cell>, tpl: &Templates, k: usize) -> Reading {
    let Some(cell) = cell else {
        return Reading {
            value: None,
            margin: 0.0,
            flagged: true,
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
        second_s[p] = (0..NCLS)
            .filter(|&c| c != bc)
            .map(|c| if allowed[c] { s[c] } else { -1.0 })
            .fold(f64::NEG_INFINITY, f64::max);
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
        a.iter()
            .map(|&p| {
                if best_c[p] == COMMA {
                    ','
                } else {
                    char::from(b'0' + best_c[p] as u8)
                }
            })
            .collect()
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
    let gm = acc
        .iter()
        .map(|&p| best_s[p] - second_s[p])
        .fold(f64::INFINITY, f64::min);
    let gm = if acc.is_empty() { 0.0 } else { gm };
    // partition ambiguity: best alternative reading with a different digit string
    let pm = alt.map_or(1.0, |a| obj - a);
    let margin = gm.min(2.0 * pm);
    if digits.is_empty() {
        return Reading {
            value: None,
            margin: 0.0,
            flagged: true,
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
    flagged |= obj < 0.55;
    let value = parsed.and_then(|n| u32::try_from(n).ok());
    Reading {
        flagged: flagged || value.is_none(),
        value,
        margin,
    }
}

// ---------------------------------------------------------------- entry point

/// Read every stat cell of a cropped scoreboard.
///
/// `scoreboard` is exactly what main.rs passes to
/// `ocr::recognize_scoreboard_cells_pre_cropped` (output of
/// `ocr::preprocess::crop_scoreboard`). Rows are ordered like ocr-v1's rows:
/// team 1 top to bottom, then team 2. The budget is checked between rows.
pub fn read_board(
    scoreboard: &DynamicImage,
    team_size: usize,
    budget: Duration,
) -> Result<BoardRead, ShadowError> {
    let t0 = Instant::now();
    if !(5..=6).contains(&team_size) {
        return Err(ShadowError::Unsupported("team size"));
    }
    let h = scoreboard.height();
    if h < MIN_BOARD_H {
        return Err(ShadowError::Unsupported("scoreboard too small"));
    }
    let tpl = templates_for(h);
    let centres = column_centres(scoreboard)?;
    let n = neutral_ink(scoreboard);
    let wins = column_windows(&centres, n.w);
    let bands = row_bands(&n, &wins, team_size);
    let pad = (pyround(4.0 * h as f64 / REF_H_1440 as f64) as usize).max(1);
    if t0.elapsed() > budget {
        return Err(ShadowError::OverBudget);
    }
    let mut rows = Vec::with_capacity(bands.len());
    for band in bands {
        let mut segs: [Option<Segment>; 6] =
            std::array::from_fn(|k| segment_cell(&n, band, wins[k], pad));
        let row_band = row_text_band(&segs);
        let cells = std::array::from_fn(|k| {
            let cell = segs[k].take().and_then(|mut seg| {
                apply_row_band(&mut seg, row_band);
                prepare_cell(seg)
            });
            let r = read_cell(cell.as_ref(), tpl, k);
            let confidence = r.margin.clamp(0.0, 1.0) as f32;
            CellRead {
                value: r.value,
                confidence,
                suspect: r.flagged || r.value.is_none() || confidence < SUSPECT_MARGIN,
            }
        });
        rows.push(RowRead { cells });
        if t0.elapsed() > budget {
            return Err(ShadowError::OverBudget);
        }
    }
    Ok(BoardRead {
        rows,
        elapsed_ms: t0.elapsed().as_millis().min(u32::MAX as u128) as u32,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgb, RgbImage};

    /// Mean glyph canvases (0..1) straight from the embedded 1440 templates.
    fn glyph_means() -> Vec<Plane> {
        let img = image::load_from_memory(TPL_1440_PNG).unwrap().to_luma8();
        (0..NCLS)
            .map(|c| {
                let mut p = Plane::zeros(CW, CH);
                for y in 0..CH {
                    for x in 0..CW {
                        p.d[y * CW + x] =
                            img.get_pixel((c * CW + x) as u32, y as u32).0[0] as f32 / 255.0;
                    }
                }
                p
            })
            .collect()
    }

    /// Glyph columns that carry ink, so glyphs can be laid out with a fixed gap.
    fn ink_cols(g: &Plane) -> (usize, usize) {
        let cols: Vec<usize> = (0..g.w)
            .filter(|&x| (0..g.h).any(|y| g.at(x, y) > 0.2))
            .collect();
        (cols[0], cols[cols.len() - 1] + 1)
    }

    fn text_for(v: u32, k: usize) -> String {
        if plain_field(k) {
            v.to_string()
        } else {
            thousands(v as u64)
        }
    }

    /// Paste `text` centred at (cx, top) in neutral grey on a dark board.
    fn draw_text(
        img: &mut RgbImage,
        glyphs: &[Plane],
        text: &str,
        cx: usize,
        top: usize,
        gap: usize,
    ) {
        let idx: Vec<usize> = text
            .chars()
            .map(|c| {
                if c == ',' {
                    COMMA
                } else {
                    c as usize - '0' as usize
                }
            })
            .collect();
        let widths: Vec<(usize, usize)> = idx.iter().map(|&c| ink_cols(&glyphs[c])).collect();
        let total: usize = widths.iter().map(|(a, b)| b - a).sum::<usize>() + gap * (idx.len() - 1);
        let mut x = cx - total / 2;
        for (&c, &(a, b)) in idx.iter().zip(&widths) {
            let g = &glyphs[c];
            for y in 0..CH {
                for gx in a..b {
                    let v = (g.at(gx, y) * 235.0) as u8;
                    let px = img.get_pixel_mut((x + gx - a) as u32, (top + y) as u32);
                    if v > px.0[0] {
                        *px = Rgb([v, v, v]);
                    }
                }
            }
            x += b - a + gap;
        }
    }

    /// 1664x1007 scoreboard (a 1440p crop): bright header with six dark labels,
    /// dark rows holding the given stat values. No name or hero pixels.
    fn synth_board(values: &[[u32; 6]], team: usize, gap: usize) -> DynamicImage {
        let (w, h) = (1664usize, 1007usize);
        let mut img = RgbImage::from_pixel(w as u32, h as u32, Rgb([18, 20, 32]));
        for y in 0..26 {
            for x in 0..w {
                img.put_pixel(x as u32, y, Rgb([225, 225, 225]));
            }
        }
        let centres: Vec<usize> = (0..6).map(|i| 960 + i * 100).collect();
        for &c in &centres {
            for y in 0..26 {
                for x in c - 6..c + 6 {
                    img.put_pixel(x as u32, y, Rgb([30, 30, 30]));
                }
            }
        }
        let glyphs = glyph_means();
        let t1 = (h as f64 * HEADER_RATIO) as usize;
        let t2 = (h as f64 * TEAM2_START_RATIO) as usize;
        let rh = (t2 - t1) / (team + 1);
        for (r, row) in values.iter().enumerate() {
            let base = if r < team { t1 } else { t2 };
            let ctr = base + (r % team) * rh + rh / 2;
            for (k, &v) in row.iter().enumerate() {
                draw_text(
                    &mut img,
                    &glyphs,
                    &text_for(v, k),
                    centres[k],
                    ctr - 10,
                    gap,
                );
            }
        }
        DynamicImage::ImageRgb8(img)
    }

    fn sample_values(team: usize) -> Vec<[u32; 6]> {
        let base: [[u32; 6]; 12] = [
            [12, 3, 3, 5480, 950, 2347],
            [0, 0, 0, 0, 0, 0],
            [27, 14, 9, 18744, 1203, 87],
            [5, 10, 1, 101, 23456, 0],
            [8, 2, 4, 9876, 33, 104512],
            [41, 6, 7, 765, 4321, 11],
            [1, 11, 2, 3300, 7, 1000],
            [16, 5, 0, 22058, 619, 4096],
            [3, 19, 8, 47, 15998, 2],
            [9, 4, 6, 6140, 280, 39017],
            [22, 7, 5, 11111, 0, 765],
            [6, 1, 3, 8023, 1450, 333],
        ];
        base.iter().take(2 * team).copied().collect()
    }

    fn assert_reads(board: &BoardRead, want: &[[u32; 6]]) {
        assert_eq!(board.rows.len(), want.len());
        for (r, (row, w)) in board.rows.iter().zip(want).enumerate() {
            for k in 0..6 {
                assert_eq!(
                    row.cells[k].value,
                    Some(w[k]),
                    "row {r} {}: {:?}",
                    FIELDS[k],
                    row.cells[k]
                );
            }
        }
    }

    #[test]
    fn reads_synthetic_6v6_board() {
        let want = sample_values(6);
        let board = synth_board(&want, 6, 3);
        let read = read_board(&board, 6, Duration::from_secs(10)).unwrap();
        assert_reads(&read, &want);
        let suspects = read
            .rows
            .iter()
            .flat_map(|r| r.cells.iter())
            .filter(|c| c.suspect)
            .count();
        assert!(suspects <= 3, "{suspects} suspect cells on a clean board");
    }

    #[test]
    fn reads_synthetic_5v5_board() {
        let want = sample_values(5);
        let board = synth_board(&want, 5, 3);
        let read = read_board(&board, 5, Duration::from_secs(10)).unwrap();
        assert_reads(&read, &want);
    }

    #[test]
    fn reads_touching_glyphs() {
        // gap 0: neighbouring glyphs share columns, exercising the wide-run DP split
        let want = sample_values(6);
        let board = synth_board(&want, 6, 0);
        let read = read_board(&board, 6, Duration::from_secs(10)).unwrap();
        assert_reads(&read, &want);
    }

    #[test]
    fn reads_rescaled_1080_board() {
        let want = sample_values(6);
        let board = synth_board(&want, 6, 3).resize_exact(1248, 756, image::imageops::Triangle);
        let read = read_board(&board, 6, Duration::from_secs(10)).unwrap();
        assert_reads(&read, &want);
    }

    #[test]
    fn empty_cell_is_none_and_suspect() {
        let want = sample_values(6);
        let board = synth_board(&want, 6, 3);
        // blank out row 0's DMG cell
        let mut img = board.to_rgb8();
        let t1 = (1007.0 * HEADER_RATIO) as u32;
        for y in t1 + 5..t1 + 70 {
            for x in 1215..1305 {
                img.put_pixel(x, y, Rgb([18, 20, 32]));
            }
        }
        let read = read_board(&DynamicImage::ImageRgb8(img), 6, Duration::from_secs(10)).unwrap();
        let cell = &read.rows[0].cells[3];
        assert_eq!(cell.value, None);
        assert!(cell.suspect);
        assert_eq!(read.rows[0].cells[4].value, Some(want[0][4]));
    }

    #[test]
    fn errors_on_bad_input() {
        let blank = DynamicImage::ImageRgb8(RgbImage::new(1664, 1007));
        assert!(matches!(
            read_board(&blank, 6, Duration::from_secs(1)),
            Err(ShadowError::Layout(_))
        ));
        let tiny = DynamicImage::ImageRgb8(RgbImage::new(300, 200));
        assert!(matches!(
            read_board(&tiny, 6, Duration::from_secs(1)),
            Err(ShadowError::Unsupported(_))
        ));
        assert!(matches!(
            read_board(&blank, 4, Duration::from_secs(1)),
            Err(ShadowError::Unsupported(_))
        ));
        let board = synth_board(&sample_values(6), 6, 3);
        assert!(matches!(
            read_board(&board, 6, Duration::ZERO),
            Err(ShadowError::OverBudget)
        ));
    }

    fn seg_with(top: usize, dh: usize) -> Segment {
        Segment {
            ink: Plane::zeros(8, 24),
            b: vec![false; 8 * 24],
            top,
            dh,
            runs: vec![(1, 7)],
            rival: 0.0,
        }
    }

    #[test]
    fn row_band_overrides_a_shrunk_lone_four() {
        // dh as measured on a real 1440p row: a lone '4' with a 2 px stem came out as 9
        let mut segs: Vec<Option<Segment>> = [(4, 13), (4, 13), (5, 9), (4, 13), (4, 14), (4, 13)]
            .iter()
            .map(|&(t, d)| Some(seg_with(t, d)))
            .collect();
        segs.push(None);
        let band = row_text_band(&segs);
        assert_eq!(band, Some((4, 13)));
        let mut four = segs[2].take().unwrap();
        apply_row_band(&mut four, band);
        assert_eq!((four.top, four.dh), (4, 13));
        // within tolerance: left alone
        let mut close = seg_with(5, 14);
        apply_row_band(&mut close, band);
        assert_eq!((close.top, close.dh), (5, 14));
        // too few cells for a consensus
        assert_eq!(
            row_text_band(&[Some(seg_with(4, 13)), None, Some(seg_with(5, 9))]),
            None
        );
    }

    #[test]
    fn short_row_keeps_per_cell_bands() {
        // three cells, two of them shrunk: a 3-cell median would pick 10 and
        // squash the good cell, so below MIN_BAND_CELLS nothing is overridden
        let mut segs: Vec<Option<Segment>> = vec![
            Some(seg_with(5, 10)),
            Some(seg_with(5, 9)),
            Some(seg_with(4, 13)),
            None,
            None,
            None,
        ];
        let band = row_text_band(&segs);
        assert_eq!(band, None);
        let want = [(5, 10), (5, 9), (4, 13)];
        for (seg, w) in segs.iter_mut().flatten().zip(want) {
            apply_row_band(seg, band);
            assert_eq!((seg.top, seg.dh), w);
        }
        // a fourth agreeing cell is enough for a consensus again
        segs[3] = Some(seg_with(4, 13));
        segs[4] = Some(seg_with(4, 13));
        assert_eq!(row_text_band(&segs), Some((4, 13)));
    }

    #[test]
    fn grammar_and_grouping() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1000), "1,000");
        assert_eq!(thousands(104512), "104,512");
        assert!(grammar_ok("12", 0));
        assert!(!grammar_ok("1,2", 0));
        assert!(!grammar_ok("01", 1));
        assert!(grammar_ok("5,480", 3));
        assert!(!grammar_ok("5480", 3));
        assert!(!grammar_ok("54,80", 5));
        assert!(!grammar_ok("1,234,567", 4));
    }

    #[test]
    fn pil_bilinear_matches_pillow() {
        // Pillow: Image.fromarray(np.array([[0,1,2,3]], 'f'), 'F').resize((2,1), BILINEAR)
        let src = Plane {
            w: 4,
            h: 1,
            d: vec![0.0, 1.0, 2.0, 3.0],
        };
        let out = pil_resize(&src, 2, 1);
        assert!((out.d[0] - 1.25 / 1.75).abs() < 1e-6, "{:?}", out.d);
        assert!((out.d[1] - (1.0 * 0.25 + 2.0 * 0.75 + 3.0 * 0.75) / 1.75).abs() < 1e-6);
        // same size is the identity
        assert_eq!(pil_resize(&src, 4, 1).d, src.d);
    }

    #[test]
    fn embedded_templates_load_for_both_heights() {
        for h in [1007, 756, 1440, 500] {
            let t = templates_for(h);
            assert_eq!(t.t.len(), NCLS);
            for c in 0..NCLS {
                let norm: f32 = t.t[c].iter().map(|v| v * v).sum();
                assert!((norm - 1.0).abs() < 1e-3);
                assert!(t.ar_lo[c] < t.ar_hi[c]);
            }
        }
    }
}
