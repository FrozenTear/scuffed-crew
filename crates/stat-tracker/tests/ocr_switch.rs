//! The OCR on/off switch (`ocr` in config.toml), in a process of its own.
//!
//! * The gate: with OCR off every public `ocr` function is blocked, counted
//!   under the line that called it, and nothing of Tesseract is started (no
//!   `LepTess`, no OCR pool, no tessdata lookup). The readers that sit on
//!   those functions hold: they return no outcome, no map and no phase.
//! * The pack check, on real pack folders (synthetic files).
//! * The startup step: off is granted only when the build reads everything
//!   without Tesseract and every pack is loaded, and a pack that goes away
//!   afterwards holds its reads without turning OCR back on.
//! * The real daemon binary: what it decides and what it writes.
//!
//! The gate test switches the process-wide mode and installs the
//! process-wide reader, so it is the only test here that does either. The
//! daemon tests run the binary with an empty HOME and XDG dirs in a temp
//! directory and no display: it reads no real config, holds no sync token,
//! and captures nothing.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use image::{DynamicImage, GrayImage, Luma, Rgb, RgbImage};
use stat_tracker::config::{Config, OcrSetting};
use stat_tracker::detect::hero_portrait::ScoreboardPreflight;
use stat_tracker::detect::{GamePhase, MatchOutcome, match_end, match_start};
use stat_tracker::ocr::{self, gate};
use stat_tracker::ocr_switch::{self, Audience, PackProblem, PackReadiness, Read, Switch};
use stat_tracker::shadow::{Reader, ReaderConfig};

const READY: PackReadiness = PackReadiness {
    heroes: true,
    maps: true,
    words: true,
    result_words: true,
    phase_titles: true,
    final_score: true,
};

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

fn load(data_dir: &Path) -> Reader {
    Reader::load(&ReaderConfig::from_data_dir(data_dir))
}

fn config(data_dir: &Path, ocr: OcrSetting) -> Config {
    Config {
        data_dir: data_dir.to_path_buf(),
        ocr,
        ..Config::default()
    }
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
            "recognize_scoreboard_cells",
            "recognize_scoreboard_cells_pre_cropped",
            "recognize_scoreboard_cells_with_team_size",
            "recognize_sparse_region",
        ],
        "one line per call above, each under its own name"
    );
    for b in &direct {
        assert_eq!(b.count, 1, "{}", b.site);
        assert!(
            b.site.file.ends_with("tests/ocr_switch.rs"),
            "a call is counted where it was made, not inside ocr: {}",
            b.site
        );
        assert_eq!(
            b.site.key(),
            format!("other.rs:1 {}", b.site.entry),
            "this file is not below the tracker's source folder, so the key \
             that leaves the process has no path"
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
        held.iter()
            .all(|b| !matches!(b.site.entry, "ocr_with" | "ocr_pool" | "tessdata_lang")),
        "every call was stopped at its entry point, none as deep as a resource: {held:?}"
    );

    // The poll tick's phase read does not ask for OCR at all while it is
    // off: a screen no title mask confirms is left unconfirmed, and the OCR
    // check behind it is not attempted, so nothing is counted for it. Asked
    // with the OCR source, so that no mask can answer first. The frame
    // holds still, so the second tick is past the stability check.
    let before_poll = gate::blocked_total();
    let rgb = frame.to_rgb8();
    let mut stability = stat_tracker::detect::stability::FrameStability::default();
    for _ in 0..3 {
        assert!(matches!(
            match_start::detect_phase_polled(
                &frame,
                &rgb,
                &mut stability,
                match_start::PhaseSource::Ocr
            ),
            GamePhase::Unknown
        ));
    }
    assert_eq!(
        gate::blocked_total(),
        before_poll,
        "the poll tick's phase OCR check is not attempted with OCR off"
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

    // ---- the daemon's startup step, with the build and the reader given.
    let empty = tempfile::tempdir().unwrap();
    let live = tempfile::tempdir().unwrap();
    hero_pack(live.path());
    map_pack(live.path());
    words_pack(live.path(), Words::Current);

    // Refused for the packs: OCR stays on, and the reason names them.
    let refused = Switch::start_with(&config(empty.path(), OcrSetting::Off), &[], load);
    assert!(!refused.is_off());
    assert!(!gate::is_off(), "a refused off leaves OCR on");
    ocr::recognize_cell(&cell).expect("OCR still reads after a refusal");
    let status = ocr_switch::read_status(empty.path()).expect("the status file");
    assert_eq!(status.pid, std::process::id());
    assert_eq!(
        (status.requested, status.effective),
        (OcrSetting::Off, OcrSetting::On)
    );
    assert_eq!(
        status.refusal.as_ref().map(|r| r.packs.clone()),
        Some(vec![
            PackProblem::HeroesMissing,
            PackProblem::MapsMissing,
            PackProblem::WordsMissing
        ])
    );
    assert_eq!(status.packs, Some(PackReadiness::default()));

    // Refused for the build: every pack is there, and one read that this
    // build would still need Tesseract for is enough.
    gate::set_mode(gate::OcrMode::Off);
    let refused = Switch::start_with(
        &config(live.path(), OcrSetting::Off),
        &[Read::StatCells],
        load,
    );
    assert!(!refused.is_off());
    assert!(!gate::is_off(), "a refusal opens the gate");
    let status = ocr_switch::read_status(live.path()).unwrap();
    assert_eq!(status.packs, Some(READY));
    let refusal = status.refusal.clone().expect("the reason");
    assert_eq!(
        (refusal.reads.clone(), refusal.packs.clone()),
        (vec![Read::StatCells], vec![])
    );
    assert_eq!(status.notice(), Some(refusal.text(Audience::Daemon)));

    // On: granted without a look at the packs, and the gate is open.
    gate::set_mode(gate::OcrMode::Off);
    let on = Switch::start_with(&config(live.path(), OcrSetting::On), &Read::ALL, |_| {
        panic!("OCR on never loads a reader")
    });
    assert!(!on.is_off());
    assert!(!gate::is_off(), "on opens the gate");
    assert_eq!(ocr_switch::read_status(live.path()).unwrap().packs, None);

    // Granted: every pack, and a build that reads everything without
    // Tesseract. The gate shuts, and the reader that was checked is the one
    // that reads from here on.
    let mut switch = Switch::start_with(&config(live.path(), OcrSetting::Off), &[], load);
    assert!(
        switch.is_off(),
        "every pack is there: {:?}",
        switch.decision
    );
    assert!(gate::is_off(), "a granted off closes the gate");
    assert_eq!(
        stat_tracker::shadow::pack_readiness(),
        READY,
        "the process-wide reader is the one the switch loaded"
    );
    let status = ocr_switch::read_status(live.path()).unwrap();
    assert_eq!(
        (status.requested, status.effective, status.refusal.clone()),
        (OcrSetting::Off, OcrSetting::Off, None)
    );
    assert_eq!(status.packs, Some(READY));
    assert_eq!(status.blocked_total, blocked);
    // The counters are the process's: the calls stopped further up are in
    // the file, and the notice counts them.
    assert!(blocked >= 12, "{blocked}");
    assert!(
        status
            .notice()
            .is_some_and(|n| n.contains(&format!("{blocked} reads still asked for it"))),
        "{:?}",
        status.notice()
    );
    assert_eq!(status.leptess_created, gate::leptess_created());
    assert!(status.leptess_created >= 1);
    let region_key = direct
        .iter()
        .find(|b| b.site.entry == "recognize_region")
        .map(|b| b.site.key())
        .unwrap();
    assert_eq!(region_key, "other.rs:1 recognize_region");
    assert!(
        status
            .blocked
            .iter()
            .any(|s| s.site == region_key && s.count >= 1),
        "the status file names the call sites: {:?}",
        status.blocked
    );
    // One line per key, every key one the site's report validator takes,
    // and the lines add up to the total.
    let keys: std::collections::BTreeSet<&str> =
        status.blocked.iter().map(|s| s.site.as_str()).collect();
    assert_eq!(keys.len(), status.blocked.len(), "{:?}", status.blocked);
    for s in &status.blocked {
        assert!(
            scuffed_types::stat_report::is_ocr_call_site(&s.site),
            "{:?}",
            s.site
        );
    }
    assert_eq!(
        status.blocked.iter().map(|s| s.count).sum::<u64>(),
        status.blocked_total
    );
    assert!(
        status
            .blocked
            .iter()
            .any(|s| s.site.starts_with("detect/match_end.rs:")),
        "a call made inside the tracker's source keeps its file: {:?}",
        status.blocked
    );
    // A call blocked later reaches the file on the next check, and the
    // notice then says a read was stopped.
    assert!(ocr::recognize_name(&cell).is_err());
    switch.check();
    let status = ocr_switch::read_status(live.path()).unwrap();
    assert_eq!(status.blocked_total, blocked + 1);
    assert!(
        status
            .notice()
            .is_some_and(|n| n.contains("still asked for it")),
        "{:?}",
        status.notice()
    );

    // A pack that goes away while the daemon runs with OCR off: its reads
    // are held and the status file says which pack, and OCR stays off.
    let packs_now = |switch: &mut Switch| {
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
        banner.starts_with("Reading on hold: the map pack is not installed"),
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

    // The real startup step, on this build.
    gate::set_mode(gate::OcrMode::On);
    let real = Switch::start(&config(live.path(), OcrSetting::Off));
    let needing = Read::needing_ocr();
    assert_eq!(
        real.is_off(),
        needing.is_empty(),
        "off is granted exactly when this build needs Tesseract for nothing"
    );
    assert_eq!(gate::is_off(), needing.is_empty());
    assert_eq!(
        real.decision.refusal.as_ref().map(|r| r.reads.clone()),
        (!needing.is_empty()).then_some(needing)
    );
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

fn hero_pack_in(data_dir: &Path, folder: &str) {
    let dir = data_dir.join("templates").join(folder);
    save_png(&dir.join("ana.png"), &blocks(64, 64, 8, 0));
    save_png(&dir.join("mercy.png"), &blocks(64, 64, 16, 4));
}

fn hero_pack(data_dir: &Path) {
    hero_pack_in(data_dir, "heroes");
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

/// Which words-v1 to write.
#[derive(Clone, Copy, PartialEq)]
enum Words {
    /// Everything the readers without an OCR fallback need.
    Current,
    /// The accolade words only: no rank screen words.
    NoRankScreen,
    /// One match-start title is missing.
    NoBanTitle,
    /// No FINAL SCORE line.
    NoFinalScore,
}

const PHRASES: [&str; 3] = ["vote_for_a_map", "ban_heroes", "select_a_preferred_hero"];

fn words_pack(data_dir: &Path, which: Words) {
    let dir = data_dir.join("templates").join("words-v1");
    let _ = std::fs::remove_dir_all(&dir);
    save_png(&dir.join("accolade_victory.png"), &blocks(224, 48, 12, 0));
    save_png(&dir.join("accolade_defeat.png"), &blocks(192, 48, 16, 6));
    if which != Words::NoRankScreen {
        save_png(
            &dir.join("rank_screen_victory.png"),
            &blocks(224, 48, 10, 2),
        );
        save_png(&dir.join("rank_screen_defeat.png"), &blocks(192, 48, 14, 5));
    }
    if which != Words::NoFinalScore {
        save_png(&dir.join("p4_final_score.png"), &blocks(256, 48, 12, 3));
    }
    let mut entries = Vec::new();
    for (i, id) in PHRASES.iter().enumerate() {
        entries.push(format!(
            "\"{id}\": {{\"roi\": [0.36, 0.11, 0.64, 0.235], \"plane\": \"min\", \"work\": 0.5}}"
        ));
        if which == Words::NoBanTitle && *id == "ban_heroes" {
            continue;
        }
        save_png(
            &dir.join(format!("phrase_{id}.png")),
            &blocks(200, 40, 8 + 2 * i as u32, i as u32),
        );
    }
    std::fs::write(
        dir.join("phrases.json"),
        format!("{{{}}}", entries.join(", ")),
    )
    .unwrap();
}

#[test]
fn the_pack_check_follows_what_each_reader_needs() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path();
    let none = PackReadiness::on_disk(data);
    assert_eq!(none, PackReadiness::default());
    assert_eq!(none.counts(), (0, 3));
    assert_eq!(
        ocr_switch::decide(OcrSetting::Off, &none, &[]).effective,
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

    // A word pack is ready only with everything its readers need.
    for (which, problem) in [
        (Words::NoRankScreen, PackProblem::WordsLackResultWords),
        (Words::NoBanTitle, PackProblem::WordsLackPhaseTitles),
        (Words::NoFinalScore, PackProblem::WordsLackFinalScore),
    ] {
        words_pack(data, which);
        let old = PackReadiness::on_disk(data);
        assert_eq!(old.problems(), [problem]);
        assert!(old.words, "the pack itself loads");
        assert_eq!(old.counts(), (2, 3));
        let refusal = ocr_switch::decide(OcrSetting::Off, &old, &[])
            .refusal
            .expect("an out-of-date pack refuses off");
        assert!(
            refusal.text(Audience::Click).contains("out of date"),
            "{}",
            refusal.text(Audience::Click)
        );
    }

    words_pack(data, Words::Current);
    let ready = PackReadiness::on_disk(data);
    assert_eq!(ready, READY);
    assert_eq!(ready.status_line(), "Reader packs ready: 3 of 3");
    let granted = ocr_switch::decide(OcrSetting::Off, &ready, &[]);
    assert_eq!(granted.effective, OcrSetting::Off);
    assert_eq!(granted.refusal, None);

    // The hand-installed folders are not the packs the switch asks for.
    let legacy = tempfile::tempdir().unwrap();
    hero_pack(legacy.path());
    map_pack(legacy.path());
    words_pack(legacy.path(), Words::Current);
    let t = legacy.path().join("templates");
    std::fs::rename(t.join("maps-v1"), t.join("maps")).unwrap();
    std::fs::rename(t.join("words-v1"), t.join("result")).unwrap();
    assert_eq!(
        PackReadiness::on_disk(legacy.path()).problems(),
        [PackProblem::MapsMissing, PackProblem::WordsMissing]
    );

    // heroes-v2-1080 alone reads no 1440p board: it is not "a hero pack".
    let small = tempfile::tempdir().unwrap();
    hero_pack_in(small.path(), "heroes-v2-1080");
    map_pack(small.path());
    words_pack(small.path(), Words::Current);
    assert_eq!(
        PackReadiness::on_disk(small.path()).problems(),
        [PackProblem::HeroesMissing]
    );
    hero_pack_in(small.path(), "heroes-v2");
    assert!(PackReadiness::on_disk(small.path()).ready());
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
    /// The same file after the daemon stopped.
    last_status: ocr_switch::Status,
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
    let missing = || -> ocr_switch::Status {
        panic!("no {} written; output:\n{out}", ocr_switch::STATUS_FILE)
    };
    DaemonRun {
        status: status.unwrap_or_else(missing),
        last_status: ocr_switch::read_status(data_dir).unwrap_or_else(missing),
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
const NEW_READER_LINE: &str = "new scoreboard reader on for saved stats";

#[test]
fn the_daemon_refuses_ocr_off_while_a_reader_pack_is_missing() {
    let (root, data_dir) = daemon_dirs();
    // heroes only: maps-v1 and words-v1 are missing
    hero_pack(&data_dir);
    let run = run_daemon(root.path(), &data_dir, "off");

    assert_eq!(run.status.pid, run.pid);
    assert_eq!(run.status.requested, OcrSetting::Off);
    assert_eq!(run.status.effective, OcrSetting::On, "OCR stays on");
    let refusal = run
        .status
        .refusal
        .clone()
        .expect("the refusal has a reason");
    assert_eq!(
        refusal.packs,
        [PackProblem::MapsMissing, PackProblem::WordsMissing],
        "the hero pack is there"
    );
    assert_eq!(refusal.reads, Read::needing_ocr());
    let reason = refusal.text(Audience::Daemon);
    // After the reads this build still needs OCR for, the packs are a
    // sentence of their own ("The map pack is not installed and ...").
    let lower = reason.to_lowercase();
    assert!(
        lower.contains("the map pack is not installed")
            && lower.contains("the word pack is not installed")
            && !lower.contains("hero pack"),
        "{reason}"
    );
    assert_eq!(run.status.notice(), Some(reason));
    assert_eq!(run.status.blocked_total, 0);

    let refused: Vec<&str> = run
        .out
        .lines()
        .filter(|l| l.contains("WARN") && l.contains("was refused"))
        .collect();
    assert_eq!(refused.len(), 1, "one WARN line; output:\n{}", run.out);
    assert!(
        refused[0].contains("maps-v1")
            && refused[0]
                .to_lowercase()
                .contains("the map pack is not installed"),
        "the log says why, with the pack id: {}",
        refused[0]
    );
    assert!(
        run.out.contains(OCR_WORKERS_LINE),
        "the daemon runs classic OCR; output:\n{}",
        run.out
    );
    assert!(!run.out.contains("classic OCR is off"), "{}", run.out);
    // a refused off runs with the file's reader, which is ocr-v1 here
    assert!(!run.out.contains(NEW_READER_LINE), "{}", run.out);
}

/// Every pack is installed. What the daemon decides then follows from the
/// build: granted once it reads everything without Tesseract, refused (and
/// saying which reads) until then. Either way no state is silent.
#[test]
fn the_daemon_with_every_pack_turns_ocr_off_only_when_the_build_can() {
    let (root, data_dir) = daemon_dirs();
    hero_pack(&data_dir);
    map_pack(&data_dir);
    words_pack(&data_dir, Words::Current);
    let run = run_daemon(root.path(), &data_dir, "off");

    assert_eq!(run.status.requested, OcrSetting::Off);
    assert_eq!(run.status.packs, Some(READY));
    assert_eq!(run.status.leptess_created, 0);
    assert_eq!(run.status.blocked_total, 0);
    assert_eq!(run.last_status.pid, run.pid, "the file is this run's");

    let needing = Read::needing_ocr();
    if needing.is_empty() {
        assert_eq!(run.status.effective, OcrSetting::Off);
        assert_eq!(run.status.refusal, None);
        assert_eq!(run.status.notice(), None);
        assert!(run.out.contains("classic OCR is off"), "{}", run.out);
        // off implies the new reader, though `reader` was never set
        assert!(run.out.contains(NEW_READER_LINE), "{}", run.out);
        assert!(
            !run.out.contains(OCR_WORKERS_LINE) && !run.out.contains("OCR worker pool ready"),
            "no OCR pool with OCR off; output:\n{}",
            run.out
        );
        assert!(!run.out.contains("was refused"), "{}", run.out);
    } else {
        assert_eq!(
            run.status.effective,
            OcrSetting::On,
            "this build still needs Tesseract for {needing:?}"
        );
        let refusal = run.status.refusal.clone().expect("the reason");
        assert_eq!(refusal.reads, needing);
        assert!(refusal.packs.is_empty(), "every pack is there");
        let notice = run.status.notice().expect("the member is told");
        assert!(
            notice.starts_with("Classic OCR stays on: this tracker version cannot read "),
            "{notice}"
        );
        assert!(
            run.out
                .lines()
                .any(|l| l.contains("WARN") && l.contains("was refused")),
            "{}",
            run.out
        );
        assert!(run.out.contains(OCR_WORKERS_LINE), "{}", run.out);
        assert!(!run.out.contains("classic OCR is off"), "{}", run.out);
        assert!(!run.out.contains(NEW_READER_LINE), "{}", run.out);
    }
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
    assert!(!run.out.contains(NEW_READER_LINE), "{}", run.out);
    // the temp file of the status write does not stay behind
    assert!(!data_dir.join(ocr_switch::STATUS_TMP_FILE).exists());
}
