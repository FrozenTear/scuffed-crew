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
