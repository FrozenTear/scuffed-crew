use std::cell::OnceCell;
use std::sync::{Mutex, PoisonError};

use image::{DynamicImage, GrayImage, Luma, RgbImage};

use super::MatchOutcome;
use super::stability::FrameStability;
use crate::shadow::result::{Outcome, ResultRead, ResultScreen, ResultScreens, ScreenRead};

/// Which detector produced an outcome. The banner color-flood is specific
/// enough to act on from a single frame; the word sources (accolade
/// screen, competitive rank screen) are cheap but weaker evidence, so the
/// poller requires two agreeing word reads inside a confirmation window.
/// Who reads those two words depends on the reader ([`ResultWords`]): OCR
/// under `reader = "ocr-v1"`, the words-v1 pack under `reader = "new"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutcomeSource {
    Banner,
    /// Top-left result word on the post-match accolade screen.
    ResultWord,
    /// Result word under the "COMPETITIVE" title on the rank-update screen.
    RankScreen,
    /// Centered italic VICTORY/DEFEAT on the in-world end-title overlay
    /// (theme-colored; no gold/red banner flood). Added 2026-08-18 after a
    /// magenta-UI frame where every other source missed.
    EndTitle,
}

/// Who looks for the FINAL SCORE line under the end title (P4), the
/// end-of-match evidence that lets the end-title read run when the title's
/// colour mass is too low (see [`read_end_title`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FinalScoreSource {
    /// Tesseract over the line (`reader = "ocr-v1"`).
    #[default]
    Ocr,
    /// The words-v1 `p4_final_score` template (`reader = "new"`). Only a
    /// sure read is a signal; a suspect or absent read is not, and no OCR
    /// runs. Without that template loaded the line is read with OCR, unless
    /// OCR is off (`ocr = "off"`): then the line is not read at all.
    ///
    /// Since alpha.7 O2 the daemon reaches this only on the fallback: the
    /// FINAL SCORE line opens the gate of the end-title OCR, and `reader =
    /// "new"` runs that OCR only while no end-title templates are loaded
    /// ([`Titles`]). With them loaded the end title is the words-v1 read,
    /// which has no gate, and this template is not read.
    ///
    /// Kept (lane S1's call): with OCR on and a word pack from before the
    /// end titles, it is still what keeps Tesseract off the FINAL SCORE
    /// line. With OCR off that fallback does not exist, so the template is
    /// not read at all and the switch does not ask for it
    /// (`ocr_switch::PackGroup`).
    Words,
}

impl FinalScoreSource {
    /// The source for this reader setting (`reader = "new"` or not).
    pub fn for_reader(use_new_reader: bool) -> Self {
        if use_new_reader {
            FinalScoreSource::Words
        } else {
            FinalScoreSource::Ocr
        }
    }
}

/// One frame's end-of-match titles, by reader: the centred end title
/// VICTORY! / DEFEAT (P3, [`read_end_title`]) and the title card that wakes
/// the poller (P5, [`detect_end_reel_with`]).
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Titles {
    /// Tesseract reads both. This is `reader = "ocr-v1"`, and also `reader
    /// = "new"` while no end-title templates are loaded (no words-v1 pack,
    /// or one without an `end_title` VICTORY and DEFEAT pair): the same
    /// reads main makes, logged once per process
    /// ([`warn_end_title_ocr_fallback`]). With OCR off that fallback is
    /// [`Titles::Held`] instead.
    #[default]
    Ocr,
    /// `reader = "new"` with classic OCR off and no end-title templates
    /// loaded: neither title is read. The end title is no outcome and the
    /// title card is no wake, and no OCR call is made. The switch does not
    /// grant off without the templates, so this is a pack that went away
    /// while the tracker runs (`ocr_switch::PackGroup::EndTitles`).
    Held,
    /// `reader = "new"` with end-title templates loaded: no OCR. This is
    /// the frame's words-v1 end-title read (`shadow::read_end_title`). Only
    /// a sure read counts: a flagged read and a frame with no word are "no
    /// end title".
    Words(ResultRead),
}

impl Titles {
    /// The titles of this frame for the reader setting. Under `reader =
    /// "new"` this reads the end title once, for both P3 and P5; under
    /// `reader = "ocr-v1"` it reads nothing.
    pub fn for_rgb(use_new_reader: bool, rgb: &RgbImage) -> Self {
        Self::for_rgb_with(use_new_reader, rgb, crate::shadow::read_end_title)
    }

    /// [`Self::for_rgb`] for a frame that may not be RGB yet (capture hands
    /// RGBA frames).
    pub fn for_frame(use_new_reader: bool, img: &DynamicImage) -> Self {
        Self::for_frame_with(use_new_reader, img, crate::shadow::read_end_title)
    }

    /// [`Self::for_rgb`] with the words-v1 read as an argument (tests).
    /// `words` is called only under `reader = "new"`; `None` from it (no
    /// end-title templates loaded) falls back to the OCR reads.
    fn for_rgb_with(
        use_new_reader: bool,
        rgb: &RgbImage,
        words: impl FnOnce(&RgbImage) -> Option<ResultRead>,
    ) -> Self {
        if !use_new_reader {
            return Titles::Ocr;
        }
        match words(rgb) {
            Some(read) => Titles::Words(read),
            // No end-title templates are loaded. With OCR off there is no
            // OCR to fall back to: both titles are held.
            None if crate::ocr::gate::is_off() => Titles::Held,
            // With OCR on, P3 and P5 run Tesseract, as on main.
            None => {
                warn_end_title_ocr_fallback();
                Titles::Ocr
            }
        }
    }

    /// [`Self::for_frame`] with the words-v1 read as an argument (tests).
    fn for_frame_with(
        use_new_reader: bool,
        img: &DynamicImage,
        words: impl FnOnce(&RgbImage) -> Option<ResultRead>,
    ) -> Self {
        if !use_new_reader {
            return Titles::Ocr;
        }
        match img.as_rgb8() {
            Some(rgb) => Self::for_rgb_with(true, rgb, words),
            None => Self::for_rgb_with(true, &img.to_rgb8(), words),
        }
    }

    /// The word of a sure end-title read under `reader = "new"`.
    fn sure_end_title(&self) -> Option<Outcome> {
        match self {
            Titles::Words(read) => read.sure(),
            Titles::Ocr | Titles::Held => None,
        }
    }
}

/// Under `reader = "new"`, the end title (P3) and the end-reel title card
/// (P5) fell back to OCR because no end-title templates are loaded (no
/// words-v1 pack, or one without an `end_title` VICTORY and DEFEAT pair).
/// Logged once per process (F263-1), the way the FINAL SCORE fallback is
/// ([`warn_final_score_ocr_fallback`]). Returns whether this call logged
/// (tests).
fn warn_end_title_ocr_fallback() -> bool {
    static WARNED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    let first = !WARNED.swap(true, std::sync::atomic::Ordering::Relaxed);
    if first {
        tracing::warn!(
            "reader = \"new\" but no end-title templates are loaded (words-v1 \
             end_title VICTORY and DEFEAT); the end title and the end-reel \
             title card are read with OCR"
        );
    }
    first
}

/// The words-v1 side of [`ResultWords`].
#[derive(Debug, Clone, Copy)]
struct WordsReader {
    /// The result screens the loaded templates can name. Reads no frame.
    screens: fn() -> ResultScreens,
    /// The frame's result-screens read, `None` when no result templates
    /// are loaded.
    read: fn(&RgbImage) -> Option<ScreenRead>,
}

/// One frame's result word on the accolade screen and on the rank screen
/// (P1 and P2, [`read_result_word`] and [`read_rank_screen_result`]), by
/// reader. Make one per frame.
///
/// `reader = "ocr-v1"` ([`Self::ocr`]): Tesseract reads each screen's title
/// crop. Nothing is read from the words pack and nothing is kept here.
///
/// `reader = "new"`: no OCR. Behind the same gates as that OCR (a lit crop
/// and, on a poll tick, a crop that held still since the previous tick) the
/// frame's result screens are read from the words pack
/// (`shadow::read_result_screens`), at most once per frame, and the read is
/// kept here. A sure read whose word was found on a screen's layout is that
/// screen's word. A flagged read, a frame with no word, and a word found
/// on another layout are all "no word", and nothing falls back to OCR.
///
/// One read is one unconfirmed word, exactly as one OCR read is. The poller
/// confirms an outcome on a second agreeing word read inside its window
/// (`resolve_word_tick`). Until then the session machine keeps the word as
/// a hint, and a hint can still become the stored outcome when a start
/// screen or a split seals it (`boundary`). That rule is not changed here,
/// and the evidence behind such a hint is not weaker than under ocr-v1:
/// the same lit crop, the same wait for it to hold still, then a certified
/// template read in place of a fuzzy OCR match.
///
/// A screen the loaded templates cannot name (no words pack, or one without
/// that screen's VICTORY and DEFEAT) keeps its OCR read.
///
/// The read behind a word is `shadow::read_result`'s read of the same
/// frame, so the caller takes it for the frame's result evidence
/// ([`Self::into_taken`]) and does not read the frame a second time.
#[derive(Debug, Default)]
pub struct ResultWords {
    /// `None` under `reader = "ocr-v1"`.
    reader: Option<WordsReader>,
    screens: OnceCell<ResultScreens>,
    read: OnceCell<Option<ScreenRead>>,
}

impl ResultWords {
    /// `reader = "ocr-v1"`: both words are read with OCR.
    pub fn ocr() -> Self {
        Self::default()
    }

    /// The result words of one frame for this reader setting (`reader =
    /// "new"` or not). Reads nothing yet.
    pub fn for_reader(use_new_reader: bool) -> Self {
        if use_new_reader {
            Self::with_reader(WordsReader {
                screens: crate::shadow::result_screens,
                read: crate::shadow::read_result_screens,
            })
        } else {
            Self::ocr()
        }
    }

    fn with_reader(reader: WordsReader) -> Self {
        Self {
            reader: Some(reader),
            ..Self::default()
        }
    }

    /// Does this frame take its result words from the words pack (`reader =
    /// "new"`)?
    pub fn from_words(&self) -> bool {
        self.reader.is_some()
    }

    /// The frame's words-v1 result read, when one was made: a screen's
    /// gates passed, the loaded templates can name that screen, and the
    /// frame shows a result-screen word.
    pub fn taken(&self) -> Option<&ResultRead> {
        self.read.get()?.as_ref()?.read.as_ref()
    }

    /// [`Self::taken`], by value: the frame's result evidence.
    pub fn into_taken(self) -> Option<ResultRead> {
        self.read.into_inner()??.read
    }

    /// The word on `at`'s screen from the words pack, for a frame whose
    /// title crop passed its gates. `None` when OCR is to read the crop:
    /// under `reader = "ocr-v1"`, and when the loaded templates cannot name
    /// this screen. Otherwise the frame's read decides, and `Unknown` is
    /// final.
    fn word_on(&self, at: &WordScreen, rgb: &RgbImage) -> Option<MatchOutcome> {
        let reader = self.reader?;
        if !self.screens.get_or_init(reader.screens).has(at.screen) {
            warn_result_word_ocr_fallback(at.screen);
            return None;
        }
        let read = self.read.get_or_init(|| (reader.read)(rgb)).as_ref();
        // The templates went away between the two looks (a pack swap).
        let Some(read) = read.filter(|r| r.screens.has(at.screen)) else {
            warn_result_word_ocr_fallback(at.screen);
            return None;
        };
        let outcome = match read.word_on(at.screen) {
            Some(Outcome::Victory) => MatchOutcome::Victory,
            Some(Outcome::Defeat) => MatchOutcome::Defeat,
            Some(Outcome::Draw) => MatchOutcome::Draw,
            None => MatchOutcome::Unknown,
        };
        match &read.read {
            Some(r) if outcome.is_decided() => {
                // The rank screen stays up across many poll ticks. Log the
                // first value, then only a change. The accolade screen is
                // short and keeps a line per read.
                if at.screen != ResultScreen::RankScreen || rank_result_is_new(outcome) {
                    tracing::info!(
                        score = r.score,
                        margin = r.margin,
                        context = at.context,
                        "result word (words-v1): {outcome:?}"
                    );
                }
            }
            read => tracing::trace!(
                ?read,
                context = at.context,
                "result word (words-v1): no sure word on this screen"
            ),
        }
        Some(outcome)
    }
}

/// True the first time `outcome` is seen on the rank screen, and again only
/// when a later poll reads a different outcome. Repeats stay quiet.
fn rank_result_should_log(previous: Option<MatchOutcome>, next: MatchOutcome) -> bool {
    previous != Some(next)
}

/// Last rank-screen outcome this process logged. Poll ticks can move
/// threads, so this is process-wide. Cleared when a game ends
/// ([`clear_rank_result_log`]) so the next game logs its first result
/// even when that result matches the game before it.
static RANK_RESULT_LOG: Mutex<Option<MatchOutcome>> = Mutex::new(None);

/// Forget the last rank-screen result. A game that ends on defeat must
/// still log defeat when the next game ends the same way.
pub fn clear_rank_result_log() {
    #[cfg(test)]
    let _hold = rank_log_test_guard();
    let mut last = RANK_RESULT_LOG
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    *last = None;
}

/// Record a rank-screen result and return whether it differs from the last
/// one logged. Public so the session close can be tested from the binary.
pub fn rank_result_is_new(outcome: MatchOutcome) -> bool {
    #[cfg(test)]
    let _hold = rank_log_test_guard();
    let mut last = RANK_RESULT_LOG
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let due = rank_result_should_log(*last, outcome);
    if due {
        *last = Some(outcome);
    }
    due
}

/// One test owns the rank-log memory while it plays two games. Other tests
/// that read a rank screen block until it finishes, so a parallel poll
/// cannot change what this test logged.
#[cfg(test)]
static RANK_LOG_TEST_LOCK: Mutex<()> = Mutex::new(());

#[cfg(test)]
thread_local! {
    static RANK_LOG_LOCK_HELD: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
struct RankLogTestGuard {
    _guard: Option<std::sync::MutexGuard<'static, ()>>,
}

#[cfg(test)]
impl Drop for RankLogTestGuard {
    fn drop(&mut self) {
        if self._guard.is_some() {
            RANK_LOG_LOCK_HELD.with(|held| held.set(false));
        }
    }
}

/// Lock the rank-log memory for this thread. A caller that already holds it
/// gets a no-op guard, so [`rank_result_is_new`] can take the same lock
/// from inside a test without deadlocking.
#[cfg(test)]
fn rank_log_test_guard() -> RankLogTestGuard {
    if RANK_LOG_LOCK_HELD.with(|held| held.get()) {
        return RankLogTestGuard { _guard: None };
    }
    let guard = RANK_LOG_TEST_LOCK
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    RANK_LOG_LOCK_HELD.with(|held| held.set(true));
    RankLogTestGuard {
        _guard: Some(guard),
    }
}

/// Under `reader = "new"`, a result word fell back to OCR because the
/// loaded templates cannot name its screen (no words-v1 pack yet, or one
/// without that screen's VICTORY and DEFEAT). Logged once per screen and
/// process. With classic OCR off there is no such fallback (the word is
/// held in [`read_outcome_word`]) and no such line. Returns whether this
/// call logged (tests).
fn warn_result_word_ocr_fallback(screen: ResultScreen) -> bool {
    use std::sync::atomic::{AtomicBool, Ordering};
    if crate::ocr::gate::is_off() {
        return false;
    }
    static WARNED: [AtomicBool; 2] = [AtomicBool::new(false), AtomicBool::new(false)];
    let first = !WARNED[screen as usize].swap(true, Ordering::Relaxed);
    if first {
        tracing::warn!(
            screen = screen.id(),
            "reader = \"new\" but no VICTORY and DEFEAT templates are loaded for this \
             screen; its result word is read with OCR"
        );
    }
    first
}

/// One-shot outcome read for a single frame (Tab captures, dev tools).
pub fn detect_outcome(img: &DynamicImage) -> MatchOutcome {
    detect_outcome_with(
        img,
        FinalScoreSource::Ocr,
        &Titles::Ocr,
        &ResultWords::ocr(),
    )
}

/// [`detect_outcome`] with the FINAL SCORE line read by `p4`, the end
/// title from `titles` and the accolade and rank-screen words by `words`.
pub fn detect_outcome_with(
    img: &DynamicImage,
    p4: FinalScoreSource,
    titles: &Titles,
    words: &ResultWords,
) -> MatchOutcome {
    let rgb = img.to_rgb8();
    detect_outcome_signal_inner(img, &rgb, None, p4, titles, words)
        .map(|(outcome, _)| outcome)
        .unwrap_or(MatchOutcome::Unknown)
}

/// Outcome detection with its evidence source, for the poller.
///
/// Order: the full-screen VICTORY/DEFEAT banner color-flood first (fast, very
/// specific), then OCR of the accolade screen's top-left result word. The word
/// OCR runs unconditionally — it used to be gated behind a "60% of pixels lean
/// blue" accolade-screen check, but custom UI color schemes (e.g. magenta)
/// break any assumption about the screen's dominant color, and the full-frame
/// pixel scan cost more than the small-crop OCR it was guarding. The Otsu-based
/// `read_result_word` is color-scheme-independent.
///
/// This entry point is ocr-v1's. The daemon's go through the reader switch:
/// [`detect_outcome_signal_polled`] and [`detect_outcome_with`].
pub fn detect_outcome_signal(img: &DynamicImage) -> Option<(MatchOutcome, OutcomeSource)> {
    let rgb = img.to_rgb8();
    detect_outcome_signal_with_rgb(img, &rgb)
}

/// Outcome detection when the caller already converted the frame to RGB (P6).
pub fn detect_outcome_signal_with_rgb(
    img: &DynamicImage,
    rgb: &RgbImage,
) -> Option<(MatchOutcome, OutcomeSource)> {
    detect_outcome_signal_inner(
        img,
        rgb,
        None,
        FinalScoreSource::Ocr,
        &Titles::Ocr,
        &ResultWords::ocr(),
    )
}

/// Poll-tick outcome detection: identical to
/// [`detect_outcome_signal_with_rgb`] except the word-OCR crops are gated on
/// temporal stability — Tesseract only runs once a crop has held still across
/// consecutive ticks (see [`FrameStability`]). The banner color-flood is never
/// gated: it is cheap, lasts ~3s, and a second tick may never come. Word
/// detection shifts at most one tick later, inside the budget of screens that
/// stay up 15–20s and already need two agreeing reads to confirm.
///
/// `p4` says who reads the FINAL SCORE line ([`FinalScoreSource`]).
/// `titles` is the frame's end title by reader ([`Titles`]). Under `reader =
/// "new"` the end title has the same two-tick rule in another form
/// ([`polled_end_title`]): its read is already made, a first sure sighting
/// is no signal (it only wakes, [`detect_end_reel_with`]), and the same
/// sure word on the next poll tick is the signal.
/// `words` reads the accolade and rank-screen words ([`ResultWords`]).
/// Under `reader = "new"` they keep both gates: the words-v1 read is made
/// only for a lit crop that held still since the previous tick, as the OCR
/// it replaces was.
pub fn detect_outcome_signal_polled(
    img: &DynamicImage,
    rgb: &RgbImage,
    stability: &mut FrameStability,
    p4: FinalScoreSource,
    titles: &Titles,
    words: &ResultWords,
) -> Option<(MatchOutcome, OutcomeSource)> {
    detect_outcome_signal_inner(img, rgb, Some(stability), p4, titles, words)
}

fn detect_outcome_signal_inner(
    img: &DynamicImage,
    rgb: &RgbImage,
    mut stability: Option<&mut FrameStability>,
    p4: FinalScoreSource,
    titles: &Titles,
    words: &ResultWords,
) -> Option<(MatchOutcome, OutcomeSource)> {
    // `reader = "new"`: settle this tick's end title before any other
    // source answers, so its two-tick memory moves on every poll tick.
    let words_title = match titles {
        Titles::Words(read) => Some(match stability.as_deref_mut() {
            Some(stability) => polled_end_title(stability, read),
            None => end_title_from_words(read),
        }),
        // Held is settled in `read_end_title`, which reads nothing for it.
        Titles::Ocr | Titles::Held => None,
    };
    if let Some(outcome) = detect_banner(rgb) {
        return Some((outcome, OutcomeSource::Banner));
    }
    match read_result_word(img, rgb, stability.as_deref_mut(), words) {
        MatchOutcome::Unknown => {}
        outcome => return Some((outcome, OutcomeSource::ResultWord)),
    }
    match read_rank_screen_result(img, rgb, stability.as_deref_mut(), words) {
        MatchOutcome::Unknown => {}
        outcome => return Some((outcome, OutcomeSource::RankScreen)),
    }
    let title = match words_title {
        Some(word) => word,
        None => read_end_title(img, rgb, stability, p4, titles),
    };
    match title {
        MatchOutcome::Unknown => None,
        outcome => Some((outcome, OutcomeSource::EndTitle)),
    }
}

/// Text-based outcome fallback for the *captured Tab frame*: the centred
/// end title ([`read_end_title`]), meant to be called only when
/// `detect_outcome` returns `Unknown` and no outcome was carried over from
/// the poller.
///
/// It used to OCR a VICTORY / DEFEAT header at the top centre first (T1:
/// x 30-70%, y 2-24% of the 16:9 rect). Removed in alpha.7 O4. T1's own
/// crop and check found no result word on 652 frames (2026-10-13):
/// * 257 Tab candidates from 67 games (member frames 2026-05-17 to
///   2026-10-13, 2026 VOD boards), 8 of them end-of-match Tab frames from 7
///   games (Escort, Hybrid, Control; 6 known competitive);
/// * end titles, accolade and rank screens of Escort, Hybrid, Control, Push
///   and Flashpoint, and two 2022-23 UI references.
///
/// Not covered: Push, Flashpoint and Clash end-of-match Tab frames, quick
/// play, arcade, custom games, Stadium, and a DRAW. The full-board OCR's
/// header line (`parse::outcome_from_board_header`, ET-2) is untouched and
/// remains the only reader of a result word on the board itself. It runs
/// under ocr-v1 only: the new reader never runs the full-board OCR (alpha.7
/// O10), so a result word on the board itself is not read there at all.
pub fn detect_outcome_text(img: &DynamicImage) -> MatchOutcome {
    detect_outcome_text_with(img, FinalScoreSource::Ocr, &Titles::Ocr)
}

/// [`detect_outcome_text`] with the FINAL SCORE line read by `p4` and the
/// end title from `titles`.
pub fn detect_outcome_text_with(
    img: &DynamicImage,
    p4: FinalScoreSource,
    titles: &Titles,
) -> MatchOutcome {
    // the words-v1 read is already made: no second RGB copy (F263-9)
    if let Titles::Words(read) = titles {
        return end_title_from_words(read);
    }
    read_end_title(img, &img.to_rgb8(), None, p4, titles)
}

/// Centered italic end-title (theme-colored VICTORY/DEFEAT over the world).
/// Cheap sat-mass / scoreline gate first so mid-fight ticks do not pay OCR.
/// The gate passes on enough title colour mass, or, when the mass is low,
/// on the FINAL SCORE line under the title (P4, read by `p4`). Either way
/// the outcome itself is the title word's OCR below.
///
/// Under `reader = "new"` with end-title templates loaded
/// ([`Titles::Words`]) the word is the frame's words-v1 end-title read and
/// nothing else runs: no gate, no FINAL SCORE read, no OCR. The gate saves
/// Tesseract calls, and it also shuts on a title with little colour (a pale
/// title, a fade-in). This is the one-shot read of a Tab frame; the poll
/// tick settles its end title in [`detect_outcome_signal_inner`], with the
/// two-tick rule of [`polled_end_title`].
fn read_end_title(
    img: &DynamicImage,
    rgb: &RgbImage,
    mut stability: Option<&mut FrameStability>,
    p4: FinalScoreSource,
    titles: &Titles,
) -> MatchOutcome {
    if let Titles::Words(read) = titles {
        return end_title_from_words(read);
    }
    if matches!(titles, Titles::Held) {
        // OCR is off and no end-title templates are loaded: no gate, no
        // FINAL SCORE read, no OCR.
        return MatchOutcome::Unknown;
    }
    let mass = center_title_mass(rgb);
    let scoreline = mass < END_TITLE_MASS_MIN
        && scoreline_looks_present(img, rgb, stability.as_deref_mut(), p4);
    if mass < END_TITLE_MASS_MIN && !scoreline {
        return MatchOutcome::Unknown;
    }

    let (fw, fh) = (img.width(), img.height());
    let (gx, gy, gw, gh) = crate::ocr::preprocess::game_rect_16_9(fw, fh);
    // Tighter than the mass window — just the italic word.
    // Calibrated on rejected_preflight_20260817_232203.png (magenta VICTORY!).
    let x = gx + gw * 320 / 1000;
    let y = gy + gh * 340 / 1000;
    let cw = gw * 360 / 1000;
    let ch = gh * 180 / 1000;
    if cw == 0 || ch == 0 || x + cw > fw || y + ch > fh {
        return MatchOutcome::Unknown;
    }
    let crop = img.crop_imm(x, y, cw, ch);

    if let Some(stability) = stability
        && !stability.check("end title", &crop)
    {
        tracing::trace!("end title crop not stable: deferring OCR");
        return MatchOutcome::Unknown;
    }

    let prepared = prepare_end_title(&crop);
    match crate::ocr::recognize_prepared_lang(
        &prepared,
        "7",
        Some("ABCDEFGHIJKLMNOPQRSTUVWXYZ!"),
        "eng",
    ) {
        Ok(text) => {
            let outcome = fuzzy_outcome_word(&text);
            if outcome.is_decided() {
                let word = match outcome {
                    MatchOutcome::Victory => "VICTORY",
                    MatchOutcome::Defeat => "DEFEAT",
                    MatchOutcome::Draw => "DRAW",
                    MatchOutcome::Unknown => "UNKNOWN",
                };
                tracing::info!(text = %text.trim(), context = "end title", "result word: {word}");
            } else {
                tracing::trace!(ocr_text = %text.trim(), "end title OCR did not match an outcome");
            }
            outcome
        }
        Err(e) => {
            tracing::debug!(error = %e, "end title OCR failed");
            MatchOutcome::Unknown
        }
    }
}

const END_TITLE_MASS_MIN: f32 = 0.06;

/// The end title under `reader = "new"`: the word of a sure words-v1
/// end-title read. A flagged read and a frame with no word are `Unknown`.
/// A DRAW is named only by a sure read (`shadow::result` drops an unsure
/// one).
///
/// This is one frame's word. A Tab frame uses it as that frame's outcome,
/// as it uses the OCR word under ocr-v1. The poll tick does not: see
/// [`polled_end_title`].
fn end_title_from_words(read: &ResultRead) -> MatchOutcome {
    let outcome = match read.sure() {
        Some(Outcome::Victory) => MatchOutcome::Victory,
        Some(Outcome::Defeat) => MatchOutcome::Defeat,
        Some(Outcome::Draw) => MatchOutcome::Draw,
        None => MatchOutcome::Unknown,
    };
    if outcome.is_decided() {
        tracing::info!(
            score = read.score,
            margin = read.margin,
            context = "end title",
            "result word (words-v1): {outcome:?}"
        );
    } else {
        tracing::trace!(
            score = read.score,
            margin = read.margin,
            suspect = read.suspect,
            "end title (words-v1): no sure word"
        );
    }
    outcome
}

/// [`FrameStability::repeats`] key of the poll tick's end-title word.
const END_TITLE_WORD: &str = "end title word";

/// The poll tick's end title under `reader = "new"`: the word of a sure
/// read, once the poll tick before this one read the same sure word.
///
/// A first sure sighting is not a signal. It wakes the poller
/// ([`detect_end_reel_with`]), so the next tick comes at full cadence, and
/// nothing else: it is no hint and no outcome. This is ocr-v1's rule in
/// another form (there the title crop must hold still across two ticks
/// before the OCR runs), and it is why one template read of one frame
/// cannot become a hint that the next start screen seals (F263-2). The
/// second sighting is one word signal like any other; the poller confirms
/// an outcome on a further agreeing word read (`resolve_word_tick`), so a
/// title that stays on screen confirms on its third poll, as under ocr-v1.
///
/// Called once per poll tick. A tick with no sure word, or another word,
/// starts over.
fn polled_end_title(stability: &mut FrameStability, read: &ResultRead) -> MatchOutcome {
    let word = end_title_from_words(read);
    let tag = match word {
        MatchOutcome::Victory => Some(1),
        MatchOutcome::Defeat => Some(2),
        MatchOutcome::Draw => Some(3),
        MatchOutcome::Unknown => None,
    };
    if stability.repeats(END_TITLE_WORD, tag) {
        return word;
    }
    if word.is_decided() {
        tracing::debug!(
            ?word,
            "end title (words-v1): first sure read, waiting for a second"
        );
    }
    MatchOutcome::Unknown
}

fn fuzzy_outcome_word(raw: &str) -> MatchOutcome {
    fuzzy_outcome_word_inner(raw, 2)
}

/// Accolade/rank (Tab) path: same VICTORY/DEFEAT window as EndTitle, but DRAW
/// is exact-contains only. Tab has no sat-mass gate and no two-read confirm;
/// lev≤1 still accepts `DRA`/`RAW` (distance 1 from DRAW).
fn fuzzy_outcome_word_accolade(raw: &str) -> MatchOutcome {
    fuzzy_outcome_word_inner(raw, 0)
}

fn fuzzy_outcome_word_inner(raw: &str, draw_max_dist: usize) -> MatchOutcome {
    let letters: String = raw
        .chars()
        .filter(|c| c.is_ascii_alphabetic())
        .map(|c| c.to_ascii_uppercase())
        .collect();
    if letters.is_empty() {
        return MatchOutcome::Unknown;
    }
    const TARGETS: &[(&str, MatchOutcome)] = &[
        ("VICTORY", MatchOutcome::Victory),
        ("DEFEAT", MatchOutcome::Defeat),
        ("DRAW", MatchOutcome::Draw),
    ];
    let hits: Vec<MatchOutcome> = TARGETS
        .iter()
        .filter(|(word, outcome)| {
            if letters.contains(word) {
                return true;
            }
            if letters.len() + 1 < word.len() {
                return false;
            }
            let max_dist = if matches!(outcome, MatchOutcome::Draw) {
                draw_max_dist
            } else {
                2
            };
            strsim::levenshtein(&letters, word) <= max_dist
        })
        .map(|(_, outcome)| *outcome)
        .collect();
    if hits.len() == 1 {
        hits[0]
    } else {
        MatchOutcome::Unknown
    }
}

fn rgb_to_hsv(r: u8, g: u8, b: u8) -> (u16, u8, u8) {
    let r = i32::from(r);
    let g = i32::from(g);
    let b = i32::from(b);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;
    let v = max as u8;
    let s = if max == 0 {
        0
    } else {
        ((delta * 255) / max) as u8
    };
    let h = if delta == 0 {
        0
    } else if max == r {
        let x = ((g - b) * 60) / delta;
        if x < 0 { x + 360 } else { x }
    } else if max == g {
        120 + ((b - r) * 60) / delta
    } else {
        240 + ((r - g) * 60) / delta
    };
    ((h as u16) % 360, s, v)
}

fn hue_near(a: u16, b: u16, window: u16) -> bool {
    let d = (i32::from(a) - i32::from(b)).unsigned_abs() as u16;
    d.min(360 - d) <= window
}

fn center_title_mass(rgb: &RgbImage) -> f32 {
    let (w, h) = rgb.dimensions();
    let (gx, gy, gw, gh) = crate::ocr::preprocess::game_rect_16_9(w, h);
    let x0 = gx + gw * 300 / 1000;
    let y0 = gy + gh * 300 / 1000;
    let x1 = x0 + gw * 400 / 1000;
    let y1 = y0 + gh * 250 / 1000;
    if x1 <= x0 || y1 <= y0 {
        return 0.0;
    }

    const STRIDE: u32 = 2;
    let mut bins = [0u32; 18];
    let mut sat_hits = 0u32;
    let mut total = 0u32;
    for y in (y0..y1.min(h)).step_by(STRIDE as usize) {
        for x in (x0..x1.min(w)).step_by(STRIDE as usize) {
            let [r, g, b] = rgb.get_pixel(x, y).0;
            total += 1;
            let (hue, sat, val) = rgb_to_hsv(r, g, b);
            if sat > 150 && val > 120 {
                sat_hits += 1;
                bins[(hue as usize) / 20] += 1;
            }
        }
    }
    if total == 0 || sat_hits == 0 {
        return 0.0;
    }
    let (dom_bin, _) = bins
        .iter()
        .enumerate()
        .max_by_key(|(_, c)| *c)
        .unwrap_or((0, &0));
    let locked = bins[dom_bin];
    locked as f32 / total as f32
}

/// P4: is the FINAL SCORE line under the end title on this frame? A cheap
/// white-pixel gate and the crop's stability come first; then `p4` reads
/// the line ([`FinalScoreSource`]).
fn scoreline_looks_present(
    img: &DynamicImage,
    rgb: &RgbImage,
    stability: Option<&mut FrameStability>,
    p4: FinalScoreSource,
) -> bool {
    scoreline_looks_present_with(img, rgb, stability, p4, crate::shadow::read_final_score)
}

/// Under `reader = "new"`, P4 fell back to OCR because no
/// `p4_final_score` template is loaded (an older words-v1 pack, or none
/// yet). Logged once per process (F257-1). With OCR off the fallback never
/// runs (see [`scoreline_looks_present_with`]).
/// Returns whether this call logged (tests).
fn warn_final_score_ocr_fallback() -> bool {
    static WARNED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    let first = !WARNED.swap(true, std::sync::atomic::Ordering::Relaxed);
    if first {
        tracing::warn!(
            "reader = \"new\" but no p4_final_score template is loaded; \
             the FINAL SCORE line is read with OCR"
        );
    }
    first
}

/// [`scoreline_looks_present`] with the words-v1 read as an argument
/// (tests). `words` is called only for [`FinalScoreSource::Words`]; `None`
/// from it (no template loaded) falls back to the OCR read while OCR is on.
/// With OCR off ([`crate::ocr::gate`]) there is no fallback: the line is not
/// read, P4 is not a signal, and no OCR call is made.
fn scoreline_looks_present_with(
    img: &DynamicImage,
    rgb: &RgbImage,
    stability: Option<&mut FrameStability>,
    p4: FinalScoreSource,
    words: impl FnOnce(&RgbImage) -> Option<crate::shadow::result::FinalScoreRead>,
) -> bool {
    let (w, h) = rgb.dimensions();
    let (gx, gy, gw, gh) = crate::ocr::preprocess::game_rect_16_9(w, h);
    let x0 = gx + gw * 300 / 1000;
    let y0 = gy + gh * 730 / 1000;
    let cw = gw * 400 / 1000;
    let ch = gh * 80 / 1000;
    if cw == 0 || ch == 0 || x0 + cw > w || y0 + ch > h {
        return false;
    }
    let mut white = 0u32;
    let mut total = 0u32;
    for y in (y0..y0 + ch).step_by(2) {
        for x in (x0..x0 + cw).step_by(2) {
            let [r, g, b] = rgb.get_pixel(x, y).0;
            total += 1;
            let (_, sat, val) = rgb_to_hsv(r, g, b);
            if val > 200 && sat < 40 {
                white += 1;
            }
        }
    }
    if total == 0 || (white as f32 / total as f32) < 0.008 {
        return false;
    }
    let crop = img.crop_imm(x0, y0, cw, ch);
    if let Some(stability) = stability
        && !stability.check("scoreline", &crop)
    {
        tracing::trace!("scoreline crop not stable: deferring OCR");
        return false;
    }
    if p4 == FinalScoreSource::Words {
        match words(rgb) {
            Some(read) => {
                tracing::trace!(?read, "final score line (words-v1)");
                return read.is_signal();
            }
            // No p4_final_score template is loaded. OCR off: hold. The
            // out-of-date words pack is what the switch reports
            // (`ocr_switch::PackProblem::WordsLackEndTitles`: a pack without the end titles).
            None if crate::ocr::gate::is_off() => return false,
            // OCR on: under `reader = "new"` P4 still runs Tesseract below.
            None => {
                warn_final_score_ocr_fallback();
            }
        }
    }
    let prepared = crate::ocr::preprocess::prepare_title(&crop);
    match crate::ocr::recognize_prepared(&prepared, "7", Some("ABCDEFGHIJKLMNOPQRSTUVWXYZ ")) {
        Ok(text) => {
            let letters: String = text
                .chars()
                .filter(|c| c.is_ascii_alphabetic())
                .map(|c| c.to_ascii_uppercase())
                .collect();
            letters.contains("FINAL") || letters.contains("SCORE")
        }
        Err(_) => false,
    }
}

fn opponent_ink(r: u8, g: u8, b: u8, dom_hue: u16) -> u8 {
    let r = i16::from(r);
    let g = i16::from(g);
    let b = i16::from(b);
    let v = if hue_near(dom_hue, 300, 40) {
        (r + b) / 2 - g
    } else if hue_near(dom_hue, 50, 40) {
        (r + g) / 2 - b
    } else if hue_near(dom_hue, 0, 25) || hue_near(dom_hue, 360, 25) {
        r - g.max(b)
    } else {
        r.max(g).max(b) - r.min(g).min(b)
    };
    v.clamp(0, 255) as u8
}

fn prepare_end_title(crop: &DynamicImage) -> GrayImage {
    let rgb = crop.to_rgb8();
    let (w, h) = rgb.dimensions();
    let mut hue_bins = [0u32; 18];
    for p in rgb.pixels() {
        let (hue, sat, val) = rgb_to_hsv(p.0[0], p.0[1], p.0[2]);
        if sat > 150 && val > 120 {
            hue_bins[(hue as usize) / 20] += 1;
        }
    }
    let dom_hue = (hue_bins
        .iter()
        .enumerate()
        .max_by_key(|(_, c)| *c)
        .map(|(i, _)| i)
        .unwrap_or(0)
        * 20) as u16;

    // Opponent-color ink for the dominant UI hue (magenta → (r+b)/2−g,
    // gold → (r+g)/2−b, red → r−max(g,b)). CLI tesseract reads the
    // magenta form as "VCTORY!"; hue-locked chroma/binary did not.
    let mut gray = GrayImage::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let [r, g, b] = rgb.get_pixel(x, y).0;
            gray.put_pixel(x, y, Luma([opponent_ink(r, g, b, dom_hue)]));
        }
    }
    let (sw, sh) = gray.dimensions();
    let scale = (120 / sh.max(1)).clamp(1, 4);
    if scale > 1 {
        image::imageops::resize(
            &gray,
            sw * scale,
            sh * scale,
            image::imageops::FilterType::CatmullRom,
        )
    } else {
        gray
    }
}

// Detect the brief VICTORY/DEFEAT full-screen banner (gold or red backdrop).
// OW2 banners saturate >40% of the screen with a very specific color range.
// Previous thresholds (15%, loose color ranges) caused false positives on
// websites with warm/red colors during normal browsing.
//
// Pixel scan uses stride 2 — ratio tests tolerate 1-in-2 sampling.
fn detect_banner(rgb: &RgbImage) -> Option<MatchOutcome> {
    let (w, h) = rgb.dimensions();
    let (gx, gy, gw, gh) = crate::ocr::preprocess::game_rect_16_9(w, h);

    // Sample the middle horizontal band (30%-70% of the 16:9 playfield) where
    // the banner colour flood is most consistent.
    let y_start = gy + gh * 30 / 100;
    let y_end = gy + gh * 70 / 100;
    let x_end = gx + gw;
    let mut gold_count = 0u32;
    let mut red_count = 0u32;
    let mut total = 0u32;
    const STRIDE: u32 = 2;

    for y in (y_start..y_end).step_by(STRIDE as usize) {
        for x in (gx..x_end).step_by(STRIDE as usize) {
            let [r, g, b] = rgb.get_pixel(x, y).0;
            total += 1;
            // OW2 victory gold: saturated warm gold, green channel well above blue
            if r > 200 && g > 140 && g < 220 && b < 60 && (r as i32 - b as i32) > 150 {
                gold_count += 1;
            }
            // OW2 defeat red: deep red, very low green and blue
            if r > 180 && g < 60 && b < 60 {
                red_count += 1;
            }
        }
    }

    if total == 0 {
        return None;
    }

    let gold_ratio = gold_count as f32 / total as f32;
    let red_ratio = red_count as f32 / total as f32;

    // OW2 banners flood >40% of the sampled region with the dominant color.
    // 35% threshold with tighter color ranges eliminates web page false positives.
    const THRESHOLD: f32 = 0.35;

    if gold_ratio > THRESHOLD {
        tracing::debug!(gold_ratio, "victory banner detected");
        Some(MatchOutcome::Victory)
    } else if red_ratio > THRESHOLD {
        tracing::debug!(red_ratio, "defeat banner detected");
        Some(MatchOutcome::Defeat)
    } else {
        None
    }
}

/// Cheap Play of the Game / highlight end-reel hint for the poller.
///
/// Not an outcome confirm — banner / two-agreeing word OCR are unchanged.
/// The poller uses this only to leave mid-match slow cadence (~8s) so the
/// short Victory/Defeat window that follows the reel is sampled at full
/// rate. Accolade / MVP lands ~15–20s after POTG (see [`read_result_word`]).
///
/// Three cheap paths, either is enough to wake:
/// 1. Cinematic letterbox (dark top+bottom bars, lit middle that looks
///    like footage, not a flat loading slab) — no OCR.
/// 2. Title-band OCR for PLAY OF THE GAME / HIGHLIGHT INTRO, gated on
///    [`cinematic_title_band`] so mid-fight ticks do not pay Tesseract.
/// 3. Nameplate POTG title card (lower-left gold battletag + white title
///    glyphs). Same class as letterbox: the cheap gate is the wake, and
///    no OCR runs on it.
///
/// Ban Heroes UI is a **distinct** signal ([`super::match_start::detect_ban_screen`])
/// and must not set `end_reel_wake_until`. Future: register voted bans;
/// this path only vetoes the wake.
pub fn detect_end_reel(img: &DynamicImage, rgb: &RgbImage) -> bool {
    detect_end_reel_with(img, rgb, &Titles::Ocr)
}

/// [`detect_end_reel`] by reader. Under `reader = "new"` with end-title
/// templates loaded ([`Titles::Words`]) path 2's phrase OCR does not run.
/// The title that wakes is the end title: a sure words-v1 read of it on
/// this frame is the wake, on its first sighting (the outcome signal waits
/// for the second, [`polled_end_title`]). It is checked before the ban veto
/// (the read is a template match; the veto is a pixel gate that also fires
/// on end titles). Paths 1 and 3 are the same under both readers.
pub fn detect_end_reel_with(img: &DynamicImage, rgb: &RgbImage, titles: &Titles) -> bool {
    if titles.sure_end_title().is_some() {
        tracing::info!("end title: end-of-match wake");
        return true;
    }
    if super::match_start::detect_ban_screen(img, rgb) {
        tracing::debug!("ban screen: not an end-reel wake");
        return false;
    }
    if end_reel_letterbox(rgb) {
        tracing::info!("end-reel letterbox: POTG / highlight wake");
        return true;
    }
    if matches!(titles, Titles::Ocr) && read_end_reel_title(img) {
        tracing::info!("end-reel title: POTG / highlight wake");
        return true;
    }
    if read_end_reel_nameplate(rgb) {
        tracing::info!("end-reel nameplate: POTG / highlight wake");
        return true;
    }
    false
}

/// OW2 POTG / highlight replay letterboxes the 16:9 playfield. Mid-match HUD
/// lights the top bar, so both-bars-dark plus a lit middle is a cheap
/// discriminator. A fade-to-black frame fails the middle check.
///
/// A flat mid slab (`ENTERING GAME` / other loading chrome) also looks
/// letterboxed (FrozenTear 2026-09-06 batch: top/bot dark ≥0.85, mid <0.5) but
/// is one dominant color, not footage. Require the mid band to spread
/// across coarse color bins.
fn end_reel_letterbox(rgb: &RgbImage) -> bool {
    let (w, h) = rgb.dimensions();
    let (gx, gy, gw, gh) = crate::ocr::preprocess::game_rect_16_9(w, h);
    let band = (gh * 70 / 1000).max(1);
    if band * 2 >= gh {
        return false;
    }
    let top_dark = dark_ratio(rgb, gx, gy, gw, band);
    let bot_dark = dark_ratio(rgb, gx, gy + gh - band, gw, band);
    let mid_y = gy + gh * 300 / 1000;
    let mid_h = gh * 400 / 1000;
    let mid_dark = dark_ratio(rgb, gx, mid_y, gw, mid_h);
    top_dark >= 0.72
        && bot_dark >= 0.72
        && mid_dark < 0.50
        && letterbox_shoulders_are_content(rgb, gx, gy, gw, gh)
        && letterbox_mid_is_footage(rgb, gx, mid_y, gw, mid_h)
}

/// Cinematic 2.35:1 bars are ~12% of 16:9. `ENTERING GAME` keeps ~35–40%
/// black above and below a thin navy strip (FrozenTear 2026-09-06 recheck on
/// rejected_preflight_013904 / poll_223050). The 7% edge bands are dark
/// in both cases; the *shoulders* just inside those edges are gameplay
/// on a real reel and still black on the loading slab.
fn letterbox_shoulders_are_content(rgb: &RgbImage, gx: u32, gy: u32, gw: u32, gh: u32) -> bool {
    let sh = (gh * LETTERBOX_SHOULDER_H_PM / 1000).max(1);
    let top = dark_ratio(
        rgb,
        gx,
        gy + gh * LETTERBOX_SHOULDER_TOP_Y_PM / 1000,
        gw,
        sh,
    );
    let bot = dark_ratio(
        rgb,
        gx,
        gy + gh * LETTERBOX_SHOULDER_BOT_Y_PM / 1000,
        gw,
        sh,
    );
    top < LETTERBOX_SHOULDER_DARK_MAX && bot < LETTERBOX_SHOULDER_DARK_MAX
}

const LETTERBOX_SHOULDER_TOP_Y_PM: u32 = 150;
const LETTERBOX_SHOULDER_BOT_Y_PM: u32 = 770;
const LETTERBOX_SHOULDER_H_PM: u32 = 80;
const LETTERBOX_SHOULDER_DARK_MAX: f32 = 0.60;

/// Reject a uniform loading / transition slab. `ENTERING GAME` is a navy
/// strip with a few white glyphs — among *lit* mid pixels one 16-wide
/// RGB bin holds ~90%+. Dark padding around that strip is ignored so a
/// thin chrome band cannot look like mixed footage. Real highlight
/// footage spreads across bins.
fn letterbox_mid_is_footage(rgb: &RgbImage, x0: u32, y0: u32, cw: u32, ch: u32) -> bool {
    mid_dominant_lit_bin_ratio(rgb, x0, y0, cw, ch) < LETTERBOX_FLAT_UI_MAX
}

const LETTERBOX_FLAT_UI_MAX: f32 = 0.82;

fn mid_dominant_lit_bin_ratio(rgb: &RgbImage, x0: u32, y0: u32, cw: u32, ch: u32) -> f32 {
    let (w, h) = rgb.dimensions();
    let x1 = (x0 + cw).min(w);
    let y1 = (y0 + ch).min(h);
    if x1 <= x0 || y1 <= y0 {
        return 1.0;
    }
    // 16-wide channel bins (16³). Coarse enough for capture noise, fine
    // enough that a character + world scene does not collapse to one bin.
    const BINS: usize = 16 * 16 * 16;
    let mut counts = [0u32; BINS];
    let mut total = 0u32;
    const STRIDE: u32 = 2;
    for y in (y0..y1).step_by(STRIDE as usize) {
        for x in (x0..x1).step_by(STRIDE as usize) {
            let [r, g, b] = rgb.get_pixel(x, y).0;
            if r.max(g).max(b) < 36 {
                continue;
            }
            let idx = ((r as usize) >> 4) * 256 + ((g as usize) >> 4) * 16 + ((b as usize) >> 4);
            counts[idx] += 1;
            total += 1;
        }
    }
    if total == 0 {
        return 1.0;
    }
    let best = counts.iter().copied().max().unwrap_or(0);
    best as f32 / total as f32
}

fn dark_ratio(rgb: &RgbImage, x0: u32, y0: u32, cw: u32, ch: u32) -> f32 {
    let (w, h) = rgb.dimensions();
    let x1 = (x0 + cw).min(w);
    let y1 = (y0 + ch).min(h);
    if x1 <= x0 || y1 <= y0 {
        return 0.0;
    }
    const STRIDE: u32 = 2;
    let mut dark = 0u32;
    let mut total = 0u32;
    for y in (y0..y1).step_by(STRIDE as usize) {
        for x in (x0..x1).step_by(STRIDE as usize) {
            let [r, g, b] = rgb.get_pixel(x, y).0;
            total += 1;
            if r.max(g).max(b) < 36 {
                dark += 1;
            }
        }
    }
    if total == 0 {
        0.0
    } else {
        dark as f32 / total as f32
    }
}

/// Title-card path: dark cinematic band + bright glyphs, then phrase OCR.
fn read_end_reel_title(img: &DynamicImage) -> bool {
    let Some(crop) = end_reel_cinematic_crop(img) else {
        return false;
    };
    if !cinematic_title_band(&crop) {
        return false;
    }
    ocr_end_reel_phrase(
        crate::ocr::preprocess::prepare_title(&crop),
        "end-reel title",
    )
}

/// Wide upper band where a cinematic POTG / highlight overlay sits.
/// High enough to miss the in-world end-title V/D word (y ~34–52%).
fn end_reel_cinematic_crop(img: &DynamicImage) -> Option<DynamicImage> {
    playfield_crop(img, 120, 80, 760, 220)
}

/// Nameplate POTG title card: gold battletag + white title glyphs.
/// Cadence wake only — same class as [`end_reel_letterbox`], no OCR. Phrase
/// OCR must not gate this wake (FrozenTear 2026-09-06: known-potg-003708 passed
/// the cheap gate, then Tesseract returned `PIAN AF TIIF AR` because the
/// endorsement toast sits on the title), and a read whose result was
/// discarded only cost two Tesseract calls per wake (#240).
fn read_end_reel_nameplate(rgb: &RgbImage) -> bool {
    nameplate_potg_signal(rgb)
}

fn playfield_crop(
    img: &DynamicImage,
    x_pm: u32,
    y_pm: u32,
    w_pm: u32,
    h_pm: u32,
) -> Option<DynamicImage> {
    let (fw, fh) = (img.width(), img.height());
    let (gx, gy, gw, gh) = crate::ocr::preprocess::game_rect_16_9(fw, fh);
    let x = gx + gw * x_pm / 1000;
    let y = gy + gh * y_pm / 1000;
    let cw = gw * w_pm / 1000;
    let ch = gh * h_pm / 1000;
    if cw == 0 || ch == 0 || x + cw > fw || y + ch > fh {
        return None;
    }
    Some(img.crop_imm(x, y, cw, ch))
}

fn ocr_end_reel_phrase(prepared: GrayImage, context: &'static str) -> bool {
    match crate::ocr::recognize_prepared(&prepared, "6", Some("ABCDEFGHIJKLMNOPQRSTUVWXYZ ")) {
        Ok(text) => {
            let hit = end_reel_title_match(&text);
            if hit {
                tracing::debug!(text = %text.trim(), context, "end-reel title OCR matched");
            } else {
                tracing::trace!(text = %text.trim(), context, "end-reel title OCR did not match");
            }
            hit
        }
        Err(e) => {
            tracing::debug!(error = %e, context, "end-reel title OCR failed");
            false
        }
    }
}

/// Cheap nameplate gate: large gold battletag in the lower-left plus
/// bright white title glyphs just above it. Does not require a dark field.
///
/// ROI + hue calibrated on known-potg-003708 (FrozenTear, 2026-09-06): the
/// first #83 gate used a mid-frame orange band (y 34–58%) and
/// `hue_near(30, 12)`. On the real 2560×1440 card the battletag sits at
/// y ≈ 990–1080 (≈69–75%) with sample `rgb(255, 221, 0)` ≈ hue 52, so
/// orange ratio was 0.0 and `detect_end_reel` returned false.
fn nameplate_potg_signal(rgb: &RgbImage) -> bool {
    // Victory banner saturates the whole playfield with the same gold
    // family as the nameplate. Reject that flood; the white-title check
    // alone is usually enough, this is the belt.
    if playfield_hit_ratio(rgb, 0, 0, 1000, 1000, is_potg_orange) > 0.30 {
        return false;
    }
    // Tab scoreboard crop (FrozenTear accepted_020455): mustard team *slab*
    // (often muted — fails is_potg_orange) plus a burgundy team above.
    // Group-count of 3+ and neon-gold hot strips miss that; a battletag
    // is a short left-aligned run, not a full-width slab.
    if nameplate_looks_like_scoreboard(rgb) {
        return false;
    }
    // Tab scoreboard: stacked gold team rows (highlighted mustard ≠ a
    // single battletag). Three or more gold-heavy bands in the lower-left
    // are a board, not PLAY OF THE GAME + name.
    if nameplate_gold_row_groups(rgb) >= 3 {
        return false;
    }
    let orange = playfield_hit_ratio(
        rgb,
        NAMEPLATE_ORANGE_X_PM,
        NAMEPLATE_ORANGE_Y_PM,
        NAMEPLATE_ORANGE_W_PM,
        NAMEPLATE_ORANGE_H_PM,
        is_potg_orange,
    );
    let white = playfield_hit_ratio(
        rgb,
        NAMEPLATE_WHITE_X_PM,
        NAMEPLATE_WHITE_Y_PM,
        NAMEPLATE_WHITE_W_PM,
        NAMEPLATE_WHITE_H_PM,
        is_title_white,
    );
    // Real known-potg-003708 orange ≈ 0.0685 (glyph, not a fill). A gold
    // scoreboard row painted into this ROI is typically >0.35. Cap sits
    // above the committed synthetic nameplate (~0.22) and below row fills.
    (NAMEPLATE_ORANGE_MIN..=NAMEPLATE_ORANGE_MAX).contains(&orange) && white >= NAMEPLATE_WHITE_MIN
}

const NAMEPLATE_ORANGE_MIN: f32 = 0.035;
const NAMEPLATE_ORANGE_MAX: f32 = 0.28;
const NAMEPLATE_WHITE_MIN: f32 = 0.012;

fn nameplate_looks_like_scoreboard(rgb: &RgbImage) -> bool {
    // Solid gold team block: many full-width hot strips. The committed
    // synthetic nameplate lights 5 strips at this threshold; a gold-team
    // slab lights ~10+. known-potg gold is a short left-side run.
    if nameplate_gold_hot_strips(rgb) >= 8 {
        return true;
    }
    let left_lower_gold = playfield_hit_ratio(rgb, 0, 500, 500, 500, is_potg_orange);
    let upper_magenta = playfield_hit_ratio(rgb, 0, 50, 1000, 430, is_team_magenta);
    // Magenta team over gold team — Tab board, not a POTG card.
    // Synthetic nameplate upper-magenta ≈ 0.003; left-lower gold ≈ 0.098.
    if upper_magenta > 0.06 && left_lower_gold > 0.04 {
        return true;
    }
    // FrozenTear accepted_020455 on 486219f: mustard team is human-bright but
    // most pixels fail is_potg_orange (val≤160 / dirtier sat). The wake
    // still sees enough neon specks + white names in the nameplate ROIs.
    // A left-aligned battletag does not paint the *right* lower half.
    let mustard_l = playfield_hit_ratio(rgb, 0, 500, 500, 500, is_scoreboard_mustard);
    let mustard_r = playfield_hit_ratio(rgb, 500, 500, 500, 500, is_scoreboard_mustard);
    if mustard_l > 0.12 && mustard_r > 0.10 {
        return true;
    }
    let maroon_up = playfield_hit_ratio(rgb, 0, 50, 1000, 430, is_scoreboard_maroon);
    maroon_up > 0.08 && mustard_l > 0.08
}

/// Tab team gold — darker / dirtier than neon POTG battletag gold.
/// Veto-only; the wake gate stays on [`is_potg_orange`].
fn is_scoreboard_mustard(r: u8, g: u8, b: u8) -> bool {
    let (hue, sat, val) = rgb_to_hsv(r, g, b);
    hue_near(hue, 48, 22) && sat > 70 && val > 70 && r > b && g > b
}

/// Tab home-team purple is often burgundy (hue ~340), outside
/// [`is_team_magenta`]'s 300±35 window. Veto-only.
fn is_scoreboard_maroon(r: u8, g: u8, b: u8) -> bool {
    let (hue, sat, val) = rgb_to_hsv(r, g, b);
    hue_near(hue, 320, 40) && sat > 40 && (21..180).contains(&val)
}

fn is_team_magenta(r: u8, g: u8, b: u8) -> bool {
    let (hue, sat, val) = rgb_to_hsv(r, g, b);
    hue_near(hue, 300, 35) && sat > 80 && val > 35
}

fn nameplate_gold_hot_strips(rgb: &RgbImage) -> u32 {
    const STRIPS: u32 = 20;
    const HOT: f32 = 0.08;
    let mut hot = 0u32;
    for i in 0..STRIPS {
        let y_pm = 400 + i * 550 / STRIPS;
        let h_pm = (550 / STRIPS).max(1);
        if playfield_hit_ratio(rgb, 0, y_pm, 1000, h_pm, is_potg_orange) > HOT {
            hot += 1;
        }
    }
    hot
}

/// Count contiguous gold-heavy horizontal strips in the lower-left half.
/// POTG nameplate ≈ 1–2 runs (title stack). Tab gold team ≈ 6 row blocks.
fn nameplate_gold_row_groups(rgb: &RgbImage) -> u32 {
    const STRIPS: u32 = 20;
    const HOT: f32 = 0.20;
    let mut groups = 0u32;
    let mut in_run = false;
    for i in 0..STRIPS {
        let y_pm = 400 + i * 550 / STRIPS;
        let h_pm = (550 / STRIPS).max(1);
        let ratio = playfield_hit_ratio(rgb, 0, y_pm, 500, h_pm, is_potg_orange);
        if ratio > HOT {
            if !in_run {
                groups += 1;
                in_run = true;
            }
        } else {
            in_run = false;
        }
    }
    groups
}
/// Lower-left battletag band (y 66–82%, x 5–45%).
const NAMEPLATE_ORANGE_X_PM: u32 = 50;
const NAMEPLATE_ORANGE_Y_PM: u32 = 660;
const NAMEPLATE_ORANGE_W_PM: u32 = 400;
const NAMEPLATE_ORANGE_H_PM: u32 = 160;
/// White "PLAY OF THE GAME" just above the name (y 58–70%).
const NAMEPLATE_WHITE_X_PM: u32 = 50;
const NAMEPLATE_WHITE_Y_PM: u32 = 580;
const NAMEPLATE_WHITE_W_PM: u32 = 420;
const NAMEPLATE_WHITE_H_PM: u32 = 120;

fn is_potg_orange(r: u8, g: u8, b: u8) -> bool {
    let (hue, sat, val) = rgb_to_hsv(r, g, b);
    // Real nameplate gold ~rgb(255,221,0) hue ~52. Window also keeps
    // deeper orange. Defeat red (~hue 0) is out. Victory-banner gold
    // (~hue 46) matches this predicate and is dropped by the flood veto.
    hue_near(hue, 48, 16) && sat > 140 && val > 160 && r > g && g > b
}

fn is_title_white(r: u8, g: u8, b: u8) -> bool {
    let (_, sat, val) = rgb_to_hsv(r, g, b);
    val > 205 && sat < 45
}

fn playfield_hit_ratio(
    rgb: &RgbImage,
    x_pm: u32,
    y_pm: u32,
    w_pm: u32,
    h_pm: u32,
    pred: impl Fn(u8, u8, u8) -> bool,
) -> f32 {
    let (w, h) = rgb.dimensions();
    let (gx, gy, gw, gh) = crate::ocr::preprocess::game_rect_16_9(w, h);
    let x0 = gx + gw * x_pm / 1000;
    let y0 = gy + gh * y_pm / 1000;
    let x1 = (x0 + gw * w_pm / 1000).min(w);
    let y1 = (y0 + gh * h_pm / 1000).min(h);
    if x1 <= x0 || y1 <= y0 {
        return 0.0;
    }
    const STRIDE: u32 = 2;
    let mut hits = 0u32;
    let mut total = 0u32;
    for y in (y0..y1).step_by(STRIDE as usize) {
        for x in (x0..x1).step_by(STRIDE as usize) {
            let [r, g, b] = rgb.get_pixel(x, y).0;
            total += 1;
            if pred(r, g, b) {
                hits += 1;
            }
        }
    }
    if total == 0 {
        0.0
    } else {
        hits as f32 / total as f32
    }
}

/// Dark cinematic field plus some bright glyphs — skips mid-fight sky/HUD
/// (bright world, low dark ratio) without a Tesseract call.
fn cinematic_title_band(crop: &DynamicImage) -> bool {
    if !title_crop_has_signal(crop) {
        return false;
    }
    let rgb = crop.to_rgb8();
    let (w, h) = rgb.dimensions();
    if w == 0 || h == 0 {
        return false;
    }
    let mut dark = 0u32;
    let mut total = 0u32;
    for y in (0..h).step_by(4) {
        for x in (0..w).step_by(4) {
            let [r, g, b] = rgb.get_pixel(x, y).0;
            total += 1;
            if r.max(g).max(b) < 50 {
                dark += 1;
            }
        }
    }
    total > 0 && (dark as f32 / total as f32) > 0.40
}

/// Phrase check for POTG / highlight-intro title OCR. Letter-only so spaces
/// and punctuation drop out; short tokens like PLAY/GAME alone do not match.
fn end_reel_title_match(raw: &str) -> bool {
    let letters: String = raw
        .chars()
        .filter(|c| c.is_ascii_alphabetic())
        .map(|c| c.to_ascii_uppercase())
        .collect();
    if letters.is_empty() {
        return false;
    }
    if letters.contains("PLAYOFTHEGAME") || letters.contains("HIGHLIGHTINTRO") {
        return true;
    }
    const TARGETS: &[&str] = &["PLAYOFTHEGAME", "HIGHLIGHTINTRO"];
    TARGETS
        .iter()
        .any(|t| letters.len() + 2 >= t.len() && strsim::levenshtein(&letters, t) <= 3)
}

/// Read the large top-left VICTORY/DEFEAT title off the post-match accolade /
/// MVP screen (shown ~15-20s after Play of the Game).
/// Region: x 0.5-25.5%, y 3.5-9.5% of the 16:9 playfield.
///
/// The right edge was 14% until 2026-08-18 (fleet::tracker-wl C6). On the
/// current title font a 2560x1440 "DEFEAT" already spans x 2.7-12.0%
/// (2026-07-15 endcards frame) and the seven wider glyphs of "VICTORY"
/// project past 14%, so the crop clipped the word to "VICTOR"/"VICTO" and
/// victories went unread while defeats still landed (store 07-31..08-17:
/// V=10 vs D=39; 08-17 was five wins, 5/5 unknown). The map name printed
/// right of the title enters the wider crop, and Tesseract PSM 7 gives up on
/// the mixed line ("" at >= 21.5% on the 05-30 reference, "DEFEATJT" on the
/// 07-15 frame), so the crop is OCR'd through `prepare_title_trimmed`, which
/// cuts the binary back to the tall title glyphs; measured on both fixtures
/// the 25% crop then reads "VICTORY" / "DEFEAT" exactly like the old 14% one.
///
/// Under `reader = "new"` the crop only gates: the word is the words-v1
/// read of the accolade layout ([`ResultWords`]).
fn read_result_word(
    img: &DynamicImage,
    rgb: &RgbImage,
    stability: Option<&mut FrameStability>,
    words: &ResultWords,
) -> MatchOutcome {
    read_outcome_word(img, rgb, &ACCOLADE_WORD, stability, words)
}

const ACCOLADE_WORD: WordScreen = WordScreen {
    x_pm: 5,
    y_pm: 35,
    w_pm: 250,
    h_pm: 60,
    context: "accolade screen",
    screen: ResultScreen::Accolade,
};

/// Read the result word off the competitive summary (rank update) screen —
/// VICTORY/DEFEAT printed under the big "COMPETITIVE" title, top-left. The
/// background is dark regardless of UI color theme, and the screen stays up
/// 40s+ (the longest-lived outcome signal, surviving even a starved poller).
/// Region measured from a real 16:9 frame: word spans x 4-12.5%, y 16-21%.
/// Right edge widened 16% -> 25% alongside the accolade crop (C6): same
/// title face, same "VICTORY is wider than DEFEAT" clipping risk — journal
/// 08-13..08-17 read DEFEAT off this screen 78 times and VICTORY zero.
///
/// Under `reader = "new"` the crop only gates: the word is the words-v1
/// read of the rank-screen layout ([`ResultWords`]).
fn read_rank_screen_result(
    img: &DynamicImage,
    rgb: &RgbImage,
    stability: Option<&mut FrameStability>,
    words: &ResultWords,
) -> MatchOutcome {
    read_outcome_word(img, rgb, &RANK_WORD, stability, words)
}

const RANK_WORD: WordScreen = WordScreen {
    x_pm: 10,
    y_pm: 145,
    w_pm: 240,
    h_pm: 80,
    context: "rank screen",
    screen: ResultScreen::RankScreen,
};

/// Read the map name printed beside the accolade screen's result word
/// ("DEFEAT  |  COLOSSEO / MATCH TIME: 10:10"). Color-scheme-independent like
/// the result word itself, and the most reliable map source when the in-game
/// top-bar OCR missed all game. Region measured on a real 16:9 frame: map
/// text block spans x 13.5-19%, y 4-8.5%; the crop starts right of the title
/// (a clipped title glyph is harmless — we only search for map names).
pub fn read_accolade_map(img: &DynamicImage) -> Option<String> {
    let (fw, fh) = (img.width(), img.height());
    let (gx, gy, gw, gh) = crate::ocr::preprocess::game_rect_16_9(fw, fh);
    let x = gx + gw * 125 / 1000;
    let y = gy + gh * 35 / 1000;
    let cw = gw * 325 / 1000;
    let ch = gh * 55 / 1000;
    if cw == 0 || ch == 0 || x + cw > fw || y + ch > fh {
        return None;
    }
    let crop = img.crop_imm(x, y, cw, ch);
    let prepared = crate::ocr::preprocess::prepare_title(&crop);
    // PSM 6 (block): the crop holds two short lines (map, match time).
    let text = crate::ocr::recognize_prepared(&prepared, "6", None).ok()?;
    let map = crate::parse::exact_map_in_text(&text);
    if let Some(m) = &map {
        tracing::info!(map = %m, raw = %text.trim(), "map read from accolade screen");
    }
    map
}

/// One result-word screen of the poller: its title crop in 1/1000ths of the
/// 16:9 playfield (via [`game_rect_16_9`]), the key of its log lines and of
/// its stability history, and the words-v1 screen that names its word under
/// `reader = "new"`.
struct WordScreen {
    x_pm: u32,
    y_pm: u32,
    w_pm: u32,
    h_pm: u32,
    context: &'static str,
    screen: ResultScreen,
}

/// The result word of one screen: VICTORY/DEFEAT/DRAW mapped to an outcome.
/// The crop's gates come first ([`gated_title_crop`]). Then `words` names
/// the word under `reader = "new"`, or OCR reads the crop
/// ([`ocr_outcome_word`]).
fn read_outcome_word(
    img: &DynamicImage,
    rgb: &RgbImage,
    at: &WordScreen,
    stability: Option<&mut FrameStability>,
    words: &ResultWords,
) -> MatchOutcome {
    let Some(crop) = gated_title_crop(img, at, stability) else {
        return MatchOutcome::Unknown;
    };
    if let Some(outcome) = words.word_on(at, rgb) {
        return outcome;
    }
    // The loaded templates cannot name this screen. Under the new reader
    // with classic OCR off there is no OCR to read the crop: the word is
    // held, and the OCR read is not asked for. The switch does not grant
    // off without both screens' words, so this is a pack that went away
    // (`ocr_switch::PackGroup::ResultWords`).
    if words.from_words() && crate::ocr::gate::is_off() {
        return MatchOutcome::Unknown;
    }
    ocr_outcome_word(&crop, at.context)
}

/// The title crop of `at`, when it is worth a read: lit and, on the poll
/// path, held still since the previous tick. `None` otherwise.
fn gated_title_crop(
    img: &DynamicImage,
    at: &WordScreen,
    stability: Option<&mut FrameStability>,
) -> Option<DynamicImage> {
    let (fw, fh) = (img.width(), img.height());
    let (gx, gy, gw, gh) = crate::ocr::preprocess::game_rect_16_9(fw, fh);
    let x = gx + gw * at.x_pm / 1000;
    let y = gy + gh * at.y_pm / 1000;
    let cw = gw * at.w_pm / 1000;
    let ch = gh * at.h_pm / 1000;
    if cw == 0 || ch == 0 || x + cw > fw || y + ch > fh {
        return None;
    }
    let crop = img.crop_imm(x, y, cw, ch);

    // P8: most idle ticks are in-game — the title crop is near-black. Skip the
    // Lanczos+Otsu+Tess pipeline when the crop has no bright glyph mass.
    if !title_crop_has_signal(&crop) {
        return None;
    }

    // PR-A: brightness cannot distinguish a lit combat frame from a real title
    // (see title_crop_has_signal), so on the poll path additionally require the
    // crop to have held still since the previous tick. Result screens are
    // static for 15-20s+; combat never is. `context` doubles as the history
    // key so the accolade and rank crops track independently.
    if let Some(stability) = stability
        && !stability.check(at.context, &crop)
    {
        let context = at.context;
        tracing::trace!(context, "title crop lit but not stable: deferring OCR");
        return None;
    }
    Some(crop)
}

/// OCR a gated title crop prepared as title text, and map
/// VICTORY/DEFEAT/DRAW to an outcome.
fn ocr_outcome_word(crop: &DynamicImage, context: &'static str) -> MatchOutcome {
    // Trimmed: the widened crops (C6) run into the map/time block right of
    // the title; the trim cuts the binary back to the title glyphs.
    let prepared = crate::ocr::preprocess::prepare_title_trimmed(crop);

    match crate::ocr::recognize_prepared(&prepared, "7", Some("ABCDEFGHIJKLMNOPQRSTUVWXYZ")) {
        Ok(text) => {
            let outcome = fuzzy_outcome_word_accolade(&text);
            if outcome.is_decided() {
                let word = match outcome {
                    MatchOutcome::Victory => "VICTORY",
                    MatchOutcome::Defeat => "DEFEAT",
                    MatchOutcome::Draw => "DRAW",
                    MatchOutcome::Unknown => "UNKNOWN",
                };
                // Same rule as the words-v1 rank read: the screen stays up
                // for many polls, so a repeat of the same outcome is quiet.
                if context != RANK_WORD.context || rank_result_is_new(outcome) {
                    tracing::info!(text = %text.trim(), context, "result word: {word}");
                }
            } else {
                tracing::trace!(ocr_text = %text.trim(), context, "no result word in region");
            }
            outcome
        }
        Err(e) => {
            // Runs every poll tick now — a broken Tesseract setup would make a
            // warn here fire every few seconds; captures fail loudly anyway.
            tracing::debug!(error = %e, context, "result word OCR failed");
            MatchOutcome::Unknown
        }
    }
}

/// Cheap pre-gate on a title crop: skip OCR only when the crop is near-black
/// (no glyph can be present). Deliberately NOT a "does this look like a title"
/// test — measured on the outcome fixtures, bright in-game frames light up
/// this region far more than a real DEFEAT title on a custom magenta UI theme
/// does (0.6% of samples at r+g+b>480 vs 15–22% on gameplay frames), so
/// brightness cannot distinguish title from game world; anything non-black
/// must go to the color-independent Otsu+Tesseract path. Max-channel is the
/// theme-independent glyph test: real titles measure ≥13% at >200 (magenta
/// defeat: 23%), while the rank crop is exactly 0% on in-game/transition
/// frames — that skip is the actual per-tick saving.
fn title_crop_has_signal(crop: &DynamicImage) -> bool {
    let rgb = crop.to_rgb8();
    let (w, h) = rgb.dimensions();
    if w == 0 || h == 0 {
        return false;
    }
    let mut lit = 0u32;
    let mut total = 0u32;
    // Sample ~every 4th pixel — enough for a go/no-go decision.
    for y in (0..h).step_by(4) {
        for x in (0..w).step_by(4) {
            let [r, g, b] = rgb.get_pixel(x, y).0;
            total += 1;
            if r.max(g).max(b) > 200 {
                lit += 1;
            }
        }
    }
    // 1% threshold = 13–23× below every measured real title.
    total > 0 && (lit as f32 / total as f32) > 0.01
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgb, RgbImage};

    fn flood(color: [u8; 3]) -> DynamicImage {
        DynamicImage::ImageRgb8(RgbImage::from_pixel(640, 360, Rgb(color)))
    }

    #[test]
    fn gold_flood_is_victory_banner() {
        assert_eq!(
            detect_outcome_signal(&flood([230, 180, 20])),
            Some((MatchOutcome::Victory, OutcomeSource::Banner))
        );
    }

    #[test]
    fn red_flood_is_defeat_banner() {
        assert_eq!(
            detect_outcome_signal(&flood([200, 30, 30])),
            Some((MatchOutcome::Defeat, OutcomeSource::Banner))
        );
    }

    #[test]
    fn black_frame_is_no_signal() {
        assert_eq!(detect_outcome_signal(&flood([0, 0, 0])), None);
        assert_eq!(detect_outcome(&flood([0, 0, 0])), MatchOutcome::Unknown);
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
        let loaded =
            Titles::for_rgb_with(true, &rgb, |_| Some(read_of(Some(Outcome::Defeat), false)));
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

        // The colour flood is read before any word, and the end title's
        // memory moves on that tick too: a banner tick with no title on it
        // breaks the run, one with the title on it continues it.
        let gold = flood([230, 180, 20]);
        let gold_rgb = gold.to_rgb8();
        let banner = Some((MatchOutcome::Victory, OutcomeSource::Banner));
        let mut st = FrameStability::default();
        assert_eq!(poll(&mut st, &defeat), None);
        assert_eq!(
            detect_outcome_signal_polled(
                &gold,
                &gold_rgb,
                &mut st,
                NEW,
                &words(None, true),
                &unread_words()
            ),
            banner
        );
        assert_eq!(poll(&mut st, &defeat), None, "the banner tick had no title");
        assert_eq!(
            detect_outcome_signal_polled(&gold, &gold_rgb, &mut st, NEW, &defeat, &unread_words()),
            banner
        );
        assert_eq!(poll(&mut st, &defeat), signal(MatchOutcome::Defeat));

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
    fn gold_flood_still_wins_over_end_title() {
        assert_eq!(
            detect_outcome_signal(&flood([230, 180, 20])),
            Some((MatchOutcome::Victory, OutcomeSource::Banner))
        );
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
                scoreline_looks_present_with(&img, &rgb, None, FinalScoreSource::Words, |_| {
                    Some(read)
                });
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
            !scoreline_looks_present_with(
                &img,
                &rgb,
                Some(&mut st),
                FinalScoreSource::Words,
                never
            ),
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
    /// templates for its screen) is logged once per screen and process.
    /// Once this test's fallbacks have run, both log lines are spent.
    #[test]
    fn a_result_word_ocr_fallback_is_logged() {
        let both = result_frame(true, true, 0);
        let mut st = seen(&both);
        let _ = poll_new(&both, &mut st, &pack_words(NO_SCREENS, None));
        for screen in ResultScreen::ALL {
            assert!(
                !warn_result_word_ocr_fallback(screen),
                "the {screen:?} fallback logged already"
            );
        }
    }
}
