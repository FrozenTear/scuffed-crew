pub mod preprocess;

use std::cell::RefCell;
use std::collections::HashMap;
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use image::{DynamicImage, ImageEncoder};
use rayon::prelude::*;

use crate::detect::hero_portrait::detect_team_size;

/// Cached column offset from a previous successful calibration (keyed by board
/// size + team size). Avoids re-running ~300 probe OCRs when the layout is stable.
struct CalibCache {
    width: u32,
    height: u32,
    team_size: usize,
    offset: f64,
}

static COLUMN_OFFSET_CACHE: Mutex<Option<CalibCache>> = Mutex::new(None);

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

// Dedicated thread pool for OCR — capped well below total CPU count so
// a capture burst doesn't saturate the system and lag the game.
static OCR_POOL: OnceLock<rayon::ThreadPool> = OnceLock::new();

fn ocr_pool() -> &'static rayon::ThreadPool {
    OCR_POOL.get_or_init(|| {
        let total = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4);
        // Use at most half the cores, between 2 and 4.
        let threads = (total / 2).clamp(2, 4);
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .thread_name(|i| format!("ocr-{i}"))
            .build()
            .expect("OCR thread pool")
    })
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

/// Run Tesseract on `png` using a thread-local instance for `lang`, returning
/// the recognized text and mean confidence. `whitelist` of `None` (or empty)
/// disables character restriction.
fn ocr_with(
    lang: &'static str,
    psm: &str,
    whitelist: Option<&str>,
    png: &[u8],
) -> Result<(String, i32), Box<dyn std::error::Error + Send + Sync>> {
    TESS_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if !cache.contains_key(lang) {
            let path = tessdata_path_for_lang(lang);
            let path_str = path.as_ref().and_then(|p| p.to_str());
            cache.insert(lang, leptess::LepTess::new(path_str, lang)?);
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

#[derive(Debug)]
pub struct OcrResult {
    pub raw_text: String,
    pub confidence: i32,
}

#[derive(Debug)]
pub struct CellOcrResult {
    pub value: String,
    pub confidence: i32,
}

#[derive(Debug)]
pub struct RowOcrResult {
    pub name: Option<CellOcrResult>,
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

fn tessdata_lang() -> &'static str {
    static LANG: OnceLock<&'static str> = OnceLock::new();
    LANG.get_or_init(|| {
        let has_koverwatch = dirs::data_dir()
            .map(|d| {
                d.join("scuffed-stat-tracker")
                    .join("tessdata")
                    .join("koverwatch.traineddata")
                    .exists()
            })
            .unwrap_or(false);
        if has_koverwatch { "koverwatch" } else { "eng" }
    })
}

pub fn recognize_region(
    img: &DynamicImage,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let png_buf = encode_png(&preprocess::prepare(img))?;
    let (text, _conf) = ocr_with(tessdata_lang(), "7", None, &png_buf)?;
    Ok(text)
}

/// OCR a multi-line screen region (phase headers like "BAN HEROES 13" or
/// "VOTE FOR A MAP") as sparse text. Feeds Tesseract the plain grayscale crop:
/// the scoreboard-tuned `prepare()` white-mask erases stylized/red UI text
/// (measured on the 2026-07 ban-patch screens), and PSM 7 assumes a single
/// line. PSM 11 finds the scattered header + subtitle lines instead.
pub fn recognize_sparse_region(
    img: &DynamicImage,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let png_buf = encode_png(&img.to_luma8())?;
    let (text, _conf) = ocr_with(tessdata_lang(), "11", None, &png_buf)?;
    Ok(text)
}

/// OCR an already-preprocessed grayscale image, skipping the scoreboard-tuned
/// `prepare()` step. Used for regions (like the VICTORY/DEFEAT title) that need
/// their own binarization. `whitelist` of `None` disables character restriction.
pub fn recognize_prepared(
    img: &image::GrayImage,
    psm: &str,
    whitelist: Option<&str>,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let png_buf = encode_png(img)?;
    let (text, _conf) = ocr_with(tessdata_lang(), psm, whitelist, &png_buf)?;
    Ok(text)
}

/// Per-cell OCR: extract and recognize a single stat cell.
/// Uses PSM 7 (single text line) for short numeric strings.
/// `whitelist` controls which characters Tesseract will consider.
pub fn recognize_cell(
    img: &DynamicImage,
) -> Result<CellOcrResult, Box<dyn std::error::Error + Send + Sync>> {
    recognize_cell_with_whitelist(img, "0123456789,")
}

fn recognize_cell_with_whitelist(
    img: &DynamicImage,
    whitelist: &str,
) -> Result<CellOcrResult, Box<dyn std::error::Error + Send + Sync>> {
    let prepared = preprocess::prepare_cell(img);
    if prepared_ink_pixels(&prepared) < MIN_CELL_INK_PIXELS {
        return Ok(CellOcrResult {
            value: String::new(),
            confidence: 0,
        });
    }
    let png_buf = encode_png(&prepared)?;
    let (text, confidence) = ocr_with("eng", "7", Some(whitelist), &png_buf)?;

    Ok(CellOcrResult {
        value: text.trim().to_string(),
        confidence,
    })
}

/// Recognize a player name cell.
pub fn recognize_name(
    img: &DynamicImage,
) -> Result<CellOcrResult, Box<dyn std::error::Error + Send + Sync>> {
    let prepared = preprocess::prepare_name_cell(img);
    let first = if prepared_ink_pixels(&prepared) < MIN_CELL_INK_PIXELS {
        CellOcrResult {
            value: String::new(),
            confidence: 0,
        }
    } else {
        let png_buf = encode_png(&prepared)?;
        let (text, confidence) = ocr_with(tessdata_lang(), "7", None, &png_buf)?;
        CellOcrResult {
            value: text.trim().to_string(),
            confidence,
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
    let (text, confidence) = ocr_with(tessdata_lang(), "7", None, &png_buf)?;
    Ok(CellOcrResult {
        value: text.trim().to_string(),
        confidence,
    })
}

/// Per-row OCR: extract name + all stat cells from a single player row.
pub fn recognize_row(
    row_img: &DynamicImage,
    columns: &preprocess::StatColumns,
    team_size: usize,
) -> RowOcrResult {
    let name = {
        let name_crop = preprocess::crop_name_cell(row_img, team_size);
        recognize_name(&name_crop).ok()
    };

    // Produce exactly STATS_PER_ROW positional cells (E, A, D, DMG, HLG, MIT).
    // Run in parallel — each cell creates its own Tesseract instance so there is
    // no shared state. Index order is preserved by collecting into a fixed-size array.
    let mut stats: Vec<(usize, CellOcrResult)> = ocr_pool().install(|| {
        (0..STATS_PER_ROW)
            .into_par_iter()
            .map(|i| {
                let whitelist = if i < 3 { "0123456789" } else { "0123456789," };
                let result = preprocess::crop_stat_cell(row_img, i, columns)
                    .and_then(|cell| recognize_cell_with_whitelist(&cell, whitelist).ok())
                    .unwrap_or_else(|| CellOcrResult {
                        value: String::new(),
                        confidence: 0,
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
        stats,
        mean_confidence,
    }
}

/// Full scoreboard OCR using per-cell extraction.
/// Returns structured results per row with higher confidence than full-image OCR.
pub fn recognize_scoreboard_cells(img: &DynamicImage) -> Vec<RowOcrResult> {
    recognize_scoreboard_cells_with_team_size(img, None)
}

/// OCR with explicit team size override. Pass None to auto-detect.
pub fn recognize_scoreboard_cells_with_team_size(
    img: &DynamicImage,
    team_size_override: Option<usize>,
) -> Vec<RowOcrResult> {
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
pub fn recognize_scoreboard_cells_pre_cropped(
    cropped: &DynamicImage,
    team_size: usize,
) -> Vec<RowOcrResult> {
    let total_rows = team_size * 2;
    let columns = calibrate_columns(cropped, team_size);

    tracing::debug!(team_size, total_rows, ?columns, "scoreboard layout");

    // Crop all rows first (fast, no OCR), then OCR all rows in parallel.
    let row_images: Vec<(usize, DynamicImage)> = (0..total_rows)
        .filter_map(|i| preprocess::crop_player_row(cropped, i, team_size).map(|img| (i, img)))
        .collect();

    let mut results: Vec<(usize, RowOcrResult)> = ocr_pool().install(|| {
        row_images
            .par_iter()
            .map(|(idx, row_img)| (*idx, recognize_row(row_img, &columns, team_size)))
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

/// Two-phase calibration: coarse sweep centered on the header-detected offset,
/// then fine-tune around the best coarse result. Caches the winning offset per
/// scoreboard resolution/team size and reuses it when a quick re-score holds.
fn calibrate_columns(scoreboard: &DynamicImage, team_size: usize) -> preprocess::StatColumns {
    let header_offset = preprocess::detect_column_offset(scoreboard);
    let (board_w, board_h) = (scoreboard.width(), scoreboard.height());

    let probe_rows: Vec<_> = [0usize, 1, team_size]
        .iter()
        .filter_map(|&i| preprocess::crop_player_row(scoreboard, i, team_size))
        .collect();

    if probe_rows.is_empty() {
        return preprocess::columns_with_offset(header_offset);
    }

    let max_score = (probe_rows.len() * 6) as i32;

    // Score an offset by how many probe-row cells OCR as clean numeric stats.
    // Each candidate offset is independent, so the sweeps are evaluated in
    // parallel on the OCR pool — calibration was previously ~288 serial OCR
    // calls and dominated capture latency.
    let score = |offset: f64| -> i32 {
        let cols = preprocess::columns_with_offset(offset);
        probe_rows.iter().map(|r| count_valid_cells(r, &cols)).sum()
    };

    // Reuse last good offset when the board geometry matches and validity has
    // not degraded (threshold: ≥75% of max probe cells still clean).
    if let Ok(guard) = COLUMN_OFFSET_CACHE.lock()
        && let Some(cached) = guard.as_ref()
        && cached.width == board_w
        && cached.height == board_h
        && cached.team_size == team_size
    {
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
            "cached column offset degraded — recalibrating"
        );
    }

    let best_of = |offsets: Vec<f64>| -> (f64, i32) {
        ocr_pool().install(|| {
            offsets
                .into_par_iter()
                .map(|o| (o, score(o)))
                // Prefer more valid cells; tie-break toward the smaller offset
                // to keep results deterministic across runs.
                .reduce(
                    || (0.0f64, -1i32),
                    |a, b| {
                        if b.1 > a.1 || (b.1 == a.1 && b.0 < a.0) {
                            b
                        } else {
                            a
                        }
                    },
                )
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

    // Early-exit: perfect coarse score means fine sweep cannot improve.
    if best_valid < max_score {
        // Phase 2: fine-tune ±0.02 around the best coarse result in 0.005 steps.
        // Only offsets not already scored in the coarse grid (skip exact coarse
        // centre — already known).
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
            let (fine_best_offset, fine_best_valid) = best_of(fine);
            if fine_best_valid > best_valid {
                best_offset = fine_best_offset;
                best_valid = fine_best_valid;
            }
        }
    }

    if let Ok(mut guard) = COLUMN_OFFSET_CACHE.lock() {
        *guard = Some(CalibCache {
            width: board_w,
            height: board_h,
            team_size,
            offset: best_offset,
        });
    }

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

fn count_valid_cells(row: &DynamicImage, cols: &preprocess::StatColumns) -> i32 {
    let mut valid = 0i32;
    for col_idx in 0..6 {
        if let Some(cell) = preprocess::crop_stat_cell(row, col_idx, cols)
            && let Ok(result) = recognize_cell(&cell)
        {
            let text = result.value.trim();
            if is_clean_stat(text) {
                valid += 1;
            }
        }
    }
    valid
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
pub fn recognize(
    img: &DynamicImage,
) -> Result<OcrResult, Box<dyn std::error::Error + Send + Sync>> {
    let cropped = preprocess::crop_scoreboard(img);

    // Primary path: adaptive thresholding
    let preprocessed = preprocess::prepare_adaptive(&cropped);
    let mut png_buf = Vec::new();
    DynamicImage::ImageLuma8(preprocessed.clone())
        .write_to(&mut Cursor::new(&mut png_buf), image::ImageFormat::Png)?;

    let lang = tessdata_lang();
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
