//! The process-wide reader's FINAL SCORE read (F257-2): `shadow::read_final_score`,
//! which P4 calls under `reader = "new"`, loads `p4_final_score.png` from the
//! data dir in `SCUFFED_TEMPLATE_DIR`. Synthetic templates, not game art. One
//! test per process, so the reader is set up exactly once, here.

use crate::common;

use image::{GrayImage, Luma, Rgb, RgbImage};
use stat_tracker::shadow::reader::TEMPLATE_DIR_ENV;
use stat_tracker::shadow::result::{FinalScoreRead, WORDS_SUBDIR};

/// Vertical bars in a seed-specific rhythm, 8-bit grey.
fn bars(seed: u32, w: u32, h: u32) -> GrayImage {
    GrayImage::from_fn(w, h, |x, y| {
        let on = (2..h - 2).contains(&y)
            && ((x / (2 + seed)).is_multiple_of(2) || (y + x * seed).is_multiple_of(11));
        Luma([if on { 229 } else { 0 }])
    })
}

#[test]
fn the_process_wide_reader_reads_the_final_score_line() {
    if !common::own_process(
        module_path!(),
        "the_process_wide_reader_reads_the_final_score_line",
    ) {
        return;
    }
    let data = tempfile::tempdir().unwrap();
    let words = data.path().join(WORDS_SUBDIR);
    std::fs::create_dir_all(&words).unwrap();
    bars(1, 70, 30)
        .save(words.join("accolade_victory.png"))
        .unwrap();
    bars(3, 60, 30)
        .save(words.join("accolade_defeat.png"))
        .unwrap();
    let line = bars(4, 123, 28);
    line.save(words.join("p4_final_score.png")).unwrap();
    // SAFETY: this test runs alone in a process of its own
    // (`common::own_process`); nothing else reads the environment
    // concurrently.
    unsafe { std::env::set_var(TEMPLATE_DIR_ENV, data.path()) };

    // the line, painted at the 1440p frame's FINAL SCORE position
    let mut frame = RgbImage::from_pixel(2560, 1440, Rgb([20, 26, 50]));
    for y in 0..line.height() * 2 {
        for x in 0..line.width() * 2 {
            if line.get_pixel(x / 2, y / 2).0[0] > 100 {
                frame.put_pixel(1102 + x, 1060 + y, Rgb([235, 235, 235]));
            }
        }
    }
    let r = stat_tracker::shadow::read_final_score(&frame);
    assert!(matches!(r, Some(FinalScoreRead::Sure(_))), "{r:?}");
    let blank = RgbImage::from_pixel(2560, 1440, Rgb([20, 26, 50]));
    let r = stat_tracker::shadow::read_final_score(&blank);
    assert!(r.is_some_and(|r| !r.is_signal()), "{r:?}");
}
