//! The 1920x1080 path of the ban results and ban vote readers (issue #931).
//!
//! The 1080p ban screens are the 1440p layout scaled by 0.75 (Scuffed
//! Vision, 90 frames of 1080p video). So a 1080p frame is upscaled x4/3
//! with Lanczos3 and read on the 1440p path, positions, scales and
//! thresholds unchanged. The hero select is not read at 1080p.
//!
//! Only the parts of the frame a reader looks at are upscaled
//! ([`RESULTS_REGIONS`], [`VOTE_REGIONS`], [`RING_REGION`]); the rest of the
//! 2560x1440 frame stays black. Inside a region the pixels are exactly
//! those of the whole-frame upscale ([`at_frame_size`]): each crop starts
//! and ends on a multiple of 3 source pixels (4 upscaled ones), so the
//! source position of every upscaled pixel is the same, and keeps
//! [`MARGIN`] source pixels around the region, which is as far as
//! Lanczos3 reaches on an upscale.
//!
//! Measured in a release build (`image` 0.25.9, thin LTO, one core of an
//! i9-11900KF, median of 30): the whole-frame upscale takes 135 ms and
//! 53 MB more peak memory (`image` resizes through a float buffer). The
//! regions take 38 ms and 18 MB for the ban results, 2.5 ms and 4.4 MB
//! for the ban vote, 0.4 ms for the vote ring.

use std::borrow::Cow;

use image::RgbImage;
use image::imageops::{self, FilterType};

use super::{FRAME_1080_H, FRAME_1080_W, FRAME_H, FRAME_W, self_ring_colour};

/// A rectangle at 2560x1440, end exclusive: `(x0, y0, x1, y1)`.
pub(super) type Region = (u32, u32, u32, u32);

/// What the ban results reader looks at: the slot field (x 502..2060,
/// y 344..992 with its edge) and the team bars under the slots of both
/// layouts (x 585..1898, y down to 1111).
pub(super) const RESULTS_REGIONS: [Region; 1] = [(496, 336, 2064, 1120)];

/// What the ban vote reader looks at: the own vote slots with the ring
/// just outside them and the matching field (x 185..375, y 448..515), and
/// the strip with its title, boxes, outer rings and counts (x 1146..1414,
/// y 61..211).
pub(super) const VOTE_REGIONS: [Region; 2] = [(176, 440, 384, 524), (1136, 52, 1424, 216)];

/// Where [`self_ring_colour`] looks (x 64..70, y 430..475).
pub(super) const RING_REGION: Region = (56, 424, 80, 484);

/// Source pixels kept around a region: Lanczos3 reaches 3 source pixels
/// to each side when it upscales.
const MARGIN: u32 = 3;

/// `img` at [`FRAME_W`]x[`FRAME_H`]: as it is when it is that size, the
/// whole frame upscaled with Lanczos3 when it is
/// [`FRAME_1080_W`]x[`FRAME_1080_H`], `None` for any other size. The
/// readers upscale only their regions; this is the whole-frame reference
/// they match, for tools.
pub fn at_frame_size(img: &RgbImage) -> Option<Cow<'_, RgbImage>> {
    match img.dimensions() {
        (FRAME_W, FRAME_H) => Some(Cow::Borrowed(img)),
        (FRAME_1080_W, FRAME_1080_H) => Some(Cow::Owned(imageops::resize(
            img,
            FRAME_W,
            FRAME_H,
            FilterType::Lanczos3,
        ))),
        _ => None,
    }
}

/// Like [`at_frame_size`], but a 1080p frame is upscaled inside `regions`
/// only.
pub(super) fn at_read_size<'a>(img: &'a RgbImage, regions: &[Region]) -> Option<Cow<'a, RgbImage>> {
    match img.dimensions() {
        (FRAME_W, FRAME_H) => Some(Cow::Borrowed(img)),
        (FRAME_1080_W, FRAME_1080_H) => Some(Cow::Owned(upscale_regions(img, regions))),
        _ => None,
    }
}

/// A 1080p `img` upscaled x4/3 inside `regions`, black elsewhere.
fn upscale_regions(img: &RgbImage, regions: &[Region]) -> RgbImage {
    let mut out = RgbImage::new(FRAME_W, FRAME_H);
    // Source bounds: down (start) or up (end) to a multiple of 3, with the
    // margin. 1920 and 1080 are multiples of 3.
    let lo = |v: u32| (v * 3 / 4).saturating_sub(MARGIN) / 3 * 3;
    let hi = |v: u32, max: u32| {
        ((v * 3).div_ceil(4) + MARGIN)
            .div_ceil(3)
            .saturating_mul(3)
            .min(max)
    };
    for &(x0, y0, x1, y1) in regions {
        let (sx0, sy0) = (lo(x0), lo(y0));
        let (sx1, sy1) = (hi(x1, FRAME_1080_W), hi(y1, FRAME_1080_H));
        if sx1 <= sx0 || sy1 <= sy0 {
            continue;
        }
        let (w, h) = (sx1 - sx0, sy1 - sy0);
        let crop = imageops::crop_imm(img, sx0, sy0, w, h);
        let up = imageops::resize(&*crop, w / 3 * 4, h / 3 * 4, FilterType::Lanczos3);
        imageops::replace(
            &mut out,
            &up,
            i64::from(sx0 / 3 * 4),
            i64::from(sy0 / 3 * 4),
        );
    }
    out
}

/// The own-team ring colour of a ban vote frame ([`self_ring_colour`]),
/// at 2560x1440 or upscaled from 1920x1080. `None` for any other size.
pub fn ban_vote_ring(img: &RgbImage) -> Option<[u8; 3]> {
    at_read_size(img, &[RING_REGION]).map(|f| self_ring_colour(&f))
}
