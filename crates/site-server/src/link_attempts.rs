//! Per-IP limit on wrong device-link user codes.
//!
//! Lookup, approve, and deny share this bucket. It is separate from the route
//! governors: those bound request rate, this one bounds guesses of the short
//! user code. A correct code does not consume a guess. The window is ten
//! minutes, the same lifetime as a code, so a burst cannot be retried against
//! the same code after the limit.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Wrong codes allowed from one client IP inside [`WINDOW`] before further
/// attempts are rejected.
pub const LINK_WRONG_CODE_LIMIT: usize = 5;

const WINDOW: Duration = Duration::from_secs(10 * 60);
const MAX_IPS: usize = 4_096;

#[derive(Default)]
struct Book {
    by_ip: HashMap<IpAddr, Vec<Instant>>,
}

/// Process-local wrong-code counts. Cloned with the app state (`Arc`).
#[derive(Clone, Default)]
pub struct LinkCodeAttempts {
    inner: Arc<Mutex<Book>>,
}

impl LinkCodeAttempts {
    pub fn new() -> Self {
        Self::default()
    }

    /// `Some(seconds)` when this IP is already at the limit.
    pub fn retry_after(&self, ip: IpAddr) -> Option<u64> {
        self.retry_after_at(ip, Instant::now())
    }

    /// Count one wrong, expired, or used code. The attempt that reaches the
    /// limit still happened; the next call to [`Self::retry_after`] reports it.
    pub fn record_failure(&self, ip: IpAddr) {
        self.record_failure_at(ip, Instant::now());
    }

    fn retry_after_at(&self, ip: IpAddr, now: Instant) -> Option<u64> {
        let book = self.lock();
        let times = book.by_ip.get(&ip)?;
        blocked(times, now)
    }

    fn record_failure_at(&self, ip: IpAddr, now: Instant) {
        let mut book = self.lock();
        if book.by_ip.len() >= MAX_IPS && !book.by_ip.contains_key(&ip) {
            book.by_ip.retain(|_, times| {
                times
                    .iter()
                    .any(|t| now.saturating_duration_since(*t) < WINDOW)
            });
            if book.by_ip.len() >= MAX_IPS {
                let victim = book
                    .by_ip
                    .iter()
                    .min_by_key(|(_, times)| times.last().copied())
                    .map(|(ip, _)| *ip);
                if let Some(victim) = victim {
                    book.by_ip.remove(&victim);
                }
            }
        }
        let times = book.by_ip.entry(ip).or_default();
        times.retain(|t| now.saturating_duration_since(*t) < WINDOW);
        if times.len() < LINK_WRONG_CODE_LIMIT {
            times.push(now);
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Book> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn blocked(times: &[Instant], now: Instant) -> Option<u64> {
    let live: Vec<Instant> = times
        .iter()
        .copied()
        .filter(|t| now.saturating_duration_since(*t) < WINDOW)
        .collect();
    if live.len() < LINK_WRONG_CODE_LIMIT {
        return None;
    }
    let oldest = live.iter().copied().min()?;
    let remain = WINDOW.saturating_sub(now.saturating_duration_since(oldest));
    Some(remain.as_secs().max(1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn ip(last: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(203, 0, 113, last))
    }

    #[test]
    fn five_wrong_codes_then_the_next_waits() {
        let attempts = LinkCodeAttempts::new();
        let client = ip(7);
        for _ in 0..LINK_WRONG_CODE_LIMIT {
            assert_eq!(attempts.retry_after(client), None);
            attempts.record_failure(client);
        }
        let wait = attempts.retry_after(client).expect("blocked");
        assert!(wait >= 1);
        assert_eq!(attempts.retry_after(ip(8)), None);
    }
}
