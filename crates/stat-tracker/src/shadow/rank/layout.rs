//! Where each field is on the rank screen, and how its text is cut out.
//! Positions were measured on saved rank screens at 2560x1440; at 1080p
//! every box and most pixel thresholds are scaled by [`Scale`]
//! (`width/2560`, `height/1440`). A few small thresholds are fixed pixel
//! counts ([`ink::split_two`]'s width, [`ink::strip_minus`]'s bar size),
//! which is one reason [`Scale`] takes only the two sizes the reader is
//! tested at; the hero line brightness is scaled by height like the boxes.
//! The colour tests are in [`super::ink`].

use image::RgbImage;

use super::ink::{
    self, Glyph, ON, Plane, adaptive, adaptive_local, cluster_right, drop_short_columns,
    first_cluster, glyphs, plane, runs, split_wide, strip_minus, tight_glyph,
};
use super::{FRAME_H, FRAME_W};

/// Screen scale relative to the 2560x1440 reference layout.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Scale {
    pub sx: f32,
    pub sy: f32,
}

/// Frame heights the layout is tested at (1080p and 1440p). [`Scale`]
/// takes these within 2%, so a client area a few rows short still reads;
/// 720p, 4K and anything between are refused rather than read with
/// untested thresholds.
pub const HEIGHTS: [u32; 2] = [1080, 1440];

impl Scale {
    /// Accept 16:9 frames (aspect within 0.02) whose height is within 2% of
    /// one of [`HEIGHTS`].
    pub fn from_dims(w: u32, h: u32) -> Option<Self> {
        if w == 0 || !HEIGHTS.iter().any(|&t| h.abs_diff(t) * 50 <= t) {
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

/// The topmost hero entry: its name line and SR line, as row spans, and the
/// right edge of the name, which the SR and its change are right aligned to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry {
    pub name: (u32, u32),
    pub sr: (u32, u32),
    /// Right edge of the hero name (scaled to its local peak, so an overlay
    /// edge crossing it does not end it early), frame x.
    pub right: u32,
}

/// The "RANK PROGRESS:" label: present on every rank update screen.
pub fn gate_word(img: &RgbImage) -> Option<Glyph> {
    let s = Scale::from_img(img)?;
    let (x0, y0) = (s.x(840), s.y(1066));
    tight_glyph(&plane(img, x0, y0, s.x(975), s.y(1092), ink::white), x0, y0)
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
/// digit is cut off ([`strip_minus`]). The minus sits before the first digit,
/// which is the first cut, or the second when the chevrons come first
/// ("<<< -29%"), so only those two are checked.
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
        let gl = gl
            .into_iter()
            .enumerate()
            .map(|(k, g)| if k < 2 { strip_minus(g) } else { g })
            .collect();
        (gl, Some(-1))
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
/// (19 to 34 rows, at least 3 taller), 4 to 22 rows below it. `None` when
/// a line above that name line is SR tall: the top hero's own pair failed
/// the shape test, and the pair found is a later hero's.
pub fn first_entry(img: &RgbImage) -> Option<Entry> {
    let s = Scale::from_img(img)?;
    let ls = hero_lines(img, s);
    let (hn0, hn1) = (s.n(11), s.n(20));
    let (hs0, hs1) = (s.n(19), s.n(34));
    let taller = s.n(3);
    let (gap0, gap1) = (s.n(4), s.n(22));
    let (k, name, sr) = ls.windows(2).enumerate().find_map(|(k, w)| {
        let (n, sr) = (w[0], w[1]);
        let (hn, hs) = (n.1 - n.0, sr.1 - sr.0);
        let gap = sr.0 - n.1;
        ((hn0..=hn1).contains(&hn)
            && (hs0..=hs1).contains(&hs)
            && hs >= hn + taller
            && (gap0..=gap1).contains(&gap))
        .then_some((k, n, sr))
    })?;
    if ls[..k].iter().any(|l| (hs0..=hs1).contains(&(l.1 - l.0))) {
        return None;
    }
    Some(Entry {
        name,
        sr,
        right: name_right(img, s, name),
    })
}

fn band_ink(img: &RgbImage, s: Scale, y0: u32, y1: u32, x1: u32) -> Plane {
    let (hx0, _, _, _) = hero_box(s);
    adaptive(&neutral_plane(img, hx0, y0, x1, y1), 0.98, 25.0)
}

/// Right edge of the hero name. The SR digits and the change are right
/// aligned to it; placement pips further right are not part of the entry.
fn name_right(img: &RgbImage, s: Scale, name: (u32, u32)) -> u32 {
    let (hx0, _, hx1, _) = hero_box(s);
    // scaled to the local peak within 6 columns ([`adaptive_local`]): a
    // chat overlay edge can cross the name, and scaled as one the dimmed
    // part would vanish and cut the SR and change windows short
    let ink = adaptive_local(
        &neutral_plane(img, hx0, name.0, hx1, name.1),
        s.n(6) as usize,
        25.0,
    );
    first_cluster(&runs(&ink.col_sums(), 0.5, 1), s.n(14) as usize)
        .map_or(hx1, |(_, b)| hx0 + b as u32)
}

/// Whether the SR window ([`sr_glyphs`], which ends at the name's right
/// edge) may cut the SR short: the SR band has ink in the few columns just
/// right of it. That happens when the name ends early (a two word name
/// whose gap is wider than the name's word gap test), so the window would
/// end inside the digits and a 4 digit SR read as its first 3.
pub fn sr_cut_short(img: &RgbImage, e: &Entry) -> bool {
    let Some(s) = Scale::from_img(img) else {
        return false;
    };
    let (hx0, _, hx1, _) = hero_box(s);
    let x1 = hx1.min(e.right + s.n(3));
    let xr = hx1.min(x1 + s.n(14));
    if xr <= x1 {
        return false;
    }
    let y0 = e.sr.0.saturating_sub(s.n(2));
    let ink = band_ink(img, s, y0, e.sr.1 + s.n(2), xr);
    let cols = ink.col_sums();
    cols[(x1 - hx0) as usize..].iter().any(|c| *c >= s.f(2.0))
}

/// The hero SR glyphs. A rank icon left of the digits that is taller or
/// lower than they are is left out by shape: the digits share their top and
/// bottom rows (within 2 reference rows), so cuts are kept from the right
/// while they line up with the rightmost digit. An icon of digit height
/// stays and is matched as [`super::SR_ICON`] by the reader. Two touching
/// digits stay one cut here.
pub fn sr_glyphs(img: &RgbImage, e: &Entry) -> Vec<Glyph> {
    let Some(s) = Scale::from_img(img) else {
        return Vec::new();
    };
    let (hx0, _, hx1, _) = hero_box(s);
    let x1 = hx1.min(e.right + s.n(3));
    let y0 = e.sr.0.saturating_sub(s.n(2));
    let gl = glyphs(
        &band_ink(img, s, y0, e.sr.1 + s.n(2), x1),
        hx0,
        y0,
        1.0,
        s.n(4),
    );
    let mut gl = cluster_right(gl, (0.75 * (e.sr.1 - e.sr.0) as f32) as u32);
    let Some(&[_, top, _, bottom]) = gl.last().map(|g| &g.bbox) else {
        return gl;
    };
    let tol = s.n(2);
    let aligned = |g: &Glyph| g.bbox[1].abs_diff(top) <= tol && g.bbox[3].abs_diff(bottom) <= tol;
    let k = gl.iter().rposition(|g| !aligned(g)).map_or(0, |k| k + 1);
    gl.split_off(k)
}

/// The hero SR change digits under the SR line and their sign (green up,
/// red down). The caret and minus are dropped as short columns.
pub fn change_glyphs(img: &RgbImage, e: &Entry) -> (Vec<Glyph>, Option<i32>) {
    let Some(s) = Scale::from_img(img) else {
        return (Vec::new(), None);
    };
    let (hx0, _, hx1, _) = hero_box(s);
    let x1 = hx1.min(e.right + s.n(3));
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
        let end = icons
            .get(k + 1)
            .map_or((b + s.n(420) as usize).min(w.w), |n| {
                n.0.saturating_sub(s.n(4) as usize)
            });
        if start >= end {
            continue;
        }
        let part = w.cols(start, end);
        if let Some((p, q)) = first_cluster(&runs(&part.col_sums(), 0.5, 1), s.n(20) as usize)
            && let Some(gl) = tight_glyph(&part.cols(p, q), x0 + (start + p) as u32, ty0 + s.n(9))
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

/// Neutral level ([`ink::neutral`]) of faint ink: white text dimmed to
/// about a third still has more.
pub const FAINT: f32 = 20.0;

/// What the role band above the emblem shows.
#[derive(Debug, Clone, PartialEq)]
pub enum RoleLine {
    /// No text: an open queue screen.
    Empty,
    /// Some ink, but not a role line (glow, an overlay, a fading line).
    Unclear,
    /// The role icon (left of the role name).
    Icon(Glyph),
}

/// The role line (icon, then the role name) centred above the emblem, about
/// rows 284 to 321 at 1440p (the band stops at 326, above the glow of a tall
/// emblem). A line is a band of 18 to 46 rows with at least 12
/// white pixels per row; the icon is everything left of the widest column
/// gap. [`RoleLine::Empty`] needs fewer than 12 pixels per row even of
/// faint ink (neutral above [`FAINT`], well under the white threshold): a
/// role line dimmed by an overlay or still fading in is then
/// [`RoleLine::Unclear`], not an open queue. Open queue bands on the
/// saved screens hold no such pixel at all.
pub fn role_icon(img: &RgbImage) -> RoleLine {
    let Some(s) = Scale::from_img(img) else {
        return RoleLine::Unclear;
    };
    let (x0, y0) = (s.x(1060), s.y(236));
    let (x1, y1) = (s.x(1500), s.y(326));
    let w = plane(img, x0, y0, x1, y1, ink::white);
    let counts = w.row_counts(ON);
    let min_px = s.nx(12) as f32;
    if counts.iter().all(|&c| c < min_px) {
        let faint = neutral_plane(img, x0, y0, x1, y1).row_counts(FAINT);
        return if faint.iter().all(|&c| c < min_px) {
            RoleLine::Empty
        } else {
            RoleLine::Unclear
        };
    }
    let (h0, h1) = (s.n(18) as usize, s.n(46) as usize);
    let Some((a, b)) = runs(&counts, min_px, s.n(3) as usize)
        .into_iter()
        .find(|(a, b)| (h0..=h1).contains(&(b - a)))
    else {
        return RoleLine::Unclear;
    };
    let band = w.sub(0, a, w.w, b);
    let cr: Vec<(usize, usize)> = runs(&band.col_sums(), 0.5, 1)
        .into_iter()
        .filter(|&(p, q)| p > 0 && q < band.w)
        .collect();
    if cr.len() < 2 {
        return RoleLine::Unclear;
    }
    let Some(k) = (0..cr.len() - 1).max_by_key(|&k| (cr[k + 1].0 - cr[k].1, usize::MAX - k)) else {
        return RoleLine::Unclear;
    };
    tight_glyph(
        &band.cols(cr[0].0, cr[k].1),
        x0 + cr[0].0 as u32,
        y0 + a as u32,
    )
    .map_or(RoleLine::Unclear, RoleLine::Icon)
}

/// The division digits on the two small emblems at the ends of the progress
/// bar: the left one shows the division below the current one, the right
/// one the division above (see [`super::neighbours`]). Each is
/// a white digit about 13 rows tall and at most 12 wide in a dark hexagon,
/// centred near x 560 (left) and x 2000 (right), rows 1147 to 1162 at
/// 1440p (Master digits sit 3 rows lower). The window is the centre plus
/// or minus 8 columns, and only its tallest row run is cut: some tiers'
/// hexagons (Master) have chevron arms that reach down to row 1149 and
/// would merge into a wider, taller cut. `None` for a
/// side with no digit (the ends of the ladder show no emblem there).
pub fn emblem_digits(img: &RgbImage) -> (Option<Glyph>, Option<Glyph>) {
    let Some(s) = Scale::from_img(img) else {
        return (None, None);
    };
    let one = |xa: u32, xb: u32| -> Option<Glyph> {
        let (x0, y0) = (s.x(xa), s.y(1147));
        let w = plane(img, x0, y0, s.x(xb), s.y(1163), ink::white);
        // the tallest row run: the tip of a chevron above the digit is cut
        // off at the empty row between them
        let (a, b) = runs(&w.row_sums(), 0.5, 1)
            .into_iter()
            .max_by_key(|&(a, b)| (b - a, usize::MAX - a))?;
        let band = w.sub(0, a, w.w, b);
        let cx = (s.x(xa) + s.x(xb)) as f32 / 2.0;
        glyphs(&band, x0, y0 + a as u32, 0.5, s.n(8))
            .into_iter()
            .min_by(|a, b| {
                let d = |g: &Glyph| ((g.bbox[0] + g.bbox[2]) as f32 / 2.0 - cx).abs();
                d(a).total_cmp(&d(b))
            })
    };
    (one(552, 569), one(1992, 2009))
}
