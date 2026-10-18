pub mod gate;
pub mod preprocess;

use std::cell::RefCell;
use std::collections::HashMap;
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use image::{DynamicImage, ImageEncoder};
use rayon::prelude::*;

use crate::detect::hero_portrait::{ScoreboardPreflight, detect_team_size};

/// How many times a below-floor calibration may be reused before it is
/// dropped and the next board calibrates again.
const BELOW_FLOOR_MAX_REUSES: u8 = 3;

/// Header-anchored column edges must agree within this fraction of board
/// width before a below-floor entry is reused.
const LAYOUT_TOLERANCE: f64 = 0.01;

/// Cached column calibration, keyed by board size and team size.
///
/// `below_floor` finished under the 75% floor. A later read reuses it only
/// when [`layouts_match`] says both sides measured a header layout and
/// those columns agree, and a cheap re-score of the cached columns on the
/// current board is at least the anchored score and the 75% floor. Anything
/// under that is a cold miss. A missing layout on either side is a cold
/// miss. Anchored columns or a fresh sweep that clear the floor replace
/// the entry. Above-floor entries are still re-scored, and kept only while
/// that score clears the floor.
struct CalibCache {
    width: u32,
    height: u32,
    team_size: usize,
    offset: f64,
    below_floor: bool,
    /// Below-floor winner was the header-anchored columns. Recompute those
    /// for this frame instead of applying `offset`.
    anchored: bool,
    /// Times this below-floor entry has been reused.
    reuses: u8,
    /// Header-anchored columns measured with this entry. `None` when the
    /// header did not yield six labels.
    layout: Option<preprocess::StatColumns>,
}

/// True only when both sides measured a header layout and the column edges
/// sit within [`LAYOUT_TOLERANCE`]. A missing layout on either side does
/// not match, so an unreadable header is always a cold miss.
fn layouts_match(
    stored: &Option<preprocess::StatColumns>,
    current: &Option<preprocess::StatColumns>,
) -> bool {
    match (stored, current) {
        (Some(stored_cols), Some(current_cols)) => {
            stored_cols
                .iter()
                .zip(current_cols.iter())
                .all(|(left, right)| {
                    (left.0 - right.0).abs() <= LAYOUT_TOLERANCE
                        && (left.1 - right.1).abs() <= LAYOUT_TOLERANCE
                })
        }
        _ => false,
    }
}

/// Drop a below-floor column calibration. Above-floor entries stay.
/// Called when a game ends so the next game can calibrate a clean board.
pub fn clear_below_floor_column_cache() {
    if let Ok(mut guard) = COLUMN_OFFSET_CACHE.lock()
        && guard.as_ref().is_some_and(|cached| cached.below_floor)
    {
        *guard = None;
    }
}

static COLUMN_OFFSET_CACHE: Mutex<Option<CalibCache>> = Mutex::new(None);

// Offset sweeps started on this thread. `COLUMN_OFFSET_SWEEPS` counts every
// `best_of` call (coarse and fine). `COLUMN_FINE_SWEEPS` counts only the
// fine pass, so a cold below-floor miss can be shown to fine-sweep once.
#[cfg(test)]
thread_local! {
    static COLUMN_OFFSET_SWEEPS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    static COLUMN_FINE_SWEEPS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    static COLUMN_SWEEP_UNDER_FLOOR: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Offsets passed to each `best_of` call, coarse batch then fine batch.
    static COLUMN_SWEEP_BATCHES: RefCell<Vec<Vec<f64>>> = const { RefCell::new(Vec::new()) };
    static CALIB_LOCK_HELD: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// When set, each calibration cell contributes this score instead of OCR.
    static FORCED_CELL_SCORE: std::cell::Cell<Option<i32>> = const { std::cell::Cell::new(None) };
}

/// Serializes calibration in tests so one test's cache write cannot land
/// between another test's two reads of the same board.
#[cfg(test)]
static CALIB_TEST_LOCK: Mutex<()> = Mutex::new(());

/// When set, calibration cells score clean only if their center sits on a
/// dark mark. A static so the offset-sweep pool sees it. Tests hold
/// [`CALIB_TEST_LOCK`].
#[cfg(test)]
static SCORE_DARK_COLUMN_MARKS: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

#[cfg(test)]
struct CalibTestGuard {
    _guard: Option<std::sync::MutexGuard<'static, ()>>,
}

#[cfg(test)]
impl Drop for CalibTestGuard {
    fn drop(&mut self) {
        if self._guard.is_some() {
            CALIB_LOCK_HELD.with(|held| held.set(false));
        }
    }
}

/// Lock calibration for this thread. A caller that already holds it (a test
/// wrapping several reads) gets a no-op guard, so `calibrate_columns` can
/// take the same lock without deadlocking.
#[cfg(test)]
fn calib_test_guard() -> CalibTestGuard {
    if CALIB_LOCK_HELD.with(|held| held.get()) {
        return CalibTestGuard { _guard: None };
    }
    let guard = CALIB_TEST_LOCK
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    CALIB_LOCK_HELD.with(|held| held.set(true));
    CalibTestGuard {
        _guard: Some(guard),
    }
}

/// Process-wide OCR debug dump switch. Set once from config/env at daemon start
/// via [`set_debug_ocr`]; GUI/tools that never call it stay off.
static DEBUG_OCR: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Enable or disable OCR debug PNG dumps for this process.
pub fn set_debug_ocr(enabled: bool) {
    DEBUG_OCR.store(enabled, std::sync::atomic::Ordering::Relaxed);
}

/// Where debug PNGs land. Set from `config.data_dir` at daemon startup;
/// unset callers fall back to the default platform data dir.
static DEBUG_DIR: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Set the directory OCR debug dumps are written to (daemon wires
/// `{data_dir}/debug` so a custom `--data-dir` keeps dumps alongside the data).
pub fn set_debug_dir(dir: PathBuf) {
    if let Ok(mut slot) = DEBUG_DIR.lock() {
        *slot = Some(dir);
    }
}

fn debug_ocr_enabled() -> bool {
    // The daemon wires the config flag through set_debug_ocr at startup;
    // the env var covers GUI/examples that never call it.
    DEBUG_OCR.load(std::sync::atomic::Ordering::Relaxed)
        || matches!(
            std::env::var("STAT_TRACKER_DEBUG_OCR").as_deref(),
            Ok("1") | Ok("true") | Ok("TRUE") | Ok("yes") | Ok("YES")
        )
}

fn debug_dir() -> Option<PathBuf> {
    if !debug_ocr_enabled() {
        return None;
    }
    let configured = DEBUG_DIR.lock().ok().and_then(|slot| slot.clone());
    let dir = match configured {
        Some(d) => d,
        None => dirs::data_dir()?.join("scuffed-stat-tracker").join("debug"),
    };
    let _ = std::fs::create_dir_all(&dir);
    Some(dir)
}

// Dedicated thread pool for OCR — size set once at daemon start via
// [`set_ocr_threads`] (config `ocr_threads` / env / CLI). Default when unset:
// half the cores, clamped 2..=4 (historical auto). Each worker keeps a
// thread-local Tesseract (~23 MB model), so lower = less RAM, higher = faster Tab.
static OCR_THREADS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static OCR_POOL: OnceLock<rayon::ThreadPool> = OnceLock::new();

/// Configure OCR parallelism before the first OCR call. Clamped to 1..=8.
/// No-op if the pool was already built (OnceLock).
pub fn set_ocr_threads(n: usize) {
    let n = n.clamp(1, 8);
    OCR_THREADS.store(n, std::sync::atomic::Ordering::Relaxed);
}

fn ocr_pool_thread_count() -> usize {
    let configured = OCR_THREADS.load(std::sync::atomic::Ordering::Relaxed);
    if configured > 0 {
        return configured.clamp(1, 8);
    }
    // Auto path when callers (tests/examples) never call set_ocr_threads.
    let total = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    (total / 2).clamp(2, 4)
}

/// True once the OCR worker pool exists. It is built on the first OCR call
/// that needs it, and never while OCR is off ([`gate`]).
pub fn pool_started() -> bool {
    OCR_POOL.get().is_some()
}

/// The OCR worker pool, built on first use. It exists only to run
/// Tesseract, so with OCR off ([`gate`]) it refuses and is not built, also
/// for a caller that did not ask the gate itself.
fn ocr_pool() -> Result<&'static rayon::ThreadPool, gate::OcrOff> {
    if gate::is_off() {
        return Err(gate::block(gate::Site {
            file: file!(),
            line: line!(),
            entry: "ocr_pool",
        }));
    }
    Ok(OCR_POOL.get_or_init(|| {
        let threads = ocr_pool_thread_count();
        tracing::info!(ocr_threads = threads, "OCR worker pool ready");
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .thread_name(|i| format!("ocr-{i}"))
            .build()
            .expect("OCR thread pool")
    }))
}

// Per-thread Tesseract instances, keyed by language. Constructing a LepTess
// loads the ~23MB trained-data model (~77ms each); a single Tab capture runs
// hundreds of cell/region OCRs, so creating a fresh instance per call cost
// 20-30s of CPU and lagged the game. Reusing one instance per (thread, lang)
// drops each call to ~6ms. PSM, whitelist and image are reset on every call,
// so a prior call's settings never leak into the next.
thread_local! {
    static TESS_CACHE: RefCell<HashMap<&'static str, leptess::LepTess>> =
        RefCell::new(HashMap::new());
}

/// Tesseract calls made on this thread so far (tests). A path that must run
/// no OCR checks that this count does not move. It is the gate's counter
/// ([`gate::tesseract_calls_on_this_thread`]): one count, kept in one place.
#[cfg(test)]
pub(crate) fn ocr_calls() -> usize {
    gate::tesseract_calls_on_this_thread() as usize
}

/// Run Tesseract on `png` using a thread-local instance for `lang`, returning
/// the recognized text and mean confidence. `whitelist` of `None` (or empty)
/// disables character restriction.
///
/// The only place a `LepTess` is created. With OCR off ([`gate`]) it returns
/// [`gate::OcrOff`] before anything else. The public functions above it ask
/// the gate first, so this check only catches one that forgot to.
fn ocr_with(
    lang: &'static str,
    psm: &str,
    whitelist: Option<&str>,
    png: &[u8],
) -> Result<(String, i32), Box<dyn std::error::Error + Send + Sync>> {
    if gate::is_off() {
        return Err(gate::block(gate::Site {
            file: file!(),
            line: line!(),
            entry: "ocr_with",
        })
        .into());
    }
    // The one place a Tesseract read is counted, for the process and for
    // this thread ([`gate::tesseract_calls`], [`ocr_calls`]).
    gate::note_tesseract_call();
    TESS_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if !cache.contains_key(lang) {
            let path = tessdata_path_for_lang(lang);
            let path_str = path.as_ref().and_then(|p| p.to_str());
            let mut lt = leptess::LepTess::new(path_str, lang)?;
            gate::note_leptess_created();
            // Tesseract prints "Warning: Invalid resolution 0 dpi" and
            // "Estimating resolution as N" to stderr on EVERY call because the
            // in-memory PNGs carry no DPI. Hundreds of cells per Tab capture ×
            // a full evening turned that into thousands of journal lines: on a
            // 50 MB journald cap the daemon's own INFO lines rotated out within
            // ~10 h, which is what made the 2026-09-01 zero-games night
            // undiagnosable from logs. Routing tesseract's debug channel to
            // /dev/null silences it without touching recognition — the DPI
            // estimate itself still runs (setting a fixed source resolution
            // would change the per-image estimate the OCR is calibrated on).
            if let Err(e) = lt.set_variable(leptess::Variable::DebugFile, "/dev/null") {
                tracing::debug!(error = %e, "could not redirect tesseract debug output");
            }
            cache.insert(lang, lt);
        }
        let lt = cache.get_mut(lang).expect("instance just inserted");
        lt.set_variable(leptess::Variable::TesseditPagesegMode, psm)?;
        // Always set the whitelist explicitly so the previous call's whitelist
        // (e.g. a numeric stat cell) does not leak into a name/region read.
        lt.set_variable(
            leptess::Variable::TesseditCharWhitelist,
            whitelist.unwrap_or(""),
        )?;
        lt.set_image_from_mem(png)?;
        let text = lt.get_utf8_text()?;
        let confidence = lt.mean_text_conf();
        Ok((text, confidence))
    })
}

/// Encode a preprocessed grayscale image to an in-memory PNG for Tesseract.
fn encode_png(img: &image::GrayImage) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    let mut buf = Vec::new();
    // Encode from the buffer without cloning the GrayImage into a DynamicImage.
    image::codecs::png::PngEncoder::new(&mut buf).write_image(
        img.as_raw(),
        img.width(),
        img.height(),
        image::ExtendedColorType::L8,
    )?;
    Ok(buf)
}

/// Number of stat columns per scoreboard row: E, A, D, DMG, HLG, MIT.
const STATS_PER_ROW: usize = 6;

/// Minimum ink (black) pixels a prepared cell must contain to be worth OCRing.
/// The smallest real glyph — a thin "1" at 720p after the 2x upscale — measures
/// ~40 ink pixels; a blank cell after the HSV white-mask carries at most a few
/// specks. Skipping Tesseract on empty cells kills both the wasted CPU and the
/// empty-page diagnostics it floods the journal with.
const MIN_CELL_INK_PIXELS: usize = 20;

/// Count ink pixels in a prepared (binarized, black-on-white) cell image.
fn prepared_ink_pixels(img: &image::GrayImage) -> usize {
    img.pixels().filter(|p| p.0[0] < 128).count()
}

/// Width of the edge band, in binarized-cell pixels, scanned for glyph ink by
/// the CG-3 suspect check (`preprocess::has_edge_ink`). ~2 px on the native
/// crop; the 2x upscale of small cells doubles it but the fill fraction is
/// scale-invariant, so the threshold below holds either way.
const EDGE_INK_COLS: u32 = 2;

/// Per-side edge fill fraction above which a stat cell is flagged `suspect`
/// (the window is clipping/bleeding a neighbour digit). Calibrated on the 20
/// real 2026-07-20 drift frames (`examples/edge_ink.rs` offset sweep): a
/// centred DMG window measures edge fill 0.000 across a broad offset band, while
/// drifting it ~1-2 digits so a neighbour glyph is pulled against the margin
/// spikes the fill to 0.167-0.500. This sits in the middle of that empirical gap
/// — well clear of clean 0.000 (room for production anti-aliasing) and below the
/// smallest observed real drift (0.167). See `capture_gate` module docs for the
/// fixture path.
const EDGE_INK_THRESHOLD: f64 = 0.12;

/// Edge-ink fraction of a raw cell crop, measured on the borderless binarized
/// image the OCR path also uses. Exposed for the drift-fixture threshold
/// validation harness (`examples/edge_ink.rs`); returns `(left, right)`.
pub fn cell_edge_ink(img: &DynamicImage) -> (f64, f64) {
    preprocess::edge_ink_fraction(&preprocess::prepare_cell_binary(img), EDGE_INK_COLS)
}

#[derive(Debug)]
pub struct OcrResult {
    pub raw_text: String,
    pub confidence: i32,
}

#[derive(Debug)]
pub struct CellOcrResult {
    pub value: String,
    pub confidence: i32,
    /// Glyph ink touches a vertical crop edge (CG-3 clip/bleed). Suspect values
    /// are still parsed — they may be right — but the capture gate strips their
    /// corroboration/un-latch influence (see `capture_gate::apply_gate`).
    pub suspect: bool,
}

#[derive(Debug)]
pub struct RowOcrResult {
    /// Read of name window 0 ([`preprocess::name_windows`]).
    pub name: Option<CellOcrResult>,
    /// Reads of the other name windows: `name_alternates[k]` is window
    /// `k + 1`, `None` where that read failed, so the index always names the
    /// window. Empty for 6v6, which has one window. The row matcher scores
    /// `name` and these together and keeps the best.
    pub name_alternates: Vec<Option<CellOcrResult>>,
    pub stats: Vec<CellOcrResult>,
    pub mean_confidence: i32,
}

fn tessdata_path_for_lang(lang: &str) -> Option<PathBuf> {
    let custom = dirs::data_dir().map(|d| d.join("scuffed-stat-tracker").join("tessdata"));
    if let Some(ref custom_dir) = custom
        && custom_dir.join(format!("{lang}.traineddata")).exists()
    {
        return Some(custom_dir.clone());
    }
    crate::setup::find_system_traineddata(lang)
}

static TESSDATA_LANG: OnceLock<&'static str> = OnceLock::new();

/// True once this process looked for the `koverwatch` tessdata. The first
/// OCR call that needs the language does it, and never while OCR is off
/// ([`gate`]).
pub fn tessdata_lang_probed() -> bool {
    TESSDATA_LANG.get().is_some()
}

/// The tessdata language for scoreboard text, looked up on first use.
/// With OCR off ([`gate`]) it refuses and the tessdata folder is not
/// looked at, also for a caller that did not ask the gate itself.
fn tessdata_lang() -> Result<&'static str, gate::OcrOff> {
    if gate::is_off() {
        return Err(gate::block(gate::Site {
            file: file!(),
            line: line!(),
            entry: "tessdata_lang",
        }));
    }
    Ok(TESSDATA_LANG.get_or_init(|| {
        let has_koverwatch = dirs::data_dir()
            .map(|d| {
                d.join("scuffed-stat-tracker")
                    .join("tessdata")
                    .join("koverwatch.traineddata")
                    .exists()
            })
            .unwrap_or(false);
        if has_koverwatch { "koverwatch" } else { "eng" }
    }))
}

#[track_caller]
pub fn recognize_region(
    img: &DynamicImage,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    gate::check("recognize_region")?;
    let png_buf = encode_png(&preprocess::prepare(img))?;
    let (text, _conf) = ocr_with(tessdata_lang()?, "7", None, &png_buf)?;
    Ok(text)
}

// Stretch retries this thread has OCR'd, and the microseconds of the latest
// one. Tests read them to prove a non-Tab frame never starts the second pass.
// Thread-local so parallel tests do not count each other's retries.
#[cfg(test)]
thread_local! {
    pub(crate) static MAP_LABEL_RETRY_RUNS: std::cell::Cell<usize> =
        const { std::cell::Cell::new(0) };
    pub(crate) static MAP_LABEL_RETRY_LAST_US: std::cell::Cell<u64> =
        const { std::cell::Cell::new(0) };
}

/// Read the Tab top-bar map label from a full frame.
///
/// `preflight` is the capture path's existing scoreboard result. The stretch
/// retry runs only for [`ScoreboardPreflight::Ready`]. This function does not
/// scan the board again.
///
/// The first pass is [`recognize_region`] on [`preprocess::crop_map_name`],
/// the same preprocess a fully lit label already uses. A label caught mid
/// fade-in is dim grey. The white mask drops it, and Tesseract returns junk.
/// That miss is retried once, on a brightness stretch inverted to
/// black-on-white. The stretch uses the channel mean only. A crop whose
/// brightness range or bright-pixel coverage is too low to hold a label is
/// not stretched. The first read is what comes back unless the retry matches
/// a map.
///
/// With OCR off ([`gate`]) the label is not read and the text is empty.
#[track_caller]
pub fn recognize_map_label(frame: &DynamicImage, preflight: ScoreboardPreflight) -> String {
    if gate::check("recognize_map_label").is_err() {
        return String::new();
    }
    read_map_crop(
        &preprocess::crop_map_name(frame),
        matches!(preflight, ScoreboardPreflight::Ready(_)),
    )
}

/// First read of a map-name crop, then at most one stretch retry.
///
/// `on_tab` is the scoreboard preflight. A gameplay frame, a menu, or any
/// other non-Tab image keeps the first read and does not pay for the retry.
#[track_caller]
pub(crate) fn read_map_crop(crop: &DynamicImage, on_tab: bool) -> String {
    if gate::check("read_map_crop").is_err() {
        return String::new();
    }
    let first = recognize_region(crop).unwrap_or_default();
    if crate::parse::match_map_in_text(&first).is_some() || !on_tab {
        return first;
    }
    let Some(retry) = preprocess::map_label_retry_image(crop) else {
        return first;
    };
    let started = Instant::now();
    let second = recognize_prepared(&retry, "7", None).unwrap_or_default();
    let elapsed_us = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX);
    tracing::debug!(elapsed_us, "map label stretch retry");
    #[cfg(test)]
    {
        MAP_LABEL_RETRY_RUNS.with(|runs| runs.set(runs.get() + 1));
        MAP_LABEL_RETRY_LAST_US.with(|last| last.set(elapsed_us));
    }
    if crate::parse::match_map_in_text(&second).is_some() {
        second
    } else {
        first
    }
}

/// OCR a multi-line screen region (phase headers like "BAN HEROES 13" or
/// "VOTE FOR A MAP") as sparse text. Feeds Tesseract the plain grayscale crop:
/// the scoreboard-tuned `prepare()` white-mask erases stylized/red UI text
/// (measured on the 2026-07 ban-patch screens), and PSM 7 assumes a single
/// line. PSM 11 finds the scattered header + subtitle lines instead.
#[track_caller]
pub fn recognize_sparse_region(
    img: &DynamicImage,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    gate::check("recognize_sparse_region")?;
    let png_buf = encode_png(&img.to_luma8())?;
    let (text, _conf) = ocr_with(tessdata_lang()?, "11", None, &png_buf)?;
    Ok(text)
}

/// OCR an already-preprocessed grayscale image, skipping the scoreboard-tuned
/// `prepare()` step. Used for regions (like the VICTORY/DEFEAT title) that need
/// their own binarization. `whitelist` of `None` disables character restriction.
#[track_caller]
pub fn recognize_prepared(
    img: &image::GrayImage,
    psm: &str,
    whitelist: Option<&str>,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    gate::check("recognize_prepared")?;
    let png_buf = encode_png(img)?;
    let (text, _conf) = ocr_with(tessdata_lang()?, psm, whitelist, &png_buf)?;
    Ok(text)
}

/// Like [`recognize_prepared`] but pins the Tesseract language. End-title
/// words are Latin UI ("VICTORY"), not the scoreboard font `koverwatch`
/// is trained on — forcing `eng` is the difference between "VCTORY!" and "VOY!".
#[track_caller]
pub fn recognize_prepared_lang(
    img: &image::GrayImage,
    psm: &str,
    whitelist: Option<&str>,
    lang: &'static str,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    gate::check("recognize_prepared_lang")?;
    let png_buf = encode_png(img)?;
    let (text, _conf) = ocr_with(lang, psm, whitelist, &png_buf)?;
    Ok(text)
}

/// Per-cell OCR: extract and recognize a single stat cell.
/// Uses PSM 7 (single text line) for short numeric strings.
/// `whitelist` controls which characters Tesseract will consider.
#[track_caller]
pub fn recognize_cell(
    img: &DynamicImage,
) -> Result<CellOcrResult, Box<dyn std::error::Error + Send + Sync>> {
    gate::check("recognize_cell")?;
    recognize_cell_with_whitelist(img, "0123456789,")
}

fn recognize_cell_with_whitelist(
    img: &DynamicImage,
    whitelist: &str,
) -> Result<CellOcrResult, Box<dyn std::error::Error + Send + Sync>> {
    let bright = recognize_cell_bright(img, whitelist)?;
    if !bright.value.is_empty() {
        return Ok(bright);
    }
    // The bright path came back empty. Either the HSV mask found almost no
    // ink, or it kept the ring (the Dorado stroke cores are about 171-186
    // and neutral) and Tesseract still returned nothing. A dim 0 can be
    // there in both cases, and the geometric check is what recovers it.
    // Anything else stays empty: a blank cell is not a zero, and a white
    // digit is not a ring.
    match preprocess::dim_zero_glyph(img, EDGE_INK_COLS) {
        Some(hit) => Ok(CellOcrResult {
            value: "0".to_string(),
            confidence: hit.confidence,
            suspect: hit.touches_edge || bright.suspect,
        }),
        None => Ok(bright),
    }
}

/// The bright-ink path alone: HSV white mask, binarize, Tesseract, with no
/// dim-zero recovery. Column calibration scores layouts with it (see
/// [`count_valid_cells`]).
fn recognize_cell_bright(
    img: &DynamicImage,
    whitelist: &str,
) -> Result<CellOcrResult, Box<dyn std::error::Error + Send + Sync>> {
    let first = read_cell_binary(&preprocess::prepare_cell_binary(img), whitelist)?;
    if !first.value.is_empty() {
        return Ok(first);
    }
    // The PR 158 upscale cap moved this cell off the factor it used to get,
    // and the capped read is empty. Read it again at the old factor, so the
    // cap cannot lose a cell the old factor read. See
    // `preprocess::prepare_cell_binary_uncapped` for the cells and factors.
    if let Some(binary) = preprocess::prepare_cell_binary_uncapped(img) {
        let retry = read_cell_binary(&binary, whitelist)?;
        if !retry.value.is_empty() {
            return Ok(retry);
        }
    }
    Ok(first)
}

/// Tesseract on one binarized cell. Measures edge ink on the borderless
/// form, OCRs the bordered one.
fn read_cell_binary(
    binary: &image::GrayImage,
    whitelist: &str,
) -> Result<CellOcrResult, Box<dyn std::error::Error + Send + Sync>> {
    if prepared_ink_pixels(binary) < MIN_CELL_INK_PIXELS {
        return Ok(CellOcrResult {
            value: String::new(),
            confidence: 0,
            suspect: false,
        });
    }
    let suspect = preprocess::has_edge_ink(binary, EDGE_INK_COLS, EDGE_INK_THRESHOLD);
    let png_buf = encode_png(&preprocess::add_cell_border(binary))?;
    let (text, confidence) = ocr_with("eng", "7", Some(whitelist), &png_buf)?;
    Ok(CellOcrResult {
        value: text.trim().to_string(),
        confidence,
        suspect,
    })
}

/// Recognize a player name cell.
#[track_caller]
pub fn recognize_name(
    img: &DynamicImage,
) -> Result<CellOcrResult, Box<dyn std::error::Error + Send + Sync>> {
    gate::check("recognize_name")?;
    let prepared = preprocess::prepare_name_cell(img);
    // Name cells feed only the fuzzy row matcher, never the numeric gate, so the
    // edge-ink suspect flag is not meaningful here.
    let first = if prepared_ink_pixels(&prepared) < MIN_CELL_INK_PIXELS {
        CellOcrResult {
            value: String::new(),
            confidence: 0,
            suspect: false,
        }
    } else {
        let png_buf = encode_png(&prepared)?;
        let (text, confidence) = ocr_with(tessdata_lang()?, "7", None, &png_buf)?;
        CellOcrResult {
            value: text.trim().to_string(),
            confidence,
            suspect: false,
        }
    };
    if !first.value.is_empty() {
        return Ok(first);
    }

    // Fallback for cosmetic nameplates: gradient plates defeat the HSV white
    // mask (tinted italic text → zero ink). A hard high-threshold grayscale
    // binarization keeps the bright glyphs; fixture 2026-07-16 row 0 reads
    // "S Froze" this way — plenty for the fuzzy row matcher.
    let hard = preprocess::prepare_name_cell_hard_threshold(img);
    if prepared_ink_pixels(&hard) < MIN_CELL_INK_PIXELS {
        return Ok(first);
    }
    let png_buf = encode_png(&hard)?;
    let (text, confidence) = ocr_with(tessdata_lang()?, "7", None, &png_buf)?;
    Ok(CellOcrResult {
        value: text.trim().to_string(),
        confidence,
        suspect: false,
    })
}

/// Per-row OCR: extract name + all stat cells from a single player row.
///
/// With OCR off ([`gate`]) nothing is read: no name, and six empty cells.
#[track_caller]
pub fn recognize_row(
    row_img: &DynamicImage,
    columns: &preprocess::StatColumns,
    team_size: usize,
) -> RowOcrResult {
    if gate::check("recognize_row").is_err() {
        return unread_row();
    }
    recognize_row_with(row_img, columns, team_size, Names::Read)
}

/// Whether the per-row OCR reads the name windows (OCR site T4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Names {
    /// ocr-v1: every name window is read.
    Read,
    /// `reader = "new"`: no name window is cropped or read by the per-cell
    /// pass. The own row comes from [`crate::detect::own_row`]. This is not
    /// "no name is read on the Tab": the lazy full-board read still covers
    /// the whole board when the cell path could not supply everything, and
    /// a board with neither a seed match nor a highlight has its name
    /// windows read by [`recognize_scoreboard_names_pre_cropped`].
    ///
    /// Since alpha.7 O10 the new reader does not run the per-cell pass at
    /// all (its stats are the digit reader's; the daemon hands over
    /// [`unread_rows`]), so the daemon has no caller of `Skip` left. It
    /// stays for tools and tests.
    Skip,
}

fn recognize_row_with(
    row_img: &DynamicImage,
    columns: &preprocess::StatColumns,
    team_size: usize,
    names: Names,
) -> RowOcrResult {
    let Ok(pool) = ocr_pool() else {
        return unread_row();
    };
    // One read per name window. 5v5 has a window per table layout (in-match,
    // in-match with perks, centred); which one holds the name is only known
    // once the configured name is compared, so all of them are read.
    let mut name_reads: Vec<Option<CellOcrResult>> = match names {
        Names::Read => preprocess::crop_name_cells(row_img, team_size)
            .par_iter()
            .map(|crop| recognize_name(crop).ok())
            .collect(),
        Names::Skip => Vec::new(),
    };
    let name = if name_reads.is_empty() {
        None
    } else {
        name_reads.remove(0)
    };
    // Failed reads stay as `None` so each index keeps naming its window.
    let name_alternates = name_reads;

    // Produce exactly STATS_PER_ROW positional cells (E, A, D, DMG, HLG, MIT).
    // Run in parallel — each cell creates its own Tesseract instance so there is
    // no shared state. Index order is preserved by collecting into a fixed-size array.
    let mut stats: Vec<(usize, CellOcrResult)> = pool.install(|| {
        (0..STATS_PER_ROW)
            .into_par_iter()
            .map(|i| {
                let whitelist = if i < 3 { "0123456789" } else { "0123456789," };
                let result = preprocess::crop_stat_cell(row_img, i, columns)
                    .and_then(|cell| recognize_cell_with_whitelist(&cell, whitelist).ok())
                    .unwrap_or_else(|| CellOcrResult {
                        value: String::new(),
                        confidence: 0,
                        suspect: false,
                    });
                (i, result)
            })
            .collect()
    });
    stats.sort_unstable_by_key(|(i, _)| *i);
    let stats: Vec<CellOcrResult> = stats.into_iter().map(|(_, c)| c).collect();

    // Average only over cells that actually produced text — empty placeholders
    // would otherwise drag a good row's confidence toward zero.
    let confidences: Vec<i32> = stats
        .iter()
        .filter(|s| !s.value.is_empty())
        .map(|s| s.confidence)
        .collect();
    let mean_confidence = if confidences.is_empty() {
        0
    } else {
        confidences.iter().sum::<i32>() / confidences.len() as i32
    };

    tracing::debug!(
        name = name.as_ref().map(|n| n.value.as_str()).unwrap_or("?"),
        stat_count = stats.len(),
        mean_confidence,
        "row OCR complete"
    );

    RowOcrResult {
        name,
        name_alternates,
        stats,
        mean_confidence,
    }
}

/// A row nothing was read from: no name, and six empty cells.
fn unread_row() -> RowOcrResult {
    RowOcrResult {
        name: None,
        name_alternates: Vec::new(),
        stats: (0..STATS_PER_ROW)
            .map(|_| CellOcrResult {
                value: String::new(),
                confidence: 0,
                suspect: false,
            })
            .collect(),
        mean_confidence: 0,
    }
}

/// Full scoreboard OCR using per-cell extraction.
/// Returns structured results per row with higher confidence than full-image OCR.
///
/// With OCR off ([`gate`]) this and the two functions below read nothing
/// and return no rows. The board is not cropped, the columns are not
/// calibrated and the OCR pool is not started.
#[track_caller]
pub fn recognize_scoreboard_cells(img: &DynamicImage) -> Vec<RowOcrResult> {
    if gate::check("recognize_scoreboard_cells").is_err() {
        return Vec::new();
    }
    recognize_scoreboard_cells_with_team_size(img, None)
}

/// OCR with explicit team size override. Pass None to auto-detect.
#[track_caller]
pub fn recognize_scoreboard_cells_with_team_size(
    img: &DynamicImage,
    team_size_override: Option<usize>,
) -> Vec<RowOcrResult> {
    if gate::check("recognize_scoreboard_cells_with_team_size").is_err() {
        return Vec::new();
    }
    let cropped = preprocess::crop_scoreboard(img);
    let team_size = team_size_override.unwrap_or_else(|| {
        let detected = detect_team_size(&cropped);
        tracing::debug!(detected, "auto-detected team size");
        detected
    });
    recognize_scoreboard_cells_pre_cropped(&cropped, team_size)
}

/// OCR a scoreboard that the caller has already cropped (and optionally already
/// measured for team size). Avoids a second full-frame scoreboard crop (P7).
#[track_caller]
pub fn recognize_scoreboard_cells_pre_cropped(
    cropped: &DynamicImage,
    team_size: usize,
) -> Vec<RowOcrResult> {
    if gate::check("recognize_scoreboard_cells_pre_cropped").is_err() {
        return Vec::new();
    }
    recognize_scoreboard_cells_pre_cropped_with(cropped, team_size, Names::Read)
}

/// [`recognize_scoreboard_cells_pre_cropped`] with the name windows read or
/// skipped. With [`Names::Skip`] every row's `name` is `None` and
/// `name_alternates` is empty; the stat cells are read exactly the same.
///
/// With OCR off ([`gate`]) it reads nothing and returns no rows, like the
/// functions above.
#[track_caller]
pub fn recognize_scoreboard_cells_pre_cropped_with(
    cropped: &DynamicImage,
    team_size: usize,
    names: Names,
) -> Vec<RowOcrResult> {
    if gate::check("recognize_scoreboard_cells_pre_cropped_with").is_err() {
        return Vec::new();
    }
    let Ok(pool) = ocr_pool() else {
        return Vec::new();
    };
    let total_rows = team_size * 2;
    let columns = calibrate_columns(cropped, team_size);

    tracing::debug!(team_size, total_rows, ?columns, "scoreboard layout");

    // Crop all rows first (fast, no OCR), then OCR all rows in parallel.
    let row_images: Vec<(usize, DynamicImage)> = (0..total_rows)
        .filter_map(|i| preprocess::crop_player_row(cropped, i, team_size).map(|img| (i, img)))
        .collect();

    let mut results: Vec<(usize, RowOcrResult)> = pool.install(|| {
        row_images
            .par_iter()
            .map(|(idx, row_img)| {
                (
                    *idx,
                    recognize_row_with(row_img, &columns, team_size, names),
                )
            })
            .collect()
    });

    results.sort_unstable_by_key(|(idx, _)| *idx);
    let results: Vec<RowOcrResult> = results.into_iter().map(|(_, r)| r).collect();

    if let Some(dir) = debug_dir() {
        // Prefer dumping the already-cropped board and row crops; full stage
        // re-preprocess only when the debug flag is on (still expensive, but opt-in).
        preprocess::save_debug_stages(cropped, &dir);
        for (idx, row_img) in &row_images {
            let _ = row_img.save(dir.join(format!("row_{idx:02}.png")));
        }
    }

    results
}

/// The name windows of every row, and nothing else: no stat cell is read.
///
/// `reader = "new"` skips the name windows in the per-cell pass
/// ([`Names::Skip`]). This is its fallback for a board on which neither an
/// own-name seed nor the highlight names a row: the same crops and the same
/// read ocr-v1 makes, so [`crate::parse::match_player_row_by_name`] sees
/// what it sees on main. Rows are in board order; `stats` is empty.
///
/// With OCR off ([`gate`]) it reads nothing and returns no rows: the caller
/// has no name fallback then, and should not ask
/// (`own_row_name_fallback` in the daemon returns before this).
#[track_caller]
pub fn recognize_scoreboard_names_pre_cropped(
    cropped: &DynamicImage,
    team_size: usize,
) -> Vec<RowOcrResult> {
    if gate::check("recognize_scoreboard_names_pre_cropped").is_err() {
        return Vec::new();
    }
    let Ok(pool) = ocr_pool() else {
        return Vec::new();
    };
    let row_images: Vec<DynamicImage> = (0..team_size * 2)
        .filter_map(|i| preprocess::crop_player_row(cropped, i, team_size))
        .collect();
    pool.install(|| {
        row_images
            .par_iter()
            .map(|row_img| {
                let mut reads: Vec<Option<CellOcrResult>> =
                    preprocess::crop_name_cells(row_img, team_size)
                        .par_iter()
                        .map(|crop| recognize_name(crop).ok())
                        .collect();
                let name = if reads.is_empty() {
                    None
                } else {
                    reads.remove(0)
                };
                RowOcrResult {
                    name,
                    name_alternates: reads,
                    stats: Vec::new(),
                    mean_confidence: 0,
                }
            })
            .collect()
    })
}

/// The rows of a board nothing was read on: `2 * team_size` rows in board
/// order with no name and no stat cell. `reader = "new"` hands these to the
/// capture path in place of the per-cell pass: its stats are the digit
/// reader's and its own row comes from [`crate::detect::own_row`], so no
/// OCR runs for them, and this starts no OCR worker.
pub fn unread_rows(team_size: usize) -> Vec<RowOcrResult> {
    (0..team_size * 2)
        .map(|_| RowOcrResult {
            name: None,
            name_alternates: Vec::new(),
            stats: Vec::new(),
            mean_confidence: 0,
        })
        .collect()
}

/// Column calibration (CG-4 revision). Preferred path: anchor each stat
/// window to its own header label center via
/// [`preprocess::columns_from_header_groups`] — per-frame, per-column,
/// layout- and resolution-independent. The 6v6 board compresses the table by
/// more than a pure translation, so the legacy single-global-offset comb
/// cannot align all six columns at once; the residual concentrated on the
/// rightmost column and decapitated MIT / injected its lead digit into HLG
/// (the 2026-07-22 Numbani 22994 defect).
///
/// Fallback path (headers unreadable or anchored windows score poorly): one
/// offset sweep around the header-detected offset. The result is cached per
/// board size and team size. An above-floor entry is reused while a quick
/// re-score holds at least 75% of the max probe score. A cold miss that
/// finishes under that floor still runs one fine sweep around the coarse
/// winner, then caches that result with the measured header layout. A later
/// read reuses that entry only when both reads measured a header layout,
/// those layouts match, and the cached columns still score at least as
/// well as the anchored columns and at least the 75% floor on this board.
/// A miss on any of those is a cold miss. Anchored columns or a fresh
/// sweep that clear the floor replace the below-floor entry. A headerless
/// cold miss runs the pre-cache coarse and fine sweep, including the
/// same-size offset as the tie-break anchor. A board that already clears
/// the floor keeps the same columns.
fn calibrate_columns(scoreboard: &DynamicImage, team_size: usize) -> preprocess::StatColumns {
    #[cfg(test)]
    let _calib_test_guard = calib_test_guard();
    let header_groups = preprocess::header_label_groups(scoreboard);
    let header_offset = preprocess::offset_from_header_groups(&header_groups, scoreboard.width());
    let (board_w, board_h) = (scoreboard.width(), scoreboard.height());

    let probe_rows: Vec<_> = [0usize, 1, team_size]
        .iter()
        .filter_map(|&i| preprocess::crop_player_row(scoreboard, i, team_size))
        .collect();

    if probe_rows.is_empty() {
        return preprocess::columns_from_header_groups(&header_groups, board_w)
            .unwrap_or_else(|| preprocess::columns_with_offset(header_offset));
    }

    // CELL_SCORE_CLEAN per clean valid cell; suspect cells (edge ink) subtract
    // CELL_SCORE_SUSPECT so a drifted geometry that still reads digits scores
    // strictly below a clean-margin one (continuous penalty, CG-4 review).
    let max_score = (probe_rows.len() * 6) as i32 * CELL_SCORE_CLEAN;

    // Score a candidate column layout by how cleanly the probe-row cells OCR.
    // Each candidate is independent, so the sweeps are evaluated in parallel on
    // the OCR pool — calibration was previously ~288 serial OCR calls and
    // dominated capture latency.
    let score_cols = |cols: &preprocess::StatColumns| -> i32 {
        probe_rows.iter().map(|r| count_valid_cells(r, cols)).sum()
    };
    let score = |offset: f64| -> i32 { score_cols(&preprocess::columns_with_offset(offset)) };
    let anchored_floor = (max_score * 3 / 4).max(1);
    let anchored_cols = preprocess::columns_from_header_groups(&header_groups, board_w);
    let remember = |offset: f64, below_floor: bool, anchored: bool| {
        if let Ok(mut guard) = COLUMN_OFFSET_CACHE.lock() {
            *guard = Some(CalibCache {
                width: board_w,
                height: board_h,
                team_size,
                offset,
                below_floor,
                anchored,
                reuses: 0,
                layout: anchored_cols,
            });
        }
    };

    // A live below-floor entry still yields to header-anchored columns that
    // clear the floor. That is the cheap probe: one layout, not the sweep.
    // Reuse also requires the measured layout to match and a re-score of
    // the cached columns on this board. The cached columns are a cold miss
    // when they score below the anchored columns or below the floor, and
    // that miss does not consume a reuse. A result that clears the floor
    // replaces the below-floor entry.
    let below_floor_hit = COLUMN_OFFSET_CACHE.lock().ok().and_then(|guard| {
        let cached = guard.as_ref()?;
        if cached.below_floor
            && cached.width == board_w
            && cached.height == board_h
            && cached.team_size == team_size
            && layouts_match(&cached.layout, &anchored_cols)
        {
            Some((cached.offset, cached.anchored, cached.reuses))
        } else {
            None
        }
    });
    let mut known_anchored_score: Option<i32> = None;
    if let Some((offset, use_anchored, reuses)) = below_floor_hit {
        if reuses >= BELOW_FLOOR_MAX_REUSES {
            clear_below_floor_column_cache();
        } else {
            known_anchored_score = anchored_cols.map(|cols| score_cols(&cols));
            if let Some(cols) = anchored_cols
                && known_anchored_score.is_some_and(|score| score >= anchored_floor)
            {
                clear_below_floor_column_cache();
                tracing::debug!(
                    "header-anchored columns cleared the floor: dropping below-floor cache"
                );
                return cols;
            }
            let (cached_cols, cached_score) = if use_anchored {
                if let Some(cols) = anchored_cols {
                    (cols, known_anchored_score.unwrap_or(0))
                } else {
                    let cols = preprocess::columns_with_offset(offset);
                    let score = score_cols(&cols);
                    (cols, score)
                }
            } else {
                let cols = preprocess::columns_with_offset(offset);
                let score = score_cols(&cols);
                (cols, score)
            };
            let worse_than_anchored =
                known_anchored_score.is_some_and(|score| cached_score < score);
            if !worse_than_anchored && cached_score >= anchored_floor {
                // These columns clear the floor on this board. Drop the
                // below-floor mark and keep them, the same as a fresh
                // calibration that clears the floor.
                remember(offset, false, use_anchored);
                tracing::debug!(
                    cached_score,
                    floor = anchored_floor,
                    "cached columns cleared the floor: replacing below-floor entry"
                );
                return cached_cols;
            }
            tracing::debug!(
                cached_score,
                floor = anchored_floor,
                "cached columns scored too low on this board: cold miss"
            );
        }
    }

    // Header-anchored per-column windows: accept immediately when they score
    // at least the same 75%-of-max floor the cached-offset reuse path uses.
    // Clearing the floor also drops a below-floor entry left by another board.
    let anchored = anchored_cols.map(|cols| {
        let score = known_anchored_score.unwrap_or_else(|| score_cols(&cols));
        (cols, score)
    });
    if let Some((cols, anchored_score)) = &anchored {
        if *anchored_score >= anchored_floor {
            clear_below_floor_column_cache();
            tracing::debug!(
                anchored_score,
                max_score,
                "using header-anchored per-column calibration"
            );
            return *cols;
        }
        tracing::debug!(
            anchored_score,
            floor = anchored_floor,
            "header-anchored columns scored low: trying offset sweep"
        );
    }

    // Anchor for tie-breaking: the last known-good offset for this geometry, if
    // any. On an equal score we prefer the candidate closest to it rather than a
    // blanket leftward bias (which latched onto the hard-left header mis-seed —
    // see the step-0 drift analysis).
    let mut anchor: Option<f64> = None;

    // Reuse last good offset when the board geometry matches and validity has
    // not degraded (threshold: ≥75% of max probe cells still clean).
    // A headerless cold miss still takes the previous same-size offset as
    // the tie-break anchor, which is what the pre-cache sweep did. Boards
    // that measured a layout do not inherit a below-floor offset that way.
    if let Ok(guard) = COLUMN_OFFSET_CACHE.lock()
        && let Some(cached) = guard.as_ref()
        && cached.width == board_w
        && cached.height == board_h
        && cached.team_size == team_size
    {
        if !cached.below_floor || anchored_cols.is_none() {
            anchor = Some(cached.offset);
        }
        if !cached.below_floor {
            let cached_score = score(cached.offset);
            let reuse_floor = (max_score * 3 / 4).max(1);
            if cached_score >= reuse_floor {
                tracing::debug!(
                    offset_px = (cached.offset * board_w as f64) as i32,
                    valid_cells = cached_score,
                    "reusing cached column calibration"
                );
                return preprocess::columns_with_offset(cached.offset);
            }
            tracing::debug!(
                valid_cells = cached_score,
                floor = reuse_floor,
                "cached column offset degraded: recalibrating"
            );
        }
    }

    let best_of = |offsets: Vec<f64>| -> (f64, i32) {
        #[cfg(test)]
        {
            COLUMN_OFFSET_SWEEPS.with(|c| c.set(c.get() + 1));
            COLUMN_SWEEP_BATCHES.with(|slot| slot.borrow_mut().push(offsets.clone()));
        }
        // No pool (OCR off) scores like an empty sweep.
        let Ok(pool) = ocr_pool() else {
            return (0.0f64, i32::MIN);
        };
        pool.install(|| {
            offsets
                .into_par_iter()
                .map(|o| (o, score(o)))
                .reduce(|| (0.0f64, i32::MIN), |a, b| better_offset(a, b, anchor))
        })
    };

    // Phase 1: coarse sweep covering both the header-detected region and the
    // standard near-zero region, in 0.02 steps
    let coarse_min = header_offset.min(-0.02) - 0.04;
    let coarse_max = header_offset.max(0.04) + 0.04;

    let mut coarse = Vec::new();
    let mut offset = coarse_min;
    while offset <= coarse_max {
        coarse.push(offset);
        offset += 0.02;
    }
    let (mut best_offset, mut best_valid) = best_of(coarse);
    #[cfg(test)]
    COLUMN_SWEEP_UNDER_FLOOR.with(|c| c.set(best_valid < anchored_floor));

    // The coarse pass already tracked the best offset. A finer sweep around
    // that winner runs once on a cold miss whenever the coarse score is not
    // already perfect, including a board that stays under the 75% floor.
    // The result is cached, so a later read of the same layout does not
    // sweep again. A board that already cleared the floor keeps today's
    // columns: this pass only moves the winner when the fine score is higher.
    if best_valid < max_score {
        // Phase 2: fine-tune ±0.02 around the best coarse result in 0.005 steps.
        // Only offsets not already scored in the coarse grid. Skip the exact
        // coarse centre, which is already known.
        let mut fine = Vec::new();
        let mut fine_offset = best_offset - 0.02;
        let fine_end = best_offset + 0.02;
        while fine_offset <= fine_end + 1e-9 {
            let near_coarse = (fine_offset - best_offset).abs() < 1e-9;
            if !near_coarse {
                fine.push(fine_offset);
            }
            fine_offset += 0.005;
        }
        if !fine.is_empty() {
            #[cfg(test)]
            COLUMN_FINE_SWEEPS.with(|c| c.set(c.get() + 1));
            let (fine_best_offset, fine_best_valid) = best_of(fine);
            if fine_best_valid > best_valid {
                best_offset = fine_best_offset;
                best_valid = fine_best_valid;
            }
        }
    }

    // A below-floor anchored layout still beats the sweep when it scores
    // higher: frame-derived geometry outranks a translated fallback comb on
    // anything but a worse read (ties go to anchored for the same reason).
    if let Some((cols, anchored_score)) = &anchored
        && *anchored_score >= best_valid
    {
        tracing::debug!(
            anchored_score,
            sweep_score = best_valid,
            "header-anchored columns beat the offset sweep"
        );
        // This path is only reached when the anchored score is under the
        // floor (the above-floor case returned before the sweep). Remember
        // that choice so the next read of this size does not sweep again.
        if *anchored_score < anchored_floor {
            remember(best_offset, true, true);
        }
        return *cols;
    }

    remember(best_offset, best_valid < anchored_floor, false);

    tracing::debug!(
        header_offset_px = (header_offset * board_w as f64) as i32,
        final_offset_px = (best_offset * board_w as f64) as i32,
        valid_cells = best_valid,
        max_score,
        probe_count = probe_rows.len(),
        "two-phase column calibration"
    );

    preprocess::columns_with_offset(best_offset)
}

/// Tie-break between two `(offset, score)` calibration candidates.
///
/// A higher valid-cell score always wins. On an EQUAL score, prefer the offset
/// closest to `anchor` (the last known-good calibration), falling back to the
/// smaller absolute offset — never a blanket leftward (more-negative) bias.
/// The old `b.0 < a.0` tie-break systematically walked the calibration left
/// onto the header mis-seed, clipping the narrow E/A/D columns (step-0 drift
/// analysis, ocr/mod.rs:482). The ordering key `(distance-to-anchor, offset)`
/// is a strict total order, so this stays associative for a parallel reduce.
fn better_offset(a: (f64, i32), b: (f64, i32), anchor: Option<f64>) -> (f64, i32) {
    use std::cmp::Ordering;
    match b.1.cmp(&a.1) {
        Ordering::Greater => b,
        Ordering::Less => a,
        Ordering::Equal => {
            let rank = |o: f64| match anchor {
                Some(anc) => ((o - anc).abs(), o),
                None => (o.abs(), o),
            };
            let (ra, rb) = (rank(a.0), rank(b.0));
            match rb.0.total_cmp(&ra.0).then(rb.1.total_cmp(&ra.1)) {
                Ordering::Less => b,
                _ => a,
            }
        }
    }
}

/// Calibration score contribution of one clean, structurally-valid cell.
pub(crate) const CELL_SCORE_CLEAN: i32 = 2;
/// Calibration score penalty for a suspect (edge-ink) cell: a drifted layout
/// that still reads digits must score strictly BELOW a clean-margin layout,
/// not merely fail to gain (CG-4 review — the old no-op treatment let a
/// drifted offset tie a clean one when both read the same digit count).
pub(crate) const CELL_SCORE_SUSPECT: i32 = 1;

/// Scores with [`recognize_cell_bright`], not the dim-zero recovery. A
/// recovered ring says a zero is somewhere in the window, not that the
/// window's edges sit on the column. Counting recovered zeros pushed the
/// header-anchored layout of the 0.75x frame `accepted_20261008_001449`
/// over the 75% floor. That skipped the offset sweep main runs there and
/// lost 10 cells main reads (Scuffed Vision baseline, PR 158). With the
/// bright path, PR 158 picks the same layout as main on all 40 frames of
/// that baseline (20 at 1440p, the same 20 at 0.75x).
fn count_valid_cells(row: &DynamicImage, cols: &preprocess::StatColumns) -> i32 {
    let mut valid = 0i32;
    for col_idx in 0..6 {
        #[cfg(test)]
        if SCORE_DARK_COLUMN_MARKS.load(std::sync::atomic::Ordering::Relaxed) {
            if let Some(cell) = preprocess::crop_stat_cell(row, col_idx, cols) {
                let rgb = cell.to_rgb8();
                if rgb.width() > 0 && rgb.height() > 0 {
                    let cx = rgb.width() / 2;
                    let x0 = cx.saturating_sub(2);
                    let x1 = (cx + 3).min(rgb.width());
                    let mut dark = 0u32;
                    let mut seen = 0u32;
                    for y in 0..rgb.height() {
                        for x in x0..x1 {
                            let px = rgb.get_pixel(x, y);
                            let brightness =
                                (u32::from(px.0[0]) + u32::from(px.0[1]) + u32::from(px.0[2])) / 3;
                            if brightness < 80 {
                                dark += 1;
                            }
                            seen += 1;
                        }
                    }
                    if seen > 0 && dark * 2 >= seen {
                        valid += CELL_SCORE_CLEAN;
                    }
                }
            }
            continue;
        }
        #[cfg(test)]
        if let Some(forced) = FORCED_CELL_SCORE.with(|slot| slot.get()) {
            valid += forced;
            continue;
        }
        if let Some(cell) = preprocess::crop_stat_cell(row, col_idx, cols)
            && let Ok(result) = recognize_cell_bright(&cell, "0123456789,")
        {
            // A cell whose glyph ink touches a crop edge (CG-3 bleed/clip)
            // SUBTRACTS from the layout's score, so the sweep prefers offsets
            // with clean margins over a drifted one that still reads 6 digits.
            let text = result.value.trim();
            if result.suspect {
                valid -= CELL_SCORE_SUSPECT;
            } else if is_valid_stat_for_column(text, col_idx) {
                valid += CELL_SCORE_CLEAN;
            }
        }
    }
    valid
}

/// Whether a cell's OCR text is structurally valid *for its column*, used to
/// score a candidate column offset. The narrow kill columns (E/A/D, indices
/// 0–2) hold small 1–2 digit integers; a 3+ digit read or an embedded comma
/// there means a wider neighbour bled into the window (calibration drift), so it
/// must NOT count toward the offset's score. The wide accumulator columns keep
/// the permissive digits-and-commas check.
///
/// A margin check (the clean discriminator between a centered "13" and a
/// clipped/bled read) was previously deferred for want of real-pixel fixtures.
/// That reason is now resolved: the 2026-07-20 drift frames calibrated an
/// edge-ink threshold (`EDGE_INK_THRESHOLD`), and `count_valid_cells` drops any
/// cell whose glyph ink touches a crop edge from the offset score — so the sweep
/// actively prefers clean-margin offsets. This structural check remains the
/// digit-count half; the edge-ink check is the margin half; the per-cell capture
/// gate (monotonic hold + rate cap + un-latch) is the runtime safety net.
pub(crate) fn is_valid_stat_for_column(text: &str, col_idx: usize) -> bool {
    let text = text.trim();
    if !is_clean_stat(text) {
        return false;
    }
    if col_idx < 3 {
        let digits = text.chars().filter(|c| c.is_ascii_digit()).count();
        !text.contains(',') && (1..=2).contains(&digits)
    } else {
        true
    }
}

/// Whether a cell's OCR text looks like a real stat value: at least one digit,
/// nothing but digits and thousands-separator commas.
pub(crate) fn is_clean_stat(text: &str) -> bool {
    if text.is_empty() {
        return false;
    }
    let has_digit = text.chars().any(|c| c.is_ascii_digit());
    let all_valid = text.chars().all(|c| c.is_ascii_digit() || c == ',');
    has_digit && all_valid
}

/// Full-image OCR (original approach with adaptive preprocessing).
/// Used as fallback and for compatibility with the existing parse pipeline.
#[track_caller]
pub fn recognize(
    img: &DynamicImage,
) -> Result<OcrResult, Box<dyn std::error::Error + Send + Sync>> {
    gate::check("recognize")?;
    let cropped = preprocess::crop_scoreboard(img);

    // Primary path: adaptive thresholding
    let preprocessed = preprocess::prepare_adaptive(&cropped);
    let mut png_buf = Vec::new();
    DynamicImage::ImageLuma8(preprocessed.clone())
        .write_to(&mut Cursor::new(&mut png_buf), image::ImageFormat::Png)?;

    let lang = tessdata_lang()?;
    let primary = run_ocr(lang, &png_buf)?;

    tracing::debug!(confidence = primary.confidence, lang, "adaptive OCR result");

    // If adaptive result is good enough, use it
    if primary.confidence >= 65 {
        save_debug_images(&cropped, &preprocessed, &png_buf);
        return try_fallback(lang, primary, &png_buf);
    }

    // Fallback: try the legacy multi-threshold sweep
    let thresholds: &[u8] = &[120, 140, 160];
    let mut best: Option<OcrResult> = Some(primary);

    for &threshold in thresholds {
        let legacy_preprocessed = preprocess::prepare_with_threshold(&cropped, threshold);
        let mut buf = Vec::new();
        if DynamicImage::ImageLuma8(legacy_preprocessed)
            .write_to(&mut Cursor::new(&mut buf), image::ImageFormat::Png)
            .is_err()
        {
            continue;
        }

        if let Ok(result) = run_ocr(lang, &buf) {
            let dominated = best
                .as_ref()
                .is_some_and(|b| b.confidence >= result.confidence);
            if !dominated {
                best = Some(result);
                png_buf = buf;
            }
            if best.as_ref().is_some_and(|b| b.confidence >= 70) {
                break;
            }
        }
    }

    save_debug_images(&cropped, &preprocessed, &png_buf);

    let best = best.ok_or_else(|| -> Box<dyn std::error::Error + Send + Sync> {
        "all OCR attempts failed".into()
    })?;

    try_fallback(lang, best, &png_buf)
}

fn try_fallback(
    lang: &'static str,
    primary: OcrResult,
    png_buf: &[u8],
) -> Result<OcrResult, Box<dyn std::error::Error + Send + Sync>> {
    let fallback_lang = if lang == "koverwatch" {
        "eng"
    } else {
        return Ok(primary);
    };

    if primary.confidence >= 65 {
        return Ok(primary);
    }

    match run_ocr(fallback_lang, png_buf) {
        Ok(fallback) if fallback.confidence > primary.confidence => {
            tracing::info!(
                primary_conf = primary.confidence,
                fallback_conf = fallback.confidence,
                "using eng fallback (higher confidence)"
            );
            Ok(fallback)
        }
        _ => Ok(primary),
    }
}

fn save_debug_images(cropped: &DynamicImage, _preprocessed: &image::GrayImage, png_buf: &[u8]) {
    let Some(dir) = debug_dir() else {
        return;
    };
    let _ = cropped.save(dir.join("crop.png"));
    // Write the already-computed preprocess buffer — do not re-run the pipeline.
    let _ = std::fs::write(dir.join("preprocessed.png"), png_buf);
    tracing::debug!(path = %dir.display(), "saved debug images");
}

fn run_ocr(
    lang: &'static str,
    png_buf: &[u8],
) -> Result<OcrResult, Box<dyn std::error::Error + Send + Sync>> {
    let (text, confidence) = ocr_with(lang, "6", None, png_buf)?;

    tracing::debug!(confidence, text_len = text.len(), lang, "OCR complete");

    Ok(OcrResult {
        raw_text: text,
        confidence,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The functions of this file, by name: `(is it pub or pub(crate), body)`.
    /// Top-level items only, up to the first test module.
    fn production_functions() -> std::collections::BTreeMap<String, (bool, String)> {
        let source = include_str!("mod.rs");
        let end = ["\n#[cfg(test)]\nmod ", "\n#[cfg(test)]\npub(crate) mod "]
            .iter()
            .filter_map(|marker| source.find(marker))
            .min()
            .expect("the test modules");
        let mut out = std::collections::BTreeMap::new();
        let mut current: Option<(String, bool, String)> = None;
        for line in source[..end].lines() {
            let header = ["pub fn ", "pub(crate) fn ", "fn "]
                .iter()
                .find_map(|prefix| line.strip_prefix(prefix).map(|rest| (*prefix, rest)));
            if let Some((prefix, rest)) = header {
                let name: String = rest
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                current = Some((name, prefix != "fn ", String::new()));
            }
            if let Some((_, _, body)) = current.as_mut() {
                body.push_str(line);
                body.push('\n');
            }
            if line == "}"
                && let Some((name, public, body)) = current.take()
            {
                out.insert(name, (public, body));
            }
        }
        out
    }

    /// `body` calls the function `name` (not one whose name only ends with it).
    fn calls(body: &str, name: &str) -> bool {
        let needle = format!("{name}(");
        body.match_indices(&needle).any(|(at, _)| {
            !body[..at]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_alphanumeric() || c == '_')
        })
    }

    /// Tesseract's three resources each refuse when the gate is off, and
    /// every `pub` or `pub(crate)` function that can reach one of them asks
    /// the gate itself, first. A new entry point that forgets fails here,
    /// before it can start the pool or be counted under the wrong site.
    #[test]
    fn every_public_function_that_can_reach_tesseract_asks_the_gate() {
        let functions = production_functions();
        const RESOURCES: [&str; 3] = ["ocr_with", "ocr_pool", "tessdata_lang"];
        for resource in RESOURCES {
            let (_, body) = functions.get(resource).expect(resource);
            // The refusal is the first statement, so nothing is started
            // before it: not after the instance or the pool is built.
            let start = body.find('{').expect("a body");
            let first_statement = body[start + 1..].trim_start();
            assert!(
                first_statement.starts_with("if gate::is_off() {\n        return Err(gate::block("),
                "{resource} has to refuse when OCR is off, before anything else"
            );
        }
        // What can reach a resource: a fixpoint over the calls in each body.
        let mut reaches: std::collections::BTreeSet<&str> = RESOURCES.into_iter().collect();
        loop {
            let more: Vec<&str> = functions
                .iter()
                .filter(|(name, (_, body))| {
                    !reaches.contains(name.as_str())
                        && reaches.iter().any(|callee| {
                            callee != &name.as_str()
                                && calls(&body[body.find('{').unwrap_or(0)..], callee)
                        })
                })
                .map(|(name, _)| name.as_str())
                .collect();
            if more.is_empty() {
                break;
            }
            reaches.extend(more);
        }
        let mut gated = Vec::new();
        for name in &reaches {
            let (public, body) = &functions[*name];
            if !public {
                continue;
            }
            let start = body.find('{').expect("a body");
            let first_statement = body[start + 1..].trim_start();
            assert!(
                first_statement.starts_with(&format!("gate::check(\"{name}\")?;"))
                    || first_statement
                        .starts_with(&format!("if gate::check(\"{name}\").is_err() {{")),
                "{name} can reach Tesseract and does not ask the gate first"
            );
            assert!(
                body.contains("#[track_caller]") || source_has_track_caller(name),
                "{name} has to count a blocked call under its caller's line"
            );
            gated.push(*name);
        }
        assert_eq!(
            gated,
            [
                "read_map_crop",
                "recognize",
                "recognize_cell",
                "recognize_map_label",
                "recognize_name",
                "recognize_prepared",
                "recognize_prepared_lang",
                "recognize_region",
                "recognize_row",
                "recognize_scoreboard_cells",
                "recognize_scoreboard_cells_pre_cropped",
                "recognize_scoreboard_cells_pre_cropped_with",
                "recognize_scoreboard_cells_with_team_size",
                "recognize_scoreboard_names_pre_cropped",
                "recognize_sparse_region",
            ],
            "the entry points changed: update this list and tests/ocr_switch.rs"
        );
        // A public function that reaches nothing is not gated and need not be.
        assert!(functions.contains_key("cell_edge_ink"));
        assert!(!reaches.contains("cell_edge_ink"));
    }

    /// The second layer, by behaviour: each resource refuses by itself when
    /// the gate is off, with the gate's own error, counted under its own
    /// name. So an entry point that forgets the check still starts nothing.
    #[test]
    fn the_three_resources_refuse_by_themselves_when_ocr_is_off() {
        let off = gate::scoped(gate::OcrMode::Off);
        let err = ocr_with("eng", "7", None, &[]).expect_err("no read with OCR off");
        assert!(gate::is_off_error(err.as_ref()), "{err}");
        assert!(ocr_pool().is_err(), "no pool with OCR off");
        assert!(tessdata_lang().is_err(), "no language lookup with OCR off");
        let blocked = off.blocked();
        let mut entries: Vec<&str> = blocked.iter().map(|b| b.site.entry).collect();
        entries.sort_unstable();
        assert_eq!(entries, ["ocr_pool", "ocr_with", "tessdata_lang"]);
        for b in &blocked {
            assert_eq!(b.count, 1, "{}", b.site);
            assert!(b.site.file.ends_with("ocr/mod.rs"), "{}", b.site);
        }
        drop(off);
        // With OCR on the same calls go past the gate.
        assert!(tessdata_lang().is_ok());
        assert!(tessdata_lang_probed());
        assert!(ocr_pool().is_ok());
        assert!(pool_started());
    }

    /// Every `.rs` file below `src/`, with the part of it that is not test
    /// code: the text before its first test module. Files that are test
    /// code as a whole are left out.
    fn production_sources() -> Vec<(String, String)> {
        fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            for entry in std::fs::read_dir(dir).expect("read src") {
                let path = entry.expect("dir entry").path();
                if path.is_dir() {
                    walk(&path, out);
                } else if path.extension().is_some_and(|ext| ext == "rs") {
                    out.push(path);
                }
            }
        }
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = Vec::new();
        walk(&src, &mut files);
        files.sort();
        files
            .into_iter()
            .filter(|path| {
                let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
                // `mod tests;` files and the OCR-off bench are test code.
                !stem.starts_with("tests") && stem != "ocr_bench"
            })
            .map(|path| {
                let text = std::fs::read_to_string(&path).expect("read a source file");
                let end = ["\n#[cfg(test)]\nmod ", "\n#[cfg(test)]\npub(crate) mod "]
                    .iter()
                    .filter_map(|marker| text.find(marker))
                    .min()
                    .unwrap_or(text.len());
                let name = path
                    .strip_prefix(&src)
                    .expect("below src")
                    .to_string_lossy()
                    .replace('\\', "/");
                (name, text[..end].to_string())
            })
            .collect()
    }

    /// Nothing outside this file can start Tesseract or open the gate:
    /// - one `LepTess` construction in the whole tree, in `ocr_with`;
    /// - `ocr_with` is called from this file only;
    /// - the process-wide mode is set from `ocr_switch.rs` only, and no
    ///   code outside tests sets a scoped mode.
    #[test]
    fn nothing_outside_the_gate_starts_tesseract_or_opens_the_gate() {
        // Built at run time, so this test does not find itself.
        let construct = format!("LepTess::{}(", "new");
        let read = format!("ocr_{}(", "with");
        let set_mode = format!("set_{}(", "mode");
        let scoped = format!("scop{}(", "ed");
        let sources = production_sources();
        assert!(
            sources.len() > 20,
            "the walk found the tree: {}",
            sources.len()
        );
        let holding = |needle: &str| -> Vec<(String, usize)> {
            sources
                .iter()
                .map(|(name, text)| (name.clone(), text.matches(needle).count()))
                .filter(|(_, count)| *count > 0)
                .collect()
        };
        assert_eq!(
            holding(&construct),
            [("ocr/mod.rs".to_string(), 1)],
            "one Tesseract instance is built, in ocr_with"
        );
        let callers = holding(&read);
        assert_eq!(callers.len(), 1, "{callers:?}");
        assert_eq!(
            callers[0].0, "ocr/mod.rs",
            "ocr_with is private to this file"
        );
        let setters: Vec<String> = holding(&set_mode)
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        assert_eq!(
            setters,
            ["ocr/gate.rs", "ocr_switch.rs"],
            "only the switch sets the process-wide mode"
        );
        let scopes: Vec<String> = holding(&scoped).into_iter().map(|(name, _)| name).collect();
        assert_eq!(scopes, ["ocr/gate.rs"], "a scoped mode is for tests");
    }

    /// The line above `fn name` (after its doc comment) is `#[track_caller]`.
    fn source_has_track_caller(name: &str) -> bool {
        let source = include_str!("mod.rs");
        ["pub fn ", "pub(crate) fn "].iter().any(|prefix| {
            source.contains(&format!("#[track_caller]\n{prefix}{name}("))
                || source.contains(&format!("#[track_caller]\n{prefix}{name}<"))
        })
    }

    /// alpha.7 O9 (OCR site T4): with [`Names::Skip`] no name window is read,
    /// on either table. ocr-v1 ([`Names::Read`]) still reads every window:
    /// one on 6v6, three on 5v5.
    #[test]
    fn skipping_names_reads_no_name_window() {
        let board = DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            1664,
            1007,
            image::Rgb([20, 24, 30]),
        ));
        for (team_size, alternates) in [(5usize, 2usize), (6, 0)] {
            let skipped =
                recognize_scoreboard_cells_pre_cropped_with(&board, team_size, Names::Skip);
            assert_eq!(skipped.len(), 2 * team_size);
            for row in &skipped {
                assert!(row.name.is_none(), "{team_size}v{team_size}");
                assert!(row.name_alternates.is_empty(), "{team_size}v{team_size}");
                assert_eq!(row.stats.len(), STATS_PER_ROW);
            }
            for rows in [
                recognize_scoreboard_cells_pre_cropped_with(&board, team_size, Names::Read),
                recognize_scoreboard_cells_pre_cropped(&board, team_size),
            ] {
                assert_eq!(rows.len(), 2 * team_size);
                for row in &rows {
                    assert!(row.name.is_some(), "the first window is read");
                    assert_eq!(row.name_alternates.len(), alternates);
                }
            }
        }
    }

    /// alpha.7 O10: the rows the new reader hands over carry nothing that
    /// was read, one per board row.
    #[test]
    fn unread_rows_carry_no_name_and_no_stat_cell() {
        for team_size in [5usize, 6] {
            let rows = unread_rows(team_size);
            assert_eq!(rows.len(), 2 * team_size);
            for row in &rows {
                assert!(row.name.is_none() && row.name_alternates.is_empty());
                assert!(row.stats.is_empty());
                assert_eq!(row.mean_confidence, 0);
            }
        }
        assert!(unread_rows(0).is_empty());
    }

    #[test]
    fn dim_zero_cell_reads_as_zero() {
        // The primary mask erases this glyph (ink under the empty-cell
        // skip). It must still come back as 0, or an early-game row with a
        // 0 in E/A/D is rejected whole.
        let img = preprocess::dim_zero_fixtures::dim_zero_cell();
        let binary = preprocess::prepare_cell_binary(&img);
        let ink = binary.pixels().filter(|p| p.0[0] < 128).count();
        assert!(
            ink < MIN_CELL_INK_PIXELS,
            "fixture must be a zero the HSV mask drops, ink={ink}"
        );
        let cell = recognize_cell(&img).expect("dim zero cell");
        assert_eq!(cell.value, "0");
        assert!(!cell.suspect);
        let hit = preprocess::dim_zero_glyph(&img, EDGE_INK_COLS).expect("ring");
        assert_eq!(
            cell.confidence, hit.confidence,
            "a recovered zero carries the ring's geometric confidence"
        );
        assert!(cell.confidence >= 70, "a clean ring is not suspect");
        let bright = recognize_cell_bright(&img, "0123456789,").expect("bright path");
        assert_eq!(
            bright.value, "",
            "column calibration scores the bright path, which has no dim-zero recovery"
        );

        let edge = recognize_cell(&preprocess::dim_zero_fixtures::dim_zero_touching_left_edge())
            .expect("edge ring");
        assert_eq!(edge.value, "0");
        assert!(
            edge.suspect,
            "touches_edge has to arrive on the cell result"
        );

        let stroke =
            recognize_cell(&preprocess::dim_zero_fixtures::dim_stroke_cell()).expect("dim stroke");
        assert_eq!(stroke.value, "", "a dim 1 must not be invented as 0");
        let blank = recognize_cell(&DynamicImage::ImageRgb8(
            preprocess::dim_zero_fixtures::cell(48, 56),
        ))
        .expect("blank");
        assert_eq!(blank.value, "");
    }

    #[test]
    fn kill_columns_reject_wide_and_multidigit_reads() {
        // E/A/D expect 1–2 digits, no comma.
        assert!(is_valid_stat_for_column("13", 0));
        assert!(is_valid_stat_for_column("7", 2));
        assert!(is_valid_stat_for_column("0", 1));
        // A wide number bleeding into a narrow window must not score.
        assert!(!is_valid_stat_for_column("5175", 0));
        assert!(!is_valid_stat_for_column("311", 0));
        assert!(!is_valid_stat_for_column("4,119", 2));
        assert!(!is_valid_stat_for_column("", 0));
    }

    #[test]
    fn wide_columns_keep_permissive_check() {
        assert!(is_valid_stat_for_column("4,119", 3));
        assert!(is_valid_stat_for_column("10009", 3));
        assert!(is_valid_stat_for_column("899", 5));
        assert!(!is_valid_stat_for_column("x", 4));
    }

    #[test]
    fn better_offset_prefers_higher_score() {
        assert_eq!(better_offset((-0.1, 3), (-0.05, 5), None), (-0.05, 5));
        assert_eq!(better_offset((-0.1, 6), (-0.05, 5), None), (-0.1, 6));
    }

    #[test]
    fn better_offset_tie_breaks_toward_anchor_not_leftward() {
        // Equal score, two candidates straddling the known-good anchor -0.17.
        // The OLD rule (smaller offset wins) would pick -0.20 (further left);
        // the anchored rule picks -0.16 (closer to the good calibration).
        let anchor = Some(-0.17);
        assert_eq!(better_offset((-0.20, 4), (-0.16, 4), anchor), (-0.16, 4));
        assert_eq!(better_offset((-0.16, 4), (-0.20, 4), anchor), (-0.16, 4));
    }

    #[test]
    fn better_offset_tie_without_anchor_prefers_smaller_magnitude() {
        assert_eq!(better_offset((-0.20, 4), (-0.05, 4), None), (-0.05, 4));
    }

    /// A flat board never reads a valid stat, so no offset clears the 75%
    /// floor. A cold miss runs the coarse sweep and the fine sweep once.
    #[test]
    fn under_threshold_board_sweeps_column_offsets_once() {
        let _hold = calib_test_guard();
        if let Ok(mut guard) = COLUMN_OFFSET_CACHE.lock() {
            *guard = None;
        }
        COLUMN_OFFSET_SWEEPS.with(|c| c.set(0));
        COLUMN_FINE_SWEEPS.with(|c| c.set(0));
        COLUMN_SWEEP_UNDER_FLOOR.with(|c| c.set(false));
        let board = DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            1664,
            1007,
            image::Rgb([20, 24, 30]),
        ));
        let _columns = calibrate_columns(&board, 5);
        assert!(
            COLUMN_SWEEP_UNDER_FLOOR.with(|c| c.get()),
            "this board must stay under the 75% floor"
        );
        assert_eq!(
            COLUMN_FINE_SWEEPS.with(|c| c.get()),
            1,
            "a cold miss runs the fine sweep once"
        );
        assert_eq!(
            COLUMN_OFFSET_SWEEPS.with(|c| c.get()),
            2,
            "a cold miss runs the coarse sweep and one fine sweep"
        );
    }

    /// Two boards with no header do not match. A missing layout is a cold
    /// miss, so the second read of the same size sweeps again.
    #[test]
    fn headerless_boards_of_the_same_size_do_not_share_a_below_floor_entry() {
        let _hold = calib_test_guard();
        if let Ok(mut guard) = COLUMN_OFFSET_CACHE.lock() {
            *guard = None;
        }
        COLUMN_OFFSET_SWEEPS.with(|c| c.set(0));
        COLUMN_FINE_SWEEPS.with(|c| c.set(0));
        let board = DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            1664,
            1007,
            image::Rgb([20, 24, 30]),
        ));
        assert!(
            preprocess::columns_from_header_groups(
                &preprocess::header_label_groups(&board),
                board.width(),
            )
            .is_none(),
            "this fixture has no header layout"
        );
        let _first = calibrate_columns(&board, 5);
        assert!(
            COLUMN_SWEEP_UNDER_FLOOR.with(|c| c.get()),
            "this board must stay under the 75% floor"
        );
        let sweeps = COLUMN_OFFSET_SWEEPS.with(|c| c.get());
        assert!(sweeps >= 1, "the first read has to sweep");
        assert_eq!(
            COLUMN_FINE_SWEEPS.with(|c| c.get()),
            1,
            "the first headerless read runs the fine sweep once"
        );
        let _second = calibrate_columns(&board, 5);
        assert!(
            COLUMN_OFFSET_SWEEPS.with(|c| c.get()) > sweeps,
            "two boards with no header must not share a below-floor entry"
        );
        assert_eq!(
            COLUMN_FINE_SWEEPS.with(|c| c.get()),
            2,
            "the second headerless read runs the cold fine sweep"
        );

        let after = COLUMN_OFFSET_SWEEPS.with(|c| c.get());
        let other = DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            1600,
            900,
            image::Rgb([20, 24, 30]),
        ));
        let _ = calibrate_columns(&other, 5);
        assert!(
            COLUMN_OFFSET_SWEEPS.with(|c| c.get()) > after,
            "a different board size must sweep again"
        );
    }

    /// Bright header with six dark stat labels, so anchoring can succeed.
    fn board_with_stat_headers(w: u32, h: u32) -> DynamicImage {
        board_with_stat_headers_at(w, h, 0.32)
    }

    /// Same fixture as [`board_with_stat_headers`], with the label row
    /// starting at `origin_frac` of the board width.
    fn board_with_stat_headers_at(w: u32, h: u32, origin_frac: f64) -> DynamicImage {
        let mut img = image::RgbImage::from_pixel(w, h, image::Rgb([232, 232, 232]));
        let origin = (w as f64 * origin_frac) as u32;
        let pitch = (w as f64 * 0.05) as u32;
        let y0 = (h as f64 * 0.008) as u32;
        let y1 = (h as f64 * 0.022) as u32;
        for i in 0..6u32 {
            let x0 = origin + i * pitch;
            for y in y0..y1 {
                for x in x0..x0.saturating_add(8) {
                    if x < w && y < h {
                        img.put_pixel(x, y, image::Rgb([16, 16, 16]));
                    }
                }
            }
        }
        DynamicImage::ImageRgb8(img)
    }

    /// Same headers as [`board_with_stat_headers`], plus dark marks at the
    /// fallback columns shifted by `offset`. The marks sit below the header
    /// strip, so the measured layout stays on the labels.
    fn board_with_headers_and_column_marks(w: u32, h: u32, offset: f64) -> DynamicImage {
        let mut img = board_with_stat_headers(w, h).to_rgb8();
        let y0 = (h as f64 * 0.04) as u32;
        for (start, width) in preprocess::columns_with_offset(offset) {
            let center = ((start + width / 2.0) * w as f64).round() as i32;
            for y in y0..h {
                for dx in -3..4 {
                    let x = center + dx;
                    if x >= 0 && (x as u32) < w {
                        img.put_pixel(x as u32, y, image::Rgb([16, 16, 16]));
                    }
                }
            }
        }
        DynamicImage::ImageRgb8(img)
    }

    fn column_offset_of(cols: &preprocess::StatColumns) -> f64 {
        cols[0].0 - preprocess::columns_with_offset(0.0)[0].0
    }

    fn reset_calib_for_test() {
        if let Ok(mut guard) = COLUMN_OFFSET_CACHE.lock() {
            *guard = None;
        }
        COLUMN_OFFSET_SWEEPS.with(|c| c.set(0));
        COLUMN_FINE_SWEEPS.with(|c| c.set(0));
        COLUMN_SWEEP_BATCHES.with(|slot| slot.borrow_mut().clear());
        COLUMN_SWEEP_UNDER_FLOOR.with(|c| c.set(false));
        FORCED_CELL_SCORE.with(|slot| slot.set(None));
        SCORE_DARK_COLUMN_MARKS.store(false, std::sync::atomic::Ordering::Relaxed);
    }

    /// A cold below-floor miss runs the fine sweep once. A repeat whose
    /// cached columns stay under the floor is a cold miss and sweeps again.
    #[test]
    fn cold_below_floor_fine_sweep_runs_once_and_a_weak_repeat_sweeps_again() {
        let _hold = calib_test_guard();
        reset_calib_for_test();
        FORCED_CELL_SCORE.with(|slot| slot.set(Some(0)));
        let board = board_with_stat_headers(1664, 1007);
        let _first = calibrate_columns(&board, 5);
        assert!(
            COLUMN_SWEEP_UNDER_FLOOR.with(|c| c.get()),
            "the cold read must stay under the 75% floor"
        );
        assert_eq!(
            COLUMN_FINE_SWEEPS.with(|c| c.get()),
            1,
            "a cold below-floor miss runs the fine sweep once"
        );
        let sweeps = COLUMN_OFFSET_SWEEPS.with(|c| c.get());
        assert!(
            sweeps >= 2,
            "the cold read runs the coarse sweep and the fine sweep"
        );
        let _second = calibrate_columns(&board, 5);
        assert!(
            COLUMN_FINE_SWEEPS.with(|c| c.get()) > 1,
            "cached columns under the floor do not skip the fine sweep"
        );
        assert!(
            COLUMN_OFFSET_SWEEPS.with(|c| c.get()) > sweeps,
            "cached columns under the floor are a cold miss"
        );
        FORCED_CELL_SCORE.with(|slot| slot.set(None));
    }

    /// Same width, height, and team size is not enough. A shifted header
    /// is a different measured layout, so it must not inherit the entry.
    #[test]
    fn different_layout_at_the_same_size_does_not_inherit_a_below_floor_entry() {
        let _hold = calib_test_guard();
        reset_calib_for_test();
        FORCED_CELL_SCORE.with(|slot| slot.set(Some(0)));
        let left = board_with_stat_headers_at(1664, 1007, 0.32);
        let right = board_with_stat_headers_at(1664, 1007, 0.48);
        let left_layout = preprocess::columns_from_header_groups(
            &preprocess::header_label_groups(&left),
            left.width(),
        )
        .expect("left headers anchor six columns");
        let right_layout = preprocess::columns_from_header_groups(
            &preprocess::header_label_groups(&right),
            right.width(),
        )
        .expect("right headers anchor six columns");
        assert!(
            left_layout
                .iter()
                .zip(right_layout)
                .any(|(a, b)| (a.0 - b.0).abs() > LAYOUT_TOLERANCE),
            "the two fixtures must measure different column layouts"
        );

        let _ = calibrate_columns(&left, 5);
        assert_eq!(
            COLUMN_FINE_SWEEPS.with(|c| c.get()),
            1,
            "the first layout runs the fine sweep once"
        );
        let sweeps = COLUMN_OFFSET_SWEEPS.with(|c| c.get());
        let _ = calibrate_columns(&right, 5);
        assert!(
            COLUMN_OFFSET_SWEEPS.with(|c| c.get()) > sweeps,
            "a different layout at the same size is a cold miss"
        );
        assert_eq!(
            COLUMN_FINE_SWEEPS.with(|c| c.get()),
            2,
            "the other layout runs its own fine sweep"
        );
        let after = COLUMN_OFFSET_SWEEPS.with(|c| c.get());
        let _ = calibrate_columns(&right, 5);
        assert!(
            COLUMN_OFFSET_SWEEPS.with(|c| c.get()) > after,
            "a repeat under the floor does not inherit the entry"
        );
        FORCED_CELL_SCORE.with(|slot| slot.set(None));
    }

    /// A board with no readable header does not reuse a below-floor entry
    /// stored for the same size. It takes the cold path, fine sweep included.
    #[test]
    fn none_layout_board_after_a_below_floor_entry_runs_the_cold_path() {
        let _hold = calib_test_guard();
        reset_calib_for_test();
        FORCED_CELL_SCORE.with(|slot| slot.set(Some(0)));
        let headed = board_with_stat_headers(1664, 1007);
        assert!(
            preprocess::columns_from_header_groups(
                &preprocess::header_label_groups(&headed),
                headed.width(),
            )
            .is_some(),
            "the first fixture must store a measured layout"
        );
        let _ = calibrate_columns(&headed, 5);
        assert!(
            COLUMN_SWEEP_UNDER_FLOOR.with(|c| c.get()),
            "the first read must stay under the 75% floor"
        );
        assert_eq!(
            COLUMN_FINE_SWEEPS.with(|c| c.get()),
            1,
            "the headed board runs the fine sweep once"
        );
        let sweeps = COLUMN_OFFSET_SWEEPS.with(|c| c.get());

        let flat = DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            1664,
            1007,
            image::Rgb([20, 24, 30]),
        ));
        assert!(
            preprocess::columns_from_header_groups(
                &preprocess::header_label_groups(&flat),
                flat.width(),
            )
            .is_none(),
            "the following board has no header layout"
        );
        let _ = calibrate_columns(&flat, 5);
        assert!(
            COLUMN_OFFSET_SWEEPS.with(|c| c.get()) > sweeps,
            "a missing layout after a below-floor entry is a cold miss"
        );
        assert_eq!(
            COLUMN_FINE_SWEEPS.with(|c| c.get()),
            2,
            "the None-layout board runs the cold fine sweep"
        );
        FORCED_CELL_SCORE.with(|slot| slot.set(None));
    }

    /// Same resolution and the same header layout, but the stat marks sit
    /// on different offsets. The second board must not keep the first
    /// board's below-floor columns.
    #[test]
    fn matching_layouts_with_different_offsets_do_not_reuse_a_below_floor_entry() {
        let _hold = calib_test_guard();
        reset_calib_for_test();
        let left = board_with_stat_headers(1248, 756);
        let right = board_with_headers_and_column_marks(1248, 756, 0.04);
        let left_layout = preprocess::columns_from_header_groups(
            &preprocess::header_label_groups(&left),
            left.width(),
        )
        .expect("left headers anchor six columns");
        let right_layout = preprocess::columns_from_header_groups(
            &preprocess::header_label_groups(&right),
            right.width(),
        )
        .expect("right headers anchor six columns");
        assert!(
            layouts_match(&Some(left_layout), &Some(right_layout)),
            "the fixtures share a header layout within tolerance"
        );

        FORCED_CELL_SCORE.with(|slot| slot.set(Some(0)));
        let first = calibrate_columns(&left, 5);
        assert!(
            COLUMN_SWEEP_UNDER_FLOOR.with(|c| c.get()),
            "the first board must stay under the 75% floor"
        );
        let stored_below = COLUMN_OFFSET_CACHE
            .lock()
            .ok()
            .and_then(|guard| guard.as_ref().map(|cached| cached.below_floor));
        assert_eq!(
            stored_below,
            Some(true),
            "the first board stores a below-floor entry"
        );
        let sweeps = COLUMN_OFFSET_SWEEPS.with(|c| c.get());

        FORCED_CELL_SCORE.with(|slot| slot.set(None));
        SCORE_DARK_COLUMN_MARKS.store(true, std::sync::atomic::Ordering::Relaxed);
        let second = calibrate_columns(&right, 5);
        assert!(
            COLUMN_OFFSET_SWEEPS.with(|c| c.get()) > sweeps,
            "a different offset at the same layout is a cold miss"
        );
        assert!(
            (column_offset_of(&second) - 0.04).abs() < 0.01,
            "the fresh sweep follows this board's column marks, got {}",
            column_offset_of(&second)
        );
        assert_ne!(
            first, second,
            "the second board must not keep the first board's columns"
        );
        let replaced = COLUMN_OFFSET_CACHE
            .lock()
            .ok()
            .and_then(|guard| guard.as_ref().map(|cached| cached.below_floor));
        assert_eq!(
            replaced,
            Some(false),
            "a fresh calibration that clears the floor replaces the below-floor entry"
        );
        SCORE_DARK_COLUMN_MARKS.store(false, std::sync::atomic::Ordering::Relaxed);
    }

    /// A headerless cold miss searches the pre-cache coarse range and the
    /// fine range around that winner. With a same-size below-floor offset
    /// already cached, equal scores tie-break toward that offset.
    #[test]
    fn headerless_cold_path_runs_the_pre_cache_coarse_and_fine_sweep() {
        let _hold = calib_test_guard();
        reset_calib_for_test();
        FORCED_CELL_SCORE.with(|slot| slot.set(Some(0)));
        let board = DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            1664,
            1007,
            image::Rgb([20, 24, 30]),
        ));
        assert!(
            preprocess::columns_from_header_groups(
                &preprocess::header_label_groups(&board),
                board.width(),
            )
            .is_none(),
            "this fixture has no header layout"
        );
        let first = calibrate_columns(&board, 5);
        let batches = COLUMN_SWEEP_BATCHES.with(|slot| slot.borrow().clone());
        assert_eq!(
            batches.len(),
            2,
            "a headerless cold miss runs the coarse sweep and the fine sweep"
        );
        let header_offset = preprocess::offset_from_header_groups(
            &preprocess::header_label_groups(&board),
            board.width(),
        );
        let coarse_min = header_offset.min(-0.02) - 0.04;
        let coarse_max = header_offset.max(0.04) + 0.04;
        let mut expected_coarse = Vec::new();
        let mut offset = coarse_min;
        while offset <= coarse_max {
            expected_coarse.push(offset);
            offset += 0.02;
        }
        let coarse = &batches[0];
        assert_eq!(
            coarse.len(),
            expected_coarse.len(),
            "coarse sweep uses the pre-cache range"
        );
        for (got, want) in coarse.iter().zip(&expected_coarse) {
            assert!(
                (got - want).abs() < 1e-12,
                "coarse offset {got} does not match the pre-cache sweep {want}"
            );
        }
        for pair in coarse.windows(2) {
            assert!(
                (pair[1] - pair[0] - 0.02).abs() < 1e-6,
                "coarse step is 0.02"
            );
        }
        assert!(
            column_offset_of(&first).abs() < 1e-6,
            "with no anchor, equal scores pick the smallest offset"
        );
        let fine = &batches[1];
        assert!(!fine.is_empty(), "the fine sweep has offsets");
        for offset in fine {
            let distance = (offset - column_offset_of(&first)).abs();
            assert!(
                distance <= 0.02 + 1e-6 && distance > 1e-9,
                "fine offsets stay within 0.02 of the coarse winner and skip it"
            );
        }

        if let Ok(mut guard) = COLUMN_OFFSET_CACHE.lock() {
            *guard = Some(CalibCache {
                width: 1664,
                height: 1007,
                team_size: 5,
                offset: 0.04,
                below_floor: true,
                anchored: false,
                reuses: 0,
                layout: None,
            });
        }
        COLUMN_OFFSET_SWEEPS.with(|c| c.set(0));
        COLUMN_FINE_SWEEPS.with(|c| c.set(0));
        COLUMN_SWEEP_BATCHES.with(|slot| slot.borrow_mut().clear());
        let second = calibrate_columns(&board, 5);
        assert_eq!(
            COLUMN_OFFSET_SWEEPS.with(|c| c.get()),
            2,
            "the headerless cold path still sweeps instead of reusing"
        );
        assert_eq!(
            COLUMN_FINE_SWEEPS.with(|c| c.get()),
            1,
            "the headerless cold path runs the fine sweep once"
        );
        let again = COLUMN_SWEEP_BATCHES.with(|slot| slot.borrow().clone());
        assert!(
            again
                .first()
                .is_some_and(|coarse| (coarse[0] - coarse_min).abs() < 1e-9),
            "the second cold miss keeps the pre-cache coarse range"
        );
        assert!(
            (column_offset_of(&second) - 0.04).abs() < 1e-6,
            "equal scores tie-break toward the cached offset, got {}",
            column_offset_of(&second)
        );
        FORCED_CELL_SCORE.with(|slot| slot.set(None));
    }

    /// A clean read at the same size beats a below-floor cache: the
    /// header-anchored columns clear the floor, that entry is dropped, and
    /// the next weak read has to sweep again.
    #[test]
    fn a_clean_board_replaces_a_below_floor_cache_at_the_same_size() {
        let _hold = calib_test_guard();
        reset_calib_for_test();
        let board = board_with_stat_headers(1664, 1007);
        let want = preprocess::columns_from_header_groups(
            &preprocess::header_label_groups(&board),
            board.width(),
        )
        .expect("fixture headers anchor six columns");

        FORCED_CELL_SCORE.with(|slot| slot.set(Some(0)));
        let _weak = calibrate_columns(&board, 5);
        assert!(
            COLUMN_SWEEP_UNDER_FLOOR.with(|c| c.get()),
            "the weak read must stay under the 75% floor"
        );
        let sweeps = COLUMN_OFFSET_SWEEPS.with(|c| c.get());
        assert!(sweeps >= 1, "the weak read has to sweep");

        FORCED_CELL_SCORE.with(|slot| slot.set(Some(CELL_SCORE_CLEAN)));
        let clean = calibrate_columns(&board, 5);
        assert_eq!(
            COLUMN_OFFSET_SWEEPS.with(|c| c.get()),
            sweeps,
            "a clean anchored read must not sweep"
        );
        assert_eq!(
            clean, want,
            "the clean board gets the header-anchored columns"
        );

        FORCED_CELL_SCORE.with(|slot| slot.set(Some(0)));
        let _again = calibrate_columns(&board, 5);
        assert!(
            COLUMN_OFFSET_SWEEPS.with(|c| c.get()) > sweeps,
            "dropping the below-floor entry lets the next weak read sweep"
        );
        FORCED_CELL_SCORE.with(|slot| slot.set(None));
    }

    /// After a below-floor read, another read whose cached columns stay
    /// under the floor does not inherit the entry.
    #[test]
    fn below_floor_entry_is_not_reused_when_cached_columns_stay_under_the_floor() {
        let _hold = calib_test_guard();
        reset_calib_for_test();
        FORCED_CELL_SCORE.with(|slot| slot.set(Some(0)));
        let board = board_with_stat_headers(1500, 900);
        let _first = calibrate_columns(&board, 5);
        let sweeps = COLUMN_OFFSET_SWEEPS.with(|c| c.get());
        assert!(sweeps >= 1, "the first read has to sweep");
        let _second = calibrate_columns(&board, 5);
        assert!(
            COLUMN_OFFSET_SWEEPS.with(|c| c.get()) > sweeps,
            "columns that stay under the floor are a cold miss"
        );
        FORCED_CELL_SCORE.with(|slot| slot.set(None));
    }

    /// Game end drops a below-floor entry and leaves an above-floor one.
    #[test]
    fn game_end_clears_a_below_floor_column_cache() {
        let _hold = calib_test_guard();
        reset_calib_for_test();
        FORCED_CELL_SCORE.with(|slot| slot.set(Some(0)));
        let board = DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            1400,
            860,
            image::Rgb([20, 24, 30]),
        ));
        let _ = calibrate_columns(&board, 5);
        let sweeps = COLUMN_OFFSET_SWEEPS.with(|c| c.get());
        clear_below_floor_column_cache();
        let _ = calibrate_columns(&board, 5);
        assert!(
            COLUMN_OFFSET_SWEEPS.with(|c| c.get()) > sweeps,
            "game end drops the below-floor entry"
        );

        if let Ok(mut guard) = COLUMN_OFFSET_CACHE.lock() {
            *guard = Some(CalibCache {
                width: 1400,
                height: 860,
                team_size: 5,
                offset: 0.0,
                below_floor: false,
                anchored: false,
                reuses: 0,
                layout: None,
            });
        }
        clear_below_floor_column_cache();
        let kept = COLUMN_OFFSET_CACHE
            .lock()
            .ok()
            .and_then(|guard| guard.as_ref().map(|cached| cached.below_floor));
        assert_eq!(kept, Some(false), "an above-floor entry stays");
        FORCED_CELL_SCORE.with(|slot| slot.set(None));
    }
}

/// Synthetic map-label pictures shared by the OCR tests and the stretch tests.
#[cfg(test)]
pub(crate) mod map_label_fixtures {
    use ab_glyph::{Font, FontRef, PxScale, ScaleFont, point};
    use image::{DynamicImage, Rgb, RgbImage};

    use super::preprocess::{crop_scoreboard, game_rect_16_9};
    use crate::detect::hero_portrait::{ScoreboardPreflight, preflight_scoreboard};

    /// Subset of DejaVu Sans Bold. Licence: `fixtures/map-label/LICENSE`.
    pub(crate) const SANS_BOLD: &[u8] =
        include_bytes!("../../fixtures/map-label/DejaVuSans-Bold.subset.ttf");

    pub(crate) const LIJIANG: &str = "Lijiang Tower";
    pub(crate) const GRIMSVOTN: &str = "Watchpoint: Gr\u{00ed}msv\u{00f6}tn";
    /// 102 / 255 is about 40% brightness.
    pub(crate) const FADED: u8 = 102;
    pub(crate) const LIT: u8 = 236;

    /// Values 40 through 52. Peak-to-peak stays under the stretch floor.
    pub(crate) fn low_noise(w: u32, h: u32) -> DynamicImage {
        let mut img = RgbImage::new(w, h);
        let mut state = 0x1234_5678u32;
        for px in img.pixels_mut() {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            let v = 40 + ((state >> 16) % 13) as u8;
            *px = Rgb([v, v, v]);
        }
        DynamicImage::ImageRgb8(img)
    }

    pub(crate) fn brightness_range(img: &DynamicImage) -> u8 {
        let rgb = img.to_rgb8();
        let mut min_v = 255u8;
        let mut max_v = 0u8;
        for px in rgb.pixels() {
            let v = ((u16::from(px.0[0]) + u16::from(px.0[1]) + u16::from(px.0[2])) / 3) as u8;
            min_v = min_v.min(v);
            max_v = max_v.max(v);
        }
        max_v.saturating_sub(min_v)
    }

    /// One line of synthetic top-bar text. Not a captured frame.
    pub(crate) fn draw_label(text: &str, ink: u8) -> DynamicImage {
        let font = FontRef::try_from_slice(SANS_BOLD).expect("bundled sans bold");
        let scale = PxScale::from(48.0);
        let scaled = font.as_scaled(scale);
        let width = 1200u32;
        let height = 88u32;
        let bg = [18u8, 20, 28];
        let mut img = RgbImage::from_pixel(width, height, Rgb(bg));
        let mut cursor = 20.0f32;
        let baseline = 14.0 + scaled.ascent();
        for ch in text.chars() {
            let id = font.glyph_id(ch);
            let mut glyph = id.with_scale(scale);
            glyph.position = point(cursor, baseline);
            if let Some(outlined) = font.outline_glyph(glyph) {
                let bounds = outlined.px_bounds();
                outlined.draw(|dx, dy, cov| {
                    if cov <= 0.0 {
                        return;
                    }
                    let x = (bounds.min.x + dx as f32).round() as i32;
                    let y = (bounds.min.y + dy as f32).round() as i32;
                    if x < 0 || y < 0 || x >= width as i32 || y >= height as i32 {
                        return;
                    }
                    let dest = img.get_pixel_mut(x as u32, y as u32);
                    let cov = cov.clamp(0.0, 1.0);
                    for channel in 0..3 {
                        let from = f32::from(dest.0[channel]);
                        let to = f32::from(ink);
                        dest.0[channel] = (from + (to - from) * cov).round() as u8;
                    }
                });
            }
            cursor += scaled.h_advance(id);
        }
        assert!(
            cursor < width as f32 - 8.0,
            "{text} ran off the crop at {cursor}"
        );
        DynamicImage::ImageRgb8(img)
    }

    /// 1080p stand-in. `tab` paints a bright header with six dark labels inside
    /// the scoreboard window, which is enough for the preflight to say Ready.
    /// The map-name window holds a bright block so a stretch would be legal.
    pub(crate) fn frame_1080(tab: bool) -> DynamicImage {
        let (w, h) = (1920u32, 1080u32);
        let mut img = RgbImage::from_pixel(w, h, Rgb([12, 14, 22]));
        let (gx, gy, gw, gh) = game_rect_16_9(w, h);
        let mx = gx + (gw as f64 * 0.68) as u32;
        let my = gy + (gh as f64 * 0.022) as u32;
        for py in my..my + 24 {
            for px in mx..mx + 120 {
                img.put_pixel(px, py, Rgb([102, 102, 102]));
            }
        }
        if tab {
            let bx = gx + (gw as f64 * 0.175) as u32;
            let by = gy + (gh as f64 * 0.15) as u32;
            let bw = (gw as f64 * 0.65) as u32;
            let bh = (gh as f64 * 0.70) as u32;
            // Cover the whole header scan (0.5% to 2.5% of the board). A dark
            // row left in that band would count as ink across every column.
            let y0 = by;
            let y1 = by + (bh as f64 * 0.030) as u32;
            for py in y0..y1 {
                for px in bx..bx + bw {
                    img.put_pixel(px, py, Rgb([230, 230, 230]));
                }
            }
            // Six dark labels in the stat area, spaced past the letter merge.
            let origin = bx + (bw as f64 * 0.40) as u32;
            for i in 0..6u32 {
                let x0 = origin + i * 36;
                for py in y0..y1 {
                    for px in x0..x0 + 4 {
                        img.put_pixel(px, py, Rgb([20, 20, 20]));
                    }
                }
            }
        }
        let frame = DynamicImage::ImageRgb8(img);
        let ready = matches!(
            preflight_scoreboard(&crop_scoreboard(&frame)).0,
            ScoreboardPreflight::Ready(_)
        );
        assert_eq!(ready, tab, "synthetic frame tab={tab}");
        frame
    }
}

#[cfg(test)]
mod map_label_ocr_tests {
    use super::map_label_fixtures::{self, FADED, GRIMSVOTN, LIJIANG, LIT};
    use super::preprocess::{self, MAP_LABEL_MIN_RANGE, crop_scoreboard};
    use super::{
        MAP_LABEL_RETRY_LAST_US, MAP_LABEL_RETRY_RUNS, read_map_crop, recognize_map_label,
        recognize_prepared, recognize_region,
    };
    use crate::detect::hero_portrait::{ScoreboardPreflight, preflight_scoreboard};
    use crate::parse::match_map_in_text;
    use image::{DynamicImage, Rgb, RgbImage};

    fn assert_no_map(img: &DynamicImage) {
        assert!(
            preprocess::map_label_retry_image(img).is_none(),
            "stretch ran on a crop that cannot hold a label"
        );
        let before = MAP_LABEL_RETRY_RUNS.with(std::cell::Cell::get);
        let raw = read_map_crop(img, true);
        assert_eq!(
            MAP_LABEL_RETRY_RUNS.with(std::cell::Cell::get),
            before,
            "stretch OCR ran"
        );
        assert!(match_map_in_text(&raw).is_none(), "got a map from {raw:?}");
    }

    #[test]
    fn faded_map_labels_read_from_the_stretched_image() {
        for (text, want) in [
            ("CONTROL | LIJIANG TOWER", LIJIANG),
            ("ESCORT | WATCHPOINT: GRIMSVOTN", GRIMSVOTN),
        ] {
            let img = map_label_fixtures::draw_label(text, FADED);
            assert!(
                map_label_fixtures::brightness_range(&img) >= MAP_LABEL_MIN_RANGE,
                "{text} range collapsed"
            );
            let first = recognize_region(&img).unwrap_or_default();
            if match_map_in_text(&first).is_some() {
                eprintln!("dim {text} matched on the first read ({first:?}); skipping that check");
            }
            let retry = preprocess::map_label_retry_image(&img).expect("stretch");
            let started = std::time::Instant::now();
            let raw = recognize_prepared(&retry, "7", None).expect("retry ocr");
            let elapsed_us = started.elapsed().as_micros();
            eprintln!("map label stretch retry {text:?}: {elapsed_us} us");
            assert_eq!(
                match_map_in_text(&raw).as_deref(),
                Some(want),
                "{text} read {raw:?}"
            );
        }
    }

    #[test]
    fn full_brightness_map_labels_stay_on_the_first_read() {
        for (text, want) in [
            ("CONTROL | LIJIANG TOWER", LIJIANG),
            ("ESCORT | WATCHPOINT: GRIMSVOTN", GRIMSVOTN),
        ] {
            let img = map_label_fixtures::draw_label(text, LIT);
            let first = recognize_region(&img).expect("full brightness ocr");
            assert_eq!(
                match_map_in_text(&first).as_deref(),
                Some(want),
                "first read of {text} changed: {first:?}"
            );
            let before = MAP_LABEL_RETRY_RUNS.with(std::cell::Cell::get);
            let raw = read_map_crop(&img, true);
            assert_eq!(raw, first, "{text} left the full-brightness path");
            assert_eq!(
                MAP_LABEL_RETRY_RUNS.with(std::cell::Cell::get),
                before,
                "a matching first read still retried"
            );
        }
    }

    #[test]
    fn junk_label_stays_unknown() {
        let img = map_label_fixtures::draw_label("ZZZQQQ NOTAMAP", FADED);
        let first = recognize_region(&img).unwrap_or_default();
        let raw = read_map_crop(&img, true);
        assert!(
            match_map_in_text(&raw).is_none(),
            "junk read as a map: {raw:?}"
        );
        assert_eq!(raw, first, "retry replaced a non-matching first read");
    }

    #[test]
    fn flat_grey_blank_and_noise_bars_return_no_map() {
        let flat = DynamicImage::ImageRgb8(RgbImage::from_pixel(320, 48, Rgb([90, 90, 90])));
        let blank = DynamicImage::ImageRgb8(RgbImage::from_pixel(320, 48, Rgb([0, 0, 0])));
        let noise = map_label_fixtures::low_noise(320, 48);
        assert!(map_label_fixtures::brightness_range(&noise) < MAP_LABEL_MIN_RANGE);
        assert_no_map(&flat);
        assert_no_map(&blank);
        assert_no_map(&noise);

        let mut speck = RgbImage::from_pixel(320, 48, Rgb([0, 0, 0]));
        for (x, y) in [(10u32, 10u32), (11, 10), (10, 11), (12, 12)] {
            speck.put_pixel(x, y, Rgb([255, 255, 255]));
        }
        let speck = DynamicImage::ImageRgb8(speck);
        assert!(map_label_fixtures::brightness_range(&speck) >= MAP_LABEL_MIN_RANGE);
        assert_no_map(&speck);
    }

    #[test]
    fn a_non_scoreboard_frame_never_retries() {
        let gameplay = map_label_fixtures::frame_1080(false);
        let tab = map_label_fixtures::frame_1080(true);
        let gameplay_preflight = preflight_scoreboard(&crop_scoreboard(&gameplay)).0;
        let tab_preflight = preflight_scoreboard(&crop_scoreboard(&tab)).0;
        assert!(
            !matches!(gameplay_preflight, ScoreboardPreflight::Ready(_)),
            "gameplay stand-in was classified Ready"
        );
        assert!(
            matches!(tab_preflight, ScoreboardPreflight::Ready(_)),
            "Tab stand-in was not Ready"
        );
        let before = MAP_LABEL_RETRY_RUNS.with(std::cell::Cell::get);
        let raw = recognize_map_label(&gameplay, gameplay_preflight);
        assert_eq!(
            MAP_LABEL_RETRY_RUNS.with(std::cell::Cell::get),
            before,
            "gameplay frame started a stretch retry: {raw:?}"
        );
        assert!(
            match_map_in_text(&raw).is_none(),
            "gameplay frame produced a map: {raw:?}"
        );

        let tab_raw = recognize_map_label(&tab, tab_preflight);
        let runs = MAP_LABEL_RETRY_RUNS.with(std::cell::Cell::get);
        assert!(
            runs > before,
            "Tab frame did not retry a miss ({tab_raw:?})"
        );
        let elapsed_us = MAP_LABEL_RETRY_LAST_US.with(std::cell::Cell::get);
        eprintln!("map label stretch retry on Tab frame: {elapsed_us} us");
        assert!(elapsed_us > 0, "retry timing was not recorded");
    }
}
