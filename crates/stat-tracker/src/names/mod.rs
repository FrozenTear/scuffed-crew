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

#[cfg(test)]
mod tests;
