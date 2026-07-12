//! Page-local DTOs for the public homepage (not shared types).

use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Overview {
    pub teams: Vec<OverviewTeam>,
    pub games: Vec<OverviewGame>,
    pub member_count: usize,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct OverviewGame {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct OverviewTeam {
    pub name: String,
    pub game_id: String,
    pub division: Option<String>,
    pub lore_quote: Option<String>,
    pub roster_count: usize,
    pub record: TeamRecord,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct TeamRecord {
    pub wins: u32,
    pub losses: u32,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Announcement {
    #[allow(dead_code)]
    pub id: String,
    pub title: String,
    pub content: String,
    pub pinned: bool,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct HomeTournament {
    pub id: String,
    pub name: String,
    #[allow(dead_code)]
    pub format: String,
    pub status: String,
    #[allow(dead_code)]
    pub is_external: bool,
    #[allow(dead_code)]
    pub starts_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Event {
    #[allow(dead_code)]
    pub id: String,
    pub title: String,
    pub day_of_week: u8,
    pub time: String,
    pub timezone: String,
}

pub fn day_name(d: u8) -> &'static str {
    match d {
        0 => "Mon",
        1 => "Tue",
        2 => "Wed",
        3 => "Thu",
        4 => "Fri",
        5 => "Sat",
        6 => "Sun",
        _ => "—",
    }
}
