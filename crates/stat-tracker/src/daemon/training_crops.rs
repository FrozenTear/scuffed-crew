//! Hero icon and stat digit training crops from an accepted Tab.
//!
//! On the same terms as the map name crop ([`archive_map_label_crop`]):
//! only from a Tab the trust gates accepted (a rejected capture never gets
//! here), only for a known game, and only while the site's last answer has
//! that kind's consent on (`sync::keep_tab_crops`). With consent off nothing
//! is cut and nothing is kept.
//!
//! * `hero_icon`: the career panel's portrait card near the box the
//!   heroes-v2 read found (`PanelRead::box_px`), cut from the full frame,
//!   raw pixels, no pack preprocess. The read's box can sit off the card
//!   (the matcher slides the template inside it), so the cut is snapped to
//!   the card's own left and top edges ([`snap_to_card`]); a Tab where they
//!   are not found keeps none. Only when the panel read passed its gate and
//!   names the Tab's hero authority. ocr-v1 reads no panel box, so it
//!   offers none.
//! * `stat_digits`: the own row's six stat cells, each on its own window
//!   under the header labels ([`training_stat_columns`]), never the name
//!   column and never another row. Only under the new reader, when its own
//!   row verdict names the Tab's row and is confirmed ([`Trust::Sure`]: two
//!   voices agree on it, such as the seed and the highlight), and only when the
//!   chat stack walk and the reach rule show no chat line can reach the row.
//!   ocr-v1 has no own row verdict (its row is a name match or the brightest
//!   row, which a hover or a party tint can light on another player's row),
//!   so it offers none.
//!
//! One set per kind per game (`hero_icon`: per hero), so a game does not
//! fill the outbox. A miss (no box, no header, chat over the row, a crop
//! over a cap) skips that crop and nothing else; a consented crop lost to
//! an error leaves the training warning. The Tab records either way.
//!
//! The pixel work (the scoreboard copy, the chat check, the cuts and the
//! PNG encodes) runs on a blocking thread, so a Tab never waits on it.

use std::collections::VecDeque;
use std::path::Path;
use std::sync::Mutex;

use crate::*;
use stat_tracker::detect::own_row::{OwnRow, Trust};
use stat_tracker::ocr::preprocess::StatColumns;
use stat_tracker::report::blank::{Chat, chat_over_board};
use stat_tracker::shadow::heroes::PanelRead;

/// What an accepted Tab offers [`archive_tab_training_crops`].
pub(crate) struct TabCropSource<'a> {
    /// The career panel's portrait read. `None` under ocr-v1.
    pub(crate) panel: Option<&'a PanelRead>,
    /// The Tab's hero authority under the new reader.
    pub(crate) pack_hero: Option<&'a str>,
    /// The row the Tab's stats came from.
    pub(crate) row: Option<usize>,
    /// The new reader's own row verdict. `None` under ocr-v1.
    pub(crate) own_row: Option<OwnRow>,
    pub(crate) team_size: usize,
    /// The hero this Tab's own read named, for the `stat_digits` meta.
    /// `None` when this Tab's hero authority abstained: the stored hero is
    /// then a held one, which a split can carry over from the last game.
    pub(crate) hero: Option<&'a str>,
}

/// A `hero_icon` crop's box in the frame and the panel read behind it.
struct HeroCrop {
    /// `(x, y, width, height)` in frame pixels, inside the frame: the
    /// card [`snap_to_card`] found, not the panel read's box.
    rect: (u32, u32, u32, u32),
    hero: String,
    score: f32,
    lead: f32,
    pack: &'static str,
}

/// `(game, kind, hero)` keys already offered in this run, newest last.
static OFFERED: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());
const OFFERED_KEEP: usize = 64;

fn offer_key(session_id: &str, kind: sync::UploadKind, hero: &str) -> String {
    format!("{session_id}\n{}\n{hero}", kind.as_str())
}

fn offered(key: &str) -> bool {
    OFFERED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .any(|k| k == key)
}

fn mark_offered(key: String) {
    let mut keys = OFFERED.lock().unwrap_or_else(|e| e.into_inner());
    if !keys.contains(&key) {
        keys.push_back(key);
    }
    while keys.len() > OFFERED_KEEP {
        keys.pop_front();
    }
}

/// Cut this Tab's `hero_icon` and `stat_digits` crops for the game
/// `session_id` and keep them on a blocking thread. Only the consent, gate
/// and box checks and, for a hero icon still wanted, the card snap (two
/// thin luma profiles) run here; the rest of the pixel work runs on that
/// thread. `None` when there is nothing to cut. The handle is for tests.
pub(crate) fn archive_tab_training_crops(
    data_dir: &Path,
    session_id: &str,
    frame: &image::DynamicImage,
    source: TabCropSource<'_>,
) -> Option<tokio::task::JoinHandle<()>> {
    use stat_tracker::sync::UploadKind::{HeroIcon, StatDigits};
    let hero = sync::training_kind_on(data_dir, HeroIcon)
        .then(|| hero_icon_crop(frame, session_id, source.panel, source.pack_hero))
        .flatten();
    let digit_row = (sync::training_kind_on(data_dir, StatDigits)
        && !offered(&offer_key(session_id, StatDigits, "")))
    .then(|| stat_digit_row(source.row, source.own_row))
    .flatten();
    if hero.is_none() && digit_row.is_none() {
        return None;
    }
    let dir = data_dir.to_path_buf();
    let session = session_id.to_string();
    let frame = frame.clone();
    let team_size = source.team_size;
    let digits_hero = source.hero.unwrap_or_default().to_string();
    Some(tokio::task::spawn_blocking(move || {
        let size = (frame.width(), frame.height());
        if let Some(crop) = hero {
            keep_hero_icon(&dir, &session, size, &frame, crop);
        }
        if let Some(row) = digit_row {
            let cells = stat_digit_crops(&frame, row, team_size);
            if !cells.is_empty() {
                keep_stat_digits(&dir, &session, size, &digits_hero, cells);
            }
        }
    }))
}

/// The row stat digit crops may come from: `row`, only when the new
/// reader's own row verdict names that same row and is confirmed
/// ([`Trust::Sure`]: seed and highlight, or one of them and the career
/// hero, agree on it). An unconfirmed row (a strong highlight before any
/// seed exists, or a name fallback) is stored unmarked, but nothing has
/// checked it is the member's, so it gives no digit crop; nor does a
/// suspect one. `None` under ocr-v1 (no verdict), where `row` is a name
/// match or the brightest row and nothing tells a hover or a party tint on
/// another row apart.
pub(crate) fn stat_digit_row(row: Option<usize>, own_row: Option<OwnRow>) -> Option<usize> {
    let row = row?;
    own_row
        .is_some_and(|own| own.row == Some(row) && own.trust == Trust::Sure)
        .then_some(row)
}

/// The career panel's portrait card in `frame`: only when the panel read
/// passed the panel gate and names `pack_hero`, the Tab's hero authority,
/// the game `session_id` has not offered that hero's crop yet, the card's
/// side is within the `hero_icon` edge cap, and [`snap_to_card`] finds the
/// card next to the read's box. `None` for no read (ocr-v1, no hero pack),
/// an abstain, a hero already offered, a card over the cap (4K: never
/// resized, so never kept), a box outside the frame, or no card found. The
/// checks before the snap keep its pixel work off a Tab whose crop could
/// not be kept anyway.
fn hero_icon_crop(
    frame: &image::DynamicImage,
    session_id: &str,
    panel: Option<&PanelRead>,
    pack_hero: Option<&str>,
) -> Option<HeroCrop> {
    let panel = panel?;
    let hero = panel.accepted()?;
    if pack_hero != Some(hero) || offered(&offer_key(session_id, sync::UploadKind::HeroIcon, hero))
    {
        return None;
    }
    let (max_w, max_h) = sync::UploadKind::HeroIcon.crop_kind().max_edges();
    if card_side(frame.width(), frame.height()) > max_w.min(max_h) {
        tracing::debug!(target: "scuffed_stat_tracker", "career panel card over the hero icon edge cap: no hero icon crop");
        return None;
    }
    let (x0, y0, x1, y1) = panel.box_px;
    let [x0, y0, x1, y1] = [x0, y0, x1, y1].map(|v| u32::try_from(v).ok());
    let (x0, y0, x1, y1) = (x0?, y0?, x1?, y1?);
    if x1 <= x0 || y1 <= y0 || x1 > frame.width() || y1 > frame.height() {
        tracing::debug!(target: "scuffed_stat_tracker", box_px = ?panel.box_px, "career panel box outside the frame: no hero icon crop");
        return None;
    }
    let Some((x, y, side)) = snap_to_card(frame, (x0, y0)) else {
        tracing::debug!(target: "scuffed_stat_tracker", box_px = ?panel.box_px, "career panel card edges not found or card past the frame: no hero icon crop");
        return None;
    };
    Some(HeroCrop {
        rect: (x, y, side, side),
        hero: hero.to_string(),
        score: panel.read.score,
        lead: panel.read.lead,
        pack: panel.pack.as_str(),
    })
}

/// Side of the career panel's portrait card, in 16:9 game-rect heights
/// (`u`, the unit [`HeroSet::read_panel`](stat_tracker::shadow::heroes::HeroSet::read_panel)
/// searches in). Measured on 2560x1440 Tabs: about 228 px, top at y 209
/// (0.145 u).
pub(crate) const CARD_SIDE: f64 = 0.158;
/// How far from the panel read's box corner the card's left and top edges
/// are searched, in `u` (43 px at 1440p). The read's box sat 22 to 23 px
/// left and 2 px above the card on one set of 1440p Tabs, and 4 to 13 px
/// right and 14 to 15 px below it on another: at most 0.016 u, so the
/// span starts at least 0.014 u (20 px at 1440p) outside the card even
/// for the worst measured box.
const CARD_SEARCH: f64 = 0.03;
/// Width of the dark and the card side compared at each edge candidate, in
/// `u` (6 px at 1440p, at least 2 px), narrow enough to fit the dark gap
/// between the header bar's right end and the card (13 to 40 px at 1440p).
const CARD_STEP_WINDOW: f64 = 0.004;
/// Most mean luma the dark side of an edge may have. The gap round the
/// card measured about 6 in live games and 16 to 17 in the Practice Range,
/// where the game world shows behind the panel. A surround with no dark
/// gap measured 35 to 40.
const CARD_GAP_MAX_LUMA: f64 = 28.0;
/// Least mean luma of the card side of an edge. The navy card fill dims
/// with the game world behind it: it measured 37 to 39 over a Practice
/// Range world of 16 to 17 and 25 to 29 over a black one (luma 0), the
/// portrait art 50 to 90 and more.
const CARD_MIN_LUMA: f64 = 24.0;
/// Least mean luma rise from the dark side to the card side. The dimmest
/// real edge, a Practice Range gap of 16 to 17 on the card fill of 37 to
/// 39, rises about 21.
const CARD_MIN_STEP: f64 = 16.0;

/// The career panel's portrait card in `frame` as `(x, y, side)`, from the
/// top left corner `(x0, y0)` of the panel read's box: the card's left
/// edge searched within [`CARD_SEARCH`] of `x0` on a column luma profile
/// over the middle half of the rows of a card sized square at `(x0, y0)`,
/// its top edge the same way on a row profile over the middle half of its
/// columns (both bands stay inside the card for any corner within the
/// search, the texture of the portrait art averages out), and a square of
/// [`CARD_SIDE`] cut from there. Each edge is the first dark to card step
/// from outside the card ([`card_edge`]). `None` when either edge is not
/// found or the square would leave the frame.
pub(crate) fn snap_to_card(
    frame: &image::DynamicImage,
    (x0, y0): (u32, u32),
) -> Option<(u32, u32, u32)> {
    use image::GenericImageView;
    let (w, h) = frame.dimensions();
    let (_, _, _, gh) = stat_tracker::frame::geometry::game_rect_16_9(w, h);
    let u = f64::from(gh);
    let side = card_side(w, h);
    let reach = (CARD_SEARCH * u).round() as i64;
    let window = ((CARD_STEP_WINDOW * u).round() as i64).max(2);
    let luma = |x: u32, y: u32| {
        let [r, g, b, _] = frame.get_pixel(x, y).0;
        0.299 * f64::from(r) + 0.587 * f64::from(g) + 0.114 * f64::from(b)
    };
    let band = |start: u32, len: u32| {
        let band = (start + side / 4).min(len)..(start + side * 3 / 4).min(len);
        (!band.is_empty()).then_some(band)
    };
    let (rows, cols) = (band(y0, h)?, band(x0, w)?);
    let (n_rows, n_cols) = (f64::from(rows.len() as u32), f64::from(cols.len() as u32));
    let column = |x: u32| rows.clone().map(|y| luma(x, y)).sum::<f64>() / n_rows;
    let row = |y: u32| cols.clone().map(|x| luma(x, y)).sum::<f64>() / n_cols;
    let left = card_edge(column, x0, reach, window, w)?;
    let top = card_edge(row, y0, reach, window, h)?;
    (left + side <= w && top + side <= h).then_some((left, top, side))
}

/// The card's side in px in a `w` x `h` frame: [`CARD_SIDE`] of the 16:9
/// game rect's height.
fn card_side(w: u32, h: u32) -> u32 {
    let (_, _, _, gh) = stat_tracker::frame::geometry::game_rect_16_9(w, h);
    (CARD_SIDE * f64::from(gh)).round() as u32
}

/// The card's edge on a luma profile (`profile(i)`: the mean luma of
/// column or row `i`) of `len` positions, scanned from `at - reach` toward
/// `at + reach`, that is from outside the card inward: the gap is always
/// outside the card, so the first dark to card step is the card's edge,
/// and brighter portrait art just inside the card is never taken for it.
///
/// A position steps when the `window` positions before it (a dark run)
/// average at most [`CARD_GAP_MAX_LUMA`], the `window` from it at least
/// [`CARD_MIN_LUMA`], and the rise between them is at least
/// [`CARD_MIN_STEP`]. Both sides are window means, so a sharp edge already
/// steps up to `window - 1` positions before it, as the card side starts
/// to take in the card. The edge is the first position from the first step
/// at least halfway from that step's dark run to its card side, or
/// [`CARD_MIN_STEP`] above the dark run where that is less (one always is:
/// the halfway point is under the card side's mean). Relative to the step,
/// not a fixed luma: a half bright anti-aliased first column of the card
/// (luma 25 between a gap of 0 to 6 and a fill of 37, or 27 on a game
/// world gap of 16) is the card's, and the [`CARD_MIN_STEP`] bound keeps a
/// dim card rim the edge when bright portrait art right behind it lifts
/// the card side's mean. A bright region outside (the header bar's right
/// end) never steps: while any of its positions is in the dark side that
/// side is too bright, and once a full dark run lies between it and the
/// card, the card side is gap until the card. `None` when no position
/// steps.
fn card_edge(
    profile: impl Fn(u32) -> f64,
    at: u32,
    reach: i64,
    window: i64,
    len: u32,
) -> Option<u32> {
    let lo = (i64::from(at) - reach).max(window);
    let hi = (i64::from(at) + reach).min(i64::from(len) - window);
    if lo > hi {
        return None;
    }
    let values: Vec<f64> = (lo - window..hi + window)
        .map(|i| profile(i as u32))
        .collect();
    let mean = |s: &[f64]| s.iter().sum::<f64>() / s.len() as f64;
    let w = window as usize;
    let (first, dark, card) = (0..=(hi - lo) as usize).find_map(|edge| {
        let i = edge + w;
        let dark = mean(&values[i - w..i]);
        let card = mean(&values[i..i + w]);
        (dark <= CARD_GAP_MAX_LUMA && card >= CARD_MIN_LUMA && card - dark >= CARD_MIN_STEP)
            .then_some((i, dark, card))
    })?;
    let bright = dark + ((card - dark) / 2.0).min(CARD_MIN_STEP);
    let edge = (first..first + w).find(|&i| values[i] >= bright)?;
    Some((lo - window + edge as i64) as u32)
}

/// The six stat column windows a `stat_digits` crop is cut on, from the
/// header label groups of a `board_w` wide board. Stricter than the
/// reader's [`columns_from_header_groups`](stat_tracker::ocr::preprocess::columns_from_header_groups),
/// which takes the first six narrow groups: there a stray mark left of E
/// is a misread, here it would shift every window one column and upload
/// every crop under the wrong `slot_type`. So exactly six narrow groups,
/// both E/A/D gaps narrower than each DMG/H/MIT gap, and the fourth group
/// (DMG) wider than each of the other five, else `None`. The width rule
/// catches a set shifted one column right (E missing, a mark past MIT),
/// whose gaps can still pass.
pub(crate) fn training_stat_columns(groups: &[(u32, u32)], board_w: u32) -> Option<StatColumns> {
    if board_w == 0 {
        return None;
    }
    let w = f64::from(board_w);
    let labels: Vec<(u32, u32)> = groups
        .iter()
        .copied()
        .filter(|&(s, e)| f64::from(e.saturating_sub(s)) / w <= ocr::preprocess::HEADER_LABEL_MAX_W)
        .collect();
    if labels.len() != sync::STAT_DIGIT_COLUMNS.len() {
        return None;
    }
    let centers: Vec<f64> = labels
        .iter()
        .map(|&(s, e)| (f64::from(s) + f64::from(e)) / 2.0 / w)
        .collect();
    let gaps: Vec<f64> = centers.windows(2).map(|p| p[1] - p[0]).collect();
    let narrow = gaps[0].max(gaps[1]);
    let wide = gaps[2].min(gaps[3]).min(gaps[4]);
    if narrow >= wide {
        return None;
    }
    let width = |&(s, e): &(u32, u32)| e.saturating_sub(s);
    let dmg = width(&labels[3]);
    if labels
        .iter()
        .enumerate()
        .any(|(i, label)| i != 3 && width(label) >= dmg)
    {
        return None;
    }
    ocr::preprocess::columns_from_header_groups(&labels, board_w)
}

/// The own row's stat cells, `(column, crop)`, from the unpadded scoreboard
/// crop of `frame`. Empty when chat can reach the row (the stack walk and
/// the reach rule: a board that cannot prove chat is clear of the row
/// keeps none), or the header labels are not the six stat labels.
fn stat_digit_crops(
    frame: &image::DynamicImage,
    row: usize,
    team_size: usize,
) -> Vec<(usize, image::DynamicImage)> {
    let board = ocr::preprocess::crop_scoreboard(frame);
    let (w, h) = (board.width(), board.height());
    let Some((_, row_y, _, row_h)) = ocr::preprocess::player_row_rect(w, h, row, team_size) else {
        return Vec::new();
    };
    let chat = chat_over_board(&stat_tracker::frame::rgb_view(frame));
    if chat == Chat::Unknown || chat.covers(row_y + row_h, h) {
        tracing::debug!(target: "scuffed_stat_tracker", "chat can reach the own row: no stat digit crops");
        return Vec::new();
    }
    let groups = ocr::preprocess::header_label_groups(&board);
    let Some(columns) = training_stat_columns(&groups, w) else {
        tracing::debug!(target: "scuffed_stat_tracker", "not the six stat header labels: no stat digit crops");
        return Vec::new();
    };
    let Some(row_img) = ocr::preprocess::crop_player_row(&board, row, team_size) else {
        return Vec::new();
    };
    (0..sync::STAT_DIGIT_COLUMNS.len())
        .filter_map(|col| {
            ocr::preprocess::crop_stat_cell(&row_img, col, &columns).map(|c| (col, c))
        })
        .collect()
}

/// `image` as an RGB PNG, raw pixels, losslessly at the best compression.
/// `None` when it cannot be encoded.
fn encode_png(image: &image::DynamicImage) -> Option<Vec<u8>> {
    use image::ImageEncoder;
    use image::codecs::png::{CompressionType, FilterType, PngEncoder};
    let rgb = stat_tracker::frame::rgb_view(image);
    let mut out = Vec::new();
    PngEncoder::new_with_quality(&mut out, CompressionType::Best, FilterType::Adaptive)
        .write_image(
            rgb.as_raw(),
            rgb.width(),
            rgb.height(),
            image::ExtendedColorType::Rgb8,
        )
        .ok()?;
    Some(out)
}

fn keep_hero_icon(
    dir: &Path,
    session: &str,
    (w, h): (u32, u32),
    frame: &image::DynamicImage,
    crop: HeroCrop,
) {
    let kind = sync::UploadKind::HeroIcon;
    let Some(meta) = sync::hero_icon_meta(w, h, &crop.hero, crop.score, crop.lead, crop.pack)
    else {
        return;
    };
    let (x, y, cw, ch) = crop.rect;
    let Some(png) = encode_png(&frame.crop_imm(x, y, cw, ch)) else {
        sync::warn_crop_lost(dir, kind);
        return;
    };
    if sync::keep_tab_crops(dir, session, kind, vec![(meta, png)]).is_some() {
        mark_offered(offer_key(session, kind, &crop.hero));
    }
}

fn keep_stat_digits(
    dir: &Path,
    session: &str,
    (w, h): (u32, u32),
    hero: &str,
    cells: Vec<(usize, image::DynamicImage)>,
) {
    let kind = sync::UploadKind::StatDigits;
    let mut crops = Vec::new();
    let mut lost = false;
    for (col, cell) in cells {
        let Some(meta) = sync::stat_digits_meta(w, h, col, hero) else {
            continue;
        };
        match encode_png(&cell) {
            Some(png) => crops.push((meta, png)),
            None => lost = true,
        }
    }
    // keep_tab_crops warns itself on an error; a later Tab of the game may
    // then try again.
    if sync::keep_tab_crops(dir, session, kind, crops).is_none() {
        return;
    }
    if lost {
        sync::warn_crop_lost(dir, kind);
        return;
    }
    mark_offered(offer_key(session, kind, ""));
}
