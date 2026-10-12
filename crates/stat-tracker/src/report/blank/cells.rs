//! Digit cells of the measured rows.
//!
//! A number is kept only where the reader copy ([`super::reader`]) reads one
//! well-formed number in the cell and every glyph of it passes every gate
//! below; a row keeps its numbers only when all six of its cells do. Kept
//! are the glyphs' binarised pixels and the part of their anti-aliased rim
//! that blends glyph colour into the cell's own background, on one flat cell
//! colour that equals no source pixel. Everything else in a row stays fill.
//!
//! Gates (the evidence for each threshold is in `docs/report-bundle.md`
//! section 4, measured on the member's 34 real boards at 1664 and 1248 px):
//! * board: digit height, white level and column centres agree with the
//!   measured table; letter templates loaded; scale is a measured one;
//! * row: saturated team tint (its own role bar, or the team's other bars);
//!   a row text band;
//! * cell: one ink group; saturated cell colour; no second ink layer; the
//!   reader's grammar for the field and none of its sanity flags; at most two
//!   digits in E A D; no slanted cut; centred on the column's consensus; the
//!   board's digit height on the row's baseline; all ink on the text line;
//! * glyph: per-class template score, calibrated margin and ink mass; not
//!   more letter-like than digit-like; digit height; commas below the line;
//!   digit spacing like the game's; brightness (a lone 0 is dim, the rest
//!   white).

use image::RgbImage;

use super::board::{Board, Row, pyround};
use super::letters;
use super::portrait::dilate;
use super::reader::{self, CANVAS, COMMA, DESC, Plane};

/// Digit analysis band inside a row, from its top (digits at +30..+44).
const BAND: (f64, f64) = (22.0, 52.0);
/// Board digit height (measured 13).
const DIGIT_H: (f64, f64) = (11.0, 15.5);
/// Per-class floors for digit pieces 0-9: best template correlation, and
/// calibrated margin `(best - second) / typ`. Set just under the lowest
/// value any real glyph reached (score - 0.04, margin - 0.10); the 1080p '7'
/// reads with a near-zero margin against '1' on real boards.
const ACC_SCORE_1440: [f64; 10] = [0.81, 0.79, 0.84, 0.81, 0.845, 0.845, 0.82, 0.84, 0.84, 0.82];
const ACC_SCORE_1080: [f64; 10] = [
    0.76, 0.80, 0.80, 0.835, 0.83, 0.835, 0.845, 0.70, 0.805, 0.865,
];
const ACC_MARGIN_1440: [f64; 10] = [0.56, 0.30, 0.51, 0.39, 0.45, 0.51, 0.34, 0.37, 0.46, 0.56];
const ACC_MARGIN_1080: [f64; 10] = [0.40, 0.33, 0.31, 0.37, 0.38, 0.30, 0.43, 0.00, 0.52, 0.64];
/// Comma piece correlation.
const ACC_COMMA: f64 = 0.60;
/// Binarised ink of a digit piece over dh^2, per class: the real range
/// widened by 0.05 (a bold letter that passes the shape test carries more).
const MASS_1440: [(f64, f64); 10] = [
    (0.37, 0.67),
    (0.12, 0.33),
    (0.33, 0.61),
    (0.31, 0.56),
    (0.30, 0.57),
    (0.32, 0.52),
    (0.32, 0.58),
    (0.25, 0.42),
    (0.38, 0.66),
    (0.33, 0.59),
];
const MASS_1080: [(f64, f64); 10] = [
    (0.35, 0.68),
    (0.14, 0.42),
    (0.31, 0.57),
    (0.31, 0.58),
    (0.30, 0.62),
    (0.29, 0.58),
    (0.34, 0.62),
    (0.25, 0.46),
    (0.40, 0.69),
    (0.34, 0.62),
];
/// A letter template beating the digit's own score by this rejects the glyph.
const LETTER_DELTA_1440: f64 = 0.04;
const LETTER_DELTA_1080: f64 = 0.06;
/// Digit top vs the row's consensus top, and digit height vs the board's
/// (x s).
const TOP_TOL: f64 = 1.6;
const DH_TOL: f64 = 1.6;
/// A number's centre vs its column's board consensus: `2.5 s + 0.5` px
/// (appending one digit-shaped glyph moves it by about 5 px).
const CENTRE_TOL: f64 = 2.5;
const CENTRE_SLACK: f64 = 0.5;
/// Consensus column centre vs `R - OFF` (x s; measured 0 to 2.5 px).
const COL_TOL: f64 = 3.0;
/// Centre distance of consecutive glyphs (x s, plus slack px): digit to
/// digit (real 9.5-12.0) and digit to comma (real 6.5-8.5).
const ADV_DIGIT: (f64, f64) = (9.0, 12.5);
const ADV_COMMA: (f64, f64) = (6.0, 9.0);
const ADV_SLACK: f64 = 0.75;
/// Touching glyphs may be cut apart only at a thin joint: the lighter column
/// beside the cut holds at most this much ink, over the text height (real
/// touching digits: at most 0.385 at 1664 px and 0.40 at 1248 px over 731
/// cuts; a bold "B8" drawn on top joins at 0.46 and 0.50).
const CUT_INK_MAX: f64 = 0.45;
/// Non-zero cell peak vs the board white level (real >= 0.857), and the
/// range of a lone dim 0 (real 0.63-0.73).
const BRIGHT_MIN: f64 = 0.80;
const DIM_ZERO: (f64, f64) = (0.55, 1.05);
/// Board white level floor (90th percentile of non-zero cell peaks).
const WHITE_MIN: f64 = 150.0;
/// Team tint and cell colour saturation for a row to keep glyphs.
pub(super) const TINT_SAT_MIN: f64 = 0.30;
const CELL_SAT_MIN: f64 = 0.30;
/// Middling ink touching no glyph core (a second ink layer), per s^2.
const ISO_MID_MAX: f64 = 3.0;
/// Rim pixel: share of the way from its local background to the glyph
/// colour, and the most it may sit off the blend ray from the cell's smooth
/// background to the glyph colour.
const RIM_T: f64 = 0.12;
const RIM_OFF_RAY: f64 = 40.0;
/// Flat cell colour: searched within this L-inf distance of the median, at
/// more than 2 levels (some channel) from every source pixel of the cell.
const PAINT_REACH: i32 = 24;
const PAINT_SEP: i32 = 2;
/// Scales with fitted floors: the 1440p and 1080p crops (+-5%).
const NATIVE: [f64; 2] = [1.0, 756.0 / 1007.0];
const NATIVE_TOL: f64 = 0.05;

/// The 1440p or 1080p crop scale (+-5%): the only scales with measured
/// floors, for digits and portraits.
pub(super) fn native_scale(s: f64) -> bool {
    NATIVE.iter().any(|n| (s / n - 1.0).abs() <= NATIVE_TOL)
}

/// Which digit template set a board uses (as `templates_for`).
fn set_1440(h: u32) -> bool {
    h.abs_diff(1007) <= h.abs_diff(756)
}

/// One segmented and read stat cell.
struct CellInfo {
    /// Board pixel of the cell plane's (0, 0).
    x0: usize,
    y0: usize,
    /// Median colour of the band patch, truncated like the prototype.
    colour: [f64; 3],
    cell: reader::Cell,
    reading: reader::Reading,
    /// Board x of the picked group's centre.
    centre: f64,
    /// More than one ink group in the window.
    extra: bool,
}

impl CellInfo {
    /// The grammatical text of a single-group cell.
    fn text(&self) -> Option<&str> {
        (!self.extra && self.reading.grammar && !self.reading.text.is_empty())
            .then_some(self.reading.text.as_str())
    }
}

/// A row's cells as read, before the gates.
struct RowCells {
    cells: [Option<CellInfo>; 6],
    /// Row consensus text band `(top, dh)` in cell rows.
    band: Option<(usize, usize)>,
}

/// What a kept row copies: per cell its window, a flat colour and the glyph
/// pixels (board coordinates).
#[derive(Debug, Clone)]
pub(super) struct KeptRow {
    pub(super) cells: Vec<KeptCell>,
}

#[derive(Debug, Clone)]
pub(super) struct KeptCell {
    pub(super) x0: u32,
    pub(super) x1: u32,
    pub(super) paint: [u8; 3],
    pub(super) value: u32,
    pub(super) pixels: Vec<(u32, u32)>,
}

/// The digit verdict for every row of the grid: `Ok` with what to copy, or
/// why the row keeps no glyph.
pub(super) fn digit_rows(
    img: &RgbImage,
    b: &Board,
    tint_sat: &[Option<f64>],
) -> Vec<Result<KeptRow, String>> {
    let rows = &b.rows;
    let (w, h) = img.dimensions();
    let s = b.s;
    if !native_scale(s) {
        let why = format!("scale {s:.3} has no fitted floors");
        return rows.iter().map(|_| Err(why.clone())).collect();
    }
    let key1440 = set_1440(h);
    let Some(lt) = letters::letters(key1440) else {
        return rows
            .iter()
            .map(|_| Err("letter templates missing".into()))
            .collect();
    };
    let tpl = reader::templates_for(h);
    let read = read_rows(img, b, tint_sat);
    // board calibration: white level, digit height, column centres
    let all: Vec<&CellInfo> = read
        .iter()
        .flatten()
        .flat_map(|rc| rc.cells.iter().flatten())
        .collect();
    let mut peaks: Vec<f64> = all
        .iter()
        .filter(|c| c.text().is_some_and(|t| t != "0"))
        .map(|c| c.cell.peak)
        .collect();
    peaks.sort_by(f64::total_cmp);
    let white = if peaks.len() >= 3 {
        peaks[(peaks.len() - 1) * 9 / 10]
    } else {
        255.0
    };
    let mut dhs: Vec<usize> = all
        .iter()
        .filter(|c| c.text().is_some())
        .map(|c| c.cell.dh)
        .collect();
    dhs.sort_unstable();
    let board_dh = (!dhs.is_empty()).then(|| dhs[(dhs.len() - 1) / 2]);
    let centres: [Option<f64>; 6] = std::array::from_fn(|k| {
        let mut cs: Vec<f64> = read
            .iter()
            .flatten()
            .filter_map(|rc| rc.cells[k].as_ref())
            .filter(|c| c.text().is_some())
            .map(|c| c.centre)
            .collect();
        cs.sort_by(f64::total_cmp);
        (cs.len() >= 3).then(|| cs[(cs.len() - 1) / 2])
    });
    let mut board_why = None;
    match board_dh {
        Some(d) if (DIGIT_H.0 * s..=DIGIT_H.1 * s).contains(&(d as f64)) => {}
        _ => board_why = Some(format!("board digit height {board_dh:?}")),
    }
    if white < WHITE_MIN {
        board_why = Some(format!("board white {white:.0}"));
    }
    for (k, (centre, col)) in centres.iter().zip(&b.cols).enumerate() {
        if centre.is_none_or(|c| (c - col).abs() > COL_TOL * s) {
            board_why = Some(format!("column {k} centre {centre:?}"));
        }
    }
    let ctx = Ctx {
        s,
        key1440,
        board_dh: board_dh.unwrap_or(0),
        white,
        centres,
        lt,
        tpl,
    };
    rows.iter()
        .enumerate()
        .map(|(ri, r)| {
            let Some(rc) = &read[ri] else {
                return Err(if r.verified {
                    "no team tint".into()
                } else {
                    "row not verified".into()
                });
            };
            if let Some(why) = &board_why {
                return Err(format!("board: {why}"));
            }
            if tint_sat[ri].is_some_and(|t| t < TINT_SAT_MIN) {
                return Err(format!(
                    "team tint near grey ({:.2})",
                    tint_sat[ri].unwrap_or(0.0)
                ));
            }
            let Some(band) = rc.band else {
                return Err("no row text band".into());
            };
            let mut kept = Vec::with_capacity(6);
            for (k, c) in rc.cells.iter().enumerate() {
                let Some(c) = c else {
                    return Err(format!("cell {k}: empty"));
                };
                accept(c, k, band, &ctx).map_err(|why| format!("cell {k}: {why}"))?;
                kept.push(kept_cell(img, c, r, b.wins[k], w));
            }
            Ok(KeptRow { cells: kept })
        })
        .collect()
}

/// Segment and read every verified row with a team tint.
fn read_rows(img: &RgbImage, b: &Board, tint_sat: &[Option<f64>]) -> Vec<Option<RowCells>> {
    let rows = &b.rows;
    let h = img.height();
    let s = b.s;
    let tpl = reader::templates_for(h);
    let board_img = image::DynamicImage::ImageRgb8(img.clone());
    let n = reader::neutral_ink(&board_img);
    let mut read: Vec<Option<RowCells>> = Vec::with_capacity(rows.len());
    for (ri, r) in rows.iter().enumerate() {
        if !r.verified || tint_sat[ri].is_none() {
            read.push(None);
            continue;
        }
        let y0 = (i64::from(r.top) + pyround(BAND.0 * s)).max(0) as usize;
        let y1 = ((i64::from(r.top) + pyround(BAND.1 * s)).max(0) as usize).min(h as usize);
        let mut segs: Vec<Option<reader::Segment>> = b
            .wins
            .iter()
            .map(|&win| reader::segment_cell(&n, (y0, y1), win, 0))
            .collect();
        let band = reader::row_text_band(&segs);
        let colours: Vec<[f64; 3]> = b
            .wins
            .iter()
            .map(|&(x0, x1)| patch_median(img, x0, x1, y0, y1))
            .collect();
        let mut cells: [Option<CellInfo>; 6] = Default::default();
        for k in 0..6 {
            let Some(mut seg) = segs[k].take() else {
                continue;
            };
            reader::apply_row_band(&mut seg, band);
            let extra = seg.groups > 1;
            let gx0 = seg.runs.first().map_or(0, |r| r.0);
            let gx1 = seg.runs.last().map_or(0, |r| r.1);
            let Some(cell) = reader::prepare_cell(seg) else {
                continue;
            };
            // dim only steers the reader's pair checks (confidence), never
            // a value or flag the gates use
            let reading = reader::read_cell(Some(&cell), tpl, k, false);
            cells[k] = Some(CellInfo {
                x0: b.wins[k].0,
                y0,
                colour: colours[k],
                centre: b.wins[k].0 as f64 + (gx0 + gx1) as f64 / 2.0,
                cell,
                reading,
                extra,
            });
        }
        read.push(Some(RowCells { cells, band }));
    }
    read
}

struct Ctx<'a> {
    s: f64,
    key1440: bool,
    board_dh: usize,
    white: f64,
    centres: [Option<f64>; 6],
    lt: &'a letters::Letters,
    tpl: &'a reader::Templates,
}

/// Per-channel median of a board patch, truncated to whole levels.
fn patch_median(img: &RgbImage, x0: usize, x1: usize, y0: usize, y1: usize) -> [f64; 3] {
    std::array::from_fn(|c| {
        let mut v: Vec<f64> = (y0..y1)
            .flat_map(|y| (x0..x1).map(move |x| (x, y)))
            .map(|(x, y)| f64::from(img.get_pixel(x as u32, y as u32).0[c]))
            .collect();
        super::board::median(&mut v).trunc()
    })
}

fn saturation(c: [f64; 3]) -> f64 {
    super::portrait::sat(c)
}

/// Middling ink (0.35..0.75) farther than `max(1, round(s))` px from any
/// core pixel (>= 0.85): a second ink layer.
fn iso_mid(ink: &Plane, s: f64) -> usize {
    let core: Vec<bool> = ink.d.iter().map(|&v| v >= 0.85).collect();
    let near = dilate(&core, ink.w, ink.h, pyround(s).max(1) as usize);
    ink.d
        .iter()
        .zip(&near)
        .filter(|&(&v, &nr)| (0.35..0.75).contains(&v) && !nr)
        .count()
}

/// Ink where touching glyphs were cut apart at column boundary `x`: the
/// lighter of the two columns beside it, over the text height.
fn touching_cut_ink(cell: &reader::Cell, x: usize) -> f64 {
    let a = cell.colsum.get(x.wrapping_sub(1)).copied().unwrap_or(0);
    let b = cell.colsum.get(x).copied().unwrap_or(0);
    f64::from(a.min(b)) / cell.dh as f64
}

/// Rows `[first, last + 1)` holding binarised ink in columns `[x0, x1)`.
fn ink_rows(cell: &reader::Cell, x0: usize, x1: usize) -> Option<(usize, usize)> {
    let w = cell.ink.w;
    let rows: Vec<usize> = (0..cell.ink.h)
        .filter(|&y| (x0..x1).any(|x| cell.b[y * w + x]))
        .collect();
    Some((*rows.first()?, *rows.last()? + 1))
}

fn accept(c: &CellInfo, k: usize, band: (usize, usize), x: &Ctx) -> Result<(), String> {
    let s = x.s;
    if c.extra {
        return Err("foreign: extra ink groups".into());
    }
    let Some(text) = c.text() else {
        return Err("unread".into());
    };
    if saturation(c.colour) < CELL_SAT_MIN {
        return Err("cell colour near grey".into());
    }
    if iso_mid(&c.cell.ink, s) as f64 > ISO_MID_MAX * (s * s).max(1.0) {
        return Err("second ink layer".into());
    }
    if c.reading.flagged {
        return Err("reader flags".into());
    }
    if k < 3 && text.len() > 2 {
        // the reader flags a 3-digit E/A/D as suspect; it is never kept
        return Err("three digits".into());
    }
    if c.reading.pieces.iter().any(|p| p.slanted) {
        return Err("slanted cut".into());
    }
    let Some(centre) = x.centres[k] else {
        return Err("no column centre".into());
    };
    if (c.centre - centre).abs() > CENTRE_TOL * s + CENTRE_SLACK {
        return Err(format!("off centre {:.1}", c.centre - centre));
    }
    let (top, dh) = (c.cell.top, c.cell.dh);
    if (dh as f64 - x.board_dh as f64).abs() > DH_TOL * s {
        return Err(format!("height {dh} vs {}", x.board_dh));
    }
    if (top as f64 - band.0 as f64).abs() > TOP_TOL * s {
        return Err("baseline".into());
    }
    let tol = pyround(TOP_TOL * s).max(1) as usize;
    let line_end = top + pyround(DESC * dh as f64) as usize + tol;
    let gx0 = c.cell.runs.first().map_or(0, |r| r.s);
    let gx1 = c.cell.runs.last().map_or(0, |r| r.e);
    let Some((r0, r1)) = ink_rows(&c.cell, gx0, gx1) else {
        return Err("no ink".into());
    };
    if r0 + tol < top || r1 > line_end {
        return Err("ink off the text line".into());
    }
    let (score_floor, margin_floor, mass, delta) = if x.key1440 {
        (
            &ACC_SCORE_1440,
            &ACC_MARGIN_1440,
            &MASS_1440,
            LETTER_DELTA_1440,
        )
    } else {
        (
            &ACC_SCORE_1080,
            &ACC_MARGIN_1080,
            &MASS_1080,
            LETTER_DELTA_1080,
        )
    };
    let mut ndig = 0;
    for p in &c.reading.pieces {
        let Some((e0, e1)) = ink_rows(&c.cell, p.x0, p.x1) else {
            return Err("empty piece".into());
        };
        if p.class == COMMA {
            if p.score < ACC_COMMA {
                return Err(format!("comma shape {:.2}", p.score));
            }
            if (e0 as f64) < top as f64 + 0.45 * dh as f64 || e1 > line_end {
                return Err("comma position".into());
            }
            continue;
        }
        ndig += 1;
        let cls = p.class;
        if p.score < score_floor[cls] {
            return Err(format!("shape {cls} {:.2}", p.score));
        }
        let ink: u32 = c.cell.colsum[p.x0..p.x1].iter().sum();
        let m = f64::from(ink) / (dh * dh) as f64;
        if !(mass[cls].0..=mass[cls].1).contains(&m) {
            return Err(format!("ink mass {cls} {m:.2}"));
        }
        let canvas = &c.cell.v[p.id * CANVAS..(p.id + 1) * CANVAS];
        let lb = x.lt.best(canvas);
        if lb > p.score + delta {
            return Err(format!("letter-like {cls} ({lb:.2} vs {:.2})", p.score));
        }
        let margin = if p.score > -1.0 {
            (p.score - p.second) / x.tpl.typ[cls]
        } else {
            -1.0
        };
        if margin < margin_floor[cls] {
            return Err(format!("margin {cls} {margin:.2}"));
        }
        if e0.abs_diff(top) > tol || e1.abs_diff(top + dh) > tol {
            return Err(format!("glyph {cls} not digit height"));
        }
    }
    if ndig == 0 {
        return Err("no digits".into());
    }
    for pair in c.reading.pieces.windows(2) {
        if pair[0].x1 == pair[1].x0 && touching_cut_ink(&c.cell, pair[1].x0) > CUT_INK_MAX {
            return Err("glyphs joined by a thick cut".into());
        }
        let d = (pair[1].x0 + pair[1].x1) as f64 / 2.0 - (pair[0].x0 + pair[0].x1) as f64 / 2.0;
        let (lo, hi) = if pair[0].class == COMMA || pair[1].class == COMMA {
            ADV_COMMA
        } else {
            ADV_DIGIT
        };
        if d < lo * s - ADV_SLACK || d > hi * s + ADV_SLACK {
            return Err(format!("glyph spacing {d:.1}"));
        }
    }
    let rel = c.cell.peak / x.white.max(1.0);
    if text == "0" {
        if !(DIM_ZERO.0..=DIM_ZERO.1).contains(&rel) {
            return Err(format!("zero brightness {rel:.2}"));
        }
    } else if rel < BRIGHT_MIN {
        return Err(format!("brightness {rel:.2}"));
    }
    Ok(())
}

/// The pixels and paint of an accepted cell.
fn kept_cell(img: &RgbImage, c: &CellInfo, row: &Row, win: (usize, usize), w: u32) -> KeptCell {
    let (cw, ch) = (c.cell.ink.w, c.cell.ink.h);
    let mut core = vec![false; cw * ch];
    for p in &c.reading.pieces {
        for y in 0..ch {
            for x in p.x0..p.x1 {
                core[y * cw + x] |= c.cell.b[y * cw + x];
            }
        }
    }
    let d1 = dilate(&core, cw, ch, 1);
    let d2 = dilate(&core, cw, ch, 2);
    let px = |x: usize, y: usize| -> [f64; 3] {
        img.get_pixel((c.x0 + x) as u32, (c.y0 + y) as u32)
            .0
            .map(f64::from)
    };
    // glyph colour: the mean (rim rule) and 90th percentile (blend ray) of
    // the core
    let core_px: Vec<[f64; 3]> = (0..ch)
        .flat_map(|y| (0..cw).map(move |x| (x, y)))
        .filter(|&(x, y)| core[y * cw + x])
        .map(|(x, y)| px(x, y))
        .collect();
    let mean: [f64; 3] = std::array::from_fn(|i| {
        if core_px.is_empty() {
            255.0
        } else {
            core_px.iter().map(|p| p[i]).sum::<f64>() / core_px.len() as f64
        }
    });
    let p90: [f64; 3] = std::array::from_fn(|i| {
        let mut v: Vec<f64> = core_px.iter().map(|p| p[i]).collect();
        v.sort_by(f64::total_cmp);
        percentile(&v, 90.0)
    });
    let plane = plane_fit(&px, cw, ch, &d2);
    let mut pixels = Vec::new();
    for y in 0..ch {
        for x in 0..cw {
            let i = y * cw + x;
            let keep = if core[i] {
                true
            } else if d1[i] {
                rim_ok(&px, cw, ch, x, y, &d1, &d2, c.colour, mean)
                    && blend_ok(px(x, y), plane(x, y), p90)
            } else {
                false
            };
            if keep {
                pixels.push(((c.x0 + x) as u32, (c.y0 + y) as u32));
            }
        }
    }
    let x1 = (win.1 as u32).min(w);
    let paint = flat_colour(img, win.0 as u32, x1, row.top, row.bot, c.colour);
    KeptCell {
        x0: win.0 as u32,
        x1,
        paint,
        value: c.reading.value.unwrap_or(0),
        pixels,
    }
}

fn percentile(v: &[f64], q: f64) -> f64 {
    if v.is_empty() {
        return 255.0;
    }
    let idx = q / 100.0 * (v.len() - 1) as f64;
    let (lo, hi) = (idx.floor() as usize, idx.ceil() as usize);
    v[lo] + (v[hi] - v[lo]) * (idx - lo as f64)
}

/// Rim rule: the pixel lies at least [`RIM_T`] of the way from its own local
/// background (the mean of its 8-neighbours one step further out) to the
/// glyph colour.
#[allow(clippy::too_many_arguments)]
fn rim_ok(
    px: &dyn Fn(usize, usize) -> [f64; 3],
    cw: usize,
    ch: usize,
    x: usize,
    y: usize,
    d1: &[bool],
    d2: &[bool],
    cell_colour: [f64; 3],
    glyph: [f64; 3],
) -> bool {
    let mut acc = [0f64; 3];
    let mut n = 0usize;
    for dy in -1i64..=1 {
        for dx in -1i64..=1 {
            if dx == 0 && dy == 0 {
                continue;
            }
            let (nx, ny) = (x as i64 + dx, y as i64 + dy);
            if nx < 0 || ny < 0 || nx >= cw as i64 || ny >= ch as i64 {
                continue;
            }
            let j = ny as usize * cw + nx as usize;
            if d2[j] && !d1[j] {
                let p = px(nx as usize, ny as usize);
                for c in 0..3 {
                    acc[c] += p[c];
                }
                n += 1;
            }
        }
    }
    let bg = if n > 0 {
        acc.map(|v| v / n as f64)
    } else {
        cell_colour
    };
    let p = px(x, y);
    let d: [f64; 3] = std::array::from_fn(|c| glyph[c] - bg[c]);
    let dd: f64 = d.iter().map(|v| v * v).sum::<f64>().max(1.0);
    let t: f64 = (0..3).map(|c| (p[c] - bg[c]) * d[c]).sum::<f64>() / dd;
    t >= RIM_T
}

/// Blend-ray rule: the pixel sits within [`RIM_OFF_RAY`] (max channel) of
/// the ray from the cell's smooth background to the glyph colour.
fn blend_ok(p: [f64; 3], bg: [f64; 3], glyph: [f64; 3]) -> bool {
    off_ray(p, bg, glyph) <= RIM_OFF_RAY
}

/// Max-channel distance of `p` from the ray `bg -> glyph` (infinite when the
/// two colours are the same).
fn off_ray(p: [f64; 3], bg: [f64; 3], glyph: [f64; 3]) -> f64 {
    let d: [f64; 3] = std::array::from_fn(|c| glyph[c] - bg[c]);
    let dd: f64 = d.iter().map(|v| v * v).sum::<f64>();
    if dd < 1.0 {
        return f64::INFINITY;
    }
    let t = ((0..3).map(|c| (p[c] - bg[c]) * d[c]).sum::<f64>() / dd).clamp(0.0, 1.6);
    (0..3)
        .map(|c| (p[c] - (bg[c] + t * d[c])).abs())
        .fold(0.0, f64::max)
}

/// Robust per-channel plane `a + b x + c y` over the cell pixels outside
/// `mask` (one 2.5-MAD trimming pass): the smooth part of the background.
fn plane_fit(
    px: &dyn Fn(usize, usize) -> [f64; 3],
    cw: usize,
    ch: usize,
    mask: &[bool],
) -> impl Fn(usize, usize) -> [f64; 3] {
    let pts: Vec<(f64, f64, [f64; 3])> = (0..ch)
        .flat_map(|y| (0..cw).map(move |x| (x, y)))
        .filter(|&(x, y)| !mask[y * cw + x])
        .map(|(x, y)| (x as f64, y as f64, px(x, y)))
        .collect();
    let coef: [[f64; 3]; 3] = std::array::from_fn(|c| {
        let all: Vec<(f64, f64, f64)> = pts.iter().map(|&(x, y, p)| (x, y, p[c])).collect();
        let Some(k) = lstsq(&all) else {
            return [median_of(&all), 0.0, 0.0];
        };
        let res: Vec<f64> = all
            .iter()
            .map(|&(x, y, v)| v - (k[0] + k[1] * x + k[2] * y))
            .collect();
        let mut r = res.clone();
        let med = super::board::median(&mut r);
        let mut dev: Vec<f64> = res.iter().map(|v| (v - med).abs()).collect();
        let mad = super::board::median(&mut dev) + 1e-6;
        let ok: Vec<(f64, f64, f64)> = all
            .iter()
            .zip(&res)
            .filter(|&(_, r)| r.abs() <= 2.5 * 1.4826 * mad + 2.0)
            .map(|(&p, _)| p)
            .collect();
        if ok.len() >= 6 {
            lstsq(&ok).unwrap_or(k)
        } else {
            k
        }
    });
    move |x: usize, y: usize| {
        std::array::from_fn(|c| coef[c][0] + coef[c][1] * x as f64 + coef[c][2] * y as f64)
    }
}

fn median_of(v: &[(f64, f64, f64)]) -> f64 {
    let mut m: Vec<f64> = v.iter().map(|p| p.2).collect();
    let r = super::board::median(&mut m);
    if r.is_nan() { 0.0 } else { r }
}

/// Least squares `v = a + b x + c y` via the normal equations, summed in a
/// fixed order.
fn lstsq(p: &[(f64, f64, f64)]) -> Option<[f64; 3]> {
    if p.len() < 3 {
        return None;
    }
    let mut m = [[0f64; 4]; 3];
    for &(x, y, v) in p {
        let r = [1.0, x, y];
        for i in 0..3 {
            for j in 0..3 {
                m[i][j] += r[i] * r[j];
            }
            m[i][3] += r[i] * v;
        }
    }
    // Gaussian elimination with partial pivoting
    for col in 0..3 {
        let piv = (col..3).max_by(|&a, &b| m[a][col].abs().total_cmp(&m[b][col].abs()))?;
        if m[piv][col].abs() < 1e-9 {
            return None;
        }
        m.swap(col, piv);
        for row in 0..3 {
            if row != col {
                let f = m[row][col] / m[col][col];
                let pivot = m[col];
                for (v, p) in m[row].iter_mut().zip(pivot).skip(col) {
                    *v -= f * p;
                }
            }
        }
    }
    Some([m[0][3] / m[0][0], m[1][3] / m[1][1], m[2][3] / m[2][2]])
}

/// The flat colour of a kept cell: the colour nearest to the median (L-inf
/// shells, then squared distance, then channel order) that sits more than
/// [`PAINT_SEP`] levels, in some channel, from every source pixel of the
/// painted region; [`super::FILL`] when none is within reach. So no painted
/// pixel reproduces a source pixel.
fn flat_colour(img: &RgbImage, x0: u32, x1: u32, y0: u32, y1: u32, median: [f64; 3]) -> [u8; 3] {
    let m: [i32; 3] = median.map(|v| v.round().clamp(0.0, 255.0) as i32);
    let g = (2 * (PAINT_REACH + PAINT_SEP) + 1) as usize;
    let off = PAINT_REACH + PAINT_SEP;
    let mut occ = vec![false; g * g * g];
    let mut seen = std::collections::HashSet::new();
    for y in y0..y1.min(img.height()) {
        for x in x0..x1.min(img.width()) {
            let p = img.get_pixel(x, y).0.map(i32::from);
            if !seen.insert(p) {
                continue;
            }
            let o = [p[0] - m[0], p[1] - m[1], p[2] - m[2]];
            if o.iter().any(|v| v.abs() > off + PAINT_SEP) {
                continue;
            }
            for d0 in -PAINT_SEP..=PAINT_SEP {
                for d1 in -PAINT_SEP..=PAINT_SEP {
                    for d2 in -PAINT_SEP..=PAINT_SEP {
                        let q = [o[0] + d0 + off, o[1] + d1 + off, o[2] + d2 + off];
                        if q.iter().all(|&v| v >= 0 && v < g as i32) {
                            occ[(q[0] as usize * g + q[1] as usize) * g + q[2] as usize] = true;
                        }
                    }
                }
            }
        }
    }
    for &c in paint_offsets() {
        let v = [m[0] + c[0], m[1] + c[1], m[2] + c[2]];
        if v.iter().any(|&x| !(0..=255).contains(&x)) {
            continue;
        }
        let q = [c[0] + off, c[1] + off, c[2] + off];
        if !occ[(q[0] as usize * g + q[1] as usize) * g + q[2] as usize] {
            return v.map(|x| x as u8);
        }
    }
    super::FILL.0
}

/// Offsets around the median in search order: L-inf shell, then squared
/// distance, then channel order (deterministic).
fn paint_offsets() -> &'static [[i32; 3]] {
    static OFFSETS: std::sync::OnceLock<Vec<[i32; 3]>> = std::sync::OnceLock::new();
    OFFSETS.get_or_init(|| {
        let mut cand: Vec<[i32; 3]> = Vec::new();
        for a in -PAINT_REACH..=PAINT_REACH {
            for b in -PAINT_REACH..=PAINT_REACH {
                for c in -PAINT_REACH..=PAINT_REACH {
                    cand.push([a, b, c]);
                }
            }
        }
        cand.sort_by_key(|c| {
            let linf = c.iter().map(|v| v.abs()).max().unwrap_or(0);
            let sq: i32 = c.iter().map(|v| v * v).sum();
            (linf, sq, c[0], c[1], c[2])
        });
        cand
    })
}

/// Neutral ink of the painted output, for [`reads_back`].
pub(super) fn output_ink(out: &RgbImage) -> Plane {
    reader::neutral_ink(&image::DynamicImage::ImageRgb8(out.clone()))
}

/// Re-read a painted row from the output (`n` = [`output_ink`]) with the
/// same geometry: every kept cell must read its kept value again.
pub(super) fn reads_back(n: &Plane, b: &Board, row: &Row, kept: &KeptRow) -> bool {
    let (h, s) = (n.h as u32, b.s);
    let tpl = reader::templates_for(h);
    let y0 = (i64::from(row.top) + pyround(BAND.0 * s)).max(0) as usize;
    let y1 = ((i64::from(row.top) + pyround(BAND.1 * s)).max(0) as usize).min(h as usize);
    let mut segs: Vec<Option<reader::Segment>> = b
        .wins
        .iter()
        .map(|&win| reader::segment_cell(n, (y0, y1), win, 0))
        .collect();
    let band = reader::row_text_band(&segs);
    kept.cells.iter().enumerate().all(|(k, kc)| {
        let Some(mut seg) = segs[k].take() else {
            return false;
        };
        reader::apply_row_band(&mut seg, band);
        let Some(cell) = reader::prepare_cell(seg) else {
            return false;
        };
        reader::read_cell(Some(&cell), tpl, k, false).value == Some(kc.value)
    })
}

/// Every piece of every cell of the verified rows, for calibration (tests).
#[cfg(test)]
pub(super) fn piece_stats(img: &RgbImage, b: &Board) -> Vec<serde_json::Value> {
    let key1440 = set_1440(img.height());
    let lt = letters::letters(key1440).expect("letters");
    let tpl = reader::templates_for(img.height());
    let tint: Vec<Option<f64>> = b.rows.iter().map(|r| r.verified.then_some(1.0)).collect();
    let mut out = Vec::new();
    for (ri, rc) in read_rows(img, b, &tint).into_iter().enumerate() {
        let Some(rc) = rc else { continue };
        for (k, c) in rc.cells.iter().enumerate() {
            let Some(c) = c else { continue };
            let dh = c.cell.dh;
            let pieces: Vec<_> = c
                .reading
                .pieces
                .iter()
                .map(|p| {
                    let ink: u32 = c.cell.colsum[p.x0..p.x1].iter().sum();
                    let canvas = &c.cell.v[p.id * CANVAS..(p.id + 1) * CANVAS];
                    serde_json::json!({
                        "class": p.class, "score": p.score, "second": p.second,
                        "margin": if p.score > -1.0 { (p.score - p.second) / tpl.typ[p.class] } else { -1.0 },
                        "mass": f64::from(ink) / (dh * dh) as f64,
                        "letter": lt.best(canvas), "ar": (p.x1 - p.x0) as f64 / dh as f64,
                        "x0": p.x0, "x1": p.x1, "slanted": p.slanted,
                    })
                })
                .collect();
            let cuts: Vec<f64> = c
                .reading
                .pieces
                .windows(2)
                .filter(|w| w[0].x1 == w[1].x0)
                .map(|w| touching_cut_ink(&c.cell, w[1].x0))
                .collect();
            out.push(serde_json::json!({
                "cuts": cuts,
                "team": b.rows[ri].team, "idx": b.rows[ri].idx, "k": k,
                "text": c.reading.text, "grammar": c.reading.grammar, "flagged": c.reading.flagged,
                "extra": c.extra, "centre": c.centre, "dh": dh, "top": c.cell.top, "peak": c.cell.peak,
                "iso_mid": iso_mid(&c.cell.ink, b.s), "pieces": pieces,
            }));
        }
    }
    out
}

/// Every rim candidate of every accepted cell: board x, y, the local-blend
/// share `t`, the blend-ray offset, and whether it is kept (tests).
#[cfg(test)]
pub(super) fn rim_stats(img: &RgbImage, b: &Board) -> Vec<(u32, u32, f64, f64, bool)> {
    let h = img.height();
    let key1440 = set_1440(h);
    let tint: Vec<Option<f64>> = b.rows.iter().map(|r| r.verified.then_some(1.0)).collect();
    let read = read_rows(img, b, &tint);
    let lt = letters::letters(key1440).expect("letters");
    let tpl = reader::templates_for(h);
    let all: Vec<&CellInfo> = read
        .iter()
        .flatten()
        .flat_map(|rc| rc.cells.iter().flatten())
        .collect();
    let mut peaks: Vec<f64> = all
        .iter()
        .filter(|c| c.text().is_some_and(|t| t != "0"))
        .map(|c| c.cell.peak)
        .collect();
    peaks.sort_by(f64::total_cmp);
    let white = if peaks.len() >= 3 {
        peaks[(peaks.len() - 1) * 9 / 10]
    } else {
        255.0
    };
    let mut dhs: Vec<usize> = all
        .iter()
        .filter(|c| c.text().is_some())
        .map(|c| c.cell.dh)
        .collect();
    dhs.sort_unstable();
    let board_dh = if dhs.is_empty() {
        0
    } else {
        dhs[(dhs.len() - 1) / 2]
    };
    let centres: [Option<f64>; 6] = std::array::from_fn(|k| {
        let mut cs: Vec<f64> = read
            .iter()
            .flatten()
            .filter_map(|rc| rc.cells[k].as_ref())
            .filter(|c| c.text().is_some())
            .map(|c| c.centre)
            .collect();
        cs.sort_by(f64::total_cmp);
        (cs.len() >= 3).then(|| cs[(cs.len() - 1) / 2])
    });
    let ctx = Ctx {
        s: b.s,
        key1440,
        board_dh,
        white,
        centres,
        lt,
        tpl,
    };
    let mut out = Vec::new();
    for rc in read.iter().flatten() {
        let Some(band) = rc.band else { continue };
        for (k, c) in rc.cells.iter().enumerate() {
            let Some(c) = c else { continue };
            if accept(c, k, band, &ctx).is_err() {
                continue;
            }
            let (cw, ch) = (c.cell.ink.w, c.cell.ink.h);
            let mut core = vec![false; cw * ch];
            for p in &c.reading.pieces {
                for y in 0..ch {
                    for x in p.x0..p.x1 {
                        core[y * cw + x] |= c.cell.b[y * cw + x];
                    }
                }
            }
            let d1 = dilate(&core, cw, ch, 1);
            let d2 = dilate(&core, cw, ch, 2);
            let px = |x: usize, y: usize| -> [f64; 3] {
                img.get_pixel((c.x0 + x) as u32, (c.y0 + y) as u32)
                    .0
                    .map(f64::from)
            };
            let core_px: Vec<[f64; 3]> = (0..ch)
                .flat_map(|y| (0..cw).map(move |x| (x, y)))
                .filter(|&(x, y)| core[y * cw + x])
                .map(|(x, y)| px(x, y))
                .collect();
            let mean: [f64; 3] = std::array::from_fn(|i| {
                core_px.iter().map(|p| p[i]).sum::<f64>() / core_px.len().max(1) as f64
            });
            let p90: [f64; 3] = std::array::from_fn(|i| {
                let mut v: Vec<f64> = core_px.iter().map(|p| p[i]).collect();
                v.sort_by(f64::total_cmp);
                percentile(&v, 90.0)
            });
            let plane = plane_fit(&px, cw, ch, &d2);
            for y in 0..ch {
                for x in 0..cw {
                    let i = y * cw + x;
                    if core[i] || !d1[i] {
                        continue;
                    }
                    let rim = rim_ok(&px, cw, ch, x, y, &d1, &d2, c.colour, mean);
                    let off = off_ray(px(x, y), plane(x, y), p90);
                    // t of the rim rule, recomputed for the record
                    out.push((
                        (c.x0 + x) as u32,
                        (c.y0 + y) as u32,
                        if rim { 1.0 } else { 0.0 },
                        off,
                        rim && off <= RIM_OFF_RAY,
                    ));
                }
            }
        }
    }
    out
}
