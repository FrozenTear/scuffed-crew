//! Upload of local rows to the site.
//!
//! - `wire`: `PersonalMatch` to the site's upload entry.
//! - `errors`: [`SyncUploadError`] and error-body parsing.
//! - `backoff`: the periodic sync clock.
//! - `batching`: the 413 caps and the 400/422 bisection.
//! - `auth_pause`: `sync_auth.json` and the GUI status text.
//! - `visibility`: the member's stats visibility, parsed from the site.
//! - `client`: [`SyncClient`], the only reqwest code.
//!
//! Every public item is re-exported here, so `crate::sync::X` paths are
//! unchanged.

mod auth_pause;
mod backoff;
mod batching;
mod client;
mod errors;
mod visibility;
mod wire;

pub use crate::version::USER_AGENT;

pub use auth_pause::{
    AuthPauseReason, SYNC_STATS_DELETE_PENDING_STATUS, SYNC_TOKEN_EXPIRED_STATUS,
    SYNC_TOKEN_REJECTED_STATUS, auth_pause_matches, auth_pause_reason, auth_pause_status,
    clear_auth_pause, delete_pending_status, format_local_date, write_auth_pause,
    write_auth_pause_reason, write_delete_pending_pause,
};
pub use backoff::{
    SYNC_BACKOFF_BASE, SYNC_BACKOFF_CAP, SyncAttempt, SyncBackoff, parse_retry_after,
    sync_backoff_delay,
};
pub use batching::{
    BatchCaps, RejectedUpload, SERVER_REFUSING_UPLOADS, SYNC_UPLOAD_MAX_DELETES,
    SYNC_UPLOAD_MAX_MATCHES, UploadIsolation, isolate_rejected, isolation_request_budget,
    lower_batch_limits, payload_names_a_row,
};
pub use client::{
    REDIRECT_OFF_SITE, REDIRECT_OFF_SITE_ABOVE, SyncClient, WebsiteUrlField, is_off_site_redirect,
    off_site_redirect_message, token_safe_redirects,
};
pub use errors::{SyncUploadError, server_error_text};
pub use visibility::{StatsVisibility, parse_stats_visibility};
pub use wire::{row_needs_review, upload_request};

// Names the test files saw through `use super::*` before the split: two of
// the old file's imports and two private items.
#[cfg(test)]
use crate::config::SyncConfig;
#[cfg(test)]
use crate::storage::PersonalMatch;
#[cfg(test)]
use auth_pause::AUTH_PAUSE_FILE;
#[cfg(test)]
use errors::parse_upload_error_body;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod tests_wire;

#[cfg(test)]
mod tests_http;

#[cfg(test)]
mod tests_backoff;

#[cfg(test)]
mod tests_auth_pause;

#[cfg(test)]
mod tests_batching;

#[cfg(test)]
mod tests_visibility;
