pub mod preprocess;

use std::io::Cursor;
use std::path::PathBuf;

use image::DynamicImage;

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
    let custom = dirs::data_dir()
        .map(|d| d.join("scuffed-stat-tracker").join("tessdata"));
    if let Some(ref custom_dir) = custom {
        if custom_dir.join(format!("{lang}.traineddata")).exists() {
            return Some(custom_dir.clone());
        }
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
    if has_koverwatch {
        "koverwatch"
    } else {
        "eng"
    }
}

pub fn recognize_region(img: &DynamicImage) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let preprocessed = preprocess::prepare(img);

    let mut png_buf = Vec::new();
    DynamicImage::ImageLuma8(preprocessed)
        .write_to(&mut Cursor::new(&mut png_buf), image::ImageFormat::Png)?;

    let lang = tessdata_lang();
    let path = tessdata_path_for_lang(lang);
    let path_str = path.as_ref().and_then(|p| p.to_str());

    let mut lt = leptess::LepTess::new(path_str, lang)?;
    lt.set_variable(leptess::Variable::TesseditPagesegMode, "7")?;
    lt.set_image_from_mem(&png_buf)?;

    Ok(lt.get_utf8_text()?)
}

/// Per-cell OCR: extract and recognize a single stat cell.
/// Uses PSM 7 (single text line) for short numeric strings.
pub fn recognize_cell(img: &DynamicImage) -> Result<CellOcrResult, Box<dyn std::error::Error + Send + Sync>> {
    let preprocessed = preprocess::prepare_cell(img);

    let mut png_buf = Vec::new();
    DynamicImage::ImageLuma8(preprocessed)
        .write_to(&mut Cursor::new(&mut png_buf), image::ImageFormat::Png)?;

    let lang = tessdata_lang();
    let path = tessdata_path_for_lang(lang);
    let path_str = path.as_ref().and_then(|p| p.to_str());

    let mut lt = leptess::LepTess::new(path_str, lang)?;
    lt.set_variable(leptess::Variable::TesseditPagesegMode, "7")?;
    // Restrict to digits and comma for stat cells
    lt.set_variable(leptess::Variable::TesseditCharWhitelist, "0123456789,")?;
    lt.set_image_from_mem(&png_buf)?;

    let text = lt.get_utf8_text()?.trim().to_string();
    let confidence = lt.mean_text_conf();

    Ok(CellOcrResult {
        value: text,
        confidence,
    })
}

/// Recognize a player name cell.
pub fn recognize_name(img: &DynamicImage) -> Result<CellOcrResult, Box<dyn std::error::Error + Send + Sync>> {
    let preprocessed = preprocess::prepare_name_cell(img);

    let mut png_buf = Vec::new();
    DynamicImage::ImageLuma8(preprocessed)
        .write_to(&mut Cursor::new(&mut png_buf), image::ImageFormat::Png)?;

    let lang = tessdata_lang();
    let path = tessdata_path_for_lang(lang);
    let path_str = path.as_ref().and_then(|p| p.to_str());

    let mut lt = leptess::LepTess::new(path_str, lang)?;
    lt.set_variable(leptess::Variable::TesseditPagesegMode, "7")?;
    lt.set_image_from_mem(&png_buf)?;

    let text = lt.get_utf8_text()?.trim().to_string();
    let confidence = lt.mean_text_conf();

    Ok(CellOcrResult {
        value: text,
        confidence,
    })
}

/// Per-row OCR: extract name + all stat cells from a single player row.
pub fn recognize_row(row_img: &DynamicImage) -> RowOcrResult {
    let name = {
        let name_crop = preprocess::crop_name_cell(row_img);
        recognize_name(&name_crop).ok()
    };

    let cells = preprocess::extract_row_cells(row_img);
    let stats: Vec<CellOcrResult> = cells
        .iter()
        .filter_map(|cell| recognize_cell(cell).ok())
        .collect();

    let confidences: Vec<i32> = stats.iter().map(|s| s.confidence).collect();
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
    let cropped = preprocess::crop_scoreboard(img);
    let mut results = Vec::new();

    for row_idx in 0..10 {
        if let Some(row_img) = preprocess::crop_player_row(&cropped, row_idx) {
            results.push(recognize_row(&row_img));
        }
    }

    if let Some(data_dir) = dirs::data_dir() {
        let debug_dir = data_dir.join("scuffed-stat-tracker").join("debug");
        let _ = std::fs::create_dir_all(&debug_dir);
        preprocess::save_debug_stages(&cropped, &debug_dir);

        // Save individual row crops for debugging
        for (idx, row_img) in (0..10)
            .filter_map(|i| preprocess::crop_player_row(&cropped, i).map(|r| (i, r)))
        {
            let _ = row_img.save(debug_dir.join(format!("row_{idx:02}.png")));
        }
    }

    results
}

/// Full-image OCR (original approach with adaptive preprocessing).
/// Used as fallback and for compatibility with the existing parse pipeline.
pub fn recognize(img: &DynamicImage) -> Result<OcrResult, Box<dyn std::error::Error + Send + Sync>> {
    let cropped = preprocess::crop_scoreboard(img);

    // Primary path: adaptive thresholding
    let preprocessed = preprocess::prepare_adaptive(&cropped);
    let mut png_buf = Vec::new();
    DynamicImage::ImageLuma8(preprocessed.clone())
        .write_to(&mut Cursor::new(&mut png_buf), image::ImageFormat::Png)?;

    let lang = tessdata_lang();
    let primary = run_ocr(lang, &png_buf)?;

    tracing::debug!(
        confidence = primary.confidence,
        lang,
        "adaptive OCR result"
    );

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
            let dominated = best.as_ref().is_some_and(|b| b.confidence >= result.confidence);
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
    lang: &str,
    primary: OcrResult,
    png_buf: &[u8],
) -> Result<OcrResult, Box<dyn std::error::Error + Send + Sync>> {
    let fallback_lang = if lang == "koverwatch" { "eng" } else { return Ok(primary); };

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
    lang: &str,
    png_buf: &[u8],
) -> Result<OcrResult, Box<dyn std::error::Error + Send + Sync>> {
    let path = tessdata_path_for_lang(lang);
    let path_str = path.as_ref().and_then(|p| p.to_str());

    let mut lt = leptess::LepTess::new(path_str, lang)?;
    lt.set_variable(leptess::Variable::TesseditPagesegMode, "6")?;
    lt.set_image_from_mem(png_buf)?;

    let text = lt.get_utf8_text()?;
    let confidence = lt.mean_text_conf();

    tracing::debug!(confidence, text_len = text.len(), lang, "OCR complete");

    Ok(OcrResult {
        raw_text: text,
        confidence,
    })
}
