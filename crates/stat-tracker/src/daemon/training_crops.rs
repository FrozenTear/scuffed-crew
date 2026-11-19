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
//!   raw pixels, no pack preprocess. A `hero_icon` is shrunk to 171 px
//!   with Lanczos3 (smaller steps only if still needed) only when its
//!   PNG is over the 65,536-byte cap; otherwise it is stored as the
//!   native card; screen fields stay the game's. The
//!   read's box can sit off the card
//!   (the matcher slides the template inside it), so the cut is snapped to
//!   the card's own left and top edges ([`snap_to_card`]) and kept only
//!   when its far edges hold and its outer tops agree with the cut's top.
//!   A Tab where an edge is not found, a far edge does not hold, or an
//!   outer top is off the cut keeps none. Only when the panel read passed
//!   its gate and names the Tab's hero authority. ocr-v1 reads no panel
//!   box, so it offers none.
//! * `stat_digits`: the own row's six stat cells, each on its own window
//!   under the header labels ([`training_stat_columns`]), never the name
//!   column and never another row. Only under the new reader, when its own
//!   row verdict names the Tab's row and is confirmed ([`Trust::Sure`]: two
//!   voices agree on it, such as the seed and the highlight), and only when the
//!   chat stack walk and the reach rule show no chat line can reach the row.
//!   Only from a Tab whose own damage, healing or mitigation has reached
//!   [`STAT_DIGITS_MIN_BIG_STAT`], so the set is not the spawn board of
//!   zeros. Only this Tab's own sure read that the gate stored counts
//!   ([`sure_accepted_big_stats`]), never a held total from an earlier
//!   Tab. A Tab under it is not marked offered: a later Tab of the same
//!   game retries, and a game that never reaches it keeps no set. A game a
//!   rejoin continued after a crash keeps no set at all, from the board
//!   that continued it to the game's end, whatever its columns read
//!   ([`digit_gate_big_stats`]).
//!   ocr-v1 has no own row verdict (its row is a name match or the brightest
//!   row, which a hover or a party tint can light on another player's row),
//!   so it offers none.
//!
//! One set per kind per game (`hero_icon`: per hero), so a game does not
//! fill the outbox. The Tab claims that key before its save starts on the
//! blocking thread ([`claim_offered`]), so two close Tabs cannot both
//! save it; a save that keeps nothing releases the claim. A miss (no box,
//! no card edge, no header, chat over the row, or a card over the 256 px
//! edge cap) skips that crop and nothing else. If a `hero_icon` is still refused after that downscale, that hero
//! is not marked offered, so a later Tab of the same game can retry. A
//! consented crop lost to an error leaves the training warning. The Tab
//! records either way.
//!
//! The card snap runs inline in `handle_capture`, after the Tab's row is
//! committed: it reads a few thin luma profiles, well under 1 ms. The frame
//! is then cloned inline too, when a hero icon or a stat_digits crop is
//! still wanted. The rest of the pixel work after the clone (the scoreboard
//! crop, the chat check, the cuts and the PNG encodes) runs on a blocking
//! thread, so a Tab never waits on it.

use std::collections::VecDeque;
use std::path::Path;
use std::sync::Mutex;

use crate::*;
use stat_tracker::detect::own_row::{OwnRow, Trust};
use stat_tracker::games::stat_reads::TabReads;
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
    /// Damage, healing and mitigation for the [`STAT_DIGITS_MIN_BIG_STAT`]
    /// gate: this Tab's own sure read that the capture gate accepted and
    /// stored ([`digit_gate_big_stats`]). `None` when the cell was not
    /// read for sure on this Tab, or was held or refused by the gate, and
    /// in every column on any Tab of a game a rejoin continued; that does
    /// not meet the gate. `Some(0)` is a sure read of zero.
    pub(crate) damage: Option<u32>,
    pub(crate) healing: Option<u32>,
    pub(crate) mitigation: Option<u32>,
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

pub(super) fn offered(key: &str) -> bool {
    OFFERED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .any(|k| k == key)
}

pub(super) fn mark_offered(key: String) {
    claim_offered(key);
}

/// Mark `key` offered unless it already is, under one lock: `true` when
/// this call marked it (the caller owns the offer), `false` when it was
/// already offered or claimed.
pub(super) fn claim_offered(key: String) -> bool {
    let mut keys = OFFERED.lock().unwrap_or_else(|e| e.into_inner());
    if keys.contains(&key) {
        return false;
    }
    keys.push_back(key);
    while keys.len() > OFFERED_KEEP {
        keys.pop_front();
    }
    true
}

/// Release a claim from [`claim_offered`], so a later frame may offer again.
pub(super) fn unmark_offered(key: &str) {
    OFFERED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .retain(|k| k != key);
}

/// Blocking crop tasks started and not yet done, so a test can wait for
/// them to land instead of sleeping.
#[cfg(test)]
pub(super) static CROP_TASKS_IN_FLIGHT: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

/// Held for writing, it keeps every blocking crop task from starting its
/// work, so a test can offer two Tabs before the first one saves.
#[cfg(test)]
pub(super) static CROP_TASK_GATE: std::sync::RwLock<()> = std::sync::RwLock::new(());

/// Counts one blocking crop task in [`CROP_TASKS_IN_FLIGHT`] until dropped.
#[cfg(test)]
struct InFlight;

#[cfg(test)]
impl InFlight {
    fn start() -> Self {
        CROP_TASKS_IN_FLIGHT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        InFlight
    }
}

#[cfg(test)]
impl Drop for InFlight {
    fn drop(&mut self) {
        CROP_TASKS_IN_FLIGHT.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}

/// Cut this Tab's `hero_icon` and `stat_digits` crops for the game
/// `session_id` and keep them on a blocking thread. Only the consent, gate
/// and box checks, for a hero icon still wanted the card snap (six thin
/// luma profiles, plus a re-read of the right one inside the cut when a
/// drop lands there), and the frame clone run here; the rest of the pixel
/// work runs on that thread. `None` when there is nothing to cut. The
/// handle is for tests.
pub(crate) fn archive_tab_training_crops(
    data_dir: &Path,
    session_id: &str,
    frame: &image::DynamicImage,
    source: TabCropSource<'_>,
) -> Option<tokio::task::JoinHandle<()>> {
    use stat_tracker::sync::UploadKind::{HeroIcon, StatDigits};
    // Each kind is claimed here, before the save starts on the blocking
    // thread, so two close Tabs cannot both pass the offered check and
    // save the same kind twice. The task releases a claim when it kept
    // nothing of that kind (no cell cut, every crop refused or lost, or
    // consent turned off mid-save), so a later Tab retries.
    let hero = sync::training_kind_on(data_dir, HeroIcon)
        .then(|| hero_icon_crop(frame, session_id, source.panel, source.pack_hero))
        .flatten()
        .filter(|crop| claim_offered(offer_key(session_id, HeroIcon, &crop.hero)));
    // A Tab under the big stat gate claims nothing, so a later Tab of this
    // game retries.
    let digit_row = (sync::training_kind_on(data_dir, StatDigits)
        && stat_digits_qualify(source.damage, source.healing, source.mitigation)
        && !offered(&offer_key(session_id, StatDigits, "")))
    .then(|| stat_digit_row(source.row, source.own_row))
    .flatten()
    .filter(|_| claim_offered(offer_key(session_id, StatDigits, "")));
    if hero.is_none() && digit_row.is_none() {
        return None;
    }
    let dir = data_dir.to_path_buf();
    let session = session_id.to_string();
    let frame = frame.clone();
    let team_size = source.team_size;
    let digits_hero = source.hero.unwrap_or_default().to_string();
    #[cfg(test)]
    let in_flight = InFlight::start();
    Some(tokio::task::spawn_blocking(move || {
        #[cfg(test)]
        let _in_flight = in_flight;
        #[cfg(test)]
        drop(CROP_TASK_GATE.read().unwrap_or_else(|e| e.into_inner()));
        let size = (frame.width(), frame.height());
        if let Some(crop) = hero {
            keep_hero_icon(&dir, &session, size, &frame, crop);
        }
        if let Some(row) = digit_row {
            let cells = stat_digit_crops(&frame, row, team_size);
            if cells.is_empty() {
                unmark_offered(&offer_key(&session, StatDigits, ""));
            } else {
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

/// Least own damage, healing or mitigation a Tab needs before its
/// `stat_digits` set is cut, so the set has multi-digit cells and is not
/// the spawn board of zeros. Scuffed Vision tunes this from the
/// benchmark. A Tab below it is not marked offered, so a later Tab of the
/// same game can retry; a game that never reaches it saves no set.
pub(crate) const STAT_DIGITS_MIN_BIG_STAT: u32 = 1000;

/// This Tab's own damage, healing and mitigation for the
/// [`STAT_DIGITS_MIN_BIG_STAT`] gate, from its stat reads after
/// `mark_gate` and the `stored` row (gate order, E A D DMG H MIT). A cell
/// counts only when this Tab read it for sure and the gate stored that
/// read. A held, gate held, unsure or unread cell is `None`, and so is a
/// sure read the gate refused (the stored value differs): a held total,
/// such as the pre-crash one on a rejoin's zero board, is not on screen.
pub(crate) fn sure_accepted_big_stats(reads: &TabReads, stored: &[u32; 6]) -> [Option<u32>; 3] {
    std::array::from_fn(|i| {
        let col = 3 + i;
        reads[col]
            .filter(|read| !read.unsure && read.value == Some(stored[col]))
            .map(|_| stored[col])
    })
}

/// The big stats [`TabCropSource`] carries for the
/// [`STAT_DIGITS_MIN_BIG_STAT`] gate: [`sure_accepted_big_stats`], except
/// on a game a rejoin continued (`rejoined`: the game's `rejoin_carry` is
/// set, from the zero board that continued it on), where every column is
/// `None` for the rest of the game. Such a game saves no `stat_digits`
/// set at all: its stored totals add the pre-crash carry to what the
/// screen shows, and a column whose carry is 0 (or one that grew past
/// 1000 after the rejoin) would otherwise still qualify.
pub(crate) fn digit_gate_big_stats(
    reads: &TabReads,
    stored: &[u32; 6],
    rejoined: bool,
) -> [Option<u32>; 3] {
    if rejoined {
        return [None; 3];
    }
    sure_accepted_big_stats(reads, stored)
}

/// Whether a Tab's own `damage`, `healing` or `mitigation` reaches
/// [`STAT_DIGITS_MIN_BIG_STAT`]. A value not read (`None`) does not.
pub(crate) fn stat_digits_qualify(
    damage: Option<u32>,
    healing: Option<u32>,
    mitigation: Option<u32>,
) -> bool {
    [damage, healing, mitigation]
        .into_iter()
        .flatten()
        .any(|n| n >= STAT_DIGITS_MIN_BIG_STAT)
}

/// The career panel's portrait card in `frame`: only when the panel read
/// passed the panel gate and names `pack_hero`, the Tab's hero authority,
/// the game `session_id` has not offered that hero's crop yet, the card's
/// side is within the `hero_icon` edge cap, and [`snap_to_card`] finds the
/// card next to the read's box. `None` for no read (ocr-v1, no hero pack),
/// an abstain, a hero already offered, a card over the 256 px edge cap
/// (a game height of 1624 or more, including 4K: skipped, not resized
/// to fit the edge cap, so never kept), a box outside the frame, or no
/// card found. The
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
    let (x, y, side) = match snap_to_card(frame, (x0, y0)) {
        Ok(cut) => cut,
        Err(SnapMiss::NearEdge) => {
            tracing::debug!(target: "scuffed_stat_tracker", box_px = ?panel.box_px, "career panel card edges not found or card past the frame: no hero icon crop");
            return None;
        }
        Err(SnapMiss::FarEdge { cut, right, bottom }) => {
            tracing::debug!(target: "scuffed_stat_tracker", box_px = ?panel.box_px, cut = ?cut, right_drop = ?right, bottom_drop = ?bottom, "career panel cut's far edges are not the card's (a drop of None: none found): no hero icon crop");
            return None;
        }
        Err(SnapMiss::OuterTop { cut, tops }) => {
            tracing::debug!(target: "scuffed_stat_tracker", box_px = ?panel.box_px, cut = ?cut, outer_tops = ?tops, "career panel cut's top is not the card's over its outer columns (a top of None: none found): no hero icon crop");
            return None;
        }
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
/// How far the left and top edges are searched from the panel read's box
/// corner, and how far the bottom drop and the outer tops are searched
/// from the cut, in `u` (43 px at 1440p). The read's box sat 22 to 23 px
/// left and 2 px above the card on one set of 1440p Tabs, and 4 to 13 px
/// right and 14 to 15 px below it on another: at most 0.016 u, so the
/// span starts at least 0.014 u (20 px at 1440p) outside the card even
/// for the worst measured box.
const CARD_SEARCH: f64 = 0.03;
/// How far past the cut's right edge the far edge check looks for the
/// card's right drop, in `u` (36 px at 1440p): shorter than
/// [`CARD_SEARCH`], which the left, top, bottom and outer-top searches
/// keep. On Practice Range Tabs the navy panel runs on for 40 px right of
/// the card and then ends. Where that end rises 16 or more (a mid-band of
/// 22, the low end of the measured 22 to 24), a reach of 0.0275 u already
/// rounds to 40 px and takes the end for a drop past the cut, so the
/// exact cut is rejected. 0.025 u stops the scan short of it. On a layout
/// with a dark gap right of the card, a left shift is then caught only up
/// to 36 px, not 43.
const CARD_RIGHT_SEARCH: f64 = 0.025;
/// Width of the dark and the card side compared at each edge candidate, in
/// `u` (6 px at 1440p, at least 2 px), narrow enough to fit the dark gap
/// between the header bar's right end and the card (13 to 40 px at 1440p).
const CARD_STEP_WINDOW: f64 = 0.004;
/// Most luma any position of the dark side of an edge may have. The gap
/// round the card measured about 6 in live games and 16 to 17 in the
/// Practice Range, where the game world shows behind the panel (17 to 21
/// under the card). A surround with no dark gap measured 35 to 40.
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
/// How far a step's card side must hold, in `u` (65 px at 1440p, at least
/// one `window`): every [`CARD_STEP_WINDOW`] window from the step to this
/// far in must stay at or above the step's edge threshold, so a thin
/// bright line is not taken for the card. On a 5v5 Tab the table's right
/// border is a 3 to 4 px line (peak 52) about 40 px left of the card, after
/// a dark run of 9 and before a gap of 5 to 7: its window rises about 17
/// to a mean of about 26, which passes [`CARD_MIN_LUMA`] and
/// [`CARD_MIN_STEP`], but the gap after it drops back to 7. The real card
/// holds: its dimmest fill, 25 to 29 for 10 px over a black world then 31
/// to 57, stays above its threshold of about 14, and the Practice Range
/// fill of 37 to 39 above its threshold of about 25. Long enough to cover
/// the search: with the box up to 23 px left of the card the scan starts
/// about 66 px left of it, and the gap before the card is at least 13 px
/// (the header bar's end), so any UI step in the span is followed by a
/// full gap window within 0.045 u. A threshold relative to the step, not
/// [`CARD_GAP_MAX_LUMA`]: the dim fill's own windows (about 27) are under
/// that.
const CARD_SUSTAIN: f64 = 0.045;

/// Why [`snap_to_card`] keeps no cut.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum SnapMiss {
    /// A near (left or top) edge not found, or the square past the frame.
    NearEdge,
    /// The cut `(x, y, side)` was found, but its far edges do not hold
    /// ([`far_edges_hold`]): the right and bottom drops found, `None` where
    /// none was.
    FarEdge {
        cut: (u32, u32, u32),
        right: Option<u32>,
        bottom: Option<u32>,
    },
    /// The cut `(x, y, side)`'s far edges hold, but its top edge, searched
    /// again over its outer left and right column bands, is not within 1 px
    /// of the cut's top: the top found on each band, `None` where none was.
    OuterTop {
        cut: (u32, u32, u32),
        tops: [Option<u32>; 2],
    },
}

/// The career panel's portrait card in `frame` as `(x, y, side)`, from the
/// top left corner `(x0, y0)` of the panel read's box: the card's left
/// edge searched within [`CARD_SEARCH`] of `x0` on a column luma profile
/// over the middle half of the rows of a card sized square at `(x0, y0)`,
/// its top edge the same way on a row profile over the middle half of its
/// columns (both bands stay inside the card for any corner within the
/// search, the texture of the portrait art averages out), and a square of
/// [`CARD_SIDE`] cut from there. Each edge is the first dark to card step
/// from outside the card whose card side holds ([`card_edge`]).
///
/// Before the cut is kept, its far edges are checked on the same kind of
/// profiles, banded on the cut ([`far_edges_hold`]), against a wrong first
/// step that keeps a square shifted off the card. The bottom drop is
/// required, so a vertical shift keeps none. The right drop catches a left
/// shift only where the Tab has a gap right of the card, and a right shift
/// of 4 px or more when the panel's end sits inside the right reach and
/// rises 16 or more (a mid-band of 22). On Practice Range Tabs the navy
/// panel often gives no right drop, so the top edge is then searched
/// again, the same way as the near top edge, on row profiles over the
/// cut's outer column bands (`side/16 .. side/4` and `3*side/4 ..
/// 15*side/16` from its left), and each must land within 1 px of the
/// cut's top: the card's top runs across its full width, while a cut
/// shifted onto a lit patch of the game world beside the card has world
/// above its outer band, whose top rarely lines up with the card's.
/// Residuals that still keep a shifted cut: a world patch under about 26
/// px wide at 1440p (19 at 1080p), whatever rows it spans, because the
/// outer-left band starts `side/16` (14 px) into the cut; a wider patch
/// that spans exactly the card's rows (its top within 1 px of the card's);
/// and a right shift beside the navy panel, whose top is the card's, unless
/// the panel's end qualifies and the right drop catches it. No bottom
/// version of this guard: dark portrait art low in the card would fail it
/// with little margin. The top version can also drop a correct card once
/// dark art covers about a quarter of an outer band near the top.
///
/// The outer top caps stay at [`CARD_GAP_MAX_LUMA`] and [`CARD_MIN_STEP`]:
/// on live 5v5 Tabs (full captures) a soft navy rim over an outer band (a
/// gap of 21 to 31, a rise of 8 to 15) drops some true cards, while caps
/// loose enough to keep them also keep a cut shifted 30 px left onto a
/// tall or partial lit world patch.
///
/// A black row band over the card's top (a bar, or a black game world
/// above a card whose bottom drop lands early) keeps no cut, by design:
/// [`SnapMiss::FarEdge`] while the band's edge is inside the top search,
/// [`SnapMiss::NearEdge`] past it. The cut is never moved up to the top
/// the bottom drop implies, which would also keep a cut shifted up by a
/// card bottom that reads short. Only a band inside the far edge's 1 px
/// tolerance still cuts, from the band's edge (1 px on a card painted the
/// full cut side; none on the measured cards, a px short). No full
/// capture has shown such a band: the one measured was the top edge of a
/// `debug/accepted` board crop (`debug_accepted_board`). Those crops, and
/// frames rebuilt from them, clip the card's top 7 px at 1440p (5 at
/// 1080p), so they cannot test the top edge: test this snap on full
/// frames only.
///
/// [`SnapMiss::NearEdge`] when either near edge is not found or the square
/// would leave the frame, [`SnapMiss::FarEdge`] when a far edge does not
/// hold, [`SnapMiss::OuterTop`] when an outer top does not.
pub(crate) fn snap_to_card(
    frame: &image::DynamicImage,
    (x0, y0): (u32, u32),
) -> Result<(u32, u32, u32), SnapMiss> {
    use image::GenericImageView;
    let (w, h) = frame.dimensions();
    let (_, _, _, gh) = stat_tracker::frame::geometry::game_rect_16_9(w, h);
    let u = f64::from(gh);
    let side = card_side(w, h);
    let reach = (CARD_SEARCH * u).round() as i64;
    let window = ((CARD_STEP_WINDOW * u).round() as i64).max(2);
    let sustain = (CARD_SUSTAIN * u).round() as i64;
    let luma = |x: u32, y: u32| {
        let [r, g, b, _] = frame.get_pixel(x, y).0;
        0.299 * f64::from(r) + 0.587 * f64::from(g) + 0.114 * f64::from(b)
    };
    let band = |start: u32, len: u32| {
        let band = (start + side / 4).min(len)..(start + side * 3 / 4).min(len);
        (!band.is_empty()).then_some(band)
    };
    let near = || {
        let (rows, cols) = (band(y0, h)?, band(x0, w)?);
        let (n_rows, n_cols) = (f64::from(rows.len() as u32), f64::from(cols.len() as u32));
        let column = |x: u32| rows.clone().map(|y| luma(x, y)).sum::<f64>() / n_rows;
        let row = |y: u32| cols.clone().map(|x| luma(x, y)).sum::<f64>() / n_cols;
        let left = card_edge(column, x0, (reach, window, sustain), w)?;
        let top = card_edge(row, y0, (reach, window, sustain), h)?;
        (left + side <= w && top + side <= h).then_some((left, top))
    };
    let (left, top) = near().ok_or(SnapMiss::NearEdge)?;
    let (rows, cols) = (band(top, h), band(left, w));
    let (rows, cols) = rows.zip(cols).ok_or(SnapMiss::NearEdge)?;
    let (n_rows, n_cols) = (f64::from(rows.len() as u32), f64::from(cols.len() as u32));
    let column = |x: u32| rows.clone().map(|y| luma(x, y)).sum::<f64>() / n_rows;
    let row = |y: u32| cols.clone().map(|x| luma(x, y)).sum::<f64>() / n_cols;
    let right_reach = (CARD_RIGHT_SEARCH * u).round() as i64;
    let far = (right_reach, reach, window);
    let cut = (left, top, side);
    far_edges_hold(column, (left + side, w), row, (top + side, h), far)
        .map_err(|(right, bottom)| SnapMiss::FarEdge { cut, right, bottom })?;
    let outer_top = |(from, to): (u32, u32)| {
        let cols = (left + from).min(w)..(left + to).min(w);
        if cols.is_empty() {
            return None;
        }
        let n_cols = f64::from(cols.len() as u32);
        let row = |y: u32| cols.clone().map(|x| luma(x, y)).sum::<f64>() / n_cols;
        card_edge(row, top, (reach, window, sustain), h)
    };
    let tops = [
        outer_top((side / 16, side / 4)),
        outer_top((side * 3 / 4, side * 15 / 16)),
    ];
    if tops.iter().all(|t| t.is_some_and(|t| t.abs_diff(top) <= 1)) {
        Ok(cut)
    } else {
        Err(SnapMiss::OuterTop { cut, tops })
    }
}

/// Whether the cut's far edges sit on the card's: its right edge at `right`
/// on the column profile `column` of `w` positions, its bottom edge at
/// `bottom` on the row profile `row` of `h` positions. Each is looked for
/// as the first card to gap drop from outside the cut within `right_reach`
/// (right) or `reach` (bottom) of it ([`gap_edge`]), and a drop counts as
/// the card's edge only within 1 px of the cut's: the cut side is
/// [`CARD_SIDE`] rounded, and the painted card can be a px shorter or
/// longer.
///
/// Measured on 55 Practice Range Tabs at 2560x1440 (card at x 1604, y 209,
/// side 228): under the card there is always a dark gap (luma 17 to 21
/// across the card's full width, first gap row at `top + side - 1`, a drop
/// of at least 18), so the bottom drop is required. Right of the card there
/// is no gap: the navy panel (luma 37, the card rim's own fill) runs on for
/// 40 px, then the mid band either falls to about 22 to 24 (38 of the 55)
/// or stays about 29 (17 of the 55). With the navy at 37 to 39, a fall to
/// 22 to 24 is a rise of 13 to 17 back into the panel. Window means of 38
/// vs 22 rise 16, at [`CARD_MIN_STEP`], so that end can qualify as a drop
/// 40 px past the cut (a band of 24 rises 14 and does not). The right scan
/// reaches only [`CARD_RIGHT_SEARCH`] (`right_reach`, 36 px at 1440p) past
/// the cut, short of the panel's end, and that is what keeps the exact cut
/// there; the bottom scan keeps `reach`. Within the right reach the panel
/// gives no drop, so the right drop is only checked when one is found. A
/// left shift of such a card then finds no right drop and passes this
/// check when its top and bottom hold. A right shift of 4 px or more can
/// bring the panel's end inside the 36 px scan, and on a band of 22 that
/// end is a drop past the cut. The right drop otherwise catches a
/// horizontal shift only where the Tab has a gap right of the card
/// ([`snap_to_card`] then checks the outer tops). On 3 of those Tabs the
/// first drop from the right is a dark patch of portrait art 24 to 31 px
/// inside the cut, with the navy panel between it and the cut's edge: a
/// drop inside the cut is the card's edge only when the gap runs on from
/// it to past the cut's edge, as it does when a cut lands right of a card
/// that has a gap beside it.
///
/// `Err((right, bottom))` with the drops found (`None` where none was) when
/// a far edge does not hold.
fn far_edges_hold(
    column: impl Fn(u32) -> f64,
    (right, w): (u32, u32),
    row: impl Fn(u32) -> f64,
    (bottom, h): (u32, u32),
    (right_reach, reach, window): (i64, i64, i64),
) -> Result<(), (Option<u32>, Option<u32>)> {
    let near = |edge: u32, cut: u32| edge.abs_diff(cut) <= 1;
    let bottom_drop = gap_edge(&row, bottom, (reach, window), h);
    let bottom_holds = bottom_drop.is_some_and(|edge| near(edge, bottom));
    let right_drop = gap_edge(&column, right, (right_reach, window), w);
    let right_holds = match right_drop {
        None => true,
        Some(edge) if near(edge, right) => true,
        Some(edge) if edge > right => false,
        Some(edge) => !(edge..=right.min(w - 1)).all(|x| column(x) <= CARD_GAP_MAX_LUMA),
    };
    if bottom_holds && right_holds {
        Ok(())
    } else {
        Err((right_drop, bottom_drop))
    }
}

/// The card's far edge on a luma profile of `len` positions near `cut`:
/// the first gap position after the first card to gap drop scanned from
/// `cut + reach` toward `cut - reach`, that is from outside the card
/// inward. [`card_edge`] on the mirrored profile, with no sustain run (the
/// card side of a far edge is the cut's own inside, already found). `None`
/// when no position drops.
fn gap_edge(
    profile: impl Fn(u32) -> f64,
    cut: u32,
    (reach, window): (i64, i64),
    len: u32,
) -> Option<u32> {
    let mirrored = |i: u32| profile(len - 1 - i);
    card_edge(mirrored, len - cut, (reach, window, window), len).map(|edge| len - edge)
}

/// The card's side in px in a `w` x `h` frame: [`CARD_SIDE`] of the 16:9
/// game rect's height.
pub(crate) fn card_side(w: u32, h: u32) -> u32 {
    let (_, _, _, gh) = stat_tracker::frame::geometry::game_rect_16_9(w, h);
    (CARD_SIDE * f64::from(gh)).round() as u32
}

/// The card's edge on a luma profile (`profile(i)`: the mean luma of
/// column or row `i`) of `len` positions, scanned from `at - reach` toward
/// `at + reach`, that is from outside the card inward: the gap is always
/// outside the card, so the first dark to card step is the card's edge,
/// and brighter portrait art just inside the card is never taken for it.
///
/// A position steps when each of the `window` positions before it (a dark
/// run) is at most [`CARD_GAP_MAX_LUMA`], the `window` from it average at
/// least [`CARD_MIN_LUMA`], and the rise between them is at least
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
/// the card side's mean. A step only counts when every `window` from it
/// within `sustain` positions (cut short at the end of the profile) also
/// averages at least that threshold ([`CARD_SUSTAIN`]): a thin bright line
/// or a narrow UI element with gap after it falls back under it. A bright
/// region outside (the header bar's right end) never steps: while any of
/// its positions is in the dark side that side is too bright, and once a
/// full dark run lies between it and the card, the card side is gap until
/// the card. The dark run is held to that bound at every position, not on
/// its mean: a run that ends on a column of the card itself (a box just
/// past reach inside the card) is not a dark run. `None` when no position
/// steps.
fn card_edge(
    profile: impl Fn(u32) -> f64,
    at: u32,
    (reach, window, sustain): (i64, i64, i64),
    len: u32,
) -> Option<u32> {
    let lo = (i64::from(at) - reach).max(window);
    let hi = (i64::from(at) + reach).min(i64::from(len) - window);
    if lo > hi {
        return None;
    }
    let sustain = sustain.max(window);
    let values: Vec<f64> = (lo - window..(hi + sustain).min(i64::from(len)))
        .map(|i| profile(i as u32))
        .collect();
    let mean = |s: &[f64]| s.iter().sum::<f64>() / s.len() as f64;
    let (w, run) = (window as usize, sustain as usize);
    let (first, bright) = (0..=(hi - lo) as usize).find_map(|edge| {
        let i = edge + w;
        let run_dark = &values[i - w..i];
        let (dark, card) = (mean(run_dark), mean(&values[i..i + w]));
        let bright = dark + ((card - dark) / 2.0).min(CARD_MIN_STEP);
        let steps = run_dark.iter().all(|&v| v <= CARD_GAP_MAX_LUMA)
            && card >= CARD_MIN_LUMA
            && card - dark >= CARD_MIN_STEP;
        let holds = || {
            values[i..(i + run).min(values.len())]
                .windows(w)
                .all(|s| mean(s) >= bright)
        };
        (steps && holds()).then_some((i, bright))
    })?;
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

/// `image` as an RGB PNG, raw pixels, losslessly at the best compression
/// the existing encoder offers (`CompressionType::Best`, Adaptive filter).
/// `None` when it cannot be encoded.
pub(crate) fn encode_png(image: &image::DynamicImage) -> Option<Vec<u8>> {
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

/// The 1080p career panel card side ([`card_side`] of 1920x1080). A larger
/// square card whose Best-encoded PNG is over the 65,536-byte `hero_icon`
/// cap is first downscaled to this, then 16 px smaller at a time down to
/// [`HERO_ICON_BYTE_SHRINK_FLOOR`] only if that PNG is still over.
pub(crate) const HERO_ICON_BYTE_FALLBACK_SIDE: u32 = 171;

/// Smallest side the shrink loop tries. Any 8-bit RGB square of 147 px or
/// less always fits the 65,536-byte cap even uncompressed: the raw PNG
/// image data is 147 x (147 x 3 + 1) = 64,974 bytes (one filter byte per
/// row). After 171 and 155 the next step (139) always fits. The
/// store-layer 8 px min edge is not used here.
pub(crate) const HERO_ICON_BYTE_SHRINK_FLOOR: u32 = 139;

/// Why [`encode_hero_icon`] did not produce a storeable PNG.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HeroIconEncodeMiss {
    /// The PNG encoder failed, at the native side or after a downscale.
    Encode,
    /// Every tried side is still over the cap. For 8-bit RGB this cannot
    /// happen at [`HERO_ICON_BYTE_SHRINK_FLOOR`] or below.
    OverCap,
}

/// Encode a square `hero_icon` crop so the PNG fits the kind's byte cap.
/// At the card's own side this is [`encode_png`] (already Best). If that
/// PNG is over the cap, downscale with Lanczos3 to
/// [`HERO_ICON_BYTE_FALLBACK_SIDE`], then 16 px smaller at a time down to
/// [`HERO_ICON_BYTE_SHRINK_FLOOR`]. `Err` when it cannot be encoded, or
/// when even that floor is still over the cap: the caller must not store
/// it or mark the hero offered.
pub(crate) fn encode_hero_icon(image: &image::DynamicImage) -> Result<Vec<u8>, HeroIconEncodeMiss> {
    debug_assert_eq!(
        image.width(),
        image.height(),
        "hero_icon encode expects a square card"
    );
    let cap = sync::UploadKind::HeroIcon.crop_kind().max_crop_bytes();
    let png = encode_png(image).ok_or(HeroIconEncodeMiss::Encode)?;
    if png.len() <= cap {
        return Ok(png);
    }
    let mut side = HERO_ICON_BYTE_FALLBACK_SIDE
        .min(image.width())
        .min(image.height());
    if side < HERO_ICON_BYTE_SHRINK_FLOOR {
        return Err(HeroIconEncodeMiss::OverCap);
    }
    loop {
        // The native Best encode at this side already missed the cap;
        // a same-size resize would not help.
        if side != image.width() || side != image.height() {
            let resized = image.resize_exact(side, side, image::imageops::FilterType::Lanczos3);
            let png = encode_png(&resized).ok_or(HeroIconEncodeMiss::Encode)?;
            if png.len() <= cap {
                tracing::debug!(
                    target: "scuffed_stat_tracker",
                    from = image.width(),
                    to = side,
                    bytes = png.len(),
                    "hero icon downscaled to fit the byte cap"
                );
                return Ok(png);
            }
        }
        if side <= HERO_ICON_BYTE_SHRINK_FLOOR {
            tracing::debug!(
                target: "scuffed_stat_tracker",
                from = image.width(),
                to = side,
                "hero icon still over the byte cap after downscale"
            );
            return Err(HeroIconEncodeMiss::OverCap);
        }
        side = side.saturating_sub(16).max(HERO_ICON_BYTE_SHRINK_FLOOR);
    }
}

/// Apply an [`encode_hero_icon`] result: keep a fitting PNG, warn on an
/// encode error, and skip an over-cap miss. Either miss releases the hero's
/// claim, so a later Tab of the game can retry.
pub(crate) fn keep_hero_icon_result(
    dir: &Path,
    session: &str,
    hero: &str,
    meta: scuffed_types::api::training_crops::TrainingCropUploadMeta,
    result: Result<Vec<u8>, HeroIconEncodeMiss>,
) {
    match result {
        Ok(png) => keep_encoded_hero_icon(dir, session, hero, meta, png),
        Err(miss) => {
            if matches!(miss, HeroIconEncodeMiss::Encode) {
                sync::warn_crop_lost(dir, sync::UploadKind::HeroIcon);
            }
            unmark_offered(&offer_key(session, sync::UploadKind::HeroIcon, hero));
        }
    }
}

/// Keep an already-encoded `hero_icon` PNG. The hero stays marked offered
/// only when at least one crop was actually kept; a cap refusal or a write
/// error releases its claim, so a later Tab of the same game can retry.
pub(crate) fn keep_encoded_hero_icon(
    dir: &Path,
    session: &str,
    hero: &str,
    meta: scuffed_types::api::training_crops::TrainingCropUploadMeta,
    png: Vec<u8>,
) {
    let kind = sync::UploadKind::HeroIcon;
    match sync::keep_tab_crops(dir, session, kind, vec![(meta, png)]) {
        Some(n) if n > 0 => mark_offered(offer_key(session, kind, hero)),
        _ => unmark_offered(&offer_key(session, kind, hero)),
    }
}

fn keep_hero_icon(
    dir: &Path,
    session: &str,
    (w, h): (u32, u32),
    frame: &image::DynamicImage,
    crop: HeroCrop,
) {
    let Some(meta) = sync::hero_icon_meta(w, h, &crop.hero, crop.score, crop.lead, crop.pack)
    else {
        unmark_offered(&offer_key(session, sync::UploadKind::HeroIcon, &crop.hero));
        return;
    };
    let (x, y, cw, ch) = crop.rect;
    let cut = frame.crop_imm(x, y, cw, ch);
    keep_hero_icon_result(dir, session, &crop.hero, meta, encode_hero_icon(&cut));
}

/// Keep this Tab's `stat_digits` cells. Like [`keep_encoded_hero_icon`],
/// the game stays marked offered only when at least one cell was kept and
/// none was lost; otherwise its claim is released, so a later Tab of the
/// same game can retry.
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
    // keep_tab_crops warns itself on an error. As for a hero icon, the
    // game stays marked offered only when at least one cell was kept and
    // none was lost. A write error, a lost cell, or a save that kept no
    // cell (every cell refused, or consent turned off mid-save) releases
    // the game's claim, so a later Tab of the game may try again; while
    // consent stays off, keep_tab_crops keeps nothing for that Tab either.
    let key = offer_key(session, kind, "");
    match sync::keep_tab_crops(dir, session, kind, crops) {
        Some(_) if lost => {
            sync::warn_crop_lost(dir, kind);
            unmark_offered(&key);
        }
        Some(n) if n > 0 => mark_offered(key),
        _ => unmark_offered(&key),
    }
}
