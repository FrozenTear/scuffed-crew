use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditAction {
    CreatedGame,
    UpdatedGame,
    CreatedTeam,
    UpdatedTeam,
    CreatedEvent,
    UpdatedEvent,
    DeletedEvent,
    AcceptedApplication,
    RejectedApplication,
    ChangedRole,
    UpdatedMember,
    AddedToRoster,
    UpdatedRosterRole,
    RemovedFromRoster,
    RecordedMatch,
    UpdatedMatch,
    CreatedAnnouncement,
    UpdatedAnnouncement,
    DeletedAnnouncement,
    CreatedModerationAction,
    LiftedModerationAction,
    UpdatedSettings,
    CreatedTournament,
    UpdatedTournament,
    ChangedTournamentStatus,
    GeneratedBracket,
    AddedTournamentParticipant,
    RemovedTournamentParticipant,
    ReportedTournamentMatch,
    PublishedCommunity,
    PublishedPost,
    PublishedReaction,
}

impl std::fmt::Display for AuditAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = serde_json::to_value(self)
            .ok()
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .unwrap_or_else(|| format!("{self:?}"));
        write!(f, "{s}")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditTargetType {
    Game,
    Team,
    Member,
    Event,
    Application,
    Roster,
    Match,
    Announcement,
    Moderation,
    Settings,
    Tournament,
    TournamentParticipant,
    TournamentMatch,
}

impl std::fmt::Display for AuditTargetType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = serde_json::to_value(self)
            .ok()
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .unwrap_or_else(|| format!("{self:?}"));
        write!(f, "{s}")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditLogEntry {
    pub id: String,
    pub actor_id: String,
    pub action: String,
    pub target_type: String,
    pub target_id: String,
    pub details: Option<String>,
    pub created_at: DateTime<Utc>,
}
