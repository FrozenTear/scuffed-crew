//! Per-token governor and batch-cap flag for `POST /api/stats/upload`.
//!
//! The governor is always on. It is separate from the per-IP buckets: the
//! key is the BLAKE3 hash of the bearer token, the same hash stored on
//! `daemon_token`. Burst and replenish come from [`scuffed_types`]. A
//! request with no `Bearer` token is not counted; the handler still returns
//! the existing 401.
//!
//! The batch cap is off unless `STATS_UPLOAD_BATCH_CAP` classifies the same
//! way as `PRODUCTION` ([`scuffed_auth::production_value_enabled`]).

use std::sync::Arc;
use std::time::Duration;

#[cfg(test)]
use std::sync::Mutex;
#[cfg(test)]
use std::sync::atomic::{AtomicU8, Ordering};

use axum::Json;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use governor::clock::Clock;
use governor::{Quota, RateLimiter};
use scuffed_auth::crypto::hash_session_token;
use scuffed_types::{STATS_UPLOAD_TOKEN_BURST, STATS_UPLOAD_TOKEN_REPLENISH_SECS};

/// `0` follows the env var, `1` forces the cap off, `2` forces it on.
///
/// Unit tests that flip the cap hold [`CAP_TEST_LOCK`] for the whole case.
/// Other uploads in that process are under the cap, so they still succeed.
#[cfg(test)]
static CAP_OVERRIDE: AtomicU8 = AtomicU8::new(0);

#[cfg(test)]
static CAP_TEST_LOCK: Mutex<()> = Mutex::new(());

/// Holds the batch-cap override until dropped. Not `Send` to keep the guard
/// on the test thread, outside the async request.
#[cfg(test)]
pub(crate) struct BatchCapForce {
    _guard: std::sync::MutexGuard<'static, ()>,
}

#[cfg(test)]
pub(crate) fn force_batch_cap(on: bool) -> BatchCapForce {
    let guard = CAP_TEST_LOCK.lock().unwrap_or_else(|err| err.into_inner());
    CAP_OVERRIDE.store(if on { 2 } else { 1 }, Ordering::SeqCst);
    BatchCapForce { _guard: guard }
}

#[cfg(test)]
impl Drop for BatchCapForce {
    fn drop(&mut self) {
        CAP_OVERRIDE.store(0, Ordering::SeqCst);
    }
}

/// Classify one `STATS_UPLOAD_BATCH_CAP` value.
///
/// `None` (unset) is off. Empty, `0`, `false`, `no`, and `off` (any ASCII
/// case, surrounding whitespace ignored) are off. Any other non-empty value
/// is on. Same arms as [`scuffed_auth::production_value_enabled`].
pub fn batch_cap_from_env_value(value: Option<&str>) -> bool {
    match value {
        None => false,
        Some(value) => scuffed_auth::production_value_enabled(value),
    }
}

/// Live read of `STATS_UPLOAD_BATCH_CAP`. Unset is off.
pub fn stats_upload_batch_cap_enabled() -> bool {
    #[cfg(test)]
    match CAP_OVERRIDE.load(Ordering::SeqCst) {
        1 => return false,
        2 => return true,
        _ => {}
    }
    match std::env::var("STATS_UPLOAD_BATCH_CAP") {
        Ok(value) => batch_cap_from_env_value(Some(&value)),
        Err(_) => false,
    }
}

/// One decision of the upload governor: `Ok` spends a cell, `Err` is `Retry-After` seconds.
type TokenAllowance = dyn Fn(&str) -> Result<(), u64> + Send + Sync;

/// Keyed governor for upload requests. Cloned cheaply; buckets are shared.
#[derive(Clone)]
pub struct StatsUploadGovernor {
    check: Arc<TokenAllowance>,
}

impl StatsUploadGovernor {
    pub fn new() -> Self {
        let burst = std::num::NonZeroU32::new(STATS_UPLOAD_TOKEN_BURST).expect("burst is non-zero");
        let quota = Quota::with_period(Duration::from_secs(STATS_UPLOAD_TOKEN_REPLENISH_SECS))
            .expect("replenish period is non-zero")
            .allow_burst(burst);
        let limiter = RateLimiter::keyed(quota);
        Self {
            check: Arc::new(move |token_hash: &str| {
                let key = token_hash.to_owned();
                match limiter.check_key(&key) {
                    Ok(_) => Ok(()),
                    Err(not_until) => {
                        let wait = not_until.wait_time_from(limiter.clock().now());
                        Err(retry_after_secs(wait))
                    }
                }
            }),
        }
    }

    fn allow(&self, token_hash: &str) -> Result<(), u64> {
        (self.check)(token_hash)
    }
}

/// Whole seconds until one cell is free. A fractional wait rounds up so a
/// client that honors `Retry-After` does not come back early. At least 1.
fn retry_after_secs(wait: Duration) -> u64 {
    let secs = wait.as_secs();
    let rounded = if wait.subsec_nanos() == 0 {
        secs
    } else {
        secs.saturating_add(1)
    };
    rounded.max(1)
}

fn bearer_token_hash(headers: &HeaderMap) -> Option<String> {
    let raw = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .filter(|token| !token.is_empty())?;
    Some(hash_session_token(raw))
}

/// Axum layer state for [`limit_stats_upload`].
pub async fn limit_stats_upload(
    State(governor): State<StatsUploadGovernor>,
    request: Request,
    next: Next,
) -> Response {
    let Some(token_hash) = bearer_token_hash(request.headers()) else {
        return next.run(request).await;
    };
    match governor.allow(&token_hash) {
        Ok(()) => next.run(request).await,
        Err(secs) => rate_limited_response(secs),
    }
}

fn rate_limited_response(secs: u64) -> Response {
    let secs = secs.max(1);
    (
        StatusCode::TOO_MANY_REQUESTS,
        [(header::RETRY_AFTER, secs.to_string())],
        Json(serde_json::json!({"code": "rate_limited"})),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::batch_cap_from_env_value;

    #[test]
    fn batch_cap_flag_matches_production_parsing() {
        assert!(!batch_cap_from_env_value(None));
        for value in ["", " ", "0", "false", "FALSE", "False", "no", "off", "OFF"] {
            assert!(
                !batch_cap_from_env_value(Some(value)),
                "expected off for {value:?}"
            );
        }
        for value in [
            "1", "true", "TRUE", "True", "yes", "YES", "on", "ON", " yes ",
        ] {
            assert!(
                batch_cap_from_env_value(Some(value)),
                "expected on for {value:?}"
            );
        }
    }
}
