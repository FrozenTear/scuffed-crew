//! Opening, reusing, retiring and closing a game session: the grace and
//! idle rules, close reasons, unrecorded-game notes and the end-screen
//! split log.

use crate::*;

/// Persist (or clear) the open-game skeleton. Fire-and-forget: losing this
/// file only degrades restart recovery, never the capture itself.
pub(crate) fn untracked_note_key(session_id: &str, reason: &str) -> String {
    format!("{session_id}\n{reason}")
}

pub(crate) fn noted_untracked()
-> &'static std::sync::Mutex<Option<std::collections::HashSet<String>>> {
    static SEEN: std::sync::Mutex<Option<std::collections::HashSet<String>>> =
        std::sync::Mutex::new(None);
    &SEEN
}

/// Process-local record of jsonl notes. The file is read once, not on every
/// Deathmatch Tab.
pub(crate) fn untracked_already_noted(
    data_dir: &std::path::Path,
    session_id: &str,
    reason: &str,
) -> bool {
    let mut guard = noted_untracked()
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    if guard.is_none() {
        let mut set = std::collections::HashSet::new();
        let path = data_dir.join("debug").join("unrecorded_games.jsonl");
        if let Ok(text) = std::fs::read_to_string(path) {
            for line in text.lines() {
                let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
                    continue;
                };
                let Some(sid) = value.get("session_id").and_then(|v| v.as_str()) else {
                    continue;
                };
                let Some(why) = value.get("reason").and_then(|v| v.as_str()) else {
                    continue;
                };
                set.insert(untracked_note_key(sid, why));
            }
        }
        *guard = Some(set);
    }
    guard
        .as_ref()
        .is_some_and(|set| set.contains(&untracked_note_key(session_id, reason)))
}

pub(crate) fn remember_untracked_note(session_id: &str, reason: &str) {
    let mut guard = noted_untracked()
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    guard
        .get_or_insert_with(std::collections::HashSet::new)
        .insert(untracked_note_key(session_id, reason));
}

pub(crate) fn append_unrecorded_row(data_dir: &std::path::Path, row: &serde_json::Value) {
    let dir = data_dir.join("debug");
    if std::fs::create_dir_all(&dir).is_ok()
        && let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join("unrecorded_games.jsonl"))
    {
        use std::io::Write;
        let _ = writeln!(file, "{row}");
    }
}

/// Deathmatch and practice are local only. Log once at info and append the
/// same jsonl the unrecorded path uses, so a later close does not warn again.
pub(crate) fn note_untracked_once(data_dir: &std::path::Path, g: &ActiveGame, reason: &str) {
    if untracked_already_noted(data_dir, &g.session_id, reason) {
        return;
    }
    tracing::info!(
        target: "scuffed_stat_tracker",
        session_id = %g.session_id,
        map = g.map.as_deref().unwrap_or("?"),
        reason,
        "untracked map: not uploaded"
    );
    let row = serde_json::json!({
        "closed_at": Utc::now().to_rfc3339(),
        "session_id": g.session_id,
        "outcome": g.outcome.to_string(),
        "map": g.map,
        "age_secs": g.opened_at.elapsed().as_secs(),
        "reason": reason,
    });
    remember_untracked_note(&g.session_id, reason);
    append_unrecorded_row(data_dir, &row);
}

pub(crate) const QUIET_CLOSE_REASON: &str = "finished game closed after quiet period";

pub(crate) const SHUTDOWN_CLOSE_REASON: &str = "daemon shutdown";

pub(crate) const UNFINISHED_MAX_REASON: &str = "unfinished game closed after several hours";

pub(crate) const EMPTY_IDLE_REASON: &str = "empty game closed after 20 minutes idle";

/// Quiet close, shutdown, and the long unfinished bound each keep their own
/// message. Every other close is a new-game boundary.
pub(crate) fn session_close_log(reason: &str) -> &'static str {
    match reason {
        QUIET_CLOSE_REASON => QUIET_CLOSE_REASON,
        SHUTDOWN_CLOSE_REASON => SHUTDOWN_CLOSE_REASON,
        UNFINISHED_MAX_REASON => UNFINISHED_MAX_REASON,
        EMPTY_IDLE_REASON => EMPTY_IDLE_REASON,
        _ => "closed session at new-game boundary",
    }
}

#[cfg(test)]
thread_local! {
    pub(crate) static LAST_END_SCREEN_SPLIT: std::cell::Cell<Option<String>> =
        const { std::cell::Cell::new(None) };
}

pub(crate) fn remember_end_screen_split(line: &str) {
    #[cfg(test)]
    LAST_END_SCREEN_SPLIT.with(|slot| slot.set(Some(line.to_string())));
    #[cfg(not(test))]
    let _ = line;
}

/// End-screen split fields for the daemon log. A missing accolade map prints
/// `none` instead of panicking. The arm flag is the session's pending boundary.
pub(crate) fn end_screen_split_log(
    pending_boundary: bool,
    open: Option<&boundary::OpenNew>,
    seal: Option<detect::MatchOutcome>,
) -> String {
    let (armed, map) = open
        .and_then(|open| boundary::end_screen_log_fields(pending_boundary, open))
        .unwrap_or_else(|| (pending_boundary, "none".to_string()));
    boundary::format_end_screen_split(armed, seal, &map)
}

/// A game that closes without a single accepted Tab capture never reached the
/// store: `session_created` stays false, the outcome and map the poller saw
/// live only in `active_game.json` and are overwritten by the next game.
/// Say so at WARN, and keep a one-line record in `debug/unrecorded_games.jsonl`
/// so the evening can be reconstructed from disk after the journal rotates.
/// A board that was just written sets `session_created`, so this does not
/// warn for that session.
pub(crate) fn note_unrecorded_game(data_dir: &std::path::Path, g: &ActiveGame, reason: &str) {
    if parse::map_is_untracked(g.map.as_deref().unwrap_or("")) {
        note_untracked_once(
            data_dir,
            g,
            parse::untracked_close_reason(g.map.as_deref().unwrap_or("")),
        );
        return;
    }
    if g.session_created {
        return;
    }
    // `reader = "new"`: boards were read, but a stat column never had a
    // sure read, so the game has no complete row and nothing was stored: a
    // placeholder is never stored or uploaded as a number (F266-1).
    let unread = g.gate.map(|gate| gate.unread_cols()).unwrap_or_default();
    // "Never read" and "read, and not taken" are told apart (R3-2): a first
    // read is not taken on a Tab where another column dropped.
    let not_taken = g.gate.map(|gate| gate.refused_cols()).unwrap_or_default();
    let never_read: Vec<&str> = unread
        .iter()
        .copied()
        .filter(|col| !not_taken.contains(col))
        .collect();
    if unread.is_empty() {
        tracing::warn!(
            target: "scuffed_stat_tracker",
            session_id = %g.session_id,
            outcome = %g.outcome,
            map = g.map.as_deref().unwrap_or("?"),
            age_secs = g.opened_at.elapsed().as_secs(),
            reason,
            "game closed with NO Tab captures: it was not recorded (no scoreboard seen: Tab not pressed, keyboard grabbed, or capture rejected)"
        );
    } else {
        tracing::warn!(
            target: "scuffed_stat_tracker",
            session_id = %g.session_id,
            outcome = %g.outcome,
            map = g.map.as_deref().unwrap_or("?"),
            age_secs = g.opened_at.elapsed().as_secs(),
            reason,
            ?never_read,
            ?not_taken,
            "game closed and NOT stored: it has no number for every stat. `never_read`: no Tab gave a sure read of the column. `not_taken`: a sure read was seen and not taken, because other columns had dropped on that Tab (it may have been the next game's)"
        );
    }
    let mut row = serde_json::json!({
        "closed_at": Utc::now().to_rfc3339(),
        "session_id": g.session_id,
        "outcome": g.outcome.to_string(),
        "map": g.map,
        "age_secs": g.opened_at.elapsed().as_secs(),
        "reason": reason,
    });
    if !unread.is_empty() {
        row["unread"] = serde_json::json!(unread);
        row["never_read"] = serde_json::json!(never_read);
        row["read_not_taken"] = serde_json::json!(not_taken);
    }
    append_unrecorded_row(data_dir, &row);
}

/// Close the in-memory session because a new game is starting.
///
/// A confirmed outcome is already in the store. A provisional hint is written
/// when `seal` is set: a map vote, a hero ban, a hero select with no board
/// yet, or a stat reset or gap that closed a hinted session. A stat split
/// passes `report.seal` the same way. A session with no Tab captures is
/// logged to `debug/unrecorded_games.jsonl`.
pub(crate) async fn retire_active_game(
    st: &mut SessionState,
    store: &storage::LocalStore,
    data_dir: &std::path::Path,
    seal: Option<detect::MatchOutcome>,
    reason: &str,
) {
    // The game leaves memory in `begin_retire`, before these store calls
    // return. Dropping this future cancels the best-guess and the sealed
    // outcome, and close_session_on_shutdown then has nothing left to save.
    let Some((g, had_outcome)) = begin_retire(st, data_dir, seal, reason) else {
        return;
    };
    commit_retired_game(st, store, data_dir, g, had_outcome, reason).await;
}

/// Test-only pause after the game has left memory and before the store
/// writes. Release builds do not have this hook.
#[cfg(test)]
pub(crate) async fn retire_after_pause<P>(
    st: &mut SessionState,
    store: &storage::LocalStore,
    data_dir: &std::path::Path,
    seal: Option<detect::MatchOutcome>,
    reason: &str,
    pause: P,
) where
    P: std::future::Future<Output = ()>,
{
    let Some((g, had_outcome)) = begin_retire(st, data_dir, seal, reason) else {
        return;
    };
    pause.await;
    commit_retired_game(st, store, data_dir, g, had_outcome, reason).await;
}

pub(crate) fn begin_retire(
    st: &mut SessionState,
    data_dir: &std::path::Path,
    seal: Option<detect::MatchOutcome>,
    reason: &str,
) -> Option<(ActiveGame, bool)> {
    // The rank-screen log must not hide the next game's same result. Clear
    // it before the early return: a Tab-opened game can close while the
    // rank screen is up even when no game is active.
    stat_tracker::detect::match_end::clear_rank_result_log();
    let mut g = st.active_game.take()?;
    // The game is over: its unblanked portrait cells leave memory and a
    // below-floor column calibration must not stick into the next game.
    stat_tracker::ocr::clear_below_floor_column_cache();
    stat_tracker::report::kit::forget_portraits(data_dir, &g.session_id);
    let had_outcome = g.finished();
    if !had_outcome {
        // Only an explicit two-signal seal. A leftover hint is not an outcome.
        let outcome = boundary::outcome_sealed_on_close(g.outcome, seal);
        if let Some(outcome) = outcome {
            g.record_outcome(outcome);
            tracing::info!(
                target: "scuffed_stat_tracker",
                outcome = %outcome,
                session_id = %g.session_id,
                reason,
                "sealing session with end-screen outcome at new-game boundary"
            );
        }
    }
    Some((g, had_outcome))
}

pub(crate) async fn commit_retired_game(
    st: &mut SessionState,
    store: &storage::LocalStore,
    data_dir: &std::path::Path,
    mut g: ActiveGame,
    had_outcome: bool,
    reason: &str,
) {
    store_best_guess_on_close(store, data_dir, &mut g).await;
    if g.session_created && g.finished() && !had_outcome {
        if let Err(e) = store
            .set_session_outcome(&g.session_id, &g.outcome.to_string())
            .await
        {
            tracing::warn!(target: "scuffed_stat_tracker", error = %e, "failed to seal session outcome");
        }
        refresh_snapshot(store, data_dir).await;
    }
    // No result ever arrived. The server cannot store that. The command
    // tick marks the rows synced locally and does not send them.
    if g.session_created && !g.finished() {
        st.settle_closed_unknown = Some(g.session_id.clone());
    }
    if g.session_created {
        tracing::info!(
            target: "scuffed_stat_tracker",
            session_id = %g.session_id,
            outcome = %g.outcome,
            map = g.map.as_deref().unwrap_or("?"),
            reason,
            "{}",
            session_close_log(reason)
        );
    } else {
        note_unrecorded_game(data_dir, &g, reason);
    }
}

/// Close a quiet session the way a boundary does, then schedule an upload.
///
/// The session id is left on the stored rows. `active_game.json` is removed
/// before the upload so a restart cannot close the same session again.
/// `sync_now` schedules [`schedule_store_sync`]. It does not run the
/// shutdown upload. No screen capture and no OCR.
pub(crate) async fn close_quiet_session_and_sync<F, Fut>(
    st: &mut SessionState,
    store: &storage::LocalStore,
    data_dir: &std::path::Path,
    now: Instant,
    close_after: std::time::Duration,
    sync_now: F,
) -> bool
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let Some(game) = st.active_game.as_ref() else {
        return false;
    };
    let Some(reason) = quiet_close_reason(game, now, close_after) else {
        return false;
    };
    let session_id = game.session_id.clone();
    let carried = game
        .deferred
        .map(|counters| (counters, game.deferred_hero.clone(), game.deferred_at));
    tracing::info!(
        target: "scuffed_stat_tracker",
        session_id = %session_id,
        reason,
        "closing quiet session"
    );
    // No seal. A finished game already has its outcome. An unfinished game
    // closes here only after [`UNFINISHED_SESSION_MAX`], or after
    // [`UNFINISHED_SESSION_IDLE`] when it is empty ([`is_empty_tab_game`]).
    retire_active_game(st, store, data_dir, None, reason).await;
    if let Some((counters, hero, at)) = carried {
        let mut opened = ActiveGame::open_at(
            format!("{:016x}", rand_id()),
            detect::MatchOutcome::Unknown,
            Vec::new(),
            now,
        );
        opened.deferred = Some(counters);
        opened.deferred_hero = hero;
        opened.deferred_at = at;
        opened.deferred_imported = true;
        st.active_game = Some(opened);
        st.last_game_open = Some(now);
        persist_active_game(data_dir, st.active_game.as_ref());
    } else {
        persist_active_game(data_dir, None);
    }
    sync_now().await;
    true
}

/// How long after a game's outcome is recorded that Tab captures still belong
/// to it. The post-match scoreboard is typically inspected right after the
/// result screens; without this window each such Tab opened a duplicate
/// session for the same match and double-counted it. Past the window, the
/// finished result must not leak onto the next match's captures.
pub(crate) const POST_MATCH_GRACE: std::time::Duration = std::time::Duration::from_secs(75);

/// Quiet time before a finished game is closed when nothing further is
/// recorded. Counted from `last_activity` (a stored capture, a recorded
/// outcome, an accolade map, or the session open). Never shorter than
/// [`POST_MATCH_GRACE`].
pub(crate) fn finished_game_close_after(configured_secs: u64) -> std::time::Duration {
    std::time::Duration::from_secs(configured_secs.max(POST_MATCH_GRACE.as_secs()))
}

/// Why the command-tick timer is closing the open session. Not a new-game
/// boundary: the session id stays, and no outcome is invented. Unfinished
/// games stay open on this timer so a late result can still attach.
/// [`UNFINISHED_SESSION_IDLE`] still starts a fresh session when a later
/// Tab arrives. An unfinished game is closed at the next new-game
/// boundary, or on the next start once the on-disk session is older than
/// [`UNFINISHED_SESSION_IDLE`]. A clean shutdown keeps that skeleton so a
/// restart inside the bound resumes it. After [`UNFINISHED_SESSION_MAX`]
/// with no result, this timer closes it. Its `unknown` rows are marked
/// synced locally and are not sent. A GUI SetOutcome requeues them with
/// `edited` set.
///
/// An empty game ([`is_empty_tab_game`]) has nothing a late result could
/// attach to, so it closes after [`UNFINISHED_SESSION_IDLE`] instead. That
/// is the same bound after which the next Tab would start a fresh game
/// anyway. Nothing is stored or uploaded for it.
pub(crate) fn quiet_close_reason(
    game: &ActiveGame,
    now: Instant,
    close_after: std::time::Duration,
) -> Option<&'static str> {
    if !game.finished() {
        let idle = now.saturating_duration_since(game.last_activity);
        if idle >= UNFINISHED_SESSION_MAX {
            return Some(UNFINISHED_MAX_REASON);
        }
        if is_empty_tab_game(game) && idle >= UNFINISHED_SESSION_IDLE {
            return Some(EMPTY_IDLE_REASON);
        }
        return None;
    }
    let close_after = close_after.max(POST_MATCH_GRACE);
    let grace_expired = game
        .outcome_recorded_at
        .is_none_or(|recorded| now.saturating_duration_since(recorded) > POST_MATCH_GRACE);
    let quiet = now.saturating_duration_since(game.last_activity) >= close_after;
    if grace_expired && quiet {
        Some(QUIET_CLOSE_REASON)
    } else {
        None
    }
}

/// A game a Tab opened whose captures were all rejected: no row stored, no
/// board read, no held board, no result hint, and not opened by a start
/// screen. On 2026-10-17 a Tab on the rank screen 81s after a Victory opened
/// one, and it stayed open for [`UNFINISHED_SESSION_MAX`].
pub(crate) fn is_empty_tab_game(game: &ActiveGame) -> bool {
    !game.session_created
        && !game.awaiting_first_board
        && game.gate.is_none()
        && game.best_guess.is_none()
        && game.deferred.is_none()
        && game.result_mark.is_none()
        && game.rejoin_carry.is_none()
}

/// How long an unfinished session stays reusable with no recorded activity.
/// If the poller misses the outcome AND the next game's start screens (likely
/// under Tab starvation, or with auto-detect off), an unbounded session would
/// absorb every capture that follows, yesterday's unfinished game swallowing
/// today's first Tab. Sized comfortably above a long match plus queue time.
pub(crate) const UNFINISHED_SESSION_IDLE: std::time::Duration =
    std::time::Duration::from_secs(20 * 60);

/// Upper bound for a session that never got a result. A long match still
/// takes its result. Past this, the command tick closes the session and
/// holds the unknown rows locally.
pub(crate) const UNFINISHED_SESSION_MAX: std::time::Duration =
    std::time::Duration::from_secs(6 * 60 * 60);

/// Minimum age of a poller-opened game before a Tab frame's accolade and
/// rank-screen word reads are trusted ([`FrameFlags::allow_banner_recovery`]);
/// a real end can't appear this early into a match. The end-title text path
/// stays available regardless, and a session freshly opened by the Tab
/// itself (daemon joined mid/post match) is exempt, there the end screen is
/// exactly the evidence being recovered. Named for the colour-flood outcome
/// this gate was made for, which false-positived on heavy mid-fight red and
/// is gone since 0.5.0-rc.9.
pub(crate) const MIN_BANNER_SESSION_AGE: std::time::Duration = std::time::Duration::from_secs(300);

/// A game restored after a restart with a result screen already seen is a
/// finished game once [`POST_MATCH_GRACE`] has passed since that result:
/// the next Tab is the next game, even on the same map.
pub(crate) fn restored_result_is_past_grace(g: &ActiveGame, now: Instant) -> bool {
    g.restored_with_result
        && g.result_mark
            .is_some_and(|m| now.saturating_duration_since(m.seen_at) > POST_MATCH_GRACE)
}

/// Whether a Tab capture should open a fresh session instead of reusing the
/// active one.
pub(crate) fn should_start_fresh_session(game: Option<&ActiveGame>, now: Instant) -> bool {
    match game {
        // No game open, daemon started mid-match or the start screen was missed.
        None => true,
        // Mid-game capture, unless the session has been idle so long it
        // can't plausibly be the same game.
        Some(g) if !g.finished() => {
            now.duration_since(g.last_activity) > UNFINISHED_SESSION_IDLE
                || restored_result_is_past_grace(g, now)
        }
        // Finished: reuse within the grace window (post-match scoreboard of the
        // same match), start fresh after it. An unstamped outcome is treated as
        // stale so a finished result can never leak forward.
        Some(g) => g
            .outcome_recorded_at
            .is_none_or(|t| now.duration_since(t) > POST_MATCH_GRACE),
    }
}

/// How recent a finished game's last accepted board must be for a Tab past
/// the grace window to be compared with it ([`finished_game_tab_probe`]).
/// After a real end, the next game's board needs the end screens, a queue,
/// hero select and play before it can reach a mature board's numbers; well
/// over this. The same gap as [`STAT_SPLIT_MIN_GAP`], below which a stat
/// drop does not split an unfinished game either.
pub(crate) const FINISHED_GAME_TAB_MAX_GAP: std::time::Duration =
    std::time::Duration::from_secs(120);

/// Whether this Tab would open a fresh session past the grace window of a
/// finished game (as [`should_start_fresh_session`] says) but its board may
/// show that same game still being played (R9-2, 0.5.0-rc.9). Then the board
/// decides ([`boundary::finished_game_tab`]): the Tab's capture carries the
/// open game's request too ([`CaptureRequest::finished_game_probe`]), and
/// the fresh session opens only when its report lands and the board was not
/// that game ([`settle_tab_probe`]).
///
/// Only a game whose last accepted board is recent
/// ([`FINISHED_GAME_TAB_MAX_GAP`]), with a stored row, a trusted map, a
/// result the tracker read itself ([`ActiveGame::outcome_locked`]), and no
/// held fresh-match board, rejoin carry or restored result. Anything else
/// opens the fresh session at the press, as before.
pub(crate) fn finished_game_tab_probe(game: Option<&ActiveGame>, now: Instant) -> bool {
    let Some(g) = game else {
        return false;
    };
    g.finished()
        && !g.outcome_locked
        && g.session_created
        && !g.restored_with_result
        && !g.awaiting_first_board
        && g.deferred.is_none()
        && g.rejoin_carry.is_none()
        && known_label(g.map.as_deref()).is_some()
        && g.map_is_trusted()
        && g.gate.is_some()
        && g.last_stats_at
            .is_some_and(|at| now.saturating_duration_since(at) <= FINISHED_GAME_TAB_MAX_GAP)
        && should_start_fresh_session(game, now)
}

pub(crate) fn known_label(s: Option<&str>) -> Option<&str> {
    s.map(str::trim)
        .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("unknown"))
}

pub(crate) fn maps_compatible(
    prev_map: Option<&str>,
    incoming_map: Option<&str>,
    vote_candidates: &[String],
) -> bool {
    match (known_label(prev_map), known_label(incoming_map)) {
        (Some(prev), Some(incoming)) => prev.eq_ignore_ascii_case(incoming),
        (Some(prev), None) if !vote_candidates.is_empty() => {
            vote_candidates.iter().any(|c| c.eq_ignore_ascii_case(prev))
        }
        // Unfinished game already has a map; the opener has no map yet
        // (lingering vote with empty OCR, or a split frame that hasn't
        // resolved). Same match until a contradictory map arrives.
        (Some(_), None) => true,
        // No confirmed previous map, identity reuse does not apply.
        // Map-vote debounce still covers a lingering vote with no Tab yet.
        (None, _) => false,
    }
}

pub(crate) fn heroes_compatible(prev: Option<&str>, incoming: Option<&str>) -> bool {
    match (known_label(prev), known_label(incoming)) {
        (Some(prev), Some(incoming)) => prev.eq_ignore_ascii_case(incoming),
        _ => true,
    }
}

/// Reuse the open session instead of splitting when an unfinished (no-outcome)
/// game with the same map+hero is still active or recently active.
///
/// The map-vote cooldown (`auto_detect.cooldown_secs`, default 120s) only
/// debounces a lingering vote opening a *second* new game. A Tab that opened
/// the first session sets `last_game_open`, so after 120s a lingering or
/// false-positive vote can open another, observed as two Games cards for one
/// match (same map/hero, empty first row + later WIN, ~6 min apart).
pub(crate) fn same_unfinished_match(
    unfinished: bool,
    activity_age: std::time::Duration,
    prev_map: Option<&str>,
    prev_hero: Option<&str>,
    incoming: IncomingIdentity<'_>,
) -> bool {
    unfinished
        && activity_age <= UNFINISHED_SESSION_IDLE
        && maps_compatible(prev_map, incoming.map, incoming.vote_candidates)
        && heroes_compatible(prev_hero, incoming.hero)
}

pub(crate) fn should_reuse_unfinished_same_match(
    previous: Option<&ActiveGame>,
    incoming: IncomingIdentity<'_>,
    now: Instant,
) -> bool {
    let Some(g) = previous else {
        return false;
    };
    same_unfinished_match(
        !g.finished(),
        now.saturating_duration_since(g.last_activity),
        g.map.as_deref(),
        g.hero_auth.accepted_hero.as_deref(),
        incoming,
    )
}

/// Map-vote opener: existing debounce, plus same-map/hero reuse so a lingering
/// vote after the 120s cooldown cannot split an unfinished match.
pub(crate) fn map_vote_should_open_new_game(
    active: Option<&ActiveGame>,
    last_game_open: Option<Instant>,
    now: Instant,
    debounce: std::time::Duration,
    vote_candidates: &[String],
) -> bool {
    let game_finished = active.is_some_and(ActiveGame::finished);
    let cooldown_elapsed =
        last_game_open.is_none_or(|t| now.saturating_duration_since(t) >= debounce);
    if !(active.is_none() || game_finished || cooldown_elapsed) {
        return false;
    }
    !should_reuse_unfinished_same_match(
        active,
        IncomingIdentity {
            map: None,
            hero: None,
            vote_candidates,
        },
        now,
    )
}

/// Map vote uses the debounce and the same-map guard. Hero select and ban
/// are blocked only while the session they just opened is still inside the
/// debounce, so a stable screen does not split itself on the next tick.
pub(crate) fn start_screen_blocked(
    active: Option<&ActiveGame>,
    last_game_open: Option<Instant>,
    now: Instant,
    debounce: std::time::Duration,
    screen: &boundary::StartScreen,
) -> bool {
    match screen {
        boundary::StartScreen::MapVote { candidates } => {
            !map_vote_should_open_new_game(active, last_game_open, now, debounce, candidates)
        }
        boundary::StartScreen::HeroSelect | boundary::StartScreen::HeroBan => {
            active.is_some_and(|g| g.awaiting_first_board)
                && last_game_open
                    .is_some_and(|opened| now.saturating_duration_since(opened) < debounce)
        }
    }
}

/// Start a session when the poller sees a start screen and nothing is open.
pub(crate) fn open_detected_game(
    st: &mut SessionState,
    data_dir: &std::path::Path,
    screen: &boundary::StartScreen,
    now: Instant,
    debounce: std::time::Duration,
) {
    let (candidates, log_vote) = match screen {
        boundary::StartScreen::MapVote { candidates } => {
            if !map_vote_should_open_new_game(None, st.last_game_open, now, debounce, candidates) {
                return;
            }
            (candidates.clone(), true)
        }
        boundary::StartScreen::HeroSelect | boundary::StartScreen::HeroBan => (Vec::new(), false),
    };
    let sid = format!("{:016x}", rand_id());
    if log_vote {
        tracing::info!(target: "scuffed_stat_tracker", ?candidates, session_id = %sid, "auto-detect: map vote: new game");
    } else {
        tracing::info!(
            target: "scuffed_stat_tracker",
            session_id = %sid,
            "auto-detect: hero select/ban: new game (map vote missed)"
        );
    }
    let mut g = ActiveGame::open_at(sid, detect::MatchOutcome::Unknown, candidates, now);
    g.awaiting_first_board = true;
    st.active_game = Some(g);
    st.last_game_open = Some(now);
    clear_cadence_wakes(st);
    st.pending_outcome = None;
    persist_active_game(data_dir, st.active_game.as_ref());
}

/// How long an outcome seen with no game open stays applicable to the next
/// session that opens. Covers "daemon started during the post-match screens";
/// without the bound, an outcome from hours ago could stamp a future game.
pub(crate) const PENDING_OUTCOME_TTL: std::time::Duration = std::time::Duration::from_secs(90);

/// Take the pending outcome if it is still fresh; stale ones are discarded.
pub(crate) fn take_fresh_pending(
    pending: &mut Option<(detect::MatchOutcome, Instant)>,
    now: Instant,
) -> Option<detect::MatchOutcome> {
    let (outcome, seen_at) = pending.take()?;
    if now.duration_since(seen_at) <= PENDING_OUTCOME_TTL {
        Some(outcome)
    } else {
        tracing::debug!(target: "scuffed_stat_tracker", ?outcome, "discarding stale pending outcome");
        None
    }
}
