//! Device-code calls for stat-tracker sign-in.
//!
//! The API is landing in parallel. These paths and field names are the
//! contract this page codes against. They may change to match that PR:
//!
//! - `POST /api/link/lookup`
//! - `POST /api/link/approve`
//! - `POST /api/link/deny`
//!
//! Every call posts JSON `{ "user_code": "<normalised code>" }`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use scuffed_api_client::{ApiClient, ClientError};

use super::normalize_user_code;

pub const LOOKUP_PATH: &str = "/api/link/lookup";
pub const APPROVE_PATH: &str = "/api/link/approve";
pub const DENY_PATH: &str = "/api/link/deny";

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

pub async fn lookup_pending_code(raw_code: &str) -> Result<PendingDeviceCode, ClientError> {
    let body = UserCodeRequest::from_raw(raw_code);
    ApiClient::web().post_json(LOOKUP_PATH, &body).await
}

pub async fn approve_pending_code(raw_code: &str) -> Result<(), ClientError> {
    let body = UserCodeRequest::from_raw(raw_code);
    ApiClient::web().post_json_empty(APPROVE_PATH, &body).await
}

pub async fn deny_pending_code(raw_code: &str) -> Result<(), ClientError> {
    let body = UserCodeRequest::from_raw(raw_code);
    ApiClient::web().post_json_empty(DENY_PATH, &body).await
}
