pub mod preprocess;

use std::io::Cursor;
use std::path::PathBuf;

use image::DynamicImage;

#[derive(Debug)]
pub struct OcrResult {
    pub raw_text: String,
    pub confidence: i32,
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
    lt.set_image_from_mem(&png_buf)?;

    Ok(lt.get_utf8_text()?)
}

pub fn recognize(img: &DynamicImage) -> Result<OcrResult, Box<dyn std::error::Error + Send + Sync>> {
    let cropped = preprocess::crop_scoreboard(img);

    // Try multiple thresholds to handle varying in-game lighting conditions.
    // Different maps/times produce different overlay brightness levels.
    let thresholds: &[u8] = &[120, 140, 160];
    let lang = tessdata_lang();
    let mut best: Option<OcrResult> = None;
    let mut best_threshold = 140u8;

    for &threshold in thresholds {
        let preprocessed = preprocess::prepare_with_threshold(&cropped, threshold);
        let mut png_buf = Vec::new();
        if DynamicImage::ImageLuma8(preprocessed)
            .write_to(&mut Cursor::new(&mut png_buf), image::ImageFormat::Png)
            .is_err()
        {
            continue;
        }

        if let Ok(result) = run_ocr(lang, &png_buf) {
            let dominated = best.as_ref().is_some_and(|b| b.confidence >= result.confidence);
            if !dominated {
                best_threshold = threshold;
                best = Some(result);
            }
            // Short-circuit: if we hit high confidence, no need to try more thresholds
            if best.as_ref().is_some_and(|b| b.confidence >= 70) {
                break;
            }
        }
    }

    // Re-render the best threshold for debug images and eng fallback
    let best_preprocessed = preprocess::prepare_with_threshold(&cropped, best_threshold);
    let mut png_buf = Vec::new();
    DynamicImage::ImageLuma8(best_preprocessed)
        .write_to(&mut Cursor::new(&mut png_buf), image::ImageFormat::Png)?;

    if let Some(data_dir) = dirs::data_dir() {
        let debug_dir = data_dir.join("scuffed-stat-tracker").join("debug");
        let _ = std::fs::create_dir_all(&debug_dir);
        let _ = cropped.save(debug_dir.join("crop.png"));
        let _ = std::fs::write(debug_dir.join("preprocessed.png"), &png_buf);
        tracing::debug!(path = %debug_dir.display(), threshold = best_threshold, "saved debug images");
    }

    let primary = best.ok_or_else(|| -> Box<dyn std::error::Error + Send + Sync> {
        "all OCR threshold attempts failed".into()
    })?;

    tracing::debug!(
        confidence = primary.confidence,
        threshold = best_threshold,
        lang,
        "best OCR result from threshold sweep"
    );

    // Try eng fallback if primary confidence is low
    let fallback_lang = if lang == "koverwatch" { "eng" } else { return Ok(primary); };

    if primary.confidence >= 65 {
        return Ok(primary);
    }

    match run_ocr(fallback_lang, &png_buf) {
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

fn run_ocr(
    lang: &str,
    png_buf: &[u8],
) -> Result<OcrResult, Box<dyn std::error::Error + Send + Sync>> {
    let path = tessdata_path_for_lang(lang);
    let path_str = path.as_ref().and_then(|p| p.to_str());

    let mut lt = leptess::LepTess::new(path_str, lang)?;
    lt.set_image_from_mem(png_buf)?;

    let text = lt.get_utf8_text()?;
    let confidence = lt.mean_text_conf();

    tracing::debug!(confidence, text_len = text.len(), lang, "OCR complete");

    Ok(OcrResult {
        raw_text: text,
        confidence,
    })
}
