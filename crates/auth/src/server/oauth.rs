use oauth2::{
    basic::BasicClient, AuthUrl, AuthorizationCode, ClientId, ClientSecret, CsrfToken,
    RedirectUrl, Scope, TokenResponse, TokenUrl,
};
use serde::de::DeserializeOwned;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;

use crate::{AuthError, AuthProvider};

/// Common OAuth provider configuration
#[derive(Clone)]
pub struct ProviderConfig {
    pub auth_url: &'static str,
    pub token_url: &'static str,
    pub user_info_url: &'static str,
    pub client_id: String,
    pub client_secret: String,
    pub redirect_url: String,
    pub scopes: Vec<String>,
}

/// Trait that each OAuth provider implements
pub trait OAuthProvider: Send + Sync + 'static {
    /// The provider-specific user info type returned from the API
    type UserInfo: DeserializeOwned + Send;

    /// Which AuthProvider variant this is
    fn provider(&self) -> AuthProvider;

    /// Build the ProviderConfig for this provider
    fn config(&self) -> ProviderConfig;

    /// Extract a display username from the provider-specific user info
    fn username(info: &Self::UserInfo) -> String;

    /// Extract a provider-specific user ID from the provider-specific user info
    fn provider_id(info: &Self::UserInfo) -> String;

    /// Extract an avatar URL from the provider-specific user info
    fn avatar_url(info: &Self::UserInfo) -> Option<String>;

    /// Get the authorization URL and CSRF token
    fn get_auth_url(&self) -> (String, CsrfToken) {
        get_auth_url(&self.config())
    }

    /// Exchange an authorization code for an access token
    fn exchange_code(
        &self,
        code: &str,
    ) -> Pin<Box<dyn Future<Output = Result<String, AuthError>> + Send + '_>> {
        let config = self.config();
        let code = code.to_string();
        Box::pin(async move { exchange_code(&config, &code).await })
    }

    /// Fetch user info from the provider's API
    fn get_user_info(
        &self,
        access_token: &str,
    ) -> Pin<Box<dyn Future<Output = Result<Self::UserInfo, AuthError>> + Send + '_>> {
        let url = self.config().user_info_url;
        let token = access_token.to_string();
        Box::pin(async move { get_user_info(url, &token).await })
    }
}

/// Registry that holds all configured OAuth providers
pub struct ProviderRegistry {
    configs: HashMap<AuthProvider, ProviderConfig>,
}

impl ProviderRegistry {
    pub fn new() -> Self {
        Self {
            configs: HashMap::new(),
        }
    }

    pub fn register(&mut self, provider: AuthProvider, config: ProviderConfig) {
        self.configs.insert(provider, config);
    }

    pub fn get(&self, provider: &AuthProvider) -> Option<&ProviderConfig> {
        self.configs.get(provider)
    }

    pub fn providers(&self) -> Vec<AuthProvider> {
        self.configs.keys().copied().collect()
    }
}

impl Default for ProviderRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Create an OAuth2 client from provider config
fn create_client(config: &ProviderConfig) -> BasicClient {
    BasicClient::new(
        ClientId::new(config.client_id.clone()),
        Some(ClientSecret::new(config.client_secret.clone())),
        AuthUrl::new(config.auth_url.to_string()).expect("Invalid auth URL (this is a bug)"),
        Some(
            TokenUrl::new(config.token_url.to_string())
                .expect("Invalid token URL (this is a bug)"),
        ),
    )
    .set_redirect_uri(
        RedirectUrl::new(config.redirect_url.clone()).unwrap_or_else(|_| {
            panic!(
                "Invalid redirect URL configuration: '{}' is not a valid URL",
                config.redirect_url
            )
        }),
    )
}

/// Get authorization URL for any OAuth provider
pub fn get_auth_url(config: &ProviderConfig) -> (String, CsrfToken) {
    let client = create_client(config);

    let mut builder = client.authorize_url(CsrfToken::new_random);
    for scope in &config.scopes {
        builder = builder.add_scope(Scope::new(scope.clone()));
    }
    let (auth_url, csrf_token) = builder.url();

    (auth_url.to_string(), csrf_token)
}

/// Exchange authorization code for access token
pub async fn exchange_code(config: &ProviderConfig, code: &str) -> Result<String, AuthError> {
    let client = create_client(config);

    let token_result = client
        .exchange_code(AuthorizationCode::new(code.to_string()))
        .request_async(oauth2::reqwest::async_http_client)
        .await
        .map_err(|e| AuthError::TokenExchange(e.to_string()))?;

    Ok(token_result.access_token().secret().clone())
}

/// Fetch user info from the provider's API
pub async fn get_user_info<T: DeserializeOwned>(
    user_info_url: &'static str,
    access_token: &str,
) -> Result<T, AuthError> {
    let client = reqwest::Client::new();

    let response = client
        .get(user_info_url)
        .header("Authorization", format!("Bearer {}", access_token))
        .send()
        .await
        .map_err(|e| AuthError::UserInfoFetch(e.to_string()))?;

    if !response.status().is_success() {
        return Err(AuthError::UserInfoFetch(format!(
            "OAuth API error: {}",
            response.status()
        )));
    }

    response
        .json()
        .await
        .map_err(|e| AuthError::UserInfoFetch(e.to_string()))
}
