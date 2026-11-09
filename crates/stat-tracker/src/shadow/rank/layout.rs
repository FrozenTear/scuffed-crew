//! Where each field is on the 2560x1440 rank screen, and how its text is
//! cut out. Positions were measured on saved rank screens at that size; the
//! colour tests are in [`super::ink`].

use image::RgbImage;

use super::ink::{
    self, Glyph, ON, Plane, adaptive, cluster_right, drop_short_columns, first_cluster, glyphs,
    plane, runs, split_wide, strip_minus, tight_glyph,
};

/// The topmost hero entry: its name line and SR line, as row spans.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry {
    pub name: (u32, u32),
    pub sr: (u32, u32),
}

/// The "RANK PROGRESS:" label: present on every rank update screen.
pub fn gate_word(img: &RgbImage) -> Option<Glyph> {
    let (x0, y0) = (840, 1066);
    tight_glyph(&plane(img, x0, y0, 975, 1092, ink::white), x0, y0)
}

/// Rank progress glyphs (digits, `%`, a leading `-`), and whether the text
/// is orange (negative, under demotion protection) rather than cyan.
pub fn pct_glyphs(img: &RgbImage) -> (Vec<Glyph>, bool) {
    let (x0, y0, x1, y1) = (960, 1058, 1140, 1100);
    let cy = plane(img, x0, y0, x1, y1, ink::cyan);
    let or = plane(img, x0, y0, x1, y1, ink::orange);
    let negative = or.sum() > cy.sum();
    let both = Plane {
        px: cy.px.iter().zip(&or.px).map(|(a, b)| a.max(*b)).collect(),
        ..cy
    };
    (glyphs(&both, x0, y0, 0.5, 4), negative)
}

/// "Higher ranked than X%" glyphs (yellow).
pub fn higher_glyphs(img: &RgbImage) -> Vec<Glyph> {
    let (x0, y0) = (1715, 1076);
    glyphs(&plane(img, x0, y0, 1800, 1104, ink::yellow), x0, y0, 0.5, 4)
}

const PX0: u32 = 600;
const PX1: u32 = 1960;
const PY0: u32 = 1116;

/// The change pill's glyphs (white text on a green or red pill) and the
/// pill's sign. A column belongs to the pill when the rows just above and
/// below the text are pill coloured; the pill's white rim and the white
/// bar fill fail that test. On a red pill a minus bar merged into the first
/// digit is cut off ([`strip_minus`]).
pub fn pill_glyphs(img: &RgbImage) -> (Vec<Glyph>, Option<i32>) {
    let mean_rows = |a: u32, b: u32| -> Vec<[f32; 3]> {
        (PX0..PX1)
            .map(|x| {
                let mut s = [0.0f32; 3];
                for y in PY0 + a..PY0 + b {
                    let p = img.get_pixel(x, y).0;
                    for k in 0..3 {
                        s[k] += p[k] as f32;
                    }
                }
                s.map(|v| v / (b - a) as f32)
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
    let mut w = plane(img, PX0, PY0 + 14, PX1, PY0 + 38, ink::white);
    for x in 0..w.w {
        if g[x].max(r[x]) <= 0.5 {
            for y in 0..w.h {
                w.set(x, y, 0.0);
            }
        }
    }
    let gl = glyphs(&w, PX0, PY0 + 14, 0.5, 4);
    let (Some(first), Some(last)) = (gl.first(), gl.last()) else {
        return (gl, None);
    };
    let span = (first.bbox[0] - PX0) as usize..(last.bbox[2] - PX0) as usize;
    let n = span.len().max(1) as f32;
    let gm: f32 = g[span.clone()].iter().sum::<f32>() / n;
    let rm: f32 = r[span].iter().sum::<f32>() / n;
    if gm > rm {
        (gl, Some(1))
    } else {
        (gl.into_iter().map(strip_minus).collect(), Some(-1))
    }
}

const HX0: u32 = 2120;
const HX1: u32 = 2342;
const HY0: u32 = 560;
const HY1: u32 = 1300;

fn neutral_plane(img: &RgbImage, x0: u32, y0: u32, x1: u32, y1: u32) -> Plane {
    plane(img, x0, y0, x1, y1, ink::neutral)
}

/// Rows of grey-white text in the hero column, as row spans.
fn hero_lines(img: &RgbImage) -> Vec<(u32, u32)> {
    let n = neutral_plane(img, HX0, HY0, HX1, HY1);
    runs(&n.row_counts(14.0), 3.0, 3)
        .into_iter()
        .map(|(a, b)| (HY0 + a as u32, HY0 + b as u32))
        .collect()
}

/// The first hero entry: a name line (11 to 20 rows) over a taller SR line
/// (19 to 34 rows, at least 3 taller), 4 to 22 rows below it.
pub fn first_entry(img: &RgbImage) -> Option<Entry> {
    let ls = hero_lines(img);
    ls.windows(2).find_map(|w| {
        let (n, s) = (w[0], w[1]);
        let (hn, hs) = (n.1 - n.0, s.1 - s.0);
        let gap = s.0 - n.1;
        ((11..=20).contains(&hn)
            && (19..=34).contains(&hs)
            && hs >= hn + 3
            && (4..=22).contains(&gap))
        .then_some(Entry { name: n, sr: s })
    })
}

fn band_ink(img: &RgbImage, y0: u32, y1: u32, x1: u32) -> Plane {
    adaptive(&neutral_plane(img, HX0, y0, x1, y1), 0.98, 25.0)
}

/// Right edge of the hero name. The SR digits and the change are right
/// aligned to it; placement pips further right are not part of the entry.
fn name_right(img: &RgbImage, e: &Entry) -> u32 {
    let ink = band_ink(img, e.name.0, e.name.1, HX1);
    first_cluster(&runs(&ink.col_sums(), 0.5, 1), 14).map_or(HX1, |(_, b)| HX0 + b as u32)
}

/// The hero SR glyphs (the rank icon left of them may be included; the
/// reader keeps the rightmost four).
pub fn sr_glyphs(img: &RgbImage, e: &Entry) -> Vec<Glyph> {
    let x1 = HX1.min(name_right(img, e) + 3);
    let y0 = e.sr.0 - 2;
    let gl = glyphs(&band_ink(img, y0, e.sr.1 + 2, x1), HX0, y0, 1.0, 4);
    cluster_right(gl, (0.75 * (e.sr.1 - e.sr.0) as f32) as u32)
}

/// The hero SR change digits under the SR line and their sign (green up,
/// red down). The caret and minus are dropped as short columns.
pub fn change_glyphs(img: &RgbImage, e: &Entry) -> (Vec<Glyph>, Option<i32>) {
    let x1 = HX1.min(name_right(img, e) + 3);
    let y0 = e.sr.1;
    let gs = plane(img, HX0, y0, x1, y0 + 30, ink::green_score);
    let rs = plane(img, HX0, y0, x1, y0 + 30, ink::red_score);
    // one or two small digits light little of the window: take the peak
    // from the top half percent
    let (pg, pr) = (gs.quantile(CHANGE_PEAK), rs.quantile(CHANGE_PEAK));
    if pg.max(pr) < 25.0 {
        return (Vec::new(), None);
    }
    let (score, sign) = if pg > pr { (gs, 1) } else { (rs, -1) };
    let ink = drop_short_columns(&adaptive(&score, CHANGE_PEAK, 25.0), 8);
    let gl = split_wide(glyphs(&ink, HX0, y0, 1.0, 4), 17.0);
    (cluster_right(gl, 10), Some(sign))
}

const CHANGE_PEAK: f32 = 0.995;

const TY0: u32 = 1180;

/// The tag words under the bar, left to right. Each tag is a coloured icon
/// disc (26 to 46 px wide) followed by a white word.
pub fn tag_words(img: &RgbImage) -> Vec<Glyph> {
    let (x0, x1) = (560u32, 1960u32);
    let disc = plane(img, x0, TY0 + 5, x1, TY0 + 40, |c| {
        ink::green(c).max(ink::red(c))
    });
    let icons: Vec<(usize, usize)> = runs(&disc.col_sums(), 2.0, 1)
        .into_iter()
        .filter(|(a, b)| (26..=46).contains(&(b - a)))
        .collect();
    let w = plane(img, x0, TY0 + 9, x1, TY0 + 34, ink::white);
    let mut out = Vec::new();
    for (k, &(_, b)) in icons.iter().enumerate() {
        let start = b + 4;
        let end = icons
            .get(k + 1)
            .map_or((b + 420).min(w.w), |n| n.0.saturating_sub(4));
        if start >= end {
            continue;
        }
        let part = w.cols(start, end);
        if let Some((p, q)) = first_cluster(&runs(&part.col_sums(), 0.5, 1), 20)
            && let Some(gl) = tight_glyph(&part.cols(p, q), x0 + (start + p) as u32, TY0 + 9)
        {
            out.push(gl);
        }
    }
    out
}

/// The mode banner text (top right), up to the timer.
pub fn banner_word(img: &RgbImage) -> Option<Glyph> {
    let (x0, y0) = (2056, 136);
    let w = plane(img, x0, y0, 2430, 186, ink::white);
    let (a, b) = first_cluster(&runs(&w.col_sums(), 0.5, 1), 30)?;
    tight_glyph(&w.cols(a, b), x0 + a as u32, y0)
}

/// The tier word and the division digit under the rank emblem. The text
/// line is the lowest band of 30 to 60 rows with over 40 white pixels per
/// row; word and digit split at the widest gap.
pub fn rank_text(img: &RgbImage) -> Option<(Glyph, Glyph)> {
    let (x0, y0) = (1150, 820);
    let w = plane(img, x0, y0, 1420, 1020, ink::white);
    let (a, b) = runs(&w.row_counts(ON), 40.0, 3)
        .into_iter()
        .rev()
        .find(|(a, b)| (30..=60).contains(&(b - a)))?;
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
