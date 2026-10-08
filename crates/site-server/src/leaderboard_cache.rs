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
//! request waits for the refresh. At most a few scans run at once.
//!
//! The cache is per process. A restart clears it, and two instances do not
//! share it. A failed load is not stored. If a previous board for the same
//! key is still in memory, that board is served instead of the error.

use std::collections::{HashMap, VecDeque};
use std::future::Future;
#[cfg(test)]
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use rand::Rng;
use scuffed_types::MemberLeaderboardRow;
use tokio::sync::{Semaphore, watch};

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

/// `limit` values the cache actually stores. Callers round up into one of these.
pub const LIMIT_BUCKETS: [u32; 4] = [10, 25, 50, 100];

/// Who the board was built for.
///
/// The public route only stores [`LeaderboardAudience::Public`]. Anonymous
/// and logged-in callers share that slot: the query already drops inactive
/// members, and the handler does not read the session. `Crew` exists so a
/// later crew-only board cannot be written into a slot an anonymous caller
/// reads.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum LeaderboardAudience {
    Public,
    /// Not produced by the public handler. See the enum docs.
    #[cfg_attr(not(test), allow(dead_code))]
    Crew,
}

/// Everything that changes the board.
///
/// Live query parameters on `GET /api/public/leaderboards` are `metric`
/// (the sort), `limit`, `season`, and `hero`. `limit` is stored as a
/// bucket from [`limit_bucket`], not the raw request. There is no role
/// filter and no game-mode filter on this handler, so those are not key
/// fields.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
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
#[derive(Clone, Debug)]
pub struct CachedBoard {
    pub rows: Vec<MemberLeaderboardRow>,
    pub cached_at: DateTime<Utc>,
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
}

#[derive(Default)]
struct Inner {
    order: VecDeque<LeaderboardKey>,
    entries: HashMap<LeaderboardKey, Entry>,
    inflight: HashMap<LeaderboardKey, watch::Receiver<FlightState>>,
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

/// Process-local leaderboard cache. Cheap to clone (`Arc` inside).
#[derive(Clone)]
pub struct LeaderboardCache {
    inner: Arc<Mutex<Inner>>,
    ttl: Duration,
    max_entries: usize,
    scans: Arc<Semaphore>,
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
            jitter,
            clock,
            #[cfg(test)]
            followers: Arc::new(AtomicU32::new(0)),
        }
    }

    pub async fn get_or_load<F, Fut, E>(
        &self,
        key: LeaderboardKey,
        load: F,
    ) -> Result<CachedBoard, CacheError<E>>
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = Result<Vec<MemberLeaderboardRow>, E>> + Send + 'static,
        E: LoadError + Send + 'static,
    {
        match self.begin(&key) {
            Begin::Hit(board) | Begin::ServeStale { board, lead: None } => Ok(board),
            Begin::ServeStale {
                board,
                lead: Some(tx),
            } => {
                let cache = self.clone();
                let key = key.clone();
                tokio::spawn(async move {
                    let _ = cache.lead(key, tx, load).await;
                });
                Ok(board)
            }
            Begin::Wait(rx) => self.finish_follower(rx, &key).await,
            Begin::Lead { tx } => self.lead(key, tx, load).await,
        }
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
            )
        });
        if let Some((board, age, fresh_for_ms)) = stored {
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

    async fn lead<F, Fut, E>(
        &self,
        key: LeaderboardKey,
        tx: watch::Sender<FlightState>,
        load: F,
    ) -> Result<CachedBoard, CacheError<E>>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<Vec<MemberLeaderboardRow>, E>>,
        E: LoadError + Send + 'static,
    {
        // Cold loads and background refreshes share this cap. Waiting here
        // is the point: a wave of expiries must not all scan at once.
        let permit = self.scans.clone().acquire_owned().await;
        // Stamp the board when the query starts. A scan that runs for a
        // second must not be labeled as if it finished just now.
        let started_ms = self.clock.now_ms();
        let started = self.clock.wall();
        let fresh_for_ms = fresh_for_ms(self.ttl_ms(), self.jitter_millis());
        let result = load().await;
        drop(permit);
        let mut guard = self.lock();
        guard.inflight.remove(&key);
        match result {
            Ok(rows) => {
                let board = Arc::new(CachedBoard {
                    rows,
                    cached_at: started,
                });
                store(
                    &mut guard,
                    self.max_entries,
                    key,
                    Entry {
                        board: Arc::clone(&board),
                        stored_at_ms: started_ms,
                        fresh_for_ms,
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
                    return Ok(stale.board.as_ref().clone());
                }
                Err(CacheError::Load(err))
            }
        }
    }

    async fn finish_follower<E>(
        &self,
        mut rx: watch::Receiver<FlightState>,
        key: &LeaderboardKey,
    ) -> Result<CachedBoard, CacheError<E>> {
        loop {
            let state = rx.borrow().clone();
            match state {
                FlightState::Pending => {
                    if rx.changed().await.is_err() {
                        break;
                    }
                }
                FlightState::Ready(board) => return Ok(board.as_ref().clone()),
                FlightState::Failed(FailKind::Rejected) => {
                    return Err(CacheError::Leader(FailKind::Rejected));
                }
                FlightState::Failed(FailKind::Unavailable) => {
                    if let Some(board) = self.stale(key) {
                        return Ok(board);
                    }
                    return Err(CacheError::Leader(FailKind::Unavailable));
                }
            }
        }
        if let Some(board) = self.stale(key) {
            return Ok(board);
        }
        Err(CacheError::Leader(FailKind::Unavailable))
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
                        Ok::<_, DbErr>((0..10).map(|i| sample(&format!("m{i}"), i)).collect())
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
}
