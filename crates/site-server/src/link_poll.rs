//! Per-IP limit for `POST /api/link/poll`.
//!
//! Three devices on one address, each polling every 5 seconds for the full
//! 10 minute code lifetime, is 0.6 requests per second. The bucket refills
//! one cell per second and holds a burst of 30, so that steady rate never
//! empties it. One cell per 2 seconds is only 0.5 requests per second and
//! would start returning 429 before the codes expire.
//!
//! The clock is injectable. Production reads wall time. Tests advance a
//! manual clock instead of sleeping for 10 minutes.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Duration, Utc};

/// Cells available at once on one client IP.
pub const LINK_POLL_BURST: u32 = 30;
/// Seconds between refill of a single cell. See the module docs for why this is 1.
pub const LINK_POLL_REFILL_SECS: u64 = 1;

const MAX_IPS: usize = 4_096;

#[derive(Clone)]
enum ClockSource {
    System,
    Manual(Arc<Mutex<DateTime<Utc>>>),
}

/// Wall clock, or a test clock that only moves when [`LinkClock::advance`] is called.
#[derive(Clone)]
pub struct LinkClock {
    source: ClockSource,
}

impl LinkClock {
    pub fn system() -> Self {
        Self {
            source: ClockSource::System,
        }
    }

    pub fn manual(start: DateTime<Utc>) -> Self {
        Self {
            source: ClockSource::Manual(Arc::new(Mutex::new(start))),
        }
    }

    pub fn now(&self) -> DateTime<Utc> {
        match &self.source {
            ClockSource::System => Utc::now(),
            ClockSource::Manual(cell) => *cell.lock().expect("link clock"),
        }
    }

    /// Move a manual clock forward. Production uses [`Self::system`], which has no step.
    pub fn advance(&self, by: Duration) {
        match &self.source {
            ClockSource::Manual(cell) => {
                let mut now = cell.lock().expect("link clock");
                *now += by;
            }
            ClockSource::System => panic!("the system link clock cannot be advanced"),
        }
    }
}

struct Bucket {
    /// Cells left after the last refill.
    tokens: u32,
    /// Time the `tokens` count was computed. Partial seconds stay here so
    /// refill does not round them away.
    as_of: DateTime<Utc>,
}

struct Book {
    by_ip: HashMap<IpAddr, Bucket>,
}

/// Process-local poll buckets. Cloned with the app state (`Arc` inside).
#[derive(Clone)]
pub struct LinkPollGate {
    clock: LinkClock,
    book: Arc<Mutex<Book>>,
}

impl LinkPollGate {
    pub fn system() -> Self {
        Self::with_clock(LinkClock::system())
    }

    pub fn with_clock(clock: LinkClock) -> Self {
        Self {
            clock,
            book: Arc::new(Mutex::new(Book {
                by_ip: HashMap::new(),
            })),
        }
    }

    pub fn now(&self) -> DateTime<Utc> {
        self.clock.now()
    }

    /// `Some(seconds)` when this IP is over the poll bucket.
    /// A miss records the request.
    pub fn take(&self, ip: IpAddr) -> Option<u64> {
        self.take_at(ip, self.clock.now())
    }

    fn take_at(&self, ip: IpAddr, now: DateTime<Utc>) -> Option<u64> {
        let mut book = self.book.lock().expect("link poll buckets");
        if book.by_ip.len() >= MAX_IPS && !book.by_ip.contains_key(&ip) {
            book.by_ip
                .retain(|_, bucket| bucket.tokens < LINK_POLL_BURST);
            if book.by_ip.len() >= MAX_IPS {
                let victim = book.by_ip.keys().next().copied();
                if let Some(victim) = victim {
                    book.by_ip.remove(&victim);
                }
            }
        }
        let bucket = book.by_ip.entry(ip).or_insert(Bucket {
            tokens: LINK_POLL_BURST,
            as_of: now,
        });
        let elapsed_ms = now
            .signed_duration_since(bucket.as_of)
            .num_milliseconds()
            .max(0) as u64;
        let refill_ms = LINK_POLL_REFILL_SECS.saturating_mul(1_000);
        let gained = (elapsed_ms / refill_ms) as u32;
        let mut leftover_ms = elapsed_ms % refill_ms;
        let mut tokens = bucket.tokens.saturating_add(gained);
        if tokens >= LINK_POLL_BURST {
            tokens = LINK_POLL_BURST;
            leftover_ms = 0;
        }
        if tokens == 0 {
            let wait_ms = refill_ms - leftover_ms;
            return Some(wait_ms.div_ceil(1_000).max(1));
        }
        bucket.tokens = tokens - 1;
        bucket.as_of = now - Duration::milliseconds(leftover_ms as i64);
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn ip() -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(203, 0, 113, 10))
    }

    #[test]
    fn three_devices_can_poll_every_five_seconds_for_ten_minutes() {
        let clock = LinkClock::manual(Utc::now());
        let gate = LinkPollGate::with_clock(clock.clone());
        let polls = 600 / 5;
        for tick in 0..polls {
            if tick > 0 {
                clock.advance(Duration::seconds(5));
            }
            for device in 0..3 {
                assert!(
                    gate.take(ip()).is_none(),
                    "tick {tick} device {device} was limited"
                );
            }
        }
    }

    #[test]
    fn burst_then_the_next_poll_waits() {
        let clock = LinkClock::manual(Utc::now());
        let gate = LinkPollGate::with_clock(clock.clone());
        for _ in 0..LINK_POLL_BURST {
            assert!(gate.take(ip()).is_none());
        }
        let wait = gate.take(ip()).expect("over the burst");
        assert!(wait >= 1);
        clock.advance(Duration::seconds(LINK_POLL_REFILL_SECS as i64));
        assert!(gate.take(ip()).is_none(), "one refill opens one cell");
    }
}
