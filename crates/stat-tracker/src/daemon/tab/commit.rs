//! Writing a Tab capture to the store: the staged rows and the carried
//! board, held and skipped boards, blank rejects and the best guess kept
//! when a session closes.

use crate::*;

/// The next `store_held_board` for this session id fails once, then clears.
/// Other sessions are left alone so parallel tests do not trip it.
#[cfg(test)]
pub(crate) static FAIL_HELD_BOARD_SESSION: std::sync::Mutex<Option<String>> =
    std::sync::Mutex::new(None);

/// Write the board that was held off the finished game onto the new session
/// at that capture's own timestamp, and record it the same way as a Tab.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn store_held_board(
    store: &storage::LocalStore,
    data_dir: &std::path::Path,
    session_id: &str,
    hero: &str,
    map_name: &str,
    outcome: &str,
    counters: Counters,
    played_at: chrono::DateTime<Utc>,
) -> anyhow::Result<()> {
    #[cfg(test)]
    {
        let mut fail = FAIL_HELD_BOARD_SESSION
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if fail.as_deref() == Some(session_id) {
            *fail = None;
            anyhow::bail!("held board write failed");
        }
    }
    let played_at = SurrealDatetime::from(played_at);
    let hero = parse::canonical_hero(hero);
    let row = storage::PersonalMatch {
        id: None,
        hero: hero.clone(),
        map_name: map_name.to_string(),
        game_mode: parse::stored_game_mode(map_name),
        role: parse::guess_role_public(&hero),
        outcome: outcome.to_string(),
        elims: counters.elims,
        deaths: counters.deaths,
        assists: counters.assists,
        damage: counters.damage,
        healing: counters.healing,
        mitigation: counters.mitigation,
        played_at,
        synced: false,
        sync_rev: 0,
        upload_reject: None,
        session_id: session_id.to_string(),
        corrected_hero: None,
        corrected_role: None,
        corrected_map_name: None,
        corrected_outcome: None,
        corrected_elims: None,
        corrected_deaths: None,
        corrected_assists: None,
        corrected_damage: None,
        corrected_healing: None,
        corrected_mitigation: None,
        edited_fields: Vec::new(),
        edited_at: None,
        heroes_played: Vec::new(),
        segment_resolutions: Vec::new(),
        recognizer: scuffed_types::RECOGNIZER_OCR_V1.to_string(),
        suspect_fields: Vec::new(),
        name_recognizer: None,
    };
    storage::append_match_log(data_dir, &row);
    store
        .insert_match(row)
        .await
        .map_err(anyhow::Error::from_boxed)
        .context("deferred board insert failed")?;
    if let Err(e) = store.append_capture(session_id, played_at, outcome).await {
        tracing::warn!(target: "scuffed_stat_tracker", error = %e, "failed to append the deferred board to the session");
    }
    if let Err(e) = store.refresh_session_hero_timeline(session_id).await {
        tracing::debug!(target: "scuffed_stat_tracker", error = %e, "failed to refresh hero timeline for the deferred board");
    }
    Ok(())
}

/// The hold-path report. Production and the night harness both build it
/// here, so the stored outcome and the career-panel flag cannot drift.
pub(crate) fn skipped_capture_report(
    plan: &boundary::CapturePlan,
    session_id: &str,
    career_panel: bool,
    hero_auth: HeroAuthState,
    held_hero: Option<String>,
    held_at: Option<chrono::DateTime<Utc>>,
) -> CaptureReport {
    CaptureReport {
        best_guess: None,
        recorded: false,
        outcome: plan.stored_outcome,
        map: None,
        map_source: None,
        session_id: session_id.to_string(),
        split: false,
        armed_reset: plan.defer,
        ignore_row: plan.ignore_row,
        reset_streak: plan.reset_streak,
        reset_baseline: plan.reset_baseline,
        baseline_row: plan.baseline_row,
        refresh_baseline: false,
        clear_hint: false,
        count_progress: false,
        career_panel,
        held_counters: plan.deferred_counters,
        held_hero,
        gate_state: None,
        hero_auth,
        seal: None,
        close_reason: None,
        held_at,
        map_replaces_text_fallback: false,
        rejoin_carry: None,
        finished_game_tab: None,
    }
}

/// The carried board is written only after the current row insert succeeds.
/// A failed current insert must leave the hold for the next stored Tab.
pub(crate) fn should_write_carried(current_inserted: bool, has_carried: bool) -> bool {
    current_inserted && has_carried
}

/// Session row for one staged capture. The carried board is not written here.
/// Production and the night harness both call [`write_carried_after_current`]
/// only after the current snapshot insert returns ok.
pub(crate) async fn write_staged_rows(
    store: &storage::LocalStore,
    staged: &StagedCapture,
    hero: &str,
    role: &str,
    now: SurrealDatetime,
) -> anyhow::Result<bool> {
    let mut create = staged.create_session;
    if staged.carried.is_some() && create {
        let played_at = staged.carried_at;
        let carried_hero = staged
            .carried_hero
            .clone()
            .unwrap_or_else(|| hero.to_string());
        let session = storage::MatchSession {
            session_id: staged.target_session.clone(),
            hero: carried_hero,
            map_name: staged.map_name.clone(),
            role: role.to_string(),
            started_at: SurrealDatetime::from(played_at),
            last_capture_at: SurrealDatetime::from(played_at),
            capture_count: 0,
            final_outcome: staged.outcome_label.clone(),
        };
        store
            .create_session(&session)
            .await
            .map_err(anyhow::Error::from_boxed)
            .context("session create failed")?;
        create = false;
    }
    if create {
        let session = storage::MatchSession {
            session_id: staged.target_session.clone(),
            hero: hero.to_string(),
            map_name: staged.map_name.clone(),
            role: role.to_string(),
            started_at: now,
            last_capture_at: now,
            capture_count: 1,
            final_outcome: staged.outcome_label.clone(),
        };
        // A failed create must abort the capture: pressing on would write
        // a snapshot the caller then marks "session created", and every
        // later outcome/map back-fill would update a session row that
        // doesn't exist. The Tab can simply be pressed again.
        store
            .create_session(&session)
            .await
            .map_err(anyhow::Error::from_boxed)
            .context("session create failed")?;
        tracing::info!(target: "scuffed_stat_tracker", session_id = %staged.target_session, "started new match session");
    } else if let Err(e) = store
        .append_capture(&staged.target_session, now, &staged.outcome_label)
        .await
    {
        tracing::warn!(target: "scuffed_stat_tracker", error = %e, "failed to append capture to session");
    }
    Ok(create)
}

/// Write the held board after the current snapshot insert has succeeded.
pub(crate) async fn write_carried_after_current(
    store: &storage::LocalStore,
    data_dir: &std::path::Path,
    staged: &StagedCapture,
    hero: &str,
    current_inserted: bool,
) -> anyhow::Result<()> {
    if !should_write_carried(current_inserted, staged.carried.is_some()) {
        return Ok(());
    }
    let Some(counters) = staged.carried else {
        return Ok(());
    };
    let carried_hero = staged
        .carried_hero
        .clone()
        .unwrap_or_else(|| hero.to_string());
    store_held_board(
        store,
        data_dir,
        &staged.target_session,
        &carried_hero,
        &staged.map_name,
        &staged.outcome_label,
        counters,
        staged.carried_at,
    )
    .await
}

/// Whether the staged row is stored or dropped before insert.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PreparedRow {
    Store,
    Drop,
}

/// Map, mode, and the PvE gate for one staged capture.
///
/// An untrusted Château Guillard read is dropped back to the session map
/// so a fuzzy board cannot rename an open game. A stat split still carries
/// the previous game on the request, so that fallback is empty and the new
/// session stays Unknown. A trusted Deathmatch map stays, and the caller
/// stores that row locally. Adlersbrunn with stats on only one team is not
/// stored as Eichenwalde.
///
/// An untrusted read of a local-only map with no open map is dropped,
/// except when this capture is the stat-reset split. That split stores
/// the new session with an empty map. The review hold keeps it unsynced
/// until a pick, and it is not uploaded as the game that just closed.
pub(crate) fn prepare_capture_row(
    parsed: &mut storage::PersonalMatch,
    staged: &mut StagedCapture,
    session_map: Option<&str>,
    panel_raw: Option<&str>,
    board_text: &str,
    both_teams: bool,
) -> PreparedRow {
    if parse::map_is_untracked(&staged.map_name)
        && !staged
            .map_source
            .is_some_and(|source| source.trusted_for_board_split())
    {
        // `session_map` is the open game. On a split that game is the one
        // being closed, and an untrusted read must not name the new session
        // after it. With no open map the row would be blank, and a blank
        // map uploads, so the capture is dropped.
        let fallback = if staged.split {
            None
        } else {
            session_map.filter(|name| !name.is_empty())
        };
        staged.map_name = fallback.unwrap_or("").to_string();
        staged.recorded_map = (!staged.map_name.is_empty()).then(|| staged.map_name.clone());
        staged.map_source = None;
        if staged.map_name.is_empty() && !staged.split {
            tracing::info!(target: "scuffed_stat_tracker", "untracked map with no open game is not stored");
            return PreparedRow::Drop;
        }
    }
    parsed.map_name = staged.map_name.clone();
    parsed.game_mode = parse::stored_game_mode(&parsed.map_name);
    let alias = panel_raw.is_some_and(parse::is_adlersbrunn_alias)
        || parse::is_adlersbrunn_alias(board_text);
    if parse::reject_pve_adlersbrunn(alias, &parsed.map_name, both_teams) {
        tracing::info!(
            target: "scuffed_stat_tracker",
            "adlersbrunn scoreboard has stats on only one team, not stored as Eichenwalde"
        );
        return PreparedRow::Drop;
    }
    PreparedRow::Store
}

pub(crate) fn blank_rejected_report(
    outcome: detect::MatchOutcome,
    session_id: &str,
    hero_auth: HeroAuthState,
    req: &CaptureRequest,
) -> CaptureReport {
    CaptureReport {
        best_guess: None,
        recorded: false,
        outcome,
        map: None,
        map_source: None,
        session_id: session_id.to_string(),
        split: false,
        armed_reset: false,
        ignore_row: false,
        reset_streak: req.reset_streak,
        reset_baseline: req.reset_baseline,
        baseline_row: req.baseline_row,
        refresh_baseline: false,
        clear_hint: false,
        count_progress: false,
        career_panel: false,
        held_counters: None,
        held_hero: None,
        gate_state: None,
        hero_auth,
        seal: None,
        close_reason: None,
        held_at: None,
        map_replaces_text_fallback: false,
        rejoin_carry: None,
        finished_game_tab: None,
    }
}

/// Insert the current row, then the carried board. The carried write runs
/// only after the insert succeeds.
/// Put the reader's best guess in each unread column of `parsed` and list
/// those columns in `suspect_fields` (upload names: `e`, `a`, `d`, `dmg`,
/// `h`, `mit`), so the app marks them for a one-click edit.
pub(crate) fn apply_best_guess(
    parsed: &mut storage::PersonalMatch,
    unread: &[&'static str],
    guess: [u32; capture_gate::GATE_COLS],
) {
    let cells = [
        &mut parsed.elims,
        &mut parsed.assists,
        &mut parsed.deaths,
        &mut parsed.damage,
        &mut parsed.healing,
        &mut parsed.mitigation,
    ];
    for (col, cell) in cells.into_iter().enumerate() {
        if unread.contains(&capture_gate::COL_FIELD[col]) {
            *cell = guess[col];
        }
    }
    for name in unread {
        if !parsed.suspect_fields.iter().any(|f| f == name) {
            parsed.suspect_fields.push((*name).to_string());
        }
    }
}

/// alpha.8: a game that closes with no stored row but a best guess stores
/// the guess, with the game's outcome and map. Returns whether it stored.
pub(crate) async fn store_best_guess_on_close(
    store: &storage::LocalStore,
    data_dir: &std::path::Path,
    g: &mut ActiveGame,
) -> bool {
    if g.session_created {
        return false;
    }
    let Some(mut row) = g.best_guess.take() else {
        return false;
    };
    row.session_id = g.session_id.clone();
    if g.finished() {
        row.outcome = g.outcome.to_string();
    }
    storage::append_match_log(data_dir, &row);
    match store.insert_match(row.clone()).await {
        Ok(_) => {
            tracing::info!(
                target: "scuffed_stat_tracker",
                session_id = %g.session_id,
                suspect = ?row.suspect_fields,
                "game closed with a stat never read for sure: stored the best guess, marked unsure"
            );
            g.session_created = true;
            if g.finished() {
                refresh_snapshot_game_finished(store, data_dir).await;
            } else {
                refresh_snapshot(store, data_dir).await;
            }
            true
        }
        Err(e) => {
            tracing::warn!(target: "scuffed_stat_tracker", error = %e, session_id = %g.session_id, "best guess not stored");
            false
        }
    }
}

pub(crate) async fn commit_capture_rows(
    store: &storage::LocalStore,
    data_dir: &std::path::Path,
    staged: &StagedCapture,
    parsed: &storage::PersonalMatch,
    now: SurrealDatetime,
) -> anyhow::Result<bool> {
    let created = write_staged_rows(store, staged, &parsed.hero, &parsed.role, now).await?;
    storage::append_match_log(data_dir, parsed);
    store
        .insert_match(parsed.clone())
        .await
        .map_err(anyhow::Error::from_boxed)
        .context("store insert failed")?;
    write_carried_after_current(
        store,
        data_dir,
        staged,
        &parsed.hero,
        should_write_carried(true, staged.carried.is_some()),
    )
    .await?;
    Ok(created)
}
