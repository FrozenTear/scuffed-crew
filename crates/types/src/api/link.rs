//! Device-link sign-in bodies shared by the site and the stat tracker.
//!
//! Start and poll are what the desktop app calls. They carry no browser
//! headers. Lookup, approve, and deny all take the short user code. Lookup
//! returns the device the member is about to approve. Approve and deny return
//! `{ok:true}` and never include the daemon token.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Body for `POST /api/link/start`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceLinkStartRequest {
    pub device_label: String,
    pub app_version: String,
}

/// `POST /api/link/start`.
///
/// `user_code` is the short code shown to the member (`XXXX-XXXX`).
/// `device_code` is the long secret the device polls with. Both are raw here
/// and only their hashes are stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceLinkStartResponse {
    pub user_code: String,
    pub device_code: String,
    pub interval: u64,
    pub expires_in: u64,
}

/// Body for `POST /api/link/poll`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceLinkPollRequest {
    pub device_code: String,
}

/// `POST /api/link/poll`.
///
/// `status` is `pending`, `slow_down`, `denied`, `expired`, or `approved`.
/// `token` is present only on the single approved handover.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceLinkPollResponse {
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
}

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
