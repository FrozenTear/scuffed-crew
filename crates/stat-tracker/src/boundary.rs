//! New-game boundaries for the stat-tracker session machine.
//!
//! One session stays open across hero swaps. It closes once that match is
//! over and a signal that belongs to the next queue shows up.
//!
//! On 2026-10-05 the user requeued from a Busan defeat straight into a
//! Junkertown victory. The poller saw the defeat accolade (`poll_streak_defeat`,
//! leaving the game in one second) but the map vote never landed on a poll
//! tick. The Junkertown scoreboard stayed on the Busan session: stat drops
//! were held as per-cell OCR noise, and the later Victory word was the first
//! *confirmed* outcome, so it replaced the defeat.
//!
//! Rules that keep a fast requeue from doing that again:
//!
//! * A single result word is a provisional hint. It expires after
//!   [`HINT_TTL`]. A newer word replaces it. A confirmed read replaces it and
//!   the post-match grace starts at that confirmation. A hero-select,
//!   hero-ban, or map-vote screen may seal the hint only when the hint itself
//!   is from after the live scoreboard. The session that opens has no
//!   outcome, no streak, and no post-match grace.
//! * An unconfirmed word never overrides or splits a confirmed result. A
//!   clock gap while accolade, rank, or POTG screens are still up does not
//!   split the session. A later scoreboard of the same match is not a gap.
//! * A stat reset after a confirmed result splits on the second consecutive
//!   validated capture of the same identified player row that still drops
//!   below the frozen baseline. The first of those captures is not written
//!   onto the finished game. The new session's outcome is Unknown. Garbage
//!   and unidentified rows never move the baseline. A hint-only session
//!   still uses the mid-match time gap.

use std::time::{Duration, Instant};

use crate::capture_gate::{self, Counters, GATE_COLS, GateState};
use crate::detect::MatchOutcome;

/// Minimum age of the previous accepted capture before a mid-match stat
/// regression may split. A clock gap is not a result-screen boundary:
/// accolade and rank screens stay up past this and are still the same match.
pub const RESULT_GAP: Duration = Duration::from_secs(120);

/// How long an unconfirmed result word stays a hint. Same length as the
/// poller's agreeing-read window. Older than this, the word is dropped.
pub const HINT_TTL: Duration = Duration::from_secs(60);

/// A result word seen this soon after a scoreboard is still on that live
/// board. A start screen in that window does not seal the hint.
pub const LIVE_BOARD: Duration = Duration::from_secs(8);

/// End-screen evidence kept on the session until it closes.
///
/// `confirmed == false` is a provisional hint. It expires after [`HINT_TTL`].
/// A newer word replaces it. A confirmed read replaces it. A hero-select,
/// hero-ban, or map-vote screen seals it only when the hint was seen after
/// the live scoreboard ([`LIVE_BOARD`]). Idle, a later result word, and a
/// stat split do not seal it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResultMark {
    pub outcome: MatchOutcome,
    pub confirmed: bool,
    pub seen_at: Instant,
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
    pub outcome_at: Option<Instant>,
    pub result: Option<ResultMark>,
    /// Scoreboard time snapshotted when the current hint was stored.
    /// `None` when the hint arrived with no scoreboard yet.
    pub board_at_hint: Option<Instant>,
    pub map: Option<&'a str>,
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
    /// Drop a hint that expired or was cleared. Does not clear a confirmed mark.
    pub clear_hint: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PollDecision {
    Keep,
    /// A contradictory word arrived with no new-game gap. The stored
    /// outcome (or the earlier streak) stays.
    IgnoreContradictory {
        kept: MatchOutcome,
        ignored: MatchOutcome,
    },
    Update(UpdateCurrent),
    Open(OpenNew),
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
    /// row (`parse::row_counts`). Unidentified and raw-text fallbacks are
    /// false. A false row neither splits nor moves the streak or baseline.
    pub row_counts: bool,
    /// Index of that identified row. Two drops count only when this stays
    /// the same row that owns the baseline.
    pub row_id: Option<u32>,
    /// Row index that established [`Self::baseline`]. `None` until a
    /// validated counting row has been accepted.
    pub baseline_row: Option<u32>,
    /// The session outcome is already confirmed. The mid-match time-gap
    /// split stays available when this is false (no result, or a hint only).
    pub confirmed_end: bool,
    /// Outcome of the session this Tab was requested for. Never written onto
    /// a session the split opens.
    pub inherited_outcome: MatchOutcome,
    /// Result read off this frame (banner, header). A split does not store
    /// it — the header on a post-match board belongs to the session being
    /// closed.
    pub frame_outcome: MatchOutcome,
}

/// What [`plan_capture`] decided. `main` stores `stored_outcome` on the
/// session the row lands on and keeps the streak fields for the next Tab.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapturePlan {
    pub split: bool,
    /// First validated drop after a confirmed result. The caller must not
    /// write this row onto the current session. The streak and frozen
    /// baseline are what the next capture compares against.
    pub defer: bool,
    /// This row is not a validated counting row. Leave the gate, streak, and
    /// baseline alone.
    pub ignore_row: bool,
    pub reset_streak: u32,
    pub reset_baseline: Option<GateState>,
    /// Row that owns `reset_baseline` after this capture.
    pub baseline_row: Option<u32>,
    /// The accepted gate of this capture becomes the baseline (the drop did
    /// not continue).
    pub refresh_baseline: bool,
    pub stored_outcome: MatchOutcome,
}

/// Decide whether this capture closes the session, and which outcome the
/// landing session stores.
///
/// After a confirmed result a sharp drop is only half a boundary. It splits
/// when a second consecutive validated capture of the same player row is
/// also sharp against the frozen baseline. One capture does not, and that
/// first capture is deferred so it is not written onto the finished game.
/// A hint-only session still uses the mid-match time gap. A confirmed
/// result does not. `classic_regressed` is [`crate`] `stats_regressed`.
pub fn plan_capture(input: &CapturePlanInput<'_>) -> CapturePlan {
    let stored_if_stay = if input.inherited_outcome.is_decided() {
        input.inherited_outcome
    } else {
        input.frame_outcome
    };
    let hold = |streak: u32| CapturePlan {
        split: false,
        defer: false,
        ignore_row: true,
        reset_streak: streak,
        reset_baseline: input.baseline.copied(),
        baseline_row: input.baseline_row,
        refresh_baseline: false,
        stored_outcome: stored_if_stay,
    };
    if input.create_session {
        return CapturePlan {
            split: false,
            defer: false,
            ignore_row: false,
            reset_streak: 0,
            reset_baseline: None,
            baseline_row: input.row_id,
            refresh_baseline: input.row_counts,
            stored_outcome: stored_if_stay,
        };
    }
    let compare = input.baseline.or(input.prev_gate);
    let same_row = match (input.baseline_row, input.row_id) {
        (Some(baseline), Some(row)) => baseline == row,
        (None, Some(_)) => true,
        _ => false,
    };
    let validated = input.row_counts
        && input.row_id.is_some()
        && same_row
        && !compare.is_some_and(|gate| mixed_garbage(gate, input.cur, input.suspect));
    if !validated {
        return hold(input.streak);
    }

    let sharp = input.confirmed_end
        && input.after_end_screen
        && compare.is_some_and(|gate| post_result_stat_reset(gate, input.cur, input.suspect));
    let (streak, baseline, refresh) = if sharp {
        (
            input.streak.saturating_add(1),
            input.baseline.copied().or(input.prev_gate.copied()),
            false,
        )
    } else {
        (0, None, true)
    };
    let post_result_split = sharp && streak >= 2;
    // A confirmed result must not fall through to the mid-match rule: one
    // wild row plus a 120s gap used to open a session by itself. A hint-only
    // session (`confirmed_end == false`) still uses that gap.
    let classic = !input.confirmed_end
        && !input.suppress_same_unfinished
        && input.age.is_some_and(|age| age >= input.min_gap)
        && input.classic_regressed;
    let split = post_result_split || classic;
    let defer = sharp && !split;
    CapturePlan {
        split,
        defer,
        ignore_row: false,
        reset_streak: if split { 0 } else { streak },
        reset_baseline: if split { None } else { baseline },
        baseline_row: if split {
            None
        } else if refresh {
            input.row_id
        } else {
            input.baseline_row.or(input.row_id)
        },
        refresh_baseline: !split && !defer && refresh,
        // A split never keeps the header on the board being closed.
        stored_outcome: if split {
            MatchOutcome::Unknown
        } else {
            stored_if_stay
        },
    }
}

/// A row that drops some columns and explodes others is not a reset and not
/// a continuation. It must not become the baseline.
fn mixed_garbage(prev: &GateState, cur: Counters, suspect: [bool; GATE_COLS]) -> bool {
    let acc = prev.accepted.to_array();
    let now = cur.to_array();
    let mut dropped = false;
    let mut exploded = false;
    for col in 0..GATE_COLS {
        if suspect[col] {
            continue;
        }
        if acc[col] >= 4 && now[col].saturating_mul(2) < acc[col] {
            dropped = true;
        }
        if acc[col] >= 4 && now[col] > acc[col].saturating_mul(4) {
            exploded = true;
        }
    }
    dropped && exploded
}

/// Outcome, grace stamp, and streak for a session a split just opened.
///
/// The previous session's outcome, hint, grace, and the header on the board
/// being closed are not arguments. The new session starts Unknown.
pub fn fresh_split_session() -> FreshSession {
    FreshSession {
        outcome: MatchOutcome::Unknown,
        outcome_at: None,
        result: None,
    }
}

/// Fields a split is allowed to put on the new session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FreshSession {
    pub outcome: MatchOutcome,
    pub outcome_at: Option<Instant>,
    pub result: Option<ResultMark>,
}

/// Outcome to write when a session closes, if it does not already have one.
///
/// `explicit_seal` is set only by the two-signal rule (a provisional hint
/// plus a hero-select, hero-ban, or map-vote screen). There is no fallback
/// to a stored hint: an idle Tab, a stat split, and a later result word must
/// not promote one unconfirmed read into a finished match.
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

/// Boundary fields the poller and the Tab path share. `main` copies these
/// to and from the live session; tests mutate them through [`commit_poll`]
/// and [`note_accepted_capture`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoundaryState {
    pub map: Option<String>,
    pub outcome: MatchOutcome,
    pub outcome_at: Option<Instant>,
    pub result: Option<ResultMark>,
    pub intervening_since_result: bool,
    pub reset_streak: u32,
    pub reset_baseline: Option<GateState>,
    /// Player row that owns [`Self::reset_baseline`] and the accepted gate.
    pub baseline_row: Option<u32>,
    /// Last validated scoreboard. A hint seen within [`LIVE_BOARD`] of this
    /// is still on that live board.
    pub last_board_at: Option<Instant>,
    /// `last_board_at` at the moment the current hint was stored.
    pub board_at_hint: Option<Instant>,
    pub gate: Option<GateState>,
}

impl BoundaryState {
    pub fn new(map: Option<String>) -> Self {
        Self {
            map,
            outcome: MatchOutcome::Unknown,
            outcome_at: None,
            result: None,
            intervening_since_result: false,
            reset_streak: 0,
            reset_baseline: None,
            baseline_row: None,
            last_board_at: None,
            board_at_hint: None,
            gate: None,
        }
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
/// writes the store; tests drive the same function.
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
                state.board_at_hint = None;
            }
            if let Some(mark) = update.result {
                let replaced = state.result.is_none_or(|prev| {
                    prev.outcome != mark.outcome || prev.seen_at != mark.seen_at
                });
                if !mark.confirmed && replaced {
                    state.board_at_hint = state.last_board_at;
                }
                if mark.confirmed {
                    state.board_at_hint = None;
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
            if open.new_result.is_some_and(|mark| !mark.confirmed) {
                state.board_at_hint = previous.last_board_at;
            }
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

/// Fold a capture that stayed on this session (the split path replaces the
/// session instead). An ignored row changes nothing. A deferred first drop
/// updates the streak and freezes the baseline, and does not replace the gate.
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
        return;
    }
    let already = has_post_result(state.outcome, state.result, now);
    state.gate = Some(accepted);
    state.reset_streak = plan.reset_streak;
    state.reset_baseline = if plan.refresh_baseline {
        Some(accepted)
    } else {
        plan.reset_baseline
    };
    state.baseline_row = plan.baseline_row;
    state.last_board_at = Some(now);
    if already {
        state.intervening_since_result = true;
    }
    // A hint seen on the live board is cleared once the match continues.
    if let Some(mark) = state.result.filter(|mark| !mark.confirmed) {
        let mid_match = state
            .board_at_hint
            .is_some_and(|board| mark.seen_at.saturating_duration_since(board) <= LIVE_BOARD);
        if mid_match {
            state.result = None;
            state.board_at_hint = None;
        }
    }
}

pub fn has_post_result(outcome: MatchOutcome, result: Option<ResultMark>, now: Instant) -> bool {
    confirmed_outcome(outcome, result).is_some() || provisional_hint(outcome, result, now).is_some()
}

/// Stored outcome, or a hint a second agreeing read (or a banner) already confirmed.
fn confirmed_outcome(outcome: MatchOutcome, result: Option<ResultMark>) -> Option<MatchOutcome> {
    if outcome.is_decided() {
        Some(outcome)
    } else {
        result
            .filter(|mark| mark.confirmed && mark.outcome.is_decided())
            .map(|mark| mark.outcome)
    }
}

/// One unconfirmed word, still inside [`HINT_TTL`], and only when nothing
/// confirmed is on the session.
fn provisional_hint(
    outcome: MatchOutcome,
    result: Option<ResultMark>,
    now: Instant,
) -> Option<MatchOutcome> {
    if confirmed_outcome(outcome, result).is_some() {
        return None;
    }
    result
        .filter(|mark| {
            !mark.confirmed
                && mark.outcome.is_decided()
                && now.saturating_duration_since(mark.seen_at) <= HINT_TTL
        })
        .map(|mark| mark.outcome)
}

/// The hint was seen after the live scoreboard, and the start screen is
/// later than the hint.
fn hint_is_post_match(input: &PollInput<'_>) -> bool {
    let Some(mark) = input
        .result
        .filter(|mark| !mark.confirmed && mark.outcome.is_decided())
    else {
        return false;
    };
    if input.now.saturating_duration_since(mark.seen_at) > HINT_TTL || input.now <= mark.seen_at {
        return false;
    }
    match input.board_at_hint {
        None => true,
        Some(board) => mark.seen_at.saturating_duration_since(board) > LIVE_BOARD,
    }
}

fn confident_map(map: Option<&str>) -> Option<&str> {
    map.map(str::trim).filter(|s| !s.is_empty())
}

/// Accolade map when it is a different confident name from the session map.
fn adopt_map(session_map: Option<&str>, accolade: Option<&str>) -> Option<String> {
    if confident_map(session_map).is_some() {
        return None;
    }
    confident_map(accolade).map(str::to_string)
}

fn open_from_screen(screen: &StartScreen, seal: Option<MatchOutcome>) -> OpenNew {
    let (reason, candidates) = match screen {
        StartScreen::MapVote { candidates } => ("superseded by map vote", candidates.clone()),
        StartScreen::HeroSelect | StartScreen::HeroBan => {
            ("superseded by hero select/ban", Vec::new())
        }
    };
    OpenNew {
        reason,
        seal_outcome: seal,
        new_outcome: MatchOutcome::Unknown,
        new_map: None,
        candidates,
        new_result: None,
    }
}

fn same_word_update(input: &PollInput<'_>, signal: MatchOutcome) -> PollDecision {
    let seen_at = input
        .result
        .filter(|mark| mark.outcome == signal)
        .map(|mark| mark.seen_at)
        .unwrap_or(input.now);
    let result = ResultMark {
        outcome: signal,
        confirmed: input.signal_confirmed
            || input
                .result
                .is_some_and(|mark| mark.outcome == signal && mark.confirmed),
        seen_at,
    };
    let record = (input.signal_confirmed && !input.outcome.is_decided()).then_some(signal);
    let adopt = adopt_map(input.map, input.accolade_map);
    if record.is_none() && adopt.is_none() && input.result == Some(result) {
        return PollDecision::Keep;
    }
    PollDecision::Update(UpdateCurrent {
        record_outcome: record,
        adopt_map: adopt,
        result: Some(result),
        clear_hint: false,
    })
}

/// What this poll tick does to the open session.
///
/// A POTG / end-reel wake is not a boundary. Neither is the clock, and
/// neither is a later scoreboard of the same match. An unconfirmed word
/// never overrides a confirmed result.
pub fn decide_poll(input: &PollInput<'_>) -> PollDecision {
    let confirmed = confirmed_outcome(input.outcome, input.result);
    let hint = provisional_hint(input.outcome, input.result, input.now);
    let hint_expired = input.result.is_some_and(|mark| {
        !mark.confirmed && input.now.saturating_duration_since(mark.seen_at) > HINT_TTL
    });

    if confirmed.is_some()
        && let Some(screen) = input.start_screen.as_ref()
    {
        return PollDecision::Open(open_from_screen(screen, None));
    }
    if hint_is_post_match(input)
        && let Some(screen) = input.start_screen.as_ref()
    {
        return PollDecision::Open(open_from_screen(screen, hint));
    }

    if let Some(signal) = input.signal.filter(|outcome| outcome.is_decided()) {
        if let Some(prev) = confirmed {
            // An unconfirmed word, or a different confirmed word, stays on
            // this session. It does not open a second finished game.
            if !input.signal_confirmed || prev != signal {
                return PollDecision::IgnoreContradictory {
                    kept: prev,
                    ignored: signal,
                };
            }
            return same_word_update(input, signal);
        }
        // Nothing confirmed. The newest word wins, confirmed or not.
        return same_word_update(input, signal);
    }

    if hint_expired {
        return PollDecision::Update(UpdateCurrent {
            record_outcome: None,
            adopt_map: adopt_map(input.map, input.accolade_map),
            result: None,
            clear_hint: true,
        });
    }

    if let Some(map) = adopt_map(input.map, input.accolade_map) {
        return PollDecision::Update(UpdateCurrent {
            record_outcome: None,
            adopt_map: Some(map),
            result: input.result,
            clear_hint: false,
        });
    }

    PollDecision::Keep
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
            outcome_at: outcome.is_decided().then_some(now),
            result,
            board_at_hint: None,
            map: None,
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
            min_gap: RESULT_GAP,
            classic_regressed: classic,
            row_counts,
            row_id: row_counts.then_some(0),
            baseline_row: Some(0),
            confirmed_end: after_end && inherited.is_decided(),
            inherited_outcome: inherited,
            frame_outcome: MatchOutcome::Unknown,
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
                };
                self.closed.push(Closed {
                    sess: prev,
                    reason: closed.reason,
                });
            }
        }

        fn capture(&mut self, cur: Counters, _now: Instant) {
            let sess = self.active.as_mut().expect("active");
            let state = &sess.state;
            let plan = plan_capture(&CapturePlanInput {
                prev_gate: state.gate.as_ref(),
                baseline: state.reset_baseline.as_ref(),
                streak: state.reset_streak,
                cur,
                suspect: CLEAN,
                after_end_screen: has_post_result(state.outcome, state.result, _now),
                create_session: false,
                suppress_same_unfinished: true,
                age: state.gate.map(|_| Duration::from_secs(10)),
                min_gap: RESULT_GAP,
                classic_regressed: false,
                row_counts: true,
                row_id: Some(0),
                baseline_row: state.baseline_row,
                confirmed_end: state.outcome.is_decided(),
                inherited_outcome: state.outcome,
                frame_outcome: MatchOutcome::Unknown,
            });
            if plan.split {
                let fresh = fresh_split_session();
                let commit = commit_poll(
                    &mut sess.state,
                    PollDecision::Open(OpenNew {
                        reason: "superseded by stat reset",
                        seal_outcome: None,
                        new_outcome: fresh.outcome,
                        new_map: None,
                        candidates: Vec::new(),
                        new_result: fresh.result,
                    }),
                    _now,
                );
                let closed = commit.closed.expect("split closes the session");
                self.n += 1;
                let id = format!("s{}", self.n);
                let prev = Sess {
                    id: std::mem::replace(&mut sess.id, id),
                    state: closed.previous,
                };
                self.closed.push(Closed {
                    sess: prev,
                    reason: closed.reason,
                });
                sess.state.outcome_at = fresh.outcome_at;
                sess.state.gate = Some(gate(cur));
                return;
            }
            let accepted = match state.gate {
                Some(prev) => {
                    apply_gate(Some((prev, Duration::from_secs(10))), cur, CLEAN, false).state
                }
                None => gate(cur),
            };
            note_accepted_capture(&mut sess.state, &plan, accepted, _now);
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
            outcome_at: s.outcome_at,
            result: s.result,
            board_at_hint: s.board_at_hint,
            map: s.map.as_deref(),
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
            let mut i = poll_of(s, now + RESULT_GAP + Duration::from_secs(30));
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
            let mut i = poll_of(s, now + RESULT_GAP + Duration::from_secs(40));
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
        assert!(!m.active().state.intervening_since_result);
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
            min_gap: RESULT_GAP,
            classic_regressed: true,
            row_counts: true,
            row_id: Some(0),
            baseline_row: Some(0),
            confirmed_end: true,
            inherited_outcome: MatchOutcome::Defeat,
            frame_outcome: MatchOutcome::Victory,
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
            outcome_at: None,
            result: None,
            board_at_hint: None,
            map: None,
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
            min_gap: RESULT_GAP,
            classic_regressed: true,
            row_counts: true,
            row_id: Some(0),
            baseline_row: Some(0),
            confirmed_end: true,
            inherited_outcome: MatchOutcome::Defeat,
            frame_outcome: MatchOutcome::Defeat,
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
            min_gap: RESULT_GAP,
            classic_regressed: false,
            row_counts: true,
            row_id: Some(0),
            baseline_row: Some(0),
            confirmed_end: true,
            inherited_outcome: MatchOutcome::Defeat,
            frame_outcome: MatchOutcome::Unknown,
        });
        assert!(!follow.split);
        assert_eq!(follow.reset_streak, 0);
        // Two sharp drops of a different row do not complete the streak.
        let wrong = plan_capture(&CapturePlanInput {
            prev_gate: Some(&prev),
            baseline: Some(&prev),
            streak: 1,
            cur: counters(1, 3, 0, 220, 80, 400),
            suspect: CLEAN,
            after_end_screen: true,
            create_session: false,
            suppress_same_unfinished: true,
            age: Some(Duration::from_secs(10)),
            min_gap: RESULT_GAP,
            classic_regressed: true,
            row_counts: true,
            row_id: Some(4),
            baseline_row: Some(0),
            confirmed_end: true,
            inherited_outcome: MatchOutcome::Defeat,
            frame_outcome: MatchOutcome::Victory,
        });
        assert!(
            !wrong.split,
            "a different player row does not finish the reset"
        );
        assert!(wrong.ignore_row);
        assert_eq!(wrong.reset_streak, 1);
        let wrong_again = plan_capture(&CapturePlanInput {
            prev_gate: Some(&prev),
            baseline: Some(&prev),
            streak: 1,
            cur: counters(0, 1, 0, 80, 20, 100),
            suspect: CLEAN,
            after_end_screen: true,
            create_session: false,
            suppress_same_unfinished: true,
            age: Some(Duration::from_secs(12)),
            min_gap: RESULT_GAP,
            classic_regressed: true,
            row_counts: true,
            row_id: Some(4),
            baseline_row: Some(0),
            confirmed_end: true,
            inherited_outcome: MatchOutcome::Defeat,
            frame_outcome: MatchOutcome::Victory,
        });
        assert!(!wrong_again.split, "two wrong-row reads do not split");
        assert_eq!(wrong_again.reset_streak, 1);
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
            min_gap: RESULT_GAP,
            classic_regressed: true,
            row_counts: true,
            row_id: Some(0),
            baseline_row: state.baseline_row,
            confirmed_end: true,
            inherited_outcome: MatchOutcome::Defeat,
            frame_outcome: MatchOutcome::Victory,
        });
        assert!(later.defer, "the real drop after garbage only arms");
        assert!(!later.split, "one capture later is not a new session");
        assert_eq!(later.stored_outcome, MatchOutcome::Defeat);
    }

    #[test]
    fn hint_expires_and_is_cleared() {
        let now = t0();
        let mut m = Machine::new("Busan", now);
        m.poll(|s| {
            let mut i = poll_of(s, now);
            i.signal = Some(MatchOutcome::Defeat);
            i.signal_confirmed = false;
            i
        });
        assert!(m.active().state.result.is_some());
        m.poll(|s| poll_of(s, now + HINT_TTL + Duration::from_secs(1)));
        assert!(
            m.active().state.result.is_none(),
            "a hint past the confirm window is dropped"
        );
        m.poll(|s| {
            let mut i = poll_of(s, now + HINT_TTL + Duration::from_secs(2));
            i.start_screen = Some(StartScreen::HeroSelect);
            i
        });
        assert!(m.closed.is_empty(), "an expired hint does not seal");
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
    fn mid_match_stray_word_plus_start_screen_does_not_split() {
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
        assert!(
            m.closed.is_empty(),
            "a stray word on the live scoreboard plus a start screen does not split"
        );
        assert!(!m.active().outcome().is_decided());
        assert_eq!(m.active().id, "busan");
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
        state.board_at_hint = Some(now - Duration::from_secs(400));
        let decision = decide_poll(&PollInput {
            outcome: MatchOutcome::Unknown,
            outcome_at: None,
            result: state.result,
            board_at_hint: state.board_at_hint,
            map: Some("Busan"),
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
            age: Some(RESULT_GAP + Duration::from_secs(5)),
            min_gap: RESULT_GAP,
            classic_regressed: true,
            row_counts: true,
            row_id: Some(0),
            baseline_row: Some(0),
            confirmed_end: false,
            inherited_outcome: MatchOutcome::Unknown,
            frame_outcome: MatchOutcome::Defeat,
        });
        assert!(
            plan.split,
            "a hint-only session keeps the mid-match gap split"
        );
        assert_eq!(plan.stored_outcome, MatchOutcome::Unknown);
        assert!(fresh_split_session().outcome_at.is_none());
    }
}
