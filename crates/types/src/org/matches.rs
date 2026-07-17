use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MatchType {
    #[default]
    Scrim,
    Official,
    Tournament,
}

impl std::fmt::Display for MatchType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MatchType::Scrim => write!(f, "scrim"),
            MatchType::Official => write!(f, "official"),
            MatchType::Tournament => write!(f, "tournament"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchResult {
    pub id: String,
    pub team_id: String,
    pub opponent: String,
    /// None when scheduled / not yet played.
    pub score_us: Option<u32>,
    pub score_them: Option<u32>,
    pub map_name: Option<String>,
    pub game_mode: Option<String>,
    pub match_type: MatchType,
    /// Set when the match has been played.
    pub played_at: Option<DateTime<Utc>>,
    /// Future fixture time (kept after play for history).
    pub scheduled_at: Option<DateTime<Utc>>,
    pub recorded_by: Option<String>,
    pub notes: Option<String>,
    /// When false, omitted from public team pages and public match lists.
    #[serde(default)]
    pub is_public: bool,
    /// Twitch/YouTube VOD URL (https + host allowlist).
    pub vod_url: Option<String>,
    /// Overwatch 2 replay code (≤16 alphanumeric).
    pub replay_code: Option<String>,
}

/// Public-safe match projection — no notes, no recorded_by.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublicMatch {
    pub id: String,
    pub team_id: String,
    pub opponent: String,
    pub score_us: Option<u32>,
    pub score_them: Option<u32>,
    pub map_name: Option<String>,
    pub game_mode: Option<String>,
    pub match_type: MatchType,
    pub played_at: Option<DateTime<Utc>>,
    pub scheduled_at: Option<DateTime<Utc>>,
    pub vod_url: Option<String>,
    pub replay_code: Option<String>,
}

impl PublicMatch {
    /// Returns None for private rows or practice (`Scrim`) matches.
    pub fn try_from_match(m: &MatchResult) -> Option<Self> {
        if !m.is_public || matches!(m.match_type, MatchType::Scrim) {
            return None;
        }
        Some(Self {
            id: m.id.clone(),
            team_id: m.team_id.clone(),
            opponent: m.opponent.clone(),
            score_us: m.score_us,
            score_them: m.score_them,
            map_name: m.map_name.clone(),
            game_mode: m.game_mode.clone(),
            match_type: m.match_type,
            played_at: m.played_at,
            scheduled_at: m.scheduled_at,
            vod_url: m.vod_url.clone(),
            replay_code: m.replay_code.clone(),
        })
    }

    /// True when this row represents a completed result (for recent-results lists).
    pub fn is_played(&self) -> bool {
        self.played_at.is_some() || (self.score_us.is_some() && self.score_them.is_some())
    }
}

/// Home "next match" card — public scheduled fixture with display names.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpcomingMatch {
    pub id: String,
    pub team_id: String,
    pub team_name: String,
    pub game_name: Option<String>,
    pub opponent: String,
    pub match_type: MatchType,
    pub scheduled_at: DateTime<Utc>,
}

/// Home results ticker row — public completed match with outcome.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecentResult {
    pub id: String,
    pub team_id: String,
    pub team_name: String,
    pub opponent: String,
    pub score_us: Option<u32>,
    pub score_them: Option<u32>,
    /// "win" | "loss" | "draw" | "unknown"
    pub outcome: String,
    pub match_type: MatchType,
    pub played_at: DateTime<Utc>,
}

impl RecentResult {
    pub fn outcome_from_scores(score_us: Option<u32>, score_them: Option<u32>) -> &'static str {
        match (score_us, score_them) {
            (Some(u), Some(t)) if u > t => "win",
            (Some(u), Some(t)) if u < t => "loss",
            (Some(u), Some(t)) if u == t => "draw",
            _ => "unknown",
        }
    }
}
