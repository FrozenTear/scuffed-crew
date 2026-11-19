//! `rank` training crops: the rank update screen after a competitive game,
//! kept for training once per game while the member's `rank` consent is on.
//!
//! Not the whole screen and not the report bundle's `crops/rank.png` band:
//! only the regions the rank reader (`stat_tracker::shadow::rank::layout`)
//! reads, each its own crop at native scale with no resize ([`RankRegion`]):
//! the role line (only when the reader finds one, [`layout::role_icon`]),
//! the emblem with its tier word, the progress bar with its labels, the
//! "Higher ranked than N%" percentile, the change pill and tags, and the
//! first hero's SR line (only the digit line the reader
//! finds, never the hero name above it). The result title (top left), the
//! mode banner and the voice overlay list under it (top right), the chat
//! box, the hero column apart from that one SR line and every player name
//! lie outside them. The bar's change tags can name the result ("DEFEAT",
//! "VICTORY"). The progress bar's left end comes near the chat box, so the
//! bar is left out of a frame where chat shows on a line beside its rows
//! ([`report::blank::chat_in_rows`]). The SR line sits where a stream or
//! voice overlay can be drawn, so its crop is refused when the box holds
//! anything but the line's own grey-white glyphs on a plain background
//! ([`sr_overlay`]): a colour that is no mix of the background with grey,
//! a glyph much shorter than the line, a line with too many or too wide
//! glyphs, or ink outside the glyphs. A missed SR crop is fine; one with
//! letters in it is not.
//!
//! A crop is taken only from a polled frame the report kit already took as
//! a rank screen (`rank_screen_pngs` gave crops) and on which the rank
//! reader finds the "RANK PROGRESS:" label ([`offer_for_tick`]), for
//! the game the kit would file it under (same idle rule). The game is
//! claimed before any pixel work, so a second rank screen tick while the
//! first one's task still runs cuts nothing; an encode or write error
//! releases the claim so a later frame may try again.
//!
//! With consent off nothing is cut, cloned or kept. A frame under the
//! kind's minimum screen size is skipped before any pixel work, with one
//! log line per game. A region outside the kind's 1536x720 edge bound (the
//! tier and the bar at 4K, an info line) or over its 384 KiB cap (a
//! warning) is left out, never scaled, kept or sent; the other regions of
//! the frame are still kept. All pixel work runs on its own blocking task, so a
//! failure here never touches the poll tick, the rank read or the game's
//! record.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use image::{DynamicImage, RgbImage};
use scuffed_types::api::training_crops::{CropSpot, TrainingCropUploadMeta};
use stat_tracker::report::kit::RankScreenPngs;
use stat_tracker::shadow::rank::Reader;
use stat_tracker::shadow::rank::ink::Glyph;
use stat_tracker::shadow::rank::layout::{self, Entry, RoleLine, Scale};
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
/// other polled frame, and only when the rank reader finds the "RANK
/// PROGRESS:" label on it ([`rank_label_shows`]). Hero highlight art and
/// the placement screen pass the kit's rank screen word but show no such
/// label: they are not offered and leave the game unclaimed, so a later
/// rank update frame of the game still is (#760). `None` when nothing is
/// cut.
pub(crate) fn offer_for_tick(
    rank_pngs: Option<&RankScreenPngs>,
    target: Option<RankCropTarget>,
    frame: &DynamicImage,
) -> Option<tokio::task::JoinHandle<()>> {
    rank_pngs?;
    let target = target?;
    if !sync::training_kind_on(&target.data_dir, UploadKind::Rank) {
        return None;
    }
    if !rank_label_shows(&target.data_dir, frame) {
        tracing::debug!(
            target: "scuffed_stat_tracker",
            "training crops: rank screen frame without the rank progress label, not offered"
        );
        return None;
    }
    offer_rank_crop(target, frame)
}

/// Rank readers by data folder, loaded on first use (only successful loads
/// are kept, so a pack that arrives later is picked up).
static LABEL_READERS: std::sync::Mutex<Vec<(PathBuf, Arc<Reader>)>> =
    std::sync::Mutex::new(Vec::new());

/// Whether the rank reader's own rank screen test
/// ([`Reader::is_rank_screen`]: the "RANK PROGRESS:" label matched against
/// the rank-v1 pack) passes on `frame`. Fails safe: no pack, a pack that
/// does not load, or a frame size the reader does not take (only 1080p and
/// 1440p) is no label, so nothing is offered. The pack is loaded whether
/// or not the rank reader switch is on; the test is a crop of about 135x26
/// reference pixels and one template match.
pub(crate) fn rank_label_shows(data_dir: &Path, frame: &DynamicImage) -> bool {
    let Some(reader) = label_reader(data_dir) else {
        return false;
    };
    let rgb = stat_tracker::frame::rgb_view(frame);
    reader.is_rank_screen(&rgb).is_ok()
}

fn label_reader(data_dir: &Path) -> Option<Arc<Reader>> {
    let mut readers = LABEL_READERS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, r)) = readers.iter().find(|(d, _)| d == data_dir) {
        return Some(Arc::clone(r));
    }
    match Reader::load(data_dir) {
        Ok(r) => {
            let r = Arc::new(r);
            readers.push((data_dir.to_path_buf(), Arc::clone(&r)));
            Some(r)
        }
        Err(error) => {
            tracing::debug!(
                target: "scuffed_stat_tracker",
                %error,
                "training crops: no rank pack to check the rank progress label, nothing offered"
            );
            None
        }
    }
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
/// [`RankRegion::Sr`] the SR line its hero search finds. No mode banner:
/// the voice overlay list can reach into its box with another person's
/// name, and nothing there tells the two apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RankRegion {
    Role,
    Tier,
    Bar,
    Sr,
}

impl RankRegion {
    /// Every region, in the order its crops are kept.
    pub(crate) const ALL: [RankRegion; 4] = [
        RankRegion::Role,
        RankRegion::Tier,
        RankRegion::Bar,
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
/// ([`layout::sr_cut_short`]), the line holds SR glyphs
/// ([`layout::sr_glyphs`]) and the box holds nothing else ([`sr_overlay`]).
/// The box is those glyphs plus 2 reference rows and columns, never above
/// the SR line's top, so the hero name stays out. [`RegionMiss::NotFound`]
/// on a frame the reader does not take (only 1080p and 1440p).
pub(crate) fn sr_spot(rgb: &RgbImage) -> Result<CropSpot, RegionMiss> {
    let s = Scale::from_img(rgb).ok_or(RegionMiss::NotFound)?;
    let e = layout::first_entry(rgb).ok_or(RegionMiss::NotFound)?;
    if layout::sr_cut_short(rgb, &e) {
        return Err(RegionMiss::NotFound);
    }
    let gl = layout::sr_glyphs(rgb, &e);
    let pad = s.n(2);
    let x0 = gl.iter().map(|g| g.bbox[0]).min();
    let x1 = gl.iter().map(|g| g.bbox[2]).max();
    let (Some(x0), Some(x1)) = (x0, x1) else {
        return Err(RegionMiss::NotFound);
    };
    let (x0, x1) = (x0.saturating_sub(pad), (x1 + pad).min(rgb.width()));
    // Between the name line and the SR line there are at least 4
    // reference rows; the box never reaches above the name line's end.
    let y0 = e.sr.0.saturating_sub(pad).max(e.name.1);
    let y1 = (e.sr.1 + pad).min(rgb.height());
    if x1 <= x0 || y1 <= y0 {
        return Err(RegionMiss::NotFound);
    }
    let spot = CropSpot {
        x: x0,
        y: y0,
        w: x1 - x0,
        h: y1 - y0,
    };
    match sr_overlay(rgb, s, &e, &gl, spot) {
        Some(why) => Err(RegionMiss::Overlay(why)),
        None => Ok(spot),
    }
}

/// What in an SR box is not the SR line's own digits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SrOverlay {
    /// Pixels of a colour that is no mix of the box's background with
    /// grey or white (overlay text, a name chip, an icon): their count.
    Colour(u32),
    /// A glyph under [`SR_MIN_HEIGHT`] of the SR line's height, at most the
    /// SR digits' own height (lower case or smaller overlay text).
    Short,
    /// A glyph over [`SR_MAX_HEIGHT`] of the SR digits' height (the rank
    /// icon alone, larger text).
    Tall,
    /// More glyphs than an SR number has, one wider than two digits, or
    /// glyphs wider than the rank icon left out of the line's cluster.
    Shape,
    /// Ink outside the glyphs (text between or around the digits): its
    /// pixel count.
    Stray(u32),
}

/// A glyph shorter than this share of the SR line's height refuses the box.
pub(crate) const SR_MIN_HEIGHT: f32 = 0.8;
/// A glyph taller than this share of the SR digits' height refuses the box.
const SR_MAX_HEIGHT: f32 = 1.25;
/// At most this many glyphs: four digits and the rank icon.
const SR_MAX_GLYPHS: usize = 5;
/// Height of the SR digits at 2560x1440 (21 to 23 rows on real screens).
const SR_DIGIT_ROWS: u32 = 22;
/// Widest span, in digit heights, of the glyphs the reader's shape rule
/// may drop left of the digits: the rank icon (about 1.2 on real screens).
const SR_ICON_SPAN: f32 = 1.5;
/// A glyph wider than this many times its height is not one or two digits.
const SR_MAX_ASPECT: f32 = 2.0;
/// Chroma (0 to 255 per channel) a pixel may stray from the background's
/// hue line before it counts as a foreign colour.
const SR_COLOUR_DIST: f32 = 48.0;
/// Channel difference from the background that counts as ink.
const SR_INK_DIST: i32 = 64;
/// Foreign colour or stray ink pixels a box may hold (anti-aliasing, a
/// background gradient), at 1440p; scaled by area at 1080p.
const SR_MAX_FOREIGN: f32 = 6.0;

/// Why the SR box `spot` on `rgb` at scale `s` (around glyphs `gl` of entry `e`) is not
/// only the SR digits, or `None` when it is. Cheap: one pass over a box of
/// a few thousand pixels. The reader's digit templates are not used (the
/// pack is not loaded here), so a line of upper case letters at digit
/// height in grey-white on the plain background, packed like digits,
/// still passes; overlays drawn on the rank screen use smaller or coloured
/// text on their own backing, which these rules refuse.
pub(crate) fn sr_overlay(
    rgb: &RgbImage,
    s: Scale,
    e: &Entry,
    gl: &[Glyph],
    spot: CropSpot,
) -> Option<SrOverlay> {
    // The line's rows can include a rank icon taller than the digits (a
    // grey icon is ink to the line search), so the digit height is capped
    // at the SR digits' own height.
    let line = (e.sr.1 - e.sr.0).min(s.n(SR_DIGIT_ROWS)) as f32;
    let height = |g: &Glyph| (g.bbox[3] - g.bbox[1]) as f32;
    let width = |g: &Glyph| (g.bbox[2] - g.bbox[0]) as f32;
    // The reader's line shape drops a rank icon left of the digits that is
    // taller or lower than they are (one glyph, or two when the icon has a
    // gap down its middle). Anything dropped wider than an icon is
    // something else drawn along the line (overlay text running into the
    // number), so the digits kept may be only a part of the SR.
    let cluster = layout::sr_cluster(rgb, e);
    let dropped = &cluster[..cluster.len().saturating_sub(gl.len())];
    let dropped_span = match (dropped.first(), dropped.last()) {
        (Some(a), Some(b)) => b.bbox[2].saturating_sub(a.bbox[0]) as f32,
        _ => 0.0,
    };
    if dropped_span > SR_ICON_SPAN * line
        || gl.len() > SR_MAX_GLYPHS
        || gl.iter().any(|g| width(g) > SR_MAX_ASPECT * height(g))
    {
        return Some(SrOverlay::Shape);
    }
    if gl.iter().any(|g| height(g) < SR_MIN_HEIGHT * line) {
        return Some(SrOverlay::Short);
    }
    // Taller than a digit: what the line kept is not the SR digits (an
    // overlay hid them and left the rank icon, or a larger text).
    let digit = s.n(SR_DIGIT_ROWS) as f32;
    if gl.iter().any(|g| height(g) > SR_MAX_HEIGHT * digit) {
        return Some(SrOverlay::Tall);
    }
    // Glyph boxes grown by one pixel for anti-aliasing: the rest of the box
    // is background.
    let in_glyph = |x: u32, y: u32| {
        gl.iter()
            .any(|g| x + 1 >= g.bbox[0] && x <= g.bbox[2] && y + 1 >= g.bbox[1] && y <= g.bbox[3])
    };
    let px = |x: u32, y: u32| rgb.get_pixel(x, y).0.map(i32::from);
    let cells =
        || (spot.y..spot.y + spot.h).flat_map(|y| (spot.x..spot.x + spot.w).map(move |x| (x, y)));
    let mut bg: [Vec<i32>; 3] = Default::default();
    for (x, y) in cells().filter(|&(x, y)| !in_glyph(x, y)) {
        for (c, v) in px(x, y).into_iter().enumerate() {
            bg[c].push(v);
        }
    }
    if bg[0].is_empty() {
        return Some(SrOverlay::Stray(0));
    }
    let bg = bg.map(|mut v| {
        v.sort_unstable();
        v[v.len() / 2]
    });
    let max_foreign = (SR_MAX_FOREIGN * s.sx * s.sy).round().max(1.0) as u32;
    let mut colour = 0;
    let mut stray = 0;
    for (x, y) in cells() {
        let p = px(x, y);
        if foreign_colour(p, bg) > SR_COLOUR_DIST {
            colour += 1;
        }
        let ink = (0..3).map(|c| (p[c] - bg[c]).abs()).max().unwrap_or(0);
        if ink > SR_INK_DIST && !in_glyph(x, y) {
            stray += 1;
        }
    }
    if colour > max_foreign {
        Some(SrOverlay::Colour(colour))
    } else if stray > max_foreign {
        Some(SrOverlay::Stray(stray))
    } else {
        None
    }
}

/// How far `p`'s colour lies from every mix of the background `bg` with a
/// grey: its chroma (the part off the grey axis) against `bg`'s chroma
/// scaled by 0 to 1.25. Grey-white digits, their anti-aliased edges and a
/// darker or lighter patch of the same background are all near 0; a
/// colour of another hue, or one much more saturated than the background,
/// is far.
fn foreign_colour(p: [i32; 3], bg: [i32; 3]) -> f32 {
    let chroma = |c: [i32; 3]| {
        let m = (c[0] + c[1] + c[2]) as f32 / 3.0;
        c.map(|v| v as f32 - m)
    };
    let (cp, cb) = (chroma(p), chroma(bg));
    let nb = cb.iter().map(|v| v * v).sum::<f32>();
    let t = if nb > 0.0 {
        (cp.iter().zip(cb).map(|(a, b)| a * b).sum::<f32>() / nb).clamp(0.0, 1.25)
    } else {
        0.0
    };
    cp.iter()
        .zip(cb)
        .map(|(a, b)| (a - t * b).powi(2))
        .sum::<f32>()
        .sqrt()
}

/// Where `region` is on `rgb`, before the edge and chat checks.
pub(crate) fn region_spot(region: RankRegion, rgb: &RgbImage) -> Result<CropSpot, RegionMiss> {
    match region.reference_box() {
        Some(b) => frame_spot(b, rgb.width(), rgb.height()).ok_or(RegionMiss::NotFound),
        None => sr_spot(rgb),
    }
}

/// Why a region gave no crop. Only [`RegionMiss::Encode`] fails the frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RegionMiss {
    /// Not on this frame (no SR line found, or a box outside the frame).
    NotFound,
    /// Nothing to label: the role band is empty (an open queue screen).
    Blank,
    /// The SR box holds more than the SR digits ([`sr_overlay`]).
    Overlay(SrOverlay),
    /// Wider or taller than the kind's edge bound, or under its 8 px
    /// minimum: native pixels are never scaled to fit.
    Edges(u32, u32),
    /// Chat shows on a line beside the bar's rows.
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

/// One region of `rgb`: its box, the edge bound, the role line's empty
/// check, the bar's chat check, then [`encode_region`].
pub(crate) fn cut_region(region: RankRegion, rgb: &RgbImage) -> Result<RankCrop, RegionMiss> {
    let spot = region_spot(region, rgb)?;
    check_edges(spot)?;
    match region {
        RankRegion::Role if layout::role_icon(rgb) == RoleLine::Empty => {
            return Err(RegionMiss::Blank);
        }
        RankRegion::Bar => {
            let [_, top, _, bottom] = region.reference_box().unwrap_or([0, 0, 0, 1440]);
            if stat_tracker::report::blank::chat_in_rows(rgb, f64::from(top), f64::from(bottom)) {
                return Err(RegionMiss::Chat);
            }
        }
        _ => {}
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
            Err(RegionMiss::Blank) => {
                tracing::debug!(
                    target: "scuffed_stat_tracker",
                    slot = region.slot_type(),
                    "training crops: rank role line empty (open queue), not kept"
                );
            }
            Err(RegionMiss::Overlay(why)) => {
                tracing::info!(
                    target: "scuffed_stat_tracker",
                    slot = region.slot_type(),
                    ?why,
                    "training crops: rank SR line left out, the box holds more than its digits"
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
                tracing::info!(
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
    size: (u32, u32),
    result: Result<Vec<RankCrop>, EncodeFailed>,
) {
    keep_rank_result_with(dir, session, size, result, |meta, png| {
        sync::keep_tab_crops(dir, session, UploadKind::Rank, vec![(meta, png)])
    });
}

/// [`keep_rank_result`] with `write` keeping one region (the store's
/// [`sync::keep_tab_crops`]; `None` on a write error), so a test can fail
/// a write after an earlier region was kept.
pub(crate) fn keep_rank_result_with(
    dir: &Path,
    session: &str,
    (width, height): (u32, u32),
    result: Result<Vec<RankCrop>, EncodeFailed>,
    mut write: impl FnMut(TrainingCropUploadMeta, Vec<u8>) -> Option<usize>,
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
        match write(meta, crop.png) {
            Some(n) => kept += n,
            None => lost = true,
        }
    }
    if lost && kept == 0 {
        unmark_offered(&rank_key(session));
    }
}
