pub mod members;
pub mod tournaments;
pub mod teams;
pub mod games;
pub mod announcements;
pub mod events;
pub mod moderation;
pub mod applications;
pub mod matches;
pub mod settings;
pub mod stats;

pub use members::*;
pub use tournaments::*;
pub use teams::*;
pub use games::*;
pub use announcements::*;
pub use events::*;
pub use moderation::*;
pub use applications::*;
pub use matches::*;
pub use settings::*;
pub use stats::*;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ApiError {
    pub error: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
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
        let offset = self
            .cursor
            .as_deref()
            .and_then(decode_cursor)
            .unwrap_or(0);
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
