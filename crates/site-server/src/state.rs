use std::sync::Arc;

use scuffed_auth::server::HasAuth;
use scuffed_auth::{AuthError, SessionConfig, User};
use scuffed_db::Database;

/// Application state shared across handlers.
#[derive(Clone)]
pub struct AppState {
    pub db: Arc<Database>,
    pub session_config: SessionConfig,
    pub oauth_config: OAuthConfig,
}

/// OAuth configuration loaded from environment.
#[derive(Clone)]
pub struct OAuthConfig {
    pub discord_client_id: String,
    pub discord_client_secret: String,
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

        if discord_client_id.is_empty() || discord_client_secret.is_empty() {
            tracing::warn!("Discord OAuth not configured — login disabled");
        }

        Self {
            discord_client_id,
            discord_client_secret,
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
        let user_id = self
            .db
            .get_session(token)
            .await
            .map_err(|e| AuthError::Database(e.to_string()))?;

        match user_id {
            Some(uid) => self
                .db
                .get_user(&uid)
                .await
                .map_err(|e| AuthError::Database(e.to_string())),
            None => Ok(None),
        }
    }
}
