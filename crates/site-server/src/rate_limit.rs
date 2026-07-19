//! Rate-limiter key extraction that does not trust spoofable forwarded headers.
//!
//! The default `tower_governor` `SmartIpKeyExtractor` derives the per-IP bucket
//! key from `X-Forwarded-For` / `X-Real-IP` / `Forwarded` **before** the peer
//! socket. Since any client can set those headers, an attacker who rotates
//! `X-Forwarded-For` on every request lands in a fresh token bucket each time
//! and the limiter never bites — defeating the brute-force / credential-stuffing
//! control on the auth routes (DR1-AUTH-001).
//!
//! [`TrustedProxyIpKeyExtractor`] only honors forwarded headers when the request
//! actually arrives from a configured trusted proxy; otherwise it keys off the
//! direct peer socket address. Forwarded entries are resolved right-to-left,
//! skipping trusted hops, so even a proxy that *appends* (rather than replaces)
//! `X-Forwarded-For` cannot be tricked into keying off an attacker-supplied
//! leftmost value.
//!
//! ## Configuration
//!
//! `TRUSTED_PROXIES` — comma-separated list of proxy IPs or CIDRs whose
//! forwarded headers are trusted (e.g. `127.0.0.1,10.89.0.0/16`). Loopback is
//! always trusted. When unset, the default trust set is loopback plus the
//! private / link-local / unique-local ranges — this covers the blessed deploy
//! (host Caddy → `127.0.0.1:HOST_PORT` → Podman-published container, where the
//! server sees the request arriving from the container-network gateway, a
//! private address) while still refusing to trust forwarded headers from a
//! public peer. In the blessed deploy the container binds `127.0.0.1` only, so a
//! public peer can only appear if an operator deliberately publishes the port on
//! a public interface — in which case that attacker's forwarded headers are
//! correctly ignored and the peer socket is used as the bucket key.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use axum::extract::ConnectInfo;
use axum::http::{HeaderMap, Request};
use ipnet::IpNet;
use tower_governor::{errors::GovernorError, key_extractor::KeyExtractor};

const X_FORWARDED_FOR: &str = "x-forwarded-for";
const X_REAL_IP: &str = "x-real-ip";

/// A [`KeyExtractor`] that keys rate limiting on the client IP, trusting
/// forwarded headers only from configured trusted proxies.
#[derive(Clone, Debug)]
pub struct TrustedProxyIpKeyExtractor {
    trusted: Arc<Vec<IpNet>>,
}

impl TrustedProxyIpKeyExtractor {
    /// Build from the `TRUSTED_PROXIES` env var (see module docs).
    pub fn from_env() -> Self {
        let trusted = match std::env::var("TRUSTED_PROXIES") {
            Ok(v) if !v.trim().is_empty() => parse_trusted_proxies(&v),
            _ => default_trusted_proxies(),
        };
        Self {
            trusted: Arc::new(trusted),
        }
    }

    /// Construct directly from a trust set (used in tests).
    #[cfg(test)]
    fn with_trusted(trusted: Vec<IpNet>) -> Self {
        Self {
            trusted: Arc::new(trusted),
        }
    }

    fn is_trusted(&self, ip: IpAddr) -> bool {
        self.trusted.iter().any(|net| net.contains(&ip))
    }

    /// Resolve the client IP used as the rate-limit bucket key.
    ///
    /// - If the peer socket is **not** a trusted proxy, use the peer IP and
    ///   ignore every forwarded header (the spoofing-resistant path).
    /// - If the peer **is** a trusted proxy, walk `X-Forwarded-For` right-to-left
    ///   skipping further trusted hops; the first untrusted entry is the client.
    ///   Fall back to `X-Real-IP`, then the peer, when XFF yields nothing.
    fn resolve_key(&self, peer: IpAddr, headers: &HeaderMap) -> IpAddr {
        if !self.is_trusted(peer) {
            return peer;
        }
        if let Some(ip) = client_from_forwarded_for(headers, &self.trusted) {
            return ip;
        }
        if let Some(ip) = single_ip_header(headers, X_REAL_IP) {
            return ip;
        }
        peer
    }
}

impl KeyExtractor for TrustedProxyIpKeyExtractor {
    type Key = IpAddr;

    fn extract<T>(&self, req: &Request<T>) -> Result<Self::Key, GovernorError> {
        // The peer socket is authoritative. It is injected by
        // `into_make_service_with_connect_info::<SocketAddr>()` (see server main).
        let peer = req
            .extensions()
            .get::<ConnectInfo<SocketAddr>>()
            .map(|ci| ci.0.ip())
            .ok_or(GovernorError::UnableToExtractKey)?;
        Ok(self.resolve_key(peer, req.headers()))
    }
}

/// Parse the single-IP `X-Real-IP` style header.
fn single_ip_header(headers: &HeaderMap, name: &str) -> Option<IpAddr> {
    headers
        .get(name)
        .and_then(|hv| hv.to_str().ok())
        .and_then(|s| s.trim().parse::<IpAddr>().ok())
}

/// Resolve the client IP from `X-Forwarded-For` given the trusted proxy set.
///
/// Entries are read left-to-right (client … nearest-proxy) across possibly
/// multiple header lines, then scanned **right-to-left**, skipping trusted proxy
/// addresses. The first untrusted address from the right is the real client:
/// a trusted proxy appends the actual peer it observed, so any attacker-injected
/// values sit to the *left* of that and are never reached. If every entry is
/// trusted (client itself inside a trusted range), the leftmost is returned.
fn client_from_forwarded_for(headers: &HeaderMap, trusted: &[IpNet]) -> Option<IpAddr> {
    let mut ips: Vec<IpAddr> = Vec::new();
    for hv in headers.get_all(X_FORWARDED_FOR).iter() {
        if let Ok(s) = hv.to_str() {
            for part in s.split(',') {
                if let Ok(ip) = part.trim().parse::<IpAddr>() {
                    ips.push(ip);
                }
            }
        }
    }
    if let Some(ip) = ips
        .iter()
        .rev()
        .find(|ip| !trusted.iter().any(|net| net.contains(*ip)))
    {
        return Some(*ip);
    }
    // All entries trusted (or none present).
    ips.first().copied()
}

/// Parse an explicit `TRUSTED_PROXIES` value: comma-separated IPs or CIDRs.
///
/// Bare IPs are treated as host networks (`/32` or `/128`). Loopback is always
/// added so the documented Caddy-on-`127.0.0.1` pattern works even when the
/// operator forgets to list it. Unparseable entries are skipped with a warning.
fn parse_trusted_proxies(raw: &str) -> Vec<IpNet> {
    let mut nets = loopback_nets();
    for part in raw.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        if let Ok(net) = part.parse::<IpNet>() {
            nets.push(net);
        } else if let Ok(ip) = part.parse::<IpAddr>() {
            nets.push(host_net(ip));
        } else {
            tracing::warn!("TRUSTED_PROXIES: ignoring unparseable entry {part:?}");
        }
    }
    nets
}

/// Default trust set: loopback + private + link-local + unique-local ranges.
fn default_trusted_proxies() -> Vec<IpNet> {
    let mut nets = loopback_nets();
    for cidr in [
        "10.0.0.0/8",
        "172.16.0.0/12",
        "192.168.0.0/16",
        "169.254.0.0/16", // IPv4 link-local
        "fc00::/7",       // IPv6 unique-local
        "fe80::/10",      // IPv6 link-local
    ] {
        nets.push(cidr.parse().expect("valid default CIDR"));
    }
    nets
}

fn loopback_nets() -> Vec<IpNet> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::ConnectInfo;
    use axum::http::Request;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    fn net(s: &str) -> IpNet {
        s.parse().unwrap()
    }

    /// Build a request carrying a ConnectInfo peer + optional XFF header.
    fn req(peer: &str, xff: Option<&str>) -> Request<()> {
        let mut b = Request::builder();
        if let Some(v) = xff {
            b = b.header("x-forwarded-for", v);
        }
        let mut r = b.body(()).unwrap();
        r.extensions_mut()
            .insert(ConnectInfo(SocketAddr::new(ip(peer), 12345)));
        r
    }

    #[test]
    fn untrusted_peer_ignores_forwarded_for() {
        // Default trust set (loopback + private). A public peer is untrusted, so
        // a rotating XFF must NOT change the key — it stays the peer IP.
        let ex = TrustedProxyIpKeyExtractor::with_trusted(default_trusted_proxies());

        let k1 = ex.extract(&req("203.0.113.7", Some("1.1.1.1"))).unwrap();
        let k2 = ex.extract(&req("203.0.113.7", Some("2.2.2.2"))).unwrap();
        let k3 = ex
            .extract(&req("203.0.113.7", Some("9.9.9.9, 8.8.8.8")))
            .unwrap();

        assert_eq!(k1, ip("203.0.113.7"));
        assert_eq!(k1, k2, "rotating XFF must not create a new bucket");
        assert_eq!(k1, k3);
    }

    #[test]
    fn trusted_proxy_honors_forwarded_for() {
        // Caddy on the container-network gateway (private, trusted): the client
        // IP it forwards is honored.
        let ex = TrustedProxyIpKeyExtractor::with_trusted(default_trusted_proxies());
        let k = ex
            .extract(&req("10.89.0.1", Some("198.51.100.23")))
            .unwrap();
        assert_eq!(k, ip("198.51.100.23"));
    }

    #[test]
    fn appending_proxy_cannot_be_spoofed() {
        // Attacker sends a spoofed XFF; the trusted proxy APPENDS the real peer.
        // Right-to-left resolution must pick the appended real client, not the
        // attacker's leftmost value — and it must not depend on the spoof.
        let ex = TrustedProxyIpKeyExtractor::with_trusted(default_trusted_proxies());

        let real = ex
            .extract(&req("127.0.0.1", Some("6.6.6.6, 198.51.100.23")))
            .unwrap();
        assert_eq!(real, ip("198.51.100.23"));

        // Attacker rotates the spoofed leftmost value: key stays the real client.
        let rotated = ex
            .extract(&req("127.0.0.1", Some("7.7.7.7, 198.51.100.23")))
            .unwrap();
        assert_eq!(
            rotated, real,
            "spoofed leftmost XFF must not shift the bucket"
        );
    }

    #[test]
    fn multi_hop_trusted_chain_skips_trusted() {
        // client -> trusted CDN -> trusted Caddy -> server. XFF from Caddy:
        // "<client>, <cdn>"; peer is Caddy. Both proxies trusted; resolution
        // walks past the trusted CDN to the untrusted client.
        let ex = TrustedProxyIpKeyExtractor::with_trusted(vec![
            net("127.0.0.0/8"),
            net("::1/128"),
            net("10.0.0.0/8"),
        ]);
        let k = ex
            .extract(&req("10.0.0.5", Some("198.51.100.23, 10.0.0.9")))
            .unwrap();
        assert_eq!(k, ip("198.51.100.23"));
    }

    #[test]
    fn trusted_proxy_without_xff_falls_back_to_peer() {
        let ex = TrustedProxyIpKeyExtractor::with_trusted(default_trusted_proxies());
        let k = ex.extract(&req("10.89.0.1", None)).unwrap();
        assert_eq!(k, ip("10.89.0.1"));
    }

    #[test]
    fn missing_connect_info_errors() {
        let ex = TrustedProxyIpKeyExtractor::with_trusted(default_trusted_proxies());
        let r: Request<()> = Request::builder().body(()).unwrap();
        assert!(ex.extract(&r).is_err());
    }

    #[test]
    fn explicit_trusted_proxies_env_parsing() {
        let nets = parse_trusted_proxies("192.0.2.10, 198.51.100.0/24, garbage");
        // Loopback always present.
        assert!(nets.iter().any(|n| n.contains(&ip("127.0.0.1"))));
        // Bare IP became a host route.
        assert!(nets.iter().any(|n| n.contains(&ip("192.0.2.10"))));
        assert!(!nets.iter().any(|n| n.contains(&ip("192.0.2.11"))));
        // CIDR honored.
        assert!(nets.iter().any(|n| n.contains(&ip("198.51.100.77"))));
    }

    #[test]
    fn xff_only_honored_from_explicit_trusted_peer() {
        // With a narrow explicit trust set, a peer outside it is untrusted.
        let ex = TrustedProxyIpKeyExtractor::with_trusted(parse_trusted_proxies("192.0.2.1"));
        // Trusted peer -> XFF honored.
        assert_eq!(
            ex.extract(&req("192.0.2.1", Some("203.0.113.9"))).unwrap(),
            ip("203.0.113.9")
        );
        // Private peer NOT in the explicit set -> untrusted -> peer key.
        assert_eq!(
            ex.extract(&req("10.0.0.1", Some("203.0.113.9"))).unwrap(),
            ip("10.0.0.1")
        );
    }
}
