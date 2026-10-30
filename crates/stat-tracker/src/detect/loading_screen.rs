//! The loading screen (`ENTERING GAME`): black above and below a flat
//! lit strip. Pixel check only, no OCR.
//!
//! It is the shape the end-reel letterbox check in [`super::match_end`]
//! turns away as a loading slab: both 7% edge bands dark, a partly lit
//! middle, both shoulders black and a middle that is one flat colour.
//! Used only by the opt-in pre-game screen saves; no boundary or outcome
//! reads it.

use image::RgbImage;

use super::match_end::{
    LETTERBOX_SHOULDER_BOT_Y_PM, LETTERBOX_SHOULDER_DARK_MAX, LETTERBOX_SHOULDER_H_PM,
    LETTERBOX_SHOULDER_TOP_Y_PM, dark_ratio, letterbox_mid_is_footage,
};

/// The middle band holds the lit strip: about 0.3 to 0.5 of it is dark on
/// the loading screen. A darker middle is a fade or a dark scene.
const MID_DARK_MAX: f32 = 0.65;

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
    shoulders_dark(rgb, gx, gy, gw, gh) && !letterbox_mid_is_footage(rgb, gx, mid_y, gw, mid_h)
}

/// The bands just inside the edge bands carry gameplay on a highlight reel
/// and are both black on the loading screen.
fn shoulders_dark(rgb: &RgbImage, gx: u32, gy: u32, gw: u32, gh: u32) -> bool {
    let sh = (gh * LETTERBOX_SHOULDER_H_PM / 1000).max(1);
    let top_y = gy + gh * LETTERBOX_SHOULDER_TOP_Y_PM / 1000;
    let bot_y = gy + gh * LETTERBOX_SHOULDER_BOT_Y_PM / 1000;
    dark_ratio(rgb, gx, top_y, gw, sh) >= LETTERBOX_SHOULDER_DARK_MAX
        && dark_ratio(rgb, gx, bot_y, gw, sh) >= LETTERBOX_SHOULDER_DARK_MAX
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

    /// 7% edge bands black, as in the end-reel tests.
    fn bars(img: &mut RgbImage) {
        for y in 0..30 {
            for x in 0..640 {
                img.put_pixel(x, y, Rgb([8, 8, 8]));
                img.put_pixel(x, 359 - y, Rgb([8, 8, 8]));
            }
        }
    }

    /// A cinematic letterbox over a flat mid colour (the end-reel tests'
    /// flat frame): the shoulders are lit.
    fn letterbox_flat() -> RgbImage {
        let mut img = RgbImage::from_pixel(640, 360, Rgb([40, 60, 90]));
        bars(&mut img);
        img
    }

    /// A letterboxed highlight reel (the end-reel tests' footage frame).
    fn letterbox_footage() -> RgbImage {
        let mut img = RgbImage::from_pixel(640, 360, Rgb([80, 90, 100]));
        for y in 30u32..330 {
            for x in 0u32..640 {
                let n = ((x.wrapping_mul(37) ^ y.wrapping_mul(17)) % 50) as u8;
                let pix = if (x / 80) % 3 == 0 {
                    Rgb([110 + n / 2, 70 + n / 3, 50 + n / 4])
                } else {
                    Rgb([50 + n / 3, 90 + n / 2, 80 + n / 3])
                };
                img.put_pixel(x, y, pix);
            }
        }
        bars(&mut img);
        img
    }

    #[test]
    fn letterboxed_frames_with_lit_shoulders_are_not() {
        assert!(!is_loading_screen(&letterbox_flat()));
        assert!(!is_loading_screen(&letterbox_footage()));
    }

    #[test]
    fn one_dark_shoulder_is_not_enough() {
        let mut img = entering_game();
        // Light the bottom shoulder (y 77% to 85%) only.
        for y in 277..306 {
            for x in 0..640 {
                img.put_pixel(x, y, Rgb([90, 110, 70]));
            }
        }
        assert!(!is_loading_screen(&img));
    }

    #[test]
    fn a_middle_three_quarters_dark_is_not() {
        // A 36-row strip in the 144-row middle band: 0.75 dark, under the
        // old 0.85 cap and over the 0.65 one.
        let mut img = RgbImage::from_pixel(640, 360, Rgb([0, 0, 0]));
        for y in 162..198 {
            for x in 0..640 {
                img.put_pixel(x, y, Rgb([24, 32, 52]));
            }
        }
        for y in 174..186 {
            for x in (200..440).filter(|x| x % 7 < 3) {
                img.put_pixel(x, y, Rgb([240, 240, 240]));
            }
        }
        assert!(!is_loading_screen(&img));
    }

    #[test]
    fn a_mostly_dark_middle_is_not() {
        // A thin lit line in a black frame: a fade or a dark scene.
        let mut img = RgbImage::from_pixel(640, 360, Rgb([0, 0, 0]));
        for y in 176..184 {
            for x in 0..640 {
                img.put_pixel(x, y, Rgb([24, 32, 52]));
            }
        }
        assert!(!is_loading_screen(&img));
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
