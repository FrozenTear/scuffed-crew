//! Device-code calls for stat-tracker sign-in.
//!
//! Matches the link routes: `POST /api/link/lookup`, `/approve`, and `/deny`.
//! Each posts JSON `{ "user_code": "..." }`. Lookup returns `device_label`,
//! `app_version`, and `created_at`. Approve and deny return `{ "ok": true }`.
//!
//! Those JSON shapes are private to the server crate. Nothing was added to
//! `scuffed_types`, so the structs below mirror the wire format.
//!
//! Session POSTs go through [`scuffed_api_client::ApiClient::web`], which sets
//! same-origin mode and same-origin credentials (the session cookie). The link
//! routes do not check a CSRF header, and this module does not add one.
//! [`EXTRA_LINK_HEADERS`] is the extra header list. It stays empty.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use scuffed_api_client::{ApiClient, ObservedError};

use super::normalize_user_code;

pub const LOOKUP_PATH: &str = "/api/link/lookup";
pub const APPROVE_PATH: &str = "/api/link/approve";
pub const DENY_PATH: &str = "/api/link/deny";

/// Headers this module adds on top of the shared same-origin session POST.
/// Empty: there is no CSRF token to send.
pub const EXTRA_LINK_HEADERS: &[(&str, &str)] = &[];

/// Body for lookup, approve, and deny.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct UserCodeRequest {
    pub user_code: String,
}

impl UserCodeRequest {
    pub fn from_raw(raw: &str) -> Self {
        Self {
            user_code: normalize_user_code(raw),
        }
    }
}

/// A pending tracker sign-in waiting for this member.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct PendingDeviceCode {
    pub device_label: String,
    pub app_version: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct LinkOk {
    pub ok: bool,
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
    body: &UserCodeRequest,
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

pub async fn lookup_pending_code(raw_code: &str) -> Result<PendingDeviceCode, LinkCallError> {
    post_link(LOOKUP_PATH, &UserCodeRequest::from_raw(raw_code)).await
}

pub async fn approve_pending_code(raw_code: &str) -> Result<(), LinkCallError> {
    let response: LinkOk = post_link(APPROVE_PATH, &UserCodeRequest::from_raw(raw_code)).await?;
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
    let response: LinkOk = post_link(DENY_PATH, &UserCodeRequest::from_raw(raw_code)).await?;
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
        let parsed: LinkOk = serde_json::from_str(r#"{"ok":true}"#).expect("ok body");
        assert!(parsed.ok);
    }
}
