//! The vote counts under the ban vote screen's TEAM BAN SCORE strip: white
//! one or two digit numbers, or a grey dash under a box with no hero yet.
//! Read by matching each glyph against ten digit shapes kept here as small
//! grids. No OCR, no art: the grids are generic bold digit shapes, the mean
//! of the white digit masks seen in 34 saved frames, downscaled to
//! [`GLYPH_W`] by [`GLYPH_H`].
//!
//! A window goes in (a native rectangle of the frame) and a [`Number`]
//! comes out, or `None` when the window holds anything else: no white
//! digits and no dash, more than two glyphs, a glyph of an odd size, a
//! glyph no digit matches clearly, or a leading zero.

use image::RgbImage;

/// Glyph grid width and height.
pub(super) const GLYPH_W: usize = 10;
pub(super) const GLYPH_H: usize = 14;

/// The ten digits, `#` ink and `.` paper, [`GLYPH_W`] by [`GLYPH_H`].
/// A glyph is scaled to the grid height and centred, keeping its aspect,
/// so a narrow `1` stays narrow.
pub(super) const GLYPHS: [[&str; GLYPH_H]; 10] = [
    // 0
    [
        "....##....",
        "..######..",
        ".########.",
        ".##...###.",
        "###....##.",
        "###....##.",
        "##.....###",
        "##.....###",
        "###....##.",
        "###....##.",
        ".##...###.",
        ".########.",
        "..######..",
        "...###....",
    ],
    // 1
    [
        "..#####...",
        "..#####...",
        "....###...",
        "....###...",
        "....###...",
        "....###...",
        "....###...",
        "....###...",
        "....###...",
        "....###...",
        "....###...",
        "....###...",
        "....###...",
        "....##....",
    ],
    // 2
    [
        "...###....",
        "..######..",
        ".########.",
        "###...###.",
        "###....##.",
        "......###.",
        "......###.",
        ".....###..",
        "....####..",
        "...####...",
        "...###....",
        "..#######.",
        ".########.",
        "#########.",
    ],
    // 3
    [
        "...###....",
        "..#####...",
        ".########.",
        ".##...###.",
        "......###.",
        "......##..",
        "....####..",
        "....#####.",
        "......###.",
        "......###.",
        "###...###.",
        "#########.",
        "..######..",
        "...####...",
    ],
    // 4
    [
        ".....####.",
        "....#####.",
        "....#####.",
        "...######.",
        "..#######.",
        "..#######.",
        ".###.####.",
        ".##..####.",
        ".#########",
        "##########",
        "##########",
        ".....####.",
        ".....####.",
        "......##..",
    ],
    // 5
    [
        "..#######.",
        "..#######.",
        "..##......",
        ".###......",
        ".######...",
        ".#######..",
        ".##.#####.",
        "......###.",
        "......####",
        "#.....####",
        "##....###.",
        "########..",
        "#######...",
        "...###....",
    ],
    // 6
    [
        "....###...",
        "....###...",
        "...###....",
        "..####....",
        "..######..",
        ".#######..",
        ".########.",
        ".##....##.",
        "###....###",
        "###....###",
        ".##....##.",
        ".########.",
        "..######..",
        "....##....",
    ],
    // 7
    [
        "##########",
        "#########.",
        ".....####.",
        ".....###..",
        "....####..",
        "....###...",
        "...####...",
        "...###....",
        "..###.....",
        "..###.....",
        ".###......",
        ".##.......",
        "###.......",
        "###.......",
    ],
    // 8
    [
        "...###....",
        "..#####...",
        "########..",
        "###...##..",
        "###...##..",
        ".##..###..",
        ".######...",
        ".#######..",
        "###...##..",
        "###...###.",
        "###...###.",
        "########..",
        ".######...",
        "..####....",
    ],
    // 9
    [
        "....##....",
        "..######..",
        ".########.",
        ".##....##.",
        "###....###",
        "###....###",
        ".###..###.",
        ".########.",
        "..#######.",
        ".....###..",
        "....####..",
        "....###...",
        "...###....",
        "...###....",
    ],
];

/// Lowest correlation a glyph's best digit may have. Over the 82 numbers
/// in the 34 frames the lowest right score was 0.896 (0.875 with the
/// frame's own glyphs left out of the grids) and the highest wrong one
/// 0.604.
pub(super) const DIGIT_MIN_SCORE: f32 = 0.75;
/// Least lead of the best digit over the second (lowest seen: 0.316).
pub(super) const DIGIT_MIN_MARGIN: f32 = 0.15;
/// A window whose brightest pixel (by its darkest channel) is below this
/// holds no white digit.
const PEAK_MIN: u8 = 170;
/// Ink is a pixel whose darkest channel is above
/// `max(WHITE_FLOOR, peak * WHITE_SHARE)`, so
/// a frame dimmed by a fade still splits into the same glyphs.
const WHITE_FLOOR: u8 = 120;
const WHITE_SHARE: f64 = 0.7;
/// Glyph size limits in native pixels (seen: 5..13 wide, 15..17 tall).
const GLYPH_MIN_W: usize = 3;
const GLYPH_MAX_W: usize = 16;
const GLYPH_MIN_H: usize = 12;
const GLYPH_MAX_H: usize = 22;
/// The digits must sit this near the window centre, in pixels.
const CENTRE_SLACK: f32 = 6.0;
/// The dash: grey (each channel at least this, channels within
/// [`DASH_SPREAD`]), 4..10 wide, 2..5 tall, mostly filled.
const DASH_MIN: u8 = 80;
const DASH_SPREAD: u8 = 40;

/// What a number position shows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Number {
    /// A vote count, with how sure the reader is (0..1).
    Count { value: u32, confidence: f32 },
    /// The grey dash of a box with no hero yet.
    Dash,
}

/// Read the number in the native rectangle `x0..=x1`, `y0..=y1`. The
/// number is expected centred in it.
pub(super) fn read_number(img: &RgbImage, x0: u32, y0: u32, x1: u32, y1: u32) -> Option<Number> {
    if x1 < x0 || y1 < y0 || x1 >= img.width() || y1 >= img.height() {
        return None;
    }
    let (w, h) = ((x1 - x0 + 1) as usize, (y1 - y0 + 1) as usize);
    let mut lo = vec![0u8; w * h];
    let mut hi = vec![0u8; w * h];
    for y in 0..h {
        for x in 0..w {
            let [r, g, b] = img.get_pixel(x0 + x as u32, y0 + y as u32).0;
            lo[y * w + x] = r.min(g).min(b);
            hi[y * w + x] = r.max(g).max(b);
        }
    }
    let centre = (w as f32 - 1.0) / 2.0;
    let peak = lo.iter().copied().max().unwrap_or(0);
    if peak < PEAK_MIN {
        return dash(&lo, &hi, w, h, centre);
    }
    let cut = WHITE_FLOOR.max((peak as f64 * WHITE_SHARE) as u8);
    let ink: Vec<bool> = lo.iter().map(|&v| v > cut).collect();
    let glyphs = split(&ink, w, h);
    if glyphs.is_empty() || glyphs.len() > 2 {
        return None;
    }
    let first = glyphs[0];
    let last = glyphs[glyphs.len() - 1];
    let mid = (first.x0 + last.x1 - 1) as f32 / 2.0;
    if (mid - centre).abs() > CENTRE_SLACK {
        return None;
    }
    let mut value = 0u32;
    let mut confidence = 1.0f32;
    for g in &glyphs {
        let (gw, gh) = (g.x1 - g.x0, g.y1 - g.y0);
        if !(GLYPH_MIN_W..=GLYPH_MAX_W).contains(&gw) || !(GLYPH_MIN_H..=GLYPH_MAX_H).contains(&gh)
        {
            return None;
        }
        let cell = normalise(&ink, w, g);
        let (digit, score, margin) = best_digit(&cell);
        if score < DIGIT_MIN_SCORE || margin < DIGIT_MIN_MARGIN {
            return None;
        }
        value = value * 10 + digit;
        confidence = confidence
            .min(0.5 * (score / 0.95).min(1.0) + 0.5 * (margin / 0.3).min(1.0));
    }
    // "07" is not a count the screen shows.
    if glyphs.len() == 2 && value < 10 {
        return None;
    }
    let r3 = |v: f32| (v * 1000.0).round() / 1000.0;
    Some(Number::Count {
        value,
        confidence: r3(confidence.clamp(0.0, 1.0)),
    })
}

/// The grey dash, when the window holds exactly that.
fn dash(lo: &[u8], hi: &[u8], w: usize, h: usize, centre: f32) -> Option<Number> {
    let (mut min_x, mut max_x, mut min_y, mut max_y) = (usize::MAX, 0, usize::MAX, 0);
    let mut n = 0usize;
    for y in 0..h {
        for x in 0..w {
            let k = y * w + x;
            if lo[k] >= DASH_MIN && hi[k] - lo[k] <= DASH_SPREAD {
                n += 1;
                min_x = min_x.min(x);
                max_x = max_x.max(x);
                min_y = min_y.min(y);
                max_y = max_y.max(y);
            }
        }
    }
    if n == 0 {
        return None;
    }
    let (bw, bh) = (max_x - min_x + 1, max_y - min_y + 1);
    let mid = (min_x + max_x) as f32 / 2.0;
    let is_dash = (4..=10).contains(&bw)
        && (2..=5).contains(&bh)
        && n as f32 >= 0.6 * (bw * bh) as f32
        && (mid - centre).abs() <= CENTRE_SLACK;
    is_dash.then_some(Number::Dash)
}

/// One glyph's box in the window: columns `x0..x1`, rows `y0..y1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct GlyphBox {
    x0: usize,
    x1: usize,
    y0: usize,
    y1: usize,
}

/// Glyphs are runs of columns that hold ink, split by empty columns.
fn split(ink: &[bool], w: usize, h: usize) -> Vec<GlyphBox> {
    let col = |x: usize| (0..h).any(|y| ink[y * w + x]);
    let mut out = Vec::new();
    let mut x = 0;
    while x < w {
        if !col(x) {
            x += 1;
            continue;
        }
        let x0 = x;
        while x < w && col(x) {
            x += 1;
        }
        let row = |y: usize| (x0..x).any(|c| ink[y * w + c]);
        let y0 = (0..h).find(|&y| row(y)).unwrap_or(0);
        let y1 = (0..h).rev().find(|&y| row(y)).map_or(0, |y| y + 1);
        out.push(GlyphBox { x0, x1: x, y0, y1 });
    }
    out
}

/// The glyph scaled to [`GLYPH_H`] rows, its width scaled by the same
/// factor (at most [`GLYPH_W`]) and centred. Each cell is the ink share of
/// a 4 by 4 grid of samples.
fn normalise(ink: &[bool], stride: usize, g: &GlyphBox) -> [f32; GLYPH_W * GLYPH_H] {
    let (w, h) = (g.x1 - g.x0, g.y1 - g.y0);
    let gw = (((w * GLYPH_H) as f64 / h as f64 + 0.5) as usize).clamp(1, GLYPH_W);
    let off = (GLYPH_W - gw) / 2;
    let mut out = [0f32; GLYPH_W * GLYPH_H];
    for gy in 0..GLYPH_H {
        for gx in 0..gw {
            let mut s = 0u32;
            for sy in 0..4 {
                for sx in 0..4 {
                    let fy = (gy as f64 + (sy as f64 + 0.5) / 4.0) * h as f64 / GLYPH_H as f64;
                    let fx = (gx as f64 + (sx as f64 + 0.5) / 4.0) * w as f64 / gw as f64;
                    let (y, x) = (g.y0 + fy as usize, g.x0 + fx as usize);
                    s += ink[y * stride + x] as u32;
                }
            }
            out[gy * GLYPH_W + gx + off] = s as f32 / 16.0;
        }
    }
    out
}

/// A digit grid as numbers: 1 for ink, 0 for paper.
pub(super) fn glyph(d: usize) -> [f32; GLYPH_W * GLYPH_H] {
    let mut out = [0f32; GLYPH_W * GLYPH_H];
    for (y, row) in GLYPHS[d].iter().enumerate() {
        for (x, c) in row.bytes().enumerate().take(GLYPH_W) {
            if c == b'#' {
                out[y * GLYPH_W + x] = 1.0;
            }
        }
    }
    out
}

/// Pearson correlation of two grids; 0 when either is flat.
fn correlation(a: &[f32], b: &[f32]) -> f32 {
    let n = a.len() as f64;
    let ma = a.iter().map(|&v| v as f64).sum::<f64>() / n;
    let mb = b.iter().map(|&v| v as f64).sum::<f64>() / n;
    let (mut ab, mut aa, mut bb) = (0f64, 0f64, 0f64);
    for (&x, &y) in a.iter().zip(b) {
        let (x, y) = (x as f64 - ma, y as f64 - mb);
        ab += x * y;
        aa += x * x;
        bb += y * y;
    }
    let d = (aa * bb).sqrt();
    if d < 1e-9 { 0.0 } else { (ab / d) as f32 }
}

/// The best digit for a glyph cell, its score and its lead over the second.
fn best_digit(cell: &[f32]) -> (u32, f32, f32) {
    let mut scores: Vec<(f32, u32)> = (0..10)
        .map(|d| (correlation(cell, &glyph(d)), d as u32))
        .collect();
    scores.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    (scores[0].1, scores[0].0, scores[0].0 - scores[1].0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgb;

    /// Draw `text` (digits) from the grids, one native pixel per grid cell,
    /// white, two pixels apart, centred on `cx` with its top row at `top`.
    fn draw_number(img: &mut RgbImage, text: &str, cx: u32, top: u32) {
        let digits: Vec<usize> = text.bytes().map(|b| (b - b'0') as usize).collect();
        let width = digits.len() as u32 * (GLYPH_W as u32 + 2);
        let mut x = cx - width / 2;
        for d in digits {
            let g = glyph(d);
            for gy in 0..GLYPH_H {
                for gx in 0..GLYPH_W {
                    if g[gy * GLYPH_W + gx] > 0.5 {
                        img.put_pixel(x + gx as u32, top + gy as u32, Rgb([255, 255, 255]));
                    }
                }
            }
            x += GLYPH_W as u32 + 2;
        }
    }

    fn dark() -> RgbImage {
        RgbImage::from_pixel(80, 40, Rgb([9, 11, 19]))
    }

    #[test]
    fn every_grid_is_ten_by_fourteen_of_ink_and_paper() {
        for (d, rows) in GLYPHS.iter().enumerate() {
            for row in rows {
                assert_eq!(row.len(), GLYPH_W, "digit {d}");
                assert!(row.bytes().all(|c| c == b'#' || c == b'.'), "digit {d}");
            }
        }
    }

    #[test]
    fn each_grid_reads_as_itself_and_no_other_digit_comes_close() {
        for d in 0..10 {
            let (got, score, margin) = best_digit(&glyph(d));
            assert_eq!(got, d as u32);
            assert!(score > 0.99, "{d}: {score}");
            assert!(margin >= DIGIT_MIN_MARGIN, "{d}: {margin}");
        }
    }

    #[test]
    fn drawn_numbers_read_back() {
        for text in ["7", "12", "15", "28", "10", "3", "96", "40"] {
            let mut img = dark();
            draw_number(&mut img, text, 40, 10);
            match read_number(&img, 0, 4, 79, 34) {
                Some(Number::Count { value, confidence }) => {
                    assert_eq!(value.to_string(), text);
                    assert!(confidence > 0.5, "{text}: {confidence}");
                }
                other => panic!("{text}: {other:?}"),
            }
        }
    }

    #[test]
    fn a_grey_dash_is_a_dash_and_an_empty_window_is_none() {
        let mut img = dark();
        assert_eq!(read_number(&img, 0, 4, 79, 34), None);
        for y in 20..23 {
            for x in 37..43 {
                img.put_pixel(x, y, Rgb([130, 130, 131]));
            }
        }
        assert_eq!(read_number(&img, 0, 4, 79, 34), Some(Number::Dash));
    }

    #[test]
    fn three_glyphs_a_blob_or_an_off_centre_number_are_none() {
        let mut img = dark();
        draw_number(&mut img, "123", 40, 10);
        assert_eq!(read_number(&img, 0, 4, 79, 34), None);
        let mut img = dark();
        for y in 10..26 {
            for x in 34..46 {
                img.put_pixel(x, y, Rgb([255, 255, 255]));
            }
        }
        assert_eq!(read_number(&img, 0, 4, 79, 34), None, "a solid block");
        let mut img = dark();
        draw_number(&mut img, "7", 12, 10);
        assert_eq!(read_number(&img, 0, 4, 79, 34), None, "off centre");
    }
}
