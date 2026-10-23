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
//!
//! ## Layout
//!
//! - `types`: the inputs, observations, effects and decisions.
//! - `board`: board-shape rules (resets, growth, maturity).
//! - `machine`: [`BoundaryState`], [`phase_of`] and [`transition`].
//! - `plan`: the Tab path ([`plan_capture`], [`plan_capture_held`]).
//! - `poll`: the poll path ([`decide_poll`]).
//! - `commit`: [`commit_poll`] and [`note_accepted_capture`].
//! - `finished_tab`: the rc.9 split guard ([`finished_game_tab`]).
//! - `log`: the end-screen split log line.
//!
//! Every public item is re-exported here, so `boundary::X` paths are
//! unchanged.

mod board;
mod commit;
mod finished_tab;
mod log;
mod machine;
mod plan;
mod poll;
mod types;

pub use board::{RESET_MIN_AGE, post_result_stat_reset};
pub use commit::{commit_poll, has_post_result, note_accepted_capture};
pub use finished_tab::{FinishedGameTab, finished_game_tab};
pub use log::{end_screen_log_fields, format_end_screen_split};
pub use machine::{BoundaryState, outcome_sealed_on_close, phase_of, transition};
pub use plan::{plan_capture, plan_capture_held};
pub use poll::decide_poll;
pub use types::{
    BoardDoubt, BoardObs, CapturePlan, CapturePlanInput, CloseReason, CloseRecord, Effect,
    MapRelation, MapSource, Obs, OpenNew, Phase, PollCommit, PollDecision, PollInput, ResultMark,
    StartScreen, Transition, UpdateCurrent,
};

// Names the test files saw through `use super::*` before the split: the
// old file's imports and two private board rules.
#[cfg(test)]
use crate::capture_gate::{Counters, GATE_COLS, GateState};
#[cfg(test)]
use crate::model::MatchOutcome;
#[cfg(test)]
use board::{counters_progressed, known_cols};
#[cfg(test)]
use std::time::{Duration, Instant};

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
