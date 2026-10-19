//! names-v1: a letter-template player-name reader (Scuffed Vision).
//!
//! A player row goes in, a name and a confidence come out, or no name. The
//! reader finds the line of name-sized white letters in the row, cuts each
//! letter to a 16x28 tile and matches it to the letter tiles of the
//! `names-v1` pack ([`pack`]). An accented letter is matched as its base
//! letter plus its mark. A row abstains rather than guess: no line, an
//! accent the pack does not know, a gap that may be a space, ink no letter
//! explains, or a margin under the pack's `min_confidence`.
//!
//! Without a usable pack nothing here runs: [`read_names`] hands the board to
//! [`crate::ocr::recognize_scoreboard_names_pre_cropped`], today's ocr-v1
//! path, and returns exactly what it returns.

pub mod pack;
mod reader;
mod resize;

pub use pack::{NamePack, PackError, Params};
pub use reader::{Abstain, RawRead};

use std::sync::Arc;

use crate::ocr::{self, CellOcrResult, RowOcrResult};
use image::DynamicImage;

/// Which reader produced a board's names. `as_str` is the value proposed for
/// the upload's optional `name_recognizer` field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameRecognizer {
    NamesV1,
    OcrV1,
}

impl NameRecognizer {
    pub fn as_str(self) -> &'static str {
        match self {
            NameRecognizer::NamesV1 => "names-v1",
            NameRecognizer::OcrV1 => "ocr-v1",
        }
    }
}

/// A name the reader is sure of.
#[derive(Debug, Clone, PartialEq)]
pub struct NameRead {
    pub text: String,
    /// Smallest best-vs-second letter margin; at least the pack's `min_confidence`.
    pub confidence: f32,
}

/// Cap height of the name letters on a board `board_h` pixels high.
/// 6v6 rows are 75/1007 of the board and 5v5 rows 88/1007; the name cap is
/// 22/1007 on 6v6 and scales with the row.
pub fn cap_height(board_h: u32, team_size: usize) -> f64 {
    let base = 22.0 * f64::from(board_h) / 1007.0;
    if team_size == 5 {
        base * 88.0 / 75.0
    } else {
        base
    }
}

fn rgb_of(row: &DynamicImage) -> (Vec<u8>, usize, usize) {
    let rgb = row.to_rgb8();
    let (w, h) = (rgb.width() as usize, rgb.height() as usize);
    (rgb.into_raw(), w, h)
}

/// The reader's view of one row, before the confidence cut. For tools.
pub fn read_row_raw(
    row: &DynamicImage,
    board_h: u32,
    team_size: usize,
    pack: &NamePack,
) -> RawRead {
    let (rgb, w, h) = rgb_of(row);
    reader::read_raw(&rgb, w, h, cap_height(board_h, team_size), pack)
}

/// Letter tiles of a row's name line (tile, width over cap height), for pack tools.
#[doc(hidden)]
pub fn cut_tiles(row: &DynamicImage, board_h: u32, team_size: usize) -> Vec<(Vec<u8>, f32)> {
    let (rgb, w, h) = rgb_of(row);
    reader::cut_tiles(&rgb, w, h, cap_height(board_h, team_size))
}

/// The name in a player row (a [`ocr::preprocess::crop_player_row`] crop), or
/// `None` when the reader is not sure.
pub fn read_row_name(
    row: &DynamicImage,
    board_h: u32,
    team_size: usize,
    pack: &NamePack,
) -> Option<NameRead> {
    let raw = read_row_raw(row, board_h, team_size, pack);
    accept(raw, pack)
}

/// The confidence cut [`read_row_name`] applies to a [`read_row_raw`] result.
pub fn accept(raw: RawRead, pack: &NamePack) -> Option<NameRead> {
    let text = raw.text.ok()?;
    (!text.is_empty() && raw.unaccounted == 0 && raw.confidence >= pack.params.min_confidence)
        .then_some(NameRead {
            text,
            confidence: raw.confidence,
        })
}

/// The names on a cropped board, and which reader read them.
#[derive(Debug)]
pub struct BoardNames {
    pub recognizer: NameRecognizer,
    pub rows: Vec<RowOcrResult>,
}

/// Every row's name on a cropped board.
///
/// With no pack this is [`ocr::recognize_scoreboard_names_pre_cropped`]
/// called as the daemon calls it, unchanged (same reads, same Tesseract
/// calls, same gate accounting). With a pack, each row gets the template
/// reader's name or none, no name windows (`name_alternates` is empty), no
/// Tesseract call, and `confidence` is the margin in thousandths. With OCR
/// off nothing is read either way.
#[track_caller]
pub fn read_names(board: &DynamicImage, team_size: usize, pack: Option<&NamePack>) -> BoardNames {
    let Some(pack) = pack else {
        return BoardNames {
            recognizer: NameRecognizer::OcrV1,
            rows: ocr::recognize_scoreboard_names_pre_cropped(board, team_size),
        };
    };
    if ocr::gate::is_off() {
        return BoardNames {
            recognizer: NameRecognizer::NamesV1,
            rows: Vec::new(),
        };
    }
    let rows = (0..team_size * 2)
        .filter_map(|i| ocr::preprocess::crop_player_row(board, i, team_size))
        .map(|row| RowOcrResult {
            name: read_row_name(&row, board.height(), team_size, pack).map(|n| CellOcrResult {
                value: n.text,
                confidence: (n.confidence * 1000.0).round() as i32,
                suspect: false,
            }),
            name_alternates: Vec::new(),
            stats: Vec::new(),
            mean_confidence: 0,
        })
        .collect();
    BoardNames {
        recognizer: NameRecognizer::NamesV1,
        rows,
    }
}

/// Load the installed pack under `data_dir`, or say why there is none.
pub fn load_installed(data_dir: &std::path::Path) -> Result<NamePack, PackError> {
    NamePack::load(&pack::dir_in(data_dir))
}

/// The name-reader switch for one process. On unless config sets
/// `name_reader = false`. Independent of the cv reader (`reader = "ocr-v1"`
/// or `"new"`).
#[derive(Debug, Clone)]
pub struct NameSwitch {
    pub enabled: bool,
    /// Loaded pack. `None` when the switch is off, or the pack is missing
    /// or refused. [`read_names`] then uses today's Tesseract path.
    pub pack: Option<Arc<NamePack>>,
}

impl NameSwitch {
    pub const fn off() -> Self {
        Self {
            enabled: false,
            pack: None,
        }
    }

    /// Load once at startup. A missing or refused pack is logged here
    /// (template count and the reason, never a player name) and left empty,
    /// so later reads do not log it again and do not add their own fallback.
    pub fn load(enabled: bool, data_dir: &std::path::Path) -> Self {
        if !enabled {
            return Self::off();
        }
        match load_installed(data_dir) {
            Ok(pack) => {
                tracing::info!(templates = pack.templates(), "names-v1 pack loaded");
                Self {
                    enabled: true,
                    pack: Some(Arc::new(pack)),
                }
            }
            Err(err) => {
                tracing::warn!(
                    reason = %err,
                    templates = 0,
                    "names-v1 pack was not loaded"
                );
                Self {
                    enabled: true,
                    pack: None,
                }
            }
        }
    }
}

/// How a list of name rows is turned into the configured player's row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoardMatch {
    /// ocr-v1's cell rows. A short list is still matched.
    Cells,
    /// A name-window read. The index picks a row on the digit reader's
    /// board, so a short list names no row (a crop that fell outside the
    /// board must not shift every later index).
    FullBoard,
}

/// A row reduced to the name reads the player-row matcher looks at.
pub fn name_only(row: &RowOcrResult) -> RowOcrResult {
    RowOcrResult {
        name: row.name.as_ref().map(copy_cell),
        name_alternates: row
            .name_alternates
            .iter()
            .map(|cell| cell.as_ref().map(copy_cell))
            .collect(),
        stats: Vec::new(),
        mean_confidence: 0,
    }
}

fn copy_cell(cell: &CellOcrResult) -> CellOcrResult {
    CellOcrResult {
        value: cell.value.clone(),
        confidence: cell.confidence,
        suspect: cell.suspect,
    }
}

fn blank_name_row() -> RowOcrResult {
    RowOcrResult {
        name: None,
        name_alternates: Vec::new(),
        stats: Vec::new(),
        mean_confidence: 0,
    }
}

/// Thousandths, the same scale [`read_names`] stores for a template margin.
fn confidence_millis(min_confidence: f32) -> i32 {
    (min_confidence * 1000.0).round() as i32
}

/// `confidence` is the thousandths [`read_names`] writes, and
/// `min_confidence` is the pack's own [`Params::min_confidence`].
fn name_meets(cell: &CellOcrResult, min_confidence: f32) -> bool {
    !cell.value.is_empty() && cell.confidence >= confidence_millis(min_confidence)
}

fn row_meets(row: &RowOcrResult, min_confidence: f32) -> bool {
    row.name
        .as_ref()
        .is_some_and(|cell| name_meets(cell, min_confidence))
}

/// Match `rows` against `player`. [`BoardMatch::FullBoard`] refuses a list
/// that is not both teams.
pub fn match_named_rows(
    rows: &[RowOcrResult],
    player: &str,
    team_size: usize,
    policy: BoardMatch,
) -> Option<crate::parse::NameMatch> {
    if policy == BoardMatch::FullBoard && rows.len() != team_size * 2 {
        return None;
    }
    crate::parse::match_player_row_by_name(rows, player)
}

fn sure_rows(rows: &[RowOcrResult], min_confidence: f32) -> Vec<RowOcrResult> {
    rows.iter()
        .map(|row| {
            if row_meets(row, min_confidence) {
                RowOcrResult {
                    name: row.name.as_ref().map(copy_cell),
                    name_alternates: Vec::new(),
                    stats: Vec::new(),
                    mean_confidence: 0,
                }
            } else {
                blank_name_row()
            }
        })
        .collect()
}

/// Keep a template name that meets `min_confidence`. A blank or lower row
/// takes the Tesseract read of that same row, including its other name
/// windows. `None` when the two lists are not the same length.
fn merge_weak_rows(
    template: &[RowOcrResult],
    tesseract: &[RowOcrResult],
    min_confidence: f32,
) -> Option<Vec<RowOcrResult>> {
    if template.len() != tesseract.len() {
        return None;
    }
    Some(
        template
            .iter()
            .zip(tesseract)
            .map(|(temp, tess)| {
                if row_meets(temp, min_confidence) {
                    RowOcrResult {
                        name: temp.name.as_ref().map(copy_cell),
                        name_alternates: Vec::new(),
                        stats: Vec::new(),
                        mean_confidence: 0,
                    }
                } else {
                    name_only(tess)
                }
            })
            .collect(),
    )
}

/// Rows [`read_names`] returned for one board, when the switch is on.
pub struct ReaderNames<'a> {
    pub rows: &'a [RowOcrResult],
    /// `None` when no pack loaded: `rows` are already today's Tesseract
    /// read. `Some` is the pack's own `min_confidence`.
    pub min_confidence: Option<f32>,
}

/// The configured player's row, and which reader produced that row's name.
///
/// [`NameRecognizer::NamesV1`] means the name came from the names pack.
/// [`NameRecognizer::OcrV1`] means it came from Tesseract, including a
/// per-row fallback. No match means no name was read for the upload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayerRowMatch {
    pub name: crate::parse::NameMatch,
    pub recognizer: NameRecognizer,
}

fn player_row(name: crate::parse::NameMatch, recognizer: NameRecognizer) -> PlayerRowMatch {
    PlayerRowMatch { name, recognizer }
}

/// The row of `player` from the name reader, or from `tesseract_rows`.
///
/// The switch off uses `tesseract_rows` and `policy` and does not look at
/// `reader`. The switch on with no pack (`min_confidence` is `None`) uses
/// `reader.rows` as a full board and does not call `tesseract_rows`: that
/// list is already the Tesseract path inside [`read_names`]. The switch on
/// with a pack uses a row whose template name meets the pack threshold.
/// When the player's own row is blank or under that threshold, only the
/// weak rows are filled from `tesseract_rows` and the match is tried again.
/// A full board of sure names that does not contain the player does not
/// call Tesseract.
///
/// The recognizer is [`NameRecognizer::NamesV1`] only when the matched row's
/// name came from the pack. A Tesseract row, including one filled in for a
/// blank or low-confidence row, is [`NameRecognizer::OcrV1`].
pub fn choose_player_row(
    player: &str,
    team_size: usize,
    enabled: bool,
    reader: Option<ReaderNames<'_>>,
    policy: BoardMatch,
    tesseract_rows: impl FnOnce() -> Vec<RowOcrResult>,
) -> Option<PlayerRowMatch> {
    if !enabled {
        return match_named_rows(&tesseract_rows(), player, team_size, policy)
            .map(|name| player_row(name, NameRecognizer::OcrV1));
    }
    let Some(reader) = reader else {
        return match_named_rows(&tesseract_rows(), player, team_size, policy)
            .map(|name| player_row(name, NameRecognizer::OcrV1));
    };
    let Some(min_confidence) = reader.min_confidence else {
        return match_named_rows(reader.rows, player, team_size, BoardMatch::FullBoard)
            .map(|name| player_row(name, NameRecognizer::OcrV1));
    };
    let full = reader.rows.len() == team_size * 2;
    if full
        && let Some(found) = match_named_rows(
            &sure_rows(reader.rows, min_confidence),
            player,
            team_size,
            BoardMatch::FullBoard,
        )
    {
        return Some(player_row(found, NameRecognizer::NamesV1));
    }
    let any_weak = !full
        || reader
            .rows
            .iter()
            .any(|row| !row_meets(row, min_confidence));
    if !any_weak {
        return None;
    }
    let tess = tesseract_rows();
    match merge_weak_rows(reader.rows, &tess, min_confidence) {
        Some(rows) => {
            let found = match_named_rows(&rows, player, team_size, policy)?;
            let from_pack = reader
                .rows
                .get(found.row)
                .is_some_and(|row| row_meets(row, min_confidence));
            let recognizer = if from_pack {
                NameRecognizer::NamesV1
            } else {
                NameRecognizer::OcrV1
            };
            Some(player_row(found, recognizer))
        }
        None => match_named_rows(&tess, player, team_size, policy)
            .map(|name| player_row(name, NameRecognizer::OcrV1)),
    }
}

/// Find `player` on `board`. `tesseract_rows` is the tracker's current name
/// read for this cv mode (cell rows under ocr-v1, name windows under the
/// new reader). See [`choose_player_row`].
pub fn match_player_row(
    board: &DynamicImage,
    team_size: usize,
    player: &str,
    switch: &NameSwitch,
    policy: BoardMatch,
    tesseract_rows: impl FnOnce() -> Vec<RowOcrResult>,
) -> Option<PlayerRowMatch> {
    if !switch.enabled {
        return choose_player_row(player, team_size, false, None, policy, tesseract_rows);
    }
    let read = read_names(board, team_size, switch.pack.as_deref());
    let min_confidence = switch.pack.as_ref().map(|pack| pack.params.min_confidence);
    choose_player_row(
        player,
        team_size,
        true,
        Some(ReaderNames {
            rows: &read.rows,
            min_confidence,
        }),
        policy,
        tesseract_rows,
    )
}

#[cfg(test)]
mod tests;
