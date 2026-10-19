//! Per-token governor and batch-cap flag for `POST /api/stats/upload`.
//!
//! The per-token governor is always on, and it is separate from the per-IP
//! bucket on that route. A cell is spent only after `validate_daemon_token`
//! returns an active token, and the key is that token's id. Unknown bearers
//! do not create entries. Idle entries are pruned every 60 seconds. Burst
//! and replenish come from [`scuffed_types`].
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
use axum::extract::Request;
use axum::http::{StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use governor::clock::Clock;
use governor::{DefaultKeyedRateLimiter, Quota, RateLimiter};
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

/// How often idle per-token buckets are dropped.
const PRUNE_EVERY: Duration = Duration::from_secs(60);

fn token_quota() -> Quota {
    let burst = std::num::NonZeroU32::new(STATS_UPLOAD_TOKEN_BURST).expect("burst is non-zero");
    Quota::with_period(Duration::from_secs(STATS_UPLOAD_TOKEN_REPLENISH_SECS))
        .expect("replenish period is non-zero")
        .allow_burst(burst)
}

/// Keyed governor for validated daemon tokens. Cloned cheaply; buckets are shared.
#[derive(Clone)]
pub struct StatsUploadGovernor {
    limiter: Arc<DefaultKeyedRateLimiter<String>>,
}

impl StatsUploadGovernor {
    pub fn new() -> Self {
        let limiter = Arc::new(RateLimiter::keyed(token_quota()));
        spawn_prune(Arc::downgrade(&limiter));
        Self { limiter }
    }

    /// Spend one cell for a token id that has already been validated.
    pub(crate) fn allow(&self, token_id: &str) -> Result<(), u64> {
        let key = token_id.to_owned();
        match self.limiter.check_key(&key) {
            Ok(_) => Ok(()),
            Err(not_until) => {
                let wait = not_until.wait_time_from(self.limiter.clock().now());
                Err(retry_after_secs(wait))
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.limiter.len()
    }
}

fn spawn_prune(limiter: std::sync::Weak<DefaultKeyedRateLimiter<String>>) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(PRUNE_EVERY);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tick.tick().await;
            let Some(limiter) = limiter.upgrade() else {
                break;
            };
            limiter.retain_recent();
            limiter.shrink_to_fit();
        }
    });
}

#[cfg(test)]
thread_local! {
    static GOVERNOR_OVERRIDE: std::cell::Cell<Option<StatsUploadGovernor>> =
        const { std::cell::Cell::new(None) };
}

/// Next `create_router` call uses this governor instead of a fresh one.
#[cfg(test)]
pub(crate) fn override_stats_upload_governor(governor: StatsUploadGovernor) {
    GOVERNOR_OVERRIDE.with(|slot| slot.set(Some(governor)));
}

#[cfg(test)]
pub(crate) fn take_stats_upload_governor_override() -> Option<StatsUploadGovernor> {
    GOVERNOR_OVERRIDE.with(|slot| slot.take())
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

/// Puts the per-token governor on the request for [`crate::routes::stats::upload_stats`].
///
/// This does not spend a cell. The handler spends one only after the bearer
/// has been validated, keyed by token id.
pub async fn attach_stats_upload_governor(
    axum::extract::State(governor): axum::extract::State<StatsUploadGovernor>,
    mut request: Request,
    next: Next,
) -> Response {
    request.extensions_mut().insert(governor);
    next.run(request).await
}

pub(crate) fn token_rate_limited_response(secs: u64) -> Response {
    let secs = secs.max(1);
    (
        StatusCode::TOO_MANY_REQUESTS,
        [
            (header::RETRY_AFTER, secs.to_string()),
            (header::CACHE_CONTROL, "no-store".to_string()),
        ],
        Json(serde_json::json!({"code": "rate_limited"})),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use governor::RateLimiter;
    use governor::clock::FakeRelativeClock;
    use scuffed_types::STATS_UPLOAD_TOKEN_REPLENISH_SECS;

    use super::batch_cap_from_env_value;
    use super::token_quota;

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

    #[test]
    fn idle_token_key_is_pruned_once_the_bucket_is_full() {
        let clock = FakeRelativeClock::default();
        let limiter = RateLimiter::dashmap_with_clock(token_quota(), clock.clone());
        limiter.check_key(&"token-id".to_string()).unwrap();
        assert_eq!(limiter.len(), 1);
        limiter.retain_recent();
        assert_eq!(limiter.len(), 1, "a key that just spent a cell stays");
        clock.advance(Duration::from_secs(STATS_UPLOAD_TOKEN_REPLENISH_SECS));
        limiter.retain_recent();
        assert_eq!(limiter.len(), 1, "one replenish period is not idle yet");
        clock.advance(Duration::from_secs(STATS_UPLOAD_TOKEN_REPLENISH_SECS));
        limiter.retain_recent();
        limiter.shrink_to_fit();
        assert_eq!(limiter.len(), 0, "an idle key is dropped");
    }
}
