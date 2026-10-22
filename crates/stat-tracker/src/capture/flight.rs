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
//!
//! This is one lock per process. The app and the daemon are two processes,
//! so a "Capture now" in the app can overlap a daemon grab. On KDE direct
//! that is harmless: KWin runs such calls one after the other on its own
//! thread. On Older screenshot it is not fully covered: an app request and
//! a daemon poll in the same second can still share one portal file, and
//! one of the two then fails (its file is gone or replaced). Capture now is
//! a rare manual click, so this stays a known gap.
//!
//! Ordinary polls never wait for the lock ([`lock_wait`]): a poll that
//! finds a request running is skipped, and the next tick polls again. That
//! keeps the poll arm of the daemon loop from sitting behind a Tab's
//! screenshot. The outcome-only poll that runs beside a Tab capture in the
//! end-reel wake (the ~3 s VICTORY window) waits a little, about a second,
//! so a Tab held over the banner does not cost it every tick. A Tab waits,
//! up to its route's Tab wait.

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
            FlightError::Busy => f.write_str(BUSY),
            FlightError::Task(err) => write!(f, "capture task failed: {err}"),
        }
    }
}

impl std::error::Error for FlightError {}

/// The words every "lock still held" error carries, whichever route
/// produced it ([`is_busy`]).
pub const BUSY: &str = "a previous capture request is still running";

/// True for an error text from a grab that found the lock held.
pub fn is_busy(err: &str) -> bool {
    err.contains(BUSY)
}

/// How long the current grab waits for the lock: `tab_wait` inside a Tab
/// capture task, else `poll_wait` (zero for an ordinary poll, about a
/// second for the end-reel outcome-only poll).
pub fn lock_wait(is_tab: bool, tab_wait: Duration, poll_wait: Duration) -> Duration {
    if is_tab { tab_wait } else { poll_wait }
}

/// Runs `request` to the end, calling `overdue` with the time waited so
/// far once at `first` and then every `every`. It never gives up: a
/// request that cannot be cancelled keeps whatever it holds until it ends.
pub async fn with_overdue_warnings<F: Future>(
    request: F,
    first: Duration,
    every: Duration,
    mut overdue: impl FnMut(Duration),
) -> F::Output {
    tokio::pin!(request);
    let started = tokio::time::Instant::now();
    let mut next = started + first;
    loop {
        tokio::select! {
            out = &mut request => return out,
            () = tokio::time::sleep_until(next) => {
                overdue(next - started);
                next += every;
            }
        }
    }
}

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
        // A zero wait takes the lock only if it is free right now. Through
        // `timeout(ZERO, lock())` a task whose coop budget is spent got
        // Pending from a free lock and a spurious Busy.
        let guard = if wait.is_zero() {
            self.lock.try_lock().map_err(|_| FlightError::Busy)?
        } else {
            tokio::time::timeout(wait, self.lock.lock())
                .await
                .map_err(|_| FlightError::Busy)?
        };
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

    #[test]
    fn polls_never_wait_and_tabs_wait_their_route_wait() {
        let tab = Duration::from_secs(4);
        let reel = Duration::from_secs(1);
        assert_eq!(lock_wait(false, tab, Duration::ZERO), Duration::ZERO);
        assert_eq!(lock_wait(false, tab, reel), reel);
        assert_eq!(lock_wait(true, tab, Duration::ZERO), tab);
        assert_eq!(lock_wait(true, tab, reel), tab);
        assert!(is_busy(&FlightError::Busy.to_string()));
        assert!(is_busy(&format!(
            "portal screenshot: {}",
            FlightError::Busy
        )));
        assert!(!is_busy("capture task failed: boom"));
    }

    /// A poll arriving while a request runs is turned away at once; a Tab
    /// waits for it. Paused time: nothing really sleeps.
    #[tokio::test(start_paused = true)]
    async fn a_poll_is_turned_away_at_once_and_a_tab_waits() {
        static FLIGHT: SingleFlight = SingleFlight::new();
        let running = tokio::spawn(FLIGHT.run(Duration::ZERO, async {
            tokio::time::sleep(Duration::from_secs(1)).await;
        }));
        tokio::task::yield_now().await;
        assert!(FLIGHT.in_flight());
        let started = tokio::time::Instant::now();
        let poll = FLIGHT
            .run(
                lock_wait(false, Duration::from_secs(4), Duration::ZERO),
                async {},
            )
            .await;
        assert_eq!(poll, Err(FlightError::Busy));
        assert_eq!(started.elapsed(), Duration::ZERO, "the poll did not wait");
        let tab = FLIGHT
            .run(
                lock_wait(true, Duration::from_secs(4), Duration::ZERO),
                async {},
            )
            .await;
        assert_eq!(tab, Ok(()));
        assert_eq!(started.elapsed(), Duration::from_secs(1), "the Tab waited");
        running.await.unwrap().unwrap();
    }

    /// A request that cannot be cancelled keeps the lock until it ends:
    /// it is warned about at 20 s and every 60 s after, and never given up.
    #[tokio::test(start_paused = true)]
    async fn an_overdue_request_is_warned_about_and_keeps_the_lock() {
        static FLIGHT: SingleFlight = SingleFlight::new();
        let (answer_tx, answer_rx) = tokio::sync::oneshot::channel::<u8>();
        let warned = Arc::new(std::sync::Mutex::new(Vec::new()));
        let warned_in_task = Arc::clone(&warned);
        let request = tokio::spawn(FLIGHT.run(Duration::ZERO, async move {
            with_overdue_warnings(
                answer_rx,
                Duration::from_secs(20),
                Duration::from_secs(60),
                |waited| warned_in_task.lock().unwrap().push(waited.as_secs()),
            )
            .await
        }));
        tokio::time::sleep(Duration::from_secs(25)).await;
        assert!(FLIGHT.in_flight(), "still held after the first warning");
        assert_eq!(*warned.lock().unwrap(), vec![20]);
        assert_eq!(
            FLIGHT.run(Duration::from_secs(1), async {}).await,
            Err(FlightError::Busy),
            "nothing runs beside it"
        );
        tokio::time::sleep(Duration::from_secs(120)).await;
        assert!(FLIGHT.in_flight());
        assert_eq!(*warned.lock().unwrap(), vec![20, 80, 140]);
        answer_tx.send(7).unwrap();
        assert_eq!(request.await.unwrap().unwrap(), Ok(7));
        assert!(!FLIGHT.in_flight());
    }

    /// A free lock is taken at once, even by a task that has spent its
    /// coop budget. Through `timeout(ZERO, lock())` that task got Pending,
    /// yielded, and a grab that ran in that yield took the lock: a Busy
    /// for a lock that was free when the poll asked.
    #[tokio::test(start_paused = true)]
    async fn a_zero_wait_takes_a_free_lock_with_the_coop_budget_spent() {
        static FLIGHT: SingleFlight = SingleFlight::new();
        // A Tab grab that runs at this task's next yield and holds the
        // lock for a second.
        let tab = tokio::spawn(FLIGHT.run(Duration::from_secs(5), async {
            tokio::time::sleep(Duration::from_secs(1)).await;
        }));
        let other = tokio::sync::Mutex::new(());
        let mut spent = 0;
        while tokio::task::coop::has_budget_remaining() && spent < 10_000 {
            drop(other.lock().await);
            spent += 1;
        }
        assert!(
            !tokio::task::coop::has_budget_remaining(),
            "the budget is spent after {spent} locks"
        );
        assert!(!FLIGHT.in_flight(), "the lock is free when the poll asks");
        assert_eq!(FLIGHT.run(Duration::ZERO, async { 5 }).await, Ok(5));
        tab.await.unwrap().unwrap();
    }

    /// The end-reel outcome-only poll waits about a second: it gets the
    /// lock behind a 0.5 s Tab grab, and is turned away behind a 1.5 s one.
    #[tokio::test(start_paused = true)]
    async fn the_end_reel_poll_waits_a_second_and_an_ordinary_poll_none() {
        static FLIGHT: SingleFlight = SingleFlight::new();
        let reel = Duration::from_secs(1);
        for (held_ms, reel_gets_it) in [(500u64, true), (1500, false)] {
            let held = tokio::spawn(FLIGHT.run(Duration::ZERO, async move {
                tokio::time::sleep(Duration::from_millis(held_ms)).await;
            }));
            tokio::task::yield_now().await;
            assert_eq!(
                FLIGHT
                    .run(
                        lock_wait(false, Duration::from_secs(4), Duration::ZERO),
                        async {}
                    )
                    .await,
                Err(FlightError::Busy),
                "an ordinary poll never waits"
            );
            let got = FLIGHT
                .run(lock_wait(false, Duration::from_secs(4), reel), async {})
                .await;
            assert_eq!(got.is_ok(), reel_gets_it, "Tab grab of {held_ms} ms");
            held.await.unwrap().unwrap();
        }
    }

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
