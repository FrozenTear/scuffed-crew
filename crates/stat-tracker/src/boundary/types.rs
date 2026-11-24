//! The boundary's inputs and outputs: poll and Tab inputs, observations,
//! effects, decisions, the capture plan types and [`CloseReason`].

use std::time::{Duration, Instant};

use crate::capture_gate::{Counters, GATE_COLS, GateState};
use crate::model::MatchOutcome;

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
///
/// [`BoundaryState`]: super::BoundaryState
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
    ///
    /// [`decide_poll`]: super::decide_poll
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
///
/// [`transition`]: super::transition
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
///
/// [`transition`]: super::transition
/// [`plan_capture`]: super::plan_capture
#[derive(Clone, Copy, Debug)]
pub struct BoardObs {
    /// Identified row whose counters sit under the fresh-match thresholds
    /// versus a mature board, at least [`RESET_MIN_AGE`] later, and not an
    /// implausible jump.
    ///
    /// [`RESET_MIN_AGE`]: super::RESET_MIN_AGE
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
    ///
    /// [`plan_capture`]: super::plan_capture
    /// [`hinted_different_map`]: super::plan::hinted_different_map
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

/// Inputs for one accepted scoreboard capture. `main` fills this from the
/// open session and the frame; tests call [`plan_capture`] with the same struct.
///
/// [`plan_capture`]: super::plan_capture
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
    ///
    /// [`plan_capture_held`]: super::plan_capture_held
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
///
/// [`plan_capture`]: super::plan_capture
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

/// A session [`commit_poll`] closed. The seal is already the value to store.
///
/// [`commit_poll`]: super::commit_poll
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CloseRecord {
    pub reason: CloseReason,
    pub seal: Option<MatchOutcome>,
}

/// Result of folding one [`PollDecision`] into [`BoundaryState`].
///
/// [`BoundaryState`]: super::BoundaryState
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PollCommit {
    pub recorded_outcome: Option<MatchOutcome>,
    pub adopted_map: Option<String>,
    pub closed: Option<CloseRecord>,
}

pub(super) fn screen_reason(screen: &StartScreen) -> (CloseReason, Vec<String>) {
    match screen {
        StartScreen::MapVote { candidates } => (CloseReason::MapVote, candidates.clone()),
        StartScreen::HeroSelect | StartScreen::HeroBan => {
            (CloseReason::HeroSelectOrBan, Vec::new())
        }
    }
}
