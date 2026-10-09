//! Device-link sign-in bodies shared by the site and the stat tracker.
//!
//! Lookup, approve, and deny all take the short user code. Lookup returns the
//! device the member is about to approve. Approve and deny return `{ok:true}`
//! and never include the daemon token.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Body for `POST /api/link/lookup`, `/approve`, and `/deny`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceLinkUserCodeRequest {
    pub user_code: String,
}

/// `POST /api/link/lookup`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceLinkLookupResponse {
    pub device_label: String,
    pub app_version: String,
    pub created_at: DateTime<Utc>,
}

/// `POST /api/link/approve` and `POST /api/link/deny`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceLinkOkResponse {
    pub ok: bool,
}
