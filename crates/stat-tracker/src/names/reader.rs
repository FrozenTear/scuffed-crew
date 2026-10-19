//! Locate a name's letters in a player row and match them to the pack.

use super::pack::{CANVAS, DIM, MARK_CANVAS, NamePack};
use super::resize::resize_l;

/// Why a row gave no name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Abstain {
    /// No line of name-sized letters in the row.
    NoLine,
    /// An accent the pack has no mark or no letter for.
    Accent,
    /// A gap that may or may not be a space.
    Space,
}

/// What the reader made of a row before any confidence cut.
#[derive(Debug, Clone, PartialEq)]
pub struct RawRead {
    pub text: Result<String, Abstain>,
    /// Smallest best-vs-second margin over the letters (0 on abstain).
    pub confidence: f32,
    /// Ink in the name line no letter explains (a descender, a clipped mark).
    pub unaccounted: usize,
    pub glyphs: usize,
}

#[derive(Clone, Copy, Debug)]
struct Comp {
    x0: i64,
    y0: i64,
    x1: i64,
    y1: i64,
    n: usize,
    id: i32,
}

/// 8-connected components in raster order of their first pixel.
fn components(mask: &[bool], w: usize, h: usize) -> (Vec<Comp>, Vec<i32>, Vec<Vec<u32>>) {
    let mut lab = vec![-1i32; w * h];
    let mut out = Vec::new();
    let mut pix: Vec<Vec<u32>> = Vec::new();
    let mut st = Vec::new();
    for start in 0..w * h {
        if !mask[start] || lab[start] >= 0 {
            continue;
        }
        let id = out.len() as i32;
        lab[start] = id;
        st.push(start);
        let (mut x0, mut y0, mut x1, mut y1, mut n) = (i64::MAX, i64::MAX, 0i64, 0i64, 0usize);
        let mut px = Vec::new();
        while let Some(p) = st.pop() {
            px.push(p as u32);
            let (cx, cy) = ((p % w) as i64, (p / w) as i64);
            x0 = x0.min(cx);
            y0 = y0.min(cy);
            x1 = x1.max(cx + 1);
            y1 = y1.max(cy + 1);
            n += 1;
            for (dy, dx) in [
                (-1, 0),
                (1, 0),
                (0, -1),
                (0, 1),
                (-1, -1),
                (-1, 1),
                (1, -1),
                (1, 1),
            ] {
                let (ny, nx) = (cy + dy, cx + dx);
                if ny < 0 || nx < 0 || ny >= h as i64 || nx >= w as i64 {
                    continue;
                }
                let q = ny as usize * w + nx as usize;
                if mask[q] && lab[q] < 0 {
                    lab[q] = id;
                    st.push(q);
                }
            }
        }
        out.push(Comp {
            x0,
            y0,
            x1,
            y1,
            n,
            id,
        });
        pix.push(px);
    }
    (out, lab, pix)
}

struct Glyph {
    x0: i64,
    x1: i64,
    ids: Vec<i32>,
}

pub(crate) struct Loc {
    w: usize,
    h: usize,
    soft: Vec<f32>,
    lab: Vec<i32>,
    glyphs: Vec<Glyph>,
    top: i64,
    bot: i64,
    /// Pixels (index into the row) of every glyph and accent part.
    pix: std::collections::HashMap<i32, Vec<u32>>,
}

fn median_trunc(mut v: Vec<i64>) -> i64 {
    v.sort_unstable();
    let n = v.len();
    let m = if n % 2 == 1 {
        v[n / 2] as f64
    } else {
        (v[n / 2 - 1] + v[n / 2]) as f64 / 2.0
    };
    m as i64
}

/// Find the name's letters in an RGB row crop.
pub(crate) fn locate(rgb: &[u8], w: usize, h: usize, hc: f64) -> Option<Loc> {
    let mut soft = vec![0f32; w * h];
    let mut hard = vec![false; w * h];
    let mut loose = vec![false; w * h];
    for i in 0..w * h {
        let (r, g, b) = (
            rgb[3 * i] as i32,
            rgb[3 * i + 1] as i32,
            rgb[3 * i + 2] as i32,
        );
        let mn = r.min(g).min(b);
        let sp = r.max(g).max(b) - mn;
        let a = ((mn as f64 - 120.0) / 80.0).clamp(0.0, 1.0);
        let c = ((70.0 - sp as f64) / 35.0).clamp(0.0, 1.0);
        soft[i] = (a * c) as f32;
        hard[i] = mn >= 165 && sp <= 50;
        loose[i] = mn >= 125 && sp <= 60;
    }
    let (cs, mut lab, hpix) = components(&hard, w, h);
    let mut tall: Vec<Comp> = cs
        .iter()
        .filter(|c| {
            let ch = (c.y1 - c.y0) as f64;
            c.n >= 4
                && 0.82 * hc <= ch
                && ch <= 1.22 * hc
                && ((c.x1 - c.x0) as f64) <= 1.6 * hc
                && c.y0 > 0
                && c.y1 < h as i64
        })
        .copied()
        .collect();
    if tall.is_empty() {
        return None;
    }
    tall.sort_by_key(|c| c.x0);
    let mut clusters: Vec<Vec<Comp>> = Vec::new();
    for c in tall {
        let mut placed = false;
        for cl in clusters.iter_mut() {
            let last = cl[cl.len() - 1];
            if ((c.y1 - last.y1).abs() as f64) <= (0.12 * hc).max(2.0)
                && ((c.x0 - last.x1) as f64) <= 1.1 * hc
            {
                cl.push(c);
                placed = true;
                break;
            }
        }
        if !placed {
            clusters.push(vec![c]);
        }
    }
    clusters.retain(|cl| cl.len() >= 2);
    let mid = h as f64 / 2.0;
    let key = |cl: &Vec<Comp>| {
        (
            cl.len(),
            -(((cl[0].y0 + cl[0].y1) as f64 / 2.0) - mid).abs(),
        )
    };
    let mut best: Option<&Vec<Comp>> = None;
    for cl in &clusters {
        if best.is_none_or(|b| {
            let (kb, kc) = (key(b), key(cl));
            kc.0 > kb.0 || (kc.0 == kb.0 && kc.1 > kb.1)
        }) {
            best = Some(cl);
        }
    }
    let cl = best?;
    let top = median_trunc(cl.iter().map(|c| c.y0).collect());
    let bot = median_trunc(cl.iter().map(|c| c.y1).collect());
    let (x0, x1) = (cl[0].x0, cl[cl.len() - 1].x1);
    let mut glyphs: Vec<Glyph> = cl
        .iter()
        .map(|c| Glyph {
            x0: c.x0,
            x1: c.x1,
            ids: vec![c.id],
        })
        .collect();
    // accents: small marks in the looser mask, fully above the cap line, over a glyph
    let cut = (top - 1).max(0) as usize;
    for v in loose.iter_mut().skip(cut.min(h) * w) {
        *v = false;
    }
    let (lcs, _llab, lpix) = components(&loose, w, h);
    let mut pix: std::collections::HashMap<i32, Vec<u32>> = glyphs
        .iter()
        .map(|g| (g.ids[0], hpix[g.ids[0] as usize].clone()))
        .collect();
    let off = lab.iter().copied().max().unwrap_or(-1) + 1;
    for c in &lcs {
        let (cw, ch) = ((c.x1 - c.x0) as f64, (c.y1 - c.y0) as f64);
        if c.n >= 3
            && ch >= 2.0
            && cw <= 0.5 * hc
            && c.y0 as f64 >= top as f64 - 0.6 * hc
            && ch < 0.5 * hc
            && x0 - 2 <= c.x0
            && c.x1 <= x1 + 2
        {
            let mut j = 0;
            let mut best_ov = i64::MIN;
            for (i, g) in glyphs.iter().enumerate() {
                let ov = c.x1.min(g.x1) - c.x0.max(g.x0);
                if ov > best_ov {
                    best_ov = ov;
                    j = i;
                }
            }
            if best_ov > 0 {
                let nid = off + c.id;
                for &p in &lpix[c.id as usize] {
                    lab[p as usize] = nid;
                }
                pix.insert(nid, lpix[c.id as usize].clone());
                glyphs[j].ids.push(nid);
            }
        }
    }
    Some(Loc {
        w,
        h,
        soft,
        lab,
        glyphs,
        top,
        bot,
        pix,
    })
}

impl Loc {
    /// `soft` kept where the 4-neighbour dilation of `ids` is, inside a window.
    fn masked(
        &self,
        ids: &[i32],
        x0: i64,
        x1: i64,
        y0: i64,
        y1: i64,
    ) -> impl Fn(i64, i64) -> f32 + '_ {
        let _ = (x0, x1, y0, y1);
        let ids = ids.to_vec();
        move |x, y| {
            let inside = |x: i64, y: i64| {
                x >= 0
                    && y >= 0
                    && (x as usize) < self.w
                    && (y as usize) < self.h
                    && ids.contains(&self.lab[y as usize * self.w + x as usize])
            };
            if inside(x, y)
                || inside(x - 1, y)
                || inside(x + 1, y)
                || inside(x, y - 1)
                || inside(x, y + 1)
            {
                self.soft[y as usize * self.w + x as usize]
            } else {
                0.0
            }
        }
    }

    fn masked_px(&self, pm: &[bool], x: i64, y: i64) -> f32 {
        let inside = |x: i64, y: i64| {
            x >= 0
                && y >= 0
                && (x as usize) < self.w
                && (y as usize) < self.h
                && pm[y as usize * self.w + x as usize]
        };
        if inside(x, y)
            || inside(x - 1, y)
            || inside(x + 1, y)
            || inside(x, y - 1)
            || inside(x, y + 1)
        {
            self.soft[y as usize * self.w + x as usize]
        } else {
            0.0
        }
    }

    /// The letter tile: rows from 0.45 cap heights above the cap line to
    /// 0.08 below the base line, one blank column each side, stretched to 16x28.
    fn canvas(&self, px: &dyn Fn(i64, i64) -> f32, x0: i64, x1: i64, hc: f64) -> Vec<u8> {
        let y0 = (self.top as f64 - 0.45 * hc).round_ties_even() as i64;
        let y1 = (self.bot as f64 + 0.08 * hc).round_ties_even() as i64;
        let (pw, ph) = ((x1 - x0 + 2) as usize, (y1 - y0).max(0) as usize);
        let mut pad = vec![0u8; pw * ph];
        for y in y0.max(0)..y1.min(self.h as i64) {
            for x in x0..x1 {
                let v = px(x, y);
                pad[(y - y0) as usize * pw + (x - x0 + 1) as usize] = (v * 255.0) as u8;
            }
        }
        resize_l(&pad, pw, ph, CANVAS.0, CANVAS.1)
    }

    fn mark_canvas(&self, ids: &[i32], bx0: i64, bx1: i64, hc: f64) -> Vec<u8> {
        let (mut mx0, mut mx1) = (i64::MAX, i64::MIN);
        for id in ids {
            for &p in &self.pix[id] {
                let x = (p as usize % self.w) as i64;
                mx0 = mx0.min(x);
                mx1 = mx1.max(x + 1);
            }
        }
        let y0 = (self.top as f64 - 0.7 * hc).round_ties_even() as i64;
        let y1 = self.top;
        let xs0 = bx0.min(mx0);
        let ww = bx1.max(mx1) - xs0;
        let (pw, ph) = ((ww + 2) as usize, (y1 - y0).max(0) as usize);
        let px = self.masked(ids, 0, 0, 0, 0);
        let mut pad = vec![0u8; pw * ph];
        for y in y0.max(0)..y1 {
            for x in xs0..(xs0 + ww).min(self.w as i64) {
                pad[(y - y0) as usize * pw + (x - xs0 + 1) as usize] = (px(x, y) * 255.0) as u8;
            }
        }
        resize_l(&pad, pw, ph, MARK_CANVAS.0, MARK_CANVAS.1)
    }

    /// Ink in the name line that no glyph explains.
    fn unaccounted(&self, hc: f64) -> usize {
        let xmin = self.glyphs.iter().map(|g| g.x0).min().unwrap_or(0);
        let xmax = self.glyphs.iter().map(|g| g.x1).max().unwrap_or(0);
        let x0 = (xmin as f64 - 0.8 * hc).max(0.0) as i64;
        let x1 = (self.w as f64).min(xmax as f64 + 0.8 * hc) as i64;
        let used: Vec<i32> = self
            .glyphs
            .iter()
            .flat_map(|g| g.ids.iter().copied())
            .collect();
        let mut rows: std::collections::BTreeMap<i32, std::collections::BTreeSet<i64>> =
            Default::default();
        for y in self.top.max(0)..self.bot.min(self.h as i64) {
            for x in x0..x1 {
                let l = self.lab[y as usize * self.w + x as usize];
                if l >= 0 && !used.contains(&l) {
                    rows.entry(l).or_default().insert(y);
                }
            }
        }
        let need = 0.6 * (self.bot - self.top) as f64;
        rows.values().filter(|r| r.len() as f64 >= need).count()
    }

    /// Straightened x range of a glyph's main part.
    fn u_range(&self, id: i32, slant: f64) -> (f64, f64) {
        let (mut lo, mut hi) = (f64::MAX, f64::MIN);
        for &p in &self.pix[&id] {
            let i = p as usize;
            {
                let (x, y) = ((i % self.w) as f64, (i / self.w) as f64);
                let u = x + slant * (y - self.bot as f64);
                lo = lo.min(u);
                hi = hi.max(u);
            }
        }
        (lo, hi)
    }
}

/// A tile as a zero-mean unit vector.
pub(crate) fn unit(t: &[u8]) -> Vec<f32> {
    let v: Vec<f32> = t.iter().map(|&p| p as f32 / 255.0).collect();
    let mean = (v.iter().map(|&x| x as f64).sum::<f64>() / v.len() as f64) as f32;
    let mut v: Vec<f32> = v.iter().map(|x| x - mean).collect();
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    let n = if n > 1e-6 { n } else { 1.0 };
    for x in v.iter_mut() {
        *x /= n;
    }
    v
}

/// `v @ whiten`, unit length.
pub(crate) fn whitened(v: &[f32], whiten: &[f32]) -> Vec<f32> {
    let mut z = vec![0f32; DIM];
    for (i, &vi) in v.iter().enumerate() {
        if vi == 0.0 {
            continue;
        }
        let row = &whiten[i * DIM..(i + 1) * DIM];
        for (zj, &w) in z.iter_mut().zip(row) {
            *zj += vi * w;
        }
    }
    let n = z.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-12);
    for x in z.iter_mut() {
        *x /= n;
    }
    z
}

struct Top2 {
    class: char,
    score: f32,
    margin: f32,
}

fn dot(a: &[f32], b: &[f32]) -> f32 {
    let mut acc = [0f32; 8];
    for (ca, cb) in a.as_chunks::<8>().0.iter().zip(b.as_chunks::<8>().0) {
        for k in 0..8 {
            acc[k] += ca[k] * cb[k];
        }
    }
    acc.iter().sum()
}

/// Best class for each query tile: (best class, its score, lead over the next class).
fn top2_many(pack: &NamePack, q: &[(Vec<f32>, f32)]) -> Vec<Top2> {
    let zs: Vec<Vec<f32>> = q.iter().map(|(v, _)| whitened(v, &pack.whiten)).collect();
    let lr: Vec<f32> = q.iter().map(|(_, r)| r.max(1e-3).ln()).collect();
    let nc = pack.classes.len();
    let mut best = vec![f32::NEG_INFINITY; q.len() * nc];
    for (t, zt) in pack.z.as_chunks::<DIM>().0.iter().enumerate() {
        let c = pack.class[t] as usize;
        let lw = pack.width[t].ln();
        for (k, z) in zs.iter().enumerate() {
            let s = dot(zt, z) - pack.params.rw * (lr[k] - lw).abs();
            let b = &mut best[k * nc + c];
            if s > *b {
                *b = s;
            }
        }
    }
    (0..q.len())
        .map(|k| {
            let b = &best[k * nc..(k + 1) * nc];
            let (mut i0, mut i1) = (usize::MAX, usize::MAX);
            for i in 0..nc {
                if i0 == usize::MAX || b[i] > b[i0] {
                    i1 = i0;
                    i0 = i;
                } else if i1 == usize::MAX || b[i] > b[i1] {
                    i1 = i;
                }
            }
            Top2 {
                class: pack.classes[i0],
                score: b[i0],
                margin: b[i0] - b[i1],
            }
        })
        .collect()
}

fn mark(pack: &NamePack, v: &[f32], parts: usize) -> Option<char> {
    let mut best: Vec<(char, f32)> = Vec::new();
    for (t, n, c) in &pack.marks {
        if *n as usize != parts {
            continue;
        }
        let s = dot(t, v);
        match best.iter_mut().find(|(m, _)| m == c) {
            Some(e) => e.1 = e.1.max(s),
            None => best.push((*c, s)),
        }
    }
    best.sort_by(|a, b| b.1.total_cmp(&a.1));
    let first = best.first()?;
    if first.1 < pack.params.mark_min {
        return None;
    }
    if best.len() > 1 && first.1 - best[1].1 < pack.params.mark_margin {
        return None;
    }
    Some(first.0)
}

/// Latin letters a Cyrillic name shares with Latin, and the letters only Cyrillic has.
const HOMO: [(char, char); 11] = [
    ('А', 'A'),
    ('В', 'B'),
    ('Е', 'E'),
    ('К', 'K'),
    ('М', 'M'),
    ('Н', 'H'),
    ('О', 'O'),
    ('Р', 'P'),
    ('С', 'C'),
    ('Т', 'T'),
    ('Х', 'X'),
];
const CYR_ONLY: &str = "БГДЖЗИЙЛПФЦЧШЩЪЫЬЭЮЯ";

/// Read the name in an RGB row crop. `hc` is the name's cap height in pixels.
pub fn read_raw(rgb: &[u8], w: usize, h: usize, hc: f64, pack: &NamePack) -> RawRead {
    let none = |why| RawRead {
        text: Err(why),
        confidence: 0.0,
        unaccounted: 0,
        glyphs: 0,
    };
    let Some(loc) = locate(rgb, w, h, hc) else {
        return none(Abstain::NoLine);
    };
    let unaccounted = loc.unaccounted(hc);
    let slant = pack.params.slant;
    // tiles: per glyph the whole tile, and for a wide one-part glyph a two-way split
    enum Plan {
        One(usize),
        Accented {
            base: usize,
            mark: Vec<f32>,
            parts: usize,
        },
        Wide {
            whole: usize,
            left: usize,
            right: usize,
        },
    }
    let mut q: Vec<(Vec<f32>, f32)> = Vec::new();
    let mut plans = Vec::new();
    for g in &loc.glyphs {
        if g.ids.len() > 1 {
            let (mut bx0, mut bx1) = (i64::MAX, i64::MIN);
            for &p in &loc.pix[&g.ids[0]] {
                let i = p as usize;
                {
                    let x = (i % loc.w) as i64;
                    bx0 = bx0.min(x);
                    bx1 = bx1.max(x + 1);
                }
            }
            let px = loc.masked(&g.ids[..1], 0, 0, 0, 0);
            let t = loc.canvas(&px, bx0, bx1, hc);
            q.push((unit(&t), ((bx1 - bx0) as f64 / hc) as f32));
            let m = unit(&loc.mark_canvas(&g.ids[1..], bx0, bx1, hc));
            plans.push(Plan::Accented {
                base: q.len() - 1,
                mark: m,
                parts: g.ids.len() - 1,
            });
            continue;
        }
        let px = loc.masked(&g.ids, 0, 0, 0, 0);
        let r = (g.x1 - g.x0) as f64 / hc;
        q.push((unit(&loc.canvas(&px, g.x0, g.x1, hc)), r as f32));
        let whole = q.len() - 1;
        if r >= 0.72 {
            let id = g.ids[0];
            let mut pts = Vec::new();
            let mut sorted = loc.pix[&id].clone();
            sorted.sort_unstable();
            for &p in &sorted {
                let i = p as usize;
                {
                    let (x, y) = ((i % loc.w) as i64, (i / loc.w) as i64);
                    pts.push((x, y, x as f64 + slant * (y - loc.bot) as f64));
                }
            }
            let lo = pts.iter().map(|p| p.2).fold(f64::MAX, f64::min);
            let hi = pts.iter().map(|p| p.2).fold(f64::MIN, f64::max);
            let (start, stop) = (lo + 0.3 * (hi - lo), lo + 0.7 * (hi - lo));
            let steps = ((stop - start) / 0.5).ceil().max(0.0) as usize;
            let mut best: Option<(usize, f64)> = None;
            for i in 0..steps {
                let c = start + i as f64 * 0.5;
                let n = pts.iter().filter(|p| (p.2 - c).abs() < 0.75).count();
                if best.is_none_or(|b| n < b.0) {
                    best = Some((n, c));
                }
            }
            if let Some((_, c)) = best {
                let mut sides = Vec::new();
                for left in [true, false] {
                    let mut pm = vec![false; loc.w * loc.h];
                    let (mut sx0, mut sx1) = (i64::MAX, i64::MIN);
                    for p in pts.iter().filter(|p| (p.2 < c) == left) {
                        pm[p.1 as usize * loc.w + p.0 as usize] = true;
                        sx0 = sx0.min(p.0);
                        sx1 = sx1.max(p.0 + 1);
                    }
                    if sx0 > sx1 {
                        break;
                    }
                    let f = |x: i64, y: i64| loc.masked_px(&pm, x, y);
                    q.push((
                        unit(&loc.canvas(&f, sx0, sx1, hc)),
                        ((sx1 - sx0) as f64 / hc) as f32,
                    ));
                    sides.push(q.len() - 1);
                }
                if let [l, rr] = sides[..] {
                    plans.push(Plan::Wide {
                        whole,
                        left: l,
                        right: rr,
                    });
                    continue;
                }
            }
        }
        plans.push(Plan::One(whole));
    }
    let tops = top2_many(pack, &q);
    let mut units: Vec<String> = Vec::new();
    let mut margins: Vec<f32> = Vec::new();
    for p in &plans {
        match p {
            Plan::One(i) => {
                units.push(tops[*i].class.to_string());
                margins.push(tops[*i].margin);
            }
            Plan::Wide { whole, left, right } => {
                let (a, b, w) = (&tops[*left], &tops[*right], &tops[*whole]);
                if (a.score + b.score) / 2.0 > w.score + 0.01 {
                    units.push(format!("{}{}", a.class, b.class));
                    margins.push(a.margin);
                    margins.push(b.margin);
                } else {
                    units.push(w.class.to_string());
                    margins.push(w.margin);
                }
            }
            Plan::Accented {
                base,
                mark: m,
                parts,
            } => {
                let b = &tops[*base];
                let Some(mk) = mark(pack, m, *parts) else {
                    return RawRead {
                        unaccounted,
                        glyphs: loc.glyphs.len(),
                        ..none(Abstain::Accent)
                    };
                };
                let Some(&ch) = pack.compose.get(&(b.class, mk)) else {
                    return RawRead {
                        unaccounted,
                        glyphs: loc.glyphs.len(),
                        ..none(Abstain::Accent)
                    };
                };
                units.push(ch.to_string());
                margins.push(b.margin);
            }
        }
    }
    // spaces from the straightened gap between neighbouring glyphs
    let ur: Vec<(f64, f64)> = loc
        .glyphs
        .iter()
        .map(|g| loc.u_range(g.ids[0], slant))
        .collect();
    let mut text = units[0].clone();
    for i in 1..units.len() {
        let gap = (ur[i].0 - ur[i - 1].1) / hc;
        if gap >= pack.params.space_lo && gap < pack.params.space_hi {
            return RawRead {
                unaccounted,
                glyphs: loc.glyphs.len(),
                ..none(Abstain::Space)
            };
        }
        if gap >= pack.params.space_hi {
            text.push(' ');
        }
        text.push_str(&units[i]);
    }
    if text.chars().any(|c| CYR_ONLY.contains(c)) {
        text = text
            .chars()
            .map(|c| HOMO.iter().find(|(_, l)| *l == c).map_or(c, |(cy, _)| *cy))
            .collect();
    }
    let confidence = margins.iter().copied().fold(f32::INFINITY, f32::min);
    RawRead {
        text: Ok(text),
        confidence,
        unaccounted,
        glyphs: loc.glyphs.len(),
    }
}

/// Letter tiles of the name line in a row, for building a pack: the 16x28
/// tile of every glyph (accent marks included) and its width over the cap height.
pub fn cut_tiles(rgb: &[u8], w: usize, h: usize, hc: f64) -> Vec<(Vec<u8>, f32)> {
    let Some(loc) = locate(rgb, w, h, hc) else {
        return Vec::new();
    };
    loc.glyphs
        .iter()
        .map(|g| {
            let px = loc.masked(&g.ids, 0, 0, 0, 0);
            (
                loc.canvas(&px, g.x0, g.x1, hc),
                ((g.x1 - g.x0) as f64 / hc) as f32,
            )
        })
        .collect()
}
