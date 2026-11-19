//! `rank` training crops: the rank update screen after a competitive game,
//! kept for training once per game while the member's `rank` consent is on.
//!
//! Not the whole screen and not the report bundle's `crops/rank.png` band:
//! only the regions the rank reader (`stat_tracker::shadow::rank::layout`)
//! reads, each its own crop at native scale with no resize ([`RankRegion`]):
//! the role line, the emblem with its tier word, the progress bar with its
//! labels and tags, the mode banner word, and the first hero's SR line
//! (only the digit line the reader finds, never the hero name above it).
//! The result title (top left), the chat box, the hero column apart from
//! that one SR line, stream overlays at the screen's edges and every
//! player name lie outside them. The bar's change tags can name the
//! result ("DEFEAT", "VICTORY"). The progress
//! bar's left end comes near the chat box, so the bar is left out of a
//! frame where chat shows ([`report::blank::chat_up_to`]).
//!
//! A crop is taken only from a polled frame the report kit already took as
//! a rank screen (`rank_screen_pngs` gave crops, [`offer_for_tick`]), for
//! the game the kit would file it under (same idle rule). The game is
//! claimed before any pixel work, so a second rank screen tick while the
//! first one's task still runs cuts nothing; an encode or write error
//! releases the claim so a later frame may try again.
//!
//! With consent off nothing is cut, cloned or kept. A frame under the
//! kind's minimum screen size is skipped before any pixel work, with one
//! log line per game. A region outside the kind's 1536x720 edge bound (the
//! tier and the bar at 4K) or over its 384 KiB cap is left out with a
//! warning in the log, never scaled, kept or sent; the other regions of the
//! frame are still kept. All pixel work runs on its own blocking task, so a
//! failure here never touches the poll tick, the rank read or the game's
//! record.

use std::path::{Path, PathBuf};

use image::{DynamicImage, RgbImage};
use scuffed_types::api::training_crops::CropSpot;
use stat_tracker::report::kit::RankScreenPngs;
use stat_tracker::shadow::rank::layout;
use stat_tracker::sync::{RANK_SLOTS, UploadKind};

use super::training_crops::{claim_offered, encode_png, offer_key, offered, unmark_offered};
use crate::*;

/// The game a rank screen frame of this tick would give its crops to.
pub(crate) struct RankCropTarget {
    pub(crate) data_dir: PathBuf,
    pub(crate) session: String,
}

/// The open game, when it may still take `rank` crops: active within the
/// rank screen idle limit (the rule the report kit and the rank reader use,
/// [`super::rank::rank_game`]) and not offered or claimed yet. No file is
/// read here, so it is cheap on every poll tick.
pub(crate) fn rank_crop_target(
    data_dir: &Path,
    game: Option<&crate::ActiveGame>,
) -> Option<RankCropTarget> {
    let session = super::rank::rank_game(game)?;
    if offered(&rank_key(session)) {
        return None;
    }
    Some(RankCropTarget {
        data_dir: data_dir.to_path_buf(),
        session: session.to_string(),
    })
}

fn rank_key(session: &str) -> String {
    offer_key(session, UploadKind::Rank, "")
}

/// The poll tick's gate: offer `frame` only when the report kit took it as
/// a rank screen (`rank_pngs` is `Some`), never a scoreboard, a Tab or any
/// other polled frame. `None` when nothing is cut.
pub(crate) fn offer_for_tick(
    rank_pngs: Option<&RankScreenPngs>,
    target: Option<RankCropTarget>,
    frame: &DynamicImage,
) -> Option<tokio::task::JoinHandle<()>> {
    rank_pngs?;
    offer_rank_crop(target?, frame)
}

/// Offer a rank screen `frame` (one the report kit took as a rank screen)
/// for the game in `target`. Only the consent and screen size checks, the
/// claim and, when all pass, the frame clone run here; the cut, the
/// encodes and the keep run on a blocking task. `None` when nothing is cut
/// (also when the game is already claimed). The handle is for tests.
pub(crate) fn offer_rank_crop(
    target: RankCropTarget,
    frame: &DynamicImage,
) -> Option<tokio::task::JoinHandle<()>> {
    let kind = UploadKind::Rank;
    if !sync::training_kind_on(&target.data_dir, kind) {
        return None;
    }
    let key = rank_key(&target.session);
    let (width, height) = (frame.width(), frame.height());
    let (min_w, min_h) = kind.crop_kind().min_screen()?;
    if width < min_w || height < min_h {
        // Once per game: the screen size does not change mid-game, so mark
        // the game offered and say why no rank crop comes from it.
        if claim_offered(key) {
            tracing::info!(
                width,
                height,
                min_w,
                min_h,
                "rank screen crop left out, screen under the rank crop minimum size"
            );
        }
        return None;
    }
    // Claimed before the task: a later tick of the same rank screen finds
    // the game taken while this one still encodes.
    if !claim_offered(key) {
        return None;
    }
    let frame = frame.clone();
    Some(tokio::task::spawn_blocking(move || {
        keep_rank_crop(&target.data_dir, &target.session, &frame);
    }))
}

/// One rank screen region: a box of the rank reader's layout, or for
/// [`RankRegion::Sr`] the SR line its hero search finds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RankRegion {
    Role,
    Tier,
    Bar,
    Banner,
    Sr,
}

impl RankRegion {
    /// Every region, in the order its crops are kept.
    pub(crate) const ALL: [RankRegion; 5] = [
        RankRegion::Role,
        RankRegion::Tier,
        RankRegion::Bar,
        RankRegion::Banner,
        RankRegion::Sr,
    ];

    /// The crop's `slot_type`.
    pub(crate) fn slot_type(self) -> &'static str {
        RANK_SLOTS[self as usize]
    }

    /// `[x0, y0, x1, y1]` (half-open) at the 2560x1440 reference layout,
    /// taken from the reader's boxes (`shadow::rank::layout`). `None` for
    /// the SR line, which is found per frame.
    pub(crate) fn reference_box(self) -> Option<[u32; 4]> {
        match self {
            // `role_icon`: the role line above the emblem.
            RankRegion::Role => Some([1060, 236, 1500, 326]),
            // The emblem down to `rank_text`'s tier word and division
            // (x 1150-1420, y 820-1020), centred like the role line.
            RankRegion::Tier => Some([1070, 340, 1490, 1020]),
            // `gate_word`, `pct_glyphs`, `higher_glyphs`, `pill_glyphs`,
            // `emblem_digits` and `tag_words`: x 552 to 2009, y 1058 to 1220.
            RankRegion::Bar => Some([540, 1048, 2040, 1232]),
            // `banner_word`: the mode banner text up to the timer.
            RankRegion::Banner => Some([2056, 136, 2430, 186]),
            RankRegion::Sr => None,
        }
    }
}

/// `[x0, x1)` x `[y0, y1)` reference box `b` in the frame pixels of a
/// `w` x `h` frame: scaled to its 16:9 playfield the way the reader scales
/// a 16:9 frame (`layout::Scale`, rounded), and clipped to the frame.
pub(crate) fn frame_spot(b: [u32; 4], w: u32, h: u32) -> Option<CropSpot> {
    let (gx, gy, gw, gh) = stat_tracker::ocr::preprocess::game_rect_16_9(w, h);
    let (sx, sy) = (f64::from(gw) / 2560.0, f64::from(gh) / 1440.0);
    let px = |v: u32| (gx + (f64::from(v) * sx).round() as u32).min(w);
    let py = |v: u32| (gy + (f64::from(v) * sy).round() as u32).min(h);
    let (x0, y0, x1, y1) = (px(b[0]), py(b[1]), px(b[2]), py(b[3]));
    (x1 > x0 && y1 > y0).then_some(CropSpot {
        x: x0,
        y: y0,
        w: x1 - x0,
        h: y1 - y0,
    })
}

/// The first hero's SR line as the rank reader finds it: only when its
/// hero search finds a name line over an SR line
/// ([`layout::first_entry`]), the SR window does not cut the digits short
/// ([`layout::sr_cut_short`]) and the line holds SR glyphs
/// ([`layout::sr_glyphs`]). The box is those glyphs plus 2 reference rows
/// and columns, never above the SR line's top, so the hero name stays out.
/// `None` on a frame the reader does not take (only 1080p and 1440p).
pub(crate) fn sr_spot(rgb: &RgbImage) -> Option<CropSpot> {
    let s = layout::Scale::from_img(rgb)?;
    let e = layout::first_entry(rgb)?;
    if layout::sr_cut_short(rgb, &e) {
        return None;
    }
    let gl = layout::sr_glyphs(rgb, &e);
    let pad = s.n(2);
    let x0 = gl.iter().map(|g| g.bbox[0]).min()?.saturating_sub(pad);
    let x1 = (gl.iter().map(|g| g.bbox[2]).max()? + pad).min(rgb.width());
    // Between the name line and the SR line there are at least 4
    // reference rows; the box never reaches above the name line's end.
    let y0 = e.sr.0.saturating_sub(pad).max(e.name.1);
    let y1 = (e.sr.1 + pad).min(rgb.height());
    (x1 > x0 && y1 > y0).then_some(CropSpot {
        x: x0,
        y: y0,
        w: x1 - x0,
        h: y1 - y0,
    })
}

/// Where `region` is on `rgb`, before the edge and chat checks.
pub(crate) fn region_spot(region: RankRegion, rgb: &RgbImage) -> Option<CropSpot> {
    match region.reference_box() {
        Some(b) => frame_spot(b, rgb.width(), rgb.height()),
        None => sr_spot(rgb),
    }
}

/// Why a region gave no crop. Only [`RegionMiss::Encode`] fails the frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RegionMiss {
    /// Not on this frame (no SR line found, or a box outside the frame).
    NotFound,
    /// Wider or taller than the kind's edge bound, or under its 8 px
    /// minimum: native pixels are never scaled to fit.
    Edges(u32, u32),
    /// Chat shows beside the bar's left end.
    Chat,
    /// Over the byte cap: the PNG's bytes.
    OverCap(usize),
    Encode,
}

/// One kept region: where it was cut and its PNG.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RankCrop {
    pub(crate) region: RankRegion,
    pub(crate) spot: CropSpot,
    pub(crate) png: Vec<u8>,
}

/// Smallest crop edge the server takes.
const MIN_CROP_EDGE: u32 = 8;

/// The native PNG of `spot` on `rgb`, when it fits the kind's edges and
/// byte cap.
pub(crate) fn encode_region(rgb: &RgbImage, spot: CropSpot) -> Result<Vec<u8>, RegionMiss> {
    let crop_kind = UploadKind::Rank.crop_kind();
    check_edges(spot)?;
    if spot.x + spot.w > rgb.width() || spot.y + spot.h > rgb.height() {
        return Err(RegionMiss::NotFound);
    }
    let cut = image::imageops::crop_imm(rgb, spot.x, spot.y, spot.w, spot.h).to_image();
    let png = encode_png(&DynamicImage::ImageRgb8(cut)).ok_or(RegionMiss::Encode)?;
    if png.len() > crop_kind.max_crop_bytes() {
        return Err(RegionMiss::OverCap(png.len()));
    }
    Ok(png)
}

/// `spot` within the kind's edge bound and the server's 8 px minimum.
fn check_edges(spot: CropSpot) -> Result<(), RegionMiss> {
    let (max_w, max_h) = UploadKind::Rank.crop_kind().max_edges();
    if (MIN_CROP_EDGE..=max_w).contains(&spot.w) && (MIN_CROP_EDGE..=max_h).contains(&spot.h) {
        Ok(())
    } else {
        Err(RegionMiss::Edges(spot.w, spot.h))
    }
}

/// One region of `rgb`: its box, the edge bound, the bar's chat check,
/// then [`encode_region`].
pub(crate) fn cut_region(region: RankRegion, rgb: &RgbImage) -> Result<RankCrop, RegionMiss> {
    let spot = region_spot(region, rgb).ok_or(RegionMiss::NotFound)?;
    check_edges(spot)?;
    if region == RankRegion::Bar {
        let top = f64::from(region.reference_box().map_or(0, |b| b[1]));
        if stat_tracker::report::blank::chat_up_to(rgb, top) {
            return Err(RegionMiss::Chat);
        }
    }
    let png = encode_region(rgb, spot)?;
    Ok(RankCrop { region, spot, png })
}

/// The frame could not be encoded: nothing of it is kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct EncodeFailed;

/// Every region of `frame` that fits, in [`RankRegion::ALL`] order. A
/// region that does not fit is left out with a log line; an encode error
/// fails the whole frame, so a retry never sends a region twice.
pub(crate) fn rank_crops(frame: &DynamicImage) -> Result<Vec<RankCrop>, EncodeFailed> {
    let rgb = stat_tracker::frame::rgb_view(frame);
    let mut out = Vec::new();
    for region in RankRegion::ALL {
        match cut_region(region, &rgb) {
            Ok(crop) => out.push(crop),
            Err(RegionMiss::Encode) => return Err(EncodeFailed),
            Err(RegionMiss::NotFound) => {
                tracing::debug!(
                    target: "scuffed_stat_tracker",
                    slot = region.slot_type(),
                    "training crops: rank region not on this frame"
                );
            }
            Err(RegionMiss::Chat) => {
                tracing::info!(
                    target: "scuffed_stat_tracker",
                    slot = region.slot_type(),
                    "training crops: rank bar left out, chat on screen beside it"
                );
            }
            Err(RegionMiss::Edges(w, h)) => {
                tracing::warn!(
                    target: "scuffed_stat_tracker",
                    slot = region.slot_type(),
                    w,
                    h,
                    "training crops: rank region left out, outside the rank crop edge bound"
                );
            }
            Err(RegionMiss::OverCap(bytes)) => {
                tracing::warn!(
                    target: "scuffed_stat_tracker",
                    slot = region.slot_type(),
                    bytes,
                    cap = UploadKind::Rank.crop_kind().max_crop_bytes(),
                    "training crops: rank region left out, over the size cap"
                );
            }
        }
    }
    Ok(out)
}

/// Cut, encode and keep the rank crops of `frame` for the game `session`
/// (claimed by [`offer_rank_crop`]; see [`keep_rank_result`]).
pub(crate) fn keep_rank_crop(dir: &Path, session: &str, frame: &DynamicImage) {
    let size = (frame.width(), frame.height());
    keep_rank_result(dir, session, size, rank_crops(frame));
}

/// Apply a [`rank_crops`] result for the game `session` from a screen of
/// `(width, height)`. The game stays claimed once anything is kept, and
/// when no region fit (every frame of one rank screen gives about the same
/// regions, so a retry would only spend the encodes again). An encode
/// error, or a write error before any region was kept, leaves the training
/// warning and releases the claim, so a later rank screen frame of the game
/// may retry. A write error after a region was kept keeps the claim, so no
/// region is sent twice.
pub(crate) fn keep_rank_result(
    dir: &Path,
    session: &str,
    (width, height): (u32, u32),
    result: Result<Vec<RankCrop>, EncodeFailed>,
) {
    let kind = UploadKind::Rank;
    let crops = match result {
        Ok(crops) => crops,
        Err(EncodeFailed) => {
            sync::warn_crop_lost(dir, kind);
            unmark_offered(&rank_key(session));
            return;
        }
    };
    if crops.is_empty() {
        tracing::info!(
            target: "scuffed_stat_tracker",
            "training crops: no rank region of this game fit, none kept"
        );
        return;
    }
    let mut kept = 0;
    let mut lost = false;
    for crop in crops {
        let meta = sync::rank_meta(width, height, crop.region.slot_type(), crop.spot);
        match sync::keep_tab_crops(dir, session, kind, vec![(meta, crop.png)]) {
            Some(n) => kept += n,
            None => lost = true,
        }
    }
    if lost && kept == 0 {
        unmark_offered(&rank_key(session));
    }
}
