pub mod preprocess;

use std::cell::RefCell;
use std::collections::HashMap;
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::OnceLock;

use image::DynamicImage;
use rayon::prelude::*;

use crate::detect::hero_portrait::detect_team_size;

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
fn encode_png(
    img: image::GrayImage,
) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    let mut buf = Vec::new();
    DynamicImage::ImageLuma8(img).write_to(&mut Cursor::new(&mut buf), image::ImageFormat::Png)?;
    Ok(buf)
}

/// Number of stat columns per scoreboard row: E, A, D, DMG, HLG, MIT.
const STATS_PER_ROW: usize = 6;

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
    let system = PathBuf::from("/usr/share/tessdata");
    if system.join(format!("{lang}.traineddata")).exists() {
        return Some(system);
    }
    None
}

fn tessdata_lang() -> &'static str {
    let has_koverwatch = dirs::data_dir()
        .map(|d| {
            d.join("scuffed-stat-tracker")
                .join("tessdata")
                .join("koverwatch.traineddata")
                .exists()
        })
        .unwrap_or(false);
    if has_koverwatch { "koverwatch" } else { "eng" }
}

pub fn recognize_region(
    img: &DynamicImage,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let png_buf = encode_png(preprocess::prepare(img))?;
    let (text, _conf) = ocr_with(tessdata_lang(), "7", None, &png_buf)?;
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
    let png_buf = encode_png(img.clone())?;
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
    let png_buf = encode_png(preprocess::prepare_cell(img))?;
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
    let png_buf = encode_png(preprocess::prepare_name_cell(img))?;
    let (text, confidence) = ocr_with(tessdata_lang(), "7", None, &png_buf)?;

    Ok(CellOcrResult {
        value: text.trim().to_string(),
        confidence,
    })
}

/// Per-row OCR: extract name + all stat cells from a single player row.
pub fn recognize_row(row_img: &DynamicImage, columns: &preprocess::StatColumns) -> RowOcrResult {
    let name = {
        let name_crop = preprocess::crop_name_cell(row_img);
        recognize_name(&name_crop).ok()
    };

    // Produce exactly STATS_PER_ROW positional cells (E, A, D, DMG, HLG, MIT).
    // Run in parallel — each cell creates its own Tesseract instance so there is
    // no shared state. Index order is preserved by collecting into a fixed-size array.
    let mut stats: Vec<(usize, CellOcrResult)> = ocr_pool().install(|| {
        (0..STATS_PER_ROW).into_par_iter().map(|i| {
            let whitelist = if i < 3 { "0123456789" } else { "0123456789," };
            let result = preprocess::crop_stat_cell(row_img, i, columns)
                .and_then(|cell| recognize_cell_with_whitelist(&cell, whitelist).ok())
                .unwrap_or_else(|| CellOcrResult {
                    value: String::new(),
                    confidence: 0,
                });
            (i, result)
        }).collect()
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
    let total_rows = team_size * 2;
    let columns = calibrate_columns(&cropped, team_size);

    tracing::debug!(team_size, total_rows, ?columns, "scoreboard layout");

    // Crop all rows first (fast, no OCR), then OCR all rows in parallel.
    let row_images: Vec<(usize, DynamicImage)> = (0..total_rows)
        .filter_map(|i| preprocess::crop_player_row(&cropped, i, team_size).map(|img| (i, img)))
        .collect();

    let mut results: Vec<(usize, RowOcrResult)> = ocr_pool().install(|| {
        row_images
            .par_iter()
            .map(|(idx, row_img)| (*idx, recognize_row(row_img, &columns)))
            .collect()
    });

    results.sort_unstable_by_key(|(idx, _)| *idx);
    let results: Vec<RowOcrResult> = results.into_iter().map(|(_, r)| r).collect();

    if let Some(data_dir) = dirs::data_dir() {
        let debug_dir = data_dir.join("scuffed-stat-tracker").join("debug");
        let _ = std::fs::create_dir_all(&debug_dir);
        preprocess::save_debug_stages(&cropped, &debug_dir);
        for (idx, row_img) in &row_images {
            let _ = row_img.save(debug_dir.join(format!("row_{idx:02}.png")));
        }
    }

    results
}

/// Two-phase calibration: coarse sweep centered on the header-detected offset,
/// then fine-tune around the best coarse result.
fn calibrate_columns(scoreboard: &DynamicImage, team_size: usize) -> preprocess::StatColumns {
    let header_offset = preprocess::detect_column_offset(scoreboard);

    let probe_rows: Vec<_> = [0usize, 1, team_size]
        .iter()
        .filter_map(|&i| preprocess::crop_player_row(scoreboard, i, team_size))
        .collect();

    if probe_rows.is_empty() {
        return preprocess::columns_with_offset(header_offset);
    }

    // Score an offset by how many probe-row cells OCR as clean numeric stats.
    // Each candidate offset is independent, so the sweeps are evaluated in
    // parallel on the OCR pool — calibration was previously ~288 serial OCR
    // calls and dominated capture latency.
    let score = |offset: f64| -> i32 {
        let cols = preprocess::columns_with_offset(offset);
        probe_rows.iter().map(|r| count_valid_cells(r, &cols)).sum()
    };
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

    // Phase 2: fine-tune ±0.02 around the best coarse result in 0.005 steps
    let mut fine = Vec::new();
    let mut fine_offset = best_offset - 0.02;
    let fine_end = best_offset + 0.02;
    while fine_offset <= fine_end {
        fine.push(fine_offset);
        fine_offset += 0.005;
    }
    let (fine_best_offset, fine_best_valid) = best_of(fine);
    if fine_best_valid > best_valid {
        best_offset = fine_best_offset;
        best_valid = fine_best_valid;
    }

    let board_w = scoreboard.width() as f64;
    tracing::debug!(
        header_offset_px = (header_offset * board_w) as i32,
        final_offset_px = (best_offset * board_w) as i32,
        valid_cells = best_valid,
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

fn is_clean_stat(text: &str) -> bool {
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
    if let Some(data_dir) = dirs::data_dir() {
        let debug_dir = data_dir.join("scuffed-stat-tracker").join("debug");
        let _ = std::fs::create_dir_all(&debug_dir);
        let _ = cropped.save(debug_dir.join("crop.png"));
        let _ = std::fs::write(debug_dir.join("preprocessed.png"), png_buf);
        preprocess::save_debug_stages(cropped, &debug_dir);
        tracing::debug!(path = %debug_dir.display(), "saved debug images");
    }
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
