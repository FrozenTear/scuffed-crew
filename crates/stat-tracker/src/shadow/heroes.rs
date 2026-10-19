//! Tab row hero matcher (shadow mode only, log only).
//!
//! Port of the Scuffed Vision prototype (`hero_tmpl_work/eval2.py`):
//!
//! * Rows: the light header bar is found first, then each team block in the
//!   role-icon column is split into `team_size` equal rows. Pitch comes from
//!   the block itself, never from fixed pixels, so the same code reads a match
//!   board (76 px rows at 1440p), a Practice Range board (97 px) or a match
//!   history team screen.
//! * Portrait box: right of the role-icon square, `0.46 * (row_h + 2)` from
//!   the header's left edge, `row_h` square.
//! * Matching: the portrait interior (6% margin cut) is area-resized to 40x40
//!   colour and compared with each template by zero-mean normalised
//!   correlation over the template's foreground mask (team-colour background
//!   removed when the template was cut), with small shifts and scales 1.0
//!   and 0.9. A coarse pass at the centre picks the closest classes, then
//!   only those get the full shift search.
//! * Classes: one per hero, plus `?placeholder`, `skull` and `empty`, which
//!   are never a hero.
//! * Gate: a read is accepted when its score is at least [`MIN_SCORE`] and it
//!   leads the next class by at least [`MIN_LEAD`]. Everything else is
//!   flagged suspect and names no hero.
//!
//! Templates are not embedded. They are loaded at start from
//! `<data_dir>/templates/heroes/` (`<hero-key>.png`, RGBA with alpha as the
//! foreground mask, plus `special/<class>*.png`). When the directory is
//! missing or holds no hero template, the hero matcher is skipped.
//!
//! The new reader ([`super::reader`]) loads a [`HeroSet`] instead: the
//! heroes-v2 pack (`templates/heroes-v2/`, 74 px cuts) for 1440p-scale
//! boards and heroes-v2-1080 (`templates/heroes-v2-1080/`) for 1080p-scale
//! boards, each with the same file layout and `special/` handling as v1.
//! heroes-v1 stays the fallback: a v2 folder that is missing, or that lacks a
//! class v1 has, is not used. On a full frame whose 5v5 board is wider than
//! the fixed scoreboard crop, the header bar starts at the crop's left edge;
//! [`HeroSet::read_frame`] then re-crops further left before matching
//! ([`clipped_header_pad`]).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use image::{DynamicImage, RgbImage, RgbaImage};
use serde::Serialize;

/// Identifies this recognizer's output in the log. Bump on any change that
/// can move a read, score or suspect flag. `hero-v1.1`: [`MIN_SCORE`] 0.65.
/// `hero-v1.2`: a clipped 5v5 board read without the wider re-crop is
/// flagged ([`clipped_5v5`]).
pub const HERO_RECOGNIZER_ID: &str = "hero-v1.2";
/// Minimum correlation for an accepted read. 0.65 since alpha.7: half-portrait
/// patches from a clipped 5v5 board scored 0.60 to 0.64 with a 0.14 to 0.17
/// lead and passed the old 0.55 gate as the wrong hero.
pub const MIN_SCORE: f32 = 0.65;
/// Minimum lead over the best other class for an accepted read.
pub const MIN_LEAD: f32 = 0.10;
/// Template directory under the data dir (heroes-v1).
pub const TEMPLATE_SUBDIR: &str = "templates/heroes";
/// heroes-v2 pack directory under the data dir (1440p-scale cuts).
pub const TEMPLATE_SUBDIR_V2: &str = "templates/heroes-v2";
/// heroes-v2-1080 pack directory under the data dir (1080p-scale cuts).
pub const TEMPLATE_SUBDIR_V2_1080: &str = "templates/heroes-v2-1080";
/// Scoreboard crops at most this many px high use the 1080 set: about
/// halfway between the 1080p crop (756 px) and the 1440p crop (1007 px).
pub const MAX_1080_CROP_H: u32 = 881;
/// Re-crop this many px (at 1440p, scaled by the 16:9 height) further left
/// when the header bar starts at the scoreboard crop's left edge. 5v5 headers
/// start 24 to 90 px left of the crop at 1440p; on the bench 90 to 200 px all
/// read the same and 60 px was not enough.
pub const CLIPPED_HEADER_PAD_1440: u32 = 120;
/// Special (non-hero) classes, in file-name form.
pub const SPECIAL_CLASSES: [&str; 3] = ["placeholder", "skull", "empty"];

const N: usize = 40;
const MARGIN: f64 = 0.06;
const SCALES: [f64; 2] = [1.0, 0.9];
/// Classes that get the full shift search after the centre-only pass.
const COARSE_TOP: usize = 6;
/// Kernels per class that get the full shift search (best by centre score).
const FINE_KERNELS: usize = 1;
/// Extra kernels searched for the two leading classes.
const TOP2_EXTRA_KERNELS: usize = 1;
const LANES: usize = 8;
const MIN_MASK_PX: usize = 50;

/// What a row shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RowClass {
    Hero,
    Placeholder,
    Skull,
    Empty,
    /// Flagged: no class passed the gate.
    Unknown,
}

/// One row's read.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HeroRead {
    pub row: usize,
    pub class: RowClass,
    /// Hero key, only for an accepted hero read.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hero: Option<String>,
    /// Best class before the gate (hero key or special class).
    pub best: String,
    pub score: f32,
    /// Lead over the best other class.
    pub lead: f32,
    pub second: String,
    pub suspect: bool,
}

impl HeroRead {
    /// The hero key this row names: an accepted hero read that passed the
    /// gate. `None` for a suspect read, an unknown icon, a placeholder, a
    /// skull or an empty slot. The board's `rN.hero` field and the new
    /// reader's hero authority both use this.
    pub fn accepted(&self) -> Option<&str> {
        match (self.class, self.hero.as_deref()) {
            (RowClass::Hero, Some(hero))
                if !self.suspect && !scuffed_types::is_placeholder_hero(hero) =>
            {
                Some(hero)
            }
            _ => None,
        }
    }
}

/// One board's reads.
#[derive(Debug, Clone, PartialEq)]
pub struct HeroBoard {
    pub rows: Vec<HeroRead>,
    pub elapsed_us: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HeroError {
    NoHeader,
    NoRows,
    Unsupported(&'static str),
    OverBudget,
}

impl std::fmt::Display for HeroError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoHeader => f.write_str("no header bar"),
            Self::NoRows => f.write_str("rows not found"),
            Self::Unsupported(w) => write!(f, "unsupported: {w}"),
            Self::OverBudget => f.write_str("over budget"),
        }
    }
}

impl std::error::Error for HeroError {}

// ---------------------------------------------------------------- geometry

/// Borrowed 8-bit RGB or RGBA pixels, so a board is never copied.
#[derive(Clone, Copy)]
pub struct View<'a> {
    raw: &'a [u8],
    w: usize,
    h: usize,
    ch: usize,
}

impl<'a> View<'a> {
    pub fn rgb(img: &'a RgbImage) -> Self {
        Self {
            raw: img.as_raw(),
            w: img.width() as usize,
            h: img.height() as usize,
            ch: 3,
        }
    }

    pub fn rgba(img: &'a RgbaImage) -> Self {
        Self {
            raw: img.as_raw(),
            w: img.width() as usize,
            h: img.height() as usize,
            ch: 4,
        }
    }

    #[inline]
    fn px(&self, x: usize, y: usize) -> [u8; 3] {
        let i = (y * self.w + x) * self.ch;
        [self.raw[i], self.raw[i + 1], self.raw[i + 2]]
    }
}

/// Header bar `(x0, x1, y0, y1)`: the longest light, neutral run in the top
/// 60% of the image, grown vertically at its centre.
pub fn find_header(img: View<'_>) -> Option<(usize, usize, usize, usize)> {
    let (w, h) = (img.w, img.h);
    if w < 50 || h < 20 {
        return None;
    }
    let light = |x: usize, y: usize| {
        let [r, g, b] = img.px(x, y);
        let (r, g, b) = (r as i32, g as i32, b as i32);
        let mn = r.min(g).min(b);
        let mx = r.max(g).max(b);
        mn > 150 && mx - mn < 30
    };
    let min_len = (w as f64 * 0.15) as usize;
    let mut best: Option<(usize, usize, usize)> = None;
    let ylim = (h as f64 * 0.6) as usize;
    for y in 0..ylim {
        let mut count = 0usize;
        let (mut run_s, mut best_s, mut best_l) = (0usize, 0usize, 0usize);
        let mut in_run = false;
        for x in 0..w {
            if light(x, y) {
                count += 1;
                if !in_run {
                    in_run = true;
                    run_s = x;
                }
                let l = x + 1 - run_s;
                if l > best_l {
                    best_l = l;
                    best_s = run_s;
                }
            } else {
                in_run = false;
            }
        }
        if (count as f64) < w as f64 * 0.15 {
            continue;
        }
        let better = match best {
            None => true,
            Some((s, e, _)) => best_l > e - s + 5,
        };
        if best_l >= min_len && better {
            best = Some((best_s, best_s + best_l, y));
        }
    }
    let (x0, x1, y) = best?;
    let cx = (x0 + x1) / 2;
    let (ca, cb) = (cx.saturating_sub(20), (cx + 20).min(w));
    let col = |yy: usize| {
        let n = (ca..cb).filter(|&x| light(x, yy)).count();
        n as f64 / (cb - ca).max(1) as f64 > 0.5
    };
    let mut y0 = y;
    while y0 > 0 && col(y0 - 1) {
        y0 -= 1;
    }
    let mut y1 = y;
    while y1 < h - 1 && col(y1 + 1) {
        y1 += 1;
    }
    Some((x0, x1, y0, y1 + 1))
}

/// Row bands `(top, bottom)` for both teams: each team block in the
/// role-icon column is split into `team` equal rows. The last band may run
/// past the bottom of a cropped scoreboard.
pub fn find_rows(
    img: View<'_>,
    hdr: (usize, usize, usize, usize),
    team: usize,
) -> Vec<(usize, usize)> {
    let (w, h) = (img.w, img.h);
    let (x0, x1, y0, y1) = hdr;
    let hw = (x1 - x0) as f64;
    let a = (x0 as f64 + 0.006 * hw) as usize;
    let b = ((x0 as f64 + 0.026 * hw) as usize).min(w);
    if b <= a || y1 >= h || team == 0 {
        return Vec::new();
    }
    let on: Vec<bool> = (y1..h)
        .map(|y| {
            let s: u32 = (a..b)
                .map(|x| {
                    let [r, g, bl] = img.px(x, y);
                    r.max(g).max(bl) as u32
                })
                .sum();
            s as f64 / (b - a) as f64 > 60.0
        })
        .collect();
    let mut runs = Vec::new();
    let mut start = None;
    for (i, &v) in on.iter().chain(std::iter::once(&false)).enumerate() {
        match (v, start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                if i - s > 3 {
                    runs.push((s, i));
                }
                start = None;
            }
            _ => {}
        }
    }
    let Some(&first) = runs.first() else {
        return Vec::new();
    };
    let hh = (y1 - y0) as f64;
    let mut blocks = vec![first];
    for &(i, j) in &runs[1..] {
        let last = blocks.last_mut().expect("non-empty");
        if (i - last.1) as f64 <= 0.12 * hh + 4.0 {
            last.1 = j;
        } else {
            blocks.push((i, j));
        }
    }
    blocks.retain(|&(i, j)| (j - i) as f64 > 2.0 * hh);
    if blocks.len() < 2 {
        return Vec::new();
    }
    let (t1, t2) = (blocks[0], blocks[1]);
    let p1 = (t1.1 - t1.0 + 2) as f64 / team as f64;
    let mut p2 = (t2.1 - t2.0 + 2) as f64 / team as f64;
    // Team 2 cut short (dark rows) or cut by the image edge: trust team 1.
    if (p2 - p1).abs() > 0.08 * p1 || y1 + t2.1 >= h - 3 {
        p2 = p1;
    }
    let mut rows = Vec::with_capacity(2 * team);
    for (start, p) in [(t1.0, p1), (t2.0, p2)] {
        for k in 0..team {
            let top = y1 + (start as f64 + k as f64 * p).round() as usize;
            let bot = (y1 + (start as f64 + (k + 1) as f64 * p).round() as usize).saturating_sub(2);
            rows.push((top, bot));
        }
    }
    rows
}

/// Portrait box `(x0, y0, x1, y1)` for one row band.
pub fn portrait_box(
    hdr: (usize, usize, usize, usize),
    row: (usize, usize),
) -> (usize, usize, usize, usize) {
    let (ry0, ry1) = row;
    let rh = ry1.saturating_sub(ry0);
    let px0 = hdr.0 + (0.46 * (rh + 2) as f64) as usize;
    (px0, ry0, px0 + rh, ry1)
}

// ---------------------------------------------------------------- pixels

/// Planar colour patch with a foreground mask.
#[derive(Debug, Clone)]
struct Patch {
    n: usize,
    /// `[channel][y * n + x]`
    c: [Vec<f32>; 3],
    mask: Vec<f32>,
}

/// Area-average resize weights (OpenCV INTER_AREA for shrinking).
fn area_weights(src: usize, dst: usize) -> Vec<Vec<(usize, f32)>> {
    let scale = src as f64 / dst as f64;
    (0..dst)
        .map(|o| {
            let (s0, s1) = (o as f64 * scale, (o + 1) as f64 * scale);
            let mut v = Vec::new();
            let mut i = s0.floor() as usize;
            while (i as f64) < s1 && i < src {
                let lo = s0.max(i as f64);
                let hi = s1.min((i + 1) as f64);
                if hi > lo {
                    v.push((i, ((hi - lo) / scale) as f32));
                }
                i += 1;
            }
            v
        })
        .collect()
}

/// Resize a `sw x sh` plane to `n x n` with area weights.
fn resize_plane(src: &[f32], sw: usize, sh: usize, n: usize) -> Vec<f32> {
    let wx = area_weights(sw, n);
    let wy = area_weights(sh, n);
    let mut tmp = vec![0f32; n * sh];
    for y in 0..sh {
        let row = &src[y * sw..(y + 1) * sw];
        for (ox, ws) in wx.iter().enumerate() {
            tmp[y * n + ox] = ws.iter().map(|&(i, w)| row[i] * w).sum();
        }
    }
    let mut out = vec![0f32; n * n];
    for (oy, ws) in wy.iter().enumerate() {
        for ox in 0..n {
            out[oy * n + ox] = ws.iter().map(|&(i, w)| tmp[i * n + ox] * w).sum();
        }
    }
    out
}

/// Cut `(x0, y0, x1, y1)`, drop the margin, resize to 40x40. `px(x, y)`
/// returns `[r, g, b, alpha]` with alpha in 0..=1. Channel order is B, G, R
/// like the prototype (the score does not care).
fn prep_with(
    (iw, ih): (usize, usize),
    bx: (usize, usize, usize, usize),
    px: impl Fn(usize, usize) -> [f32; 4],
) -> Option<Patch> {
    let (x0, y0, x1, y1) = bx;
    // The last row can run past a scoreboard crop's bottom edge: read the
    // visible part (as the prototype did) when at most 20% is cut off.
    let full_h = y1.checked_sub(y0)?;
    let y1 = y1.min(ih);
    let (w, h) = (x1.checked_sub(x0)?, y1.checked_sub(y0)?);
    if w < 8 || h < 8 || x1 > iw || h * 5 < full_h * 4 {
        return None;
    }
    let m = (w.min(h) as f64 * MARGIN).round() as usize;
    let (cw, ch) = (w - 2 * m, h - 2 * m);
    let mut planes: [Vec<f32>; 4] = std::array::from_fn(|_| Vec::with_capacity(cw * ch));
    for y in y0 + m..y0 + m + ch {
        for x in x0 + m..x0 + m + cw {
            let p = px(x, y);
            planes[0].push(p[2]);
            planes[1].push(p[1]);
            planes[2].push(p[0]);
            planes[3].push(p[3]);
        }
    }
    let c = std::array::from_fn(|k| resize_plane(&planes[k], cw, ch, N));
    let mask = resize_plane(&planes[3], cw, ch, N);
    Some(Patch { n: N, c, mask })
}

/// Template image (alpha is the foreground mask when present).
fn prep_rgba(img: &RgbaImage, use_alpha: bool) -> Option<Patch> {
    let (w, h) = (img.width() as usize, img.height() as usize);
    prep_with((w, h), (0, 0, w, h), |x, y| {
        let p = img.get_pixel(x as u32, y as u32).0;
        let a = if use_alpha { p[3] as f32 / 255.0 } else { 1.0 };
        [p[0] as f32, p[1] as f32, p[2] as f32, a]
    })
}

/// Query crop from the board.
fn prep_view(img: View<'_>, bx: (usize, usize, usize, usize)) -> Option<Patch> {
    prep_with((img.w, img.h), bx, |x, y| {
        let [r, g, b] = img.px(x, y);
        [r as f32, g as f32, b as f32, 1.0]
    })
}

fn rescale(p: &Patch, k: usize) -> Patch {
    if k == p.n {
        return p.clone();
    }
    Patch {
        n: k,
        c: std::array::from_fn(|i| resize_plane(&p.c[i], p.n, p.n, k)),
        mask: resize_plane(&p.mask, p.n, p.n, k),
    }
}

/// A kernel at full size (40 px, shift search) and half size (20 px, the
/// centre-only ranking pass, a quarter of the work).
#[derive(Debug, Clone)]
struct KernelPair {
    full: Kernel,
    small: Kernel,
}

impl KernelPair {
    fn new(p: &Patch, small: &Patch, scale: f64) -> Self {
        Self {
            full: Kernel::new(p, scale),
            small: Kernel::new(small, scale),
        }
    }
}

/// One template at one scale, ready for correlation. Dense `side x side`
/// arrays with the mask as 0/1 weights, so the inner loops vectorise.
#[derive(Debug, Clone)]
struct Kernel {
    side: usize,
    /// Foreground pixel count.
    n: f32,
    /// 1.0 on the foreground, 0.0 elsewhere.
    w: Vec<f32>,
    /// Zero-mean template values per channel, 0.0 off the foreground.
    dev: [Vec<f32>; 3],
    norm: f32,
}

impl Kernel {
    fn new(p: &Patch, scale: f64) -> Self {
        let k = (p.n as f64 * scale).round() as usize;
        let t = rescale(p, k);
        let side = (k as f64 * 0.8) as usize;
        let o = (k - side) / 2;
        let at = |y: usize, x: usize| (y + o) * k + x + o;
        let mut w: Vec<f32> = (0..side * side)
            .map(|i| f32::from(t.mask[at(i / side, i % side)] > 0.5))
            .collect();
        if w.iter().sum::<f32>() < MIN_MASK_PX as f32 {
            w.fill(1.0);
        }
        let n = w.iter().sum::<f32>();
        let mut norm = 0f32;
        let dev = std::array::from_fn(|ch| {
            let vals: Vec<f32> = (0..side * side)
                .map(|i| t.c[ch][at(i / side, i % side)])
                .collect();
            let mean = vals.iter().zip(&w).map(|(v, m)| v * m).sum::<f32>() / n;
            let d: Vec<f32> = vals.iter().zip(&w).map(|(v, m)| (v - mean) * m).collect();
            norm += d.iter().map(|a| a * a).sum::<f32>();
            d
        });
        Self {
            side,
            n,
            w,
            dev,
            norm,
        }
    }

    /// Correlation at window offset `(dy, dx)` in the query. Sums run in
    /// [`LANES`] independent lanes so the compiler can vectorise them.
    fn at(&self, q: &Patch, dy: usize, dx: usize) -> f32 {
        let side = self.side;
        let (mut num, mut var) = (0f32, 0f32);
        for ch in 0..3 {
            let mut s = [0f32; LANES];
            let mut s2 = [0f32; LANES];
            let mut sp = [0f32; LANES];
            let plane = &q.c[ch];
            for y in 0..side {
                let start = (y + dy) * q.n + dx;
                let qrow = &plane[start..start + side];
                let wrow = &self.w[y * side..(y + 1) * side];
                let drow = &self.dev[ch][y * side..(y + 1) * side];
                let (qc, qt) = qrow.as_chunks::<LANES>();
                let (wc, wt) = wrow.as_chunks::<LANES>();
                let (dc, dt) = drow.as_chunks::<LANES>();
                let tail = (qt, wt, dt);
                for ((qv, wv), dv) in qc.iter().zip(wc).zip(dc) {
                    for l in 0..LANES {
                        let vm = qv[l] * wv[l];
                        s[l] += vm;
                        s2[l] += vm * qv[l];
                        sp[l] += qv[l] * dv[l];
                    }
                }
                for ((&v, &m), &d) in tail.0.iter().zip(tail.1).zip(tail.2) {
                    let vm = v * m;
                    s[0] += vm;
                    s2[0] += vm * v;
                    sp[0] += v * d;
                }
            }
            let (s, s2, sp): (f32, f32, f32) = (s.iter().sum(), s2.iter().sum(), sp.iter().sum());
            num += sp;
            var += s2 - s * s / self.n;
        }
        let den = (self.norm * var.max(0.0)).sqrt();
        if den < 1e-3 { 0.0 } else { num / den }
    }

    fn centre(&self, q: &Patch) -> f32 {
        let o = (q.n - self.side) / 2;
        self.at(q, o, o)
    }

    /// Best correlation over offsets: hill climb from the centre, first in
    /// steps of 2 px, then 1 px. Matches the exhaustive search on our data
    /// (see the bench) at a fraction of the cost.
    fn best(&self, q: &Patch) -> f32 {
        let span = (q.n - self.side) as i32;
        let c = span / 2;
        let mut seen = vec![f32::NAN; ((span + 1) * (span + 1)) as usize];
        let mut eval = |y: i32, x: i32| {
            let i = (y * (span + 1) + x) as usize;
            if seen[i].is_nan() {
                seen[i] = self.at(q, y as usize, x as usize);
            }
            seen[i]
        };
        let (mut by, mut bx) = (c, c);
        let mut best = eval(c, c);
        for step in [2, 1] {
            loop {
                let mut moved = false;
                for (dy, dx) in [
                    (-1, -1),
                    (-1, 0),
                    (-1, 1),
                    (0, -1),
                    (0, 1),
                    (1, -1),
                    (1, 0),
                    (1, 1),
                ] {
                    let (y, x) = (by + dy * step, bx + dx * step);
                    if (0..=span).contains(&y) && (0..=span).contains(&x) {
                        let v = eval(y, x);
                        if v > best {
                            (best, by, bx, moved) = (v, y, x, true);
                        }
                    }
                }
                if !moved {
                    break;
                }
            }
        }
        best
    }

    /// Exhaustive search, for tests.
    #[cfg(test)]
    fn best_full(&self, q: &Patch) -> f32 {
        let span = q.n - self.side;
        let mut best = -1f32;
        for dy in 0..=span {
            for dx in 0..=span {
                best = best.max(self.at(q, dy, dx));
            }
        }
        best
    }
}

// ---------------------------------------------------------------- templates

/// Class index, its best centre score, and `(score, kernel)` sorted best first.
type Ranked = (usize, f32, Vec<(f32, usize)>);

/// Decode one template file into kernels for `class`; unreadable files are
/// skipped (logged at debug).
fn add_template(by: &mut HashMap<String, Vec<KernelPair>>, class: String, path: &Path) {
    match image::open(path) {
        Ok(img) => {
            let alpha = img.color().has_alpha();
            if let Some(p) = prep_rgba(&img.to_rgba8(), alpha) {
                let small = rescale(&p, N / 2);
                by.entry(class)
                    .or_default()
                    .extend(SCALES.iter().map(|&s| KernelPair::new(&p, &small, s)));
            }
        }
        Err(e) => tracing::debug!(error = %e, path = %path.display(), "hero template skipped"),
    }
}

/// Top-level placeholder, skull, and empty icons are not hero classes.
fn template_class(stem: &str) -> String {
    match special_row_class(stem) {
        Some(RowClass::Skull) => "?skull".to_string(),
        Some(RowClass::Empty) => "?empty".to_string(),
        Some(_) => "?placeholder".to_string(),
        None => stem.to_string(),
    }
}

/// The accept gate: at least [`MIN_SCORE`] and a lead of [`MIN_LEAD`].
pub fn passes_gate(score: f32, lead: f32) -> bool {
    score >= MIN_SCORE && lead >= MIN_LEAD
}

/// Placeholder, skull, and empty reads name no hero.
fn special_row_class(name: &str) -> Option<RowClass> {
    if !scuffed_types::is_placeholder_hero(name) {
        return None;
    }
    let bare = scuffed_types::pack_file_stem(name)
        .trim_start_matches('?')
        .to_ascii_lowercase();
    if bare == "skull" || bare.starts_with("skull_") || bare.starts_with("skull-") {
        Some(RowClass::Skull)
    } else if bare == "empty" || bare.starts_with("empty_") || bare.starts_with("empty-") {
        Some(RowClass::Empty)
    } else {
        Some(RowClass::Placeholder)
    }
}

/// Loaded template set: one entry per class, each with one or more kernels.
#[derive(Debug, Clone)]
pub struct HeroTemplates {
    classes: Vec<(String, Vec<KernelPair>)>,
    heroes: usize,
}

impl HeroTemplates {
    /// Default location under the data dir.
    pub fn dir_in(data_dir: &Path) -> PathBuf {
        data_dir.join(TEMPLATE_SUBDIR)
    }

    /// Load `<dir>/<hero>.png` and `<dir>/special/<class>*.png`. `None` when
    /// the directory is missing or holds no readable hero template.
    pub fn load_dir(dir: &Path) -> Option<Self> {
        let mut by: HashMap<String, Vec<KernelPair>> = HashMap::new();
        let pngs = |d: &Path| -> Vec<PathBuf> {
            let mut v: Vec<PathBuf> = std::fs::read_dir(d)
                .map(|it| {
                    it.filter_map(|e| e.ok().map(|e| e.path()))
                        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("png")))
                        .collect()
                })
                .unwrap_or_default();
            v.sort();
            v
        };
        for p in pngs(dir) {
            if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                add_template(&mut by, template_class(stem), &p);
            }
        }
        let heroes = by.keys().filter(|name| !name.starts_with('?')).count();
        if heroes == 0 {
            return None;
        }
        for p in pngs(&dir.join("special")) {
            let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            if let Some(c) = SPECIAL_CLASSES.iter().find(|c| stem.starts_with(**c)) {
                add_template(&mut by, format!("?{c}"), &p);
            }
        }
        let mut classes: Vec<_> = by.into_iter().collect();
        classes.sort_by(|a, b| a.0.cmp(&b.0));
        Some(Self { classes, heroes })
    }

    pub fn hero_count(&self) -> usize {
        self.heroes
    }

    pub fn class_count(&self) -> usize {
        self.classes.len()
    }

    /// True when this set has every class `other` has (heroes and special
    /// classes).
    pub fn covers(&self, other: &HeroTemplates) -> bool {
        other
            .classes
            .iter()
            .all(|(name, _)| self.classes.iter().any(|(n, _)| n == name))
    }

    /// Best and runner-up `(class, score)` for one 40x40 query.
    fn rank(&self, q: &Patch) -> [(usize, f32); 2] {
        let qs = rescale(q, N / 2);
        // Centre-only pass over every kernel; remember each class's best kernels.
        let mut coarse: Vec<Ranked> = self
            .classes
            .iter()
            .enumerate()
            .map(|(i, (_, ks))| {
                let mut per: Vec<(f32, usize)> = ks
                    .iter()
                    .enumerate()
                    .map(|(j, k)| (k.small.centre(&qs), j))
                    .collect();
                per.sort_by(|a, b| b.0.total_cmp(&a.0));
                (i, per[0].0, per)
            })
            .collect();
        coarse.sort_by(|a, b| b.1.total_cmp(&a.1));
        let mut fine: Vec<(usize, f32)> = coarse
            .iter()
            .take(COARSE_TOP)
            .map(|(i, _, per)| {
                let ks = &self.classes[*i].1;
                let s = per
                    .iter()
                    .take(FINE_KERNELS)
                    .map(|&(_, j)| ks[j].full.best(q))
                    .fold(-1.0, f32::max);
                (*i, s)
            })
            .collect();
        fine.sort_by(|a, b| b.1.total_cmp(&a.1));
        // The two leaders decide the read and its lead: search their other
        // kernels (the other scale) too.
        for f in fine.iter_mut().take(2) {
            let ks = &self.classes[f.0].1;
            let per = &coarse.iter().find(|c| c.0 == f.0).expect("ranked").2;
            for &(_, j) in per.iter().skip(FINE_KERNELS).take(TOP2_EXTRA_KERNELS) {
                f.1 = f.1.max(ks[j].full.best(q));
            }
        }
        fine.sort_by(|a, b| b.1.total_cmp(&a.1));
        let second = fine.get(1).copied().unwrap_or((fine[0].0, -1.0));
        [fine[0], second]
    }

    fn read_patch(&self, row: usize, q: &Patch) -> HeroRead {
        let [(b, s), (b2, s2)] = self.rank(q);
        let best = self.classes[b].0.clone();
        let lead = s - s2;
        let ok = passes_gate(s, lead);
        let class = if !ok {
            RowClass::Unknown
        } else {
            special_row_class(&best).unwrap_or(RowClass::Hero)
        };
        HeroRead {
            row,
            class,
            hero: (class == RowClass::Hero).then(|| best.clone()),
            best,
            score: s,
            lead,
            second: self.classes[b2].0.clone(),
            suspect: !ok,
        }
    }

    /// Read every row of one scoreboard image.
    pub fn read_board(
        &self,
        board: &DynamicImage,
        team_size: usize,
        budget: Duration,
    ) -> Result<HeroBoard, HeroError> {
        let t0 = Instant::now();
        if !(5..=6).contains(&team_size) {
            return Err(HeroError::Unsupported("team size"));
        }
        let mut owned = None;
        let view = view_of(board, &mut owned);
        let hdr = find_header(view).ok_or(HeroError::NoHeader)?;
        self.read_rows(view, hdr, team_size, budget, t0)
    }

    /// Rows and portraits of one board whose header bar is already known.
    fn read_rows(
        &self,
        view: View<'_>,
        hdr: (usize, usize, usize, usize),
        team_size: usize,
        budget: Duration,
        t0: Instant,
    ) -> Result<HeroBoard, HeroError> {
        let bands = find_rows(view, hdr, team_size);
        if bands.len() != 2 * team_size {
            return Err(HeroError::NoRows);
        }
        let clipped = clipped_5v5(team_size, hdr.0);
        let mut rows = Vec::with_capacity(bands.len());
        for (i, band) in bands.into_iter().enumerate() {
            if t0.elapsed() > budget {
                return Err(HeroError::OverBudget);
            }
            let q = prep_view(view, portrait_box(hdr, band)).ok_or(HeroError::NoRows)?;
            let mut read = self.read_patch(i, &q);
            if clipped {
                read.class = RowClass::Unknown;
                read.hero = None;
                read.suspect = true;
            }
            rows.push(read);
        }
        Ok(HeroBoard {
            rows,
            elapsed_us: t0.elapsed().as_micros() as u64,
        })
    }
}

/// A 5v5 header bar that starts at the image's left edge: the table runs
/// past it, so the role-icon column and the portraits' left edge are cut off
/// and rows and portrait boxes cannot be placed. On 20 real 1440p 5v5 crops
/// read this way (no re-crop), 9 of 200 rows passed the gate, all 9 as the
/// wrong hero. Every row of such a read is flagged; [`HeroSet::read_frame`]
/// re-crops wider first, so only a read that still sees a clipped header is.
fn clipped_5v5(team_size: usize, header_x0: usize) -> bool {
    team_size == 5 && header_x0 == 0
}

/// Borrow `img` as a [`View`], converting into `owned` only when it is not
/// 8-bit RGB or RGBA.
fn view_of<'a>(img: &'a DynamicImage, owned: &'a mut Option<RgbImage>) -> View<'a> {
    match img {
        DynamicImage::ImageRgb8(r) => View::rgb(r),
        DynamicImage::ImageRgba8(r) => View::rgba(r),
        other => View::rgb(owned.insert(other.to_rgb8())),
    }
}

// ---------------------------------------------------------------- packs

/// Which pack a template set came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum HeroPack {
    #[serde(rename = "heroes-v1")]
    V1,
    #[serde(rename = "heroes-v2")]
    V2,
    #[serde(rename = "heroes-v2-1080")]
    V2At1080,
}

impl HeroPack {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::V1 => "heroes-v1",
            Self::V2 => "heroes-v2",
            Self::V2At1080 => "heroes-v2-1080",
        }
    }
}

/// The new reader's hero templates: one set for 1440p-scale boards and an
/// optional one for 1080p-scale boards.
///
/// * 1440p-scale (crop higher than [`MAX_1080_CROP_H`]): heroes-v2 when it
///   is complete, else heroes-v1. heroes-v2-1080 is never used here.
/// * 1080p-scale (crop at most [`MAX_1080_CROP_H`]): heroes-v2-1080 when it
///   is complete, else the 1440p set.
///
/// A v2 set is complete when it has every class heroes-v1 has (each hero and
/// each special class). Without heroes-v1 there is nothing to compare with,
/// and a v2 set that loads is used as it is.
#[derive(Debug, Clone)]
pub struct HeroSet {
    main: Option<(HeroPack, HeroTemplates)>,
    at_1080: Option<(HeroPack, HeroTemplates)>,
}

/// One board's reads through a [`HeroSet`], and how they were made.
#[derive(Debug, Clone, PartialEq)]
pub struct HeroFrameRead {
    pub board: HeroBoard,
    pub pack: HeroPack,
    /// Frame px the hero crop was widened to the left by (clipped header).
    pub pad_px: Option<u32>,
}

impl HeroSet {
    /// `<data_dir>/templates/{heroes,heroes-v2,heroes-v2-1080}`.
    pub fn load_data_dir(data_dir: &Path) -> Option<Self> {
        Self::load_dirs(
            Some(&HeroTemplates::dir_in(data_dir)),
            Some(&data_dir.join(TEMPLATE_SUBDIR_V2)),
            Some(&data_dir.join(TEMPLATE_SUBDIR_V2_1080)),
        )
    }

    /// Load the three sets and pick as described on [`HeroSet`]. `None`
    /// when no set loads.
    pub fn load_dirs(v1: Option<&Path>, v2: Option<&Path>, v2_1080: Option<&Path>) -> Option<Self> {
        let v1 = v1.and_then(HeroTemplates::load_dir);
        let complete = |dir: Option<&Path>, pack: HeroPack| {
            let t = dir.and_then(HeroTemplates::load_dir)?;
            match &v1 {
                Some(base) if !t.covers(base) => {
                    tracing::warn!(
                        pack = pack.as_str(),
                        classes = t.class_count(),
                        v1_classes = base.class_count(),
                        "hero pack incomplete, not used"
                    );
                    None
                }
                _ => Some((pack, t)),
            }
        };
        let v2 = complete(v2, HeroPack::V2);
        let v2_1080 = complete(v2_1080, HeroPack::V2At1080);
        let main = v2.or_else(|| v1.map(|t| (HeroPack::V1, t)));
        if main.is_none() && v2_1080.is_none() {
            return None;
        }
        Some(Self {
            main,
            at_1080: v2_1080,
        })
    }

    /// The pack used for 1440p-scale boards, if any.
    pub fn main_pack(&self) -> Option<HeroPack> {
        self.main.as_ref().map(|m| m.0)
    }

    /// The pack used for 1080p-scale boards, if any.
    pub fn pack_1080(&self) -> Option<HeroPack> {
        self.at_1080.as_ref().or(self.main.as_ref()).map(|m| m.0)
    }

    /// The set for a scoreboard crop `board_h` px high. `None` for a
    /// 1440p-scale board when only heroes-v2-1080 is installed.
    pub fn for_board_height(&self, board_h: u32) -> Option<(HeroPack, &HeroTemplates)> {
        let pick = if board_h <= MAX_1080_CROP_H {
            self.at_1080.as_ref().or(self.main.as_ref())
        } else {
            self.main.as_ref()
        };
        pick.map(|(pack, t)| (*pack, t))
    }

    fn set_for(&self, board_h: u32) -> Result<(HeroPack, &HeroTemplates), HeroError> {
        self.for_board_height(board_h)
            .ok_or(HeroError::Unsupported("no hero pack for this board size"))
    }

    /// Read a scoreboard crop with the set for its height (no re-crop).
    pub fn read_board(
        &self,
        board: &DynamicImage,
        team_size: usize,
        budget: Duration,
    ) -> Result<HeroFrameRead, HeroError> {
        let (pack, t) = self.set_for(board.height())?;
        Ok(HeroFrameRead {
            board: t.read_board(board, team_size, budget)?,
            pack,
            pad_px: None,
        })
    }

    /// Read the heroes of `scoreboard`, cut from the full `frame` by
    /// `crop_scoreboard`. When the header bar starts at the crop's left
    /// edge, the board is wider than the crop: re-cut it
    /// [`clipped_header_pad`] px further left and read that instead. When
    /// the wide crop gives no header or no rows, the fixed crop is read.
    pub fn read_frame(
        &self,
        frame: &DynamicImage,
        scoreboard: &DynamicImage,
        team_size: usize,
        budget: Duration,
    ) -> Result<HeroFrameRead, HeroError> {
        let t0 = Instant::now();
        if !(5..=6).contains(&team_size) {
            return Err(HeroError::Unsupported("team size"));
        }
        let (pack, t) = self.set_for(scoreboard.height())?;
        let mut owned = None;
        let view = view_of(scoreboard, &mut owned);
        let hdr = find_header(view).ok_or(HeroError::NoHeader)?;
        if let Some(pad) = clipped_header_pad(frame.width(), frame.height(), scoreboard, hdr.0) {
            let wide = hero_crop(frame, pad);
            let mut wide_owned = None;
            let wide_view = view_of(&wide, &mut wide_owned);
            let read = find_header(wide_view)
                .ok_or(HeroError::NoHeader)
                .and_then(|h| t.read_rows(wide_view, h, team_size, budget, t0));
            match read {
                Ok(board) => {
                    return Ok(HeroFrameRead {
                        board,
                        pack,
                        pad_px: Some(pad),
                    });
                }
                Err(e) => tracing::debug!(error = %e, "wide hero crop failed, fixed crop read"),
            }
        }
        Ok(HeroFrameRead {
            board: t.read_rows(view, hdr, team_size, budget, t0)?,
            pack,
            pad_px: None,
        })
    }
}

/// The career panel's hero portrait on one full Tab frame: the large icon
/// above the hero name, right of the table (OCR site T5 read the name under
/// it as text).
#[derive(Debug, Clone, PartialEq)]
pub struct PanelRead {
    /// The read of the best box (`row` is 0), through the board's gate.
    pub read: HeroRead,
    pub pack: HeroPack,
    /// Box in frame px `(x0, y0, x1, y1)`.
    pub box_px: (usize, usize, usize, usize),
    /// Box left relative to the header bar's right end, top relative to the
    /// game rect's top, and side, in 16:9 game-rect heights.
    pub rel: (f32, f32, f32),
}

/// Panel gate: the best of many searched boxes is kept, so the panel read
/// needs more than the board's [`MIN_SCORE`] / [`MIN_LEAD`]. On the alpha.7
/// gold frames all 99 right panel reads scored at least 0.959 with a lead
/// of at least 0.318, and every wrong best box at most 0.614 (lead at most
/// 0.19).
pub const PANEL_MIN_SCORE: f32 = 0.85;
/// See [`PANEL_MIN_SCORE`].
pub const PANEL_MIN_LEAD: f32 = 0.20;
/// Searched box left for [`HeroSet::read_panel`], in game-rect heights from
/// the header bar's right end: the panel moves sideways with the table
/// (perks, 5v5 and 6v6). Right reads on the gold frames landed at 0 to 0.03.
pub const PANEL_DX: (f64, f64) = (0.0, 0.06);
/// Searched box top, in game-rect heights from the game rect's top. The
/// panel does not move up or down with the table: a table pushed down by a
/// mode banner keeps the panel where it was.
pub const PANEL_TOP: (f64, f64) = (0.108, 0.168);
pub const PANEL_SIDES: [f64; 3] = [0.145, 0.16, 0.175];
const PANEL_STEP: f64 = 0.012;

impl PanelRead {
    /// The hero key when the read passed the panel gate: an accepted hero
    /// read ([`HeroRead::accepted`]) at [`PANEL_MIN_SCORE`] and
    /// [`PANEL_MIN_LEAD`] or more. `None` is an abstain.
    pub fn accepted(&self) -> Option<&str> {
        let r = &self.read;
        (r.score >= PANEL_MIN_SCORE && r.lead >= PANEL_MIN_LEAD)
            .then(|| r.accepted())
            .flatten()
    }
}

impl HeroSet {
    /// Read the hero portrait on the career panel right of the Tab table.
    /// The panel moves with the table (perks, 5v5 and 6v6), so the box is
    /// searched next to the header bar's right end: a coarse grid, then a
    /// finer one round the best box. `scoreboard` is the `crop_scoreboard`
    /// cut of `frame`.
    pub fn read_panel(
        &self,
        frame: &DynamicImage,
        scoreboard: &DynamicImage,
        budget: Duration,
    ) -> Result<PanelRead, HeroError> {
        let t0 = Instant::now();
        let (pack, t) = self.set_for(scoreboard.height())?;
        let (fw, fh) = (frame.width(), frame.height());
        let (cx, _, cw, ch) = crate::ocr::preprocess::crop_scoreboard_rect(fw, fh);
        if (scoreboard.width(), scoreboard.height()) != (cw, ch) {
            return Err(HeroError::Unsupported(
                "scoreboard is not this frame's crop",
            ));
        }
        let mut owned = None;
        let sview = view_of(scoreboard, &mut owned);
        let hdr = find_header(sview).ok_or(HeroError::NoHeader)?;
        let (_, gy, _, gh) = crate::ocr::preprocess::game_rect_16_9(fw, fh);
        let u = gh as f64;
        let (ax, ay) = (cx as f64 + hdr.1 as f64, gy as f64);
        let mut fowned = None;
        let fview = view_of(frame, &mut fowned);
        let eval = |dx: f64, dy: f64, s: f64| -> Option<PanelRead> {
            let (x0, y0, side) = (
                (ax + dx * u).round(),
                (ay + dy * u).round(),
                (s * u).round(),
            );
            if x0 < 0.0 || y0 < 0.0 {
                return None;
            }
            let bx = (
                x0 as usize,
                y0 as usize,
                (x0 + side) as usize,
                (y0 + side) as usize,
            );
            let q = prep_view(fview, bx)?;
            Some(PanelRead {
                read: t.read_patch(0, &q),
                pack,
                box_px: bx,
                rel: (dx as f32, dy as f32, s as f32),
            })
        };
        let better = |a: &Option<PanelRead>, b: &PanelRead| {
            a.as_ref().is_none_or(|a| b.read.score > a.read.score)
        };
        let mut best: Option<PanelRead> = None;
        let steps = |lo: f64, hi: f64, step: f64| {
            let n = ((hi - lo) / step).round() as usize;
            (0..=n).map(move |k| lo + k as f64 * step)
        };
        for s in PANEL_SIDES {
            for dy in steps(PANEL_TOP.0, PANEL_TOP.1, PANEL_STEP) {
                for dx in steps(PANEL_DX.0, PANEL_DX.1, PANEL_STEP) {
                    if t0.elapsed() > budget {
                        return Err(HeroError::OverBudget);
                    }
                    if let Some(r) = eval(dx, dy, s)
                        && better(&best, &r)
                    {
                        best = Some(r);
                    }
                }
            }
        }
        let coarse = best.clone().ok_or(HeroError::NoRows)?;
        let (bdx, bdy, bs) = (
            coarse.rel.0 as f64,
            coarse.rel.1 as f64,
            coarse.rel.2 as f64,
        );
        let h = PANEL_STEP / 2.0;
        for s in [bs - 0.0075, bs, bs + 0.0075] {
            for dy in [bdy - h, bdy, bdy + h] {
                for dx in [bdx - h, bdx, bdx + h] {
                    if let Some(r) = eval(dx, dy, s)
                        && better(&best, &r)
                    {
                        best = Some(r);
                    }
                }
            }
        }
        best.ok_or(HeroError::NoRows)
    }
}

/// Frame px to widen the hero crop to the left by, or `None` to keep the
/// scoreboard crop. Only when the header bar starts at the crop's left edge
/// (`header_x0 == 0`), `scoreboard` is the `crop_scoreboard` cut of a
/// `frame_w` x `frame_h` frame, and the frame has pixels left of the crop.
/// [`CLIPPED_HEADER_PAD_1440`] scaled by the 16:9 height, at most up to the
/// frame's left edge.
pub fn clipped_header_pad(
    frame_w: u32,
    frame_h: u32,
    scoreboard: &DynamicImage,
    header_x0: usize,
) -> Option<u32> {
    if header_x0 != 0 {
        return None;
    }
    let (x, _, cw, ch) = crate::ocr::preprocess::crop_scoreboard_rect(frame_w, frame_h);
    if x == 0 || (scoreboard.width(), scoreboard.height()) != (cw, ch) {
        return None;
    }
    let (_, _, _, gh) = crate::ocr::preprocess::game_rect_16_9(frame_w, frame_h);
    let pad = (CLIPPED_HEADER_PAD_1440 as f64 * gh as f64 / 1440.0).round() as u32;
    Some(pad.clamp(1, x))
}

/// `crop_scoreboard` widened `pad` px to the left: same top, height and
/// right edge.
pub fn hero_crop(frame: &DynamicImage, pad: u32) -> DynamicImage {
    let (x, y, cw, ch) =
        crate::ocr::preprocess::crop_scoreboard_rect(frame.width(), frame.height());
    let x0 = x.saturating_sub(pad);
    frame.crop_imm(x0, y, cw + (x - x0), ch)
}

// ---------------------------------------------------------------- live set

/// The three hero pack folders a [`HeroSet`] loads from.
#[derive(Debug, Clone, Default)]
pub struct HeroDirs {
    pub v1: Option<PathBuf>,
    pub v2: Option<PathBuf>,
    pub v2_1080: Option<PathBuf>,
}

/// Identity of one pack folder: device, inode and mtime, or `None` when it
/// is missing. Pack sync swaps a folder in with `renameat2(RENAME_EXCHANGE)`
/// or a rename, so a new pack gives the live path a new inode.
pub(crate) type DirStamp = Option<(u64, u64, i64, i64)>;
type Stamp = [DirStamp; 3];

pub(crate) fn dir_stamp(dir: Option<&Path>) -> DirStamp {
    use std::os::unix::fs::MetadataExt;
    let m = std::fs::metadata(dir?).ok()?;
    m.is_dir()
        .then(|| (m.dev(), m.ino(), m.mtime(), m.mtime_nsec()))
}

impl HeroDirs {
    pub fn in_data_dir(data_dir: &Path) -> Self {
        Self {
            v1: Some(HeroTemplates::dir_in(data_dir)),
            v2: Some(data_dir.join(TEMPLATE_SUBDIR_V2)),
            v2_1080: Some(data_dir.join(TEMPLATE_SUBDIR_V2_1080)),
        }
    }

    fn stamp(&self) -> Stamp {
        [
            dir_stamp(self.v1.as_deref()),
            dir_stamp(self.v2.as_deref()),
            dir_stamp(self.v2_1080.as_deref()),
        ]
    }

    /// Load the set, or `None` when a folder changed while it loaded (a
    /// swap ran meanwhile; the caller retries later). `during` runs between
    /// the first stamp and the load (tests use it to swap a folder in).
    fn load_stable(&self, during: impl FnOnce()) -> Option<(Option<HeroSet>, Stamp)> {
        let before = self.stamp();
        during();
        let set = HeroSet::load_dirs(
            self.v1.as_deref(),
            self.v2.as_deref(),
            self.v2_1080.as_deref(),
        );
        (self.stamp() == before).then_some((set, before))
    }
}

/// The new reader's [`HeroSet`], re-loaded when a hero pack folder changes.
///
/// Pack sync installs heroes-v2 and heroes-v2-1080 in the background after
/// the reader is set up. Each board checks the folders' stamps (three
/// `stat` calls). On a change it starts one background load and keeps
/// reading with the set it has, so capture never waits on a load. A load
/// that saw a folder change while it ran is thrown away and retried on a
/// later board, so a set never mixes files from before and after a swap.
#[derive(Debug, Default)]
pub struct LiveHeroSet {
    inner: std::sync::Arc<LiveInner>,
}

type Current = (Option<std::sync::Arc<HeroSet>>, Stamp);

#[derive(Debug, Default)]
struct LiveInner {
    dirs: HeroDirs,
    current: std::sync::RwLock<Current>,
    loader: std::sync::Mutex<Option<std::thread::JoinHandle<()>>>,
    #[cfg(test)]
    hooks: TestHooks,
}

/// Test probes on the reload thread: a spawn count, a delay before the
/// load, and a step run between the first stamp and the load.
#[cfg(test)]
#[derive(Default)]
struct TestHooks {
    spawns: std::sync::atomic::AtomicUsize,
    delay_ms: std::sync::atomic::AtomicU64,
    during: std::sync::Mutex<Option<Box<dyn FnOnce() + Send>>>,
}

#[cfg(test)]
impl std::fmt::Debug for TestHooks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TestHooks")
    }
}

impl LiveInner {
    /// What runs between the reload's first stamp and its load: nothing,
    /// except in tests.
    fn before_load(&self) -> impl FnOnce() + use<> {
        #[cfg(test)]
        {
            use std::sync::atomic::Ordering;
            let ms = self.hooks.delay_ms.load(Ordering::SeqCst);
            std::thread::sleep(std::time::Duration::from_millis(ms));
            let during = self.hooks.during.lock().unwrap().take();
            move || {
                if let Some(f) = during {
                    f();
                }
            }
        }
        #[cfg(not(test))]
        || {}
    }

    fn install(&self, (set, stamp): (Option<HeroSet>, Stamp)) {
        tracing::info!(
            heroes = set.is_some(),
            hero_pack = set
                .as_ref()
                .and_then(HeroSet::main_pack)
                .map(HeroPack::as_str),
            hero_pack_1080 = set
                .as_ref()
                .and_then(HeroSet::pack_1080)
                .map(HeroPack::as_str),
            "new reader hero packs loaded"
        );
        *self.current.write().unwrap_or_else(|e| e.into_inner()) = (set.map(Into::into), stamp);
    }
}

impl LiveHeroSet {
    /// Load now (reader setup). A folder that keeps changing during the
    /// first tries leaves the set empty; the next board loads it again.
    pub fn load(dirs: HeroDirs) -> Self {
        let inner = LiveInner {
            dirs,
            ..Default::default()
        };
        let first = (0..3).find_map(|_| inner.dirs.load_stable(|| {}));
        // An all-zero stamp never matches a real one: the next board reloads.
        inner.install(first.unwrap_or((None, [Some((0, 0, 0, 0)); 3])));
        Self {
            inner: inner.into(),
        }
    }

    /// The set for this board. Starts a background reload when a pack
    /// folder changed since the set was loaded.
    pub fn current(&self) -> Option<std::sync::Arc<HeroSet>> {
        let (set, stamp) = self
            .inner
            .current
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if self.inner.dirs.stamp() != stamp {
            self.start_reload();
        }
        set
    }

    fn start_reload(&self) {
        let Ok(mut loader) = self.inner.loader.try_lock() else {
            return;
        };
        if loader.as_ref().is_some_and(|h| !h.is_finished()) {
            return;
        }
        let inner = std::sync::Arc::clone(&self.inner);
        let spawned = std::thread::Builder::new()
            .name("hero-pack-reload".into())
            .spawn(move || {
                let during = inner.before_load();
                match inner.dirs.load_stable(during) {
                    Some(loaded) => inner.install(loaded),
                    None => tracing::debug!("hero pack changed while loading, retry later"),
                }
            });
        match spawned {
            Ok(handle) => {
                #[cfg(test)]
                self.inner
                    .hooks
                    .spawns
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                *loader = Some(handle);
            }
            Err(e) => tracing::warn!(error = %e, "hero pack reload not started"),
        }
    }

    /// Reload threads started so far (tests).
    #[cfg(test)]
    pub(crate) fn reload_spawns(&self) -> usize {
        self.inner
            .hooks
            .spawns
            .load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Make each reload wait `ms` before it loads (tests).
    #[cfg(test)]
    pub(crate) fn set_load_delay_ms(&self, ms: u64) {
        self.inner
            .hooks
            .delay_ms
            .store(ms, std::sync::atomic::Ordering::SeqCst);
    }

    /// Run `f` inside the next reload, between its first stamp and its load
    /// (tests).
    #[cfg(test)]
    pub(crate) fn during_next_load(&self, f: impl FnOnce() + Send + 'static) {
        *self.inner.hooks.during.lock().unwrap() = Some(Box::new(f));
    }

    /// Wait for a reload in flight (tests).
    #[cfg(test)]
    pub(crate) fn wait_reload(&self) {
        let handle = self.inner.loader.lock().unwrap().take();
        if let Some(h) = handle {
            h.join().unwrap();
        }
    }
}

// ---------------------------------------------------------------- voting

/// Per-row hero across the frames of one session. The first accepted hero
/// read sets a row; a different hero needs two accepted reads in a row to
/// replace it (mid-game swaps). Flagged and special reads change nothing.
#[derive(Debug, Default, Clone)]
pub struct HeroVoter {
    session: String,
    rows: Vec<RowVote>,
}

#[derive(Debug, Default, Clone)]
struct RowVote {
    current: Option<String>,
    pending: Option<(String, u8)>,
}

impl HeroVoter {
    /// Feed one board; returns the voted hero per row after it.
    pub fn update(&mut self, session: &str, reads: &[HeroRead]) -> Vec<Option<String>> {
        if session != self.session {
            self.session = session.to_string();
            self.rows.clear();
        }
        for r in reads {
            if self.rows.len() <= r.row {
                self.rows.resize_with(r.row + 1, RowVote::default);
            }
            let Some(hero) = r.hero.as_ref() else {
                continue;
            };
            let v = &mut self.rows[r.row];
            match &v.current {
                None => v.current = Some(hero.clone()),
                Some(c) if c == hero => v.pending = None,
                Some(_) => {
                    let n = match &v.pending {
                        Some((p, n)) if p == hero => n + 1,
                        _ => 1,
                    };
                    if n >= 2 {
                        v.current = Some(hero.clone());
                        v.pending = None;
                    } else {
                        v.pending = Some((hero.clone(), n));
                    }
                }
            }
        }
        self.rows.iter().map(|v| v.current.clone()).collect()
    }
}

// ---------------------------------------------------------------- log record

/// One row in the hero log line.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HeroRowLog {
    #[serde(flatten)]
    pub read: HeroRead,
    /// Row hero after frame voting in this session.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub voted: Option<String>,
}

/// One line of `<data_dir>/shadow/heroes.jsonl`. Hero keys and scores only:
/// no player names, no images.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HeroRecord {
    pub ts: String,
    pub session: String,
    pub recognizer: &'static str,
    /// Template pack that made the reads (fixed names, [`HeroPack`]).
    pub hero_pack: HeroPack,
    /// The hero crop was widened left for a clipped header.
    pub recropped: bool,
    pub resolution: u32,
    pub team_size: usize,
    pub owner_row: Option<usize>,
    pub elapsed_us: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub rows: Vec<HeroRowLog>,
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) mod test_fixtures {
    //! Synthetic boards and templates for tests. No game art.
    use super::*;
    use image::{Rgb, Rgba};

    pub(crate) const PURPLE: [u8; 3] = [110, 20, 120];
    pub(crate) const YELLOW: [u8; 3] = [150, 125, 10];
    pub(crate) const TEAL: [u8; 3] = [20, 110, 120];

    /// Deterministic synthetic "portrait" for class `id`: a few coloured
    /// shapes on the background. No game art.
    pub(crate) fn portrait(id: u32, size: u32, bg: Option<[u8; 3]>) -> RgbaImage {
        let mut img = RgbaImage::from_pixel(size, size, Rgba([0, 0, 0, 0]));
        if let Some(b) = bg {
            for p in img.pixels_mut() {
                *p = Rgba([b[0], b[1], b[2], 255]);
            }
        }
        let mut seed = id.wrapping_mul(2_654_435_761).wrapping_add(12345);
        let mut rnd = |m: u32| {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed % m
        };
        let s = size as f32;
        for _ in 0..6 {
            let (cx, cy) = (rnd(1000) as f32 / 1000.0 * s, rnd(1000) as f32 / 1000.0 * s);
            let r = (0.12 + rnd(1000) as f32 / 1000.0 * 0.25) * s;
            let col = [rnd(256) as u8, rnd(256) as u8, rnd(256) as u8];
            let square = rnd(2) == 0;
            for y in 0..size {
                for x in 0..size {
                    let (dx, dy) = (x as f32 - cx, y as f32 - cy);
                    let inside = if square {
                        dx.abs() < r && dy.abs() < r
                    } else {
                        dx * dx + dy * dy < r * r
                    };
                    if inside {
                        img.put_pixel(x, y, Rgba([col[0], col[1], col[2], 255]));
                    }
                }
            }
        }
        img
    }

    /// Synthetic board: header bar, two team blocks of `team` rows with
    /// role column, separators and a portrait per row.
    pub(crate) fn board(ids: &[u32], team: usize, pitch: u32, colours: [[u8; 3]; 2]) -> RgbImage {
        let hh = (pitch as f32 * 0.39) as u32;
        let w = pitch * 14;
        let gap = pitch;
        let h = hh + 2 * team as u32 * pitch + gap + pitch;
        let mut img = RgbImage::from_pixel(w, h, Rgb([12, 14, 24]));
        let x0 = pitch / 2;
        for y in 0..hh {
            for x in x0..w - pitch / 2 {
                img.put_pixel(x, y, Rgb([205, 208, 212]));
            }
        }
        for (t, colour) in colours.iter().enumerate() {
            let top = hh + t as u32 * (team as u32 * pitch + gap);
            for k in 0..team as u32 {
                let ry0 = top + k * pitch;
                let rh = pitch - 2;
                for y in ry0..ry0 + rh {
                    for x in x0..w - pitch / 2 {
                        img.put_pixel(x, y, Rgb(*colour));
                    }
                }
                let (px0, py0, px1, _) =
                    portrait_box((x0 as usize, 0, 0, 0), (ry0 as usize, (ry0 + rh) as usize));
                let p = portrait(ids[t * team + k as usize], (px1 - px0) as u32, None);
                for (x, y, px) in p.enumerate_pixels() {
                    if px.0[3] > 0 {
                        img.put_pixel(
                            px0 as u32 + x,
                            py0 as u32 + y,
                            Rgb([px.0[0], px.0[1], px.0[2]]),
                        );
                    }
                }
            }
        }
        img
    }

    /// Templates as the cutting script writes them: portrait on PURPLE with
    /// alpha marking the foreground, 95 px.
    pub(crate) fn template_dir(ids: &[u32]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        write_templates(dir.path(), ids, 95, 0);
        dir
    }

    /// Write `hero-{id}.png` for each id into `dir` (created if missing):
    /// `size` px, drawn as synthetic portrait `id + art`, so two packs can
    /// hold different art under the same hero keys.
    pub(crate) fn write_templates(dir: &Path, ids: &[u32], size: u32, art: u32) {
        std::fs::create_dir_all(dir).unwrap();
        for &id in ids {
            let fg = portrait(id + art, size, None);
            let mut t = RgbaImage::new(size, size);
            for (x, y, p) in fg.enumerate_pixels() {
                let v = if p.0[3] > 0 {
                    Rgba([p.0[0], p.0[1], p.0[2], 255])
                } else {
                    Rgba([PURPLE[0], PURPLE[1], PURPLE[2], 0])
                };
                t.put_pixel(x, y, v);
            }
            t.save(dir.join(format!("hero-{id}.png"))).unwrap();
        }
    }

    /// A `special/skull_01.png` in `dir`.
    pub(crate) fn write_skull(dir: &Path) {
        std::fs::create_dir_all(dir.join("special")).unwrap();
        portrait(501, 74, Some(PURPLE))
            .save(dir.join("special/skull_01.png"))
            .unwrap();
    }
}

/// Write `count` synthetic hero icons into `dir` (74 px, the heroes-v2
/// cut). No game art. The portrait matcher and the hero reader both load
/// this directory.
pub fn write_synthetic_hero_icons(dir: &Path, count: usize) -> usize {
    let ids: Vec<u32> = (1..=count as u32).collect();
    test_fixtures::write_templates(dir, &ids, 74, 0);
    ids.len()
}

#[cfg(test)]
mod tests {
    use super::test_fixtures::*;
    use super::*;
    use image::Rgba;

    fn load(dir: &tempfile::TempDir) -> HeroTemplates {
        HeroTemplates::load_dir(dir.path()).expect("templates")
    }

    #[test]
    fn missing_or_empty_template_dir_is_none() {
        assert!(HeroTemplates::load_dir(Path::new("/nonexistent/scuffed/heroes")).is_none());
        let dir = tempfile::tempdir().unwrap();
        assert!(HeroTemplates::load_dir(dir.path()).is_none());
        std::fs::write(dir.path().join("broken.png"), b"not a png").unwrap();
        assert!(HeroTemplates::load_dir(dir.path()).is_none());
        assert_eq!(
            HeroTemplates::dir_in(Path::new("/data")),
            Path::new("/data/templates/heroes")
        );
    }

    #[test]
    fn rows_follow_the_board_not_fixed_pixels() {
        for (pitch, team) in [(76u32, 6usize), (97, 5), (57, 6)] {
            let img = board(&[1; 12], team, pitch, [PURPLE, YELLOW]);
            let hdr = find_header(View::rgb(&img)).expect("header");
            let rows = find_rows(View::rgb(&img), hdr, team);
            assert_eq!(rows.len(), 2 * team, "pitch {pitch}");
            for w in rows[..team].windows(2) {
                let d = w[1].0 as i64 - w[0].0 as i64;
                assert!((d - pitch as i64).abs() <= 1, "pitch {pitch}: step {d}");
            }
            assert_eq!(rows[0].0, hdr.3);
        }
    }

    #[test]
    fn reads_every_row_on_other_team_colours_and_sizes() {
        let ids: Vec<u32> = (1..=12).collect();
        let dir = template_dir(&ids);
        let tpl = load(&dir);
        assert_eq!(tpl.hero_count(), 12);
        for (pitch, colours) in [
            (76, [YELLOW, TEAL]),
            (57, [PURPLE, YELLOW]),
            (97, [TEAL, YELLOW]),
        ] {
            let img = board(&ids, 6, pitch, colours);
            let b = tpl
                .read_board(&DynamicImage::ImageRgb8(img), 6, Duration::from_secs(5))
                .expect("read");
            for r in &b.rows {
                assert_eq!(
                    r.hero.as_deref(),
                    Some(format!("hero-{}", ids[r.row]).as_str()),
                    "pitch {pitch}: {r:?}"
                );
                assert!(!r.suspect && r.score >= MIN_SCORE && r.lead >= MIN_LEAD);
            }
        }
    }

    #[test]
    fn grid_search_finds_the_exhaustive_best_on_shifted_icons() {
        let t = prep_rgba(&portrait(7, 95, None), false).unwrap();
        for scale in SCALES {
            let k = Kernel::new(&t, scale);
            for shift in 0..5usize {
                let img = portrait(7, 80, Some(TEAL));
                let mut moved =
                    RgbaImage::from_pixel(80, 80, Rgba([TEAL[0], TEAL[1], TEAL[2], 255]));
                image::imageops::overlay(&mut moved, &img, shift as i64, shift as i64 / 2);
                let q = prep_rgba(&moved, false).unwrap();
                let (g, f) = (k.best(&q), k.best_full(&q));
                assert!(
                    f - g < 0.02,
                    "scale {scale} shift {shift}: grid {g} full {f}"
                );
            }
        }
    }

    #[test]
    fn unknown_icon_is_flagged_and_names_no_hero() {
        let dir = template_dir(&[1, 2, 3, 4, 5, 6]);
        let tpl = load(&dir);
        let ids = [1, 2, 3, 4, 5, 6, 901, 902, 903, 904, 905, 906];
        let img = board(&ids, 6, 76, [PURPLE, YELLOW]);
        let b = tpl
            .read_board(&DynamicImage::ImageRgb8(img), 6, Duration::from_secs(5))
            .unwrap();
        for r in &b.rows[6..] {
            assert!(
                r.suspect && r.hero.is_none() && r.class == RowClass::Unknown,
                "{r:?}"
            );
        }
        assert!(b.rows[..6].iter().all(|r| r.class == RowClass::Hero));
    }

    #[test]
    fn special_classes_are_never_heroes() {
        let dir = template_dir(&[1, 2, 3]);
        std::fs::create_dir(dir.path().join("special")).unwrap();
        portrait(500, 74, Some(YELLOW))
            .save(dir.path().join("special/placeholder_01.png"))
            .unwrap();
        portrait(501, 74, Some(PURPLE))
            .save(dir.path().join("special/skull_01.png"))
            .unwrap();
        portrait(502, 74, Some(TEAL))
            .save(dir.path().join("special/ignored_name.png"))
            .unwrap();
        let tpl = load(&dir);
        assert_eq!((tpl.hero_count(), tpl.class_count()), (3, 5));
        let q = prep_rgba(&portrait(500, 74, Some(YELLOW)), false).unwrap();
        let r = tpl.read_patch(4, &q);
        assert_eq!(
            (r.class, r.hero.as_ref(), r.best.as_str()),
            (RowClass::Placeholder, None, "?placeholder")
        );
        assert!(!r.suspect);
    }

    #[test]
    fn a_top_level_placeholder_file_is_not_a_hero() {
        let dir = template_dir(&[1]);
        portrait(500, 74, Some(YELLOW))
            .save(dir.path().join("empty_01.png"))
            .unwrap();
        let tpl = load(&dir);
        assert_eq!(tpl.hero_count(), 1);
        let q = prep_rgba(&portrait(500, 74, Some(YELLOW)), false).unwrap();
        let r = tpl.read_patch(0, &q);
        assert_eq!(r.class, RowClass::Empty);
        assert!(r.hero.is_none());
        assert_ne!(r.best, "empty_01");
    }

    #[test]
    fn bad_input_is_an_error_not_a_panic() {
        let tpl = load(&template_dir(&[1]));
        let blank = DynamicImage::new_rgb8(300, 200);
        assert_eq!(
            tpl.read_board(&blank, 6, Duration::from_secs(1)),
            Err(HeroError::NoHeader)
        );
        assert_eq!(
            tpl.read_board(&blank, 7, Duration::from_secs(1)),
            Err(HeroError::Unsupported("team size"))
        );
        let img = board(&[1; 12], 6, 76, [PURPLE, YELLOW]);
        assert_eq!(
            tpl.read_board(&DynamicImage::ImageRgb8(img), 6, Duration::ZERO),
            Err(HeroError::OverBudget)
        );
    }

    fn read(row: usize, hero: Option<&str>) -> HeroRead {
        HeroRead {
            row,
            class: if hero.is_some() {
                RowClass::Hero
            } else {
                RowClass::Skull
            },
            hero: hero.map(str::to_string),
            best: hero.unwrap_or("?skull").into(),
            score: 0.9,
            lead: 0.3,
            second: "x".into(),
            suspect: false,
        }
    }

    #[test]
    fn voting_needs_two_reads_to_switch_and_resets_per_session() {
        let mut v = HeroVoter::default();
        let s = |o: Vec<Option<String>>| {
            o.into_iter()
                .map(|h| h.unwrap_or_default())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            s(v.update("a", &[read(0, Some("ana")), read(1, None)])),
            ["ana", ""]
        );
        assert_eq!(
            s(v.update("a", &[read(0, Some("mercy"))])),
            ["ana", ""],
            "one read is not a swap"
        );
        assert_eq!(
            s(v.update("a", &[read(0, None)])),
            ["ana", ""],
            "a skull keeps the row"
        );
        assert_eq!(
            s(v.update("a", &[read(0, Some("mercy"))])),
            ["mercy", ""],
            "two in a row swap"
        );
        assert_eq!(
            s(v.update("a", &[read(0, Some("ana")), read(0, Some("mercy"))])),
            ["mercy", ""]
        );
        assert_eq!(
            s(v.update("b", &[read(1, Some("dva"))])),
            ["", "dva"],
            "new session starts clean"
        );
    }

    #[test]
    fn record_serialises_hero_keys_and_scores_only() {
        let rec = HeroRecord {
            ts: "2026-10-09T05:00:00.000Z".into(),
            session: "s".into(),
            recognizer: HERO_RECOGNIZER_ID,
            hero_pack: HeroPack::V2,
            recropped: true,
            resolution: 1440,
            team_size: 6,
            owner_row: Some(0),
            elapsed_us: Some(1500),
            error: None,
            rows: vec![HeroRowLog {
                read: read(0, Some("ana")),
                voted: Some("ana".into()),
            }],
        };
        let v: serde_json::Value = serde_json::to_value(&rec).unwrap();
        assert_eq!(v["recognizer"], "hero-v1.2");
        assert_eq!(v["hero_pack"], "heroes-v2");
        assert_eq!(v["recropped"], true);
        assert_eq!(v["rows"][0]["hero"], "ana");
        assert_eq!(v["rows"][0]["class"], "hero");
        assert_eq!(v["rows"][0]["voted"], "ana");
        assert!(v.get("error").is_none());
    }

    // ------------------------------------------------ heroes-v2 and re-crop

    const IDS: [u32; 6] = [1, 2, 3, 4, 5, 6];
    /// Art offsets: the same hero key is drawn differently in each pack, so
    /// a read shows which pack made it.
    const ART_V2: u32 = 100;
    const ART_1080: u32 = 200;

    /// `<data>/templates/{heroes,heroes-v2,heroes-v2-1080}` with the given
    /// packs; each has a skull in `special/`.
    fn data_dir(v1: bool, v2: Option<&[u32]>, v2_1080: Option<&[u32]>) -> tempfile::TempDir {
        let data = tempfile::tempdir().unwrap();
        let mut packs = Vec::new();
        if v1 {
            packs.push((data.path().join(TEMPLATE_SUBDIR), &IDS[..], 95, 0));
        }
        if let Some(ids) = v2 {
            packs.push((data.path().join(TEMPLATE_SUBDIR_V2), ids, 74, ART_V2));
        }
        if let Some(ids) = v2_1080 {
            packs.push((data.path().join(TEMPLATE_SUBDIR_V2_1080), ids, 56, ART_1080));
        }
        for (dir, ids, size, art) in packs {
            write_templates(&dir, ids, size, art);
            write_skull(&dir);
        }
        data
    }

    /// Heroes read on a synthetic 6v6 board drawn with art `art`; `None`
    /// for a row that is not an accepted hero.
    fn reads(set: &HeroSet, art: u32, pitch: u32) -> (HeroPack, Vec<Option<String>>) {
        let ids: Vec<u32> = IDS.iter().chain(&IDS).map(|i| i + art).collect();
        let img = DynamicImage::ImageRgb8(board(&ids, 6, pitch, [YELLOW, TEAL]));
        let r = set.read_board(&img, 6, Duration::from_secs(5)).unwrap();
        (r.pack, r.board.rows.into_iter().map(|r| r.hero).collect())
    }

    fn all_right(heroes: &[Option<String>]) -> bool {
        heroes
            .iter()
            .enumerate()
            .all(|(i, h)| h.as_deref() == Some(format!("hero-{}", IDS[i % 6]).as_str()))
    }

    #[test]
    fn v2_is_preferred_when_complete() {
        let data = data_dir(true, Some(&IDS), None);
        let set = HeroSet::load_data_dir(data.path()).unwrap();
        assert_eq!(set.main_pack(), Some(HeroPack::V2));
        let (pack, heroes) = reads(&set, ART_V2, 76);
        assert_eq!(pack, HeroPack::V2);
        assert!(all_right(&heroes), "{heroes:?}");
        // v1 art is not in the set that is used
        let (_, heroes) = reads(&set, 0, 76);
        assert!(heroes.iter().all(Option::is_none), "{heroes:?}");
        // without v1 there is nothing to compare with: v2 alone is used
        let data = data_dir(false, Some(&IDS), None);
        let set = HeroSet::load_data_dir(data.path()).unwrap();
        assert_eq!(set.main_pack(), Some(HeroPack::V2));
        // and the reader looks where P1 installs the packs
        for (name, sub) in [
            ("heroes-v2.tar", TEMPLATE_SUBDIR_V2),
            ("heroes-v2-1080.tar", TEMPLATE_SUBDIR_V2_1080),
        ] {
            let spec = crate::packs::pack_spec(name).unwrap();
            assert_eq!(spec.dir_in(Path::new("/d")), Path::new("/d").join(sub));
        }
    }

    #[test]
    fn v1_is_the_fallback_when_v2_is_missing_or_partial() {
        let missing = data_dir(true, None, None);
        let partial = data_dir(true, Some(&IDS[..5]), None);
        // a v2 folder without the special classes v1 has is partial too
        let no_special = data_dir(true, None, None);
        write_templates(
            &no_special.path().join(TEMPLATE_SUBDIR_V2),
            &IDS,
            74,
            ART_V2,
        );
        for (what, data) in [
            ("missing", &missing),
            ("partial", &partial),
            ("no special", &no_special),
        ] {
            let set = HeroSet::load_data_dir(data.path()).unwrap();
            assert_eq!(set.main_pack(), Some(HeroPack::V1), "{what}");
            let (pack, heroes) = reads(&set, 0, 76);
            assert_eq!(pack, HeroPack::V1, "{what}");
            assert!(all_right(&heroes), "{what}: {heroes:?}");
        }
        assert!(HeroSet::load_data_dir(tempfile::tempdir().unwrap().path()).is_none());
    }

    #[test]
    fn the_1080_set_is_used_at_1080_scale() {
        let data = data_dir(true, Some(&IDS), Some(&IDS));
        let set = HeroSet::load_data_dir(data.path()).unwrap();
        assert_eq!(set.pack_1080(), Some(HeroPack::V2At1080));
        // crop heights: 1080p 756, 1440p 1007, 4K 1512; 881 and below use
        // the 1080 set
        assert_eq!(MAX_1080_CROP_H, 881);
        for (h, want) in [
            (756, HeroPack::V2At1080),
            (880, HeroPack::V2At1080),
            (881, HeroPack::V2At1080),
            (882, HeroPack::V2),
            (1007, HeroPack::V2),
            (1512, HeroPack::V2),
        ] {
            assert_eq!(
                set.for_board_height(h).map(|p| p.0),
                Some(want),
                "height {h}"
            );
        }
        // a 57 px pitch board is 820 px high (1080 scale), 76 px is 1093
        let (pack, heroes) = reads(&set, ART_1080, 57);
        assert_eq!(pack, HeroPack::V2At1080);
        assert!(all_right(&heroes), "{heroes:?}");
        let (pack, heroes) = reads(&set, ART_V2, 76);
        assert_eq!(pack, HeroPack::V2);
        assert!(all_right(&heroes), "{heroes:?}");
        // a partial 1080 set falls back to the 1440p set
        let data = data_dir(true, Some(&IDS), Some(&IDS[1..]));
        let set = HeroSet::load_data_dir(data.path()).unwrap();
        assert_eq!(set.pack_1080(), Some(HeroPack::V2));
        let (pack, heroes) = reads(&set, ART_V2, 57);
        assert_eq!(pack, HeroPack::V2);
        assert!(all_right(&heroes), "{heroes:?}");
    }

    /// A 2560x1440 frame with a synthetic 5v5 board whose header bar starts
    /// `left` px left of the scoreboard crop's left edge (negative: right of
    /// it), at the crop's top.
    fn frame_with_board(ids: &[u32], left: i64) -> DynamicImage {
        let pitch = 80;
        let b = board(ids, 5, pitch, [PURPLE, YELLOW]);
        let (cx, cy, _, _) = crate::ocr::preprocess::crop_scoreboard_rect(2560, 1440);
        let mut f = RgbImage::from_pixel(2560, 1440, image::Rgb([12, 14, 24]));
        let x = cx as i64 - left - (pitch / 2) as i64;
        image::imageops::overlay(&mut f, &b, x, cy as i64);
        DynamicImage::ImageRgb8(f)
    }

    fn five_v_five_right(rows: &[HeroRead]) -> usize {
        rows.iter()
            .filter(|r| r.hero.as_deref() == Some(format!("hero-{}", r.row % 5 + 1).as_str()))
            .count()
    }

    #[test]
    fn a_clipped_header_is_re_cropped_and_a_normal_one_is_not() {
        let data = data_dir(true, None, None);
        let set = HeroSet::load_data_dir(data.path()).unwrap();
        let ids = [1, 2, 3, 4, 5, 1, 2, 3, 4, 5];
        let read = |frame: &DynamicImage| {
            let sb = crate::ocr::preprocess::crop_scoreboard(frame);
            let fixed = set.read_board(&sb, 5, Duration::from_secs(5));
            let r = set
                .read_frame(frame, &sb, 5, Duration::from_secs(5))
                .unwrap();
            (fixed, r)
        };
        for left in [24, 60, 90] {
            let (fixed, r) = read(&frame_with_board(&ids, left));
            assert_eq!(r.pad_px, Some(CLIPPED_HEADER_PAD_1440), "{left}");
            assert_eq!(five_v_five_right(&r.board.rows), 10, "{left}: {r:?}");
            // the fixed crop alone misses the portraits
            let fixed_right = fixed.map_or(0, |f| five_v_five_right(&f.board.rows));
            assert!(fixed_right < 10, "{left}: fixed crop read {fixed_right}");
        }
        for left in [-40, -1] {
            let (fixed, r) = read(&frame_with_board(&ids, left));
            assert_eq!(r.pad_px, None, "{left}");
            assert_eq!(five_v_five_right(&r.board.rows), 10, "{left}");
            assert_eq!(
                fixed.unwrap().board.rows,
                r.board.rows,
                "{left}: same as the fixed crop"
            );
        }
    }

    /// A 5v5 crop whose header starts at its left edge, read with no
    /// re-crop, accepts no row: on real crops those reads named the wrong
    /// hero. Synthetic board, no game art.
    #[test]
    fn a_clipped_5v5_crop_read_alone_accepts_no_row() {
        use crate::report::blank::synth::{Teams, tab_crop};
        let data = data_dir(true, None, None);
        let set = HeroSet::load_data_dir(data.path()).unwrap();
        let crop = DynamicImage::ImageRgb8(tab_crop(Teams::Five, 5));
        // The clipped crop finds no rows today (NoRows). A read that does
        // find rows must accept none of them. Any other result fails.
        match set.read_board(&crop, 5, Duration::from_secs(5)) {
            Err(HeroError::NoRows) => {}
            Ok(r) => {
                assert!(r.board.rows.iter().all(|h| h.accepted().is_none()), "{r:?}");
                assert!(r.board.rows.iter().all(|h| h.suspect), "{r:?}");
            }
            Err(e) => panic!("unexpected read error on the clipped crop: {e:?}"),
        }
        // the clipped synthetic 5v5 frame read alone: every row flagged
        let ids = [1, 2, 3, 4, 5, 1, 2, 3, 4, 5];
        let frame = frame_with_board(&ids, 60);
        let sb = crate::ocr::preprocess::crop_scoreboard(&frame);
        let fixed = set.read_board(&sb, 5, Duration::from_secs(5)).unwrap();
        assert!(fixed.board.rows.iter().all(|h| h.accepted().is_none()));
        // a 6v6 header at x 0 is a measured layout and is not flagged
        assert!(!clipped_5v5(6, 0));
        assert!(!clipped_5v5(5, 1));
    }

    /// O8: the hero authority's own-row read goes through the new reader's
    /// pack choice (heroes-v2) and the clipped-header re-crop, and an icon
    /// outside the pack on the own row is an abstain, not a guess.
    #[test]
    fn the_own_row_read_uses_v2_and_the_re_crop_and_abstains_on_an_unknown_icon() {
        let data = data_dir(true, Some(&IDS), None);
        let reader =
            crate::shadow::Reader::load(&crate::shadow::ReaderConfig::from_data_dir(data.path()));
        let mut ids: Vec<u32> = [1, 2, 3, 4, 5, 1, 2, 3, 4, 5]
            .iter()
            .map(|i| i + ART_V2)
            .collect();
        let read = |ids: &[u32]| {
            let frame = frame_with_board(ids, 60);
            let sb = crate::ocr::preprocess::crop_scoreboard(&frame);
            reader.read_heroes(&frame, &sb, 5)
        };
        let t = read(&ids);
        let r = t.read.as_ref().expect("a read");
        assert_eq!(r.pack, HeroPack::V2);
        assert_eq!(r.pad_px, Some(CLIPPED_HEADER_PAD_1440), "clipped header");
        for row in 0..10 {
            let want = format!("hero-{}", row % 5 + 1);
            assert_eq!(t.own_hero(row), Some(want.as_str()), "row {row}");
        }
        // v1 art on row 2 is not in the set used, and no other class fits
        ids[2] = 3;
        let t = read(&ids);
        let row2 = &t.read.as_ref().unwrap().board.rows[2];
        assert!(row2.suspect, "{row2:?}");
        assert_eq!(t.own_hero(2), None);
        assert_eq!(t.own_hero(1), Some("hero-2"));
    }

    /// Paint synthetic portrait `id` (side `s` game-rect heights) `dx` right
    /// of the header bar's right end and `top` below the frame's top, where
    /// the career panel's portrait sits.
    fn paint_panel(frame: &mut DynamicImage, id: u32, (dx, top, s): (f64, f64, f64)) {
        let sb = crate::ocr::preprocess::crop_scoreboard(frame);
        let mut owned = None;
        let hdr = find_header(view_of(&sb, &mut owned)).unwrap();
        let (cx, _, _, _) = crate::ocr::preprocess::crop_scoreboard_rect(2560, 1440);
        let (x, y) = (cx as f64 + hdr.1 as f64 + dx * 1440.0, top * 1440.0);
        let p = portrait(id, (s * 1440.0) as u32, None);
        let f = frame.as_mut_rgb8().unwrap();
        for (px, py, c) in p.enumerate_pixels() {
            if c.0[3] > 0 {
                f.put_pixel(
                    x as u32 + px,
                    y as u32 + py,
                    image::Rgb([c.0[0], c.0[1], c.0[2]]),
                );
            }
        }
    }

    /// [`frame_with_board`] (header not clipped) with the board `down` px
    /// lower, as when a mode banner pushes the table down.
    fn frame_with_board_down(ids: &[u32], down: i64) -> DynamicImage {
        let pitch = 80;
        let b = board(ids, 5, pitch, [PURPLE, YELLOW]);
        let (cx, cy, _, _) = crate::ocr::preprocess::crop_scoreboard_rect(2560, 1440);
        let mut f = RgbImage::from_pixel(2560, 1440, image::Rgb([12, 14, 24]));
        image::imageops::overlay(&mut f, &b, cx as i64, cy as i64 + down);
        DynamicImage::ImageRgb8(f)
    }

    /// O8: the career panel's portrait is found right of the header bar, at
    /// the panel's fixed height, and read with the board's pack. Art outside
    /// that pack, or no portrait, is not a hero.
    #[test]
    fn the_panel_portrait_is_found_by_the_header_and_read_with_v2() {
        let data = data_dir(true, Some(&IDS), None);
        let set = HeroSet::load_data_dir(data.path()).unwrap();
        let ids: Vec<u32> = [1, 2, 3, 4, 5, 1, 2, 3, 4, 5]
            .iter()
            .map(|i| i + ART_V2)
            .collect();
        let read = |down: i64, panel: Option<(u32, (f64, f64, f64))>| {
            let mut frame = frame_with_board_down(&ids, down);
            if let Some((id, at)) = panel {
                paint_panel(&mut frame, id, at);
            }
            let sb = crate::ocr::preprocess::crop_scoreboard(&frame);
            set.read_panel(&frame, &sb, Duration::from_secs(5))
        };
        // inside the searched window, on and off the grid
        for at in [
            (0.03, 0.142, 0.16),
            (0.012, 0.12, 0.175),
            (0.021, 0.155, 0.15),
        ] {
            let r = read(0, Some((4 + ART_V2, at))).unwrap();
            assert_eq!(r.pack, HeroPack::V2);
            assert_eq!(r.accepted(), Some("hero-4"), "{at:?}: {r:?}");
        }
        // a table pushed down by a banner leaves the panel where it was
        let r = read(150, Some((2 + ART_V2, (0.03, 0.142, 0.16)))).unwrap();
        assert_eq!(r.accepted(), Some("hero-2"), "{r:?}");
        // v1 art is not in the set used, and no portrait names no hero
        let v1 = read(0, Some((4, (0.03, 0.142, 0.16)))).unwrap();
        assert_eq!(v1.accepted(), None, "{v1:?}");
        let none = read(0, None).unwrap();
        assert_eq!(none.accepted(), None, "{none:?}");
        // a frame that is not this crop's frame is refused
        let frame = frame_with_board(&ids, -40);
        let small = DynamicImage::new_rgb8(100, 100);
        assert!(
            set.read_panel(&frame, &small, Duration::from_secs(5))
                .is_err()
        );
    }

    /// The panel gate is stricter than the board's: the panel read is the
    /// best of many boxes.
    #[test]
    fn the_panel_gate_needs_a_high_score_and_lead() {
        let panel = |class: RowClass, hero: Option<&str>, score: f32, lead: f32| PanelRead {
            read: HeroRead {
                row: 0,
                class,
                hero: hero.map(str::to_string),
                best: hero.unwrap_or("?").to_string(),
                score,
                lead,
                second: "x".into(),
                suspect: !passes_gate(score, lead),
            },
            pack: HeroPack::V2,
            box_px: (0, 0, 1, 1),
            rel: (0.0, 0.0, 0.16),
        };
        assert_eq!(
            panel(RowClass::Hero, Some("ana"), 0.96, 0.4).accepted(),
            Some("ana")
        );
        assert_eq!(
            panel(RowClass::Hero, Some("ana"), PANEL_MIN_SCORE, PANEL_MIN_LEAD).accepted(),
            Some("ana")
        );
        // passes the board's gate, not the panel's
        assert_eq!(
            panel(RowClass::Hero, Some("ana"), 0.80, 0.4).accepted(),
            None
        );
        assert_eq!(
            panel(RowClass::Hero, Some("ana"), 0.96, 0.15).accepted(),
            None
        );
        // a special class or a suspect read is never a hero
        assert_eq!(panel(RowClass::Skull, None, 0.96, 0.4).accepted(), None);
        assert_eq!(
            panel(RowClass::Hero, Some("skull_06"), 0.96, 0.4).accepted(),
            None
        );
        let mut sus = panel(RowClass::Hero, Some("ana"), 0.96, 0.4);
        sus.read.suspect = true;
        assert_eq!(sus.accepted(), None);
    }

    #[test]
    fn re_crop_pad_scales_with_the_frame_and_needs_a_clipped_full_frame() {
        let sb = |w: u32, h: u32| {
            let (_, _, cw, ch) = crate::ocr::preprocess::crop_scoreboard_rect(w, h);
            DynamicImage::new_rgb8(cw, ch)
        };
        for (w, h, pad) in [(2560, 1440, 120), (1920, 1080, 90), (3840, 2160, 180)] {
            assert_eq!(clipped_header_pad(w, h, &sb(w, h), 0), Some(pad), "{w}x{h}");
            assert_eq!(clipped_header_pad(w, h, &sb(w, h), 1), None, "{w}x{h}");
        }
        // 16:10 is letterboxed: the 16:9 height is 1440 of 1600, so the pad
        // is 120 (the frame height would give 133)
        assert_eq!(
            clipped_header_pad(2560, 1600, &sb(2560, 1600), 0),
            Some(120)
        );
        // ultrawide: scaled by the 16:9 height
        assert_eq!(
            clipped_header_pad(3440, 1440, &sb(3440, 1440), 0),
            Some(120)
        );
        // a board crop passed as the frame is not a full frame
        assert_eq!(clipped_header_pad(1664, 1007, &sb(2560, 1440), 0), None);
        let wide = hero_crop(&DynamicImage::new_rgb8(2560, 1440), 120);
        assert_eq!(
            (wide.width(), wide.height()),
            (1664 + 120, 1007),
            "same right edge, top and height"
        );
    }

    #[test]
    fn a_failed_wide_crop_falls_back_to_the_fixed_crop() {
        let data = data_dir(true, None, None);
        let set = HeroSet::load_data_dir(data.path()).unwrap();
        let ids = [1, 2, 3, 4, 5, 1, 2, 3, 4, 5];
        // the header starts right at the crop edge, so the board is read as
        // clipped; a light strip left of it on the header rows, with no role
        // column under it, leaves the wide crop without rows
        let mut f = frame_with_board(&ids, 0).to_rgb8();
        let (cx, cy, _, _) = crate::ocr::preprocess::crop_scoreboard_rect(2560, 1440);
        for x in cx - CLIPPED_HEADER_PAD_1440..cx {
            for y in cy..cy + 31 {
                f.put_pixel(x, y, image::Rgb([205, 208, 212]));
            }
        }
        let frame = DynamicImage::ImageRgb8(f);
        let sb = crate::ocr::preprocess::crop_scoreboard(&frame);
        let wide = hero_crop(&frame, CLIPPED_HEADER_PAD_1440);
        assert_eq!(
            set.read_board(&wide, 5, Duration::from_secs(5)),
            Err(HeroError::NoRows),
            "the wide crop alone fails"
        );
        let r = set
            .read_frame(&frame, &sb, 5, Duration::from_secs(5))
            .expect("fixed crop read");
        assert_eq!(r.pad_px, None);
        // the fixed crop still shows a header at its left edge: a 5v5 read
        // that cannot be placed is flagged, not guessed (hero-v1.2)
        assert!(
            r.board
                .rows
                .iter()
                .all(|h| h.suspect && h.accepted().is_none()),
            "{r:?}"
        );
    }

    #[test]
    fn a_lone_1080_set_never_reads_1440p_boards() {
        let data = data_dir(false, None, Some(&IDS));
        let set = HeroSet::load_data_dir(data.path()).unwrap();
        assert_eq!(set.main_pack(), None);
        assert_eq!(set.pack_1080(), Some(HeroPack::V2At1080));
        for h in [882, 1007, 1512] {
            assert!(set.for_board_height(h).is_none(), "height {h}");
        }
        let img = DynamicImage::ImageRgb8(board(&[1; 12], 6, 76, [YELLOW, TEAL]));
        assert_eq!(
            set.read_board(&img, 6, Duration::from_secs(5)),
            Err(HeroError::Unsupported("no hero pack for this board size"))
        );
        let (pack, heroes) = reads(&set, ART_1080, 57);
        assert_eq!(pack, HeroPack::V2At1080);
        assert!(all_right(&heroes), "{heroes:?}");
    }

    /// Install `templates/heroes-v2` the way pack sync does: build it in a
    /// staging folder, then rename it into place.
    fn swap_in_v2(data: &Path) {
        let staging = data.join("templates/.heroes-v2-staging");
        write_templates(&staging, &IDS, 74, ART_V2);
        write_skull(&staging);
        std::fs::rename(&staging, data.join(TEMPLATE_SUBDIR_V2)).unwrap();
    }

    #[test]
    fn the_live_set_picks_up_v2_after_it_is_swapped_in() {
        let data = data_dir(true, None, None);
        let live = LiveHeroSet::load(HeroDirs::in_data_dir(data.path()));
        let pack = |live: &LiveHeroSet| live.current().and_then(|s| s.main_pack());
        assert_eq!(pack(&live), Some(HeroPack::V1));
        live.wait_reload();
        assert_eq!(pack(&live), Some(HeroPack::V1), "no change, no reload");
        swap_in_v2(data.path());
        // this board still reads with the set it has; the load runs aside
        assert_eq!(pack(&live), Some(HeroPack::V1));
        live.wait_reload();
        assert_eq!(pack(&live), Some(HeroPack::V2));
        let (got, heroes) = reads(&live.current().unwrap(), ART_V2, 76);
        assert_eq!(got, HeroPack::V2);
        assert!(all_right(&heroes), "{heroes:?}");
    }

    /// F249-6: one reload thread at a time. Boards that arrive while a load
    /// is in flight start no second one.
    #[test]
    fn only_one_reload_runs_at_a_time() {
        let data = data_dir(true, None, None);
        let live = LiveHeroSet::load(HeroDirs::in_data_dir(data.path()));
        live.set_load_delay_ms(400);
        swap_in_v2(data.path());
        for _ in 0..50 {
            let set = live.current().unwrap();
            assert_eq!(set.main_pack(), Some(HeroPack::V1), "load still in flight");
        }
        assert_eq!(live.reload_spawns(), 1);
        live.wait_reload();
        assert_eq!(live.current().unwrap().main_pack(), Some(HeroPack::V2));
        assert_eq!(live.reload_spawns(), 1, "stamp matches again: no reload");
    }

    /// Replace `templates/heroes-v2` through pack sync's own swap
    /// (`renameat2` exchange, or its rename fallback).
    fn swap_v2_with_art(data: &Path, art: u32) {
        let staging = data.join(format!("templates/.heroes-v2-staging-{art}"));
        write_templates(&staging, &IDS, 74, art);
        write_skull(&staging);
        crate::packs::swap_dir(&staging, &data.join(TEMPLATE_SUBDIR_V2)).unwrap();
    }

    /// F249-7: a newer heroes-v2 over an installed one (the folder exists
    /// before and after; only its identity changes) is picked up.
    #[test]
    fn an_updated_v2_pack_is_picked_up() {
        let data = data_dir(true, Some(&IDS), None);
        let live = LiveHeroSet::load(HeroDirs::in_data_dir(data.path()));
        let old = live.current().unwrap();
        assert!(all_right(&reads(&old, ART_V2, 76).1));
        swap_v2_with_art(data.path(), 300);
        live.current();
        live.wait_reload();
        let new = live.current().unwrap();
        assert!(!std::sync::Arc::ptr_eq(&old, &new), "re-loaded");
        let (pack, heroes) = reads(&new, 300, 76);
        assert_eq!(pack, HeroPack::V2);
        assert!(all_right(&heroes), "new art: {heroes:?}");
    }

    /// F249-8: the reload thread itself throws away a load that saw a swap,
    /// and a later board loads again.
    #[test]
    fn the_reload_thread_discards_a_load_that_saw_a_swap() {
        let data = data_dir(true, None, None);
        let live = LiveHeroSet::load(HeroDirs::in_data_dir(data.path()));
        swap_in_v2(data.path());
        let path = data.path().to_path_buf();
        live.during_next_load(move || swap_v2_with_art(&path, 300));
        assert_eq!(live.current().unwrap().main_pack(), Some(HeroPack::V1));
        live.wait_reload();
        // nothing was installed; this board keeps v1 and starts a retry
        assert_eq!(live.current().unwrap().main_pack(), Some(HeroPack::V1));
        live.wait_reload();
        let set = live.current().unwrap();
        assert_eq!(set.main_pack(), Some(HeroPack::V2));
        assert!(all_right(&reads(&set, 300, 76).1), "the art after the swap");
        assert_eq!(live.reload_spawns(), 2);
    }

    #[test]
    fn a_load_that_sees_a_swap_is_thrown_away() {
        let data = data_dir(true, None, None);
        let dirs = HeroDirs::in_data_dir(data.path());
        assert!(
            dirs.load_stable(|| swap_in_v2(data.path())).is_none(),
            "a folder changed while loading"
        );
        let (set, _) = dirs.load_stable(|| {}).expect("stable");
        assert_eq!(set.unwrap().main_pack(), Some(HeroPack::V2));
    }

    #[test]
    fn the_gate_rejects_half_portrait_scores() {
        // clipped 5v5 boards: half-portrait patches scored 0.60 to 0.64 with
        // a 0.14 to 0.17 lead
        for (score, lead) in [(0.60, 0.14), (0.62, 0.17), (0.64, 0.17), (0.6499, 0.3)] {
            assert!(!passes_gate(score, lead), "{score} {lead}");
        }
        assert!(passes_gate(0.65, 0.10));
        assert!(passes_gate(0.95, 0.35));
        assert!(!passes_gate(0.95, 0.09), "lead gate unchanged");
    }

    /// Local bench on labelled boards (box only, never in CI): set
    /// `SCUFFED_HERO_BENCH` to a TSV of `path<TAB>team_size<TAB>12 comma-separated labels<TAB>tag`
    /// and `SCUFFED_HERO_TEMPLATES` to a template dir. Labels: hero key,
    /// `?placeholder`, `?skull`, `?empty`, or `-` to skip a row.
    #[test]
    #[ignore = "needs local labelled boards"]
    fn bench_labelled_boards() {
        let (Ok(tsv), Ok(tdir)) = (
            std::env::var("SCUFFED_HERO_BENCH"),
            std::env::var("SCUFFED_HERO_TEMPLATES"),
        ) else {
            return;
        };
        let t_load = Instant::now();
        let tpl = HeroTemplates::load_dir(Path::new(&tdir)).expect("templates");
        eprintln!(
            "loaded {} classes in {} ms",
            tpl.class_count(),
            t_load.elapsed().as_millis()
        );
        let mut stats: HashMap<String, [usize; 6]> = HashMap::new();
        let mut times = Vec::new();
        let mut out = String::new();
        for line in std::fs::read_to_string(tsv).unwrap().lines() {
            let f: Vec<&str> = line.split('\t').collect();
            let img = image::open(f[0]).unwrap();
            let team: usize = f[1].parse().unwrap();
            let labels: Vec<&str> = f[2].split(',').collect();
            let tag = f[3].to_string();
            let b = match tpl.read_board(&img, team, Duration::from_secs(5)) {
                Ok(b) => b,
                Err(e) => {
                    eprintln!("ERR {} {e}", f[0]);
                    continue;
                }
            };
            times.push(b.elapsed_us);
            for r in &b.rows {
                let want = labels[r.row];
                if want == "-" {
                    continue;
                }
                let st = stats.entry(tag.clone()).or_default();
                let accepted = !r.suspect;
                if want.starts_with('?') {
                    st[3] += 1;
                    if accepted && r.class == RowClass::Hero {
                        st[4] += 1;
                    } else if accepted && r.best == want {
                        st[5] += 1;
                    }
                } else if !accepted {
                    st[1] += 1;
                } else if r.hero.as_deref() == Some(want) {
                    st[0] += 1;
                } else {
                    st[2] += 1;
                }
                out.push_str(&format!(
                    "{}\t{}\t{}\t{}\t{:.4}\t{:.4}\t{}\t{}\n",
                    f[0], r.row, want, r.best, r.score, r.lead, r.second, r.suspect
                ));
            }
        }
        if let Ok(p) = std::env::var("SCUFFED_HERO_BENCH_OUT") {
            std::fs::write(p, out).unwrap();
        }
        let mut tags: Vec<_> = stats.keys().cloned().collect();
        tags.sort();
        for t in tags {
            let s = stats[&t];
            eprintln!(
                "{t}: heroes right {} flagged {} wrong {} | \
                 special {} named-as-hero {} right-class {}",
                s[0], s[1], s[2], s[3], s[4], s[5]
            );
        }
        times.sort_unstable();
        let pct = |q: f64| times[((times.len() - 1) as f64 * q) as usize];
        eprintln!(
            "boards {} per-board us: median {} p90 {} max {}",
            times.len(),
            pct(0.5),
            pct(0.9),
            pct(1.0)
        );
    }
}
