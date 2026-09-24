use crate::config::{SyncConfig, SyncUrlReject, validate_sync_server_url};
use crate::storage::PersonalMatch;

use scuffed_types::api::{
    DaemonConfigResponse, StatsUploadEntry, StatsUploadRequest, StatsUploadResponse,
};

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
    /// Server or network error. Back off before the next periodic attempt.
    ServerError,
    /// Nothing to upload, or the failure was local. Leave the clock alone.
    NoServerCall,
}

/// Bounded exponential backoff for periodic sync. Reset on a successful upload.
/// An empty queue does not count as success or failure.
#[derive(Debug, Clone, Default)]
pub struct SyncBackoff {
    failures: u32,
    next_attempt_at: Option<std::time::Instant>,
}

impl SyncBackoff {
    pub fn failures(&self) -> u32 {
        self.failures
    }

    pub fn should_attempt(&self, now: std::time::Instant) -> bool {
        self.next_attempt_at.is_none_or(|at| now >= at)
    }

    pub fn record_failure(&mut self, now: std::time::Instant) {
        self.failures = self.failures.saturating_add(1);
        self.next_attempt_at = Some(now + sync_backoff_delay(self.failures));
    }

    pub fn record_success(&mut self) {
        self.failures = 0;
        self.next_attempt_at = None;
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
            SyncAttempt::ServerError => self.record_failure(now),
            SyncAttempt::NoServerCall => {}
        }
    }
}

#[derive(Clone)]
pub struct SyncClient {
    config: SyncConfig,
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
        Self { http, config }
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
        validate_sync_server_url(&self.config.server_url)?;
        Ok(())
    }

    /// Fetch daemon configuration from the server (player_name, etc.).
    /// Called on startup when local config has no player_name.
    pub async fn fetch_daemon_config(
        &self,
    ) -> Result<DaemonConfigResponse, Box<dyn std::error::Error + Send + Sync>> {
        self.refuse_unsafe_transport()?;
        let url = format!("{}/api/stats/daemon-config", self.config.server_url);
        let resp = self
            .http
            .get(&url)
            .bearer_auth(&self.config.token)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("daemon-config fetch failed ({status}): {body}").into());
        }

        Ok(resp.json::<DaemonConfigResponse>().await?)
    }

    pub async fn upload_matches(
        &self,
        matches: &[PersonalMatch],
        deleted_sessions: &[String],
    ) -> Result<StatsUploadResponse, Box<dyn std::error::Error + Send + Sync>> {
        self.refuse_unsafe_transport()?;
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

        let url = format!("{}/api/stats/upload", self.config.server_url);
        let resp = self
            .http
            .post(&url)
            .bearer_auth(&self.config.token)
            .json(&StatsUploadRequest {
                matches: entries,
                deleted_sessions: deleted_sessions.to_vec(),
            })
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("Upload failed ({status}): {body}").into());
        }

        let result: StatsUploadResponse = resp.json().await?;
        tracing::info!(
            inserted = result.inserted,
            skipped = result.skipped,
            deleted = result.deleted,
            "stats upload complete"
        );
        Ok(result)
    }
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
                    let _ = std::io::Write::write_all(
                        &mut sock,
                        b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n",
                    );
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

        backoff.observe(SyncAttempt::ServerError, t0);
        assert_eq!(backoff.retry_after(t0), SYNC_BACKOFF_BASE);
        backoff.observe(SyncAttempt::Uploaded, t0);
        assert!(backoff.should_attempt(t0), "success resets the backoff");
        assert_eq!(backoff.failures(), 0);
    }
}
