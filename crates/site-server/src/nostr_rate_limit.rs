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
//! who happen to share a NAT'd egress IP are not throttled collectively.
//!
//! ## Two independent buckets (NP-1a)
//!
//! Each member gets **two independent token buckets**, one per traffic class
//! (see [`RateClass`]):
//!
//! * [`RateClass::Interactive`] — cheap, high-frequency ops (challenge, verify,
//!   dm-send). [`INTERACTIVE_CAPACITY`] = 30, [`INTERACTIVE_REFILL_PER_SEC`] = 1,
//!   so a burst of ~30 calls then ~60/min sustained per member.
//! * [`RateClass::KeyOp`] — rare, sensitive NIP-49 key ops (export-backup,
//!   import-key). [`KEY_OP_CAPACITY`] = 5, [`KEY_OP_REFILL_PER_SEC`] = 1/60,
//!   so a burst of 5 then ~1/min sustained per member.
//!
//! Each op charges **only its own bucket**, so draining one class never delays
//! the other. This matters most for the key ops: identity-critical recovery and
//! migration (export/import/unlink) must not be throttled just because the
//! member happened to send a lot of chat DMs. Previously both classes shared a
//! single bucket, so 30 DMs could block a key export — the worst moment to be
//! throttled.
//!
//! ### Why these key-op numbers
//!
//! Key ops are genuinely rare in normal use: a member exports a backup or
//! imports/migrates a key a handful of times ever, not per session. A burst
//! [`KEY_OP_CAPACITY`] of 5 comfortably covers a legitimate retry/typo loop
//! (e.g. mistyped passphrase on import) while the 1/min sustained refill still
//! caps server-side Argon2 work — and thus offline/brute-force probing of the
//! NIP-49 passphrase — at ~60 attempts/hour per member. That is a real bound,
//! not an effectively-unlimited budget; it is simply decoupled from chat volume.
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

/// Burst capacity of the interactive bucket (challenge, verify, dm-send).
const INTERACTIVE_CAPACITY: f64 = 30.0;
/// Interactive tokens replenished per second (~60/min sustained).
const INTERACTIVE_REFILL_PER_SEC: f64 = 1.0;

/// Burst capacity of the key-op bucket (export-backup, import-key). Small by
/// design — key ops are rare and each runs a server-side Argon2 KDF.
const KEY_OP_CAPACITY: f64 = 5.0;
/// Key-op tokens replenished per second — one per minute. Bounds brute-force
/// probing of the NIP-49 passphrase without depending on chat volume.
const KEY_OP_REFILL_PER_SEC: f64 = 1.0 / 60.0;

/// Traffic class selecting which of a member's two independent buckets an op
/// charges. Each class is rate-limited on its own, so exhausting one never
/// throttles the other.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RateClass {
    /// Cheap, high-frequency secret-touching ops: challenge, verify, dm-send.
    Interactive,
    /// Expensive NIP-49 key ops: export-backup, import-key (and any future
    /// key-mutation such as unlink). These run Argon2 on the server.
    KeyOp,
}

impl RateClass {
    /// Burst capacity of this class's bucket.
    fn capacity(self) -> f64 {
        match self {
            RateClass::Interactive => INTERACTIVE_CAPACITY,
            RateClass::KeyOp => KEY_OP_CAPACITY,
        }
    }

    /// Tokens replenished per second for this class's bucket.
    fn refill_per_sec(self) -> f64 {
        match self {
            RateClass::Interactive => INTERACTIVE_REFILL_PER_SEC,
            RateClass::KeyOp => KEY_OP_REFILL_PER_SEC,
        }
    }
}

struct Bucket {
    tokens: f64,
    last: Instant,
}

impl Bucket {
    fn full(class: RateClass, now: Instant) -> Self {
        Bucket {
            tokens: class.capacity(),
            last: now,
        }
    }

    /// Refill lazily by elapsed time, then try to spend one token. Returns
    /// `true` (and deducts) if a token was available, `false` otherwise.
    fn try_spend(&mut self, class: RateClass, now: Instant) -> bool {
        let elapsed = now.duration_since(self.last).as_secs_f64();
        self.tokens = (self.tokens + elapsed * class.refill_per_sec()).min(class.capacity());
        self.last = now;
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

/// The two independent per-member buckets.
#[derive(Default)]
struct MemberBuckets {
    interactive: Option<Bucket>,
    key_op: Option<Bucket>,
}

impl MemberBuckets {
    fn bucket_mut(&mut self, class: RateClass) -> &mut Option<Bucket> {
        match class {
            RateClass::Interactive => &mut self.interactive,
            RateClass::KeyOp => &mut self.key_op,
        }
    }
}

/// Thread-safe, cheaply-cloneable (`Arc` inner) per-member token-bucket limiter
/// with two independent buckets per member (see [`RateClass`]).
#[derive(Clone, Default)]
pub struct NostrRateLimiter {
    inner: Arc<Mutex<HashMap<String, MemberBuckets>>>,
}

impl NostrRateLimiter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Try to spend one token from `member_id`'s bucket for the given
    /// [`RateClass`].
    ///
    /// Returns `true` if the bucket had a token (the caller may proceed, a
    /// token is deducted) or `false` if that class is currently throttled
    /// (nothing is deducted). A first-seen member/class starts with a full
    /// bucket. The two classes are fully independent: exhausting one never
    /// affects the other.
    pub fn check(&self, member_id: &str, class: RateClass) -> bool {
        let now = Instant::now();
        // Recover from a poisoned lock rather than panicking: a limiter panic
        // must never take down the secret-op routes.
        let mut map = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let buckets = map.entry(member_id.to_string()).or_default();
        let bucket = buckets
            .bucket_mut(class)
            .get_or_insert_with(|| Bucket::full(class, now));
        bucket.try_spend(class, now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interactive_burst_then_throttle() {
        let rl = NostrRateLimiter::new();
        // A full interactive bucket (30) allows 30 interactive ops, then throttles.
        for _ in 0..30 {
            assert!(rl.check("m1", RateClass::Interactive));
        }
        assert!(!rl.check("m1", RateClass::Interactive));
    }

    #[test]
    fn key_op_burst_then_throttle() {
        let rl = NostrRateLimiter::new();
        // A full key-op bucket (5) allows 5 key ops, then throttles.
        for _ in 0..5 {
            assert!(rl.check("m1", RateClass::KeyOp));
        }
        assert!(!rl.check("m1", RateClass::KeyOp));
    }

    #[test]
    fn buckets_are_per_member() {
        let rl = NostrRateLimiter::new();
        // Exhaust one member's key-op budget.
        for _ in 0..5 {
            assert!(rl.check("m1", RateClass::KeyOp));
        }
        assert!(!rl.check("m1", RateClass::KeyOp));
        // A different member is unaffected — the bucket is keyed on member id.
        assert!(rl.check("m2", RateClass::KeyOp));
    }

    #[test]
    fn exhausting_interactive_does_not_block_key_ops() {
        let rl = NostrRateLimiter::new();
        // Drain the interactive bucket completely.
        for _ in 0..30 {
            assert!(rl.check("m1", RateClass::Interactive));
        }
        assert!(!rl.check("m1", RateClass::Interactive));
        // The key-op bucket is a separate budget — identity-critical key ops
        // must still go through even when chat traffic has saturated interactive.
        for _ in 0..5 {
            assert!(rl.check("m1", RateClass::KeyOp));
        }
    }

    #[test]
    fn exhausting_key_ops_does_not_block_interactive() {
        let rl = NostrRateLimiter::new();
        // Drain the key-op bucket completely.
        for _ in 0..5 {
            assert!(rl.check("m1", RateClass::KeyOp));
        }
        assert!(!rl.check("m1", RateClass::KeyOp));
        // Interactive traffic is a separate budget and stays fully available.
        for _ in 0..30 {
            assert!(rl.check("m1", RateClass::Interactive));
        }
    }
}
