pub mod announcements;
pub mod applications;
pub mod articles;
pub mod chat;
pub mod events;
pub mod games;
pub mod matches;
pub mod members;
pub mod moderation;
pub mod seasons;
pub mod settings;
pub mod stats;
pub mod teams;
pub mod tournaments;

pub use announcements::*;
pub use applications::*;
pub use articles::*;
pub use chat::*;
pub use events::*;
pub use games::*;
pub use matches::*;
pub use members::*;
pub use moderation::*;
pub use seasons::*;
pub use settings::*;
pub use stats::*;
pub use teams::*;
pub use tournaments::*;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ApiError {
    pub error: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
}

/// Seconds from a governor JSON body's `retry_after`.
///
/// `{"error":"rate_limited","retry_after":N}` yields `N` when `N` is a JSON
/// number or a numeric string in `1..=3600`. Any other body is `None`,
/// including plain text, password-lockout JSON, negatives, and waits over an hour.
pub fn json_retry_after(body: &str) -> Option<u64> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    if value.get("error").and_then(|error| error.as_str()) != Some("rate_limited") {
        return None;
    }
    match value.get("retry_after")? {
        serde_json::Value::Number(number) => number
            .as_u64()
            .filter(|seconds| (1..=3600).contains(seconds)),
        serde_json::Value::String(text) => text
            .trim()
            .parse::<u64>()
            .ok()
            .filter(|seconds| (1..=3600).contains(seconds)),
        _ => None,
    }
}

/// Member copy for a tower-governor 429.
///
/// The body `{"error":"rate_limited","retry_after":N}` becomes
/// `Try again in N s`. Any other body returns `None`, including plain text
/// (`Too Many Requests! Wait for Ns`) and password-lockout JSON
/// (`{"error":"too many login attempts"}`), so callers keep their existing copy.
pub fn rate_limited_retry_message(status: u16, body: &str) -> Option<String> {
    if status != 429 {
        return None;
    }
    let secs = json_retry_after(body)?;
    Some(format!("Try again in {secs} s"))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ApiSuccess<T> {
    pub data: T,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PaginatedResponse<T> {
    pub data: Vec<T>,
    pub total: u64,
    pub page: u32,
    pub per_page: u32,
}

/// Query parameters for cursor-based pagination.
#[derive(Clone, Debug, Deserialize)]
pub struct PaginationParams {
    /// Opaque cursor from a previous response's `next_cursor`.
    pub cursor: Option<String>,
    /// Number of items per page (default 25, max 100).
    #[serde(default = "default_pagination_limit")]
    pub limit: u32,
}

fn default_pagination_limit() -> u32 {
    25
}

impl PaginationParams {
    /// Returns clamped limit (1..=100) and decoded offset.
    pub fn resolve(&self) -> (u32, u32) {
        let limit = self.limit.clamp(1, 100);
        let offset = self.cursor.as_deref().and_then(decode_cursor).unwrap_or(0);
        (limit, offset)
    }
}

/// Cursor-paginated response wrapper.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CursorResponse<T> {
    pub data: Vec<T>,
    /// Opaque cursor for fetching the next page. `None` means no more pages.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

impl<T> CursorResponse<T> {
    /// Build a CursorResponse from a vec fetched with limit+1 strategy.
    /// Pass the actual limit requested (not limit+1).
    pub fn from_oversized(mut items: Vec<T>, limit: u32, offset: u32) -> Self {
        let has_more = items.len() as u32 > limit;
        if has_more {
            items.truncate(limit as usize);
        }
        let next_cursor = if has_more {
            Some(encode_cursor(offset + limit))
        } else {
            None
        };
        CursorResponse {
            data: items,
            next_cursor,
        }
    }
}

fn encode_cursor(offset: u32) -> String {
    format!("{offset:08x}")
}

fn decode_cursor(s: &str) -> Option<u32> {
    u32::from_str_radix(s, 16).ok()
}

#[cfg(test)]
mod tests {
    use super::{json_retry_after, rate_limited_retry_message};

    #[test]
    fn json_retry_after_reads_number_or_numeric_string() {
        assert_eq!(
            json_retry_after(r#"{"error":"rate_limited","retry_after":12}"#),
            Some(12)
        );
        assert_eq!(
            json_retry_after(r#"{"error":"rate_limited","retry_after":" 4 "}"#),
            Some(4)
        );
        assert_eq!(
            json_retry_after(r#"{"error":"rate_limited","retry_after":0}"#),
            None
        );
        assert_eq!(
            json_retry_after(r#"{"error":"rate_limited","retry_after":"0"}"#),
            None
        );
        assert_eq!(json_retry_after(r#"{"error":"rate_limited"}"#), None);
        assert_eq!(
            json_retry_after(r#"{"error":"too many login attempts","retry_after":9}"#),
            None
        );
        assert_eq!(json_retry_after("Too Many Requests! Wait for 9s"), None);
        assert_eq!(
            json_retry_after(r#"{"error":"rate_limited","retry_after":3600}"#),
            Some(3600)
        );
        assert_eq!(
            json_retry_after(r#"{"error":"rate_limited","retry_after":3601}"#),
            None
        );
        assert_eq!(
            json_retry_after(r#"{"error":"rate_limited","retry_after":"3601"}"#),
            None
        );
        assert_eq!(
            json_retry_after(r#"{"error":"rate_limited","retry_after":-1}"#),
            None
        );
        assert_eq!(
            json_retry_after(r#"{"error":"rate_limited","retry_after":"-5"}"#),
            None
        );
        assert_eq!(json_retry_after("[]"), None);
        assert_eq!(json_retry_after("null"), None);
    }

    #[test]
    fn governor_json_429_uses_retry_after() {
        assert_eq!(
            rate_limited_retry_message(429, r#"{"error":"rate_limited","retry_after":9}"#)
                .as_deref(),
            Some("Try again in 9 s")
        );
        assert_eq!(
            rate_limited_retry_message(429, r#"{"error":"rate_limited","retry_after":1}"#)
                .as_deref(),
            Some("Try again in 1 s")
        );
    }

    #[test]
    fn plain_text_and_lockout_429_are_not_governor_json() {
        assert_eq!(
            rate_limited_retry_message(429, "Too Many Requests! Wait for 9s"),
            None
        );
        assert_eq!(
            rate_limited_retry_message(429, r#"{"error":"too many login attempts"}"#),
            None
        );
        assert_eq!(
            rate_limited_retry_message(400, r#"{"error":"rate_limited","retry_after":9}"#),
            None
        );
        assert_eq!(
            rate_limited_retry_message(429, r#"{"error":"rate_limited"}"#),
            None
        );
    }
}
