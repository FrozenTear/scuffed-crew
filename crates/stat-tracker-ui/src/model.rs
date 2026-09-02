use chrono::{DateTime, Utc};
use stat_tracker::detect::MatchOutcome;
use stat_tracker::storage::{HeroSegment, PersonalMatch};

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

    pub fn all_playable() -> [Role; 3] {
        [Role::Tank, Role::Damage, Role::Support]
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

    /// Canonical store / `StoreCommand::SetOutcome` spelling.
    pub fn store_label(self) -> &'static str {
        match self {
            Outcome::Win => "victory",
            Outcome::Loss => "defeat",
            Outcome::Draw => "draw",
            Outcome::Unknown => "unknown",
        }
    }
}

/// Toggleable role chips. No chip on = every role (same as unset).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RoleFilter {
    pub tank: bool,
    pub damage: bool,
    pub support: bool,
}

impl RoleFilter {
    pub fn toggle(mut self, role: Role) -> Self {
        match role {
            Role::Tank => self.tank = !self.tank,
            Role::Damage => self.damage = !self.damage,
            Role::Support => self.support = !self.support,
            Role::Unknown => {}
        }
        self
    }

    pub fn is_on(self, role: Role) -> bool {
        match role {
            Role::Tank => self.tank,
            Role::Damage => self.damage,
            Role::Support => self.support,
            Role::Unknown => false,
        }
    }

    pub fn unrestricted(self) -> bool {
        !self.tank && !self.damage && !self.support
    }

    pub fn matches(self, role: Role) -> bool {
        self.unrestricted() || self.is_on(role)
    }

    pub fn selected_roles(self) -> Vec<Role> {
        Role::all_playable()
            .into_iter()
            .filter(|r| self.is_on(*r))
            .collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Screen {
    #[default]
    Overview,
    Games,
    Heroes,
    Maps,
    Seasons,
}

impl Screen {
    pub fn label(self) -> &'static str {
        match self {
            Screen::Overview => "Overview",
            Screen::Games => "Games",
            Screen::Heroes => "Heroes",
            Screen::Maps => "Maps",
            Screen::Seasons => "Seasons",
        }
    }

    pub fn all() -> [Screen; 5] {
        [
            Screen::Overview,
            Screen::Games,
            Screen::Heroes,
            Screen::Maps,
            Screen::Seasons,
        ]
    }
}

/// Immutable OCR reads used for the "edited" badge / corrections list.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GameOcr {
    pub hero: String,
    pub role: String,
    pub map_name: String,
    pub outcome: String,
    pub elims: u32,
    pub deaths: u32,
    pub assists: u32,
    pub damage: u32,
    pub healing: u32,
    pub mitigation: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SegmentView {
    pub index: u32,
    pub hero: String,
    pub role: Role,
    pub snapshots: u32,
    pub confirmed: bool,
    pub dismissed: bool,
    pub resolution: Option<String>,
}

impl SegmentView {
    pub fn from_segment(index: u32, seg: &HeroSegment) -> Self {
        Self {
            index,
            hero: seg.hero.clone(),
            role: Role::parse(&seg.role),
            snapshots: seg.snapshots,
            confirmed: seg.confirmed,
            dismissed: seg.is_dismissed(),
            resolution: seg.resolution.clone(),
        }
    }

    pub fn status_label(&self) -> &'static str {
        if self.dismissed {
            "dismissed"
        } else if self.confirmed {
            "confirmed"
        } else {
            "unconfirmed"
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
    pub edited: bool,
    pub edited_fields: Vec<String>,
    pub ocr: GameOcr,
    pub segments: Vec<SegmentView>,
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
            edited: m.is_edited(),
            edited_fields: m.edited_fields.clone(),
            ocr: GameOcr {
                hero: m.hero.clone(),
                role: m.role.clone(),
                map_name: m.map_name.clone(),
                outcome: m.outcome.clone(),
                elims: m.elims,
                deaths: m.deaths,
                assists: m.assists,
                damage: m.damage,
                healing: m.healing,
                mitigation: m.mitigation,
            },
            segments: m
                .heroes_played
                .iter()
                .enumerate()
                .map(|(i, s)| SegmentView::from_segment(i as u32, s))
                .collect(),
        }
    }

    pub fn display_hero(&self) -> String {
        display_hero_name(&self.hero)
    }

    pub fn has_stat_line(&self) -> bool {
        self.elims + self.deaths + self.assists + self.damage + self.healing + self.mitigation > 0
    }

    pub fn show_timeline(&self) -> bool {
        !self.session_id.is_empty()
            && (self.segments.len() > 1
                || self
                    .segments
                    .iter()
                    .any(|s| !s.confirmed || s.resolution.is_some()))
    }

    /// OCR → corrected pairs for fields that carry a manual overlay.
    pub fn corrections(&self) -> Vec<(&'static str, String, String)> {
        let mut out = Vec::new();
        let push =
            |out: &mut Vec<_>, field: &str, label: &'static str, ocr: String, shown: String| {
                if self.edited_fields.iter().any(|f| f == field) && ocr != shown {
                    out.push((label, empty_dash(ocr), shown));
                }
            };
        push(
            &mut out,
            "hero",
            "Hero",
            self.ocr.hero.clone(),
            self.hero.clone(),
        );
        push(
            &mut out,
            "role",
            "Role",
            self.ocr.role.clone(),
            self.role.label().to_string(),
        );
        push(
            &mut out,
            "map_name",
            "Map",
            self.ocr.map_name.clone(),
            self.map_name.clone(),
        );
        push(
            &mut out,
            "outcome",
            "Result",
            self.ocr.outcome.clone(),
            self.outcome.store_label().to_string(),
        );
        push(
            &mut out,
            "elims",
            "Elims",
            self.ocr.elims.to_string(),
            self.elims.to_string(),
        );
        push(
            &mut out,
            "deaths",
            "Deaths",
            self.ocr.deaths.to_string(),
            self.deaths.to_string(),
        );
        push(
            &mut out,
            "assists",
            "Assists",
            self.ocr.assists.to_string(),
            self.assists.to_string(),
        );
        push(
            &mut out,
            "damage",
            "Damage",
            self.ocr.damage.to_string(),
            self.damage.to_string(),
        );
        push(
            &mut out,
            "healing",
            "Healing",
            self.ocr.healing.to_string(),
            self.healing.to_string(),
        );
        push(
            &mut out,
            "mitigation",
            "Mitigation",
            self.ocr.mitigation.to_string(),
            self.mitigation.to_string(),
        );
        out
    }
}

pub fn display_hero_name(name: &str) -> String {
    let t = name.trim();
    if t.is_empty() || t.eq_ignore_ascii_case("unknown") {
        "Unknown hero".into()
    } else {
        t.to_string()
    }
}

fn empty_dash(s: String) -> String {
    if s.is_empty() { "—".into() } else { s }
}

#[cfg(test)]
mod tests {
    use super::display_hero_name;

    #[test]
    fn unknown_hero_label() {
        assert_eq!(display_hero_name(""), "Unknown hero");
        assert_eq!(display_hero_name("   "), "Unknown hero");
        assert_eq!(display_hero_name("Unknown"), "Unknown hero");
        assert_eq!(display_hero_name("unknown"), "Unknown hero");
        assert_eq!(display_hero_name("Ana"), "Ana");
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
