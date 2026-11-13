//! Upload of local rows to the site.
//!
//! - `wire`: `PersonalMatch` to the site's upload entry.
//! - `errors`: [`SyncUploadError`] and error-body parsing.
//! - `backoff`: the periodic sync clock.
//! - `batching`: the 413 caps and the 400/422 bisection.
//! - `auth_pause`: `sync_auth.json` and the GUI status text.
//! - `visibility`: the member's stats visibility, parsed from the site.
//! - `client`: [`SyncClient`] and its redirect policy (reqwest).
//! - `training`, `training_store`, `training_client`, `training_pass`,
//!   `training_collect`: opt-in training crops (kinds and caps, the outbox,
//!   the routes, one send pass, the Tab crops the daemon offers).
//!   `training_client` is the other reqwest file.
//!
//! Every public item is re-exported here, so `crate::sync::X` paths are
//! unchanged.

mod auth_pause;
mod backoff;
mod bans_picks;
mod batching;
mod client;
mod errors;
mod training;
mod training_client;
mod training_collect;
mod training_pass;
mod training_store;
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
    ACCEPT_PROOF_WINDOW, BatchCaps, RefusedRows, RejectedUpload, SERVER_REFUSING_UPLOADS,
    SYNC_UPLOAD_MAX_DELETES, SYNC_UPLOAD_MAX_MATCHES, UploadIsolation, isolate_rejected,
    isolate_rejected_in_pass, isolation_request_budget, lower_batch_limits, payload_names_a_row,
};
pub use client::{
    REDIRECT_OFF_SITE, REDIRECT_OFF_SITE_ABOVE, SyncClient, WebsiteUrlField, is_off_site_redirect,
    off_site_redirect_message, token_safe_redirects,
};
pub use errors::{SyncUploadError, server_error_text};
pub use training::{
    CropRefusal, QueuedCrop, STAT_DIGIT_COLUMNS, UploadKind, batch_len, check_crop, game_token_ok,
    hero_field, hero_icon_meta, map_name_meta, meta_json, multipart_body, png_size, random_token,
    stat_digits_meta,
};
pub use training_client::TrainingCallError;
pub use training_collect::{keep_tab_crops, training_kind_on, warn_crop_lost};
pub use training_pass::{TrainingPassReport, TrainingRetry, run_training_pass};
pub use training_store::{
    OUTBOX_KEEP_PER_KIND, TrainingStatus, adopt_training_owner, cached_training_consent,
    clear_training_outbox, drop_kind, forget_training_crops, game_token_for, has_queued_crops,
    keep_map_name_crop, keep_training_crop, mark_training_delete, queued_crops,
    read_training_status, record_training_delete_ok, training_warning,
};
pub use visibility::{StatsVisibility, parse_hero_icon_consent, parse_stats_visibility};
pub(crate) use wire::rank_update_changes_upload;
pub use wire::{UploadEntry, UploadRequest, row_needs_review, upload_request};

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
mod tests_bans_picks;

#[cfg(test)]
mod tests_queue_title;

#[cfg(test)]
mod tests_rank_update;

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

#[cfg(test)]
mod tests_training;

#[cfg(test)]
mod tests_training_collect;

#[cfg(test)]
mod tests_training_http;

#[cfg(test)]
mod tests_consent_fetch;
