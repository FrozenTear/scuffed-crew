//! Upload batching: the 413 caps ([`BatchCaps`]) and the bisection of a
//! refused 400/422 batch ([`isolate_rejected`]).

use scuffed_types::api::StatsUploadResponse;

use super::errors::SyncUploadError;

/// Matches per stats upload. The server rejects a larger body with HTTP 413
/// `batch_too_large`. This client stays at or under the cap, and shrinks
/// further when a 413 names a smaller one. Same value as
/// [`scuffed_types::STATS_UPLOAD_MAX_MATCHES`].
pub const SYNC_UPLOAD_MAX_MATCHES: usize = scuffed_types::STATS_UPLOAD_MAX_MATCHES;

/// Deleted session ids per stats upload. Same 413 path as
/// [`SYNC_UPLOAD_MAX_MATCHES`]. Same value as
/// [`scuffed_types::STATS_UPLOAD_MAX_DELETED_SESSIONS`].
pub const SYNC_UPLOAD_MAX_DELETES: usize = scuffed_types::STATS_UPLOAD_MAX_DELETED_SESSIONS;

/// Upload chunk sizes for one process. Starts at the client caps and only
/// falls, for the life of the [`SyncClient`] that holds them.
///
/// [`SyncClient`]: super::SyncClient
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatchCaps {
    pub matches: usize,
    pub deletes: usize,
}

impl Default for BatchCaps {
    fn default() -> Self {
        Self {
            matches: SYNC_UPLOAD_MAX_MATCHES,
            deletes: SYNC_UPLOAD_MAX_DELETES,
        }
    }
}

/// Drop the in-pass caps to the server's 413 values. A missing field keeps
/// the current cap. Values below 1 become 1. The cap never rises. Returns
/// whether either cap actually fell, so the caller can retry once and stop
/// when the server repeats the same limits.
pub fn lower_batch_limits(
    match_limit: &mut usize,
    delete_limit: &mut usize,
    max_matches: Option<u64>,
    max_deleted_sessions: Option<u64>,
) -> bool {
    let next_matches = lowered_cap(*match_limit, max_matches);
    let next_deletes = lowered_cap(*delete_limit, max_deleted_sessions);
    let progress = next_matches < *match_limit || next_deletes < *delete_limit;
    *match_limit = next_matches;
    *delete_limit = next_deletes;
    progress
}

fn lowered_cap(current: usize, server_max: Option<u64>) -> usize {
    let Some(raw) = server_max else {
        return current;
    };
    let asked = usize::try_from(raw).unwrap_or(usize::MAX).max(1);
    asked.min(current)
}

/// Logged when a pass accepts no row. The rows stay queued and the clock
/// backs off, the same shape as a 5xx.
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

/// True when the server named one row of this request (`matches[<digits>]:`).
/// The index is only valid for that body, so a named row is bisected rather
/// than trusted as a stable id. A 400 or 422 without this prefix is a refusal
/// of the request as a whole.
pub fn payload_names_a_row(message: &str) -> bool {
    let Some(rest) = message.trim().strip_prefix("matches[") else {
        return false;
    };
    let Some(end) = rest.find("]:") else {
        return false;
    };
    let index = &rest[..end];
    !index.is_empty() && index.bytes().all(|byte| byte.is_ascii_digit())
}

/// One row the server refused while splitting a batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RejectedUpload {
    pub index: usize,
    /// Server message, still carrying a `matches[n]:` prefix when the body had one.
    pub message: String,
    /// JSON `code` when the body sent one. Absent for a plain validation message.
    pub code: Option<String>,
}

/// Result of [`isolate_rejected`]. Indices are into the batch that was passed in.
#[derive(Debug)]
pub struct UploadIsolation {
    pub accepted: Vec<usize>,
    pub rejected: Vec<RejectedUpload>,
    pub deferred: Vec<usize>,
    /// 5xx, network, 401, 403, 409, 429, 413, or a server-wide 400/422 (no
    /// row in the pass was accepted, so nothing was quarantined). Splitting
    /// stops. Already accepted indexes stay accepted. A 409 never quarantines.
    pub stopped: Option<SyncUploadError>,
    pub requests: usize,
}

/// Upload `rows`. On HTTP 400 or 422 that names a row (`matches[n]:`), split
/// the slice in half and retry each half until the bad singletons are
/// isolated. A singleton is quarantined only when some other row in the
/// same chunk was accepted. If no row was accepted, nothing is quarantined:
/// the batch is deferred and [`UploadIsolation::stopped`] is
/// [`SERVER_REFUSING_UPLOADS`] (backoff, same as a 5xx).
///
/// A 400 or 422 that does not name a row is not bisected. One single-row
/// probe is sent. If that row fails with 400 or 422, only that row is
/// blamed and the rest of the slice is retried, whether or not the body
/// names a row. A good row after the bad one still uploads. The same end
/// guard applies: when the chunk accepts nothing, blamed rows are deferred
/// instead of quarantined.
///
/// Request count stays at or under [`isolation_request_budget`]
/// (about `2 * log2(n) + n`).
///
/// A 5xx, a network error, 401, 403, 409, 413, or 429 does not split. 429
/// keeps `Retry-After` on [`UploadIsolation::stopped`]. When the chunk
/// accepted nothing and some rows were already refused, that 429 is reported
/// as [`SERVER_REFUSING_UPLOADS`] and the wait stays on the error as a
/// backoff floor. The rest of the chunk stops and those indexes are
/// [`UploadIsolation::deferred`]. 409 `stats_delete_pending` is the same
/// shape: one request, nothing quarantined.
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
            Err(err)
                if err.is_payload_reject()
                    && range.len() > 1
                    && payload_names_a_row(&err.message) =>
            {
                let mid = range.start + range.len() / 2;
                stack.push(mid..range.end);
                stack.push(range.start..mid);
            }
            Err(err) if err.is_payload_reject() && range.len() > 1 => {
                if requests >= budget {
                    deferred.extend(range);
                    continue;
                }
                // One single-row send. A 400/422 blames only that row.
                // The rest of the slice is retried.
                requests += 1;
                let probe = range.start..range.start + 1;
                match upload(&rows[probe]).await {
                    Err(one) if one.is_payload_reject() => {
                        rejected.push(rejected_upload(range.start, &one));
                        let rest = range.start + 1..range.end;
                        if !rest.is_empty() {
                            stack.push(rest);
                        }
                    }
                    Ok(_) => {
                        accepted.push(range.start);
                        let rest = range.start + 1..range.end;
                        if !rest.is_empty() {
                            stack.push(rest);
                        }
                    }
                    Err(other) => {
                        deferred.extend(range);
                        stopped = Some(other);
                        halt = true;
                    }
                }
            }
            Err(err) if err.is_payload_reject() => {
                rejected.push(rejected_upload(range.start, &err));
            }
            Err(err) => {
                deferred.extend(range);
                stopped = Some(err);
                halt = true;
            }
        }
    }
    // No accepted row means the 400/422s were not single bad payloads
    // among good ones. A server-wide refusal (unknown recognizer id, for
    // example) refuses every subset. Keep the batch queued and back off.
    // A 429 that stops the walk is the same refusal: the per-token limit
    // ran out while every row was still being refused. Count that as a
    // failure and keep Retry-After as a floor. 401, 403, 409, and 413 stay.
    if accepted.is_empty() && !rejected.is_empty() {
        let mut queued: Vec<usize> = rejected.iter().map(|row| row.index).collect();
        queued.append(&mut deferred);
        queued.sort_unstable();
        deferred = queued;
        let rate_limited = stopped.as_ref().is_some_and(|err| err.status == Some(429));
        if stopped.is_none() || rate_limited {
            let retry_after = stopped.as_ref().and_then(|err| err.retry_after);
            stopped = Some(server_wide_refusal(&rejected[0].message, retry_after));
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

fn server_wide_refusal(detail: &str, retry_after: Option<std::time::Duration>) -> SyncUploadError {
    SyncUploadError {
        message: format!("{SERVER_REFUSING_UPLOADS}: {detail}"),
        retry_after,
        ..SyncUploadError::plain("")
    }
}

fn rejected_upload(index: usize, err: &SyncUploadError) -> RejectedUpload {
    RejectedUpload {
        index,
        message: err.message.clone(),
        code: err.code.clone(),
    }
}
