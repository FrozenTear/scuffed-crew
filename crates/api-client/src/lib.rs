use scuffed_types::MeResponse;
use serde::de::DeserializeOwned;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ClientError {
    #[error("{}", format_http_error(*status, body))]
    Http { status: u16, body: String },
    #[error("Network error: {0}")]
    Network(String),
    #[error("Deserialization error: {0}")]
    Deserialize(String),
}

/// Servers reply with `{"error": "..."}` bodies; surface that message so user-facing
/// toasts explain the failure instead of only showing the status code.
fn format_http_error(status: u16, body: &str) -> String {
    let message = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v.get("error")?.as_str().map(str::to_owned));
    match message {
        Some(msg) => format!("HTTP error {status}: {msg}"),
        None => format!("HTTP error: {status}"),
    }
}

/// Base URL for API requests.
/// In web mode, this is empty (same-origin). In native mode, it's configurable.
pub struct ApiClient {
    base_url: String,
    #[cfg(feature = "native")]
    token: Option<String>,
}

impl ApiClient {
    pub fn new(base_url: &str) -> Self {
        Self {
            base_url: base_url.to_string(),
            #[cfg(feature = "native")]
            token: None,
        }
    }

    /// Create a client for same-origin web requests.
    pub fn web() -> Self {
        Self::new("")
    }

    #[cfg(feature = "native")]
    pub fn with_token(mut self, token: String) -> Self {
        self.token = Some(token);
        self
    }

    // -- Typed convenience methods --

    pub async fn get_me(&self) -> Result<MeResponse, ClientError> {
        self.get("/api/auth/me").await
    }

    pub async fn logout(&self) -> Result<(), ClientError> {
        self.post_empty("/api/auth/logout").await
    }

    // -- Generic typed methods --

    pub async fn fetch<T: DeserializeOwned>(&self, path: &str) -> Result<T, ClientError> {
        self.get(path).await
    }

    pub async fn post_json<B: serde::Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T, ClientError> {
        self.do_post_json(path, body).await
    }

    pub async fn post_json_empty<B: serde::Serialize>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<(), ClientError> {
        let _: serde_json::Value = self.do_post_json(path, body).await?;
        Ok(())
    }

    pub async fn put_json<B: serde::Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T, ClientError> {
        self.do_put_json(path, body).await
    }

    pub async fn put_json_empty<B: serde::Serialize>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<(), ClientError> {
        let _: serde_json::Value = self.do_put_json(path, body).await?;
        Ok(())
    }

    pub async fn patch_json<B: serde::Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T, ClientError> {
        self.do_patch_json(path, body).await
    }

    pub async fn patch_json_empty<B: serde::Serialize>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<(), ClientError> {
        let _: serde_json::Value = self.do_patch_json(path, body).await?;
        Ok(())
    }

    pub async fn delete(&self, path: &str) -> Result<(), ClientError> {
        self.do_delete(path).await
    }
}

// Platform-specific implementations
#[cfg(feature = "web")]
mod web_impl;

#[cfg(feature = "native")]
mod native_impl;

// Shared dispatch to platform implementations
impl ApiClient {
    #[cfg(feature = "web")]
    async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, ClientError> {
        web_impl::get(&self.base_url, path).await
    }
    #[cfg(feature = "web")]
    async fn post_empty(&self, path: &str) -> Result<(), ClientError> {
        web_impl::post_empty(&self.base_url, path).await
    }
    #[cfg(feature = "web")]
    async fn do_post_json<B: serde::Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T, ClientError> {
        web_impl::post_json(&self.base_url, path, body).await
    }
    #[cfg(feature = "web")]
    async fn do_put_json<B: serde::Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T, ClientError> {
        web_impl::put_json(&self.base_url, path, body).await
    }
    #[cfg(feature = "web")]
    async fn do_patch_json<B: serde::Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T, ClientError> {
        web_impl::patch_json(&self.base_url, path, body).await
    }
    #[cfg(feature = "web")]
    async fn do_delete(&self, path: &str) -> Result<(), ClientError> {
        web_impl::delete(&self.base_url, path).await
    }

    #[cfg(feature = "native")]
    async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, ClientError> {
        native_impl::get(&self.base_url, path, self.token.as_deref()).await
    }
    #[cfg(feature = "native")]
    async fn post_empty(&self, path: &str) -> Result<(), ClientError> {
        native_impl::post_empty(&self.base_url, path, self.token.as_deref()).await
    }
    #[cfg(feature = "native")]
    async fn do_post_json<B: serde::Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T, ClientError> {
        native_impl::post_json(&self.base_url, path, body, self.token.as_deref()).await
    }
    #[cfg(feature = "native")]
    async fn do_put_json<B: serde::Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T, ClientError> {
        native_impl::put_json(&self.base_url, path, body, self.token.as_deref()).await
    }
    #[cfg(feature = "native")]
    async fn do_patch_json<B: serde::Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T, ClientError> {
        native_impl::patch_json(&self.base_url, path, body, self.token.as_deref()).await
    }
    #[cfg(feature = "native")]
    async fn do_delete(&self, path: &str) -> Result<(), ClientError> {
        native_impl::delete(&self.base_url, path, self.token.as_deref()).await
    }
}

#[cfg(test)]
mod tests {
    use super::format_http_error;

    #[test]
    fn http_error_includes_server_message() {
        assert_eq!(
            format_http_error(400, r#"{"error":"Member already has this role"}"#),
            "HTTP error 400: Member already has this role"
        );
    }

    #[test]
    fn http_error_falls_back_without_message() {
        assert_eq!(format_http_error(400, ""), "HTTP error: 400");
        assert_eq!(format_http_error(500, "not json"), "HTTP error: 500");
        assert_eq!(
            format_http_error(404, r#"{"detail":"x"}"#),
            "HTTP error: 404"
        );
    }
}
