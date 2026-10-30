//! The loading screen (`ENTERING GAME`): black above and below a flat
//! lit strip. Pixel check only, no OCR.
//!
//! It is the shape the end-reel letterbox check in [`super::match_end`]
//! turns away as a loading slab: both 7% edge bands dark, a lit middle,
//! and then black shoulders or a middle that is one flat colour. Used only
//! by the opt-in pre-game screen saves; no boundary or outcome reads it.

use image::RgbImage;

use super::match_end::{
    LETTERBOX_SHOULDER_BOT_Y_PM, LETTERBOX_SHOULDER_DARK_MAX, LETTERBOX_SHOULDER_H_PM,
    LETTERBOX_SHOULDER_TOP_Y_PM, dark_ratio, letterbox_mid_is_footage,
};

/// The middle band must hold some lit pixels: a black frame is a fade, not
/// the loading strip.
const MID_DARK_MAX: f32 = 0.85;

/// Edge bands at least this dark, as in the end-reel letterbox check.
const EDGE_DARK_MIN: f32 = 0.72;

/// Whether `rgb` looks like the loading screen.
pub fn is_loading_screen(rgb: &RgbImage) -> bool {
    let (w, h) = rgb.dimensions();
    let (gx, gy, gw, gh) = crate::frame::geometry::game_rect_16_9(w, h);
    let band = (gh * 70 / 1000).max(1);
    if band * 2 >= gh {
        return false;
    }
    let top_dark = dark_ratio(rgb, gx, gy, gw, band);
    let bot_dark = dark_ratio(rgb, gx, gy + gh - band, gw, band);
    let mid_y = gy + gh * 300 / 1000;
    let mid_h = gh * 400 / 1000;
    let mid_dark = dark_ratio(rgb, gx, mid_y, gw, mid_h);
    if top_dark < EDGE_DARK_MIN || bot_dark < EDGE_DARK_MIN || mid_dark >= MID_DARK_MAX {
        return false;
    }
    !shoulders_lit(rgb, gx, gy, gw, gh) || !letterbox_mid_is_footage(rgb, gx, mid_y, gw, mid_h)
}

/// The bands just inside the edge bands carry gameplay on a highlight reel
/// and stay black on the loading screen.
fn shoulders_lit(rgb: &RgbImage, gx: u32, gy: u32, gw: u32, gh: u32) -> bool {
    let sh = (gh * LETTERBOX_SHOULDER_H_PM / 1000).max(1);
    let top_y = gy + gh * LETTERBOX_SHOULDER_TOP_Y_PM / 1000;
    let bot_y = gy + gh * LETTERBOX_SHOULDER_BOT_Y_PM / 1000;
    dark_ratio(rgb, gx, top_y, gw, sh) < LETTERBOX_SHOULDER_DARK_MAX
        && dark_ratio(rgb, gx, bot_y, gw, sh) < LETTERBOX_SHOULDER_DARK_MAX
}

#[cfg(test)]
mod tests {
    use image::Rgb;

    use super::*;

    /// Black bars and a navy strip with white glyphs, like `ENTERING GAME`.
    fn entering_game() -> RgbImage {
        let mut img = RgbImage::from_pixel(640, 360, Rgb([0, 0, 0]));
        for y in 136..224 {
            for x in 0..640 {
                img.put_pixel(x, y, Rgb([24, 32, 52]));
            }
        }
        for y in 168..192 {
            for x in (200..440).filter(|x| x % 7 < 3) {
                img.put_pixel(x, y, Rgb([240, 240, 240]));
            }
        }
        img
    }

    /// Gameplay-like noise over the whole frame.
    fn footage() -> RgbImage {
        RgbImage::from_fn(640, 360, |x, y| {
            Rgb([
                (x * 7 + y * 3) as u8,
                ((x * 13) ^ (y * 5)) as u8,
                (y * 11 + x) as u8,
            ])
        })
    }

    #[test]
    fn the_entering_game_strip_is_the_loading_screen() {
        assert!(is_loading_screen(&entering_game()));
    }

    #[test]
    fn footage_and_black_are_not() {
        assert!(!is_loading_screen(&footage()));
        assert!(!is_loading_screen(&RgbImage::from_pixel(
            640,
            360,
            Rgb([0, 0, 0])
        )));
    }
}
