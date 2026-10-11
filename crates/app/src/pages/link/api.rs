//! Device-code calls for stat-tracker sign-in.
//!
//! Matches the link routes: `POST /api/link/lookup`, `/approve`, and `/deny`.
//! Request and response bodies are the shared types in `scuffed_types`.
//!
//! Session POSTs go through [`scuffed_api_client::ApiClient::web`], which sets
//! same-origin mode and same-origin credentials (the session cookie). The
//! browser sends `Origin` on that POST. This module adds no CSRF header.
//! [`EXTRA_LINK_HEADERS`] is the extra header list. It stays empty.

use scuffed_api_client::{ApiClient, ObservedError};
use scuffed_types::{DeviceLinkLookupResponse, DeviceLinkOkResponse, DeviceLinkUserCodeRequest};

use super::normalize_user_code;

pub const LOOKUP_PATH: &str = "/api/link/lookup";
pub const APPROVE_PATH: &str = "/api/link/approve";
pub const DENY_PATH: &str = "/api/link/deny";

/// Headers this module adds on top of the shared same-origin session POST.
/// Empty: the browser already sends `Origin`, and there is no CSRF token.
pub const EXTRA_LINK_HEADERS: &[(&str, &str)] = &[];

fn user_code_request(raw: &str) -> DeviceLinkUserCodeRequest {
    DeviceLinkUserCodeRequest {
        user_code: normalize_user_code(raw),
    }
}

/// Failure from a link POST. `retry_after` is the raw `Retry-After` header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkCallError {
    pub status: Option<u16>,
    pub body: String,
    pub retry_after: Option<String>,
}

impl LinkCallError {
    fn from_observed(err: ObservedError) -> Self {
        match err {
            ObservedError::Http {
                status,
                body,
                retry_after,
            } => Self {
                status: Some(status),
                body,
                retry_after,
            },
            ObservedError::Network(message) | ObservedError::Deserialize(message) => Self {
                status: None,
                body: message,
                retry_after: None,
            },
        }
    }
}

async fn post_link<T: serde::de::DeserializeOwned>(
    path: &str,
    body: &DeviceLinkUserCodeRequest,
) -> Result<T, LinkCallError> {
    debug_assert!(
        EXTRA_LINK_HEADERS
            .iter()
            .all(|(name, _)| !name.to_ascii_lowercase().contains("csrf"))
    );
    ApiClient::web()
        .post_json_observed(path, body)
        .await
        .map_err(LinkCallError::from_observed)
}

pub async fn lookup_pending_code(
    raw_code: &str,
) -> Result<DeviceLinkLookupResponse, LinkCallError> {
    post_link(LOOKUP_PATH, &user_code_request(raw_code)).await
}

pub async fn approve_pending_code(raw_code: &str) -> Result<(), LinkCallError> {
    let response: DeviceLinkOkResponse =
        post_link(APPROVE_PATH, &user_code_request(raw_code)).await?;
    if response.ok {
        Ok(())
    } else {
        Err(LinkCallError {
            status: Some(200),
            body: String::new(),
            retry_after: None,
        })
    }
}

pub async fn deny_pending_code(raw_code: &str) -> Result<(), LinkCallError> {
    let response: DeviceLinkOkResponse = post_link(DENY_PATH, &user_code_request(raw_code)).await?;
    if response.ok {
        Ok(())
    } else {
        Err(LinkCallError {
            status: Some(200),
            body: String::new(),
            retry_after: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_post_adds_no_csrf_header() {
        assert!(EXTRA_LINK_HEADERS.is_empty());
        assert!(
            EXTRA_LINK_HEADERS
                .iter()
                .all(|(name, _)| !name.to_ascii_lowercase().contains("csrf"))
        );
    }

    #[test]
    fn approve_response_is_ok_true() {
        let parsed: DeviceLinkOkResponse = serde_json::from_str(r#"{"ok":true}"#).expect("ok body");
        assert!(parsed.ok);
    }
}
