//! One training crop send pass: remove what an earlier delete left on the
//! website, read the consent, drop what is no longer consented, send at
//! most one batch per consented kind, and leave a warning for the desktop
//! app when something did not work.
//!
//! The daemon runs it on its own task (`daemon/training_sched.rs`), so a
//! slow or failing site never holds up recording or stats sync. A pass
//! never touches the local store, the stats sync clock or the stats sync
//! pause file. Its file reads and writes run on blocking threads, and only
//! the crops of the batch being sent are read from disk.
//!
//! Every call uses the website and token the pass started with. When
//! Settings saves another pair meanwhile, the pass stops before the next
//! kind and writes nothing it learned (consent, warning, ids to delete)
//! under the new pair.

use std::path::Path;
use std::time::Duration;

use scuffed_types::api::training_crops::{
    TRAINING_CROP_ERR_CONSENT_REQUIRED, TRAINING_CROP_ERR_LIMIT_REACHED,
    TRAINING_CROP_ERR_NOT_FOUND, TrainingCropConsent,
};

use crate::config::SyncConfig;

use super::client::SyncClient;
use super::training::UploadKind;
use super::training_client::TrainingCallError;
use super::training_store::{
    adopt_training_owner, deleted_again_since, drop_kind, next_batch, pending_deletes,
    read_training_status, remove_crops, training_delete_at, training_in_use, while_owner,
    write_cached_consent, write_pending_deletes, write_training_warning,
};

/// When the next pass may run, as the pass sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrainingRetry {
    /// It worked (or there was nothing to do): the normal cadence.
    Normal,
    /// The site asked to wait (429 or 503 with `Retry-After`).
    After(Duration),
    /// Network or server error: back off, growing with each failure.
    Backoff,
    /// The token was refused or the store is off: wait long.
    Pause,
}

/// What one pass did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrainingPassReport {
    /// Crops the server stored.
    pub sent: usize,
    /// The consent was read in this pass.
    pub consent_read: bool,
    pub retry: TrainingRetry,
    /// The warning left for the desktop app, if any.
    pub warning: Option<String>,
}

const WAITING_NOTE: &str = "They stay on this PC and are tried again later.";

/// Shown while crops the website stored during a delete are still there.
pub(super) const TRAINING_DELETE_OWED: &str = "Training crops: some crops sent while you deleted your training crops are still on the website. The tracker keeps trying to remove them.";

/// Shown when a send got no answer while the member deleted: the website
/// may have stored those crops after the delete, and the tracker has no ids
/// to remove. Stays until a delete the member presses again works (see
/// `deleted_again_since`).
pub(super) const TRAINING_DELETE_AGAIN: &str = "Training crops: the website did not answer a send that ran while you deleted your training crops, so some of those crops may still be on the website. Press Delete my training crops again to remove them.";

fn retry_for(error: &TrainingCallError) -> TrainingRetry {
    match error {
        TrainingCallError::Transport(_) => TrainingRetry::Backoff,
        TrainingCallError::Status {
            status: 429 | 503,
            retry_after: Some(after),
            ..
        } => TrainingRetry::After(*after),
        TrainingCallError::Status {
            status: 401 | 403 | 503,
            ..
        } => TrainingRetry::Pause,
        TrainingCallError::Status { .. } => TrainingRetry::Backoff,
    }
}

fn call_warning(what: &str, error: &TrainingCallError) -> (String, TrainingRetry) {
    let text = format!(
        "Training crops: could not {what}. {} {WAITING_NOTE}",
        error.plain_text()
    );
    (text, retry_for(error))
}

/// The later of two retries: a pause wins over a wait, a wait over a
/// backoff.
fn worse(a: TrainingRetry, b: TrainingRetry) -> TrainingRetry {
    fn rank(r: TrainingRetry) -> u8 {
        match r {
            TrainingRetry::Normal => 0,
            TrainingRetry::Backoff => 1,
            TrainingRetry::After(_) => 2,
            TrainingRetry::Pause => 3,
        }
    }
    match (a, b) {
        (TrainingRetry::After(x), TrainingRetry::After(y)) => TrainingRetry::After(x.max(y)),
        _ if rank(b) > rank(a) => b,
        _ => a,
    }
}

/// Run `work` on the training crop files on a blocking thread, off the
/// daemon's async threads.
async fn on_disk<T, F>(data_dir: &Path, work: F) -> T
where
    T: Send + 'static,
    F: FnOnce(&Path) -> T + Send + 'static,
{
    let dir = data_dir.to_path_buf();
    tokio::task::spawn_blocking(move || work(&dir))
        .await
        .expect("training crop file work")
}

/// Delete `ids` on `owner`'s website, first to last. A crop already gone
/// (404 `crop_not_found`) counts as deleted. Stops at the first other
/// failure, and `ids` keeps the ones not deleted yet.
async fn delete_by_id(
    client: &SyncClient,
    owner: &SyncConfig,
    ids: &mut Vec<String>,
) -> Result<(), TrainingCallError> {
    while let Some(id) = ids.first() {
        match client.delete_training_crop(owner, id).await {
            Ok(_) => {}
            Err(error)
                if error.status() == Some(404)
                    && error.code() == Some(TRAINING_CROP_ERR_NOT_FOUND) => {}
            Err(error) => return Err(error),
        }
        ids.remove(0);
    }
    Ok(())
}

/// What a pass writes when it ends.
#[derive(Default)]
struct PassEnd {
    /// The consent as read, less kinds the upload found off.
    cached: Option<TrainingCropConsent>,
    warnings: Vec<String>,
    /// Crops were dropped: the warning stays until a send works.
    sticky: bool,
    /// Ids still to delete on the website.
    pending: Vec<String>,
    /// There were ids to delete when the pass started.
    had_pending: bool,
    /// The delete marker under which a send got no answer: ask the member
    /// to delete again ([`TRAINING_DELETE_AGAIN`]).
    delete_again: Option<i64>,
    /// An earlier ask to delete again is answered: the marker changed.
    deleted_again: bool,
}

/// One pass. See the module doc.
pub async fn run_training_pass(client: &SyncClient, data_dir: &Path) -> TrainingPassReport {
    let mut report = TrainingPassReport {
        sent: 0,
        consent_read: false,
        retry: TrainingRetry::Normal,
        warning: None,
    };
    // Crops and consent kept for another website or token are dropped
    // before anything is sent.
    let owner = client.credentials();
    let (url, token) = (owner.server_url.clone(), owner.token.clone());
    let (changed, pending, asked_at, deleted_again) = on_disk(data_dir, move |dir| {
        let changed = adopt_training_owner(dir, &url, &token);
        let asked_at = read_training_status(dir).and_then(|status| status.delete_again_at);
        (
            changed,
            pending_deletes(dir),
            asked_at,
            deleted_again_since(dir, asked_at),
        )
    })
    .await;
    if changed {
        tracing::info!("training crops: website or token changed, waiting crops dropped");
    }
    // An ask to delete again stays until the member deletes again and the
    // website answers that delete with success.
    let delete_again = asked_at.filter(|_| !deleted_again);
    let mut end = PassEnd {
        had_pending: !pending.is_empty(),
        pending,
        delete_again,
        deleted_again,
        ..PassEnd::default()
    };
    // Crops an earlier delete left on the website go first, before any
    // upload. A site that fails them gets no more calls this pass.
    if let Err(error) = delete_by_id(client, &owner, &mut end.pending).await {
        tracing::warn!(?error, "could not remove training crops left by a delete");
        report.retry = retry_for(&error);
        return finish(client, data_dir, &owner, report, end).await;
    }
    let consent = match client.fetch_training_consent_as(&owner).await {
        Ok(consent) => consent,
        Err(error) => {
            let (text, retry) = call_warning("read your consent from the website", &error);
            tracing::warn!(?error, "training crop consent read failed");
            report.retry = retry;
            // A member who never turned a kind on, with nothing waiting,
            // is not warned about a feature they do not use.
            if on_disk(data_dir, training_in_use).await {
                end.warnings.push(text);
            }
            return finish(client, data_dir, &owner, report, end).await;
        }
    };
    report.consent_read = true;
    end.cached = Some(consent);
    for kind in UploadKind::ALL {
        if client.credentials() != owner {
            // Settings saved another website or token mid-pass.
            break;
        }
        if !kind.consented(&consent) {
            // Never sent, and not kept either.
            on_disk(data_dir, move |dir| drop_kind(dir, kind)).await;
            continue;
        }
        // The delete marker as it is before the batch is picked. Crops kept
        // before it are dropped; any change after the upload means a
        // delete ran meanwhile.
        let (seen, crops) = on_disk(data_dir, move |dir| {
            let seen = training_delete_at(dir);
            (seen, next_batch(dir, kind, seen))
        })
        .await;
        if crops.is_empty() {
            continue;
        }
        let ids: Vec<String> = crops.iter().map(|crop| crop.id.clone()).collect();
        let result = client.upload_training_crops(&owner, kind, &crops).await;
        drop(crops);
        match result {
            Ok(answer) => {
                let now = on_disk(data_dir, move |dir| {
                    remove_crops(dir, kind, &ids);
                    training_delete_at(dir)
                })
                .await;
                if now != seen {
                    // The member asked to delete everything while this
                    // upload ran, and that delete may have reached the
                    // site before these crops were stored: remove them by
                    // id. What fails is saved and tried on every pass.
                    let mut stored: Vec<String> =
                        answer.crops.into_iter().map(|crop| crop.id).collect();
                    let failed = delete_by_id(client, &owner, &mut stored).await;
                    end.pending.extend(stored);
                    if let Err(error) = failed {
                        tracing::warn!(?error, "could not remove crops sent during a delete");
                        report.retry = worse(report.retry, retry_for(&error));
                        break;
                    }
                    continue;
                }
                report.sent += answer.crops.len();
            }
            Err(error) => match (error.status(), error.code()) {
                (Some(403), Some(TRAINING_CROP_ERR_CONSENT_REQUIRED)) => {
                    // Consent went off since the read: stop this kind.
                    on_disk(data_dir, move |dir| drop_kind(dir, kind)).await;
                    if let Some(cached) = end.cached.as_mut() {
                        set_consent(cached, kind, false);
                    }
                }
                (Some(409), Some(TRAINING_CROP_ERR_LIMIT_REACHED)) => {
                    on_disk(data_dir, move |dir| drop_kind(dir, kind)).await;
                    end.sticky = true;
                    end.warnings.push(format!(
                        "Training crops: the website already keeps as many {} as it takes from you. Waiting ones were dropped.",
                        kind.label().to_lowercase()
                    ));
                }
                (Some(400 | 413 | 415 | 422), code) => {
                    let text = format!(
                        "Training crops: the website refused {} {} ({}). They were dropped.",
                        ids.len(),
                        kind.label().to_lowercase(),
                        code.unwrap_or("no reason given")
                    );
                    on_disk(data_dir, move |dir| remove_crops(dir, kind, &ids)).await;
                    end.sticky = true;
                    end.warnings.push(text);
                }
                _ => {
                    let (text, retry) = call_warning("send them", &error);
                    tracing::warn!(?error, kind = kind.as_str(), "training crop upload failed");
                    // No answer (a timeout or a dropped connection) may
                    // still have stored the batch. When a delete ran
                    // meanwhile, the ids are unknown, and the next pass
                    // drops the local copies as kept before the delete: ask
                    // the member to delete again.
                    let now = match error {
                        TrainingCallError::Transport(_) => {
                            on_disk(data_dir, training_delete_at).await
                        }
                        TrainingCallError::Status { .. } => seen,
                    };
                    if now.is_some() && now != seen {
                        end.delete_again = now;
                    } else {
                        end.warnings.push(text);
                    }
                    report.retry = worse(report.retry, retry);
                    // A down or limiting site gets no more calls this pass.
                    break;
                }
            },
        }
    }
    finish(client, data_dir, &owner, report, end).await
}

/// Write what the pass learned: the consent, the ids still to delete and
/// the warning. Nothing is written when the website or token changed
/// meanwhile, since it belongs to the old pair.
async fn finish(
    client: &SyncClient,
    data_dir: &Path,
    owner: &SyncConfig,
    mut report: TrainingPassReport,
    end: PassEnd,
) -> TrainingPassReport {
    let PassEnd {
        cached,
        mut warnings,
        sticky,
        pending,
        had_pending,
        delete_again,
        deleted_again,
    } = end;
    let owed = !pending.is_empty();
    if owed {
        warnings.insert(0, TRAINING_DELETE_OWED.to_string());
    }
    if delete_again.is_some() {
        warnings.insert(0, TRAINING_DELETE_AGAIN.to_string());
    }
    report.warning = (!warnings.is_empty()).then(|| warnings.join(" "));
    if client.credentials() != *owner {
        tracing::info!("training crops: website or token changed during the pass, nothing kept");
        return report;
    }
    // A warning about dropped crops stays until a send works, one about
    // crops still on the website until they are gone, and an ask to delete
    // again until the member does.
    let sticky = sticky || owed || delete_again.is_some();
    let clear_sticky = report.sent > 0 || (had_pending && !owed) || deleted_again;
    let warning = report.warning.clone();
    let (url, token) = (owner.server_url.clone(), owner.token.clone());
    let written = on_disk(data_dir, move |dir| {
        while_owner(dir, &url, &token, || {
            write_pending_deletes(dir, &pending);
            if let Some(consent) = cached
                && let Err(error) = write_cached_consent(dir, consent)
            {
                tracing::warn!(%error, "could not cache the training crop consent");
            }
            write_training_warning(dir, warning.as_deref(), sticky, clear_sticky, delete_again);
        })
    })
    .await;
    if written.is_none() {
        tracing::info!("training crops: website or token changed during the pass, nothing kept");
    }
    report
}

fn set_consent(consent: &mut TrainingCropConsent, kind: UploadKind, on: bool) {
    match kind {
        UploadKind::HeroIcon => consent.hero_icon = on,
        UploadKind::StatDigits => consent.stat_digits = on,
        UploadKind::MapName => consent.map_name = on,
    }
}
