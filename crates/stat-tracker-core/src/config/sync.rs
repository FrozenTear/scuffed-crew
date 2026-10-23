//! `[sync]`: the site URL and daemon token, the rule that a missing token
//! means sync is off, and the check that the token is never sent in the
//! clear.

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct SyncConfig {
    pub server_url: String,
    pub token: String,
}

/// `[sync]` as written in config.toml, before the "no token means off" rule.
#[derive(Deserialize)]
struct StoredSync {
    server_url: Option<String>,
    token: Option<String>,
}

/// Reads `Config::sync`. A `[sync]` table whose `token` is missing, empty or
/// only spaces is sync off (`None`), like a file with no `[sync]` at all, and
/// the rest of the file loads as usual. Settings then shows sync as off, and
/// the daemon starts without a sync client.
///
/// A table that has a token but no `server_url` is still an error: reading it
/// as off would let a later full save drop the token.
pub(super) fn deserialize_sync<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<SyncConfig>, D::Error> {
    let Some(stored) = Option::<StoredSync>::deserialize(deserializer)? else {
        return Ok(None);
    };
    let Some(token) = stored.token.filter(|token| !token.trim().is_empty()) else {
        return Ok(None);
    };
    let server_url = stored
        .server_url
        .ok_or_else(|| D::Error::missing_field("server_url"))?;
    Ok(Some(SyncConfig { server_url, token }))
}

/// Settings copy and the daemon log share this sentence. Plain `http` to a
/// public host would put the bearer token on the wire for any network observer.
pub const SYNC_URL_HTTPS_REQUIRED: &str =
    "Website URL must use https. Plain http is only allowed for localhost, 127.0.0.1, and [::1].";

pub const SYNC_URL_INVALID: &str = "Website URL is not a valid http(s) address.";

/// Why a server URL must not carry the sync bearer token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncUrlReject {
    /// `http://` to a host that is not loopback, or a non-http(s) scheme.
    HttpsRequired,
    /// Empty, unparseable, or missing a host.
    Invalid,
}

impl SyncUrlReject {
    pub fn message(self) -> &'static str {
        match self {
            Self::HttpsRequired => SYNC_URL_HTTPS_REQUIRED,
            Self::Invalid => SYNC_URL_INVALID,
        }
    }
}

impl std::fmt::Display for SyncUrlReject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}

impl std::error::Error for SyncUrlReject {}

/// `Ok` only when attaching the bearer token cannot leave the machine in the
/// clear. `https` is allowed for any host. `http` is allowed only for
/// loopback (`localhost`, `127.0.0.0/8`, `::1`, and IPv4-mapped loopback),
/// which covers local dev (`http://localhost`, `http://127.0.0.1`,
/// `http://[::1]`, any port).
///
/// Parsing is the URL standard (the `url` crate's `Url`, which is what
/// `reqwest::Url` re-exports), so `http://127.0.0.1.evil`
/// and `http://10.0.0.1` are rejected without a DNS lookup. Existing configs
/// still deserialize; callers refuse to send rather than crash.
pub fn validate_sync_server_url(raw: &str) -> Result<(), SyncUrlReject> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(SyncUrlReject::Invalid);
    }
    let parsed = url::Url::parse(raw).map_err(|_| SyncUrlReject::Invalid)?;
    match parsed.scheme() {
        "https" if parsed.host_str().is_some_and(|h| !h.is_empty()) => Ok(()),
        "http" if http_host_is_loopback(&parsed) => Ok(()),
        _ => Err(SyncUrlReject::HttpsRequired),
    }
}

fn http_host_is_loopback(url: &url::Url) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    // `host_str` wraps IPv6 in brackets (`[::1]`).
    let bare = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);
    if let Ok(v4) = bare.parse::<std::net::Ipv4Addr>() {
        return v4.is_loopback();
    }
    if let Ok(v6) = bare.parse::<std::net::Ipv6Addr>() {
        if v6.is_loopback() {
            return true;
        }
        if let Some(mapped) = v6.to_ipv4_mapped() {
            return mapped.is_loopback();
        }
    }
    false
}
