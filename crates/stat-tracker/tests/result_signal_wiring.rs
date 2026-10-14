//! The daemon's accolade and rank-screen result word under `reader = "new"`
//! (alpha.7 O11), end to end: `ResultWords::for_reader` reads the
//! process-wide reader, which loads the result templates from the data dir
//! in `SCUFFED_TEMPLATE_DIR`, and the poll tick's signal, its result
//! evidence and the Tab frame's outcome come from that read. Synthetic
//! templates, not game art. One test per process, so the reader is set up
//! exactly once, here.

use image::{DynamicImage, GrayImage, Luma, Rgb, RgbImage};
use stat_tracker::detect::MatchOutcome;
use stat_tracker::detect::match_end::{
    FinalScoreSource, OutcomeSource, ResultWords, Titles, detect_outcome_signal,
    detect_outcome_signal_polled, detect_outcome_with,
};
use stat_tracker::detect::stability::FrameStability;
use stat_tracker::shadow::reader::TEMPLATE_DIR_ENV;
use stat_tracker::shadow::result::{ACCOLADE, Outcome, RANK_SCREEN, WORDS_SUBDIR};

/// Vertical bars in a seed-specific rhythm, 8-bit grey.
fn bars(seed: u32, w: u32, h: u32) -> GrayImage {
    GrayImage::from_fn(w, h, |x, y| {
        let on = (2..h - 2).contains(&y)
            && ((x / (2 + seed)).is_multiple_of(2) || (y + x * seed).is_multiple_of(11));
        Luma([if on { 229 } else { 0 }])
    })
}

/// Pseudo-random 3 px blocks, 8-bit grey: lit texture that is no word.
fn noise(seed: u32, w: u32, h: u32) -> GrayImage {
    GrayImage::from_fn(w, h, |x, y| {
        let v = (x / 3).wrapping_mul(73_856_093)
            ^ (y / 3).wrapping_mul(19_349_663)
            ^ seed.wrapping_mul(83_492_791);
        Luma([if v % 5 < 2 { 229 } else { 0 }])
    })
}

/// A 1440p frame of one colour with `word` painted (doubled) at (x0, y0).
fn frame(back: [u8; 3], word: Option<(&GrayImage, u32, u32, [u8; 3])>) -> RgbImage {
    let mut frame = RgbImage::from_pixel(2560, 1440, Rgb(back));
    if let Some((word, x0, y0, colour)) = word {
        for y in 0..word.height() * 2 {
            for x in 0..word.width() * 2 {
                if word.get_pixel(x / 2, y / 2).0[0] > 100 {
                    frame.put_pixel(x0 + x, y0 + y, Rgb(colour));
                }
            }
        }
    }
    frame
}

const DARK: [u8; 3] = [20, 26, 50];
const WARM: [u8; 3] = [150, 100, 30];

#[test]
fn the_daemon_reads_the_result_word_from_the_words_pack() {
    let data = tempfile::tempdir().unwrap();
    let words_dir = data.path().join(WORDS_SUBDIR);
    std::fs::create_dir_all(&words_dir).unwrap();
    let accolade = (bars(1, 70, 30), bars(3, 60, 30));
    let rank = (bars(5, 66, 20), bars(7, 50, 20));
    for (name, img) in [
        ("accolade_victory.png", &accolade.0),
        ("accolade_defeat.png", &accolade.1),
        ("rank_screen_victory.png", &rank.0),
        ("rank_screen_defeat.png", &rank.1),
        // end titles too, so the new reader reads no title with OCR
        ("end_title_victory.png", &bars(2, 90, 34)),
        ("end_title_defeat.png", &bars(4, 80, 34)),
    ] {
        img.save(words_dir.join(name)).unwrap();
    }
    // SAFETY: this is the only test in this binary; nothing else reads the
    // environment concurrently.
    unsafe { std::env::set_var(TEMPLATE_DIR_ENV, data.path()) };
    let p4 = FinalScoreSource::for_reader(true);
    let gold = [255, 220, 0];
    let white = [245, 245, 245];

    // The accolade word sits top-left, in a team colour; the rank screen's
    // word is white, under the title.
    for (rgb, outcome, want, layout, source) in [
        (
            frame(DARK, Some((&accolade.0, 32, 60, gold))),
            Outcome::Victory,
            MatchOutcome::Victory,
            ACCOLADE,
            OutcomeSource::ResultWord,
        ),
        (
            frame(DARK, Some((&accolade.1, 32, 60, gold))),
            Outcome::Defeat,
            MatchOutcome::Defeat,
            ACCOLADE,
            OutcomeSource::ResultWord,
        ),
        (
            frame(WARM, Some((&rank.0, 45, 218, white))),
            Outcome::Victory,
            MatchOutcome::Victory,
            RANK_SCREEN,
            OutcomeSource::RankScreen,
        ),
        (
            frame(WARM, Some((&rank.1, 45, 218, white))),
            Outcome::Defeat,
            MatchOutcome::Defeat,
            RANK_SCREEN,
            OutcomeSource::RankScreen,
        ),
    ] {
        let img = DynamicImage::ImageRgb8(rgb.clone());
        // no end title on these frames
        let titles = Titles::for_rgb(true, &rgb);
        let Titles::Words(title) = &titles else {
            panic!("no end-title read: {titles:?}");
        };
        assert_eq!((title.outcome, title.sure()), (None, None), "{title:?}");

        // reader = "new", the poll tick. A first sighting is deferred, and
        // the frame is not read.
        let mut stability = FrameStability::default();
        let words = ResultWords::for_reader(true);
        assert!(words.from_words());
        assert_eq!(
            detect_outcome_signal_polled(&img, &rgb, &mut stability, p4, &titles, &words),
            None,
            "{layout}: first sighting"
        );
        assert_eq!(words.taken(), None);
        // The crop held still: the sure word is the signal, with the source
        // the accolade map read and the report kit are keyed on.
        let words = ResultWords::for_reader(true);
        assert_eq!(
            detect_outcome_signal_polled(&img, &rgb, &mut stability, p4, &titles, &words),
            Some((want, source)),
            "{layout}"
        );
        // One read: the read behind the signal is the frame's result read,
        // which the tick adds to the game's result evidence.
        let taken = words.into_taken().expect("the signal's read");
        assert_eq!(taken.sure(), Some(outcome), "{taken:?}");
        assert_eq!(taken.source.as_deref(), Some(layout));
        assert_eq!(taken, stat_tracker::shadow::read_result(&img));

        // reader = "new", the Tab frame's vision pass: no stability wait
        assert_eq!(
            detect_outcome_with(&img, p4, &titles, &ResultWords::for_reader(true)),
            want
        );

        // reader = "ocr-v1" reads nothing from the pack: bars are no text
        let ocr = ResultWords::ocr();
        let mut stability = FrameStability::default();
        for _ in 0..2 {
            assert_eq!(
                detect_outcome_signal_polled(
                    &img,
                    &rgb,
                    &mut stability,
                    FinalScoreSource::Ocr,
                    &Titles::Ocr,
                    &ocr
                ),
                None
            );
        }
        assert_eq!(ocr.taken(), None);
        assert_eq!(detect_outcome_signal(&img), None);
    }

    // No word on the frame: lit texture where the accolade word sits, where
    // the rank word sits, and in both places (not flat planes, so the read
    // has something to score). No signal, and no read to keep: the first
    // look at the two screens finds no sure word, so the frame is not read
    // in full.
    let (speckle, grain) = (noise(1, 110, 30), noise(2, 100, 36));
    let mut both = frame(DARK, Some((&speckle, 32, 60, gold)));
    for (x, y, px) in frame(DARK, Some((&grain, 45, 218, white))).enumerate_pixels() {
        if (200..300).contains(&y) {
            both.put_pixel(x, y, *px);
        }
    }
    for rgb in [
        frame(DARK, Some((&speckle, 32, 60, gold))),
        frame(WARM, Some((&grain, 45, 218, white))),
        both,
    ] {
        let img = DynamicImage::ImageRgb8(rgb.clone());
        let titles = Titles::for_rgb(true, &rgb);
        let mut stability = FrameStability::default();
        for _ in 0..2 {
            let words = ResultWords::for_reader(true);
            assert_eq!(
                detect_outcome_signal_polled(&img, &rgb, &mut stability, p4, &titles, &words),
                None
            );
            assert_eq!(words.taken(), None);
        }
        assert_eq!(
            detect_outcome_with(&img, p4, &titles, &ResultWords::for_reader(true)),
            MatchOutcome::Unknown
        );
    }
}
