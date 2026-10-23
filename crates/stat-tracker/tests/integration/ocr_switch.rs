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
//! process-wide reader, so it is the only test here that does either, and
//! it runs in a process of its own (`common::own_process`). The daemon
//! tests run the binary with an empty HOME and XDG dirs in a temp directory
//! and no display: it reads no real config, holds no sync token, and
//! captures nothing.

use crate::common;

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use image::{DynamicImage, GrayImage, Luma, Rgb, RgbImage};
use stat_tracker::config::{Config, OcrSetting, ReaderSetting};
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
    end_titles: true,
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

/// [`config`] with an explicit `reader = "ocr-v1"`. The gate test below
/// starts the switch several times in one process and checks the classic
/// OCR path: under ocr-v1 a refused off loads no process-wide reader, so the
/// reader the later granted start loads is the one that reads. The new
/// reader's version of that test is `tests/integration/ocr_switch_new_reader.rs`.
fn config_ocr_v1(data_dir: &Path, ocr: OcrSetting) -> Config {
    Config {
        reader: ReaderSetting::OcrV1,
        ..config(data_dir, ocr)
    }
}

#[test]
fn with_ocr_off_every_entry_is_blocked_and_nothing_of_tesseract_starts() {
    if !common::own_process(
        module_path!(),
        "with_ocr_off_every_entry_is_blocked_and_nothing_of_tesseract_starts",
    ) {
        return;
    }
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
    for names in [ocr::Names::Read, ocr::Names::Skip] {
        // one call site, counted twice
        assert!(ocr::recognize_scoreboard_cells_pre_cropped_with(&board, 5, names).is_empty());
    }
    assert!(ocr::recognize_scoreboard_names_pre_cropped(&board, 5).is_empty());
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
            "recognize_scoreboard_cells_pre_cropped_with",
            "recognize_scoreboard_cells_with_team_size",
            "recognize_scoreboard_names_pre_cropped",
            "recognize_sparse_region",
        ],
        "one line per call above, each under its own name"
    );
    for b in &direct {
        let calls = if b.site.entry == "recognize_scoreboard_cells_pre_cropped_with" {
            2
        } else {
            1
        };
        assert_eq!(b.count, calls, "{}", b.site);
        assert!(
            b.site.file.ends_with("tests/integration/ocr_switch.rs"),
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
    assert_eq!(gate::blocked_total(), 15);

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
        .filter(|b| !b.site.file.ends_with("tests/integration/ocr_switch.rs"))
        .collect();
    let in_file = |name: &str| -> u64 {
        held.iter()
            .filter(|b| b.site.file.ends_with(name))
            .map(|b| b.count)
            .sum()
    };
    // The result word (accolade and rank screen) and the accolade map each
    // reach the gate while this build needs OCR for them. Once a read is
    // marked as done without OCR its call must be gone: that direction is
    // tests/integration/ocr_flags.rs, per read.
    let still_ocr = [(Read::ResultWord, 2), (Read::AccoladeMap, 1)]
        .into_iter()
        .filter(|(read, _)| !read.without_ocr())
        .map(|(_, calls)| calls)
        .sum::<u64>();
    assert!(
        in_file("detect/match_end.rs") >= still_ocr,
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

    // A scoped mode can only tighten: in a process that runs with OCR off,
    // a scoped "on" opens nothing on its thread.
    {
        let on = gate::scoped(gate::OcrMode::On);
        assert!(gate::is_off(), "the process-wide off wins over a scope");
        assert!(ocr::recognize_cell(&cell).is_err());
        assert_eq!(gate::leptess_created(), 0);
        assert_eq!(gate::tesseract_calls(), 0);
        drop(on);
    }

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
    let refused = Switch::start_with(&config_ocr_v1(empty.path(), OcrSetting::Off), &[], load);
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

    // Refused for the packs keeps a watch: the packs may still arrive.
    assert!(refused.watches());
    assert_eq!(
        status.health_notice(),
        None,
        "a refused off is not a health problem"
    );

    // Refused for the build: one read that this build would still need
    // Tesseract for is enough. No pack can change that answer, so the packs
    // are not loaded, not named and not watched.
    gate::set_mode(gate::OcrMode::Off);
    let refused = Switch::start_with(
        &config_ocr_v1(live.path(), OcrSetting::Off),
        &[Read::StatCells],
        |_| panic!("a build that refuses off never loads the packs"),
    );
    assert!(!refused.is_off());
    assert!(!refused.watches(), "nothing to watch");
    assert!(!gate::is_off(), "a refusal opens the gate");
    let status = ocr_switch::read_status(live.path()).unwrap();
    assert_eq!(status.packs, None, "the packs were not looked at");
    let refusal = status.refusal.clone().expect("the reason");
    assert_eq!(
        (refusal.reads.clone(), refusal.packs.clone()),
        (vec![Read::StatCells], vec![])
    );
    assert_eq!(status.notice(), Some(refusal.text(Audience::Daemon)));
    assert!(
        refusal
            .text(Audience::Daemon)
            .ends_with(ocr_switch::TURN_BACK_ON),
        "the member is told how to clear it: {}",
        refusal.text(Audience::Daemon)
    );
    assert_eq!(status.health_notice(), None);

    // On: granted without a look at the packs, and the gate is open.
    gate::set_mode(gate::OcrMode::Off);
    let on = Switch::start_with(
        &config_ocr_v1(live.path(), OcrSetting::On),
        &Read::ALL,
        |_| panic!("OCR on never loads a reader"),
    );
    assert!(!on.is_off());
    assert!(!gate::is_off(), "on opens the gate");
    assert_eq!(ocr_switch::read_status(live.path()).unwrap().packs, None);

    // Granted: every pack, and a build that reads everything without
    // Tesseract. The gate shuts, and the reader that was checked is the one
    // that reads from here on.
    let mut switch = Switch::start_with(&config_ocr_v1(live.path(), OcrSetting::Off), &[], load);
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
            .any(|s| s.site.starts_with("detect/match_start.rs:")),
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
    let real = Switch::start(&config_ocr_v1(live.path(), OcrSetting::Off));
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
    /// A version 1 pack: no `selecting_map` (added in version 2, #269).
    Version1,
    /// No end-title templates.
    NoEndTitles,
}

const PHRASES: [&str; 4] = [
    "vote_for_a_map",
    "selecting_map",
    "ban_heroes",
    "select_a_preferred_hero",
];

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
    if which != Words::NoEndTitles {
        save_png(&dir.join("end_title_victory.png"), &blocks(224, 48, 9, 1));
        save_png(&dir.join("end_title_defeat.png"), &blocks(192, 48, 13, 7));
    }
    // The FINAL SCORE line: in the published pack, and not asked for by
    // the switch (with OCR off nothing reads it).
    save_png(&dir.join("p4_final_score.png"), &blocks(256, 48, 12, 3));
    let mut entries = Vec::new();
    for (i, id) in PHRASES.iter().enumerate() {
        if which == Words::Version1 && *id == "selecting_map" {
            continue;
        }
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
    let none = ocr_switch::pack_readiness_on_disk(data);
    assert_eq!(none, PackReadiness::default());
    assert_eq!(none.counts(), (0, 3));
    assert_eq!(
        ocr_switch::decide(OcrSetting::Off, &none, &[]).effective,
        OcrSetting::On,
        "no packs: off is refused"
    );

    hero_pack(data);
    assert_eq!(
        ocr_switch::pack_readiness_on_disk(data).problems(),
        [PackProblem::MapsMissing, PackProblem::WordsMissing]
    );
    map_pack(data);
    assert_eq!(
        ocr_switch::pack_readiness_on_disk(data).problems(),
        [PackProblem::WordsMissing]
    );

    // A word pack is ready only with everything its readers need.
    for (which, problem) in [
        (Words::NoRankScreen, PackProblem::WordsLackResultWords),
        (Words::NoBanTitle, PackProblem::WordsLackPhaseTitles),
        // On purpose: the version 1 word pack has no SELECTING MAP title.
        // With it the map vote would go unconfirmed once the vote closed,
        // so off is refused until the version 2 pack is there.
        (Words::Version1, PackProblem::WordsLackPhaseTitles),
        (Words::NoEndTitles, PackProblem::WordsLackEndTitles),
    ] {
        words_pack(data, which);
        let old = ocr_switch::pack_readiness_on_disk(data);
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
    let ready = ocr_switch::pack_readiness_on_disk(data);
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
        ocr_switch::pack_readiness_on_disk(legacy.path()).problems(),
        [PackProblem::MapsMissing, PackProblem::WordsMissing]
    );

    // heroes-v2-1080 alone reads no 1440p board: it is not "a hero pack".
    let small = tempfile::tempdir().unwrap();
    hero_pack_in(small.path(), "heroes-v2-1080");
    map_pack(small.path());
    words_pack(small.path(), Words::Current);
    assert_eq!(
        ocr_switch::pack_readiness_on_disk(small.path()).problems(),
        [PackProblem::HeroesMissing]
    );
    hero_pack_in(small.path(), "heroes-v2");
    assert!(ocr_switch::pack_readiness_on_disk(small.path()).ready());
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

struct DaemonRun {
    /// Everything the daemon printed.
    out: String,
    /// `ocr_switch.json` as it was while the daemon ran.
    status: ocr_switch::Status,
    /// The same file after the daemon stopped.
    last_status: ocr_switch::Status,
    pid: u32,
}

/// How a test daemon is told what `ocr` is.
#[derive(Clone, Copy)]
enum Ask<'a> {
    /// `STAT_TRACKER_OCR`.
    Env(&'a str),
    /// `ocr = "..."` in config.toml, and no env override.
    File(&'a str),
}

/// The daemon binary on `data_dir`, with an empty HOME and XDG dirs below
/// `root`, its output in `log`. Started through `tests/common`, so it never
/// outlives the test: killed on drop, and by the kernel if the test binary
/// itself is killed.
fn spawn_daemon(
    root: &Path,
    data_dir: &Path,
    ask: Ask,
    args: &[&str],
    log: &Path,
) -> common::Daemon {
    let home = root.join("home");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(data_dir).unwrap();
    let out = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log)
        .unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_scuffed-stat-tracker"));
    cmd.args(args)
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("HOME", &home)
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("XDG_DATA_HOME", root.join("data"))
        .env("XDG_CACHE_HOME", root.join("cache"))
        .env("NO_COLOR", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::from(out.try_clone().unwrap()))
        .stderr(Stdio::from(out));
    match ask {
        Ask::Env(ocr) => {
            cmd.env(stat_tracker::config::OCR_ENV, ocr);
        }
        Ask::File(ocr) => {
            let dir = root.join("config").join("scuffed-stat-tracker");
            std::fs::create_dir_all(&dir).unwrap();
            // `data_dir` is a required key of the file.
            std::fs::write(
                dir.join("config.toml"),
                format!("data_dir = {:?}\nocr = \"{ocr}\"\n", data_dir),
            )
            .unwrap();
        }
    }
    common::spawn_daemon(cmd)
}

/// Wait until the daemon's SIGTERM handler is installed (in the run loop;
/// before that the signal's default action would kill it).
/// False at once when the daemon ended before that.
fn wait_until_ready(log: &Path, daemon: &mut common::Daemon) -> bool {
    let ready = |log: &Path| read(log).contains("signal handlers ready");
    wait_for("signal handlers ready", Duration::from_secs(90), || {
        ready(log) || matches!(daemon.child.try_wait(), Ok(Some(_)))
    }) && ready(log)
}

/// SIGTERM, then wait for the exit.
fn stop(daemon: &mut common::Daemon) -> std::process::ExitStatus {
    // SAFETY: plain kill(2) on the child this test spawned.
    unsafe { libc::kill(daemon.child.id() as i32, libc::SIGTERM) };
    wait_for("the daemon to exit", Duration::from_secs(60), || {
        matches!(daemon.child.try_wait(), Ok(Some(_)))
    });
    let _ = daemon.child.kill();
    daemon.child.wait().expect("wait for the daemon")
}

/// Start the daemon on `data_dir`, wait until it is in its run loop, read
/// the status file, then stop it with SIGTERM.
fn run_daemon(root: &Path, data_dir: &Path, ask: Ask) -> DaemonRun {
    let log_path = root.join("daemon.out");
    let mut daemon = spawn_daemon(root, data_dir, ask, &[], &log_path);
    let pid = daemon.child.id();
    let ready = wait_until_ready(&log_path, &mut daemon);
    let status = ocr_switch::read_status(data_dir);
    let exit = if ready {
        stop(&mut daemon)
    } else {
        let _ = daemon.child.kill();
        daemon.child.wait().expect("wait for the daemon")
    };
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
    let run = run_daemon(root.path(), &data_dir, Ask::Env("off"));

    assert_eq!(run.status.pid, run.pid);
    assert_eq!(run.status.requested, OcrSetting::Off);
    assert_eq!(run.status.effective, OcrSetting::On, "OCR stays on");
    let refusal = run
        .status
        .refusal
        .clone()
        .expect("the refusal has a reason");
    let needing = Read::needing_ocr();
    assert_eq!(refusal.reads, needing);
    let reason = refusal.text(Audience::Daemon);
    let lower = reason.to_lowercase();
    let refused: Vec<&str> = run
        .out
        .lines()
        .filter(|l| l.contains("WARN") && l.contains("was refused"))
        .collect();
    assert_eq!(refused.len(), 1, "one WARN line; output:\n{}", run.out);
    if needing.is_empty() {
        // This build could turn OCR off: the packs decide, and are named.
        assert_eq!(
            refusal.packs,
            [PackProblem::MapsMissing, PackProblem::WordsMissing],
            "the hero pack is there"
        );
        assert!(
            lower.contains("the map pack is not installed")
                && lower.contains("the word pack is not installed")
                && !lower.contains("hero pack"),
            "{reason}"
        );
        assert!(
            refused[0].contains("maps-v1")
                && refused[0]
                    .to_lowercase()
                    .contains("the map pack is not installed"),
            "the log says why, with the pack id: {}",
            refused[0]
        );
    } else {
        // This build refuses whatever the packs are: they are not looked
        // at, and the member is not sent to download any.
        assert!(refusal.packs.is_empty(), "no pack is named");
        assert_eq!(run.status.packs, None, "the packs were not loaded");
        assert!(!lower.contains("pack"), "{reason}");
        assert!(reason.ends_with(ocr_switch::TURN_BACK_ON), "{reason}");
    }
    assert_eq!(run.status.notice(), Some(reason));
    assert_eq!(
        run.status.health_notice(),
        None,
        "the tracker reads everything with classic OCR: not a health problem"
    );
    assert_eq!(run.status.blocked_total, 0);
    assert!(
        run.out.contains(OCR_WORKERS_LINE),
        "the daemon runs classic OCR; output:\n{}",
        run.out
    );
    assert!(!run.out.contains("classic OCR is off"), "{}", run.out);
    // a refused off runs with the file's reader: no `reader` line, so the
    // new reader (the default since alpha.12), with classic OCR still on
    assert!(run.out.contains(NEW_READER_LINE), "{}", run.out);
}

/// `ocr` is read from config.toml (the other daemon tests use the env
/// override), in any case.
#[test]
fn the_daemon_takes_ocr_off_from_config_toml() {
    let (root, data_dir) = daemon_dirs();
    let run = run_daemon(root.path(), &data_dir, Ask::File("Off"));
    assert_eq!(run.status.requested, OcrSetting::Off, "{}", run.out);
    assert_eq!(run.status.pid, run.pid);
    // No pack is installed, so off is refused on every build.
    assert_eq!(run.status.effective, OcrSetting::On);
    assert!(run.status.refusal.is_some());
    assert!(run.out.contains("was refused"), "{}", run.out);
}

/// The switch is decided below the pid guard and the `--vacuum` exit. A
/// second start that is refused for the running daemon, and a maintenance
/// run, leave the running daemon's status file as it is, byte for byte.
#[test]
fn a_second_start_and_a_vacuum_run_leave_the_status_file_alone() {
    let (root, data_dir) = daemon_dirs();
    let log = root.path().join("daemon.out");
    let mut live = spawn_daemon(root.path(), &data_dir, Ask::Env("on"), &[], &log);
    assert!(wait_until_ready(&log, &mut live), "output:\n{}", read(&log));
    let status_path = data_dir.join(ocr_switch::STATUS_FILE);
    let before = std::fs::read(&status_path).expect("the live daemon's status file");
    let live_status = ocr_switch::read_status(&data_dir).unwrap();
    assert_eq!(live_status.pid, live.child.id());
    assert_eq!(live_status.requested, OcrSetting::On);

    // Each of these asks for off, so a status written by them would differ.
    for (what, args) in [
        ("a second start", &[][..]),
        ("a vacuum run", &["--vacuum"][..]),
    ] {
        let other_log = root.path().join(format!("other-{}.out", args.len()));
        let mut other = spawn_daemon(root.path(), &data_dir, Ask::Env("off"), args, &other_log);
        let exited = wait_for(what, Duration::from_secs(60), || {
            matches!(other.child.try_wait(), Ok(Some(_)))
        });
        let out = read(&other_log);
        assert!(
            exited,
            "{what} did not end while a daemon runs; output:\n{out}"
        );
        let exit = other.child.wait().unwrap();
        assert!(
            !exit.success(),
            "{what} is refused for the running daemon; output:\n{out}"
        );
        assert!(
            !out.contains("was refused: classic OCR stays on")
                && !out.contains("classic OCR is off"),
            "{what} never reached the switch; output:\n{out}"
        );
        assert_eq!(
            std::fs::read(&status_path).unwrap(),
            before,
            "{what} rewrote the running daemon's status file"
        );
    }
    let exit = stop(&mut live);
    assert!(exit.success(), "exit {exit:?}; output:\n{}", read(&log));
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
    let run = run_daemon(root.path(), &data_dir, Ask::Env("off"));

    assert_eq!(run.status.requested, OcrSetting::Off);
    // The packs are looked at only when this build could turn OCR off.
    assert_eq!(
        run.status.packs,
        Read::needing_ocr().is_empty().then_some(READY)
    );
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
    let run = run_daemon(root.path(), &data_dir, Ask::Env("on"));
    assert_eq!(run.status.requested, OcrSetting::On);
    assert_eq!(run.status.effective, OcrSetting::On);
    assert_eq!(run.status.refusal, None);
    assert_eq!(run.status.packs, None, "the packs are not looked at");
    assert_eq!(run.status.notice(), None);
    assert!(run.out.contains(OCR_WORKERS_LINE), "{}", run.out);
    // no `reader` line: the new reader is the default, OCR stays on for
    // the boards it finds nothing on
    assert!(run.out.contains(NEW_READER_LINE), "{}", run.out);
    // the temp file of the status write does not stay behind
    assert!(!data_dir.join(ocr_switch::STATUS_TMP_FILE).exists());
}
