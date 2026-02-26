use serde::Deserialize;

use crate::AuthProvider;
use super::oauth::{OAuthProvider, ProviderConfig};

/// Discord user info from API
#[derive(Debug, Deserialize)]
pub struct DiscordUser {
    pub id: String,
    pub username: String,
    pub discriminator: String,
    pub avatar: Option<String>,
    pub global_name: Option<String>,
}

impl DiscordUser {
    pub fn avatar_url(&self) -> Option<String> {
        self.avatar.as_ref().map(|hash| {
            format!(
                "https://cdn.discordapp.com/avatars/{}/{}.png",
                self.id, hash
            )
        })
    }
}

/// Discord OAuth provider
pub struct DiscordProvider {
    pub client_id: String,
    pub client_secret: String,
    pub redirect_base_url: String,
}

impl OAuthProvider for DiscordProvider {
    type UserInfo = DiscordUser;

    fn provider(&self) -> AuthProvider {
        AuthProvider::Discord
    }

    fn config(&self) -> ProviderConfig {
        ProviderConfig {
            auth_url: "https://discord.com/api/oauth2/authorize",
            token_url: "https://discord.com/api/oauth2/token",
            user_info_url: "https://discord.com/api/users/@me",
            client_id: self.client_id.clone(),
            client_secret: self.client_secret.clone(),
            redirect_url: format!("{}/api/auth/discord/callback", self.redirect_base_url),
            scopes: vec!["identify".to_string()],
        }
    }

    fn username(info: &DiscordUser) -> String {
        info.global_name
            .clone()
            .unwrap_or_else(|| info.username.clone())
    }

    fn provider_id(info: &DiscordUser) -> String {
        info.id.clone()
    }

    fn avatar_url(info: &DiscordUser) -> Option<String> {
        info.avatar_url()
    }
}
