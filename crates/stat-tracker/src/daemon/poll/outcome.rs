//! What a poll tick decides: the outcome word streak and its confirm
//! window, the result read, the accolade map, and applying the decision.

use crate::*;

/// Window for two word-OCR outcome reads to confirm each other. Sized to span
/// the accolade → rank-screen transition under a starved poller (measured 45s
/// between the last accolade tick and the first rank-screen tick) while still
/// bounding how long a single stray read stays actionable.
pub(crate) const OUTCOME_CONFIRM_WINDOW: std::time::Duration = std::time::Duration::from_secs(60);

/// Apply one [`boundary::decide_poll`] result: record on the open session, or
/// close it and start the next one.
pub(crate) async fn apply_poll_decision(
    st: &mut SessionState,
    store: &storage::LocalStore,
    data_dir: &std::path::Path,
    decision: boundary::PollDecision,
    now: Instant,
    use_new_reader: bool,
) {
    if let boundary::PollDecision::IgnoreContradictory { kept, ignored } = &decision {
        tracing::info!(
            target: "scuffed_stat_tracker",
            kept = %kept,
            ignored = %ignored,
            session_id = st.active_game.as_ref().map(|g| g.session_id.as_str()),
            "result word ignored: it does not override the open session"
        );
    }
    let Some(current) = st.active_game.as_ref() else {
        return;
    };
    let mut state = current.boundary_state();
    let before = state.clone();
    let carried_deferred = current
        .deferred
        .map(|counters| (counters, current.deferred_hero.clone(), current.deferred_at));
    let pending_boundary = current.pending_boundary;
    let (candidates, end_screen) = match &decision {
        boundary::PollDecision::Open(open)
            if open.reason == boundary::CloseReason::EndScreenMap =>
        {
            (open.candidates.clone(), Some(open.clone()))
        }
        boundary::PollDecision::Open(open) => (open.candidates.clone(), None),
        _ => (Vec::new(), None),
    };
    let commit = boundary::commit_poll(&mut state, decision, now);
    if let Some(closed) = commit.closed {
        let new_id = format!("{:016x}", rand_id());
        match closed.reason {
            boundary::CloseReason::MapVote => {
                tracing::info!(
                    target: "scuffed_stat_tracker",
                    ?candidates,
                    session_id = %new_id,
                    "auto-detect: map vote: new game"
                );
            }
            boundary::CloseReason::HeroSelectOrBan => {
                tracing::info!(
                    target: "scuffed_stat_tracker",
                    session_id = %new_id,
                    "auto-detect: hero select/ban: new game (map vote missed)"
                );
            }
            boundary::CloseReason::EndScreenMap => {
                let split =
                    end_screen_split_log(pending_boundary, end_screen.as_ref(), closed.seal);
                remember_end_screen_split(&split);
                tracing::info!(
                    target: "scuffed_stat_tracker",
                    reason = closed.reason.log(),
                    session_id = %new_id,
                    split = %split,
                    "auto-detect: new game boundary"
                );
            }
            other => {
                tracing::info!(target: "scuffed_stat_tracker", reason = other.log(), session_id = %new_id, "auto-detect: new game boundary");
            }
        }
        retire_active_game(st, store, data_dir, closed.seal, closed.reason.log()).await;
        let mut g = ActiveGame::open_at(new_id, state.outcome, candidates, now);
        g.apply_boundary_state(&state);
        // A different-map end screen is an accolade read. That is the new
        // session's map, and it is trusted for a later board-case split.
        if g.map.is_some() {
            g.map_source = Some(boundary::MapSource::Accolade);
        }
        // The deferred board belongs to the session being opened. The fresh
        // state has none; put it back so the next stored capture writes it
        // once. That capture clears the hold.
        if let Some((counters, hero, at)) = carried_deferred {
            g.deferred = Some(counters);
            g.deferred_hero = hero;
            g.deferred_at = at;
            g.deferred_imported = true;
        }
        st.active_game = Some(g);
        st.last_game_open = Some(now);
        clear_cadence_wakes(st);
        st.pending_outcome = None;
        persist_active_game(data_dir, st.active_game.as_ref());
        return;
    }
    let Some(g) = st.active_game.as_mut() else {
        return;
    };
    g.apply_boundary_state(&state);
    if let Some(outcome) = commit.recorded_outcome {
        g.touch_at(now);
        tracing::info!(
            target: "scuffed_stat_tracker",
            ?outcome,
            session_id = %g.session_id,
            "auto-detect: outcome confirmed from post-match screens"
        );
        if g.session_created {
            if let Err(e) = store
                .set_session_outcome(&g.session_id, &g.outcome.to_string())
                .await
            {
                tracing::warn!(target: "scuffed_stat_tracker", error = %e, "failed to back-fill session outcome");
            }
            refresh_snapshot_game_finished(store, data_dir).await;
        }
    }
    if let Some(map) = commit.adopted_map {
        if !g.map_candidates.is_empty() && !g.map_candidates.contains(&map) {
            tracing::warn!(
                target: "scuffed_stat_tracker",
                map = %map,
                candidates = ?g.map_candidates,
                "accolade map is not a vote candidate"
            );
        }
        g.map_source = Some(boundary::MapSource::Accolade);
        tracing::info!(
            target: "scuffed_stat_tracker",
            map = %map,
            session_id = %g.session_id,
            "map recovered from accolade screen"
        );
        g.touch_at(now);
        if g.session_created {
            // `reader = "new"`: a member's own map choice is never renamed,
            // and the mode stored with it is never rewritten (the top-bar
            // rule of F252-3, for the accolade). That is a rule for the
            // rows that carry the choice, not for the session (F265-1): a
            // choice is stamped on the rows that exist when it is made, and
            // the rows captured after it, the game's newest row among them,
            // take the accolade map like any other row. With ocr-v1 every
            // row is written as before.
            let written = if use_new_reader {
                store
                    .set_session_map_keeping_member_choices(&g.session_id, &map)
                    .await
            } else {
                store.set_session_map(&g.session_id, &map).await.map(|()| 0)
            };
            match written {
                Ok(0) => {}
                Ok(kept) => tracing::info!(
                    target: "scuffed_stat_tracker",
                    session_id = %g.session_id,
                    kept,
                    "accolade map not written to the rows whose map a member chose"
                ),
                Err(e) => {
                    tracing::warn!(target: "scuffed_stat_tracker", error = %e, "failed to set session map")
                }
            }
            refresh_snapshot(store, data_dir).await;
        }
    }
    if state != before {
        persist_active_game(data_dir, Some(g));
    }
}

/// Record this result-word tick and return the map the decision should see.
///
/// A tick with no map keeps the map from the previous read of the same
/// outcome, and only while that read is still inside
/// [`OUTCOME_CONFIRM_WINDOW`]. A different outcome, or an older read, drops
/// the carried map so it cannot relabel the session.
pub(crate) fn note_word_streak(
    streak: &mut Option<WordStreak>,
    outcome: detect::MatchOutcome,
    seen_at: Instant,
    this_map: Option<String>,
) -> Option<String> {
    let carried = streak.as_ref().and_then(|prior| {
        let age = seen_at.saturating_duration_since(prior.seen_at);
        (prior.outcome == outcome && age <= OUTCOME_CONFIRM_WINDOW)
            .then(|| prior.map.clone())
            .flatten()
    });
    let map = this_map.or(carried);
    *streak = Some(WordStreak {
        outcome,
        seen_at,
        map: map.clone(),
    });
    map
}

/// What one poll tick decided about the result word, before the session
/// machine runs. Production and the night harness both call this, so the
/// carried map and the confirm rule cannot drift.
pub(crate) struct ResolvedWordTick {
    pub(crate) signal: Option<detect::MatchOutcome>,
    pub(crate) signal_confirmed: bool,
    pub(crate) accolade_map: Option<String>,
}

pub(crate) fn resolve_word_tick(
    streak: &mut Option<WordStreak>,
    signal: Option<(detect::MatchOutcome, detect::match_end::OutcomeSource)>,
    seen_at: Instant,
    this_map: Option<String>,
) -> ResolvedWordTick {
    let prior = streak.as_ref().map(|prior| {
        (
            prior.outcome,
            seen_at.saturating_duration_since(prior.seen_at),
        )
    });
    let hit = poll_debug_hit(signal, prior, OUTCOME_CONFIRM_WINDOW);
    match signal {
        Some((outcome, source)) => {
            let agreed = matches!(hit, Some((PollDebugHit::Confirm, _)));
            let accolade_map = note_word_streak(streak, outcome, seen_at, this_map);
            if !agreed {
                tracing::debug!(
                    target: "scuffed_stat_tracker",
                    ?outcome,
                    ?source,
                    "result word read: awaiting agreeing read"
                );
            }
            ResolvedWordTick {
                signal: Some(outcome),
                signal_confirmed: agreed,
                accolade_map,
            }
        }
        None => ResolvedWordTick {
            signal: None,
            signal_confirmed: false,
            accolade_map: this_map,
        },
    }
}

/// The words-pack result read one poll tick adds to the open game's result
/// evidence (`reader = "new"`, #253). Only a tick with an outcome signal
/// adds one, as before.
///
/// `taken` is the read the tick's signal detection already made of this
/// frame's result words (the accolade or rank screen, alpha.7 O11). It is
/// `shadow::read_result`'s read of the same frame, so it is the evidence
/// and the frame is not read again. Without one (an end title, or a word
/// OCR read because the pack lacks that screen) `read`
/// reads the frame here.
pub(crate) fn tick_result_read(
    use_new_reader: bool,
    has_signal: bool,
    taken: Option<shadow::result::ResultRead>,
    read: impl FnOnce() -> shadow::result::ResultRead,
) -> Option<shadow::result::ResultRead> {
    if !(use_new_reader && has_signal) {
        return None;
    }
    Some(taken.unwrap_or_else(read))
}

/// The [`boundary::PollInput`] for one open session. `map_trusted` is the
/// stored source, so a text fallback stays absent on the poll path.
pub(crate) fn poll_input_from_game<'a>(
    game: &'a ActiveGame,
    signal: Option<detect::MatchOutcome>,
    signal_confirmed: bool,
    accolade_map: Option<&'a str>,
    start_screen: Option<boundary::StartScreen>,
    block_map_vote: bool,
    now: Instant,
) -> boundary::PollInput<'a> {
    boundary::PollInput {
        outcome: game.outcome,
        result: game.result_mark,
        pending_boundary: game.pending_boundary,
        awaiting_first_board: game.awaiting_first_board,
        has_board: game.gate.is_some(),
        reset_streak: game.reset_streak,
        map: game.map.as_deref(),
        map_trusted: game.map_is_trusted(),
        hero: game.hero_auth.accepted_hero.as_deref(),
        signal,
        signal_confirmed,
        accolade_map: accolade_map.map(|read| {
            // The replaced text-fallback name is this session's own map
            // (F252-1).
            match (
                game.replaced_text_fallback_map.as_deref(),
                game.map.as_deref(),
            ) {
                (Some(old), Some(map)) if read.trim().eq_ignore_ascii_case(old.trim()) => map,
                _ => read,
            }
        }),
        start_screen,
        block_map_vote,
        deferred: game.deferred.is_some(),
        text_fallback_locked: game.text_fallback_locked,
        now,
    }
}

/// One poll tick after the word is resolved. [`run_loop`] and the night
/// harness both call this, so the post-tick glue cannot drift: the
/// start-screen block, [`poll_input_from_game`] (including the carried
/// accolade map), [`apply_poll_decision`], a confirmed word with no game
/// open, and [`open_detected_game`].
#[allow(clippy::too_many_arguments)]
pub(crate) async fn apply_poll_tick(
    st: &mut SessionState,
    store: &storage::LocalStore,
    data_dir: &std::path::Path,
    tick: &ResolvedWordTick,
    start_screen: Option<boundary::StartScreen>,
    now: Instant,
    debounce: std::time::Duration,
    use_new_reader: bool,
) {
    let block_map_vote = start_screen.as_ref().is_some_and(|screen| {
        start_screen_blocked(
            st.active_game.as_ref(),
            st.last_game_open,
            now,
            debounce,
            screen,
        )
    });
    let decision = st.active_game.as_ref().map(|g| {
        boundary::decide_poll(&poll_input_from_game(
            g,
            tick.signal,
            tick.signal_confirmed,
            tick.accolade_map.as_deref(),
            start_screen.clone(),
            block_map_vote,
            now,
        ))
    });
    let opened = matches!(decision, Some(boundary::PollDecision::Open(_)));
    if let Some(decision) = decision {
        apply_poll_decision(st, store, data_dir, decision, now, use_new_reader).await;
    } else if tick.signal_confirmed
        && let Some(outcome) = tick.signal
    {
        // No game open yet, applies to the next session if one opens
        // within the TTL.
        st.pending_outcome = Some((outcome, now));
    }
    // An open session is only closed by `decide_poll`. This arm starts a
    // session when nothing is active yet.
    if !opened
        && st.active_game.is_none()
        && let Some(screen) = &start_screen
    {
        open_detected_game(st, data_dir, screen, now, debounce);
    }
}

/// Under `reader = "new"`, the accolade map was read with OCR because no
/// map pack is loaded (none synced yet, or none that loads). Logged once
/// per process (F265-3), like the FINAL SCORE line's fallback. With OCR off
/// there is no such fallback and no such line ([`choose_accolade_map`]).
/// Returns whether this call logged (tests).
pub(crate) fn warn_accolade_map_ocr_fallback() -> bool {
    static WARNED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    let first = !WARNED.swap(true, std::sync::atomic::Ordering::Relaxed);
    if first {
        tracing::warn!(
            target: "scuffed_stat_tracker",
            "reader = \"new\" but no map pack is loaded; \
             the accolade map is read with OCR"
        );
    }
    first
}

/// The map on this poll tick's accolade screen, for the session machine.
///
/// Only a result-word tick reads the map. The rank screen does not, and a
/// banner tick is the gameplay HUD under a result flash, so that crop is not
/// a map. A tick taken while Tab cell OCR is in flight (`outcome_only`) reads
/// no map either.
///
/// With `reader = "ocr-v1"` the map is the title OCR of the strip, as
/// before. With `reader = "new"` it is the map pack's read of the accolade
/// map line ([`shadow::read_accolade_map`]): sure and storable, or nothing.
/// When the pack read the frame and abstains there is no map; the OCR is
/// not run in its place, and a panic in the pack reader is an abstain too,
/// so it never costs the tick its result word. When no map pack is loaded
/// nothing read the frame, and the OCR reads it as under ocr-v1, with one
/// warning ([`warn_accolade_map_ocr_fallback`]): the rule of the top bar,
/// the FINAL SCORE line and the phase titles without their packs. With
/// classic OCR off there is no OCR to read it: no map pack means no map,
/// and the OCR read is not asked for (the switch does not grant off
/// without the map pack, so this is a pack that went away). What the
/// session does with the map is the same for both readers
/// (`boundary::decide_poll`).
pub(crate) fn choose_accolade_map(
    use_new_reader: bool,
    outcome_only: bool,
    signal: Option<detect::match_end::OutcomeSource>,
    pack: impl FnOnce() -> shadow::AccoladeRead,
    ocr: impl FnOnce() -> Option<String>,
) -> Option<String> {
    if outcome_only || signal != Some(detect::match_end::OutcomeSource::ResultWord) {
        return None;
    }
    if !use_new_reader {
        return ocr();
    }
    let read = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(pack)) {
        Ok(shadow::AccoladeRead::Map(read)) => read,
        Ok(shadow::AccoladeRead::Abstain) => return None,
        // No map pack, and classic OCR is off: held. `ocr` is not called.
        Ok(shadow::AccoladeRead::NoPack) if ocr::gate::is_off() => return None,
        Ok(shadow::AccoladeRead::NoPack) => {
            warn_accolade_map_ocr_fallback();
            return ocr();
        }
        Err(_) => {
            tracing::warn!(target: "scuffed_stat_tracker", "accolade map reader panicked; no accolade map for this tick");
            return None;
        }
    };
    tracing::info!(
        target: "scuffed_stat_tracker",
        map = %read.name,
        score = read.score,
        margin = read.margin,
        recognizer = shadow::banner::ACCOLADE_MAP_RECOGNIZER_ID,
        "map read from accolade screen (map pack)"
    );
    Some(read.name)
}
