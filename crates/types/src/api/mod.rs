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
