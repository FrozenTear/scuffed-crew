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
        // Redirects are followed only on the configured server's own
        // scheme, host and port (`token_safe_redirects`), so the bearer
        // token never follows a redirect to another host or to cleartext.
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
            .map_err(|e| SyncUploadError::plain(send_error_text(&e)))?;

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
            .map_err(|e| SyncUploadError::plain(send_error_text(&e)))?;

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

/// Follow a redirect only when it stays on the scheme, host and port of the
/// first request, which is the configured server. The site's stats routes
/// never redirect; a proxy may still send a same-site path redirect. Anything
/// else (another host, another port, `https` to `http` or back) is an error,
/// so neither the bearer token nor the upload body goes to a server the
/// member did not configure. reqwest would drop `Authorization` on a
/// cross-host hop, but the request would still go there, and a 401 from it
/// would pause sync as a rejected token.
///
/// Public for every client that sends the sync token or a link secret: the
/// desktop app's token check, sign-in link and report upload use it too.
pub fn token_safe_redirects() -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(|attempt| {
        if attempt.previous().len() >= 10 {
            return attempt.error(std::io::Error::other("too many sync redirects"));
        }
        let same_server = attempt
            .previous()
            .first()
            .is_some_and(|first| same_origin(first, attempt.url()));
        if !same_server {
            let refused = OffSiteRedirect::to(attempt.url());
            return attempt.error(refused);
        }
        if let Err(reject) = validate_sync_server_url(attempt.url().as_str()) {
            return attempt.error(reject);
        }
        attempt.follow()
    })
}

/// `e` as text. A refused redirect also names why: reqwest's own text is
/// only "error following redirect for url (...)", and the reason sits in
/// its source.
fn send_error_text(e: &reqwest::Error) -> String {
    let mut text = e.to_string();
    if e.is_redirect() {
        let mut source = std::error::Error::source(e);
        while let Some(cause) = source {
            text.push_str(": ");
            text.push_str(&cause.to_string());
            source = cause.source();
        }
    }
    text
}

/// The advice every redirect text ends with, kept in one place. `$field`
/// says where the Website URL field is.
macro_rules! website_url_fix {
    ($field:literal) => {
        concat!("the Website URL ", $field, " to the address the site uses")
    };
}

/// Error text for a redirect away from the configured server (daemon log),
/// when the target is not known.
pub(super) const SYNC_REDIRECT_OFF_SITE: &str = concat!(
    "the site redirected sync to a different address; set ",
    website_url_fix!("in Settings")
);

/// The same advice as a sentence for the desktop app, shown when
/// [`is_off_site_redirect`] holds for a request error and the target is not
/// known. [`off_site_redirect_message`] names the target when it is.
pub const REDIRECT_OFF_SITE: &str = concat!(
    "The site redirected to a different address. Set ",
    website_url_fix!("in Settings"),
    "."
);

/// The desktop app's redirect advice. `target` is the redirect's origin
/// (scheme, host and port, never a path), named in place of "a different
/// address" when known.
pub(super) fn redirect_off_site_text(target: Option<&str>) -> String {
    match target {
        Some(target) => format!(
            concat!(
                "The site redirected to {}. Set ",
                website_url_fix!("in Settings"),
                "."
            ),
            target
        ),
        None => REDIRECT_OFF_SITE.to_string(),
    }
}

/// The error [`token_safe_redirects`] refuses an off-site hop with, so a
/// caller can tell it apart from other failures by type, not by text.
/// `target` is the refused hop's origin only, so no path or query (which
/// may carry a secret) ends up in a message.
#[derive(Debug)]
struct OffSiteRedirect {
    target: Option<String>,
}

impl OffSiteRedirect {
    fn to(url: &reqwest::Url) -> Self {
        let origin = url.origin();
        Self {
            target: origin.is_tuple().then(|| origin.ascii_serialization()),
        }
    }
}

impl std::fmt::Display for OffSiteRedirect {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.target {
            Some(target) => write!(
                f,
                concat!(
                    "the site redirected sync to {}; set ",
                    website_url_fix!("in Settings")
                ),
                target
            ),
            None => f.write_str(SYNC_REDIRECT_OFF_SITE),
        }
    }
}

impl std::error::Error for OffSiteRedirect {}

/// The [`OffSiteRedirect`] in `e`'s sources, if any.
fn off_site_redirect(e: &reqwest::Error) -> Option<&OffSiteRedirect> {
    if !e.is_redirect() {
        return None;
    }
    let mut source = std::error::Error::source(e);
    while let Some(cause) = source {
        if let Some(off_site) = cause.downcast_ref::<OffSiteRedirect>() {
            return Some(off_site);
        }
        source = cause.source();
    }
    None
}

/// `true` when `e` is [`token_safe_redirects`] refusing a redirect to
/// another scheme, host or port. Other redirect errors (too many hops)
/// and network failures are `false`.
pub fn is_off_site_redirect(e: &reqwest::Error) -> bool {
    off_site_redirect(e).is_some()
}

/// The desktop app's advice for `e` when it is an off-site redirect
/// refusal ([`is_off_site_redirect`]), naming the target's origin when
/// known. `None` for any other error.
pub fn off_site_redirect_message(e: &reqwest::Error) -> Option<String> {
    off_site_redirect(e).map(|off_site| redirect_off_site_text(off_site.target.as_deref()))
}

/// Same scheme, host and port (the default port counts as written).
pub(super) fn same_origin(a: &reqwest::Url, b: &reqwest::Url) -> bool {
    a.scheme() == b.scheme()
        && a.host_str() == b.host_str()
        && a.port_or_known_default() == b.port_or_known_default()
}
