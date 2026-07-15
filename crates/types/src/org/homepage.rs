use serde::{Deserialize, Serialize};

use super::home_identity::{HomeShell, HomeSkin};

/// Public homepage layout variant (admin-selectable).
///
/// **Deprecated for composition** once `home_shell` / `home_skin` ship (PR2).
/// Retained for one-release dual-write compatibility with Hub/Landing empty policy.
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

fn default_true() -> bool {
    true
}

/// Which homepage blocks are enabled. Hero is always shown.
/// Hub layout may still hide empty *data* sections; these flags force-hide copy blocks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HomepageSections {
    #[serde(default = "default_true")]
    pub ethos: bool,
    #[serde(default = "default_true")]
    pub schedule: bool,
    #[serde(default = "default_true")]
    pub tournaments: bool,
    #[serde(default = "default_true")]
    pub teams: bool,
    #[serde(default = "default_true")]
    pub news: bool,
    #[serde(default = "default_true")]
    pub recruit: bool,
}

impl Default for HomepageSections {
    fn default() -> Self {
        Self {
            ethos: true,
            schedule: true,
            tournaments: true,
            teams: true,
            news: true,
            recruit: true,
        }
    }
}

impl HomepageSections {
    pub const fn all_on() -> Self {
        Self {
            ethos: true,
            schedule: true,
            tournaments: true,
            teams: true,
            news: true,
            recruit: true,
        }
    }

    /// Lean public page: hero + teams + recruit.
    pub const fn lean() -> Self {
        Self {
            ethos: true,
            schedule: false,
            tournaments: false,
            teams: true,
            news: false,
            recruit: true,
        }
    }
}

/// Editable homepage copy. Stored as JSON on site_settings; missing keys use defaults.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HomepageContent {
    /// How primary homepage text and blocks are aligned.
    #[serde(default)]
    pub content_align: ContentAlign,
    /// Toggle which sections render (hero always on).
    #[serde(default)]
    pub sections: HomepageSections,
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

/// Product-neutral defaults for new installs and empty settings.
/// Clan-specific voice lives in [`homepage_presets`], not here.
impl Default for HomepageContent {
    fn default() -> Self {
        HomepagePreset::neutral().content
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

// ---------------------------------------------------------------------------
// Homepage presets (starter templates)
// ---------------------------------------------------------------------------

/// Optional brand accents suggested by a template (empty strings = leave brand alone).
#[derive(Debug, Clone, Copy, Default)]
pub struct PresetBrand {
    pub accent_dark: &'static str,
    pub accent_light: &'static str,
}

/// A named starter pack for homepage copy (+ shell / skin / layout / brand).
#[derive(Debug, Clone)]
pub struct HomepagePreset {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    /// Composition shell (preferred identity field).
    pub suggested_shell: HomeShell,
    /// Visual skin.
    pub suggested_skin: HomeSkin,
    /// Dual-write mirror of shell → Hub/Landing (until PublicLayout is removed).
    pub suggested_layout: PublicLayout,
    /// When non-empty, Apply template can also set brand accents.
    pub suggested_brand: PresetBrand,
    pub content: HomepageContent,
}

/// Catalog of homepage starter templates (stable ids for admin UI + setup).
pub fn homepage_presets() -> Vec<HomepagePreset> {
    vec![
        HomepagePreset::neutral(),
        HomepagePreset::competitive(),
        HomepagePreset::casual(),
        HomepagePreset::privacy(),
        HomepagePreset::scuffed(),
    ]
}

/// Look up a preset by id (`neutral`, `competitive`, …).
pub fn homepage_preset_by_id(id: &str) -> Option<HomepagePreset> {
    homepage_presets().into_iter().find(|p| p.id == id)
}

impl HomepagePreset {
    /// Clean install default — placeholders, all sections on, product purple.
    pub fn neutral() -> Self {
        Self {
            id: "neutral",
            name: "Neutral",
            description: "Clean starter. Generic labels you replace with your clan’s voice.",
            suggested_shell: HomeShell::OpsHub,
            suggested_skin: HomeSkin::Clean,
            suggested_layout: PublicLayout::Hub,
            suggested_brand: PresetBrand {
                accent_dark: "#8f73ff",
                accent_light: "#6d4aff",
            },
            content: HomepageContent {
                content_align: ContentAlign::Left,
                sections: HomepageSections::all_on(),
                hero_badge: "Gaming clan".into(),
                hero_title: "Your".into(),
                hero_title_accent: "Clan".into(),
                hero_sub: "Roster, schedule, and a home page you control. Edit this copy in Settings."
                    .into(),
                cta_primary: "Apply to join".into(),
                cta_secondary: "See teams".into(),
                ethos_kicker: "About".into(),
                ethos_title: "How we play".into(),
                ethos_body: "Write a short description of your culture, expectations, and what makes the clan worth joining.".into(),
                ethos_rules: vec![
                    "Show up when you commit — or tell your squad.".into(),
                    "Respect teammates. Keep drama out of voice.".into(),
                    "Communicate clearly on play nights.".into(),
                ],
                teams_kicker: "Roster".into(),
                teams_title: "Teams".into(),
                teams_empty: "No teams listed yet.".into(),
                news_kicker: "News".into(),
                news_title: "Announcements".into(),
                news_empty: "No announcements yet.".into(),
                news_view_all: "All announcements".into(),
                tournaments_kicker: "Compete".into(),
                tournaments_title: "Tournaments".into(),
                tournaments_empty: "No open or live tournaments.".into(),
                tournaments_view_all: "All tournaments".into(),
                schedule_kicker: "Calendar".into(),
                schedule_title: "Play nights".into(),
                schedule_empty: "No recurring events yet.".into(),
                calendar_cta: "Subscribe (.ics)".into(),
                recruit_kicker: "Join".into(),
                recruit_title: "Want in?".into(),
                recruit_body: "Apply and tell us what you play. Officers review applications.".into(),
                recruit_cta: "Start application".into(),
                recruit_expectations_title: "What we expect".into(),
                recruit_expectations: vec![
                    "Be respectful and communicate".into(),
                    "Honor the schedule you sign up for".into(),
                    "Follow age and platform rules set by officers".into(),
                ],
                never_ask_title: "What we never ask for".into(),
                never_ask_body: "Passwords · account recovery codes · unnecessary personal data"
                    .into(),
                seeking_label: "Looking for".into(),
                seeking_tags: vec![],
                footer_note: String::new(),
            },
        }
    }

    /// Structure-first competitive clan.
    pub fn competitive() -> Self {
        Self {
            id: "competitive",
            name: "Competitive clan",
            description: "Squads, schedule, tryouts. Ops-first hub layout.",
            suggested_shell: HomeShell::OpsHub,
            suggested_skin: HomeSkin::Esports,
            suggested_layout: PublicLayout::Hub,
            suggested_brand: PresetBrand {
                accent_dark: "#38bdf8",
                accent_light: "#0284c7",
            },
            content: HomepageContent {
                content_align: ContentAlign::Left,
                sections: HomepageSections::all_on(),
                hero_badge: "Competitive · multi-game".into(),
                hero_title: "Structured".into(),
                hero_title_accent: "play".into(),
                hero_sub: "Small squads, real rosters, scheduled nights. No ghost members.".into(),
                cta_primary: "Apply to join".into(),
                cta_secondary: "See teams".into(),
                ethos_kicker: "Standards".into(),
                ethos_title: "Not a LFG dump.".into(),
                ethos_body: "Life first, games second — when you commit a night, you show up. Communicate or lose the slot.".into(),
                ethos_rules: vec![
                    "Squads of ~5–10 — the team is the unit, the org is the scaffold.".into(),
                    "Multi-game is fine; one culture across titles.".into(),
                    "Voice for play nights; async chat for everything else.".into(),
                    "No drama. Show up. Don’t ghost.".into(),
                ],
                teams_kicker: "Roster".into(),
                teams_title: "Active squads".into(),
                teams_empty: "No squads listed yet.".into(),
                news_kicker: "Board".into(),
                news_title: "Announcements".into(),
                news_empty: "Nothing on the board yet.".into(),
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
                recruit_body: "Rosters stay intentional. Apply, show up a few nights, we match you to a squad.".into(),
                recruit_cta: "Start application".into(),
                recruit_expectations_title: "What we expect".into(),
                recruit_expectations: vec![
                    "Old enough to communicate and commit (see age requirement)".into(),
                    "Tell your squad if you can’t make it".into(),
                    "Competitive is fine; being a jerk isn’t".into(),
                    "Mic for play nights".into(),
                ],
                never_ask_title: "What we’ll never ask for".into(),
                never_ask_body: "Account passwords · recovery codes · unnecessary personal data"
                    .into(),
                seeking_label: "Looking for".into(),
                seeking_tags: vec!["Players who show up".into(), "Clear comms".into()],
                footer_note: String::new(),
            },
        }
    }

    /// Friendly, low-pressure community.
    pub fn casual() -> Self {
        Self {
            id: "casual",
            name: "Casual community",
            description: "Warm welcome page. Landing layout, softer copy.",
            suggested_shell: HomeShell::RecruitLanding,
            suggested_skin: HomeSkin::Clean,
            suggested_layout: PublicLayout::Landing,
            suggested_brand: PresetBrand {
                accent_dark: "#46d8a4",
                accent_light: "#0ea66e",
            },
            content: HomepageContent {
                content_align: ContentAlign::Center,
                sections: HomepageSections {
                    ethos: true,
                    schedule: true,
                    tournaments: true,
                    teams: true,
                    news: true,
                    recruit: true,
                },
                hero_badge: "Community · all skill levels".into(),
                hero_title: "Play".into(),
                hero_title_accent: "together".into(),
                hero_sub: "Regular games without the toxic grind. Pull up a chair.".into(),
                cta_primary: "Join us".into(),
                cta_secondary: "Meet the teams".into(),
                ethos_kicker: "Vibe".into(),
                ethos_title: "Come as you are.".into(),
                ethos_body:
                    "We organize nights so you don’t have to LFG alone. Good company first.".into(),
                ethos_rules: vec![
                    "Be kind — new players welcome.".into(),
                    "No pressure to sweat every night.".into(),
                    "RSVP when you can; life happens.".into(),
                ],
                teams_kicker: "Groups".into(),
                teams_title: "Who we play with".into(),
                teams_empty: "Teams coming soon.".into(),
                news_kicker: "Updates".into(),
                news_title: "What’s new".into(),
                news_empty: "Check back soon.".into(),
                news_view_all: "All updates".into(),
                tournaments_kicker: "Events".into(),
                tournaments_title: "Fun comps".into(),
                tournaments_empty: "No events lined up right now.".into(),
                tournaments_view_all: "All events".into(),
                schedule_kicker: "When".into(),
                schedule_title: "Hangout nights".into(),
                schedule_empty: "Schedule still cooking.".into(),
                calendar_cta: "Add to calendar".into(),
                recruit_kicker: "Join".into(),
                recruit_title: "Pull up a chair".into(),
                recruit_body: "Tell us what you play — we’ll help you find a group.".into(),
                recruit_cta: "Apply".into(),
                recruit_expectations_title: "House rules".into(),
                recruit_expectations: vec![
                    "Respect everyone".into(),
                    "No harassment".into(),
                    "Have fun".into(),
                ],
                never_ask_title: "Privacy".into(),
                never_ask_body: "We won’t ask for passwords or random personal data.".into(),
                seeking_label: "Open to".into(),
                seeking_tags: vec!["All roles".into(), "New friends".into()],
                footer_note: String::new(),
            },
        }
    }

    /// Privacy-forward community narrative.
    pub fn privacy() -> Self {
        Self {
            id: "privacy",
            name: "Privacy-first",
            description: "Minimal data story. Ethos + recruit focused; comps optional.",
            suggested_shell: HomeShell::Manifesto,
            suggested_skin: HomeSkin::Clean,
            suggested_layout: PublicLayout::Landing,
            suggested_brand: PresetBrand {
                accent_dark: "#c084fc",
                accent_light: "#9333ea",
            },
            content: HomepageContent {
                content_align: ContentAlign::Left,
                sections: HomepageSections {
                    ethos: true,
                    schedule: true,
                    tournaments: false,
                    teams: true,
                    news: true,
                    recruit: true,
                },
                hero_badge: "Privacy-respecting community".into(),
                hero_title: "Play".into(),
                hero_title_accent: "without the harvest".into(),
                hero_sub: "Clan tools that don’t need your real name, phone, or social graph."
                    .into(),
                cta_primary: "Apply".into(),
                cta_secondary: "How we operate".into(),
                ethos_kicker: "Principles".into(),
                ethos_title: "Less data. More game.".into(),
                ethos_body: "We run our own stack where we can. You shouldn’t need to dox yourself to join a squad.".into(),
                ethos_rules: vec![
                    "Collect only what the roster needs.".into(),
                    "No selling data. No dark patterns.".into(),
                    "Prefer self-hosted comms when it fits.".into(),
                    "Show up and communicate — that’s the bar.".into(),
                ],
                teams_kicker: "Roster".into(),
                teams_title: "Squads".into(),
                teams_empty: "No squads listed yet.".into(),
                news_kicker: "Board".into(),
                news_title: "Announcements".into(),
                news_empty: "Nothing posted yet.".into(),
                news_view_all: "All announcements".into(),
                tournaments_kicker: "Compete".into(),
                tournaments_title: "Tournaments".into(),
                tournaments_empty: "No open tournaments.".into(),
                tournaments_view_all: "All tournaments".into(),
                schedule_kicker: "Calendar".into(),
                schedule_title: "Play nights".into(),
                schedule_empty: "No nights scheduled yet.".into(),
                calendar_cta: "Subscribe (.ics)".into(),
                recruit_kicker: "Join".into(),
                recruit_title: "Interested?".into(),
                recruit_body: "Apply with a handle and what you play. We won’t demand a personal dossier.".into(),
                recruit_cta: "Start application".into(),
                recruit_expectations_title: "What we expect".into(),
                recruit_expectations: vec![
                    "Communicate and don’t ghost".into(),
                    "Respect privacy of other members".into(),
                    "Follow age rules set by officers".into(),
                ],
                never_ask_title: "What we’ll never ask for".into(),
                never_ask_body: "Real name · email harvest · phone · socials · location · ID · contacts · process scans".into(),
                seeking_label: "Looking for".into(),
                seeking_tags: vec!["People who care about privacy".into()],
                footer_note: String::new(),
            },
        }
    }

    /// Flagship / demo voice for The Scuffed Crew — opt-in only.
    pub fn scuffed() -> Self {
        Self {
            id: "scuffed",
            name: "Scuffed Crew (demo)",
            description: "Flagship Scuffed Crew copy + purple brand. For your clan or demos.",
            suggested_shell: HomeShell::OpsHub,
            suggested_skin: HomeSkin::Esports,
            suggested_layout: PublicLayout::Hub,
            suggested_brand: PresetBrand {
                accent_dark: "#8f73ff",
                accent_light: "#6d4aff",
            },
            content: HomepageContent {
                content_align: ContentAlign::Left,
                sections: HomepageSections::all_on(),
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
            },
        }
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
    fn sections_default_when_missing_from_legacy_json() {
        let json = r#"{"hero_badge":"x","hero_title":"T","hero_title_accent":"A","hero_sub":"s","cta_primary":"c","cta_secondary":"c","ethos_kicker":"k","ethos_title":"t","ethos_body":"b","ethos_rules":[],"teams_kicker":"k","teams_title":"t","teams_empty":"e","news_kicker":"k","news_title":"t","news_empty":"e","news_view_all":"v","tournaments_kicker":"k","tournaments_title":"t","tournaments_empty":"e","tournaments_view_all":"v","schedule_kicker":"k","schedule_title":"t","schedule_empty":"e","calendar_cta":"c","recruit_kicker":"k","recruit_title":"t","recruit_body":"b","recruit_cta":"c","recruit_expectations_title":"t","recruit_expectations":[],"never_ask_title":"t","never_ask_body":"b","seeking_label":"l","seeking_tags":[],"footer_note":"f"}"#;
        let hp = HomepageContent::from_json(json);
        assert!(hp.sections.ethos);
        assert!(hp.sections.teams);
        assert_eq!(hp.hero_badge, "x");
    }

    #[test]
    fn default_is_neutral_not_scuffed() {
        let d = HomepageContent::default();
        assert_eq!(d.hero_title, "Your");
        assert_eq!(d.hero_title_accent, "Clan");
        assert!(!d.hero_title.to_lowercase().contains("scuffed"));
        assert!(d.seeking_tags.is_empty());
    }

    #[test]
    fn presets_catalog_has_unique_ids() {
        let presets = homepage_presets();
        assert!(presets.len() >= 4);
        let mut ids: Vec<&str> = presets.iter().map(|p| p.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), presets.len());
        assert!(homepage_preset_by_id("scuffed").is_some());
        assert!(homepage_preset_by_id("neutral").is_some());
        assert!(homepage_preset_by_id("nope").is_none());
    }

    #[test]
    fn scuffed_preset_keeps_flagship_copy() {
        let s = HomepagePreset::scuffed();
        assert_eq!(s.content.hero_title, "The Scuffed");
        assert!(s.content.seeking_tags.iter().any(|t| t.contains("OW2")));
        assert!(!s.suggested_brand.accent_dark.is_empty());
        assert_eq!(s.suggested_shell, HomeShell::OpsHub);
        assert_eq!(s.suggested_skin, HomeSkin::Esports);
    }

    #[test]
    fn privacy_hides_tournaments_by_default() {
        let p = HomepagePreset::privacy();
        assert!(!p.content.sections.tournaments);
        assert!(p.content.sections.ethos);
        assert_eq!(p.suggested_shell, HomeShell::Manifesto);
        assert_eq!(p.suggested_skin, HomeSkin::Clean);
    }

    #[test]
    fn preset_shell_skin_mapping() {
        assert_eq!(HomepagePreset::neutral().suggested_skin, HomeSkin::Clean);
        assert_eq!(
            HomepagePreset::competitive().suggested_skin,
            HomeSkin::Esports
        );
        assert_eq!(
            HomepagePreset::casual().suggested_shell,
            HomeShell::RecruitLanding
        );
    }
}
