use serde::{Deserialize, Serialize};

/// Public homepage layout variant (admin-selectable).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PublicLayout {
    /// Operational hub: tight hero, rules list, live data first. Less "landing page".
    #[default]
    Hub,
    /// Classic marketing scroll with sectioned content (still uses editable copy).
    Landing,
}

impl PublicLayout {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Hub => "hub",
            Self::Landing => "landing",
        }
    }

    pub fn from_str_lossy(s: &str) -> Self {
        match s {
            "landing" => Self::Landing,
            _ => Self::Hub,
        }
    }
}

impl std::fmt::Display for PublicLayout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Text / block alignment for the public homepage (admin-selectable).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ContentAlign {
    #[default]
    Left,
    Center,
}

impl ContentAlign {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Center => "center",
        }
    }

    pub fn from_str_lossy(s: &str) -> Self {
        match s {
            "center" => Self::Center,
            _ => Self::Left,
        }
    }

    pub fn css_class(self) -> &'static str {
        match self {
            Self::Left => "align-left",
            Self::Center => "align-center",
        }
    }
}

impl std::fmt::Display for ContentAlign {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Editable homepage copy. Stored as JSON on site_settings; missing keys use defaults.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HomepageContent {
    /// How primary homepage text and blocks are aligned.
    #[serde(default)]
    pub content_align: ContentAlign,
    pub hero_badge: String,
    pub hero_title: String,
    pub hero_title_accent: String,
    pub hero_sub: String,
    pub cta_primary: String,
    pub cta_secondary: String,
    pub ethos_kicker: String,
    pub ethos_title: String,
    pub ethos_body: String,
    /// Short rule lines (shown as a list, not icon cards).
    pub ethos_rules: Vec<String>,
    pub teams_kicker: String,
    pub teams_title: String,
    pub teams_empty: String,
    pub news_kicker: String,
    pub news_title: String,
    pub news_empty: String,
    pub news_view_all: String,
    pub tournaments_kicker: String,
    pub tournaments_title: String,
    pub tournaments_empty: String,
    pub tournaments_view_all: String,
    pub schedule_kicker: String,
    pub schedule_title: String,
    pub schedule_empty: String,
    pub calendar_cta: String,
    pub recruit_kicker: String,
    pub recruit_title: String,
    pub recruit_body: String,
    pub recruit_cta: String,
    pub recruit_expectations_title: String,
    pub recruit_expectations: Vec<String>,
    pub never_ask_title: String,
    pub never_ask_body: String,
    pub seeking_label: String,
    pub seeking_tags: Vec<String>,
    pub footer_note: String,
}

impl Default for HomepageContent {
    fn default() -> Self {
        Self {
            content_align: ContentAlign::Left,
            hero_badge: "Multi-game crew · EMEA · Est. 2026".into(),
            hero_title: "The Scuffed".into(),
            hero_title_accent: "Crew".into(),
            hero_sub: "Small teams, real structure, scheduled play nights. No ghost members. No dead servers.".into(),
            cta_primary: "Apply to join".into(),
            cta_secondary: "See teams".into(),
            ethos_kicker: "The rulebook".into(),
            ethos_title: "Not a server. A clan.".into(),
            ethos_body: "Life comes first — the games come second, but we still show up. Communicate or lose your slot.".into(),
            ethos_rules: vec![
                "Squads of ~5–10 — your team is your crew, the org is the scaffold.".into(),
                "Multi-game under one roof — play what you play, still one culture.".into(),
                "TeamSpeak for play nights, Matrix for everything else.".into(),
                "No politics, no drama. Show up. Don’t ghost.".into(),
            ],
            teams_kicker: "Roster".into(),
            teams_title: "Active squads".into(),
            teams_empty: "No squads listed yet.".into(),
            news_kicker: "Board".into(),
            news_title: "Announcements".into(),
            news_empty: "Nothing on the board. Check Matrix.".into(),
            news_view_all: "All announcements".into(),
            tournaments_kicker: "Compete".into(),
            tournaments_title: "Tournaments".into(),
            tournaments_empty: "No open or live tournaments.".into(),
            tournaments_view_all: "All tournaments".into(),
            schedule_kicker: "Calendar".into(),
            schedule_title: "Play nights".into(),
            schedule_empty: "No recurring nights set — officers, fix that.".into(),
            calendar_cta: "Subscribe (.ics)".into(),
            recruit_kicker: "Join".into(),
            recruit_title: "Want in?".into(),
            recruit_body: "Rosters stay intentional. Apply, show up a few nights, we match you to a squad that fits.".into(),
            recruit_cta: "Start application".into(),
            recruit_expectations_title: "What we expect".into(),
            recruit_expectations: vec![
                "16+ — old enough to communicate and commit".into(),
                "PC only".into(),
                "Tell your squad if you can’t make it".into(),
                "Competitive is fine; being a jerk isn’t".into(),
                "Mic for play nights".into(),
                "TeamSpeak when you’re on a roster".into(),
            ],
            never_ask_title: "What we’ll never ask for".into(),
            never_ask_body: "Real name · email harvest · phone · socials · location · ID · contacts · process scans".into(),
            seeking_label: "Looking for".into(),
            seeking_tags: vec!["OW2 DPS".into(), "OW2 Support".into(), "D2 PvP".into()],
            footer_note: "© The Scuffed Crew · Est. EMEA".into(),
        }
    }
}

impl HomepageContent {
    /// Parse JSON; on empty/invalid, return defaults (optionally merged).
    pub fn from_json(s: &str) -> Self {
        if s.trim().is_empty() {
            return Self::default();
        }
        match serde_json::from_str::<Self>(s) {
            Ok(v) => v,
            Err(_) => {
                // Partial / legacy blobs: try merge via Value so new fields (e.g. content_align)
                // still get defaults without wiping known keys.
                let Ok(mut value) = serde_json::from_str::<serde_json::Value>(s) else {
                    return Self::default();
                };
                let default = serde_json::to_value(Self::default()).unwrap_or_default();
                if let (Some(obj), Some(def_obj)) = (value.as_object_mut(), default.as_object()) {
                    for (k, v) in def_obj {
                        obj.entry(k.clone()).or_insert_with(|| v.clone());
                    }
                }
                serde_json::from_value(value).unwrap_or_default()
            }
        }
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_align_roundtrips_in_json() {
        let mut hp = HomepageContent::default();
        hp.content_align = ContentAlign::Center;
        let json = hp.to_json();
        assert!(
            json.contains("\"content_align\":\"center\""),
            "expected content_align in json, got {json}"
        );
        let back = HomepageContent::from_json(&json);
        assert_eq!(back.content_align, ContentAlign::Center);
    }

    #[test]
    fn content_align_defaults_when_missing() {
        let json = r#"{"hero_badge":"x","hero_title":"T","hero_title_accent":"A","hero_sub":"s","cta_primary":"c","cta_secondary":"c","ethos_kicker":"k","ethos_title":"t","ethos_body":"b","ethos_rules":[],"teams_kicker":"k","teams_title":"t","teams_empty":"e","news_kicker":"k","news_title":"t","news_empty":"e","news_view_all":"v","tournaments_kicker":"k","tournaments_title":"t","tournaments_empty":"e","tournaments_view_all":"v","schedule_kicker":"k","schedule_title":"t","schedule_empty":"e","calendar_cta":"c","recruit_kicker":"k","recruit_title":"t","recruit_body":"b","recruit_cta":"c","recruit_expectations_title":"t","recruit_expectations":[],"never_ask_title":"t","never_ask_body":"b","seeking_label":"l","seeking_tags":[],"footer_note":"f"}"#;
        let hp = HomepageContent::from_json(json);
        assert_eq!(hp.content_align, ContentAlign::Left);
        assert_eq!(hp.hero_badge, "x");
    }
}
