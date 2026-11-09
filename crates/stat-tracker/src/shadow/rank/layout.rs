//! Where each field is on the rank screen, and how its text is cut out.
//! Positions were measured on saved rank screens at 2560x1440; at other 16:9
//! sizes (1080p, …) every box and pixel threshold is scaled by
//! [`Scale`] (`width/2560`, `height/1440`). The colour tests are in
//! [`super::ink`].

use image::RgbImage;

use super::ink::{
    self, Glyph, ON, Plane, adaptive, cluster_right, drop_short_columns, first_cluster, glyphs,
    plane, runs, split_wide, strip_minus, tight_glyph,
};
use super::{FRAME_H, FRAME_W};

/// Screen scale relative to the 2560x1440 reference layout.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Scale {
    pub sx: f32,
    pub sy: f32,
}

impl Scale {
    /// Accept ~16:9 frames whose height is in 720..=2160 (covers 1080p and
    /// 1440p; other aspect ratios are refused).
    pub fn from_dims(w: u32, h: u32) -> Option<Self> {
        if !(720..=2160).contains(&h) || w == 0 {
            return None;
        }
        let aspect = w as f32 / h as f32;
        if (aspect - 16.0 / 9.0).abs() > 0.02 {
            return None;
        }
        Some(Self {
            sx: w as f32 / FRAME_W as f32,
            sy: h as f32 / FRAME_H as f32,
        })
    }

    pub fn from_img(img: &RgbImage) -> Option<Self> {
        let (w, h) = img.dimensions();
        Self::from_dims(w, h)
    }

    pub fn x(self, v: u32) -> u32 {
        (v as f32 * self.sx).round() as u32
    }

    pub fn y(self, v: u32) -> u32 {
        (v as f32 * self.sy).round() as u32
    }

    /// Length measured in reference pixels, scaled by height.
    pub fn n(self, v: u32) -> u32 {
        ((v as f32 * self.sy).round() as u32).max(1)
    }

    /// Count that grows with width (e.g. white pixels across a band).
    pub fn nx(self, v: u32) -> u32 {
        ((v as f32 * self.sx).round() as u32).max(1)
    }

    pub fn f(self, v: f32) -> f32 {
        v * self.sy
    }
}

/// The topmost hero entry: its name line and SR line, as row spans.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry {
    pub name: (u32, u32),
    pub sr: (u32, u32),
}

/// The "RANK PROGRESS:" label: present on every rank update screen.
pub fn gate_word(img: &RgbImage) -> Option<Glyph> {
    let s = Scale::from_img(img)?;
    let (x0, y0) = (s.x(840), s.y(1066));
    tight_glyph(
        &plane(img, x0, y0, s.x(975), s.y(1092), ink::white),
        x0,
        y0,
    )
}

/// Rank progress glyphs (digits, `%`, a leading `-`), and whether the text
/// is orange (negative, under demotion protection) rather than cyan.
pub fn pct_glyphs(img: &RgbImage) -> (Vec<Glyph>, bool) {
    let Some(s) = Scale::from_img(img) else {
        return (Vec::new(), false);
    };
    let (x0, y0, x1, y1) = (s.x(960), s.y(1058), s.x(1140), s.y(1100));
    let cy = plane(img, x0, y0, x1, y1, ink::cyan);
    let or = plane(img, x0, y0, x1, y1, ink::orange);
    let negative = or.sum() > cy.sum();
    let both = Plane {
        px: cy.px.iter().zip(&or.px).map(|(a, b)| a.max(*b)).collect(),
        ..cy
    };
    (glyphs(&both, x0, y0, 0.5, s.n(4)), negative)
}

/// "Higher ranked than X%" glyphs (yellow).
pub fn higher_glyphs(img: &RgbImage) -> Vec<Glyph> {
    let Some(s) = Scale::from_img(img) else {
        return Vec::new();
    };
    let (x0, y0) = (s.x(1715), s.y(1076));
    glyphs(
        &plane(img, x0, y0, s.x(1800), s.y(1104), ink::yellow),
        x0,
        y0,
        0.5,
        s.n(4),
    )
}

/// The change pill's glyphs (white text on a green or red pill) and the
/// pill's sign. A column belongs to the pill when the rows just above and
/// below the text are pill coloured; the pill's white rim and the white
/// bar fill fail that test. On a red pill a minus bar merged into the first
/// digit is cut off ([`strip_minus`]).
pub fn pill_glyphs(img: &RgbImage) -> (Vec<Glyph>, Option<i32>) {
    let Some(s) = Scale::from_img(img) else {
        return (Vec::new(), None);
    };
    let px0 = s.x(600);
    let px1 = s.x(1960);
    let py0 = s.y(1116);
    let mean_rows = |a: u32, b: u32| -> Vec<[f32; 3]> {
        let a = s.n(a);
        let b = s.n(b);
        (px0..px1)
            .map(|x| {
                let mut sum = [0.0f32; 3];
                for y in py0 + a..py0 + b {
                    let p = img.get_pixel(x, y).0;
                    for k in 0..3 {
                        sum[k] += p[k] as f32;
                    }
                }
                let den = (b - a).max(1) as f32;
                sum.map(|v| v / den)
            })
            .collect()
    };
    let (top, bot) = (mean_rows(10, 13), mean_rows(38, 41));
    let g: Vec<f32> = top
        .iter()
        .zip(&bot)
        .map(|(a, b)| ink::green(*a).min(ink::green(*b)))
        .collect();
    let r: Vec<f32> = top
        .iter()
        .zip(&bot)
        .map(|(a, b)| ink::red(*a).min(ink::red(*b)))
        .collect();
    let mut w = plane(img, px0, py0 + s.n(14), px1, py0 + s.n(38), ink::white);
    for x in 0..w.w {
        if g[x].max(r[x]) <= 0.5 {
            for y in 0..w.h {
                w.set(x, y, 0.0);
            }
        }
    }
    let gl = glyphs(&w, px0, py0 + s.n(14), 0.5, s.n(4));
    let (Some(first), Some(last)) = (gl.first(), gl.last()) else {
        return (gl, None);
    };
    let span = (first.bbox[0] - px0) as usize..(last.bbox[2] - px0) as usize;
    let n = span.len().max(1) as f32;
    let gm: f32 = g[span.clone()].iter().sum::<f32>() / n;
    let rm: f32 = r[span].iter().sum::<f32>() / n;
    if gm > rm {
        (gl, Some(1))
    } else {
        (gl.into_iter().map(strip_minus).collect(), Some(-1))
    }
}

fn hero_box(s: Scale) -> (u32, u32, u32, u32) {
    (s.x(2120), s.y(560), s.x(2342), s.y(1300))
}

fn neutral_plane(img: &RgbImage, x0: u32, y0: u32, x1: u32, y1: u32) -> Plane {
    plane(img, x0, y0, x1, y1, ink::neutral)
}

/// Rows of grey-white text in the hero column, as row spans.
fn hero_lines(img: &RgbImage, s: Scale) -> Vec<(u32, u32)> {
    let (hx0, hy0, hx1, hy1) = hero_box(s);
    let n = neutral_plane(img, hx0, hy0, hx1, hy1);
    runs(&n.row_counts(s.f(14.0)), s.f(3.0), s.n(3) as usize)
        .into_iter()
        .map(|(a, b)| (hy0 + a as u32, hy0 + b as u32))
        .collect()
}

/// The first hero entry: a name line (11 to 20 rows) over a taller SR line
/// (19 to 34 rows, at least 3 taller), 4 to 22 rows below it.
pub fn first_entry(img: &RgbImage) -> Option<Entry> {
    let s = Scale::from_img(img)?;
    let ls = hero_lines(img, s);
    let (hn0, hn1) = (s.n(11), s.n(20));
    let (hs0, hs1) = (s.n(19), s.n(34));
    let taller = s.n(3);
    let (gap0, gap1) = (s.n(4), s.n(22));
    ls.windows(2).find_map(|w| {
        let (n, sr) = (w[0], w[1]);
        let (hn, hs) = (n.1 - n.0, sr.1 - sr.0);
        let gap = sr.0 - n.1;
        ((hn0..=hn1).contains(&hn)
            && (hs0..=hs1).contains(&hs)
            && hs >= hn + taller
            && (gap0..=gap1).contains(&gap))
        .then_some(Entry { name: n, sr })
    })
}

fn band_ink(img: &RgbImage, s: Scale, y0: u32, y1: u32, x1: u32) -> Plane {
    let (hx0, _, _, _) = hero_box(s);
    adaptive(&neutral_plane(img, hx0, y0, x1, y1), 0.98, 25.0)
}

/// Right edge of the hero name. The SR digits and the change are right
/// aligned to it; placement pips further right are not part of the entry.
fn name_right(img: &RgbImage, s: Scale, e: &Entry) -> u32 {
    let (hx0, _, hx1, _) = hero_box(s);
    let ink = band_ink(img, s, e.name.0, e.name.1, hx1);
    first_cluster(&runs(&ink.col_sums(), 0.5, 1), s.n(14) as usize)
        .map_or(hx1, |(_, b)| hx0 + b as u32)
}

/// The hero SR glyphs (the rank icon left of them may be included; the
/// reader keeps the rightmost four).
pub fn sr_glyphs(img: &RgbImage, e: &Entry) -> Vec<Glyph> {
    let Some(s) = Scale::from_img(img) else {
        return Vec::new();
    };
    let (hx0, _, hx1, _) = hero_box(s);
    let x1 = hx1.min(name_right(img, s, e) + s.n(3));
    let y0 = e.sr.0.saturating_sub(s.n(2));
    let gl = glyphs(
        &band_ink(img, s, y0, e.sr.1 + s.n(2), x1),
        hx0,
        y0,
        1.0,
        s.n(4),
    );
    cluster_right(gl, (0.75 * (e.sr.1 - e.sr.0) as f32) as u32)
}

/// The hero SR change digits under the SR line and their sign (green up,
/// red down). The caret and minus are dropped as short columns.
pub fn change_glyphs(img: &RgbImage, e: &Entry) -> (Vec<Glyph>, Option<i32>) {
    let Some(s) = Scale::from_img(img) else {
        return (Vec::new(), None);
    };
    let (hx0, _, hx1, _) = hero_box(s);
    let x1 = hx1.min(name_right(img, s, e) + s.n(3));
    let y0 = e.sr.1;
    let gs = plane(img, hx0, y0, x1, y0 + s.n(30), ink::green_score);
    let rs = plane(img, hx0, y0, x1, y0 + s.n(30), ink::red_score);
    // one or two small digits light little of the window: take the peak
    // from the top half percent
    let (pg, pr) = (gs.quantile(CHANGE_PEAK), rs.quantile(CHANGE_PEAK));
    if pg.max(pr) < 25.0 {
        return (Vec::new(), None);
    }
    let (score, sign) = if pg > pr { (gs, 1) } else { (rs, -1) };
    let ink = drop_short_columns(&adaptive(&score, CHANGE_PEAK, 25.0), s.n(8) as usize);
    let gl = split_wide(glyphs(&ink, hx0, y0, 1.0, s.n(4)), s.f(17.0));
    (cluster_right(gl, s.n(10)), Some(sign))
}

const CHANGE_PEAK: f32 = 0.995;

/// The tag words under the bar, left to right. Each tag is a coloured icon
/// disc (26 to 46 px wide at 1440p) followed by a white word.
pub fn tag_words(img: &RgbImage) -> Vec<Glyph> {
    let Some(s) = Scale::from_img(img) else {
        return Vec::new();
    };
    let (x0, x1) = (s.x(560), s.x(1960));
    let ty0 = s.y(1180);
    let disc = plane(img, x0, ty0 + s.n(5), x1, ty0 + s.n(40), |c| {
        ink::green(c).max(ink::red(c))
    });
    let (d0, d1) = (s.n(26) as usize, s.n(46) as usize);
    let icons: Vec<(usize, usize)> = runs(&disc.col_sums(), 2.0, 1)
        .into_iter()
        .filter(|(a, b)| (d0..=d1).contains(&(b - a)))
        .collect();
    let w = plane(img, x0, ty0 + s.n(9), x1, ty0 + s.n(34), ink::white);
    let mut out = Vec::new();
    for (k, &(_, b)) in icons.iter().enumerate() {
        let start = b + s.n(4) as usize;
        let end = icons.get(k + 1).map_or(
            (b + s.n(420) as usize).min(w.w),
            |n| n.0.saturating_sub(s.n(4) as usize),
        );
        if start >= end {
            continue;
        }
        let part = w.cols(start, end);
        if let Some((p, q)) =
            first_cluster(&runs(&part.col_sums(), 0.5, 1), s.n(20) as usize)
            && let Some(gl) = tight_glyph(
                &part.cols(p, q),
                x0 + (start + p) as u32,
                ty0 + s.n(9),
            )
        {
            out.push(gl);
        }
    }
    out
}

/// The mode banner text (top right), up to the timer.
pub fn banner_word(img: &RgbImage) -> Option<Glyph> {
    let s = Scale::from_img(img)?;
    let (x0, y0) = (s.x(2056), s.y(136));
    let w = plane(img, x0, y0, s.x(2430), s.y(186), ink::white);
    let (a, b) = first_cluster(&runs(&w.col_sums(), 0.5, 1), s.n(30) as usize)?;
    tight_glyph(&w.cols(a, b), x0 + a as u32, y0)
}

/// The tier word and the division digit under the rank emblem. The text
/// line is the lowest band of 30 to 60 rows with over 40 white pixels per
/// row; word and digit split at the widest gap.
pub fn rank_text(img: &RgbImage) -> Option<(Glyph, Glyph)> {
    let s = Scale::from_img(img)?;
    let (x0, y0) = (s.x(1150), s.y(820));
    let w = plane(img, x0, y0, s.x(1420), s.y(1020), ink::white);
    let (b0, b1) = (s.n(30) as usize, s.n(60) as usize);
    let (a, b) = runs(&w.row_counts(ON), s.nx(40) as f32, s.n(3) as usize)
        .into_iter()
        .rev()
        .find(|(a, b)| (b0..=b1).contains(&(b - a)))?;
    let band = w.sub(0, a, w.w, b);
    let cr: Vec<(usize, usize)> = runs(&band.col_sums(), 0.5, 1)
        .into_iter()
        .filter(|&(p, q)| p > 0 && q < band.w)
        .collect();
    if cr.len() < 2 {
        return None;
    }
    let k = (0..cr.len() - 1).max_by_key(|&k| (cr[k + 1].0 - cr[k].1, usize::MAX - k))?;
    let oy = y0 + a as u32;
    let word = tight_glyph(&band.cols(cr[0].0, cr[k].1), x0 + cr[0].0 as u32, oy)?;
    let digit = tight_glyph(
        &band.cols(cr[k + 1].0, cr[cr.len() - 1].1),
        x0 + cr[k + 1].0 as u32,
        oy,
    )?;
    Some((word, digit))
}
