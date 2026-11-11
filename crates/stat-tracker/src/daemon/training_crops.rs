//! Hero icon and stat digit training crops from an accepted Tab.
//!
//! On the same terms as the map name crop ([`archive_map_label_crop`]):
//! only from a Tab the trust gates accepted (a rejected capture never gets
//! here), only for a known game, and only while the site's last answer has
//! that kind's consent on (`sync::keep_tab_crops`). With consent off nothing
//! is cut and nothing is kept.
//!
//! * `hero_icon`: the career panel's portrait box the heroes-v2 read found
//!   (`PanelRead::box_px`), cut from the full frame, raw pixels, no pack
//!   preprocess. Only when the panel read passed its gate and names the
//!   Tab's hero authority. ocr-v1 reads no panel box, so it offers none.
//! * `stat_digits`: the own row's six stat cells, each on its own window
//!   under the header labels (`columns_from_header_groups`), never the name
//!   column and never another row. Only when the own row is not suspect and
//!   no chat line can reach it. Any reader.
//!
//! One set per kind per game (`hero_icon`: per hero), so a game does not
//! fill the outbox. A miss (no box, no header, chat over the row, a crop
//! over a cap) skips that crop and nothing else; a consented crop lost to
//! an error leaves the training warning. The Tab records either way.

use std::collections::VecDeque;
use std::path::Path;
use std::sync::Mutex;

use crate::*;
use stat_tracker::report::blank::{Chat, chat_over_board};
use stat_tracker::shadow::heroes::PanelRead;

/// What an accepted Tab offers [`archive_tab_training_crops`].
pub(crate) struct TabCropSource<'a> {
    /// The career panel's portrait read. `None` under ocr-v1.
    pub(crate) panel: Option<&'a PanelRead>,
    /// The Tab's hero authority under the new reader.
    pub(crate) pack_hero: Option<&'a str>,
    pub(crate) own_row: Option<usize>,
    pub(crate) own_row_suspect: bool,
    pub(crate) team_size: usize,
    /// The hero stored for this Tab, for the `stat_digits` meta.
    pub(crate) hero: &'a str,
}

/// A `hero_icon` crop and the panel read behind it.
struct HeroCrop {
    image: image::DynamicImage,
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
/// `session_id` and keep them on a blocking thread. `None` when there is
/// nothing to keep. The handle is for tests.
pub(crate) fn archive_tab_training_crops(
    data_dir: &Path,
    session_id: &str,
    frame: &image::DynamicImage,
    source: TabCropSource<'_>,
) -> Option<tokio::task::JoinHandle<()>> {
    use stat_tracker::sync::UploadKind::{HeroIcon, StatDigits};
    let hero = sync::training_kind_on(data_dir, HeroIcon)
        .then(|| hero_icon_crop(frame, source.panel, source.pack_hero))
        .flatten()
        .filter(|crop| !offered(&offer_key(session_id, HeroIcon, &crop.hero)));
    let digits_on = sync::training_kind_on(data_dir, StatDigits)
        && !offered(&offer_key(session_id, StatDigits, ""));
    let digits = if digits_on {
        stat_digit_crops(frame, &source)
    } else {
        Vec::new()
    };
    if hero.is_none() && digits.is_empty() {
        return None;
    }
    let dir = data_dir.to_path_buf();
    let session = session_id.to_string();
    let size = (frame.width(), frame.height());
    let stored_hero = source.hero.to_string();
    Some(tokio::task::spawn_blocking(move || {
        if let Some(crop) = hero {
            keep_hero_icon(&dir, &session, size, crop);
        }
        if !digits.is_empty() {
            keep_stat_digits(&dir, &session, size, &stored_hero, digits);
        }
    }))
}

/// The career panel's portrait, cut from `frame` at the panel read's box:
/// only when that read passed the panel gate and names `pack_hero`, the
/// Tab's hero authority. `None` for no read (ocr-v1, no hero pack), an
/// abstain, or a box outside the frame.
fn hero_icon_crop(
    frame: &image::DynamicImage,
    panel: Option<&PanelRead>,
    pack_hero: Option<&str>,
) -> Option<HeroCrop> {
    let panel = panel?;
    let hero = panel.accepted()?;
    if pack_hero != Some(hero) {
        return None;
    }
    let (x0, y0, x1, y1) = panel.box_px;
    let [x0, y0, x1, y1] = [x0, y0, x1, y1].map(|v| u32::try_from(v).ok());
    let (x0, y0, x1, y1) = (x0?, y0?, x1?, y1?);
    if x1 <= x0 || y1 <= y0 || x1 > frame.width() || y1 > frame.height() {
        tracing::debug!(target: "scuffed_stat_tracker", box_px = ?panel.box_px, "career panel box outside the frame: no hero icon crop");
        return None;
    }
    Some(HeroCrop {
        image: frame.crop_imm(x0, y0, x1 - x0, y1 - y0),
        hero: hero.to_string(),
        score: panel.read.score,
        lead: panel.read.lead,
        pack: panel.pack.as_str(),
    })
}

/// The own row's stat cells, `(column, crop)`, from the unpadded scoreboard
/// crop of `frame`. Empty for no own row, a suspect own row, chat that can
/// reach the row, or header labels not found.
fn stat_digit_crops(
    frame: &image::DynamicImage,
    source: &TabCropSource<'_>,
) -> Vec<(usize, image::DynamicImage)> {
    let Some(row) = source.own_row.filter(|_| !source.own_row_suspect) else {
        return Vec::new();
    };
    let board = ocr::preprocess::crop_scoreboard(frame);
    let (w, h) = (board.width(), board.height());
    let Some((_, row_y, _, row_h)) = ocr::preprocess::player_row_rect(w, h, row, source.team_size)
    else {
        return Vec::new();
    };
    let chat = chat_over_board(&stat_tracker::frame::rgb_view(frame));
    if chat == Chat::Unknown || chat.covers(row_y + row_h, h) {
        tracing::debug!(target: "scuffed_stat_tracker", "chat can reach the own row: no stat digit crops");
        return Vec::new();
    }
    let groups = ocr::preprocess::header_label_groups(&board);
    let Some(columns) = ocr::preprocess::columns_from_header_groups(&groups, w) else {
        tracing::debug!(target: "scuffed_stat_tracker", "stat header labels not found: no stat digit crops");
        return Vec::new();
    };
    let Some(row_img) = ocr::preprocess::crop_player_row(&board, row, source.team_size) else {
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

fn keep_hero_icon(dir: &Path, session: &str, (w, h): (u32, u32), crop: HeroCrop) {
    let kind = sync::UploadKind::HeroIcon;
    let Some(meta) = sync::hero_icon_meta(w, h, &crop.hero, crop.score, crop.lead, crop.pack)
    else {
        return;
    };
    let Some(png) = encode_png(&crop.image) else {
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
