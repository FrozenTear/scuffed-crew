//! The daemon's end-title path under `reader = "new"` (alpha.7 O2), end to
//! end: `Titles::for_rgb` reads the process-wide reader, which loads the
//! end-title templates from the data dir in `SCUFFED_TEMPLATE_DIR`, and the
//! poll tick's signal, the wake and the Tab frame's outcome come from that
//! read. Synthetic templates, not game art. One test per process, so the
//! reader is set up exactly once, here.

use image::{DynamicImage, GrayImage, Luma, Rgb, RgbImage};
use stat_tracker::detect::MatchOutcome;
use stat_tracker::detect::match_end::{
    FinalScoreSource, OutcomeSource, Titles, detect_end_reel_with, detect_outcome_signal_polled,
    detect_outcome_text_with, detect_outcome_with,
};
use stat_tracker::detect::stability::FrameStability;
use stat_tracker::shadow::reader::TEMPLATE_DIR_ENV;
use stat_tracker::shadow::result::{Outcome, WORDS_SUBDIR};

/// Vertical bars in a seed-specific rhythm, 8-bit grey.
fn bars(seed: u32, w: u32, h: u32) -> GrayImage {
    GrayImage::from_fn(w, h, |x, y| {
        let on = (2..h - 2).contains(&y)
            && ((x / (2 + seed)).is_multiple_of(2) || (y + x * seed).is_multiple_of(11));
        Luma([if on { 229 } else { 0 }])
    })
}

/// A dark 1440p frame, with `word` painted (doubled, in a team colour)
/// where the end title sits.
fn frame(word: Option<&GrayImage>) -> RgbImage {
    let mut frame = RgbImage::from_pixel(2560, 1440, Rgb([20, 26, 50]));
    if let Some(word) = word {
        for y in 0..word.height() * 2 {
            for x in 0..word.width() * 2 {
                if word.get_pixel(x / 2, y / 2).0[0] > 100 {
                    frame.put_pixel(1100 + x, 640 + y, Rgb([150, 20, 160]));
                }
            }
        }
    }
    frame
}

#[test]
fn the_daemon_reads_the_end_title_from_the_words_pack() {
    let data = tempfile::tempdir().unwrap();
    let words = data.path().join(WORDS_SUBDIR);
    std::fs::create_dir_all(&words).unwrap();
    let (victory, defeat) = (bars(2, 90, 34), bars(4, 80, 34));
    for (name, img) in [
        ("accolade_victory.png", &bars(1, 70, 30)),
        ("accolade_defeat.png", &bars(3, 60, 30)),
        ("end_title_victory.png", &victory),
        ("end_title_defeat.png", &defeat),
    ] {
        img.save(words.join(name)).unwrap();
    }
    // SAFETY: this is the only test in this binary; nothing else reads the
    // environment concurrently.
    unsafe { std::env::set_var(TEMPLATE_DIR_ENV, data.path()) };
    let new = FinalScoreSource::for_reader(true);

    for (word, outcome, want) in [
        (&victory, Outcome::Victory, MatchOutcome::Victory),
        (&defeat, Outcome::Defeat, MatchOutcome::Defeat),
    ] {
        let rgb = frame(Some(word));
        let img = DynamicImage::ImageRgb8(rgb.clone());
        // reader = "new": one read, a sure word
        let titles = Titles::for_rgb(true, &rgb);
        let Titles::Words(Some(read)) = &titles else {
            panic!("no end-title read: {titles:?}");
        };
        assert_eq!(read.sure(), Some(outcome), "{read:?}");
        assert_eq!(Titles::for_frame(true, &img), titles);
        // the poll tick: a signal on the first sighting, and the wake
        let mut stability = FrameStability::default();
        assert_eq!(
            detect_outcome_signal_polled(&img, &rgb, &mut stability, new, &titles),
            Some((want, OutcomeSource::EndTitle))
        );
        assert!(detect_end_reel_with(&img, &rgb, &titles));
        // the Tab frame's vision pass, with and without banner recovery
        assert_eq!(detect_outcome_with(&img, new, &titles), want);
        assert_eq!(detect_outcome_text_with(&img, new, &titles), want);

        // reader = "ocr-v1" takes no read from the pack; bars are no text
        let ocr = Titles::for_rgb(false, &rgb);
        assert_eq!(ocr, Titles::Ocr);
        let mut stability = FrameStability::default();
        assert_eq!(
            detect_outcome_signal_polled(&img, &rgb, &mut stability, FinalScoreSource::Ocr, &ocr),
            None
        );
        assert!(!detect_end_reel_with(&img, &rgb, &ocr));
    }

    // no title on the frame: a read that names no word, no signal, no wake
    let rgb = frame(None);
    let img = DynamicImage::ImageRgb8(rgb.clone());
    let titles = Titles::for_rgb(true, &rgb);
    let Titles::Words(Some(read)) = &titles else {
        panic!("no end-title read: {titles:?}");
    };
    assert_eq!((read.outcome, read.sure()), (None, None), "{read:?}");
    let mut stability = FrameStability::default();
    assert_eq!(
        detect_outcome_signal_polled(&img, &rgb, &mut stability, new, &titles),
        None
    );
    assert!(!detect_end_reel_with(&img, &rgb, &titles));
    assert_eq!(
        detect_outcome_text_with(&img, new, &titles),
        MatchOutcome::Unknown
    );
}
