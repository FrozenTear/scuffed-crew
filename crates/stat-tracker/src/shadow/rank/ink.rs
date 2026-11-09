//! Pixel steps for the rank-v1 reader: colour "ink" planes, column and row
//! runs, tight crops, glyph cuts and the fixed-size vector a glyph is
//! compared as. Pixels only: no positions, no templates.
//!
//! These work on float ink planes (0 = background, 1 = text) rather than on
//! the 8-bit grey and gradient planes of the bans and picks reader, because
//! the rank screen's text is told apart by colour (cyan, yellow, white on a
//! green pill, red or green deltas), not by edges.

use image::RgbImage;

/// Float image, row-major.
#[derive(Debug, Clone, PartialEq)]
pub struct Plane {
    pub w: usize,
    pub h: usize,
    pub px: Vec<f32>,
}

impl Plane {
    pub fn new(w: usize, h: usize) -> Self {
        Self {
            w,
            h,
            px: vec![0.0; w * h],
        }
    }

    pub fn get(&self, x: usize, y: usize) -> f32 {
        self.px[y * self.w + x]
    }

    pub fn set(&mut self, x: usize, y: usize, v: f32) {
        self.px[y * self.w + x] = v;
    }

    /// Columns `x0..x1`, all rows.
    pub fn cols(&self, x0: usize, x1: usize) -> Plane {
        self.sub(x0, 0, x1, self.h)
    }

    pub fn sub(&self, x0: usize, y0: usize, x1: usize, y1: usize) -> Plane {
        let mut p = Plane::new(x1 - x0, y1 - y0);
        for y in y0..y1 {
            for x in x0..x1 {
                p.set(x - x0, y - y0, self.get(x, y));
            }
        }
        p
    }

    pub fn col_sums(&self) -> Vec<f32> {
        let mut s = vec![0.0; self.w];
        for y in 0..self.h {
            for (x, v) in s.iter_mut().enumerate() {
                *v += self.get(x, y);
            }
        }
        s
    }

    pub fn row_sums(&self) -> Vec<f32> {
        (0..self.h)
            .map(|y| self.px[y * self.w..(y + 1) * self.w].iter().sum())
            .collect()
    }

    /// Per row, how many pixels are above `t`.
    pub fn row_counts(&self, t: f32) -> Vec<f32> {
        (0..self.h)
            .map(|y| {
                self.px[y * self.w..(y + 1) * self.w]
                    .iter()
                    .filter(|&&v| v > t)
                    .count() as f32
            })
            .collect()
    }

    pub fn sum(&self) -> f32 {
        self.px.iter().sum()
    }

    pub fn map(&self, f: impl Fn(f32) -> f32) -> Plane {
        Plane {
            w: self.w,
            h: self.h,
            px: self.px.iter().map(|&v| f(v)).collect(),
        }
    }

    /// The `q` quantile (0..1) of the values, nearest rank.
    pub fn quantile(&self, q: f32) -> f32 {
        if self.px.is_empty() {
            return 0.0;
        }
        let mut v = self.px.clone();
        let k = ((v.len() - 1) as f32 * q).round() as usize;
        let (_, m, _) = v.select_nth_unstable_by(k, |a, b| a.total_cmp(b));
        *m
    }
}

/// A pixel is ink when its plane value is above this.
pub const ON: f32 = 0.5;

/// A cut of text: its frame box `[x0, y0, x1, y1)` and its ink.
#[derive(Debug, Clone, PartialEq)]
pub struct Glyph {
    pub bbox: [u32; 4],
    pub ink: Plane,
}

fn clip(v: f32) -> f32 {
    v.clamp(0.0, 1.0)
}

/// A score per pixel of the frame rectangle `x0..x1`, `y0..y1` (clipped to
/// the frame).
pub fn plane(
    img: &RgbImage,
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
    f: impl Fn([f32; 3]) -> f32,
) -> Plane {
    let x1 = x1.min(img.width());
    let y1 = y1.min(img.height());
    let (w, h) = (
        x1.saturating_sub(x0) as usize,
        y1.saturating_sub(y0) as usize,
    );
    let mut p = Plane::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let [r, g, b] = img.get_pixel(x0 + x as u32, y0 + y as u32).0;
            p.set(x, y, f([r as f32, g as f32, b as f32]));
        }
    }
    p
}

/// Grey-white text: bright and colourless. Negative on saturated colours.
pub fn neutral([r, g, b]: [f32; 3]) -> f32 {
    r.min(g).min(b) - 0.6 * r.max(g).max(b)
}
pub fn white(c: [f32; 3]) -> f32 {
    clip((neutral(c) - 20.0) / 50.0)
}
pub fn cyan([r, g, b]: [f32; 3]) -> f32 {
    clip((g.min(b) - r - 50.0) / 100.0)
}
pub fn orange([r, g, b]: [f32; 3]) -> f32 {
    if g > 60.0 {
        clip((r - b - 90.0) / 90.0)
    } else {
        0.0
    }
}
pub fn yellow([r, g, b]: [f32; 3]) -> f32 {
    clip((r.min(g) - b - 80.0) / 100.0)
}
/// How much greener than red or blue.
pub fn green_score([r, g, b]: [f32; 3]) -> f32 {
    g - r.max(b)
}
/// How much redder than green or blue.
pub fn red_score([r, g, b]: [f32; 3]) -> f32 {
    r - g.max(b)
}
pub fn green(c: [f32; 3]) -> f32 {
    clip((green_score(c) - 40.0) / 80.0)
}
pub fn red(c: [f32; 3]) -> f32 {
    clip((red_score(c) - 40.0) / 80.0)
}

/// Ink scaled to the plane's own peak (its `q` quantile, at least `floor`),
/// so text dimmed by an overlay reads like bright text.
pub fn adaptive(score: &Plane, q: f32, floor: f32) -> Plane {
    let p = score.quantile(q).max(floor);
    score.map(|v| clip((v - 0.25 * p) / (0.5 * p)))
}

/// Ink scaled column by column to the local peak: the brightest value in
/// the columns within `r` of it (at least `floor`). A line that an overlay
/// dims over part of its length (a chat panel edge crossing a hero name)
/// then reads as one line, where [`adaptive`] would keep only the bright
/// part.
pub fn adaptive_local(score: &Plane, r: usize, floor: f32) -> Plane {
    let colmax: Vec<f32> = (0..score.w)
        .map(|x| {
            (0..score.h)
                .map(|y| score.get(x, y))
                .fold(f32::MIN, f32::max)
        })
        .collect();
    let mut out = Plane::new(score.w, score.h);
    for x in 0..score.w {
        let lo = x.saturating_sub(r);
        let hi = (x + r + 1).min(score.w);
        let p = colmax[lo..hi].iter().copied().fold(floor, f32::max);
        for y in 0..score.h {
            out.set(x, y, clip((score.get(x, y) - 0.25 * p) / (0.5 * p)));
        }
    }
    out
}

/// `[a, b)` spans where `prof > thr`; spans closer than `gap` merge.
pub fn runs(prof: &[f32], thr: f32, gap: usize) -> Vec<(usize, usize)> {
    let mut out: Vec<(usize, usize)> = Vec::new();
    let mut start = None;
    for (x, &v) in prof.iter().enumerate() {
        match (v > thr, start) {
            (true, None) => start = Some(x),
            (false, Some(s)) => {
                push_run(&mut out, (s, x), gap);
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        push_run(&mut out, (s, prof.len()), gap);
    }
    out
}

fn push_run(out: &mut Vec<(usize, usize)>, r: (usize, usize), gap: usize) {
    match out.last_mut() {
        Some(last) if r.0 - last.1 < gap => last.1 = r.1,
        _ => out.push(r),
    }
}

/// The first spans from the left, joined while the gap is under `maxgap`.
pub fn first_cluster(rs: &[(usize, usize)], maxgap: usize) -> Option<(usize, usize)> {
    let (&(a, mut b), rest) = rs.split_first()?;
    for &(p, q) in rest {
        if p - b >= maxgap {
            break;
        }
        b = q;
    }
    Some((a, b))
}

/// Crop `sub` (whose top-left is frame `(ox, oy)`) to its pixels above
/// [`ON`].
pub fn tight(sub: &Plane, ox: u32, oy: u32) -> Option<([u32; 4], Plane)> {
    let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
    for y in 0..sub.h {
        for x in 0..sub.w {
            if sub.get(x, y) > ON {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
            }
        }
    }
    if x1 == 0 {
        return None;
    }
    Some((
        [
            ox + x0 as u32,
            oy + y0 as u32,
            ox + x1 as u32,
            oy + y1 as u32,
        ],
        sub.sub(x0, y0, x1, y1),
    ))
}

pub fn tight_glyph(sub: &Plane, ox: u32, oy: u32) -> Option<Glyph> {
    tight(sub, ox, oy).map(|(bbox, ink)| Glyph { bbox, ink })
}

/// Cut `ink` into glyphs at columns whose ink sum is at most `thr`, each
/// cropped tight; glyphs under `minh` rows are dropped.
pub fn glyphs(ink: &Plane, ox: u32, oy: u32, thr: f32, minh: u32) -> Vec<Glyph> {
    runs(&ink.col_sums(), thr, 1)
        .into_iter()
        .filter_map(|(a, b)| tight_glyph(&ink.cols(a, b), ox + a as u32, oy))
        .filter(|g| g.bbox[3] - g.bbox[1] >= minh)
        .collect()
}

/// The glyphs at the right end, walking left while the gap is at most
/// `maxgap`.
pub fn cluster_right(mut gl: Vec<Glyph>, maxgap: u32) -> Vec<Glyph> {
    let Some(mut k) = gl.len().checked_sub(1) else {
        return gl;
    };
    while k > 0 && gl[k].bbox[0].saturating_sub(gl[k - 1].bbox[2]) <= maxgap {
        k -= 1;
    }
    gl.split_off(k)
}

/// Split glyphs wider than `1.15 * h` at their thinnest middle column (two
/// touching digits).
pub fn split_wide(gl: Vec<Glyph>, h: f32) -> Vec<Glyph> {
    let mut out = Vec::new();
    for g in gl {
        let w = (g.bbox[2] - g.bbox[0]) as f32;
        match (w > 1.15 * h).then(|| split_two(&g)).flatten() {
            Some((a, b)) => out.extend([a, b]),
            None => out.push(g),
        }
    }
    out
}

/// Cut a glyph in two at its thinnest column in the middle 40%: two digits
/// that touch (at 1080p a soft edge often joins them). `None` when the
/// glyph is too narrow to hold two.
pub fn split_two(g: &Glyph) -> Option<(Glyph, Glyph)> {
    if g.ink.w <= 8 {
        return None;
    }
    let prof = g.ink.col_sums();
    let m = prof.len() / 2;
    let lo = (m as f32 * 0.6) as usize;
    let hi = prof.len() - lo;
    let k = (lo..hi)
        .min_by(|&a, &b| prof[a].total_cmp(&prof[b]))
        .unwrap_or(m);
    Some((
        tight_glyph(&g.ink.cols(0, k), g.bbox[0], g.bbox[1])?,
        tight_glyph(&g.ink.cols(k, g.ink.w), g.bbox[0] + k as u32, g.bbox[1])?,
    ))
}

/// Rows a column's ink (above 0.4) spans, first to last.
fn extents(ink: &Plane) -> Vec<usize> {
    (0..ink.w)
        .map(|x| {
            let ys: Vec<usize> = (0..ink.h).filter(|&y| ink.get(x, y) > 0.4).collect();
            match (ys.first(), ys.last()) {
                (Some(a), Some(b)) => b - a + 1,
                _ => 0,
            }
        })
        .collect()
}

/// Zero columns whose ink spans fewer than `minext` rows: a minus sign or a
/// small caret beside the digits.
pub fn drop_short_columns(ink: &Plane, minext: usize) -> Plane {
    let ext = extents(ink);
    let mut out = ink.clone();
    for (x, &e) in ext.iter().enumerate() {
        if e < minext {
            for y in 0..out.h {
                out.set(x, y, 0.0);
            }
        }
    }
    out
}

/// Drop a leading minus bar (at least 4 columns at most 3 rows tall) that
/// touches the next glyph.
pub fn strip_minus(g: Glyph) -> Glyph {
    let ext = extents(&g.ink);
    let k = ext.iter().take_while(|&&e| e <= 3).count();
    if k < 4 || k >= g.ink.w {
        return g;
    }
    tight_glyph(&g.ink.cols(k, g.ink.w), g.bbox[0] + k as u32, g.bbox[1]).unwrap_or(g)
}

/// Area-average resample to `w` by `h` (each output pixel is the mean of the
/// source area it covers, with fractional edges).
pub fn resize(src: &Plane, w: usize, h: usize) -> Plane {
    let mut out = Plane::new(w, h);
    if src.w == 0 || src.h == 0 || w == 0 || h == 0 {
        return out;
    }
    let sx = src.w as f32 / w as f32;
    let sy = src.h as f32 / h as f32;
    let spans = |n: usize, s: f32, lim: usize| -> Vec<Vec<(usize, f32)>> {
        (0..n)
            .map(|i| {
                let (a, b) = (i as f32 * s, (i + 1) as f32 * s);
                let mut v = Vec::new();
                let mut k = a.floor() as usize;
                while (k as f32) < b && k < lim {
                    let wgt = (b.min(k as f32 + 1.0) - a.max(k as f32)).max(0.0);
                    if wgt > 0.0 {
                        v.push((k, wgt));
                    }
                    k += 1;
                }
                v
            })
            .collect()
    };
    let xs = spans(w, sx, src.w);
    let ys = spans(h, sy, src.h);
    for (oy, yw) in ys.iter().enumerate() {
        for (ox, xw) in xs.iter().enumerate() {
            let (mut acc, mut tot) = (0.0, 0.0);
            for &(y, wy) in yw {
                for &(x, wx) in xw {
                    acc += src.get(x, y) * wx * wy;
                    tot += wx * wy;
                }
            }
            out.set(ox, oy, if tot > 0.0 { acc / tot } else { 0.0 });
        }
    }
    out
}

/// Empty border, in pixels, around a glyph in its vector, so a solid shape
/// (a minus bar) still has an edge to correlate.
const PAD: usize = 2;

/// The vector a cut is compared as: `w` by `h`, zero mean, unit length.
/// A glyph-shaped target (`w <= 1.5 h`) keeps the cut's aspect: it is scaled
/// to height `h - 4` and centred with a [`PAD`] border. A word-shaped target
/// stretches the cut.
pub fn vector(ink: &Plane, w: usize, h: usize) -> Vec<f32> {
    unit(canvas(ink, w, h).px)
}

fn canvas(ink: &Plane, w: usize, h: usize) -> Plane {
    if (w as f32) <= h as f32 * 1.5 {
        let (iw, ih) = (w - 2 * PAD, h - 2 * PAD);
        let nw = ((ink.w as f32 * ih as f32 / ink.h.max(1) as f32).round() as usize).clamp(1, iw);
        let g = resize(ink, nw, ih);
        let mut can = Plane::new(w, h);
        let x0 = (w - nw) / 2;
        for y in 0..ih {
            for x in 0..nw {
                can.set(x0 + x, PAD + y, g.get(x, y));
            }
        }
        can
    } else {
        resize(ink, w, h)
    }
}

/// [`vector`] and its copies moved by one vector pixel in each direction
/// (9 in all, unmoved first). A small glyph (12 rows at 1080p) is placed
/// in its vector from a tight crop of soft ink, so its cut can sit a pixel
/// off the template; matching takes the best of these.
pub fn shifted_vectors(ink: &Plane, w: usize, h: usize) -> Vec<Vec<f32>> {
    let can = canvas(ink, w, h);
    let mut out = Vec::with_capacity(9);
    for (dx, dy) in [
        (0i32, 0i32),
        (-1, 0),
        (1, 0),
        (0, -1),
        (0, 1),
        (-1, -1),
        (-1, 1),
        (1, -1),
        (1, 1),
    ] {
        let mut p = Plane::new(w, h);
        for y in 0..h as i32 {
            for x in 0..w as i32 {
                let (sx, sy) = (x - dx, y - dy);
                if (0..w as i32).contains(&sx) && (0..h as i32).contains(&sy) {
                    p.set(x as usize, y as usize, can.get(sx as usize, sy as usize));
                }
            }
        }
        out.push(unit(p.px));
    }
    out
}

/// Zero mean, unit length (all zeros stays zeros).
pub fn unit(mut v: Vec<f32>) -> Vec<f32> {
    let mean = v.iter().sum::<f32>() / v.len().max(1) as f32;
    v.iter_mut().for_each(|x| *x -= mean);
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if n > 1e-6 {
        v.iter_mut().for_each(|x| *x /= n);
    }
    v
}

/// Width over height.
pub fn aspect(ink: &Plane) -> f32 {
    ink.w as f32 / ink.h.max(1) as f32
}
