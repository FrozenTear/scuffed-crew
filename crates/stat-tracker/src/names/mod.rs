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
//!
//! Toolchain: this module uses `<[T]>::as_chunks`, stable since Rust 1.88,
//! as other modules of the crate already do. The crate as a whole needs
//! Rust 1.90 today through its dependencies (`roaring` 0.11, via
//! surrealdb), so a distro toolchain older than that cannot build it.

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
    /// Smallest best-vs-second letter margin ([`Params::min_confidence`] says
    /// what the scale means); at least the pack's `min_confidence`, 0.016 by
    /// default.
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
///
/// The two readers fill `rows` on different scales. With
/// [`NameRecognizer::OcrV1`], a name's `CellOcrResult::confidence` is
/// Tesseract's 0 to 100 and `name_alternates` holds the other name windows.
/// With [`NameRecognizer::NamesV1`], `confidence` is the letter margin times
/// 1000, rounded (16 and up at the default minimum of 0.016, see
/// [`Params::min_confidence`]), and `name_alternates` is always empty.
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
// ocr-v1 call does; `tests/names_fallback.rs` checks this.
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

/// The name reader's pack, loaded once and then kept.
///
/// A names-v1 pack is about 29 MB and takes about 250 ms to load, so a name
/// read must not load it again. Startup loads the folder when it is already
/// there. When pack sync installs it later, the next name read starts one
/// background load and still uses Tesseract; after that load the cached
/// pack is what every later read uses. A folder swapped in after that is
/// left for the next tracker start.
#[derive(Debug, Clone)]
pub struct LiveNames {
    inner: Arc<LiveInner>,
}

#[derive(Debug)]
struct LiveInner {
    dir: std::path::PathBuf,
    cached: std::sync::RwLock<Option<Arc<NamePack>>>,
    /// `true` once a pack is cached, or the one late load has finished.
    /// Further reads do not look at the folder.
    settled: std::sync::atomic::AtomicBool,
    /// The one late load. `Some` means it has been started, finished or not.
    loader: std::sync::Mutex<Option<std::thread::JoinHandle<()>>>,
    #[cfg(test)]
    spawns: std::sync::atomic::AtomicUsize,
}

impl LiveInner {
    fn cache(&self, pack: NamePack) {
        tracing::info!(templates = pack.templates(), "names-v1 pack loaded");
        *self.cached.write().unwrap_or_else(|err| err.into_inner()) = Some(Arc::new(pack));
        self.settled
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    fn give_up(&self, err: &PackError) {
        tracing::warn!(reason = %err, templates = 0, "names-v1 pack was not loaded");
        self.settled
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

impl LiveNames {
    /// Load now when the folder is already installed. A missing folder stays
    /// empty until [`Self::current`] sees it once.
    pub fn load(dir: std::path::PathBuf) -> Self {
        let inner = LiveInner {
            dir,
            cached: std::sync::RwLock::new(None),
            settled: std::sync::atomic::AtomicBool::new(false),
            loader: std::sync::Mutex::new(None),
            #[cfg(test)]
            spawns: std::sync::atomic::AtomicUsize::new(0),
        };
        match NamePack::load(&inner.dir) {
            Ok(pack) => inner.cache(pack),
            Err(err) => {
                tracing::warn!(reason = %err, templates = 0, "names-v1 pack was not loaded");
            }
        }
        Self {
            inner: Arc::new(inner),
        }
    }

    /// The cached pack. When startup found no folder, the first call that
    /// sees `templates/names-v1/` starts the one late load and returns
    /// `None` for that call. Later calls use the cache and do not load again.
    pub fn current(&self) -> Option<Arc<NamePack>> {
        if self.inner.settled.load(std::sync::atomic::Ordering::SeqCst) {
            return self.cached();
        }
        if self.inner.dir.is_dir() && self.start_late_load() {
            return None;
        }
        self.cached()
    }

    fn cached(&self) -> Option<Arc<NamePack>> {
        self.inner
            .cached
            .read()
            .unwrap_or_else(|err| err.into_inner())
            .clone()
    }

    /// Start the one background load. `true` when this call started it.
    fn start_late_load(&self) -> bool {
        let Ok(mut loader) = self.inner.loader.try_lock() else {
            return false;
        };
        if loader.is_some() {
            return false;
        }
        let inner = Arc::clone(&self.inner);
        let spawned = std::thread::Builder::new()
            .name("name-pack-reload".into())
            .spawn(move || match NamePack::load(&inner.dir) {
                Ok(pack) => inner.cache(pack),
                Err(err) => inner.give_up(&err),
            });
        match spawned {
            Ok(handle) => {
                #[cfg(test)]
                self.inner
                    .spawns
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                *loader = Some(handle);
                true
            }
            Err(err) => {
                tracing::warn!(error = %err, "names-v1 reload not started");
                false
            }
        }
    }

    /// How many late loads have been started (tests).
    #[cfg(test)]
    pub(crate) fn reload_spawns(&self) -> usize {
        self.inner.spawns.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Wait for the late load (tests).
    #[cfg(test)]
    pub(crate) fn wait_reload(&self) {
        let handle = self.inner.loader.lock().unwrap().take();
        if let Some(handle) = handle {
            handle.join().unwrap();
        }
    }
}

/// The name-reader switch for one process. On unless config sets
/// `name_reader = false`. Independent of the cv reader (`reader = "ocr-v1"`
/// or `"new"`).
#[derive(Debug, Clone)]
pub struct NameSwitch {
    pub enabled: bool,
    /// Shared pack. `None` when the switch is off. Loaded once at startup,
    /// or once when the folder appears later.
    live: Option<LiveNames>,
}

impl NameSwitch {
    pub const fn off() -> Self {
        Self {
            enabled: false,
            live: None,
        }
    }

    /// Load at startup. A missing or refused pack is logged here (template
    /// count and the reason, never a player name). A pack that lands later
    /// is loaded once by [`Self::pack`]. Until that load finishes,
    /// [`read_names`] stays on Tesseract. A pack that is already cached is
    /// not loaded again.
    pub fn load(enabled: bool, data_dir: &std::path::Path) -> Self {
        if !enabled {
            return Self::off();
        }
        Self {
            enabled: true,
            live: Some(LiveNames::load(pack::dir_in(data_dir))),
        }
    }

    /// The cached pack. `None` when the switch is off, the folder is missing
    /// or refused, or the one late load has not finished. [`read_names`]
    /// then uses today's Tesseract path.
    pub fn pack(&self) -> Option<Arc<NamePack>> {
        self.live.as_ref().and_then(LiveNames::current)
    }

    /// Late loads started so far (tests). Startup does not count.
    #[cfg(test)]
    pub(crate) fn reload_spawns(&self) -> usize {
        self.live
            .as_ref()
            .map(LiveNames::reload_spawns)
            .unwrap_or(0)
    }

    /// Wait for the late load (tests).
    #[cfg(test)]
    pub(crate) fn wait_reload(&self) {
        if let Some(live) = &self.live {
            live.wait_reload();
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
    let pack = switch.pack();
    let read = read_names(board, team_size, pack.as_deref());
    let min_confidence = pack.as_ref().map(|pack| pack.params.min_confidence);
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
