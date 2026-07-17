use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Competitive season window for leaderboard filters.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Season {
    pub id: String,
    pub name: String,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    #[serde(default)]
    pub is_current: bool,
}

/// Public-safe hero aggregate for profiles / leaderboards.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeroAgg {
    pub hero: String,
    pub games: u32,
    pub wins: u32,
    pub losses: u32,
    pub draws: u32,
    /// 0.0–1.0
    pub winrate: f32,
    pub avg_elims: f64,
    pub avg_deaths: f64,
}

/// Member row on the overall leaderboard.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemberLeaderboardRow {
    pub member_id: String,
    pub display_name: String,
    pub games: u32,
    pub winrate: f32,
    pub kd: f64,
}

/// Hero-specific leaderboard row.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeroLeaderboardRow {
    pub member_id: String,
    pub display_name: String,
    pub hero: String,
    pub games: u32,
    pub winrate: f32,
    pub kd: f64,
}
