//! A Tab that shows a finished game's last board again (#809).
//!
//! Runasapi, 17 Nov: game A ended on 0/0/3/2007/772/1502. Fifty minutes
//! and two other games later, the Tab that opened game B read that same
//! board, every cell sure, on the Runasapi top bar. The tracker holds no
//! board of A by then (A's gate went with A, and nothing was carried), so
//! the numbers were on the screen that Tab grabbed: Overwatch was showing
//! A's board again. B stored it as its first row, its gate took A's
//! numbers as B's own, and B's next Tab (0/0/2/648/307/600) was held at
//! A's values by the monotonic gate.
//!
//! The first board of a game is compared with the last stored board of
//! the most recent other games. A board on that game's map, equal to it on
//! every cell both boards read for sure, is that game's board shown again,
//! not this game's: it is not stored, and it gives this game no gate, so
//! this game's own next board is its first.

use crate::*;
use stat_tracker::games::stat_reads::STATS;

/// How many of the most recent other games a first board is compared with.
pub(crate) const FINISHED_BOARDS_CHECKED: usize = 10;

/// Damage, healing and mitigation together, over the cells compared, a
/// finished board needs before an exact match counts. A board that big is
/// not reached by another game to the exact number in every column; an
/// empty or near-empty start of a game can be.
pub(crate) const FINISHED_BOARD_MIN_BIG: u32 = 1000;

/// Whether this Tab's own row is `finished`, another game's last stored
/// board. Only the cells read for sure on both boards are compared: not
/// held or suspect on this Tab, and not in `finished_unsure` (the stored
/// row's suspect fields, which may hold an older value than the board
/// showed). At least [`reader_apply::MIN_SURE_CELLS`] are compared, every
/// one is equal, and the damage, healing and mitigation among them come to
/// [`FINISHED_BOARD_MIN_BIG`] or more.
pub(crate) fn shows_finished_board(
    facts: &BoardFacts<'_>,
    finished: Counters,
    finished_unsure: [bool; capture_gate::GATE_COLS],
) -> bool {
    let cols = |c: Counters| {
        [
            c.elims,
            c.assists,
            c.deaths,
            c.damage,
            c.healing,
            c.mitigation,
        ]
    };
    let read = cols(facts.counters);
    let board = cols(finished);
    let compared: Vec<usize> = (0..capture_gate::GATE_COLS)
        .filter(|&col| !facts.held[col] && !facts.suspect[col] && !finished_unsure[col])
        .collect();
    let big = compared
        .iter()
        .filter(|&&col| col >= 3)
        .fold(0u32, |sum, &col| sum.saturating_add(board[col]));
    big >= FINISHED_BOARD_MIN_BIG
        && compared.len() >= reader_apply::MIN_SURE_CELLS
        && compared.iter().all(|&col| read[col] == board[col])
}

fn row_counters(row: &storage::PersonalMatch) -> Counters {
    Counters {
        elims: row.elims,
        assists: row.assists,
        deaths: row.deaths,
        damage: row.damage,
        healing: row.healing,
        mitigation: row.mitigation,
    }
}

/// The stat columns `row` stored as unsure (its `suspect_fields`).
fn row_unsure(row: &storage::PersonalMatch) -> [bool; capture_gate::GATE_COLS] {
    std::array::from_fn(|col| row.suspect_fields.iter().any(|f| f == STATS[col]))
}

/// Whether `row` was stored on `map`, as read or as the member set it.
fn row_on_map(row: &storage::PersonalMatch, map: &str) -> bool {
    std::iter::once(row.map_name.as_str())
        .chain(row.corrected_map_name.as_deref())
        .any(|stored| stored.eq_ignore_ascii_case(map))
}

/// The map this Tab read on its own top bar or banner; empty when none.
fn tab_map(facts: &BoardFacts<'_>) -> String {
    resolved_map(None, facts.map_from_panel, facts.parsed_map, &[]).0
}

/// The other game whose last stored board this capture shows, while the
/// game it is for has no board of its own yet (no gate for the store,
/// [`gate_prev_for_store`]). The board shown again carries its game's top
/// bar too, so the map this Tab read must be that game's; a Tab with no
/// map read compares nothing. A store error compares nothing either: the
/// board is then taken as before.
///
/// A Tab that splits off the open game carries that game's gate, so it is
/// not compared: a finished board shown again on a split is stored as the
/// split's first row, as before #809. A split already needs two of elims,
/// deaths and damage below the board it splits from.
pub(crate) async fn finished_game_shown(
    store: &storage::LocalStore,
    req: &CaptureRequest,
    facts: &BoardFacts<'_>,
) -> Option<String> {
    if gate_prev_for_store(req.prev_gate, req.awaiting_first_board).is_some() {
        return None;
    }
    let map = tab_map(facts);
    if map.is_empty() {
        return None;
    }
    let sessions = match store.recent_sessions(FINISHED_BOARDS_CHECKED + 1).await {
        Ok(sessions) => sessions,
        Err(e) => {
            tracing::warn!(target: "scuffed_stat_tracker", error = %e, "recent games unreadable; first board not compared with them");
            return None;
        }
    };
    for session in sessions
        .iter()
        .filter(|s| s.session_id != req.session_id)
        .take(FINISHED_BOARDS_CHECKED)
    {
        let rows = match store.get_session_snapshots(&session.session_id).await {
            Ok(rows) => rows,
            Err(e) => {
                tracing::warn!(
                    target: "scuffed_stat_tracker",
                    session_id = %session.session_id,
                    error = %e,
                    "recent game's rows unreadable; first board not compared with it"
                );
                continue;
            }
        };
        if rows.last().is_some_and(|last| {
            row_on_map(last, &map)
                && shows_finished_board(facts, row_counters(last), row_unsure(last))
        }) {
            return Some(session.session_id.clone());
        }
    }
    None
}

/// The report for a Tab that shows another game's last board
/// ([`finished_game_shown`]): nothing stored, no gate. `None` for a board
/// of this game.
pub(crate) async fn finished_board_report(
    ctx: &DaemonCtx,
    req: &CaptureRequest,
    facts: &BoardFacts<'_>,
    finished_game_tab: Option<boundary::FinishedGameTab>,
) -> Option<CaptureReport> {
    let shown = finished_game_shown(&ctx.store, req, facts).await?;
    tracing::warn!(
        target: "scuffed_stat_tracker",
        session_id = %req.session_id,
        finished_session = %shown,
        map = %tab_map(facts),
        board = ?facts.counters,
        held = ?facts.held,
        suspect = ?facts.suspect,
        "Tab shows a finished game's last board: not stored on this game"
    );
    Some(CaptureReport {
        finished_game_tab,
        ..blank_rejected_report(
            req.game_outcome,
            &req.session_id,
            req.hero_auth.clone(),
            req,
        )
    })
}
