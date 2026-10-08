//! In-process cache for `GET /api/public/leaderboards`.
//!
//! The query aggregates every `personal_match` row. A load test at main
//! `9590e52` measured p95 322 ms at 60k rows, 1.2 s at 300k, and 5.7 s at
//! 1.5M, and those scans queued behind the single Surreal socket so uploads
//! slowed down too. Results only change when a game is uploaded, so a short
//! TTL plus a generation bump on upload is enough.
//!
//! The cache is per process. A restart clears it, and two instances do not
//! share it. Correctness does not depend on the generation bump: a missed
//! invalidation still expires within the TTL. A failed load is not stored.
//! If a previous board for the same key is still in memory, that board is
//! served instead of the error.

use std::collections::{HashMap, VecDeque};
use std::future::Future;
#[cfg(test)]
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use scuffed_types::MemberLeaderboardRow;
use tokio::sync::watch;

/// Default lifetime when `LEADERBOARD_CACHE_TTL_SECS` is unset, blank, or not an integer.
pub const DEFAULT_TTL_SECS: u64 = 30;
/// Lower clamp for `LEADERBOARD_CACHE_TTL_SECS`.
pub const MIN_TTL_SECS: u64 = 5;
/// Upper clamp for `LEADERBOARD_CACHE_TTL_SECS`.
pub const MAX_TTL_SECS: u64 = 300;
/// Cap on distinct query keys. Extra keys evict the least recently used.
pub const MAX_KEYS: usize = 64;

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
/// (the sort), `limit`, `season`, and `hero`. There is no role filter and
/// no game-mode filter on this handler, so those are not key fields.
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

/// A stored board. `cached_at` is wall time from when the load finished.
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
    stored_at_ms: u64,
    generation: u64,
}

#[derive(Clone)]
enum FlightState {
    Pending,
    Ready(Arc<CachedBoard>),
    Failed(FailKind),
}

#[derive(Default)]
struct Inner {
    generation: u64,
    order: VecDeque<LeaderboardKey>,
    entries: HashMap<LeaderboardKey, Entry>,
    inflight: HashMap<LeaderboardKey, watch::Receiver<FlightState>>,
}

trait CacheClock: Send + Sync {
    fn now_ms(&self) -> u64;
}

struct SystemClock {
    start: Instant,
}

impl CacheClock for SystemClock {
    fn now_ms(&self) -> u64 {
        self.start.elapsed().as_millis() as u64
    }
}

#[cfg(test)]
#[derive(Clone)]
struct ManualClock {
    ms: Arc<AtomicU64>,
}

#[cfg(test)]
impl ManualClock {
    fn new() -> Self {
        Self {
            ms: Arc::new(AtomicU64::new(0)),
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
}

enum Begin {
    Hit(CachedBoard),
    Wait(watch::Receiver<FlightState>),
    Lead {
        generation: u64,
        tx: watch::Sender<FlightState>,
    },
}

/// Process-local leaderboard cache. Cheap to clone (`Arc` inside).
#[derive(Clone)]
pub struct LeaderboardCache {
    inner: Arc<Mutex<Inner>>,
    ttl: Duration,
    max_entries: usize,
    clock: Arc<dyn CacheClock>,
    #[cfg(test)]
    followers: Arc<AtomicU32>,
}

impl LeaderboardCache {
    /// TTL from `LEADERBOARD_CACHE_TTL_SECS`, clamped to 5..=300 seconds.
    /// Unset, blank, and non-numeric values use 30 seconds.
    pub fn from_env() -> Self {
        let raw = std::env::var("LEADERBOARD_CACHE_TTL_SECS").ok();
        Self::with_clock(
            ttl_from_raw(raw.as_deref()),
            MAX_KEYS,
            Arc::new(SystemClock {
                start: Instant::now(),
            }),
        )
    }

    fn with_clock(ttl: Duration, max_entries: usize, clock: Arc<dyn CacheClock>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner::default())),
            ttl,
            max_entries: max_entries.max(1),
            clock,
            #[cfg(test)]
            followers: Arc::new(AtomicU32::new(0)),
        }
    }

    /// Drop every fresh hit. In-flight loads still finish, but they do not
    /// store if this generation moved while they were running. Previous
    /// boards stay available as the stale fallback when a reload fails.
    pub fn invalidate(&self) {
        let mut guard = self.lock();
        guard.generation = guard.generation.wrapping_add(1);
    }

    pub async fn get_or_load<F, Fut, E>(
        &self,
        key: LeaderboardKey,
        load: F,
    ) -> Result<CachedBoard, CacheError<E>>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<Vec<MemberLeaderboardRow>, E>>,
        E: LoadError,
    {
        match self.begin(&key) {
            Begin::Hit(board) => Ok(board),
            Begin::Wait(rx) => self.finish_follower(rx, &key).await,
            Begin::Lead { generation, tx } => self.lead(key, generation, tx, load).await,
        }
    }

    fn begin(&self, key: &LeaderboardKey) -> Begin {
        let mut guard = self.lock();
        if let Some(board) = take_fresh(&mut guard, key, self.ttl_ms(), self.clock.now_ms()) {
            return Begin::Hit(board);
        }
        if let Some(rx) = guard.inflight.get(key) {
            #[cfg(test)]
            self.followers.fetch_add(1, Ordering::SeqCst);
            return Begin::Wait(rx.clone());
        }
        let (tx, rx) = watch::channel(FlightState::Pending);
        guard.inflight.insert(key.clone(), rx);
        let generation = guard.generation;
        Begin::Lead { generation, tx }
    }

    async fn lead<F, Fut, E>(
        &self,
        key: LeaderboardKey,
        generation: u64,
        tx: watch::Sender<FlightState>,
        load: F,
    ) -> Result<CachedBoard, CacheError<E>>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<Vec<MemberLeaderboardRow>, E>>,
        E: LoadError,
    {
        let result = load().await;
        let now_ms = self.clock.now_ms();
        let mut guard = self.lock();
        // Drop the flight before storing so a request that arrives after an
        // invalidation starts its own load instead of joining this one.
        guard.inflight.remove(&key);
        match result {
            Ok(rows) => {
                let board = Arc::new(CachedBoard {
                    rows,
                    cached_at: Utc::now(),
                });
                if guard.generation == generation {
                    store(
                        &mut guard,
                        self.max_entries,
                        key,
                        Entry {
                            board: Arc::clone(&board),
                            stored_at_ms: now_ms,
                            generation,
                        },
                    );
                }
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

    #[cfg(test)]
    fn len(&self) -> usize {
        self.lock().entries.len()
    }

    #[cfg(test)]
    fn follower_count(&self) -> u32 {
        self.followers.load(Ordering::SeqCst)
    }
}

fn take_fresh(
    inner: &mut Inner,
    key: &LeaderboardKey,
    ttl_ms: u64,
    now_ms: u64,
) -> Option<CachedBoard> {
    let entry = inner.entries.get(key)?;
    if entry.generation != inner.generation {
        return None;
    }
    if now_ms.saturating_sub(entry.stored_at_ms) >= ttl_ms {
        return None;
    }
    let board = entry.board.as_ref().clone();
    touch(inner, key);
    Some(board)
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
        let clock = ManualClock::new();
        let cache = LeaderboardCache::with_clock(ttl, max_entries, Arc::new(clock.clone()));
        (cache, clock)
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
            .get_or_load(key, {
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
        assert_eq!(expired.rows[0].member_id, "d");
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
                    .get_or_load(key, || async move {
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

        // The error must not refresh the entry or be stored as success.
        clock.advance(10);
        let fresh = cache
            .get_or_load(key, {
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
        clock.advance(5);
        let err = cache
            .get_or_load(key, || async {
                Err::<Vec<MemberLeaderboardRow>, Rejected>(Rejected)
            })
            .await;
        assert!(matches!(err, Err(CacheError::Load(Rejected))));
    }

    #[tokio::test]
    async fn invalidate_forces_a_reload() {
        let (cache, _clock) = cache_with(Duration::from_secs(30), 4);
        let loads = Arc::new(AtomicU32::new(0));
        let key = public_key("games", 25, "", "");

        let load = |loads: Arc<AtomicU32>, id: &'static str| {
            move || {
                let loads = Arc::clone(&loads);
                async move {
                    loads.fetch_add(1, Ordering::SeqCst);
                    Ok::<_, DbErr>(vec![sample(id, 1)])
                }
            }
        };

        let first = cache
            .get_or_load(key.clone(), load(Arc::clone(&loads), "before"))
            .await
            .unwrap();
        assert_eq!(first.rows[0].member_id, "before");
        cache.invalidate();
        let second = cache
            .get_or_load(key, load(Arc::clone(&loads), "after"))
            .await
            .unwrap();
        assert_eq!(second.rows[0].member_id, "after");
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
}
