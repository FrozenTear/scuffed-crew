use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TournamentFormat {
    SingleElim,
    DoubleElim,
    RoundRobin,
    Swiss,
}

impl std::fmt::Display for TournamentFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TournamentFormat::SingleElim => write!(f, "single_elim"),
            TournamentFormat::DoubleElim => write!(f, "double_elim"),
            TournamentFormat::RoundRobin => write!(f, "round_robin"),
            TournamentFormat::Swiss => write!(f, "swiss"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TournamentStatus {
    Draft,
    Registration,
    InProgress,
    Completed,
    Archived,
}

impl std::fmt::Display for TournamentStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TournamentStatus::Draft => write!(f, "draft"),
            TournamentStatus::Registration => write!(f, "registration"),
            TournamentStatus::InProgress => write!(f, "in_progress"),
            TournamentStatus::Completed => write!(f, "completed"),
            TournamentStatus::Archived => write!(f, "archived"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParticipantStatus {
    Registered,
    CheckedIn,
    Active,
    Eliminated,
    Withdrawn,
    Disqualified,
}

impl std::fmt::Display for ParticipantStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParticipantStatus::Registered => write!(f, "registered"),
            ParticipantStatus::CheckedIn => write!(f, "checked_in"),
            ParticipantStatus::Active => write!(f, "active"),
            ParticipantStatus::Eliminated => write!(f, "eliminated"),
            ParticipantStatus::Withdrawn => write!(f, "withdrawn"),
            ParticipantStatus::Disqualified => write!(f, "disqualified"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BracketStage {
    Main,
    Winners,
    Losers,
    GrandFinal,
    Group,
}

impl std::fmt::Display for BracketStage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BracketStage::Main => write!(f, "main"),
            BracketStage::Winners => write!(f, "winners"),
            BracketStage::Losers => write!(f, "losers"),
            BracketStage::GrandFinal => write!(f, "grand_final"),
            BracketStage::Group => write!(f, "group"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TournamentMatchStatus {
    Pending,
    Scheduled,
    InProgress,
    Completed,
    Bye,
}

impl std::fmt::Display for TournamentMatchStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TournamentMatchStatus::Pending => write!(f, "pending"),
            TournamentMatchStatus::Scheduled => write!(f, "scheduled"),
            TournamentMatchStatus::InProgress => write!(f, "in_progress"),
            TournamentMatchStatus::Completed => write!(f, "completed"),
            TournamentMatchStatus::Bye => write!(f, "bye"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoundStatus {
    Pending,
    InProgress,
    Completed,
}

impl std::fmt::Display for RoundStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RoundStatus::Pending => write!(f, "pending"),
            RoundStatus::InProgress => write!(f, "in_progress"),
            RoundStatus::Completed => write!(f, "completed"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tournament {
    pub id: String,
    pub name: String,
    pub game_id: Option<String>,
    pub format: TournamentFormat,
    pub status: TournamentStatus,
    pub max_teams: Option<u32>,
    pub best_of: u32,
    pub swiss_rounds: Option<u32>,
    pub is_external: bool,
    pub is_open: bool,
    pub external_url: Option<String>,
    pub rules: Option<String>,
    pub description: Option<String>,
    pub starts_at: Option<DateTime<Utc>>,
    pub ends_at: Option<DateTime<Utc>>,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TournamentParticipant {
    pub id: String,
    pub tournament_id: String,
    pub team_id: Option<String>,
    pub external_name: Option<String>,
    pub seed: Option<u32>,
    pub group_label: Option<String>,
    pub status: ParticipantStatus,
    pub created_at: DateTime<Utc>,
}

impl TournamentParticipant {
    pub fn display_name(&self) -> &str {
        self.external_name.as_deref().unwrap_or("TBD")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TournamentRound {
    pub id: String,
    pub tournament_id: String,
    pub round_number: u32,
    pub stage: BracketStage,
    pub status: RoundStatus,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TournamentMatch {
    pub id: String,
    pub tournament_id: String,
    pub round_id: String,
    pub bracket_position: u32,
    pub participant_a_id: Option<String>,
    pub participant_b_id: Option<String>,
    pub score_a: Option<u32>,
    pub score_b: Option<u32>,
    pub winner_id: Option<String>,
    pub status: TournamentMatchStatus,
    pub scheduled_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub match_result_id: Option<String>,
    pub next_match_id: Option<String>,
    pub next_match_slot: Option<String>,
    pub loser_next_match_id: Option<String>,
    pub loser_next_match_slot: Option<String>,
    pub notes: Option<String>,
    pub replay_codes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwissStanding {
    pub participant_id: String,
    pub participant_name: String,
    pub wins: u32,
    pub losses: u32,
    pub draws: u32,
    pub game_wins: u32,
    pub game_losses: u32,
    pub buchholz: f64,
    pub rank: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TournamentBracket {
    pub tournament: Tournament,
    pub participants: Vec<TournamentParticipant>,
    pub rounds: Vec<TournamentRound>,
    pub matches: Vec<TournamentMatch>,
}
