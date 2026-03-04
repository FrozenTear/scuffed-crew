use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Game {
    pub id: String,
    pub name: String,
    pub abbreviation: Option<String>,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Team {
    pub id: String,
    pub name: String,
    pub game_id: String,
    pub color: Option<String>,
    pub division: Option<String>,
    pub lore_quote: Option<String>,
    pub logo_url: Option<String>,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TeamRole {
    Captain,
    Player,
    Sub,
    Coach,
}

impl std::fmt::Display for TeamRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TeamRole::Captain => write!(f, "captain"),
            TeamRole::Player => write!(f, "player"),
            TeamRole::Sub => write!(f, "sub"),
            TeamRole::Coach => write!(f, "coach"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RosterEntry {
    pub id: String,
    pub member_id: String,
    pub team_id: String,
    pub team_role: TeamRole,
    pub joined_at: DateTime<Utc>,
    pub is_active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamRecord {
    pub wins: u32,
    pub losses: u32,
    pub draws: u32,
}
