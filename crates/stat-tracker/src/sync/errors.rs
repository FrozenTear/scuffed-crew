//! [`SyncUploadError`] and the parsing of a server error body. No reqwest:
//! [`super::client`] reads the response and hands the body here.

use super::auth_pause::STATS_DELETE_PENDING_CODE;
use super::backoff::SyncAttempt;

/// Upload (or the transport under it) failed. `status` / `retry_after` are
/// set when the server actually answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncUploadError {
    pub message: String,
    pub status: Option<u16>,
    pub retry_after: Option<std::time::Duration>,
    /// Machine code from the JSON body: the `code` field, or `error` when
    /// that string is `batch_too_large`, `token_expired`, `rate_limited`,
    /// or `stats_delete_pending`.
    pub code: Option<String>,
    /// `max_matches` from a 413 `batch_too_large` body.
    pub max_matches: Option<u64>,
    /// `max_deleted_sessions` from a 413 `batch_too_large` body.
    pub max_deleted_sessions: Option<u64>,
}

impl SyncUploadError {
    pub fn plain(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            status: None,
            retry_after: None,
            code: None,
            max_matches: None,
            max_deleted_sessions: None,
        }
    }

    /// 401 `token_expired` pauses sync with the relink copy. Other 401 and
    /// 403 responses pause with the rejected-token copy. 409
    /// `stats_delete_pending` stops the pass until a new token is saved. 429 is
    /// rate-limit backoff and does not grow the failure streak. A 413
    /// `batch_too_large` that names a smaller cap is shrunk and resent inside
    /// the pass, so the clock never sees it. One the pass cannot shrink is
    /// [`SyncAttempt::BatchTooLarge`] and does back off. 503 may carry
    /// `Retry-After` as a wait floor. Other statuses, including a bare 400,
    /// 422, or any other 409, are server errors and ignore `Retry-After`.
    /// A refusal with no HTTP status keeps `Retry-After` as a floor under
    /// the exponential backoff. The sync pass does not observe a 400 or 422
    /// that way: [`isolate_rejected`] splits the batch. A bad row is
    /// quarantined only when some other row in that chunk was accepted. If
    /// none was, the rows stay queued and the clock backs off. A 409 is
    /// never split.
    ///
    /// [`isolate_rejected`]: super::isolate_rejected
    pub fn attempt(&self) -> SyncAttempt {
        if self.is_token_expired() {
            return SyncAttempt::TokenExpired;
        }
        if self.is_batch_too_large() {
            return SyncAttempt::BatchTooLarge;
        }
        if self.is_stats_delete_pending() {
            return SyncAttempt::StatsDeletePending;
        }
        match self.status {
            Some(401) | Some(403) => SyncAttempt::AuthRejected,
            Some(429) => SyncAttempt::RateLimited {
                retry_after: self.retry_after,
            },
            Some(503) => SyncAttempt::ServerError {
                retry_after: self.retry_after,
            },
            None => SyncAttempt::ServerError {
                retry_after: self.retry_after,
            },
            _ => SyncAttempt::ServerError { retry_after: None },
        }
    }

    /// HTTP 401 whose JSON code is `token_expired`.
    pub fn is_token_expired(&self) -> bool {
        self.status == Some(401) && self.code.as_deref() == Some("token_expired")
    }

    /// HTTP 413 whose JSON code is `batch_too_large`. The server wrote nothing
    /// for that request.
    pub fn is_batch_too_large(&self) -> bool {
        self.status == Some(413) && self.code.as_deref() == Some("batch_too_large")
    }

    /// HTTP 409 whose JSON code is `stats_delete_pending`. The server wrote
    /// nothing. The rows stay queued.
    pub fn is_stats_delete_pending(&self) -> bool {
        self.status == Some(409) && self.code.as_deref() == Some(STATS_DELETE_PENDING_CODE)
    }

    /// HTTP 400 or 422: the payload was refused. Splitting the batch can
    /// isolate the bad rows. 413 `batch_too_large` is not one of these; the
    /// sync pass shrinks the chunk and resends. 5xx, transport errors, 401,
    /// 403, 409, and 429 are not either.
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

pub(super) struct ParsedUploadError {
    pub(super) code: Option<String>,
    pub(super) max_matches: Option<u64>,
    pub(super) max_deleted_sessions: Option<u64>,
    pub(super) stats_delete_scheduled_at: Option<String>,
}

/// Pull `code` / `max_matches` / `max_deleted_sessions` out of an error body.
/// `error` is used as the code only for the machine codes the sync pass
/// branches on. A validation message stays in the display text.
pub(super) fn parse_upload_error_body(body: &str) -> ParsedUploadError {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(body) else {
        return ParsedUploadError {
            code: None,
            max_matches: None,
            max_deleted_sessions: None,
            stats_delete_scheduled_at: None,
        };
    };
    let code = value
        .get("code")
        .and_then(|item| item.as_str())
        .map(str::trim)
        .filter(|code| !code.is_empty())
        .map(str::to_string)
        .or_else(|| {
            value
                .get("error")
                .and_then(|item| item.as_str())
                .map(str::trim)
                .filter(|code| {
                    matches!(
                        *code,
                        "batch_too_large"
                            | "token_expired"
                            | "rate_limited"
                            | "stats_delete_pending"
                    )
                })
                .map(str::to_string)
        });
    ParsedUploadError {
        code,
        max_matches: json_u64(value.get("max_matches")),
        max_deleted_sessions: json_u64(value.get("max_deleted_sessions")),
        stats_delete_scheduled_at: json_text(value.get("stats_delete_scheduled_at")),
    }
}

fn json_text(value: Option<&serde_json::Value>) -> Option<String> {
    let text = value?.as_str()?.trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

fn json_u64(value: Option<&serde_json::Value>) -> Option<u64> {
    let value = value?;
    if let Some(n) = value.as_u64() {
        return Some(n);
    }
    if let Some(n) = value.as_i64() {
        return Some(u64::try_from(n.max(0)).unwrap_or(u64::MAX));
    }
    value.as_str().and_then(|text| text.trim().parse().ok())
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
