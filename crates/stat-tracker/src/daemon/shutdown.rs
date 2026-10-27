//! Shutting down on a signal: the capture drain, closing the session and
//! the screencast within their stages, and the final upload.

use crate::*;

/// The most the shutdown waits for queued perf.log lines: 500 ms, or less
/// when less of the shutdown deadline is left.
pub(crate) fn shutdown_flush_wait(left: std::time::Duration) -> std::time::Duration {
    left.min(std::time::Duration::from_millis(500))
}

/// Let an in-flight capture task finish before shutdown's final sync, so its
/// snapshot is uploaded and no store write is torn by the runtime dropping it.
pub(crate) async fn drain_capture(task: Option<tokio::task::JoinHandle<()>>) {
    if let Some(t) = task
        && !t.is_finished()
    {
        tracing::info!(target: "scuffed_stat_tracker", "waiting for in-flight capture to finish");
        let _ = t.await;
    }
}

/// Close or keep the open session before the final sync.
///
/// A held board is written onto the current session only when
/// `deferred_imported` is set (it already belongs to this session). A
/// fresh-match hold belongs to the next game: it is not written onto a
/// finished session. That session's decided rows stay the ones that upload.
/// An unfinished game keeps its skeleton so a clean restart resumes it.
/// If the imported write fails, the hold stays on the skeleton. A resume
/// within 20 minutes leaves it pending. A stale start (more than 20
/// minutes, or a clock that moved backwards) archives it with the
/// persisted map and outcome.
pub(crate) async fn close_session_on_shutdown(
    st: &mut SessionState,
    store: &storage::LocalStore,
    data_dir: &std::path::Path,
) {
    let imported_write = if let Some(game) = st.active_game.as_ref()
        && game.deferred_imported
        && game.deferred.is_some()
    {
        Some(write_imported_hold(game, store, data_dir).await)
    } else {
        None
    };
    match imported_write {
        Some(Ok(())) => {
            if let Some(game) = st.active_game.as_mut() {
                clear_imported_hold(game);
            }
        }
        Some(Err(e)) => {
            tracing::warn!(target: "scuffed_stat_tracker", error = %e, "failed to store a held board on shutdown");
            persist_active_game(data_dir, st.active_game.as_ref());
            return;
        }
        None => {}
    }
    let fresh_hold = st.active_game.as_ref().and_then(|game| {
        if game.deferred_imported {
            None
        } else {
            game.deferred
                .map(|counters| (counters, game.deferred_hero.clone(), game.deferred_at))
        }
    });
    let finished = st.active_game.as_ref().is_some_and(|game| game.finished());
    if !finished {
        if let Some(game) = st.active_game.as_mut() {
            game.deferred = fresh_hold.as_ref().map(|(counters, _, _)| *counters);
        }
        persist_active_game(data_dir, st.active_game.as_ref());
        return;
    }
    if let Some(game) = st.active_game.as_mut()
        && fresh_hold.is_some()
    {
        game.deferred = None;
        game.deferred_hero = None;
        game.deferred_at = None;
        game.deferred_imported = false;
    }
    if st.active_game.is_some() {
        retire_active_game(st, store, data_dir, None, SHUTDOWN_CLOSE_REASON).await;
    }
    if let Some((counters, hero, at)) = fresh_hold {
        let mut opened = ActiveGame::open_at(
            format!("{:016x}", rand_id()),
            detect::MatchOutcome::Unknown,
            Vec::new(),
            Instant::now(),
        );
        opened.deferred = Some(counters);
        opened.deferred_hero = hero;
        opened.deferred_at = at;
        opened.deferred_imported = true;
        st.active_game = Some(opened);
        persist_active_game(data_dir, st.active_game.as_ref());
    } else {
        persist_active_game(data_dir, None);
    }
}

/// One step of [`shutdown_from_signal`]. The steps run in
/// [`SHUTDOWN_STAGES`] order, and the deadline log names the one running.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ShutdownStage {
    /// Wait for the in-flight Tab capture and apply its report.
    CaptureDrain,
    /// Close or keep the game, and write the skeleton.
    SessionSave,
    /// Close the screen share, bounded by [`SCREENCAST_CLOSE_TIMEOUT`].
    ScreencastClose,
    /// Join the in-flight sync, then the last upload.
    FinalUpload,
    /// Write the GUI snapshot.
    SnapshotExport,
}

impl ShutdownStage {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::CaptureDrain => "in-flight capture",
            Self::SessionSave => "session save",
            Self::ScreencastClose => "screencast session",
            Self::FinalUpload => "final upload",
            Self::SnapshotExport => "snapshot export",
        }
    }
}

/// Shutdown order. The game is saved right after the in-flight capture is
/// applied, before the portal close and the upload: those wait on other
/// processes and the network, and must not cost the save its share of the
/// deadline (MAIN-3). The snapshot export reads the store after the upload.
pub(crate) const SHUTDOWN_STAGES: [ShutdownStage; 5] = [
    ShutdownStage::CaptureDrain,
    ShutdownStage::SessionSave,
    ShutdownStage::ScreencastClose,
    ShutdownStage::FinalUpload,
    ShutdownStage::SnapshotExport,
];

/// A portal that does not answer gets this long. The upload keeps the rest.
pub(crate) const SCREENCAST_CLOSE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(1);

/// Deadline time the capture drain leaves for the session save. A capture
/// stuck in OCR or a screenshot fallback is abandoned at that point, and the
/// game is saved without its report.
pub(crate) const SESSION_SAVE_RESERVE: std::time::Duration = std::time::Duration::from_millis(1500);

/// Wait for the in-flight capture until only [`SESSION_SAVE_RESERVE`] of
/// `left` remains. Returns false when the capture was abandoned; its task is
/// detached, not aborted, so a store write it has started is not torn.
pub(crate) async fn drain_capture_within(
    task: Option<tokio::task::JoinHandle<()>>,
    left: std::time::Duration,
) -> bool {
    let budget = left.saturating_sub(SESSION_SAVE_RESERVE);
    if tokio::time::timeout(budget, drain_capture(task))
        .await
        .is_ok()
    {
        return true;
    }
    tracing::warn!(
        target: "scuffed_stat_tracker",
        budget_ms = budget.as_millis() as u64,
        "in-flight capture abandoned at shutdown; saving the game without it"
    );
    false
}

/// The deadline log for a shutdown stopped at `SHUTDOWN_STAGES[at]`.
pub(crate) fn shutdown_overrun_at(at: usize) -> String {
    let at = at.min(SHUTDOWN_STAGES.len() - 1);
    let not_reached: Vec<&str> = SHUTDOWN_STAGES[at + 1..]
        .iter()
        .map(|stage| stage.label())
        .collect();
    stat_tracker::replace::shutdown_overrun_line(SHUTDOWN_STAGES[at].label(), &not_reached)
}

/// Apply every capture report already sent. [`drain_capture`] waits for the
/// task; its report is in the channel, and would be lost with it: session
/// splits, the previous game's seal, the gate, a held board, the map and the
/// hero authority (MAIN-2).
pub(crate) async fn apply_landed_capture_reports(
    st: &mut SessionState,
    capture_rx: &mut CaptureReports,
    store: &storage::LocalStore,
    data_dir: &std::path::Path,
) {
    while let Ok((sid, result)) = capture_rx.try_recv() {
        // The shutdown sync that follows uploads a game this closes.
        let sid = settle_tab_probe(st, store, data_dir, sid, &result, Instant::now(), || {
            std::future::ready(())
        })
        .await;
        apply_capture_report(st, store, data_dir, &sid, result, Instant::now()).await;
    }
}

pub(crate) async fn close_screencast_on_shutdown() {
    if tokio::time::timeout(SCREENCAST_CLOSE_TIMEOUT, capture::close_if_open())
        .await
        .is_err()
    {
        tracing::warn!(
            target: "scuffed_stat_tracker",
            timeout_ms = SCREENCAST_CLOSE_TIMEOUT.as_millis() as u64,
            "screencast session did not close in time; shutdown goes on"
        );
    }
}

/// Save the game and exit within the shared deadline. Returns the step the
/// deadline stopped, or `None` when every step finished.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn shutdown_from_signal(
    st: &mut SessionState,
    capture_task: &mut Option<tokio::task::JoinHandle<()>>,
    capture_rx: &mut CaptureReports,
    sync_task: &mut Option<tokio::task::JoinHandle<()>>,
    sync_client: Option<&sync::SyncClient>,
    sync_backoff: &std::sync::Mutex<sync::SyncBackoff>,
    store: &storage::LocalStore,
    data_dir: &std::path::Path,
) -> Option<ShutdownStage> {
    let at = std::sync::atomic::AtomicUsize::new(0);
    tokio::select! {
        _ = async {
            for (i, stage) in SHUTDOWN_STAGES.iter().enumerate() {
                at.store(i, std::sync::atomic::Ordering::Relaxed);
                match stage {
                    ShutdownStage::CaptureDrain => {
                        drain_capture_within(
                            capture_task.take(),
                            stat_tracker::replace::shutdown_time_left(),
                        )
                        .await;
                        apply_landed_capture_reports(st, capture_rx, store, data_dir).await;
                    }
                    ShutdownStage::SessionSave => {
                        close_session_on_shutdown(st, store, data_dir).await;
                    }
                    ShutdownStage::ScreencastClose => close_screencast_on_shutdown().await,
                    ShutdownStage::FinalUpload => {
                        finish_sync_on_shutdown(
                            sync_client,
                            sync_backoff,
                            sync_task,
                            store,
                            data_dir,
                        )
                        .await;
                    }
                    ShutdownStage::SnapshotExport => {
                        flush_snapshot_if_dirty(store, data_dir).await;
                        // perf.log lines already queued go to disk before the
                        // process ends (the open window is not closed here).
                        // Bounded by what is left of the shutdown deadline,
                        // off the async workers.
                        let wait = shutdown_flush_wait(stat_tracker::replace::shutdown_time_left());
                        let flushed = tokio::task::spawn_blocking(move || {
                            stat_tracker::perf_log::flush(wait)
                        })
                        .await
                        .unwrap_or(false);
                        if !flushed {
                            tracing::debug!(
                                target: "scuffed_stat_tracker",
                                wait_ms = wait.as_millis() as u64,
                                "perf.log lines may be lost: the log thread did not answer in time"
                            );
                        }
                    }
                }
            }
        } => None,
        _ = tokio::time::sleep(stat_tracker::replace::shutdown_time_left()) => {
            // Returning here used to leave the runtime drop waiting on
            // blocking tasks. `main` calls `Runtime::shutdown_timeout` with
            // this same deadline, so those tasks cannot hold the exit.
            // The step is read now, not when shutdown began.
            let stopped = at
                .load(std::sync::atomic::Ordering::Relaxed)
                .min(SHUTDOWN_STAGES.len() - 1);
            tracing::error!(target: "scuffed_stat_tracker", "{}", shutdown_overrun_at(stopped));
            Some(SHUTDOWN_STAGES[stopped])
        }
    }
}

pub(crate) async fn finish_sync_on_shutdown(
    client: Option<&sync::SyncClient>,
    backoff: &std::sync::Mutex<sync::SyncBackoff>,
    task: &mut Option<tokio::task::JoinHandle<()>>,
    store: &storage::LocalStore,
    data_dir: &std::path::Path,
) {
    drain_sync_then(task, || async {
        let outcome = upload_once(client, backoff, store, data_dir, None).await;
        apply_sync_backoff(backoff, outcome);
    })
    .await;
}
