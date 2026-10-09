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

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use image::{DynamicImage, RgbImage, RgbaImage};
use serde::Serialize;

/// Identifies this recognizer's output in the log. Bump on any change that
/// can move a read, score or suspect flag.
pub const HERO_RECOGNIZER_ID: &str = "hero-v1";
/// Minimum correlation for an accepted read.
pub const MIN_SCORE: f32 = 0.55;
/// Minimum lead over the best other class for an accepted read.
pub const MIN_LEAD: f32 = 0.10;
/// Template directory under the data dir.
pub const TEMPLATE_SUBDIR: &str = "templates/heroes";
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
                let (qc, wc, dc) = (
                    qrow.chunks_exact(LANES),
                    wrow.chunks_exact(LANES),
                    drow.chunks_exact(LANES),
                );
                let tail = (qc.remainder(), wc.remainder(), dc.remainder());
                for ((qv, wv), dv) in qc.zip(wc).zip(dc) {
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
                add_template(&mut by, stem.to_string(), &p);
            }
        }
        let heroes = by.len();
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
        let ok = s >= MIN_SCORE && lead >= MIN_LEAD;
        let class = if !ok {
            RowClass::Unknown
        } else {
            match best.as_str() {
                "?placeholder" => RowClass::Placeholder,
                "?skull" => RowClass::Skull,
                "?empty" => RowClass::Empty,
                _ => RowClass::Hero,
            }
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
        let owned;
        let view = match board {
            DynamicImage::ImageRgb8(r) => View::rgb(r),
            DynamicImage::ImageRgba8(r) => View::rgba(r),
            other => {
                owned = other.to_rgb8();
                View::rgb(&owned)
            }
        };
        let hdr = find_header(view).ok_or(HeroError::NoHeader)?;
        let bands = find_rows(view, hdr, team_size);
        if bands.len() != 2 * team_size {
            return Err(HeroError::NoRows);
        }
        let mut rows = Vec::with_capacity(bands.len());
        for (i, band) in bands.into_iter().enumerate() {
            if t0.elapsed() > budget {
                return Err(HeroError::OverBudget);
            }
            let q = prep_view(view, portrait_box(hdr, band)).ok_or(HeroError::NoRows)?;
            rows.push(self.read_patch(i, &q));
        }
        Ok(HeroBoard {
            rows,
            elapsed_us: t0.elapsed().as_micros() as u64,
        })
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
    pub resolution: u32,
    pub team_size: usize,
    pub owner_row: Option<usize>,
    pub elapsed_us: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub rows: Vec<HeroRowLog>,
}

#[cfg(test)]
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
        for &id in ids {
            let fg = portrait(id, 95, None);
            let mut t = RgbaImage::new(95, 95);
            for (x, y, p) in fg.enumerate_pixels() {
                let v = if p.0[3] > 0 {
                    Rgba([p.0[0], p.0[1], p.0[2], 255])
                } else {
                    Rgba([PURPLE[0], PURPLE[1], PURPLE[2], 0])
                };
                t.put_pixel(x, y, v);
            }
            t.save(dir.path().join(format!("hero-{id}.png"))).unwrap();
        }
        dir
    }
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
        assert_eq!(v["recognizer"], "hero-v1");
        assert_eq!(v["rows"][0]["hero"], "ana");
        assert_eq!(v["rows"][0]["class"], "hero");
        assert_eq!(v["rows"][0]["voted"], "ana");
        assert!(v.get("error").is_none());
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
                "{t}: heroes right {} flagged {} wrong {} | special {} named-as-hero {} right-class {}",
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
