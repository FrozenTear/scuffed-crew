//! Asking for a Tab capture: the request, the probe that settles a Tab
//! against the open game, unlatching a false outcome, and opening the game
//! a Tab starts.

use crate::*;

/// The Tab request the capture task runs. Ages are measured from `now`,
/// which is [`Instant::now`] in the daemon and the injected clock in tests.
pub(crate) fn build_capture_request(
    g: &ActiveGame,
    opened_by_this_tab: bool,
    now: Instant,
) -> CaptureRequest {
    let banner_ok =
        opened_by_this_tab || now.saturating_duration_since(g.opened_at) >= MIN_BANNER_SESSION_AGE;
    // This session's own gate. A start screen's first board has none, and
    // the previous game is not an anchor for it.
    let prev_gate = match (g.gate, g.last_stats_at) {
        (Some(state), Some(at)) => Some((state, now.saturating_duration_since(at))),
        _ => None,
    };
    CaptureRequest {
        tab_at: stat_tracker::report::blank::CaptureTime {
            wall: Utc::now(),
            mono: now,
        },
        session_id: g.session_id.clone(),
        create_session: !g.session_created,
        game_outcome: g.outcome,
        session_map: g.map.clone(),
        session_map_source: g.map_source,
        map_candidates: g.map_candidates.clone(),
        allow_banner_recovery: banner_ok,
        prev_gate,
        hero_auth: g.hero_auth.clone(),
        after_end_screen: g.has_post_result(),
        baseline_age: g.baseline_at.map(|at| now.saturating_duration_since(at)),
        progressed_boards: g.progressed_boards,
        deferred_imported: g.deferred_imported,
        reset_streak: g.reset_streak,
        reset_baseline: g.reset_baseline,
        baseline_row: g.baseline_row,
        hint: g
            .result_mark
            .filter(|mark| !mark.confirmed && mark.outcome.is_decided())
            .map(|mark| mark.outcome),
        pending_boundary: g.pending_boundary,
        awaiting_first_board: g.awaiting_first_board,
        text_fallback_locked: g.text_fallback_locked,
        member_map_edit: false,
        carried_deferred: g.deferred,
        carried_deferred_hero: g.deferred_hero.clone(),
        carried_deferred_at: g.deferred_at,
        result_evidence: g.result_evidence.clone(),
        rejoin_window_open: g.rejoin_carry.is_none()
            && !g.finished()
            && g.result_mark.is_none()
            && g.restart_recovered_at
                .is_some_and(|at| now.saturating_duration_since(at) <= REJOIN_CONTINUE_WINDOW),
        rejoin_carry: g.rejoin_carry,
        finished_game_probe: None,
        last_board_after_result: g
            .last_board_pressed_at
            .zip(g.outcome_recorded_at)
            .is_some_and(|(pressed, result)| pressed > result),
    }
}

/// Whether this Tab closes a game that has a result and stored rows. That
/// game is then uploaded right away ([`open_fresh_for_tab_and_sync`]).
pub(crate) fn tab_closes_a_recorded_game(game: Option<&ActiveGame>, now: Instant) -> bool {
    game.is_some_and(|g| g.finished() && g.session_created) && should_start_fresh_session(game, now)
}

/// [`open_fresh_if_tab_starts_one`], then `sync_now` when the game this Tab
/// closed had a result and stored rows. Without this the closed game waited
/// for the next game's periodic sync, and the quiet-close timer only looks
/// at the open game, so a late Tab off the scoreboard left the last game of
/// the night unsent for hours (2026-10-17, Oasis). `sync_now` schedules
/// [`schedule_store_sync`]; it does not block the loop.
pub(crate) async fn open_fresh_for_tab_and_sync<F, Fut>(
    st: &mut SessionState,
    store: &storage::LocalStore,
    data_dir: &std::path::Path,
    now: Instant,
    sync_now: F,
) -> bool
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let upload_closed = tab_closes_a_recorded_game(st.active_game.as_ref(), now);
    let opened = open_fresh_if_tab_starts_one(st, store, data_dir, now).await;
    if opened && upload_closed {
        sync_now().await;
    }
    opened
}

/// The capture request for the Tab pressed at `press`: the session choice of
/// [`open_fresh_for_tab_and_sync`], or past the grace window of a finished
/// game that its board may continue ([`finished_game_tab_probe`]), the fresh
/// session's request with the finished game's request inside. Nothing is
/// closed or opened then; [`settle_tab_probe`] does that when the report
/// lands.
pub(crate) async fn tab_capture_request<F, Fut>(
    st: &mut SessionState,
    store: &storage::LocalStore,
    data_dir: &std::path::Path,
    press: Instant,
    sync_now: F,
) -> CaptureRequest
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    st.tab_press = Some(press);
    if finished_game_tab_probe(st.active_game.as_ref(), press) {
        let old = st.active_game.as_ref().expect("checked by the probe");
        let fresh_session = format!("{:016x}", rand_id());
        // Peeked, not taken: the pending outcome is taken when the fresh
        // session opens.
        let mut pending = st.pending_outcome;
        let fresh = tab_opened_game(
            fresh_session.clone(),
            take_fresh_pending(&mut pending, press),
            carried_deferred(Some(old)),
            press,
        );
        let mut req = build_capture_request(&fresh, true, press);
        req.finished_game_probe = Some(Box::new(build_capture_request(old, false, press)));
        tracing::info!(
            target: "scuffed_stat_tracker",
            session_id = %old.session_id,
            outcome = %old.outcome,
            "Tab past the post-match grace window: the board decides whether this is the same game"
        );
        st.tab_probe = Some(TabProbe {
            old_session: old.session_id.clone(),
            fresh_session,
            press,
        });
        return req;
    }
    let opened_by_this_tab =
        open_fresh_for_tab_and_sync(st, store, data_dir, press, sync_now).await;
    build_capture_request(
        st.active_game.as_ref().expect("active_game set above"),
        opened_by_this_tab,
        press,
    )
}

/// The capture of a [`TabProbe`] Tab reported (`sid` is the request's
/// session, the fresh one). Returns the session the report is for.
///
/// * The board was the finished game's: that game stays open. The same
///   board ([`boundary::FinishedGameTab::SameBoard`]) is not stored and the
///   result stays. Still being played
///   ([`boundary::FinishedGameTab::StillPlaying`]), the capture stored the
///   board there, and the recorded result was false and is cleared
///   ([`unlatch_false_outcome`]).
/// * Anything else (another game's board, a board that cannot be compared,
///   a rejected or missed frame, a failed capture): the fresh session opens
///   as of the press and the finished game closes, as a Tab past the grace
///   window did before rc.9, with the upload of a closed game that had rows
///   ([`open_fresh_for_tab_and_sync`]). Not when another game took over
///   while the capture ran.
pub(crate) async fn settle_tab_probe<F, Fut>(
    st: &mut SessionState,
    store: &storage::LocalStore,
    data_dir: &std::path::Path,
    sid: String,
    result: &Result<CaptureReport, String>,
    now: Instant,
    sync_now: F,
) -> String
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let Some(probe) = st.tab_probe.take_if(|probe| probe.fresh_session == sid) else {
        return sid;
    };
    let still_open = st
        .active_game
        .as_ref()
        .is_some_and(|g| g.session_id == probe.old_session);
    match result
        .as_ref()
        .ok()
        .and_then(|report| report.finished_game_tab)
    {
        Some(boundary::FinishedGameTab::SameBoard) => {
            tracing::info!(
                target: "scuffed_stat_tracker",
                session_id = %probe.old_session,
                "Tab past the grace window shows the same board: same game, result kept"
            );
            probe.old_session
        }
        Some(boundary::FinishedGameTab::StillPlaying) => {
            // A result the member set while this capture ran stays (the
            // report re-seals it on the row).
            let unlocked = st.active_game.as_ref().is_some_and(|g| !g.outcome_locked);
            if still_open && unlocked {
                unlatch_false_outcome(st, store, data_dir, now).await;
            }
            probe.old_session
        }
        Some(boundary::FinishedGameTab::NewGame) | None => {
            if still_open || st.active_game.is_none() {
                let upload_closed = st
                    .active_game
                    .as_ref()
                    .is_some_and(|g| g.finished() && g.session_created);
                open_tab_game(st, store, data_dir, probe.fresh_session, probe.press).await;
                if upload_closed {
                    sync_now().await;
                }
            } else {
                tracing::warn!(
                    target: "scuffed_stat_tracker",
                    session_id = %sid,
                    "another game opened while this Tab was read: its report is not applied"
                );
            }
            sid
        }
    }
}

/// R9-2: the open game's board moved on past the grace window
/// ([`boundary::FinishedGameTab::StillPlaying`]), so its recorded result was
/// false (0.5.0-rc.8 report of 2026-10-22: DEFEAT taken off a red mid-match
/// frame). The result is cleared on the game and in the store, with the
/// result words read for this game (`reader = "new"` stamps them on every
/// later row otherwise), and the next confirmed result word sets it.
///
/// Result words read while the false result was held are not replayed:
/// those ticks were ignored (the game was past its end), and their reads
/// go with the cleared evidence. A real end screen already past is then not
/// read again, and the game ends without a result unless the member sets
/// one. A word streak from at or before the cleared result is dropped, so
/// a repeating misread needs two new reads again; a newer read (an end
/// screen seen while the false result was held) still pairs with the next
/// one inside the 60 s window.
///
/// Rows already uploaded with the false result keep it on the site:
/// `unknown` rows are not uploaded (`storage::LocalStore::get_unsynced`
/// selects `outcome != 'unknown'`, and the site skips them), so the site
/// is corrected only when a result is set on this game again.
pub(crate) async fn unlatch_false_outcome(
    st: &mut SessionState,
    store: &storage::LocalStore,
    data_dir: &std::path::Path,
    now: Instant,
) {
    let Some(g) = st.active_game.as_mut() else {
        return;
    };
    if let Some(recorded) = g.outcome_recorded_at
        && st
            .word_outcome_streak
            .as_ref()
            .is_some_and(|streak| streak.seen_at <= recorded)
    {
        st.word_outcome_streak = None;
    }
    let cleared = g.outcome;
    g.outcome = detect::MatchOutcome::Unknown;
    g.outcome_recorded_at = None;
    g.result_mark = None;
    // Cleared with the hint ([`ActiveGame::text_fallback_locked`]).
    g.text_fallback_locked = false;
    g.result_evidence = Default::default();
    g.touch_at(now);
    tracing::warn!(
        target: "scuffed_stat_tracker",
        session_id = %g.session_id,
        cleared = %cleared,
        "the board moved on after the result was recorded: the game is still being played, result cleared \
         (if its rows were already uploaded, the site keeps the old result until one is set again)"
    );
    if g.session_created
        && let Err(e) = store
            .set_session_outcome(&g.session_id, &detect::MatchOutcome::Unknown.to_string())
            .await
    {
        tracing::warn!(target: "scuffed_stat_tracker", error = %e, "failed to clear the session outcome");
    }
    persist_active_game(data_dir, Some(g));
    refresh_snapshot(store, data_dir).await;
}

/// Open a session when this Tab is past grace or the unfinished idle bound.
/// Returns whether this Tab opened it. The deferred board of the session
/// being closed is carried onto the new one.
pub(crate) async fn open_fresh_if_tab_starts_one(
    st: &mut SessionState,
    store: &storage::LocalStore,
    data_dir: &std::path::Path,
    now: Instant,
) -> bool {
    if !should_start_fresh_session(st.active_game.as_ref(), now) {
        return false;
    }
    open_tab_game(st, store, data_dir, format!("{:016x}", rand_id()), now).await;
    true
}

pub(crate) fn carried_deferred(previous: Option<&ActiveGame>) -> Option<CarriedDeferred> {
    previous.and_then(|g| {
        g.deferred
            .map(|counters| (counters, g.deferred_hero.clone(), g.deferred_at))
    })
}

/// The game a Tab opens as `session_id` at `now`: the pending outcome
/// `inherited`, and the deferred board of the game it replaces.
pub(crate) fn tab_opened_game(
    session_id: String,
    inherited: Option<detect::MatchOutcome>,
    carried: Option<CarriedDeferred>,
    now: Instant,
) -> ActiveGame {
    let mut opened = ActiveGame::open_at(
        session_id,
        inherited.unwrap_or(detect::MatchOutcome::Unknown),
        Vec::new(),
        now,
    );
    if let Some((counters, hero, at)) = carried {
        opened.deferred = Some(counters);
        opened.deferred_hero = hero;
        opened.deferred_at = at;
        opened.deferred_imported = true;
    }
    opened
}

/// Close the open game and open the Tab's game `session_id` at `now`.
pub(crate) async fn open_tab_game(
    st: &mut SessionState,
    store: &storage::LocalStore,
    data_dir: &std::path::Path,
    session_id: String,
    now: Instant,
) {
    let carried = carried_deferred(st.active_game.as_ref());
    retire_active_game(st, store, data_dir, None, "superseded by Tab-opened game").await;
    let inherited = take_fresh_pending(&mut st.pending_outcome, now);
    st.active_game = Some(tab_opened_game(session_id, inherited, carried, now));
    st.last_game_open = Some(now);
    clear_cadence_wakes(st);
    persist_active_game(data_dir, st.active_game.as_ref());
}

/// How long direct capture and the screenshot portal wait after the Tab
/// press for the game to draw the scoreboard. The screen stream waits for a
/// buffer instead.
pub(crate) const DIRECT_TAB_DRAW_WAIT: std::time::Duration = std::time::Duration::from_millis(400);

/// Start the Tab capture for `press` (the key-event instant): pick or open
/// the session, then spawn the single-flight capture task, which reports on
/// `capture_tx`. Returns the task and its shared flight state.
pub(crate) async fn start_tab_capture(
    st: &mut SessionState,
    ctx: &Arc<DaemonCtx>,
    sync_task: &mut Option<tokio::task::JoinHandle<()>>,
    sync_backoff: &Arc<std::sync::Mutex<sync::SyncBackoff>>,
    capture_tx: &tokio::sync::mpsc::UnboundedSender<(String, Result<CaptureReport, String>)>,
    press: Instant,
    stream_hold: capture::KeyHold,
) -> (
    tokio::task::JoinHandle<()>,
    Arc<stat_tracker::tab_retry::TabFlight>,
) {
    let store = &ctx.store;
    let data_dir: &std::path::Path = &ctx.data_dir;
    let sync_client = ctx.sync_client.as_ref();
    // Session choice: reuse the active game (mid-game, or post-match
    // scoreboard within the grace window), or open a fresh one, inheriting
    // a still-fresh outcome the poller saw before any game was open. Past
    // the grace window of a finished game, the board may decide (R9-2).
    let req = tab_capture_request(st, store, data_dir, press, || {
        schedule_store_sync(sync_task, sync_backoff, store, sync_client, data_dir, None);
        std::future::ready(())
    })
    .await;
    let on_stream = capture::stream_route_active();
    let sid = req.session_id.clone();
    let tx = capture_tx.clone();
    let ctx_task = Arc::clone(ctx);
    let flight = stat_tracker::tab_retry::TabFlight::new(press);
    let task = tokio::spawn(stat_tracker::tab_retry::with_flight(
        Arc::clone(&flight),
        capture::with_tab_press(press, async move {
            // Direct capture and the screenshot portal wait for the overlay
            // to finish drawing, counted from the key event. The screen
            // stream waits for a buffer instead.
            if !on_stream {
                tokio::time::sleep_until(tokio::time::Instant::from_std(
                    press + DIRECT_TAB_DRAW_WAIT,
                ))
                .await;
            }
            // Keeps a paused screen stream running from the key event until
            // the Tab retry sequence ends (`capture::end_tab_stream_hold`).
            let result = capture::with_tab_stream_hold(stream_hold, handle_capture(&ctx_task, req))
                .await
                // `{:#}` keeps anyhow's context chain in the string.
                .map_err(|e| format!("{e:#}"));
            let _ = tx.send((sid, result));
        }),
    ));
    (task, flight)
}
