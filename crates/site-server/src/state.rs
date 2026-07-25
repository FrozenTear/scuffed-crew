use std::path::PathBuf;
use std::sync::Arc;

use scuffed_auth::crypto::CryptoService;
use scuffed_auth::server::HasAuth;
use scuffed_auth::{AuthError, SessionConfig, User};
use scuffed_db::Database;

use crate::dm_subscriber::DmEventBus;
use crate::notifications::Notifier;

/// Application state shared across handlers.
#[derive(Clone)]
pub struct AppState {
    pub db: Arc<Database>,
    pub session_config: SessionConfig,
    pub oauth_config: OAuthConfig,
    pub upload_dir: PathBuf,
    /// Fan-out Matrix + Discord notifications. `None` when neither is configured.
    pub notifier: Option<Notifier>,
    /// 32-byte key for HMAC-signing Nostr challenge tokens.
    pub nostr_challenge_key: [u8; 32],
    /// One-time store of consumed Nostr login/link challenges (replay guard).
    /// Per-process; see [`crate::challenge_store`] for the multi-instance caveat.
    pub consumed_challenges: crate::challenge_store::ConsumedChallengeStore,
    /// Per-member token-bucket limiter for the secret-touching Nostr routes
    /// (challenge/verify/export/import/dm-send). See [`crate::nostr_rate_limit`].
    pub nostr_rate_limiter: crate::nostr_rate_limit::NostrRateLimiter,
    /// Shared encryption service (same `Arc` as `db.crypto`).
    /// `None` when `ENCRYPTION_KEY` is not configured.
    pub crypto: Option<Arc<CryptoService>>,
    /// WebSocket URL for the Nostr relay (e.g., `ws://strfry:7777`).
    /// Used for publishing kind 0 profile metadata and NIP-05 relay hints.
    /// `None` when `NOSTR_RELAY_URL` is unset or blank (F-AUI-003).
    pub relay_url: Option<String>,
    /// In-process event bus fed by the persistent DM relay subscriber.
    /// `None` when real-time delivery is disabled (no relay or no encryption
    /// configured); SSE handlers should treat that as a 503.
    pub dm_events: Option<DmEventBus>,
    /// Public domain that serves `/.well-known/nostr.json`, used as the
    /// right-hand side of members' NIP-05 identifiers (`name@domain`).
    /// `None` when no *valid public* domain is configured — in that case we
    /// publish kind-0 metadata **without** a `nip05` field rather than minting
    /// an identity that cannot verify. See [`nip05_domain_from_env`].
    pub nip05_domain: Option<String>,
    /// Whether the kind-0 republish endpoint is armed (`NIP05_REPUBLISH_ENABLED=1`).
    ///
    /// Off by default and deliberately not inferred from anything else.
    /// Republishing writes new immutable events to public relays on members'
    /// behalf, so it takes an explicit operator action to even become callable
    /// — see `routes::members::republish_profiles`.
    pub nip05_republish_enabled: bool,
}

/// Treat blank/whitespace as unset so `NOSTR_RELAY_URL=""` does not report
/// `configured: true` with an empty URL (F-AUI-003).
pub fn normalize_relay_url(url: Option<String>) -> Option<String> {
    url.and_then(|u| {
        let t = u.trim();
        if t.is_empty() {
            None
        } else {
            Some(t.to_string())
        }
    })
}

/// Load primary relay URL from `NOSTR_RELAY_URL`, ignoring empty values.
pub fn relay_url_from_env() -> Option<String> {
    normalize_relay_url(std::env::var("NOSTR_RELAY_URL").ok())
}

/// Validate a candidate NIP-05 domain, returning it normalized or `None`.
///
/// A NIP-05 identifier is `name@domain`, and verifiers fetch
/// `https://<domain>/.well-known/nostr.json`. Publishing one we do not control
/// is worse than publishing none: kind-0 events are immutable on relays, so a
/// wrong domain is a permanently-broken identity for every member — and an
/// identity someone else can take over by registering that domain.
///
/// Accepts a bare host (`ow.scuffedcrew.no`) or a full URL
/// (`https://ow.scuffedcrew.no/`), and rejects anything that cannot work as a
/// public verification target:
/// - loopback / private / link-local hosts, and bare IP literals
/// - non-public TLDs (`.local`, `.internal`, `.test`, `.invalid`, `.localhost`)
/// - single-label hosts with no dot at all
/// - anything carrying a port (NIP-05 verification is https/443 only)
pub fn validate_nip05_domain(candidate: &str) -> Option<String> {
    let mut host = candidate.trim().to_lowercase();

    // Accept a full URL by stripping scheme, then any path/query/fragment.
    if let Some(rest) = host.split("://").nth(1) {
        host = rest.to_string();
    }
    host = host
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default()
        .to_string();
    // Strip userinfo if someone pasted one.
    if let Some(after_at) = host.rsplit('@').next() {
        host = after_at.to_string();
    }
    let host = host.trim_end_matches('.').to_string();

    if host.is_empty() {
        return None;
    }

    // A port means this is not a plain https origin — NIP-05 clients fetch
    // https://<domain>/.well-known/nostr.json on 443 and would drop the port.
    // Bracketed IPv6 also lands here.
    if host.contains(':') || host.starts_with('[') {
        return None;
    }

    // Bare IP literals can never be a NIP-05 domain.
    if host.parse::<std::net::IpAddr>().is_ok() {
        return None;
    }

    // Must be a dotted, public-looking name.
    if !host.contains('.') {
        return None;
    }

    const NON_PUBLIC_SUFFIXES: [&str; 6] = [
        ".local",
        ".localhost",
        ".localdomain",
        ".internal",
        ".test",
        ".invalid",
    ];
    if host == "localhost" || NON_PUBLIC_SUFFIXES.iter().any(|s| host.ends_with(s)) {
        return None;
    }

    // Reject obvious label garbage rather than publishing it.
    if !host
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
        || host.starts_with('.')
        || host.contains("..")
    {
        return None;
    }

    Some(host)
}

/// Resolve the NIP-05 domain from configuration.
///
/// Order: explicit `NIP05_DOMAIN`, else a *validated* derivation from
/// `REDIRECT_BASE_URL`. The derivation is deliberately not a blind reuse —
/// `REDIRECT_BASE_URL` defaults to `http://localhost:3000` here and to
/// `127.0.0.1:3000` in `compose.yml`, and the installer accepts a blank
/// public URL, so a naive fallback would publish immutable
/// `name@127.0.0.1:3000` identities on any default-configured deploy.
pub fn nip05_domain_from_env() -> Option<String> {
    if let Ok(explicit) = std::env::var("NIP05_DOMAIN")
        && !explicit.trim().is_empty()
    {
        return match validate_nip05_domain(&explicit) {
            Some(d) => Some(d),
            None => {
                tracing::warn!(
                    "NIP05_DOMAIN={explicit:?} is not a usable public domain — \
                     publishing kind-0 profiles without a nip05 field"
                );
                None
            }
        };
    }

    match std::env::var("REDIRECT_BASE_URL")
        .ok()
        .and_then(|u| validate_nip05_domain(&u))
    {
        Some(d) => Some(d),
        None => {
            tracing::warn!(
                "No public NIP-05 domain configured (set NIP05_DOMAIN) — \
                 kind-0 profiles will publish without a nip05 field"
            );
            None
        }
    }
}

/// Whether the kind-0 republish endpoint is armed.
///
/// Strictly opt-in: only the exact string `1` arms it. Anything else — unset,
/// empty, `true`, `yes` — leaves it off. A republish writes immutable events to
/// public relays for every member, so "I typed something truthy" is not a good
/// enough signal.
pub fn nip05_republish_enabled_from_env() -> bool {
    std::env::var("NIP05_REPUBLISH_ENABLED").is_ok_and(|v| v.trim() == "1")
}

/// Build a member's NIP-05 identifier, or `None` when we have no valid domain
/// or the display name normalizes to nothing.
pub fn nip05_identifier(display_name: &str, domain: Option<&str>) -> Option<String> {
    let domain = domain?;
    let name: String = display_name
        .to_lowercase()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();
    if name.is_empty() {
        None
    } else {
        Some(format!("{name}@{domain}"))
    }
}

#[cfg(test)]
mod tests {
    use super::{nip05_identifier, normalize_relay_url, validate_nip05_domain};

    #[test]
    fn blank_env_style_urls_are_not_configured() {
        assert_eq!(normalize_relay_url(None), None);
        assert_eq!(normalize_relay_url(Some(String::new())), None);
        assert_eq!(normalize_relay_url(Some("   ".into())), None);
        assert_eq!(
            normalize_relay_url(Some("  wss://relay.example  ".into())),
            Some("wss://relay.example".into())
        );
    }

    #[test]
    fn accepts_public_domains_bare_or_url() {
        for input in [
            "ow.scuffedcrew.no",
            "  OW.ScuffedCrew.no  ",
            "https://ow.scuffedcrew.no",
            "https://ow.scuffedcrew.no/",
            "https://ow.scuffedcrew.no/some/path?q=1#frag",
            "ow.scuffedcrew.no.",
        ] {
            assert_eq!(
                validate_nip05_domain(input).as_deref(),
                Some("ow.scuffedcrew.no"),
                "should accept {input:?}"
            );
        }
    }

    /// The whole point of the item: a default-configured deploy must publish
    /// **no** nip05 rather than an immutable `name@127.0.0.1:3000` identity.
    #[test]
    fn rejects_loopback_private_and_portful_hosts() {
        for input in [
            "",
            "   ",
            "localhost",
            "http://localhost:3000",
            "http://127.0.0.1:3000",
            "127.0.0.1",
            "192.168.1.10",
            "10.0.0.5",
            "::1",
            "[::1]:3000",
            "ow.scuffedcrew.no:8443",
            "myserver",
            "box.local",
            "svc.internal",
            "thing.test",
            "nope.invalid",
            "app.localhost",
            "..",
            "-",
        ] {
            assert_eq!(
                validate_nip05_domain(input),
                None,
                "should reject {input:?}"
            );
        }
    }

    #[test]
    fn identifier_needs_both_a_name_and_a_domain() {
        assert_eq!(
            nip05_identifier("Frozen Tear", Some("ow.scuffedcrew.no")).as_deref(),
            Some("frozentear@ow.scuffedcrew.no")
        );
        // No configured domain → no identifier at all.
        assert_eq!(nip05_identifier("Frozen Tear", None), None);
        // Name that normalizes to nothing → no identifier.
        assert_eq!(nip05_identifier("!!!", Some("ow.scuffedcrew.no")), None);
    }
}

/// OAuth configuration loaded from environment.
#[derive(Clone)]
pub struct OAuthConfig {
    pub discord_client_id: String,
    pub discord_client_secret: String,
    pub google_client_id: String,
    pub google_client_secret: String,
    pub redirect_base_url: String,
    pub allowed_origins: Vec<String>,
}

impl OAuthConfig {
    pub fn from_env() -> Self {
        let redirect_base_url = std::env::var("REDIRECT_BASE_URL")
            .unwrap_or_else(|_| "http://localhost:3000".to_string());

        let allowed_origins = std::env::var("ALLOWED_ORIGINS")
            .map(|s| s.split(',').map(|s| s.trim().to_string()).collect())
            .unwrap_or_else(|_| vec![redirect_base_url.clone()]);

        let discord_client_id = std::env::var("DISCORD_CLIENT_ID").unwrap_or_default();
        let discord_client_secret = std::env::var("DISCORD_CLIENT_SECRET").unwrap_or_default();
        let google_client_id = std::env::var("GOOGLE_CLIENT_ID").unwrap_or_default();
        let google_client_secret = std::env::var("GOOGLE_CLIENT_SECRET").unwrap_or_default();

        if discord_client_id.is_empty() || discord_client_secret.is_empty() {
            tracing::warn!("Discord OAuth not configured — login disabled");
        }
        if google_client_id.is_empty() || google_client_secret.is_empty() {
            tracing::warn!("Google OAuth not configured — login disabled");
        }

        Self {
            discord_client_id,
            discord_client_secret,
            google_client_id,
            google_client_secret,
            redirect_base_url,
            allowed_origins,
        }
    }
}

impl HasAuth for AppState {
    fn session_config(&self) -> &SessionConfig {
        &self.session_config
    }

    async fn get_session_user(&self, token: &str) -> Result<Option<User>, AuthError> {
        self.db.get_session_user(token).await.map_err(|e| {
            tracing::error!("Session user lookup failed: {e}");
            AuthError::Database(e.to_string())
        })
    }
}
