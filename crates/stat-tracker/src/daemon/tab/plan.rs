//! Planning a Tab capture: which session and game the board belongs to,
//! the map it resolves to, the stats split and gate seeds, member edits
//! carried over, and applying the capture report.

use crate::*;

/// Minimum time since the session's last accepted capture before a stat
/// regression is allowed to split off a new session. Real between-game gaps
/// (result screens + queue + load) measured ≥3 min; consecutive captures of
/// the same board are seconds apart. The gap guard prevents a garbage OCR row
/// followed by a correct one from faking a regression (observed 2026-07-14:
/// a misread E9/D11/DMG61029 row would otherwise split on the next capture).
pub(crate) const STAT_SPLIT_MIN_GAP: std::time::Duration = std::time::Duration::from_secs(120);

/// Whether a capture's player stats regressed versus the session's previous
/// accepted capture. Elims, deaths, and damage are cumulative within one
/// match (hero swaps included), they never decrease. Requiring at least two
/// of the three to drop keeps a single misread column (e.g. an inflated
/// elims read) from faking a boundary, while a real new game, all counters
/// restarting near zero, trips it reliably.
///
/// DUP-1 guard: a column whose CURRENT read is `cur_suspect` (edge-ink
/// clipped/bled, CG-3) must NOT vote as a drop.
///
/// F-CG2-1 raw-continuity vote: DUP-1 alone is not enough, because
/// latch-recovery reads are CLEAN. A CG-2-latched `accepted` sits high while the
/// gate's `last_raw` tracks reality, so after the ≥120s gap the first clean real
/// read (streak still 0, un-latch not yet fired) drops versus the latched
/// `accepted` and would split MID-game (Rialto 2026-07-20T22:23:41Z: "active
/// game replaced after stat-regression split"). So a column votes only if the
/// read also dropped versus `prev_raw` beyond the corroboration band, a
/// latch-recovery read is continuous with `last_raw` and does not, while a
/// genuine new-game reset drops versus BOTH and still splits. When `prev_raw`
/// for a column is itself suspect (untrustworthy for continuity) we fall back
/// conservatively to the DUP-1 behaviour: an accepted-drop alone may vote.
pub(crate) fn stats_regressed(
    prev_acc: (u32, u32, u32),
    prev_raw: (u32, u32, u32),
    cur: (u32, u32, u32),
    cur_suspect: (bool, bool, bool),
    prev_raw_suspect: (bool, bool, bool),
) -> bool {
    let votes = |cur: u32, acc: u32, raw: u32, cur_sus: bool, raw_sus: bool| -> bool {
        if cur_sus || cur >= acc {
            // Current read untrustworthy, or no drop versus accepted → no vote.
            return false;
        }
        if raw_sus {
            // last_raw is not a reliable continuity anchor → DUP-1 fallback.
            true
        } else {
            // Real new-game resets drop below last_raw too; latch-recovery
            // reads stay continuous with it (no vote).
            capture_gate::raw_dropped(raw, cur)
        }
    };
    let drops = [
        votes(
            cur.0,
            prev_acc.0,
            prev_raw.0,
            cur_suspect.0,
            prev_raw_suspect.0,
        ),
        votes(
            cur.1,
            prev_acc.1,
            prev_raw.1,
            cur_suspect.1,
            prev_raw_suspect.1,
        ),
        votes(
            cur.2,
            prev_acc.2,
            prev_raw.2,
            cur_suspect.2,
            prev_raw_suspect.2,
        ),
    ];
    drops.iter().filter(|&&d| d).count() >= 2
}

/// The map to store on a capture snapshot.
///
/// Priority: the session's confirmed map > top-bar label OCR > the fuzzy
/// scoreboard-text read. Map-vote names never appear here, the vote winner
/// is unknowable at vote time (recording candidates as the played map was
/// wrong ~2/3 of the time with 2+ candidates), but when candidates are known
/// they veto OCR reads that aren't among them: the played map must be one of
/// the voted maps, so a read outside the set is a misread.
#[cfg(test)]
pub(crate) fn resolve_map(
    session_map: Option<&str>,
    panel_read: Option<&str>,
    text_read: &str,
    candidates: &[String],
) -> String {
    resolved_map(session_map, panel_read, text_read, candidates).0
}

/// The map to store, and which read supplied it.
///
/// A session that already has a map keeps that name. A top-bar label that
/// agrees with it is [`MapSource::TopBar`], so a later Tab can upgrade a
/// text fallback. A top-bar label on a session with no map is
/// [`MapSource::TopBar`]. The full-board text is
/// [`MapSource::TextFallback`] and is not trusted for a poll split or a
/// later different-map split until a top bar or an accolade agrees.
///
/// Residual risk (PR 163): a Gibraltar vote card read only as `WATCHPOINT`
/// is not a candidate, because a bare watchpoint is not a map. The
/// candidate veto can then drop a real Gibraltar top-bar read.
pub(crate) fn resolved_map(
    session_map: Option<&str>,
    panel_read: Option<&str>,
    text_read: &str,
    candidates: &[String],
) -> (String, Option<boundary::MapSource>) {
    let plausible = |m: &&str| candidates.is_empty() || candidates.iter().any(|c| c == m);
    if let Some(map) = session_map.map(str::trim).filter(|name| !name.is_empty()) {
        let upgrade = panel_read
            .map(str::trim)
            .filter(|panel| {
                !panel.is_empty() && panel.eq_ignore_ascii_case(map) && plausible(panel)
            })
            .map(|_| boundary::MapSource::TopBar);
        return (map.to_string(), upgrade);
    }
    let dropped = |m: &&str| {
        if !plausible(m) {
            tracing::debug!(
                target: "scuffed_stat_tracker",
                read = %m,
                ?candidates,
                "map read is not a vote candidate: dropping as misread"
            );
        }
    };
    if let Some(panel) = panel_read.inspect(dropped).filter(plausible) {
        return (panel.to_string(), Some(boundary::MapSource::TopBar));
    }
    if let Some(text) = Some(text_read)
        .filter(|m| !m.is_empty())
        .inspect(dropped)
        .filter(plausible)
    {
        return (text.to_string(), Some(boundary::MapSource::TextFallback));
    }
    (String::new(), None)
}

/// Gate, map, and carried-board decisions for one planned capture.
///
/// `plan.skip_store` is the hold path and is not staged: nothing is written.
/// The trust bit rides on [`BoardFacts`] with the counters, so this does not
/// grow another argument.
pub(crate) fn stage_capture(
    req: &CaptureRequest,
    plan: &boundary::CapturePlan,
    facts: &BoardFacts<'_>,
    captured_at: chrono::DateTime<Utc>,
) -> StagedCapture {
    let split = plan.split;
    let outcome = plan.stored_outcome;
    let target_session = if split {
        format!("{:016x}", rand_id())
    } else {
        req.session_id.clone()
    };
    let carried = carried_counters_to_write(req.carried_deferred, split, req.deferred_imported);
    let carried_at = req
        .carried_deferred_at
        .unwrap_or(captured_at - chrono::Duration::seconds(1));
    let gate_prev = gate_prev_for_store(req.prev_gate, req.awaiting_first_board);
    let gate_suspect = if facts.suspect_row {
        [true; capture_gate::GATE_COLS]
    } else {
        facts.suspect
    };
    let gate = capture_gate::apply_gate_seeded(
        gate_prev,
        facts.counters,
        gate_suspect,
        facts.held,
        split,
        facts.trusted_cells && !facts.suspect_row,
        gate_seed(carried, carried_at, captured_at, &facts.held),
    );
    let (mut map_name, mut map_source) = if split {
        resolved_map(None, facts.map_from_panel, facts.parsed_map, &[])
    } else {
        resolved_map(
            req.session_map.as_deref(),
            facts.map_from_panel,
            facts.parsed_map,
            &req.map_candidates,
        )
    };
    let replacement = banner_replaces_text_fallback(req, facts, split);
    let replaced_text_fallback = replacement.is_some();
    if let Some(banner) = replacement {
        tracing::info!(
            target: "scuffed_stat_tracker",
            text_fallback = %map_name,
            banner = %banner,
            "a sure top-bar banner read replaces the text-fallback session map"
        );
        map_name = banner;
        map_source = Some(boundary::MapSource::TopBar);
    }
    StagedCapture {
        target_session,
        create_session: split || req.create_session,
        recorded_map: (!map_name.is_empty()).then(|| map_name.clone()),
        map_name,
        map_source,
        outcome,
        outcome_label: outcome.to_string(),
        gate,
        carried,
        carried_hero: req.carried_deferred_hero.clone(),
        carried_at,
        split,
        replaced_text_fallback,
    }
}

/// The carried board as the capture gate's seed
/// ([`capture_gate::apply_gate_seeded`]), for a capture with held cells.
///
/// The carried board is written onto the same session just before this
/// capture, so a column this capture has no number for takes the carried
/// board's instead of a placeholder 0. `None` when nothing is held (every
/// ocr-v1 capture), so the gate is then the plain one.
pub(crate) fn gate_seed(
    carried: Option<Counters>,
    carried_at: chrono::DateTime<Utc>,
    captured_at: chrono::DateTime<Utc>,
    held: &[bool; capture_gate::GATE_COLS],
) -> Option<(Counters, std::time::Duration)> {
    carried_board(carried, carried_at, captured_at).filter(|_| held.iter().any(|&cell| cell))
}

/// The carried board with its age at this capture.
pub(crate) fn carried_board(
    carried: Option<Counters>,
    carried_at: chrono::DateTime<Utc>,
    captured_at: chrono::DateTime<Utc>,
) -> Option<(Counters, std::time::Duration)> {
    let age = (captured_at - carried_at).to_std().unwrap_or_default();
    carried.map(|board| (board, age))
}

/// A sure banner read that renames the session's untrusted map (F251-1).
///
/// The session map came from the full-board text (or has no source), and
/// this Tab's top-bar map is a sure banner read ([`choose_top_bar_map`])
/// naming another map. It replaces the text-fallback name only while no
/// boundary signal has been seen: no recorded outcome, no unconfirmed result
/// hint (any hint, not only a conflicting one), no armed boundary, no
/// text-fallback lock, no primed reset and no carried board. This is
/// deliberately stricter than the accolade rule
/// (`boundary::text_fallback_open_for_accolade`), which lets an agreeing hint
/// through; do not loosen it to match. After any of those signals the Tab
/// may already belong to the next game, so the old session keeps its name.
///
/// Two more bounds (F252-2, F252-3):
/// * the session's last accepted capture is at most [`STAT_SPLIT_MIN_GAP`]
///   (120 s) before this Tab, the same gap the stat-regression splitter uses
///   to tell one match's boards from the next game's. A longer gap, or no
///   earlier board, is where a missed game end hides, so game 2's first
///   banner read never renames game 1's rows;
/// * no row of the session carries a member's own map choice
///   (`member_map_edit`): a member's pick is never renamed, and its stored
///   mode is never rewritten.
///
/// The vote-candidate veto applies, and a split is a new session that takes
/// the read anyway. A trusted session map (top bar, accolade) is never
/// replaced: the first trusted map wins. The top-bar OCR never replaces it,
/// so `reader = "ocr-v1"` is unchanged.
pub(crate) fn banner_replaces_text_fallback(
    req: &CaptureRequest,
    facts: &BoardFacts<'_>,
    split: bool,
) -> Option<String> {
    if split || !facts.panel_from_banner {
        return None;
    }
    let banner = facts
        .map_from_panel
        .map(str::trim)
        .filter(|m| !m.is_empty())?;
    let session = req
        .session_map
        .as_deref()
        .map(str::trim)
        .filter(|m| !m.is_empty())?;
    let trusted = req
        .session_map_source
        .is_some_and(boundary::MapSource::trusted_for_board_split);
    let window_open = !req.text_fallback_locked
        && !req.pending_boundary
        && !req.game_outcome.is_decided()
        && req.hint.is_none()
        && req.reset_streak == 0
        && req.carried_deferred.is_none()
        && !req.member_map_edit
        && req
            .prev_gate
            .is_some_and(|(_, age)| age <= STAT_SPLIT_MIN_GAP);
    let plausible = req.map_candidates.is_empty() || req.map_candidates.iter().any(|c| c == banner);
    (!trusted && window_open && plausible && !session.eq_ignore_ascii_case(banner))
        .then(|| banner.to_string())
}

/// A row of the open session carries a member's own map choice (a map edit
/// or pick: `corrected_map_name`, or `map_name` in `edited_fields`). Only
/// asked when the session map is untrusted, the one case a banner read could
/// rename it; a store error answers true, so nothing is renamed.
pub(crate) async fn session_has_member_map_edit(
    store: &storage::LocalStore,
    req: &CaptureRequest,
) -> bool {
    let untrusted_map = req
        .session_map
        .as_deref()
        .is_some_and(|m| !m.trim().is_empty())
        && !req
            .session_map_source
            .is_some_and(boundary::MapSource::trusted_for_board_split);
    if !untrusted_map {
        return false;
    }
    session_rows_have_member_map_edit(store, &req.session_id).await
}

/// A row of `session_id` carries a member's own map choice (a map edit or
/// pick: `corrected_map_name`, or `map_name` in `edited_fields`). A store
/// error answers true, so nothing of the member's is renamed.
pub(crate) async fn session_rows_have_member_map_edit(
    store: &storage::LocalStore,
    session_id: &str,
) -> bool {
    match store.get_session_snapshots(session_id).await {
        Ok(rows) => rows.iter().any(|m| m.has_member_map_edit()),
        Err(e) => {
            tracing::warn!(target: "scuffed_stat_tracker", error = %e, "session rows unreadable; the map is not renamed");
            true
        }
    }
}

/// `reader = "new"`: copy the member's own hero pick on `session_id`
/// ([`storage::member_hero_edit`]) onto `parsed`, a row of that session about
/// to be written. The stored read stays as read; the pick is the overlay the
/// display and upload use, so the read never overrides it. A store error
/// carries nothing and is logged: the pick cannot be known then.
pub(crate) async fn carry_session_hero_edit(
    store: &storage::LocalStore,
    session_id: &str,
    parsed: &mut storage::PersonalMatch,
) {
    match store.get_session_snapshots(session_id).await {
        Ok(rows) => {
            if let Some(edited) = storage::member_hero_edit(&rows)
                && storage::carry_member_hero_edit(edited, parsed)
            {
                tracing::info!(target: "scuffed_stat_tracker", read = %parsed.hero, "member hero pick kept on this capture");
            }
        }
        Err(e) => {
            tracing::warn!(target: "scuffed_stat_tracker", error = %e, "session rows unreadable; member hero pick not checked")
        }
    }
}

/// The previous game is not a latch for a session that has no board yet.
///
/// Guard. [`build_capture_request`] already omits a gate this session does
/// not have, so production passes `None` while awaiting the first board.
/// The filter stays so a caller that still holds the previous game's gate
/// cannot latch this row to it. [`gate_prev_for_store`] is what both the
/// daemon and the night harness apply.
pub(crate) fn gate_prev_for_store(
    prev: Option<(GateState, std::time::Duration)>,
    awaiting_first_board: bool,
) -> Option<(GateState, std::time::Duration)> {
    prev.filter(|_| !awaiting_first_board)
}

/// A held board is written when this capture splits, or when it was carried
/// in from the session that just closed.
pub(crate) fn carried_counters_to_write(
    carried: Option<Counters>,
    split: bool,
    deferred_imported: bool,
) -> Option<Counters> {
    carried.filter(|_| split || deferred_imported)
}

/// The capture plan for one analyzed board. Production and the night harness
/// both call this, so the gap, the same-map guard, and the hinted-map split
/// cannot drift.
pub(crate) fn plan_from_board(
    req: &CaptureRequest,
    facts: &BoardFacts<'_>,
) -> boundary::CapturePlan {
    let incoming_map = facts
        .map_from_panel
        .filter(|s| !s.is_empty())
        .or(req.session_map.as_deref())
        .or(Some(facts.parsed_map).filter(|s| !s.is_empty()));
    let suppress_split = same_unfinished_match(
        matches!(req.game_outcome, detect::MatchOutcome::Unknown),
        std::time::Duration::ZERO,
        req.session_map.as_deref(),
        req.hero_auth.accepted_hero.as_deref(),
        IncomingIdentity {
            map: incoming_map,
            hero: Some(facts.hero),
            vote_candidates: &req.map_candidates,
        },
    );
    let (gate_age, classic_regressed) = match req.prev_gate {
        Some((state, age)) => {
            let classic = stats_regressed(
                state.accepted.edd(),
                state.last_raw.edd(),
                (
                    facts.counters.elims,
                    facts.counters.deaths,
                    facts.counters.damage,
                ),
                (facts.suspect[0], facts.suspect[2], facts.suspect[3]),
                (
                    state.last_raw_suspect[0],
                    state.last_raw_suspect[2],
                    state.last_raw_suspect[3],
                ),
            );
            (Some(age), classic)
        }
        None => (None, false),
    };
    let (prev_gate, baseline) = match req.prev_gate {
        Some((state, _)) => (Some(state), req.reset_baseline),
        None => (None, req.reset_baseline),
    };
    let input = boundary::CapturePlanInput {
        prev_gate: prev_gate.as_ref(),
        baseline: baseline.as_ref(),
        streak: req.reset_streak,
        cur: facts.counters,
        suspect: facts.suspect,
        // A game with boards read but no row stored yet (`reader = "new"`,
        // a column still unread) has a gate of its own to regress from. Its
        // session row does not exist, and that alone is not "no board yet".
        create_session: req.create_session && req.prev_gate.is_none(),
        suppress_same_unfinished: suppress_split,
        age: gate_age,
        min_gap: STAT_SPLIT_MIN_GAP,
        classic_regressed,
        row_counts: facts.trusted_cells,
        row_id: facts.row_id,
        baseline_row: req.baseline_row,
        confirmed_end: req.game_outcome.is_decided(),
        inherited_outcome: req.game_outcome,
        frame_outcome: facts.frame_outcome,
        session_hero: req.hero_auth.accepted_hero.as_deref(),
        hint: req.hint,
        pending_boundary: req.pending_boundary,
        awaiting_first_board: req.awaiting_first_board,
        baseline_age: req.baseline_age,
        progressed_boards: req.progressed_boards,
        session_map: req.session_map.as_deref(),
        session_map_source: req.session_map_source,
        incoming_map,
    };
    boundary::plan_capture_held(
        &input,
        boundary::BoardDoubt {
            held: facts.held,
            suspect_row: facts.suspect_row,
            // A board deferred on this session. One carried in from the
            // session that closed is this game's own row, not a signal.
            board_deferred: req.carried_deferred.is_some() && !req.deferred_imported,
        },
    )
}

/// Fold one capture report into the open session. Store writes already
/// happened. Returns whether this report counts toward the periodic sync.
pub(crate) async fn apply_capture_report(
    st: &mut SessionState,
    store: &storage::LocalStore,
    data_dir: &std::path::Path,
    sid: &str,
    result: Result<CaptureReport, String>,
    now: Instant,
) -> bool {
    // The press of this report's Tab (one capture runs at a time). A caller
    // that made no request through `tab_capture_request` has none: then the
    // report time stands in.
    let pressed = st.tab_press.take().unwrap_or(now);
    match result {
        Err(e) => {
            tracing::error!(target: "scuffed_stat_tracker", error = %e, "capture cycle failed");
            false
        }
        Ok(report) if report.armed_reset => {
            if let Some(g) = st.active_game.as_mut().filter(|g| g.session_id == sid) {
                let mut state = g.boundary_state();
                boundary::note_accepted_capture(
                    &mut state,
                    &boundary::CapturePlan {
                        split: false,
                        defer: true,
                        ignore_row: false,
                        skip_store: true,
                        clear_hint: false,
                        reset_streak: report.reset_streak,
                        reset_baseline: report.reset_baseline,
                        baseline_row: report.baseline_row,
                        refresh_baseline: false,
                        count_progress: false,
                        stored_outcome: g.outcome,
                        deferred_counters: report.held_counters,
                        seal: None,
                        close_reason: None,
                    },
                    GateState::default(),
                    now,
                );
                g.apply_boundary_state(&state);
                g.deferred_imported = false;
                g.deferred_hero = report.held_hero.clone();
                if g.deferred.is_some() {
                    g.deferred_at = report.held_at.or(Some(Utc::now()));
                }
                persist_active_game(data_dir, Some(g));
            }
            false
        }
        Ok(report) if !report.recorded && !report.pending() => {
            // A fuzzy Guillard or Practice Range read must not rename an open
            // Busan session. Only a trusted map (top bar or accolade) may
            // adopt an untracked map, and only onto an empty session or one
            // that is already untracked.
            if let Some(map) = report.map.clone()
                && parse::map_is_untracked(&map)
                && report
                    .map_source
                    .is_some_and(|source| source.trusted_for_board_split())
                && let Some(g) = st.active_game.as_mut().filter(|g| g.session_id == sid)
            {
                let current = g.map.as_deref().unwrap_or("");
                if current.is_empty() || parse::map_is_untracked(current) {
                    let reason = parse::untracked_close_reason(&map);
                    g.map = Some(map);
                    g.map_source = report.map_source;
                    note_untracked_once(data_dir, g, reason);
                    persist_active_game(data_dir, Some(g));
                }
                return false;
            }
            if !matches!(report.outcome, detect::MatchOutcome::Unknown)
                && let Some(g) = st.active_game.as_mut().filter(|g| g.session_id == sid)
                && !g.finished()
            {
                g.record_outcome_at(report.outcome, now);
                g.note_result(report.outcome, true);
                // Adopting the frame's outcome closes the armed boundary.
                // The hint is now the recorded result.
                g.pending_boundary = false;
                tracing::info!(
                    target: "scuffed_stat_tracker",
                    outcome = ?report.outcome,
                    session_id = %g.session_id,
                    "outcome recovered from rejected capture frame: adopting"
                );
                if g.session_created {
                    if let Err(e) = store
                        .set_session_outcome(&g.session_id, &g.outcome.to_string())
                        .await
                    {
                        tracing::warn!(target: "scuffed_stat_tracker", error = %e, "failed to back-fill session outcome");
                    }
                    refresh_snapshot(store, data_dir).await;
                }
                persist_active_game(data_dir, Some(g));
            }
            false
        }
        Ok(report) if report.split => {
            if st.active_game.as_ref().is_some_and(|g| g.session_id == sid) {
                let reason = report
                    .close_reason
                    .map(|reason| reason.log())
                    .unwrap_or("superseded by stat reset");
                retire_active_game(st, store, data_dir, report.seal, reason).await;
                let mut g = session_opened_by_split(
                    report.session_id.clone(),
                    report.outcome,
                    report.hero_auth.clone(),
                    report.career_panel,
                    report.gate_state,
                    report.map.clone(),
                    now,
                );
                g.map_source = report.map_source;
                g.baseline_row = report.baseline_row;
                // A pending board (`reader = "new"`, a column not read yet)
                // opened the game without a row: the session row is made by
                // the first capture that stores one.
                g.session_created = report.recorded;
                if !report.recorded {
                    g.best_guess = report.best_guess.clone();
                }
                tracing::info!(
                    target: "scuffed_stat_tracker",
                    old_session = %sid,
                    session_id = %g.session_id,
                    reason,
                    "active game replaced after stat-regression split"
                );
                st.active_game = Some(g);
                st.last_game_open = Some(now);
                clear_cadence_wakes(st);
                persist_active_game(data_dir, st.active_game.as_ref());
            }
            true
        }
        Ok(report) => {
            // A stored row, or a pending board (`CaptureReport::pending`):
            // the gate, the boundary plan, the hero authority and the map
            // move on either way. Only a stored row creates the session row
            // and counts toward the sync cadence.
            let recorded = report.recorded;
            if let Some(g) = st.active_game.as_mut().filter(|g| g.session_id == sid) {
                if recorded {
                    g.session_created = true;
                    g.best_guess = None;
                } else if report.best_guess.is_some()
                    && report
                        .gate_state
                        .is_none_or(|state| state.refused_cols().is_empty())
                {
                    // A Tab whose first read of a column was not taken
                    // (other columns dropped: likely the next game's board)
                    // does not replace the guess.
                    g.best_guess = report.best_guess.clone();
                }
                g.touch_at(now);
                if report.rejoin_carry.is_some() {
                    g.rejoin_carry = report.rejoin_carry;
                }
                if let Some(accepted) = report.gate_state {
                    let mut state = g.boundary_state();
                    boundary::note_accepted_capture(
                        &mut state,
                        &boundary::CapturePlan {
                            split: false,
                            defer: false,
                            ignore_row: report.ignore_row,
                            skip_store: false,
                            clear_hint: report.clear_hint,
                            reset_streak: report.reset_streak,
                            reset_baseline: report.reset_baseline,
                            baseline_row: report.baseline_row,
                            refresh_baseline: report.refresh_baseline,
                            count_progress: report.count_progress,
                            stored_outcome: report.outcome,
                            deferred_counters: None,
                            seal: None,
                            close_reason: None,
                        },
                        accepted,
                        now,
                    );
                    g.apply_boundary_state(&state);
                    g.last_board_pressed_at = Some(pressed);
                }
                // The imported board was written with this stored capture.
                // Drop it so a later Tab that does not refresh the baseline
                // does not write it again, and so it no longer blocks this
                // session's own accolade. A deferral taken on this session
                // stays: `deferred_imported` is false there.
                if recorded && g.deferred_imported {
                    g.deferred = None;
                    g.deferred_hero = None;
                    g.deferred_at = None;
                    g.deferred_imported = false;
                }
                g.hero_auth = report.hero_auth.clone();
                if let Some(map) = report.map.clone() {
                    let agrees = g
                        .map
                        .as_ref()
                        .is_some_and(|stored| stored.eq_ignore_ascii_case(&map));
                    let untrusted = !g.map_is_trusted();
                    let incoming_trusted = report
                        .map_source
                        .is_some_and(boundary::MapSource::trusted_for_board_split);
                    if g.map.is_none() {
                        g.map = Some(map.clone());
                        g.map_source = report.map_source;
                        // No session row yet for a pending game: its first
                        // stored capture creates the row with this map.
                        if g.session_created
                            && let Err(e) = store.set_session_map(&g.session_id, &map).await
                        {
                            tracing::warn!(target: "scuffed_stat_tracker", error = %e, "failed to set session map");
                        }
                    } else if agrees && untrusted && incoming_trusted {
                        // A later top bar that names the map already stored
                        // upgrades a text fallback. The name stays.
                        g.map_source = report.map_source;
                    } else if report.map_replaces_text_fallback && untrusted && incoming_trusted {
                        // A sure banner read inside the text-fallback window
                        // renames the session (banner_replaces_text_fallback).
                        g.replaced_text_fallback_map = g.map.replace(map.clone());
                        g.map_source = report.map_source;
                        if g.session_created
                            && let Err(e) = store.set_session_map(&g.session_id, &map).await
                        {
                            tracing::warn!(target: "scuffed_stat_tracker", error = %e, "failed to replace session map");
                        }
                    }
                }
                if !g.finished() && !matches!(report.outcome, detect::MatchOutcome::Unknown) {
                    g.record_outcome_at(report.outcome, now);
                    g.note_result(report.outcome, true);
                    g.pending_boundary = false;
                    tracing::info!(
                        target: "scuffed_stat_tracker",
                        outcome = ?report.outcome,
                        session_id = %g.session_id,
                        "outcome recovered from captured frame: back-filling session"
                    );
                    if g.session_created
                        && let Err(e) = store
                            .set_session_outcome(&g.session_id, &g.outcome.to_string())
                            .await
                    {
                        tracing::warn!(target: "scuffed_stat_tracker", error = %e, "failed to back-fill session outcome");
                    }
                } else if recorded
                    && g.finished()
                    && g.session_created
                    && report.outcome != g.outcome
                {
                    // MAIN-1: this capture took its outcome when Tab was
                    // pressed. The game was decided while it ran (an end-reel
                    // poll, a GUI edit), so its row and session write carry
                    // the old one. Seal the decided result over them again.
                    tracing::info!(
                        target: "scuffed_stat_tracker",
                        stored = %report.outcome,
                        outcome = %g.outcome,
                        session_id = %g.session_id,
                        "capture finished after the result was decided: re-sealing the session"
                    );
                    if let Err(e) = store
                        .set_session_outcome(&g.session_id, &g.outcome.to_string())
                        .await
                    {
                        tracing::warn!(target: "scuffed_stat_tracker", error = %e, "failed to re-seal session outcome");
                    }
                }
                persist_active_game(data_dir, Some(g));
            }
            recorded
        }
    }
}

/// Session a stat split just opened. A decided `outcome` is the current
/// frame's own outcome (gap split) and stamps grace. Unknown is a reset split.
///
/// `career_ever_ok` is per game. It is set when this capture read the career
/// panel. A flag carried from the previous game does not. Portrait switch
/// state does not carry.
pub(crate) fn session_opened_by_split(
    session_id: String,
    outcome: detect::MatchOutcome,
    hero_auth: HeroAuthState,
    career_panel: bool,
    gate: Option<GateState>,
    map: Option<String>,
    now: Instant,
) -> ActiveGame {
    let mut g = ActiveGame::open_at(session_id, outcome, Vec::new(), now);
    g.session_created = true;
    g.map = map;
    g.gate = gate;
    g.last_stats_at = Some(now);
    g.baseline_at = gate.map(|_| now);
    g.reset_baseline = gate;
    g.hero_auth = HeroAuthState {
        career_ever_ok: career_panel,
        accepted_hero: hero_auth.accepted_hero,
        portrait_pending: None,
    };
    g
}
