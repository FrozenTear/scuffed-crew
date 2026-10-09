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
    /// error that may carry `Retry-After` as a wait floor. Other statuses,
    /// including a bare 400 or 422 from one request, are server errors and
    /// ignore `Retry-After`. The sync pass does not observe a 400 or 422
    /// that way: [`isolate_rejected`] splits the batch and quarantines the
    /// bad rows instead.
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

    /// HTTP 400 or 422: the payload was refused. Splitting the batch can
    /// isolate the bad rows. 5xx, transport errors, 401, 403, and 429 are not.
    pub fn is_payload_reject(&self) -> bool {
        matches!(self.status, Some(400) | Some(422))
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
        message: server_error_text(what, status.as_u16(), &body),
        status: Some(status.as_u16()),
        retry_after,
    }
}

/// Text to store and show for a non-2xx body. A JSON `error` field is the
/// server's message. Anything else is the body, clipped so a HTML error
/// page does not land in the desktop UI.
pub fn server_error_text(what: &str, status: u16, body: &str) -> String {
    let body = body.trim();
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(body)
        && let Some(err) = value
            .get("error")
            .and_then(|item| item.as_str())
            .map(str::trim)
            .filter(|err| !err.is_empty())
    {
        return clip_text(err);
    }
    if body.is_empty() {
        format!("{what} failed (HTTP {status})")
    } else {
        clip_text(body)
    }
}

fn clip_text(text: &str) -> String {
    const MAX: usize = 400;
    if text.chars().count() <= MAX {
        return text.to_string();
    }
    let end = text
        .char_indices()
        .nth(MAX)
        .map(|(index, _)| index)
        .unwrap_or(text.len());
    format!("{}...", &text[..end])
}

/// Logged when every probed row in a pass comes back 400 or 422.
pub const SERVER_REFUSING_UPLOADS: &str = "Server is refusing uploads";

/// How many HTTP calls one sync pass may spend isolating 400/422 rows.
/// `2 * ceil(log2(n)) + n` once the batch is larger than one row.
pub fn isolation_request_budget(n: usize) -> usize {
    if n <= 1 {
        return n;
    }
    let log2 = usize::BITS - (n - 1).leading_zeros();
    2 * (log2 as usize) + n
}

/// One row the server refused while splitting a batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RejectedUpload {
    pub index: usize,
    pub message: String,
}

/// Result of [`isolate_rejected`]. Indices are into the batch that was passed in.
#[derive(Debug)]
pub struct UploadIsolation {
    pub accepted: Vec<usize>,
    pub rejected: Vec<RejectedUpload>,
    pub deferred: Vec<usize>,
    /// 5xx, network, 401, 403, 429, or a server-wide 400/422 (every probed
    /// row refused, so nothing was quarantined). Splitting stops. Already
    /// accepted indexes stay accepted.
    pub stopped: Option<SyncUploadError>,
    pub requests: usize,
}

/// Upload `rows`. On HTTP 400 or 422, split the slice in half and retry
/// each half until a singleton is refused or the request budget runs out.
/// A singleton is quarantined only when some other row in the same pass
/// was accepted. If both halves of the first split are fully refused, or
/// every probed row comes back 400 or 422, nothing is quarantined: the
/// whole batch is deferred and [`UploadIsolation::stopped`] is
/// [`SERVER_REFUSING_UPLOADS`] (backoff, same as a 5xx).
/// A 5xx, a network error, 401, 403, or 429 does not split: the rest of
/// the pass stops and those indexes are [`UploadIsolation::deferred`].
pub async fn isolate_rejected<T, F, Fut>(rows: &[T], mut upload: F) -> UploadIsolation
where
    F: FnMut(&[T]) -> Fut,
    Fut: std::future::Future<Output = Result<StatsUploadResponse, SyncUploadError>>,
{
    let budget = isolation_request_budget(rows.len());
    let mut accepted = Vec::new();
    let mut rejected = Vec::new();
    let mut deferred = Vec::new();
    let mut stopped = None;
    let mut requests = 0usize;
    let mut halt = false;
    let mut stack = Vec::new();
    if !rows.is_empty() {
        stack.push(0..rows.len());
    }
    while let Some(range) = stack.pop() {
        if halt || requests >= budget {
            deferred.extend(range);
            continue;
        }
        requests += 1;
        match upload(&rows[range.start..range.end]).await {
            Ok(_) => accepted.extend(range),
            Err(err) if err.is_payload_reject() && range.len() > 1 => {
                let mid = range.start + range.len() / 2;
                stack.push(mid..range.end);
                stack.push(range.start..mid);
            }
            Err(err) if err.is_payload_reject() => {
                rejected.push(RejectedUpload {
                    index: range.start,
                    message: err.message,
                });
            }
            Err(err) => {
                deferred.extend(range);
                stopped = Some(err);
                halt = true;
            }
        }
    }
    // No accepted row means the 400/422s were not single bad payloads.
    // A validator change refuses every subset, including both halves of
    // the first split. Keep the batch queued and back off.
    if accepted.is_empty() && !rejected.is_empty() {
        let mut queued: Vec<usize> = rejected.iter().map(|row| row.index).collect();
        queued.append(&mut deferred);
        queued.sort_unstable();
        deferred = queued;
        if stopped.is_none() {
            stopped = Some(server_wide_refusal(&rejected[0].message));
        }
        rejected.clear();
    }
    UploadIsolation {
        accepted,
        rejected,
        deferred,
        stopped,
        requests,
    }
}

fn server_wide_refusal(detail: &str) -> SyncUploadError {
    SyncUploadError {
        message: format!("{SERVER_REFUSING_UPLOADS}: {detail}"),
        status: None,
        retry_after: None,
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

/// True when this row must stay on the machine until the member picks
/// a map or hero. See [`crate::parse::review_suspect_fields`].
pub fn row_needs_review(row: &PersonalMatch) -> bool {
    !crate::parse::review_suspect_fields(row.display_map_name(), &row.game_mode, row.display_hero())
        .is_empty()
}

/// The JSON body `upload_matches` posts. A pure function so tests can pin
/// the exact bytes (for example, that shadow mode leaves them unchanged).
///
/// A row whose map is empty or unrecognised, whose sent mode would be
/// empty, or whose hero is `Unknown` is left out. The server stores a
/// required string for `map_name` and `hero`, so an empty map and the
/// literal `Unknown` are what showed up in stats. Null and omitted `hero`
/// both fail decode, so the match is omitted instead of either of those.
pub fn upload_request(
    matches: &[PersonalMatch],
    deleted_sessions: &[String],
) -> StatsUploadRequest {
    let entries: Vec<StatsUploadEntry> = matches
        .iter()
        .filter(|m| !row_needs_review(m))
        // Upload the effective (corrected-if-present, else OCR) values so
        // server aggregates and the leaderboard reflect manual fixes, and
        // flag edited rows for the site badge. The immutable OCR reads stay
        // local; the transparency detail lives in the tracker GUI.
        .filter_map(|m| {
            let hero = m.display_hero().to_string();
            let map_name = m.display_map_name().to_string();
            let game_mode = crate::parse::uploaded_game_mode(&map_name, &m.game_mode);
            if crate::parse::upload_identity_blank(&map_name, &game_mode, &hero) {
                return None;
            }
            Some(StatsUploadEntry {
                session_id: m.session_id.clone(),
                hero,
                map_name,
                game_mode,
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
                recognizer: m.stored_recognizer().to_string(),
                suspect_fields: crate::reader_apply::upload_suspect_fields(&m.suspect_fields),
            })
        })
        .collect();
    StatsUploadRequest {
        matches: entries,
        deleted_sessions: deleted_sessions.to_vec(),
    }
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

    fn played() -> surrealdb_types::Datetime {
        surrealdb_types::Datetime::from(
            chrono::DateTime::parse_from_rfc3339("2026-07-01T20:00:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
        )
    }

    fn row(hero: &str, map: &str, mode: &str) -> PersonalMatch {
        PersonalMatch {
            id: None,
            hero: hero.into(),
            map_name: map.into(),
            game_mode: mode.into(),
            role: "Support".into(),
            outcome: "victory".into(),
            elims: 4,
            deaths: 1,
            assists: 2,
            damage: 1000,
            healing: 4000,
            mitigation: 0,
            played_at: played(),
            synced: false,
            sync_rev: 0,
            upload_reject: None,
            session_id: "sess-hold".into(),
            corrected_hero: None,
            corrected_role: None,
            corrected_map_name: None,
            corrected_outcome: None,
            corrected_elims: None,
            corrected_deaths: None,
            corrected_assists: None,
            corrected_damage: None,
            corrected_healing: None,
            corrected_mitigation: None,
            edited_fields: Vec::new(),
            edited_at: None,
            heroes_played: Vec::new(),
            segment_resolutions: Vec::new(),
            recognizer: String::new(),
            suspect_fields: Vec::new(),
        }
    }

    fn body_of(rows: &[PersonalMatch]) -> String {
        serde_json::to_string(&upload_request(rows, &[])).unwrap()
    }

    #[test]
    fn no_map_read_never_sends_an_empty_map_name() {
        let missed = row("Ana", "", "");
        assert_eq!(
            crate::parse::review_suspect_fields(
                missed.display_map_name(),
                &missed.game_mode,
                missed.display_hero()
            ),
            vec!["map", "mode"]
        );
        assert!(row_needs_review(&missed));
        let unrecognised = row("Ana", "Not a map", "");
        assert!(row_needs_review(&unrecognised));
        let ready = row("Ana", "Busan", "Control");
        let body = body_of(&[missed, unrecognised, ready]);
        assert!(!body.contains("\"map_name\":\"\""));
        assert!(!body.contains("Not a map"));
        let parsed: scuffed_types::api::StatsUploadRequest = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed.matches.len(), 1);
        assert_eq!(parsed.matches[0].map_name, "Busan");
        assert_eq!(parsed.matches[0].game_mode, "Control");
        assert_eq!(parsed.matches[0].hero, "Ana");
    }

    #[test]
    fn empty_game_mode_and_unknown_hero_are_not_sent() {
        let mode_only = row("Ana", "Busan", "");
        let mode_body = body_of(&[mode_only]);
        assert!(!mode_body.contains("\"game_mode\":\"\""));
        let mode_parsed: scuffed_types::api::StatsUploadRequest =
            serde_json::from_str(&mode_body).unwrap();
        assert_eq!(mode_parsed.matches.len(), 1);
        assert_eq!(mode_parsed.matches[0].game_mode, "Control");
        assert_eq!(mode_parsed.matches[0].map_name, "Busan");

        let blank_mode = row("Ana", "", "");
        let unknown_hero = row("Unknown", "Busan", "");
        let lower = row("unknown", "Ilios", "Control");
        let body = body_of(&[blank_mode, unknown_hero, lower]);
        assert!(
            upload_request(&[row("Unknown", "Busan", "")], &[])
                .matches
                .is_empty()
        );
        assert!(!body.contains("\"game_mode\":\"\""));
        assert!(!body.contains("\"map_name\":\"\""));
        assert!(!body.contains("Unknown"));
        assert!(!body.contains("unknown"));
        assert!(!body.contains("Ilios"));
        let parsed: scuffed_types::api::StatsUploadRequest = serde_json::from_str(&body).unwrap();
        assert!(parsed.matches.is_empty());
    }

    #[test]
    fn picked_map_and_hero_clear_the_hold() {
        let mut held = row("Unknown", "", "");
        assert_eq!(
            crate::parse::review_suspect_fields(
                held.display_map_name(),
                &held.game_mode,
                held.display_hero()
            ),
            vec!["map", "mode", "hero"]
        );
        assert!(upload_request(&[held.clone()], &[]).matches.is_empty());

        held.corrected_map_name = Some("King's Row".into());
        held.game_mode = crate::parse::stored_game_mode("King's Row");
        held.corrected_hero = Some("Ana".into());
        held.corrected_role = Some(crate::parse::guess_role_public("Ana"));
        assert!(!row_needs_review(&held));
        let parsed = upload_request(&[held], &[]);
        assert_eq!(parsed.matches.len(), 1);
        assert_eq!(parsed.matches[0].map_name, "King's Row");
        assert_eq!(parsed.matches[0].game_mode, "Hybrid");
        assert_eq!(parsed.matches[0].hero, "Ana");
        assert_eq!(parsed.matches[0].role, "Support");
        let body = serde_json::to_string(&parsed).unwrap();
        assert!(!body.contains("\"map_name\":\"\""));
        assert!(!body.contains("\"game_mode\":\"\""));
        assert!(!body.contains("Unknown"));
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

    #[test]
    fn server_error_text_prefers_the_json_error_field() {
        assert_eq!(
            server_error_text(
                "Upload",
                400,
                r#"{"error":"matches[0]: hero is not allowed"}"#
            ),
            "matches[0]: hero is not allowed"
        );
        assert_eq!(
            server_error_text("Upload", 422, "  plain refusal  "),
            "plain refusal"
        );
        assert_eq!(
            server_error_text("Upload", 500, ""),
            "Upload failed (HTTP 500)"
        );
    }

    #[tokio::test]
    async fn upload_400_keeps_the_server_error_field() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let body = r#"{"error":"matches[0]: hero is not allowed"}"#;
        let response = format!(
            "HTTP/1.1 400 no\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let response_bytes = response.into_bytes();
        let server = std::thread::spawn(move || exchange_http(listener, &response_bytes));
        let c = client(&format!("http://127.0.0.1:{port}"));
        let err = c.upload_matches(&[], &[]).await.expect_err("400");
        let _ = server.join().expect("server");
        assert_eq!(err.status, Some(400));
        assert_eq!(err.message, "matches[0]: hero is not allowed");
        assert!(err.is_payload_reject());
        assert!(matches!(
            err.attempt(),
            SyncAttempt::ServerError { retry_after: None }
        ));
    }

    fn payload_reject(status: u16) -> SyncUploadError {
        SyncUploadError {
            message: "matches[0]: hero is not allowed".into(),
            status: Some(status),
            retry_after: None,
        }
    }

    #[tokio::test]
    async fn one_bad_row_uploads_the_rest_of_the_batch() {
        for status in [400_u16, 422] {
            let n = 16usize;
            let bad = 5usize;
            let uploaded = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
            let rows: Vec<usize> = (0..n).collect();
            let isolation = isolate_rejected(&rows, |batch| {
                let uploaded = std::sync::Arc::clone(&uploaded);
                let bad_here = batch.contains(&bad);
                let ids = batch.to_vec();
                async move {
                    if bad_here {
                        Err(payload_reject(status))
                    } else {
                        uploaded.lock().unwrap().extend(ids);
                        Ok(scuffed_types::api::StatsUploadResponse {
                            inserted: 1,
                            skipped: 0,
                            deleted: 0,
                        })
                    }
                }
            })
            .await;
            let mut got = uploaded.lock().unwrap().clone();
            got.sort_unstable();
            got.dedup();
            let expected: Vec<usize> = (0..n).filter(|index| *index != bad).collect();
            assert_eq!(got, expected, "status {status} should upload N-1 rows");
            assert_eq!(isolation.rejected.len(), 1);
            assert_eq!(isolation.rejected[0].index, bad);
            assert_eq!(
                isolation.rejected[0].message,
                "matches[0]: hero is not allowed"
            );
            assert!(isolation.deferred.is_empty());
            assert!(isolation.stopped.is_none());
            assert!(
                isolation.requests <= isolation_request_budget(n),
                "status {status} used {} requests, budget {}",
                isolation.requests,
                isolation_request_budget(n)
            );
            assert!(isolation.requests > 1, "the batch must be split");
        }
    }

    #[tokio::test]
    async fn payload_reject_split_stays_inside_the_request_budget() {
        let n = 32usize;
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let rows: Vec<u32> = (0..n as u32).collect();
        let isolation = isolate_rejected(&rows, |_| {
            calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            async {
                Err(SyncUploadError {
                    message: "nope".into(),
                    status: Some(422),
                    retry_after: None,
                })
            }
        })
        .await;
        let budget = isolation_request_budget(n);
        assert!(isolation.requests <= budget);
        assert_eq!(
            isolation.requests,
            calls.load(std::sync::atomic::Ordering::SeqCst)
        );
        assert!(
            isolation.requests > 1,
            "the batch is split before giving up"
        );
        assert!(isolation.accepted.is_empty());
        assert!(
            isolation.rejected.is_empty(),
            "a pass that accepts nothing quarantines nothing"
        );
        assert_eq!(isolation.deferred, (0..n).collect::<Vec<_>>());
        let stopped = isolation.stopped.expect("server-wide refusal");
        assert!(stopped.message.starts_with(SERVER_REFUSING_UPLOADS));
        assert!(stopped.message.contains("nope"), "{}", stopped.message);
        assert!(matches!(
            stopped.attempt(),
            SyncAttempt::ServerError { retry_after: None }
        ));
    }

    #[tokio::test]
    async fn one_bad_row_in_eight_uploads_seven() {
        let n = 8usize;
        let bad = 3usize;
        let uploaded = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let rows: Vec<usize> = (0..n).collect();
        let isolation = isolate_rejected(&rows, |batch| {
            let uploaded = std::sync::Arc::clone(&uploaded);
            let bad_here = batch.contains(&bad);
            let ids = batch.to_vec();
            async move {
                if bad_here {
                    Err(payload_reject(400))
                } else {
                    let inserted = ids.len() as u32;
                    uploaded.lock().unwrap().extend(ids);
                    Ok(scuffed_types::api::StatsUploadResponse {
                        inserted,
                        skipped: 0,
                        deleted: 0,
                    })
                }
            }
        })
        .await;
        let mut got = uploaded.lock().unwrap().clone();
        got.sort_unstable();
        got.dedup();
        assert_eq!(got.len(), n - 1);
        assert!(!got.contains(&bad));
        assert_eq!(isolation.rejected.len(), 1);
        assert_eq!(isolation.rejected[0].index, bad);
        assert!(isolation.stopped.is_none());
        assert_eq!(isolation.accepted.len(), n - 1);
    }

    #[tokio::test]
    async fn bad_rows_in_both_halves_still_upload_the_rest() {
        let n = 8usize;
        let bad = [0usize, 4];
        let uploaded = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let rows: Vec<usize> = (0..n).collect();
        let isolation = isolate_rejected(&rows, |batch| {
            let uploaded = std::sync::Arc::clone(&uploaded);
            let bad_here = batch.iter().any(|index| bad.contains(index));
            let ids = batch.to_vec();
            async move {
                if bad_here {
                    Err(payload_reject(400))
                } else {
                    uploaded.lock().unwrap().extend(ids);
                    Ok(scuffed_types::api::StatsUploadResponse {
                        inserted: 1,
                        skipped: 0,
                        deleted: 0,
                    })
                }
            }
        })
        .await;
        let mut got = uploaded.lock().unwrap().clone();
        got.sort_unstable();
        got.dedup();
        assert_eq!(got, vec![1, 2, 3, 5, 6, 7]);
        assert_eq!(isolation.rejected.len(), 2);
        assert!(isolation.stopped.is_none());
    }

    #[tokio::test]
    async fn server_error_mid_split_keeps_the_accepted_half_and_retry_after() {
        // Left half of 8 succeeds. The right half splits, then a 503 lands
        // on its left child while the sibling is still queued. Further
        // requests must not run: if `halt` stayed false, 6..8 would be sent.
        let rows: Vec<usize> = (0..8).collect();
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let retry = std::time::Duration::from_secs(30);
        let isolation = isolate_rejected(&rows, |batch| {
            calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let first = batch[0];
            let len = batch.len();
            let wait = retry;
            async move {
                if len == 8 {
                    Err(payload_reject(400))
                } else if first == 0 {
                    Ok(scuffed_types::api::StatsUploadResponse {
                        inserted: len as u32,
                        skipped: 0,
                        deleted: 0,
                    })
                } else if len == 4 {
                    Err(payload_reject(422))
                } else if first == 4 && len == 2 {
                    Err(SyncUploadError {
                        message: "unavailable".into(),
                        status: Some(503),
                        retry_after: Some(wait),
                    })
                } else {
                    panic!("request after 503 must not be sent, got {first}..+{len}");
                }
            }
        })
        .await;
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 4);
        assert_eq!(isolation.requests, 4);
        assert_eq!(isolation.accepted, vec![0, 1, 2, 3]);
        assert!(isolation.rejected.is_empty());
        assert_eq!(isolation.deferred, vec![4, 5, 6, 7]);
        let stopped = isolation.stopped.expect("503 stops the pass");
        assert_eq!(stopped.status, Some(503));
        assert_eq!(stopped.retry_after, Some(retry));
        assert!(matches!(
            stopped.attempt(),
            SyncAttempt::ServerError {
                retry_after: Some(wait)
            } if wait == retry
        ));
    }

    #[tokio::test]
    async fn batch_sizes_and_several_bad_rows() {
        // N=1 refused cannot be told from a server-wide refusal.
        let alone = isolate_case(1, &[0]).await;
        assert!(alone.accepted.is_empty());
        assert!(alone.rejected.is_empty());
        assert_eq!(alone.deferred, vec![0]);
        assert!(
            alone
                .stopped
                .expect("n=1")
                .message
                .starts_with(SERVER_REFUSING_UPLOADS)
        );

        let one_ok = isolate_case(1, &[]).await;
        assert_eq!(one_ok.accepted, vec![0]);
        assert!(one_ok.rejected.is_empty());
        assert!(one_ok.stopped.is_none());
        assert_eq!(one_ok.requests, 1);

        // N=2, one bad row.
        let pair = isolate_case(2, &[1]).await;
        assert_eq!(pair.accepted, vec![0]);
        assert_eq!(pair.rejected.len(), 1);
        assert_eq!(pair.rejected[0].index, 1);
        assert!(pair.stopped.is_none());

        // N=2, both refused.
        let both = isolate_case(2, &[0, 1]).await;
        assert!(both.accepted.is_empty());
        assert!(both.rejected.is_empty());
        assert_eq!(both.deferred, vec![0, 1]);
        assert!(both.stopped.is_some());

        // Odd length, one bad row in the longer half.
        let odd = isolate_case(5, &[2]).await;
        assert_eq!(odd.rejected.len(), 1);
        assert_eq!(odd.rejected[0].index, 2);
        assert_eq!(odd.accepted.len(), 4);
        assert!(odd.stopped.is_none());
        assert!(!odd.accepted.contains(&2));

        // Several bad rows.
        let many = isolate_case(7, &[1, 4, 6]).await;
        let mut rejected: Vec<usize> = many.rejected.iter().map(|row| row.index).collect();
        rejected.sort_unstable();
        assert_eq!(rejected, vec![1, 4, 6]);
        assert_eq!(many.accepted.len(), 4);
        assert!(many.stopped.is_none());
        for index in [1, 4, 6] {
            assert!(!many.accepted.contains(&index));
        }
    }

    async fn isolate_case(n: usize, bad: &[usize]) -> UploadIsolation {
        let rows: Vec<usize> = (0..n).collect();
        isolate_rejected(&rows, |batch| {
            let bad_here = batch.iter().any(|index| bad.contains(index));
            let inserted = batch.len() as u32;
            async move {
                if bad_here {
                    Err(payload_reject(400))
                } else {
                    Ok(scuffed_types::api::StatsUploadResponse {
                        inserted,
                        skipped: 0,
                        deleted: 0,
                    })
                }
            }
        })
        .await
    }

    #[tokio::test]
    async fn server_and_auth_errors_do_not_split_the_batch() {
        let cases = [
            None,
            Some(500_u16),
            Some(503),
            Some(401),
            Some(403),
            Some(429),
        ];
        for status in cases {
            let calls = std::sync::atomic::AtomicUsize::new(0);
            let rows = ["a", "b", "c", "d"];
            let isolation = isolate_rejected(&rows, |_| {
                calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let code = status;
                async move {
                    Err(SyncUploadError {
                        message: "down".into(),
                        status: code,
                        retry_after: None,
                    })
                }
            })
            .await;
            assert_eq!(
                calls.load(std::sync::atomic::Ordering::SeqCst),
                1,
                "{status:?} must not split"
            );
            assert!(isolation.accepted.is_empty());
            assert!(isolation.rejected.is_empty());
            assert_eq!(isolation.deferred, vec![0, 1, 2, 3]);
            let attempt = isolation.stopped.expect("stopped").attempt();
            match status {
                Some(401) | Some(403) => {
                    assert!(matches!(attempt, SyncAttempt::AuthRejected));
                }
                Some(429) => {
                    assert!(matches!(attempt, SyncAttempt::RateLimited { .. }));
                }
                _ => {
                    assert!(matches!(attempt, SyncAttempt::ServerError { .. }));
                }
            }
        }
    }
}
