use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ApplicationStatus {
    Pending,
    Trial,
    Accepted,
    Rejected,
    Withdrawn,
}

impl std::fmt::Display for ApplicationStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApplicationStatus::Pending => write!(f, "pending"),
            ApplicationStatus::Trial => write!(f, "trial"),
            ApplicationStatus::Accepted => write!(f, "accepted"),
            ApplicationStatus::Rejected => write!(f, "rejected"),
            ApplicationStatus::Withdrawn => write!(f, "withdrawn"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Application {
    pub id: String,
    pub user_id: String,
    pub status: ApplicationStatus,
    pub preferred_games: Vec<String>,
    pub preferred_roles: Vec<String>,
    pub message: Option<String>,
    pub reviewed_by: Option<String>,
    pub review_notes: Option<String>,
    pub trial_started_at: Option<DateTime<Utc>>,
    pub trial_ends_at: Option<DateTime<Utc>>,
    pub mentor_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
