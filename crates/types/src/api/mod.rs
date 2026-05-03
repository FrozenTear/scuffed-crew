pub mod announcements;
pub mod applications;
pub mod articles;
pub mod events;
pub mod games;
pub mod matches;
pub mod members;
pub mod moderation;
pub mod settings;
pub mod teams;
pub mod tournaments;

pub use announcements::*;
pub use applications::*;
pub use articles::*;
pub use events::*;
pub use games::*;
pub use matches::*;
pub use members::*;
pub use moderation::*;
pub use settings::*;
pub use teams::*;
pub use tournaments::*;

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
