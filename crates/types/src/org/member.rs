use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OrgRole {
    Admin,
    Officer,
    Member,
    Recruit,
}

impl OrgRole {
    pub fn is_at_least(&self, required: OrgRole) -> bool {
        self.level() >= required.level()
    }

    fn level(&self) -> u8 {
        match self {
            OrgRole::Admin => 3,
            OrgRole::Officer => 2,
            OrgRole::Member => 1,
            OrgRole::Recruit => 0,
        }
    }
}

impl std::fmt::Display for OrgRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OrgRole::Admin => write!(f, "admin"),
            OrgRole::Officer => write!(f, "officer"),
            OrgRole::Member => write!(f, "member"),
            OrgRole::Recruit => write!(f, "recruit"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Member {
    pub id: String,
    pub user_id: String,
    pub org_role: OrgRole,
    pub display_name: String,
    pub bio: Option<String>,
    pub avatar_url: Option<String>,
    pub timezone: Option<String>,
    pub pronouns: Option<String>,
    pub availability_status: Option<String>,
    pub joined_at: DateTime<Utc>,
    pub is_active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameAccount {
    pub id: String,
    pub member_id: String,
    pub game_id: String,
    pub account_name: String,
    pub account_id: Option<String>,
    pub created_at: DateTime<Utc>,
}
