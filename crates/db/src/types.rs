use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Organization role levels, ordered by privilege.
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

/// An org member (extends a user with org-specific data).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Member {
    pub id: String,
    pub user_id: String,
    pub org_role: OrgRole,
    pub display_name: String,
    pub bio: Option<String>,
    pub joined_at: DateTime<Utc>,
    pub is_active: bool,
}

/// A game team/squad.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Team {
    pub id: String,
    pub name: String,
    pub game: String,
    pub color: Option<String>,
    pub division: Option<String>,
    pub lore_quote: Option<String>,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
}

/// Role within a specific team.
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

/// A roster entry (member's role on a team).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RosterEntry {
    pub id: String,
    pub member_id: String,
    pub team_id: String,
    pub team_role: TeamRole,
    pub joined_at: DateTime<Utc>,
    pub is_active: bool,
}

/// A scheduled event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub id: String,
    pub title: String,
    pub day_of_week: u8,
    pub time: String,
    pub timezone: String,
    pub is_recurring: bool,
    pub team_id: Option<String>,
    pub created_by: String,
    pub is_active: bool,
}

/// Application status in the recruitment pipeline.
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

/// A recruitment application.
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
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A match result record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchResult {
    pub id: String,
    pub team_id: String,
    pub opponent: String,
    pub score_us: u32,
    pub score_them: u32,
    pub map_name: Option<String>,
    pub game_mode: Option<String>,
    pub played_at: DateTime<Utc>,
    pub recorded_by: String,
    pub notes: Option<String>,
}

/// Win-loss record for a team.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamRecord {
    pub wins: u32,
    pub losses: u32,
    pub draws: u32,
}
