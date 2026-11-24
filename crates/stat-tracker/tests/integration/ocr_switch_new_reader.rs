//! The OCR switch off under the new reader (`reader` left at its default),
//! in a process of its own: the switch installs the process-wide reader and
//! shuts the process-wide gate, and nothing of Tesseract starts while the
//! new reader reads. The ocr-v1 version is the gate test in
//! `tests/integration/ocr_switch.rs`.

use crate::common;

use std::path::Path;

use image::{DynamicImage, GrayImage, Luma, Rgb, RgbImage};
use stat_tracker::config::{Config, OcrSetting, ReaderSetting};
use stat_tracker::detect::{GamePhase, MatchOutcome, match_end, match_start};
use stat_tracker::ocr::{self, gate};
use stat_tracker::ocr_switch::{self, PackReadiness, Switch};
use stat_tracker::shadow::{Reader, ReaderConfig};

const READY: PackReadiness = PackReadiness {
    heroes: true,
    maps: true,
    words: true,
    result_words: true,
    phase_titles: true,
    end_titles: true,
};

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

/// The same synthetic packs as `tests/integration/ocr_switch.rs` writes for a granted off.
fn packs(data_dir: &Path) {
    let heroes = data_dir.join("templates").join("heroes");
    save_png(&heroes.join("ana.png"), &blocks(64, 64, 8, 0));
    save_png(&heroes.join("mercy.png"), &blocks(64, 64, 16, 4));

    let maps = data_dir.join("templates").join("maps-v1");
    let mut glyphs = Vec::new();
    for (i, c) in ('A'..='Z').enumerate() {
        let file = format!("g{:04x}.png", c as u32);
        save_png(
            &maps.join(&file),
            &blocks(18, 32, 4 + (i as u32 % 3), i as u32),
        );
        glyphs.push(format!("\"{c}\": \"{file}\""));
    }
    std::fs::write(
        maps.join("atlas.json"),
        format!(
            "{{\"height\": 32, \"gap\": 2.0, \"space\": 10.0, \"glyphs\": {{{}}}}}",
            glyphs.join(", ")
        ),
    )
    .unwrap();

    let words = data_dir.join("templates").join("words-v1");
    save_png(&words.join("accolade_victory.png"), &blocks(224, 48, 12, 0));
    save_png(&words.join("accolade_defeat.png"), &blocks(192, 48, 16, 6));
    save_png(
        &words.join("rank_screen_victory.png"),
        &blocks(224, 48, 10, 2),
    );
    save_png(
        &words.join("rank_screen_defeat.png"),
        &blocks(192, 48, 14, 5),
    );
    save_png(&words.join("end_title_victory.png"), &blocks(224, 48, 9, 1));
    save_png(&words.join("end_title_defeat.png"), &blocks(192, 48, 13, 7));
    save_png(&words.join("p4_final_score.png"), &blocks(256, 48, 12, 3));
    let mut entries = Vec::new();
    for (i, id) in [
        "vote_for_a_map",
        "selecting_map",
        "ban_heroes",
        "select_a_preferred_hero",
    ]
    .iter()
    .enumerate()
    {
        entries.push(format!(
            "\"{id}\": {{\"roi\": [0.36, 0.11, 0.64, 0.235], \"plane\": \"min\", \"work\": 0.5}}"
        ));
        save_png(
            &words.join(format!("phrase_{id}.png")),
            &blocks(200, 40, 8 + 2 * i as u32, i as u32),
        );
    }
    std::fs::write(
        words.join("phrases.json"),
        format!("{{{}}}", entries.join(", ")),
    )
    .unwrap();
}

/// A dark navy 1080p frame with white bars where the end screens print the
/// result word, so the result readers go on past their "is there a title"
/// gate.
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

fn nothing_of_tesseract_started(when: &str) {
    assert_eq!(gate::leptess_created(), 0, "{when}: a LepTess was created");
    assert!(!ocr::pool_started(), "{when}: the OCR pool was built");
    assert!(
        !ocr::tessdata_lang_probed(),
        "{when}: the tessdata was looked up"
    );
}

#[test]
fn with_ocr_off_the_new_reader_starts_nothing_of_tesseract() {
    if !common::own_process(
        module_path!(),
        "with_ocr_off_the_new_reader_starts_nothing_of_tesseract",
    ) {
        return;
    }
    assert_eq!(
        gate::mode(),
        gate::OcrMode::On,
        "a process starts with OCR on"
    );
    nothing_of_tesseract_started("at start");

    let live = tempfile::tempdir().unwrap();
    packs(live.path());
    let config = Config {
        data_dir: live.path().to_path_buf(),
        ocr: OcrSetting::Off,
        ..Config::default()
    };
    assert_eq!(config.reader, ReaderSetting::New, "the default reader");

    let switch = Switch::start_with(&config, &[], |dir| {
        Reader::load(&ReaderConfig::from_data_dir(dir))
    });
    assert!(
        switch.is_off(),
        "every pack is there: {:?}",
        switch.decision
    );
    assert!(switch.decision.reader(&config).is_new());
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
    nothing_of_tesseract_started("after the switch");

    // The screens the new reader reads, on a frame that gets past every
    // reader's first gate: nothing reaches Tesseract.
    let frame = end_screen_like_frame();
    assert_eq!(match_end::detect_outcome(&frame), MatchOutcome::Unknown);
    let _ = match_end::detect_outcome_signal(&frame);
    let _ = match_end::read_accolade_map(&frame);
    assert!(matches!(
        match_start::detect_phase(&frame),
        GamePhase::Unknown
    ));
    let board = DynamicImage::ImageRgb8(RgbImage::from_pixel(1248, 756, Rgb([20, 22, 30])));
    let _ = stat_tracker::shadow::read_board_with(&board, None, None);
    nothing_of_tesseract_started("after the new reader read");
    gate::set_mode(gate::OcrMode::On);
}
