//! `rank` training crops: the rank update screen after a competitive game,
//! kept for training once per game while the member's `rank` consent is on.
//!
//! The crop is the report bundle's `crops/rank.png` region
//! (`report::blank::RANK_BAND`), cut by the same `rank_screen_bands` from the
//! same blanked copy of the frame: the role line, the emblem and tier, the
//! progress bar and the hero column. The result word box is filled, and the
//! band holds no player names. It is taken only from a polled frame the
//! report kit already took as a rank screen (`rank_screen_pngs` gave crops),
//! for the game the kit would file it under (same idle rule).
//!
//! With consent off nothing is cut, cloned or kept. A frame under the
//! kind's minimum screen size is skipped before any pixel work. The band is
//! scaled to fit the kind's 1536x720 edge bound; a PNG over the 384 KiB cap
//! is shrunk in a few steps, and one still over the cap at the floor is left
//! out with a warning in the log, never kept or sent. All pixel work runs on
//! its own blocking task, so a failure here never touches the poll tick,
//! the rank read or the game's record.

use std::path::{Path, PathBuf};

use image::DynamicImage;
use stat_tracker::sync::UploadKind;

use super::training_crops::{encode_png, mark_offered, offer_key, offered};
use crate::*;

/// The game a rank screen frame of this tick would give its crop to.
pub(crate) struct RankCropTarget {
    pub(crate) data_dir: PathBuf,
    pub(crate) session: String,
}

/// The open game, when it may still take a `rank` crop: active within the
/// rank screen idle limit (the rule the report kit and the rank reader use,
/// [`super::rank::rank_game`]) and not offered one yet. No file is read
/// here, so it is cheap on every poll tick.
pub(crate) fn rank_crop_target(
    data_dir: &Path,
    game: Option<&crate::ActiveGame>,
) -> Option<RankCropTarget> {
    let session = super::rank::rank_game(game)?;
    if offered(&offer_key(session, UploadKind::Rank, "")) {
        return None;
    }
    Some(RankCropTarget {
        data_dir: data_dir.to_path_buf(),
        session: session.to_string(),
    })
}

/// Offer a rank screen `frame` (one the report kit took as a rank screen)
/// for the game in `target`. Only the consent and screen size checks and,
/// when both pass, the frame clone run here; the cut, the encode and the
/// keep run on a blocking task. `None` when nothing is cut. The handle is
/// for tests.
pub(crate) fn offer_rank_crop(
    target: RankCropTarget,
    frame: &DynamicImage,
) -> Option<tokio::task::JoinHandle<()>> {
    let kind = UploadKind::Rank;
    if !sync::training_kind_on(&target.data_dir, kind) {
        return None;
    }
    let (width, height) = (frame.width(), frame.height());
    let (min_w, min_h) = kind.crop_kind().min_screen()?;
    if width < min_w || height < min_h {
        return None;
    }
    let frame = frame.clone();
    Some(tokio::task::spawn_blocking(move || {
        keep_rank_crop(&target.data_dir, &target.session, &frame);
    }))
}

/// The report bundle's rank region of `frame`: `crops/rank.png` before the
/// kit scales it.
pub(crate) fn rank_band(frame: &DynamicImage) -> image::RgbImage {
    let [_, rank, _] = stat_tracker::report::blank::rank_screen_bands(frame);
    rank
}

/// Narrowest width the byte cap shrink tries, in pixels.
pub(crate) const RANK_SHRINK_FLOOR_WIDTH: u32 = 768;

/// Pixels taken off the width per shrink step.
const RANK_SHRINK_STEP: u32 = 128;

/// `(width, height)` of `(w, h)` scaled down, aspect kept, so it fits
/// `max_w` by `max_h`. Never scaled up.
pub(crate) fn fit_box((w, h): (u32, u32), (max_w, max_h): (u32, u32)) -> (u32, u32) {
    if w <= max_w && h <= max_h {
        return (w, h);
    }
    let (w64, h64) = (u64::from(w), u64::from(h));
    // Scale by the tighter of the two bounds.
    let (num, den) = if w64 * u64::from(max_h) >= h64 * u64::from(max_w) {
        (u64::from(max_w), w64)
    } else {
        (u64::from(max_h), h64)
    };
    let scale = |v: u64| ((v * num / den) as u32).max(1);
    (scale(w64).min(max_w), scale(h64).min(max_h))
}

/// Why [`encode_rank_crop`] gave no PNG.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RankEncodeMiss {
    Encode,
    /// Still over the byte cap at [`RANK_SHRINK_FLOOR_WIDTH`]: the bytes of
    /// the last try.
    OverCap(usize),
}

/// The band scaled into the `rank` edge bound and encoded under the byte
/// cap: at the fitted size, then [`RANK_SHRINK_STEP`] px narrower at a time
/// down to [`RANK_SHRINK_FLOOR_WIDTH`], aspect kept.
pub(crate) fn encode_rank_crop(band: image::RgbImage) -> Result<Vec<u8>, RankEncodeMiss> {
    let crop_kind = UploadKind::Rank.crop_kind();
    let cap = crop_kind.max_crop_bytes();
    let band = DynamicImage::ImageRgb8(band);
    let (fit_w, fit_h) = fit_box((band.width(), band.height()), crop_kind.max_edges());
    let mut width = fit_w;
    loop {
        let height = ((u64::from(fit_h) * u64::from(width) / u64::from(fit_w)) as u32).max(1);
        let sized = if (width, height) == (band.width(), band.height()) {
            band.clone()
        } else {
            band.resize_exact(width, height, image::imageops::FilterType::Triangle)
        };
        let png = encode_png(&sized).ok_or(RankEncodeMiss::Encode)?;
        if png.len() <= cap {
            return Ok(png);
        }
        if width <= RANK_SHRINK_FLOOR_WIDTH {
            return Err(RankEncodeMiss::OverCap(png.len()));
        }
        width = width
            .saturating_sub(RANK_SHRINK_STEP)
            .max(RANK_SHRINK_FLOOR_WIDTH);
    }
}

/// Cut, encode and keep the rank crop of `frame` for the game `session`.
/// The game is marked offered once a crop is kept, and also when it is
/// over the cap (every frame of one rank screen gives about the same
/// bytes, so a retry would only spend the encodes again). An encode or
/// write error leaves the training warning and is not marked, so a later
/// rank screen frame of the game may retry.
pub(crate) fn keep_rank_crop(dir: &Path, session: &str, frame: &DynamicImage) {
    let meta = sync::rank_meta(frame.width(), frame.height());
    keep_rank_result(dir, session, meta, encode_rank_crop(rank_band(frame)));
}

/// Apply an [`encode_rank_crop`] result for the game `session` (see
/// [`keep_rank_crop`]).
pub(crate) fn keep_rank_result(
    dir: &Path,
    session: &str,
    meta: scuffed_types::api::training_crops::TrainingCropUploadMeta,
    result: Result<Vec<u8>, RankEncodeMiss>,
) {
    let kind = UploadKind::Rank;
    match result {
        Ok(png) => {
            if let Some(n) = sync::keep_tab_crops(dir, session, kind, vec![(meta, png)])
                && n > 0
            {
                mark_offered(offer_key(session, kind, ""));
            }
        }
        Err(RankEncodeMiss::Encode) => sync::warn_crop_lost(dir, kind),
        Err(RankEncodeMiss::OverCap(bytes)) => {
            tracing::warn!(
                target: "scuffed_stat_tracker",
                bytes,
                cap = kind.crop_kind().max_crop_bytes(),
                "training crops: rank screen crop left out, over the size cap"
            );
            mark_offered(offer_key(session, kind, ""));
        }
    }
}
