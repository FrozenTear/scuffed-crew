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
    pub score_us: u32,
    pub score_them: u32,
    pub map_name: Option<String>,
    pub game_mode: Option<String>,
    pub match_type: MatchType,
    pub played_at: DateTime<Utc>,
    pub recorded_by: String,
    pub notes: Option<String>,
    /// When false, omitted from public team pages and public match lists.
    #[serde(default)]
    pub is_public: bool,
}

/// Public-safe match projection — no notes, no recorded_by.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublicMatch {
    pub id: String,
    pub team_id: String,
    pub opponent: String,
    pub score_us: u32,
    pub score_them: u32,
    pub map_name: Option<String>,
    pub game_mode: Option<String>,
    pub match_type: MatchType,
    pub played_at: DateTime<Utc>,
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
        })
    }
}
