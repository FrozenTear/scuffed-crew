use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Authentication provider
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuthProvider {
    Discord,
    Google,
    Matrix,
    Local,
    /// Self-sovereign Nostr key (NIP-07 challenge login). provider_id = pubkey hex.
    Nostr,
}

impl std::fmt::Display for AuthProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuthProvider::Discord => write!(f, "discord"),
            AuthProvider::Google => write!(f, "google"),
            AuthProvider::Matrix => write!(f, "matrix"),
            AuthProvider::Local => write!(f, "local"),
            AuthProvider::Nostr => write!(f, "nostr"),
        }
    }
}

/// User ID type alias for clarity
pub type UserId = String;

/// User profile from OAuth
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: UserId,
    pub provider: AuthProvider,
    pub provider_id: String,
    pub username: String,
    pub avatar_url: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl User {
    pub fn new(
        provider: AuthProvider,
        provider_id: String,
        username: String,
        avatar_url: Option<String>,
    ) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            provider,
            provider_id,
            username,
            avatar_url,
            created_at: Utc::now(),
        }
    }
}

/// Minimal user info for display (e.g., in collaboration)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserInfo {
    pub id: UserId,
    pub username: String,
    pub avatar_url: Option<String>,
}

impl From<User> for UserInfo {
    fn from(user: User) -> Self {
        Self {
            id: user.id,
            username: user.username,
            avatar_url: user.avatar_url,
        }
    }
}

impl From<&User> for UserInfo {
    fn from(user: &User) -> Self {
        Self {
            id: user.id.clone(),
            username: user.username.clone(),
            avatar_url: user.avatar_url.clone(),
        }
    }
}

/// Session cookie configuration, parameterized per-app
#[derive(Debug, Clone)]
pub struct SessionConfig {
    /// Cookie name for the session token (e.g. "sc_session" or "ow_session")
    pub cookie_name: String,
    /// Cookie name for CSRF state during OAuth flow
    pub csrf_cookie_name: String,
    /// Session duration in hours (default: 168 = 1 week)
    pub duration_hours: i64,
    /// CSRF cookie max age in minutes (default: 10)
    pub csrf_max_age_minutes: i64,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            cookie_name: "sc_session".to_string(),
            csrf_cookie_name: "sc_oauth_state".to_string(),
            duration_hours: 168,
            csrf_max_age_minutes: 10,
        }
    }
}

/// Auth-related errors
#[derive(Debug, Error)]
pub enum AuthError {
    #[error("OAuth token exchange failed: {0}")]
    TokenExchange(String),
    #[error("Failed to fetch user info: {0}")]
    UserInfoFetch(String),
    #[error("CSRF validation failed: {0}")]
    CsrfInvalid(String),
    #[error("Session not found or expired")]
    SessionNotFound,
    #[error("Database error: {0}")]
    Database(String),
    #[error("Internal error: {0}")]
    Internal(String),
}
