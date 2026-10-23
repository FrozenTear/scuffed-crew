//! Name arbitration: names-v1 (the letter-template reader in
//! [`crate::names`]) or Tesseract (ocr-v1), per board and per row. Moved
//! here from `names/mod.rs` so the cv name reader has no path to Tesseract
//! or the OCR gate; `names` re-exports these at their old paths.

use image::DynamicImage;

use crate::model::{CellOcrResult, RowOcrResult};
use crate::names::{
    BoardMatch, NamePack, NameRecognizer, NameSwitch, PlayerRowMatch, ReaderNames,
    choose_player_row, match_named_rows, player_row, read_row_name,
};
use crate::ocr;

/// The names on a cropped board, and which reader read them.
///
/// The two readers fill `rows` on different scales. With
/// [`NameRecognizer::OcrV1`], a name's `CellOcrResult::confidence` is
/// Tesseract's 0 to 100 and `name_alternates` holds the other name windows.
/// With [`NameRecognizer::NamesV1`], `confidence` is the letter margin times
/// 1000, rounded (16 and up at the default minimum of 0.016, see
/// [`crate::names::Params::min_confidence`]), and `name_alternates` is always empty.
/// Never compare a confidence across the two.
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
// `#[track_caller]` is needed: the ocr-v1 call below is `#[track_caller]`
// too, and the OCR gate records the line that asked when it refuses a read
// (OCR off). With it, a refusal names the daemon's call site, as a direct
// ocr-v1 call does; `tests/integration/names_fallback.rs` checks this.
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
        .filter_map(|i| crate::frame::geometry::crop_player_row(board, i, team_size))
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

/// Find `player` on `board`. `tesseract_rows` is the tracker's current name
/// read for this cv mode (cell rows under ocr-v1, name windows under the
/// new reader). See [`choose_player_row`].
///
/// With the switch on and no pack loaded (sync off, before install, or the
/// read that notices a late install), this does not call [`read_names`].
/// That would be a second Tesseract name read under ocr-v1, and it would
/// match the new rows as a full board instead of the cell rows already in
/// `tesseract_rows`. Those rows are matched with `policy` and tagged
/// [`NameRecognizer::OcrV1`].
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
    let Some(pack) = switch.pack() else {
        return match_named_rows(&tesseract_rows(), player, team_size, policy)
            .map(|name| player_row(name, NameRecognizer::OcrV1));
    };
    let read = read_names(board, team_size, Some(pack.as_ref()));
    choose_player_row(
        player,
        team_size,
        true,
        Some(ReaderNames {
            rows: &read.rows,
            min_confidence: Some(pack.params.min_confidence),
        }),
        policy,
        tesseract_rows,
    )
}
