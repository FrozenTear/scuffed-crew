pub mod preprocess;

use std::io::Cursor;
use std::path::PathBuf;

use image::DynamicImage;

#[derive(Debug)]
pub struct OcrResult {
    pub raw_text: String,
    pub confidence: i32,
}

fn tessdata_path() -> Option<PathBuf> {
    let custom = dirs::data_dir()?.join("scuffed-stat-tracker").join("tessdata");
    if custom.exists() {
        return Some(custom);
    }
    let system = PathBuf::from("/usr/share/tessdata");
    if system.exists() {
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

    let path = tessdata_path();
    let path_str = path.as_ref().and_then(|p| p.to_str());

    let mut lt = leptess::LepTess::new(path_str, tessdata_lang())?;
    lt.set_image_from_mem(&png_buf)?;

    Ok(lt.get_utf8_text()?)
}

pub fn recognize(img: &DynamicImage) -> Result<OcrResult, Box<dyn std::error::Error + Send + Sync>> {
    let cropped = preprocess::crop_scoreboard(img);
    let preprocessed = preprocess::prepare(&cropped);

    let mut png_buf = Vec::new();
    DynamicImage::ImageLuma8(preprocessed)
        .write_to(&mut Cursor::new(&mut png_buf), image::ImageFormat::Png)?;

    let path = tessdata_path();
    let path_str = path.as_ref().and_then(|p| p.to_str());
    let lang = tessdata_lang();

    let mut lt = leptess::LepTess::new(path_str, lang)?;
    lt.set_image_from_mem(&png_buf)?;

    let text = lt.get_utf8_text()?;
    let confidence = lt.mean_text_conf();

    tracing::debug!(confidence, text_len = text.len(), lang, "OCR complete");

    Ok(OcrResult {
        raw_text: text,
        confidence,
    })
}
