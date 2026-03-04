use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModerationActionType {
    Note,
    Warning,
    Suspension,
    Ban,
}

impl std::fmt::Display for ModerationActionType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ModerationActionType::Note => write!(f, "note"),
            ModerationActionType::Warning => write!(f, "warning"),
            ModerationActionType::Suspension => write!(f, "suspension"),
            ModerationActionType::Ban => write!(f, "ban"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModerationAction {
    pub id: String,
    pub member_id: String,
    pub action_type: ModerationActionType,
    pub reason: String,
    pub issued_by: String,
    pub expires_at: Option<DateTime<Utc>>,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
}
