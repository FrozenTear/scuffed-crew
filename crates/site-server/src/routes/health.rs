use std::future::Future;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::extract::State;
use axum::http::StatusCode;
use tokio::sync::{Mutex as AsyncMutex, watch};

use crate::state::AppState;

/// How long a readiness result is reused. A burst of probes runs `RETURN true`
/// at most once per process per this interval. Concurrent cache misses share
/// that one probe.
const READY_TTL: Duration = Duration::from_secs(2);

struct ReadySlot {
    at: Instant,
    ok: bool,
}

/// Cache plus the in-flight probe. The std mutex is never held across `.await`.
struct ReadyFlight {
    cache: Mutex<Option<ReadySlot>>,
    /// `None` while idle. While a probe runs, waiters clone this receiver.
    /// The value stays `None` until the probe finishes, then `Some(ok)`.
    inflight: AsyncMutex<Option<watch::Receiver<Option<bool>>>>,
}

impl ReadyFlight {
    const fn new() -> Self {
        Self {
            cache: Mutex::new(None),
            inflight: AsyncMutex::const_new(None),
        }
    }

    fn cached(&self, now: Instant, ttl: Duration) -> Option<bool> {
        let guard = self.cache.lock().unwrap_or_else(|err| err.into_inner());
        lookup(&guard, now, ttl)
    }

    fn remember(&self, now: Instant, ok: bool) {
        let mut guard = self.cache.lock().unwrap_or_else(|err| err.into_inner());
        *guard = Some(ReadySlot { at: now, ok });
    }

    /// Return the cached result, or run `probe` once and share it.
    ///
    /// Followers wait on the leader's result. They do not call `probe`, so a
    /// failed check is logged once.
    async fn lookup_or_probe<F, Fut>(&self, ttl: Duration, probe: F) -> bool
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = bool>,
    {
        let now = Instant::now();
        if let Some(ok) = self.cached(now, ttl) {
            return ok;
        }
        let mut gate = self.inflight.lock().await;
        if let Some(ok) = self.cached(Instant::now(), ttl) {
            return ok;
        }
        if let Some(rx) = gate.as_ref() {
            let mut rx = rx.clone();
            drop(gate);
            loop {
                if let Some(ok) = *rx.borrow() {
                    return ok;
                }
                if rx.changed().await.is_err() {
                    return false;
                }
            }
        }
        let (tx, rx) = watch::channel(None);
        *gate = Some(rx);
        drop(gate);

        let ok = probe().await;
        self.remember(Instant::now(), ok);
        {
            let mut gate = self.inflight.lock().await;
            *gate = None;
        }
        let _ = tx.send(Some(ok));
        ok
    }
}

static READY: ReadyFlight = ReadyFlight::new();

/// Liveness. Does not touch the database.
pub async fn health() -> StatusCode {
    StatusCode::OK
}

/// Readiness. 200 when a cached `RETURN true` succeeded, 503 when that probe failed.
///
/// The cache is 2 seconds, for both results. Concurrent misses share one
/// probe. A 200 does not mean the relay login exists, that migrations
/// finished, or that the relay password is correct. The body is empty. The
/// error is logged once and is not returned.
pub async fn ready(State(state): State<AppState>) -> StatusCode {
    let ok = READY
        .lookup_or_probe(READY_TTL, || async {
            ready_status(state.db.ping().await) == StatusCode::OK
        })
        .await;
    if ok {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use scuffed_db::DbError;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

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

    #[tokio::test]
    async fn concurrent_misses_share_one_probe() {
        let flight = Arc::new(ReadyFlight::new());
        let probes = Arc::new(AtomicUsize::new(0));
        let hold = Arc::new(AtomicBool::new(true));
        let mut tasks = Vec::new();
        for _ in 0..8 {
            let flight = Arc::clone(&flight);
            let probes = Arc::clone(&probes);
            let hold = Arc::clone(&hold);
            tasks.push(tokio::spawn(async move {
                flight
                    .lookup_or_probe(Duration::from_secs(2), || {
                        let probes = Arc::clone(&probes);
                        let hold = Arc::clone(&hold);
                        async move {
                            probes.fetch_add(1, Ordering::SeqCst);
                            while hold.load(Ordering::SeqCst) {
                                tokio::task::yield_now().await;
                            }
                            false
                        }
                    })
                    .await
            }));
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        while probes.load(Ordering::SeqCst) == 0 {
            if Instant::now() > deadline {
                panic!("probe did not start");
            }
            tokio::task::yield_now().await;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(
            probes.load(Ordering::SeqCst),
            1,
            "concurrent misses started more than one probe"
        );
        hold.store(false, Ordering::SeqCst);
        for task in tasks {
            assert!(
                !task.await.expect("task"),
                "followers must observe the shared failure"
            );
        }
        assert_eq!(probes.load(Ordering::SeqCst), 1);

        let after = flight
            .lookup_or_probe(Duration::from_secs(2), || async {
                probes.fetch_add(1, Ordering::SeqCst);
                true
            })
            .await;
        assert!(!after, "a cached failure must not probe again");
        assert_eq!(probes.load(Ordering::SeqCst), 1);
    }
}
