//! GUI screens and edit forms. The game rows, roles and outcomes live in
//! `stat_tracker::games::model` (shared with the tracker) and are re-exported
//! here, so `crate::model::Game` keeps working.

pub use stat_tracker::games::model::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Screen {
    #[default]
    Overview,
    Games,
    Heroes,
    Maps,
    Seasons,
    Settings,
}

impl Screen {
    pub fn label(self) -> &'static str {
        match self {
            Screen::Overview => "Overview",
            Screen::Games => "Games",
            Screen::Heroes => "Heroes",
            Screen::Maps => "Maps",
            Screen::Seasons => "Seasons",
            Screen::Settings => "Settings",
        }
    }

    pub const fn all() -> [Screen; 6] {
        [
            Screen::Overview,
            Screen::Games,
            Screen::Heroes,
            Screen::Maps,
            Screen::Seasons,
            Screen::Settings,
        ]
    }
}

/// Editable text state for `EditMatch`. One `String` per field (parsed on save).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct EditForm {
    pub session_id: String,
    pub hero: String,
    pub role: String,
    pub map_name: String,
    pub elims: String,
    pub deaths: String,
    pub assists: String,
    pub damage: String,
    pub healing: String,
    pub mitigation: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditField {
    Hero,
    Role,
    Map,
    Elims,
    Deaths,
    Assists,
    Damage,
    Healing,
    Mitigation,
}

impl EditForm {
    pub fn from_game(g: &Game) -> Self {
        Self {
            session_id: g.session_id.clone(),
            hero: g.hero.clone(),
            role: g.role.label().to_string(),
            map_name: g.map_name.clone(),
            elims: g.elims.to_string(),
            deaths: g.deaths.to_string(),
            assists: g.assists.to_string(),
            damage: g.damage.to_string(),
            healing: g.healing.to_string(),
            mitigation: g.mitigation.to_string(),
        }
    }

    pub fn set(&mut self, field: EditField, value: String) {
        match field {
            EditField::Hero => self.hero = value,
            EditField::Role => self.role = value,
            EditField::Map => self.map_name = value,
            EditField::Elims => self.elims = value,
            EditField::Deaths => self.deaths = value,
            EditField::Assists => self.assists = value,
            EditField::Damage => self.damage = value,
            EditField::Healing => self.healing = value,
            EditField::Mitigation => self.mitigation = value,
        }
    }

    /// Only fields that differ from the game's current effective values.
    pub fn diff(&self, g: &Game) -> stat_tracker::storage::MatchEdit {
        let mut e = stat_tracker::storage::MatchEdit::default();
        let txt = |cur: &str, disp: &str| {
            let t = cur.trim();
            (!t.is_empty() && t != disp).then(|| t.to_string())
        };
        e.hero = txt(&self.hero, &g.hero);
        e.role = txt(&self.role, g.role.label());
        e.map_name = txt(&self.map_name, &g.map_name);
        let num = |cur: &str, disp: u32| cur.trim().parse::<u32>().ok().filter(|v| *v != disp);
        e.elims = num(&self.elims, g.elims);
        e.deaths = num(&self.deaths, g.deaths);
        e.assists = num(&self.assists, g.assists);
        e.damage = num(&self.damage, g.damage);
        e.healing = num(&self.healing, g.healing);
        e.mitigation = num(&self.mitigation, g.mitigation);
        e
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
