use std::cell::OnceCell;
use std::sync::{Mutex, PoisonError};

use image::{DynamicImage, GrayImage, Luma, RgbImage};

use super::MatchOutcome;
use super::stability::FrameStability;
use crate::shadow::result::{Outcome, ResultRead, ResultScreen, ResultScreens, ScreenRead};

/// Which detector produced an outcome. Every source is a result word
/// (accolade screen, competitive rank screen, end title). The poller
/// confirms a result only on two agreeing reads inside a confirmation
/// window. A Tab frame still takes its outcome from one read of its own
/// frame (the accolade or rank-screen word when the session is old enough,
/// the end title, or the scoreboard OCR header line under ocr-v1), as
/// before. Who reads
/// those words depends on the reader ([`ResultWords`]): OCR under
/// `reader = "ocr-v1"`, the words-v1 pack under `reader = "new"`.
///
/// There is no colour source. A gold or red colour flood across the
/// middle of the screen used to be read as VICTORY or DEFEAT from one
/// frame; red hero effects mid-match (Vendetta, 0.5.0-rc.8 report of
/// 2026-10-22) set a game's result to Defeat while it was still being
/// played, so it was removed in 0.5.0-rc.9 (R9-1). Every real end shows
/// the result word on the end title, the accolade screen or the rank
/// screen; a player who skips all of them sets Win or Loss by hand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutcomeSource {
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
    pub fn sure_end_title(&self) -> Option<Outcome> {
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
    use std::sync::atomic::AtomicBool;
    if crate::ocr::gate::is_off() {
        return false;
    }
    static WARNED: [AtomicBool; 2] = [AtomicBool::new(false), AtomicBool::new(false)];
    log_result_word_ocr_fallback(screen, &WARNED[screen as usize])
}

fn log_result_word_ocr_fallback(
    screen: ResultScreen,
    warned: &std::sync::atomic::AtomicBool,
) -> bool {
    let first = !warned.swap(true, std::sync::atomic::Ordering::Relaxed);
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
    let rgb = crate::frame::rgb_view(img);
    detect_outcome_signal_inner(img, &rgb, None, p4, titles, words)
        .map(|(outcome, _)| outcome)
        .unwrap_or(MatchOutcome::Unknown)
}

/// Outcome detection with its evidence source, for the poller.
///
/// Order: OCR of the accolade screen's top-left result word, the rank
/// screen's word, then the end title. No colour flood is an outcome
/// ([`OutcomeSource`]). The word OCR runs unconditionally — it used to be gated behind a "60% of pixels lean
/// blue" accolade-screen check, but custom UI color schemes (e.g. magenta)
/// break any assumption about the screen's dominant color, and the full-frame
/// pixel scan cost more than the small-crop OCR it was guarding. The Otsu-based
/// `read_result_word` is color-scheme-independent.
///
/// This entry point is ocr-v1's. The daemon's go through the reader switch:
/// [`detect_outcome_signal_polled`] and [`detect_outcome_with`].
pub fn detect_outcome_signal(img: &DynamicImage) -> Option<(MatchOutcome, OutcomeSource)> {
    let rgb = crate::frame::rgb_view(img);
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
/// consecutive ticks (see [`FrameStability`]). Word
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
    read_end_title(img, &crate::frame::rgb_view(img), None, p4, titles)
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

/// Cheap Play of the Game / highlight end-reel hint for the poller.
///
/// Not an outcome confirm: that takes two agreeing word reads.
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

pub(super) const LETTERBOX_SHOULDER_TOP_Y_PM: u32 = 150;
pub(super) const LETTERBOX_SHOULDER_BOT_Y_PM: u32 = 770;
pub(super) const LETTERBOX_SHOULDER_H_PM: u32 = 80;
pub(super) const LETTERBOX_SHOULDER_DARK_MAX: f32 = 0.60;

/// Reject a uniform loading / transition slab. `ENTERING GAME` is a navy
/// strip with a few white glyphs — among *lit* mid pixels one 16-wide
/// RGB bin holds ~90%+. Dark padding around that strip is ignored so a
/// thin chrome band cannot look like mixed footage. Real highlight
/// footage spreads across bins.
pub(super) fn letterbox_mid_is_footage(rgb: &RgbImage, x0: u32, y0: u32, cw: u32, ch: u32) -> bool {
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

pub(super) fn dark_ratio(rgb: &RgbImage, x0: u32, y0: u32, cw: u32, ch: u32) -> f32 {
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
#[path = "tests_match_end.rs"]
mod tests;
