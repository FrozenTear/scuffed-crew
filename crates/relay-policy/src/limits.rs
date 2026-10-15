//! Per-client connection and message limits for the read gate.
//!
//! strfry sees every public connection as 127.0.0.1, because the gate dials it
//! from inside the container. strfry's per-IP limits therefore do not see
//! browsers. These limits run on the gate, before that dial.
//!
//! `X-Forwarded-For` is used only when the TCP peer is a trusted proxy
//! (loopback, plus `RELAY_GATE_TRUSTED_PROXIES`). The client is the rightmost
//! address that is not itself a trusted proxy. A direct connection cannot
//! choose its own address by sending the header.

use std::collections::{HashMap, VecDeque};
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ipnet::IpNet;

const DEFAULT_MAX_CONNECTIONS: usize = 32;
const DEFAULT_NEW_CONNECTIONS: usize = 60;
const DEFAULT_MESSAGES: usize = 240;
const DEFAULT_WINDOW_SECS: u64 = 60;

#[derive(Debug)]
pub enum AdmitError {
    Limited,
}

struct Bucket {
    connections: usize,
    conn_times: VecDeque<Instant>,
    msg_times: VecDeque<Instant>,
}

impl Bucket {
    fn prune(&mut self, now: Instant, window: Duration) {
        while self
            .conn_times
            .front()
            .is_some_and(|t| now.saturating_duration_since(*t) > window)
        {
            self.conn_times.pop_front();
        }
        while self
            .msg_times
            .front()
            .is_some_and(|t| now.saturating_duration_since(*t) > window)
        {
            self.msg_times.pop_front();
        }
    }

    fn idle(&self) -> bool {
        self.connections == 0 && self.conn_times.is_empty() && self.msg_times.is_empty()
    }
}

struct Inner {
    max_connections: usize,
    max_new_per_window: usize,
    max_messages_per_window: usize,
    window: Duration,
    trusted: Vec<IpNet>,
    buckets: Mutex<HashMap<IpAddr, Bucket>>,
}

/// Shared limiter. Clone is cheap; every connection uses the same buckets.
#[derive(Clone)]
pub struct GateLimits {
    inner: Arc<Inner>,
}

/// Held for the life of an admitted connection. Drop releases the slot.
pub struct Admission {
    ip: IpAddr,
    inner: Arc<Inner>,
}

impl Drop for Admission {
    fn drop(&mut self) {
        let mut buckets = self
            .inner
            .buckets
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let Some(bucket) = buckets.get_mut(&self.ip) else {
            return;
        };
        bucket.connections = bucket.connections.saturating_sub(1);
        bucket.prune(Instant::now(), self.inner.window);
        if bucket.idle() {
            buckets.remove(&self.ip);
        }
    }
}

impl GateLimits {
    pub fn from_env() -> Self {
        let limits = Self::build(
            env_usize("RELAY_GATE_MAX_CONN_PER_IP", DEFAULT_MAX_CONNECTIONS),
            env_usize("RELAY_GATE_CONN_RATE", DEFAULT_NEW_CONNECTIONS),
            env_usize("RELAY_GATE_MSG_RATE", DEFAULT_MESSAGES),
            Duration::from_secs(env_u64("RELAY_GATE_RATE_WINDOW_SECS", DEFAULT_WINDOW_SECS)),
            trusted_from_env(),
        );
        tracing::info!(
            max_connections = limits.inner.max_connections,
            new_connections = limits.inner.max_new_per_window,
            messages = limits.inner.max_messages_per_window,
            window_secs = limits.inner.window.as_secs(),
            trusted_nets = limits.inner.trusted.len(),
            "read gate per-ip limits"
        );
        limits
    }

    /// High caps for tests that are not about limiting.
    pub fn generous() -> Self {
        Self::build(
            10_000,
            10_000,
            10_000,
            Duration::from_secs(60),
            loopback_nets(),
        )
    }

    pub fn build(
        max_connections: usize,
        max_new_per_window: usize,
        max_messages_per_window: usize,
        window: Duration,
        trusted: Vec<IpNet>,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                max_connections,
                max_new_per_window,
                max_messages_per_window,
                window,
                trusted,
                buckets: Mutex::new(HashMap::new()),
            }),
        }
    }

    pub fn trusted(&self) -> &[IpNet] {
        &self.inner.trusted
    }

    pub fn try_admit(&self, ip: IpAddr) -> Result<Admission, AdmitError> {
        let mut buckets = self
            .inner
            .buckets
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let window = self.inner.window;
        let max_connections = self.inner.max_connections;
        let max_new = self.inner.max_new_per_window;
        let bucket = buckets.entry(ip).or_insert_with(|| Bucket {
            connections: 0,
            conn_times: VecDeque::new(),
            msg_times: VecDeque::new(),
        });
        let now = Instant::now();
        bucket.prune(now, window);
        bucket.conn_times.push_back(now);
        if bucket.conn_times.len() > max_new || bucket.connections >= max_connections {
            return Err(AdmitError::Limited);
        }
        bucket.connections += 1;
        Ok(Admission {
            ip,
            inner: Arc::clone(&self.inner),
        })
    }

    /// Count one client frame. `false` means the frame must not be forwarded.
    pub fn allow_message(&self, ip: IpAddr) -> bool {
        let mut buckets = self
            .inner
            .buckets
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let window = self.inner.window;
        let max_messages = self.inner.max_messages_per_window;
        let bucket = buckets.entry(ip).or_insert_with(|| Bucket {
            connections: 0,
            conn_times: VecDeque::new(),
            msg_times: VecDeque::new(),
        });
        let now = Instant::now();
        bucket.prune(now, window);
        if bucket.msg_times.len() >= max_messages {
            return false;
        }
        bucket.msg_times.push_back(now);
        true
    }
}

/// Peer address, unless that peer is a trusted proxy. Then the client is the
/// rightmost `X-Forwarded-For` entry that is not itself trusted.
pub fn resolve_client_ip(peer: IpAddr, forwarded: &[IpAddr], trusted: &[IpNet]) -> IpAddr {
    if !trusted.iter().any(|net| net.contains(&peer)) {
        return peer;
    }
    if let Some(ip) = forwarded
        .iter()
        .rev()
        .find(|ip| !trusted.iter().any(|net| net.contains(*ip)))
    {
        return *ip;
    }
    forwarded.first().copied().unwrap_or(peer)
}

fn trusted_from_env() -> Vec<IpNet> {
    let raw = std::env::var("RELAY_GATE_TRUSTED_PROXIES").unwrap_or_default();
    parse_trusted_proxies(&raw)
}

pub fn parse_trusted_proxies(raw: &str) -> Vec<IpNet> {
    let mut nets = loopback_nets();
    for part in raw.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        if let Ok(net) = part.parse::<IpNet>() {
            nets.push(net);
        } else if let Ok(ip) = part.parse::<IpAddr>() {
            nets.push(host_net(ip));
        } else {
            tracing::warn!("RELAY_GATE_TRUSTED_PROXIES: ignoring unparseable entry {part:?}");
        }
    }
    nets
}

pub fn loopback_nets() -> Vec<IpNet> {
    vec![
        "127.0.0.0/8".parse().expect("valid loopback v4"),
        "::1/128".parse().expect("valid loopback v6"),
    ]
}

fn host_net(ip: IpAddr) -> IpNet {
    match ip {
        IpAddr::V4(v4) => IpNet::V4(ipnet::Ipv4Net::new(v4, 32).expect("v4 /32")),
        IpAddr::V6(v6) => IpNet::V6(ipnet::Ipv6Net::new(v6, 128).expect("v6 /128")),
    }
}

fn env_usize(key: &str, default: usize) -> usize {
    match std::env::var(key) {
        Ok(value) if value.trim().is_empty() => default,
        Ok(value) => match value.trim().parse::<usize>() {
            Ok(parsed) if parsed > 0 => parsed,
            _ => {
                tracing::warn!(%key, %value, "invalid limit, using default");
                default
            }
        },
        Err(_) => default,
    }
}

fn env_u64(key: &str, default: u64) -> u64 {
    match std::env::var(key) {
        Ok(value) if value.trim().is_empty() => default,
        Ok(value) => match value.trim().parse::<u64>() {
            Ok(parsed) if parsed > 0 => parsed,
            _ => {
                tracing::warn!(%key, %value, "invalid limit window, using default");
                default
            }
        },
        Err(_) => default,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(text: &str) -> IpAddr {
        text.parse().unwrap()
    }

    #[test]
    fn untrusted_peer_ignores_forwarded_for() {
        let trusted = loopback_nets();
        let peer = ip("203.0.113.8");
        let forwarded = [ip("1.2.3.4")];
        assert_eq!(resolve_client_ip(peer, &forwarded, &trusted), peer);
    }

    #[test]
    fn trusted_peer_uses_rightmost_untrusted_forwarded_for() {
        let mut trusted = loopback_nets();
        trusted.push("10.0.0.1/32".parse().unwrap());
        let forwarded = [ip("1.2.3.4"), ip("203.0.113.9"), ip("10.0.0.1")];
        assert_eq!(
            resolve_client_ip(ip("127.0.0.1"), &forwarded, &trusted),
            ip("203.0.113.9")
        );
    }

    #[test]
    fn missing_forwarded_for_stays_on_the_peer() {
        let trusted = loopback_nets();
        assert_eq!(
            resolve_client_ip(ip("127.0.0.1"), &[], &trusted),
            ip("127.0.0.1")
        );
    }

    #[test]
    fn connection_cap_is_per_ip_and_releases_on_drop() {
        let limits = GateLimits::build(1, 100, 100, Duration::from_secs(60), loopback_nets());
        let first = limits.try_admit(ip("203.0.113.5")).unwrap();
        assert!(limits.try_admit(ip("203.0.113.5")).is_err());
        assert!(limits.try_admit(ip("203.0.113.6")).is_ok());
        drop(first);
        assert!(limits.try_admit(ip("203.0.113.5")).is_ok());
    }

    #[test]
    fn new_connection_rate_counts_rejected_attempts() {
        let limits = GateLimits::build(10, 2, 100, Duration::from_secs(60), loopback_nets());
        let a = limits.try_admit(ip("203.0.113.5")).unwrap();
        let b = limits.try_admit(ip("203.0.113.5")).unwrap();
        assert!(limits.try_admit(ip("203.0.113.5")).is_err());
        drop(a);
        drop(b);
        assert!(
            limits.try_admit(ip("203.0.113.5")).is_err(),
            "closing the sockets does not reset the window"
        );
    }

    #[test]
    fn message_rate_is_per_ip() {
        let limits = GateLimits::build(10, 100, 2, Duration::from_secs(60), loopback_nets());
        let client = ip("203.0.113.5");
        assert!(limits.allow_message(client));
        assert!(limits.allow_message(client));
        assert!(!limits.allow_message(client));
        assert!(limits.allow_message(ip("198.51.100.9")));
    }

    #[test]
    fn parse_trusted_proxies_keeps_loopback_and_bare_ips() {
        let nets = parse_trusted_proxies("10.89.0.1, 198.51.100.0/24, garbage");
        assert!(nets.iter().any(|net| net.contains(&ip("127.0.0.1"))));
        assert!(nets.iter().any(|net| net.contains(&ip("10.89.0.1"))));
        assert!(nets.iter().any(|net| net.contains(&ip("198.51.100.20"))));
        assert!(!nets.iter().any(|net| net.contains(&ip("203.0.113.1"))));
    }
}
