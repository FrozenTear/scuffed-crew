use std::path::Path;
use std::sync::{Arc, Mutex};

use crate::config::{SyncConfig, SyncUrlReject, validate_sync_server_url};
use crate::storage::PersonalMatch;

use scuffed_types::api::{
    DaemonConfigResponse, StatsUploadEntry, StatsUploadRequest, StatsUploadResponse,
};

/// Overview health copy when the server refused the bearer token.
/// The GUI shows this string; the hint is the Settings path.
pub const SYNC_TOKEN_REJECTED_STATUS: &str = "Sync token rejected — update the token in Settings";

const AUTH_PAUSE_FILE: &str = "sync_auth.json";

/// First wait after a server/network upload failure. Doubles each consecutive
/// failure until [`SYNC_BACKOFF_CAP`].
pub const SYNC_BACKOFF_BASE: std::time::Duration = std::time::Duration::from_secs(15);
/// Ceiling so a down server is retried, not forgotten, and not hammered.
pub const SYNC_BACKOFF_CAP: std::time::Duration = std::time::Duration::from_secs(5 * 60);

/// How long to wait before the next periodic sync after `consecutive_failures`
/// server/network errors in a row. Zero when there have been none.
pub fn sync_backoff_delay(consecutive_failures: u32) -> std::time::Duration {
    if consecutive_failures == 0 {
        return std::time::Duration::ZERO;
    }
    // 2^(n-1), shift capped so a long outage cannot overflow the factor.
    let exp = consecutive_failures.saturating_sub(1).min(16);
    let factor = 1u32 << exp;
    SYNC_BACKOFF_BASE
        .saturating_mul(factor)
        .min(SYNC_BACKOFF_CAP)
}

/// Result of one sync attempt, for the periodic scheduler's backoff clock.
/// Shutdown's final upload does not consult this clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncAttempt {
    /// The server accepted the upload.
    Uploaded,
    /// Server or network error, including HTTP 503. Back off before the next
    /// periodic attempt. `retry_after` is a floor when the server sent one.
    ServerError {
        retry_after: Option<std::time::Duration>,
    },
    /// HTTP 429. Wait, then try again. This is not a hard failure: the
    /// exponential failure streak is left unchanged and the rows stay queued.
    RateLimited {
        retry_after: Option<std::time::Duration>,
    },
    /// HTTP 401 or 403 from upload or daemon-config. Stop periodic retries
    /// until the saved server URL or token changes.
    AuthRejected,
    /// Nothing to upload, or the failure was local. Leave the clock alone.
    NoServerCall,
}

/// Upload (or the transport under it) failed. `status` / `retry_after` are
/// set when the server actually answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncUploadError {
    pub message: String,
    pub status: Option<u16>,
    pub retry_after: Option<std::time::Duration>,
}

impl SyncUploadError {
    pub fn plain(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            status: None,
            retry_after: None,
        }
    }

    /// 401 and 403 pause sync. 429 is rate-limit backoff. 503 is a server
    /// error that may carry `Retry-After` as a wait floor. Other statuses
    /// are server errors and ignore `Retry-After`.
    pub fn attempt(&self) -> SyncAttempt {
        match self.status {
            Some(401) | Some(403) => SyncAttempt::AuthRejected,
            Some(429) => SyncAttempt::RateLimited {
                retry_after: self.retry_after,
            },
            Some(503) => SyncAttempt::ServerError {
                retry_after: self.retry_after,
            },
            _ => SyncAttempt::ServerError { retry_after: None },
        }
    }
}

impl std::fmt::Display for SyncUploadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for SyncUploadError {}

/// `Retry-After` as delay-seconds or an HTTP-date (IMF-fixdate). Values that
/// do not fit in `Duration` saturate instead of panicking. A date in the past
/// is a zero wait ("retry now"). Unrecognized text is `None`.
pub fn parse_retry_after(
    value: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> Option<std::time::Duration> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    if let Ok(secs) = value.parse::<u64>() {
        return Some(saturating_duration_secs(secs));
    }
    let target = chrono::DateTime::parse_from_rfc2822(value)
        .ok()
        .map(|dt| dt.with_timezone(&chrono::Utc))
        .or_else(|| {
            chrono::NaiveDateTime::parse_from_str(value, "%a %b %e %H:%M:%S %Y")
                .ok()
                .map(|naive| naive.and_utc())
        })?;
    let delta = target.signed_duration_since(now);
    if delta <= chrono::TimeDelta::zero() {
        return Some(std::time::Duration::ZERO);
    }
    Some(delta.to_std().unwrap_or(SYNC_BACKOFF_CAP))
}

fn saturating_duration_secs(secs: u64) -> std::time::Duration {
    if secs >= std::time::Duration::MAX.as_secs() {
        std::time::Duration::MAX
    } else {
        std::time::Duration::from_secs(secs)
    }
}

/// Bounded exponential backoff for periodic sync. Reset on a successful upload.
/// An empty queue does not count as success or failure.
#[derive(Debug, Clone, Default)]
pub struct SyncBackoff {
    failures: u32,
    next_attempt_at: Option<std::time::Instant>,
    /// Set on HTTP 401/403. Periodic sync stays off until
    /// [`Self::clear_auth_rejected`] (saved URL or token changed).
    auth_rejected: bool,
}

impl SyncBackoff {
    pub fn failures(&self) -> u32 {
        self.failures
    }

    pub fn auth_rejected(&self) -> bool {
        self.auth_rejected
    }

    pub fn should_attempt(&self, now: std::time::Instant) -> bool {
        if self.auth_rejected {
            return false;
        }
        self.next_attempt_at.is_none_or(|at| now >= at)
    }

    pub fn record_failure(&mut self, now: std::time::Instant) {
        self.failures = self.failures.saturating_add(1);
        self.next_attempt_at = Some(now + sync_backoff_delay(self.failures));
    }

    pub fn record_success(&mut self) {
        self.failures = 0;
        self.next_attempt_at = None;
        self.auth_rejected = false;
    }

    /// Pause periodic sync. Does not grow the failure streak and does not
    /// schedule a retry — the next attempt waits for a credential change.
    pub fn record_auth_rejected(&mut self) {
        self.auth_rejected = true;
        self.next_attempt_at = None;
    }

    pub fn clear_auth_rejected(&mut self) {
        self.auth_rejected = false;
    }

    /// HTTP 429. Schedule a wait of at least `retry_after` (the base delay
    /// when the server sent none), capped at [`SYNC_BACKOFF_CAP`]. Does not
    /// increment [`Self::failures`].
    pub fn record_rate_limit(
        &mut self,
        now: std::time::Instant,
        retry_after: Option<std::time::Duration>,
    ) {
        let hinted = retry_after.unwrap_or(SYNC_BACKOFF_BASE);
        self.schedule_at_least(now, hinted);
    }

    /// Raise the next-attempt time so it is at least `hint`, still capped.
    /// Used for `Retry-After` on 503 after the exponential delay is set.
    pub fn extend_for_retry_after(
        &mut self,
        now: std::time::Instant,
        retry_after: Option<std::time::Duration>,
    ) {
        let Some(hint) = retry_after else {
            return;
        };
        self.schedule_at_least(now, hint);
    }

    fn schedule_at_least(&mut self, now: std::time::Instant, hint: std::time::Duration) {
        let hinted = hint.min(SYNC_BACKOFF_CAP);
        let wait = hinted.max(self.retry_after(now));
        self.next_attempt_at = Some(now + wait);
    }

    /// Remaining delay. Zero when an attempt is allowed.
    pub fn retry_after(&self, now: std::time::Instant) -> std::time::Duration {
        match self.next_attempt_at {
            Some(at) if at > now => at.saturating_duration_since(now),
            _ => std::time::Duration::ZERO,
        }
    }

    pub fn observe(&mut self, attempt: SyncAttempt, now: std::time::Instant) {
        match attempt {
            SyncAttempt::Uploaded => self.record_success(),
            SyncAttempt::ServerError { retry_after } => {
                self.record_failure(now);
                self.extend_for_retry_after(now, retry_after);
            }
            SyncAttempt::RateLimited { retry_after } => self.record_rate_limit(now, retry_after),
            SyncAttempt::AuthRejected => self.record_auth_rejected(),
            SyncAttempt::NoServerCall => {}
        }
    }
}

#[derive(Clone)]
pub struct SyncClient {
    config: Arc<Mutex<SyncConfig>>,
    http: reqwest::Client,
}

impl SyncClient {
    pub fn new(config: SyncConfig) -> Self {
        // A hung connection must never hang the caller: sync runs concurrently
        // with capture, and shutdown does a final inline upload. Fail closed —
        // a client without the timeout re-opens the M4 daemon-stall bug.
        // Redirects to a URL that fails the same check are errors, so a
        // downgrade to cleartext cannot carry the bearer token.
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .redirect(token_safe_redirects())
            .build()
            .expect("reqwest client with timeout must build");
        Self {
            http,
            config: Arc::new(Mutex::new(config)),
        }
    }

    pub fn credentials(&self) -> SyncConfig {
        self.config
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Swap the URL and token after Settings saved a different pair.
    /// The HTTP client (timeout, redirect policy) stays.
    pub fn replace_credentials(&self, config: SyncConfig) {
        *self.config.lock().unwrap_or_else(|e| e.into_inner()) = config;
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
            return Err(error_from_response(resp, "daemon-config fetch").await);
        }

        resp.json::<DaemonConfigResponse>()
            .await
            .map_err(|e| SyncUploadError::plain(e.to_string()))
    }

    pub async fn upload_matches(
        &self,
        matches: &[PersonalMatch],
        deleted_sessions: &[String],
    ) -> Result<StatsUploadResponse, SyncUploadError> {
        self.refuse_unsafe_transport()
            .map_err(|e| SyncUploadError::plain(e.to_string()))?;
        let creds = self.credentials();
        let entries: Vec<StatsUploadEntry> = matches
            .iter()
            // Upload the effective (corrected-if-present, else OCR) values so
            // server aggregates and the leaderboard reflect manual fixes, and
            // flag edited rows for the site badge. The immutable OCR reads stay
            // local — the transparency detail lives in the tracker GUI.
            .map(|m| StatsUploadEntry {
                session_id: m.session_id.clone(),
                hero: m.display_hero().to_string(),
                map_name: m.display_map_name().to_string(),
                game_mode: m.game_mode.clone(),
                role: m.display_role().to_string(),
                outcome: m.display_outcome().to_string(),
                elims: m.display_elims(),
                deaths: m.display_deaths(),
                assists: m.display_assists(),
                damage: m.display_damage(),
                healing: m.display_healing(),
                mitigation: m.display_mitigation(),
                played_at: chrono::DateTime::<chrono::Utc>::from(m.played_at),
                edited: m.is_edited(),
            })
            .collect();

        let url = format!("{}/api/stats/upload", creds.server_url);
        let resp = self
            .http
            .post(&url)
            .bearer_auth(&creds.token)
            .json(&StatsUploadRequest {
                matches: entries,
                deleted_sessions: deleted_sessions.to_vec(),
            })
            .send()
            .await
            .map_err(|e| SyncUploadError::plain(e.to_string()))?;

        if !resp.status().is_success() {
            return Err(error_from_response(resp, "Upload").await);
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

/// Honor `Retry-After` only on 429 and 503 — the statuses the API uses for
/// rate limits and "try later". Other errors keep the plain exponential clock.
async fn error_from_response(resp: reqwest::Response, what: &str) -> SyncUploadError {
    let status = resp.status();
    let retry_after = retry_after_from_response(&resp);
    let body = resp.text().await.unwrap_or_default();
    SyncUploadError {
        message: format!("{what} failed ({status}): {body}"),
        status: Some(status.as_u16()),
        retry_after,
    }
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

fn auth_pause_path(data_dir: &Path) -> std::path::PathBuf {
    data_dir.join(AUTH_PAUSE_FILE)
}

/// Stable equality token for the bearer. Not a password hash — the file is
/// mode 0600 beside `config.toml`, which already stores the token. It only
/// answers "did this exact token change?".
fn token_fingerprint(token: &str) -> String {
    let mut h1: u64 = 0xcbf29ce484222325;
    let mut h2: u64 = 0x84222325cbf29ce4;
    for (i, byte) in token.as_bytes().iter().enumerate() {
        h1 ^= u64::from(*byte);
        h1 = h1.wrapping_mul(0x100000001b3);
        h2 ^= u64::from(*byte).wrapping_add(i as u64);
        h2 = h2.wrapping_mul(0xcbf29ce484222325);
    }
    format!("{h1:016x}{h2:016x}")
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct AuthPauseFile {
    server_url: String,
    token_fingerprint: String,
}

/// Remember that this URL + token was refused (401/403). Mode 0600.
pub fn write_auth_pause(data_dir: &Path, server_url: &str, token: &str) -> std::io::Result<()> {
    if let Some(parent) = auth_pause_path(data_dir).parent() {
        std::fs::create_dir_all(parent)?;
        crate::fs_mode::tighten_private_dir(parent);
    }
    let file = AuthPauseFile {
        server_url: server_url.to_string(),
        token_fingerprint: token_fingerprint(token),
    };
    let path = auth_pause_path(data_dir);
    let bytes = serde_json::to_vec(&file).map_err(std::io::Error::other)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, bytes)?;
    crate::fs_mode::tighten_private_file(&tmp);
    std::fs::rename(&tmp, &path)?;
    crate::fs_mode::tighten_private_file(&path);
    Ok(())
}

pub fn clear_auth_pause(data_dir: &Path) {
    let _ = std::fs::remove_file(auth_pause_path(data_dir));
}

/// True when the on-disk pause was recorded for this exact URL and token.
pub fn auth_pause_matches(data_dir: &Path, server_url: &str, token: &str) -> bool {
    let Ok(bytes) = std::fs::read(auth_pause_path(data_dir)) else {
        return false;
    };
    let Ok(file) = serde_json::from_slice::<AuthPauseFile>(&bytes) else {
        return false;
    };
    file.server_url == server_url && file.token_fingerprint == token_fingerprint(token)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn client(url: &str) -> SyncClient {
        SyncClient::new(SyncConfig {
            server_url: url.to_string(),
            token: "super-secret-token".to_string(),
        })
    }

    #[test]
    fn try_new_rejects_cleartext_non_loopback_and_allows_https_and_loopback() {
        assert!(
            SyncClient::try_new(SyncConfig {
                server_url: "http://example.com".into(),
                token: "t".into(),
            })
            .is_err()
        );
        assert!(
            SyncClient::try_new(SyncConfig {
                server_url: "https://example.com".into(),
                token: "t".into(),
            })
            .is_ok()
        );
        assert!(
            SyncClient::try_new(SyncConfig {
                server_url: "http://127.0.0.1:9".into(),
                token: "t".into(),
            })
            .is_ok()
        );
        assert!(
            SyncClient::try_new(SyncConfig {
                server_url: "http://[::1]:9".into(),
                token: "t".into(),
            })
            .is_ok()
        );
        assert!(
            SyncClient::try_new(SyncConfig {
                server_url: "http://localhost:9".into(),
                token: "t".into(),
            })
            .is_ok()
        );
    }

    #[tokio::test]
    async fn cleartext_non_loopback_does_not_send_bearer() {
        // TEST-NET-3. If the guard did not return first, reqwest would sit
        // on the 30s client timeout (or a connect error). A fast policy
        // error that omits the token is the refusal.
        let started = std::time::Instant::now();
        let c = client("http://203.0.113.10/stats");
        let fetch_err = c.fetch_daemon_config().await.unwrap_err().to_string();
        let upload_err = c.upload_matches(&[], &[]).await.unwrap_err().to_string();
        assert!(
            started.elapsed() < std::time::Duration::from_secs(2),
            "refused in {:?}",
            started.elapsed()
        );
        for err in [&fetch_err, &upload_err] {
            assert!(
                err.contains("https"),
                "expected the scheme refusal, got {err}"
            );
            assert!(
                !err.contains("super-secret-token"),
                "token leaked into the error: {err}"
            );
            assert!(
                !err.contains("error sending request"),
                "request left the process: {err}"
            );
        }
    }

    #[tokio::test]
    async fn loopback_http_sends_bearer_token() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let port = listener.local_addr().expect("addr").port();
        let server = std::thread::spawn(move || recv_one_http(listener));
        let c = client(&format!("http://127.0.0.1:{port}"));
        let _ = c.fetch_daemon_config().await;
        let req = server.join().expect("server thread");
        let head = req.to_ascii_lowercase();
        assert!(
            head.contains("authorization: bearer super-secret-token"),
            "loopback http is the local-dev exception and must still authenticate; got {req:?}"
        );
        assert!(
            head.contains("get /api/stats/daemon-config"),
            "unexpected request {req:?}"
        );
    }

    fn recv_one_http(listener: std::net::TcpListener) -> String {
        exchange_http(
            listener,
            b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n",
        )
    }

    fn exchange_http(listener: std::net::TcpListener, response: &[u8]) -> String {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        listener.set_nonblocking(true).expect("nonblocking");
        loop {
            match listener.accept() {
                Ok((mut sock, _)) => {
                    sock.set_nonblocking(false).ok();
                    sock.set_read_timeout(Some(std::time::Duration::from_secs(2)))
                        .ok();
                    let mut buf = [0u8; 4096];
                    let mut got = Vec::new();
                    loop {
                        match std::io::Read::read(&mut sock, &mut buf) {
                            Ok(0) => break,
                            Ok(n) => {
                                got.extend_from_slice(&buf[..n]);
                                if got.windows(4).any(|w| w == b"\r\n\r\n") {
                                    break;
                                }
                            }
                            Err(_) => break,
                        }
                    }
                    let _ = std::io::Write::write_all(&mut sock, response);
                    return String::from_utf8_lossy(&got).into_owned();
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    if std::time::Instant::now() > deadline {
                        return String::new();
                    }
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                Err(_) => return String::new(),
            }
        }
    }

    #[test]
    fn backoff_grows_caps_and_resets_on_success() {
        let d1 = sync_backoff_delay(1);
        let d2 = sync_backoff_delay(2);
        let d3 = sync_backoff_delay(3);
        assert_eq!(d1, SYNC_BACKOFF_BASE);
        assert!(d2 > d1 && d3 > d2, "delay must grow: {d1:?} {d2:?} {d3:?}");
        assert_eq!(d2, SYNC_BACKOFF_BASE * 2);
        assert_eq!(sync_backoff_delay(0), std::time::Duration::ZERO);
        assert_eq!(sync_backoff_delay(20), SYNC_BACKOFF_CAP);
        assert!(sync_backoff_delay(5) <= SYNC_BACKOFF_CAP);
        assert_eq!(sync_backoff_delay(6), SYNC_BACKOFF_CAP);

        let mut backoff = SyncBackoff::default();
        let t0 = std::time::Instant::now();
        assert!(backoff.should_attempt(t0));
        backoff.record_failure(t0);
        assert_eq!(backoff.failures(), 1);
        assert!(!backoff.should_attempt(t0));
        assert_eq!(backoff.retry_after(t0), d1);
        assert!(backoff.should_attempt(t0 + d1));
        assert!(!backoff.should_attempt(t0 + d1 - std::time::Duration::from_millis(1)));

        backoff.record_failure(t0);
        assert_eq!(backoff.failures(), 2);
        assert_eq!(backoff.retry_after(t0), d2);
        assert!(d2 > d1);

        backoff.observe(SyncAttempt::NoServerCall, t0);
        assert_eq!(
            backoff.failures(),
            2,
            "a local/empty attempt must not move the clock"
        );

        backoff.record_success();
        assert_eq!(backoff.failures(), 0);
        assert!(backoff.should_attempt(t0));
        assert_eq!(backoff.retry_after(t0), std::time::Duration::ZERO);

        backoff.observe(SyncAttempt::ServerError { retry_after: None }, t0);
        assert_eq!(backoff.retry_after(t0), SYNC_BACKOFF_BASE);
        backoff.observe(SyncAttempt::Uploaded, t0);
        assert!(backoff.should_attempt(t0), "success resets the backoff");
        assert_eq!(backoff.failures(), 0);
    }

    #[test]
    fn retry_after_parses_seconds_and_http_date() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-09-24T08:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        assert_eq!(
            parse_retry_after("120", now),
            Some(std::time::Duration::from_secs(120))
        );
        assert_eq!(
            parse_retry_after("  45  ", now),
            Some(std::time::Duration::from_secs(45))
        );
        assert_eq!(
            parse_retry_after("Thu, 24 Sep 2026 08:02:00 GMT", now),
            Some(std::time::Duration::from_secs(120))
        );
        assert_eq!(
            parse_retry_after("Thu, 24 Sep 2026 07:00:00 GMT", now),
            Some(std::time::Duration::ZERO),
            "a Retry-After date in the past means retry now"
        );
        assert!(parse_retry_after("not-a-date", now).is_none());
        assert_eq!(
            parse_retry_after("999999999999999", now),
            Some(std::time::Duration::from_secs(999_999_999_999_999))
        );
        assert_eq!(
            parse_retry_after(&u64::MAX.to_string(), now),
            Some(std::time::Duration::MAX),
            "a delay-seconds that overflows Duration must saturate, not panic"
        );
    }

    #[test]
    fn retry_after_429_waits_and_is_not_a_hard_failure() {
        let mut backoff = SyncBackoff::default();
        let t0 = std::time::Instant::now();
        backoff.record_failure(t0);
        assert_eq!(backoff.failures(), 1);

        backoff.observe(
            SyncAttempt::RateLimited {
                retry_after: Some(std::time::Duration::from_secs(120)),
            },
            t0,
        );
        assert_eq!(
            backoff.failures(),
            1,
            "429 must not advance the failure streak"
        );
        assert_eq!(backoff.retry_after(t0), std::time::Duration::from_secs(120));
        assert!(!backoff.should_attempt(t0 + std::time::Duration::from_secs(119)));
        assert!(backoff.should_attempt(t0 + std::time::Duration::from_secs(120)));

        backoff.observe(
            SyncAttempt::RateLimited {
                retry_after: Some(std::time::Duration::from_secs(24 * 60 * 60)),
            },
            t0,
        );
        assert_eq!(backoff.failures(), 1);
        assert_eq!(
            backoff.retry_after(t0),
            SYNC_BACKOFF_CAP,
            "Retry-After is capped at the backoff maximum"
        );

        let mut fresh = SyncBackoff::default();
        fresh.observe(SyncAttempt::RateLimited { retry_after: None }, t0);
        assert_eq!(fresh.failures(), 0);
        assert_eq!(fresh.retry_after(t0), SYNC_BACKOFF_BASE);
    }

    #[test]
    fn retry_after_503_is_a_floor_on_the_exponential_delay() {
        let mut backoff = SyncBackoff::default();
        let t0 = std::time::Instant::now();
        backoff.observe(
            SyncAttempt::ServerError {
                retry_after: Some(std::time::Duration::from_secs(120)),
            },
            t0,
        );
        assert_eq!(backoff.failures(), 1, "503 still counts as a server error");
        assert_eq!(backoff.retry_after(t0), std::time::Duration::from_secs(120));

        backoff.observe(
            SyncAttempt::ServerError {
                retry_after: Some(std::time::Duration::from_secs(24 * 60 * 60)),
            },
            t0,
        );
        assert_eq!(backoff.failures(), 2);
        assert_eq!(backoff.retry_after(t0), SYNC_BACKOFF_CAP);
    }

    #[tokio::test]
    async fn upload_reads_retry_after_on_429_and_503() {
        let secs = upload_rejected(429, "120").await;
        assert_eq!(secs.status, Some(429));
        assert_eq!(secs.retry_after, Some(std::time::Duration::from_secs(120)));
        assert!(matches!(
            secs.attempt(),
            SyncAttempt::RateLimited {
                retry_after: Some(d)
            } if d == std::time::Duration::from_secs(120)
        ));

        let when = chrono::Utc::now() + chrono::TimeDelta::seconds(90);
        let header = when.format("%a, %d %b %Y %H:%M:%S GMT").to_string();
        let dated = upload_rejected(503, &header).await;
        assert_eq!(dated.status, Some(503));
        let wait = dated.retry_after.expect("HTTP-date Retry-After");
        assert!(
            (80..=100).contains(&wait.as_secs()),
            "expected about 90s from the HTTP-date, got {wait:?}"
        );
        assert!(matches!(
            dated.attempt(),
            SyncAttempt::ServerError {
                retry_after: Some(_)
            }
        ));
    }

    async fn upload_rejected(status: u16, retry_after: &str) -> SyncUploadError {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let port = listener.local_addr().expect("addr").port();
        let response = format!(
            "HTTP/1.1 {status} no\r\nRetry-After: {retry_after}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        let response_bytes = response.into_bytes();
        let server = std::thread::spawn(move || exchange_http(listener, &response_bytes));
        let c = client(&format!("http://127.0.0.1:{port}"));
        let err = c.upload_matches(&[], &[]).await.expect_err("rejected");
        let _ = server.join().expect("server thread");
        err
    }

    #[tokio::test]
    async fn upload_401_and_403_pause_without_growing_the_failure_streak() {
        let t0 = std::time::Instant::now();
        for status in [401_u16, 403] {
            let err = upload_rejected(status, "30").await;
            assert_eq!(err.status, Some(status));
            assert!(
                err.retry_after.is_none(),
                "401/403 must ignore Retry-After, got {:?}",
                err.retry_after
            );
            assert!(matches!(err.attempt(), SyncAttempt::AuthRejected));
            let mut backoff = SyncBackoff::default();
            backoff.observe(err.attempt(), t0);
            assert!(backoff.auth_rejected());
            assert_eq!(
                backoff.failures(),
                0,
                "auth rejection is not a server-error streak"
            );
            assert!(!backoff.should_attempt(t0));
            assert!(
                !backoff.should_attempt(t0 + SYNC_BACKOFF_CAP + std::time::Duration::from_secs(1)),
                "a rejected token must not become due again on the backoff clock"
            );
        }
    }

    #[tokio::test]
    async fn daemon_config_401_is_the_same_auth_rejection() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let server = std::thread::spawn(move || {
            exchange_http(
                listener,
                b"HTTP/1.1 401 no\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            )
        });
        let c = client(&format!("http://127.0.0.1:{port}"));
        let err = c.fetch_daemon_config().await.expect_err("401");
        let _ = server.join().expect("server");
        assert_eq!(err.status, Some(401));
        assert!(matches!(err.attempt(), SyncAttempt::AuthRejected));
    }

    #[tokio::test]
    async fn upload_500_stays_a_server_error_and_ignores_retry_after() {
        let err = upload_rejected(500, "30").await;
        assert!(matches!(
            err.attempt(),
            SyncAttempt::ServerError { retry_after: None }
        ));
        let t0 = std::time::Instant::now();
        let mut backoff = SyncBackoff::default();
        backoff.observe(err.attempt(), t0);
        assert!(!backoff.auth_rejected());
        assert_eq!(backoff.failures(), 1);
        assert_eq!(backoff.retry_after(t0), SYNC_BACKOFF_BASE);
    }

    #[test]
    fn auth_pause_matches_only_the_rejected_url_and_token() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_auth_pause(dir.path(), "https://crew.example", "old-token").unwrap();
        assert!(auth_pause_matches(
            dir.path(),
            "https://crew.example",
            "old-token"
        ));
        assert!(
            !auth_pause_matches(dir.path(), "https://crew.example", "new-token"),
            "a new token must clear the pause"
        );
        assert!(
            !auth_pause_matches(dir.path(), "https://other.example", "old-token"),
            "a new URL must clear the pause"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.path().join(AUTH_PAUSE_FILE))
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600);
        }
        clear_auth_pause(dir.path());
        assert!(!auth_pause_matches(
            dir.path(),
            "https://crew.example",
            "old-token"
        ));
    }

    #[test]
    fn clearing_auth_rejection_allows_the_next_attempt_immediately() {
        let t0 = std::time::Instant::now();
        let mut backoff = SyncBackoff::default();
        backoff.observe(SyncAttempt::AuthRejected, t0);
        assert!(!backoff.should_attempt(t0));
        backoff.clear_auth_rejected();
        assert!(!backoff.auth_rejected());
        assert!(backoff.should_attempt(t0));
        assert_eq!(backoff.failures(), 0);
    }
}
