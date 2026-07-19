//! Per-member token-bucket rate limiter for the secret-touching Nostr routes
//! (DR1-NOSTR-006).
//!
//! The Nostr challenge/verify/export-backup/import-key/dm-send handlers all
//! touch member secret material and — for the NIP-49 key backup/import paths —
//! run an expensive Argon2 KDF on the server. None of them were bounded at the
//! application layer: the per-IP `GovernorLayer` (see [`crate::rate_limit`])
//! only guards the OAuth/login and upload routes, and the relay-policy limiter
//! only rate-limits *relay* events. So a single authenticated member could
//! hammer NIP-49 encrypt/decrypt (CPU burn) or flood gift-wrap publishes.
//!
//! This limiter keys on the authenticated **member id** rather than the source
//! IP, so a member cannot spread the load across rotating addresses, and members
//! who happen to share a NAT'd egress IP are not throttled collectively. It is a
//! single shared token bucket per member: cheap interactive ops
//! ([`COST_INTERACTIVE`]) draw one token; the expensive NIP-49 key ops
//! ([`COST_KEY_OP`]) draw ten. With [`CAPACITY`] = 30 and [`REFILL_PER_SEC`] = 1
//! that yields a burst of ~30 interactive calls (then ~60/min sustained) or ~3
//! key ops (then ~1 per 10s) per member.
//!
//! ## Bounds & multi-instance caveat
//!
//! State is per process, like [`crate::challenge_store`]. The map is bounded by
//! the number of distinct authenticated members seen in the process lifetime
//! (org membership is bounded), so there is no explicit eviction. In a
//! multi-replica deployment each replica keeps its own buckets, so the effective
//! limit is per-replica — acceptable for a defense-in-depth CPU/flood guard.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// Maximum tokens a member's bucket holds (burst capacity).
const CAPACITY: f64 = 30.0;
/// Tokens replenished per second.
const REFILL_PER_SEC: f64 = 1.0;

/// Token cost of a cheap, interactive secret-touching op (challenge, verify,
/// dm-send).
pub const COST_INTERACTIVE: f64 = 1.0;
/// Token cost of an expensive NIP-49 key op (export-backup, import-key). These
/// run Argon2 on the server, so they draw a much larger share of the budget.
pub const COST_KEY_OP: f64 = 10.0;

struct Bucket {
    tokens: f64,
    last: Instant,
}

/// Thread-safe, cheaply-cloneable (`Arc` inner) per-member token-bucket limiter.
#[derive(Clone, Default)]
pub struct NostrRateLimiter {
    inner: Arc<Mutex<HashMap<String, Bucket>>>,
}

impl NostrRateLimiter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Try to spend `cost` tokens from `member_id`'s bucket.
    ///
    /// Returns `true` if the bucket had enough tokens (the caller may proceed,
    /// tokens are deducted) or `false` if the member is currently throttled
    /// (nothing is deducted). A first-seen member starts with a full bucket.
    pub fn check(&self, member_id: &str, cost: f64) -> bool {
        let now = Instant::now();
        // Recover from a poisoned lock rather than panicking: a limiter panic
        // must never take down the secret-op routes.
        let mut map = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let bucket = map.entry(member_id.to_string()).or_insert(Bucket {
            tokens: CAPACITY,
            last: now,
        });
        let elapsed = now.duration_since(bucket.last).as_secs_f64();
        bucket.tokens = (bucket.tokens + elapsed * REFILL_PER_SEC).min(CAPACITY);
        bucket.last = now;
        if bucket.tokens >= cost {
            bucket.tokens -= cost;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interactive_burst_then_throttle() {
        let rl = NostrRateLimiter::new();
        // A full bucket (30) allows 30 interactive ops, then throttles.
        for _ in 0..30 {
            assert!(rl.check("m1", COST_INTERACTIVE));
        }
        assert!(!rl.check("m1", COST_INTERACTIVE));
    }

    #[test]
    fn key_ops_draw_a_large_share() {
        let rl = NostrRateLimiter::new();
        // 30 capacity / 10 cost = 3 immediate key ops, then throttle.
        assert!(rl.check("m1", COST_KEY_OP));
        assert!(rl.check("m1", COST_KEY_OP));
        assert!(rl.check("m1", COST_KEY_OP));
        assert!(!rl.check("m1", COST_KEY_OP));
    }

    #[test]
    fn buckets_are_per_member() {
        let rl = NostrRateLimiter::new();
        // Exhaust one member's key-op budget.
        for _ in 0..3 {
            assert!(rl.check("m1", COST_KEY_OP));
        }
        assert!(!rl.check("m1", COST_KEY_OP));
        // A different member is unaffected — the bucket is keyed on member id.
        assert!(rl.check("m2", COST_KEY_OP));
    }
}
