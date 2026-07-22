use std::path::{Path, PathBuf};
use std::process::Command;

const FONT_ZIP_URL: &str = "https://font.download/dl/font/koverwatch.zip";
const FONT_FAMILY: &str = "Koverwatch";

/// Float ("best") English LSTM model. System `eng.traineddata` is usually the
/// tessdata_fast integer variant (Arch and Ubuntu both ship it), which
/// lstmtraining cannot `--continue_from` ("eng.lstm is an integer (fast)
/// model, cannot continue training"). Fine-tuning requires the float model, so
/// we fetch it on demand. Generation already needs network (font download), so
/// this adds no new requirement.
const TESSDATA_BEST_ENG_URL: &str = "https://github.com/tesseract-ocr/tessdata_best/raw/e12c65a915945e4c28e237a9b52bc4a8f39a0cec/eng.traineddata";

/// text2image busy-spins forever (100% CPU, no output, no error) when a page's
/// text does not fit its --ysize — it is not slow, it never finishes, so the
/// timeout is a backstop for that spin, not a render budget. Reproduced on
/// Debian bookworm (pango 1.50) 2026-07-22; the 2026-07-21 "killed mid-render"
/// reading was this same spin. Page geometry below must keep every page
/// comfortably taller than its text (~60px per line at the default 12pt/300dpi
/// plus margin). On pango >= 1.56 hosts text2image also hangs/segfaults
/// regardless of geometry — releases ship a CI-trained koverwatch.traineddata
/// so end-user machines never run this pipeline.
const TEXT2IMAGE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(180);

const TRAINING_PAGES: &[TrainingPage] = &[
    TrainingPage {
        text: TRAINING_DIGITS,
        xsize: 1200,
        ysize: 300,
        exposure: 0,
    },
    TrainingPage {
        text: TRAINING_DIGITS_COMMAS,
        xsize: 1600,
        ysize: 420,
        exposure: 0,
    },
    TrainingPage {
        text: TRAINING_HEROES_1,
        xsize: 1800,
        ysize: 360,
        exposure: 0,
    },
    TrainingPage {
        text: TRAINING_HEROES_2,
        xsize: 1800,
        ysize: 360,
        exposure: 0,
    },
    TrainingPage {
        text: TRAINING_MAPS,
        xsize: 1800,
        ysize: 540,
        exposure: 0,
    },
    TrainingPage {
        text: TRAINING_MIXED,
        xsize: 1800,
        ysize: 360,
        exposure: 0,
    },
    TrainingPage {
        text: TRAINING_DIGITS,
        xsize: 900,
        ysize: 300,
        exposure: 1,
    },
    TrainingPage {
        text: TRAINING_BATTLETAGS,
        xsize: 1800,
        ysize: 360,
        exposure: 0,
    },
    TrainingPage {
        text: TRAINING_SCOREBOARD_SIM,
        xsize: 1800,
        ysize: 660,
        exposure: 0,
    },
    TrainingPage {
        text: TRAINING_DIGITS_COMMAS,
        xsize: 800,
        ysize: 420,
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

/// Candidate system directories that may contain `{lang}.traineddata`.
///
/// Distros disagree on layout:
/// - Arch / many: `/usr/share/tessdata`
/// - Debian/Ubuntu packages: `/usr/share/tesseract-ocr/<ver>/tessdata`
/// - Fedora: `/usr/share/tesseract/tessdata`
/// - overrides: `TESSDATA_PREFIX` (file dir or parent of `tessdata/`)
pub fn system_tessdata_candidates() -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    if let Ok(prefix) = std::env::var("TESSDATA_PREFIX") {
        let p = PathBuf::from(prefix);
        // Tesseract accepts either the tessdata directory itself or its parent.
        dirs.push(p.clone());
        if p.file_name().and_then(|n| n.to_str()) != Some("tessdata") {
            dirs.push(p.join("tessdata"));
        }
    }

    dirs.push(PathBuf::from("/usr/share/tessdata"));
    dirs.push(PathBuf::from("/usr/local/share/tessdata"));
    dirs.push(PathBuf::from("/usr/share/tesseract/tessdata")); // Fedora

    // Debian/Ubuntu versioned layouts: /usr/share/tesseract-ocr/*/tessdata
    if let Ok(entries) = std::fs::read_dir("/usr/share/tesseract-ocr") {
        for entry in entries.flatten() {
            let candidate = entry.path().join("tessdata");
            if candidate.is_dir() {
                dirs.push(candidate);
            }
        }
    }

    dirs
}

/// Return the directory containing `{lang}.traineddata` on the system, if any.
pub fn find_system_traineddata(lang: &str) -> Option<PathBuf> {
    let file = format!("{lang}.traineddata");
    system_tessdata_candidates()
        .into_iter()
        .find(|dir| dir.join(&file).is_file())
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

/// Download the float "best" English model into `dir` for LSTM fine-tuning.
/// Returns the path to the downloaded `eng.traineddata`.
fn download_best_eng_traineddata(
    dir: &Path,
) -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    tracing::info!("downloading tessdata_best eng.traineddata for LSTM training");
    let dest = dir.join("eng.traineddata");

    let output = Command::new("curl")
        .args(["-sL", TESSDATA_BEST_ENG_URL, "-o"])
        .arg(&dest)
        .output()?;
    if !output.status.success() {
        return Err(format!(
            "curl failed to download best eng.traineddata: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }

    // The best eng model is ~15MB; anything under 1MB means the download failed
    // (e.g. an HTML error page from GitHub) rather than the real float model.
    let size = std::fs::metadata(&dest).map(|m| m.len()).unwrap_or(0);
    if size < 1_000_000 {
        return Err(format!(
            "downloaded eng.traineddata is too small ({size} bytes) — expected ~15MB float 'best' model; download likely failed"
        )
        .into());
    }

    tracing::info!(bytes = size, "downloaded tessdata_best eng.traineddata");
    Ok(dest)
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

fn generate_tessdata_lstm(dir: &Path) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // lstmtraining needs the float "best" model — the system eng.traineddata is
    // usually the integer "fast" variant it refuses to continue from. Fetch the
    // best model on demand into the training temp dir.
    let eng_traineddata = download_best_eng_traineddata(dir)?;

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
        let result = run_with_timeout(
            Command::new("text2image")
                .arg("--text")
                .arg(&training_txt)
                .arg("--outputbase")
                .arg(&output_base)
                .arg("--font")
                .arg(FONT_FAMILY)
                // text2image builds its own fontconfig in a temp dir and never
                // scans the user font dir install_font populates — the font is
                // only found via an explicit --fonts_dir. The zip extracts the
                // ttf straight into `dir`, so point it there.
                .arg("--fonts_dir")
                .arg(dir)
                .arg(format!("--exposure={}", page.exposure))
                .arg(format!("--xsize={}", page.xsize))
                .arg(format!("--ysize={}", page.ysize)),
            TEXT2IMAGE_TIMEOUT,
        );
        match result {
            Ok(o) if o.status.success() => {}
            Ok(o) => {
                tracing::warn!(
                    page = i,
                    stderr = %String::from_utf8_lossy(&o.stderr),
                    "text2image failed for page, skipping"
                );
                continue;
            }
            Err(e) => {
                tracing::warn!(page = i, error = %e, "text2image timed out or errored, skipping");
                continue;
            }
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

/// Spawn a command with a wall-clock timeout. Kills the child if it exceeds the limit.
fn run_with_timeout(
    cmd: &mut Command,
    timeout: std::time::Duration,
) -> Result<std::process::Output, Box<dyn std::error::Error + Send + Sync>> {
    let mut child = cmd.spawn()?;
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait()? {
            let output = std::process::Output {
                status,
                stdout: Vec::new(),
                stderr: Vec::new(),
            };
            return Ok(output);
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            return Err("process timed out".into());
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
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

    let result = run_with_timeout(
        Command::new("text2image")
            .arg("--text")
            .arg(&training_txt)
            .arg("--outputbase")
            .arg(&output_base)
            .arg("--font")
            .arg(FONT_FAMILY)
            // Same --fonts_dir requirement as the LSTM path: without it,
            // text2image cannot see the extracted ttf in `dir`.
            .arg("--fonts_dir")
            .arg(dir)
            .arg("--exposure=0")
            .arg("--xsize=1800")
            .arg("--ysize=240"),
        TEXT2IMAGE_TIMEOUT,
    )?;
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
