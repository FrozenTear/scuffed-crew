//! Per-account failed-login backoff for password login only.
//!
//! The per-IP governor on `/api/auth/local/login` (see [`crate::rate_limit`])
//! does not stop a distributed guesser, and it does not slow repeated guesses
//! of one password. This store counts failed password checks per normalized
//! username and, after a few failures inside a window, returns a temporary
//! lock. The lock duration escalates and is capped; it is never permanent.
//! A successful password check clears the record.
//!
//! Unknown usernames are counted the same way as real ones, so a lockout
//! response does not reveal whether the account exists.
//!
//! Not used for bearer tokens (stat-tracker daemon sync), OAuth, or Nostr
//! login. Those paths must not call this store.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Client-facing lockout text. Identical for existing and missing usernames.
pub const LOCKOUT_MESSAGE: &str = "too many login attempts";

/// Failures inside this window count toward a lock. Older ones age out.
/// Longer than [`MAX_LOCK`] so a lock can expire and the next failure still
/// sees the recent guesses, but short enough that stopping the attack (or a
/// successful login) returns the account to normal without a permanent ban.
const FAILURE_WINDOW: Duration = Duration::from_secs(30 * 60);

/// First lock fires after this many failures in the window.
pub const FIRST_LOCK_AFTER: usize = 5;
/// Seconds the first tier locks the account.
pub const FIRST_LOCK_SECS: u64 = 30;

/// Hard cap on how long a single lock can last. Further failures never extend
/// past this, so an attacker cannot pin a real user out forever.
const MAX_LOCK: Duration = Duration::from_secs(15 * 60);

/// Bound the map so a spray of random usernames cannot grow it without limit.
/// Past the cap, new keys are not tracked (existing accounts keep their state).
const MAX_TRACKED_ACCOUNTS: usize = 8_192;

struct Tier {
    failures: usize,
    lock: Duration,
}

/// Checked from the highest matching tier. The first tier matches
/// [`FIRST_LOCK_AFTER`] / [`FIRST_LOCK_SECS`]; the last tier is [`MAX_LOCK`].
const TIERS: &[Tier] = &[
    Tier {
        failures: FIRST_LOCK_AFTER,
        lock: Duration::from_secs(FIRST_LOCK_SECS),
    },
    Tier {
        failures: 8,
        lock: Duration::from_secs(2 * 60),
    },
    Tier {
        failures: 12,
        lock: MAX_LOCK,
    },
];

#[derive(Default)]
struct Record {
    failures: Vec<Instant>,
    locked_until: Option<Instant>,
}

/// Process-local failed-login state. Shared via [`Clone`] (`Arc`).
#[derive(Clone, Default)]
pub struct LoginLockout {
    inner: Arc<Mutex<HashMap<String, Record>>>,
}

impl LoginLockout {
    pub fn new() -> Self {
        Self::default()
    }

    /// `Some(seconds)` when `username` is currently locked.
    pub fn retry_after(&self, username: &str) -> Option<u64> {
        self.retry_after_at(username, Instant::now())
    }

    /// Record one failed password check.
    ///
    /// Returns `Some(seconds)` when the account is locked after this failure
    /// (including when it was already locked — further failures during a lock
    /// do not extend it or pile on more counts).
    pub fn record_failure(&self, username: &str) -> Option<u64> {
        self.record_failure_at(username, Instant::now())
    }

    /// Drop the record. A correct password ends the backoff immediately.
    pub fn record_success(&self, username: &str) {
        self.lock().remove(username);
    }

    fn retry_after_at(&self, username: &str, now: Instant) -> Option<u64> {
        self.lock()
            .get(username)
            .and_then(|rec| remaining(rec, now))
    }

    fn record_failure_at(&self, username: &str, now: Instant) -> Option<u64> {
        let mut map = self.lock();
        if let Some(secs) = map.get(username).and_then(|rec| remaining(rec, now)) {
            // Already locked. Do not record this attempt: hammering during the
            // lock must not push the window forward or stretch the deadline.
            return Some(secs);
        }
        if !map.contains_key(username) && map.len() >= MAX_TRACKED_ACCOUNTS {
            map.retain(|_, rec| !is_idle(rec, now));
            if map.len() >= MAX_TRACKED_ACCOUNTS {
                return None;
            }
        }
        let rec = map.entry(username.to_string()).or_default();
        rec.failures
            .retain(|t| now.saturating_duration_since(*t) < FAILURE_WINDOW);
        rec.failures.push(now);
        let n = rec.failures.len();
        let lock_for = TIERS
            .iter()
            .rev()
            .find(|tier| n >= tier.failures)
            .map(|tier| tier.lock);
        if let Some(lock_for) = lock_for {
            rec.locked_until = Some(now + lock_for);
            Some(retry_secs(lock_for))
        } else {
            rec.locked_until = None;
            None
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Record>> {
        // A poisoned lock is process-local bookkeeping, not a reason to 500
        // the login request. Keep serving from the recovered map.
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }
}

fn remaining(rec: &Record, now: Instant) -> Option<u64> {
    let until = rec.locked_until?;
    if until <= now {
        return None;
    }
    Some(retry_secs(until.saturating_duration_since(now)))
}

fn is_idle(rec: &Record, now: Instant) -> bool {
    remaining(rec, now).is_none()
        && rec
            .failures
            .iter()
            .all(|t| now.saturating_duration_since(*t) >= FAILURE_WINDOW)
}

fn retry_secs(dur: Duration) -> u64 {
    let secs = dur.as_secs();
    if dur.subsec_nanos() > 0 {
        secs.saturating_add(1).max(1)
    } else {
        secs.max(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(base: Instant, secs: u64) -> Instant {
        base + Duration::from_secs(secs)
    }

    #[test]
    fn locks_on_fifth_failure_and_resets_on_success() {
        let store = LoginLockout::new();
        let t0 = Instant::now();
        for _ in 0..(FIRST_LOCK_AFTER - 1) {
            assert_eq!(store.record_failure_at("boss", t0), None);
        }
        assert_eq!(store.record_failure_at("boss", t0), Some(FIRST_LOCK_SECS));
        assert_eq!(store.retry_after_at("boss", t0), Some(FIRST_LOCK_SECS));

        store.record_success("boss");
        assert_eq!(store.retry_after_at("boss", t0), None);
        assert_eq!(store.record_failure_at("boss", t0), None);
    }

    #[test]
    fn hammering_during_lock_does_not_extend_it() {
        let store = LoginLockout::new();
        let t0 = Instant::now();
        for _ in 0..FIRST_LOCK_AFTER {
            store.record_failure_at("boss", t0);
        }
        let during = at(t0, 10);
        let left = store.record_failure_at("boss", during).unwrap();
        assert!(left <= FIRST_LOCK_SECS - 10);
        assert!(left >= FIRST_LOCK_SECS - 11);
        // After the original deadline the lock is over, even though the
        // attacker kept posting during it.
        assert_eq!(store.retry_after_at("boss", at(t0, FIRST_LOCK_SECS)), None);
    }

    #[test]
    fn escalates_then_caps() {
        let store = LoginLockout::new();
        let t0 = Instant::now();
        let mut now = t0;
        let mut lock = 0u64;
        for n in 1..=13 {
            if lock > 0 {
                // Attempts during a lock are ignored, so the next counted
                // failure has to land after the deadline.
                now += Duration::from_secs(lock + 1);
            }
            lock = store.record_failure_at("boss", now).unwrap_or(0);
            let expect = if n < 5 {
                0
            } else if n < 8 {
                FIRST_LOCK_SECS
            } else if n < 12 {
                2 * 60
            } else {
                15 * 60
            };
            assert_eq!(lock, expect, "failure {n}");
        }
    }

    #[test]
    fn window_expiry_drops_the_count() {
        let store = LoginLockout::new();
        let t0 = Instant::now();
        for _ in 0..FIRST_LOCK_AFTER {
            store.record_failure_at("boss", t0);
        }
        let later = t0 + FAILURE_WINDOW + Duration::from_secs(1);
        assert_eq!(store.retry_after_at("boss", later), None);
        assert_eq!(store.record_failure_at("boss", later), None);
    }

    #[test]
    fn unknown_and_known_keys_are_independent() {
        let store = LoginLockout::new();
        let t0 = Instant::now();
        for _ in 0..FIRST_LOCK_AFTER {
            store.record_failure_at("ghost", t0);
        }
        assert!(store.retry_after_at("ghost", t0).is_some());
        assert_eq!(store.retry_after_at("boss", t0), None);
    }
}
