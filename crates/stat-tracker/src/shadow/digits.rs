//! TEMPORARY STUB of the shadow digit matcher. The real implementation lands
//! from `feat/tracker-shadow-digits-core`; this file only pins the interface
//! so the worker and log can be built and tested first.

use std::time::Duration;

pub const FIELDS: [&str; 6] = ["E", "A", "D", "DMG", "H", "MIT"];

#[derive(Debug, Clone, PartialEq)]
pub struct CellRead {
    pub value: Option<u32>,
    pub confidence: f32,
    pub suspect: bool,
}

#[derive(Debug, Clone)]
pub struct RowRead {
    pub cells: [CellRead; 6],
}

#[derive(Debug, Clone)]
pub struct BoardRead {
    pub rows: Vec<RowRead>,
    pub elapsed_ms: u32,
}

#[derive(Debug)]
pub enum ShadowError {
    Layout(&'static str),
    OverBudget,
    Unsupported(&'static str),
}

/// Flag a cell when best minus second best score is below this.
pub const SUSPECT_MARGIN: f32 = 0.06;

/// `scoreboard` is exactly what main.rs passes to
/// `ocr::recognize_scoreboard_cells_pre_cropped`. Rows ordered like ocr-v1.
pub fn read_board(
    _scoreboard: &image::DynamicImage,
    _team_size: usize,
    _budget: Duration,
) -> Result<BoardRead, ShadowError> {
    Err(ShadowError::Unsupported("matcher not merged yet"))
}
