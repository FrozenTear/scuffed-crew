//! Per-IP limits for `GET /api/tracker/packs` and `/api/tracker/packs/{name}`.
//!
//! One place so the server limiter and the tracker's download code agree.

/// Requests one client IP may send to the pack routes before waiting.
///
/// A first install fetches the list and then every pack, so this must stay
/// above the shipped pack count plus one. 16 leaves room for new packs and a
/// second PC on the same home IP without a 429 on first install.
pub const PACK_IP_BURST: u32 = 16;

/// Seconds until the pack per-IP governor replenishes one request.
pub const PACK_IP_REPLENISH_SECS: u64 = 10;
