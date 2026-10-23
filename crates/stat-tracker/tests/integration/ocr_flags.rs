//! Each capability flag of the OCR switch against the gate, in a process of
//! its own.
//!
//! `ocr = "off"` is granted only when every read is marked as done without
//! Tesseract ([`Read::without_ocr`]). Those marks are set by hand, one per
//! lane. This test ties each mark to the code: it turns OCR off for the
//! whole process, drives the daemon's real entry points on frames that
//! reach each read, and looks at what arrived at the gate.
//!
//! * A read marked `true` must send nothing to the gate. Flip a mark with
//!   no reader wired in behind it and this fails.
//! * A read marked `false` must send at least one call on its frame. That
//!   is what shows the frame reaches the read, so the first rule means
//!   something on the day the mark is flipped.
//!
//! The reads of the Tab pass (stat cells, own row, hero, top-bar map) need
//! the binary's `analyze_frame`: they have the same check in `main.rs`,
//! also alone in a process. The co-op board check has no OCR call of its
//! own and is tested by function there, through `handle_capture`.
//!
//! The process-wide mode and reader are set here, so this file holds one
//! test.

use crate::common;

use std::cell::Cell;
use std::collections::BTreeMap;

use image::{DynamicImage, Rgb, RgbImage};
use stat_tracker::config::{Config, OcrSetting, ReaderSetting};
use stat_tracker::detect::match_end::{self, FinalScoreSource, ResultWords, Titles};
use stat_tracker::detect::match_start::{self, PhaseSource};
use stat_tracker::detect::stability::FrameStability;
use stat_tracker::detect::{GamePhase, MatchOutcome};
use stat_tracker::ocr::gate;
use stat_tracker::ocr_switch::{PackProblem, PackReadiness, Read, Switch};
use stat_tracker::shadow::{self, Reader, ReaderConfig};

/// Where a read's mark is tied to the code.
#[derive(Debug, PartialEq)]
enum Checked {
    /// In this file, by a driver below.
    Here,
    /// `main.rs`: `own_process_tab_pass_with_ocr_off`.
    TabPass,
    /// `main.rs`: `with_ocr_off_a_coop_board_is_not_stored_and_a_two_team_adlersbrunn_game_is`.
    PveBoard,
    /// `main.rs`: `with_ocr_off_the_accolade_map_is_never_asked_of_ocr`. The
    /// choice between the map pack's read and the OCR read is made in the
    /// daemon (`choose_accolade_map`), not in the library.
    AccoladeChoice,
}

/// Every read has a place. A new [`Read`] does not compile until it has one.
fn checked(read: Read) -> Checked {
    match read {
        Read::ResultWord | Read::EndTitle | Read::MatchStart | Read::FinalScore => Checked::Here,
        Read::AccoladeMap => Checked::AccoladeChoice,
        Read::StatCells | Read::OwnRow | Read::Hero | Read::TopBarMap => Checked::TabPass,
        Read::PveBoard => Checked::PveBoard,
    }
}

fn frame(background: [u8; 3]) -> RgbImage {
    RgbImage::from_pixel(1920, 1080, Rgb(background))
}

fn fill(img: &mut RgbImage, (x0, y0, x1, y1): (u32, u32, u32, u32), colour: [u8; 3]) {
    for y in y0..y1 {
        for x in x0..x1 {
            img.put_pixel(x, y, Rgb(colour));
        }
    }
}

/// Dark navy with white bars where the accolade screen and the rank screen
/// print the result word. It passes both result readers' "is there a title"
/// gate and the vote screen's colour gate.
fn end_screen_like() -> DynamicImage {
    let mut img = frame([16, 25, 50]);
    fill(&mut img, (40, 45, 400, 95), [250, 250, 250]);
    fill(&mut img, (40, 165, 400, 235), [250, 250, 250]);
    DynamicImage::ImageRgb8(img)
}

/// A magenta band where the centred end title is drawn: enough colour mass
/// for the end title's gate.
fn end_title_like() -> DynamicImage {
    let mut img = frame([10, 10, 10]);
    fill(&mut img, (576, 378, 1344, 540), [220, 40, 220]);
    DynamicImage::ImageRgb8(img)
}

/// A dark cinematic band with bright glyph-like strokes where the end reel
/// prints its title, and nothing else: no letterbox, no nameplate.
fn end_reel_like() -> DynamicImage {
    let mut img = frame([12, 12, 12]);
    for y in 110..300 {
        for x in (240..1680).step_by(3) {
            img.put_pixel(x, y, Rgb([220, 220, 220]));
        }
    }
    DynamicImage::ImageRgb8(img)
}

/// No title colour, and a white line where FINAL SCORE is printed.
fn final_score_like() -> DynamicImage {
    let mut img = frame([10, 10, 10]);
    fill(&mut img, (700, 810, 1220, 850), [250, 250, 250]);
    DynamicImage::ImageRgb8(img)
}

type Sites = BTreeMap<(&'static str, u32, &'static str), u64>;

fn sites() -> Sites {
    gate::blocked_calls()
        .into_iter()
        .map(|b| ((b.site.file, b.site.line, b.site.entry), b.count))
        .collect()
}

/// The blocked calls `drive` made: `(file, entry, count)`.
fn blocked_during(drive: impl FnOnce()) -> Vec<(&'static str, &'static str, u64)> {
    let before = sites();
    drive();
    sites()
        .into_iter()
        .filter_map(|((file, line, entry), count)| {
            let new = count - before.get(&(file, line, entry)).copied().unwrap_or(0);
            (new > 0).then_some((file, entry, new))
        })
        .collect()
}

/// Calls among `blocked` in `file` (a path suffix) under one of `entries`.
fn count(blocked: &[(&str, &str, u64)], file: &str, entries: &[&str]) -> u64 {
    blocked
        .iter()
        .filter(|(f, entry, _)| f.ends_with(file) && entries.contains(entry))
        .map(|(_, _, n)| n)
        .sum()
}

/// What [`judge`] found wrong. Collected, so one run names every read that
/// is out of step with its mark, not only the first.
static PROBLEMS: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// The rule, for one read on one driver.
fn judge(read: Read, what: &str, calls: u64, blocked: &[(&str, &str, u64)]) {
    assert_eq!(checked(read), Checked::Here);
    let problem = if read.without_ocr() && calls > 0 {
        format!(
            "{read:?} is marked as read without OCR, and {what} still sent {calls} call(s) to \
             the gate: {blocked:?}"
        )
    } else if !read.without_ocr() && calls == 0 {
        format!(
            "{read:?} still needs OCR, and {what} never reached its OCR call, so this frame \
             would not notice the read being flipped: {blocked:?}"
        )
    } else {
        return;
    };
    PROBLEMS.lock().unwrap().push(problem);
}

#[test]
fn a_read_marked_as_done_without_ocr_never_reaches_the_gate() {
    if !common::own_process(
        module_path!(),
        "a_read_marked_as_done_without_ocr_never_reaches_the_gate",
    ) {
        return;
    }
    for read in Read::ALL {
        // Compiles only while every read has a place; say it at run time too.
        let _ = checked(read);
    }

    // ---- The reader, once (F261-3). Off refused for a pack under
    // `reader = "new"`: the reader the switch looked at becomes the
    // process-wide one, so the daemon does not load the packs a second time.
    let empty = tempfile::tempdir().unwrap();
    let loads = Cell::new(0u32);
    let config = Config {
        data_dir: empty.path().to_path_buf(),
        ocr: OcrSetting::Off,
        reader: ReaderSetting::New,
        ..Config::default()
    };
    let switch = Switch::start_with(&config, &[], |data_dir| {
        loads.set(loads.get() + 1);
        Reader::load(&ReaderConfig::from_data_dir(data_dir))
    });
    assert!(!switch.is_off(), "no pack: off is refused");
    assert_eq!(
        switch.decision.refusal.as_ref().map(|r| r.packs.clone()),
        Some(vec![
            PackProblem::HeroesMissing,
            PackProblem::MapsMissing,
            PackProblem::WordsMissing
        ])
    );
    assert!(switch.watches(), "the packs may still arrive");
    assert!(switch.decision.reader(&config).is_new());
    // What the daemon does next under the new reader: nothing is loaded.
    shadow::init(&ReaderConfig::from_data_dir(empty.path()));
    assert_eq!(loads.get(), 1, "the packs are loaded once");
    assert_eq!(shadow::pack_readiness(), PackReadiness::default());
    assert!(!gate::is_off(), "a refusal leaves OCR on");

    // ---- From here the whole process runs with OCR off, as a daemon that
    // was granted off does. (Set directly: the switch refuses it today.)
    gate::set_mode(gate::OcrMode::Off);
    assert_eq!(gate::mode(), gate::OcrMode::Off);
    let words = FinalScoreSource::for_reader(true);
    const END: &str = "detect/match_end.rs";

    // The result word on the accolade screen and the rank screen: the poll
    // tick's signal, and the Tab pass's frame outcome.
    let end = end_screen_like();
    let end_rgb = end.to_rgb8();
    let blocked = blocked_during(|| {
        let mut stability = FrameStability::default();
        for _ in 0..2 {
            // as the poll tick does: the titles first, then the signal
            let titles = Titles::for_rgb(true, &end_rgb);
            assert_eq!(
                match_end::detect_outcome_signal_polled(
                    &end,
                    &end_rgb,
                    &mut stability,
                    words,
                    &titles,
                    &ResultWords::for_reader(true)
                ),
                None
            );
        }
        // as the Tab pass does
        assert_eq!(
            match_end::detect_outcome_with(
                &end,
                words,
                &Titles::for_frame(true, &end),
                &ResultWords::for_reader(true)
            ),
            MatchOutcome::Unknown
        );
    });
    judge(
        Read::ResultWord,
        "the poll tick and the Tab frame outcome on an end screen",
        count(&blocked, END, &["recognize_prepared"]),
        &blocked,
    );

    // The centred end title. No end-title templates are loaded here, and
    // with OCR off there is no OCR to fall back to: the title is held.
    let title = end_title_like();
    assert_eq!(Titles::for_frame(true, &title), Titles::Held);
    let blocked = blocked_during(|| {
        assert_eq!(
            match_end::detect_outcome_text_with(&title, words, &Titles::for_frame(true, &title)),
            MatchOutcome::Unknown
        );
    });
    let title_calls = count(&blocked, END, &["recognize_prepared_lang"]);
    judge(Read::EndTitle, "the end title read", title_calls, &blocked);

    // The end-of-match wake: the end reel's title.
    let reel = end_reel_like();
    let reel_rgb = reel.to_rgb8();
    let blocked = blocked_during(|| {
        let titles = Titles::for_rgb(true, &reel_rgb);
        let _ = match_end::detect_end_reel_with(&reel, &reel_rgb, &titles);
    });
    judge(
        Read::EndTitle,
        "the end reel's title read",
        count(&blocked, END, &["recognize_prepared"]),
        &blocked,
    );

    // The FINAL SCORE line under a faint end title. No template is loaded
    // here, and with OCR off there is no OCR to fall back to.
    let score = final_score_like();
    let blocked = blocked_during(|| {
        assert_eq!(
            match_end::detect_outcome_text_with(&score, words, &Titles::for_frame(true, &score)),
            MatchOutcome::Unknown
        );
    });
    judge(
        Read::FinalScore,
        "the final score line",
        blocked.iter().map(|(_, _, n)| n).sum(),
        &blocked,
    );

    // The match-start screens on the poll tick. The frame passes the vote
    // gate and holds still, and no title mask is loaded.
    let blocked = blocked_during(|| {
        let mut stability = FrameStability::default();
        for _ in 0..3 {
            assert!(matches!(
                match_start::detect_phase_polled(
                    &end,
                    &end_rgb,
                    &mut stability,
                    PhaseSource::for_reader(true)
                ),
                GamePhase::Unknown
            ));
        }
    });
    judge(
        Read::MatchStart,
        "the poll tick's phase read",
        blocked.iter().map(|(_, _, n)| n).sum(),
        &blocked,
    );

    let problems = PROBLEMS.lock().unwrap().clone();
    assert!(
        problems.is_empty(),
        "{} read(s) out of step with their mark:\n{}",
        problems.len(),
        problems.join("\n")
    );

    // Whatever was blocked, nothing of Tesseract was started.
    assert_eq!(gate::leptess_created(), 0, "no LepTess");
    assert_eq!(gate::tesseract_calls(), 0, "no Tesseract read");
    assert!(!stat_tracker::ocr::pool_started(), "no OCR pool");
    assert!(
        !stat_tracker::ocr::tessdata_lang_probed(),
        "no tessdata lookup"
    );
}
