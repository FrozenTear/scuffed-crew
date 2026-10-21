//! Short-lived in-process caches for anonymous endpoints that are expensive
//! per hit (API-9 / SEC-5): `/sitemap.xml` (about nine queries plus a thread
//! list per forum board), `/api/nostr/feed` (a new relay WebSocket and a
//! member scan), and the relay probe in `/api/nostr/health` (an outbound
//! HTTP request).
//!
//! The per-IP public governor bounds one source. These caches bound the
//! work many sources can cause, so a burst from many addresses costs one
//! build per TTL instead of one per request.
//!
//! `AppState` is built in many places (both server binaries, every test
//! file), so the caches are process-wide statics rather than state fields.
//! Each entry is scoped to the `Arc<Database>` that produced it and is only
//! served to that same live database. A dropped database's entries are
//! never served, even if a new one is allocated at the same address.

use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use scuffed_db::Database;

/// `/sitemap.xml` body. Crawlers re-read it on the order of hours; the
/// response already says `max-age=3600`.
pub(crate) const SITEMAP_TTL: Duration = Duration::from_secs(600);
/// Relay feed result for one `(hashtag, limit, since)` query, and the
/// pubkey → display name map.
pub(crate) const NOSTR_FEED_TTL: Duration = Duration::from_secs(15);
/// `reachable` from the relay health probe.
pub(crate) const RELAY_PROBE_TTL: Duration = Duration::from_secs(30);

/// TTL cache whose entries belong to one `Database`.
///
/// `now` is a parameter so tests can step time without sleeping. `cap`
/// bounds memory when the key comes from the query string: inserting past
/// it drops the oldest entry.
pub(crate) struct ScopedTtlCache<K, V> {
    ttl: Duration,
    cap: usize,
    entries: Mutex<Vec<Entry<K, V>>>,
}

struct Entry<K, V> {
    owner: Weak<Database>,
    key: K,
    stored: Instant,
    value: V,
}

fn same_owner(weak: &Weak<Database>, owner: &Arc<Database>) -> bool {
    weak.strong_count() > 0 && std::ptr::eq(weak.as_ptr(), Arc::as_ptr(owner))
}

impl<K: PartialEq, V: Clone> ScopedTtlCache<K, V> {
    pub(crate) const fn new(ttl: Duration, cap: usize) -> Self {
        Self {
            ttl,
            cap,
            entries: Mutex::new(Vec::new()),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Entry<K, V>>> {
        self.entries.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn prune(&self, entries: &mut Vec<Entry<K, V>>, now: Instant) {
        entries.retain(|e| {
            e.owner.strong_count() > 0 && now.saturating_duration_since(e.stored) < self.ttl
        });
    }

    /// A fresh value for `key` stored by `owner`, if any.
    pub(crate) fn get(&self, owner: &Arc<Database>, key: &K, now: Instant) -> Option<V> {
        let mut entries = self.lock();
        self.prune(&mut entries, now);
        entries
            .iter()
            .find(|e| same_owner(&e.owner, owner) && e.key == *key)
            .map(|e| e.value.clone())
    }

    pub(crate) fn insert(&self, owner: &Arc<Database>, key: K, value: V, now: Instant) {
        let mut entries = self.lock();
        self.prune(&mut entries, now);
        entries.retain(|e| !(same_owner(&e.owner, owner) && e.key == key));
        while entries.len() >= self.cap.max(1) {
            let oldest = entries
                .iter()
                .enumerate()
                .min_by_key(|(_, e)| e.stored)
                .map(|(i, _)| i)
                .unwrap_or(0);
            entries.remove(oldest);
        }
        entries.push(Entry {
            owner: Arc::downgrade(owner),
            key,
            stored: now,
            value,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn db() -> Arc<Database> {
        Arc::new(Database::connect_memory().await.unwrap())
    }

    #[tokio::test]
    async fn serves_within_ttl_and_expires_after() {
        let cache: ScopedTtlCache<&str, u32> = ScopedTtlCache::new(Duration::from_secs(10), 4);
        let owner = db().await;
        let t0 = Instant::now();
        cache.insert(&owner, "k", 1, t0);
        assert_eq!(cache.get(&owner, &"k", t0), Some(1));
        assert_eq!(
            cache.get(&owner, &"k", t0 + Duration::from_secs(9)),
            Some(1)
        );
        assert_eq!(cache.get(&owner, &"k", t0 + Duration::from_secs(10)), None);
        assert_eq!(cache.get(&owner, &"other", t0), None);
    }

    #[tokio::test]
    async fn entries_belong_to_their_database() {
        let cache: ScopedTtlCache<(), u32> = ScopedTtlCache::new(Duration::from_secs(60), 4);
        let a = db().await;
        let b = db().await;
        let now = Instant::now();
        cache.insert(&a, (), 1, now);
        assert_eq!(cache.get(&a, &(), now), Some(1));
        assert_eq!(cache.get(&b, &(), now), None, "another database misses");
        drop(a);
        let c = db().await;
        assert_eq!(
            cache.get(&c, &(), now),
            None,
            "a dropped database's entry is never served"
        );
        assert!(cache.lock().is_empty(), "dead owners are pruned");
    }

    #[tokio::test]
    async fn cap_drops_the_oldest_entry() {
        let cache: ScopedTtlCache<u32, u32> = ScopedTtlCache::new(Duration::from_secs(60), 2);
        let owner = db().await;
        let t0 = Instant::now();
        cache.insert(&owner, 1, 1, t0);
        cache.insert(&owner, 2, 2, t0 + Duration::from_secs(1));
        cache.insert(&owner, 3, 3, t0 + Duration::from_secs(2));
        let now = t0 + Duration::from_secs(2);
        assert_eq!(cache.get(&owner, &1, now), None);
        assert_eq!(cache.get(&owner, &2, now), Some(2));
        assert_eq!(cache.get(&owner, &3, now), Some(3));
        // Re-inserting a key replaces it instead of adding a second entry.
        cache.insert(&owner, 3, 30, now);
        assert_eq!(cache.get(&owner, &3, now), Some(30));
        assert_eq!(cache.lock().len(), 2);
    }
}
