//! Portrait cells: the role bar and the hero portrait of a row, copied whole
//! only when nothing is drawn over the bar.
//!
//! Chat, toasts, menus and stream overlays that reach a portrait from the
//! left must cross the opaque role bar. A real bar is one flat team colour
//! plus a white role icon centred in it, so a bar with a sharp foreign
//! stroke, white outside the icon zone, or an off-centre or lopsided icon
//! keeps the portrait out (the row's digits are judged on their own).

use image::RgbImage;

use super::board::{BAR_FULL, Fail, Row, median, pyround};

/// Share of the bar body that must sit within 40 of its median colour (real
/// bars >= 0.9).
const BAR_UNIFORM: f64 = 0.55;
/// Portrait cell end, `L + PORTRAIT_END * s` (measured on all four Tab
/// layouts and the post-match table).
const PORTRAIT_END: f64 = 104.0;
/// A sharp stroke: a channel more than this from the 5x5 median. Real bars
/// never show more than 1 such px outside the icon at 30 (816 rows at 1664
/// and 1248 px); chat drawn within 30 of the bar colour stays invisible here.
const SAL_THR: i32 = 30;
/// Role icon zone: x from `L`, y from the row centre (x s); measured
/// 5.3..24 and -12.7..15.
const ICON_X: (f64, f64) = (4.0, 26.0);
const ICON_Y: (f64, f64) = (-15.0, 17.0);
/// Mirror asymmetry of the icon's white mask (measured <= 0.34).
const ICON_ASYM: f64 = 0.45;
/// Strips beside the portrait cell: text that continues out of it. Right:
/// `[pe + 1, pe + 7)` (x s), white/grey or bright bar-coloured pixels (real
/// rows: 0). Left: the 8 px (x s) before the bar, white/grey pixels (real
/// rows: at most 1). More than `STRIP_MAX` drops the portrait.
const STRIP_W: (f64, f64) = (6.0, 8.0);
const STRIP_MAX: usize = 2;
/// White/grey: every channel >= 150 and spread <= 25% of the brightest.
const NEUTRAL_MIN: u8 = 150;
/// Bright bar-coloured: within 40 of the bar colour in every channel and at
/// least 0.8 of its brightest channel.
const BAR_LIKE: f64 = 40.0;
/// The portrait's right edge must show on the row: at most this share of its
/// inner pixel rows may have `sum |p(pe) - p(pe - 1)| < EDGE_WEAK` (real max
/// 0.52 over 816 rows; a menu box laid across the edge hides it on 0.66+).
const EDGE_WEAK: f64 = 20.0;
const EDGE_HIDDEN_MAX: f64 = 0.60;

/// Saturation of a colour: `(max - min) / max`.
pub(super) fn sat(c: [f64; 3]) -> f64 {
    let mx = c[0].max(c[1]).max(c[2]);
    let mn = c[0].min(c[1]).min(c[2]);
    (mx - mn) / mx.max(1.0)
}

/// The row's role bar colour: the median of its non-icon pixels, when one
/// opaque colour holds most of the bar. Text over part of the bar still
/// leaves the colour measurable; [`verdict`] judges the overlay.
pub(super) fn bar_colour(
    img: &RgbImage,
    l: u32,
    row: &Row,
    s: f64,
) -> Result<[f64; 3], &'static str> {
    let y0 = i64::from(row.top) + pyround(4.0 * s);
    let y1 = i64::from(row.bot) - pyround(4.0 * s);
    let x0 = i64::from(l) + pyround(3.0 * s);
    let x1 = i64::from(l) + pyround((BAR_FULL - 4.0) * s);
    if y1 - y0 < 10 || x1 - x0 < 5 {
        return Err("bar too small");
    }
    // a slice past the crop edge is cut short, as in the prototype
    let (x1, y1) = (
        x1.min(i64::from(img.width())),
        y1.min(i64::from(img.height())),
    );
    let mut body: Vec<[u8; 3]> = Vec::new();
    let mut total = 0usize;
    for y in y0 as u32..y1 as u32 {
        for x in x0 as u32..x1 as u32 {
            let p = img.get_pixel(x, y).0;
            total += 1;
            if p[0].min(p[1]).min(p[2]) < 200 {
                body.push(p);
            }
        }
    }
    if (body.len() as f64) < 0.3 * total as f64 {
        return Err("bar mostly white");
    }
    let med: [f64; 3] = std::array::from_fn(|c| {
        median(&mut body.iter().map(|p| f64::from(p[c])).collect::<Vec<_>>())
    });
    let near = body
        .iter()
        .filter(|p| (0..3).all(|c| (f64::from(p[c]) - med[c]).abs() <= 40.0))
        .count();
    if (near as f64) < BAR_UNIFORM * body.len() as f64 {
        return Err("bar colour ambiguous");
    }
    Ok(med)
}

/// Right edge of the portrait column: per row the strongest vertical edge
/// within `L + 104 s +- 4 s`; the board median must hold half the rows and
/// sit within 2.5 s of `L + 104 s`.
pub(super) fn portrait_edge(img: &RgbImage, l: u32, rows: &[&Row], s: f64) -> Result<u32, Fail> {
    let w = i64::from(img.width());
    let nom = f64::from(l) + PORTRAIT_END * s;
    let lo = pyround(nom - 4.0 * s).max(0);
    let hi = pyround(nom + 4.0 * s).min(w - 2);
    if hi <= lo {
        return Err(Fail("portrait edge outside the crop".into()));
    }
    let mut votes = Vec::new();
    for r in rows {
        let y0 = i64::from(r.top) + pyround(3.0 * s);
        let y1 = i64::from(r.bot) - pyround(3.0 * s);
        if y1 <= y0 {
            continue;
        }
        let mut best = (f64::NEG_INFINITY, 0i64);
        for x in lo..hi {
            let mut sum = 0f64;
            for y in y0..y1 {
                let a = img.get_pixel(x as u32, y as u32).0;
                let b = img.get_pixel(x as u32 + 1, y as u32).0;
                sum += (0..3)
                    .map(|c| (f64::from(b[c]) - f64::from(a[c])).abs())
                    .sum::<f64>();
            }
            let d = sum / (y1 - y0) as f64;
            if d > best.0 {
                best = (d, x);
            }
        }
        votes.push(best.1 + 1);
    }
    if votes.len() < 3 {
        return Err(Fail("portrait edge: too few rows".into()));
    }
    votes.sort_unstable();
    let med = votes[votes.len() / 2];
    let tol = pyround(s).max(1);
    let held = votes.iter().filter(|&&v| (v - med).abs() <= tol).count();
    if (held as f64) < 0.5 * votes.len() as f64 {
        return Err(Fail("portrait edge ambiguous".into()));
    }
    if (med as f64 - nom).abs() > 2.5 * s {
        return Err(Fail("portrait edge off".into()));
    }
    Ok(med as u32)
}

/// Sharp-stroke map of columns `[x0, x1)`: any channel more than
/// [`SAL_THR`] from its 5x5 median (edges replicated, like Pillow's
/// `MedianFilter(5)`). Row-major, `x1 - x0` wide.
pub(super) fn salient_map(img: &RgbImage, x0: u32, x1: u32) -> Vec<bool> {
    let (w, h) = img.dimensions();
    let x1 = x1.min(w);
    let sw = x1.saturating_sub(x0) as usize;
    let mut out = vec![false; sw * h as usize];
    let clamp = |v: i64, n: u32| v.clamp(0, i64::from(n) - 1) as u32;
    let mut win = [0u8; 25];
    for y in 0..h {
        for xi in 0..sw {
            let x = x0 + xi as u32;
            let p = img.get_pixel(x, y).0;
            let mut hit = false;
            for (c, &pc) in p.iter().enumerate() {
                let mut n = 0;
                for dy in -2i64..=2 {
                    for dx in -2i64..=2 {
                        // `Image.filter` runs on the crop [x0, x1), whose
                        // edges are replicated
                        let sx = clamp(i64::from(x) + dx, x1).max(x0);
                        let sy = clamp(i64::from(y) + dy, h);
                        win[n] = img.get_pixel(sx, sy).0[c];
                        n += 1;
                    }
                }
                win.sort_unstable();
                if (i32::from(pc) - i32::from(win[12])).abs() > SAL_THR {
                    hit = true;
                    break;
                }
            }
            out[y as usize * sw + xi] = hit;
        }
    }
    out
}

/// Binary dilation by a `(2r + 1)^2` square (outside the plane is empty).
pub(super) fn dilate(m: &[bool], w: usize, h: usize, r: usize) -> Vec<bool> {
    let mut out = m.to_vec();
    for y in 0..h {
        for x in 0..w {
            if !m[y * w + x] {
                continue;
            }
            for yy in y.saturating_sub(r)..(y + r + 1).min(h) {
                for xx in x.saturating_sub(r)..(x + r + 1).min(w) {
                    out[yy * w + xx] = true;
                }
            }
        }
    }
    out
}

/// Whether a row's portrait cell `[L, pe)` may be copied: nothing drawn over
/// its role bar. `sal` is [`salient_map`] of columns starting at `sal_x0`.
pub(super) fn verdict(
    img: &RgbImage,
    sal: &[bool],
    sal_x0: u32,
    sal_w: usize,
    l: u32,
    row: &Row,
    s: f64,
) -> Result<(), &'static str> {
    let (t, b) = (i64::from(row.top), i64::from(row.bot));
    let x0 = i64::from(l) + pyround(2.0 * s);
    let x1 = i64::from(l) + pyround(30.0 * s);
    let (y0, y1) = (t + 1, b - 1);
    if ((y1 - y0) as f64) < 20.0 * s {
        return Err("row too short");
    }
    if x0 < i64::from(sal_x0) || x1 > i64::from(sal_x0) + sal_w as i64 {
        return Err("bar outside the stroke map");
    }
    let (bw, bh) = ((x1 - x0) as usize, (y1 - y0) as usize);
    let white: Vec<bool> = (0..bh)
        .flat_map(|yy| (0..bw).map(move |xx| (xx, yy)))
        .map(|(xx, yy)| {
            let p = img
                .get_pixel((x0 as usize + xx) as u32, (y0 as usize + yy) as u32)
                .0;
            p[0].min(p[1]).min(p[2]) >= 200
        })
        .collect();
    let near = dilate(&white, bw, bh, pyround(2.0 * s).max(2) as usize);
    let strokes = (0..bh)
        .flat_map(|yy| (0..bw).map(move |xx| (xx, yy)))
        .filter(|&(xx, yy)| {
            let sx = (x0 - i64::from(sal_x0)) as usize + xx;
            let sy = y0 as usize + yy;
            sal[sy * sal_w + sx] && !near[yy * bw + xx]
        })
        .count();
    if strokes > 2 {
        return Err("strokes on the role bar");
    }
    let pts: Vec<(usize, usize)> = (0..bh)
        .flat_map(|yy| (0..bw).map(move |xx| (xx, yy)))
        .filter(|&(xx, yy)| white[yy * bw + xx])
        .collect();
    if !pts.is_empty() {
        let cy = (b - t) as f64 / 2.0 - 1.0;
        let xs = pts
            .iter()
            .map(|p| (p.0 as f64 + (x0 - i64::from(l)) as f64) / s);
        let ys = pts.iter().map(|p| (p.1 as f64 - cy) / s);
        let (xmin, xmax) = xs.fold((f64::INFINITY, f64::NEG_INFINITY), |a, v| {
            (a.0.min(v), a.1.max(v))
        });
        let (ymin, ymax) = ys.fold((f64::INFINITY, f64::NEG_INFINITY), |a, v| {
            (a.0.min(v), a.1.max(v))
        });
        if xmin < ICON_X.0 || xmax > ICON_X.1 || ymin < ICON_Y.0 || ymax > ICON_Y.1 {
            return Err("white outside the role icon zone");
        }
        let pxmin = pts.iter().map(|p| p.0).min().unwrap_or(0);
        let pxmax = pts.iter().map(|p| p.0).max().unwrap_or(0);
        let cx = (pxmin + pxmax) as f64 / 2.0;
        if (cx + x0 as f64 - (f64::from(l) + 15.5 * s)).abs() > 2.5 * s {
            return Err("role icon off centre");
        }
        let hits = pts
            .iter()
            .filter(|&&(px, py)| {
                let m = (2.0 * cx - px as f64).round_ties_even() as i64;
                m >= 0 && (m as usize) < bw && white[py * bw + m as usize]
            })
            .count();
        if 1.0 - hits as f64 / pts.len() as f64 > ICON_ASYM {
            return Err("role icon asymmetric");
        }
    }
    Ok(())
}

fn neutral_bright(p: [u8; 3]) -> bool {
    let mx = p[0].max(p[1]).max(p[2]);
    let mn = p[0].min(p[1]).min(p[2]);
    mn >= NEUTRAL_MIN && f64::from(mx - mn) <= 0.25 * f64::from(mx.max(1))
}

fn bar_like(p: [u8; 3], bar: [f64; 3]) -> bool {
    let top = bar[0].max(bar[1]).max(bar[2]);
    (0..3).all(|c| (f64::from(p[c]) - bar[c]).abs() <= BAR_LIKE)
        && f64::from(p[0].max(p[1]).max(p[2])) >= 0.8 * top
}

/// Text continuing out of the portrait cell to either side, and a portrait
/// edge something is drawn across.
pub(super) fn surroundings(
    img: &RgbImage,
    l: u32,
    pe: u32,
    row: &Row,
    s: f64,
    bar: [f64; 3],
) -> Result<(), &'static str> {
    let (w, h) = img.dimensions();
    let (y0, y1) = (row.top + 1, row.bot.saturating_sub(1).min(h));
    let rw = pyround(STRIP_W.0 * s).max(4) as u32;
    let right = (pe + 1..(pe + 1 + rw).min(w))
        .flat_map(|x| (y0..y1).map(move |y| (x, y)))
        .filter(|&(x, y)| {
            let p = img.get_pixel(x, y).0;
            neutral_bright(p) || bar_like(p, bar)
        })
        .count();
    if right > STRIP_MAX {
        return Err("text continues right of the portrait");
    }
    let lw = pyround(STRIP_W.1 * s) as u32;
    if l >= lw {
        let left = (l - lw..l.saturating_sub(1))
            .flat_map(|x| (y0..y1).map(move |y| (x, y)))
            .filter(|&(x, y)| neutral_bright(img.get_pixel(x, y).0))
            .count();
        if left > STRIP_MAX {
            return Err("text continues left of the bar");
        }
    }
    if pe == 0 || pe >= w {
        return Err("portrait edge outside the crop");
    }
    let (e0, e1) = (
        i64::from(row.top) + pyround(3.0 * s),
        (i64::from(row.bot) - pyround(3.0 * s)).min(i64::from(h)),
    );
    if e1 <= e0 {
        return Err("row too short");
    }
    let weak = (e0..e1)
        .filter(|&y| {
            let a = img.get_pixel(pe - 1, y as u32).0;
            let b = img.get_pixel(pe, y as u32).0;
            let d: i32 = (0..3)
                .map(|c| (i32::from(b[c]) - i32::from(a[c])).abs())
                .sum();
            f64::from(d) < EDGE_WEAK
        })
        .count();
    if weak as f64 > EDGE_HIDDEN_MAX * (e1 - e0) as f64 {
        return Err("portrait edge hidden");
    }
    Ok(())
}
