use std::path::{Path, PathBuf};
use std::process::Command;

const FONT_ZIP_URL: &str = "https://font.download/dl/font/koverwatch.zip";
const FONT_FAMILY: &str = "Koverwatch";

const TRAINING_PAGES: &[TrainingPage] = &[
    TrainingPage {
        text: TRAINING_DIGITS,
        xsize: 2400,
        ysize: 200,
        exposure: 0,
    },
    TrainingPage {
        text: TRAINING_DIGITS_COMMAS,
        xsize: 3200,
        ysize: 300,
        exposure: 0,
    },
    TrainingPage {
        text: TRAINING_HEROES_1,
        xsize: 3600,
        ysize: 400,
        exposure: 0,
    },
    TrainingPage {
        text: TRAINING_HEROES_2,
        xsize: 3600,
        ysize: 400,
        exposure: 0,
    },
    TrainingPage {
        text: TRAINING_MAPS,
        xsize: 3600,
        ysize: 480,
        exposure: 0,
    },
    TrainingPage {
        text: TRAINING_MIXED,
        xsize: 3600,
        ysize: 480,
        exposure: 0,
    },
    TrainingPage {
        text: TRAINING_DIGITS,
        xsize: 1800,
        ysize: 150,
        exposure: 1,
    },
    TrainingPage {
        text: TRAINING_BATTLETAGS,
        xsize: 3600,
        ysize: 400,
        exposure: 0,
    },
    TrainingPage {
        text: TRAINING_SCOREBOARD_SIM,
        xsize: 3600,
        ysize: 600,
        exposure: 0,
    },
    TrainingPage {
        text: TRAINING_DIGITS_COMMAS,
        xsize: 1600,
        ysize: 200,
        exposure: 1,
    },
];

struct TrainingPage {
    text: &'static str,
    xsize: u32,
    ysize: u32,
    exposure: i32,
}

const TRAINING_DIGITS: &str = "\
0 1 2 3 4 5 6 7 8 9\n\
00 01 02 03 04 05 06 07 08 09\n\
10 11 12 13 14 15 16 17 18 19 20\n\
21 22 23 24 25 30 35 40 45 50\n";

const TRAINING_DIGITS_COMMAS: &str = "\
1,234 2,567 3,891 4,012 5,678\n\
6,789 7,890 8,901 9,012 10,234\n\
11,456 12,789 13,012 14,345 15,678\n\
16,901 17,234 18,567 19,890 20,123\n\
21,456 22,789 25,000 30,000 40,000\n\
0 1 2 3 4 5 6 7 8 9 10 11 12\n";

const TRAINING_HEROES_1: &str = "\
Ana Anran Ashe Baptiste Bastion\n\
Brigitte Cassidy D.Va Domina Doomfist\n\
Echo Emre Freja Genji Hanzo\n\
Hazard Illari Junker Queen Junkrat Juno\n\
Kiriko Lifeweaver Lucio Mauga Mei\n";

const TRAINING_HEROES_2: &str = "\
Mercy Mizuki Moira Orisa Pharah\n\
Ramattra Reaper Reinhardt Roadhog Sierra\n\
Sigma Sojourn Soldier: 76 Sombra Symmetra\n\
Torbjorn Tracer Vendetta Venture Widowmaker\n\
Winston Wrecking Ball Wuyang Zarya Zenyatta\n";

const TRAINING_MAPS: &str = "\
King's Row Circuit Royal Dorado Havana\n\
Junkertown Rialto Route 66 Shambali Monastery\n\
Watchpoint: Gibraltar Blizzard World Eichenwalde\n\
Hollywood Midtown Numbani Paraiso\n\
Antarctic Peninsula Busan Ilios Lijiang Tower\n\
Nepal Oasis Samoa Colosseo Esperanca\n\
New Queen Street Runasapi New Junk City\n\
Suravasa Hanaoka Throne of Anubis\n";

const TRAINING_MIXED: &str = "\
VICTORY DEFEAT DRAW\n\
ABCDEFGHIJKLMNOPQRSTUVWXYZ\n\
abcdefghijklmnopqrstuvwxyz\n\
0123456789 / - : . , ( )\n\
ELIMINATIONS ASSISTS DEATHS DAMAGE HEALING MITIGATION\n";

const TRAINING_BATTLETAGS: &str = "\
xXShadow99 ProGamer2024 NightHawk xFireStorm\n\
NoobSlayer CyberWolf360 DragonBlade IceQueen\n\
SilentStrike420 BlazeMaster OmegaForce DarkPhoenix\n\
PixelHunter999 NeonViper SteelTitan CrimsonFox\n\
ThunderGod77 FrostByte GhostRider ToxicAvenger\n";

const TRAINING_SCOREBOARD_SIM: &str = "\
ProGamer Ana 5 3 2 4,521 8,932 0\n\
NightHawk Reinhardt 8 1 4 3,200 0 12,450\n\
DragonBlade Genji 12 5 3 7,891 0 0\n\
IceQueen Mercy 2 15 1 1,234 14,567 0\n\
BlazeMaster Winston 6 4 5 5,678 0 8,901\n\
SteelTitan Zarya 9 3 2 6,789 0 10,234\n\
FrostByte Kiriko 3 12 2 2,345 11,678 0\n\
GhostRider Widowmaker 15 2 4 9,012 0 0\n\
ThunderGod Sigma 4 7 3 4,567 0 15,678\n\
PixelHunter Sojourn 10 4 3 8,234 0 0\n";

const LSTM_MAX_ITERATIONS: u32 = 800;

pub fn tessdata_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("scuffed-stat-tracker")
        .join("tessdata")
}

pub fn ensure_koverwatch_tessdata() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let tessdata = tessdata_dir();
    let traineddata = tessdata.join("koverwatch.traineddata");

    if traineddata.exists() {
        tracing::debug!("koverwatch.traineddata already present");
        return Ok(());
    }

    tracing::info!("koverwatch.traineddata not found — generating via LSTM fine-tuning");

    let tmp = tempfile::tempdir()?;
    let tmp_path = tmp.path();

    download_and_extract_font(tmp_path)?;
    install_font(tmp_path)?;

    let lstm_available = generate_tessdata_lstm(tmp_path);
    if let Err(e) = &lstm_available {
        tracing::warn!(error = %e, "LSTM training failed, falling back to legacy pipeline");
        generate_tessdata_legacy(tmp_path)?;
    }

    std::fs::create_dir_all(&tessdata)?;
    let generated = tmp_path.join("koverwatch.traineddata");
    if !generated.exists() {
        return Err("training pipeline produced no output".into());
    }
    std::fs::copy(&generated, &traineddata)?;
    tracing::info!(path = %traineddata.display(), "koverwatch.traineddata installed");

    Ok(())
}

pub fn regenerate_koverwatch_tessdata() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let tessdata = tessdata_dir();
    let traineddata = tessdata.join("koverwatch.traineddata");

    if traineddata.exists() {
        std::fs::remove_file(&traineddata)?;
        tracing::info!("removed existing koverwatch.traineddata for regeneration");
    }

    ensure_koverwatch_tessdata()
}

fn download_and_extract_font(dir: &Path) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
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

fn install_font(dir: &Path) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let font_dir = dirs::data_dir().ok_or("no data dir")?.join("fonts");
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

fn find_system_traineddata() -> Option<PathBuf> {
    let candidates = [
        PathBuf::from("/usr/share/tessdata/eng.traineddata"),
        PathBuf::from("/usr/local/share/tessdata/eng.traineddata"),
    ];

    if let Ok(prefix) = std::env::var("TESSDATA_PREFIX") {
        let custom = PathBuf::from(prefix).join("eng.traineddata");
        if custom.exists() {
            return Some(custom);
        }
    }

    candidates.into_iter().find(|p| p.exists())
}

fn generate_tessdata_lstm(dir: &Path) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let eng_traineddata = find_system_traineddata()
        .ok_or("eng.traineddata not found — install tesseract-data-eng for LSTM training")?;

    tracing::info!(
        "LSTM fine-tuning: using base model from {}",
        eng_traineddata.display()
    );

    let eng_lstm = dir.join("eng.lstm");
    let result = Command::new("combine_tessdata")
        .arg("-e")
        .arg(&eng_traineddata)
        .arg(&eng_lstm)
        .output()?;
    if !result.status.success() || !eng_lstm.exists() {
        return Err("failed to extract LSTM model from eng.traineddata".into());
    }

    let mut lstmf_files = Vec::new();

    for (i, page) in TRAINING_PAGES.iter().enumerate() {
        let page_name = format!("koverwatch_page{:02}", i);
        let training_txt = dir.join(format!("{page_name}.txt"));
        std::fs::write(&training_txt, page.text)?;

        let output_base = dir.join(&page_name);

        tracing::debug!(
            page = i,
            xsize = page.xsize,
            ysize = page.ysize,
            "generating training image"
        );
        let result = Command::new("text2image")
            .arg("--text")
            .arg(&training_txt)
            .arg("--outputbase")
            .arg(&output_base)
            .arg("--font")
            .arg(FONT_FAMILY)
            .arg(format!("--exposure={}", page.exposure))
            .arg(format!("--xsize={}", page.xsize))
            .arg(format!("--ysize={}", page.ysize))
            .output()?;
        if !result.status.success() {
            tracing::warn!(
                page = i,
                stderr = %String::from_utf8_lossy(&result.stderr),
                "text2image failed for page, skipping"
            );
            continue;
        }

        let tif = dir.join(format!("{page_name}.tif"));
        let box_file = dir.join(format!("{page_name}.box"));
        if !tif.exists() || !box_file.exists() {
            tracing::warn!(page = i, "text2image produced no output, skipping");
            continue;
        }

        let lstmf = dir.join(format!("{page_name}.lstmf"));
        let result = Command::new("tesseract")
            .arg(&tif)
            .arg(&output_base)
            .arg("--psm")
            .arg("6")
            .arg("lstm.train")
            .current_dir(dir)
            .output()?;
        if !result.status.success() {
            tracing::warn!(
                page = i,
                stderr = %String::from_utf8_lossy(&result.stderr),
                "tesseract lstm.train failed, skipping"
            );
            continue;
        }

        if lstmf.exists() {
            lstmf_files.push(lstmf);
        }
    }

    if lstmf_files.is_empty() {
        return Err("no LSTMF training files generated".into());
    }

    tracing::info!(count = lstmf_files.len(), "generated LSTMF training files");

    let train_list = dir.join("train_listfile.txt");
    let list_content: String = lstmf_files
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(&train_list, &list_content)?;

    tracing::info!(
        max_iterations = LSTM_MAX_ITERATIONS,
        "starting LSTM fine-tuning"
    );
    let model_output = dir.join("koverwatch");
    let result = Command::new("lstmtraining")
        .arg("--continue_from")
        .arg(&eng_lstm)
        .arg("--traineddata")
        .arg(&eng_traineddata)
        .arg("--train_listfile")
        .arg(&train_list)
        .arg("--model_output")
        .arg(&model_output)
        .arg("--max_iterations")
        .arg(LSTM_MAX_ITERATIONS.to_string())
        .arg("--target_error_rate")
        .arg("0.01")
        .current_dir(dir)
        .output()?;

    let stderr = String::from_utf8_lossy(&result.stderr);
    tracing::debug!(stderr = %stderr, "lstmtraining output");

    let checkpoint = dir.join("koverwatch_checkpoint");
    if !checkpoint.exists() {
        return Err(format!("lstmtraining produced no checkpoint: {}", stderr).into());
    }

    tracing::info!("finalizing traineddata from checkpoint");
    let result = Command::new("lstmtraining")
        .arg("--stop_training")
        .arg("--continue_from")
        .arg(&checkpoint)
        .arg("--traineddata")
        .arg(&eng_traineddata)
        .arg("--model_output")
        .arg(dir.join("koverwatch.traineddata"))
        .current_dir(dir)
        .output()?;
    if !result.status.success() {
        return Err(format!(
            "lstmtraining --stop_training failed: {}",
            String::from_utf8_lossy(&result.stderr)
        )
        .into());
    }

    Ok(())
}

fn generate_tessdata_legacy(dir: &Path) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing::info!("using legacy Tesseract 3.x training pipeline as fallback");

    let legacy_text = format!(
        "{}\n{}\n{}\n{}\n{}",
        TRAINING_DIGITS,
        TRAINING_DIGITS_COMMAS,
        TRAINING_HEROES_1,
        TRAINING_HEROES_2,
        TRAINING_MIXED,
    );
    let training_txt = dir.join("training_text.txt");
    std::fs::write(&training_txt, &legacy_text)?;

    let output_base = dir.join("koverwatch");

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
