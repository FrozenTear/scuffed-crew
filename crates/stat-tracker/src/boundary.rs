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
//! an armed end screen on a different map, or an unblocked map vote or
//! hero ban seals it onto this session. The first progressed board keeps
//! the hint. "Progressed" is measured from the reset baseline: a counter
//! moved forward, the totals are not the same, the frame is not a decided
//! result header, and the jump is not implausible. Wall-clock time is the
//! caller's 75-second post-match grace, the 120-second stat gap, the
//! 45-second wait before the first reset board counts, the 20-minute
//! unfinished-session idle bound, and the map-vote debounce.
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
//! | Idle, LiveMatch | unconfirmed word | Remember the hint when the map matches, or when the boundary is not armed and the read has no map. A different map is ignored. |
//! | Idle, LiveMatch | map vote, not blocked | Split. No seal. The same-map-plus-hero guard and the debounce are the caller's block. |
//! | Idle, LiveMatch | hero ban, no deferred board | Split at once. No seal. |
//! | Idle, LiveMatch | hero ban after a deferred board | Ignore. One misread plus a ban is not a split. |
//! | Idle, LiveMatch | hero select, no deferred board | Prime the reset streak. Do not split. |
//! | Idle, LiveMatch | hero select after a deferred board | Ignore. |
//! | Idle | scoreboard | Append. There is no mature board to reset from. |
//! | LiveMatch | fresh-match board, streak already primed | Split. New outcome is Unknown. LiveMatch has no hint to seal. |
//! | LiveMatch | fresh-match board, streak clear | Defer the first. Split on the second fresh board. |
//! | LiveMatch | other scoreboard | Append. An unidentified or implausible row leaves a held board and the streak. A plausible same-row board refreshes the baseline. A different row that is not fresh-shaped re-anchors it. |
//! | Idle | 120s gap | Not applicable. Ignore. |
//! | LiveMatch | 120s gap | Split. The new session keeps this frame's header. The caller's guard is the same map plus the same hero. |
//! | Idle | end screen, different map | Not applicable. Ignore. |
//! | LiveMatch | end screen, different map, not armed | Seal. The session already has a map, which is what made this a different map. |
//! | LiveMatch | end screen, different map, armed | Split. Seal the old hint. The new session takes the end-screen outcome. |
//! | PostResultStreak | confirmed word | Seal that word. Do not split. Grace starts now. |
//! | PostResultStreak | unconfirmed word | Replaces the hint when the map matches, or when the boundary is not armed and the read has no map. A different map does not replace the hint. While armed, a word with no map does not replace the hint. |
//! | PostResultStreak | map vote, not blocked, or hero ban | Split and seal the hint. |
//! | PostResultStreak | hero select, no board yet | Seal the hint and split. |
//! | PostResultStreak | hero select after a board | Arm a pending boundary. That arm is the first reset signal. |
//! | PostResultStreak | first progressed board | Append. The hint stays. |
//! | PostResultStreak | second progressed board, or one after an arm | Clear the hint and append. |
//! | PostResultStreak | same totals, or a decided header | Append. The hint stays. |
//! | PostResultStreak | fresh-match board | Defer, then split on the next fresh board. The split seals the hint. An armed streak splits on this board. |
//! | PostResultStreak | 120s gap | Split. Seal the hint. Keep this frame's header. |
//! | PostResultStreak | end screen, different map, not armed | Seal the word. Do not split on the map. |
//! | PostResultStreak | end screen, different map, armed | Split. Seal the old hint. The new session takes the end-screen outcome and the map on this read, including a map the caller carried from the previous agreeing word. An unconfirmed word on a different map does not replace the hint. While armed, a word with no map does not replace it either. A banner has no map, so a banner-only confirmation still seals onto this session. |
//! | PostResultStreak | hinted Tab, different map | Split. Seal the hint. The new session keeps this frame's header. [`plan_capture`] builds [`Obs::HintedDifferentMap`] only when the stored map is the top bar or the accolade and the gap has elapsed, or when the session has no board yet. A text-fallback map does not build it. |
//! | PostMatch | word | Ignore the outcome. Adopt an accolade map when this session has none. |
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

use std::time::{Duration, Instant};

use crate::capture_gate::{self, Counters, GATE_COLS, GateState};
use crate::detect::MatchOutcome;

/// The first fresh-match board has to land at least this long after the last
/// accepted board. A drop a few seconds later is still this match.
pub const RESET_MIN_AGE: Duration = Duration::from_secs(45);

/// Why a session closed. The log line is [`CloseReason::log`]; callers match
/// the enum, not the text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseReason {
    MapVote,
    HeroSelectOrBan,
    StatReset,
    StatRegression,
}

impl CloseReason {
    pub fn log(self) -> &'static str {
        match self {
            Self::MapVote => "superseded by map vote",
            Self::HeroSelectOrBan => "superseded by hero select/ban",
            Self::StatReset => "superseded by stat reset",
            Self::StatRegression => "superseded by stat regression",
        }
    }
}

/// End-screen evidence kept on the session until a second progressed board
/// clears it, or one progressed board after an arm, or a confirmed read
/// seals it.
///
/// `confirmed == false` is a hint. A reset, a gap, a different-map Tab, an
/// armed different-map end screen, or a no-board start screen seals it. An
/// unconfirmed word replaces it when the maps match, or when the boundary
/// is not armed and the read has no map. A different map never replaces it.
/// While a boundary is armed, a word with no map does not replace it, so
/// the confirming read can seal this hint and open the next session. A
/// banner carries no map, so a banner-only confirmation still seals onto
/// this session. An idle close does not seal the hint. A full-board text
/// fallback is not a map here: the poll treats it as absent, an accolade
/// can replace it, and a different-map split does not use it.
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
    /// False when `map` is a full-board text fallback, or a pre-0.4.18
    /// skeleton that had a map and no source. [`decide_poll`] treats that
    /// map as absent: a different accolade does not split, and an accolade
    /// can replace the stored name.
    pub map_trusted: bool,
    pub hero: Option<&'a str>,
    pub signal: Option<MatchOutcome>,
    /// Banner, or the second agreeing word inside the confirm window.
    pub signal_confirmed: bool,
    pub accolade_map: Option<&'a str>,
    pub start_screen: Option<StartScreen>,
    /// Map vote that the same-map guard or the debounce refused.
    pub block_map_vote: bool,
    /// A fresh-match board is held off this session. The poll path has to
    /// say so: `start_screen` ignores a ban or select after that hold.
    pub deferred: bool,
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
    /// Drop a hint because a second progressed board, or one after an arm, arrived.
    pub clear_hint: bool,
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
    /// One side has no name. While a boundary is armed this does not replace
    /// the hint. While it is not, a mapless word still replaces the hint.
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
    /// row (`parse::row_counts`).
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
    /// Result read off this frame (banner, header). A gap split keeps it.
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
    let mut state = BoundaryState::new(None);
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
            return plan_from_effect(input, &decided.effect, false);
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
            return plan_from_effect(input, &decided.effect, false);
        }
    }

    let compare = input.baseline.or(input.prev_gate);
    let elapsed = input.baseline_age.or(input.age).unwrap_or(Duration::ZERO);
    let implausible =
        compare.is_some_and(|gate| implausible_growth(&gate.accepted, &input.cur, elapsed));
    let fresh_reset = is_fresh_reset(input, compare, implausible);
    let progressed =
        !implausible && compare.is_some_and(|gate| counters_progressed(&gate.accepted, &input.cur));
    let decided = transition(
        &state,
        &Obs::Board(BoardObs {
            fresh_reset,
            progressed,
            implausible,
            frame_outcome: input.frame_outcome,
        }),
    );
    plan_from_effect(input, &decided.effect, implausible)
}

fn plan_from_effect(
    input: &CapturePlanInput<'_>,
    effect: &Effect,
    implausible: bool,
) -> CapturePlan {
    let stored_if_stay = if input.inherited_outcome.is_decided() {
        input.inherited_outcome
    } else {
        input.frame_outcome
    };
    let anchor = input.baseline.or(input.prev_gate);
    let fresh_shaped = anchor.is_some_and(|gate| under_fresh_match(&gate.accepted, &input.cur));
    let same_or_unset =
        input.baseline_row.is_none() || same_baseline_row(input.baseline_row, input.row_id);
    // A different row re-anchors only when the stats continue. A low-total
    // teammate row is fresh-shaped: it is stored, and it neither replaces
    // the baseline nor counts toward the reset streak.
    let refresh = input.row_id.is_some()
        && input.row_counts
        && !implausible
        && (same_or_unset || !fresh_shaped);
    // An unidentified or implausible row is stored and leaves a held board
    // where it is. Only a refresh, or a counted progression, ends the streak.
    let counted_progress =
        input.row_counts && matches!(effect, Effect::CountProgress | Effect::ClearHintAndAppend);
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
            clear_hint: matches!(effect, Effect::ClearHintAndAppend) && input.row_counts,
            count_progress: matches!(effect, Effect::CountProgress) && input.row_counts,
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
            deferred_counters: Some(input.cur),
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

fn implausible_growth(prev: &Counters, cur: &Counters, elapsed: Duration) -> bool {
    if elapsed < SHORT_ALL_INCREASE && uniform_two_to_four_times(prev, cur) {
        return true;
    }
    let acc = prev.to_array();
    let now = cur.to_array();
    let secs = elapsed.as_secs();
    let kill_ceiling = ((secs / capture_gate::KILL_RATE_DIVISOR_SECS) as u32)
        .saturating_add(capture_gate::KILL_RATE_SLACK);
    let wide_ceiling = (secs as u32)
        .saturating_mul(capture_gate::WIDE_RATE_PER_SEC)
        .saturating_add(capture_gate::WIDE_RATE_SLACK);
    for col in 0..GATE_COLS {
        if now[col] <= acc[col] {
            continue;
        }
        let ceiling = if col <= 2 { kill_ceiling } else { wide_ceiling };
        if now[col] - acc[col] > ceiling {
            return true;
        }
    }
    false
}

/// Every mature column (accepted ≥ 4) grew by 2x to 4x. One spiked column
/// is ordinary play. This band is under the rate ceiling and used to replace
/// the baseline.
fn uniform_two_to_four_times(prev: &Counters, cur: &Counters) -> bool {
    let pairs = [
        (prev.elims, cur.elims),
        (prev.assists, cur.assists),
        (prev.deaths, cur.deaths),
        (prev.damage, cur.damage),
        (prev.healing, cur.healing),
        (prev.mitigation, cur.mitigation),
    ];
    let mut comparable = 0u32;
    for (before, after) in pairs {
        if before < 4 {
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

fn counters_progressed(prev: &Counters, cur: &Counters) -> bool {
    let grew = cur.elims > prev.elims
        || cur.assists > prev.assists
        || cur.deaths > prev.deaths
        || cur.damage > prev.damage
        || cur.healing > prev.healing
        || cur.mitigation > prev.mitigation;
    let dropped = cur.elims < prev.elims || cur.deaths < prev.deaths || cur.damage < prev.damage;
    grew && !dropped
}

fn mature_board(prev: &Counters) -> bool {
    prev.elims >= 4 || prev.damage >= 400
}

/// Live-match fresh-start thresholds. A real slot change across a reset is
/// near zero. Another player's totals fail this and re-anchor instead.
fn under_fresh_match(prev: &Counters, cur: &Counters) -> bool {
    let elims_max = 2.max(prev.elims / 4);
    let deaths_max = 1.max(prev.deaths / 4);
    let damage_max = prev.damage / 4;
    cur.elims <= elims_max && cur.deaths <= deaths_max && cur.damage <= damage_max
}

fn edd_clean(suspect: &[bool; GATE_COLS]) -> bool {
    !suspect[0] && !suspect[2] && !suspect[3]
}

fn same_baseline_row(baseline_row: Option<u32>, row_id: Option<u32>) -> bool {
    matches!((baseline_row, row_id), (Some(baseline), Some(row)) if baseline == row)
}

fn is_fresh_reset(
    input: &CapturePlanInput<'_>,
    compare: Option<&GateState>,
    implausible: bool,
) -> bool {
    let Some(prev) = compare else {
        return false;
    };
    same_baseline_row(input.baseline_row, input.row_id)
        && input.row_counts
        && edd_clean(&input.suspect)
        && !implausible
        && mature_board(&prev.accepted)
        && input.age.is_some_and(|age| age >= RESET_MIN_AGE)
        && under_fresh_match(&prev.accepted, &input.cur)
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
/// boundary is not armed and the read has no map. A different map never
/// replaces it.
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
        // Armed by a hero select: the next match's confirmed word on a
        // different map must not overwrite this hint. An unconfirmed word
        // that does not match is [`Effect::Ignore`] from [`unconfirmed_word`].
        // A banner has no map, so it is a confirmed word and still seals
        // onto this session.
        Phase::LiveMatch | Phase::PostResultStreak if state.pending_boundary => Effect::Split {
            reason: CloseReason::StatRegression,
            seal: sealable_hint(state),
            new_outcome: outcome,
        },
        Phase::LiveMatch | Phase::PostResultStreak => Effect::Seal { outcome },
    }
}

fn board_effect(state: &BoundaryState, _phase: Phase, board: &BoardObs) -> Effect {
    if board.fresh_reset {
        if state.reset_streak >= 1 || state.pending_boundary {
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
            if update.clear_hint {
                state.result = None;
                state.pending_boundary = false;
                state.progressed_boards = 0;
            }
            if update.clear_awaiting {
                state.awaiting_first_board = false;
            }
            if let Some(mark) = update.result {
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
            // `adopt_map` is set only when the caller treats the stored map as
            // absent. That includes an untrusted text fallback, so the
            // accolade replaces it.
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
        state.reset_streak = plan.reset_streak;
        state.reset_baseline = plan.reset_baseline;
        state.baseline_row = plan.baseline_row;
        state.deferred = plan.deferred_counters;
        return;
    }
    if plan.refresh_baseline || plan.count_progress || plan.clear_hint {
        state.deferred = None;
    }
    state.gate = Some(accepted);
    state.reset_streak = plan.reset_streak;
    state.reset_baseline = if plan.refresh_baseline {
        Some(accepted)
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
    }
}

pub fn has_post_result(outcome: MatchOutcome, result: Option<ResultMark>) -> bool {
    outcome.is_decided() || result.is_some_and(|mark| mark.outcome.is_decided())
}

fn adopt_map(session_map: Option<&str>, accolade: Option<&str>) -> Option<String> {
    if confident_map(session_map).is_some() {
        return None;
    }
    confident_map(accolade).map(str::to_string)
}

fn confident_map(map: Option<&str>) -> Option<&str> {
    map.map(str::trim).filter(|s| !s.is_empty())
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
        let adopt = adopt_map(trusted_session_map(input), input.accolade_map);
        return if let Some(map) = adopt {
            PollDecision::Update(UpdateCurrent {
                record_outcome: None,
                adopt_map: Some(map),
                result: input.result,
                clear_hint: false,
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
            if let Some(map) = adopt_map(trusted_session_map(input), input.accolade_map) {
                return PollDecision::Update(UpdateCurrent {
                    record_outcome: None,
                    adopt_map: Some(map),
                    result: input.result,
                    clear_hint: false,
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
            adopt_map: adopt_map(trusted_session_map(input), input.accolade_map),
            result: Some(confirmed_mark(input, outcome)),
            clear_hint: false,
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
                    adopt_map: adopt_map(trusted_session_map(input), input.accolade_map),
                    result: Some(result),
                    clear_hint: false,
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
            clear_hint: false,
            arm_pending: true,
            arm_reset: false,
            clear_awaiting: false,
        }),
        Effect::PrimeReset => PollDecision::Update(UpdateCurrent {
            record_outcome: None,
            adopt_map: None,
            result: input.result,
            clear_hint: false,
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
                Obs::EndScreenDifferentMap { map, .. } => {
                    confident_map(Some(map)).map(str::to_string)
                }
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
mod tests {
    use super::*;
    use crate::capture_gate::{HoldKind, apply_gate};

    fn t0() -> Instant {
        Instant::now() + Duration::from_secs(3_600)
    }

    fn counters(e: u32, a: u32, d: u32, dmg: u32, hlg: u32, mit: u32) -> Counters {
        Counters {
            elims: e,
            assists: a,
            deaths: d,
            damage: dmg,
            healing: hlg,
            mitigation: mit,
        }
    }

    fn gate(c: Counters) -> GateState {
        GateState {
            accepted: c,
            last_raw: c,
            ..GateState::default()
        }
    }

    const CLEAN: [bool; GATE_COLS] = [false; GATE_COLS];

    #[allow(clippy::too_many_arguments)]
    fn plan_at(
        prev: &GateState,
        cur: Counters,
        after_end: bool,
        streak: u32,
        classic: bool,
        age_secs: u64,
        suppress: bool,
        row_counts: bool,
        inherited: MatchOutcome,
    ) -> CapturePlan {
        plan_capture(&CapturePlanInput {
            prev_gate: Some(prev),
            baseline: None,
            streak,
            cur,
            suspect: CLEAN,
            create_session: false,
            suppress_same_unfinished: suppress,
            age: Some(Duration::from_secs(age_secs)),
            min_gap: Duration::from_secs(120),
            classic_regressed: classic,
            row_counts,
            row_id: row_counts.then_some(0),
            baseline_row: Some(0),
            confirmed_end: after_end && inherited.is_decided(),
            inherited_outcome: inherited,
            frame_outcome: MatchOutcome::Unknown,
            session_hero: None,
            hint: (after_end && !inherited.is_decided()).then_some(MatchOutcome::Defeat),
            pending_boundary: false,
            awaiting_first_board: false,
            baseline_age: None,
            progressed_boards: 0,
            session_map: None,
            incoming_map: None,
            session_map_source: None,
        })
    }

    /// Poll + Tab stand-in. State changes go through [`commit_poll`],
    /// [`plan_capture`], and [`note_accepted_capture`] — the functions `main` calls.
    struct Machine {
        active: Option<Sess>,
        closed: Vec<Closed>,
        n: u32,
    }

    struct Closed {
        sess: Sess,
        reason: CloseReason,
    }

    #[derive(Clone)]
    struct Sess {
        id: String,
        state: BoundaryState,
        /// First board of this session after a split. Later tabs replace the gate.
        opened_with: Option<Counters>,
    }

    impl Sess {
        fn outcome(&self) -> MatchOutcome {
            self.state.outcome
        }
        fn map(&self) -> Option<&str> {
            self.state.map.as_deref()
        }
    }

    impl Machine {
        fn new(map: &str) -> Self {
            Self {
                active: Some(Sess {
                    id: "busan".into(),
                    state: BoundaryState::new(Some(map.into())),
                    opened_with: None,
                }),
                closed: Vec::new(),
                n: 0,
            }
        }

        fn active(&self) -> &Sess {
            self.active.as_ref().expect("active session")
        }

        fn poll(&mut self, mut build: impl for<'a> FnMut(&'a BoundaryState) -> PollInput<'a>) {
            let Some(sess) = self.active.as_mut() else {
                return;
            };
            let (decision, now) = {
                let input = build(&sess.state);
                (decide_poll(&input), input.now)
            };
            let mut previous = sess.state.clone();
            let commit = commit_poll(&mut sess.state, decision, now);
            if let Some(closed) = commit.closed {
                if let Some(outcome) = closed.seal {
                    previous.outcome = outcome;
                    previous.outcome_at = Some(now);
                }
                self.n += 1;
                let id = format!("s{}", self.n);
                let prev = Sess {
                    id: std::mem::replace(&mut sess.id, id),
                    state: previous,
                    opened_with: std::mem::take(&mut sess.opened_with),
                };
                self.closed.push(Closed {
                    sess: prev,
                    reason: closed.reason,
                });
            }
        }

        fn capture(&mut self, cur: Counters, now: Instant) {
            self.capture_board(cur, None, Some(0), now);
        }

        fn capture_board(
            &mut self,
            cur: Counters,
            hero: Option<&str>,
            row_id: Option<u32>,
            now: Instant,
        ) {
            let sess = self.active.as_mut().expect("active");
            let age = sess
                .state
                .last_board_at
                .map(|at| now.saturating_duration_since(at))
                .unwrap_or(Duration::from_secs(30));
            let baseline_age = sess
                .state
                .baseline_at
                .map(|at| now.saturating_duration_since(at));
            let classic = sess.state.gate.as_ref().is_some_and(|gate| {
                age >= Duration::from_secs(120) && post_result_stat_reset(gate, cur, CLEAN)
            });
            let plan = plan_capture(&CapturePlanInput {
                prev_gate: sess.state.gate.as_ref(),
                baseline: sess.state.reset_baseline.as_ref(),
                streak: sess.state.reset_streak,
                cur,
                suspect: CLEAN,
                create_session: false,
                // Same map and hero stay suppressed. A capture that names no
                // other map is this match; the night harness is the path
                // where a different map leaves the gap on.
                suppress_same_unfinished: sess.state.map.is_some(),
                age: sess.state.gate.map(|_| age),
                min_gap: Duration::from_secs(120),
                classic_regressed: classic,
                row_counts: row_id.is_some(),
                row_id,
                baseline_row: sess.state.baseline_row,
                confirmed_end: sess.state.outcome.is_decided(),
                inherited_outcome: sess.state.outcome,
                frame_outcome: MatchOutcome::Unknown,
                session_hero: sess.state.hero.as_deref(),
                hint: sess
                    .state
                    .result
                    .filter(|mark| !mark.confirmed && mark.outcome.is_decided())
                    .map(|mark| mark.outcome),
                pending_boundary: sess.state.pending_boundary,
                awaiting_first_board: sess.state.awaiting_first_board,
                baseline_age,
                progressed_boards: sess.state.progressed_boards,
                session_map: sess.state.map.as_deref(),
                incoming_map: sess.state.map.as_deref(),
                session_map_source: None,
            });
            if plan.split {
                let held = sess.state.deferred;
                let new_outcome = plan.stored_outcome;
                let mut previous = sess.state.clone();
                let commit = commit_poll(
                    &mut sess.state,
                    PollDecision::Open(OpenNew {
                        reason: plan.close_reason.unwrap_or(CloseReason::StatReset),
                        seal_outcome: plan.seal,
                        new_outcome,
                        new_map: None,
                        candidates: Vec::new(),
                        new_result: None,
                    }),
                    now,
                );
                let closed = commit.closed.expect("split closes the session");
                if let Some(outcome) = closed.seal {
                    previous.outcome = outcome;
                    previous.outcome_at = Some(now);
                }
                self.n += 1;
                let id = format!("s{}", self.n);
                let prev = Sess {
                    id: std::mem::replace(&mut sess.id, id),
                    state: previous,
                    opened_with: std::mem::take(&mut sess.opened_with),
                };
                self.closed.push(Closed {
                    sess: prev,
                    reason: closed.reason,
                });
                sess.state.outcome = new_outcome;
                sess.state.outcome_at = new_outcome.is_decided().then_some(now);
                sess.state.gate = Some(gate(cur));
                sess.state.last_board_at = Some(now);
                sess.state.hero = hero.map(str::to_string);
                sess.state.awaiting_first_board = false;
                sess.opened_with = held.or(Some(cur));
                return;
            }
            let accepted = match sess.state.gate {
                Some(prev) => apply_gate(Some((prev, age)), cur, CLEAN, false).state,
                None => gate(cur),
            };
            note_accepted_capture(&mut sess.state, &plan, accepted, now);
            if let Some(hero) = hero
                && !plan.ignore_row
                && !plan.defer
            {
                sess.state.hero = Some(hero.to_string());
            }
        }
    }

    fn confirm_defeat(m: &mut Machine, now: Instant) {
        // First word is a streak; the agreeing read confirms.
        m.poll(|s| {
            let mut i = poll_of(s, now);
            i.signal = Some(MatchOutcome::Defeat);
            i.signal_confirmed = false;
            i
        });
        assert!(
            !m.active().outcome().is_decided(),
            "a single defeat word is a streak, not a stored outcome"
        );
        m.poll(|s| {
            let mut i = poll_of(s, now + Duration::from_secs(4));
            i.signal = Some(MatchOutcome::Defeat);
            i.signal_confirmed = true;
            i
        });
        assert_eq!(m.active().outcome(), MatchOutcome::Defeat);
        assert_eq!(m.active().map(), Some("Busan"));
    }

    fn poll_of(s: &BoundaryState, now: Instant) -> PollInput<'_> {
        PollInput {
            outcome: s.outcome,
            result: s.result,
            pending_boundary: s.pending_boundary,
            awaiting_first_board: s.awaiting_first_board,
            has_board: s.gate.is_some(),
            reset_streak: s.reset_streak,
            map: s.map.as_deref(),
            map_trusted: true,
            hero: s.hero.as_deref(),
            signal: None,
            signal_confirmed: false,
            accolade_map: None,
            start_screen: None,
            block_map_vote: false,
            deferred: s.deferred.is_some(),
            now,
        }
    }

    #[test]
    fn busan_defeat_then_hero_select_then_junkertown_victory() {
        let now = t0();
        let mut m = Machine::new("Busan");
        m.capture(
            counters(14, 22, 6, 2400, 9800, 400),
            now + Duration::from_secs(60),
        );
        confirm_defeat(&mut m, now + Duration::from_secs(8 * 60));

        // Fast requeue: no map-vote frame. Hero select closes Busan.
        m.poll(|s| {
            let mut i = poll_of(s, now + Duration::from_secs(8 * 60 + 20));
            i.start_screen = Some(StartScreen::HeroSelect);
            i
        });
        assert_eq!(m.closed.len(), 1);
        assert_eq!(m.closed[0].reason, CloseReason::HeroSelectOrBan);
        assert_eq!(m.closed[0].sess.map(), Some("Busan"));
        assert_eq!(m.closed[0].sess.outcome(), MatchOutcome::Defeat);
        assert_eq!(m.active().id, "s1");
        assert!(m.active().map().is_none());

        m.capture(
            counters(2, 1, 0, 800, 200, 1500),
            now + Duration::from_secs(9 * 60),
        );
        assert_eq!(
            m.active().id,
            "s1",
            "junkertown tabs stay on the new session"
        );

        m.poll(|s| {
            let mut i = poll_of(s, now + Duration::from_secs(19 * 60));
            i.signal = Some(MatchOutcome::Victory);
            i.signal_confirmed = true;
            i.accolade_map = Some("Junkertown");
            i
        });
        // Second agreeing tick is what the poller calls confirmed; the first
        // word only marks the streak. Drive the confirm the way the daemon
        // does: one confirmed signal on a session that has no prior result.
        assert_eq!(m.active().outcome(), MatchOutcome::Victory);
        assert_eq!(m.active().map(), Some("Junkertown"));
        assert_eq!(m.closed.len(), 1);
        assert_eq!(m.closed[0].sess.outcome(), MatchOutcome::Defeat);
    }

    #[test]
    fn busan_defeat_then_stat_reset_without_map_vote() {
        let now = t0();
        let mut m = Machine::new("Busan");
        m.capture(
            counters(18, 7, 9, 6400, 11000, 800),
            now + Duration::from_secs(60),
        );
        confirm_defeat(&mut m, now + Duration::from_secs(8 * 60));

        // First Junkertown board. Same-match suppress is true and the age is
        // 10s. One drop only arms the streak — it must not inherit nothing yet,
        // and it must not open a session.
        let first = counters(1, 3, 0, 220, 80, 400);
        m.capture(first, now + Duration::from_secs(8 * 60 + 25));
        assert!(
            m.closed.is_empty(),
            "one post-result drop does not open a session"
        );
        assert_eq!(m.active().outcome(), MatchOutcome::Defeat);
        assert_eq!(m.active().state.reset_streak, 1);
        assert_eq!(
            m.active().state.gate.map(|g| g.accepted.elims),
            Some(18),
            "the first drop is not written onto the finished game"
        );

        // Second consecutive drop. The new session keeps the raw counters and
        // none of Busan's outcome, hint, or grace.
        m.capture(
            counters(2, 4, 1, 400, 120, 700),
            now + Duration::from_secs(8 * 60 + 40),
        );
        assert_eq!(m.closed.len(), 1, "the second drop opens the session");
        assert_eq!(m.closed[0].reason, CloseReason::StatReset);
        assert_eq!(m.closed[0].sess.outcome(), MatchOutcome::Defeat);
        assert_eq!(m.closed[0].sess.map(), Some("Busan"));
        assert!(
            !m.active().outcome().is_decided(),
            "the junkertown session must not be stored as Busan's defeat"
        );
        assert!(m.active().state.outcome_at.is_none());
        assert!(m.active().state.result.is_none());
        assert_eq!(
            m.active().state.gate.map(|g| g.accepted.elims),
            Some(2),
            "the new session keeps the raw reset, it does not hold Busan's elims"
        );

        m.capture(
            counters(4, 5, 1, 900, 300, 2200),
            now + Duration::from_secs(12 * 60),
        );
        m.poll(|s| {
            let mut i = poll_of(s, now + Duration::from_secs(19 * 60));
            i.signal = Some(MatchOutcome::Victory);
            i.signal_confirmed = true;
            i.accolade_map = Some("Junkertown");
            i
        });
        assert_eq!(m.active().outcome(), MatchOutcome::Victory);
        assert_eq!(m.active().map(), Some("Junkertown"));
        assert_eq!(m.closed.len(), 1);
    }

    #[test]
    fn hero_select_after_a_board_arms_and_the_next_board_drops_the_hint() {
        // Live shape: one defeat word (`poll_streak_defeat`), queue pops
        // before the agreeing read, hero select with no map vote.
        let now = t0();
        let mut m = Machine::new("Busan");
        m.capture(counters(11, 20, 5, 2100, 8000, 100), now);
        m.poll(|s| {
            let mut i = poll_of(s, now + Duration::from_secs(30));
            i.signal = Some(MatchOutcome::Defeat);
            i.signal_confirmed = false;
            i
        });
        assert!(!m.active().outcome().is_decided());
        m.poll(|s| {
            let mut i = poll_of(s, now + Duration::from_secs(45));
            i.start_screen = Some(StartScreen::HeroSelect);
            i
        });
        assert!(
            m.closed.is_empty(),
            "a hero select after a board-followed hint arms, it does not seal"
        );
        assert!(m.active().state.pending_boundary);
        assert_eq!(
            m.active().state.result.map(|mark| mark.outcome),
            Some(MatchOutcome::Defeat)
        );
        assert!(!m.active().outcome().is_decided());
        m.capture(
            counters(14, 22, 6, 2600, 9000, 200),
            now + Duration::from_secs(70),
        );
        assert!(
            m.active().state.result.is_none(),
            "a progressed board after the armed start screen drops the hint"
        );
        assert!(!m.active().state.pending_boundary);
        assert!(m.closed.is_empty());
    }

    #[test]
    fn hero_ban_is_a_boundary_after_a_result() {
        let now = t0();
        let mut m = Machine::new("Busan");
        confirm_defeat(&mut m, now);
        m.poll(|s| {
            let mut i = poll_of(s, now + Duration::from_secs(15));
            i.start_screen = Some(StartScreen::HeroBan);
            i
        });
        assert_eq!(m.closed.len(), 1);
        assert_eq!(m.closed[0].reason, CloseReason::HeroSelectOrBan);
        assert_eq!(m.closed[0].sess.outcome(), MatchOutcome::Defeat);
    }

    #[test]
    fn long_post_match_screen_does_not_split_on_time_alone() {
        let now = t0();
        let mut m = Machine::new("Busan");
        confirm_defeat(&mut m, now);
        // Accolade / rank still up. The clock passed two minutes and the map
        // OCR disagrees, but nothing has left this post-match flow.
        m.poll(|s| {
            let mut i = poll_of(s, now + Duration::from_secs(120) + Duration::from_secs(30));
            i.signal = Some(MatchOutcome::Defeat);
            i.signal_confirmed = true;
            i.accolade_map = Some("Junkertown");
            i
        });
        assert!(m.closed.is_empty());
        assert_eq!(m.active().outcome(), MatchOutcome::Defeat);
        assert_eq!(m.active().map(), Some("Busan"));
        // A contradictory word with only that clock gap is ignored too.
        m.poll(|s| {
            let mut i = poll_of(s, now + Duration::from_secs(120) + Duration::from_secs(40));
            i.signal = Some(MatchOutcome::Victory);
            i.signal_confirmed = true;
            i
        });
        assert!(m.closed.is_empty());
        assert_eq!(m.active().outcome(), MatchOutcome::Defeat);
        assert_eq!(m.active().map(), Some("Busan"));
    }

    #[test]
    fn contradictory_result_without_a_gap_does_not_overwrite() {
        let now = t0();
        let mut m = Machine::new("Busan");
        confirm_defeat(&mut m, now);
        m.poll(|s| {
            let mut i = poll_of(s, now + Duration::from_secs(20));
            i.signal = Some(MatchOutcome::Victory);
            i.signal_confirmed = true;
            i.accolade_map = Some("Junkertown");
            i
        });
        assert!(m.closed.is_empty());
        assert_eq!(m.active().outcome(), MatchOutcome::Defeat);
        assert_eq!(
            m.active().map(),
            Some("Busan"),
            "a same-screen map read must not replace the session map"
        );
    }

    #[test]
    fn rising_scoreboard_between_results_does_not_split() {
        let now = t0();
        let mut m = Machine::new("Busan");
        m.capture(counters(10, 4, 3, 3000, 1000, 0), now);
        confirm_defeat(&mut m, now + Duration::from_secs(30));
        // Same match, stats still climbing. That tab is not a new game, and
        // a later contradictory word must not split off a second finished session.
        m.capture(
            counters(12, 5, 3, 3400, 1400, 0),
            now + Duration::from_secs(40),
        );
        assert!(m.closed.is_empty());
        m.poll(|s| {
            let mut i = poll_of(s, now + Duration::from_secs(70));
            i.signal = Some(MatchOutcome::Victory);
            i.signal_confirmed = true;
            i.accolade_map = Some("Junkertown");
            i
        });
        assert!(
            m.closed.is_empty(),
            "a rising scoreboard is not a gap that opens another finished game"
        );
        assert_eq!(m.active().outcome(), MatchOutcome::Defeat);
        assert_eq!(m.active().map(), Some("Busan"));
    }

    #[test]
    fn accolade_map_mismatch_after_a_scoreboard_does_not_split() {
        let now = t0();
        let mut m = Machine::new("Busan");
        m.capture(counters(10, 4, 3, 3000, 1000, 0), now);
        confirm_defeat(&mut m, now + Duration::from_secs(30));
        // Rank-screen tab: stats did not reset. A misread map on the next
        // accolade must not open a second session.
        m.capture(
            counters(11, 4, 3, 3200, 1100, 0),
            now + Duration::from_secs(50),
        );
        m.poll(|s| {
            let mut i = poll_of(s, now + Duration::from_secs(70));
            i.signal = Some(MatchOutcome::Defeat);
            i.signal_confirmed = true;
            i.accolade_map = Some("Junkertown");
            i
        });
        assert!(
            m.closed.is_empty(),
            "a map misread after a same-match scoreboard does not split"
        );
        assert_eq!(m.active().map(), Some("Busan"));
        assert_eq!(m.active().outcome(), MatchOutcome::Defeat);
    }

    #[test]
    fn end_reel_wake_does_not_split_or_open_a_gap() {
        // `decide_poll` has no end-reel input. The wake only changes poll
        // cadence in `main`. A tick with no new-game screen stays on Busan.
        let now = t0();
        let mut m = Machine::new("Busan");
        confirm_defeat(&mut m, now);
        m.poll(|s| poll_of(s, now + Duration::from_secs(10)));
        assert!(m.closed.is_empty());
        assert_eq!(m.active().outcome(), MatchOutcome::Defeat);
        // A contradictory word still inside the couple-of-minutes window,
        // with only the POTG wake in between, stays on Busan.
        m.poll(|s| {
            let mut i = poll_of(s, now + Duration::from_secs(30));
            i.signal = Some(MatchOutcome::Victory);
            i.signal_confirmed = true;
            i
        });
        assert!(m.closed.is_empty());
        assert_eq!(m.active().outcome(), MatchOutcome::Defeat);
        assert_eq!(m.active().map(), Some("Busan"));
    }

    #[test]
    fn hero_select_mid_match_does_not_open_a_session() {
        let now = t0();
        let mut m = Machine::new("Busan");
        m.poll(|s| {
            let mut i = poll_of(s, now + Duration::from_secs(30));
            i.start_screen = Some(StartScreen::HeroSelect);
            i
        });
        assert!(m.closed.is_empty());
        assert_eq!(m.active().id, "busan");
    }

    #[test]
    fn single_cell_ocr_noise_is_held_and_does_not_split() {
        let prev_c = counters(22, 7, 5, 6400, 11000, 800);
        let prev = gate(prev_c);
        let mut cur = prev_c;
        cur.elims = 2;
        assert!(
            !post_result_stat_reset(&prev, cur, CLEAN),
            "one sharp column is an OCR miss, not a new game"
        );
        let held = plan_at(
            &prev,
            cur,
            true,
            0,
            false,
            5,
            true,
            true,
            MatchOutcome::Defeat,
        );
        assert!(
            !held.split,
            "after an end screen a single-column drop still does not split"
        );
        let early = plan_at(
            &prev,
            counters(1, 1, 0, 100, 100, 100),
            false,
            0,
            true,
            5,
            false,
            true,
            MatchOutcome::Unknown,
        );
        assert!(
            !early.split,
            "mid-match, even a classic regression waits out the gap"
        );
        let gated = apply_gate(Some((prev, Duration::from_secs(30))), cur, CLEAN, false);
        assert!(
            gated
                .holds
                .iter()
                .any(|h| h.col == 0 && h.kind == HoldKind::Monotonic && h.raw == 2),
            "the gate still holds the noisy cell: {holds:?}",
            holds = gated.holds
        );
        assert_eq!(gated.accepted.elims, 22);
    }

    #[test]
    fn post_result_reset_is_several_columns_and_ignores_the_mid_match_gap() {
        let prev = gate(counters(18, 7, 9, 6400, 11000, 800));
        let cur = counters(1, 3, 0, 220, 80, 400);
        assert!(post_result_stat_reset(&prev, cur, CLEAN));
        let first = plan_at(
            &prev,
            cur,
            true,
            0,
            true,
            200,
            true,
            true,
            MatchOutcome::Defeat,
        );
        assert!(
            !first.split,
            "one sharp drop does not split, even past the mid-match gap"
        );
        assert!(
            first.defer,
            "the first drop is not written onto the finished game"
        );
        assert_eq!(first.reset_streak, 1);
        assert_eq!(first.stored_outcome, MatchOutcome::Defeat);
        let second = plan_capture(&CapturePlanInput {
            prev_gate: Some(&prev),
            baseline: first.reset_baseline.as_ref(),
            streak: first.reset_streak,
            cur: counters(3, 4, 1, 500, 200, 600),
            suspect: CLEAN,
            create_session: false,
            suppress_same_unfinished: true,
            age: Some(Duration::from_secs(50)),
            min_gap: Duration::from_secs(120),
            classic_regressed: true,
            row_counts: true,
            row_id: Some(0),
            baseline_row: Some(0),
            confirmed_end: true,
            inherited_outcome: MatchOutcome::Defeat,
            frame_outcome: MatchOutcome::Victory,
            session_hero: None,
            hint: None,
            pending_boundary: false,
            awaiting_first_board: false,
            baseline_age: None,
            progressed_boards: 0,
            session_map: None,
            incoming_map: None,
            session_map_source: None,
        });
        assert!(second.split, "the second consecutive drop splits");
        assert_eq!(
            second.stored_outcome,
            MatchOutcome::Unknown,
            "a split does not keep the header result from the board being closed"
        );
        assert!(
            second.seal.is_none(),
            "a confirmed result is already stored"
        );
        // Healing + damage + elims, deaths not sharp (9 → 6).
        let cur = counters(4, 8, 6, 900, 400, 900);
        assert!(post_result_stat_reset(&prev, cur, CLEAN));
    }

    #[test]
    fn first_accolade_fills_an_empty_map_and_does_not_split() {
        let now = t0();
        let decision = decide_poll(&PollInput {
            outcome: MatchOutcome::Unknown,
            result: None,
            pending_boundary: false,
            has_board: false,
            reset_streak: 0,
            block_map_vote: false,
            awaiting_first_board: false,
            map: None,
            map_trusted: false,
            hero: None,
            signal: Some(MatchOutcome::Defeat),
            signal_confirmed: true,
            accolade_map: Some("Busan"),
            start_screen: None,
            now,

            deferred: false,
        });
        match decision {
            PollDecision::Update(u) => {
                assert_eq!(u.record_outcome, Some(MatchOutcome::Defeat));
                assert_eq!(u.adopt_map.as_deref(), Some("Busan"));
            }
            other => panic!("expected update, got {other:?}"),
        }
    }

    #[test]
    fn confirmed_victory_replaces_an_unconfirmed_defeat_hint() {
        let now = t0();
        let mut m = Machine::new("Busan");
        m.poll(|s| {
            let mut i = poll_of(s, now);
            i.signal = Some(MatchOutcome::Defeat);
            i.signal_confirmed = false;
            i
        });
        assert!(!m.active().outcome().is_decided());
        m.poll(|s| {
            let mut i = poll_of(s, now + Duration::from_secs(8));
            i.signal = Some(MatchOutcome::Victory);
            i.signal_confirmed = true;
            i
        });
        assert!(m.closed.is_empty(), "the confirm stays on this session");
        assert_eq!(m.active().outcome(), MatchOutcome::Victory);
        assert_eq!(m.active().map(), Some("Busan"));
    }

    #[test]
    fn hint_plus_later_result_does_not_finish_two_sessions() {
        let now = t0();
        let mut m = Machine::new("Busan");
        m.capture(counters(10, 4, 3, 3000, 1000, 0), now);
        m.poll(|s| {
            let mut i = poll_of(s, now + Duration::from_secs(20));
            i.signal = Some(MatchOutcome::Defeat);
            i.signal_confirmed = false;
            i
        });
        m.capture(
            counters(12, 5, 3, 3400, 1200, 0),
            now + Duration::from_secs(30),
        );
        m.poll(|s| {
            let mut i = poll_of(s, now + Duration::from_secs(40));
            i.signal = Some(MatchOutcome::Victory);
            i.signal_confirmed = true;
            i.accolade_map = Some("Junkertown");
            i
        });
        assert!(
            m.closed.is_empty(),
            "a confirmed read wins in place; it does not seal the hint onto a second session"
        );
        assert_eq!(m.active().outcome(), MatchOutcome::Victory);
    }

    #[test]
    fn idle_close_does_not_seal_a_provisional_word() {
        let hint = ResultMark {
            outcome: MatchOutcome::Defeat,
            confirmed: false,
            seen_at: t0(),
        };
        assert_eq!(
            outcome_sealed_on_close(MatchOutcome::Unknown, None),
            None,
            "an idle Tab passes no seal; the stray word must not become the outcome"
        );
        assert_eq!(
            outcome_sealed_on_close(MatchOutcome::Unknown, Some(hint.outcome)),
            Some(MatchOutcome::Defeat),
            "a start screen is the explicit second signal that may seal the hint"
        );
        assert_eq!(
            outcome_sealed_on_close(MatchOutcome::Victory, Some(MatchOutcome::Defeat)),
            None,
            "a stored outcome is not sealed a second time"
        );
    }

    #[test]
    fn garbage_and_unidentified_rows_do_not_count_toward_a_reset() {
        let prev = gate(counters(18, 7, 9, 6400, 11000, 800));
        let garbage = counters(9, 11, 11, 61029, 100, 100);
        let ignored = plan_at(
            &prev,
            garbage,
            true,
            1,
            true,
            86,
            false,
            false,
            MatchOutcome::Defeat,
        );
        assert!(!ignored.split);
        assert!(!ignored.defer);
        assert_eq!(
            ignored.reset_streak, 1,
            "an unidentified row leaves the streak and a held board alone"
        );
        let one = plan_at(
            &prev,
            counters(1, 3, 0, 220, 80, 400),
            true,
            0,
            true,
            86,
            true,
            true,
            MatchOutcome::Defeat,
        );
        assert!(!one.split);
        assert!(one.defer);
        // 2026-07-14 field case. The garbage row reads as an increase on
        // damage and a drop on elims. It must not become the baseline, so
        // the next real board is not "one capture later" from a split.
        let mixed = plan_capture(&CapturePlanInput {
            prev_gate: Some(&prev),
            baseline: Some(&prev),
            streak: 0,
            cur: garbage,
            suspect: CLEAN,
            create_session: false,
            suppress_same_unfinished: false,
            age: Some(Duration::from_secs(86)),
            min_gap: Duration::from_secs(120),
            classic_regressed: true,
            row_counts: true,
            row_id: Some(0),
            baseline_row: Some(0),
            confirmed_end: true,
            inherited_outcome: MatchOutcome::Defeat,
            frame_outcome: MatchOutcome::Defeat,
            session_hero: None,
            hint: None,
            pending_boundary: false,
            awaiting_first_board: false,
            baseline_age: None,
            progressed_boards: 0,
            session_map: None,
            incoming_map: None,
            session_map_source: None,
        });
        assert!(
            !mixed.ignore_row && !mixed.skip_store,
            "an implausible row is stored"
        );
        assert!(!mixed.refresh_baseline, "it does not become the baseline");
        assert!(!mixed.split);
        assert_eq!(mixed.reset_streak, 0);
        let follow = plan_capture(&CapturePlanInput {
            prev_gate: Some(&prev),
            baseline: Some(&prev),
            streak: 0,
            cur: counters(20, 8, 10, 7000, 12000, 900),
            suspect: CLEAN,
            create_session: false,
            suppress_same_unfinished: false,
            age: Some(Duration::from_secs(90)),
            min_gap: Duration::from_secs(120),
            classic_regressed: false,
            row_counts: true,
            row_id: Some(0),
            baseline_row: Some(0),
            confirmed_end: true,
            inherited_outcome: MatchOutcome::Defeat,
            frame_outcome: MatchOutcome::Unknown,
            session_hero: None,
            hint: None,
            pending_boundary: false,
            awaiting_first_board: false,
            baseline_age: None,
            progressed_boards: 0,
            session_map: None,
            incoming_map: None,
            session_map_source: None,
        });
        assert!(!follow.split);
        assert_eq!(follow.reset_streak, 0);
        // A low-total teammate row is a different slot. It is stored, it does
        // not count as a fresh reset, and it does not replace the baseline.
        let moved = plan_capture(&CapturePlanInput {
            prev_gate: Some(&prev),
            baseline: Some(&prev),
            streak: 0,
            cur: counters(1, 3, 0, 220, 80, 400),
            suspect: CLEAN,
            create_session: false,
            suppress_same_unfinished: true,
            age: Some(Duration::from_secs(50)),
            min_gap: Duration::from_secs(120),
            classic_regressed: true,
            row_counts: true,
            row_id: Some(4),
            baseline_row: Some(0),
            confirmed_end: true,
            inherited_outcome: MatchOutcome::Defeat,
            frame_outcome: MatchOutcome::Victory,
            session_hero: None,
            hint: None,
            pending_boundary: false,
            awaiting_first_board: false,
            baseline_age: None,
            progressed_boards: 0,
            session_map: None,
            incoming_map: None,
            session_map_source: None,
        });
        assert!(
            !moved.split && !moved.defer,
            "a different row is not a fresh reset"
        );
        assert!(!moved.refresh_baseline);
        assert_eq!(moved.baseline_row, Some(0));
        assert!(!moved.ignore_row);
        assert_eq!(moved.reset_streak, 0);
        let moved_again = plan_capture(&CapturePlanInput {
            prev_gate: Some(&prev),
            baseline: Some(&prev),
            streak: 1,
            cur: counters(0, 1, 0, 80, 20, 100),
            suspect: CLEAN,
            create_session: false,
            suppress_same_unfinished: true,
            age: Some(Duration::from_secs(70)),
            min_gap: Duration::from_secs(120),
            classic_regressed: true,
            row_counts: true,
            row_id: Some(4),
            baseline_row: Some(0),
            confirmed_end: true,
            inherited_outcome: MatchOutcome::Defeat,
            frame_outcome: MatchOutcome::Victory,
            session_hero: None,
            hint: None,
            pending_boundary: false,
            awaiting_first_board: false,
            baseline_age: None,
            progressed_boards: 0,
            session_map: None,
            incoming_map: None,
            session_map_source: None,
        });
        assert!(
            !moved_again.split && !moved_again.defer,
            "a second read of a different row is still not a reset"
        );
        assert_eq!(moved_again.baseline_row, Some(0));
        assert_eq!(moved_again.stored_outcome, MatchOutcome::Defeat);
        // The garbage row must not become the baseline, so the next real
        // drop is the first of the pair, not a split one capture later.
        let mut state = BoundaryState::new(Some("Busan".into()));
        state.outcome = MatchOutcome::Defeat;
        state.gate = Some(prev);
        state.reset_baseline = Some(prev);
        state.baseline_row = Some(0);
        note_accepted_capture(&mut state, &mixed, prev, t0());
        assert_eq!(state.reset_baseline, Some(prev));
        assert_eq!(state.gate, Some(prev));
        assert_eq!(state.reset_streak, 0);
        let later = plan_capture(&CapturePlanInput {
            prev_gate: state.gate.as_ref(),
            baseline: state.reset_baseline.as_ref(),
            streak: state.reset_streak,
            cur: counters(1, 3, 0, 220, 80, 400),
            suspect: CLEAN,
            create_session: false,
            suppress_same_unfinished: true,
            age: Some(Duration::from_secs(50)),
            min_gap: Duration::from_secs(120),
            classic_regressed: true,
            row_counts: true,
            row_id: Some(0),
            baseline_row: state.baseline_row,
            confirmed_end: true,
            inherited_outcome: MatchOutcome::Defeat,
            frame_outcome: MatchOutcome::Victory,
            session_hero: None,
            hint: None,
            pending_boundary: false,
            awaiting_first_board: false,
            baseline_age: None,
            progressed_boards: 0,
            session_map: None,
            incoming_map: None,
            session_map_source: None,
        });
        assert!(later.defer, "the real drop after garbage only arms");
        assert!(!later.split, "one capture later is not a new session");
        assert_eq!(later.stored_outcome, MatchOutcome::Defeat);
    }

    #[test]
    fn time_does_not_drop_a_sealable_hint() {
        let now = t0();
        let mut m = Machine::new("Busan");
        m.poll(|s| {
            let mut i = poll_of(s, now);
            i.signal = Some(MatchOutcome::Defeat);
            i.signal_confirmed = false;
            i
        });
        assert!(m.active().state.result.is_some());
        m.poll(|s| poll_of(s, now + Duration::from_secs(180)));
        assert!(
            m.active().state.result.is_some(),
            "a hint stays sealable until a clean board arrives after it"
        );
        assert_eq!(phase_of(&m.active().state), Phase::PostResultStreak);
    }

    #[test]
    fn newest_unconfirmed_word_replaces_an_older_hint() {
        let now = t0();
        let mut m = Machine::new("Busan");
        m.poll(|s| {
            let mut i = poll_of(s, now);
            i.signal = Some(MatchOutcome::Defeat);
            i.signal_confirmed = false;
            i
        });
        m.poll(|s| {
            let mut i = poll_of(s, now + Duration::from_secs(5));
            i.signal = Some(MatchOutcome::Victory);
            i.signal_confirmed = false;
            i
        });
        assert!(m.closed.is_empty());
        assert!(!m.active().outcome().is_decided());
        assert_eq!(
            m.active().state.result.map(|mark| mark.outcome),
            Some(MatchOutcome::Victory),
            "the newest unconfirmed word replaces the older hint"
        );
    }

    #[test]
    fn unconfirmed_word_does_not_split_a_confirmed_result() {
        let now = t0();
        let mut m = Machine::new("Busan");
        confirm_defeat(&mut m, now);
        m.capture(
            counters(12, 5, 3, 3400, 1400, 0),
            now + Duration::from_secs(20),
        );
        m.poll(|s| {
            let mut i = poll_of(s, now + Duration::from_secs(30));
            i.signal = Some(MatchOutcome::Victory);
            i.signal_confirmed = false;
            i.accolade_map = Some("Junkertown");
            i
        });
        assert!(m.closed.is_empty());
        assert_eq!(m.active().outcome(), MatchOutcome::Defeat);
        assert_eq!(m.active().map(), Some("Busan"));
    }

    #[test]
    fn word_then_start_screen_seals_without_a_clock_window() {
        // A board plus one word, then a map vote. Ninety seconds later is
        // the same as three: there is no hint timer. The vote seals.
        let now = t0();
        let mut m = Machine::new("Busan");
        m.capture(counters(11, 20, 5, 2100, 8000, 100), now);
        m.poll(|s| {
            let mut i = poll_of(s, now + Duration::from_secs(2));
            i.signal = Some(MatchOutcome::Victory);
            i.signal_confirmed = false;
            i
        });
        m.poll(|s| {
            let mut i = poll_of(s, now + Duration::from_secs(90));
            i.start_screen = Some(StartScreen::MapVote {
                candidates: vec!["Junkertown".into()],
            });
            i
        });
        assert_eq!(m.closed.len(), 1);
        assert_eq!(m.closed[0].sess.outcome(), MatchOutcome::Victory);
        assert!(!m.active().outcome().is_decided());
    }

    #[test]
    fn live_board_continuation_clears_a_hint() {
        let now = t0();
        let mut m = Machine::new("Busan");
        m.capture(counters(8, 3, 2, 1800, 4000, 100), now);
        m.poll(|s| {
            let mut i = poll_of(s, now + Duration::from_secs(2));
            i.signal = Some(MatchOutcome::Victory);
            i.signal_confirmed = false;
            i
        });
        assert!(m.active().state.result.is_some());
        m.capture(
            counters(10, 4, 2, 2100, 4500, 120),
            now + Duration::from_secs(10),
        );
        assert_eq!(
            m.active().state.result.map(|mark| mark.outcome),
            Some(MatchOutcome::Victory),
            "the first progressed board keeps the hint"
        );
        m.capture(
            counters(12, 5, 3, 2500, 5000, 160),
            now + Duration::from_secs(20),
        );
        assert!(
            m.active().state.result.is_none(),
            "the second progressed board drops the hint"
        );
        assert!(m.closed.is_empty());
    }

    #[test]
    fn confirmed_grace_starts_at_the_confirming_tick() {
        let now = t0();
        let mut state = BoundaryState::new(Some("Busan".into()));
        state.result = Some(ResultMark {
            outcome: MatchOutcome::Defeat,
            confirmed: false,
            seen_at: now - Duration::from_secs(200),
        });
        let decision = decide_poll(&PollInput {
            outcome: MatchOutcome::Unknown,
            result: state.result,
            pending_boundary: false,
            has_board: false,
            reset_streak: 0,
            block_map_vote: false,
            awaiting_first_board: false,
            map: Some("Busan"),
            map_trusted: true,
            hero: None,
            signal: Some(MatchOutcome::Defeat),
            signal_confirmed: true,
            accolade_map: None,
            start_screen: None,
            now,

            deferred: false,
        });
        let commit = commit_poll(&mut state, decision, now);
        assert_eq!(commit.recorded_outcome, Some(MatchOutcome::Defeat));
        assert_eq!(
            state.outcome_at,
            Some(now),
            "grace starts when the result is confirmed, not at the first sighting"
        );
    }

    #[test]
    fn hint_only_session_still_splits_on_the_stat_gap() {
        let prev = gate(counters(29, 8, 5, 9242, 1000, 200));
        let plan = plan_capture(&CapturePlanInput {
            prev_gate: Some(&prev),
            baseline: None,
            streak: 0,
            cur: counters(3, 1, 0, 200, 50, 10),
            suspect: CLEAN,
            create_session: false,
            suppress_same_unfinished: false,
            age: Some(Duration::from_secs(120) + Duration::from_secs(5)),
            min_gap: Duration::from_secs(120),
            classic_regressed: true,
            row_counts: true,
            row_id: Some(0),
            baseline_row: Some(0),
            confirmed_end: false,
            inherited_outcome: MatchOutcome::Unknown,
            frame_outcome: MatchOutcome::Defeat,
            session_hero: None,
            hint: Some(MatchOutcome::Defeat),
            pending_boundary: false,
            awaiting_first_board: false,
            baseline_age: None,
            progressed_boards: 0,
            session_map: None,
            incoming_map: None,
            session_map_source: None,
        });
        assert!(
            plan.split,
            "a hint-only session keeps the mid-match gap split"
        );
        assert_eq!(
            plan.stored_outcome,
            MatchOutcome::Defeat,
            "a gap split keeps this frame's header, not an inherited outcome"
        );
        assert_eq!(plan.seal, Some(MatchOutcome::Defeat));
    }

    fn effect_tag(effect: &Effect) -> String {
        match effect {
            Effect::Ignore => "ignore".into(),
            Effect::Append => "append".into(),
            Effect::Seal { outcome } => format!("seal:{outcome}"),
            Effect::RememberHint { outcome } => format!("hint:{outcome}"),
            Effect::ClearHintAndAppend => "clear".into(),
            Effect::Defer => "defer".into(),
            Effect::ArmPending => "arm".into(),
            Effect::PrimeReset => "prime".into(),
            Effect::CountProgress => "progress".into(),
            Effect::Split {
                new_outcome, seal, ..
            } => format!(
                "split:{new_outcome}:{}",
                seal.map(|outcome| outcome.to_string())
                    .unwrap_or_else(|| "-".into())
            ),
        }
    }

    fn idle_state() -> BoundaryState {
        BoundaryState::new(None)
    }

    fn live_state() -> BoundaryState {
        let mut state = BoundaryState::new(Some("Busan".into()));
        state.gate = Some(gate(counters(14, 22, 6, 2400, 9800, 400)));
        state.hero = Some("Zenyatta".into());
        state.baseline_row = Some(2);
        state
    }

    fn streak_state() -> BoundaryState {
        let mut state = live_state();
        state.result = Some(ResultMark {
            outcome: MatchOutcome::Defeat,
            confirmed: false,
            seen_at: t0(),
        });
        state
    }

    fn hint_only_state() -> BoundaryState {
        let mut state = BoundaryState::new(Some("Busan".into()));
        state.result = Some(ResultMark {
            outcome: MatchOutcome::Defeat,
            confirmed: false,
            seen_at: t0(),
        });
        state
    }

    fn post_state() -> BoundaryState {
        let mut state = live_state();
        state.outcome = MatchOutcome::Defeat;
        state.outcome_at = Some(t0());
        state
    }

    fn starting_state() -> BoundaryState {
        let mut state = BoundaryState::new(None);
        state.awaiting_first_board = true;
        state
    }

    fn expect_transition(label: &str, state: &BoundaryState, obs: &Obs<'_>, tag: &str) {
        let got = effect_tag(&transition(state, obs).effect);
        assert_eq!(got, tag, "{label}");
    }

    fn board(fresh_reset: bool, progressed: bool, frame: MatchOutcome) -> BoardObs {
        BoardObs {
            fresh_reset,
            progressed,
            implausible: false,
            frame_outcome: frame,
        }
    }

    #[test]
    fn transition_table() {
        let hero = StartScreen::HeroSelect;
        let ban = StartScreen::HeroBan;
        let vote = StartScreen::MapVote {
            candidates: vec!["Junkertown".into(), "Ilios".into()],
        };
        let word = MatchOutcome::Defeat;
        let stay = board(false, false, MatchOutcome::Unknown);
        let progressed = board(false, true, MatchOutcome::Unknown);
        let header = board(false, true, MatchOutcome::Victory);
        let reset = board(true, false, MatchOutcome::Victory);
        let implausible = BoardObs {
            fresh_reset: false,
            progressed: false,
            implausible: true,
            frame_outcome: MatchOutcome::Unknown,
        };
        let gap = Obs::Gap {
            frame_outcome: MatchOutcome::Defeat,
        };
        let end = Obs::EndScreenDifferentMap {
            outcome: word,
            map: "Junkertown",
        };
        let confirmed = Obs::ConfirmedWord { outcome: word };
        let unconfirmed = Obs::UnconfirmedWord {
            outcome: MatchOutcome::Victory,
            relation: MapRelation::Absent,
        };
        let hero_select = Obs::StartScreen {
            screen: &hero,
            block_map_vote: false,
        };
        let hero_ban = Obs::StartScreen {
            screen: &ban,
            block_map_vote: false,
        };
        let map_vote = Obs::StartScreen {
            screen: &vote,
            block_map_vote: false,
        };
        let map_blocked = Obs::StartScreen {
            screen: &vote,
            block_map_vote: true,
        };

        // Idle and LiveMatch share the poll rows. LiveMatch is the one with
        // a mature board, so only that state defers a fresh-match read.
        let cases: &[(&str, BoundaryState, &Obs<'_>, &str)] = &[
            ("idle confirmed", idle_state(), &confirmed, "seal:defeat"),
            ("live confirmed", live_state(), &confirmed, "seal:defeat"),
            (
                "idle unconfirmed",
                idle_state(),
                &unconfirmed,
                "hint:victory",
            ),
            (
                "live unconfirmed",
                live_state(),
                &unconfirmed,
                "hint:victory",
            ),
            ("idle hero select", idle_state(), &hero_select, "prime"),
            ("live hero select", live_state(), &hero_select, "prime"),
            ("idle hero ban", idle_state(), &hero_ban, "split:unknown:-"),
            ("live hero ban", live_state(), &hero_ban, "split:unknown:-"),
            ("idle map vote", idle_state(), &map_vote, "split:unknown:-"),
            ("live map vote", live_state(), &map_vote, "split:unknown:-"),
            (
                "live map vote blocked",
                live_state(),
                &map_blocked,
                "ignore",
            ),
            ("idle board", idle_state(), &Obs::Board(stay), "append"),
            ("live board", live_state(), &Obs::Board(stay), "append"),
            (
                "live progressed",
                live_state(),
                &Obs::Board(progressed),
                "append",
            ),
            (
                "live implausible still stored",
                live_state(),
                &Obs::Board(implausible),
                "append",
            ),
            (
                "live fresh reset",
                live_state(),
                &Obs::Board(reset),
                "defer",
            ),
            ("live gap", live_state(), &gap, "split:defeat:-"),
            ("live end screen", live_state(), &end, "seal:defeat"),
            (
                "streak confirmed",
                streak_state(),
                &confirmed,
                "seal:defeat",
            ),
            (
                "streak unconfirmed",
                streak_state(),
                &unconfirmed,
                "hint:victory",
            ),
            (
                "streak unarmed different map",
                streak_state(),
                &Obs::UnconfirmedWord {
                    outcome: MatchOutcome::Victory,
                    relation: MapRelation::Differs,
                },
                "ignore",
            ),
            (
                "streak start after a board",
                streak_state(),
                &hero_select,
                "arm",
            ),
            (
                "hint with no board then start",
                hint_only_state(),
                &hero_select,
                "split:unknown:defeat",
            ),
            (
                "streak first progressed board keeps the hint",
                streak_state(),
                &Obs::Board(progressed),
                "progress",
            ),
            (
                "streak same totals",
                streak_state(),
                &Obs::Board(stay),
                "append",
            ),
            (
                "streak header does not clear",
                streak_state(),
                &Obs::Board(header),
                "append",
            ),
            (
                "streak implausible stays stored",
                streak_state(),
                &Obs::Board(implausible),
                "append",
            ),
            (
                "streak fresh reset",
                streak_state(),
                &Obs::Board(reset),
                "defer",
            ),
            (
                "streak gap seals the hint",
                streak_state(),
                &gap,
                "split:defeat:defeat",
            ),
            ("streak end screen", streak_state(), &end, "seal:defeat"),
            ("post confirmed", post_state(), &confirmed, "ignore"),
            ("post unconfirmed", post_state(), &unconfirmed, "ignore"),
            ("post start", post_state(), &hero_select, "split:unknown:-"),
            ("post board", post_state(), &Obs::Board(stay), "append"),
            (
                "post implausible still stored",
                post_state(),
                &Obs::Board(implausible),
                "append",
            ),
            (
                "post fresh reset",
                post_state(),
                &Obs::Board(reset),
                "defer",
            ),
            ("post gap stays", post_state(), &gap, "ignore"),
            ("post end screen", post_state(), &end, "ignore"),
            (
                "starting confirmed",
                starting_state(),
                &confirmed,
                "seal:defeat",
            ),
            (
                "starting unconfirmed",
                starting_state(),
                &unconfirmed,
                "hint:victory",
            ),
            (
                "starting start",
                starting_state(),
                &map_vote,
                "split:unknown:-",
            ),
            (
                "starting board",
                starting_state(),
                &Obs::Board(stay),
                "append",
            ),
            ("starting gap", starting_state(), &gap, "ignore"),
            (
                "streak vote after a board seals the hint",
                streak_state(),
                &map_vote,
                "split:unknown:defeat",
            ),
            (
                "streak ban after a board seals the hint",
                streak_state(),
                &hero_ban,
                "split:unknown:defeat",
            ),
            (
                "streak vote blocked",
                streak_state(),
                &map_blocked,
                "ignore",
            ),
            ("post vote blocked", post_state(), &map_blocked, "ignore"),
            (
                "starting ban",
                starting_state(),
                &hero_ban,
                "split:unknown:-",
            ),
            ("idle gap", idle_state(), &gap, "ignore"),
            ("idle end screen", idle_state(), &end, "ignore"),
            (
                "starting hero select",
                starting_state(),
                &hero_select,
                "ignore",
            ),
            (
                "starting start still inside the debounce",
                starting_state(),
                &map_blocked,
                "ignore",
            ),
        ];
        for (label, state, obs, tag) in cases {
            expect_transition(label, state, obs, tag);
        }

        let mut second = post_state();
        second.reset_streak = 1;
        expect_transition(
            "post second fresh-match board",
            &second,
            &Obs::Board(reset),
            "split:unknown:-",
        );
        let mut second_hint = streak_state();
        second_hint.reset_streak = 1;
        expect_transition(
            "streak second fresh-match board seals the hint",
            &second_hint,
            &Obs::Board(reset),
            "split:unknown:defeat",
        );
        expect_transition(
            "hero select after a primed streak arms and does not split",
            &second_hint,
            &hero_select,
            "arm",
        );
        let mut deferred = live_state();
        deferred.deferred = Some(counters(1, 0, 0, 10, 0, 0));
        deferred.reset_streak = 1;
        expect_transition(
            "hero select after a deferred board does not split",
            &deferred,
            &hero_select,
            "ignore",
        );
        expect_transition(
            "hero ban after a deferred board does not split",
            &deferred,
            &hero_ban,
            "ignore",
        );
        let mut armed = streak_state();
        armed.pending_boundary = true;
        expect_transition(
            "armed end screen on a different map seals the old hint",
            &armed,
            &end,
            "split:defeat:defeat",
        );
        let mut pending = streak_state();
        pending.pending_boundary = true;
        expect_transition(
            "armed boundary plus a continuation drops the hint",
            &pending,
            &Obs::Board(progressed),
            "clear",
        );
        expect_transition(
            "armed boundary plus the first reset board splits and seals",
            &pending,
            &Obs::Board(reset),
            "split:unknown:defeat",
        );
        let mut live_primed = live_state();
        live_primed.reset_streak = 1;
        expect_transition(
            "live second fresh board",
            &live_primed,
            &Obs::Board(reset),
            "split:unknown:-",
        );
        let hinted = Obs::HintedDifferentMap {
            frame_outcome: MatchOutcome::Victory,
        };
        expect_transition(
            "hinted different map seals the hint",
            &streak_state(),
            &hinted,
            "split:victory:defeat",
        );
        expect_transition(
            "armed word with no map keeps the hint",
            &pending,
            &Obs::UnconfirmedWord {
                outcome: MatchOutcome::Victory,
                relation: MapRelation::Absent,
            },
            "ignore",
        );
        expect_transition(
            "armed word on a different map keeps the hint",
            &pending,
            &Obs::UnconfirmedWord {
                outcome: MatchOutcome::Victory,
                relation: MapRelation::Differs,
            },
            "ignore",
        );
        let starting_ban_blocked = Obs::StartScreen {
            screen: &ban,
            block_map_vote: true,
        };
        expect_transition(
            "starting ban inside the debounce",
            &starting_state(),
            &starting_ban_blocked,
            "ignore",
        );
    }

    #[test]
    fn stray_word_then_clean_continuation_then_start_screen_does_not_split() {
        let now = t0();
        let mut m = Machine::new("Busan");
        m.capture_board(
            counters(8, 3, 2, 1800, 4000, 100),
            Some("Zenyatta"),
            Some(2),
            now,
        );
        m.poll(|s| {
            let mut i = poll_of(s, now + Duration::from_secs(30));
            i.signal = Some(MatchOutcome::Victory);
            i.signal_confirmed = false;
            i
        });
        m.capture_board(
            counters(10, 4, 2, 2200, 4600, 140),
            Some("Mercy"),
            Some(2),
            now + Duration::from_secs(40),
        );
        assert_eq!(
            m.active().state.result.map(|mark| mark.outcome),
            Some(MatchOutcome::Victory),
            "the first progressed board keeps the hint"
        );
        m.capture_board(
            counters(12, 5, 3, 2600, 5000, 180),
            Some("Mercy"),
            Some(2),
            now + Duration::from_secs(50),
        );
        assert!(
            m.active().state.result.is_none(),
            "the second progressed board drops the hint"
        );
        m.poll(|s| {
            let mut i = poll_of(s, now + Duration::from_secs(50));
            i.start_screen = Some(StartScreen::HeroSelect);
            i
        });
        assert!(
            m.closed.is_empty(),
            "hero swap on a climbing board is the same match"
        );
        assert!(!m.active().outcome().is_decided());
        assert_eq!(m.active().state.hero.as_deref(), Some("Mercy"));
    }

    #[test]
    fn hero_select_a_minute_later_keeps_the_hint_armed() {
        let now = t0();
        let mut m = Machine::new("Busan");
        m.capture(counters(11, 20, 5, 2100, 8000, 100), now);
        m.poll(|s| {
            let mut i = poll_of(s, now + Duration::from_secs(10));
            i.signal = Some(MatchOutcome::Defeat);
            i.signal_confirmed = false;
            i
        });
        m.poll(|s| {
            let mut i = poll_of(s, now + Duration::from_secs(90));
            i.start_screen = Some(StartScreen::HeroSelect);
            i
        });
        assert!(m.closed.is_empty());
        assert!(m.active().state.pending_boundary);
        assert_eq!(
            m.active().state.result.map(|mark| mark.outcome),
            Some(MatchOutcome::Defeat),
            "a minute later the hint is still sealable"
        );
    }

    #[test]
    fn all_increase_garbage_does_not_become_the_baseline() {
        let prev = gate(counters(18, 7, 9, 6400, 11000, 800));
        let garbage = counters(90, 40, 40, 30000, 50000, 5000);
        let ignored = plan_capture(&CapturePlanInput {
            prev_gate: Some(&prev),
            baseline: Some(&prev),
            streak: 0,
            cur: garbage,
            suspect: CLEAN,
            create_session: false,
            suppress_same_unfinished: false,
            age: Some(Duration::from_secs(40)),
            min_gap: Duration::from_secs(120),
            classic_regressed: false,
            row_counts: true,
            row_id: Some(0),
            baseline_row: Some(0),
            confirmed_end: true,
            inherited_outcome: MatchOutcome::Defeat,
            frame_outcome: MatchOutcome::Defeat,
            session_hero: None,
            hint: None,
            pending_boundary: false,
            awaiting_first_board: false,
            baseline_age: None,
            progressed_boards: 0,
            session_map: None,
            incoming_map: None,
            session_map_source: None,
        });
        assert!(!ignored.ignore_row);
        assert!(!ignored.skip_store);
        assert!(!ignored.refresh_baseline);
        assert!(!ignored.split);
        assert_eq!(ignored.reset_streak, 0);
        let follow = plan_capture(&CapturePlanInput {
            prev_gate: Some(&prev),
            baseline: Some(&prev),
            streak: 0,
            cur: counters(20, 8, 10, 7000, 12000, 900),
            suspect: CLEAN,
            create_session: false,
            suppress_same_unfinished: true,
            age: Some(Duration::from_secs(50)),
            min_gap: Duration::from_secs(120),
            classic_regressed: false,
            row_counts: true,
            row_id: Some(0),
            baseline_row: Some(0),
            confirmed_end: true,
            inherited_outcome: MatchOutcome::Defeat,
            frame_outcome: MatchOutcome::Unknown,
            session_hero: None,
            hint: None,
            pending_boundary: false,
            awaiting_first_board: false,
            baseline_age: None,
            progressed_boards: 0,
            session_map: None,
            incoming_map: None,
            session_map_source: None,
        });
        assert!(
            !follow.split,
            "a continuation above the real baseline does not split"
        );
        assert!(!follow.ignore_row);
    }

    #[test]
    fn two_to_four_x_all_increase_is_stored_and_not_the_baseline() {
        let prev = gate(counters(10, 8, 6, 2000, 3000, 500));
        let jumped = counters(24, 20, 14, 5000, 7500, 1200);
        let plan = plan_capture(&CapturePlanInput {
            prev_gate: Some(&prev),
            baseline: Some(&prev),
            streak: 0,
            cur: jumped,
            suspect: CLEAN,
            create_session: false,
            suppress_same_unfinished: true,
            age: Some(Duration::from_secs(30)),
            min_gap: Duration::from_secs(120),
            classic_regressed: false,
            row_counts: true,
            row_id: Some(0),
            baseline_row: Some(0),
            confirmed_end: false,
            inherited_outcome: MatchOutcome::Unknown,
            frame_outcome: MatchOutcome::Unknown,
            session_hero: None,
            hint: None,
            pending_boundary: false,
            awaiting_first_board: false,
            baseline_age: Some(Duration::from_secs(30)),
            progressed_boards: 0,
            session_map: None,
            incoming_map: None,
            session_map_source: None,
        });
        assert!(!plan.ignore_row && !plan.skip_store && !plan.split);
        assert!(
            !plan.refresh_baseline,
            "a 2-4x all-increase inside a minute must not replace the baseline"
        );
        assert_eq!(plan.reset_streak, 0);
        let uneven = plan_capture(&CapturePlanInput {
            prev_gate: Some(&prev),
            baseline: Some(&prev),
            streak: 0,
            cur: counters(12, 9, 7, 6000, 3400, 560),
            suspect: CLEAN,
            create_session: false,
            suppress_same_unfinished: true,
            age: Some(Duration::from_secs(90)),
            min_gap: Duration::from_secs(120),
            classic_regressed: false,
            row_counts: true,
            row_id: Some(0),
            baseline_row: Some(0),
            confirmed_end: false,
            inherited_outcome: MatchOutcome::Unknown,
            frame_outcome: MatchOutcome::Unknown,
            session_hero: None,
            hint: None,
            pending_boundary: false,
            awaiting_first_board: false,
            baseline_age: None,
            progressed_boards: 0,
            session_map: None,
            incoming_map: None,
            session_map_source: None,
        });
        assert!(
            uneven.refresh_baseline,
            "one wide-column burst still refreshes the baseline"
        );
    }

    #[test]
    fn gap_split_ignores_a_changed_row_and_keeps_the_frame_header() {
        let prev = gate(counters(29, 8, 5, 9242, 1000, 200));
        let plan = plan_capture(&CapturePlanInput {
            prev_gate: Some(&prev),
            baseline: Some(&prev),
            streak: 0,
            cur: counters(3, 1, 0, 200, 50, 10),
            suspect: CLEAN,
            create_session: false,
            suppress_same_unfinished: false,
            age: Some(Duration::from_secs(125)),
            min_gap: Duration::from_secs(120),
            classic_regressed: true,
            row_counts: true,
            row_id: Some(1),
            baseline_row: Some(0),
            confirmed_end: false,
            inherited_outcome: MatchOutcome::Unknown,
            frame_outcome: MatchOutcome::Defeat,
            session_hero: Some("Zenyatta"),
            hint: None,
            pending_boundary: false,
            awaiting_first_board: false,
            baseline_age: None,
            progressed_boards: 0,
            session_map: None,
            incoming_map: None,
            session_map_source: None,
        });
        assert!(plan.split);
        assert_eq!(plan.stored_outcome, MatchOutcome::Defeat);
    }

    #[test]
    fn confirmed_post_match_screen_does_not_split_on_the_stat_gap() {
        let prev = gate(counters(18, 7, 9, 6400, 11000, 800));
        let plan = plan_capture(&CapturePlanInput {
            prev_gate: Some(&prev),
            baseline: Some(&prev),
            streak: 0,
            cur: counters(20, 8, 10, 7000, 12000, 900),
            suspect: CLEAN,
            create_session: false,
            suppress_same_unfinished: false,
            age: Some(Duration::from_secs(200)),
            min_gap: Duration::from_secs(120),
            classic_regressed: true,
            row_counts: true,
            row_id: Some(0),
            baseline_row: Some(0),
            confirmed_end: true,
            inherited_outcome: MatchOutcome::Defeat,
            frame_outcome: MatchOutcome::Victory,
            session_hero: None,
            hint: None,
            pending_boundary: false,
            awaiting_first_board: false,
            baseline_age: None,
            progressed_boards: 0,
            session_map: None,
            incoming_map: None,
            session_map_source: None,
        });
        assert!(!plan.split);
        assert_eq!(plan.stored_outcome, MatchOutcome::Defeat);
    }

    #[test]
    fn clean_new_row_reanchors_the_baseline() {
        let prev = gate(counters(18, 7, 9, 6400, 11000, 800));
        let plan = plan_capture(&CapturePlanInput {
            prev_gate: Some(&prev),
            baseline: Some(&prev),
            streak: 0,
            cur: counters(19, 8, 9, 6600, 11200, 820),
            suspect: CLEAN,
            create_session: false,
            suppress_same_unfinished: true,
            age: Some(Duration::from_secs(15)),
            min_gap: Duration::from_secs(120),
            classic_regressed: false,
            row_counts: true,
            row_id: Some(3),
            baseline_row: Some(0),
            confirmed_end: true,
            inherited_outcome: MatchOutcome::Defeat,
            frame_outcome: MatchOutcome::Defeat,
            session_hero: Some("Zenyatta"),
            hint: None,
            pending_boundary: false,
            awaiting_first_board: false,
            baseline_age: None,
            progressed_boards: 0,
            session_map: None,
            incoming_map: None,
            session_map_source: None,
        });
        assert!(!plan.split);
        assert!(!plan.ignore_row);
        assert_eq!(plan.baseline_row, Some(3));
        assert!(plan.refresh_baseline);
    }

    #[test]
    fn deferred_board_is_held_for_the_new_session() {
        let now = t0();
        let mut m = Machine::new("Busan");
        m.capture(
            counters(18, 7, 9, 6400, 11000, 800),
            now + Duration::from_secs(60),
        );
        confirm_defeat(&mut m, now + Duration::from_secs(8 * 60));
        let first = counters(1, 3, 0, 220, 80, 400);
        m.capture(first, now + Duration::from_secs(8 * 60 + 25));
        assert!(m.closed.is_empty());
        assert_eq!(m.active().state.deferred, Some(first));
        assert!(
            m.active()
                .state
                .gate
                .is_some_and(|g| g.accepted.elims == 18)
        );
        let second = counters(2, 4, 1, 400, 120, 700);
        m.capture(second, now + Duration::from_secs(8 * 60 + 40));
        assert_eq!(m.closed.len(), 1);
        assert_eq!(m.active().opened_with, Some(first));
        assert_eq!(m.active().state.gate.map(|g| g.accepted), Some(second));
        assert_eq!(
            m.closed[0].sess.state.gate.map(|g| g.accepted.elims),
            Some(18)
        );
    }

    #[test]
    fn normal_growth_is_folded_and_refreshes_the_baseline() {
        let prev = gate(counters(2, 1, 0, 300, 40, 100));
        let plan = plan_capture(&CapturePlanInput {
            prev_gate: Some(&prev),
            baseline: Some(&prev),
            streak: 0,
            cur: counters(6, 2, 1, 1400, 180, 400),
            suspect: CLEAN,
            create_session: false,
            suppress_same_unfinished: true,
            age: Some(Duration::from_secs(135)),
            min_gap: Duration::from_secs(120),
            classic_regressed: false,
            row_counts: true,
            row_id: Some(0),
            baseline_row: Some(0),
            confirmed_end: false,
            inherited_outcome: MatchOutcome::Unknown,
            frame_outcome: MatchOutcome::Unknown,
            session_hero: None,
            hint: None,
            pending_boundary: false,
            awaiting_first_board: false,
            baseline_age: None,
            progressed_boards: 0,
            session_map: None,
            incoming_map: None,
            session_map_source: None,
        });
        assert!(!plan.split && !plan.defer && !plan.skip_store);
        assert!(plan.refresh_baseline);
    }

    #[test]
    fn hero_change_alone_does_not_split_a_live_match() {
        let now = t0();
        let mut m = Machine::new("Busan");
        m.capture_board(
            counters(8, 3, 2, 1800, 4000, 100),
            Some("Zenyatta"),
            Some(0),
            now,
        );
        m.capture_board(
            counters(10, 4, 2, 2200, 4600, 140),
            Some("Wrecking Ball"),
            Some(0),
            now + Duration::from_secs(60),
        );
        assert!(m.closed.is_empty());
        assert_eq!(m.active().state.hero.as_deref(), Some("Wrecking Ball"));
    }

    #[test]
    fn one_low_board_does_not_split_and_a_young_drop_does_not_defer() {
        let prev = gate(counters(14, 22, 6, 2400, 9800, 400));
        let low = counters(2, 1, 0, 350, 60, 800);
        let young = plan_at(
            &prev,
            low,
            false,
            0,
            false,
            20,
            true,
            true,
            MatchOutcome::Unknown,
        );
        assert!(
            !young.defer && !young.split,
            "under 45 seconds is still this match"
        );
        let armed = plan_at(
            &prev,
            low,
            false,
            0,
            false,
            50,
            true,
            true,
            MatchOutcome::Unknown,
        );
        assert!(armed.defer && !armed.split);
        assert!(armed.seal.is_none());
        let other_player = plan_at(
            &prev,
            counters(18, 9, 7, 5000, 2000, 800),
            false,
            0,
            false,
            50,
            true,
            true,
            MatchOutcome::Unknown,
        );
        assert!(
            !other_player.defer && !other_player.split,
            "another player's totals are not a fresh match"
        );
        let unidentified = plan_capture(&CapturePlanInput {
            prev_gate: Some(&prev),
            baseline: Some(&prev),
            streak: 0,
            cur: low,
            suspect: CLEAN,
            create_session: false,
            suppress_same_unfinished: true,
            age: Some(Duration::from_secs(80)),
            min_gap: Duration::from_secs(120),
            classic_regressed: false,
            row_counts: false,
            row_id: None,
            baseline_row: Some(0),
            confirmed_end: false,
            inherited_outcome: MatchOutcome::Unknown,
            frame_outcome: MatchOutcome::Unknown,
            session_hero: None,
            hint: None,
            pending_boundary: false,
            awaiting_first_board: false,
            baseline_age: None,
            progressed_boards: 0,
            session_map: None,
            incoming_map: None,
            session_map_source: None,
        });
        assert!(!unidentified.defer && !unidentified.split);
        assert!(!unidentified.refresh_baseline);
        assert!(!unidentified.skip_store);
    }

    #[test]
    fn same_stats_after_a_word_do_not_clear_the_hint() {
        let now = t0();
        let board = counters(8, 3, 2, 1800, 4000, 100);
        let mut m = Machine::new("Busan");
        m.capture(board, now);
        m.poll(|s| {
            let mut i = poll_of(s, now + Duration::from_secs(30));
            i.signal = Some(MatchOutcome::Defeat);
            i.signal_confirmed = false;
            i
        });
        m.capture(board, now + Duration::from_secs(40));
        assert_eq!(
            m.active().state.result.map(|mark| mark.outcome),
            Some(MatchOutcome::Defeat)
        );
        assert!(m.closed.is_empty());
    }

    #[test]
    fn map_vote_during_a_live_match_splits_without_sealing() {
        let now = t0();
        let mut m = Machine::new("Busan");
        m.capture(counters(8, 3, 2, 1800, 4000, 100), now);
        m.poll(|s| {
            let mut i = poll_of(s, now + Duration::from_secs(200));
            i.start_screen = Some(StartScreen::MapVote {
                candidates: vec!["Junkertown".into(), "Ilios".into()],
            });
            i
        });
        assert_eq!(m.closed.len(), 1);
        assert_eq!(m.closed[0].reason, CloseReason::MapVote);
        assert!(!m.closed[0].sess.outcome().is_decided());
        assert!(m.active().state.awaiting_first_board);
    }

    #[test]
    fn gap_after_a_hint_seals_it_on_the_old_session() {
        let prev = gate(counters(29, 8, 5, 9242, 1000, 200));
        let plan = plan_capture(&CapturePlanInput {
            prev_gate: Some(&prev),
            baseline: None,
            streak: 0,
            cur: counters(3, 1, 0, 200, 50, 10),
            suspect: CLEAN,
            create_session: false,
            suppress_same_unfinished: false,
            age: Some(Duration::from_secs(130)),
            min_gap: Duration::from_secs(120),
            classic_regressed: true,
            row_counts: true,
            row_id: Some(0),
            baseline_row: Some(0),
            confirmed_end: false,
            inherited_outcome: MatchOutcome::Unknown,
            frame_outcome: MatchOutcome::Defeat,
            session_hero: None,
            hint: Some(MatchOutcome::Defeat),
            pending_boundary: false,
            awaiting_first_board: false,
            baseline_age: None,
            progressed_boards: 0,
            session_map: None,
            incoming_map: None,
            session_map_source: None,
        });
        assert!(plan.split);
        assert_eq!(plan.seal, Some(MatchOutcome::Defeat));
        assert_eq!(plan.stored_outcome, MatchOutcome::Defeat);
        assert_eq!(plan.close_reason, Some(CloseReason::StatRegression));
    }

    #[test]
    fn post_match_adopts_an_accolade_map_without_changing_the_outcome() {
        let now = t0();
        let mut state = BoundaryState::new(None);
        state.outcome = MatchOutcome::Defeat;
        state.outcome_at = Some(now);
        let decision = decide_poll(&PollInput {
            outcome: MatchOutcome::Defeat,
            result: None,
            pending_boundary: false,
            awaiting_first_board: false,
            has_board: true,
            reset_streak: 0,
            map: None,
            map_trusted: false,
            hero: None,
            signal: Some(MatchOutcome::Victory),
            signal_confirmed: false,
            accolade_map: Some("Busan"),
            start_screen: None,
            block_map_vote: false,
            now,

            deferred: false,
        });
        let commit = commit_poll(&mut state, decision, now);
        assert!(commit.recorded_outcome.is_none());
        assert_eq!(commit.adopted_map.as_deref(), Some("Busan"));
        assert_eq!(state.outcome, MatchOutcome::Defeat);
        assert_eq!(state.map.as_deref(), Some("Busan"));
    }

    #[test]
    fn new_game_starting_exits_on_a_word_and_a_start_screen_and_ignores_a_gap() {
        let now = t0();
        let mut state = starting_state();
        let decision = decide_poll(&PollInput {
            outcome: MatchOutcome::Unknown,
            result: None,
            pending_boundary: false,
            awaiting_first_board: true,
            has_board: false,
            reset_streak: 0,
            map: None,
            map_trusted: false,
            hero: None,
            signal: Some(MatchOutcome::Defeat),
            signal_confirmed: true,
            accolade_map: None,
            start_screen: None,
            block_map_vote: false,
            now,

            deferred: false,
        });
        commit_poll(&mut state, decision, now);
        assert_eq!(state.outcome, MatchOutcome::Defeat);
        assert!(!state.awaiting_first_board);

        let screen = StartScreen::MapVote {
            candidates: vec!["Ilios".into()],
        };
        expect_transition(
            "second start screen",
            &starting_state(),
            &Obs::StartScreen {
                screen: &screen,
                block_map_vote: false,
            },
            "split:unknown:-",
        );
        expect_transition(
            "starting gap",
            &starting_state(),
            &Obs::Gap {
                frame_outcome: MatchOutcome::Victory,
            },
            "ignore",
        );
    }

    fn hinted_map_plan(source: Option<MapSource>, age_secs: u64, incoming: &str) -> CapturePlan {
        let prev = gate(counters(14, 22, 6, 2400, 9800, 400));
        plan_capture(&CapturePlanInput {
            prev_gate: Some(&prev),
            baseline: Some(&prev),
            streak: 0,
            cur: counters(16, 23, 6, 2600, 9900, 500),
            suspect: CLEAN,
            create_session: false,
            suppress_same_unfinished: false,
            age: Some(Duration::from_secs(age_secs)),
            min_gap: Duration::from_secs(120),
            classic_regressed: false,
            row_counts: true,
            row_id: Some(2),
            baseline_row: Some(2),
            confirmed_end: false,
            inherited_outcome: MatchOutcome::Unknown,
            frame_outcome: MatchOutcome::Unknown,
            session_hero: Some("Zenyatta"),
            hint: Some(MatchOutcome::Defeat),
            pending_boundary: false,
            awaiting_first_board: false,
            baseline_age: Some(Duration::from_secs(age_secs)),
            progressed_boards: 0,
            session_map: Some("Busan"),
            session_map_source: source,
            incoming_map: Some(incoming),
        })
    }

    #[test]
    fn a_trusted_map_splits_after_the_gap_and_a_text_fallback_does_not() {
        assert!(
            hinted_map_plan(Some(MapSource::TopBar), 150, "Junkertown").split,
            "a top-bar map and a later different Tab close the hinted session"
        );
        assert!(
            hinted_map_plan(Some(MapSource::Accolade), 150, "Junkertown").split,
            "an accolade map is the same kind of read"
        );
        assert!(
            !hinted_map_plan(Some(MapSource::TextFallback), 150, "Junkertown").split,
            "a full-board text fallback is not the match's map"
        );
        assert!(
            !hinted_map_plan(Some(MapSource::TopBar), 30, "Junkertown").split,
            "inside the gap a different top-bar read stays this match"
        );
        assert!(
            !hinted_map_plan(Some(MapSource::TopBar), 150, "Busan").split,
            "the same map after the gap is still this match"
        );
    }

    #[test]
    fn an_armed_unconfirmed_word_on_a_different_map_keeps_the_hint() {
        let now = t0();
        let mut state = streak_state();
        state.pending_boundary = true;
        let decision = decide_poll(&PollInput {
            outcome: MatchOutcome::Unknown,
            result: state.result,
            pending_boundary: true,
            awaiting_first_board: false,
            has_board: true,
            reset_streak: 0,
            map: Some("Busan"),
            map_trusted: true,
            hero: Some("Zenyatta"),
            signal: Some(MatchOutcome::Victory),
            signal_confirmed: false,
            accolade_map: Some("Junkertown"),
            start_screen: None,
            block_map_vote: false,
            deferred: false,
            now,
        });
        assert_eq!(decision, PollDecision::Keep);
        let confirmed = decide_poll(&PollInput {
            outcome: MatchOutcome::Unknown,
            result: state.result,
            pending_boundary: true,
            awaiting_first_board: false,
            has_board: true,
            reset_streak: 0,
            map: Some("Busan"),
            map_trusted: true,
            hero: Some("Zenyatta"),
            signal: Some(MatchOutcome::Victory),
            signal_confirmed: true,
            accolade_map: Some("Junkertown"),
            start_screen: None,
            block_map_vote: false,
            deferred: false,
            now,
        });
        match confirmed {
            PollDecision::Open(open) => {
                assert_eq!(open.seal_outcome, Some(MatchOutcome::Defeat));
                assert_eq!(open.new_outcome, MatchOutcome::Victory);
                assert_eq!(open.new_map.as_deref(), Some("Junkertown"));
            }
            other => panic!("expected the confirming read to split, got {other:?}"),
        }
    }

    fn armed_busan_word(accolade: Option<&str>, confirmed: bool) -> PollDecision {
        let now = t0();
        decide_poll(&PollInput {
            outcome: MatchOutcome::Unknown,
            result: Some(ResultMark {
                outcome: MatchOutcome::Defeat,
                confirmed: false,
                seen_at: now,
            }),
            pending_boundary: true,
            awaiting_first_board: false,
            has_board: true,
            reset_streak: 0,
            map: Some("Busan"),
            map_trusted: true,
            hero: Some("Zenyatta"),
            signal: Some(MatchOutcome::Victory),
            signal_confirmed: confirmed,
            accolade_map: accolade,
            start_screen: None,
            block_map_vote: false,
            deferred: false,
            now,
        })
    }

    #[test]
    fn an_armed_word_with_no_map_keeps_the_hint() {
        assert_eq!(
            armed_busan_word(None, false),
            PollDecision::Keep,
            "an end title before the accolade must not replace A's hint"
        );
        assert_eq!(
            armed_busan_word(Some("unknown"), false),
            PollDecision::Keep,
            "an unnamed accolade is not a different map and does not replace the hint while armed"
        );
    }

    #[test]
    fn an_unarmed_different_map_word_does_not_replace_the_hint() {
        let now = t0();
        let hint = Some(ResultMark {
            outcome: MatchOutcome::Defeat,
            confirmed: false,
            seen_at: now,
        });
        let word = |accolade: Option<&str>| {
            decide_poll(&PollInput {
                outcome: MatchOutcome::Unknown,
                result: hint,
                pending_boundary: false,
                awaiting_first_board: false,
                has_board: true,
                reset_streak: 0,
                map: Some("Busan"),
                map_trusted: true,
                hero: Some("Zenyatta"),
                signal: Some(MatchOutcome::Victory),
                signal_confirmed: false,
                accolade_map: accolade,
                start_screen: None,
                block_map_vote: false,
                deferred: false,
                now,
            })
        };
        assert_eq!(
            word(Some("Junkertown")),
            PollDecision::Keep,
            "a different map does not replace the hint, armed or not"
        );
        match word(Some("Busan")) {
            PollDecision::Update(update) => {
                assert_eq!(
                    update.result.map(|mark| mark.outcome),
                    Some(MatchOutcome::Victory),
                    "the same map still replaces the hint"
                );
            }
            other => panic!("expected the same map to replace the hint, got {other:?}"),
        }
        match word(None) {
            PollDecision::Update(update) => {
                assert_eq!(
                    update.result.map(|mark| mark.outcome),
                    Some(MatchOutcome::Victory),
                    "an unarmed word with no map still replaces the hint"
                );
            }
            other => panic!("expected a mapless word to replace the hint, got {other:?}"),
        }
    }

    #[test]
    fn an_untrusted_text_map_is_absent_on_the_poll_path() {
        let now = t0();
        let mut state = BoundaryState::new(Some("Dorado".into()));
        state.result = Some(ResultMark {
            outcome: MatchOutcome::Defeat,
            confirmed: false,
            seen_at: now,
        });
        let decision = decide_poll(&PollInput {
            outcome: MatchOutcome::Unknown,
            result: state.result,
            pending_boundary: false,
            awaiting_first_board: false,
            has_board: true,
            reset_streak: 0,
            map: Some("Dorado"),
            map_trusted: false,
            hero: None,
            signal: Some(MatchOutcome::Victory),
            signal_confirmed: true,
            accolade_map: Some("Junkertown"),
            start_screen: None,
            block_map_vote: false,
            deferred: false,
            now,
        });
        match &decision {
            PollDecision::Open(_) => {
                panic!("an untrusted map must not split on a different accolade")
            }
            PollDecision::Update(update) => {
                assert_eq!(update.record_outcome, Some(MatchOutcome::Victory));
                assert_eq!(
                    update.adopt_map.as_deref(),
                    Some("Junkertown"),
                    "an accolade replaces a text-fallback map"
                );
            }
            other => panic!("expected the word to seal and adopt the accolade, got {other:?}"),
        }
        let commit = commit_poll(&mut state, decision, now);
        assert_eq!(commit.adopted_map.as_deref(), Some("Junkertown"));
        assert_eq!(state.map.as_deref(), Some("Junkertown"));
        assert_eq!(state.outcome, MatchOutcome::Victory);
    }

    #[test]
    fn awaiting_first_board_keeps_its_first_tab() {
        let prev = gate(counters(14, 22, 6, 2400, 9800, 400));
        let plan = plan_capture(&CapturePlanInput {
            prev_gate: Some(&prev),
            baseline: Some(&prev),
            streak: 0,
            cur: counters(2, 1, 0, 350, 60, 800),
            suspect: CLEAN,
            create_session: true,
            suppress_same_unfinished: false,
            age: Some(Duration::from_secs(130)),
            min_gap: Duration::from_secs(120),
            classic_regressed: true,
            row_counts: true,
            row_id: Some(0),
            baseline_row: Some(2),
            confirmed_end: false,
            inherited_outcome: MatchOutcome::Unknown,
            frame_outcome: MatchOutcome::Unknown,
            session_hero: None,
            hint: None,
            pending_boundary: false,
            awaiting_first_board: true,
            baseline_age: Some(Duration::from_secs(130)),
            progressed_boards: 0,
            session_map: None,
            incoming_map: None,
            session_map_source: None,
        });
        assert!(
            !plan.split,
            "the first board belongs to the session the start screen opened"
        );
        assert!(plan.close_reason.is_none());
    }

    #[test]
    fn suspect_or_other_row_is_not_a_fresh_reset() {
        let prev = gate(counters(14, 22, 6, 2400, 9800, 400));
        let low = counters(2, 1, 0, 350, 60, 800);
        let mut suspect = CLEAN;
        suspect[0] = true;
        let flagged = plan_capture(&CapturePlanInput {
            prev_gate: Some(&prev),
            baseline: Some(&prev),
            streak: 0,
            cur: low,
            suspect,
            create_session: false,
            suppress_same_unfinished: true,
            age: Some(Duration::from_secs(60)),
            min_gap: Duration::from_secs(120),
            classic_regressed: false,
            row_counts: true,
            row_id: Some(2),
            baseline_row: Some(2),
            confirmed_end: false,
            inherited_outcome: MatchOutcome::Unknown,
            frame_outcome: MatchOutcome::Unknown,
            session_hero: None,
            hint: None,
            pending_boundary: false,
            awaiting_first_board: false,
            baseline_age: Some(Duration::from_secs(60)),
            progressed_boards: 0,
            session_map: None,
            incoming_map: None,
            session_map_source: None,
        });
        assert!(
            !flagged.defer && !flagged.split,
            "a suspect elim read is not a fresh reset"
        );
        let other_row = plan_capture(&CapturePlanInput {
            prev_gate: Some(&prev),
            baseline: Some(&prev),
            streak: 0,
            cur: low,
            suspect: CLEAN,
            create_session: false,
            suppress_same_unfinished: true,
            age: Some(Duration::from_secs(60)),
            min_gap: Duration::from_secs(120),
            classic_regressed: false,
            row_counts: true,
            row_id: Some(0),
            baseline_row: Some(2),
            confirmed_end: false,
            inherited_outcome: MatchOutcome::Unknown,
            frame_outcome: MatchOutcome::Unknown,
            session_hero: None,
            hint: None,
            pending_boundary: false,
            awaiting_first_board: false,
            baseline_age: Some(Duration::from_secs(60)),
            progressed_boards: 0,
            session_map: None,
            incoming_map: None,
            session_map_source: None,
        });
        assert!(
            !other_row.defer && !other_row.split,
            "a different row never counts as a fresh reset"
        );
        assert!(!other_row.skip_store);
    }
}
