//! `rank.png` and `rank_banner.png`: the rank update screen the rank reader
//! reads, kept with `outcome.png` from the same frame.
//!
//! They are cut whenever `outcome.png` is (the same rank screen frames,
//! [`super::rank_outcome_allowed`]), stay in the private kit folder with the
//! rest of the kit, and are pruned with it. A report carries them only when
//! the member ticks "Include rank screen" (`GameReportInput::rank_included`).

use std::io;
use std::path::Path;

use image::{DynamicImage, RgbImage};

use super::super::blank;
use super::store::{
    OUTCOME_PNG, RANK_BANNER_PNG, RANK_PNG, encode_png, kit_root, private_dir, prune_locked,
    retention, session_dir, was_removed, write_lock, write_private,
};

/// Longest side of a stored rank crop, in pixels. A bigger cut is scaled
/// down to it, so `rank.png` stays well under the report's per-file cap
/// (`bundle::RANK_CROP_MAX_BYTES`). `rank_banner.png` is smaller than this
/// at 1440p and is kept at full size.
pub const RANK_CROP_MAX_EDGE: u32 = 1280;

/// The crops of one rank screen frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RankScreenPngs {
    /// `outcome.png`, the same bytes [`super::rank_outcome_png`] gives.
    pub outcome: Vec<u8>,
    /// `rank.png`, or `None` when it could not be encoded.
    pub rank: Option<Vec<u8>>,
    /// `rank_banner.png`, or `None` when it could not be encoded.
    pub rank_banner: Option<Vec<u8>>,
}

/// The crops for one polled frame, or `None` when the frame may not give
/// `outcome.png` ([`super::rank_outcome_allowed`] decides).
pub fn rank_screen_pngs(
    source: Option<crate::detect::match_end::OutcomeSource>,
    frame: &DynamicImage,
) -> Option<RankScreenPngs> {
    if !super::outcome::rank_frame_allowed(source, frame) {
        return None;
    }
    let [outcome, rank, banner] = blank::rank_screen_bands(frame);
    Some(RankScreenPngs {
        outcome: encode_png(&outcome).ok()?,
        rank: encode_small_png(&fit_edge(rank, RANK_CROP_MAX_EDGE)),
        rank_banner: encode_small_png(&fit_edge(banner, RANK_CROP_MAX_EDGE)),
    })
}

/// `img` scaled down so its longest side is at most `max`, aspect kept.
pub fn fit_edge(img: RgbImage, max: u32) -> RgbImage {
    let (w, h) = img.dimensions();
    let long = w.max(h);
    if long <= max || max == 0 {
        return img;
    }
    let scale = |v: u32| ((u64::from(v) * u64::from(max) / u64::from(long)) as u32).max(1);
    image::imageops::resize(
        &img,
        scale(w),
        scale(h),
        image::imageops::FilterType::Triangle,
    )
}

/// Lossless RGB PNG at the best compression. `None` when it cannot be
/// encoded.
fn encode_small_png(img: &RgbImage) -> Option<Vec<u8>> {
    use image::ImageEncoder;
    use image::codecs::png::{CompressionType, FilterType, PngEncoder};
    let mut out = Vec::new();
    PngEncoder::new_with_quality(&mut out, CompressionType::Best, FilterType::Adaptive)
        .write_image(
            img.as_raw(),
            img.width(),
            img.height(),
            image::ExtendedColorType::Rgb8,
        )
        .ok()?;
    Some(out)
}

/// Store the crops from [`rank_screen_pngs`] in the session's kit. A crop
/// this frame did not give leaves the stored one as it was.
pub fn record_rank_screen_pngs(
    data_dir: &Path,
    session_id: &str,
    pngs: &RankScreenPngs,
) -> io::Result<()> {
    let Some(dir) = session_dir(data_dir, session_id) else {
        return Ok(());
    };
    let _guard = write_lock();
    if was_removed(&dir) {
        return Ok(());
    }
    private_dir(&kit_root(data_dir))?;
    private_dir(&dir)?;
    write_private(&dir, OUTCOME_PNG, &pngs.outcome)?;
    for (name, png) in [
        (RANK_PNG, pngs.rank.as_ref()),
        (RANK_BANNER_PNG, pngs.rank_banner.as_ref()),
    ] {
        if let Some(png) = png {
            write_private(&dir, name, png)?;
        }
    }
    prune_locked(data_dir, retention());
    Ok(())
}
