use std::path::{Path, PathBuf};
use std::process::Command;

const FONT_ZIP_URL: &str = "https://font.download/dl/font/koverwatch.zip";
const FONT_FAMILY: &str = "Koverwatch";

const TRAINING_TEXT: &str = "\
0123456789 VICTORY DEFEAT DRAW\n\
ABCDEFGHIJKLMNOPQRSTUVWXYZ\n\
abcdefghijklmnopqrstuvwxyz\n\
/ - : . , ( )\n";

pub fn tessdata_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("scuffed-stat-tracker")
        .join("tessdata")
}

pub fn ensure_koverwatch_tessdata() -> Result<(), Box<dyn std::error::Error>> {
    let tessdata = tessdata_dir();
    let traineddata = tessdata.join("koverwatch.traineddata");

    if traineddata.exists() {
        tracing::debug!("koverwatch.traineddata already present");
        return Ok(());
    }

    tracing::info!("koverwatch.traineddata not found — generating from font");

    let tmp = tempfile::tempdir()?;
    let tmp_path = tmp.path();

    download_and_extract_font(tmp_path)?;
    install_font(tmp_path)?;
    generate_tessdata(tmp_path)?;

    std::fs::create_dir_all(&tessdata)?;
    let generated = tmp_path.join("koverwatch.traineddata");
    if !generated.exists() {
        return Err("training pipeline produced no output".into());
    }
    std::fs::copy(&generated, &traineddata)?;
    tracing::info!(path = %traineddata.display(), "koverwatch.traineddata installed");

    Ok(())
}

fn download_and_extract_font(dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    tracing::info!("downloading Koverwatch font");
    let zip_path = dir.join("koverwatch.zip");

    let output = Command::new("curl")
        .args(["-sL", FONT_ZIP_URL, "-o"])
        .arg(&zip_path)
        .output()?;
    if !output.status.success() {
        return Err(format!("curl failed: {}", String::from_utf8_lossy(&output.stderr)).into());
    }

    let output = Command::new("unzip")
        .args(["-o"])
        .arg(&zip_path)
        .arg("-d")
        .arg(dir)
        .output()?;
    if !output.status.success() {
        return Err(format!("unzip failed: {}", String::from_utf8_lossy(&output.stderr)).into());
    }

    Ok(())
}

fn install_font(dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let font_dir = dirs::data_dir()
        .ok_or("no data dir")?
        .join("fonts");
    std::fs::create_dir_all(&font_dir)?;

    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("ttf") {
            let dest = font_dir.join(entry.file_name());
            std::fs::copy(&path, &dest)?;
            tracing::debug!(font = %dest.display(), "installed font");
        }
    }

    Command::new("fc-cache").arg("-f").output().ok();
    Ok(())
}

fn generate_tessdata(dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let training_txt = dir.join("training_text.txt");
    std::fs::write(&training_txt, TRAINING_TEXT)?;

    let output_base = dir.join("koverwatch");

    tracing::info!("generating training images with text2image");
    let result = Command::new("text2image")
        .arg("--text")
        .arg(&training_txt)
        .arg("--outputbase")
        .arg(&output_base)
        .arg("--font")
        .arg(FONT_FAMILY)
        .arg("--exposure=0")
        .arg("--xsize=3600")
        .arg("--ysize=480")
        .output()?;
    if !result.status.success() {
        return Err(format!(
            "text2image failed: {}",
            String::from_utf8_lossy(&result.stderr)
        )
        .into());
    }

    let tif = dir.join("koverwatch.tif");
    if !tif.exists() {
        return Err("text2image produced no .tif output".into());
    }

    tracing::info!("running tesseract training");
    let result = Command::new("tesseract")
        .arg(&tif)
        .arg(&output_base)
        .arg("box.train")
        .current_dir(dir)
        .output()?;
    if !result.status.success() {
        return Err(format!(
            "tesseract box.train failed: {}",
            String::from_utf8_lossy(&result.stderr)
        )
        .into());
    }

    let box_file = dir.join("koverwatch.box");
    let result = Command::new("unicharset_extractor")
        .arg(&box_file)
        .current_dir(dir)
        .output()?;
    if !result.status.success() {
        return Err("unicharset_extractor failed".into());
    }

    let font_properties = dir.join("font_properties");
    std::fs::write(&font_properties, "Koverwatch 0 0 0 0 0\n")?;

    let tr_file = dir.join("koverwatch.tr");
    let result = Command::new("mftraining")
        .args(["-F"])
        .arg(&font_properties)
        .args(["-U"])
        .arg(dir.join("unicharset"))
        .args(["-O"])
        .arg(dir.join("koverwatch.unicharset"))
        .arg(&tr_file)
        .current_dir(dir)
        .output()?;
    if !result.status.success() {
        return Err(format!(
            "mftraining failed: {}",
            String::from_utf8_lossy(&result.stderr)
        )
        .into());
    }

    let result = Command::new("cntraining")
        .arg(&tr_file)
        .current_dir(dir)
        .output()?;
    if !result.status.success() {
        return Err("cntraining failed".into());
    }

    for name in ["inttemp", "normproto", "pffmtable", "shapetable"] {
        let src = dir.join(name);
        let dst = dir.join(format!("koverwatch.{name}"));
        if src.exists() {
            std::fs::rename(&src, &dst)?;
        }
    }

    tracing::info!("combining tessdata");
    let result = Command::new("combine_tessdata")
        .arg(dir.join("koverwatch."))
        .current_dir(dir)
        .output()?;
    if !result.status.success() {
        return Err(format!(
            "combine_tessdata failed: {}",
            String::from_utf8_lossy(&result.stderr)
        )
        .into());
    }

    Ok(())
}
