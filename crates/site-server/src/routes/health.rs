use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::extract::State;
use axum::http::StatusCode;

use crate::state::AppState;

/// How long a readiness result is reused. A burst of probes runs `RETURN true`
/// at most once per process per this interval.
const READY_TTL: Duration = Duration::from_secs(2);

struct ReadySlot {
    at: Instant,
    ok: bool,
}

static READY_CACHE: Mutex<Option<ReadySlot>> = Mutex::new(None);

/// Liveness. Does not touch the database.
pub async fn health() -> StatusCode {
    StatusCode::OK
}

/// Readiness. 200 when a cached `RETURN true` succeeded, 503 when that probe failed.
///
/// The cache is 2 seconds, for both results. A 200 does not mean the relay
/// login exists, that migrations finished, or that the relay password is
/// correct. The body is empty. The error is logged and is not returned.
pub async fn ready(State(state): State<AppState>) -> StatusCode {
    if let Some(ok) = ready_cached(Instant::now()) {
        return if ok {
            StatusCode::OK
        } else {
            StatusCode::SERVICE_UNAVAILABLE
        };
    }
    let status = ready_status(state.db.ping().await);
    remember_ready(Instant::now(), status == StatusCode::OK);
    status
}

pub(crate) fn ready_status(result: scuffed_db::DbResult<()>) -> StatusCode {
    match result {
        Ok(()) => StatusCode::OK,
        Err(err) => {
            tracing::warn!(error = %err, "readiness check failed");
            StatusCode::SERVICE_UNAVAILABLE
        }
    }
}

fn lookup(slot: &Option<ReadySlot>, now: Instant, ttl: Duration) -> Option<bool> {
    let slot = slot.as_ref()?;
    let age = now.checked_duration_since(slot.at)?;
    if age < ttl { Some(slot.ok) } else { None }
}

fn ready_cached(now: Instant) -> Option<bool> {
    let guard = READY_CACHE.lock().unwrap_or_else(|err| err.into_inner());
    lookup(&guard, now, READY_TTL)
}

fn remember_ready(now: Instant, ok: bool) {
    let mut guard = READY_CACHE.lock().unwrap_or_else(|err| err.into_inner());
    *guard = Some(ReadySlot { at: now, ok });
}

#[cfg(test)]
mod tests {
    use super::*;
    use scuffed_db::DbError;

    #[test]
    fn ready_status_is_503_when_the_probe_fails() {
        assert_eq!(ready_status(Ok(())), StatusCode::OK);
        assert_eq!(
            ready_status(Err(DbError::Timeout)),
            StatusCode::SERVICE_UNAVAILABLE
        );
    }

    #[test]
    fn ready_cache_skips_a_probe_inside_the_ttl() {
        let start = Instant::now();
        let fresh = Some(ReadySlot {
            at: start,
            ok: true,
        });
        assert_eq!(
            lookup(&fresh, start + Duration::from_millis(500), READY_TTL),
            Some(true)
        );
        assert_eq!(lookup(&fresh, start + READY_TTL, READY_TTL), None);
        let failed = Some(ReadySlot {
            at: start,
            ok: false,
        });
        assert_eq!(
            lookup(&failed, start + Duration::from_millis(100), READY_TTL),
            Some(false)
        );
    }
}
