//! Tests for [`super`] (`detect::match_end`).

use super::*;
use image::{Rgb, RgbImage};

fn flood(color: [u8; 3]) -> DynamicImage {
    DynamicImage::ImageRgb8(RgbImage::from_pixel(640, 360, Rgb(color)))
}

#[test]
fn black_frame_is_no_signal() {
    assert_eq!(detect_outcome_signal(&flood([0, 0, 0])), None);
    assert_eq!(detect_outcome(&flood([0, 0, 0])), MatchOutcome::Unknown);
}

/// R9-1 (0.5.0-rc.9): on 2026-10-22 one mid-match poll frame of a
/// Vendetta game was taken as Defeat because red covered more than 35%
/// of the middle band of the screen, and the game was split in two. A
/// red or gold flood is no outcome on any entry point, under either
/// reader, on any number of ticks.
#[test]
fn a_red_flooded_mid_match_frame_is_no_outcome() {
    // A Vendetta-red middle band over a dark game frame, and the
    // full-screen floods the colour check took as DEFEAT and VICTORY.
    let mut band = RgbImage::from_pixel(640, 360, Rgb([20, 24, 30]));
    for y in 90..270 {
        for x in 0..640 {
            band.put_pixel(x, y, Rgb([200, 30, 30]));
        }
    }
    let frames = [
        DynamicImage::ImageRgb8(band),
        flood([200, 30, 30]),
        flood([230, 180, 20]),
    ];
    // A lit crop is read by the words pack (it finds no word there).
    let words = || pack_words(BOTH, Some(unread_frame()));
    for img in &frames {
        let rgb = img.to_rgb8();
        assert_eq!(detect_outcome_signal(img), None);
        assert_eq!(detect_outcome(img), MatchOutcome::Unknown);
        assert_eq!(
            detect_outcome_with(img, NEW, &no_title(), &words()),
            MatchOutcome::Unknown
        );
        let mut st = FrameStability::default();
        for tick in 0..3 {
            assert_eq!(
                poll_new(&(img.clone(), rgb.clone()), &mut st, &words()),
                None
            );
            assert_eq!(
                poll_ocr(&(img.clone(), rgb.clone()), &mut st),
                None,
                "tick {tick}"
            );
        }
    }
}

#[test]
fn fuzzy_outcome_accepts_near_miss_and_rejects_ambiguity() {
    assert_eq!(fuzzy_outcome_word("VCTORY!"), MatchOutcome::Victory);
    assert_eq!(fuzzy_outcome_word("SVICTORY"), MatchOutcome::Victory);
    assert_eq!(fuzzy_outcome_word("DEFEA"), MatchOutcome::Defeat);
    assert_eq!(fuzzy_outcome_word("DRAW"), MatchOutcome::Draw);
    // VECTOR is uniquely closer to VICTORY than DEFEAT/DRAW at lev≤2.
    assert_eq!(fuzzy_outcome_word("VECTOR"), MatchOutcome::Victory);
    assert_eq!(fuzzy_outcome_word(""), MatchOutcome::Unknown);
    assert_eq!(fuzzy_outcome_word("HELLO"), MatchOutcome::Unknown);
    // Two targets inside the window → Unknown (do not pick a winner).
    assert_eq!(fuzzy_outcome_word("VICTORYDEFEAT"), MatchOutcome::Unknown);
    // 2-letter fragments are lev≤2 from DRAW; must not become a Draw.
    assert_eq!(fuzzy_outcome_word("DR"), MatchOutcome::Unknown);
    assert_eq!(fuzzy_outcome_word("RA"), MatchOutcome::Unknown);
    assert_eq!(fuzzy_outcome_word("AW"), MatchOutcome::Unknown);
}

#[test]
fn accolade_fuzzy_reads_near_victory_and_defeat() {
    assert_eq!(
        fuzzy_outcome_word_accolade("VCTORY!"),
        MatchOutcome::Victory
    );
    assert_eq!(fuzzy_outcome_word_accolade("DEFEATM"), MatchOutcome::Defeat);
    assert_eq!(fuzzy_outcome_word_accolade("DRAW"), MatchOutcome::Draw);
    assert_eq!(fuzzy_outcome_word_accolade("DRW"), MatchOutcome::Unknown);
}

#[test]
fn accolade_fuzzy_rejects_three_letter_draw_garbage() {
    // Tab path has no sat-mass gate and no 2-read confirm. lev≤2 on DRAW
    // would accept DRA/RAW/DAG; keep those Unknown on this path.
    assert_eq!(fuzzy_outcome_word_accolade("DRA"), MatchOutcome::Unknown);
    assert_eq!(fuzzy_outcome_word_accolade("RAW"), MatchOutcome::Unknown);
    assert_eq!(fuzzy_outcome_word_accolade("DAG"), MatchOutcome::Unknown);
    assert_eq!(fuzzy_outcome_word_accolade("HELLO"), MatchOutcome::Unknown);
}

#[test]
fn center_title_mass_sees_magenta_blob_not_black() {
    let mut img = RgbImage::from_pixel(640, 360, Rgb([10, 10, 10]));
    // Magenta title band: x 30–70%, y 35–50% of the 16:9 frame.
    for y in 126..180 {
        for x in 192..448 {
            img.put_pixel(x, y, Rgb([220, 40, 220]));
        }
    }
    assert!(
        center_title_mass(&img) >= END_TITLE_MASS_MIN,
        "mass={}",
        center_title_mass(&img)
    );
    assert_eq!(
        center_title_mass(&RgbImage::from_pixel(640, 360, Rgb([0, 0, 0]))),
        0.0
    );
}

// ------------------------- end title and its wake without OCR (O2)

use crate::ocr::ocr_calls;
use crate::shadow::result::END_TITLE;

/// The FINAL SCORE source `reader = "new"` passes along with its titles.
const NEW: FinalScoreSource = FinalScoreSource::Words;

fn dark_frame() -> (DynamicImage, RgbImage) {
    let rgb = RgbImage::from_pixel(640, 360, Rgb([10, 10, 10]));
    (DynamicImage::ImageRgb8(rgb.clone()), rgb)
}

/// A frame whose centre passes the end-title colour-mass gate, so
/// ocr-v1 runs its end-title OCR on it.
fn title_mass_frame() -> (DynamicImage, RgbImage) {
    let mut rgb = RgbImage::from_pixel(640, 360, Rgb([10, 10, 10]));
    for y in 126..180 {
        for x in 192..448 {
            rgb.put_pixel(x, y, Rgb([220, 40, 220]));
        }
    }
    assert!(center_title_mass(&rgb) >= END_TITLE_MASS_MIN);
    (DynamicImage::ImageRgb8(rgb.clone()), rgb)
}

/// No title colour, and a white bar on the FINAL SCORE line: ocr-v1
/// runs the FINAL SCORE OCR on it (P4).
fn scoreline_bar_frame() -> (DynamicImage, RgbImage) {
    let mut rgb = RgbImage::from_pixel(640, 360, Rgb([10, 10, 10]));
    for y in 268..284 {
        for x in 240..400 {
            rgb.put_pixel(x, y, Rgb([245, 245, 245]));
        }
    }
    assert!(center_title_mass(&rgb) < END_TITLE_MASS_MIN);
    (DynamicImage::ImageRgb8(rgb.clone()), rgb)
}

/// A dark frame with bright bars in the upper band: it passes the title
/// card's pixel gate, so ocr-v1 runs its phrase OCR on it. The bars
/// spell no phrase, and no other path wakes on it.
fn title_card_frame() -> (DynamicImage, RgbImage) {
    let mut rgb = RgbImage::from_pixel(640, 360, Rgb([12, 12, 12]));
    for y in 50..90 {
        for x in 120..520 {
            if x % 3 == 0 {
                rgb.put_pixel(x, y, Rgb([225, 225, 225]));
            }
        }
    }
    let img = DynamicImage::ImageRgb8(rgb.clone());
    assert!(cinematic_title_band(
        &end_reel_cinematic_crop(&img).unwrap()
    ));
    assert!(!end_reel_letterbox(&rgb) && !read_end_reel_nameplate(&rgb));
    (img, rgb)
}

fn read_of(outcome: Option<Outcome>, suspect: bool) -> ResultRead {
    ResultRead {
        outcome,
        source: outcome.map(|_| END_TITLE.to_string()),
        score: if outcome.is_some() { 0.9 } else { 0.2 },
        margin: 0.4,
        suspect,
    }
}

fn words(outcome: Option<Outcome>, suspect: bool) -> Titles {
    Titles::Words(read_of(outcome, suspect))
}

/// Every words-v1 read that is not a sure word.
fn unsure_words() -> Vec<Titles> {
    vec![
        words(Some(Outcome::Victory), true),
        words(Some(Outcome::Defeat), true),
        words(Some(Outcome::Draw), true),
        words(None, true),
    ]
}

/// `reader = "ocr-v1"` takes no words-v1 read, and its end title is
/// still the gated OCR read.
#[test]
fn ocr_v1_reads_the_end_title_with_ocr_only() {
    assert_eq!(Titles::default(), Titles::Ocr);
    let (img, rgb) = title_mass_frame();
    assert_eq!(Titles::for_rgb(false, &rgb), Titles::Ocr);
    assert_eq!(Titles::for_frame(false, &img), Titles::Ocr);
    // and the words pack is not asked
    let never = |_: &RgbImage| -> Option<ResultRead> { panic!("words-v1 read under ocr-v1") };
    assert_eq!(Titles::for_rgb_with(false, &rgb, never), Titles::Ocr);
    assert_eq!(Titles::for_frame_with(false, &img, never), Titles::Ocr);
    // past the colour-mass gate: one Tesseract call, on the title crop
    let before = ocr_calls();
    let _ = read_end_title(&img, &rgb, None, FinalScoreSource::Ocr, &Titles::Ocr);
    assert_eq!(ocr_calls() - before, 1);
    // a dark frame stays behind the gate
    let (img, rgb) = dark_frame();
    let before = ocr_calls();
    assert_eq!(
        read_end_title(&img, &rgb, None, FinalScoreSource::Ocr, &Titles::Ocr),
        MatchOutcome::Unknown
    );
    assert_eq!(ocr_calls(), before);
}

/// `reader = "new"` with no end-title templates loaded (F263-1): the
/// titles are the OCR ones, so P3 and P5 make the Tesseract calls they
/// make under ocr-v1, and the fallback is logged once per process. With
/// templates loaded the titles are the words-v1 read, and no OCR runs
/// on the same frames.
#[test]
fn the_new_reader_without_end_title_templates_reads_the_titles_with_ocr() {
    let none = |_: &RgbImage| -> Option<ResultRead> { None };
    let (img, rgb) = title_mass_frame();
    let fallback = Titles::for_rgb_with(true, &rgb, none);
    assert_eq!(fallback, Titles::Ocr);
    assert_eq!(Titles::for_frame_with(true, &img, none), Titles::Ocr);
    assert!(
        !warn_end_title_ocr_fallback(),
        "the fallback logged already"
    );
    // P3 on a Tab frame: the gated end-title OCR, one Tesseract call
    let before = ocr_calls();
    let _ = read_end_title(&img, &rgb, None, NEW, &fallback);
    assert_eq!(ocr_calls() - before, 1);
    let before = ocr_calls();
    let _ = detect_outcome_text_with(&img, NEW, &fallback);
    assert_eq!(ocr_calls() - before, 1);
    // P3 on the poll tick: the stability wait, then the OCR
    let mut st = FrameStability::default();
    let before = ocr_calls();
    let _ = detect_outcome_signal_polled(&img, &rgb, &mut st, NEW, &fallback, &unread_words());
    assert_eq!(ocr_calls(), before, "a first sighting is deferred");
    let _ = detect_outcome_signal_polled(&img, &rgb, &mut st, NEW, &fallback, &unread_words());
    assert!(ocr_calls() > before, "the second tick reads with OCR");
    // P5: the phrase OCR behind the title card's pixel gate
    let (card_img, card_rgb) = title_card_frame();
    let card = Titles::for_rgb_with(true, &card_rgb, none);
    let before = ocr_calls();
    assert!(!detect_end_reel_with(&card_img, &card_rgb, &card));
    assert_eq!(ocr_calls() - before, 1, "the phrase OCR ran");

    // templates loaded: the words-v1 read, and no OCR on either frame
    let loaded = Titles::for_rgb_with(true, &rgb, |_| Some(read_of(Some(Outcome::Defeat), false)));
    assert_eq!(loaded, words(Some(Outcome::Defeat), false));
    let blank = Titles::for_rgb_with(true, &rgb, |_| Some(read_of(None, true)));
    assert_eq!(blank, words(None, true));
    let before = ocr_calls();
    assert_eq!(
        read_end_title(&img, &rgb, None, NEW, &loaded),
        MatchOutcome::Defeat
    );
    assert_eq!(
        read_end_title(&img, &rgb, None, NEW, &blank),
        MatchOutcome::Unknown
    );
    assert!(detect_end_reel_with(&card_img, &card_rgb, &loaded));
    assert!(!detect_end_reel_with(&card_img, &card_rgb, &blank));
    assert_eq!(ocr_calls(), before, "no OCR with the templates loaded");
}

/// Capture hands RGBA frames. The words-v1 read gets that frame's RGB
/// pixels, and its read is the frame's titles (F263-7).
#[test]
fn an_rgba_frame_is_read_as_its_rgb_pixels() {
    let (rgb8, rgb) = title_mass_frame();
    let rgba = DynamicImage::ImageRgba8(rgb8.to_rgba8());
    assert!(rgba.as_rgb8().is_none() && rgb8.as_rgb8().is_some());
    for (frame, word) in [(&rgba, Outcome::Victory), (&rgb8, Outcome::Draw)] {
        let seen = std::cell::Cell::new(0);
        let titles = Titles::for_frame_with(true, frame, |got| {
            seen.set(seen.get() + 1);
            assert_eq!(got, &rgb, "the frame's RGB pixels");
            Some(read_of(Some(word), false))
        });
        assert_eq!(seen.get(), 1, "one read per frame");
        assert_eq!(titles, words(Some(word), false));
    }
    // and without templates the RGBA frame falls back like an RGB one
    assert_eq!(Titles::for_frame_with(true, &rgba, |_| None), Titles::Ocr);
}

/// `reader = "new"`: the word of a sure words-v1 read is the end title
/// of a one-shot read. A flagged read and a frame with no word are "no
/// end title". No Tesseract call is made, on frames where ocr-v1 makes
/// one for the title (P3) or for the FINAL SCORE line (P4).
#[test]
fn only_a_sure_words_read_is_an_end_title() {
    let (bar_img, bar_rgb) = scoreline_bar_frame();
    let before = ocr_calls();
    let _ = read_end_title(
        &bar_img,
        &bar_rgb,
        None,
        FinalScoreSource::Ocr,
        &Titles::Ocr,
    );
    assert!(ocr_calls() > before, "ocr-v1 reads the FINAL SCORE line");

    let before = ocr_calls();
    for (img, rgb) in [title_mass_frame(), (bar_img, bar_rgb), dark_frame()] {
        for (o, want) in [
            (Outcome::Victory, MatchOutcome::Victory),
            (Outcome::Defeat, MatchOutcome::Defeat),
            (Outcome::Draw, MatchOutcome::Draw),
        ] {
            let got = read_end_title(&img, &rgb, None, NEW, &words(Some(o), false));
            assert_eq!(got, want);
        }
        for titles in unsure_words() {
            assert_eq!(
                read_end_title(&img, &rgb, None, NEW, &titles),
                MatchOutcome::Unknown,
                "{titles:?}"
            );
        }
    }
    assert_eq!(ocr_calls(), before, "no OCR under the new reader");
}

/// The poll tick under `reader = "new"` (F263-2): the first sure end
/// title is no signal, it only wakes. The same sure word on the next
/// poll tick is the `EndTitle` signal. Another word, a tick with no
/// sure word and a reset each start over. A Tab frame is one shot.
#[test]
fn the_poll_tick_needs_the_same_sure_end_title_on_two_ticks() {
    let (img, rgb) = dark_frame();
    let before = ocr_calls();
    let poll = |st: &mut FrameStability, titles: &Titles| {
        detect_outcome_signal_polled(&img, &rgb, st, NEW, titles, &unread_words())
    };
    let signal = |o| Some((o, OutcomeSource::EndTitle));
    let defeat = words(Some(Outcome::Defeat), false);
    let victory = words(Some(Outcome::Victory), false);
    let draw = words(Some(Outcome::Draw), false);

    // first sighting: no signal, and the wake
    let mut st = FrameStability::default();
    assert_eq!(poll(&mut st, &defeat), None);
    assert!(detect_end_reel_with(&img, &rgb, &defeat));
    // the same word on the next tick, and on every tick it stays
    assert_eq!(poll(&mut st, &defeat), signal(MatchOutcome::Defeat));
    assert_eq!(poll(&mut st, &defeat), signal(MatchOutcome::Defeat));
    // another word starts over
    assert_eq!(poll(&mut st, &victory), None);
    assert_eq!(poll(&mut st, &defeat), None);
    assert_eq!(poll(&mut st, &victory), None);
    assert_eq!(poll(&mut st, &victory), signal(MatchOutcome::Victory));
    assert_eq!(poll(&mut st, &draw), None);
    assert_eq!(poll(&mut st, &draw), signal(MatchOutcome::Draw));
    // a tick with no sure word between two sightings starts over
    for unsure in unsure_words() {
        let mut st = FrameStability::default();
        assert_eq!(poll(&mut st, &defeat), None);
        assert_eq!(poll(&mut st, &unsure), None, "{unsure:?}");
        assert_eq!(poll(&mut st, &defeat), None, "{unsure:?}");
        assert_eq!(poll(&mut st, &defeat), signal(MatchOutcome::Defeat));
    }
    // unsure reads never add up
    for unsure in unsure_words() {
        let mut st = FrameStability::default();
        for _ in 0..3 {
            assert_eq!(poll(&mut st, &unsure), None, "{unsure:?}");
        }
    }
    // a reset (the game went away) forgets the first sighting
    let mut st = FrameStability::default();
    assert_eq!(poll(&mut st, &defeat), None);
    st.reset();
    assert_eq!(poll(&mut st, &defeat), None);
    assert_eq!(poll(&mut st, &defeat), signal(MatchOutcome::Defeat));

    // A colour flood is no outcome (R9-1): the end title on that tick
    // decides, and its memory moves on as on any other tick. A flood
    // tick with no title on it breaks the run, one with the title on
    // it continues it.
    let gold = flood([230, 180, 20]);
    let gold_rgb = gold.to_rgb8();
    // The flood lights the accolade and rank crops, so the words pack
    // is asked; it finds no word.
    let flood_words = pack_words(BOTH, Some(unread_frame()));
    let mut st = FrameStability::default();
    assert_eq!(poll(&mut st, &defeat), None);
    assert_eq!(
        detect_outcome_signal_polled(
            &gold,
            &gold_rgb,
            &mut st,
            NEW,
            &words(None, true),
            &flood_words
        ),
        None
    );
    assert_eq!(poll(&mut st, &defeat), None, "the flood tick had no title");
    assert_eq!(
        detect_outcome_signal_polled(&gold, &gold_rgb, &mut st, NEW, &defeat, &flood_words),
        signal(MatchOutcome::Defeat)
    );

    // a Tab frame is one shot: a sure word is that frame's outcome
    assert_eq!(
        detect_outcome_with(&img, NEW, &victory, &unread_words()),
        MatchOutcome::Victory
    );
    assert_eq!(
        detect_outcome_with(&img, NEW, &words(None, true), &unread_words()),
        MatchOutcome::Unknown
    );
    assert_eq!(ocr_calls(), before, "no OCR under the new reader");

    // ocr-v1 on a first sighting: deferred, and no signal
    let (img, rgb) = title_mass_frame();
    let mut st = FrameStability::default();
    assert_eq!(
        detect_outcome_signal_polled(
            &img,
            &rgb,
            &mut st,
            FinalScoreSource::Ocr,
            &Titles::Ocr,
            &ResultWords::ocr()
        ),
        None
    );
}

/// The Tab frame's text fallback under `reader = "new"` ends in the
/// words-v1 end title, not the end-title OCR.
#[test]
fn the_tab_text_fallback_takes_the_end_title_from_words() {
    let (img, _) = dark_frame();
    assert_eq!(
        detect_outcome_text_with(&img, NEW, &words(Some(Outcome::Victory), false)),
        MatchOutcome::Victory
    );
    for titles in unsure_words() {
        assert_eq!(
            detect_outcome_text_with(&img, NEW, &titles),
            MatchOutcome::Unknown,
            "{titles:?}"
        );
    }
}

/// `reader = "new"`: a sure end title is the wake. An unsure one is not,
/// and the title card's phrase OCR never runs.
#[test]
fn a_sure_end_title_wakes_under_the_new_reader() {
    let (img, rgb) = dark_frame();
    for o in [Outcome::Victory, Outcome::Defeat, Outcome::Draw] {
        assert!(detect_end_reel_with(&img, &rgb, &words(Some(o), false)));
    }
    for titles in unsure_words() {
        assert!(!detect_end_reel_with(&img, &rgb, &titles), "{titles:?}");
    }
    assert!(!detect_end_reel_with(&img, &rgb, &Titles::Ocr));
    assert!(!detect_end_reel(&img, &rgb));

    // ocr-v1 reads the phrase behind the title card's pixel gate
    let (img, rgb) = title_card_frame();
    let before = ocr_calls();
    assert!(!detect_end_reel(&img, &rgb), "bars spell no phrase");
    assert_eq!(ocr_calls() - before, 1, "ocr-v1 ran the phrase OCR");
    // the new reader does not, and bars are no wake
    let before = ocr_calls();
    for titles in unsure_words() {
        assert!(!detect_end_reel_with(&img, &rgb, &titles), "{titles:?}");
    }
    assert!(detect_end_reel_with(
        &img,
        &rgb,
        &words(Some(Outcome::Victory), false)
    ));
    assert_eq!(ocr_calls(), before, "no OCR under the new reader");
}

/// The pixel paths wake the same under both readers: the letterbox
/// still wakes with no end title on the frame.
#[test]
fn the_letterbox_wakes_under_both_readers() {
    let reel = letterbox_footage_frame();
    let img = DynamicImage::ImageRgb8(reel.clone());
    assert!(detect_end_reel_with(&img, &reel, &Titles::Ocr));
    for titles in unsure_words() {
        assert!(detect_end_reel_with(&img, &reel, &titles), "{titles:?}");
    }
}

/// So does the nameplate (F263-7): it is the only path that wakes on
/// this frame, and under the new reader it wakes with no end title and
/// no OCR.
#[test]
fn the_nameplate_wakes_under_both_readers() {
    let rgb = nameplate_potg_frame();
    let img = DynamicImage::ImageRgb8(rgb.clone());
    assert!(!super::super::match_start::detect_ban_screen(&img, &rgb));
    assert!(!end_reel_letterbox(&rgb) && read_end_reel_nameplate(&rgb));
    assert!(detect_end_reel_with(&img, &rgb, &Titles::Ocr));
    let before = ocr_calls();
    for titles in unsure_words() {
        assert!(detect_end_reel_with(&img, &rgb, &titles), "{titles:?}");
    }
    assert_eq!(ocr_calls(), before, "no OCR under the new reader");
}

#[test]
fn end_reel_title_match_accepts_potg_phrases() {
    assert!(end_reel_title_match("PLAY OF THE GAME"));
    assert!(end_reel_title_match("YOUR PLAY OF THE GAME"));
    assert!(end_reel_title_match("PLAYOFTHEGAME"));
    assert!(end_reel_title_match("HIGHLIGHT INTRO"));
    assert!(end_reel_title_match("P1AY OF THE GAME")); // letters → PAYOFTHEGAME, lev 1
    assert!(end_reel_title_match("PLAY OF THE GARIE"));
    assert!(!end_reel_title_match("VICTORY"));
    assert!(!end_reel_title_match("DEFEAT"));
    assert!(!end_reel_title_match("PLAY"));
    assert!(!end_reel_title_match("GAME"));
    assert!(!end_reel_title_match("HIGHLIGHTS"));
    assert!(!end_reel_title_match("HELLO"));
    assert!(!end_reel_title_match(""));
}

fn letterbox_bars(img: &mut RgbImage) {
    // 7% of 360 = 25.2 → paint 30px bars so the 70/1000 band is black.
    for y in 0..30 {
        for x in 0..640 {
            img.put_pixel(x, y, Rgb([8, 8, 8]));
            img.put_pixel(x, 359 - y, Rgb([8, 8, 8]));
        }
    }
}

/// Flat mid band — loading / `ENTERING GAME` chrome, not footage.
fn letterbox_flat_frame(mid: [u8; 3]) -> RgbImage {
    let mut img = RgbImage::from_pixel(640, 360, Rgb(mid));
    letterbox_bars(&mut img);
    img
}

/// Letterboxed highlight-style mid: spatial color spread so the flat-UI
/// veto does not fire.
fn letterbox_footage_frame() -> RgbImage {
    let mut img = RgbImage::from_pixel(640, 360, Rgb([80, 90, 100]));
    for y in 30u32..330 {
        for x in 0u32..640 {
            let n = ((x.wrapping_mul(37) ^ y.wrapping_mul(17)) % 50) as u8;
            let warm = (x / 80) % 3 == 0;
            let pix = if warm {
                Rgb([110 + n / 2, 70 + n / 3, 50 + n / 4])
            } else {
                Rgb([50 + n / 3, 90 + n / 2, 80 + n / 3])
            };
            img.put_pixel(x, y, pix);
        }
    }
    // Brighter "character" blob so one bin cannot dominate.
    for y in 90..250 {
        for x in 280..480 {
            img.put_pixel(x, y, Rgb([160, 120, 90]));
        }
    }
    letterbox_bars(&mut img);
    img
}

fn entering_game_frame() -> RgbImage {
    // FrozenTear 013904 / 223050: ~38% black bars + a ~24% navy strip.
    // A vertical navy gradient splits 16-wide bins (old footage
    // check) but the shoulders at y 15–23% / 77–85% stay black.
    let mut img = RgbImage::from_pixel(640, 360, Rgb([0, 0, 0]));
    for y in 136..224 {
        let t = (y - 136) as u8;
        let navy = Rgb([20 + t / 4, 28 + t / 5, 42 + t / 3]);
        for x in 0..640 {
            img.put_pixel(x, y, navy);
        }
    }
    // Centered white "ENTERING GAME" glyphs.
    for y in 168..192 {
        for x in 200..440 {
            if x % 7 < 3 {
                img.put_pixel(x, y, Rgb([240, 240, 240]));
            }
        }
    }
    // Thin HUD line in the top 7% band (MangoHud / FPS overlay).
    for x in 8..400 {
        img.put_pixel(x, 4, Rgb([220, 220, 220]));
    }
    img
}

fn tab_scoreboard_frame() -> RgbImage {
    let mut img = RgbImage::from_pixel(640, 360, Rgb([0, 0, 0]));
    // FrozenTear accepted_020455: one magenta team *slab* over one mustard
    // slab (no dark row gaps). Group-count of 3+ misses this.
    for y in 12..165 {
        for x in 24..620 {
            img.put_pixel(x, y, Rgb([90, 40, 140]));
        }
    }
    for y in 185..348 {
        for x in 24..620 {
            img.put_pixel(x, y, Rgb([188, 146, 0]));
        }
    }
    // White names / stats on both teams (nameplate white gate).
    for y in [40, 70, 100, 130, 210, 240, 270, 300] {
        for x in 80..240 {
            if x % 5 < 2 {
                img.put_pixel(x, y, Rgb([235, 235, 235]));
            }
        }
    }
    img
}

#[test]
fn end_reel_letterbox_needs_dark_bars_and_lit_footage() {
    let reel = letterbox_footage_frame();
    assert!(end_reel_letterbox(&reel), "letterboxed reel should wake");
    assert!(
        detect_end_reel(&DynamicImage::ImageRgb8(reel.clone()), &reel),
        "letterbox path should wake without title OCR"
    );

    let black = RgbImage::from_pixel(640, 360, Rgb([0, 0, 0]));
    assert!(
        !end_reel_letterbox(&black),
        "fade-to-black is not a reel (middle is dark)"
    );
    assert!(!detect_end_reel(
        &DynamicImage::ImageRgb8(black.clone()),
        &black
    ));

    let gray = RgbImage::from_pixel(640, 360, Rgb([80, 80, 80]));
    assert!(!end_reel_letterbox(&gray));

    let gold = RgbImage::from_pixel(640, 360, Rgb([230, 180, 20]));
    assert!(
        !end_reel_letterbox(&gold),
        "victory banner flood is not a letterbox"
    );

    let flat = letterbox_flat_frame([80, 90, 100]);
    assert!(
        !end_reel_letterbox(&flat),
        "uniform mid slab is loading chrome, not a reel"
    );
}

#[test]
fn entering_game_loading_does_not_wake() {
    let rgb = entering_game_frame();
    let img = DynamicImage::ImageRgb8(rgb.clone());
    assert!(
        !end_reel_letterbox(&rgb),
        "ENTERING GAME dark bars + navy slab must not letterbox-wake"
    );
    assert!(
        !detect_end_reel(&img, &rgb),
        "loading transition must not set end-reel wake"
    );
    assert!(!super::super::match_start::detect_ban_screen(&img, &rgb));
}

#[test]
fn ban_heroes_layout_does_not_wake_end_reel() {
    // Same layout helper as match_start's hook test — keep the wake
    // veto in this module so a letterboxed hero grid cannot sneak
    // through detect_end_reel.
    use super::super::match_start::detect_ban_screen;
    let mut img = RgbImage::from_pixel(640, 360, Rgb([12, 16, 28]));
    for y in 0..40 {
        for x in 0..640 {
            img.put_pixel(x, y, Rgb([8, 8, 12]));
            img.put_pixel(x, 359 - y, Rgb([8, 8, 12]));
        }
    }
    for row in 0..3 {
        for col in 0..8 {
            let x0 = 40 + col * 72;
            let y0 = 70 + row * 70;
            let color = Rgb([
                40 + (col as u8) * 24,
                80 + (row as u8) * 40,
                160u8.saturating_sub(col as u8 * 12),
            ]);
            for y in y0..y0 + 50 {
                for x in x0..x0 + 60 {
                    if x < 640 && y < 360 {
                        img.put_pixel(x, y, color);
                    }
                }
            }
        }
    }
    for y in 166..196 {
        for x in 0..640 {
            img.put_pixel(x, y, Rgb([180, 28, 32]));
        }
    }
    for y in 8..22 {
        for x in 250..390 {
            img.put_pixel(x, y, Rgb([200, 24, 24]));
        }
    }
    let dyn_img = DynamicImage::ImageRgb8(img.clone());
    assert!(
        detect_ban_screen(&dyn_img, &img),
        "Ban Heroes must still be a first-class hook"
    );
    assert!(
        !detect_end_reel(&dyn_img, &img),
        "ban UI must not set end_reel_wake_until"
    );
    for titles in unsure_words() {
        assert!(
            !detect_end_reel_with(&dyn_img, &img, &titles),
            "nor under the new reader: {titles:?}"
        );
    }
    // the ban gate also fires on real end titles: a sure end-title read
    // is the stronger evidence, and wakes
    assert!(detect_end_reel_with(
        &dyn_img,
        &img,
        &words(Some(Outcome::Defeat), false)
    ));

    // The frame above wakes through no path even without the veto
    // (F263-7). This one would: highlight-style letterbox footage under
    // the red vote bar. The veto holds it under both readers.
    let mut reel = letterbox_footage_frame();
    for y in 158..202 {
        for x in 0..640 {
            reel.put_pixel(x, y, Rgb([180, 28, 32]));
        }
    }
    let reel_img = DynamicImage::ImageRgb8(reel.clone());
    assert!(end_reel_letterbox(&reel), "the letterbox path would wake");
    assert!(detect_ban_screen(&reel_img, &reel));
    assert!(!detect_end_reel(&reel_img, &reel));
    assert!(!detect_end_reel_with(&reel_img, &reel, &Titles::Ocr));
    for titles in unsure_words() {
        assert!(
            !detect_end_reel_with(&reel_img, &reel, &titles),
            "nor under the new reader: {titles:?}"
        );
    }
    assert!(detect_end_reel_with(
        &reel_img,
        &reel,
        &words(Some(Outcome::Victory), false)
    ));
}

#[test]
fn tab_scoreboard_does_not_nameplate_wake() {
    let rgb = tab_scoreboard_frame();
    let img = DynamicImage::ImageRgb8(rgb.clone());
    assert!(
        !nameplate_potg_signal(&rgb),
        "gold team rows are not a POTG battletag stack"
    );
    assert!(!detect_end_reel(&img, &rgb), "Tab scoreboard must not wake");
}

#[test]
fn tab_scoreboard_muted_mustard_does_not_nameplate_wake() {
    // Geometry that passed 486219f: white names in the title ROI +
    // enough neon-gold specks for the cheap gate, but the mustard
    // slab fails is_potg_orange so hot-strips / flood / magenta miss.
    let rgb = tab_scoreboard_muted_mustard_frame();
    let img = DynamicImage::ImageRgb8(rgb.clone());
    let orange = playfield_hit_ratio(
        &rgb,
        NAMEPLATE_ORANGE_X_PM,
        NAMEPLATE_ORANGE_Y_PM,
        NAMEPLATE_ORANGE_W_PM,
        NAMEPLATE_ORANGE_H_PM,
        is_potg_orange,
    );
    let white = playfield_hit_ratio(
        &rgb,
        NAMEPLATE_WHITE_X_PM,
        NAMEPLATE_WHITE_Y_PM,
        NAMEPLATE_WHITE_W_PM,
        NAMEPLATE_WHITE_H_PM,
        is_title_white,
    );
    assert!(
        (NAMEPLATE_ORANGE_MIN..=NAMEPLATE_ORANGE_MAX).contains(&orange)
            && white >= NAMEPLATE_WHITE_MIN,
        "fixture must still look like a nameplate to the cheap gate (orange={orange:.4} white={white:.4})"
    );
    assert!(
        nameplate_gold_hot_strips(&rgb) < 8,
        "486219f hot-strip veto must miss this slab"
    );
    assert!(
        nameplate_gold_row_groups(&rgb) < 3,
        "gapped-row veto must miss a solid mustard block"
    );
    assert!(
        !nameplate_potg_signal(&rgb),
        "muted mustard team slab must not be a POTG battletag"
    );
    assert!(
        !detect_end_reel(&img, &rgb),
        "Tab scoreboard crop accepted_020455 must not wake"
    );
}

/// FrozenTear accepted_020455: tight Tab crop, burgundy team over a *muted*
/// mustard slab (human-bright, but most pixels fail `is_potg_orange`
/// val>160 / sat>140). White names sit on the gold. 486219f slab
/// vetoes used the strict orange predicate and missed this.
fn tab_scoreboard_muted_mustard_frame() -> RgbImage {
    let mut img = RgbImage::from_pixel(640, 360, Rgb([12, 10, 14]));
    // Header strip (E A D DMG H MIT).
    for y in 0..18 {
        for x in 0..640 {
            img.put_pixel(x, y, Rgb([168, 168, 172]));
        }
    }
    // Burgundy / maroon team — hue ~340, outside is_team_magenta 300±35.
    for y in 18..168 {
        let row = ((y - 18) / 25) as u8;
        let fill = Rgb([72 + row * 4, 22, 40 + row]);
        for x in 8..632 {
            img.put_pixel(x, y, fill);
        }
    }
    // Highlighted top row: pink/magenta border (FROZEN), not gold.
    for y in 20..44 {
        for x in 8..632 {
            if !(23..=41).contains(&y) || !(12..=628).contains(&x) {
                img.put_pixel(x, y, Rgb([220, 50, 150]));
            }
        }
    }
    // VS divider.
    for y in 168..184 {
        for x in 0..640 {
            img.put_pixel(x, y, Rgb([24, 24, 26]));
        }
    }
    for y in 172..180 {
        for x in 300..340 {
            img.put_pixel(x, y, Rgb([230, 230, 230]));
        }
    }
    // Mustard team slab. Base #9A7A12 fails val>160; sparse #C6A307
    // specks keep the nameplate orange ROI in [0.035, 0.28].
    for y in 184..358 {
        for x in 8..632 {
            img.put_pixel(x, y, Rgb([154, 122, 18]));
        }
    }
    // Gray hero portraits + speaker column.
    for row in 0..6 {
        let y0 = 188 + row * 28;
        for y in y0..y0 + 22 {
            for x in 16..40 {
                img.put_pixel(x, y, Rgb([96, 96, 100]));
            }
            for x in 600..624 {
                img.put_pixel(x, y, Rgb([110, 110, 114]));
            }
        }
    }
    // White names on gold. y 58–70% is the nameplate title ROI —
    // the first gold-team rows sit there on a real Tab crop.
    for y in 210..250 {
        if y % 6 > 2 {
            continue;
        }
        for x in 56..240 {
            if x % 4 < 2 {
                img.put_pixel(x, y, Rgb([240, 240, 240]));
            }
        }
    }
    for y in [268, 296, 324] {
        for x in 56..220 {
            if x % 5 < 2 {
                img.put_pixel(x, y, Rgb([240, 240, 240]));
            }
        }
    }
    // Brighter gold patches in the battletag ROI so strict orange
    // still enters the wake window (the rest of the slab does not).
    for y in 240..292 {
        for x in 48..260 {
            if (x + y) % 7 == 0 {
                img.put_pixel(x, y, Rgb([198, 163, 7]));
            }
        }
    }
    img
}

#[test]
fn cinematic_title_band_needs_dark_field_and_glyphs() {
    let dark = DynamicImage::ImageRgb8(RgbImage::from_pixel(80, 40, Rgb([10, 10, 10])));
    assert!(
        !cinematic_title_band(&dark),
        "dark with no glyphs should skip OCR"
    );
    let sky = DynamicImage::ImageRgb8(RgbImage::from_pixel(80, 40, Rgb([180, 200, 220])));
    assert!(
        !cinematic_title_band(&sky),
        "bright mid-fight sky should skip OCR"
    );
    let mut card = RgbImage::from_pixel(80, 40, Rgb([12, 12, 12]));
    for y in 10..30 {
        for x in 10..70 {
            if x % 3 == 0 {
                card.put_pixel(x, y, Rgb([220, 220, 220]));
            }
        }
    }
    assert!(cinematic_title_band(&DynamicImage::ImageRgb8(card)));
}

/// First #83 mid-band orange ROI — the real battletag sits lower.
const NAMEPLATE_ORANGE_OLD_X_PM: u32 = 30;
const NAMEPLATE_ORANGE_OLD_Y_PM: u32 = 340;
const NAMEPLATE_ORANGE_OLD_W_PM: u32 = 460;
const NAMEPLATE_ORANGE_OLD_H_PM: u32 = 240;

/// Lit indoor frame matching known-potg-003708 geometry: no letterbox,
/// toast lights the cinematic band, gold battletag in the lower-left
/// (y ~70%, rgb 255,221,0), white PLAY OF THE GAME just above it.
fn nameplate_potg_frame() -> RgbImage {
    let mut img = RgbImage::from_pixel(640, 360, Rgb([110, 95, 85]));
    for y in 0..360 {
        for x in 280..640 {
            img.put_pixel(x, y, Rgb([150, 120, 100]));
        }
    }
    // Yellow endorsement toast in the cinematic band (y ~8–30%).
    for y in 40..70 {
        for x in 20..280 {
            img.put_pixel(x, y, Rgb([240, 210, 40]));
        }
    }
    // Chat-like bright line at the top-left.
    for y in 8..14 {
        for x in 10..180 {
            img.put_pixel(x, y, Rgb([255, 140, 40]));
        }
    }
    // White title glyphs at y ~64–67% (real PLAY OF THE GAME).
    // Thick strokes so stride-2 sampling still hits.
    for y in 230..242 {
        for x in 40..260 {
            if x % 8 < 3 {
                img.put_pixel(x, y, Rgb([235, 235, 235]));
            }
        }
    }
    // Gold battletag at y ~70–75%, sample from the real frame.
    // Glyph-like gaps: a solid plate trips NAMEPLATE_ORANGE_MAX
    // (scoreboard row fills). Real known-potg-003708 is ~0.0685.
    for y in 252..272 {
        for x in 40..280 {
            if x % 6 < 3 {
                img.put_pixel(x, y, Rgb([255, 221, 0]));
            }
        }
    }
    img
}

#[test]
fn nameplate_potg_signal_fires_when_cinematic_band_does_not() {
    let frame = nameplate_potg_frame();
    let img = DynamicImage::ImageRgb8(frame.clone());
    assert!(
        !end_reel_letterbox(&frame),
        "nameplate card is not letterboxed"
    );
    let crop = end_reel_cinematic_crop(&img).expect("cinematic crop");
    assert!(
        !cinematic_title_band(&crop),
        "toast + hero light the cinematic band: old gate must miss"
    );
    let old_orange = playfield_hit_ratio(
        &frame,
        NAMEPLATE_ORANGE_OLD_X_PM,
        NAMEPLATE_ORANGE_OLD_Y_PM,
        NAMEPLATE_ORANGE_OLD_W_PM,
        NAMEPLATE_ORANGE_OLD_H_PM,
        is_potg_orange,
    );
    assert!(
        old_orange < NAMEPLATE_ORANGE_MIN,
        "mid-band ROI from first #83 must miss the lower-third name (old_orange={old_orange:.4})"
    );
    let orange = playfield_hit_ratio(
        &frame,
        NAMEPLATE_ORANGE_X_PM,
        NAMEPLATE_ORANGE_Y_PM,
        NAMEPLATE_ORANGE_W_PM,
        NAMEPLATE_ORANGE_H_PM,
        is_potg_orange,
    );
    let white = playfield_hit_ratio(
        &frame,
        NAMEPLATE_WHITE_X_PM,
        NAMEPLATE_WHITE_Y_PM,
        NAMEPLATE_WHITE_W_PM,
        NAMEPLATE_WHITE_H_PM,
        is_title_white,
    );
    assert!(
        nameplate_potg_signal(&frame),
        "gold name + white title must open the nameplate path (orange={orange:.4} white={white:.4})"
    );
    assert!(
        detect_end_reel(&img, &frame),
        "signal-true nameplate must wake even when title OCR would fail"
    );
}

#[test]
fn nameplate_potg_signal_rejects_combat_and_partial_cues() {
    let sky = RgbImage::from_pixel(640, 360, Rgb([180, 200, 220]));
    assert!(!nameplate_potg_signal(&sky), "mid-fight sky");
    assert!(!detect_end_reel(
        &DynamicImage::ImageRgb8(sky.clone()),
        &sky
    ));

    let mut orange_only = RgbImage::from_pixel(640, 360, Rgb([40, 40, 40]));
    for y in 252..272 {
        for x in 40..280 {
            if x % 6 < 3 {
                orange_only.put_pixel(x, y, Rgb([255, 221, 0]));
            }
        }
    }
    assert!(
        !nameplate_potg_signal(&orange_only),
        "gold blob without white title glyphs"
    );

    let mut white_only = RgbImage::from_pixel(640, 360, Rgb([40, 40, 40]));
    for y in 230..242 {
        for x in 40..260 {
            if x % 8 < 3 {
                white_only.put_pixel(x, y, Rgb([235, 235, 235]));
            }
        }
    }
    assert!(
        !nameplate_potg_signal(&white_only),
        "white HUD without gold name"
    );

    let gold = RgbImage::from_pixel(640, 360, Rgb([230, 180, 20]));
    assert!(
        !nameplate_potg_signal(&gold),
        "victory gold flood is not a POTG nameplate"
    );

    let red = RgbImage::from_pixel(640, 360, Rgb([200, 30, 30]));
    assert!(!nameplate_potg_signal(&red), "defeat red flood");
}

/// The nameplate step's answer is the pixel gate alone: true on the
/// nameplate card, false off it. Deleting its discarded phrase OCR (#240)
/// must not change either.
#[test]
fn nameplate_step_returns_the_pixel_gate() {
    assert!(read_end_reel_nameplate(&nameplate_potg_frame()));
    for (frame, what) in [
        (RgbImage::from_pixel(640, 360, Rgb([180, 200, 220])), "sky"),
        (RgbImage::from_pixel(640, 360, Rgb([230, 180, 20])), "gold"),
        (RgbImage::from_pixel(640, 360, Rgb([200, 30, 30])), "red"),
        (RgbImage::from_pixel(640, 360, Rgb([0, 0, 0])), "black"),
    ] {
        assert!(!read_end_reel_nameplate(&frame), "{what}");
    }
}

// ------------------------------------------------ FINAL SCORE line (O3)

use crate::shadow::result::FinalScoreRead;

/// A 1440p frame, with a white bar on the FINAL SCORE line when `white`
/// (enough for P4's white-pixel gate).
fn scoreline_frame(white: bool) -> (DynamicImage, RgbImage) {
    let mut rgb = RgbImage::from_pixel(2560, 1440, Rgb([30, 34, 60]));
    if white {
        for y in 1080..1110 {
            for x in 1080..1480 {
                rgb.put_pixel(x, y, Rgb([245, 245, 245]));
            }
        }
    }
    (DynamicImage::ImageRgb8(rgb.clone()), rgb)
}

fn never(_: &RgbImage) -> Option<FinalScoreRead> {
    panic!("the words-v1 template was read")
}

/// `reader = "ocr-v1"` reads the line with OCR only, through every
/// public entry point: the template is never consulted.
#[test]
fn ocr_v1_never_reads_the_final_score_template() {
    assert_eq!(FinalScoreSource::default(), FinalScoreSource::Ocr);
    assert_eq!(FinalScoreSource::for_reader(false), FinalScoreSource::Ocr);
    assert_eq!(FinalScoreSource::for_reader(true), FinalScoreSource::Words);
    let (img, rgb) = scoreline_frame(true);
    let _ = scoreline_looks_present_with(&img, &rgb, None, FinalScoreSource::Ocr, never);
}

/// Under `reader = "new"` the template's read decides: only a sure read
/// is a signal.
#[test]
fn words_v1_decides_the_final_score_line() {
    let (img, rgb) = scoreline_frame(true);
    for (read, want) in [
        (FinalScoreRead::Sure(0.9), true),
        (FinalScoreRead::Suspect(0.65), false),
        (FinalScoreRead::Absent(0.2), false),
    ] {
        let got =
            scoreline_looks_present_with(&img, &rgb, None, FinalScoreSource::Words, |_| Some(read));
        assert_eq!(got, want, "{read:?}");
    }
    // no template loaded: the OCR read, which finds no words on a bar
    assert!(!scoreline_looks_present_with(
        &img,
        &rgb,
        None,
        FinalScoreSource::Words,
        |_| None
    ));
}

/// P4's white-pixel gate and crop stability still come first under
/// `reader = "new"`.
#[test]
fn the_final_score_gates_hold_under_words_v1() {
    let (img, rgb) = scoreline_frame(false);
    assert!(!scoreline_looks_present_with(
        &img,
        &rgb,
        None,
        FinalScoreSource::Words,
        never
    ));
    let (img, rgb) = scoreline_frame(true);
    let mut st = FrameStability::default();
    assert!(
        !scoreline_looks_present_with(&img, &rgb, Some(&mut st), FinalScoreSource::Words, never),
        "a first sighting is never stable"
    );
    assert!(scoreline_looks_present_with(
        &img,
        &rgb,
        Some(&mut st),
        FinalScoreSource::Words,
        |_| Some(FinalScoreRead::Sure(0.9))
    ));
}

/// A 1440p frame with FINAL SCORE in white on the P4 line.
fn final_score_frame() -> (DynamicImage, RgbImage) {
    let (_, mut rgb) = scoreline_frame(false);
    let label = crate::ocr::map_label_fixtures::draw_label("FINAL SCORE", 250).to_rgb8();
    // P4's crop is x 768..1792, y 1051..1166 at 1440p.
    for (x, y, px) in label.enumerate_pixels().filter(|(x, ..)| *x < 520) {
        rgb.put_pixel(1270 + x, 1066 + y, *px);
    }
    (DynamicImage::ImageRgb8(rgb.clone()), rgb)
}

/// With OCR on, `reader = "new"` without a template reads the line with
/// OCR and gets what ocr-v1 gets (F257-10: the fallback in its positive
/// direction).
#[test]
fn without_a_template_the_ocr_fallback_reads_the_line_while_ocr_is_on() {
    let (img, rgb) = final_score_frame();
    assert!(
        scoreline_looks_present_with(&img, &rgb, None, FinalScoreSource::Ocr, never),
        "ocr-v1 reads FINAL SCORE on the synthetic line"
    );
    assert!(scoreline_looks_present_with(
        &img,
        &rgb,
        None,
        FinalScoreSource::Words,
        |_| None
    ));
}

/// With classic OCR off, a result word the loaded templates cannot name
/// is held: no outcome, and the OCR read behind it is not asked for.
/// With OCR on the same frame falls back to OCR, as #270 left it.
#[test]
fn with_ocr_off_a_result_word_the_templates_cannot_name_is_held() {
    // White bars where the accolade screen and the rank screen print
    // the word: both title crops pass their gate.
    let mut rgb = RgbImage::from_pixel(1920, 1080, Rgb([16, 25, 50]));
    for (x0, y0, x1, y1) in [(40, 45, 400, 95), (40, 165, 400, 235)] {
        for y in y0..y1 {
            for x in x0..x1 {
                rgb.put_pixel(x, y, Rgb([250, 250, 250]));
            }
        }
    }
    let img = DynamicImage::ImageRgb8(rgb.clone());
    // The new reader with no result templates loaded.
    let no_templates = || {
        ResultWords::with_reader(WordsReader {
            screens: ResultScreens::default,
            read: |_| None,
        })
    };
    let read = |words: &ResultWords| {
        detect_outcome_signal_inner(
            &img,
            &rgb,
            None,
            FinalScoreSource::Words,
            &Titles::Held,
            words,
        )
    };

    // OCR on: each screen's word is read with OCR.
    let before = ocr_calls();
    let _ = read(&no_templates());
    assert_eq!(ocr_calls() - before, 2, "both screens reach OCR");

    let off = crate::ocr::gate::scoped(crate::ocr::gate::OcrMode::Off);
    let before = ocr_calls();
    assert_eq!(read(&no_templates()), None);
    assert_eq!(ocr_calls(), before, "no Tesseract read");
    assert_eq!(
        off.blocked(),
        [],
        "the result word was not asked of OCR under the new reader"
    );
    // ocr-v1's own read is not held by this: it is stopped at the gate.
    assert_eq!(read(&ResultWords::ocr()), None);
    assert_eq!(off.blocked_total(), 2);
}

/// With OCR off and no end-title templates loaded the titles are held:
/// the end title is no outcome, the title card is no wake, and nothing
/// is sent to OCR, not even to the gate. With OCR on the same frames
/// fall back to OCR, as on main.
#[test]
fn with_ocr_off_and_no_end_title_templates_both_titles_are_held() {
    let none = |_: &RgbImage| None::<ResultRead>;
    // a frame with the end title's colour mass, and a title card
    let mut title = RgbImage::from_pixel(640, 360, Rgb([10, 10, 10]));
    for y in 126..180 {
        for x in 192..448 {
            title.put_pixel(x, y, Rgb([220, 40, 220]));
        }
    }
    let title_img = DynamicImage::ImageRgb8(title.clone());
    let mut card = RgbImage::from_pixel(640, 360, Rgb([12, 12, 12]));
    for y in 36..100 {
        for x in (80..560).step_by(3) {
            card.put_pixel(x, y, Rgb([220, 220, 220]));
        }
    }
    let card_img = DynamicImage::ImageRgb8(card.clone());

    // OCR on: the fallback, and both frames reach Tesseract.
    assert_eq!(Titles::for_rgb_with(true, &title, none), Titles::Ocr);
    let before = ocr_calls();
    let _ = read_end_title(
        &title_img,
        &title,
        None,
        FinalScoreSource::Words,
        &Titles::Ocr,
    );
    assert_eq!(ocr_calls() - before, 1, "the end title frame reaches OCR");
    let before = ocr_calls();
    let _ = detect_end_reel_with(&card_img, &card, &Titles::Ocr);
    assert_eq!(ocr_calls() - before, 1, "the title card reaches OCR");

    let off = crate::ocr::gate::scoped(crate::ocr::gate::OcrMode::Off);
    let before = ocr_calls();
    for (img, rgb) in [(&title_img, &title), (&card_img, &card)] {
        assert_eq!(Titles::for_rgb_with(true, rgb, none), Titles::Held);
        assert_eq!(Titles::for_frame_with(true, img, none), Titles::Held);
        let held = Titles::Held;
        assert_eq!(held.sure_end_title(), None);
        assert_eq!(
            read_end_title(img, rgb, None, FinalScoreSource::Words, &held),
            MatchOutcome::Unknown
        );
        assert_eq!(
            detect_outcome_text_with(img, FinalScoreSource::Words, &held),
            MatchOutcome::Unknown
        );
        let mut st = FrameStability::default();
        for _ in 0..2 {
            let _ = detect_outcome_signal_polled(
                img,
                rgb,
                &mut st,
                FinalScoreSource::Words,
                &held,
                &ResultWords::ocr(),
            );
        }
        assert!(!detect_end_reel_with(img, rgb, &held), "no wake");
    }
    // Nothing of the end title or the title card went to the gate. (The
    // result word reads of the polled signal are another read's.)
    assert_eq!(ocr_calls(), before, "no Tesseract read");
    assert!(
        off.blocked()
            .iter()
            .all(|b| b.site.entry != "recognize_prepared_lang"),
        "the end title was not asked for: {:?}",
        off.blocked()
    );
    // With the templates loaded the words read decides, also with OCR off.
    let read = read_of(Some(Outcome::Defeat), false);
    assert_eq!(
        Titles::for_rgb_with(true, &title, |_| Some(read.clone())),
        Titles::Words(read)
    );
    // ocr-v1 is not held by this: it reads with OCR, and is blocked.
    assert_eq!(Titles::for_rgb_with(false, &title, none), Titles::Ocr);
}

/// With OCR off there is no fallback: no template means the line is not
/// read, and the OCR entry point is not even called.
#[test]
fn with_ocr_off_a_missing_template_holds_and_never_calls_ocr() {
    let (img, rgb) = final_score_frame();
    let off = crate::ocr::gate::scoped(crate::ocr::gate::OcrMode::Off);
    assert!(!scoreline_looks_present_with(
        &img,
        &rgb,
        None,
        FinalScoreSource::Words,
        |_| None
    ));
    assert_eq!(
        off.blocked(),
        [],
        "the fallback is not attempted, so nothing reaches the gate"
    );
    // a loaded template still decides, with OCR off
    assert!(scoreline_looks_present_with(
        &img,
        &rgb,
        None,
        FinalScoreSource::Words,
        |_| Some(FinalScoreRead::Sure(0.9))
    ));
    assert_eq!(off.blocked_total(), 0);
    // ocr-v1's own read is blocked at the gate and is not a signal
    assert!(!scoreline_looks_present_with(
        &img,
        &rgb,
        None,
        FinalScoreSource::Ocr,
        never
    ));
    assert_eq!(off.blocked_total(), 1);
}

/// Under `reader = "new"`, a P4 that falls back to OCR (no template) is
/// logged once per process (F257-1). Once this test's fallback has run,
/// the one log line is spent.
#[test]
fn a_final_score_ocr_fallback_is_logged() {
    let (img, rgb) = scoreline_frame(true);
    let _ = scoreline_looks_present_with(&img, &rgb, None, FinalScoreSource::Words, |_| None);
    assert!(
        !warn_final_score_ocr_fallback(),
        "the fallback logged already"
    );
}

// ------------- accolade and rank-screen result words without OCR (O11)

use crate::shadow::result::{ACCOLADE, RANK_SCREEN};
use std::cell::{Cell, RefCell};

const BOTH: ResultScreens = ResultScreens {
    accolade: true,
    rank_screen: true,
};
const NO_SCREENS: ResultScreens = ResultScreens {
    accolade: false,
    rank_screen: false,
};

thread_local! {
    /// What the test words pack can name, what it reads on every
    /// frame, and how many frames it was asked to read.
    static PACK_SCREENS: Cell<ResultScreens> = const { Cell::new(NO_SCREENS) };
    static PACK_READ: RefCell<Option<ScreenRead>> = const { RefCell::new(None) };
    static PACK_READS: Cell<usize> = const { Cell::new(0) };
}

fn pack_reads() -> usize {
    PACK_READS.get()
}

/// `reader = "new"` with a words pack that can name `screens` and reads
/// `read` on every frame it is asked to read (`None`: no pack loaded).
fn pack_words(screens: ResultScreens, read: Option<ScreenRead>) -> ResultWords {
    PACK_SCREENS.set(screens);
    PACK_READ.set(read);
    ResultWords::with_reader(WordsReader {
        screens: || PACK_SCREENS.get(),
        read: |_| {
            PACK_READS.set(PACK_READS.get() + 1);
            PACK_READ.with_borrow(|r| r.clone())
        },
    })
}

/// `reader = "new"` for a frame whose accolade and rank crops stay
/// behind their gates: the words pack is not even asked what it loaded.
fn unread_words() -> ResultWords {
    ResultWords::with_reader(WordsReader {
        screens: || panic!("the words pack was asked for its screens"),
        read: |_| panic!("the words pack read a frame behind its gates"),
    })
}

/// A read of a pack that names both screens: `outcome` found on layout
/// `source`, sure unless `suspect`.
fn screen_read(outcome: Option<Outcome>, source: &str, suspect: bool) -> ScreenRead {
    ScreenRead {
        read: Some(ResultRead {
            outcome,
            source: outcome.map(|_| source.to_string()),
            score: if outcome.is_some() { 0.9 } else { 0.2 },
            margin: 0.4,
            suspect,
        }),
        screens: BOTH,
    }
}

/// The first look found no result-screen word: the frame was not read
/// in full.
fn unread_frame() -> ScreenRead {
    ScreenRead {
        read: None,
        screens: BOTH,
    }
}

fn sure(outcome: Outcome, source: &str) -> Option<ScreenRead> {
    Some(screen_read(Some(outcome), source, false))
}

/// A dark frame with bright bars in the accolade title crop and / or in
/// the rank-screen crop, shifted right by `dx`. A crop with bars passes
/// its lit gate, so ocr-v1 runs Tesseract on it once it held still; the
/// bars spell no word.
fn result_frame(accolade: bool, rank: bool, dx: u32) -> (DynamicImage, RgbImage) {
    let mut rgb = RgbImage::from_pixel(640, 360, Rgb([10, 10, 10]));
    let mut bars = |rows: std::ops::Range<u32>| {
        for y in rows {
            for x in (12 + dx)..(110 + dx) {
                if (x - dx) % 6 < 3 {
                    rgb.put_pixel(x, y, Rgb([235, 235, 235]));
                }
            }
        }
    };
    if accolade {
        bars(16..30);
    }
    if rank {
        bars(58..76);
    }
    (DynamicImage::ImageRgb8(rgb.clone()), rgb)
}

/// `reader = "new"` with end-title templates loaded and no end title on
/// the frame.
fn no_title() -> Titles {
    Titles::Words(read_of(None, true))
}

/// One poll tick under `reader = "new"`, with no end title on the frame.
fn poll_new(
    frame: &(DynamicImage, RgbImage),
    st: &mut FrameStability,
    words: &ResultWords,
) -> Option<(MatchOutcome, OutcomeSource)> {
    detect_outcome_signal_polled(&frame.0, &frame.1, st, NEW, &no_title(), words)
}

/// One poll tick under `reader = "ocr-v1"`.
fn poll_ocr(
    frame: &(DynamicImage, RgbImage),
    st: &mut FrameStability,
) -> Option<(MatchOutcome, OutcomeSource)> {
    detect_outcome_signal_polled(
        &frame.0,
        &frame.1,
        st,
        FinalScoreSource::Ocr,
        &Titles::Ocr,
        &ResultWords::ocr(),
    )
}

/// A stability history that has seen `frame` once: the next tick on the
/// same frame is past the stability wait.
fn seen(frame: &(DynamicImage, RgbImage)) -> FrameStability {
    let mut st = FrameStability::default();
    assert_eq!(poll_new(frame, &mut st, &unread_words()), None);
    st
}

const WORDS: [(Outcome, MatchOutcome); 3] = [
    (Outcome::Victory, MatchOutcome::Victory),
    (Outcome::Defeat, MatchOutcome::Defeat),
    (Outcome::Draw, MatchOutcome::Draw),
];

/// `reader = "new"`: on a lit title crop that held still, a sure
/// words-v1 read found on that screen is the signal, with the source
/// ocr-v1 gives that screen. Tesseract is not called, on frames where
/// ocr-v1 calls it. The read is kept for the caller.
#[test]
fn a_sure_words_read_is_the_result_word_signal() {
    for (frame, layout, source) in [
        (
            result_frame(true, false, 0),
            ACCOLADE,
            OutcomeSource::ResultWord,
        ),
        (
            result_frame(false, true, 0),
            RANK_SCREEN,
            OutcomeSource::RankScreen,
        ),
    ] {
        // ocr-v1 on this frame: one Tesseract call once the crop held still
        let mut st = FrameStability::default();
        let before = ocr_calls();
        assert_eq!(poll_ocr(&frame, &mut st), None);
        assert_eq!(ocr_calls(), before, "a first sighting is deferred");
        assert_eq!(poll_ocr(&frame, &mut st), None, "bars spell no word");
        assert_eq!(ocr_calls() - before, 1, "{layout}");

        for (word, want) in WORDS {
            let mut st = seen(&frame);
            let words = pack_words(BOTH, sure(word, layout));
            let (ocr_before, reads_before) = (ocr_calls(), pack_reads());
            assert_eq!(poll_new(&frame, &mut st, &words), Some((want, source)));
            assert_eq!(ocr_calls(), ocr_before, "no OCR under the new reader");
            assert_eq!(pack_reads() - reads_before, 1);
            let read = screen_read(Some(word), layout, false).read;
            assert_eq!(words.taken(), read.as_ref());
            assert_eq!(words.into_taken(), read);
        }
    }
}

/// A flagged read and a frame with no word are no signal, and nothing
/// falls back to OCR. A read that was made is still kept: it is the
/// frame's read.
#[test]
fn a_flagged_or_absent_words_read_is_no_signal_and_no_ocr() {
    let frame = result_frame(true, true, 0);
    for read in [
        screen_read(Some(Outcome::Victory), ACCOLADE, true),
        screen_read(Some(Outcome::Defeat), ACCOLADE, true),
        screen_read(Some(Outcome::Victory), RANK_SCREEN, true),
        screen_read(Some(Outcome::Defeat), RANK_SCREEN, true),
        screen_read(None, ACCOLADE, true),
        unread_frame(),
        // not a state the reader produces: a word with no layout
        ScreenRead {
            read: Some(ResultRead {
                source: None,
                ..screen_read(Some(Outcome::Victory), ACCOLADE, false)
                    .read
                    .unwrap()
            }),
            screens: BOTH,
        },
    ] {
        let mut st = seen(&frame);
        let words = pack_words(BOTH, Some(read.clone()));
        let before = ocr_calls();
        assert_eq!(poll_new(&frame, &mut st, &words), None, "{read:?}");
        assert_eq!(words.taken(), read.read.as_ref());
        // the Tab frame's read: no stability wait, the same answer
        let words = pack_words(BOTH, Some(read.clone()));
        assert_eq!(
            detect_outcome_with(&frame.0, NEW, &no_title(), &words),
            MatchOutcome::Unknown,
            "{read:?}"
        );
        assert_eq!(ocr_calls(), before, "no OCR under the new reader");
    }
}

/// A DRAW is a signal only when the read is sure. (The reader itself
/// names an unsure DRAW nothing at all, `shadow::result`.)
#[test]
fn a_draw_is_a_signal_only_when_sure() {
    let frame = result_frame(true, true, 0);
    for layout in [ACCOLADE, RANK_SCREEN] {
        let mut st = seen(&frame);
        let words = pack_words(BOTH, sure(Outcome::Draw, layout));
        assert_eq!(
            poll_new(&frame, &mut st, &words).map(|s| s.0),
            Some(MatchOutcome::Draw)
        );
        let mut st = seen(&frame);
        let flagged = Some(screen_read(Some(Outcome::Draw), layout, true));
        assert_eq!(poll_new(&frame, &mut st, &pack_words(BOTH, flagged)), None);
    }
}

/// A screen's word is a sure word found on that screen's layout (the id
/// or a variant of it). A word found on the other screen, on the end
/// title or on any other layout is not this screen's word.
#[test]
fn a_word_found_on_another_layout_is_not_this_screens_word() {
    let accolade = result_frame(true, false, 0);
    let rank = result_frame(false, true, 0);
    let both = result_frame(true, true, 0);
    let signal = |frame: &(DynamicImage, RgbImage), layout: &str| {
        let mut st = seen(frame);
        let words = pack_words(BOTH, sure(Outcome::Defeat, layout));
        poll_new(frame, &mut st, &words).map(|s| s.1)
    };
    // only the accolade crop is lit
    assert_eq!(signal(&accolade, ACCOLADE), Some(OutcomeSource::ResultWord));
    assert_eq!(
        signal(&accolade, "accolade_wide"),
        Some(OutcomeSource::ResultWord)
    );
    for other in [
        RANK_SCREEN,
        END_TITLE,
        "end_title_plain",
        "tab_header",
        "accolades",
    ] {
        assert_eq!(signal(&accolade, other), None, "{other}");
    }
    // only the rank crop is lit
    assert_eq!(signal(&rank, RANK_SCREEN), Some(OutcomeSource::RankScreen));
    assert_eq!(
        signal(&rank, "rank_screen_wide"),
        Some(OutcomeSource::RankScreen)
    );
    for other in [
        ACCOLADE,
        END_TITLE,
        "tab_header",
        "rank_screens",
        "rank_title",
    ] {
        assert_eq!(signal(&rank, other), None, "{other}");
    }
    // both lit: the word's own screen names the source
    assert_eq!(signal(&both, ACCOLADE), Some(OutcomeSource::ResultWord));
    assert_eq!(signal(&both, RANK_SCREEN), Some(OutcomeSource::RankScreen));
    assert_eq!(signal(&both, END_TITLE), None);
}

/// The lit-crop gate and the stability wait still come before the read
/// under `reader = "new"`: behind them the words pack is not read, even
/// when it would name a sure word.
#[test]
fn the_gates_still_come_before_the_words_read() {
    let victory = || pack_words(BOTH, sure(Outcome::Victory, ACCOLADE));
    let reads_before = pack_reads();
    // a dark crop: never read, on the poll tick and on the Tab frame
    let dark = dark_frame();
    let mut st = FrameStability::default();
    for _ in 0..3 {
        assert_eq!(poll_new(&dark, &mut st, &victory()), None);
    }
    assert_eq!(
        detect_outcome_with(&dark.0, NEW, &no_title(), &victory()),
        MatchOutcome::Unknown
    );
    assert_eq!(pack_reads(), reads_before, "read behind the lit gate");

    // a lit crop: not on its first sighting, then on the next tick
    let lit = result_frame(true, false, 0);
    let mut st = FrameStability::default();
    let words = victory();
    assert_eq!(poll_new(&lit, &mut st, &words), None, "first sighting");
    assert_eq!(words.taken(), None);
    assert_eq!(
        pack_reads(),
        reads_before,
        "read before the crop held still"
    );
    assert_eq!(
        poll_new(&lit, &mut st, &victory()),
        Some((MatchOutcome::Victory, OutcomeSource::ResultWord))
    );
    assert_eq!(pack_reads() - reads_before, 1);
    // the crop moved: deferred again
    let moved = result_frame(true, false, 24);
    assert_eq!(poll_new(&moved, &mut st, &victory()), None, "crop changed");
    assert_eq!(pack_reads() - reads_before, 1);
    // the Tab frame has no stability wait: a lit crop is read at once
    assert_eq!(
        detect_outcome_with(&lit.0, NEW, &no_title(), &victory()),
        MatchOutcome::Victory
    );
    assert_eq!(pack_reads() - reads_before, 2);
}

/// One words-v1 read per frame: both screens' gates pass, and the frame
/// is read once for the two of them.
#[test]
fn the_frame_is_read_once_for_both_screens() {
    let both = result_frame(true, true, 0);
    for read in [
        unread_frame(),
        screen_read(None, ACCOLADE, true),
        screen_read(Some(Outcome::Victory), ACCOLADE, true),
        screen_read(Some(Outcome::Victory), ACCOLADE, false),
        screen_read(Some(Outcome::Defeat), RANK_SCREEN, false),
    ] {
        let mut st = seen(&both);
        let words = pack_words(BOTH, Some(read.clone()));
        let reads_before = pack_reads();
        let _ = poll_new(&both, &mut st, &words);
        assert_eq!(pack_reads() - reads_before, 1, "{read:?}");
        assert_eq!(words.taken(), read.read.as_ref());
    }
}

/// Without the templates of a screen its word is read with OCR, exactly
/// as ocr-v1 reads it: the same Tesseract calls and the same answer.
#[test]
fn without_the_templates_a_result_word_is_read_with_ocr() {
    let both = result_frame(true, true, 0);
    let mut st = seen(&both);
    let before = ocr_calls();
    let ocr_v1 = poll_ocr(&both, &mut st);
    assert_eq!(ocr_calls() - before, 2, "ocr-v1 reads both crops");

    let accolade_only = ResultScreens {
        accolade: true,
        rank_screen: false,
    };
    let absent = Some(unread_frame());
    for (what, screens, read, ocr_reads, pack_read) in [
        // no result templates at all
        ("no pack", NO_SCREENS, None, 2, 0),
        // the pack names the accolade screen only: the rank crop is OCR's
        ("no rank templates", accolade_only, absent.clone(), 1, 1),
        // the pack went away between the two looks
        ("pack unloaded", BOTH, None, 2, 1),
        (
            "pack swapped",
            BOTH,
            Some(ScreenRead {
                screens: NO_SCREENS,
                ..screen_read(Some(Outcome::Victory), ACCOLADE, false)
            }),
            2,
            1,
        ),
    ] {
        let mut st = seen(&both);
        let words = pack_words(screens, read);
        let (ocr_before, reads_before) = (ocr_calls(), pack_reads());
        assert_eq!(poll_new(&both, &mut st, &words), ocr_v1, "{what}");
        assert_eq!(ocr_calls() - ocr_before, ocr_reads, "{what}");
        assert_eq!(pack_reads() - reads_before, pack_read, "{what}");
    }
    // with no pack nothing is kept for the caller
    let mut st = seen(&both);
    let words = pack_words(NO_SCREENS, None);
    let _ = poll_new(&both, &mut st, &words);
    assert_eq!(words.taken(), None);
    assert_eq!(words.into_taken(), None);
}

/// `reader = "ocr-v1"` has no words reader at all: every ocr-v1 entry
/// point reads the two words with Tesseract, and keeps no read.
#[test]
fn ocr_v1_never_reads_the_words_pack() {
    for words in [
        ResultWords::ocr(),
        ResultWords::default(),
        ResultWords::for_reader(false),
    ] {
        assert!(!words.from_words());
        assert!(words.reader.is_none());
    }
    assert!(ResultWords::for_reader(true).from_words());

    let both = result_frame(true, true, 0);
    let words = ResultWords::ocr();
    assert_eq!(words.word_on(&ACCOLADE_WORD, &both.1), None);
    assert_eq!(words.word_on(&RANK_WORD, &both.1), None);
    assert_eq!(words.taken(), None);

    let reads_before = pack_reads();
    let before = ocr_calls();
    assert_eq!(detect_outcome(&both.0), MatchOutcome::Unknown);
    assert_eq!(detect_outcome_signal(&both.0), None);
    assert_eq!(detect_outcome_signal_with_rgb(&both.0, &both.1), None);
    assert_eq!(ocr_calls() - before, 6, "two Tesseract calls per frame");
    assert_eq!(pack_reads(), reads_before);
}

/// The Tab frame's outcome read under `reader = "new"`: a sure word on
/// the accolade or the rank screen, with no OCR.
#[test]
fn the_tab_frame_takes_its_result_word_from_the_words_pack() {
    for (frame, layout) in [
        (result_frame(true, false, 0), ACCOLADE),
        (result_frame(false, true, 0), RANK_SCREEN),
    ] {
        let before = ocr_calls();
        assert_eq!(detect_outcome(&frame.0), MatchOutcome::Unknown);
        assert_eq!(ocr_calls() - before, 1, "ocr-v1 reads the crop");
        let before = ocr_calls();
        for (word, want) in WORDS {
            let words = pack_words(BOTH, sure(word, layout));
            assert_eq!(
                detect_outcome_with(&frame.0, NEW, &no_title(), &words),
                want
            );
        }
        assert_eq!(ocr_calls(), before, "no OCR under the new reader");
    }
}

/// The rank screen stays up across many poll ticks. The first value is
/// logged. A repeat is quiet. A different outcome is logged again.
#[test]
fn rank_result_is_logged_on_the_first_poll_and_only_again_when_it_changes() {
    let mut last = None;
    let mut logged = Vec::new();
    for outcome in [
        MatchOutcome::Defeat,
        MatchOutcome::Defeat,
        MatchOutcome::Defeat,
        MatchOutcome::Victory,
        MatchOutcome::Victory,
        MatchOutcome::Defeat,
    ] {
        if rank_result_should_log(last, outcome) {
            last = Some(outcome);
            logged.push(outcome);
        }
    }
    assert_eq!(
        logged,
        vec![
            MatchOutcome::Defeat,
            MatchOutcome::Victory,
            MatchOutcome::Defeat,
        ]
    );
    assert!(
        !rank_result_should_log(Some(MatchOutcome::Draw), MatchOutcome::Draw),
        "a repeat of the same rank result stays quiet"
    );
    assert!(rank_result_should_log(None, MatchOutcome::Draw));
}

/// Two games in a row can finish with the same rank result. Each game
/// logs that result on its first poll. A second poll of the same game
/// stays quiet. Game end clears the memory, the same clear
/// [`clear_rank_result_log`] the session close runs beside the column
/// cache reset.
#[test]
fn the_same_rank_result_is_logged_in_both_games() {
    let _hold = rank_log_test_guard();
    clear_rank_result_log();
    let mut logged = Vec::new();
    for game in 0..2 {
        for outcome in [MatchOutcome::Defeat, MatchOutcome::Defeat] {
            if rank_result_is_new(outcome) {
                logged.push((game, outcome));
            }
        }
        clear_rank_result_log();
    }
    assert_eq!(
        logged,
        vec![(0, MatchOutcome::Defeat), (1, MatchOutcome::Defeat),]
    );
}

/// Under `reader = "new"`, a result word that falls back to OCR (no
/// templates for its screen) is logged once per screen. A second call
/// for that screen stays quiet.
#[test]
fn a_result_word_ocr_fallback_is_logged() {
    for screen in ResultScreen::ALL {
        let warned = std::sync::atomic::AtomicBool::new(false);
        let first = fallback_warn_log(|| {
            assert!(log_result_word_ocr_fallback(screen, &warned));
        });
        assert_eq!(
            first.matches("its result word is read with OCR").count(),
            1,
            "{screen:?}: {first}"
        );
        assert!(first.contains("WARN"), "{first}");
        assert!(first.contains(screen.id()), "{first}");
        let second = fallback_warn_log(|| {
            assert!(!log_result_word_ocr_fallback(screen, &warned));
        });
        assert!(
            !second.contains("its result word is read with OCR"),
            "the {screen:?} fallback is logged once: {second}"
        );
    }
    let both = result_frame(true, true, 0);
    let mut st = seen(&both);
    let _ = poll_new(&both, &mut st, &pack_words(NO_SCREENS, None));
    for screen in ResultScreen::ALL {
        assert!(
            !warn_result_word_ocr_fallback(screen),
            "the production path spends the {screen:?} warning"
        );
    }
}

fn fallback_warn_log(f: impl FnOnce()) -> String {
    let buf = std::sync::Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
    let writer = buf.clone();
    let sub = tracing_subscriber::fmt()
        .with_writer(move || {
            struct W(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
            impl std::io::Write for W {
                fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
                    self.0.lock().unwrap().extend_from_slice(data);
                    Ok(data.len())
                }
                fn flush(&mut self) -> std::io::Result<()> {
                    Ok(())
                }
            }
            W(writer.clone())
        })
        .with_ansi(false)
        .with_max_level(tracing::Level::WARN)
        .finish();
    tracing::subscriber::with_default(sub, f);
    String::from_utf8(buf.lock().unwrap().clone()).unwrap()
}
