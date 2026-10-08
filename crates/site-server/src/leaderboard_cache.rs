//! In-process cache for `GET /api/public/leaderboards`.
//!
//! The query aggregates every `personal_match` row. A load test at main
//! `9590e52` measured p95 322 ms at 60k rows, 1.2 s at 300k, and 5.7 s at
//! 1.5M, and those scans queued behind the single Surreal socket so uploads
//! slowed down too. Freshness is the TTL alone (default 30s, clamped to
//! 5..=300). Uploads do not clear the cache. A generation bump on every
//! upload dropped in-flight results whenever uploads arrived faster than
//! the scan, so a busy process never stored a board. A finished load is
//! always stored. `cached_at` is the wall time when that query started, so
//! the label does not claim the board is newer than the scan.
//!
//! After the fresh window the previous board is served as-is while one
//! refresh runs in the background, so a restart does not make every key
//! block again on the same tick. Each entry's window is the TTL times a
//! factor in 0.8..=1.2. A board older than 10 TTLs is not served; that
//! request waits for a scan. Background refreshes may use at most one
//! fewer slot than the scan cap (and at least one). They do not wait in
//! line. If no refresh slot is free the key joins a queue ordered by age.
//! When a scan finishes, its refresh slot goes straight to the oldest key
//! still waiting, so refreshes run back to back while a slot is free.
//! A skipped key that is not handed a slot is tried from a later request
//! after a jittered backoff of 0.25 to 1 times the TTL, so those retries
//! do not all land on the same millisecond. Cold keys and boards past 10
//! TTLs may use any free slot, including one a refresh cannot take, so
//! they are not queued behind those refreshes. A blocking read whose key
//! is already refreshing joins that scan.
//!
//! The public handler stores one grouped snapshot per season. Hero, metric,
//! and limit are projections of that snapshot, so they do not each take a
//! scan.
//!
//! The cache is per process. A restart clears it, and two instances do not
//! share it. A failed load is not stored. If a previous board for the same
//! key is still in memory, that board is served instead of the error.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::future::Future;
#[cfg(test)]
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use rand::Rng;
use scuffed_types::MemberLeaderboardRow;
use tokio::sync::{OwnedSemaphorePermit, Semaphore, watch};

/// Default lifetime when `LEADERBOARD_CACHE_TTL_SECS` is unset, blank, or not an integer.
pub const DEFAULT_TTL_SECS: u64 = 30;
/// Lower clamp for `LEADERBOARD_CACHE_TTL_SECS`.
pub const MIN_TTL_SECS: u64 = 5;
/// Upper clamp for `LEADERBOARD_CACHE_TTL_SECS`.
pub const MAX_TTL_SECS: u64 = 300;
/// Cap on distinct query keys. Extra keys evict the least recently used.
pub const MAX_KEYS: usize = 64;
/// Default number of leaderboard scans allowed at once.
pub const DEFAULT_MAX_SCANS: usize = 2;
/// Lower clamp for `LEADERBOARD_CACHE_MAX_SCANS`.
pub const MIN_MAX_SCANS: usize = 1;
/// Upper clamp for `LEADERBOARD_CACHE_MAX_SCANS`.
pub const MAX_MAX_SCANS: usize = 8;
/// Past this many TTLs a request waits for the refresh instead of taking the stale board.
pub const MAX_STALE_FACTOR: u64 = 10;
/// Inclusive jitter applied to each entry, in thousandths of the TTL (0.8..=1.2).
const JITTER_MIN: u64 = 800;
const JITTER_MAX: u64 = 1200;
/// Inclusive fallback backoff, in thousandths of the TTL (0.25..=1).
const BACKOFF_MIN: u64 = 250;
const BACKOFF_MAX: u64 = 1000;

/// Historical `limit` buckets. The public handler no longer scans per bucket.
/// It stores one snapshot per season and cuts the rows to the requested limit.
pub const LIMIT_BUCKETS: [u32; 4] = [10, 25, 50, 100];

/// Who the board was built for.
///
/// The public route only stores [`LeaderboardAudience::Public`]. Anonymous
/// and logged-in callers share that slot: the query already drops inactive
/// members, and the handler does not read the session. `Crew` exists so a
/// later crew-only board cannot be written into a slot an anonymous caller
/// reads.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub enum LeaderboardAudience {
    Public,
    /// Not produced by the public handler. See the enum docs.
    #[cfg_attr(not(test), allow(dead_code))]
    Crew,
}

/// Everything that changes a stored board.
///
/// The public handler stores one snapshot per season under metric `"scan"`,
/// limit 0, and an empty hero. Metric, limit, and hero on the request are
/// projections of that snapshot. There is no role filter and no game-mode
/// filter on this handler, so those are not key fields.
#[derive(Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct LeaderboardKey {
    pub audience: LeaderboardAudience,
    pub metric: String,
    pub limit: u32,
    /// Empty means all time. Callers pass the trimmed season id.
    pub season_id: String,
    /// Empty means every hero. Callers pass the canonical hero name.
    pub hero: String,
}

impl LeaderboardKey {
    pub fn public_board(metric: &str, limit: u32, season_id: &str, hero: &str) -> Self {
        Self {
            audience: LeaderboardAudience::Public,
            metric: metric.to_string(),
            limit,
            season_id: season_id.to_string(),
            hero: hero.to_string(),
        }
    }
}

/// A stored board. `cached_at` is the wall time when the load started.
///
/// `snapshot` is set when the load was one grouped scan. Every hero, metric,
/// and limit for that season is a projection of it.
#[derive(Clone, Debug)]
pub struct CachedBoard {
    pub rows: Vec<MemberLeaderboardRow>,
    pub cached_at: DateTime<Utc>,
    pub snapshot: Option<Arc<scuffed_db::LeaderboardSnapshot>>,
}

/// What a loader returns. A plain row vec is the single-key case.
pub struct LoadedBoard {
    pub rows: Vec<MemberLeaderboardRow>,
    pub snapshot: Option<Arc<scuffed_db::LeaderboardSnapshot>>,
}

pub trait IntoLoaded {
    fn into_loaded(self) -> LoadedBoard;
}

impl IntoLoaded for Vec<MemberLeaderboardRow> {
    fn into_loaded(self) -> LoadedBoard {
        LoadedBoard {
            rows: self,
            snapshot: None,
        }
    }
}

impl IntoLoaded for LoadedBoard {
    fn into_loaded(self) -> LoadedBoard {
        self
    }
}

/// What a failed load should do with a previous board for the same key.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OnError {
    /// Serve the previous board when one exists. Do not store the error.
    ServeStale,
    /// Surface the error. Do not substitute another board.
    Surface,
}

pub trait LoadError {
    fn on_error(&self) -> OnError;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FailKind {
    Unavailable,
    Rejected,
}

#[derive(Debug)]
pub enum CacheError<E> {
    Load(E),
    /// A joined caller whose leader failed. The original error stays with the leader.
    Leader(FailKind),
}

struct Entry {
    board: Arc<CachedBoard>,
    /// Monotonic time when the query started. Freshness and max staleness
    /// are both measured from here. `cached_at` on the board is the matching
    /// wall time and is not rewritten while this entry is served stale.
    stored_at_ms: u64,
    /// How long this entry stays fresh. `TTL * jitter`, with jitter in 0.8..=1.2.
    fresh_for_ms: u64,
    /// Earliest monotonic time a skipped background refresh may be tried again.
    /// Zero means there is no backoff.
    next_refresh_ms: u64,
}

#[derive(Clone, Copy)]
enum JitterMode {
    #[cfg(test)]
    Fixed(u64),
    Random,
}

#[derive(Clone)]
enum FlightState {
    Pending,
    Ready(Arc<CachedBoard>),
    Failed(FailKind),
    /// The background refresh did not get a slot. No scan ran.
    Skipped,
}

/// Permits held for the duration of one scan.
///
/// A refresh holds both the shared cap and a refresh-budget permit, so it
/// cannot occupy the slot reserved for blocking reads. A blocking read holds
/// only the shared cap.
struct HeldScan {
    _total: OwnedSemaphorePermit,
    _refresh: Option<OwnedSemaphorePermit>,
}

impl HeldScan {
    fn is_refresh(&self) -> bool {
        self._refresh.is_some()
    }
}

struct QueuedRefresh {
    launch: Box<dyn FnOnce(watch::Sender<FlightState>, HeldScan) + Send>,
}

#[derive(Default)]
struct Inner {
    order: VecDeque<LeaderboardKey>,
    entries: HashMap<LeaderboardKey, Entry>,
    inflight: HashMap<LeaderboardKey, watch::Receiver<FlightState>>,
    /// Stale keys waiting for a refresh slot, oldest `stored_at_ms` first.
    refresh_wait: BTreeMap<(u64, LeaderboardKey), QueuedRefresh>,
}

trait CacheClock: Send + Sync {
    fn now_ms(&self) -> u64;
    fn wall(&self) -> DateTime<Utc>;
}

struct SystemClock {
    start: Instant,
}

impl CacheClock for SystemClock {
    fn now_ms(&self) -> u64 {
        self.start.elapsed().as_millis() as u64
    }

    fn wall(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

#[cfg(test)]
#[derive(Clone)]
struct ManualClock {
    ms: Arc<AtomicU64>,
    origin: DateTime<Utc>,
}

#[cfg(test)]
impl ManualClock {
    fn new() -> Self {
        Self {
            ms: Arc::new(AtomicU64::new(0)),
            origin: DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
        }
    }

    fn advance(&self, ms: u64) {
        self.ms.fetch_add(ms, Ordering::SeqCst);
    }
}

#[cfg(test)]
impl CacheClock for ManualClock {
    fn now_ms(&self) -> u64 {
        self.ms.load(Ordering::SeqCst)
    }

    fn wall(&self) -> DateTime<Utc> {
        self.origin + chrono::Duration::milliseconds(self.now_ms() as i64)
    }
}

enum Begin {
    Hit(CachedBoard),
    /// Previous board. `lead` is set only for the caller that starts the refresh.
    ServeStale {
        board: CachedBoard,
        lead: Option<watch::Sender<FlightState>>,
    },
    Wait(watch::Receiver<FlightState>),
    Lead {
        tx: watch::Sender<FlightState>,
    },
}

enum Follow<E> {
    Ready(Result<CachedBoard, CacheError<E>>),
    /// The flight we joined did not scan. Become the leader if we still must.
    Retry,
}

/// Process-local leaderboard cache. Cheap to clone (`Arc` inside).
#[derive(Clone)]
pub struct LeaderboardCache {
    inner: Arc<Mutex<Inner>>,
    ttl: Duration,
    max_entries: usize,
    /// Shared cap. Blocking reads wait on this. Refreshes only try it.
    scans: Arc<Semaphore>,
    /// Refresh budget: `max(1, MAX_SCANS - 1)`. Never waited on.
    refresh_scans: Arc<Semaphore>,
    jitter: JitterMode,
    clock: Arc<dyn CacheClock>,
    #[cfg(test)]
    followers: Arc<AtomicU32>,
}

impl LeaderboardCache {
    /// TTL from `LEADERBOARD_CACHE_TTL_SECS` (5..=300, default 30) and scan
    /// cap from `LEADERBOARD_CACHE_MAX_SCANS` (1..=8, default 2).
    pub fn from_env() -> Self {
        let ttl_raw = std::env::var("LEADERBOARD_CACHE_TTL_SECS").ok();
        let scans_raw = std::env::var("LEADERBOARD_CACHE_MAX_SCANS").ok();
        Self::build(
            ttl_from_raw(ttl_raw.as_deref()),
            MAX_KEYS,
            scans_from_raw(scans_raw.as_deref()),
            Arc::new(SystemClock {
                start: Instant::now(),
            }),
            JitterMode::Random,
        )
    }

    fn build(
        ttl: Duration,
        max_entries: usize,
        scans: usize,
        clock: Arc<dyn CacheClock>,
        jitter: JitterMode,
    ) -> Self {
        let scans = scans.clamp(MIN_MAX_SCANS, MAX_MAX_SCANS);
        Self {
            inner: Arc::new(Mutex::new(Inner::default())),
            ttl,
            max_entries: max_entries.max(1),
            scans: Arc::new(Semaphore::new(scans)),
            refresh_scans: Arc::new(Semaphore::new(refresh_slots(scans))),
            jitter,
            clock,
            #[cfg(test)]
            followers: Arc::new(AtomicU32::new(0)),
        }
    }

    pub async fn get_or_load<F, Fut, T, E>(
        &self,
        key: LeaderboardKey,
        load: F,
    ) -> Result<CachedBoard, CacheError<E>>
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = Result<T, E>> + Send + 'static,
        T: IntoLoaded + Send + 'static,
        E: LoadError + Send + 'static,
    {
        let mut load = Some(load);
        loop {
            match self.begin(&key) {
                Begin::Hit(board) | Begin::ServeStale { board, lead: None } => return Ok(board),
                Begin::ServeStale {
                    board,
                    lead: Some(tx),
                } => {
                    if let Some(permit) = self.try_refresh_permit() {
                        let load = load.take().expect("loader is used once");
                        let cache = self.clone();
                        let key = key.clone();
                        tokio::spawn(async move {
                            let _ = cache.lead(key, tx, load, Some(permit)).await;
                        });
                    } else {
                        // No refresh slot. Queue the loader so the next scan
                        // to finish can hand its slot over. Do not call `load`
                        // on this request.
                        let load = load.take().expect("loader is used once");
                        self.enqueue_refresh(&key, tx, load);
                    }
                    return Ok(board);
                }
                Begin::Wait(rx) => match self.finish_follower(rx, &key).await {
                    Follow::Ready(result) => return result,
                    Follow::Retry => continue,
                },
                Begin::Lead { tx } => {
                    let load = load.take().expect("loader is used once");
                    let permit = self.acquire_blocking().await;
                    return self.lead(key, tx, load, permit).await;
                }
            }
        }
    }

    /// Take a refresh slot without waiting. `None` means skip this attempt.
    fn try_refresh_permit(&self) -> Option<HeldScan> {
        let refresh = self.refresh_scans.clone().try_acquire_owned().ok()?;
        let total = match self.scans.clone().try_acquire_owned() {
            Ok(total) => total,
            Err(_) => return None,
        };
        Some(HeldScan {
            _total: total,
            _refresh: Some(refresh),
        })
    }

    /// Wait for any free scan slot. Refreshes never sit in this queue.
    async fn acquire_blocking(&self) -> Option<HeldScan> {
        let total = self.scans.clone().acquire_owned().await.ok()?;
        Some(HeldScan {
            _total: total,
            _refresh: None,
        })
    }

    fn enqueue_refresh<F, Fut, T, E>(
        &self,
        key: &LeaderboardKey,
        tx: watch::Sender<FlightState>,
        load: F,
    ) where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = Result<T, E>> + Send + 'static,
        T: IntoLoaded + Send + 'static,
        E: LoadError + Send + 'static,
    {
        let stored_at = self
            .lock()
            .entries
            .get(key)
            .map(|entry| entry.stored_at_ms)
            .unwrap_or(0);
        let backoff = self.refresh_backoff_ms();
        let cache = self.clone();
        let key_owned = key.clone();
        let launch = Box::new(move |tx: watch::Sender<FlightState>, permit: HeldScan| {
            tokio::spawn(async move {
                let _ = cache.lead(key_owned, tx, load, Some(permit)).await;
            });
        });
        {
            let mut guard = self.lock();
            let now = self.clock.now_ms();
            if let Some(entry) = guard.entries.get_mut(key) {
                entry.next_refresh_ms = now.saturating_add(backoff);
            }
            guard.inflight.remove(key);
            let stale_keys: Vec<_> = guard
                .refresh_wait
                .keys()
                .filter(|(_, queued)| queued == key)
                .cloned()
                .collect();
            for stale_key in stale_keys {
                guard.refresh_wait.remove(&stale_key);
            }
            let wait_key = (stored_at, key.clone());
            while guard.refresh_wait.len() >= self.max_entries {
                let Some(freshest) = guard.refresh_wait.keys().next_back().cloned() else {
                    break;
                };
                if wait_key >= freshest {
                    let _ = tx.send(FlightState::Skipped);
                    return;
                }
                guard.refresh_wait.remove(&freshest);
            }
            guard
                .refresh_wait
                .insert(wait_key, QueuedRefresh { launch });
        }
        let _ = tx.send(FlightState::Skipped);
    }

    /// Start the stalest queued refresh with `permit`. Consumes the permit
    /// either way.
    fn handoff(&self, permit: HeldScan) -> bool {
        let now = self.clock.now_ms();
        let mut guard = self.lock();
        let candidates: Vec<_> = guard.refresh_wait.keys().cloned().collect();
        for candidate in candidates {
            let Some(queued) = guard.refresh_wait.remove(&candidate) else {
                continue;
            };
            let key = candidate.1.clone();
            if guard.inflight.contains_key(&key) {
                continue;
            }
            let still_due = guard
                .entries
                .get(&key)
                .is_none_or(|entry| now.saturating_sub(entry.stored_at_ms) >= entry.fresh_for_ms);
            if !still_due {
                continue;
            }
            let (tx, rx) = watch::channel(FlightState::Pending);
            guard.inflight.insert(key, rx);
            drop(guard);
            (queued.launch)(tx, permit);
            return true;
        }
        drop(permit);
        false
    }

    fn begin(&self, key: &LeaderboardKey) -> Begin {
        let mut guard = self.lock();
        let now = self.clock.now_ms();
        let max_stale = self.ttl_ms().saturating_mul(MAX_STALE_FACTOR);
        let stored = guard.entries.get(key).map(|entry| {
            (
                entry.board.as_ref().clone(),
                now.saturating_sub(entry.stored_at_ms),
                entry.fresh_for_ms,
                entry.next_refresh_ms,
            )
        });
        if let Some((board, age, fresh_for_ms, next_refresh_ms)) = stored {
            if age < fresh_for_ms {
                touch(&mut guard, key);
                return Begin::Hit(board);
            }
            let too_stale = age >= max_stale;
            if let Some(rx) = guard.inflight.get(key) {
                if too_stale {
                    #[cfg(test)]
                    self.followers.fetch_add(1, Ordering::SeqCst);
                    return Begin::Wait(rx.clone());
                }
                return Begin::ServeStale { board, lead: None };
            }
            // A request does not start another refresh until the jittered
            // backoff elapses, so a busy cap does not spin. A finished scan
            // can still hand its slot to this key sooner. Past 10 TTLs this
            // does not apply: that request has to scan.
            if !too_stale && now < next_refresh_ms {
                return Begin::ServeStale { board, lead: None };
            }
            let (tx, rx) = watch::channel(FlightState::Pending);
            guard.inflight.insert(key.clone(), rx);
            touch(&mut guard, key);
            if too_stale {
                return Begin::Lead { tx };
            }
            return Begin::ServeStale {
                board,
                lead: Some(tx),
            };
        }
        if let Some(rx) = guard.inflight.get(key) {
            #[cfg(test)]
            self.followers.fetch_add(1, Ordering::SeqCst);
            return Begin::Wait(rx.clone());
        }
        let (tx, rx) = watch::channel(FlightState::Pending);
        guard.inflight.insert(key.clone(), rx);
        Begin::Lead { tx }
    }

    async fn lead<F, Fut, T, E>(
        &self,
        key: LeaderboardKey,
        tx: watch::Sender<FlightState>,
        load: F,
        permit: Option<HeldScan>,
    ) -> Result<CachedBoard, CacheError<E>>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T, E>>,
        T: IntoLoaded,
        E: LoadError + Send + 'static,
    {
        // Stamp the board when the query starts. A scan that runs for a
        // second must not be labeled as if it finished just now.
        let started_ms = self.clock.now_ms();
        let started = self.clock.wall();
        let fresh_for_ms = fresh_for_ms(self.ttl_ms(), self.jitter_millis());
        let result = load().await;
        let refresh = permit.as_ref().is_some_and(|held| held.is_refresh());
        let mut guard = self.lock();
        guard.inflight.remove(&key);
        let outcome = match result {
            Ok(loaded) => {
                let loaded = loaded.into_loaded();
                let board = Arc::new(CachedBoard {
                    rows: loaded.rows,
                    cached_at: started,
                    snapshot: loaded.snapshot,
                });
                store(
                    &mut guard,
                    self.max_entries,
                    key,
                    Entry {
                        board: Arc::clone(&board),
                        stored_at_ms: started_ms,
                        fresh_for_ms,
                        next_refresh_ms: 0,
                    },
                );
                let _ = tx.send(FlightState::Ready(Arc::clone(&board)));
                Ok(board.as_ref().clone())
            }
            Err(err) => {
                let kind = match err.on_error() {
                    OnError::ServeStale => FailKind::Unavailable,
                    OnError::Surface => FailKind::Rejected,
                };
                let _ = tx.send(FlightState::Failed(kind));
                if kind == FailKind::Unavailable
                    && let Some(stale) = guard.entries.get(&key)
                {
                    Ok(stale.board.as_ref().clone())
                } else {
                    Err(CacheError::Load(err))
                }
            }
        };
        drop(guard);
        self.release_scan(permit, refresh);
        outcome
    }

    fn release_scan(&self, permit: Option<HeldScan>, refresh: bool) {
        if refresh && let Some(permit) = permit {
            let _ = self.handoff(permit);
            return;
        }
        drop(permit);
        if let Some(permit) = self.try_refresh_permit() {
            let _ = self.handoff(permit);
        }
    }

    async fn finish_follower<E>(
        &self,
        mut rx: watch::Receiver<FlightState>,
        key: &LeaderboardKey,
    ) -> Follow<E> {
        loop {
            let state = rx.borrow().clone();
            match state {
                FlightState::Pending => {
                    if rx.changed().await.is_err() {
                        break;
                    }
                }
                FlightState::Ready(board) => {
                    return Follow::Ready(Ok(board.as_ref().clone()));
                }
                FlightState::Failed(FailKind::Rejected) => {
                    return Follow::Ready(Err(CacheError::Leader(FailKind::Rejected)));
                }
                FlightState::Failed(FailKind::Unavailable) => {
                    if let Some(board) = self.stale(key) {
                        return Follow::Ready(Ok(board));
                    }
                    return Follow::Ready(Err(CacheError::Leader(FailKind::Unavailable)));
                }
                // The refresh we joined never scanned. Try again so a
                // blocking reader becomes the leader instead of hanging.
                FlightState::Skipped => {
                    if self.must_block(key) {
                        return Follow::Retry;
                    }
                    if let Some(board) = self.stale(key) {
                        return Follow::Ready(Ok(board));
                    }
                    return Follow::Retry;
                }
            }
        }
        if self.must_block(key) {
            return Follow::Retry;
        }
        if let Some(board) = self.stale(key) {
            return Follow::Ready(Ok(board));
        }
        Follow::Ready(Err(CacheError::Leader(FailKind::Unavailable)))
    }

    fn must_block(&self, key: &LeaderboardKey) -> bool {
        let guard = self.lock();
        let now = self.clock.now_ms();
        let max_stale = self.ttl_ms().saturating_mul(MAX_STALE_FACTOR);
        match guard.entries.get(key) {
            None => true,
            Some(entry) => now.saturating_sub(entry.stored_at_ms) >= max_stale,
        }
    }

    fn refresh_backoff_ms(&self) -> u64 {
        let ttl = self.ttl_ms().max(1);
        let factor = rand::thread_rng().gen_range(BACKOFF_MIN..=BACKOFF_MAX);
        (ttl.saturating_mul(factor) / 1000).max(1)
    }

    fn stale(&self, key: &LeaderboardKey) -> Option<CachedBoard> {
        let guard = self.lock();
        guard
            .entries
            .get(key)
            .map(|entry| entry.board.as_ref().clone())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|err| err.into_inner())
    }

    fn ttl_ms(&self) -> u64 {
        self.ttl.as_millis() as u64
    }

    fn jitter_millis(&self) -> u64 {
        match self.jitter {
            #[cfg(test)]
            JitterMode::Fixed(ms) => ms.clamp(JITTER_MIN, JITTER_MAX),
            JitterMode::Random => rand::thread_rng().gen_range(JITTER_MIN..=JITTER_MAX),
        }
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.lock().entries.len()
    }

    #[cfg(test)]
    fn follower_count(&self) -> u32 {
        self.followers.load(Ordering::SeqCst)
    }

    #[cfg(test)]
    fn inflight_len(&self) -> usize {
        self.lock().inflight.len()
    }

    #[cfg(test)]
    fn fresh_window_ms(&self, key: &LeaderboardKey) -> Option<u64> {
        self.lock().entries.get(key).map(|entry| entry.fresh_for_ms)
    }

    #[cfg(test)]
    fn next_refresh_ms(&self, key: &LeaderboardKey) -> Option<u64> {
        self.lock()
            .entries
            .get(key)
            .map(|entry| entry.next_refresh_ms)
    }
}

fn touch(inner: &mut Inner, key: &LeaderboardKey) {
    if let Some(pos) = inner.order.iter().position(|item| item == key) {
        inner.order.remove(pos);
        inner.order.push_back(key.clone());
    }
}

fn store(inner: &mut Inner, max_entries: usize, key: LeaderboardKey, entry: Entry) {
    if inner.entries.contains_key(&key) {
        touch(inner, &key);
        inner.entries.insert(key, entry);
        return;
    }
    while inner.entries.len() >= max_entries {
        let Some(old) = inner.order.pop_front() else {
            break;
        };
        inner.entries.remove(&old);
    }
    inner.order.push_back(key.clone());
    inner.entries.insert(key, entry);
}

/// Round `limit` up to 10, 25, 50, or 100. Values outside 1..=100 are clamped
/// first, so 7 and 10 share a bucket and 0 becomes 10.
pub fn limit_bucket(limit: u32) -> u32 {
    let limit = limit.clamp(1, 100);
    for bucket in LIMIT_BUCKETS {
        if limit <= bucket {
            return bucket;
        }
    }
    LIMIT_BUCKETS[LIMIT_BUCKETS.len() - 1]
}

/// Drop rows past the caller's limit. The cache stores the bucket; the
/// response is the prefix the caller asked for.
pub fn truncate_to_requested<T>(mut rows: Vec<T>, requested: u32) -> Vec<T> {
    let n = requested.clamp(1, 100) as usize;
    if rows.len() > n {
        rows.truncate(n);
    }
    rows
}

/// Background refreshes may use at most one fewer slot than `max_scans`,
/// and always at least one. The leftover slot is for blocking reads.
fn refresh_slots(max_scans: usize) -> usize {
    max_scans.saturating_sub(1).max(1)
}

fn fresh_for_ms(ttl_ms: u64, jitter_millis: u64) -> u64 {
    let jitter = jitter_millis.clamp(JITTER_MIN, JITTER_MAX);
    ttl_ms.saturating_mul(jitter) / 1000
}

/// Parse `LEADERBOARD_CACHE_MAX_SCANS`.
///
/// Missing, blank, and non-numeric values are the default (2). Integers
/// outside 1..=8 are clamped.
pub fn scans_from_raw(raw: Option<&str>) -> usize {
    let Some(text) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return DEFAULT_MAX_SCANS;
    };
    let Ok(n) = text.parse::<usize>() else {
        tracing::warn!(
            value = text,
            default = DEFAULT_MAX_SCANS,
            "LEADERBOARD_CACHE_MAX_SCANS is not an integer; using the default"
        );
        return DEFAULT_MAX_SCANS;
    };
    let clamped = n.clamp(MIN_MAX_SCANS, MAX_MAX_SCANS);
    if clamped != n {
        tracing::warn!(
            value = n,
            clamped,
            "LEADERBOARD_CACHE_MAX_SCANS is outside 1..=8; clamped"
        );
    }
    clamped
}

/// Parse `LEADERBOARD_CACHE_TTL_SECS`.
///
/// Missing, blank, and non-numeric values are the default (30s). Integers
/// outside 5..=300 are clamped. Callers log the clamp when it happens.
pub fn ttl_from_raw(raw: Option<&str>) -> Duration {
    let Some(text) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return Duration::from_secs(DEFAULT_TTL_SECS);
    };
    let Ok(secs) = text.parse::<u64>() else {
        tracing::warn!(
            value = text,
            default = DEFAULT_TTL_SECS,
            "LEADERBOARD_CACHE_TTL_SECS is not an integer; using the default"
        );
        return Duration::from_secs(DEFAULT_TTL_SECS);
    };
    let clamped = secs.clamp(MIN_TTL_SECS, MAX_TTL_SECS);
    if clamped != secs {
        tracing::warn!(
            value = secs,
            clamped,
            "LEADERBOARD_CACHE_TTL_SECS is outside 5..=300; clamped"
        );
    }
    Duration::from_secs(clamped)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct DbErr;

    impl LoadError for DbErr {
        fn on_error(&self) -> OnError {
            OnError::ServeStale
        }
    }

    #[derive(Debug)]
    struct Rejected;

    impl LoadError for Rejected {
        fn on_error(&self) -> OnError {
            OnError::Surface
        }
    }

    fn sample(id: &str, games: u32) -> MemberLeaderboardRow {
        MemberLeaderboardRow {
            member_id: id.to_string(),
            display_name: id.to_string(),
            games,
            winrate: 1.0,
            kd: games as f64,
        }
    }

    fn cache_with(ttl: Duration, max_entries: usize) -> (LeaderboardCache, ManualClock) {
        cache_built(ttl, max_entries, 8, JitterMode::Fixed(1000))
    }

    fn cache_built(
        ttl: Duration,
        max_entries: usize,
        scans: usize,
        jitter: JitterMode,
    ) -> (LeaderboardCache, ManualClock) {
        let clock = ManualClock::new();
        let cache =
            LeaderboardCache::build(ttl, max_entries, scans, Arc::new(clock.clone()), jitter);
        (cache, clock)
    }

    async fn wait_until(mut pred: impl FnMut() -> bool) {
        let start = Instant::now();
        while !pred() {
            if start.elapsed() > Duration::from_secs(5) {
                panic!("timed out waiting for the cache");
            }
            tokio::task::yield_now().await;
        }
    }

    fn public_key(metric: &str, limit: u32, season: &str, hero: &str) -> LeaderboardKey {
        LeaderboardKey::public_board(metric, limit, season, hero)
    }

    #[test]
    fn ttl_env_defaults_and_clamps() {
        assert_eq!(ttl_from_raw(None), Duration::from_secs(30));
        assert_eq!(ttl_from_raw(Some("")), Duration::from_secs(30));
        assert_eq!(ttl_from_raw(Some("   ")), Duration::from_secs(30));
        assert_eq!(ttl_from_raw(Some("nope")), Duration::from_secs(30));
        assert_eq!(ttl_from_raw(Some("30")), Duration::from_secs(30));
        assert_eq!(ttl_from_raw(Some("5")), Duration::from_secs(5));
        assert_eq!(ttl_from_raw(Some("4")), Duration::from_secs(5));
        assert_eq!(ttl_from_raw(Some("0")), Duration::from_secs(5));
        assert_eq!(ttl_from_raw(Some("300")), Duration::from_secs(300));
        assert_eq!(ttl_from_raw(Some("301")), Duration::from_secs(300));
        assert_eq!(ttl_from_raw(Some(" 45 ")), Duration::from_secs(45));

        assert_eq!(scans_from_raw(None), 2);
        assert_eq!(scans_from_raw(Some("")), 2);
        assert_eq!(scans_from_raw(Some("nope")), 2);
        assert_eq!(scans_from_raw(Some("2")), 2);
        assert_eq!(scans_from_raw(Some("0")), 1);
        assert_eq!(scans_from_raw(Some("1")), 1);
        assert_eq!(scans_from_raw(Some("8")), 8);
        assert_eq!(scans_from_raw(Some("9")), 8);
        assert_eq!(refresh_slots(1), 1);
        assert_eq!(refresh_slots(2), 1);
        assert_eq!(refresh_slots(8), 7);

        assert_eq!(limit_bucket(0), 10);
        assert_eq!(limit_bucket(1), 10);
        assert_eq!(limit_bucket(7), 10);
        assert_eq!(limit_bucket(10), 10);
        assert_eq!(limit_bucket(11), 25);
        assert_eq!(limit_bucket(25), 25);
        assert_eq!(limit_bucket(26), 50);
        assert_eq!(limit_bucket(50), 50);
        assert_eq!(limit_bucket(51), 100);
        assert_eq!(limit_bucket(100), 100);
        assert_eq!(limit_bucket(1000), 100);

        assert_eq!(fresh_for_ms(1_000, 800), 800);
        assert_eq!(fresh_for_ms(1_000, 1_000), 1_000);
        assert_eq!(fresh_for_ms(1_000, 1_200), 1_200);
        assert_eq!(fresh_for_ms(1_000, 100), 800);
        assert_eq!(fresh_for_ms(30, 800), 24);
    }

    #[tokio::test]
    async fn hit_skips_the_second_load_and_miss_loads_again_after_ttl() {
        let (cache, clock) = cache_with(Duration::from_millis(30), 8);
        let loads = Arc::new(AtomicU32::new(0));
        let key = public_key("winrate", 25, "", "");

        let first = cache
            .get_or_load(key.clone(), {
                let loads = Arc::clone(&loads);
                move || {
                    let loads = Arc::clone(&loads);
                    async move {
                        loads.fetch_add(1, Ordering::SeqCst);
                        Ok::<_, DbErr>(vec![sample("a", 1)])
                    }
                }
            })
            .await
            .unwrap();
        assert_eq!(first.rows[0].member_id, "a");
        assert!(first.cached_at.timestamp() > 0);

        let second = cache
            .get_or_load(key.clone(), {
                let loads = Arc::clone(&loads);
                move || {
                    let loads = Arc::clone(&loads);
                    async move {
                        loads.fetch_add(1, Ordering::SeqCst);
                        Ok::<_, DbErr>(vec![sample("b", 2)])
                    }
                }
            })
            .await
            .unwrap();
        assert_eq!(second.rows[0].member_id, "a");
        assert_eq!(second.cached_at, first.cached_at);
        assert_eq!(loads.load(Ordering::SeqCst), 1);

        clock.advance(29);
        let still = cache
            .get_or_load(key.clone(), {
                let loads = Arc::clone(&loads);
                move || {
                    let loads = Arc::clone(&loads);
                    async move {
                        loads.fetch_add(1, Ordering::SeqCst);
                        Ok::<_, DbErr>(vec![sample("c", 3)])
                    }
                }
            })
            .await
            .unwrap();
        assert_eq!(still.rows[0].member_id, "a");
        assert_eq!(loads.load(Ordering::SeqCst), 1);

        clock.advance(1);
        let expired = cache
            .get_or_load(key.clone(), {
                let loads = Arc::clone(&loads);
                move || {
                    let loads = Arc::clone(&loads);
                    async move {
                        loads.fetch_add(1, Ordering::SeqCst);
                        Ok::<_, DbErr>(vec![sample("d", 4)])
                    }
                }
            })
            .await
            .unwrap();
        // Past the fresh window the caller still gets the previous board.
        // The new rows land on a background refresh, not on this call.
        assert_eq!(expired.rows[0].member_id, "a");
        assert_eq!(expired.cached_at, first.cached_at);
        wait_until(|| loads.load(Ordering::SeqCst) >= 2 && cache.inflight_len() == 0).await;
        let refreshed = cache
            .get_or_load(key, || async {
                Ok::<_, DbErr>(vec![sample("should-not-load", 1)])
            })
            .await
            .unwrap();
        assert_eq!(refreshed.rows[0].member_id, "d");
        assert_eq!(loads.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn single_flight_one_load_for_concurrent_misses() {
        let (cache, _clock) = cache_with(Duration::from_secs(30), 8);
        let loads = Arc::new(AtomicU32::new(0));
        let (tx, rx) = watch::channel(false);
        let key = public_key("games", 25, "", "");
        let mut tasks = Vec::new();
        for _ in 0..8 {
            let cache = cache.clone();
            let loads = Arc::clone(&loads);
            let mut gate = rx.clone();
            let key = key.clone();
            tasks.push(tokio::spawn(async move {
                cache
                    .get_or_load(key, move || async move {
                        loads.fetch_add(1, Ordering::SeqCst);
                        while !*gate.borrow() {
                            if gate.changed().await.is_err() {
                                break;
                            }
                        }
                        Ok::<_, DbErr>(vec![sample("one", 1)])
                    })
                    .await
                    .unwrap()
            }));
        }

        let start = Instant::now();
        while cache.follower_count() < 7 || loads.load(Ordering::SeqCst) < 1 {
            if start.elapsed() > Duration::from_secs(5) {
                panic!(
                    "timed out waiting for followers: {} loads {}",
                    cache.follower_count(),
                    loads.load(Ordering::SeqCst)
                );
            }
            tokio::task::yield_now().await;
        }
        assert_eq!(loads.load(Ordering::SeqCst), 1);
        tx.send(true).unwrap();
        for task in tasks {
            let board = task.await.unwrap();
            assert_eq!(board.rows[0].member_id, "one");
        }
        assert_eq!(loads.load(Ordering::SeqCst), 1);
    }

    async fn load_labeled(
        cache: &LeaderboardCache,
        loads: &Arc<AtomicU32>,
        key: LeaderboardKey,
        id: &str,
    ) -> CachedBoard {
        let id_owned = id.to_string();
        let loads = Arc::clone(loads);
        cache
            .get_or_load(key, move || async move {
                loads.fetch_add(1, Ordering::SeqCst);
                Ok::<_, DbErr>(vec![sample(&id_owned, 1)])
            })
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn keys_split_on_params_and_audience() {
        let (cache, _clock) = cache_with(Duration::from_secs(30), 16);
        let loads = Arc::new(AtomicU32::new(0));
        let base = public_key("winrate", 25, "", "");
        let first = load_labeled(&cache, &loads, base.clone(), "row-0").await;
        assert_eq!(first.rows[0].member_id, "row-0");

        let again = cache
            .get_or_load(base.clone(), || async {
                Ok::<_, DbErr>(vec![sample("should-not-load", 1)])
            })
            .await
            .unwrap();
        assert_eq!(again.rows[0].member_id, "row-0");

        for (key, id) in [
            (public_key("kd", 25, "", ""), "by-metric"),
            (public_key("winrate", 10, "", ""), "by-limit"),
            (public_key("winrate", 25, "season-1", ""), "by-season"),
            (public_key("winrate", 25, "", "Ana"), "by-hero"),
        ] {
            let board = load_labeled(&cache, &loads, key, id).await;
            assert_eq!(board.rows[0].member_id, id);
        }

        let crew = LeaderboardKey {
            audience: LeaderboardAudience::Crew,
            ..base.clone()
        };
        let crew_board = load_labeled(&cache, &loads, crew, "crew-only").await;
        assert_eq!(crew_board.rows[0].member_id, "crew-only");

        let public_again = cache
            .get_or_load(base, || async {
                Ok::<_, DbErr>(vec![sample("should-not-load", 1)])
            })
            .await
            .unwrap();
        assert_eq!(public_again.rows[0].member_id, "row-0");
        assert_ne!(public_again.rows[0].member_id, "crew-only");
        assert_eq!(loads.load(Ordering::SeqCst), 6);
    }

    #[tokio::test]
    async fn public_error_does_not_return_a_crew_board() {
        let (cache, _clock) = cache_with(Duration::from_secs(30), 8);
        let crew = LeaderboardKey {
            audience: LeaderboardAudience::Crew,
            metric: "games".into(),
            limit: 25,
            season_id: String::new(),
            hero: String::new(),
        };
        cache
            .get_or_load(crew, || async {
                Ok::<_, DbErr>(vec![sample("crew-only", 9)])
            })
            .await
            .unwrap();

        let public_key = public_key("games", 25, "", "");
        let err = cache
            .get_or_load(public_key, || async {
                Err::<Vec<MemberLeaderboardRow>, DbErr>(DbErr)
            })
            .await;
        assert!(matches!(err, Err(CacheError::Load(DbErr))));
    }

    #[tokio::test]
    async fn error_is_not_cached_and_stale_same_key_is_served() {
        let (cache, clock) = cache_with(Duration::from_millis(10), 4);
        let key = public_key("games", 25, "", "");
        let loads = Arc::new(AtomicU32::new(0));

        cache
            .get_or_load(key.clone(), {
                let loads = Arc::clone(&loads);
                move || {
                    let loads = Arc::clone(&loads);
                    async move {
                        loads.fetch_add(1, Ordering::SeqCst);
                        Ok::<_, DbErr>(vec![sample("kept", 1)])
                    }
                }
            })
            .await
            .unwrap();

        clock.advance(10);
        let stale = cache
            .get_or_load(key.clone(), {
                let loads = Arc::clone(&loads);
                move || {
                    let loads = Arc::clone(&loads);
                    async move {
                        loads.fetch_add(1, Ordering::SeqCst);
                        Err::<Vec<MemberLeaderboardRow>, DbErr>(DbErr)
                    }
                }
            })
            .await
            .unwrap();
        assert_eq!(stale.rows[0].member_id, "kept");
        wait_until(|| loads.load(Ordering::SeqCst) >= 2 && cache.inflight_len() == 0).await;

        // The failed refresh must not make the entry fresh or store the error.
        let still = cache
            .get_or_load(key.clone(), {
                let loads = Arc::clone(&loads);
                move || {
                    let loads = Arc::clone(&loads);
                    async move {
                        loads.fetch_add(1, Ordering::SeqCst);
                        Ok::<_, DbErr>(vec![sample("new", 2)])
                    }
                }
            })
            .await
            .unwrap();
        assert_eq!(still.rows[0].member_id, "kept");
        wait_until(|| loads.load(Ordering::SeqCst) >= 3 && cache.inflight_len() == 0).await;
        let fresh = cache
            .get_or_load(key, || async {
                Ok::<_, DbErr>(vec![sample("should-not-load", 1)])
            })
            .await
            .unwrap();
        assert_eq!(fresh.rows[0].member_id, "new");
        assert_eq!(loads.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn surfaced_error_does_not_serve_stale() {
        let (cache, clock) = cache_with(Duration::from_millis(5), 4);
        let key = public_key("winrate", 25, "missing", "");
        cache
            .get_or_load(key.clone(), || async {
                Ok::<_, Rejected>(vec![sample("old", 1)])
            })
            .await
            .unwrap();
        // 10x the 5ms TTL: too old to serve while the refresh runs.
        clock.advance(50);
        let err = cache
            .get_or_load(key, || async {
                Err::<Vec<MemberLeaderboardRow>, Rejected>(Rejected)
            })
            .await;
        assert!(matches!(err, Err(CacheError::Load(Rejected))));
    }

    #[tokio::test]
    async fn load_finishing_during_uploads_is_stored_until_ttl() {
        let (cache, clock) = cache_with(Duration::from_millis(100), 4);
        let loads = Arc::new(AtomicU32::new(0));
        let uploads = Arc::new(AtomicU32::new(0));
        let (tx, rx) = watch::channel(false);
        let key = public_key("games", 25, "", "");
        let started = clock.wall();

        let cache_bg = cache.clone();
        let loads_bg = Arc::clone(&loads);
        let uploads_bg = Arc::clone(&uploads);
        let mut gate = rx.clone();
        let task = tokio::spawn(async move {
            cache_bg
                .get_or_load(key, move || {
                    let loads_bg = Arc::clone(&loads_bg);
                    let uploads_bg = Arc::clone(&uploads_bg);
                    async move {
                        loads_bg.fetch_add(1, Ordering::SeqCst);
                        while !*gate.borrow() {
                            if gate.changed().await.is_err() {
                                break;
                            }
                        }
                        assert!(
                            uploads_bg.load(Ordering::SeqCst) >= 1,
                            "uploads must overlap the in-flight query"
                        );
                        Ok::<_, DbErr>(vec![sample("held", 4)])
                    }
                })
                .await
                .unwrap()
        });

        let wait = Instant::now();
        while loads.load(Ordering::SeqCst) < 1 {
            if wait.elapsed() > Duration::from_secs(5) {
                panic!("load did not start");
            }
            tokio::task::yield_now().await;
        }
        // Uploads used to bump a generation and drop this result on the way out.
        uploads.fetch_add(3, Ordering::SeqCst);
        clock.advance(40);
        tx.send(true).unwrap();

        let board = task.await.unwrap();
        assert_eq!(board.rows[0].member_id, "held");
        assert_eq!(board.cached_at, started);

        let hit = cache
            .get_or_load(public_key("games", 25, "", ""), || async {
                Ok::<_, DbErr>(vec![sample("should-not-load", 1)])
            })
            .await
            .unwrap();
        assert_eq!(hit.rows[0].member_id, "held");
        assert_eq!(hit.cached_at, started);
        assert_eq!(loads.load(Ordering::SeqCst), 1);

        clock.advance(59);
        let still = cache
            .get_or_load(public_key("games", 25, "", ""), || async {
                Ok::<_, DbErr>(vec![sample("should-not-load", 1)])
            })
            .await
            .unwrap();
        assert_eq!(still.rows[0].member_id, "held");
        assert_eq!(loads.load(Ordering::SeqCst), 1);

        clock.advance(1);
        let expired = cache
            .get_or_load(public_key("games", 25, "", ""), {
                let loads = Arc::clone(&loads);
                move || {
                    let loads = Arc::clone(&loads);
                    async move {
                        loads.fetch_add(1, Ordering::SeqCst);
                        Ok::<_, DbErr>(vec![sample("after-ttl", 1)])
                    }
                }
            })
            .await
            .unwrap();
        assert_eq!(expired.rows[0].member_id, "held");
        assert_eq!(expired.cached_at, started);
        wait_until(|| loads.load(Ordering::SeqCst) >= 2 && cache.inflight_len() == 0).await;
        let refreshed = cache
            .get_or_load(public_key("games", 25, "", ""), || async {
                Ok::<_, DbErr>(vec![sample("should-not-load", 1)])
            })
            .await
            .unwrap();
        assert_eq!(refreshed.rows[0].member_id, "after-ttl");
        assert_eq!(loads.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn bounded_keys_evict_least_recently_used() {
        let (cache, _clock) = cache_with(Duration::from_secs(60), 2);
        let loads = Arc::new(AtomicU32::new(0));

        async fn put(cache: &LeaderboardCache, loads: &Arc<AtomicU32>, name: &str) {
            let key = public_key(name, 25, "", "");
            let name = name.to_string();
            let loads = Arc::clone(loads);
            cache
                .get_or_load(key, move || async move {
                    loads.fetch_add(1, Ordering::SeqCst);
                    Ok::<_, DbErr>(vec![sample(&name, 1)])
                })
                .await
                .unwrap();
        }

        put(&cache, &loads, "a").await;
        put(&cache, &loads, "b").await;
        assert_eq!(cache.len(), 2);
        // Touch "a" so "b" is the oldest.
        let key_a = public_key("a", 25, "", "");
        let touched = cache
            .get_or_load(key_a, || async {
                Ok::<_, DbErr>(vec![sample("reloaded-a", 1)])
            })
            .await
            .unwrap();
        assert_eq!(touched.rows[0].member_id, "a");
        put(&cache, &loads, "c").await;
        assert_eq!(cache.len(), 2);

        let key_a = public_key("a", 25, "", "");
        let survived = cache
            .get_or_load(key_a, || async {
                Ok::<_, DbErr>(vec![sample("reloaded-a", 1)])
            })
            .await
            .unwrap();
        assert_eq!(survived.rows[0].member_id, "a");

        let loads_before = loads.load(Ordering::SeqCst);
        let key_b = public_key("b", 25, "", "");
        cache
            .get_or_load(key_b, {
                let loads = Arc::clone(&loads);
                move || async move {
                    loads.fetch_add(1, Ordering::SeqCst);
                    Ok::<_, DbErr>(vec![sample("b", 1)])
                }
            })
            .await
            .unwrap();
        assert_eq!(loads.load(Ordering::SeqCst), loads_before + 1);
    }

    #[tokio::test]
    async fn stale_board_is_served_while_one_refresh_runs() {
        let (cache, clock) = cache_with(Duration::from_millis(20), 4);
        let loads = Arc::new(AtomicU32::new(0));
        let key = public_key("winrate", 25, "", "");
        let (tx, rx) = watch::channel(false);

        let first = cache
            .get_or_load(key.clone(), {
                let loads = Arc::clone(&loads);
                move || {
                    let loads = Arc::clone(&loads);
                    async move {
                        loads.fetch_add(1, Ordering::SeqCst);
                        Ok::<_, DbErr>(vec![sample("old", 1)])
                    }
                }
            })
            .await
            .unwrap();

        clock.advance(20);
        let mut gate = rx.clone();
        let stale = cache
            .get_or_load(key.clone(), {
                let loads = Arc::clone(&loads);
                move || {
                    let loads = Arc::clone(&loads);
                    async move {
                        loads.fetch_add(1, Ordering::SeqCst);
                        while !*gate.borrow() {
                            if gate.changed().await.is_err() {
                                break;
                            }
                        }
                        Ok::<_, DbErr>(vec![sample("new", 2)])
                    }
                }
            })
            .await
            .unwrap();
        assert_eq!(stale.rows[0].member_id, "old");
        assert_eq!(stale.cached_at, first.cached_at);
        wait_until(|| loads.load(Ordering::SeqCst) >= 2).await;

        let again = cache
            .get_or_load(key.clone(), {
                let loads = Arc::clone(&loads);
                move || {
                    let loads = Arc::clone(&loads);
                    async move {
                        loads.fetch_add(1, Ordering::SeqCst);
                        Ok::<_, DbErr>(vec![sample("extra", 3)])
                    }
                }
            })
            .await
            .unwrap();
        assert_eq!(again.rows[0].member_id, "old");
        assert_eq!(again.cached_at, first.cached_at);
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert_eq!(loads.load(Ordering::SeqCst), 2);

        tx.send(true).unwrap();
        wait_until(|| cache.inflight_len() == 0).await;
        let refreshed = cache
            .get_or_load(key, || async {
                Ok::<_, DbErr>(vec![sample("should-not-load", 1)])
            })
            .await
            .unwrap();
        assert_eq!(refreshed.rows[0].member_id, "new");
        assert_ne!(refreshed.cached_at, first.cached_at);
        assert_eq!(loads.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn jitter_windows_stay_inside_bounds() {
        let (cache, _clock) = cache_built(Duration::from_millis(1_000), 64, 8, JitterMode::Random);
        let mut saw_below = false;
        let mut saw_above = false;
        for i in 0..40 {
            let key = public_key("winrate", 25, &i.to_string(), "");
            cache
                .get_or_load(key.clone(), || async {
                    Ok::<_, DbErr>(vec![sample("x", 1)])
                })
                .await
                .unwrap();
            let window = cache.fresh_window_ms(&key).expect("stored");
            assert!(
                (800..=1_200).contains(&window),
                "fresh window {window} outside 0.8..=1.2 of the TTL"
            );
            if window < 1_000 {
                saw_below = true;
            }
            if window > 1_000 {
                saw_above = true;
            }
        }
        assert!(
            saw_below && saw_above,
            "expected jitter on both sides of the TTL"
        );
    }

    #[tokio::test]
    async fn scans_never_exceed_the_cap() {
        let (cache, _clock) = cache_built(Duration::from_secs(30), 16, 2, JitterMode::Fixed(1000));
        let inflight = Arc::new(AtomicU32::new(0));
        let peak = Arc::new(AtomicU32::new(0));
        let (tx, rx) = watch::channel(false);
        let mut tasks = Vec::new();
        for i in 0..6 {
            let cache = cache.clone();
            let inflight = Arc::clone(&inflight);
            let peak = Arc::clone(&peak);
            let mut gate = rx.clone();
            let key = public_key("games", 25, &i.to_string(), "");
            tasks.push(tokio::spawn(async move {
                cache
                    .get_or_load(key, move || {
                        let inflight = Arc::clone(&inflight);
                        let peak = Arc::clone(&peak);
                        async move {
                            let now = inflight.fetch_add(1, Ordering::SeqCst) + 1;
                            peak.fetch_max(now, Ordering::SeqCst);
                            while !*gate.borrow() {
                                if gate.changed().await.is_err() {
                                    break;
                                }
                            }
                            inflight.fetch_sub(1, Ordering::SeqCst);
                            Ok::<_, DbErr>(vec![sample("x", 1)])
                        }
                    })
                    .await
                    .unwrap()
            }));
        }

        wait_until(|| inflight.load(Ordering::SeqCst) >= 2).await;
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(inflight.load(Ordering::SeqCst) <= 2);
        assert!(peak.load(Ordering::SeqCst) <= 2);
        tx.send(true).unwrap();
        for task in tasks {
            task.await.unwrap();
        }
        assert!(peak.load(Ordering::SeqCst) <= 2);
    }

    #[tokio::test]
    async fn limit_seven_and_ten_share_a_key_and_the_response_has_seven_rows() {
        let (cache, _clock) = cache_with(Duration::from_secs(30), 4);
        let loads = Arc::new(AtomicU32::new(0));
        assert_eq!(limit_bucket(7), 10);
        assert_eq!(limit_bucket(10), 10);
        let key = public_key("games", limit_bucket(7), "", "");
        let stored = cache
            .get_or_load(key, {
                let loads = Arc::clone(&loads);
                move || {
                    let loads = Arc::clone(&loads);
                    async move {
                        loads.fetch_add(1, Ordering::SeqCst);
                        Ok::<_, DbErr>(
                            (0..10)
                                .map(|i| sample(&format!("m{i}"), i))
                                .collect::<Vec<_>>(),
                        )
                    }
                }
            })
            .await
            .unwrap();
        assert_eq!(stored.rows.len(), 10);

        let response = truncate_to_requested(stored.rows.clone(), 7);
        assert_eq!(response.len(), 7);
        assert_eq!(response[0].member_id, "m0");
        assert_eq!(response[6].member_id, "m6");

        let shared = cache
            .get_or_load(public_key("games", limit_bucket(10), "", ""), || async {
                Ok::<_, DbErr>(vec![sample("should-not-load", 1)])
            })
            .await
            .unwrap();
        assert_eq!(shared.rows.len(), 10);
        assert_eq!(loads.load(Ordering::SeqCst), 1);
        assert_eq!(truncate_to_requested(shared.rows, 10).len(), 10);
    }

    #[tokio::test]
    async fn max_staleness_waits_for_the_refresh() {
        let (cache, clock) = cache_with(Duration::from_millis(10), 4);
        let loads = Arc::new(AtomicU32::new(0));
        let done = Arc::new(AtomicU32::new(0));
        let key = public_key("kd", 25, "", "");
        let (tx, rx) = watch::channel(false);

        cache
            .get_or_load(key.clone(), {
                let loads = Arc::clone(&loads);
                move || {
                    let loads = Arc::clone(&loads);
                    async move {
                        loads.fetch_add(1, Ordering::SeqCst);
                        Ok::<_, DbErr>(vec![sample("old", 1)])
                    }
                }
            })
            .await
            .unwrap();

        clock.advance(100);
        let mut gate = rx.clone();
        let cache_bg = cache.clone();
        let loads_bg = Arc::clone(&loads);
        let done_bg = Arc::clone(&done);
        let task = tokio::spawn(async move {
            let board = cache_bg
                .get_or_load(key, move || {
                    let loads_bg = Arc::clone(&loads_bg);
                    async move {
                        loads_bg.fetch_add(1, Ordering::SeqCst);
                        while !*gate.borrow() {
                            if gate.changed().await.is_err() {
                                break;
                            }
                        }
                        Ok::<_, DbErr>(vec![sample("fresh", 2)])
                    }
                })
                .await
                .unwrap();
            done_bg.store(1, Ordering::SeqCst);
            board
        });

        wait_until(|| loads.load(Ordering::SeqCst) >= 2).await;
        tokio::time::sleep(Duration::from_millis(80)).await;
        assert_eq!(
            done.load(Ordering::SeqCst),
            0,
            "a board older than 10 TTLs must wait for the refresh"
        );

        tx.send(true).unwrap();
        let board = task.await.unwrap();
        assert_eq!(board.rows[0].member_id, "fresh");
        assert_eq!(done.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn cold_miss_does_not_wait_behind_a_busy_refresh() {
        let (cache, clock) = cache_built(Duration::from_millis(20), 8, 2, JitterMode::Fixed(1000));
        assert_eq!(refresh_slots(2), 1);
        let refresh_loads = Arc::new(AtomicU32::new(0));
        let cold_started = Arc::new(AtomicU32::new(0));
        let (hold_tx, hold_rx) = watch::channel(false);
        let refresh_key = public_key("winrate", 25, "refresh", "");

        cache
            .get_or_load(refresh_key.clone(), {
                let refresh_loads = Arc::clone(&refresh_loads);
                move || {
                    let refresh_loads = Arc::clone(&refresh_loads);
                    async move {
                        refresh_loads.fetch_add(1, Ordering::SeqCst);
                        Ok::<_, DbErr>(vec![sample("old", 1)])
                    }
                }
            })
            .await
            .unwrap();
        clock.advance(20);

        let mut hold = hold_rx.clone();
        let cache_bg = cache.clone();
        let refresh_loads_bg = Arc::clone(&refresh_loads);
        let refresh_task = tokio::spawn(async move {
            cache_bg
                .get_or_load(refresh_key, move || {
                    let refresh_loads_bg = Arc::clone(&refresh_loads_bg);
                    async move {
                        refresh_loads_bg.fetch_add(1, Ordering::SeqCst);
                        while !*hold.borrow() {
                            if hold.changed().await.is_err() {
                                break;
                            }
                        }
                        Ok::<_, DbErr>(vec![sample("refreshed", 2)])
                    }
                })
                .await
                .unwrap()
        });
        wait_until(|| refresh_loads.load(Ordering::SeqCst) >= 2).await;

        let cold = tokio::time::timeout(Duration::from_millis(500), {
            cache.get_or_load(public_key("winrate", 25, "cold", ""), {
                let cold_started = Arc::clone(&cold_started);
                move || {
                    let cold_started = Arc::clone(&cold_started);
                    async move {
                        cold_started.fetch_add(1, Ordering::SeqCst);
                        Ok::<_, DbErr>(vec![sample("cold", 1)])
                    }
                }
            })
        })
        .await
        .expect("cold miss waited behind the refresh")
        .unwrap();
        assert_eq!(cold.rows[0].member_id, "cold");
        assert_eq!(cold_started.load(Ordering::SeqCst), 1);
        assert_eq!(
            refresh_loads.load(Ordering::SeqCst),
            2,
            "the refresh must still be the only scan on its key"
        );

        hold_tx.send(true).unwrap();
        let refreshed = refresh_task.await.unwrap();
        assert_eq!(refreshed.rows[0].member_id, "old");
    }

    #[tokio::test]
    async fn refresh_without_a_slot_is_skipped_and_stale_is_served() {
        let (cache, clock) = cache_built(Duration::from_millis(20), 8, 2, JitterMode::Fixed(1000));
        let (hold_tx, hold_rx) = watch::channel(false);
        let busy_loads = Arc::new(AtomicU32::new(0));
        let skipped_loads = Arc::new(AtomicU32::new(0));
        let busy_key = public_key("games", 25, "busy", "");
        let skipped_key = public_key("games", 25, "skipped", "");

        cache
            .get_or_load(busy_key.clone(), {
                let busy_loads = Arc::clone(&busy_loads);
                move || {
                    let busy_loads = Arc::clone(&busy_loads);
                    async move {
                        busy_loads.fetch_add(1, Ordering::SeqCst);
                        Ok::<_, DbErr>(vec![sample("busy-old", 1)])
                    }
                }
            })
            .await
            .unwrap();
        let first = cache
            .get_or_load(skipped_key.clone(), {
                let skipped_loads = Arc::clone(&skipped_loads);
                move || {
                    let skipped_loads = Arc::clone(&skipped_loads);
                    async move {
                        skipped_loads.fetch_add(1, Ordering::SeqCst);
                        Ok::<_, DbErr>(vec![sample("kept", 1)])
                    }
                }
            })
            .await
            .unwrap();
        clock.advance(20);

        let mut hold = hold_rx.clone();
        let cache_bg = cache.clone();
        let busy_loads_bg = Arc::clone(&busy_loads);
        let busy_task = tokio::spawn(async move {
            cache_bg
                .get_or_load(busy_key, move || {
                    let busy_loads_bg = Arc::clone(&busy_loads_bg);
                    async move {
                        busy_loads_bg.fetch_add(1, Ordering::SeqCst);
                        while !*hold.borrow() {
                            if hold.changed().await.is_err() {
                                break;
                            }
                        }
                        Ok::<_, DbErr>(vec![sample("busy-new", 2)])
                    }
                })
                .await
                .unwrap()
        });
        wait_until(|| busy_loads.load(Ordering::SeqCst) >= 2).await;

        let started = Instant::now();
        let served = tokio::time::timeout(Duration::from_millis(200), {
            let skipped_loads = Arc::clone(&skipped_loads);
            cache.get_or_load(skipped_key.clone(), move || {
                let skipped_loads = Arc::clone(&skipped_loads);
                async move {
                    skipped_loads.fetch_add(1, Ordering::SeqCst);
                    Ok::<_, DbErr>(vec![sample("should-not-load", 2)])
                }
            })
        })
        .await
        .expect("a skipped refresh must not block")
        .unwrap();
        assert!(started.elapsed() < Duration::from_millis(200));
        assert_eq!(served.rows[0].member_id, "kept");
        assert_eq!(served.cached_at, first.cached_at);
        assert_eq!(skipped_loads.load(Ordering::SeqCst), 1);

        let again = cache
            .get_or_load(skipped_key.clone(), {
                let skipped_loads = Arc::clone(&skipped_loads);
                move || {
                    let skipped_loads = Arc::clone(&skipped_loads);
                    async move {
                        skipped_loads.fetch_add(1, Ordering::SeqCst);
                        Ok::<_, DbErr>(vec![sample("should-not-load", 3)])
                    }
                }
            })
            .await
            .unwrap();
        assert_eq!(again.rows[0].member_id, "kept");
        assert_eq!(skipped_loads.load(Ordering::SeqCst), 1);

        hold_tx.send(true).unwrap();
        busy_task.await.unwrap();
        // The finished scan hands its slot to the queued loader. That loader
        // is the first skip, which stores "should-not-load" without waiting
        // out the backoff.
        wait_until(|| skipped_loads.load(Ordering::SeqCst) >= 2 && cache.inflight_len() == 0).await;
        let stored = cache
            .get_or_load(public_key("games", 25, "skipped", ""), || async {
                Ok::<_, DbErr>(vec![sample("should-not-load", 5)])
            })
            .await
            .unwrap();
        assert_eq!(stored.rows[0].member_id, "should-not-load");
        assert_eq!(skipped_loads.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn blocking_miss_joins_the_inflight_refresh() {
        let (cache, clock) = cache_built(Duration::from_millis(20), 8, 2, JitterMode::Fixed(1000));
        let loads = Arc::new(AtomicU32::new(0));
        let done = Arc::new(AtomicU32::new(0));
        let key = public_key("kd", 25, "join", "");
        let (tx, rx) = watch::channel(false);

        cache
            .get_or_load(key.clone(), {
                let loads = Arc::clone(&loads);
                move || {
                    let loads = Arc::clone(&loads);
                    async move {
                        loads.fetch_add(1, Ordering::SeqCst);
                        Ok::<_, DbErr>(vec![sample("old", 1)])
                    }
                }
            })
            .await
            .unwrap();
        clock.advance(20);

        let mut gate = rx.clone();
        let cache_bg = cache.clone();
        let loads_bg = Arc::clone(&loads);
        let refresh_key = key.clone();
        let refresh_task = tokio::spawn(async move {
            cache_bg
                .get_or_load(refresh_key, move || {
                    let loads_bg = Arc::clone(&loads_bg);
                    async move {
                        loads_bg.fetch_add(1, Ordering::SeqCst);
                        while !*gate.borrow() {
                            if gate.changed().await.is_err() {
                                break;
                            }
                        }
                        Ok::<_, DbErr>(vec![sample("shared", 2)])
                    }
                })
                .await
                .unwrap()
        });
        wait_until(|| loads.load(Ordering::SeqCst) >= 2).await;

        // 10x the 20ms TTL, while that refresh is still the in-flight scan.
        clock.advance(180);
        let mut join_gate = rx.clone();
        let cache_bg = cache.clone();
        let loads_bg = Arc::clone(&loads);
        let done_bg = Arc::clone(&done);
        let join_key = key.clone();
        let join_task = tokio::spawn(async move {
            let board = cache_bg
                .get_or_load(join_key, move || {
                    let loads_bg = Arc::clone(&loads_bg);
                    async move {
                        loads_bg.fetch_add(1, Ordering::SeqCst);
                        while !*join_gate.borrow() {
                            if join_gate.changed().await.is_err() {
                                break;
                            }
                        }
                        Ok::<_, DbErr>(vec![sample("second-scan", 3)])
                    }
                })
                .await
                .unwrap();
            done_bg.store(1, Ordering::SeqCst);
            board
        });

        tokio::time::sleep(Duration::from_millis(80)).await;
        assert_eq!(done.load(Ordering::SeqCst), 0);
        assert_eq!(loads.load(Ordering::SeqCst), 2);

        tx.send(true).unwrap();
        let joined = join_task.await.unwrap();
        let refreshed = refresh_task.await.unwrap();
        assert_eq!(joined.rows[0].member_id, "shared");
        assert_eq!(refreshed.rows[0].member_id, "old");
        assert_eq!(loads.load(Ordering::SeqCst), 2);
    }

    async fn wait_gate(mut gate: watch::Receiver<bool>) {
        while !*gate.borrow() {
            if gate.changed().await.is_err() {
                break;
            }
        }
    }

    #[tokio::test]
    async fn finished_scan_starts_the_stalest_waiting_refresh() {
        let (cache, clock) = cache_built(Duration::from_millis(100), 8, 2, JitterMode::Fixed(1000));
        let a_loads = Arc::new(AtomicU32::new(0));
        let b_loads = Arc::new(AtomicU32::new(0));
        let h_loads = Arc::new(AtomicU32::new(0));
        let (a_tx, a_rx) = watch::channel(false);
        let (b_tx, b_rx) = watch::channel(false);
        let (h_tx, h_rx) = watch::channel(false);
        let a_key = public_key("games", 25, "a", "");
        let b_key = public_key("games", 25, "b", "");
        let h_key = public_key("games", 25, "h", "");

        cache
            .get_or_load(a_key.clone(), {
                let a_loads = Arc::clone(&a_loads);
                move || {
                    let a_loads = Arc::clone(&a_loads);
                    async move {
                        a_loads.fetch_add(1, Ordering::SeqCst);
                        Ok::<_, DbErr>(vec![sample("a-old", 1)])
                    }
                }
            })
            .await
            .unwrap();
        clock.advance(10);
        cache
            .get_or_load(b_key.clone(), {
                let b_loads = Arc::clone(&b_loads);
                move || {
                    let b_loads = Arc::clone(&b_loads);
                    async move {
                        b_loads.fetch_add(1, Ordering::SeqCst);
                        Ok::<_, DbErr>(vec![sample("b-old", 1)])
                    }
                }
            })
            .await
            .unwrap();
        cache
            .get_or_load(h_key.clone(), {
                let h_loads = Arc::clone(&h_loads);
                move || {
                    let h_loads = Arc::clone(&h_loads);
                    async move {
                        h_loads.fetch_add(1, Ordering::SeqCst);
                        Ok::<_, DbErr>(vec![sample("h-old", 1)])
                    }
                }
            })
            .await
            .unwrap();
        // Fresh window is 100ms. At t=120 every board is stale, and A (t=0)
        // is older than B (t=10).
        clock.advance(110);

        let h_gate = h_rx.clone();
        let cache_bg = cache.clone();
        let h_loads_bg = Arc::clone(&h_loads);
        let h_task = tokio::spawn(async move {
            cache_bg
                .get_or_load(h_key, move || {
                    let h_loads_bg = Arc::clone(&h_loads_bg);
                    async move {
                        h_loads_bg.fetch_add(1, Ordering::SeqCst);
                        wait_gate(h_gate).await;
                        Ok::<_, DbErr>(vec![sample("h-new", 2)])
                    }
                })
                .await
                .unwrap()
        });
        wait_until(|| h_loads.load(Ordering::SeqCst) >= 2).await;

        let a_gate = a_rx.clone();
        let served_a = cache
            .get_or_load(a_key, {
                let a_loads = Arc::clone(&a_loads);
                move || {
                    let a_loads = Arc::clone(&a_loads);
                    async move {
                        a_loads.fetch_add(1, Ordering::SeqCst);
                        wait_gate(a_gate).await;
                        Ok::<_, DbErr>(vec![sample("a-new", 2)])
                    }
                }
            })
            .await
            .unwrap();
        assert_eq!(served_a.rows[0].member_id, "a-old");
        assert_eq!(a_loads.load(Ordering::SeqCst), 1);

        let b_gate = b_rx.clone();
        let served_b = cache
            .get_or_load(b_key, {
                let b_loads = Arc::clone(&b_loads);
                move || {
                    let b_loads = Arc::clone(&b_loads);
                    async move {
                        b_loads.fetch_add(1, Ordering::SeqCst);
                        wait_gate(b_gate).await;
                        Ok::<_, DbErr>(vec![sample("b-new", 2)])
                    }
                }
            })
            .await
            .unwrap();
        assert_eq!(served_b.rows[0].member_id, "b-old");
        assert_eq!(b_loads.load(Ordering::SeqCst), 1);

        h_tx.send(true).unwrap();
        h_task.await.unwrap();
        wait_until(|| a_loads.load(Ordering::SeqCst) >= 2).await;
        assert_eq!(
            b_loads.load(Ordering::SeqCst),
            1,
            "the fresher waiter must not start while the older one holds the slot"
        );

        a_tx.send(true).unwrap();
        wait_until(|| b_loads.load(Ordering::SeqCst) >= 2).await;
        b_tx.send(true).unwrap();
        wait_until(|| cache.inflight_len() == 0).await;
    }

    #[tokio::test]
    async fn skipped_keys_do_not_retry_on_the_same_instant() {
        let (cache, clock) =
            cache_built(Duration::from_millis(1000), 64, 2, JitterMode::Fixed(1000));
        let (hold_tx, hold_rx) = watch::channel(false);
        let hold_loads = Arc::new(AtomicU32::new(0));
        let hold_key = public_key("games", 25, "hold", "");
        cache
            .get_or_load(hold_key.clone(), {
                let hold_loads = Arc::clone(&hold_loads);
                move || {
                    let hold_loads = Arc::clone(&hold_loads);
                    async move {
                        hold_loads.fetch_add(1, Ordering::SeqCst);
                        Ok::<_, DbErr>(vec![sample("hold-old", 1)])
                    }
                }
            })
            .await
            .unwrap();
        for i in 0..24 {
            cache
                .get_or_load(public_key("games", 25, &format!("k{i}"), ""), || async {
                    Ok::<_, DbErr>(vec![sample("old", 1)])
                })
                .await
                .unwrap();
        }
        clock.advance(1000);

        let hold_gate = hold_rx.clone();
        let cache_bg = cache.clone();
        let hold_loads_bg = Arc::clone(&hold_loads);
        let hold_task = tokio::spawn(async move {
            cache_bg
                .get_or_load(hold_key, move || {
                    let hold_loads_bg = Arc::clone(&hold_loads_bg);
                    async move {
                        hold_loads_bg.fetch_add(1, Ordering::SeqCst);
                        wait_gate(hold_gate).await;
                        Ok::<_, DbErr>(vec![sample("hold-new", 2)])
                    }
                })
                .await
                .unwrap()
        });
        wait_until(|| hold_loads.load(Ordering::SeqCst) >= 2).await;

        let now = clock.now_ms();
        let mut deltas = Vec::new();
        for i in 0..24 {
            let key = public_key("games", 25, &format!("k{i}"), "");
            let served = cache
                .get_or_load(key.clone(), || async {
                    Ok::<_, DbErr>(vec![sample("should-not-run", 2)])
                })
                .await
                .unwrap();
            assert_eq!(served.rows[0].member_id, "old");
            let next = cache.next_refresh_ms(&key).unwrap();
            let delta = next.saturating_sub(now);
            assert!(
                (250..=1000).contains(&delta),
                "backoff {delta} ms is outside 0.25x..=1x TTL"
            );
            deltas.push(delta);
        }
        assert!(
            deltas.iter().any(|delta| *delta != deltas[0]),
            "every skipped key retried at {deltas:?}"
        );

        hold_tx.send(true).unwrap();
        hold_task.await.unwrap();
        wait_until(|| cache.inflight_len() == 0).await;
    }

    #[tokio::test]
    async fn one_grouped_load_serves_every_hero() {
        assert_eq!(
            scuffed_types::HEROES.len(),
            54,
            "the QA harness's 54 keys are one per hero"
        );
        let (cache, _) = cache_with(Duration::from_secs(30), 8);
        let loads = Arc::new(AtomicU32::new(0));
        let key = public_key("scan", 0, "season", "");
        let loads_bg = Arc::clone(&loads);
        let board = cache
            .get_or_load(key.clone(), move || {
                let loads_bg = Arc::clone(&loads_bg);
                async move {
                    loads_bg.fetch_add(1, Ordering::SeqCst);
                    let snapshot = scuffed_db::LeaderboardSnapshot::from_rows(vec![
                        ("Ana".into(), hero_agg("a", 6, 6)),
                        ("Genji".into(), hero_agg("a", 3, 0)),
                        ("Genji".into(), hero_agg("b", 8, 4)),
                    ]);
                    Ok::<_, DbErr>(LoadedBoard {
                        rows: Vec::new(),
                        snapshot: Some(Arc::new(snapshot)),
                    })
                }
            })
            .await
            .unwrap();
        assert!(board.snapshot.is_some());

        let loads_bg = Arc::clone(&loads);
        let again = cache
            .get_or_load(key, move || {
                let loads_bg = Arc::clone(&loads_bg);
                async move {
                    loads_bg.fetch_add(1, Ordering::SeqCst);
                    Ok::<_, DbErr>(LoadedBoard {
                        rows: vec![sample("nope", 1)],
                        snapshot: None,
                    })
                }
            })
            .await
            .unwrap();
        assert_eq!(loads.load(Ordering::SeqCst), 1);
        let snapshot = again.snapshot.expect("grouped snapshot is stored");
        let ana = snapshot.project("games", "Ana", 10);
        assert_eq!(ana.len(), 1);
        assert_eq!(ana[0].member_id, "a");
        assert_eq!(ana[0].games, 6);
        let genji = snapshot.project("winrate", "Genji", 10);
        assert_eq!(genji.len(), 1);
        assert_eq!(genji[0].member_id, "b");
        let all = snapshot.project("games", "", 2);
        assert_eq!(all[0].member_id, "a");
        assert_eq!(all[0].games, 9);
        assert_eq!(all[1].member_id, "b");
        assert_eq!(all[1].games, 8);
        let top = snapshot.project("games", "", 1);
        assert_eq!(top.len(), 1);
        assert_eq!(top[0].member_id, all[0].member_id);
    }

    fn hero_agg(id: &str, games: u32, wins: u32) -> scuffed_db::LeaderboardHeroAgg {
        scuffed_db::LeaderboardHeroAgg {
            member_id: id.to_string(),
            display_name: id.to_string(),
            games,
            wins,
            elims: wins,
            deaths: 1,
        }
    }
}
