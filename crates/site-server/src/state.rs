use std::path::PathBuf;
use std::sync::Arc;

use scuffed_auth::crypto::CryptoService;
use scuffed_auth::server::HasAuth;
use scuffed_auth::{AuthError, SessionConfig, User};
use scuffed_db::Database;

use crate::dm_subscriber::DmEventBus;
use crate::notifications::MatrixNotifier;

/// Application state shared across handlers.
#[derive(Clone)]
pub struct AppState {
    pub db: Arc<Database>,
    pub session_config: SessionConfig,
    pub oauth_config: OAuthConfig,
    pub upload_dir: PathBuf,
    pub notifier: Option<MatrixNotifier>,
    /// 32-byte key for HMAC-signing Nostr challenge tokens.
    pub nostr_challenge_key: [u8; 32],
    /// Shared encryption service (same `Arc` as `db.crypto`).
    /// `None` when `ENCRYPTION_KEY` is not configured.
    pub crypto: Option<Arc<CryptoService>>,
    /// WebSocket URL for the Nostr relay (e.g., `ws://strfry:7777`).
    /// Used for publishing kind 0 profile metadata and NIP-05 relay hints.
    /// `None` when `NOSTR_RELAY_URL` is not set.
    pub relay_url: Option<String>,
    /// In-process event bus fed by the persistent DM relay subscriber.
    /// `None` when real-time delivery is disabled (no relay or no encryption
    /// configured); SSE handlers should treat that as a 503.
    pub dm_events: Option<DmEventBus>,
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
