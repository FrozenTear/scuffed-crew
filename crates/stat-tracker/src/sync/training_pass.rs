//! One training crop send pass: read the consent, drop what is no longer
//! consented, send at most one batch per consented kind, and leave a
//! warning for the desktop app when something did not work.
//!
//! A pass never touches the local store, the stats sync clock or the stats
//! sync pause file. The daemon runs it on its own task
//! (`daemon/training_sched.rs`), so a slow or failing site never holds up
//! recording or stats sync.

use std::path::Path;
use std::time::Duration;

use scuffed_types::api::training_crops::{
    TRAINING_CROP_ERR_CONSENT_REQUIRED, TRAINING_CROP_ERR_LIMIT_REACHED,
};

use super::client::SyncClient;
use super::training::{UploadKind, batch_len, check_crop};
use super::training_client::TrainingCallError;
use super::training_store::{
    adopt_training_owner, crop_kept_at, drop_kind, queued_crops, remove_crops, training_delete_at,
    write_cached_consent, write_training_warning,
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

fn call_warning(what: &str, error: &TrainingCallError) -> (String, TrainingRetry) {
    let retry = match error {
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
    };
    let text = format!(
        "Training crops: could not {what}. {} {WAITING_NOTE}",
        error.plain_text()
    );
    (text, retry)
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
    if adopt_training_owner(data_dir, &owner.server_url, &owner.token) {
        tracing::info!("training crops: website or token changed, waiting crops dropped");
    }
    let consent = match client.fetch_training_consent().await {
        Ok(consent) => consent,
        Err(error) => {
            let (text, retry) = call_warning("read your consent from the website", &error);
            tracing::warn!(?error, "training crop consent read failed");
            report.retry = retry;
            write_training_warning(data_dir, Some(&text), false, false);
            report.warning = Some(text);
            return report;
        }
    };
    report.consent_read = true;
    let mut cached = consent;
    let mut warnings: Vec<String> = Vec::new();
    let mut sticky = false;
    for kind in UploadKind::ALL {
        if client.credentials() != owner {
            // Settings saved another website or token mid-pass.
            break;
        }
        if !kind.consented(&consent) {
            // Never sent, and not kept either.
            drop_kind(data_dir, kind);
            continue;
        }
        let mut crops = queued_crops(data_dir, kind);
        let mut refused = Vec::new();
        // Kept before the member's last "Delete my training crops": gone.
        let deleted_at = training_delete_at(data_dir);
        crops.retain(|crop| {
            let before_delete = deleted_at
                .zip(crop_kept_at(&crop.id))
                .is_some_and(|(at, kept)| kept <= at);
            if before_delete {
                refused.push(crop.id.clone());
            }
            !before_delete
        });
        crops.retain(|crop| match check_crop(kind, crop) {
            Ok(()) => true,
            Err(refusal) => {
                tracing::info!(
                    kind = kind.as_str(),
                    ?refusal,
                    "training crop over a cap: dropped, not sent"
                );
                refused.push(crop.id.clone());
                false
            }
        });
        remove_crops(data_dir, kind, &refused);
        crops.truncate(batch_len(&crops));
        if crops.is_empty() {
            continue;
        }
        let ids: Vec<String> = crops.iter().map(|crop| crop.id.clone()).collect();
        let posted_at = chrono::Utc::now().timestamp_millis();
        match client.upload_training_crops(kind, &crops).await {
            Ok(answer) => {
                remove_crops(data_dir, kind, &ids);
                if training_delete_at(data_dir).is_some_and(|at| at >= posted_at) {
                    // The member asked to delete everything while this
                    // upload ran: remove what it stored again.
                    for stored in &answer.crops {
                        if let Err(error) = client.delete_training_crop(&stored.id).await {
                            tracing::warn!(?error, "could not remove a crop sent during a delete");
                        }
                    }
                    continue;
                }
                report.sent += answer.crops.len();
            }
            Err(error) => match (error.status(), error.code()) {
                (Some(403), Some(TRAINING_CROP_ERR_CONSENT_REQUIRED)) => {
                    // Consent went off since the read: stop this kind.
                    drop_kind(data_dir, kind);
                    set_consent(&mut cached, kind, false);
                }
                (Some(409), Some(TRAINING_CROP_ERR_LIMIT_REACHED)) => {
                    drop_kind(data_dir, kind);
                    sticky = true;
                    warnings.push(format!(
                        "Training crops: the website already keeps as many {} as it takes from you. Waiting ones were dropped.",
                        kind.label().to_lowercase()
                    ));
                }
                (Some(400 | 413 | 415 | 422), code) => {
                    remove_crops(data_dir, kind, &ids);
                    sticky = true;
                    warnings.push(format!(
                        "Training crops: the website refused {} {} ({}). They were dropped.",
                        ids.len(),
                        kind.label().to_lowercase(),
                        code.unwrap_or("no reason given")
                    ));
                }
                _ => {
                    let (text, retry) = call_warning("send them", &error);
                    tracing::warn!(?error, kind = kind.as_str(), "training crop upload failed");
                    warnings.push(text);
                    report.retry = worse(report.retry, retry);
                    // A down or limiting site gets no more calls this pass.
                    break;
                }
            },
        }
    }
    if let Err(error) = write_cached_consent(data_dir, cached) {
        tracing::warn!(%error, "could not cache the training crop consent");
    }
    report.warning = (!warnings.is_empty()).then(|| warnings.join(" "));
    write_training_warning(data_dir, report.warning.as_deref(), sticky, report.sent > 0);
    report
}

fn set_consent(
    consent: &mut scuffed_types::api::training_crops::TrainingCropConsent,
    kind: UploadKind,
    on: bool,
) {
    match kind {
        UploadKind::HeroIcon => consent.hero_icon = on,
        UploadKind::StatDigits => consent.stat_digits = on,
        UploadKind::MapName => consent.map_name = on,
    }
}
