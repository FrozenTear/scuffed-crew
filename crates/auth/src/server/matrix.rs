use serde::Deserialize;

use super::oauth::{OAuthProvider, ProviderConfig};
use crate::AuthProvider;

/// Matrix user info from MAS (Matrix Authentication Service) OIDC
///
/// TODO: Flesh out when MAS/OIDC integration is ready.
/// Matrix uses OpenID Connect via MAS, so the flow is similar to
/// Google/Discord but with a self-hosted identity provider.
#[derive(Debug, Deserialize)]
pub struct MatrixUser {
    pub sub: String,
    pub name: Option<String>,
    pub preferred_username: Option<String>,
    pub picture: Option<String>,
}

/// Matrix OAuth/OIDC provider (stub)
pub struct MatrixProvider {
    pub client_id: String,
    pub client_secret: String,
    pub issuer_url: String,
    pub redirect_base_url: String,
}

impl OAuthProvider for MatrixProvider {
    type UserInfo = MatrixUser;

    fn provider(&self) -> AuthProvider {
        AuthProvider::Matrix
    }

    fn config(&self) -> ProviderConfig {
        // MAS exposes standard OIDC endpoints under the issuer URL
        // These will be populated from discovery in the full implementation
        ProviderConfig {
            auth_url: "https://example.com/authorize", // TODO: from OIDC discovery
            token_url: "https://example.com/token",    // TODO: from OIDC discovery
            user_info_url: "https://example.com/userinfo", // TODO: from OIDC discovery
            client_id: self.client_id.clone(),
            client_secret: self.client_secret.clone(),
            redirect_url: format!("{}/api/auth/matrix/callback", self.redirect_base_url),
            scopes: vec!["openid".to_string(), "profile".to_string()],
        }
    }

    fn username(info: &MatrixUser) -> String {
        info.preferred_username
            .clone()
            .or_else(|| info.name.clone())
            .unwrap_or_else(|| info.sub.clone())
    }

    fn provider_id(info: &MatrixUser) -> String {
        info.sub.clone()
    }

    fn avatar_url(info: &MatrixUser) -> Option<String> {
        info.picture.clone()
    }
}
