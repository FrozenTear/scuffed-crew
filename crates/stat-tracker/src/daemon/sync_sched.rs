//! Upload scheduling: the single-flight sync task, its periodic and idle
//! triggers, backoff and paused credentials, local-only rows, batched
//! uploads and tombstones.

use crate::*;

pub(crate) const SYNC_EVERY_N_CAPTURES: u32 = 5;

/// How often the command tick looks for unsent rows or tombstones and
/// starts a sync on its own (DATA-2), so an upload that failed after the
/// evening's last game is retried without a Tab or a restart.
pub(crate) const IDLE_SYNC_EVERY: std::time::Duration = std::time::Duration::from_secs(60);

/// How many shutdown drains are blocked inside the in-flight sync join.
/// Tests wait on this so "final upload started early" cannot pass by winning
/// a race against a task that has not been scheduled yet.
#[cfg(test)]
pub(crate) static SYNC_DRAIN_WAITING: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

/// Finish `task` (including its `mark_synced`) before `then`. Shutdown must
/// not start a second upload while the first still holds ids from its read:
/// both would mark those ids, and the older HTTP body can land on the server
/// after the newer one. Aborting the task would drop it between the response
/// and `mark_synced`, so the join waits instead. The handle stays in `task`
/// until the join is done: a deadline that drops this future leaves it there
/// for [`finish_or_abort_sync`].
pub(crate) async fn drain_sync_then<F, Fut>(task: &mut Option<tokio::task::JoinHandle<()>>, then: F)
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future,
{
    if let Some(t) = task.as_mut() {
        if !t.is_finished() {
            tracing::info!(target: "scuffed_stat_tracker", "waiting for in-flight sync to finish before final upload");
        }
        #[cfg(test)]
        SYNC_DRAIN_WAITING.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let _ = t.await;
        #[cfg(test)]
        SYNC_DRAIN_WAITING.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        *task = None;
    }
    then().await;
}

pub(crate) async fn try_sync(
    store: &storage::LocalStore,
    client: &sync::SyncClient,
    data_dir: &std::path::Path,
    settle_session: Option<String>,
) -> sync::SyncAttempt {
    let mut caps = client.batch_caps();
    let outcome = try_sync_with_caps(store, data_dir, settle_session, &mut caps, {
        let client = client.clone();
        move |matches, tombstones| {
            let client = client.clone();
            async move { client.upload_matches(&matches, &tombstones).await }
        }
    })
    .await;
    client.remember_batch_caps(caps);
    let creds = client.credentials();
    if outcome == sync::SyncAttempt::StatsDeletePending {
        let scheduled_at = client.take_delete_scheduled_at();
        if let Err(e) = sync::write_delete_pending_pause(
            data_dir,
            &creds.server_url,
            &creds.token,
            scheduled_at.as_deref(),
        ) {
            tracing::warn!(target: "scuffed_stat_tracker", error = %e, "could not record the stats-delete pause");
        }
    }
    let pause = match outcome {
        sync::SyncAttempt::AuthRejected => Some(sync::AuthPauseReason::Rejected),
        sync::SyncAttempt::TokenExpired => Some(sync::AuthPauseReason::Expired),
        sync::SyncAttempt::StatsDeletePending => None,
        sync::SyncAttempt::Uploaded => {
            if sync::auth_pause_reason(data_dir, &creds.server_url, &creds.token)
                == Some(sync::AuthPauseReason::DeletePending)
            {
                sync::clear_auth_pause(data_dir);
            }
            None
        }
        _ => None,
    };
    if let Some(reason) = pause
        && let Err(e) =
            sync::write_auth_pause_reason(data_dir, &creds.server_url, &creds.token, reason)
    {
        tracing::warn!(target: "scuffed_stat_tracker", error = %e, "could not record sync auth pause");
    }
    outcome
}

/// True when this capture should start a periodic sync task.
///
/// Single-flight (`task_in_flight`) and the backoff window are both gates.
/// Shutdown does not use this predicate, it joins the in-flight task and
/// uploads once regardless of the backoff clock.
pub(crate) fn periodic_sync_due(
    capture_count: u32,
    task_in_flight: bool,
    backoff_allows: bool,
) -> bool {
    capture_count.is_multiple_of(SYNC_EVERY_N_CAPTURES) && !task_in_flight && backoff_allows
}

/// True when the idle tick should start a sync (DATA-2): at least
/// [`IDLE_SYNC_EVERY`] since the last idle check, no sync in flight, the
/// backoff window open, and something left to send.
pub(crate) fn idle_sync_due(
    since_last_check: std::time::Duration,
    task_in_flight: bool,
    backoff_allows: bool,
    has_pending: bool,
) -> bool {
    since_last_check >= IDLE_SYNC_EVERY && !task_in_flight && backoff_allows && has_pending
}

/// The idle retry on the command tick. Without it an upload started only on
/// every fifth Tab, a game close, a GUI command, startup, or shutdown, so a
/// game whose upload failed while the site was down waited for the next
/// session. Single-flight: a sync already running is never doubled.
pub(crate) async fn maybe_spawn_idle_sync(
    sync_task: &mut Option<tokio::task::JoinHandle<()>>,
    backoff: &Arc<std::sync::Mutex<sync::SyncBackoff>>,
    store: &storage::LocalStore,
    client: Option<&sync::SyncClient>,
    data_dir: &std::path::Path,
    last_check: &mut Instant,
    now: Instant,
) {
    if client.is_none() || now.saturating_duration_since(*last_check) < IDLE_SYNC_EVERY {
        return;
    }
    let since = now.saturating_duration_since(*last_check);
    *last_check = now;
    let in_flight = sync_task.as_ref().is_some_and(|t| !t.is_finished());
    let allow = backoff
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .should_attempt(now);
    if in_flight || !allow {
        return;
    }
    let has_pending = match (
        store.get_unsynced().await,
        store.get_pending_tombstones().await,
    ) {
        (Ok(rows), Ok(tombstones)) => !rows.is_empty() || !tombstones.is_empty(),
        // A failed local read leaves the decision to try_sync, which logs it.
        _ => true,
    };
    if idle_sync_due(since, in_flight, allow, has_pending) {
        tracing::debug!(target: "scuffed_stat_tracker", "idle sync: unsent rows or deletes are waiting");
        schedule_store_sync(sync_task, backoff, store, client, data_dir, None);
    }
}

pub(crate) fn maybe_spawn_periodic_sync(
    sync_task: &mut Option<tokio::task::JoinHandle<()>>,
    backoff: &Arc<std::sync::Mutex<sync::SyncBackoff>>,
    store: &storage::LocalStore,
    client: Option<&sync::SyncClient>,
    data_dir: &std::path::Path,
    capture_count: u32,
) {
    let Some(client) = client else {
        return;
    };
    let in_flight = sync_task.as_ref().is_some_and(|t| !t.is_finished());
    let allow = backoff
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .should_attempt(Instant::now());
    if !periodic_sync_due(capture_count, in_flight, allow) {
        if capture_count.is_multiple_of(SYNC_EVERY_N_CAPTURES) && !in_flight && !allow {
            tracing::debug!(target: "scuffed_stat_tracker", "sync skipped: backing off before the next attempt");
        }
        return;
    }
    let store = store.clone();
    let client = client.clone();
    let data_dir = data_dir.to_path_buf();
    let backoff = Arc::clone(backoff);
    *sync_task = Some(tokio::spawn(async move {
        let outcome = try_sync(&store, &client, &data_dir, None).await;
        apply_sync_backoff(&backoff, outcome);
    }));
}

pub(crate) fn apply_sync_backoff(
    backoff: &std::sync::Mutex<sync::SyncBackoff>,
    outcome: sync::SyncAttempt,
) {
    let now = Instant::now();
    let mut clock = backoff.lock().unwrap_or_else(|e| e.into_inner());
    clock.observe(outcome, now);
    match outcome {
        sync::SyncAttempt::ServerError { .. } => {
            tracing::warn!(
                target: "scuffed_stat_tracker",
                failures = clock.failures(),
                retry_in_secs = clock.retry_after(now).as_secs(),
                "sync upload failed: backing off so a down server is not hammered"
            );
        }
        sync::SyncAttempt::RateLimited { .. } => {
            tracing::warn!(
                target: "scuffed_stat_tracker",
                retry_in_secs = clock.retry_after(now).as_secs(),
                "sync rate-limited: backing off before the next upload"
            );
        }
        sync::SyncAttempt::AuthRejected => {
            tracing::error!(
                target: "scuffed_stat_tracker",
                "sync token rejected: pausing until the server URL or token changes in Settings"
            );
        }
        sync::SyncAttempt::TokenExpired => {
            tracing::error!(target: "scuffed_stat_tracker", "sync token expired: pausing until it is relinked in Settings");
        }
        sync::SyncAttempt::BatchTooLarge => {
            tracing::warn!(
                target: "scuffed_stat_tracker",
                failures = clock.failures(),
                retry_in_secs = clock.retry_after(now).as_secs(),
                "sync batch was still over the server cap after shrinking; backing off"
            );
        }
        sync::SyncAttempt::StatsDeletePending => {
            tracing::warn!(
                target: "scuffed_stat_tracker",
                "site stats are scheduled for deletion: pausing until a new sync token is saved"
            );
        }
        sync::SyncAttempt::Uploaded | sync::SyncAttempt::NoServerCall => {}
    }
}

pub(crate) fn sync_credentials_rejected(
    data_dir: &std::path::Path,
    client: Option<&sync::SyncClient>,
) -> bool {
    let Some(client) = client else {
        return false;
    };
    let creds = client.credentials();
    matches!(
        sync::auth_pause_reason(data_dir, &creds.server_url, &creds.token),
        Some(sync::AuthPauseReason::Rejected | sync::AuthPauseReason::Expired)
    )
}

/// While sync is paused on a rejected token or a stats-delete hold, re-read
/// Settings. A rejected token clears when the URL or token changes. A
/// stats-delete hold clears only when the saved token changes, because
/// undoing the delete does not restore the old token. The rest of the
/// config is left alone.
pub(crate) fn maybe_resume_sync_after_settings_change(
    client: Option<&sync::SyncClient>,
    backoff: &std::sync::Mutex<sync::SyncBackoff>,
    data_dir: &std::path::Path,
) {
    let paused = {
        let clock = backoff.lock().unwrap_or_else(|e| e.into_inner());
        clock.auth_rejected() || clock.delete_pending()
    };
    if !paused {
        return;
    }
    let Ok(cfg) = config::Config::load() else {
        return;
    };
    let Some(sync_cfg) = cfg.sync else {
        return;
    };
    if sync::auth_pause_matches(data_dir, &sync_cfg.server_url, &sync_cfg.token) {
        return;
    }
    let Some(client) = client else {
        return;
    };
    match sync::SyncClient::try_new(sync_cfg) {
        Ok(fresh) => {
            client.replace_credentials(fresh.credentials());
            sync::clear_auth_pause(data_dir);
            backoff
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clear_auth_rejected();
            tracing::info!(target: "scuffed_stat_tracker", "sync resumed: server URL or token changed");
        }
        Err(e) => {
            tracing::warn!(
                target: "scuffed_stat_tracker",
                error = %e,
                "sync stays paused: the saved server URL is not safe for the token"
            );
        }
    }
}

/// One upload. Auth rejection and an open stats-delete hold skip the HTTP
/// call. The caller feeds the result to [`apply_sync_backoff`].
pub(crate) async fn upload_once(
    client: Option<&sync::SyncClient>,
    backoff: &std::sync::Mutex<sync::SyncBackoff>,
    store: &storage::LocalStore,
    data_dir: &std::path::Path,
    settle_session: Option<String>,
) -> sync::SyncAttempt {
    let (rejected, delete_held) = {
        let clock = backoff.lock().unwrap_or_else(|e| e.into_inner());
        (clock.auth_rejected(), clock.delete_pending())
    };
    if rejected {
        tracing::warn!(target: "scuffed_stat_tracker", "sync paused, token rejected; skipping this upload");
        return sync::SyncAttempt::NoServerCall;
    }
    if delete_held {
        tracing::warn!(target: "scuffed_stat_tracker", "sync paused, site stats are scheduled for deletion; skipping this upload");
        return sync::SyncAttempt::NoServerCall;
    }
    match client {
        Some(client) => try_sync(store, client, data_dir, settle_session).await,
        None => sync::SyncAttempt::NoServerCall,
    }
}

/// Run `task` after `previous` has finished. The join happens inside the
/// spawned task, so the caller returns immediately.
pub(crate) fn spawn_after_previous<F, Fut>(
    previous: Option<tokio::task::JoinHandle<()>>,
    task: F,
) -> tokio::task::JoinHandle<()>
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    tokio::spawn(async move {
        if let Some(previous) = previous {
            // Held so an abort of this task (at exit) reaches the one it
            // waits on too: dropping a plain handle would leave that one
            // running with its own store clone.
            let mut previous = AbortOnDrop(previous);
            let _ = (&mut previous.0).await;
        }
        task().await;
    })
}

/// Aborts the task when dropped. After the task finished it does nothing.
struct AbortOnDrop(tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Longest wait for an aborted sync task to be dropped. The abort lands at
/// the task's next await, which an upload reaches at once.
pub(crate) const SYNC_ABORT_JOIN: std::time::Duration = std::time::Duration::from_millis(250);

/// Let the sync task left at exit finish within `within`, then abort it.
/// Its store clone keeps SurrealKV open, so this runs before the store's
/// close. An abort mid-upload loses nothing: rows are marked synced and
/// tombstones cleared only after the server's 2xx, so whatever was not
/// marked stays queued and goes up on the next start. A chunk the server
/// took but that was not yet marked is sent again; the server keeps one
/// row per session and replaces it. Returns true when the task finished
/// by itself.
pub(crate) async fn finish_or_abort_sync(
    task: Option<tokio::task::JoinHandle<()>>,
    within: std::time::Duration,
) -> bool {
    let Some(mut task) = task else {
        return true;
    };
    if tokio::time::timeout(within, &mut task).await.is_ok() {
        return true;
    }
    task.abort();
    let _ = tokio::time::timeout(SYNC_ABORT_JOIN, task).await;
    tracing::warn!(
        target: "scuffed_stat_tracker",
        "sync still running at exit; stopped it, unsent games go up on the next start"
    );
    false
}

/// Queue an upload on the single-flight sync task. The previous handle is
/// joined inside the new task. The result updates the backoff clock.
pub(crate) fn schedule_store_sync(
    sync_task: &mut Option<tokio::task::JoinHandle<()>>,
    backoff: &Arc<std::sync::Mutex<sync::SyncBackoff>>,
    store: &storage::LocalStore,
    client: Option<&sync::SyncClient>,
    data_dir: &std::path::Path,
    settle_session: Option<String>,
) {
    let previous = sync_task.take();
    let client = client.cloned();
    let store = store.clone();
    let data_dir = data_dir.to_path_buf();
    let backoff = Arc::clone(backoff);
    *sync_task = Some(spawn_after_previous(previous, move || async move {
        let outcome =
            upload_once(client.as_ref(), &backoff, &store, &data_dir, settle_session).await;
        apply_sync_backoff(&backoff, outcome);
    }));
}

/// Mark one closed session's unknown rows synced locally. Other unknown
/// rows stay unsynced. Nothing is sent for the settled session.
pub(crate) fn schedule_settled_unknown(
    st: &mut SessionState,
    sync_task: &mut Option<tokio::task::JoinHandle<()>>,
    backoff: &Arc<std::sync::Mutex<sync::SyncBackoff>>,
    store: &storage::LocalStore,
    client: Option<&sync::SyncClient>,
    data_dir: &std::path::Path,
) {
    if let Some(session_id) = st.settle_closed_unknown.take() {
        schedule_store_sync(
            sync_task,
            backoff,
            store,
            client,
            data_dir,
            Some(session_id),
        );
    }
}

pub(crate) fn sync_attempt_from_error(err: &sync::SyncUploadError) -> sync::SyncAttempt {
    let attempt = err.attempt();
    match attempt {
        sync::SyncAttempt::RateLimited { .. } => {
            tracing::warn!(target: "scuffed_stat_tracker", error = %err, "sync rate-limited, will retry after backoff");
        }
        sync::SyncAttempt::AuthRejected => {
            tracing::error!(
                target: "scuffed_stat_tracker",
                error = %err,
                "sync token rejected, will not retry until the URL or token changes"
            );
        }
        sync::SyncAttempt::TokenExpired => {
            tracing::error!(
                target: "scuffed_stat_tracker",
                error = %err,
                "sync token expired, will not retry until it is relinked in Settings"
            );
        }
        sync::SyncAttempt::BatchTooLarge => {
            tracing::warn!(
                target: "scuffed_stat_tracker",
                error = %err,
                "sync batch was too large and the server cap did not shrink"
            );
        }
        sync::SyncAttempt::StatsDeletePending => {
            tracing::warn!(
                target: "scuffed_stat_tracker",
                error = %err,
                "site stats are scheduled for deletion, pausing until a new token is saved"
            );
        }
        _ if err.message.starts_with(sync::SERVER_REFUSING_UPLOADS) => {
            tracing::error!(target: "scuffed_stat_tracker", "{}", err.message);
        }
        _ => tracing::error!(target: "scuffed_stat_tracker", error = %err, "sync upload failed"),
    }
    attempt
}

/// Read unsynced rows, upload, then mark synced only where `sync_rev` is
/// still the revision captured here. `upload` is the HTTP call (or a test
/// double). It runs after the read and before the mark, which is the window
/// a local outcome/map/hero/GUI write can revise a row.
pub(crate) fn claims_covering(
    claims: &[storage::SyncClaim],
    rows: &[storage::PersonalMatch],
    indexes: impl IntoIterator<Item = usize>,
) -> Vec<storage::SyncClaim> {
    let mut picked = vec![false; claims.len()];
    for index in indexes {
        let Some(row) = rows.get(index) else {
            continue;
        };
        for (i, claim) in claims.iter().enumerate() {
            if claim.covers_row(row) {
                picked[i] = true;
            }
        }
    }
    claims
        .iter()
        .enumerate()
        .filter(|(i, _)| picked[*i])
        .map(|(_, claim)| claim.clone())
        .collect()
}

#[cfg(test)]
pub(crate) async fn try_sync_with<F, Fut>(
    store: &storage::LocalStore,
    data_dir: &std::path::Path,
    settle_session: Option<String>,
    upload: F,
) -> sync::SyncAttempt
where
    F: FnMut(Vec<storage::PersonalMatch>, Vec<String>) -> Fut,
    Fut: std::future::Future<
            Output = Result<scuffed_types::api::StatsUploadResponse, sync::SyncUploadError>,
        >,
{
    let mut caps = sync::BatchCaps::default();
    try_sync_with_caps(store, data_dir, settle_session, &mut caps, upload).await
}

pub(crate) async fn try_sync_with_caps<F, Fut>(
    store: &storage::LocalStore,
    data_dir: &std::path::Path,
    settle_session: Option<String>,
    caps: &mut sync::BatchCaps,
    mut upload: F,
) -> sync::SyncAttempt
where
    F: FnMut(Vec<storage::PersonalMatch>, Vec<String>) -> Fut,
    Fut: std::future::Future<
            Output = Result<scuffed_types::api::StatsUploadResponse, sync::SyncUploadError>,
        >,
{
    // Errors are stringified immediately: `Box<dyn Error>` isn't `Send`, and
    // this future runs on a spawned task.
    //
    // The server cannot store an outcome-less game. On a close, mark those
    // rows synced locally and do not put them in the request. A GUI
    // SetOutcome writes a decided outcome and `synced = false`, which the
    // decided query below then uploads with `edited` set.
    if let Some(session_id) = settle_session.as_deref() {
        match store
            .get_unsynced_unknown()
            .await
            .map_err(|e| e.to_string())
        {
            Ok(unknown) => {
                let unknown: Vec<_> = unknown
                    .into_iter()
                    .filter(|row| row.session_id == session_id)
                    .collect();
                if !unknown.is_empty() {
                    let claims = storage::SyncClaim::capture(&unknown);
                    match store.mark_synced(&claims).await.map_err(|e| e.to_string()) {
                        Ok(()) => tracing::info!(
                            target: "scuffed_stat_tracker",
                            rows = unknown.len(),
                            session_id,
                            "held unknown-outcome rows locally, not sent"
                        ),
                        Err(e) => {
                            tracing::error!(
                                target: "scuffed_stat_tracker",
                                error = %e,
                                "failed to hold an unknown-outcome row locally"
                            )
                        }
                    }
                }
            }
            Err(e) => {
                tracing::error!(target: "scuffed_stat_tracker", error = %e, "failed to query unknown-outcome rows")
            }
        }
    }
    let pending = store.get_unsynced().await;
    let unsynced = match pending.map_err(|e| e.to_string()) {
        Ok(u) => u,
        Err(e) => {
            tracing::error!(target: "scuffed_stat_tracker", error = %e, "failed to query unsynced matches");
            return sync::SyncAttempt::NoServerCall;
        }
    };
    let LocalOnlySplit {
        local_only,
        send: unsynced,
        retire,
    } = split_local_only(unsynced);
    // Only a session that was ever read as a tracked game can be on the
    // site. A plain Deathmatch game sends no delete, so its periodic syncs
    // stay request-free.
    let mut retire_checked = Vec::with_capacity(retire.len());
    for session_id in retire {
        match store.get_session_snapshots(&session_id).await {
            Ok(rows) if !session_may_be_on_site(&rows) => {}
            // A read error queues the delete anyway: an extra no-op delete
            // is harmless, a missed one leaves the game on the site.
            _ => retire_checked.push(session_id),
        }
    }
    let retire = retire_checked;
    if !local_only.is_empty() {
        // The tombstone is stored before the rows are marked synced. If the
        // tombstone write fails the rows stay queued and the next pass tries
        // again; the other order could lose the server delete for good.
        let queued = if retire.is_empty() {
            Ok(())
        } else {
            store
                .queue_tombstones(&retire)
                .await
                .map_err(|e| e.to_string())
        };
        match queued {
            Ok(()) => {
                if !retire.is_empty() {
                    tracing::info!(
                        target: "scuffed_stat_tracker",
                        sessions = retire.len(),
                        "queued server deletes for games that are now local-only"
                    );
                }
                let claims = storage::SyncClaim::capture(&local_only);
                if let Err(e) = store.mark_synced(&claims).await.map_err(|e| e.to_string()) {
                    tracing::error!(target: "scuffed_stat_tracker", error = %e, "failed to keep a deathmatch row local");
                }
            }
            Err(e) => {
                tracing::error!(target: "scuffed_stat_tracker", error = %e, "failed to queue a server delete for a local-only game");
            }
        }
    }
    // Locally-deleted sessions whose server rows must go too.
    let tombstones = match store
        .get_pending_tombstones()
        .await
        .map_err(|e| e.to_string())
    {
        Ok(t) => t,
        Err(e) => {
            tracing::error!(target: "scuffed_stat_tracker", error = %e, "failed to query pending tombstones");
            Vec::new()
        }
    };
    if unsynced.is_empty() && tombstones.is_empty() {
        return sync::SyncAttempt::NoServerCall;
    }

    // The server keeps one row per session, so only the final snapshot of
    // each session is worth sending. Every fetched row is *claimed* at its
    // current `sync_rev`; `mark_synced` commits a claim only when that
    // revision is unchanged (a write during `upload` bumps it and stays
    // queued). Collapsed snapshots are represented by the uploaded one, but
    // a revision miss on any of them leaves the whole session queued so the
    // next sync cannot upload an older snapshot by itself.
    let claims = storage::SyncClaim::capture(&unsynced);
    let mut newest_first = unsynced;
    newest_first.reverse(); // get_unsynced is played_at ASC
    let staged = storage::latest_per_game(newest_first);
    let (review_held, to_upload): (Vec<_>, Vec<_>) =
        staged.into_iter().partition(sync::row_needs_review);
    if !review_held.is_empty() {
        tracing::info!(target: "scuffed_stat_tracker", games = review_held.len(), "held games for review, not sent");
    }
    // A pending tombstone for a session this pass uploads again is out of
    // date: the newer snapshot is what the game is now. Sent together, the
    // server would store the game and then delete it, and with the rows
    // marked synced and the tombstone cleared nothing would bring it back.
    // The upsert already replaces the server row, so drop the tombstone.
    let (superseded, tombstones) = split_superseded_tombstones(tombstones, &to_upload);
    if !superseded.is_empty() {
        if let Err(e) = store
            .clear_tombstones(superseded.clone())
            .await
            .map_err(|e| e.to_string())
        {
            // Sending the rows now would let the stale tombstone delete them
            // on a later pass. Retry the whole pass next time.
            tracing::error!(target: "scuffed_stat_tracker", error = %e, "failed to drop a superseded tombstone");
            return sync::SyncAttempt::NoServerCall;
        }
        tracing::info!(
            target: "scuffed_stat_tracker",
            sessions = superseded.len(),
            "dropped server deletes for games uploaded again"
        );
    }
    if to_upload.is_empty() && tombstones.is_empty() {
        return sync::SyncAttempt::NoServerCall;
    }
    tracing::info!(
        target: "scuffed_stat_tracker",
        rows = claims.len(),
        games = to_upload.len(),
        tombstones = tombstones.len(),
        "syncing unsynced matches"
    );
    // Chunks stay at or under the server batch cap. Each chunk is acknowledged
    // on its own: mark_synced / clear_tombstones run only for the rows that
    // request wrote. A later chunk is not sent after a hard failure.
    upload_chunked(
        store,
        data_dir,
        &claims,
        &to_upload,
        tombstones,
        caps,
        &mut upload,
    )
    .await
}

/// Split pending tombstones into those whose session is in `to_upload`
/// (superseded by the newer upload) and the rest, in input order.
pub(crate) fn split_superseded_tombstones(
    tombstones: Vec<String>,
    to_upload: &[storage::PersonalMatch],
) -> (Vec<String>, Vec<String>) {
    let uploading: std::collections::HashSet<&str> = to_upload
        .iter()
        .filter(|row| !row.session_id.is_empty())
        .map(|row| row.session_id.as_str())
        .collect();
    tombstones
        .into_iter()
        .partition(|sid| uploading.contains(sid.as_str()))
}

/// A row the site does not store: an untracked map or mode (Deathmatch,
/// Workshop, Château Guillard, ...), judged on the displayed map and on the
/// mode that would be uploaded.
pub(crate) fn row_is_local_only(row: &storage::PersonalMatch) -> bool {
    let sent = parse::uploaded_game_mode(row.display_map_name(), &row.game_mode);
    !parse::stats_row_is_tracked(row.display_map_name(), &row.game_mode)
        || !parse::stats_row_is_tracked(row.display_map_name(), &sent)
}

/// True when any snapshot of a session was read, or is shown, as a tracked
/// game, so an upload of it may have reached the site. Raw fields count as
/// well as the edited ones: a GUI map edit rewrites the displayed map of
/// every snapshot, but the uploaded copy was the raw read.
pub(crate) fn session_may_be_on_site(snapshots: &[storage::PersonalMatch]) -> bool {
    snapshots.iter().any(|row| {
        !row_is_local_only(row) || parse::stats_row_is_tracked(&row.map_name, &row.game_mode)
    })
}

/// Unsynced rows split for one sync pass. See [`split_local_only`].
#[derive(Debug, Default)]
pub(crate) struct LocalOnlySplit {
    /// Rows marked synced locally and never sent.
    pub(crate) local_only: Vec<storage::PersonalMatch>,
    /// Rows that go to the upload path.
    pub(crate) send: Vec<storage::PersonalMatch>,
    /// Sessions that need a server delete (DATA-4).
    pub(crate) retire: Vec<String>,
}

/// Split unsynced rows (`played_at` ASC, as `get_unsynced` returns them)
/// into rows to keep local and rows to upload.
///
/// The newest unsynced row of a session decides that session. When it is
/// local-only (a game first read as Control and later re-read or edited to
/// a Deathmatch map), every unsynced row of the session stays local and the
/// session is queued for a server delete, because an earlier snapshot may
/// already be on the site. When the newest row is tracked, older local-only
/// snapshots of the same session stay local without a tombstone and the
/// tracked rows upload as before. Rows without a session id are judged one
/// by one and are never tombstoned.
pub(crate) fn split_local_only(rows: Vec<storage::PersonalMatch>) -> LocalOnlySplit {
    let mut newest_local: std::collections::HashMap<String, bool> =
        std::collections::HashMap::new();
    for row in &rows {
        if !row.session_id.is_empty() {
            // ASC order: the last row seen per session is the newest.
            newest_local.insert(row.session_id.clone(), row_is_local_only(row));
        }
    }
    let mut split = LocalOnlySplit::default();
    for row in rows {
        let retired = newest_local.get(&row.session_id).copied().unwrap_or(false);
        if retired {
            if !split.retire.contains(&row.session_id) {
                split.retire.push(row.session_id.clone());
            }
            split.local_only.push(row);
        } else if row_is_local_only(&row) {
            split.local_only.push(row);
        } else {
            split.send.push(row);
        }
    }
    split
}

/// Send `to_upload` and `tombstones` in capped requests.
///
/// At most [`sync::SYNC_UPLOAD_MAX_MATCHES`] games and
/// [`sync::SYNC_UPLOAD_MAX_DELETES`] tombstones go out together, or the
/// lower caps already learned by this process. HTTP 400 and 422 split
/// inside the chunk until a single bad row is found. That row is
/// quarantined only when some other row in the chunk was accepted. If the
/// chunk accepts nothing, nothing is quarantined, the rows stay queued, and
/// the pass backs off. A 400 or 422 that does not name a row blames only
/// the probed row and retries the rest. HTTP 413 `batch_too_large` lowers
/// those caps (never below 1, never raised later) and resends immediately;
/// that request wrote nothing. A 413 that repeats the same caps, or sends
/// none, stops the pass and counts as a failure. HTTP 429, 401, 403, 409,
/// and other errors stop the pass. 429 keeps `Retry-After`. 409
/// `stats_delete_pending` keeps every row queued and does not quarantine.
/// Tombstones ride on the chunk they were sent with, including 400 splits
/// of that chunk, and are cleared only after a 2xx. A deletes-only 400
/// still backs off like a 5xx: there are no match rows to isolate, so one
/// bad id keeps that delete chunk retrying.
pub(crate) async fn upload_chunked<F, Fut>(
    store: &storage::LocalStore,
    data_dir: &std::path::Path,
    claims: &[storage::SyncClaim],
    to_upload: &[storage::PersonalMatch],
    mut tombstones: Vec<String>,
    caps: &mut sync::BatchCaps,
    upload: &mut F,
) -> sync::SyncAttempt
where
    F: FnMut(Vec<storage::PersonalMatch>, Vec<String>) -> Fut,
    Fut: std::future::Future<
            Output = Result<scuffed_types::api::StatsUploadResponse, sync::SyncUploadError>,
        >,
{
    let mut match_limit = caps.matches.max(1);
    let mut delete_limit = caps.deletes.max(1);
    let mut remaining: Vec<usize> = (0..to_upload.len()).collect();
    let mut uploaded = 0usize;
    let mut quarantined = 0usize;
    let mut requests = 0usize;
    let mut progress = false;
    let mut stop: Option<sync::SyncAttempt> = None;

    loop {
        if remaining.is_empty() && tombstones.is_empty() {
            break;
        }
        let match_take = match_limit.min(remaining.len());
        let delete_take = delete_limit.min(tombstones.len());
        let chunk_indexes: Vec<usize> = remaining.iter().take(match_take).copied().collect();
        let chunk_rows: Vec<storage::PersonalMatch> = chunk_indexes
            .iter()
            .map(|&index| to_upload[index].clone())
            .collect();
        let chunk_deletes: Vec<String> = tombstones.iter().take(delete_take).cloned().collect();

        if chunk_rows.is_empty() {
            requests += 1;
            match upload(Vec::new(), chunk_deletes.clone()).await {
                Ok(resp) => {
                    tracing::info!(target: "scuffed_stat_tracker", deleted = resp.deleted, "sync delete chunk complete");
                    clear_acknowledged_tombstones(store, &chunk_deletes).await;
                    tombstones.drain(..delete_take);
                    progress = true;
                }
                Err(err) if err.is_batch_too_large() => {
                    if !sync::lower_batch_limits(
                        &mut match_limit,
                        &mut delete_limit,
                        err.max_matches,
                        err.max_deleted_sessions,
                    ) {
                        stop = Some(sync_attempt_from_error(&err));
                        break;
                    }
                    caps.matches = match_limit;
                    caps.deletes = delete_limit;
                    tracing::info!(
                        target: "scuffed_stat_tracker",
                        max_matches = match_limit,
                        max_deleted_sessions = delete_limit,
                        "server asked for a smaller delete batch, retrying"
                    );
                }
                Err(err) => {
                    stop = Some(sync_attempt_from_error(&err));
                    break;
                }
            }
            continue;
        }

        let isolation = sync::isolate_rejected(&chunk_rows, |batch| {
            upload(batch.to_vec(), chunk_deletes.clone())
        })
        .await;
        requests += isolation.requests;

        for rejected in &isolation.rejected {
            let Some(&global) = chunk_indexes.get(rejected.index) else {
                continue;
            };
            let covered = claims_covering(claims, to_upload, [global]);
            let session_id = to_upload
                .get(global)
                .map(|row| row.session_id.as_str())
                .unwrap_or("");
            tracing::warn!(
                target: "scuffed_stat_tracker",
                session_id,
                error = %rejected.message,
                "upload rejected this row"
            );
            if let Err(e) = store
                .quarantine_upload(&covered, rejected.code.as_deref(), &rejected.message)
                .await
                .map_err(|e| e.to_string())
            {
                tracing::error!(target: "scuffed_stat_tracker", error = %e, session_id, "failed to quarantine a rejected row");
            }
            quarantined += 1;
            progress = true;
        }

        if !isolation.accepted.is_empty() {
            let accepted_global: Vec<usize> = isolation
                .accepted
                .iter()
                .filter_map(|&local| chunk_indexes.get(local).copied())
                .collect();
            let accepted = claims_covering(claims, to_upload, accepted_global.iter().copied());
            if !accepted.is_empty()
                && let Err(e) = store
                    .mark_synced(&accepted)
                    .await
                    .map_err(|e| e.to_string())
            {
                tracing::error!(target: "scuffed_stat_tracker", error = %e, "failed to mark matches as synced");
            }
            clear_acknowledged_tombstones(store, &chunk_deletes).await;
            tombstones.drain(..delete_take);
            uploaded += accepted_global.len();
            progress = true;
        }

        let mut done_local = vec![false; chunk_rows.len()];
        for &local in &isolation.accepted {
            if let Some(slot) = done_local.get_mut(local) {
                *slot = true;
            }
        }
        for rejected in &isolation.rejected {
            if let Some(slot) = done_local.get_mut(rejected.index) {
                *slot = true;
            }
        }
        let mut next_remaining = Vec::with_capacity(remaining.len());
        for (offset, &global) in remaining.iter().enumerate() {
            if offset < match_take && done_local.get(offset).copied().unwrap_or(false) {
                continue;
            }
            next_remaining.push(global);
        }
        remaining = next_remaining;

        if let Some(err) = isolation.stopped {
            if err.is_batch_too_large()
                && sync::lower_batch_limits(
                    &mut match_limit,
                    &mut delete_limit,
                    err.max_matches,
                    err.max_deleted_sessions,
                )
            {
                caps.matches = match_limit;
                caps.deletes = delete_limit;
                tracing::info!(
                    target: "scuffed_stat_tracker",
                    max_matches = match_limit,
                    max_deleted_sessions = delete_limit,
                    "server asked for a smaller sync batch, retrying"
                );
                continue;
            }
            stop = Some(sync_attempt_from_error(&err));
            break;
        }

        if !isolation.deferred.is_empty() {
            tracing::warn!(
                target: "scuffed_stat_tracker",
                deferred = isolation.deferred.len(),
                requests,
                "sync left rows queued after the request cap"
            );
            break;
        }
    }

    if progress {
        refresh_snapshot_force(store, data_dir).await;
    }
    if let Some(attempt) = stop {
        return attempt;
    }
    if !progress {
        return sync::SyncAttempt::NoServerCall;
    }
    tracing::info!(target: "scuffed_stat_tracker", uploaded, quarantined, requests, "sync pass finished");
    sync::SyncAttempt::Uploaded
}

pub(crate) async fn clear_acknowledged_tombstones(
    store: &storage::LocalStore,
    session_ids: &[String],
) {
    if session_ids.is_empty() {
        return;
    }
    if let Err(e) = store
        .clear_tombstones(session_ids.to_vec())
        .await
        .map_err(|e| e.to_string())
    {
        tracing::error!(target: "scuffed_stat_tracker", error = %e, "failed to clear acknowledged tombstones");
    }
}
