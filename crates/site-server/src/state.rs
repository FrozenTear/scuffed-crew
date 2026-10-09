#[cfg(test)]
use std::future::Future;
use std::path::PathBuf;
#[cfg(test)]
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::time::Instant;

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
    /// Failed password-login backoff, keyed by normalized username.
    /// Password login only — not bearer tokens or OAuth. See [`crate::login_lockout`].
    pub login_lockout: crate::login_lockout::LoginLockout,
    /// Wrong device-link user codes, keyed by client IP. See [`crate::link_attempts`].
    pub link_code_attempts: crate::link_attempts::LinkCodeAttempts,
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
    /// In-memory copy of the anonymous settings embedded in the SPA shell.
    /// Write paths call [`PublicSettingsCache::invalidate`]. Per process:
    /// a restart clears it, and it is not shared across instances.
    pub public_settings: PublicSettingsCache,
    /// In-process public leaderboard cache. See [`crate::leaderboard_cache`].
    pub leaderboard_cache: crate::leaderboard_cache::LeaderboardCache,
}

/// How long a cached public-settings blob may be served before a re-read.
///
/// Writes invalidate immediately. The TTL only covers a missed invalidation.
pub(crate) const PUBLIC_SETTINGS_TTL: Duration = Duration::from_secs(10);

/// After a failed or timed-out refresh, further misses serve the stale blob
/// immediately for this long instead of starting another read. An invalidation
/// clears this by moving `generation`. A spawned read that is still in flight
/// and older than the embed cap is also served stale, until that read drops
/// its lock. See [`PublicSettingsCache::refresh_in_flight_older_than`].
pub(crate) const EMBED_REFRESH_BACKOFF: Duration = Duration::from_secs(1);

/// DB-outage warnings for the shell embed, at most once per interval.
const SETTINGS_WARN_INTERVAL: Duration = Duration::from_secs(60);

/// Cached rewritten head: `(template Arc, head prefix)`, compared with `Arc::ptr_eq`.
type RenderedHead = std::sync::Arc<Mutex<Option<(std::sync::Arc<str>, std::sync::Arc<str>)>>>;

/// Anonymous settings already rendered for the HTML shell.
///
/// `script_block` is produced once, when the blob is stored. The rewritten
/// head prefix is filled on the first response that uses this blob and reused
/// after that, so a hit does not scan the template again.
#[derive(Clone, Debug)]
pub(crate) struct CachedPublicSettings {
    pub script_block: String,
    pub org_name: String,
    pub site_description: String,
    /// Rewritten head prefix for the template this blob was last rendered with.
    rendered_head: RenderedHead,
}

impl CachedPublicSettings {
    pub(crate) fn from_parts(
        script_block: String,
        org_name: String,
        site_description: String,
    ) -> Self {
        Self {
            script_block,
            org_name,
            site_description,
            rendered_head: std::sync::Arc::new(Mutex::new(None)),
        }
    }

    /// Rewritten head prefix for `template`, computed once for this blob.
    ///
    /// The slot keeps the template `Arc` and matches it with [`Arc::ptr_eq`],
    /// so a later template allocated at the same address is not reused.
    pub(crate) fn rendered_head(
        &self,
        template: &std::sync::Arc<str>,
        build: impl FnOnce() -> String,
    ) -> std::sync::Arc<str> {
        let mut slot = self
            .rendered_head
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        if let Some((key, head)) = slot.as_ref()
            && std::sync::Arc::ptr_eq(key, template)
        {
            return std::sync::Arc::clone(head);
        }
        let head: std::sync::Arc<str> = std::sync::Arc::from(build());
        *slot = Some((
            std::sync::Arc::clone(template),
            std::sync::Arc::clone(&head),
        ));
        head
    }
}

#[derive(Debug)]
struct PublicSettingsEntry {
    payload: Arc<CachedPublicSettings>,
    stored_at: Instant,
    generation: u64,
}

#[derive(Debug)]
struct PublicSettingsInner {
    generation: u64,
    entry: Option<PublicSettingsEntry>,
    /// `(generation, when)` of the last failed refresh for that generation.
    refresh_failed: Option<(u64, Instant)>,
    /// When the spawned read holding `refresh` started. Cleared when that
    /// lock drops.
    refresh_started: Option<Instant>,
}

/// Test-only stand-in for the settings read, invoked at the real load point
/// (inside the refresh lock and the embed timeout). Not in release builds.
#[cfg(test)]
pub(crate) type EmbedLoader = Arc<
    dyn Fn() -> Pin<Box<dyn Future<Output = Result<CachedPublicSettings, String>> + Send>>
        + Send
        + Sync,
>;

/// Test-only hook around a settings database write.
///
/// As the before-write hook it runs after the early invalidation and before
/// the database update, so a test can start an in-flight read in that window.
/// As the after-write hook it runs after that update returns and before the
/// handler returns, while the drop guard is still held.
#[cfg(test)]
pub(crate) type SettingsWriteHook =
    Arc<dyn Fn() -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

/// Loader and the before-write and after-write hooks. Manual `Debug` because
/// the trait objects are not.
#[cfg(test)]
#[derive(Default)]
struct SettingsTestHooks {
    loader: Mutex<Option<EmbedLoader>>,
    write_hook: Mutex<Option<SettingsWriteHook>>,
    after_write_hook: Mutex<Option<SettingsWriteHook>>,
}

#[cfg(test)]
impl std::fmt::Debug for SettingsTestHooks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SettingsTestHooks")
    }
}

/// Process-local cache of the anonymous settings embed.
///
/// Shared across `AppState` clones (the router and the SPA fallback hold the
/// same `Arc`). A settings write bumps `generation` and drops any blob stored
/// before that bump, so an in-flight read cannot store or later serve a
/// pre-write row. A blob that is merely past the TTL stays available for
/// stale-on-error.
#[derive(Clone, Debug)]
pub struct PublicSettingsCache {
    inner: Arc<Mutex<PublicSettingsInner>>,
    /// One settings read at a time. Concurrent misses wait on this instead of
    /// each querying the database.
    refresh: Arc<tokio::sync::Mutex<()>>,
    last_warn: Arc<Mutex<Option<Instant>>>,
    #[cfg(test)]
    hooks: Arc<SettingsTestHooks>,
}

impl Default for PublicSettingsCache {
    fn default() -> Self {
        Self::new()
    }
}

impl PublicSettingsCache {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(PublicSettingsInner {
                generation: 0,
                entry: None,
                refresh_failed: None,
                refresh_started: None,
            })),
            refresh: Arc::new(tokio::sync::Mutex::new(())),
            last_warn: Arc::new(Mutex::new(None)),
            #[cfg(test)]
            hooks: Arc::new(SettingsTestHooks::default()),
        }
    }

    /// Bump the generation and drop every blob stored before this call.
    ///
    /// Settings writes call this before the database update and again when
    /// that update returns, success or error. The second bump discards a
    /// read that re-cached the pre-write row, so that row cannot stay fresh
    /// or be served later as the stale-on-error fallback.
    pub fn invalidate(&self) {
        let mut guard = self.lock();
        guard.generation = guard.generation.wrapping_add(1);
        guard.refresh_failed = None;
        guard.entry = None;
    }

    /// Second [`Self::invalidate`] when the guard drops.
    ///
    /// Held across the write so every return path, including a validation
    /// error or a dropped request, bumps the generation again after the attempt.
    #[must_use = "bind it so it drops after the write"]
    pub(crate) fn invalidate_on_drop(&self) -> InvalidateOnDrop<'_> {
        InvalidateOnDrop(self)
    }

    pub(crate) fn generation(&self) -> u64 {
        self.lock().generation
    }

    /// Fresh blob, if one was stored for the current generation inside the TTL.
    pub(crate) fn fresh(&self) -> Option<Arc<CachedPublicSettings>> {
        let guard = self.lock();
        let entry = guard.entry.as_ref()?;
        if entry.generation != guard.generation {
            return None;
        }
        if entry.stored_at.elapsed() >= PUBLIC_SETTINGS_TTL {
            return None;
        }
        Some(Arc::clone(&entry.payload))
    }

    /// Last blob from the current generation, including after the TTL.
    ///
    /// [`Self::store`] writes nothing when the generation has moved, and
    /// [`Self::invalidate`] clears the entry. The generation check is only a
    /// safety net for a blob that is still in the slot after a bump that did
    /// not clear it.
    pub(crate) fn stale(&self) -> Option<Arc<CachedPublicSettings>> {
        let guard = self.lock();
        let entry = guard.entry.as_ref()?;
        if entry.generation != guard.generation {
            return None;
        }
        Some(Arc::clone(&entry.payload))
    }

    /// Store `payload` only if no invalidation landed since `generation`.
    ///
    /// `started` is when the read began. The TTL is measured from then, so a
    /// slow read does not add its own duration on top of the 10 seconds.
    /// Returns whether the cache accepted the write.
    pub(crate) fn store(
        &self,
        generation: u64,
        payload: CachedPublicSettings,
        started: Instant,
    ) -> bool {
        let mut guard = self.lock();
        if guard.generation != generation {
            return false;
        }
        guard.refresh_failed = None;
        guard.entry = Some(PublicSettingsEntry {
            payload: Arc::new(payload),
            stored_at: started,
            generation,
        });
        true
    }

    /// Record that the spawned read holding the refresh lock started at `started`.
    pub(crate) fn note_refresh_started(&self, started: Instant) {
        self.lock().refresh_started = Some(started);
    }

    /// Clear the in-flight start. Called when that read drops the refresh lock.
    pub(crate) fn clear_refresh_started(&self) {
        self.lock().refresh_started = None;
    }

    /// `true` when a spawned read is still in flight and started more than `cap` ago.
    ///
    /// Callers should serve [`Self::stale`] immediately instead of waiting on
    /// that read's lock.
    pub(crate) fn refresh_in_flight_older_than(&self, cap: Duration) -> bool {
        self.lock()
            .refresh_started
            .is_some_and(|started| started.elapsed() > cap)
    }

    /// `true` when a refresh for this generation just failed and callers
    /// should serve [`Self::stale`] instead of starting another read.
    pub(crate) fn refresh_suppressed(&self) -> bool {
        let guard = self.lock();
        match guard.refresh_failed {
            Some((generation, at)) => {
                generation == guard.generation && at.elapsed() < EMBED_REFRESH_BACKOFF
            }
            None => false,
        }
    }

    pub(crate) fn note_refresh_failure(&self, generation: u64) {
        let mut guard = self.lock();
        if guard.generation == generation {
            guard.refresh_failed = Some((generation, Instant::now()));
        }
    }

    #[cfg(test)]
    pub(crate) async fn refresh_lock(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.refresh.lock().await
    }

    /// Owned guard so a settings read can outlive the request that started it.
    pub(crate) async fn refresh_lock_owned(&self) -> tokio::sync::OwnedMutexGuard<()> {
        Arc::clone(&self.refresh).lock_owned().await
    }

    /// Log `message` at most once per [`SETTINGS_WARN_INTERVAL`].
    pub(crate) fn note_embed_failure(&self, message: &str) -> bool {
        let mut slot = self.last_warn.lock().unwrap_or_else(|err| err.into_inner());
        let now = Instant::now();
        if slot.is_some_and(|prev| now.saturating_duration_since(prev) < SETTINGS_WARN_INTERVAL) {
            return false;
        }
        *slot = Some(now);
        tracing::warn!("{message}");
        true
    }

    #[cfg(test)]
    pub(crate) fn set_loader(&self, loader: Option<EmbedLoader>) {
        *self
            .hooks
            .loader
            .lock()
            .unwrap_or_else(|err| err.into_inner()) = loader;
    }

    #[cfg(test)]
    pub(crate) fn loader(&self) -> Option<EmbedLoader> {
        self.hooks
            .loader
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone()
    }

    /// Install a hook for the next settings write. The handler takes it once,
    /// before the database update.
    #[cfg(test)]
    pub(crate) fn set_write_hook(&self, hook: Option<SettingsWriteHook>) {
        *self
            .hooks
            .write_hook
            .lock()
            .unwrap_or_else(|err| err.into_inner()) = hook;
    }

    /// Hook that runs after `update_settings` returns and before the handler does.
    #[cfg(test)]
    pub(crate) fn set_after_write_hook(&self, hook: Option<SettingsWriteHook>) {
        *self
            .hooks
            .after_write_hook
            .lock()
            .unwrap_or_else(|err| err.into_inner()) = hook;
    }

    /// Run and clear the write hook, if a test installed one.
    #[cfg(test)]
    pub(crate) async fn run_write_hook(&self) {
        Self::take_hook(&self.hooks.write_hook).await;
    }

    /// Run and clear the post-write hook, if a test installed one.
    #[cfg(test)]
    pub(crate) async fn run_after_write_hook(&self) {
        Self::take_hook(&self.hooks.after_write_hook).await;
    }

    #[cfg(test)]
    async fn take_hook(slot: &Mutex<Option<SettingsWriteHook>>) {
        let hook = slot.lock().unwrap_or_else(|err| err.into_inner()).take();
        if let Some(hook) = hook {
            hook().await;
        }
    }

    /// Push the stored blob past the TTL without sleeping.
    #[cfg(test)]
    pub(crate) fn expire_for_test(&self) {
        let mut guard = self.lock();
        if let Some(entry) = guard.entry.as_mut() {
            entry.stored_at = Instant::now() - (PUBLIC_SETTINGS_TTL + Duration::from_secs(1));
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, PublicSettingsInner> {
        self.inner.lock().unwrap_or_else(|err| err.into_inner())
    }
}

/// Calls [`PublicSettingsCache::invalidate`] when dropped.
pub struct InvalidateOnDrop<'a>(&'a PublicSettingsCache);

impl Drop for InvalidateOnDrop<'_> {
    fn drop(&mut self) {
        self.0.invalidate();
    }
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
    use super::{
        nip05_identifier, normalize_relay_url, parse_allowed_origins, validate_nip05_domain,
    };

    #[test]
    fn allowed_origins_blank_or_unset_falls_back_to_redirect() {
        let fallback = "http://localhost:3000";
        // Unset
        assert_eq!(
            parse_allowed_origins(None, fallback),
            vec![fallback.to_string()]
        );
        // Empty / whitespace (compose `ALLOWED_ORIGINS=`)
        assert_eq!(
            parse_allowed_origins(Some(""), fallback),
            vec![fallback.to_string()]
        );
        assert_eq!(
            parse_allowed_origins(Some("   "), fallback),
            vec![fallback.to_string()]
        );
        assert_eq!(
            parse_allowed_origins(Some(","), fallback),
            vec![fallback.to_string()]
        );
        assert_eq!(
            parse_allowed_origins(Some(" ,  , "), fallback),
            vec![fallback.to_string()]
        );
    }

    #[test]
    fn allowed_origins_explicit_list_is_kept() {
        assert_eq!(
            parse_allowed_origins(Some("https://ow.scuffedcrew.no"), "http://localhost:3000"),
            vec!["https://ow.scuffedcrew.no".to_string()]
        );
        assert_eq!(
            parse_allowed_origins(
                Some(" https://a.example , , https://b.example "),
                "http://localhost:3000"
            ),
            vec![
                "https://a.example".to_string(),
                "https://b.example".to_string()
            ]
        );
    }

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

#[cfg(test)]
mod public_settings_cache_tests {
    use super::{CachedPublicSettings, Instant, PublicSettingsCache};

    fn payload(name: &str) -> CachedPublicSettings {
        CachedPublicSettings::from_parts(
            format!(
                "<script id=\"sc-settings\" type=\"application/json\">{{\"org_name\":\"{name}\"}}</script>"
            ),
            name.to_string(),
            "tagline".into(),
        )
    }

    #[tokio::test]
    async fn stale_writer_cannot_overwrite_a_newer_invalidation() {
        let cache = PublicSettingsCache::new();
        let generation = cache.generation();
        assert!(cache.store(generation, payload("First"), Instant::now()));
        assert_eq!(cache.fresh().unwrap().org_name, "First");

        cache.invalidate();
        assert!(cache.fresh().is_none());
        assert!(
            cache.stale().is_none(),
            "invalidate drops blobs stored before the bump"
        );
        assert!(
            !cache.store(generation, payload("Stale"), Instant::now()),
            "a read that started before invalidation must not store"
        );
        assert!(cache.stale().is_none());

        let next = cache.generation();
        assert_ne!(next, generation);
        assert!(cache.store(next, payload("Fresh"), Instant::now()));
        assert_eq!(cache.fresh().unwrap().org_name, "Fresh");
        assert!(cache.fresh().unwrap().script_block.contains("Fresh"));
    }

    #[tokio::test]
    async fn ttl_expiry_keeps_the_blob_for_stale_on_error() {
        let cache = PublicSettingsCache::new();
        assert!(cache.store(cache.generation(), payload("Cached"), Instant::now()));
        assert!(cache.fresh().is_some());
        cache.expire_for_test();
        assert!(
            cache.fresh().is_none(),
            "an expired blob is not a fresh hit"
        );
        assert_eq!(cache.stale().unwrap().org_name, "Cached");
        assert!(
            cache
                .stale()
                .unwrap()
                .script_block
                .contains(r#"{"org_name":"Cached"}"#)
        );
    }

    #[tokio::test]
    async fn ttl_is_measured_from_when_the_read_started() {
        let cache = PublicSettingsCache::new();
        tokio::time::pause();
        let started = Instant::now();
        tokio::time::advance(super::PUBLIC_SETTINGS_TTL - std::time::Duration::from_secs(1)).await;
        assert!(cache.store(cache.generation(), payload("Late"), started));
        assert!(
            cache.fresh().is_some(),
            "a slow read must not restart the TTL when it stores"
        );
        tokio::time::advance(std::time::Duration::from_secs(2)).await;
        assert!(
            cache.fresh().is_none(),
            "the blob expires 10s after the read started, not after it stored"
        );
        assert_eq!(cache.stale().unwrap().org_name, "Late");
    }

    #[tokio::test]
    async fn embed_failure_warning_is_rate_limited() {
        let cache = PublicSettingsCache::new();
        assert!(cache.note_embed_failure("settings read failed"));
        assert!(!cache.note_embed_failure("settings read failed again"));
    }

    #[tokio::test]
    async fn failed_refresh_suppresses_a_second_read_until_invalidate() {
        let cache = PublicSettingsCache::new();
        let generation = cache.generation();
        assert!(!cache.refresh_suppressed());
        cache.note_refresh_failure(generation);
        assert!(cache.refresh_suppressed());
        cache.invalidate();
        assert!(!cache.refresh_suppressed());
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

/// Parse `ALLOWED_ORIGINS`. Blank / whitespace-only (and empty after split)
/// is treated as unset and falls back to `redirect_base_url` (F-API-004).
///
/// Compose historically injects `ALLOWED_ORIGINS=` even when the host env
/// is unset (`${ALLOWED_ORIGINS:-}`), which used to yield `[""]` and 403
/// every browser WebSocket / CORS request.
pub fn parse_allowed_origins(raw: Option<&str>, redirect_base_url: &str) -> Vec<String> {
    let origins: Vec<String> = raw
        .unwrap_or("")
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect();
    if origins.is_empty() {
        vec![redirect_base_url.to_string()]
    } else {
        origins
    }
}

impl OAuthConfig {
    pub fn from_env() -> Self {
        let redirect_base_url = std::env::var("REDIRECT_BASE_URL")
            .unwrap_or_else(|_| "http://localhost:3000".to_string());

        let allowed_origins = parse_allowed_origins(
            std::env::var("ALLOWED_ORIGINS").ok().as_deref(),
            &redirect_base_url,
        );

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
