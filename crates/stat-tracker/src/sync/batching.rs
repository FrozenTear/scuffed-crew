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

/// Logged when refused rows are held ([`UploadIsolation::held`]), whether or
/// not an earlier chunk of the pass was accepted. The rows stay queued and
/// the clock backs off, the same shape as a 5xx.
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
    /// 5xx, network, 401, 403, 409, 429, 413, or a 400/422 that proved
    /// nothing about a row (see [`Self::held`]). Splitting stops. Already
    /// accepted indexes stay accepted. A 409 never quarantines.
    pub stopped: Option<SyncUploadError>,
    /// Rows refused on their own with no proof the refusal is about the
    /// row: the chunk accepted none, and either no proof covers the row
    /// ([`isolate_rejected_in_pass`]) or the refusal did not name it. They
    /// are also in [`Self::deferred`] and stay queued. See [`RefusedRows`].
    pub held: Vec<usize>,
    pub requests: usize,
}

/// How long a row the server accepted proves that it takes uploads. Inside
/// this window a row the server refuses on its own and by name is
/// quarantined even when its chunk and pass accept no row. Ten minutes
/// covers the passes that follow a 429 or a 5xx (a minute or two apart,
/// failure backoff at most 5 minutes). The cost is bounded by the window: a
/// server-wide refusal that starts inside it (a PC clock pushed more than 24
/// hours ahead, or a site deploy with a bug) can set aside the rows queued
/// then. They keep the site's reason and a Retry button, and the next
/// tracker version puts them back on the queue.
pub const ACCEPT_PROOF_WINDOW: std::time::Duration = std::time::Duration::from_secs(10 * 60);

/// Rows the server refused on their own with no proof the refusal is about
/// the row (see [`UploadIsolation::held`]). They are not quarantined,
/// because a server-wide refusal looks the same. The next pass sends them
/// after the other queued rows, so a good row behind them is tried first.
/// The server allows only a short burst of requests per token, so without
/// this a few refused rows at the head of the queue used up every pass and
/// the games behind them never uploaded. Keys are session ids (or the store
/// id for a row without one). Kept for the life of the [`SyncClient`], like
/// [`BatchCaps`].
///
/// It also keeps when the server last accepted a row. For
/// [`ACCEPT_PROOF_WINDOW`] after that, a row the server refuses on its own
/// and names in its message is quarantined right away, held before or not,
/// and whether or not this pass accepted a row. So a refused game left
/// alone after the good games went up is set aside instead of retried on
/// every pass. The time is for the URL and token it was earned with.
///
/// [`SyncClient`]: super::SyncClient
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RefusedRows {
    keys: std::collections::HashSet<String>,
    last_accepted: Option<std::time::Instant>,
}

impl RefusedRows {
    pub fn contains(&self, key: &str) -> bool {
        self.keys.contains(key)
    }

    pub fn insert(&mut self, key: String) {
        self.keys.insert(key);
    }

    pub fn remove(&mut self, key: &str) {
        self.keys.remove(key);
    }

    /// Forget rows that are no longer queued (uploaded, quarantined, or
    /// deleted), so the set never outgrows the queue.
    pub fn retain_queued<'a>(&mut self, queued: impl IntoIterator<Item = &'a str>) {
        let queued: std::collections::HashSet<&str> = queued.into_iter().collect();
        self.keys.retain(|key| queued.contains(key.as_str()));
    }

    /// Record that the server accepted a row at `at`.
    pub fn note_accepted(&mut self, at: std::time::Instant) {
        self.last_accepted = Some(at);
    }

    /// Drop the accept time, after the token or URL changed.
    pub fn forget_accepted(&mut self) {
        self.last_accepted = None;
    }

    /// True when the server accepted a row less than
    /// [`ACCEPT_PROOF_WINDOW`] before `now`.
    pub fn accepted_recently(&self, now: std::time::Instant) -> bool {
        self.last_accepted
            .is_some_and(|at| now.saturating_duration_since(at) < ACCEPT_PROOF_WINDOW)
    }
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
/// This is [`isolate_rejected_in_pass`] for a pass that has not had a row
/// accepted yet.
///
/// A 5xx, a network error, 401, 403, 409, 413, or 429 does not split. 429
/// keeps `Retry-After` on [`UploadIsolation::stopped`]. When the chunk
/// accepted nothing and some rows were already refused, that 429 is reported
/// as [`SERVER_REFUSING_UPLOADS`] and the wait stays on the error as a
/// backoff floor. The rest of the chunk stops and those indexes are
/// [`UploadIsolation::deferred`]. 409 `stats_delete_pending` is the same
/// shape: one request, nothing quarantined.
pub async fn isolate_rejected<T, F, Fut>(rows: &[T], upload: F) -> UploadIsolation
where
    F: FnMut(&[T]) -> Fut,
    Fut: std::future::Future<Output = Result<StatsUploadResponse, SyncUploadError>>,
{
    isolate_rejected_in_pass(rows, false, &[], upload).await
}

/// [`isolate_rejected`] for one chunk of a sync pass. `server_accepts` is
/// true when the server accepted a row recently: earlier in this pass, or
/// within [`ACCEPT_PROOF_WINDOW`] ([`RefusedRows`]). `requeued` lists the
/// rows of `rows` the server refused before and a tracker update put back on
/// the queue with that reason kept. Either one proves the refusal is about
/// the row, so when this chunk accepts no row, a covered row refused on its
/// own is still quarantined instead of staying queued. That proof is used
/// only when the refusal names the row (`matches[n]:`, see
/// [`payload_names_a_row`]). A refusal of the request as a whole, such as a
/// bad id in `deleted_sessions` that rides only on this chunk, says nothing
/// about the probed row, so that row is held and the pass backs off.
pub async fn isolate_rejected_in_pass<T, F, Fut>(
    rows: &[T],
    server_accepts: bool,
    requeued: &[usize],
    mut upload: F,
) -> UploadIsolation
where
    F: FnMut(&[T]) -> Fut,
    Fut: std::future::Future<Output = Result<StatsUploadResponse, SyncUploadError>>,
{
    let budget = isolation_request_budget(rows.len());
    let mut accepted = Vec::new();
    let mut rejected = Vec::new();
    let mut deferred = Vec::new();
    let mut stopped = None;
    let mut held = Vec::new();
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
    // among good ones. A server-wide refusal (a PC clock more than 24 hours
    // fast, or a site bug) refuses every subset. Keep the batch queued and
    // back off. A 429 that stops the walk is the same refusal: the per-token
    // limit ran out while every row was still being refused. Count that as
    // a failure and keep Retry-After as a floor. 401, 403, 409, and 413
    // stay. A row the server accepted recently, or an earlier refusal kept
    // on a requeued row, rules that out, but only for a refusal that names
    // the row.
    if accepted.is_empty() && !rejected.is_empty() {
        let (proven, unproven): (Vec<_>, Vec<_>) = rejected.into_iter().partition(|row| {
            payload_names_a_row(&row.message) && (server_accepts || requeued.contains(&row.index))
        });
        rejected = proven;
        if let Some(first) = unproven.first() {
            held = unproven.iter().map(|row| row.index).collect();
            let mut queued = held.clone();
            queued.append(&mut deferred);
            queued.sort_unstable();
            deferred = queued;
            let rate_limited = stopped.as_ref().is_some_and(|err| err.status == Some(429));
            if stopped.is_none() || rate_limited {
                let retry_after = stopped.as_ref().and_then(|err| err.retry_after);
                stopped = Some(server_wide_refusal(&first.message, retry_after));
            }
        }
    }
    UploadIsolation {
        accepted,
        rejected,
        deferred,
        stopped,
        held,
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
