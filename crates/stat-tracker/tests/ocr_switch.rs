//! The OCR on/off switch (`ocr` in config.toml), in a process of its own.
//!
//! * The gate: with OCR off every public `ocr` function is blocked, counted
//!   under the line that called it, and nothing of Tesseract is started (no
//!   `LepTess`, no OCR pool, no tessdata lookup). The readers that sit on
//!   those functions hold: they return no outcome, no map and no phase.
//! * The pack check, on real pack folders (synthetic files).
//! * The real daemon binary: `ocr = "off"` is refused while a reader pack is
//!   missing, and granted with every pack installed.
//!
//! The gate test switches the process-wide mode, so it is the only test here
//! that calls OCR in this process. The daemon tests run the binary with an
//! empty HOME and XDG dirs in a temp directory and no display: it reads no
//! real config, holds no sync token, and captures nothing.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use image::{DynamicImage, GrayImage, Luma, Rgb, RgbImage};
use stat_tracker::config::OcrSetting;
use stat_tracker::detect::hero_portrait::ScoreboardPreflight;
use stat_tracker::detect::{GamePhase, MatchOutcome, match_end, match_start};
use stat_tracker::ocr::{self, gate};
use stat_tracker::ocr_switch::{self, PackProblem, PackReadiness};

// ------------------------------------------------------------- the gate

/// A dark navy 1080p frame with white bars where the accolade screen and
/// the rank screen print the result word. Enough for the vote screen's
/// colour gate and both result readers' "is there a title" gate, so each of
/// them goes on to its OCR call.
fn end_screen_like_frame() -> DynamicImage {
    let mut rgb = RgbImage::from_pixel(1920, 1080, Rgb([16, 25, 50]));
    for (x0, y0, x1, y1) in [(40, 45, 400, 95), (40, 165, 400, 235)] {
        for y in y0..y1 {
            for x in x0..x1 {
                rgb.put_pixel(x, y, Rgb([250, 250, 250]));
            }
        }
    }
    DynamicImage::ImageRgb8(rgb)
}

/// A stat cell with a stroke in it: enough ink to reach Tesseract.
fn cell_with_ink() -> DynamicImage {
    let mut rgb = RgbImage::from_pixel(64, 48, Rgb([20, 22, 30]));
    for y in 10..38 {
        for x in 28..36 {
            rgb.put_pixel(x, y, Rgb([250, 250, 250]));
        }
    }
    DynamicImage::ImageRgb8(rgb)
}

#[test]
fn with_ocr_off_every_entry_is_blocked_and_nothing_of_tesseract_starts() {
    assert_eq!(
        gate::mode(),
        gate::OcrMode::On,
        "a process starts with OCR on"
    );
    assert_eq!(gate::blocked_total(), 0);
    gate::set_mode(gate::OcrMode::Off);
    assert!(gate::is_off());

    let cell = cell_with_ink();
    let gray = GrayImage::from_pixel(64, 32, Luma([255]));
    let board = DynamicImage::ImageRgb8(RgbImage::from_pixel(1248, 756, Rgb([20, 22, 30])));
    let frame = end_screen_like_frame();
    let columns = ocr::preprocess::columns_with_offset(0.0);

    // Every public function that can reach Tesseract, once each. The ones
    // with a `Result` return the gate's own error; the others read nothing.
    let off = |r: Result<(), Box<dyn std::error::Error + Send + Sync>>, what: &str| {
        let err = r.expect_err(what);
        assert!(gate::is_off_error(err.as_ref()), "{what}: {err}");
    };
    off(ocr::recognize_region(&cell).map(drop), "recognize_region");
    off(
        ocr::recognize_sparse_region(&cell).map(drop),
        "recognize_sparse_region",
    );
    off(
        ocr::recognize_prepared(&gray, "7", None).map(drop),
        "recognize_prepared",
    );
    off(
        ocr::recognize_prepared_lang(&gray, "7", None, "eng").map(drop),
        "recognize_prepared_lang",
    );
    off(ocr::recognize_cell(&cell).map(drop), "recognize_cell");
    off(ocr::recognize_name(&cell).map(drop), "recognize_name");
    off(ocr::recognize(&frame).map(drop), "recognize");
    let row = ocr::recognize_row(&cell, &columns, 5);
    assert!(row.name.is_none() && row.name_alternates.is_empty());
    assert_eq!(row.stats.len(), 6);
    assert!(row.stats.iter().all(|c| c.value.is_empty() && !c.suspect));
    assert!(ocr::recognize_scoreboard_cells(&frame).is_empty());
    assert!(ocr::recognize_scoreboard_cells_with_team_size(&frame, Some(5)).is_empty());
    assert!(ocr::recognize_scoreboard_cells_pre_cropped(&board, 5).is_empty());
    assert_eq!(
        ocr::recognize_map_label(&frame, ScoreboardPreflight::Ready(5)),
        ""
    );

    let direct = gate::blocked_calls();
    let mut entries: Vec<&str> = direct.iter().map(|b| b.site.entry).collect();
    entries.sort_unstable();
    assert_eq!(
        entries,
        [
            "recognize",
            "recognize_cell",
            "recognize_map_label",
            "recognize_name",
            "recognize_prepared",
            "recognize_prepared_lang",
            "recognize_region",
            "recognize_row",
            "recognize_scoreboard_cells_pre_cropped",
            // `recognize_scoreboard_cells` goes through this one
            "recognize_scoreboard_cells_with_team_size",
            "recognize_scoreboard_cells_with_team_size",
            "recognize_sparse_region",
        ],
        "one line per call above"
    );
    for b in &direct {
        assert_eq!(b.count, 1, "{}", b.site);
        assert!(
            b.site.file.ends_with("tests/ocr_switch.rs"),
            "a call is counted where it was made, not inside ocr: {}",
            b.site
        );
    }
    assert_eq!(gate::blocked_total(), 12);

    // The readers on top hold: no outcome, no map, no phase, and no panic.
    assert_eq!(match_end::detect_outcome_signal(&frame), None);
    assert_eq!(match_end::detect_outcome(&frame), MatchOutcome::Unknown);
    assert_eq!(match_end::read_accolade_map(&frame), None);
    assert!(matches!(
        match_start::detect_phase(&frame),
        GamePhase::Unknown
    ));
    let held: Vec<_> = gate::blocked_calls()
        .into_iter()
        .filter(|b| !b.site.file.ends_with("tests/ocr_switch.rs"))
        .collect();
    let in_file = |name: &str| -> u64 {
        held.iter()
            .filter(|b| b.site.file.ends_with(name))
            .map(|b| b.count)
            .sum()
    };
    assert!(
        in_file("detect/match_end.rs") >= 3,
        "the result word, rank screen and accolade map reads reached the gate: {held:?}"
    );
    assert!(
        in_file("detect/match_start.rs") >= 1,
        "the vote screen read reached the gate: {held:?}"
    );
    assert!(
        held.iter().all(|b| b.site.entry != "ocr_with"),
        "every call was stopped at its entry point, none as deep as ocr_with: {held:?}"
    );

    // Nothing of Tesseract was started.
    assert_eq!(gate::leptess_created(), 0, "no LepTess was created");
    assert_eq!(gate::tesseract_calls(), 0, "no Tesseract read ran");
    assert!(!ocr::pool_started(), "the OCR pool was not built");
    assert!(
        !ocr::tessdata_lang_probed(),
        "the tessdata language was not looked up"
    );

    // Back on, the same cell is read, and that is what the counters count.
    gate::set_mode(gate::OcrMode::On);
    let blocked = gate::blocked_total();
    ocr::recognize_cell(&cell).expect("a cell read with OCR on");
    assert_eq!(
        gate::blocked_total(),
        blocked,
        "nothing is blocked with OCR on"
    );
    assert!(gate::tesseract_calls() >= 1);
    assert_eq!(gate::leptess_created(), 1, "one instance for this thread");
    ocr::recognize_region(&cell).expect("a region read with OCR on");
    assert!(ocr::tessdata_lang_probed());

    // The daemon's startup step sets the gate from the decision. Off is
    // granted with every pack and refused without, and the pack check runs
    // only when off is asked for.
    let dir = tempfile::tempdir().unwrap();
    let config = |ocr| stat_tracker::config::Config {
        data_dir: dir.path().to_path_buf(),
        ocr,
        ..stat_tracker::config::Config::default()
    };
    let ready = PackReadiness {
        heroes: true,
        maps: true,
        words: true,
        words_final_score: true,
    };
    let granted = ocr_switch::Switch::start_with(&config(OcrSetting::Off), |_| ready);
    assert!(granted.is_off());
    assert!(gate::is_off(), "a granted off closes the gate");
    let status = ocr_switch::read_status(dir.path()).expect("the status file");
    assert_eq!(
        (status.requested, status.effective, status.refusal.clone()),
        (OcrSetting::Off, OcrSetting::Off, None)
    );
    assert_eq!(status.pid, std::process::id());
    assert_eq!(status.packs, Some(ready));
    assert_eq!(status.blocked_total, blocked);
    assert_eq!(status.leptess_created, gate::leptess_created());
    assert!(status.leptess_created >= 1);
    let region_site = direct
        .iter()
        .find(|b| b.site.entry == "recognize_region")
        .map(|b| b.site.to_string())
        .unwrap();
    assert!(region_site.ends_with(" recognize_region"), "{region_site}");
    assert!(
        status
            .blocked
            .iter()
            .any(|s| s.site == region_site && s.count == 1),
        "the status file names the call sites: {:?}",
        status.blocked
    );
    assert!(ocr::recognize_cell(&cell).is_err());
    let blocked = blocked + 1;

    let refused = ocr_switch::Switch::start_with(&config(OcrSetting::Off), |_| PackReadiness {
        maps: false,
        ..ready
    });
    assert!(!refused.is_off());
    assert!(!gate::is_off(), "a refused off leaves OCR on");
    ocr::recognize_cell(&cell).expect("OCR still reads after a refusal");
    let status = ocr_switch::read_status(dir.path()).unwrap();
    assert_eq!(status.effective, OcrSetting::On);
    assert!(status.refusal.unwrap().contains("maps-v1"));

    gate::set_mode(gate::OcrMode::Off);
    let on = ocr_switch::Switch::start_with(&config(OcrSetting::On), |_| {
        panic!("OCR on never looks at the packs")
    });
    assert!(!on.is_off());
    assert!(!gate::is_off(), "on opens the gate again");
    assert_eq!(ocr_switch::read_status(dir.path()).unwrap().packs, None);

    // A pack that goes away while the daemon runs with OCR off: its reads
    // are held and the status file says which pack, and OCR stays off.
    // This is the real startup step, on the process-wide reader.
    let live = tempfile::tempdir().unwrap();
    hero_pack(live.path());
    map_pack(live.path());
    words_pack(live.path(), true);
    let mut switch = ocr_switch::Switch::start(&stat_tracker::config::Config {
        data_dir: live.path().to_path_buf(),
        ocr: OcrSetting::Off,
        ..stat_tracker::config::Config::default()
    });
    assert!(
        switch.is_off() && gate::is_off(),
        "every pack is there: {:?}",
        switch.decision
    );
    let status = ocr_switch::read_status(live.path()).unwrap();
    assert_eq!(status.packs, Some(ready));
    assert_eq!(status.notice(), None);
    assert_eq!(status.blocked_total, blocked);
    // A call blocked later reaches the file on the next check.
    assert!(ocr::recognize_name(&cell).is_err());
    switch.check();
    assert_eq!(
        ocr_switch::read_status(live.path()).unwrap().blocked_total,
        blocked + 1
    );

    let packs_now = |switch: &mut ocr_switch::Switch| {
        switch.check();
        ocr_switch::read_status(live.path()).and_then(|s| s.packs)
    };
    std::fs::remove_dir_all(live.path().join("templates").join("maps-v1")).unwrap();
    assert!(
        wait_for("the missing pack to show", Duration::from_secs(30), || {
            packs_now(&mut switch).is_some_and(|p| !p.maps)
        }),
        "the watch never saw maps-v1 go away"
    );
    let status = ocr_switch::read_status(live.path()).unwrap();
    assert_eq!(status.effective, OcrSetting::Off, "OCR stays off");
    let banner = status.notice().expect("a hold banner");
    assert!(
        banner.starts_with("Reading on hold: the maps-v1 reader pack is not installed"),
        "{banner}"
    );
    assert!(gate::is_off(), "a missing pack never turns OCR back on");
    let instances = gate::leptess_created();
    assert!(ocr::recognize_cell(&cell).is_err());
    assert_eq!(gate::leptess_created(), instances);

    map_pack(live.path());
    assert!(
        wait_for("the pack to come back", Duration::from_secs(30), || {
            packs_now(&mut switch).is_some_and(|p| p.ready())
        }),
        "the watch never saw maps-v1 come back"
    );
    assert_eq!(ocr_switch::read_status(live.path()).unwrap().notice(), None);
    gate::set_mode(gate::OcrMode::On);
}

// ------------------------------------------------------------ the packs

/// Blocks of two greys: not flat, and it survives every loader's downscale.
fn blocks(w: u32, h: u32, cell: u32, phase: u32) -> GrayImage {
    GrayImage::from_fn(w, h, |x, y| {
        if ((x + phase) / cell + y / cell).is_multiple_of(2) {
            Luma([235])
        } else {
            Luma([25])
        }
    })
}

fn save_png(path: &Path, img: &GrayImage) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    img.save(path).unwrap();
}

fn hero_pack(data_dir: &Path) {
    let dir = data_dir.join("templates").join("heroes");
    save_png(&dir.join("ana.png"), &blocks(64, 64, 8, 0));
    save_png(&dir.join("mercy.png"), &blocks(64, 64, 16, 4));
}

fn map_pack(data_dir: &Path) {
    let dir = data_dir.join("templates").join("maps-v1");
    let mut glyphs = Vec::new();
    for (i, c) in ('A'..='Z').enumerate() {
        let file = format!("g{:04x}.png", c as u32);
        save_png(
            &dir.join(&file),
            &blocks(18, 32, 4 + (i as u32 % 3), i as u32),
        );
        glyphs.push(format!("\"{c}\": \"{file}\""));
    }
    let atlas = format!(
        "{{\"height\": 32, \"gap\": 2.0, \"space\": 10.0, \"glyphs\": {{{}}}}}",
        glyphs.join(", ")
    );
    std::fs::write(dir.join("atlas.json"), atlas).unwrap();
}

/// words-v1 with both result words. `final_score` adds the FINAL SCORE
/// template; without it the pack is the older, out-of-date one.
fn words_pack(data_dir: &Path, final_score: bool) {
    let dir = data_dir.join("templates").join("words-v1");
    save_png(&dir.join("accolade_victory.png"), &blocks(224, 48, 12, 0));
    save_png(&dir.join("accolade_defeat.png"), &blocks(192, 48, 16, 6));
    let p4 = dir.join("p4_final_score.png");
    if final_score {
        save_png(&p4, &blocks(256, 48, 12, 3));
    } else {
        let _ = std::fs::remove_file(p4);
    }
}

#[test]
fn the_pack_check_follows_the_pack_folders() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path();
    let none = PackReadiness::on_disk(data);
    assert_eq!(none, PackReadiness::default());
    assert_eq!(none.counts(), (0, 3));
    assert_eq!(
        ocr_switch::decide(OcrSetting::Off, &none).effective,
        OcrSetting::On,
        "no packs: off is refused"
    );

    hero_pack(data);
    assert_eq!(
        PackReadiness::on_disk(data).problems(),
        [PackProblem::MapsMissing, PackProblem::WordsMissing]
    );
    map_pack(data);
    assert_eq!(
        PackReadiness::on_disk(data).problems(),
        [PackProblem::WordsMissing]
    );
    words_pack(data, false);
    let stale = PackReadiness::on_disk(data);
    assert_eq!(stale.problems(), [PackProblem::WordsOutOfDate]);
    assert_eq!(stale.counts(), (2, 3));
    assert!(stale.refusal().unwrap().contains("out of date"));

    words_pack(data, true);
    let ready = PackReadiness::on_disk(data);
    assert!(ready.ready(), "{ready:?}");
    assert_eq!(ready.status_line(), "Reader packs ready: 3 of 3");
    let granted = ocr_switch::decide(OcrSetting::Off, &ready);
    assert_eq!(granted.effective, OcrSetting::Off);
    assert_eq!(granted.refusal, None);

    // The hand-installed folders are not the packs the switch asks for.
    let legacy = tempfile::tempdir().unwrap();
    hero_pack(legacy.path());
    map_pack(legacy.path());
    words_pack(legacy.path(), true);
    let t = legacy.path().join("templates");
    std::fs::rename(t.join("maps-v1"), t.join("maps")).unwrap();
    std::fs::rename(t.join("words-v1"), t.join("result")).unwrap();
    assert_eq!(
        PackReadiness::on_disk(legacy.path()).problems(),
        [PackProblem::MapsMissing, PackProblem::WordsMissing]
    );
}

// ----------------------------------------------------------- the daemon

fn wait_for(what: &str, timeout: Duration, mut ok: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    while !ok() {
        if start.elapsed() > timeout {
            eprintln!("timed out waiting for {what}");
            return false;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    true
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

/// Kills the daemon when the test leaves, also on a panic. A daemon that
/// outlives its test keeps every file descriptor it inherited.
struct KillOnDrop(std::process::Child);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct DaemonRun {
    /// Everything the daemon printed.
    out: String,
    /// `ocr_switch.json` as it was while the daemon ran.
    status: ocr_switch::Status,
    pid: u32,
}

/// Start the daemon on `data_dir` with `ocr`, wait until it is in its run
/// loop, read the status file, then stop it with SIGTERM.
fn run_daemon(root: &Path, data_dir: &Path, ocr: &str) -> DaemonRun {
    let home = root.join("home");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(data_dir).unwrap();
    let log_path = root.join("daemon.out");
    let log = std::fs::File::create(&log_path).unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_scuffed-stat-tracker"))
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("HOME", &home)
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("XDG_DATA_HOME", root.join("data"))
        .env("XDG_CACHE_HOME", root.join("cache"))
        .env("NO_COLOR", "1")
        .env(stat_tracker::config::OCR_ENV, ocr)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log.try_clone().unwrap()))
        .stderr(Stdio::from(log))
        .spawn()
        .expect("spawn daemon");
    let mut child = KillOnDrop(child);
    let pid = child.0.id();
    // The SIGTERM handler is installed in the run loop; before that the
    // signal's default action would kill the daemon.
    let ready = wait_for("signal handlers ready", Duration::from_secs(90), || {
        read(&log_path).contains("signal handlers ready")
    });
    let status = ocr_switch::read_status(data_dir);
    if ready {
        // SAFETY: plain kill(2) on the child this test spawned.
        unsafe { libc::kill(pid as i32, libc::SIGTERM) };
        wait_for("the daemon to exit", Duration::from_secs(60), || {
            matches!(child.0.try_wait(), Ok(Some(_)))
        });
    }
    let _ = child.0.kill();
    let exit = child.0.wait().expect("wait for the daemon");
    let out = read(&log_path);
    assert!(
        ready,
        "the daemon never reached its run loop; output:\n{out}"
    );
    assert!(exit.success(), "exit {exit:?}; output:\n{out}");
    DaemonRun {
        status: status
            .unwrap_or_else(|| panic!("no {} written; output:\n{out}", ocr_switch::STATUS_FILE)),
        out,
        pid,
    }
}

fn daemon_dirs() -> (tempfile::TempDir, PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let data_dir = root.path().join("data").join("scuffed-stat-tracker");
    (root, data_dir)
}

/// The line that says the OCR pool size. Printed only with OCR on.
const OCR_WORKERS_LINE: &str = "OCR workers (each holds";

#[test]
fn the_daemon_refuses_ocr_off_while_a_reader_pack_is_missing() {
    let (root, data_dir) = daemon_dirs();
    // heroes only: maps-v1 and words-v1 are missing
    hero_pack(&data_dir);
    let run = run_daemon(root.path(), &data_dir, "off");

    assert_eq!(run.status.pid, run.pid);
    assert_eq!(run.status.requested, OcrSetting::Off);
    assert_eq!(run.status.effective, OcrSetting::On, "OCR stays on");
    let reason = run
        .status
        .refusal
        .clone()
        .expect("the refusal has a reason");
    assert!(
        reason.contains("maps-v1") && reason.contains("words-v1"),
        "{reason}"
    );
    assert!(!reason.contains("hero"), "the hero pack is there: {reason}");
    assert_eq!(run.status.notice(), Some(reason.clone()));
    assert_eq!(run.status.blocked_total, 0);

    let refused: Vec<&str> = run
        .out
        .lines()
        .filter(|l| l.contains("WARN") && l.contains("was refused"))
        .collect();
    assert_eq!(refused.len(), 1, "one WARN line; output:\n{}", run.out);
    assert!(
        refused[0].contains("maps-v1"),
        "the log says why: {}",
        refused[0]
    );
    assert!(
        run.out.contains(OCR_WORKERS_LINE),
        "the daemon runs classic OCR; output:\n{}",
        run.out
    );
    assert!(!run.out.contains("classic OCR is off"), "{}", run.out);
}

#[test]
fn the_daemon_turns_ocr_off_with_every_reader_pack() {
    let (root, data_dir) = daemon_dirs();
    hero_pack(&data_dir);
    map_pack(&data_dir);
    words_pack(&data_dir, true);
    let run = run_daemon(root.path(), &data_dir, "off");

    assert_eq!(run.status.requested, OcrSetting::Off);
    assert_eq!(run.status.effective, OcrSetting::Off);
    assert_eq!(run.status.refusal, None);
    assert_eq!(run.status.packs, Some(PackReadiness::on_disk(&data_dir)));
    assert!(run.status.packs.unwrap().ready());
    assert_eq!(run.status.notice(), None);
    assert_eq!(run.status.leptess_created, 0);
    assert_eq!(run.status.blocked_total, 0);

    assert!(run.out.contains("classic OCR is off"), "{}", run.out);
    // off implies the new reader, though `reader` was never set
    assert!(
        run.out.contains("new scoreboard reader on for saved stats"),
        "{}",
        run.out
    );
    assert!(
        !run.out.contains(OCR_WORKERS_LINE) && !run.out.contains("OCR worker pool ready"),
        "no OCR pool with OCR off; output:\n{}",
        run.out
    );
    assert!(!run.out.contains("was refused"), "{}", run.out);
}

#[test]
fn the_daemon_runs_classic_ocr_when_nothing_asks_for_off() {
    let (root, data_dir) = daemon_dirs();
    let run = run_daemon(root.path(), &data_dir, "on");
    assert_eq!(run.status.requested, OcrSetting::On);
    assert_eq!(run.status.effective, OcrSetting::On);
    assert_eq!(run.status.refusal, None);
    assert_eq!(run.status.packs, None, "the packs are not looked at");
    assert_eq!(run.status.notice(), None);
    assert!(run.out.contains(OCR_WORKERS_LINE), "{}", run.out);
    assert!(
        !run.out.contains("new scoreboard reader on for saved stats"),
        "{}",
        run.out
    );
}
