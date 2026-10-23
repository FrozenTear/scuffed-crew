//! [`SyncClient`]: the HTTP client (reqwest), its redirect policy and the
//! two endpoints. The only file in `sync` that uses reqwest.

use std::sync::{Arc, Mutex};

use crate::config::{SyncConfig, SyncUrlReject, validate_sync_server_url};
use crate::storage::PersonalMatch;
use crate::version::USER_AGENT;

use scuffed_types::api::{DaemonConfigResponse, StatsUploadResponse};

use super::backoff::parse_retry_after;
use super::batching::BatchCaps;
use super::errors::{SyncUploadError, parse_upload_error_body, server_error_text};
use super::wire::upload_request;

#[derive(Clone)]
pub struct SyncClient {
    config: Arc<Mutex<SyncConfig>>,
    http: reqwest::Client,
    /// Caps learned from HTTP 413. Shared by clones. Never raised.
    batch_caps: Arc<Mutex<BatchCaps>>,
    /// `stats_delete_scheduled_at` from the latest 409 `stats_delete_pending`.
    /// Kept off [`SyncUploadError`] so that error stays small. Shared by clones.
    delete_scheduled_at: Arc<Mutex<Option<String>>>,
}

impl SyncClient {
    pub fn new(config: SyncConfig) -> Self {
        // A hung connection must never hang the caller: sync runs concurrently
        // with capture, and shutdown does a final inline upload. Fail closed,
        // a client without the timeout re-opens the M4 daemon-stall bug.
        // Redirects to a URL that fails the same check are errors, so a
        // downgrade to cleartext cannot carry the bearer token.
        let http = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(std::time::Duration::from_secs(30))
            .redirect(token_safe_redirects())
            .build()
            .expect("reqwest client with timeout must build");
        Self {
            http,
            config: Arc::new(Mutex::new(config)),
            batch_caps: Arc::new(Mutex::new(BatchCaps::default())),
            delete_scheduled_at: Arc::new(Mutex::new(None)),
        }
    }

    /// Take the deletion timestamp from the latest 409, if the body sent one.
    pub fn take_delete_scheduled_at(&self) -> Option<String> {
        self.delete_scheduled_at
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
    }

    fn remember_delete_scheduled_at(&self, scheduled_at: Option<String>) {
        *self
            .delete_scheduled_at
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = scheduled_at;
    }

    /// Chunk sizes for the next upload. Lowered caps stick for this process.
    pub fn batch_caps(&self) -> BatchCaps {
        *self.batch_caps.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Keep `caps` when either limit fell. A higher value is ignored.
    pub fn remember_batch_caps(&self, caps: BatchCaps) {
        let mut slot = self.batch_caps.lock().unwrap_or_else(|e| e.into_inner());
        if caps.matches < slot.matches {
            slot.matches = caps.matches.max(1);
        }
        if caps.deletes < slot.deletes {
            slot.deletes = caps.deletes.max(1);
        }
    }

    pub fn credentials(&self) -> SyncConfig {
        self.config
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Swap the URL and token after Settings saved a different pair.
    /// The HTTP client (timeout, redirect policy) stays. A different server
    /// URL drops caps learned from the previous host. A token-only change
    /// keeps them.
    pub fn replace_credentials(&self, config: SyncConfig) {
        let mut slot = self.config.lock().unwrap_or_else(|e| e.into_inner());
        let url_changed = slot.server_url != config.server_url;
        *slot = config;
        drop(slot);
        if url_changed {
            *self.batch_caps.lock().unwrap_or_else(|e| e.into_inner()) = BatchCaps::default();
        }
    }

    /// `new`, but refuse an unsafe server URL before a client exists.
    /// The daemon logs this once and keeps running with sync off.
    pub fn try_new(config: SyncConfig) -> Result<Self, SyncUrlReject> {
        validate_sync_server_url(&config.server_url)?;
        Ok(Self::new(config))
    }

    /// Last line before `bearer_auth`. `try_new` already rejected unsafe
    /// configs; this still runs so a client built with `new` cannot send.
    fn refuse_unsafe_transport(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        validate_sync_server_url(&self.credentials().server_url)?;
        Ok(())
    }

    /// Fetch daemon configuration from the server (player_name, etc.).
    /// Called on startup when local config has no player_name.
    /// 401 and 403 are [`SyncAttempt::AuthRejected`], same as upload.
    ///
    /// [`SyncAttempt::AuthRejected`]: super::SyncAttempt::AuthRejected
    pub async fn fetch_daemon_config(&self) -> Result<DaemonConfigResponse, SyncUploadError> {
        self.refuse_unsafe_transport()
            .map_err(|e| SyncUploadError::plain(e.to_string()))?;
        let creds = self.credentials();
        let url = format!("{}/api/stats/daemon-config", creds.server_url);
        let resp = self
            .http
            .get(&url)
            .bearer_auth(&creds.token)
            .send()
            .await
            .map_err(|e| SyncUploadError::plain(e.to_string()))?;

        if !resp.status().is_success() {
            return Err(error_from_response(resp, "daemon-config fetch").await.0);
        }

        resp.json::<DaemonConfigResponse>()
            .await
            .map_err(|e| SyncUploadError::plain(e.to_string()))
    }

    /// POST `/api/stats/upload`.
    ///
    /// HTTP 400 and 422 are returned as [`SyncUploadError`]. The sync pass
    /// ([`isolate_rejected`], called from `try_sync_with`) bisects those
    /// batches. This function sends one request. 413, 401, 403, 409, and 429
    /// are not payload rejects. 429 and 503 keep `Retry-After`. 409
    /// `stats_delete_pending` stops the pass without a split.
    ///
    /// [`isolate_rejected`]: super::isolate_rejected
    pub async fn upload_matches(
        &self,
        matches: &[PersonalMatch],
        deleted_sessions: &[String],
    ) -> Result<StatsUploadResponse, SyncUploadError> {
        self.refuse_unsafe_transport()
            .map_err(|e| SyncUploadError::plain(e.to_string()))?;
        let creds = self.credentials();
        let url = format!("{}/api/stats/upload", creds.server_url);
        let resp = self
            .http
            .post(&url)
            .bearer_auth(&creds.token)
            .json(&upload_request(matches, deleted_sessions))
            .send()
            .await
            .map_err(|e| SyncUploadError::plain(e.to_string()))?;

        if !resp.status().is_success() {
            let (err, scheduled_at) = error_from_response(resp, "Upload").await;
            if err.is_stats_delete_pending() {
                self.remember_delete_scheduled_at(scheduled_at);
            }
            return Err(err);
        }

        let result: StatsUploadResponse = resp
            .json()
            .await
            .map_err(|e| SyncUploadError::plain(e.to_string()))?;
        tracing::info!(
            inserted = result.inserted,
            skipped = result.skipped,
            deleted = result.deleted,
            "stats upload complete"
        );
        Ok(result)
    }
}

/// Honor `Retry-After` only on 429 and 503, the statuses the API uses for
/// rate limits and "try later". Other errors keep the plain exponential clock.
async fn error_from_response(
    resp: reqwest::Response,
    what: &str,
) -> (SyncUploadError, Option<String>) {
    let status = resp.status();
    let retry_after = retry_after_from_response(&resp);
    let body = resp.text().await.unwrap_or_default();
    let parsed = parse_upload_error_body(&body);
    let scheduled_at = parsed.stats_delete_scheduled_at;
    (
        SyncUploadError {
            message: server_error_text(what, status.as_u16(), &body),
            status: Some(status.as_u16()),
            retry_after,
            code: parsed.code,
            max_matches: parsed.max_matches,
            max_deleted_sessions: parsed.max_deleted_sessions,
        },
        scheduled_at,
    )
}

fn retry_after_from_response(resp: &reqwest::Response) -> Option<std::time::Duration> {
    let code = resp.status().as_u16();
    if code != 429 && code != 503 {
        return None;
    }
    let raw = resp
        .headers()
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?;
    parse_retry_after(raw, chrono::Utc::now())
}

fn token_safe_redirects() -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(|attempt| {
        if attempt.previous().len() >= 10 {
            return attempt.error(std::io::Error::other("too many sync redirects"));
        }
        if let Err(reject) = validate_sync_server_url(attempt.url().as_str()) {
            return attempt.error(reject);
        }
        attempt.follow()
    })
}
