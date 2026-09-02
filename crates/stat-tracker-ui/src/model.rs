use chrono::{DateTime, Utc};
use stat_tracker::detect::MatchOutcome;
use stat_tracker::storage::PersonalMatch;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Tank,
    Damage,
    Support,
    Unknown,
}

impl Role {
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "tank" => Role::Tank,
            "damage" | "dps" => Role::Damage,
            "support" => Role::Support,
            _ => Role::Unknown,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Role::Tank => "Tank",
            Role::Damage => "Damage",
            Role::Support => "Support",
            Role::Unknown => "Role",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Win,
    Loss,
    Draw,
    Unknown,
}

impl Outcome {
    pub fn parse(s: &str) -> Self {
        match MatchOutcome::parse_lenient(s) {
            MatchOutcome::Victory => Outcome::Win,
            MatchOutcome::Defeat => Outcome::Loss,
            MatchOutcome::Draw => Outcome::Draw,
            MatchOutcome::Unknown => Outcome::Unknown,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Outcome::Win => "Win",
            Outcome::Loss => "Loss",
            Outcome::Draw => "Draw",
            Outcome::Unknown => "—",
        }
    }
}

/// Display-normalized game row. Aggregation and views share this — not the
/// daemon's storage struct — so season math stays a pure UI-crate function.
#[derive(Debug, Clone, PartialEq)]
pub struct Game {
    pub session_id: String,
    pub hero: String,
    pub map_name: String,
    pub role: Role,
    pub outcome: Outcome,
    pub elims: u32,
    pub deaths: u32,
    pub assists: u32,
    pub damage: u32,
    pub healing: u32,
    pub mitigation: u32,
    pub played_at: DateTime<Utc>,
}

impl Game {
    pub fn from_match(m: &PersonalMatch) -> Self {
        Self {
            session_id: m.session_id.clone(),
            hero: m.display_hero().to_string(),
            map_name: m.display_map_name().to_string(),
            role: Role::parse(m.display_role()),
            outcome: Outcome::parse(m.display_outcome()),
            elims: m.display_elims(),
            deaths: m.display_deaths(),
            assists: m.display_assists(),
            damage: m.display_damage(),
            healing: m.display_healing(),
            mitigation: m.display_mitigation(),
            played_at: m.played_at.into(),
        }
    }

    pub fn has_stat_line(&self) -> bool {
        self.elims + self.deaths + self.assists + self.damage + self.healing + self.mitigation > 0
    }
}

/// `None` = all time. `Some(id)` = that season.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum SeasonSel {
    #[default]
    AllTime,
    Season(String),
}

impl SeasonSel {
    pub fn as_id(&self) -> Option<&str> {
        match self {
            SeasonSel::AllTime => None,
            SeasonSel::Season(id) => Some(id.as_str()),
        }
    }
}
