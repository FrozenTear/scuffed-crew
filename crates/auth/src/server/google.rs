use serde::Deserialize;

use super::oauth::{OAuthProvider, ProviderConfig};
use crate::AuthProvider;

/// Google user info from API
#[derive(Debug, Deserialize)]
pub struct GoogleUser {
    pub id: String,
    pub name: String,
    pub email: Option<String>,
    pub picture: Option<String>,
}

/// Google OAuth provider
pub struct GoogleProvider {
    pub client_id: String,
    pub client_secret: String,
    pub redirect_base_url: String,
}

impl OAuthProvider for GoogleProvider {
    type UserInfo = GoogleUser;

    fn provider(&self) -> AuthProvider {
        AuthProvider::Google
    }

    fn config(&self) -> ProviderConfig {
        ProviderConfig {
            auth_url: "https://accounts.google.com/o/oauth2/v2/auth",
            token_url: "https://oauth2.googleapis.com/token",
            user_info_url: "https://www.googleapis.com/oauth2/v2/userinfo",
            client_id: self.client_id.clone(),
            client_secret: self.client_secret.clone(),
            redirect_url: format!("{}/api/auth/google/callback", self.redirect_base_url),
            scopes: vec!["profile".to_string(), "email".to_string()],
        }
    }

    fn username(info: &GoogleUser) -> String {
        info.name.clone()
    }

    fn provider_id(info: &GoogleUser) -> String {
        info.id.clone()
    }

    fn avatar_url(info: &GoogleUser) -> Option<String> {
        info.picture.clone()
    }
}
