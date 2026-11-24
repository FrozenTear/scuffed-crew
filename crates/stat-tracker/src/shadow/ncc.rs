//! The normalised cross-correlation kernel the words-v1 readers share: the
//! [`Gray`] plane, pyramid levels with integral images, template stacks, the
//! coarse-to-fine search ([`search_fast`] and the exhaustive
//! [`search_full`]), the projection bounds and the certification that rules
//! a template in or out against a threshold ([`certify`]). The result word
//! reader (`result`), its phrase reads (`result::phrase`) and the map banner
//! reader (`banner`, for [`Gray`]) use it. Moved out of `result.rs` and
//! `banner.rs` (0.6 lane D V5); both re-export what they exported before.
//! Pixels only: no reader state, no packs, no Tesseract.

use std::path::Path;

use image::imageops::{self, FilterType};
use image::{ImageBuffer, Luma};

/// Float grey image, row-major, values 0..1.
#[derive(Debug, Clone, PartialEq)]
pub struct Gray {
    pub w: usize,
    pub h: usize,
    pub px: Vec<f32>,
}

impl Gray {
    pub fn new(w: usize, h: usize) -> Self {
        Self {
            w,
            h,
            px: vec![0.0; w * h],
        }
    }

    #[inline]
    pub fn at(&self, x: usize, y: usize) -> f32 {
        self.px[y * self.w + x]
    }

    /// Bilinear (triangle) resize, area-aware when shrinking. Values must be
    /// in 0..1: the image crate clamps float pixels to that range.
    pub fn resize(&self, w: usize, h: usize) -> Gray {
        if w == self.w && h == self.h {
            return self.clone();
        }
        let Some(buf) = ImageBuffer::<Luma<f32>, Vec<f32>>::from_raw(
            self.w as u32,
            self.h as u32,
            self.px.clone(),
        ) else {
            return Gray::new(w, h);
        };
        let out = imageops::resize(&buf, w.max(1) as u32, h.max(1) as u32, FilterType::Triangle);
        Gray {
            w: w.max(1),
            h: h.max(1),
            px: out.into_raw(),
        }
    }

    pub(super) fn mean(&self) -> f64 {
        self.px.iter().map(|&v| v as f64).sum::<f64>() / self.px.len().max(1) as f64
    }
}

/// Below this, no result word is on the frame.
pub const PRESENT: f32 = 0.50;

pub(super) const SCALES: [f64; 3] = [0.96, 1.0, 1.04];

/// The coarse search level keeps at least this many template pixels on the
/// template's short side.
const MIN_COARSE_PX: usize = 12;

/// Deepest pyramid level: 4x downscaled.
const MAX_LEVEL: usize = 2;

/// A coarser level is used only when the template keeps at least this
/// share of its per-pixel variance there. Text strokes survive a 4x box
/// filter (the words-v1 templates keep 0.84 to 0.97); fine stripes average
/// out, and a coarse search over them would rank the wrong peaks.
pub(super) const MIN_KEPT: f64 = 0.8;

/// Coarse peaks refined per template and scale.
pub(super) const PEAKS: usize = 12;

/// Coarse peaks closer than this (coarse pixels, either axis) are one peak.
const PEAK_SEP: usize = 1;

/// Refine window half-width on the first level below the coarse one, then
/// on every later level (pixels of that level).
pub(super) const REFINE_FIRST: usize = 3;

pub(super) const REFINE: usize = 2;

// ------------------------------------------------------------------ search

/// A window's variance sum at or below this was flat before [`FLAT_ROUNDING`]
/// and still is: the new rule only adds flat windows.
pub(super) const FLAT_VAR: f64 = 1e-9;
/// The rounding error of a window's variance sum as [`Level::window`]
/// computes it, per window pixel and per level pixel (the bound is derived
/// there). A window at or under it is flat.
const FLAT_ROUNDING: f64 = 12.0 * f64::EPSILON;

/// One pyramid level of a region: pixels plus integral images (sum and sum
/// of squares) for window statistics. Pixels are in 0..1.
pub(super) struct Level {
    pub(super) w: usize,
    pub(super) h: usize,
    pub(super) px: Vec<f32>,
    pub(super) s1: Vec<f64>,
    pub(super) s2: Vec<f64>,
    /// [`FLAT_ROUNDING`] times this level's pixel count: what the variance
    /// sum of a flat window can be per window pixel ([`Level::window`]).
    pub(super) flat: f64,
}

impl Level {
    pub(super) fn new(w: usize, h: usize, px: Vec<f32>) -> Self {
        let mut s1 = vec![0.0f64; (w + 1) * (h + 1)];
        let mut s2 = vec![0.0f64; (w + 1) * (h + 1)];
        for y in 0..h {
            for x in 0..w {
                let v = px[y * w + x] as f64;
                let i = (y + 1) * (w + 1) + x + 1;
                s1[i] = v + s1[i - 1] + s1[i - w - 1] - s1[i - w - 2];
                s2[i] = v * v + s2[i - 1] + s2[i - w - 1] - s2[i - w - 2];
            }
        }
        let flat = FLAT_ROUNDING * (w * h) as f64;
        Self {
            w,
            h,
            px,
            s1,
            s2,
            flat,
        }
    }

    /// Sum of `s` over the `tw` x `th` window at (x, y).
    #[inline]
    pub(super) fn rect(&self, s: &[f64], x: usize, y: usize, tw: usize, th: usize) -> f64 {
        let a = y * (self.w + 1) + x;
        let b = (y + th) * (self.w + 1) + x;
        s[b + tw] - s[b] - s[a + tw] + s[a]
    }

    /// The pixel sum and the variance sum (`sum((p - mean)^2)`, not divided
    /// by the pixel count) of the `tw` x `th` window at (x, y), or `None`
    /// when the window is flat: nothing in it can match a word.
    ///
    /// A window of one grey value has a variance of 0, but the value
    /// computed here from the f64 integral images is their rounding error,
    /// positive or negative. On a 512 x 230 band of one grey value it
    /// reaches 1e-7 for an end-title window (R3-1). The old test, a fixed
    /// [`FLAT_VAR`], let about a third of those windows through, and
    /// [`ncc_at`] scored them: the rounding error of its f32 dot product
    /// over the square root of that noise. Measured on such bands: up to
    /// 0.38 for a words-v1 end title, 0.63 for a synthetic word of the
    /// tests. So a window is flat when its variance sum is within what that
    /// rounding can be.
    ///
    /// The bound, with `A` this level's pixel count, `n` the window's, and
    /// `u = EPSILON / 2`. An integral-image entry is three rounded additions
    /// of values up to `A`, `2 A` and `A` (pixels are in 0..1, and a square
    /// of an f32 is exact in f64), so it is off by at most `4 u A` from the
    /// same sum of exact entries. The recurrence is linear, so the table is
    /// the exact table of pixels that are each off by that much, and a
    /// window sum, with the three additions of [`Self::rect`], is off by at
    /// most `(4 n + 3) u A`, for the sums and for the squares. The variance
    /// sum `s2 - s1 * s1 / n` is then off by at most that for `s2`, twice
    /// that for `s1` (its error times `2 * mean`, and the mean is at most 1)
    /// and `3 u n` for its own three operations:
    /// `3 (4 n + 3) u A + 3 u n <= 24 u n A = 12 EPSILON n A`
    /// ([`FLAT_ROUNDING`]). A window of one value is therefore always flat.
    ///
    /// Nothing real is that faint. A title one 8-bit grey level off its
    /// background over 2% of the window has a variance sum of `3e-7 n`; the
    /// bound is `3.1e-10 n` on the end-title band (512 x 230) and `2.5e-9 n`
    /// on the largest region there is, the whole 1280 x 720 work rect.
    #[inline]
    pub(super) fn window(&self, x: usize, y: usize, tw: usize, th: usize) -> Option<(f64, f64)> {
        let n = (tw * th) as f64;
        let sum = self.rect(&self.s1, x, y, tw, th);
        let var = self.rect(&self.s2, x, y, tw, th) - sum * sum / n;
        (var > FLAT_VAR.max(n * self.flat)).then_some((sum, var))
    }
}

/// 2x2 box average, dropping an odd last row or column. Pixel (x, y) of the
/// result covers pixels (2x..2x+2, 2y..2y+2), so a position at one level is
/// twice that position one level down.
pub(super) fn half(w: usize, h: usize, px: &[f32]) -> (usize, usize, Vec<f32>) {
    let (hw, hh) = (w / 2, h / 2);
    let mut out = Vec::with_capacity(hw * hh);
    for y in 0..hh {
        for x in 0..hw {
            let i = 2 * y * w + 2 * x;
            out.push((px[i] + px[i + 1] + px[i + w] + px[i + w + 1]) * 0.25);
        }
    }
    (hw, hh, out)
}

/// One template at one scale and pyramid level: zero-mean pixels and norm.
#[derive(Debug, Clone)]
pub(super) struct Kernel {
    pub(super) w: usize,
    pub(super) h: usize,
    pub(super) z: Vec<f32>,
    pub(super) norm: f64,
}

impl Kernel {
    pub(super) fn new(w: usize, h: usize, px: &[f32]) -> Option<Self> {
        if w == 0 || h == 0 {
            return None;
        }
        let mean = px.iter().map(|&v| v as f64).sum::<f64>() / (w * h) as f64;
        let z: Vec<f32> = px.iter().map(|&v| (v as f64 - mean) as f32).collect();
        let norm = z.iter().map(|&v| v as f64 * v as f64).sum::<f64>().sqrt();
        (norm > 0.0).then_some(Self { w, h, z, norm })
    }

    /// Per-pixel variance.
    pub(super) fn var(&self) -> f64 {
        self.norm * self.norm / (self.w * self.h) as f64
    }
}

/// One template at one scale: level 0 is the work scale, level `l` is
/// 2^l times smaller. Only levels whose short side keeps
/// [`MIN_COARSE_PX`] pixels are built.
#[derive(Debug, Clone)]
pub(super) struct Stack {
    pub(super) levels: Vec<Kernel>,
    /// The template's block projections for the certification bound
    /// ([`certify`]), coarsest first: `f` = 16, 8, 4, 2 while the template
    /// keeps [`MIN_PROJ_BLOCKS`] blocks on each side.
    pub(super) projs: Vec<Proj>,
}

/// A template projected onto `f` x `f` blocks: `ct` is each block's mean
/// minus the whole template's mean (`cw` x `ch` blocks, the ones the
/// coarse kernel covers), `res` the norm of what the projection leaves
/// out. With `P` the block projection and `t` the zero-mean template,
/// `|t|^2 = f^2 |ct|^2 + res^2`.
#[derive(Debug, Clone)]
pub(super) struct Proj {
    pub(super) f: usize,
    pub(super) cw: usize,
    pub(super) ch: usize,
    pub(super) ct: Vec<f64>,
    res: f64,
}

impl Proj {
    fn new(t: &Gray, top: usize, norm: f64) -> Self {
        let (mut cw, mut ch, mut px) = (t.w, t.h, t.px.clone());
        for _ in 0..top {
            (cw, ch, px) = half(cw, ch, &px);
        }
        let f = 1usize << top;
        let mean = t.px.iter().map(|&v| v as f64).sum::<f64>() / (t.w * t.h) as f64;
        let ct: Vec<f64> = px.iter().map(|&v| v as f64 - mean).collect();
        let proj2 = (f * f) as f64 * ct.iter().map(|&v| v * v).sum::<f64>();
        let res = (norm * norm - proj2).max(0.0).sqrt();
        Self { f, cw, ch, ct, res }
    }
}

impl Stack {
    pub(super) fn new(t: &Gray) -> Option<Self> {
        Self::with_kept(t, MIN_KEPT)
    }

    /// [`Stack::new`] with the kept-variance rule as an argument (tests).
    pub(super) fn with_kept(t: &Gray, min_kept: f64) -> Option<Self> {
        let base = Kernel::new(t.w, t.h, &t.px)?;
        let var0 = base.var();
        let mut levels = vec![base];
        let (mut w, mut h, mut px) = (t.w, t.h, t.px.clone());
        while levels.len() <= MAX_LEVEL && (w / 2).min(h / 2) >= MIN_COARSE_PX {
            (w, h, px) = half(w, h, &px);
            match Kernel::new(w, h, &px) {
                Some(k) if k.var() >= min_kept * var0 => levels.push(k),
                _ => break,
            }
        }
        let projs = (1..=4)
            .rev()
            .filter(|&l| (t.w >> l).min(t.h >> l) >= MIN_PROJ_BLOCKS)
            .map(|l| Proj::new(t, l, levels[0].norm))
            .collect();
        Some(Self { levels, projs })
    }
}

#[inline]
pub(super) fn dot(a: &[f32], b: &[f32]) -> f32 {
    let mut acc = [0.0f32; 8];
    let (ca, ra) = a.as_chunks::<8>();
    let (cb, rb) = b.as_chunks::<8>();
    for (x, y) in ca.iter().zip(cb) {
        for i in 0..8 {
            acc[i] += x[i] * y[i];
        }
    }
    let mut s = ((acc[0] + acc[1]) + (acc[2] + acc[3])) + ((acc[4] + acc[5]) + (acc[6] + acc[7]));
    for (x, y) in ra.iter().zip(rb) {
        s += x * y;
    }
    s
}

/// Zero-mean NCC of `k` at (x, y) of `p`; `None` on a flat window
/// ([`Level::window`]).
#[inline]
pub(super) fn ncc_at(p: &Level, k: &Kernel, x: usize, y: usize) -> Option<f64> {
    let (_, var) = p.window(x, y, k.w, k.h)?;
    let mut num = 0.0f64;
    for ty in 0..k.h {
        let row = &p.px[(y + ty) * p.w + x..][..k.w];
        num += dot(row, &k.z[ty * k.w..][..k.w]) as f64;
    }
    Some(num / (var.sqrt() * k.norm))
}

/// Best score of one search and its multiply-adds (window pixels scored).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Found {
    pub(super) score: f64,
    pub(super) macs: usize,
}

pub(super) const NONE_FOUND: Found = Found {
    score: -1.0,
    macs: 0,
};

/// Every position at level 0.
pub(super) fn search_full(region: &[Level], st: &Stack) -> Found {
    let (p, k) = (&region[0], &st.levels[0]);
    if k.w > p.w || k.h > p.h {
        return NONE_FOUND;
    }
    let mut out = NONE_FOUND;
    for y in 0..=p.h - k.h {
        for x in 0..=p.w - k.w {
            out.macs += k.w * k.h;
            if let Some(v) = ncc_at(p, k, x, y)
                && v > out.score
            {
                out.score = v;
            }
        }
    }
    out
}

/// Coarse to fine: every position at the coarsest level where the template
/// fits, then the best [`PEAKS`] peaks, each refined level by level in a
/// small window around twice its position. The score is level 0's.
///
/// Peaks that sit close together refine over the same windows. A window is
/// scored once per search (F263-3): its score is the same every time, so
/// the result is the same and only the repeats are saved.
///
/// [`search_fast_skipping`] also leaves out the level-0 windows that
/// cannot reach [`PRESENT`].
pub(super) fn search_fast(region: &[Level], st: &Stack) -> Found {
    search_fast_with(region, st, PEAKS, REFINE_FIRST, REFINE)
}

/// [`search_fast`] with its peak count and refine windows as arguments
/// (tests pin each one).
pub(super) fn search_fast_with(
    region: &[Level],
    st: &Stack,
    n_peaks: usize,
    refine_first: usize,
    refine: usize,
) -> Found {
    search_fast_once(region, st, n_peaks, refine_first, refine, true, &[])
}

/// Block sizes of the projection bounds [`search_fast_skipping`] checks,
/// coarsest first. Measured on 107 in-match frames with the words-v1 end
/// titles (2026-10-13): the 8-block bound puts 96.7% of the level-0 refine
/// windows under [`PRESENT`] and the 4-block bound the rest; they cost
/// about 1.5% and 6% of one exact score.
pub(super) const SKIP_BLOCKS: [usize; 2] = [8, 4];

/// [`search_fast`] for a read that runs on every poll tick (F263-3): a
/// level-0 refine window is scored only when no bound of `skip`
/// ([`bound_at`], an upper bound on its score) is below [`PRESENT`].
///
/// A window that is left out scores under [`PRESENT`], so a best score at
/// or above [`PRESENT`] is the same score [`search_fast`] returns, and one
/// below it stays below (it may be lower, or -1). The refine path does not
/// change: the levels above 0 are scored as before. The bound has the
/// float error [`certify`] describes (about 2e-4), far from the
/// `result::FLOOR` a sure read needs.
pub(super) fn search_fast_skipping(
    region: &[Level],
    st: &Stack,
    skip: &[(&Proj, &Phased)],
) -> Found {
    search_fast_once(region, st, PEAKS, REFINE_FIRST, REFINE, true, skip)
}

/// [`search_fast_with`]; `once` = false scores a window again every time a
/// peak's refine reaches it (the reference the tests compare with), and
/// `skip` is [`search_fast_skipping`]'s bounds (empty: every refine window
/// is scored).
pub(super) fn search_fast_once(
    region: &[Level],
    st: &Stack,
    n_peaks: usize,
    refine_first: usize,
    refine: usize,
    once: bool,
    skip: &[(&Proj, &Phased)],
) -> Found {
    let fits = |l: usize| {
        l < region.len()
            && l < st.levels.len()
            && st.levels[l].w <= region[l].w
            && st.levels[l].h <= region[l].h
    };
    let Some(top) = (0..st.levels.len()).rev().find(|&l| fits(l)) else {
        return NONE_FOUND;
    };
    if top == 0 || !fits(0) {
        return search_full(region, st);
    }
    let (p, k) = (&region[top], &st.levels[top]);
    let mut macs = 0;
    let mut coarse: Vec<(f64, usize, usize)> = Vec::new();
    for y in 0..=p.h - k.h {
        for x in 0..=p.w - k.w {
            macs += k.w * k.h;
            if let Some(v) = ncc_at(p, k, x, y) {
                coarse.push((v, x, y));
            }
        }
    }
    coarse.sort_by(|a, b| b.0.total_cmp(&a.0));
    let mut peaks: Vec<(usize, usize)> = Vec::with_capacity(n_peaks);
    for &(_, x, y) in &coarse {
        if peaks.len() == n_peaks {
            break;
        }
        if peaks
            .iter()
            .all(|&(px, py)| px.abs_diff(x) > PEAK_SEP || py.abs_diff(y) > PEAK_SEP)
        {
            peaks.push((x, y));
        }
    }
    let mut best = -1.0f64;
    // The windows scored so far in this search: (level, x, y) to its score.
    let mut scored: std::collections::HashMap<(usize, usize, usize), Option<f64>> =
        std::collections::HashMap::new();
    for (mut x, mut y) in peaks {
        let mut score = None;
        for l in (0..top).rev() {
            let (p, k) = (&region[l], &st.levels[l]);
            let r = if l + 1 == top { refine_first } else { refine };
            let (cx, cy) = ((2 * x).min(p.w - k.w), (2 * y).min(p.h - k.h));
            let mut lvl_best: Option<(f64, usize, usize)> = None;
            for yy in cy.saturating_sub(r)..=(cy + r).min(p.h - k.h) {
                for xx in cx.saturating_sub(r)..=(cx + r).min(p.w - k.w) {
                    let key = (l, xx, yy);
                    let v = match scored.get(&key) {
                        Some(&v) if once => v,
                        _ => {
                            // a flat window has no score either way
                            let out = l == 0
                                && skip.iter().any(|(pr, ph)| {
                                    bound_at(p, ph, k, pr, xx, yy)
                                        .is_none_or(|ub| ub + BOUND_SLACK < PRESENT as f64)
                                });
                            let v = if out {
                                None
                            } else {
                                macs += k.w * k.h;
                                ncc_at(p, k, xx, yy)
                            };
                            scored.insert(key, v);
                            v
                        }
                    };
                    if let Some(v) = v
                        && lvl_best.is_none_or(|b| v > b.0)
                    {
                        lvl_best = Some((v, xx, yy));
                    }
                }
            }
            (x, y) = lvl_best.map_or((cx, cy), |b| (b.1, b.2));
            score = lvl_best.map(|b| b.0);
        }
        if let Some(s) = score
            && s > best
        {
            best = s;
        }
    }
    Found { score: best, macs }
}

// ---------------------------------------------------------- certification

/// One phase image of box means, in f64, with its integral images.
struct Means {
    w: usize,
    px: Vec<f64>,
    s1: Vec<f64>,
    s2: Vec<f64>,
}

impl Means {
    fn new(w: usize, h: usize, px: Vec<f64>) -> Self {
        let mut s1 = vec![0.0f64; (w + 1) * (h + 1)];
        let mut s2 = vec![0.0f64; (w + 1) * (h + 1)];
        for y in 0..h {
            for x in 0..w {
                let v = px[y * w + x];
                let i = (y + 1) * (w + 1) + x + 1;
                s1[i] = v + s1[i - 1] + s1[i - w - 1] - s1[i - w - 2];
                s2[i] = v * v + s2[i - 1] + s2[i - w - 1] - s2[i - w - 2];
            }
        }
        Self { w, px, s1, s2 }
    }

    /// Sum of `s` over the `tw` x `th` window at (x, y).
    #[inline]
    fn rect(&self, s: &[f64], x: usize, y: usize, tw: usize, th: usize) -> f64 {
        let a = y * (self.w + 1) + x;
        let b = (y + th) * (self.w + 1) + x;
        s[b + tw] - s[b] - s[a + tw] + s[a]
    }
}

/// Box means of a level-0 region over `f` x `f` squares at every position,
/// split by phase: `phases[ay * f + ax]` holds the means at
/// `(ax + f i, ay + f j)`, so a window's blocks are a contiguous
/// `cw` x `ch` patch of one phase image. The means stay in f64
/// ([`bound_at`] subtracts the window mean from them).
pub(super) struct Phased {
    pub(super) f: usize,
    phases: Vec<Means>,
}

impl Phased {
    pub(super) fn new(p: &Level, f: usize) -> Self {
        let ff = (f * f) as f64;
        let count = |n: usize, a: usize| if n >= f + a { (n - f - a) / f + 1 } else { 0 };
        let mut phases = Vec::with_capacity(f * f);
        for ay in 0..f {
            for ax in 0..f {
                let (pw, ph) = (count(p.w, ax), count(p.h, ay));
                let mut px = Vec::with_capacity(pw * ph);
                for j in 0..ph {
                    for i in 0..pw {
                        px.push(p.rect(&p.s1, ax + f * i, ay + f * j, f, f) / ff);
                    }
                }
                phases.push(Means::new(pw, ph, px));
            }
        }
        Self { f, phases }
    }
}

/// `sum((a_i - m) * b_i)` in f64.
#[inline]
fn dot_centred(a: &[f64], b: &[f64], m: f64) -> f64 {
    let mut acc = [0.0f64; 4];
    let (ca, ra) = a.as_chunks::<4>();
    let (cb, rb) = b.as_chunks::<4>();
    for (x, y) in ca.iter().zip(cb) {
        for i in 0..4 {
            acc[i] += (x[i] - m) * y[i];
        }
    }
    let mut s = (acc[0] + acc[1]) + (acc[2] + acc[3]);
    for (x, y) in ra.iter().zip(rb) {
        s += (x - m) * y;
    }
    s
}

/// An upper bound on the NCC of `k` at (x, y) of `p`, from the block
/// projection. With `P` the projection onto the window's `f` x `f` block
/// means, `<p, t> = <Pp, Pt> + <(I-P)p, (I-P)t> <= <Pp, Pt> + |(I-P)p| |(I-P)t|`
/// (the two parts are orthogonal; Cauchy-Schwarz on the second).
/// `<Pp, Pt>` is a dot product of `cw * ch` block means instead of the
/// whole window. `None` on a flat window, where [`ncc_at`] has no score
/// either.
///
/// The block means are centred on the window mean before the dot product,
/// and the sum is in f64 (F253-11): on a bright, nearly flat window the
/// uncentred f32 sum lost up to 1.2e-3 of the score to cancellation.
#[inline]
pub(super) fn bound_at(
    p: &Level,
    ph: &Phased,
    k: &Kernel,
    pr: &Proj,
    x: usize,
    y: usize,
) -> Option<f64> {
    let n = (k.w * k.h) as f64;
    let (sum, var) = p.window(x, y, k.w, k.h)?;
    let m = sum / n;
    let f = ph.f;
    let q = &ph.phases[(y % f) * f + x % f];
    let (ox, oy) = (x / f, y / f);
    let sb = q.rect(&q.s1, ox, oy, pr.cw, pr.ch);
    let sb2 = q.rect(&q.s2, ox, oy, pr.cw, pr.ch);
    let mut db = 0.0f64;
    for j in 0..pr.ch {
        db += dot_centred(
            &q.px[(oy + j) * q.w + ox..][..pr.cw],
            &pr.ct[j * pr.cw..][..pr.cw],
            m,
        );
    }
    let ff = (f * f) as f64;
    let pp = ff * db;
    let proj2 = ff * (sb2 - 2.0 * m * sb + (pr.cw * pr.ch) as f64 * m * m);
    let res_p = (var - proj2).max(0.0).sqrt();
    Some((pp + res_p * pr.res) / (var.sqrt() * k.norm))
}

/// Fewest blocks on a template side for a projection bound.
const MIN_PROJ_BLOCKS: usize = 3;
/// Slack on the bound for float rounding (the f32 region pixels; the
/// projection itself is in f64). It covers the bound's own error, not
/// [`ncc_at`]'s: that score is an uncentred f32 dot and overshoots the exact
/// score by up to about 1.7e-4 on bright, nearly flat windows (F257-5).
pub(super) const BOUND_SLACK: f64 = 1e-4;
/// Multiply-adds one read may spend on exact scores while certifying.
pub(super) const CERT_MACS: usize = 300_000_000;

/// Whether a template's exhaustive best reaches `tau`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Cert {
    /// No window scores `tau` or more.
    Below,
    /// A window scores this (`>= tau`).
    Above(f64),
    /// The exact-score budget ran out first.
    Unknown,
}

/// Decide exactly whether any window of `st` scores at least `tau`: a
/// window is ruled out when any bound of the chain ([`bound_at`], one per
/// projection, coarsest and cheapest first) is below `tau`; every other
/// window is scored with [`ncc_at`]. The same answer as [`search_full`]
/// against `tau`, at a small part of its cost when the bounds are tight,
/// up to [`ncc_at`]'s own f32 error (about 1.7e-4 at most, on bright,
/// nearly flat windows; F257-5). Both ways: a window that error lifts over
/// `tau` may be ruled out by its bound, so `certify` says `Below` where
/// [`search_full`] reports a score at `tau`; and a window that is scored
/// exactly can return `Above` on that error. For a result read `Above`
/// only flags it. Without projections the stack is searched in full.
pub(super) fn certify(
    region: &[Level],
    chain: &[(&Proj, &Phased)],
    st: &Stack,
    tau: f64,
    budget: &mut usize,
) -> Cert {
    let (p, k) = (&region[0], &st.levels[0]);
    if k.w > p.w || k.h > p.h {
        return Cert::Below;
    }
    if chain.is_empty() {
        let full = search_full(region, st);
        *budget = budget.saturating_sub(full.macs);
        return if full.score >= tau {
            Cert::Above(full.score)
        } else {
            Cert::Below
        };
    }
    for y in 0..=p.h - k.h {
        'window_x: for x in 0..=p.w - k.w {
            for (pr, ph) in chain {
                match bound_at(p, ph, k, pr, x, y) {
                    None => continue 'window_x,
                    Some(ub) if ub + BOUND_SLACK < tau => continue 'window_x,
                    Some(_) => {}
                }
            }
            let cost = k.w * k.h;
            if *budget < cost {
                return Cert::Unknown;
            }
            *budget -= cost;
            if let Some(v) = ncc_at(p, k, x, y)
                && v >= tau
            {
                return Cert::Above(v);
            }
        }
    }
    Cert::Below
}

/// A work-scale region and its box-downscaled levels.
pub(super) fn pyramid(g: Gray) -> Vec<Level> {
    let mut levels = vec![Level::new(g.w, g.h, g.px)];
    while levels.len() <= MAX_LEVEL {
        let last = &levels[levels.len() - 1];
        let (hw, hh, px) = half(last.w, last.h, &last.px);
        if hw == 0 || hh == 0 {
            break;
        }
        levels.push(Level::new(hw, hh, px));
    }
    levels
}

/// The largest template PNG [`load_gray`] decodes: the 16:9 game rect at the
/// work scale. A template is a cut of a region of that rect, so a larger
/// file is not a template (F260-7).
pub(super) const MAX_TEMPLATE_PX: (u32, u32) = (1280, 720);

/// An 8-bit grey PNG as a [`Gray`] in 0..1, or `None` (missing, unreadable,
/// empty, or larger than [`MAX_TEMPLATE_PX`]). The size is read from the
/// header, before any pixel is decoded.
pub(super) fn load_gray(p: &Path) -> Option<Gray> {
    let (w, h) = image::image_dimensions(p).ok()?;
    if w > MAX_TEMPLATE_PX.0 || h > MAX_TEMPLATE_PX.1 {
        tracing::warn!(file = %p.display(), w, h, "template image too large; left out");
        return None;
    }
    let l = image::open(p).ok()?.to_luma8();
    if l.width() == 0 || l.height() == 0 {
        return None;
    }
    Some(Gray {
        w: l.width() as usize,
        h: l.height() as usize,
        px: l.pixels().map(|p| p.0[0] as f32 / 255.0).collect(),
    })
}

/// One template's stacks at the search scales ([`SCALES`]).
pub(super) fn scaled_stacks(t: &Gray) -> Vec<Stack> {
    SCALES
        .iter()
        .filter_map(|sc| {
            let tw = ((t.w as f64 * sc).round_ties_even() as usize).max(1);
            let th = ((t.h as f64 * sc).round_ties_even() as usize).max(1);
            Stack::new(&t.resize(tw, th))
        })
        .collect()
}

/// Make the phase images of region `ri` at block size `f`, once.
pub(super) fn make_phased(
    cache: &mut Vec<((usize, usize), Phased)>,
    ri: usize,
    levels: &[Level],
    f: usize,
) {
    if !cache.iter().any(|(k, _)| *k == (ri, f)) {
        cache.push(((ri, f), Phased::new(&levels[0], f)));
    }
}
