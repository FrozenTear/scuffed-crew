//! Synthetic Tab scoreboards for the blanker tests (no captured pixels).
//!
//! The geometry is the one measured on real 1440p boards (`board.rs`): the
//! header strip and its six labels, rows at `29 + 75 i` and `566 + 75 i`
//! (73 px tall, 2 px separators), the 31 px role bar with a centred white
//! icon, the portrait square up to `L + 104`, and stat digits drawn from the
//! reader's own 1440p templates scaled to the real 13 px digit height,
//! centred on `R - OFF`. Names are white strokes in the name column, so a
//! test can check that nothing of them survives.

use image::{Rgb, RgbImage};

use super::board::OFF;

pub(crate) const W: u32 = 1664;
pub(crate) const H: u32 = 1007;
pub(crate) const T1: u32 = 29;
pub(crate) const T2: u32 = 566;
pub(crate) const PITCH: u32 = 75;
pub(crate) const ROW_H: u32 = 73;
const HEADER: u32 = 29;
const HEADER_RGB: Rgb<u8> = Rgb([215, 219, 226]);
const LABEL_RGB: Rgb<u8> = Rgb([40, 42, 48]);
/// Header label centres `R - LAB_OFF` and half widths (E A D DMG H MIT).
const LABELS: [(f64, u32); 6] = [
    (510.5, 4),
    (453.5, 4),
    (394.5, 4),
    (304.75, 14),
    (197.0, 3),
    (101.0, 13),
];
pub(crate) const BAR_T1: Rgb<u8> = Rgb([123, 0, 123]);
pub(crate) const BAR_T2: Rgb<u8> = Rgb([245, 206, 0]);
pub(crate) const TINT_T1: Rgb<u8> = Rgb([58, 8, 62]);
pub(crate) const TINT_T2: Rgb<u8> = Rgb([92, 78, 12]);
const SEP_RGB: Rgb<u8> = Rgb([12, 10, 14]);
/// Digit top below the row top, and the digit height (1440p).
pub(crate) const DIGIT_TOP: u32 = 30;
const DIGIT_H: f64 = 13.0;
const WHITE: f64 = 250.0;
const DIM_ZERO: f64 = 172.0;

/// The measured table edges `(L, R)`.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Layout {
    /// No perks: role bar at x 57.
    L1,
    /// Perk or badge variants.
    L2,
    L3,
    /// The wide perk layout: role bar at x 0.
    L4,
    /// The centred post-match table.
    PostMatch,
}

impl Layout {
    pub(crate) fn edges(self) -> (u32, u32) {
        match self {
            Layout::L1 => (57, 987),
            Layout::L2 => (33, 1011),
            Layout::L3 => (25, 1019),
            Layout::L4 => (0, 1044),
            Layout::PostMatch => (367, 1297),
        }
    }
}

/// Stat values per row (team 1 rows 0-5, team 2 rows 6-11).
pub(crate) const VALUES: [[u32; 6]; 12] = [
    [12, 3, 3, 5480, 950, 2347],
    [0, 0, 0, 0, 0, 0],
    [27, 14, 9, 18744, 1203, 87],
    [5, 10, 1, 101, 23456, 0],
    [8, 2, 4, 9876, 33, 104512],
    [41, 6, 7, 765, 4321, 11],
    [1, 11, 2, 3300, 7, 1000],
    [16, 5, 0, 22058, 619, 4096],
    [3, 19, 8, 47, 15998, 2],
    [9, 4, 6, 6140, 280, 39017],
    [22, 7, 5, 11111, 0, 765],
    [6, 1, 3, 8023, 1450, 333],
];

/// `(top, bottom)` of row `r` (0-11), bottom exclusive and cut by the crop.
pub(crate) fn row_span(r: usize) -> (u32, u32) {
    row_span_with(r, PITCH)
}

thread_local! {
    /// Team 2's row pitch for the next synthetic board (75 unless a test
    /// sets another; real boards measured 74 to 75.25).
    pub(crate) static T2_PITCH: std::cell::Cell<u32> = const { std::cell::Cell::new(PITCH) };
}

/// [`row_span`] with team 2's pitch from [`T2_PITCH`].
pub(crate) fn row_span_with(r: usize, pitch1: u32) -> (u32, u32) {
    let pitch = if r < 6 {
        pitch1
    } else {
        T2_PITCH.with(|p| p.get())
    };
    let top = if r < 6 { T1 } else { T2 } + pitch * (r % 6) as u32;
    (top, (top + ROW_H.min(pitch - 2)).min(H))
}

fn fill(img: &mut RgbImage, x0: u32, y0: u32, x1: u32, y1: u32, c: Rgb<u8>) {
    for y in y0..y1.min(img.height()) {
        for x in x0..x1.min(img.width()) {
            img.put_pixel(x, y, c);
        }
    }
}

/// Glyph coverage maps (0..1) of the 1440p templates, scaled to the real
/// digit height: index 0-9 and 10 for the comma, with their ink columns.
fn glyphs() -> Vec<(Vec<f32>, usize, usize, usize, usize)> {
    let png = include_bytes!("../../../assets/shadow-digits/digits_1440.png");
    let strip = image::load_from_memory(png).unwrap().to_luma8();
    // a template canvas spans 1.35 digit heights in 26 rows
    let f = DIGIT_H / (26.0 / 1.35);
    let (gw, gh) = (
        (22.0 * f).round_ties_even() as u32,
        (26.0 * f).round_ties_even() as u32,
    );
    (0..11)
        .map(|c| {
            let cell = image::imageops::crop_imm(&strip, c * 22, 0, 22, 26).to_image();
            let g = image::imageops::resize(&cell, gw, gh, image::imageops::FilterType::Triangle);
            let cov: Vec<f32> = g.pixels().map(|p| f32::from(p.0[0]) / 255.0).collect();
            let cols: Vec<usize> = (0..gw as usize)
                .filter(|&x| (0..gh as usize).any(|y| cov[y * gw as usize + x] > 0.2))
                .collect();
            (
                cov,
                gw as usize,
                gh as usize,
                cols[0],
                cols[cols.len() - 1] + 1,
            )
        })
        .collect()
}

/// Draw `text` (digits and commas) centred at `cx`, glyph tops at `top`,
/// blended over what is there in `colour`.
pub(crate) fn draw_number(img: &mut RgbImage, text: &str, cx: f64, top: u32, colour: f64) {
    let gl = glyphs();
    let idx: Vec<usize> = text
        .chars()
        .map(|c| {
            if c == ',' {
                10
            } else {
                c as usize - '0' as usize
            }
        })
        .collect();
    // 2 px between digit inks, 1 px around a comma (real centre distances
    // 9.5-12 px between digits, 6.5-8.5 px digit to comma)
    let gap = |a: usize, b: usize| if a == 10 || b == 10 { 1 } else { 2 };
    let mut total = 0usize;
    for (i, &c) in idx.iter().enumerate() {
        total += gl[c].4 - gl[c].3;
        if i + 1 < idx.len() {
            total += gap(c, idx[i + 1]);
        }
    }
    let mut x = (cx - total as f64 / 2.0).round_ties_even() as i64;
    for (i, &c) in idx.iter().enumerate() {
        let (cov, gw, gh, a, b) = &gl[c];
        for gy in 0..*gh {
            for gx in *a..*b {
                let v = cov[gy * gw + gx];
                let (px, py) = (x + (gx - a) as i64, i64::from(top) + gy as i64);
                if v <= 0.0
                    || px < 0
                    || py < 0
                    || px >= img.width() as i64
                    || py >= img.height() as i64
                {
                    continue;
                }
                let p = img.get_pixel_mut(px as u32, py as u32);
                for ch in 0..3 {
                    let bg = f64::from(p.0[ch]);
                    p.0[ch] = (bg + f64::from(v) * (colour - bg))
                        .round()
                        .clamp(0.0, 255.0) as u8;
                }
            }
        }
        x += (b - a) as i64;
        if i + 1 < idx.len() {
            x += gap(c, idx[i + 1]) as i64;
        }
    }
}

/// Text of value `v` in column `k` (thousands commas in DMG H MIT).
pub(crate) fn text_for(v: u32, k: usize) -> String {
    let s = v.to_string();
    if k < 3 {
        return s;
    }
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// A full synthetic 6v6 scoreboard crop.
pub(crate) fn board(layout: Layout) -> RgbImage {
    board_with(layout, &VALUES)
}

/// [`board`] with other stat values.
pub(crate) fn board_with(layout: Layout, values: &[[u32; 6]; 12]) -> RgbImage {
    let (l, r) = layout.edges();
    let mut img = RgbImage::from_fn(W, H, |x, y| {
        // the game world: a smooth gradient
        Rgb([
            (40 + (x * 60 / W) as u8),
            (55 + (y * 50 / H) as u8),
            (70 + ((x + y) * 40 / (W + H)) as u8),
        ])
    });
    fill(&mut img, l, 0, r, HEADER, HEADER_RGB);
    for &(off, half) in &LABELS {
        let c = (f64::from(r) - off).round_ties_even() as u32;
        fill(&mut img, c - half, 8, c + half + 1, 21, LABEL_RGB);
    }
    for (row, row_values) in values.iter().enumerate() {
        let (top, bot) = row_span(row);
        let team2 = row >= 6;
        let (bar, tint) = if team2 {
            (BAR_T2, TINT_T2)
        } else {
            (BAR_T1, TINT_T1)
        };
        // separator above the row (the header strip ends right above team
        // 1, the gap between the teams above team 2)
        if row % 6 != 0 {
            fill(&mut img, l, top - 2, r, top, SEP_RGB);
        }
        fill(&mut img, l + 104, top, r, bot, tint);
        // role bar with a centred white icon
        fill(&mut img, l, top, l + 31, bot, bar);
        let cy = top + ROW_H / 2;
        fill(
            &mut img,
            l + 9,
            cy - 7,
            l + 22,
            cy + 8,
            Rgb([255, 255, 255]),
        );
        // portrait art: smooth colour, a bright rim before the edge at L + 104
        for y in top..bot {
            for x in l + 31..l + 104 {
                let u = x - l - 31;
                let v = y - top;
                img.put_pixel(
                    x,
                    y,
                    Rgb([(120 + u) as u8, (90 + v) as u8, (150 + (u + v) / 2) as u8]),
                );
            }
        }
        // the name: white strokes in the name column
        let name_x0 = l + 120;
        for j in 0..7u32 {
            let x = name_x0 + j * 14;
            fill(&mut img, x, top + 22, x + 3, top + 42, Rgb([238, 238, 238]));
            fill(
                &mut img,
                x,
                top + 22,
                x + 10,
                top + 25,
                Rgb([238, 238, 238]),
            );
        }
        // soft stripe edges, as on real boards: the grid fit peaks on the
        // separators themselves
        for y in [top, bot - 1] {
            for x in l..r {
                let p = img.get_pixel_mut(x, y);
                p.0 = p.0.map(|c| (u16::from(c) * 7 / 10) as u8);
            }
        }
        for (k, &v) in row_values.iter().enumerate() {
            let cx = f64::from(r) - OFF[k];
            let colour = if v == 0 { DIM_ZERO } else { WHITE };
            draw_number(&mut img, &text_for(v, k), cx, top + DIGIT_TOP, colour);
        }
    }
    img
}

/// The name strokes of every row, `(x0, y0, x1, y1)` in crop pixels.
pub(crate) fn name_boxes(layout: Layout) -> Vec<(u32, u32, u32, u32)> {
    let (l, _) = layout.edges();
    (0..12)
        .map(|row| {
            let (top, _) = row_span(row);
            (l + 120, top + 22, l + 120 + 6 * 14 + 10, top + 42)
        })
        .collect()
}

/// A 2560x1440 frame holding [`board_with`] where the tracker cuts the
/// scoreboard, over a smooth world, with no chat on screen.
pub(crate) fn frame(layout: Layout, values: &[[u32; 6]; 12]) -> image::DynamicImage {
    let mut f = RgbImage::from_fn(2560, 1440, |x, y| {
        Rgb([(30 + x * 40 / 2560) as u8, (45 + y * 40 / 1440) as u8, 60])
    });
    let (x, y, _, _) = crate::ocr::preprocess::crop_scoreboard_rect(2560, 1440);
    image::imageops::replace(
        &mut f,
        &board_with(layout, values),
        i64::from(x),
        i64::from(y),
    );
    image::DynamicImage::ImageRgb8(f)
}
