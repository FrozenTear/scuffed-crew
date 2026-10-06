//! New-game boundaries for the stat-tracker session machine.
//!
//! One session stays open across hero swaps. It closes when the match is over
//! and the next queue shows up. The 2026-10-05 requeue is the shape this
//! machine is for: Busan, Zenyatta, defeat, then Junkertown, Wrecking Ball,
//! victory, with no map vote in between. The Junkertown stats (last board
//! 23/5/9, 6792/1463/3006) must not land on the Busan session.
//!
//! Mid-match versus post-match is board order. A hint stays sealable until a
//! clean live board is accepted after it. Wall-clock time is used only for
//! the caller's 120s stat gap and for the post-match grace stamped when a
//! result is recorded.
//!
//! # States
//!
//! | State | Meaning |
//! |---|---|
//! | Idle | A session with no live board, no hint, and no confirmed result. |
//! | LiveMatch | A clean board has been accepted and no sealable hint is open. |
//! | PostResultStreak | A result word is remembered and no clean board has arrived after it. |
//! | PostMatch | The result is confirmed. Grace starts at this confirmation. |
//! | NewGameStarting | A start screen opened the next session and its first board has not arrived. |
//!
//! # Transitions
//!
//! `Seal` records a result on this session. `Split` closes it and opens the
//! next one. `Append` keeps the board on this session. `Ignore` changes nothing.
//! `Defer` holds a post-result reset board off the finished game until the
//! next reset board commits the split. Row identity and garbage are consulted
//! only inside PostResultStreak and PostMatch reset detection.
//!
//! | State | Input | Effect |
//! |---|---|---|
//! | Idle | confirmed word | Seal. Grace starts now. |
//! | Idle | unconfirmed word | Remember the hint. Enter PostResultStreak. |
//! | Idle | start screen | Ignore. |
//! | Idle | clean board, same or new row | Append. Enter LiveMatch. The row is the baseline. |
//! | Idle | garbage board | Ignore. It is not a baseline. |
//! | Idle | stat reset | Append. There is no prior board to reset from. |
//! | Idle | 120s gap | Split. The new session keeps this frame's header result. |
//! | Idle | end screen, different map | Seal, and adopt the map when this session has none. |
//! | Idle | idle close | Ignore. |
//! | LiveMatch | confirmed word | Seal. Do not split. |
//! | LiveMatch | unconfirmed word | Remember the hint. Enter PostResultStreak. |
//! | LiveMatch | start screen | Ignore. A hero swap without a sealable hint is not a new game. |
//! | LiveMatch | clean board, same or new row | Append. A new row re-anchors the baseline. |
//! | LiveMatch | garbage board | Ignore. The baseline stays. |
//! | LiveMatch | stat reset | Ignore the split. One mid-match drop is not a new game. |
//! | LiveMatch | 120s gap | Split, even if the row id changed. Keep this frame's header. |
//! | LiveMatch | end screen, different map | Seal the word. Keep the session map. |
//! | LiveMatch | idle close | Ignore. |
//! | PostResultStreak | confirmed word | Seal that word, same or different. Do not split. Grace starts now. |
//! | PostResultStreak | unconfirmed word | Newest word replaces the hint. Do not split. |
//! | PostResultStreak | start screen | Seal the hint and split. No time limit. |
//! | PostResultStreak | clean continuation | Clear the hint and append. A board after the word means the match continued. |
//! | PostResultStreak | garbage board | Ignore. The hint stays sealable. |
//! | PostResultStreak | clean new row, not a reset | Clear the hint, append, re-anchor. |
//! | PostResultStreak | stat reset, same hero | Defer the first board. Split on the second, even if the row changed. New outcome is Unknown. |
//! | PostResultStreak | stat reset, hero changed | Split now. The new session gets this board. |
//! | PostResultStreak | 120s gap | Split. Keep this frame's header. The row check does not apply. |
//! | PostResultStreak | end screen, different map | Seal the word. Do not split on the map. |
//! | PostResultStreak | idle close | Ignore. An idle close does not seal. |
//! | PostMatch | confirmed word | Ignore. It does not override the result or restart grace. |
//! | PostMatch | unconfirmed word | Ignore. |
//! | PostMatch | start screen | Split. The new session is Unknown and has no grace. |
//! | PostMatch | clean continuation | Append. This is the post-match board of the same game. |
//! | PostMatch | garbage board | Ignore. Not stored on the finished game and not the baseline. |
//! | PostMatch | clean new row, not a reset | Append and re-anchor. |
//! | PostMatch | stat reset, same hero | Defer, then split. The deferred board is stored on the new session. |
//! | PostMatch | stat reset, hero changed | Split now. Old session keeps its hero and stats. |
//! | PostMatch | 120s gap | Ignore. A long post-match screen stays on this game. |
//! | PostMatch | end screen, different map | Ignore. |
//! | PostMatch | idle close | Ignore. The result is already confirmed. |
//! | NewGameStarting | confirmed or unconfirmed word | Ignore. A lingering end screen is the previous game. |
//! | NewGameStarting | start screen | Ignore. |
//! | NewGameStarting | clean board or stat reset | Append. Enter LiveMatch. |
//! | NewGameStarting | garbage board | Ignore. |
//! | NewGameStarting | 120s gap | Ignore. |
//! | NewGameStarting | end screen, different map | Ignore. |
//! | NewGameStarting | idle close | Ignore. |

use std::time::{Duration, Instant};

use crate::capture_gate::{self, Counters, GATE_COLS, GateState};
use crate::detect::MatchOutcome;

/// End-screen evidence kept on the session until a clean board after it, or
/// until a confirmed read replaces it.
///
/// `confirmed == false` is a hint. It stays sealable until a clean live board
/// is accepted after it. A newer unconfirmed word replaces it. Idle, a later
/// contradictory word, and a stat split do not seal it.
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
    /// A clean live board was accepted after the current hint.
    pub clean_board_after_hint: bool,
    /// The session was opened by a start screen and has no board yet.
    pub awaiting_first_board: bool,
    pub map: Option<&'a str>,
    pub hero: Option<&'a str>,
    pub signal: Option<MatchOutcome>,
    /// Banner, or the second agreeing word inside the confirm window.
    pub signal_confirmed: bool,
    pub accolade_map: Option<&'a str>,
    pub start_screen: Option<StartScreen>,
    pub now: Instant,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenNew {
    pub reason: &'static str,
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
    /// Drop a hint because a clean board arrived after it, or because the
    /// caller is replacing it. Does not clear a confirmed mark.
    pub clear_hint: bool,
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

/// What one observation does. Every poll tick and every Tab goes through
/// [`transition`]; the wrappers only pack and apply this.
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
    /// A clean board arrived after the hint. The match continued.
    ClearHintAndAppend,
    /// First post-result reset board. Not appended to the finished game.
    Defer,
    Split {
        reason: &'static str,
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

/// One scoreboard, as [`transition`] sees it. `sharp_reset` is a drop from
/// the frozen baseline. `garbage` is a mixed or all-increase read.
#[derive(Clone, Copy, Debug)]
pub struct BoardObs<'a> {
    pub clean: bool,
    pub garbage: bool,
    pub row_id: Option<u32>,
    pub sharp_reset: bool,
    pub hero: Option<&'a str>,
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
    },
    StartScreen(&'a StartScreen),
    Board(BoardObs<'a>),
    /// The caller already measured a 120s gap and a real stat regression.
    Gap {
        frame_outcome: MatchOutcome,
    },
    EndScreenDifferentMap {
        outcome: MatchOutcome,
        map: &'a str,
    },
    IdleClose,
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
    /// Frozen counters from before a pending post-result drop. Sharp reads
    /// compare to this, not to a garbage row that landed in between.
    pub baseline: Option<&'a GateState>,
    pub streak: u32,
    pub cur: Counters,
    pub suspect: [bool; GATE_COLS],
    pub after_end_screen: bool,
    pub create_session: bool,
    pub suppress_same_unfinished: bool,
    pub age: Option<Duration>,
    pub min_gap: Duration,
    pub classic_regressed: bool,
    /// True only when this capture's stats came from the identified player
    /// row (`parse::row_counts`).
    pub row_counts: bool,
    /// Index of that identified row. A slot change re-anchors; it does not
    /// freeze the gate. Passed from `player_row_idx` in `main`.
    pub row_id: Option<u32>,
    /// Row index that established the current baseline.
    pub baseline_row: Option<u32>,
    /// The session outcome is already confirmed.
    pub confirmed_end: bool,
    /// Outcome of the session this Tab was requested for. Never written onto
    /// a session a reset split opens.
    pub inherited_outcome: MatchOutcome,
    /// Result read off this frame (banner, header). A gap split keeps it.
    /// A post-result reset split does not.
    pub frame_outcome: MatchOutcome,
    /// Hero resolved for this board.
    pub hero: Option<&'a str>,
    /// Hero already stored on the session.
    pub session_hero: Option<&'a str>,
}

/// What [`plan_capture`] decided. `main` stores `stored_outcome` on the
/// session the row lands on and keeps the streak fields for the next Tab.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapturePlan {
    pub split: bool,
    /// First validated drop after a confirmed result. The caller must not
    /// write this row onto the current session. It is held and written onto
    /// the new session when the reset commits.
    pub defer: bool,
    /// Do not insert this row and do not move the gate or the baseline.
    pub ignore_row: bool,
    /// `defer` or a garbage row on a finished game. `handle_capture` returns
    /// before the store insert.
    pub skip_store: bool,
    pub clear_hint: bool,
    pub reset_streak: u32,
    pub reset_baseline: Option<GateState>,
    /// Row that owns `reset_baseline` after this capture.
    pub baseline_row: Option<u32>,
    /// The accepted gate of this capture becomes the baseline.
    pub refresh_baseline: bool,
    pub stored_outcome: MatchOutcome,
    /// Counters to hold until a reset split stores them on the new session.
    pub deferred_counters: Option<Counters>,
}

/// True when `handle_capture` must return before inserting. The deferred
/// board is held on the session and stored on the new one; a garbage row on
/// a finished game is dropped.
pub fn skip_store(plan: &CapturePlan) -> bool {
    plan.skip_store
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
    state.awaiting_first_board = input.create_session;
    if input.after_end_screen && !input.confirmed_end {
        state.result = Some(ResultMark {
            outcome: MatchOutcome::Defeat,
            confirmed: false,
            seen_at: Instant::now(),
        });
        state.clean_board_after_hint = false;
    }
    if input.confirmed_end {
        state.outcome = input.inherited_outcome;
    }

    let gap = !input.create_session
        && !input.confirmed_end
        && !input.suppress_same_unfinished
        && input.age.is_some_and(|age| age >= input.min_gap)
        && input.classic_regressed;
    if gap {
        let decided = transition(
            &state,
            &Obs::Gap {
                frame_outcome: input.frame_outcome,
            },
            Instant::now(),
        );
        if let Effect::Split { .. } = &decided.effect {
            return plan_from_effect(input, &decided.effect);
        }
    }

    let compare = input.baseline.or(input.prev_gate);
    let garbage = compare.is_some_and(|gate| row_is_garbage(gate, input.cur, input.suspect));
    let clean = input.row_counts && input.row_id.is_some() && !garbage;
    let sharp_reset = clean
        && (input.confirmed_end || input.after_end_screen)
        && compare.is_some_and(|gate| post_result_stat_reset(gate, input.cur, input.suspect));
    let decided = transition(
        &state,
        &Obs::Board(BoardObs {
            clean,
            garbage,
            row_id: input.row_id,
            sharp_reset,
            hero: input.hero,
            frame_outcome: input.frame_outcome,
        }),
        Instant::now(),
    );
    plan_from_effect(input, &decided.effect)
}

fn plan_from_effect(input: &CapturePlanInput<'_>, effect: &Effect) -> CapturePlan {
    let stored_if_stay = if input.inherited_outcome.is_decided() {
        input.inherited_outcome
    } else {
        input.frame_outcome
    };
    let hold = CapturePlan {
        split: false,
        defer: false,
        ignore_row: true,
        skip_store: input.confirmed_end,
        clear_hint: false,
        reset_streak: input.streak,
        reset_baseline: input.baseline.copied(),
        baseline_row: input.baseline_row,
        refresh_baseline: false,
        stored_outcome: stored_if_stay,
        deferred_counters: None,
    };
    match effect {
        Effect::Ignore => hold,
        Effect::Append | Effect::ClearHintAndAppend => CapturePlan {
            split: false,
            defer: false,
            ignore_row: false,
            skip_store: false,
            clear_hint: matches!(effect, Effect::ClearHintAndAppend),
            reset_streak: 0,
            reset_baseline: None,
            baseline_row: input.row_id,
            refresh_baseline: input.row_counts && !row_garbage_flag(input),
            stored_outcome: stored_if_stay,
            deferred_counters: None,
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
            stored_outcome: stored_if_stay,
            deferred_counters: Some(input.cur),
        },
        Effect::Split { new_outcome, .. } => CapturePlan {
            split: true,
            defer: false,
            ignore_row: false,
            skip_store: false,
            clear_hint: false,
            reset_streak: 0,
            reset_baseline: None,
            baseline_row: None,
            refresh_baseline: false,
            stored_outcome: *new_outcome,
            deferred_counters: None,
        },
        Effect::Seal { .. } | Effect::RememberHint { .. } => hold,
    }
}

fn row_garbage_flag(input: &CapturePlanInput<'_>) -> bool {
    input
        .baseline
        .or(input.prev_gate)
        .is_some_and(|gate| row_is_garbage(gate, input.cur, input.suspect))
}

/// A row that explodes a real column (all-increase, or a drop mixed with an
/// explosion) is not a reset and not a baseline. A pure drop is not garbage.
fn row_is_garbage(prev: &GateState, cur: Counters, suspect: [bool; GATE_COLS]) -> bool {
    let acc = prev.accepted.to_array();
    let now = cur.to_array();
    for col in 0..GATE_COLS {
        if suspect[col] {
            continue;
        }
        if acc[col] >= 4 && now[col] > acc[col].saturating_mul(4) {
            return true;
        }
    }
    false
}

/// Outcome and grace for a session a reset split just opened.
///
/// The previous session's outcome, hint, and grace are not arguments. The
/// new session starts Unknown. A gap split uses the frame header instead;
/// that value arrives as [`CapturePlan::stored_outcome`].
pub fn fresh_split_session() -> FreshSession {
    FreshSession {
        outcome: MatchOutcome::Unknown,
        outcome_at: None,
        result: None,
    }
}

/// Fields a reset split puts on the new session before the frame header of
/// a gap split is applied by the caller.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FreshSession {
    pub outcome: MatchOutcome,
    pub outcome_at: Option<Instant>,
    pub result: Option<ResultMark>,
}

/// Outcome to write when a session closes, if it does not already have one.
///
/// Only the transition's explicit seal (a hint plus a start screen) returns
/// a value. An idle close does not.
pub fn outcome_sealed_on_close(
    stored: MatchOutcome,
    explicit_seal: Option<MatchOutcome>,
) -> Option<MatchOutcome> {
    // Idle close is [`Effect::Ignore`]. The only seal is the one [`transition`]
    // already put on a start-screen split.
    if stored.is_decided()
        || matches!(
            transition(&BoundaryState::new(None), &Obs::IdleClose, Instant::now()).effect,
            Effect::Seal { .. }
        )
    {
        return None;
    }
    explicit_seal.filter(|outcome| outcome.is_decided())
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
    /// Player row that owns the baseline. A later clean row replaces it.
    pub baseline_row: Option<u32>,
    /// Last clean scoreboard. Not a clock window.
    pub last_board_at: Option<Instant>,
    /// Set when a clean continuation was accepted after the current hint.
    pub clean_board_after_hint: bool,
    /// Opened by a start screen; the first board has not been accepted.
    pub awaiting_first_board: bool,
    pub hero: Option<String>,
    /// First post-result reset board, held off the finished game.
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
            clean_board_after_hint: false,
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
    if hint && !state.clean_board_after_hint {
        return Phase::PostResultStreak;
    }
    if state.gate.is_some() || state.map.is_some() || state.hero.is_some() {
        return Phase::LiveMatch;
    }
    Phase::Idle
}

/// The one transition. Poll and capture both call this.
pub fn transition(state: &BoundaryState, obs: &Obs<'_>, _now: Instant) -> Transition {
    let phase = phase_of(state);
    let effect = match obs {
        Obs::ConfirmedWord { outcome } => confirmed_word(state, phase, *outcome),
        Obs::UnconfirmedWord { outcome } => unconfirmed_word(state, phase, *outcome),
        Obs::StartScreen(_) => start_screen(state, phase),
        Obs::Board(board) => board_effect(state, phase, board),
        Obs::Gap { frame_outcome } => gap_effect(phase, *frame_outcome),
        Obs::EndScreenDifferentMap { outcome, .. } => end_screen(state, phase, *outcome),
        Obs::IdleClose => Effect::Ignore,
    };
    Transition { effect, phase }
}

fn confirmed_word(state: &BoundaryState, phase: Phase, outcome: MatchOutcome) -> Effect {
    match phase {
        Phase::PostMatch | Phase::NewGameStarting => Effect::Ignore,
        Phase::Idle | Phase::LiveMatch | Phase::PostResultStreak => {
            if state.outcome.is_decided() {
                Effect::Ignore
            } else {
                Effect::Seal { outcome }
            }
        }
    }
}

fn unconfirmed_word(state: &BoundaryState, phase: Phase, outcome: MatchOutcome) -> Effect {
    match phase {
        Phase::PostMatch | Phase::NewGameStarting => Effect::Ignore,
        Phase::Idle | Phase::LiveMatch | Phase::PostResultStreak => {
            let confirmed_other = state
                .result
                .is_some_and(|mark| mark.confirmed && mark.outcome != outcome);
            if state.outcome.is_decided() || confirmed_other {
                Effect::Ignore
            } else {
                Effect::RememberHint { outcome }
            }
        }
    }
}

fn start_screen(state: &BoundaryState, phase: Phase) -> Effect {
    match phase {
        Phase::PostMatch => Effect::Split {
            reason: "superseded by hero select/ban",
            seal: None,
            new_outcome: MatchOutcome::Unknown,
        },
        Phase::PostResultStreak => Effect::Split {
            reason: "superseded by hero select/ban",
            seal: state
                .result
                .filter(|mark| !mark.confirmed && mark.outcome.is_decided())
                .map(|mark| mark.outcome),
            new_outcome: MatchOutcome::Unknown,
        },
        Phase::Idle | Phase::LiveMatch | Phase::NewGameStarting => Effect::Ignore,
    }
}

fn gap_effect(phase: Phase, frame_outcome: MatchOutcome) -> Effect {
    match phase {
        Phase::Idle | Phase::LiveMatch | Phase::PostResultStreak => Effect::Split {
            reason: "superseded by stat regression",
            seal: None,
            new_outcome: if frame_outcome.is_decided() {
                frame_outcome
            } else {
                MatchOutcome::Unknown
            },
        },
        Phase::PostMatch | Phase::NewGameStarting => Effect::Ignore,
    }
}

fn end_screen(state: &BoundaryState, phase: Phase, outcome: MatchOutcome) -> Effect {
    match phase {
        Phase::PostMatch | Phase::NewGameStarting => Effect::Ignore,
        Phase::Idle | Phase::LiveMatch | Phase::PostResultStreak => {
            if state.outcome.is_decided() {
                Effect::Ignore
            } else {
                Effect::Seal { outcome }
            }
        }
    }
}

fn board_effect(state: &BoundaryState, phase: Phase, board: &BoardObs<'_>) -> Effect {
    if matches!(phase, Phase::PostResultStreak | Phase::PostMatch) {
        return reset_or_continuation(state, phase, board);
    }
    if board.garbage || !board.clean {
        return Effect::Ignore;
    }
    // A sharp drop with nothing confirmed is not a post-match reset. The
    // 120s gap is a separate observation and is not blocked by this row.
    Effect::Append
}

fn reset_or_continuation(state: &BoundaryState, phase: Phase, board: &BoardObs<'_>) -> Effect {
    if board.garbage || !board.clean {
        return Effect::Ignore;
    }
    if !board.sharp_reset {
        return if phase == Phase::PostResultStreak {
            Effect::ClearHintAndAppend
        } else {
            Effect::Append
        };
    }
    if hero_changed(state.hero.as_deref(), board.hero) {
        return Effect::Split {
            reason: "superseded by stat reset",
            seal: None,
            new_outcome: MatchOutcome::Unknown,
        };
    }
    if state.reset_streak.saturating_add(1) >= 2 {
        return Effect::Split {
            reason: "superseded by stat reset",
            seal: None,
            new_outcome: MatchOutcome::Unknown,
        };
    }
    Effect::Defer
}

fn hero_changed(session_hero: Option<&str>, board_hero: Option<&str>) -> bool {
    match (session_hero, board_hero) {
        (Some(prev), Some(next)) => !prev.eq_ignore_ascii_case(next),
        _ => false,
    }
}

/// A session [`commit_poll`] closed, after the seal rule has been applied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CloseRecord {
    pub reason: &'static str,
    pub seal: Option<MatchOutcome>,
    pub previous: BoundaryState,
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
            if update.clear_hint {
                state.result = None;
                state.clean_board_after_hint = true;
            }
            if let Some(mark) = update.result {
                if !mark.confirmed {
                    state.clean_board_after_hint = false;
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
                state.clean_board_after_hint = false;
            }
            let adopted = update.adopt_map.filter(|_| state.map.is_none());
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
            let mut previous = std::mem::replace(state, BoundaryState::new(None));
            let seal = outcome_sealed_on_close(previous.outcome, open.seal_outcome);
            if let Some(outcome) = seal {
                previous.outcome = outcome;
                previous.outcome_at = Some(now);
            }
            state.outcome = open.new_outcome;
            state.outcome_at = open.new_outcome.is_decided().then_some(now);
            state.map = open.new_map;
            state.result = open.new_result;
            state.awaiting_first_board = !open.new_outcome.is_decided()
                && matches!(
                    open.reason,
                    "superseded by map vote" | "superseded by hero select/ban"
                );
            PollCommit {
                recorded_outcome: None,
                adopted_map: None,
                closed: Some(CloseRecord {
                    reason: open.reason,
                    seal,
                    previous,
                }),
            }
        }
    }
}

/// Fold a capture that stayed on this session. A deferred board is held.
/// An ignored row changes nothing. A continuation may clear a hint.
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
    state.deferred = None;
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
    if plan.clear_hint {
        state.result = None;
        state.clean_board_after_hint = true;
    }
}

pub fn has_post_result(outcome: MatchOutcome, result: Option<ResultMark>) -> bool {
    outcome.is_decided()
        || result.is_some_and(|mark| mark.confirmed && mark.outcome.is_decided())
        || result.is_some_and(|mark| !mark.confirmed && mark.outcome.is_decided())
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

fn screen_reason(screen: &StartScreen) -> (&'static str, Vec<String>) {
    match screen {
        StartScreen::MapVote { candidates } => ("superseded by map vote", candidates.clone()),
        StartScreen::HeroSelect | StartScreen::HeroBan => {
            ("superseded by hero select/ban", Vec::new())
        }
    }
}

/// What this poll tick does to the open session. The rule order is
/// [`transition`].
pub fn decide_poll(input: &PollInput<'_>) -> PollDecision {
    let mut state = BoundaryState::new(input.map.map(str::to_string));
    state.outcome = input.outcome;
    state.result = input.result;
    state.clean_board_after_hint = input.clean_board_after_hint;
    state.awaiting_first_board = input.awaiting_first_board;
    state.hero = input.hero.map(str::to_string);

    let obs = if let Some(screen) = input.start_screen.as_ref() {
        Obs::StartScreen(screen)
    } else if let Some(signal) = input.signal.filter(|outcome| outcome.is_decided()) {
        let map_differs = input.map.is_some()
            && input
                .accolade_map
                .is_some_and(|accolade| confident_map(Some(accolade)).is_some())
            && !input
                .map
                .unwrap_or("")
                .eq_ignore_ascii_case(input.accolade_map.unwrap_or(""));
        if map_differs && input.signal_confirmed {
            Obs::EndScreenDifferentMap {
                outcome: signal,
                map: input.accolade_map.unwrap_or(""),
            }
        } else if input.signal_confirmed {
            Obs::ConfirmedWord { outcome: signal }
        } else {
            Obs::UnconfirmedWord { outcome: signal }
        }
    } else {
        let adopt = adopt_map(input.map, input.accolade_map);
        return if let Some(map) = adopt {
            PollDecision::Update(UpdateCurrent {
                record_outcome: None,
                adopt_map: Some(map),
                result: input.result,
                clear_hint: false,
            })
        } else {
            PollDecision::Keep
        };
    };

    let decided = transition(&state, &obs, input.now);
    match decided.effect {
        Effect::Ignore => {
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
            adopt_map: adopt_map(input.map, input.accolade_map),
            result: Some(confirmed_mark(input, outcome)),
            clear_hint: false,
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
            if input.result == Some(result) {
                PollDecision::Keep
            } else {
                PollDecision::Update(UpdateCurrent {
                    record_outcome: None,
                    adopt_map: None,
                    result: Some(result),
                    clear_hint: false,
                })
            }
        }
        Effect::Split {
            reason,
            seal,
            new_outcome,
        } => {
            let (reason, candidates) = if let Some(screen) = input.start_screen.as_ref() {
                screen_reason(screen)
            } else {
                (reason, Vec::new())
            };
            PollDecision::Open(OpenNew {
                reason,
                seal_outcome: seal,
                new_outcome,
                new_map: None,
                candidates,
                new_result: None,
            })
        }
        Effect::Append | Effect::ClearHintAndAppend | Effect::Defer => PollDecision::Keep,
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

    fn input<'a>(outcome: MatchOutcome, result: Option<ResultMark>, now: Instant) -> PollInput<'a> {
        PollInput {
            outcome,
            result,
            clean_board_after_hint: false,
            awaiting_first_board: false,
            map: None,
            hero: None,
            signal: None,
            signal_confirmed: false,
            accolade_map: None,
            start_screen: None,
            now,
        }
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
            after_end_screen: after_end,
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
            hero: None,
            session_hero: None,
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
        reason: &'static str,
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
        fn new(map: &str, _now: Instant) -> Self {
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
            let commit = commit_poll(&mut sess.state, decision, now);
            if let Some(closed) = commit.closed {
                self.n += 1;
                let id = format!("s{}", self.n);
                let prev = Sess {
                    id: std::mem::replace(&mut sess.id, id),
                    state: closed.previous,
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
            let plan = plan_capture(&CapturePlanInput {
                prev_gate: sess.state.gate.as_ref(),
                baseline: sess.state.reset_baseline.as_ref(),
                streak: sess.state.reset_streak,
                cur,
                suspect: CLEAN,
                after_end_screen: has_post_result(sess.state.outcome, sess.state.result),
                create_session: false,
                suppress_same_unfinished: true,
                age: sess.state.gate.map(|_| age),
                min_gap: Duration::from_secs(120),
                classic_regressed: false,
                row_counts: row_id.is_some(),
                row_id,
                baseline_row: sess.state.baseline_row,
                confirmed_end: sess.state.outcome.is_decided(),
                inherited_outcome: sess.state.outcome,
                frame_outcome: MatchOutcome::Unknown,
                hero,
                session_hero: sess.state.hero.as_deref(),
            });
            if plan.split {
                let held = sess.state.deferred;
                let new_outcome = plan.stored_outcome;
                let commit = commit_poll(
                    &mut sess.state,
                    PollDecision::Open(OpenNew {
                        reason: "superseded by stat reset",
                        seal_outcome: None,
                        new_outcome,
                        new_map: None,
                        candidates: Vec::new(),
                        new_result: None,
                    }),
                    now,
                );
                let closed = commit.closed.expect("split closes the session");
                self.n += 1;
                let id = format!("s{}", self.n);
                let prev = Sess {
                    id: std::mem::replace(&mut sess.id, id),
                    state: closed.previous,
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
            if let Some(hero) = hero {
                if !plan.ignore_row {
                    sess.state.hero = Some(hero.to_string());
                }
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
            clean_board_after_hint: s.clean_board_after_hint,
            awaiting_first_board: s.awaiting_first_board,
            map: s.map.as_deref(),
            hero: s.hero.as_deref(),
            signal: None,
            signal_confirmed: false,
            accolade_map: None,
            start_screen: None,
            now,
        }
    }

    #[test]
    fn busan_defeat_then_hero_select_then_junkertown_victory() {
        let now = t0();
        let mut m = Machine::new("Busan", now);
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
        assert_eq!(m.closed[0].reason, "superseded by hero select/ban");
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
        let mut m = Machine::new("Busan", now);
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
        assert_eq!(m.closed[0].reason, "superseded by stat reset");
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
    fn defeat_streak_then_hero_select_keeps_the_loss() {
        // Live shape: one defeat word (`poll_streak_defeat`), queue pops
        // before the agreeing read, hero select with no map vote.
        let now = t0();
        let mut m = Machine::new("Busan", now);
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
        assert_eq!(m.closed.len(), 1);
        assert_eq!(m.closed[0].sess.outcome(), MatchOutcome::Defeat);
        assert_eq!(m.closed[0].sess.map(), Some("Busan"));
        assert!(!m.active().outcome().is_decided());
    }

    #[test]
    fn hero_ban_is_a_boundary_after_a_result() {
        let now = t0();
        let mut m = Machine::new("Busan", now);
        confirm_defeat(&mut m, now);
        m.poll(|s| {
            let mut i = poll_of(s, now + Duration::from_secs(15));
            i.start_screen = Some(StartScreen::HeroBan);
            i
        });
        assert_eq!(m.closed.len(), 1);
        assert_eq!(m.closed[0].reason, "superseded by hero select/ban");
        assert_eq!(m.closed[0].sess.outcome(), MatchOutcome::Defeat);
    }

    #[test]
    fn long_post_match_screen_does_not_split_on_time_alone() {
        let now = t0();
        let mut m = Machine::new("Busan", now);
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
        let mut m = Machine::new("Busan", now);
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
    fn scoreboard_between_results_is_a_gap_for_a_contradictory_word() {
        let now = t0();
        let mut m = Machine::new("Busan", now);
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
    fn accolade_map_mismatch_after_a_scoreboard_splits() {
        let now = t0();
        let mut m = Machine::new("Busan", now);
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
        let mut m = Machine::new("Busan", now);
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
        let mut m = Machine::new("Busan", now);
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
            after_end_screen: true,
            create_session: false,
            suppress_same_unfinished: true,
            age: Some(Duration::from_secs(10)),
            min_gap: Duration::from_secs(120),
            classic_regressed: true,
            row_counts: true,
            row_id: Some(0),
            baseline_row: Some(0),
            confirmed_end: true,
            inherited_outcome: MatchOutcome::Defeat,
            frame_outcome: MatchOutcome::Victory,
            hero: None,
            session_hero: None,
        });
        assert!(second.split, "the second consecutive drop splits");
        assert_eq!(
            second.stored_outcome,
            MatchOutcome::Unknown,
            "a split does not keep the header result from the board being closed"
        );
        let fresh = fresh_split_session();
        assert!(fresh.outcome_at.is_none());
        assert!(fresh.result.is_none());
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
            clean_board_after_hint: false,
            awaiting_first_board: false,
            map: None,
            hero: None,
            signal: Some(MatchOutcome::Defeat),
            signal_confirmed: true,
            accolade_map: Some("Busan"),
            start_screen: None,
            now,
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
    fn input_builder_uses_session_fields() {
        // Keeps `input()` alive so a future caller can see the default shape
        // (no signal, no wake) is Keep.
        let now = t0();
        let decision = decide_poll(&input(MatchOutcome::Unknown, None, now));
        assert_eq!(decision, PollDecision::Keep);
    }

    #[test]
    fn confirmed_victory_replaces_an_unconfirmed_defeat_hint() {
        let now = t0();
        let mut m = Machine::new("Busan", now);
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
        let mut m = Machine::new("Busan", now);
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
        assert_eq!(
            ignored.reset_streak, 1,
            "a non-counting row leaves the streak alone"
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
            after_end_screen: true,
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
            hero: None,
            session_hero: None,
        });
        assert!(mixed.ignore_row, "a mixed garbage row does not count");
        assert!(!mixed.refresh_baseline);
        assert!(!mixed.split);
        assert_eq!(mixed.reset_streak, 0);
        let follow = plan_capture(&CapturePlanInput {
            prev_gate: Some(&prev),
            baseline: Some(&prev),
            streak: 0,
            cur: counters(20, 8, 10, 7000, 12000, 900),
            suspect: CLEAN,
            after_end_screen: true,
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
            hero: None,
            session_hero: None,
        });
        assert!(!follow.split);
        assert_eq!(follow.reset_streak, 0);
        // A clean sharp row on a different slot still counts. The row id
        // re-anchors; it does not freeze the gate or swallow the gap.
        let moved = plan_capture(&CapturePlanInput {
            prev_gate: Some(&prev),
            baseline: Some(&prev),
            streak: 0,
            cur: counters(1, 3, 0, 220, 80, 400),
            suspect: CLEAN,
            after_end_screen: true,
            create_session: false,
            suppress_same_unfinished: true,
            age: Some(Duration::from_secs(10)),
            min_gap: Duration::from_secs(120),
            classic_regressed: true,
            row_counts: true,
            row_id: Some(4),
            baseline_row: Some(0),
            confirmed_end: true,
            inherited_outcome: MatchOutcome::Defeat,
            frame_outcome: MatchOutcome::Victory,
            hero: None,
            session_hero: None,
        });
        assert!(
            !moved.split,
            "the first sharp row only arms, whatever its slot"
        );
        assert!(moved.defer);
        assert!(!moved.ignore_row);
        assert_eq!(moved.reset_streak, 1);
        let moved_again = plan_capture(&CapturePlanInput {
            prev_gate: Some(&prev),
            baseline: Some(&prev),
            streak: 1,
            cur: counters(0, 1, 0, 80, 20, 100),
            suspect: CLEAN,
            after_end_screen: true,
            create_session: false,
            suppress_same_unfinished: true,
            age: Some(Duration::from_secs(12)),
            min_gap: Duration::from_secs(120),
            classic_regressed: true,
            row_counts: true,
            row_id: Some(4),
            baseline_row: Some(0),
            confirmed_end: true,
            inherited_outcome: MatchOutcome::Defeat,
            frame_outcome: MatchOutcome::Victory,
            hero: None,
            session_hero: None,
        });
        assert!(
            moved_again.split,
            "a row change across the reset still splits"
        );
        assert_eq!(moved_again.stored_outcome, MatchOutcome::Unknown);
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
            after_end_screen: true,
            create_session: false,
            suppress_same_unfinished: true,
            age: Some(Duration::from_secs(10)),
            min_gap: Duration::from_secs(120),
            classic_regressed: true,
            row_counts: true,
            row_id: Some(0),
            baseline_row: state.baseline_row,
            confirmed_end: true,
            inherited_outcome: MatchOutcome::Defeat,
            frame_outcome: MatchOutcome::Victory,
            hero: None,
            session_hero: None,
        });
        assert!(later.defer, "the real drop after garbage only arms");
        assert!(!later.split, "one capture later is not a new session");
        assert_eq!(later.stored_outcome, MatchOutcome::Defeat);
    }

    #[test]
    fn time_does_not_drop_a_sealable_hint() {
        let now = t0();
        let mut m = Machine::new("Busan", now);
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
        let mut m = Machine::new("Busan", now);
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
        let mut m = Machine::new("Busan", now);
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
        // A real end-of-match word read two seconds after the Tab, then hero
        // select. Board order seals this. There is no 8s live-board window.
        let now = t0();
        let mut m = Machine::new("Busan", now);
        m.capture(counters(8, 3, 2, 1800, 4000, 100), now);
        m.poll(|s| {
            let mut i = poll_of(s, now + Duration::from_secs(2));
            i.signal = Some(MatchOutcome::Victory);
            i.signal_confirmed = false;
            i
        });
        m.poll(|s| {
            let mut i = poll_of(s, now + Duration::from_secs(3));
            i.start_screen = Some(StartScreen::HeroSelect);
            i
        });
        assert_eq!(m.closed.len(), 1);
        assert_eq!(m.closed[0].sess.outcome(), MatchOutcome::Victory);
        assert!(!m.active().outcome().is_decided());
    }

    #[test]
    fn live_board_continuation_clears_a_hint() {
        let now = t0();
        let mut m = Machine::new("Busan", now);
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
        assert!(
            m.active().state.result.is_none(),
            "a hint on the live board is cleared when the match continues"
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
        state.clean_board_after_hint = false;
        let decision = decide_poll(&PollInput {
            outcome: MatchOutcome::Unknown,
            result: state.result,
            clean_board_after_hint: false,
            awaiting_first_board: false,
            map: Some("Busan"),
            hero: None,
            signal: Some(MatchOutcome::Defeat),
            signal_confirmed: true,
            accolade_map: None,
            start_screen: None,
            now,
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
            after_end_screen: true,
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
            hero: None,
            session_hero: None,
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
        assert!(fresh_split_session().outcome_at.is_none());
    }

    fn effect_tag(effect: &Effect) -> String {
        match effect {
            Effect::Ignore => "ignore".into(),
            Effect::Append => "append".into(),
            Effect::Seal { outcome } => format!("seal:{outcome}"),
            Effect::RememberHint { outcome } => format!("hint:{outcome}"),
            Effect::ClearHintAndAppend => "clear".into(),
            Effect::Defer => "defer".into(),
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
        state.outcome = MatchOutcome::Unknown;
        state.result = Some(ResultMark {
            outcome: MatchOutcome::Defeat,
            confirmed: false,
            seen_at: t0(),
        });
        state.clean_board_after_hint = false;
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
        let got = effect_tag(&transition(state, obs, t0()).effect);
        assert_eq!(got, tag, "{label}");
    }

    #[test]
    fn transition_table() {
        let screen = StartScreen::HeroSelect;
        let word = MatchOutcome::Defeat;
        let clean = BoardObs {
            clean: true,
            garbage: false,
            row_id: Some(0),
            sharp_reset: false,
            hero: Some("Zenyatta"),
            frame_outcome: MatchOutcome::Unknown,
        };
        let garbage = BoardObs {
            clean: false,
            garbage: true,
            row_id: Some(0),
            sharp_reset: false,
            hero: None,
            frame_outcome: MatchOutcome::Unknown,
        };
        let new_row = BoardObs {
            row_id: Some(3),
            ..clean
        };
        let reset_same = BoardObs {
            sharp_reset: true,
            row_id: Some(1),
            frame_outcome: MatchOutcome::Victory,
            ..clean
        };
        let reset_hero = BoardObs {
            sharp_reset: true,
            hero: Some("Wrecking Ball"),
            frame_outcome: MatchOutcome::Victory,
            ..clean
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
        };
        let start = Obs::StartScreen(&screen);

        let cases: &[(&str, BoundaryState, &Obs<'_>, &str)] = &[
            ("idle confirmed", idle_state(), &confirmed, "seal:defeat"),
            (
                "idle unconfirmed",
                idle_state(),
                &unconfirmed,
                "hint:victory",
            ),
            ("idle start", idle_state(), &start, "ignore"),
            ("idle clean", idle_state(), &Obs::Board(clean), "append"),
            ("idle garbage", idle_state(), &Obs::Board(garbage), "ignore"),
            (
                "idle stat reset",
                idle_state(),
                &Obs::Board(reset_same),
                "append",
            ),
            ("idle gap", idle_state(), &gap, "split:defeat:-"),
            ("idle end screen", idle_state(), &end, "seal:defeat"),
            ("idle close", idle_state(), &Obs::IdleClose, "ignore"),
            ("live confirmed", live_state(), &confirmed, "seal:defeat"),
            (
                "live unconfirmed",
                live_state(),
                &unconfirmed,
                "hint:victory",
            ),
            ("live start", live_state(), &start, "ignore"),
            ("live clean", live_state(), &Obs::Board(clean), "append"),
            ("live garbage", live_state(), &Obs::Board(garbage), "ignore"),
            (
                "live stat reset",
                live_state(),
                &Obs::Board(reset_same),
                "append",
            ),
            ("live gap", live_state(), &gap, "split:defeat:-"),
            ("live end screen", live_state(), &end, "seal:defeat"),
            ("live close", live_state(), &Obs::IdleClose, "ignore"),
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
                "streak start",
                streak_state(),
                &start,
                "split:unknown:defeat",
            ),
            ("streak clean", streak_state(), &Obs::Board(clean), "clear"),
            (
                "streak garbage",
                streak_state(),
                &Obs::Board(garbage),
                "ignore",
            ),
            (
                "streak new row",
                streak_state(),
                &Obs::Board(new_row),
                "clear",
            ),
            (
                "streak reset same hero",
                streak_state(),
                &Obs::Board(reset_same),
                "defer",
            ),
            (
                "streak reset hero changed",
                streak_state(),
                &Obs::Board(reset_hero),
                "split:unknown:-",
            ),
            ("streak gap", streak_state(), &gap, "split:defeat:-"),
            ("streak end screen", streak_state(), &end, "seal:defeat"),
            ("streak close", streak_state(), &Obs::IdleClose, "ignore"),
            ("post confirmed", post_state(), &confirmed, "ignore"),
            ("post unconfirmed", post_state(), &unconfirmed, "ignore"),
            ("post start", post_state(), &start, "split:unknown:-"),
            ("post clean", post_state(), &Obs::Board(clean), "append"),
            ("post garbage", post_state(), &Obs::Board(garbage), "ignore"),
            ("post new row", post_state(), &Obs::Board(new_row), "append"),
            (
                "post reset same hero",
                post_state(),
                &Obs::Board(reset_same),
                "defer",
            ),
            (
                "post reset hero changed",
                post_state(),
                &Obs::Board(reset_hero),
                "split:unknown:-",
            ),
            ("post gap", post_state(), &gap, "ignore"),
            ("post end screen", post_state(), &end, "ignore"),
            ("post close", post_state(), &Obs::IdleClose, "ignore"),
            ("starting confirmed", starting_state(), &confirmed, "ignore"),
            (
                "starting unconfirmed",
                starting_state(),
                &unconfirmed,
                "ignore",
            ),
            ("starting start", starting_state(), &start, "ignore"),
            (
                "starting clean",
                starting_state(),
                &Obs::Board(clean),
                "append",
            ),
            (
                "starting reset",
                starting_state(),
                &Obs::Board(reset_same),
                "append",
            ),
            (
                "starting garbage",
                starting_state(),
                &Obs::Board(garbage),
                "ignore",
            ),
            ("starting gap", starting_state(), &gap, "ignore"),
            ("starting end screen", starting_state(), &end, "ignore"),
            (
                "starting close",
                starting_state(),
                &Obs::IdleClose,
                "ignore",
            ),
        ];
        for (label, state, obs, tag) in cases {
            expect_transition(label, state, obs, tag);
        }

        let mut second = post_state();
        second.reset_streak = 1;
        expect_transition(
            "post second sharp even on a new row",
            &second,
            &Obs::Board(reset_same),
            "split:unknown:-",
        );
        let mut second_hint = streak_state();
        second_hint.reset_streak = 1;
        expect_transition(
            "streak second sharp even on a new row",
            &second_hint,
            &Obs::Board(reset_same),
            "split:unknown:-",
        );
    }

    #[test]
    fn stray_word_then_clean_continuation_then_start_screen_does_not_split() {
        let now = t0();
        let mut m = Machine::new("Busan", now);
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
        assert!(
            m.active().state.result.is_none(),
            "a clean board after the word clears the hint"
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
    fn start_screen_more_than_sixty_seconds_after_the_word_still_seals() {
        let now = t0();
        let mut m = Machine::new("Busan", now);
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
        assert_eq!(m.closed.len(), 1);
        assert_eq!(m.closed[0].sess.outcome(), MatchOutcome::Defeat);
        assert_eq!(m.closed[0].reason, "superseded by hero select/ban");
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
            after_end_screen: true,
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
            hero: None,
            session_hero: None,
        });
        assert!(ignored.ignore_row);
        assert!(ignored.skip_store);
        assert!(!ignored.refresh_baseline);
        assert!(!ignored.split);
        assert_eq!(ignored.reset_streak, 0);
        let follow = plan_capture(&CapturePlanInput {
            prev_gate: Some(&prev),
            baseline: Some(&prev),
            streak: 0,
            cur: counters(20, 8, 10, 7000, 12000, 900),
            suspect: CLEAN,
            after_end_screen: true,
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
            hero: None,
            session_hero: None,
        });
        assert!(
            !follow.split,
            "a continuation above the real baseline does not split"
        );
        assert!(!follow.ignore_row);
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
            after_end_screen: true,
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
            hero: Some("Wrecking Ball"),
            session_hero: Some("Zenyatta"),
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
            after_end_screen: true,
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
            hero: None,
            session_hero: None,
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
            after_end_screen: true,
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
            hero: Some("Zenyatta"),
            session_hero: Some("Zenyatta"),
        });
        assert!(!plan.split);
        assert!(!plan.ignore_row);
        assert_eq!(plan.baseline_row, Some(3));
        assert!(plan.refresh_baseline);
    }

    #[test]
    fn deferred_board_is_held_for_the_new_session() {
        let now = t0();
        let mut m = Machine::new("Busan", now);
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
    fn a_normal_night_is_one_session_per_game() {
        let now = t0();
        let mut m = Machine::new("Busan", now);
        m.capture_board(
            counters(10, 4, 3, 2000, 800, 100),
            Some("Zenyatta"),
            Some(0),
            now + Duration::from_secs(60),
        );
        m.poll(|s| {
            let mut i = poll_of(s, now + Duration::from_secs(8 * 60));
            i.signal = Some(MatchOutcome::Victory);
            i.signal_confirmed = false;
            i
        });
        m.poll(|s| {
            let mut i = poll_of(s, now + Duration::from_secs(8 * 60 + 4));
            i.signal = Some(MatchOutcome::Victory);
            i.signal_confirmed = true;
            i.accolade_map = Some("Busan");
            i
        });
        m.poll(|s| {
            let mut i = poll_of(s, now + Duration::from_secs(8 * 60 + 20));
            i.start_screen = Some(StartScreen::MapVote {
                candidates: vec!["Junkertown".into(), "Ilios".into()],
            });
            i
        });
        m.capture_board(
            counters(4, 6, 2, 900, 3000, 200),
            Some("Ana"),
            Some(1),
            now + Duration::from_secs(12 * 60),
        );
        m.poll(|s| {
            let mut i = poll_of(s, now + Duration::from_secs(20 * 60));
            i.signal = Some(MatchOutcome::Defeat);
            i.signal_confirmed = true;
            i.accolade_map = Some("Junkertown");
            i
        });
        m.poll(|s| {
            let mut i = poll_of(s, now + Duration::from_secs(20 * 60 + 15));
            i.start_screen = Some(StartScreen::HeroSelect);
            i
        });
        m.capture_board(
            counters(6, 2, 1, 1500, 200, 4000),
            Some("Reinhardt"),
            Some(0),
            now + Duration::from_secs(24 * 60),
        );
        m.poll(|s| {
            let mut i = poll_of(s, now + Duration::from_secs(32 * 60));
            i.signal = Some(MatchOutcome::Victory);
            i.signal_confirmed = true;
            i.accolade_map = Some("Ilios");
            i
        });
        assert_eq!(m.closed.len(), 2, "one closed session per finished game");
        assert_eq!(m.closed[0].sess.outcome(), MatchOutcome::Victory);
        assert_eq!(m.closed[0].sess.map(), Some("Busan"));
        assert_eq!(m.closed[1].sess.outcome(), MatchOutcome::Defeat);
        assert_eq!(m.closed[1].sess.map(), Some("Junkertown"));
        assert_eq!(m.active().outcome(), MatchOutcome::Victory);
        assert_eq!(m.active().map(), Some("Ilios"));
    }

    #[test]
    fn busan_zenyatta_defeat_does_not_take_junkertown_wrecking_ball_stats() {
        // Server row a4b28494e7aae583 is labeled Busan but carries Junkertown's
        // Wrecking Ball victory. The rows are all role Tank; the split is the
        // hero change plus the stat reset, with no map vote.
        let now = t0();
        let busan = counters(14, 22, 6, 2400, 9800, 400);
        let reset = counters(2, 1, 0, 400, 80, 900);
        let junkertown = counters(23, 9, 5, 6792, 1463, 3006);
        let prev = gate(busan);
        let same_hero = plan_capture(&CapturePlanInput {
            prev_gate: Some(&prev),
            baseline: Some(&prev),
            streak: 0,
            cur: reset,
            suspect: CLEAN,
            after_end_screen: true,
            create_session: false,
            suppress_same_unfinished: true,
            age: Some(Duration::from_secs(20)),
            min_gap: Duration::from_secs(120),
            classic_regressed: true,
            row_counts: true,
            row_id: Some(0),
            baseline_row: Some(2),
            confirmed_end: true,
            inherited_outcome: MatchOutcome::Defeat,
            frame_outcome: MatchOutcome::Victory,
            hero: Some("Zenyatta"),
            session_hero: Some("Zenyatta"),
        });
        assert!(
            same_hero.defer && !same_hero.split,
            "the same hero only arms on the first reset board"
        );
        let changed = plan_capture(&CapturePlanInput {
            hero: Some("Wrecking Ball"),
            session_hero: Some("Zenyatta"),
            ..CapturePlanInput {
                prev_gate: Some(&prev),
                baseline: Some(&prev),
                streak: 0,
                cur: reset,
                suspect: CLEAN,
                after_end_screen: true,
                create_session: false,
                suppress_same_unfinished: true,
                age: Some(Duration::from_secs(20)),
                min_gap: Duration::from_secs(120),
                classic_regressed: true,
                row_counts: true,
                row_id: Some(0),
                baseline_row: Some(2),
                confirmed_end: true,
                inherited_outcome: MatchOutcome::Defeat,
                frame_outcome: MatchOutcome::Victory,
                hero: Some("Wrecking Ball"),
                session_hero: Some("Zenyatta"),
            }
        });
        assert!(
            changed.split,
            "Zenyatta to Wrecking Ball plus a stat reset splits with no map vote"
        );
        assert_eq!(changed.stored_outcome, MatchOutcome::Unknown);

        let mut m = Machine::new("Busan", now);
        m.capture_board(
            busan,
            Some("Zenyatta"),
            Some(2),
            now + Duration::from_secs(60),
        );
        confirm_defeat(&mut m, now + Duration::from_secs(8 * 60));
        m.capture_board(
            reset,
            Some("Wrecking Ball"),
            Some(0),
            now + Duration::from_secs(8 * 60 + 20),
        );
        assert_eq!(m.closed.len(), 1);
        assert_eq!(m.closed[0].reason, "superseded by stat reset");
        assert_eq!(m.closed[0].sess.outcome(), MatchOutcome::Defeat);
        assert_eq!(m.closed[0].sess.map(), Some("Busan"));
        assert_eq!(m.closed[0].sess.state.hero.as_deref(), Some("Zenyatta"));
        assert_eq!(
            m.closed[0].sess.state.gate.map(|g| g.accepted),
            Some(busan),
            "Busan keeps its own last board"
        );
        assert_ne!(
            m.closed[0].sess.state.gate.map(|g| g.accepted),
            Some(junkertown)
        );
        assert_eq!(m.active().opened_with, Some(reset));
        // Each column stays under a 4× jump. A bigger jump is a garbage read
        // and must not become the baseline.
        m.capture_board(
            counters(6, 3, 1, 1600, 320, 2000),
            Some("Wrecking Ball"),
            Some(0),
            now + Duration::from_secs(12 * 60),
        );
        m.capture_board(
            counters(16, 7, 4, 6400, 1280, 2800),
            Some("Wrecking Ball"),
            Some(0),
            now + Duration::from_secs(16 * 60),
        );
        m.capture_board(
            junkertown,
            Some("Wrecking Ball"),
            Some(0),
            now + Duration::from_secs(18 * 60),
        );
        m.poll(|s| {
            let mut i = poll_of(s, now + Duration::from_secs(19 * 60));
            i.signal = Some(MatchOutcome::Victory);
            i.signal_confirmed = true;
            i.accolade_map = Some("Junkertown");
            i
        });
        assert_eq!(m.closed.len(), 1);
        assert_eq!(m.active().outcome(), MatchOutcome::Victory);
        assert_eq!(m.active().map(), Some("Junkertown"));
        assert_eq!(m.active().state.hero.as_deref(), Some("Wrecking Ball"));
        let last = m.active().state.gate.expect("junkertown board").accepted;
        assert_eq!(last.elims, 23);
        assert_eq!(last.deaths, 5);
        assert_eq!(last.assists, 9);
        assert_eq!(last.damage, 6792);
        assert_eq!(last.healing, 1463);
        assert_eq!(last.mitigation, 3006);
        assert_eq!(
            m.closed[0].sess.state.gate.map(|g| g.accepted.healing),
            Some(9800),
            "Busan never receives the Wrecking Ball healing"
        );
    }
}
