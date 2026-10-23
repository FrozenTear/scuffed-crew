use ab_glyph::{Font, FontRef, PxScale, ScaleFont, point};
use image::{DynamicImage, Rgb, RgbImage};

use crate::frame::geometry::{crop_scoreboard, game_rect_16_9};
use crate::frame::layout::{ScoreboardPreflight, preflight_scoreboard};

/// Subset of DejaVu Sans Bold. Licence: `fixtures/map-label/LICENSE`.
pub(crate) const SANS_BOLD: &[u8] =
    include_bytes!("../../fixtures/map-label/DejaVuSans-Bold.subset.ttf");

pub(crate) const LIJIANG: &str = "Lijiang Tower";
pub(crate) const GRIMSVOTN: &str = "Watchpoint: Gr\u{00ed}msv\u{00f6}tn";
/// 102 / 255 is about 40% brightness.
pub(crate) const FADED: u8 = 102;
pub(crate) const LIT: u8 = 236;

/// Values 40 through 52. Peak-to-peak stays under the stretch floor.
pub(crate) fn low_noise(w: u32, h: u32) -> DynamicImage {
    let mut img = RgbImage::new(w, h);
    let mut state = 0x1234_5678u32;
    for px in img.pixels_mut() {
        state = state.wrapping_mul(1664525).wrapping_add(1013904223);
        let v = 40 + ((state >> 16) % 13) as u8;
        *px = Rgb([v, v, v]);
    }
    DynamicImage::ImageRgb8(img)
}

pub(crate) fn brightness_range(img: &DynamicImage) -> u8 {
    let rgb = img.to_rgb8();
    let mut min_v = 255u8;
    let mut max_v = 0u8;
    for px in rgb.pixels() {
        let v = ((u16::from(px.0[0]) + u16::from(px.0[1]) + u16::from(px.0[2])) / 3) as u8;
        min_v = min_v.min(v);
        max_v = max_v.max(v);
    }
    max_v.saturating_sub(min_v)
}

/// One line of synthetic top-bar text. Not a captured frame.
pub(crate) fn draw_label(text: &str, ink: u8) -> DynamicImage {
    let font = FontRef::try_from_slice(SANS_BOLD).expect("bundled sans bold");
    let scale = PxScale::from(48.0);
    let scaled = font.as_scaled(scale);
    let width = 1200u32;
    let height = 88u32;
    let bg = [18u8, 20, 28];
    let mut img = RgbImage::from_pixel(width, height, Rgb(bg));
    let mut cursor = 20.0f32;
    let baseline = 14.0 + scaled.ascent();
    for ch in text.chars() {
        let id = font.glyph_id(ch);
        let mut glyph = id.with_scale(scale);
        glyph.position = point(cursor, baseline);
        if let Some(outlined) = font.outline_glyph(glyph) {
            let bounds = outlined.px_bounds();
            outlined.draw(|dx, dy, cov| {
                if cov <= 0.0 {
                    return;
                }
                let x = (bounds.min.x + dx as f32).round() as i32;
                let y = (bounds.min.y + dy as f32).round() as i32;
                if x < 0 || y < 0 || x >= width as i32 || y >= height as i32 {
                    return;
                }
                let dest = img.get_pixel_mut(x as u32, y as u32);
                let cov = cov.clamp(0.0, 1.0);
                for channel in 0..3 {
                    let from = f32::from(dest.0[channel]);
                    let to = f32::from(ink);
                    dest.0[channel] = (from + (to - from) * cov).round() as u8;
                }
            });
        }
        cursor += scaled.h_advance(id);
    }
    assert!(
        cursor < width as f32 - 8.0,
        "{text} ran off the crop at {cursor}"
    );
    DynamicImage::ImageRgb8(img)
}

/// 1080p stand-in. `tab` paints a bright header with six dark labels inside
/// the scoreboard window, which is enough for the preflight to say Ready.
/// The map-name window holds a bright block so a stretch would be legal.
pub(crate) fn frame_1080(tab: bool) -> DynamicImage {
    let (w, h) = (1920u32, 1080u32);
    let mut img = RgbImage::from_pixel(w, h, Rgb([12, 14, 22]));
    let (gx, gy, gw, gh) = game_rect_16_9(w, h);
    let mx = gx + (gw as f64 * 0.68) as u32;
    let my = gy + (gh as f64 * 0.022) as u32;
    for py in my..my + 24 {
        for px in mx..mx + 120 {
            img.put_pixel(px, py, Rgb([102, 102, 102]));
        }
    }
    if tab {
        let bx = gx + (gw as f64 * 0.175) as u32;
        let by = gy + (gh as f64 * 0.15) as u32;
        let bw = (gw as f64 * 0.65) as u32;
        let bh = (gh as f64 * 0.70) as u32;
        // Cover the whole header scan (0.5% to 2.5% of the board). A dark
        // row left in that band would count as ink across every column.
        let y0 = by;
        let y1 = by + (bh as f64 * 0.030) as u32;
        for py in y0..y1 {
            for px in bx..bx + bw {
                img.put_pixel(px, py, Rgb([230, 230, 230]));
            }
        }
        // Six dark labels in the stat area, spaced past the letter merge.
        let origin = bx + (bw as f64 * 0.40) as u32;
        for i in 0..6u32 {
            let x0 = origin + i * 36;
            for py in y0..y1 {
                for px in x0..x0 + 4 {
                    img.put_pixel(px, py, Rgb([20, 20, 20]));
                }
            }
        }
    }
    let frame = DynamicImage::ImageRgb8(img);
    let ready = matches!(
        preflight_scoreboard(&crop_scoreboard(&frame)).0,
        ScoreboardPreflight::Ready(_)
    );
    assert_eq!(ready, tab, "synthetic frame tab={tab}");
    frame
}
