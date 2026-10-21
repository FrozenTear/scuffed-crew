//! One capture request at a time per route.
//!
//! The KDE screenshot portal names its file after the current second and
//! overwrites it, so two requests in one second share one file, and one
//! request's cleanup can delete the other's picture. KWin's own screenshot
//! interface must never see two of our calls at once either (KDE bug
//! 524840 is a crash class in that code path). [`SingleFlight::run`] keeps
//! one request in flight.
//!
//! The lock is taken before the request starts and moves into the task
//! that runs it. A caller that is dropped (a Tab cancelling a poll, a
//! timeout) only detaches that task: the task keeps the lock until the
//! request and its cleanup have finished, so the next request cannot start
//! beside it. A caller that is dropped while it still waits for the lock
//! starts nothing.

use std::future::Future;
use std::time::Duration;

/// Why [`SingleFlight::run`] returned no result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlightError {
    /// Another request still held the lock after the wait.
    Busy,
    /// The task running the request panicked or was cancelled.
    Task(String),
}

impl std::fmt::Display for FlightError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FlightError::Busy => f.write_str("a previous capture request is still running"),
            FlightError::Task(err) => write!(f, "capture task failed: {err}"),
        }
    }
}

impl std::error::Error for FlightError {}

/// A lock that one capture request holds from its start until its cleanup.
pub struct SingleFlight {
    lock: tokio::sync::Mutex<()>,
}

impl SingleFlight {
    pub const fn new() -> Self {
        Self {
            lock: tokio::sync::Mutex::const_new(()),
        }
    }

    /// Waits up to `wait` for the request before it to finish, then runs
    /// `request` on its own task while holding the lock. Dropping the
    /// returned future after the lock was taken does not stop `request`
    /// or release the lock early.
    pub async fn run<T, F>(&'static self, wait: Duration, request: F) -> Result<T, FlightError>
    where
        F: Future<Output = T> + Send + 'static,
        T: Send + 'static,
    {
        let guard = tokio::time::timeout(wait, self.lock.lock())
            .await
            .map_err(|_| FlightError::Busy)?;
        let task = tokio::spawn(async move {
            let out = request.await;
            drop(guard);
            out
        });
        task.await.map_err(|err| FlightError::Task(err.to_string()))
    }

    /// True while a request holds the lock.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn in_flight(&self) -> bool {
        self.lock.try_lock().is_err()
    }
}

impl Default for SingleFlight {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    #[tokio::test]
    async fn requests_never_overlap() {
        static FLIGHT: SingleFlight = SingleFlight::new();
        let running = Arc::new(AtomicUsize::new(0));
        let overlapped = Arc::new(AtomicBool::new(false));
        let mut joins = Vec::new();
        for _ in 0..6 {
            let running = Arc::clone(&running);
            let overlapped = Arc::clone(&overlapped);
            joins.push(tokio::spawn(async move {
                FLIGHT
                    .run(Duration::from_secs(5), async move {
                        if running.fetch_add(1, Ordering::SeqCst) != 0 {
                            overlapped.store(true, Ordering::SeqCst);
                        }
                        tokio::time::sleep(Duration::from_millis(5)).await;
                        running.fetch_sub(1, Ordering::SeqCst);
                    })
                    .await
            }));
        }
        for join in joins {
            join.await.unwrap().unwrap();
        }
        assert!(!overlapped.load(Ordering::SeqCst));
        assert!(!FLIGHT.in_flight());
    }

    /// A Tab drops the poll that started a request. The request still
    /// finishes its cleanup, and the next request cannot start until it
    /// has, so it can never delete the next request's file.
    #[tokio::test]
    async fn a_dropped_request_keeps_the_lock_until_its_cleanup_ends() {
        static FLIGHT: SingleFlight = SingleFlight::new();
        let (finish_tx, finish_rx) = tokio::sync::oneshot::channel::<()>();
        let cleaned = Arc::new(AtomicBool::new(false));
        let cleaned_in_task = Arc::clone(&cleaned);
        let poll = FLIGHT.run(Duration::from_secs(1), async move {
            let _ = finish_rx.await;
            cleaned_in_task.store(true, Ordering::SeqCst);
        });
        // The poll's caller gives up after the request started.
        let dropped = tokio::time::timeout(Duration::from_millis(20), poll).await;
        assert!(dropped.is_err(), "the caller was dropped mid-request");
        assert!(
            FLIGHT.in_flight(),
            "the detached request still holds the lock"
        );

        // A Tab arriving now waits; with a short wait it is turned away
        // instead of running beside the detached request.
        let tab = FLIGHT.run(Duration::from_millis(20), async { "tab" }).await;
        assert_eq!(tab, Err(FlightError::Busy));
        assert!(!cleaned.load(Ordering::SeqCst));

        // The detached request ends and cleans up; then the Tab runs.
        finish_tx.send(()).unwrap();
        let tab = FLIGHT.run(Duration::from_secs(1), async { "tab" }).await;
        assert_eq!(tab, Ok("tab"));
        assert!(cleaned.load(Ordering::SeqCst), "cleanup ran before the Tab");
    }

    #[tokio::test]
    async fn a_caller_dropped_while_waiting_starts_nothing() {
        static FLIGHT: SingleFlight = SingleFlight::new();
        let (finish_tx, finish_rx) = tokio::sync::oneshot::channel::<()>();
        let first = tokio::spawn(FLIGHT.run(Duration::from_secs(1), async move {
            let _ = finish_rx.await;
        }));
        tokio::time::timeout(Duration::from_secs(5), async {
            while !FLIGHT.in_flight() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the first request holds the lock");
        let started = Arc::new(AtomicBool::new(false));
        let started_in_task = Arc::clone(&started);
        let waiting = FLIGHT.run(Duration::from_secs(5), async move {
            started_in_task.store(true, Ordering::SeqCst);
        });
        assert!(
            tokio::time::timeout(Duration::from_millis(20), waiting)
                .await
                .is_err()
        );
        finish_tx.send(()).unwrap();
        first.await.unwrap().unwrap();
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert!(!started.load(Ordering::SeqCst));
    }
}
