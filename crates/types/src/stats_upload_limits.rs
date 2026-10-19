//! Numeric limits for `POST /api/stats/upload` and new daemon tokens.
//!
//! One place so the handler, the per-token governor, and token creation
//! cannot drift. The batch cap is off unless `STATS_UPLOAD_BATCH_CAP` is
//! enabled. The per-token governor and the 180-day expiry are always on.

/// Matches allowed in one upload when the batch cap is enabled.
pub const STATS_UPLOAD_MAX_MATCHES: usize = 200;

/// `deleted_sessions` allowed in one upload when the batch cap is enabled.
pub const STATS_UPLOAD_MAX_DELETED_SESSIONS: usize = 500;

/// Requests one daemon token may send before the upload governor waits.
pub const STATS_UPLOAD_TOKEN_BURST: u32 = 10;

/// Seconds until the upload governor replenishes one request for a token.
pub const STATS_UPLOAD_TOKEN_REPLENISH_SECS: u64 = 6;

/// Requests one client IP may send to `POST /api/stats/upload` before waiting.
///
/// Looser than [`STATS_UPLOAD_TOKEN_BURST`] times a few devices, so two PCs
/// on one home IP, or a short backlog of batches, hit the token limit first.
pub const STATS_UPLOAD_IP_BURST: u32 = 40;

/// Seconds until the stats-upload per-IP governor replenishes one request.
pub const STATS_UPLOAD_IP_REPLENISH_SECS: u64 = 2;

/// Lifetime of a daemon token created from now on, in days.
pub const DAEMON_TOKEN_TTL_DAYS: i64 = 180;
