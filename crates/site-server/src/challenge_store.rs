//! In-memory one-time store for consumed Nostr login/link challenges.
//!
//! This blocks replay of a captured, victim-signed kind-22242 event *within*
//! its freshness/TTL window (defense-in-depth for DR1-NOSTR-001). The signed
//! event's `created_at` freshness check bounds *how long* an event is
//! replayable; this store makes each issued challenge single-use so the same
//! event cannot be submitted twice even inside that window.
//!
//! ## Multi-instance caveat
//!
//! This store is **per process**. In a multi-replica deployment behind a load
//! balancer, replicas do NOT share consumed-challenge state, so a replay routed
//! to a *different* replica could still succeed within the freshness window.
//! The `created_at` freshness window + short challenge TTL bound that residual
//! exposure; fully closing it across replicas would require a shared store
//! (SurrealDB row with a unique index, or Redis) — deliberately out of scope
//! for this single-process server. See `// HS/NOSTR follow-up` below.
//
// HS/NOSTR follow-up: if the deployment ever scales to >1 server replica,
// promote this to a shared/atomic store (e.g. a `consumed_challenge` table with
// a UNIQUE index on the challenge string + `INSERT ... ` that fails on
// duplicate, TTL-swept by the existing session-cleanup task).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Thread-safe, cheaply-cloneable (`Arc` inner) set of consumed challenge
/// strings with per-entry expiry.
#[derive(Clone, Default)]
pub struct ConsumedChallengeStore {
    inner: Arc<Mutex<HashMap<String, Instant>>>,
}

impl ConsumedChallengeStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Attempt to consume `challenge`.
    ///
    /// Returns `true` if it was newly consumed (the caller may proceed), or
    /// `false` if it was already consumed and is still live (replay → reject).
    /// Expired entries are evicted on each call, so the map self-bounds to the
    /// set of challenges seen within the last `ttl`.
    pub fn consume(&self, challenge: &str, ttl: Duration) -> bool {
        let now = Instant::now();
        // Recover from a poisoned lock rather than panicking: a challenge store
        // panic must never become a login-path DoS.
        let mut map = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        map.retain(|_, expiry| *expiry > now);
        if map.contains_key(challenge) {
            return false;
        }
        map.insert(challenge.to_string(), now + ttl);
        true
    }

    /// Number of live (unexpired-at-insert) entries — test/introspection only.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_consume_succeeds_replay_rejected() {
        let store = ConsumedChallengeStore::new();
        let ttl = Duration::from_secs(300);
        assert!(store.consume("scuffedclan-login:abc", ttl));
        // Same challenge again within TTL → replay rejected.
        assert!(!store.consume("scuffedclan-login:abc", ttl));
        // A different challenge is unaffected.
        assert!(store.consume("scuffedclan-login:def", ttl));
    }

    #[test]
    fn expired_entries_are_evicted_and_reusable() {
        let store = ConsumedChallengeStore::new();
        // Zero TTL: the entry is already expired by the next call, so eviction
        // clears it and the "same" challenge can be consumed again.
        assert!(store.consume("c", Duration::from_secs(0)));
        assert!(store.consume("c", Duration::from_secs(0)));
        assert_eq!(store.len(), 1);
    }
}
