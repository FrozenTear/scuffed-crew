//! New-game boundaries for the stat-tracker session machine.
//!
//! One session stays open across hero swaps. It closes when the match is over
//! and the next queue shows up. The 2026-10-05 requeue is the shape this
//! machine is for: Busan, Zenyatta, defeat, then Junkertown, Wrecking Ball,
//! victory, with no map vote in between. The Junkertown stats (last board
//! 23/5/9, 6792/1463/3006) must not land on the Busan session.
//!
//! A hint stays sealable until a second board with progressed stats is
//! accepted after it, or one progressed board after a hero select has armed
//! a boundary, or until a reset, a gap, a Tab that names a different map,
//! an end screen whose map and the session map are both trusted and
//! differ, or an unblocked map vote or hero ban seals it onto this
//! session. The first progressed board keeps the hint. "Progressed" is
//! measured from the reset baseline: a counter moved forward, the totals
//! are not the same, the frame is not a decided result header, and the
//! jump is not implausible. Wall-clock time is the caller's 75-second
//! post-match grace, the 120-second stat gap, the 45-second wait before
//! the first reset board counts, the 20-minute unfinished-session idle
//! bound, and the map-vote debounce.
//!
//! # States
//!
//! | State | Meaning |
//! |---|---|
//! | Idle | No live board, no hint, and no confirmed result. |
//! | LiveMatch | A board, map, or hero is stored and no sealable hint is open. |
//! | PostResultStreak | A result word is remembered. The hint is the streak. |
//! | PostMatch | The result is confirmed. Grace starts at this confirmation. |
//! | NewGameStarting | A start screen opened the next session and its first board has not arrived. |
//!
//! Idle and LiveMatch share every poll row. They differ on a scoreboard:
//! LiveMatch can defer a fresh-match reset because it has a mature board.
//! Idle has no mature board, so that read is stored as the first board.
//!
//! # Transitions
//!
//! `Seal` records a result on this session. `Split` closes it and opens the
//! next one. `Append` stores the board on this session. `Defer` holds the
//! first fresh-match board off this session until a second fresh board
//! commits the split. `ArmPending` is a hero select after a board-followed
//! hint: it is the first reset signal and does not seal. The next fresh
//! board then splits and seals. A progressed board drops the hint.
//! `PrimeReset` is a hero select before any fresh board, with no hint to
//! arm. `Ignore` is a poll observation this phase does not act on. A stored
//! scoreboard is never `Ignore`: every parsed row is folded, and an
//! implausible jump is stored without becoming the reset baseline.
//!
//! A fresh-match board is the same identified row as the baseline, with
//! clean elims, deaths, and damage, not an implausible jump, at least 45
//! seconds after the last accepted board, compared with a mature board
//! (elims ≥ 4 or damage ≥ 400), with elims ≤ max(2, prev/4), deaths ≤
//! max(1, prev/4), and damage ≤ prev/4. A different row never counts.
//! `row_id: None` never counts. A hero select or ban before that board is
//! the first signal. A select after a deferred board is not a second signal.
//!
//! | State | Input | Effect |
//! |---|---|---|
//! | Idle, LiveMatch | confirmed word | Seal. Grace starts now. |
//! | Idle, LiveMatch | unconfirmed word | Remember the hint when the maps match, or when the boundary is not armed and either side has no trusted name. A different map is ignored. |
//! | Idle, LiveMatch | map vote, not blocked | Split. No seal. The same-map-plus-hero guard and the debounce are the caller's block. |
//! | Idle, LiveMatch | hero ban, no deferred board | Split at once. No seal. |
//! | Idle, LiveMatch | hero ban after a deferred board | Ignore. One misread plus a ban is not a split. |
//! | Idle, LiveMatch | hero select, no deferred board | Prime the reset streak. Do not split. |
//! | Idle, LiveMatch | hero select after a deferred board | Ignore. |
//! | Idle | scoreboard | Append. There is no mature board to reset from. |
//! | LiveMatch | fresh-match board, streak already primed | Split. New outcome is Unknown. LiveMatch has no hint to seal. |
//! | LiveMatch | fresh-match board, streak clear | Defer the first. Split on the second fresh board. |
//! | LiveMatch | other scoreboard | Append. An unidentified or implausible row leaves a held board and the streak. A plausible same-row board refreshes the baseline, unless it is fresh-shaped against a mature baseline: that board is a reset signal arriving early, and it leaves the baseline, the streak and a held board. A different row that is not fresh-shaped re-anchors it. |
//! | Idle | 120s gap | Not applicable. Ignore. |
//! | LiveMatch | 120s gap | Split. The new session keeps this frame's header. The caller's guard is the same map plus the same hero. |
//! | Idle | end screen, different map | Not applicable. Ignore. |
//! | LiveMatch | end screen, both maps trusted and different | Split, armed or not. The new session takes the end-screen outcome. If either side has no trusted name, this is a confirmed word and it seals onto this session. |
//! | PostResultStreak | confirmed word | Seal that word. Do not split. Grace starts now. |
//! | PostResultStreak | unconfirmed word | Replaces the hint when the maps match, or when the boundary is not armed and either side has no trusted name. A different map does not replace the hint. While armed, a word where either side has no trusted name does not replace the hint. |
//! | PostResultStreak | map vote, not blocked, or hero ban | Split and seal the hint. |
//! | PostResultStreak | hero select, no board yet | Seal the hint and split. |
//! | PostResultStreak | hero select after a board | Arm a pending boundary. That arm is the first reset signal. |
//! | PostResultStreak | first progressed board | Append. The hint stays. |
//! | PostResultStreak | second progressed board, or one after an arm | Clear the hint and append. |
//! | PostResultStreak | same totals, or a decided header | Append. The hint stays. |
//! | PostResultStreak | fresh-match board | Defer, then split on the next fresh board. The split seals the hint. An armed streak splits on this board. |
//! | PostResultStreak | 120s gap | Split. Seal the hint. Keep this frame's header. |
//! | PostResultStreak | end screen, both maps trusted and different | Split, armed or not. Seal the old hint. The new session takes the end-screen outcome and that map, including a map carried from the previous read of the same outcome. That map is re-stored on every word read (accolade, rank screen or end title) and kept while the previous read is inside the confirm window. If either side has no trusted name, the word seals onto this session. A confirmation with no map (rank screen or end title reads only) still seals onto this session. The close is an end-screen map, counted apart from a stat regression. |
//! | PostResultStreak | hinted Tab, different map | Split. Seal the hint. The new session keeps this frame's header. [`plan_capture`] builds [`Obs::HintedDifferentMap`] only when the stored map is the top bar or the accolade and the gap has elapsed, or when the session has no board yet. A text-fallback map does not build it. |
//! | PostMatch | word | Ignore the outcome. Adopt an accolade map when this session has none. Do not rewrite a text-fallback map once the outcome is recorded. |
//! | PostMatch | start screen, not blocked | Split. No seal. The new session is Unknown and has no grace. |
//! | PostMatch | scoreboard, not a fresh-match reset | Append. A confirmed mark is not cleared. |
//! | PostMatch | fresh-match board | Defer, then split. The deferred board is stored on the new session. |
//! | PostMatch | 120s gap | Ignore. |
//! | PostMatch | end screen | Ignore. |
//! | NewGameStarting | confirmed word | Seal. This session leaves the starting phase. |
//! | NewGameStarting | unconfirmed word | Remember the hint and leave the starting phase. |
//! | NewGameStarting | hero select | Ignore, including after the debounce. A swap before the first Tab is this game. |
//! | NewGameStarting | map vote or hero ban, debounce still open | Ignore. Vote candidates stay on this session. |
//! | NewGameStarting | map vote or hero ban, debounce elapsed | Split. No seal. A map vote replaces the candidates. |
//! | NewGameStarting | scoreboard | Append. Enter LiveMatch. The first Tab stays on this session. |
//! | NewGameStarting | 120s gap | Not applicable. Ignore. The previous game is not an anchor. |
//! | NewGameStarting | end screen, different map | Ignore. Practically unreachable: a start screen has not stored a trusted pair the end screen can differ from. |

use std::time::{Duration, Instant};

use crate::capture_gate::{self, Counters, GATE_COLS, GateState};
use crate::model::MatchOutcome;

/// The first fresh-match board has to land at least this long after the
/// board it is compared with (the reset baseline, or the gate when there is
/// none yet). A drop a few seconds later is still this match. A row of
/// another kind stored in between (an unidentified row, another player's)
/// does not restart this clock: it says nothing about the own row.
pub const RESET_MIN_AGE: Duration = Duration::from_secs(45);

/// Why a session closed. The log line is [`CloseReason::log`]; callers match
/// the enum, not the text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseReason {
    MapVote,
    HeroSelectOrBan,
    StatReset,
    StatRegression,
    /// Both maps are trusted and they differ on an end-screen word.
    /// Counted apart from a stat regression. Correct requeues use this
    /// same reason. The daemon log also records whether a boundary was
    /// armed, the sealed hint, and the accolade map.
    EndScreenMap,
}

impl CloseReason {
    pub fn log(self) -> &'static str {
        match self {
            Self::MapVote => "superseded by map vote",
            Self::HeroSelectOrBan => "superseded by hero select/ban",
            Self::StatReset => "superseded by stat reset",
            Self::StatRegression => "superseded by stat regression",
            Self::EndScreenMap => "superseded by end-screen map",
        }
    }
}

/// The end-screen split, as the daemon log prints it.
///
/// `armed` is whether a boundary was already pending. `sealed_hint` is the
/// hint stored on the session being closed. `accolade_map` is the name the
/// new session takes; this close always has one. A missing hint prints
/// `none`, the same spelling as a present hint's name, not `Some` or `None`.
pub fn format_end_screen_split(
    armed: bool,
    sealed_hint: Option<MatchOutcome>,
    accolade_map: &str,
) -> String {
    let hint = match sealed_hint {
        Some(outcome) => outcome.to_string(),
        None => "none".to_string(),
    };
    format!("armed={armed} sealed_hint={hint} accolade_map={accolade_map}")
}

/// Arm flag and accolade map for an end-screen split log.
///
/// `None` unless this open is an end-screen close that carries a map. The
/// map is required: this close always has one, so there is no `none` stand-in.
pub fn end_screen_log_fields(armed: bool, open: &OpenNew) -> Option<(bool, String)> {
    if open.reason != CloseReason::EndScreenMap {
        return None;
    }
    Some((armed, open.new_map.clone()?))
}

/// End-screen evidence kept on the session until a second progressed board
/// clears it, or one progressed board after an arm, or a confirmed read
/// seals it.
///
/// `confirmed == false` is a hint. A reset, a gap, a different-map Tab, a
/// trusted different-map end screen, or a no-board start screen seals it.
/// An unconfirmed word replaces it when the maps match, or when the
/// boundary is not armed and either side has no trusted name. That unarmed
/// exception lets a rank screen or an end title with no map replace the
/// hint, and a second such read then seals it onto this session. While a
/// boundary is armed, a word where either side has no trusted name does
/// not replace the hint: a real result read once with no map is dropped,
/// and the earlier false hint is sealed by the next vote. A different map
/// never replaces the hint. A confirming read whose map and the session
/// map are both trusted and differ opens the next session, armed or not.
/// That close is logged as an end-screen map, apart from a stat
/// regression, with whether a boundary was armed, the sealed hint, and
/// the accolade map. A confirmation with no map (rank screen or end title
/// reads only) still seals onto this session. Two mapless reads do the
/// same, including when a Tab capture is in flight and the accolade crop
/// is skipped. An idle close does not seal the hint. A full-board text
/// fallback is not a map for the split. An accolade may replace that name
/// only while no boundary signal has been seen yet: no arm, no start
/// screen, no primed or held reset, the outcome not recorded, and no hint
/// of a different result. Those checks are not the same as "this session's
/// own end screen". A hint that matches the next result, or a session with
/// no hint, can still take the next game's accolade. Replacing a hint of a
/// different result sets a lock that closes the window, so the confirming
/// read cannot relabel the map. A misread end title that sets that lock,
/// and an arm, keep the text name. The tick that reads this session's own
/// accolade records the result then, so the hint is not cleared and the
/// text name stays. The text name changes only when progressed boards
/// clear the hint before that end screen. Clearing the hint clears the
/// lock, except when recovery drops the hint because its timestamp cannot
/// be mapped onto this boot: the lock stays. A disagreement between a
/// trusted top bar and a trusted accolade inside one match opens a new
/// session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResultMark {
    pub outcome: MatchOutcome,
    pub confirmed: bool,
    pub seen_at: Instant,
}

/// Where the open session sits. Derived from [`BoundaryState`]; stored only
/// as the fields that derivation reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Idle,
    LiveMatch,
    PostResultStreak,
    PostMatch,
    NewGameStarting,
}

/// A screen that starts the next match. Hero ban is first-class: recording
/// which heroes were banned stays on
/// [`crate::detect::match_start::detect_ban_screen`] (future work). This
/// variant only answers "the ban UI is up, so the previous match is over."
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StartScreen {
    MapVote { candidates: Vec<String> },
    HeroSelect,
    HeroBan,
}

/// One poll tick, as the session machine sees it.
#[derive(Clone, Debug)]
pub struct PollInput<'a> {
    pub outcome: MatchOutcome,
    pub result: Option<ResultMark>,
    /// A hero select after a board-followed hint is waiting for the next board.
    /// A vote or a ban does not arm; only a hero select does.
    pub pending_boundary: bool,
    /// The session was opened by a start screen and has no board yet.
    pub awaiting_first_board: bool,
    /// A scoreboard has been accepted. A hero select then arms instead of sealing.
    pub has_board: bool,
    /// Consecutive fresh-match boards already deferred.
    pub reset_streak: u32,
    pub map: Option<&'a str>,
    /// False when `map` is a full-board text fallback, or a pre-0.4.19
    /// skeleton that had a map and no source. [`decide_poll`] treats that
    /// map as absent, so a different accolade does not split. An accolade
    /// may replace the stored name only while no boundary signal has been
    /// seen yet and [`Self::text_fallback_locked`] is false. A hint that
    /// matches the next result, or no hint at all, can still take the next
    /// game's accolade. The lock is cleared when the hint is cleared,
    /// except when recovery drops the hint because its timestamp cannot be
    /// mapped onto this boot: the lock stays.
    pub map_trusted: bool,
    pub hero: Option<&'a str>,
    pub signal: Option<MatchOutcome>,
    /// The second agreeing word inside the confirm window.
    pub signal_confirmed: bool,
    pub accolade_map: Option<&'a str>,
    pub start_screen: Option<StartScreen>,
    /// Map vote that the same-map guard or the debounce refused.
    pub block_map_vote: bool,
    /// A fresh-match board is held off this session. The poll path has to
    /// say so: `start_screen` ignores a ban or select after that hold.
    pub deferred: bool,
    /// A different result already replaced the hint. The accolade that came
    /// with that result must not rewrite a text-fallback map on the
    /// confirming tick, after the hint no longer shows the disagreement.
    pub text_fallback_locked: bool,
    pub now: Instant,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenNew {
    pub reason: CloseReason,
    /// Outcome to store on the session being closed when it does not already
    /// have one (a defeat streak the second poll never confirmed).
    pub seal_outcome: Option<MatchOutcome>,
    pub new_outcome: MatchOutcome,
    pub new_map: Option<String>,
    pub candidates: Vec<String>,
    pub new_result: Option<ResultMark>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpdateCurrent {
    pub record_outcome: Option<MatchOutcome>,
    pub adopt_map: Option<String>,
    pub result: Option<ResultMark>,
    /// Hero select after a board-followed hint. The first reset signal.
    pub arm_pending: bool,
    /// Hero select before any fresh board, with no hint to arm.
    pub arm_reset: bool,
    /// A result word on a session that was waiting for its first board.
    pub clear_awaiting: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PollDecision {
    Keep,
    /// A contradictory word arrived and this session keeps its result.
    IgnoreContradictory {
        kept: MatchOutcome,
        ignored: MatchOutcome,
    },
    Update(UpdateCurrent),
    Open(OpenNew),
}

/// What one observation does. Every word, start screen, and Tab goes through
/// [`transition`], including an armed word whose map does not match and a
/// hinted Tab whose map differs. A tick with no word and no start screen
/// adopts a map or keeps the session. The wrappers only pack and apply this.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    Ignore,
    Append,
    /// Record `outcome` on this session. The caller stamps grace at `now`.
    Seal {
        outcome: MatchOutcome,
    },
    RememberHint {
        outcome: MatchOutcome,
    },
    /// The second progressed board after a hint, or the first after an arm.
    /// The match continued.
    ClearHintAndAppend,
    /// First fresh-match board. Not appended to the current session.
    Defer,
    /// Hero select after a board-followed hint. The first reset signal, not a seal.
    ArmPending,
    /// Hero select before any fresh board. Not a split.
    PrimeReset,
    /// First progressed board after a hint. The hint stays.
    CountProgress,
    Split {
        reason: CloseReason,
        seal: Option<MatchOutcome>,
        /// Outcome of the session being opened. Unknown unless this is a
        /// gap split that keeps the current frame's own header.
        new_outcome: MatchOutcome,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transition {
    pub effect: Effect,
    pub phase: Phase,
}

/// One scoreboard, as [`transition`] sees it. The flags are computed by
/// [`plan_capture`] from the raw counters.
#[derive(Clone, Copy, Debug)]
pub struct BoardObs {
    /// Identified row whose counters sit under the fresh-match thresholds
    /// versus a mature board, at least [`RESET_MIN_AGE`] later, and not an
    /// implausible jump.
    pub fresh_reset: bool,
    /// A counter moved forward, and the jump is plausible.
    pub progressed: bool,
    /// Growth past the time-scaled ceiling. Stored, and not a baseline.
    pub implausible: bool,
    pub frame_outcome: MatchOutcome,
    /// `reader = "new"`: the own row is suspect ([`BoardDoubt::suspect_row`]).
    /// A fresh-match board on such a row never commits a split by itself.
    pub suspect_row: bool,
}

/// An observation the machine can accept.
#[derive(Clone, Debug)]
pub enum Obs<'a> {
    ConfirmedWord {
        outcome: MatchOutcome,
    },
    UnconfirmedWord {
        outcome: MatchOutcome,
        /// How this read's map sits against the session map. A text-fallback
        /// session map is [`MapRelation::Absent`].
        relation: MapRelation,
    },
    StartScreen {
        screen: &'a StartScreen,
        block_map_vote: bool,
    },
    Board(BoardObs),
    /// The caller already measured a 120s gap and a real stat regression.
    Gap {
        frame_outcome: MatchOutcome,
    },
    EndScreenDifferentMap {
        outcome: MatchOutcome,
        map: &'a str,
    },
    /// A hinted session's Tab named a different map. [`plan_capture`] only
    /// builds this after [`hinted_different_map`] accepts the pair.
    HintedDifferentMap {
        frame_outcome: MatchOutcome,
    },
}

/// How an accolade map sits against the session map. Empty and `unknown`
/// are not names. A text-fallback session map is not compared.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapRelation {
    /// Both sides name a map, and the names agree.
    Matches,
    /// Both sides name a map, and the names differ.
    Differs,
    /// Either side has no trusted name. While a boundary is armed this does
    /// not replace the hint. While it is not, the word still replaces the hint.
    Absent,
}

/// Where a stored map name was read. The board case of a different-map Tab
/// trusts the top bar and the accolade. A full-board text fallback does not
/// name the match, and the poll path treats it as no map.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum MapSource {
    TopBar,
    Accolade,
    TextFallback,
}

impl MapSource {
    /// A map this reliable may close the session when a later Tab disagrees.
    pub fn trusted_for_board_split(self) -> bool {
        matches!(self, Self::TopBar | Self::Accolade)
    }
}

/// Whether `cur` is a real, sharp fall from the accepted counter: under half,
/// past the OCR jitter band, and not an edge-ink suspect read.
fn sharp_vote(acc: u32, raw: u32, cur: u32, cur_suspect: bool, raw_suspect: bool) -> bool {
    if cur_suspect || acc < 4 || cur.saturating_mul(2) >= acc {
        return false;
    }
    if raw_suspect {
        true
    } else {
        capture_gate::raw_dropped(raw, cur)
    }
}

/// After an end screen, a sharp drop across several cumulative columns is a
/// new game. Two of elims/deaths/damage, or any three of the six counters.
/// One column is still an OCR miss — the capture gate holds that cell.
///
/// Reset detection itself uses the fresh-match thresholds in [`plan_capture`].
/// This is the multi-column drop the 120-second gap compares.
pub fn post_result_stat_reset(
    prev: &GateState,
    cur: Counters,
    cur_suspect: [bool; GATE_COLS],
) -> bool {
    let acc = prev.accepted.to_array();
    let raw = prev.last_raw.to_array();
    let now = cur.to_array();
    let mut sharp = [false; GATE_COLS];
    for col in 0..GATE_COLS {
        sharp[col] = sharp_vote(
            acc[col],
            raw[col],
            now[col],
            cur_suspect[col],
            prev.last_raw_suspect[col],
        );
    }
    // edd order matches `Counters::edd`: elims, deaths, damage = cols 0, 2, 3.
    let edd = u8::from(sharp[0]) + u8::from(sharp[2]) + u8::from(sharp[3]);
    let n = sharp.iter().filter(|s| **s).count();
    edd >= 2 || n >= 3
}

/// What a Tab past the post-match grace window of a finished game shows
/// ([`finished_game_tab`]). 0.5.0-rc.9, R9-2.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FinishedGameTab {
    /// Not shown to be the same game: the next game, as before rc.9.
    NewGame,
    /// The same game's board with nothing moved on, or only one column (one
    /// column can be a misread): a late look at the same board. The game
    /// keeps its result, and the board is not stored (it adds nothing, and
    /// a one-column rise may be a misread).
    SameBoard,
    /// The same game, still being played: a board accepted after the result
    /// was recorded has since moved on in two or more columns, and none fell
    /// except at most one by an OCR-sized step. The recorded result was
    /// false (a mid-match frame taken for an end).
    StillPlaying,
}

/// The last accepted board is far enough into a match that neither a new
/// game nor a late quick-play backfill can be mistaken for it soon after:
/// elims at least 4, deaths at least 2 and damage at least 1500, all with a
/// number. Stricter than the reset rule's [`mature_board`], because a wrong
/// merge here clears a real result.
fn probe_mature(prev: &Counters, known: &[bool; GATE_COLS]) -> bool {
    known[0] && known[2] && known[3] && prev.elims >= 4 && prev.deaths >= 2 && prev.damage >= 1500
}

/// Whether a Tab past the grace window of a finished game reads that same
/// game's board. `prev` is the game's last accepted board, `elapsed` the time
/// since it was accepted, `board_after_result` whether that board was
/// accepted after the result was recorded.
///
/// On 2026-10-22 a red mid-match frame was taken for DEFEAT, and the first
/// Tab after the grace window opened a new session for a board on the same
/// map whose elims and deaths were still rising (9/8 taken after the false
/// result, then 10/9): one game split in two. A board is the same game only
/// when every check below holds; anything that cannot be compared is
/// [`FinishedGameTab::NewGame`], the rule before rc.9:
///
/// * the map read on this Tab names the game's map;
/// * the own row is identified, its cells parsed, and not suspect;
/// * elims, deaths and damage have a number on both boards: read on this
///   Tab (not held, not edge-ink suspect) and confirmed on the last
///   accepted board;
/// * the last accepted board is mature ([`probe_mature`]), so a new game's
///   board, a quick-play backfill included, can never look like it;
/// * the board is judged as a set, not on one value: no compared column is
///   below the last accepted board, except that one column may be a little
///   below it (one kill-column count, or a tenth of a wide column), an OCR
///   misread, when two or more others moved on. A new game starts every
///   column back near zero, so it fails this on several columns at once;
/// * no growth above the rate ceiling for `elapsed`.
///
/// A board that moved on in two or more columns is
/// [`FinishedGameTab::StillPlaying`] only when the board it is compared with
/// was accepted after the result. Compared with a board from before the
/// result, the rise is what any real end looks like (the last fight between
/// that Tab and the end), so it is not evidence against the result:
/// [`FinishedGameTab::NewGame`], as before rc.9, which keeps the result.
///
/// The caller asks only while the last accepted board is recent (main's
/// `FINISHED_GAME_TAB_MAX_GAP`).
#[allow(clippy::too_many_arguments)]
pub fn finished_game_tab(
    prev: &GateState,
    session_map: Option<&str>,
    incoming_map: Option<&str>,
    cur: &Counters,
    suspect: &[bool; GATE_COLS],
    held: &[bool; GATE_COLS],
    own_row_trusted: bool,
    elapsed: Duration,
    board_after_result: bool,
) -> FinishedGameTab {
    let same_map = matches!(
        (named_map(session_map), named_map(incoming_map)),
        (Some(session), Some(incoming)) if session.eq_ignore_ascii_case(incoming)
    );
    if !same_map || !own_row_trusted {
        return FinishedGameTab::NewGame;
    }
    let known: [bool; GATE_COLS] = std::array::from_fn(|col| {
        !held[col] && !suspect[col] && !prev.unread[col] && !prev.unconfirmed[col]
    });
    if !probe_mature(&prev.accepted, &known) {
        return FinishedGameTab::NewGame;
    }
    let before = prev.accepted.to_array();
    let now = cur.to_array();
    let fell: Vec<usize> = (0..GATE_COLS)
        .filter(|&col| known[col] && now[col] < before[col])
        .collect();
    // An OCR-sized step down: one count in elims, assists or deaths, a
    // tenth of damage, healing or mitigation.
    let small_step = |col: usize| {
        let drop = before[col] - now[col];
        if col <= 2 {
            drop <= 1
        } else {
            drop.saturating_mul(10) <= before[col]
        }
    };
    if fell.len() > 1 || fell.iter().any(|&col| !small_step(col)) {
        return FinishedGameTab::NewGame;
    }
    // A column only this Tab has no number for is left out of the ceiling
    // too: `implausible_growth` skips held and never-read columns.
    let skipped: [bool; GATE_COLS] = std::array::from_fn(|col| !known[col]);
    if implausible_growth(prev, cur, &skipped, elapsed) {
        return FinishedGameTab::NewGame;
    }
    let grew = (0..GATE_COLS)
        .filter(|&col| known[col] && now[col] > before[col])
        .count();
    match (grew >= 2, fell.is_empty()) {
        (true, _) if board_after_result => FinishedGameTab::StillPlaying,
        // Rising past a board from before the result: the game's own final
        // board, or another game. Not evidence against the result.
        (true, _) => FinishedGameTab::NewGame,
        (false, true) => FinishedGameTab::SameBoard,
        // A small drop with nothing else moving is not shown to be anything.
        (false, false) => FinishedGameTab::NewGame,
    }
}

/// Inputs for one accepted scoreboard capture. `main` fills this from the
/// open session and the frame; tests call [`plan_capture`] with the same struct.
pub struct CapturePlanInput<'a> {
    pub prev_gate: Option<&'a GateState>,
    /// Counters the reset is measured from. Set when the first fresh-match
    /// board is held. A refresh replaces them. An unidentified or implausible
    /// row leaves them.
    pub baseline: Option<&'a GateState>,
    pub streak: u32,
    pub cur: Counters,
    pub suspect: [bool; GATE_COLS],
    pub create_session: bool,
    pub suppress_same_unfinished: bool,
    pub age: Option<Duration>,
    pub min_gap: Duration,
    pub classic_regressed: bool,
    /// True only when this capture's stats came from the identified player
    /// row (`parse::row_counts`). A row with cells the new reader held still
    /// counts: [`plan_capture_held`] leaves those columns out.
    pub row_counts: bool,
    /// Index of that identified row. `None` (the raw-text fallback) is stored
    /// and never counts toward a reset.
    pub row_id: Option<u32>,
    /// Row index that established the current baseline.
    pub baseline_row: Option<u32>,
    /// The session outcome is already confirmed.
    pub confirmed_end: bool,
    /// Outcome of the session this Tab was requested for. Never written onto
    /// a session a reset split opens.
    pub inherited_outcome: MatchOutcome,
    /// Result read off this Tab frame (a result word, the end title, the
    /// scoreboard header). A gap split keeps it.
    /// A reset split does not.
    pub frame_outcome: MatchOutcome,
    /// Hero already stored on the session. Not a split signal.
    pub session_hero: Option<&'a str>,
    /// Unconfirmed decided word already on the session. Not invented here.
    pub hint: Option<MatchOutcome>,
    /// Time since the reset baseline was taken. Rate checks use this, not
    /// [`Self::age`], which is time since the last stored row.
    pub baseline_age: Option<Duration>,
    /// Progressed boards already accepted since the current hint.
    pub progressed_boards: u8,
    /// A hero select armed a boundary after a board-followed hint.
    pub pending_boundary: bool,
    /// Opened by a start screen; the first board has not been accepted.
    pub awaiting_first_board: bool,
    /// Map already stored on the session.
    pub session_map: Option<&'a str>,
    /// Where `session_map` was read. The board case of a different-map split
    /// runs only for [`MapSource::TopBar`] and [`MapSource::Accolade`].
    pub session_map_source: Option<MapSource>,
    /// Map read on this Tab. A confident difference from `session_map` on a
    /// hinted session seals the hint: on the first board when the session
    /// has none yet, and after the gap when the stored map is trusted.
    pub incoming_map: Option<&'a str>,
}

/// What [`plan_capture`] decided. `main` stores `stored_outcome` on the
/// session the row lands on and keeps the streak fields for the next Tab.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapturePlan {
    pub split: bool,
    /// First fresh-match board. The caller must not write this row onto the
    /// current session. It is held and written onto the new session when the
    /// reset commits.
    pub defer: bool,
    /// Do not move the gate or the baseline. A stored row is never ignored.
    pub ignore_row: bool,
    /// `defer` only. `handle_capture` returns before the store insert.
    pub skip_store: bool,
    pub clear_hint: bool,
    pub reset_streak: u32,
    pub reset_baseline: Option<GateState>,
    /// Row that owns `reset_baseline` after this capture.
    pub baseline_row: Option<u32>,
    /// The accepted gate of this capture becomes the baseline.
    pub refresh_baseline: bool,
    /// First progressed board after a hint. The hint stays; the count moves.
    pub count_progress: bool,
    pub stored_outcome: MatchOutcome,
    /// Counters to hold until a reset split stores them on the new session.
    pub deferred_counters: Option<Counters>,
    /// Hint to seal on the session being closed, when `split` is set.
    pub seal: Option<MatchOutcome>,
    pub close_reason: Option<CloseReason>,
}

/// A confident map name. Empty and `unknown` are not a map.
fn named_map(map: Option<&str>) -> Option<&str> {
    map.map(str::trim)
        .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("unknown"))
}

/// A hinted session whose Tab names a different map is the next game.
///
/// A session with no board of its own splits on that first board. Its map
/// came from the accolade on the hint tick; without that read there is no
/// stored map to differ from. A session that already has a board waits out
/// the gap, and only when the stored map was the top bar or the accolade.
/// A full-board text fallback is not that map. The same map does not split:
/// a late Tab of this match names the map already stored.
fn hinted_different_map(input: &CapturePlanInput<'_>) -> bool {
    if input.confirmed_end || input.hint.filter(|outcome| outcome.is_decided()).is_none() {
        return false;
    }
    match (named_map(input.session_map), named_map(input.incoming_map)) {
        (Some(session), Some(incoming)) if !session.eq_ignore_ascii_case(incoming) => {
            if input.prev_gate.is_none() {
                return true;
            }
            input
                .session_map_source
                .is_some_and(MapSource::trusted_for_board_split)
                && input.age.is_some_and(|age| age >= input.min_gap)
        }
        _ => false,
    }
}

/// Decide whether this capture closes the session, and which outcome the
/// landing session stores. The decision is [`transition`].
pub fn plan_capture(input: &CapturePlanInput<'_>) -> CapturePlan {
    plan_capture_held(input, BoardDoubt::default())
}

/// What `reader = "new"` knows about one board beyond its counters. All
/// false under ocr-v1.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BoardDoubt {
    /// Columns this capture has no number for.
    pub held: [bool; GATE_COLS],
    /// The own row is suspect (alpha.7 O9): one voice named it where two
    /// could have, or a weak highlight alone. It may be another player's.
    pub suspect_row: bool,
    /// A trusted fresh-match board is already held off this game, to be
    /// carried onto the next one.
    pub board_deferred: bool,
}

/// [`plan_capture`] for a board with cells the reader did not vouch for
/// (`reader = "new"`).
///
/// `held[col]` means this capture has no number for the column;
/// `input.cur` carries the game's last accepted value there, and
/// `input.suspect` has the column set. A column the game has never read
/// ([`GateState::unread`] on the board it is compared with) has no number on
/// that side either. Such a column abstains: it is left out of every
/// comparison, so it neither shows a move nor vetoes one.
///
/// * A fresh-match reset is decided on the elims, deaths and damage columns
///   that both boards have a number for. At least two of the three are
///   needed, all of them under the fresh-match thresholds, and the earlier
///   board has to be mature on one of them. A held column no longer blocks
///   the reset, and a board with a held cell re-anchors the reset baseline
///   like any other identified row. Before this, one held cell anywhere kept
///   the baseline unset for the whole game, and a missed game end could not
///   be recovered by the reset rule at all (F266-2).
/// * Growth is measured over the time since the column's last accepted read
///   ([`GateState::held_secs`]), and a never-read column is not growth: its
///   placeholder 0 is not a number (F266-5).
/// * A fresh-match board with a held cell is held off this session like any
///   other, and counts as the first reset signal, but it is not carried
///   onto the next session as a row: it has no number for that cell.
///
/// # A suspect own row
///
/// A suspect own row ([`BoardDoubt::suspect_row`]) may be another player's.
/// It takes part in the plan like this:
///
/// * **It anchors.** On the baseline's own row, or when the game has no
///   baseline yet, it re-anchors the reset baseline like any identified row.
///   Without that, a game whose every Tab has a suspect row has no baseline,
///   no reset can fire for it, and the next game's boards are appended to it
///   (R3-1). It never moves the baseline to another row.
/// * **It can say "reset", never alone.** A fresh-match board on a suspect
///   row is held off the game and counts as a reset signal, but it is not
///   carried as a row and it does not commit the split by itself: it splits
///   only after two earlier signals, or when a trusted fresh-match board is
///   already held ([`BoardDoubt::board_deferred`]). A trusted board splits on
///   one earlier signal, as before. So one wrong suspect row cannot split a
///   real game, also not after a hero-select screen primed the streak.
/// * **It is not progress.** It does not count toward dropping a result
///   hint.
///
/// With nothing held, nothing unread and no suspect row this is exactly
/// [`plan_capture`].
pub fn plan_capture_held(input: &CapturePlanInput<'_>, doubt: BoardDoubt) -> CapturePlan {
    let held = doubt.held;
    let mut state = BoundaryState::new(None);
    if doubt.board_deferred {
        // Only its presence is read (`board_effect`), as in `decide_poll`.
        state.deferred = Some(Counters::default());
    }
    state.outcome = input.inherited_outcome;
    state.reset_streak = input.streak;
    state.reset_baseline = input.baseline.copied();
    state.baseline_row = input.baseline_row;
    state.gate = input.prev_gate.copied();
    state.hero = input.session_hero.map(str::to_string);
    state.pending_boundary = input.pending_boundary;
    state.progressed_boards = input.progressed_boards;
    state.awaiting_first_board = input.awaiting_first_board;
    if let Some(outcome) = input.hint.filter(|outcome| outcome.is_decided())
        && !input.confirmed_end
    {
        state.result = Some(ResultMark {
            outcome,
            confirmed: false,
            seen_at: Instant::now(),
        });
    }

    // The first Tab of a hinted session that names a different map is the
    // next game. A late Tab of the same match names the same map and stays.
    // A session that already has a board uses the same rule only after the
    // gap, and only when its stored map came from the top bar or the accolade.
    if hinted_different_map(input) {
        let decided = transition(
            &state,
            &Obs::HintedDifferentMap {
                frame_outcome: input.frame_outcome,
            },
        );
        if let Effect::Split { .. } = &decided.effect {
            return plan_from_effect(input, doubt, &decided.effect, false);
        }
    }

    // A session a start screen just opened has no board of its own. The
    // previous game is not an anchor, and a gap is not applicable there.
    let gap = !input.confirmed_end
        && !input.suppress_same_unfinished
        && input.age.is_some_and(|age| age >= input.min_gap)
        && input.classic_regressed
        && !input.create_session
        && !input.awaiting_first_board;
    if gap {
        let decided = transition(
            &state,
            &Obs::Gap {
                frame_outcome: input.frame_outcome,
            },
        );
        if let Effect::Split { .. } = &decided.effect {
            return plan_from_effect(input, doubt, &decided.effect, false);
        }
    }

    let compare = input.baseline.or(input.prev_gate);
    let elapsed = input.baseline_age.or(input.age).unwrap_or(Duration::ZERO);
    let implausible =
        compare.is_some_and(|gate| implausible_growth(gate, &input.cur, &held, elapsed));
    let fresh_reset = is_fresh_reset(input, &held, compare, implausible);
    let progressed = !implausible
        && compare.is_some_and(|gate| {
            counters_progressed(&gate.accepted, &input.cur, &known_cols(gate, &held))
        });
    let decided = transition(
        &state,
        &Obs::Board(BoardObs {
            fresh_reset,
            progressed,
            implausible,
            frame_outcome: input.frame_outcome,
            suspect_row: doubt.suspect_row,
        }),
    );
    plan_from_effect(input, doubt, &decided.effect, implausible)
}

fn plan_from_effect(
    input: &CapturePlanInput<'_>,
    doubt: BoardDoubt,
    effect: &Effect,
    implausible: bool,
) -> CapturePlan {
    let held = doubt.held;
    let stored_if_stay = if input.inherited_outcome.is_decided() {
        input.inherited_outcome
    } else {
        input.frame_outcome
    };
    let anchor = input.baseline.or(input.prev_gate);
    let fresh_shaped = anchor.is_some_and(|gate| {
        under_fresh_match(&gate.accepted, &input.cur, &known_cols(gate, &held))
    });
    let same_or_unset =
        input.baseline_row.is_none() || same_baseline_row(input.baseline_row, input.row_id);
    // The baseline's own row fell under the fresh-match thresholds of a
    // mature board: the shape of a new game. Taken as the baseline, the new
    // game's low counters would leave nothing mature to reset from, and the
    // two games would merge (CAP-8). It is stored, and the baseline, the
    // streak and a held board stay, so the reset can still fire.
    let reset_shaped = same_baseline_row(input.baseline_row, input.row_id)
        && anchor.is_some_and(|gate| {
            let known = known_cols(gate, &held);
            mature_board(&gate.accepted, &known)
                && under_fresh_match(&gate.accepted, &input.cur, &known)
        });
    // A different row re-anchors only when the stats continue. A low-total
    // teammate row is fresh-shaped: it is stored, and it neither replaces
    // the baseline nor counts toward the reset streak.
    // A suspect row anchors its own row's baseline, or the first one. It
    // does not take the baseline over from another row.
    let refresh = input.row_id.is_some()
        && input.row_counts
        && !implausible
        && !reset_shaped
        && (same_or_unset || (!fresh_shaped && !doubt.suspect_row));
    // A suspect row is not progress: it may be another player's.
    let counts = input.row_counts && !doubt.suspect_row;
    // An unidentified or implausible row is stored and leaves a held board
    // where it is. Only a refresh, or a counted progression, ends the streak.
    let counted_progress =
        counts && matches!(effect, Effect::CountProgress | Effect::ClearHintAndAppend);
    let drop_streak = refresh || counted_progress;
    match effect {
        Effect::Ignore
        | Effect::Seal { .. }
        | Effect::RememberHint { .. }
        | Effect::ArmPending
        | Effect::PrimeReset => {
            unreachable!("poll effects are not capture plans")
        }
        Effect::Append | Effect::ClearHintAndAppend | Effect::CountProgress => CapturePlan {
            split: false,
            defer: false,
            ignore_row: false,
            skip_store: false,
            clear_hint: matches!(effect, Effect::ClearHintAndAppend) && counts,
            count_progress: matches!(effect, Effect::CountProgress) && counts,
            reset_streak: if drop_streak { 0 } else { input.streak },
            reset_baseline: if refresh {
                None
            } else {
                input.baseline.copied()
            },
            baseline_row: if refresh {
                input.row_id
            } else {
                input.baseline_row
            },
            refresh_baseline: refresh,
            stored_outcome: stored_if_stay,
            deferred_counters: None,
            seal: None,
            close_reason: None,
        },
        Effect::Defer => CapturePlan {
            split: false,
            defer: true,
            ignore_row: false,
            skip_store: true,
            clear_hint: false,
            reset_streak: input.streak.saturating_add(1),
            reset_baseline: input.baseline.copied().or(input.prev_gate.copied()),
            baseline_row: input.baseline_row.or(input.row_id),
            refresh_baseline: false,
            count_progress: false,
            stored_outcome: stored_if_stay,
            // A board with a held cell is not a row: `input.cur` has the
            // previous game's number in that column. A suspect row may be
            // another player's. Either still counts as a reset signal;
            // nothing is carried onto the next session.
            deferred_counters: (!held.iter().any(|&cell| cell) && !doubt.suspect_row)
                .then_some(input.cur),
            seal: None,
            close_reason: None,
        },
        Effect::Split {
            new_outcome,
            seal,
            reason,
        } => CapturePlan {
            split: true,
            defer: false,
            ignore_row: false,
            skip_store: false,
            clear_hint: false,
            reset_streak: 0,
            reset_baseline: None,
            baseline_row: input.row_id,
            refresh_baseline: false,
            count_progress: false,
            stored_outcome: *new_outcome,
            deferred_counters: None,
            seal: *seal,
            close_reason: Some(*reason),
        },
    }
}

/// Growth that must not become the reset baseline.
///
/// A single column past the time-scaled ceiling is an implausible jump. A
/// drop is not. Wide columns use the ceiling even when the read is clean;
/// the store gate still leaves clean wide columns uncapped.
///
/// A row whose mature columns all landed between 2x and 4x inside
/// [`SHORT_ALL_INCREASE`] is the residual all-increase misread. The same
/// shape over a longer stretch is ordinary growth. The row is stored either
/// way, and a short-age hit does not replace the baseline.
const SHORT_ALL_INCREASE: Duration = Duration::from_secs(60);

fn implausible_growth(
    prev: &GateState,
    cur: &Counters,
    held: &[bool; GATE_COLS],
    elapsed: Duration,
) -> bool {
    let known = known_cols(prev, held);
    if elapsed < SHORT_ALL_INCREASE && uniform_two_to_four_times(&prev.accepted, cur, &known) {
        return true;
    }
    let acc = prev.accepted.to_array();
    let now = cur.to_array();
    for col in 0..GATE_COLS {
        // A column one side has no number for did not grow: a held cell
        // repeats the accepted value, and the first read of a never-read
        // column rises from a placeholder, not from a number.
        if !known[col] || now[col] <= acc[col] {
            continue;
        }
        // The accepted value is as old as the column's last accepted read.
        let secs = elapsed
            .as_secs()
            .saturating_add(u64::from(prev.held_secs[col]));
        let ceiling = if col <= 2 {
            ((secs / capture_gate::KILL_RATE_DIVISOR_SECS) as u32)
                .saturating_add(capture_gate::KILL_RATE_SLACK)
        } else {
            (secs as u32)
                .saturating_mul(capture_gate::WIDE_RATE_PER_SEC)
                .saturating_add(capture_gate::WIDE_RATE_SLACK)
        };
        if now[col] - acc[col] > ceiling {
            return true;
        }
    }
    false
}

/// Columns both boards have a number for: read on this capture (`held` is
/// false) and read at some point on the board it is compared with. All six
/// under ocr-v1.
fn known_cols(prev: &GateState, held: &[bool; GATE_COLS]) -> [bool; GATE_COLS] {
    std::array::from_fn(|col| !held[col] && !prev.unread[col])
}

/// Every mature column (accepted ≥ 4) grew by 2x to 4x. One spiked column
/// is ordinary play. This band is under the rate ceiling and used to replace
/// the baseline.
fn uniform_two_to_four_times(prev: &Counters, cur: &Counters, known: &[bool; GATE_COLS]) -> bool {
    let pairs = [
        (prev.elims, cur.elims),
        (prev.assists, cur.assists),
        (prev.deaths, cur.deaths),
        (prev.damage, cur.damage),
        (prev.healing, cur.healing),
        (prev.mitigation, cur.mitigation),
    ];
    let mut comparable = 0u32;
    for (col, (before, after)) in pairs.into_iter().enumerate() {
        if before < 4 || !known[col] {
            continue;
        }
        comparable += 1;
        let lo = before.saturating_mul(2);
        let hi = before.saturating_mul(4);
        if after < lo || after > hi {
            return false;
        }
    }
    comparable >= 3
}

/// Elims, deaths and damage: the columns a reset is read from.
const EDD_COLS: [usize; 3] = [0, 2, 3];

fn counters_progressed(prev: &Counters, cur: &Counters, known: &[bool; GATE_COLS]) -> bool {
    let before = prev.to_array();
    let now = cur.to_array();
    let grew = (0..GATE_COLS).any(|col| known[col] && now[col] > before[col]);
    let dropped = EDD_COLS
        .iter()
        .any(|&col| known[col] && now[col] < before[col]);
    grew && !dropped
}

/// The earlier board is far enough into a match to reset from, judged on a
/// column both boards have a number for.
fn mature_board(prev: &Counters, known: &[bool; GATE_COLS]) -> bool {
    (known[0] && prev.elims >= 4) || (known[3] && prev.damage >= 400)
}

/// Live-match fresh-start thresholds. A real slot change across a reset is
/// near zero. Another player's totals fail this and re-anchor instead. A
/// column one board has no number for is not compared.
fn under_fresh_match(prev: &Counters, cur: &Counters, known: &[bool; GATE_COLS]) -> bool {
    let elims_max = 2.max(prev.elims / 4);
    let deaths_max = 1.max(prev.deaths / 4);
    let damage_max = prev.damage / 4;
    (!known[0] || cur.elims <= elims_max)
        && (!known[2] || cur.deaths <= deaths_max)
        && (!known[3] || cur.damage <= damage_max)
}

/// No edge-ink suspect read in elims, deaths or damage. A held cell is not
/// such a read: it has no number, and abstains instead of vetoing.
fn edd_clean(suspect: &[bool; GATE_COLS], held: &[bool; GATE_COLS]) -> bool {
    EDD_COLS.iter().all(|&col| !suspect[col] || held[col])
}

fn same_baseline_row(baseline_row: Option<u32>, row_id: Option<u32>) -> bool {
    matches!((baseline_row, row_id), (Some(baseline), Some(row)) if baseline == row)
}

fn is_fresh_reset(
    input: &CapturePlanInput<'_>,
    held: &[bool; GATE_COLS],
    compare: Option<&GateState>,
    implausible: bool,
) -> bool {
    let Some(prev) = compare else {
        return false;
    };
    let known = known_cols(prev, held);
    // One column falling is a misread cell. Two of the three reset columns
    // have to be there to say the board fell.
    let enough = EDD_COLS.iter().filter(|&&col| known[col]).count() >= 2;
    same_baseline_row(input.baseline_row, input.row_id)
        && input.row_counts
        && edd_clean(&input.suspect, held)
        && enough
        && !implausible
        && mature_board(&prev.accepted, &known)
        && compare_age(input).is_some_and(|age| age >= RESET_MIN_AGE)
        && under_fresh_match(&prev.accepted, &input.cur, &known)
}

/// Age of the board a reset is measured from: the baseline's own time when
/// there is a baseline, else the gate's. Before CAP-8 this was the time since
/// the last stored row of any kind, so an unidentified row stored between two
/// fresh boards made the second one "too young", and it became the baseline.
/// A baseline with no recorded time falls back to the gate's age, which is
/// never older than the baseline.
fn compare_age(input: &CapturePlanInput<'_>) -> Option<Duration> {
    if input.baseline.is_some() {
        input.baseline_age.or(input.age)
    } else {
        input.age
    }
}

/// Outcome to write when a session closes, if it does not already have one.
///
/// An idle close passes no seal. A stored result is left as it is.
pub fn outcome_sealed_on_close(
    stored: MatchOutcome,
    explicit_seal: Option<MatchOutcome>,
) -> Option<MatchOutcome> {
    if stored.is_decided() {
        None
    } else {
        explicit_seal.filter(|outcome| outcome.is_decided())
    }
}

/// Boundary fields the poller and the Tab path share.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoundaryState {
    pub map: Option<String>,
    pub outcome: MatchOutcome,
    pub outcome_at: Option<Instant>,
    pub result: Option<ResultMark>,
    pub reset_streak: u32,
    pub reset_baseline: Option<GateState>,
    /// Player row that owns the baseline. A later plausible row replaces it.
    pub baseline_row: Option<u32>,
    /// Last stored scoreboard. A deferred reset board does not move this.
    pub last_board_at: Option<Instant>,
    /// Hero select after a board-followed hint. The first reset signal.
    pub pending_boundary: bool,
    /// Progressed boards accepted since the current hint. The second one drops it.
    pub progressed_boards: u8,
    /// When the reset baseline was taken. Rate checks measure from here.
    pub baseline_at: Option<Instant>,
    /// Opened by a start screen; the first board has not been accepted.
    pub awaiting_first_board: bool,
    pub hero: Option<String>,
    /// First fresh-match board, held off the current session.
    pub deferred: Option<Counters>,
    pub gate: Option<GateState>,
    /// A different result replaced the hint. Stays set so the confirming
    /// read cannot relabel a text-fallback map.
    pub text_fallback_locked: bool,
}

impl BoundaryState {
    pub fn new(map: Option<String>) -> Self {
        Self {
            map,
            outcome: MatchOutcome::Unknown,
            outcome_at: None,
            result: None,
            reset_streak: 0,
            reset_baseline: None,
            baseline_row: None,
            last_board_at: None,
            pending_boundary: false,
            progressed_boards: 0,
            baseline_at: None,
            awaiting_first_board: false,
            hero: None,
            deferred: None,
            gate: None,
            text_fallback_locked: false,
        }
    }
}

pub fn phase_of(state: &BoundaryState) -> Phase {
    if state.outcome.is_decided() {
        return Phase::PostMatch;
    }
    if state.awaiting_first_board {
        return Phase::NewGameStarting;
    }
    let hint = state
        .result
        .is_some_and(|mark| !mark.confirmed && mark.outcome.is_decided());
    if hint {
        return Phase::PostResultStreak;
    }
    if state.gate.is_some() || state.map.is_some() || state.hero.is_some() {
        return Phase::LiveMatch;
    }
    Phase::Idle
}

/// The one transition. Poll and capture both call this.
pub fn transition(state: &BoundaryState, obs: &Obs<'_>) -> Transition {
    let phase = phase_of(state);
    let effect = match obs {
        Obs::ConfirmedWord { outcome } => confirmed_word(phase, *outcome),
        Obs::UnconfirmedWord { outcome, relation } => {
            unconfirmed_word(state, phase, *outcome, *relation)
        }
        Obs::StartScreen {
            screen,
            block_map_vote,
        } => start_screen(state, phase, screen, *block_map_vote),
        Obs::Board(board) => board_effect(state, phase, board),
        Obs::Gap { frame_outcome } => gap_effect(state, phase, *frame_outcome),
        Obs::EndScreenDifferentMap { outcome, .. } => end_screen(state, phase, *outcome),
        // Same close as a gap: seal the hint, keep this frame's header.
        Obs::HintedDifferentMap { frame_outcome } => gap_effect(state, phase, *frame_outcome),
    };
    Transition { effect, phase }
}

fn sealable_hint(state: &BoundaryState) -> Option<MatchOutcome> {
    state
        .result
        .filter(|mark| !mark.confirmed && mark.outcome.is_decided())
        .map(|mark| mark.outcome)
}

fn confirmed_word(phase: Phase, outcome: MatchOutcome) -> Effect {
    match phase {
        Phase::PostMatch => Effect::Ignore,
        Phase::Idle | Phase::LiveMatch | Phase::PostResultStreak | Phase::NewGameStarting => {
            Effect::Seal { outcome }
        }
    }
}

/// An unconfirmed word replaces the hint when the maps match, or when the
/// boundary is not armed and either side has no trusted name. A different
/// map never replaces it.
fn unconfirmed_replaces(pending_boundary: bool, relation: MapRelation) -> bool {
    match relation {
        MapRelation::Matches => true,
        MapRelation::Differs => false,
        MapRelation::Absent => !pending_boundary,
    }
}

fn unconfirmed_word(
    state: &BoundaryState,
    phase: Phase,
    outcome: MatchOutcome,
    relation: MapRelation,
) -> Effect {
    if !unconfirmed_replaces(state.pending_boundary, relation) {
        return Effect::Ignore;
    }
    match phase {
        Phase::PostMatch => Effect::Ignore,
        Phase::Idle | Phase::LiveMatch | Phase::PostResultStreak | Phase::NewGameStarting => {
            Effect::RememberHint { outcome }
        }
    }
}

fn start_screen(
    state: &BoundaryState,
    phase: Phase,
    screen: &StartScreen,
    block_map_vote: bool,
) -> Effect {
    let split_unsealed = |screen: &StartScreen| Effect::Split {
        reason: screen_reason(screen).0,
        seal: None,
        new_outcome: MatchOutcome::Unknown,
    };
    let split_sealed = |screen: &StartScreen, state: &BoundaryState| Effect::Split {
        reason: screen_reason(screen).0,
        seal: sealable_hint(state),
        new_outcome: MatchOutcome::Unknown,
    };
    match phase {
        Phase::PostMatch => match screen {
            StartScreen::MapVote { .. } if block_map_vote => Effect::Ignore,
            _ => split_unsealed(screen),
        },
        Phase::PostResultStreak => match screen {
            StartScreen::MapVote { .. } if block_map_vote => Effect::Ignore,
            StartScreen::MapVote { .. } | StartScreen::HeroBan => split_sealed(screen, state),
            StartScreen::HeroSelect if state.gate.is_some() => Effect::ArmPending,
            StartScreen::HeroSelect => split_sealed(screen, state),
        },
        Phase::Idle | Phase::LiveMatch => match screen {
            StartScreen::MapVote { .. } if block_map_vote => Effect::Ignore,
            StartScreen::MapVote { .. } => split_unsealed(screen),
            // A ban after one deferred misread is not the second signal.
            StartScreen::HeroBan if state.deferred.is_some() => Effect::Ignore,
            StartScreen::HeroBan => split_unsealed(screen),
            // Select after a deferral does not split. Select before a fresh
            // board is the first signal.
            StartScreen::HeroSelect if state.deferred.is_some() => Effect::Ignore,
            StartScreen::HeroSelect => Effect::PrimeReset,
        },
        // A stable start screen repeats every tick. The debounce blocks a
        // repeated vote or ban. A hero select before the first Tab is a swap
        // on the game this screen just opened, including after the debounce.
        Phase::NewGameStarting => match screen {
            StartScreen::HeroSelect => Effect::Ignore,
            StartScreen::MapVote { .. } | StartScreen::HeroBan if block_map_vote => Effect::Ignore,
            _ => split_unsealed(screen),
        },
    }
}

fn gap_effect(state: &BoundaryState, phase: Phase, frame_outcome: MatchOutcome) -> Effect {
    match phase {
        // Idle has no board to regress from. NewGameStarting's first Tab
        // belongs to the session the start screen opened.
        Phase::Idle | Phase::NewGameStarting | Phase::PostMatch => Effect::Ignore,
        Phase::LiveMatch | Phase::PostResultStreak => Effect::Split {
            reason: CloseReason::StatRegression,
            seal: sealable_hint(state),
            new_outcome: if frame_outcome.is_decided() {
                frame_outcome
            } else {
                MatchOutcome::Unknown
            },
        },
    }
}

fn end_screen(state: &BoundaryState, phase: Phase, outcome: MatchOutcome) -> Effect {
    match phase {
        Phase::PostMatch | Phase::Idle | Phase::NewGameStarting => Effect::Ignore,
        // Both names are trusted and they differ. Armed or not, this word
        // belongs to the next session. A word with no trusted name never
        // reaches here: it is a confirmed word and seals here.
        Phase::LiveMatch | Phase::PostResultStreak => Effect::Split {
            reason: CloseReason::EndScreenMap,
            seal: sealable_hint(state),
            new_outcome: outcome,
        },
    }
}

fn board_effect(state: &BoundaryState, _phase: Phase, board: &BoardObs) -> Effect {
    if board.fresh_reset {
        // A trusted board splits on one earlier signal. A suspect row needs
        // two, or a trusted fresh board already held: it never commits the
        // split by itself (`plan_capture_held`).
        let commits = if board.suspect_row {
            state.reset_streak >= 2 || state.deferred.is_some()
        } else {
            state.reset_streak >= 1 || state.pending_boundary
        };
        if commits {
            return Effect::Split {
                reason: CloseReason::StatReset,
                seal: sealable_hint(state),
                new_outcome: MatchOutcome::Unknown,
            };
        }
        return Effect::Defer;
    }
    let hint_open = sealable_hint(state).is_some() && !state.outcome.is_decided();
    if hint_open && board.progressed && !board.implausible && !board.frame_outcome.is_decided() {
        // An arm is already the first "the match may be over" signal, so one
        // progressed board drops the hint. Otherwise the first progressed
        // board keeps it and the second drops it.
        if state.pending_boundary || state.progressed_boards >= 1 {
            return Effect::ClearHintAndAppend;
        }
        return Effect::CountProgress;
    }
    Effect::Append
}

/// A session [`commit_poll`] closed. The seal is already the value to store.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CloseRecord {
    pub reason: CloseReason,
    pub seal: Option<MatchOutcome>,
}

/// Result of folding one [`PollDecision`] into [`BoundaryState`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PollCommit {
    pub recorded_outcome: Option<MatchOutcome>,
    pub adopted_map: Option<String>,
    pub closed: Option<CloseRecord>,
}

/// Apply a poll decision. This is the state change `main` runs before it
/// writes the store. The decision itself came from [`transition`].
pub fn commit_poll(state: &mut BoundaryState, decision: PollDecision, now: Instant) -> PollCommit {
    match decision {
        PollDecision::Keep | PollDecision::IgnoreContradictory { .. } => PollCommit {
            recorded_outcome: None,
            adopted_map: None,
            closed: None,
        },
        PollDecision::Update(update) => {
            if update.arm_pending {
                state.pending_boundary = true;
                if state.deferred.is_none() {
                    state.reset_streak = state.reset_streak.max(1);
                }
            }
            if update.arm_reset && state.deferred.is_none() {
                state.reset_streak = state.reset_streak.max(1);
            }
            if update.clear_awaiting {
                state.awaiting_first_board = false;
            }
            if let Some(mark) = update.result {
                // The hint that was replaced was a different result. The
                // accolade belongs to the next game, and the new hint must
                // not reopen the text-fallback window on the confirming tick.
                if state
                    .result
                    .is_some_and(|prev| prev.outcome.is_decided() && prev.outcome != mark.outcome)
                {
                    state.text_fallback_locked = true;
                }
                let replaced = state.result.is_none_or(|prev| {
                    prev.outcome != mark.outcome || prev.confirmed != mark.confirmed
                });
                if replaced {
                    state.progressed_boards = 0;
                }
                state.result = Some(mark);
            }
            let recorded = update
                .record_outcome
                .filter(|outcome| outcome.is_decided() && !state.outcome.is_decided());
            if let Some(outcome) = recorded {
                state.outcome = outcome;
                // Grace starts when the result is recorded, not when the
                // word was first sighted.
                state.outcome_at = Some(now);
                state.pending_boundary = false;
                state.awaiting_first_board = false;
            }
            // `adopt_map` is set when the session has no map, or when a
            // text-fallback name is still open for an accolade. The in-memory
            // name is replaced, not only filled when empty.
            let adopted = update.adopt_map.clone();
            if let Some(map) = adopted.clone() {
                state.map = Some(map);
            }
            PollCommit {
                recorded_outcome: recorded,
                adopted_map: adopted,
                closed: None,
            }
        }
        PollDecision::Open(open) => {
            let previous = std::mem::replace(state, BoundaryState::new(None));
            let seal = outcome_sealed_on_close(previous.outcome, open.seal_outcome);
            state.outcome = open.new_outcome;
            state.outcome_at = open.new_outcome.is_decided().then_some(now);
            state.map = open.new_map;
            state.result = open.new_result;
            state.awaiting_first_board = !open.new_outcome.is_decided()
                && matches!(
                    open.reason,
                    CloseReason::MapVote | CloseReason::HeroSelectOrBan
                );
            PollCommit {
                recorded_outcome: None,
                adopted_map: None,
                closed: Some(CloseRecord {
                    reason: open.reason,
                    seal,
                }),
            }
        }
    }
}

/// Fold a capture that stayed on this session. A deferred board is held and
/// does not move the gate. A stored row always updates the gate.
pub fn note_accepted_capture(
    state: &mut BoundaryState,
    plan: &CapturePlan,
    accepted: GateState,
    now: Instant,
) {
    if plan.ignore_row {
        return;
    }
    if plan.defer {
        // A deferral on a game with no baseline yet takes the gate as the
        // baseline. Its time is the gate's, so the reset age is measured
        // from that board (`compare_age`).
        if state.reset_baseline.is_none() && plan.reset_baseline.is_some() {
            state.baseline_at = state.last_board_at;
        }
        state.reset_streak = plan.reset_streak;
        state.reset_baseline = plan.reset_baseline;
        state.baseline_row = plan.baseline_row;
        state.deferred = plan.deferred_counters;
        return;
    }
    if plan.refresh_baseline || plan.count_progress || plan.clear_hint {
        state.deferred = None;
    }
    // A discarded suspect row does not move the monotonic gate. Its raw
    // counters can still anchor the reset baseline, which is what a later
    // fresh board is compared with.
    let mut monotonic = accepted;
    let anchor = monotonic.boundary_anchor.take();
    state.gate = Some(monotonic);
    state.reset_streak = plan.reset_streak;
    state.reset_baseline = if plan.refresh_baseline {
        Some(anchor.map_or(monotonic, |raw| capture_gate::GateState {
            accepted: raw,
            last_raw: raw,
            ..capture_gate::GateState::default()
        }))
    } else {
        plan.reset_baseline
    };
    state.baseline_row = plan.baseline_row;
    state.last_board_at = Some(now);
    state.awaiting_first_board = false;
    if plan.refresh_baseline {
        state.baseline_at = Some(now);
    }
    if plan.count_progress {
        state.progressed_boards = state.progressed_boards.saturating_add(1);
    }
    let confirmed_mark =
        state.outcome.is_decided() || state.result.is_some_and(|mark| mark.confirmed);
    if plan.clear_hint && !confirmed_mark {
        state.result = None;
        state.pending_boundary = false;
        state.progressed_boards = 0;
        // Progressed boards are the capture path that drops the hint.
        // Drop the lock with it, or a later accolade still cannot correct
        // a text-fallback name after the hint is gone.
        state.text_fallback_locked = false;
    }
}

pub fn has_post_result(outcome: MatchOutcome, result: Option<ResultMark>) -> bool {
    outcome.is_decided() || result.is_some_and(|mark| mark.outcome.is_decided())
}

/// One map filter for the relation and for adopt: [`named_map`]. Empty and
/// `unknown` are not names. A looser trim-only check used to accept
/// `unknown`, which let a non-name block a real accolade or get stored.
fn adopt_map(input: &PollInput<'_>) -> Option<String> {
    let accolade = named_map(input.accolade_map)?;
    if named_map(trusted_session_map(input)).is_some() {
        return None;
    }
    if input.map.is_some() && !text_fallback_open_for_accolade(input) {
        return None;
    }
    Some(accolade.to_string())
}

/// A text-fallback name may be replaced only while no boundary signal has
/// been seen yet. An arm, a start screen, a primed or held reset, a recorded
/// outcome, or a hint of a different result closes the window. A hint of
/// the same result, or no hint, still lets the next game's accolade
/// through: the checks cannot tell those apart from this session's own
/// end screen.
fn text_fallback_open_for_accolade(input: &PollInput<'_>) -> bool {
    if input.text_fallback_locked
        || input.pending_boundary
        || input.start_screen.is_some()
        || input.deferred
        || input.reset_streak > 0
        || input.outcome.is_decided()
    {
        return false;
    }
    if let (Some(hint), Some(signal)) = (
        input
            .result
            .filter(|mark| !mark.confirmed && mark.outcome.is_decided()),
        input.signal.filter(|outcome| outcome.is_decided()),
    ) && hint.outcome != signal
    {
        return false;
    }
    true
}

/// Compare two map reads. Empty and `unknown` are not names, so a missing
/// accolade is [`MapRelation::Absent`] rather than a different map.
fn map_relation(session: Option<&str>, accolade: Option<&str>) -> MapRelation {
    match (named_map(session), named_map(accolade)) {
        (Some(session), Some(accolade)) if session.eq_ignore_ascii_case(accolade) => {
            MapRelation::Matches
        }
        (Some(_), Some(_)) => MapRelation::Differs,
        _ => MapRelation::Absent,
    }
}

/// The session map [`decide_poll`] is allowed to compare. A text fallback
/// is absent.
fn trusted_session_map<'a>(input: &'a PollInput<'a>) -> Option<&'a str> {
    input.map.filter(|_| input.map_trusted)
}

fn screen_reason(screen: &StartScreen) -> (CloseReason, Vec<String>) {
    match screen {
        StartScreen::MapVote { candidates } => (CloseReason::MapVote, candidates.clone()),
        StartScreen::HeroSelect | StartScreen::HeroBan => {
            (CloseReason::HeroSelectOrBan, Vec::new())
        }
    }
}

/// What this poll tick does to the open session. The rule order is
/// [`transition`].
pub fn decide_poll(input: &PollInput<'_>) -> PollDecision {
    let mut state = BoundaryState::new(input.map.map(str::to_string));
    state.outcome = input.outcome;
    state.result = input.result;
    state.pending_boundary = input.pending_boundary;
    state.awaiting_first_board = input.awaiting_first_board;
    state.reset_streak = input.reset_streak;
    state.hero = input.hero.map(str::to_string);
    if input.has_board {
        state.gate = Some(GateState::default());
    }
    if input.deferred {
        state.deferred = Some(Counters {
            elims: 0,
            assists: 0,
            deaths: 0,
            damage: 0,
            healing: 0,
            mitigation: 0,
        });
    }

    let obs = if let Some(screen) = input.start_screen.as_ref() {
        Obs::StartScreen {
            screen,
            block_map_vote: input.block_map_vote,
        }
    } else if let Some(signal) = input.signal.filter(|outcome| outcome.is_decided()) {
        let session_map = trusted_session_map(input);
        let relation = map_relation(session_map, input.accolade_map);
        if relation == MapRelation::Differs && input.signal_confirmed {
            Obs::EndScreenDifferentMap {
                outcome: signal,
                map: named_map(input.accolade_map).unwrap_or(""),
            }
        } else if input.signal_confirmed {
            Obs::ConfirmedWord { outcome: signal }
        } else {
            Obs::UnconfirmedWord {
                outcome: signal,
                relation,
            }
        }
    } else {
        let adopt = adopt_map(input);
        return if let Some(map) = adopt {
            PollDecision::Update(UpdateCurrent {
                record_outcome: None,
                adopt_map: Some(map),
                result: input.result,
                arm_pending: false,
                arm_reset: false,
                clear_awaiting: false,
            })
        } else {
            PollDecision::Keep
        };
    };

    let decided = transition(&state, &obs);
    match decided.effect {
        Effect::Ignore => {
            // An unconfirmed word that must not replace the hint. This is
            // [`PollDecision::Keep`], reached through [`transition`], so the
            // next game's word is not sealed onto this session.
            if let Obs::UnconfirmedWord { relation, .. } = &obs
                && !unconfirmed_replaces(input.pending_boundary, *relation)
            {
                return PollDecision::Keep;
            }
            if let Some(map) = adopt_map(input) {
                return PollDecision::Update(UpdateCurrent {
                    record_outcome: None,
                    adopt_map: Some(map),
                    result: input.result,
                    arm_pending: false,
                    arm_reset: false,
                    clear_awaiting: false,
                });
            }
            let kept = state
                .outcome
                .is_decided()
                .then_some(state.outcome)
                .or_else(|| {
                    state
                        .result
                        .filter(|mark| mark.outcome.is_decided())
                        .map(|mark| mark.outcome)
                });
            if let Some(signal) = input.signal.filter(|outcome| outcome.is_decided())
                && let Some(kept) = kept
                && kept != signal
            {
                PollDecision::IgnoreContradictory {
                    kept,
                    ignored: signal,
                }
            } else {
                PollDecision::Keep
            }
        }
        Effect::Seal { outcome } => PollDecision::Update(UpdateCurrent {
            record_outcome: Some(outcome),
            adopt_map: adopt_map(input),
            result: Some(confirmed_mark(input, outcome)),
            arm_pending: false,
            arm_reset: false,
            clear_awaiting: true,
        }),
        Effect::RememberHint { outcome } => {
            let seen_at = input
                .result
                .filter(|mark| mark.outcome == outcome)
                .map(|mark| mark.seen_at)
                .unwrap_or(input.now);
            let result = ResultMark {
                outcome,
                confirmed: false,
                seen_at,
            };
            if input.result == Some(result) && !input.awaiting_first_board {
                PollDecision::Keep
            } else {
                PollDecision::Update(UpdateCurrent {
                    record_outcome: None,
                    adopt_map: adopt_map(input),
                    result: Some(result),
                    arm_pending: false,
                    arm_reset: false,
                    clear_awaiting: true,
                })
            }
        }
        Effect::ArmPending => PollDecision::Update(UpdateCurrent {
            record_outcome: None,
            adopt_map: None,
            result: input.result,
            arm_pending: true,
            arm_reset: false,
            clear_awaiting: false,
        }),
        Effect::PrimeReset => PollDecision::Update(UpdateCurrent {
            record_outcome: None,
            adopt_map: None,
            result: input.result,
            arm_pending: false,
            arm_reset: true,
            clear_awaiting: false,
        }),
        Effect::Split {
            reason,
            seal,
            new_outcome,
        } => {
            let (reason, candidates) = if matches!(reason, CloseReason::StatReset) {
                (reason, Vec::new())
            } else if let Some(screen) = input.start_screen.as_ref() {
                screen_reason(screen)
            } else {
                (reason, Vec::new())
            };
            let new_map = match &obs {
                Obs::EndScreenDifferentMap { map, .. } => named_map(Some(map)).map(str::to_string),
                _ => None,
            };
            PollDecision::Open(OpenNew {
                reason,
                seal_outcome: seal,
                new_outcome,
                new_map,
                candidates,
                new_result: None,
            })
        }
        Effect::Append | Effect::ClearHintAndAppend | Effect::CountProgress | Effect::Defer => {
            PollDecision::Keep
        }
    }
}

fn confirmed_mark(input: &PollInput<'_>, outcome: MatchOutcome) -> ResultMark {
    let seen_at = input
        .result
        .filter(|mark| mark.outcome == outcome)
        .map(|mark| mark.seen_at)
        .unwrap_or(input.now);
    ResultMark {
        outcome,
        confirmed: true,
        seen_at,
    }
}
#[cfg(test)]
mod tests;

#[cfg(test)]
mod tests_finished_tab;

#[cfg(test)]
mod tests_plan;

#[cfg(test)]
mod tests_scenarios;

#[cfg(test)]
mod tests_transitions;

#[cfg(test)]
mod tests_text_map;
