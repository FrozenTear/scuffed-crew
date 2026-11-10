//! Synthetic loading-screen stand-ins made in code, no real screenshot:
//! the `ENTERING GAME` slab and `APPLYING UPDATE` bands, drawn natively at
//! any 16:9 size from one 640x360 layout. Shared by the loading check's
//! tests (`detect/loading_screen.rs`) and the pre-game saver's tests
//! (`daemon/tests_pregame_reveal.rs`, which loads this file by path).

use image::{Rgb, RgbImage};
use std::ops::Range;

/// The near-white of the stand-in glyphs.
pub(crate) const GLYPH: [u8; 3] = [240, 240, 240];

/// `v`, given on the 640x360 layout's axis of length `of`, on an axis of
/// length `size`.
fn on(v: u32, of: u32, size: u32) -> u32 {
    v * size / of
}

/// A `w` x `h` frame: black bars around the strip's vertical navy
/// gradient (rows 136 to 224 of 360, as on the loading screen).
pub(crate) fn navy_band(w: u32, h: u32) -> RgbImage {
    let mut img = RgbImage::from_pixel(w, h, Rgb([0, 0, 0]));
    let (y0, y1) = (on(136, 360, h), on(224, 360, h));
    for y in y0..y1 {
        let t = ((y - y0) * 88 / (y1 - y0)) as u8;
        let navy = Rgb([20 + t / 4, 28 + t / 5, 42 + t / 3]);
        for x in 0..w {
            img.put_pixel(x, y, navy);
        }
    }
    img
}

/// White glyph strokes over `xs` and `ys` (given on the 640x360 layout):
/// three lit columns in every seven of the layout.
pub(crate) fn glyphs(img: &mut RgbImage, xs: Range<u32>, ys: Range<u32>) {
    let (w, h) = img.dimensions();
    for y in on(ys.start, 360, h)..on(ys.end, 360, h) {
        for x in on(xs.start, 640, w)..on(xs.end, 640, w) {
            if (x * 640 / w) % 7 < 3 {
                img.put_pixel(x, y, Rgb(GLYPH));
            }
        }
    }
}

/// A near-white block over `xs` and `ys` (given on the 640x360 layout).
pub(crate) fn block(img: &mut RgbImage, xs: Range<u32>, ys: Range<u32>) {
    let (w, h) = img.dimensions();
    for y in on(ys.start, 360, h)..on(ys.end, 360, h) {
        for x in on(xs.start, 640, w)..on(xs.end, 640, w) {
            img.put_pixel(x, y, Rgb([230, 232, 236]));
        }
    }
}

/// The `ENTERING GAME` slab: one line of white glyphs centred across the
/// screen, 0.375 of its width, at half height.
pub(crate) fn entering_game_slab(w: u32, h: u32) -> RgbImage {
    let mut img = navy_band(w, h);
    glyphs(&mut img, 200..440, 168..192);
    img
}

/// The navy band with one line of glyph strokes centred across it, `share`
/// of its width: no two words (the strokes split into many).
pub(crate) fn centred_text_band(w: u32, h: u32, share: f32) -> RgbImage {
    let mut img = navy_band(w, h);
    let half = (320.0 * share).round() as u32;
    glyphs(&mut img, 320 - half..320 + half, 168..192);
    img
}

/// Words as wide as on real frames: `ENTERING GAME` 0.173 of the width
/// with the second word 0.61 of the first, `APPLYING UPDATE` 0.186 and
/// 0.74.
pub(crate) const ENTERING_WORDS: (f32, f32) = (0.173, 0.61);
pub(crate) const UPDATE_WORDS: (f32, f32) = (0.186, 0.74);

/// Two solid words of white centred across the navy band: `(width,
/// ratio)` the whole line as a share of the width and the second word's
/// width over the first's, 0.012 apart. `spinner`: a block 0.03 wide on
/// the left (`Some(true)`) or right, 0.025 from the words, the whole line
/// still centred.
pub(crate) fn two_word_band(
    w: u32,
    h: u32,
    (width, ratio): (f32, f32),
    spinner: Option<bool>,
) -> RgbImage {
    let mut img = navy_band(w, h);
    let (gap, spin, spin_gap) = (0.012, 0.03, 0.025);
    let first = (width - gap) / (1.0 + ratio);
    let line = width
        + if spinner.is_some() {
            spin + spin_gap
        } else {
            0.0
        };
    let mut x = 0.5 - line / 2.0;
    let mut parts = Vec::new();
    if spinner == Some(true) {
        parts.push((x, spin));
        x += spin + spin_gap;
    }
    parts.push((x, first));
    x += first + gap;
    parts.push((x, first * ratio));
    x += first * ratio;
    if spinner == Some(false) {
        parts.push((x + spin_gap, spin));
    }
    let (y0, y1) = (on(168, 360, h), on(192, 360, h));
    for (x0, share) in parts {
        let (a, b) = ((x0 * w as f32) as u32, ((x0 + share) * w as f32) as u32);
        for y in y0..y1 {
            for x in a..b {
                img.put_pixel(x, y, Rgb(GLYPH));
            }
        }
    }
    img
}

/// An `APPLYING UPDATE` band: the same navy band, its text left of centre,
/// with a progress bar under it when `bar` is set, or centred above a bar
/// when `centred` is set.
pub(crate) fn applying_update_band(w: u32, h: u32, centred: bool, bar: bool) -> RgbImage {
    let mut img = navy_band(w, h);
    let (text, track) = if centred {
        (220..420, 120..520)
    } else {
        (48..248, 48..592)
    };
    glyphs(&mut img, text, 160..176);
    if bar {
        block(&mut img, track, 196..200);
    }
    img
}
