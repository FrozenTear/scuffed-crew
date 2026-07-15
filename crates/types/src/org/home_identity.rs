//! Homepage identity: shells (composition), skins (visual), empty-data policy, org initials.
//!
//! See `docs/superpowers/specs/2026-07-12-homepage-identity-packs-design.md`.

use serde::{Deserialize, Serialize};

use super::homepage::{HomepageContent, PublicLayout};

// ---------------------------------------------------------------------------
// Shells
// ---------------------------------------------------------------------------

/// Homepage composition shell (section order, empty policy, teams presentation).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum HomeShell {
    /// Dense command board — current homepage composition (default install).
    #[default]
    OpsHub,
    /// Marketing / apply-first landing.
    RecruitLanding,
    /// Minimal: hero + optional teams strip + recruit.
    Minimal,
    /// Ethos / principles first; tourneys de-emphasized.
    Manifesto,
}

impl HomeShell {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OpsHub => "ops_hub",
            Self::RecruitLanding => "recruit_landing",
            Self::Minimal => "minimal",
            Self::Manifesto => "manifesto",
        }
    }

    /// Strict parse for API writes. Unknown → error at API layer.
    pub fn from_str_strict(s: &str) -> Option<Self> {
        match s.trim() {
            "ops_hub" => Some(Self::OpsHub),
            "recruit_landing" => Some(Self::RecruitLanding),
            "minimal" => Some(Self::Minimal),
            "manifesto" => Some(Self::Manifesto),
            _ => None,
        }
    }

    /// Lossy parse for reads / migration.
    pub fn from_str_lossy(s: &str) -> Self {
        Self::from_str_strict(s).unwrap_or(Self::OpsHub)
    }

    /// Dual-write mirror for one-release compatibility with `public_layout`.
    pub fn to_public_layout(self) -> PublicLayout {
        match self {
            Self::OpsHub | Self::Minimal => PublicLayout::Hub,
            Self::RecruitLanding | Self::Manifesto => PublicLayout::Landing,
        }
    }

    /// Map legacy Hub/Landing → shell (simple migration).
    pub fn from_public_layout(layout: PublicLayout) -> Self {
        match layout {
            PublicLayout::Landing => Self::RecruitLanding,
            PublicLayout::Hub => Self::OpsHub,
        }
    }

    /// Section order **excluding Hero** (Hero always renders first in `Home()`).
    ///
    /// `ops_hub` matches current `home.rs`: Ethos → Live → Teams → News → Recruit.
    pub fn section_order(self) -> &'static [HomeSectionId] {
        match self {
            Self::OpsHub => &[
                HomeSectionId::Ethos,
                HomeSectionId::Live,
                HomeSectionId::Teams,
                HomeSectionId::News,
                HomeSectionId::Recruit,
            ],
            Self::RecruitLanding => &[
                HomeSectionId::Recruit,
                HomeSectionId::Teams,
                HomeSectionId::Ethos,
                HomeSectionId::Live,
                HomeSectionId::News,
            ],
            Self::Minimal => &[HomeSectionId::Teams, HomeSectionId::Recruit],
            Self::Manifesto => &[
                HomeSectionId::Ethos,
                HomeSectionId::Recruit,
                HomeSectionId::Teams,
                HomeSectionId::Live,
                HomeSectionId::News,
            ],
        }
    }

    /// Whether a *data-backed* section renders when its data list is empty.
    /// Ethos / Recruit are not gated by this (use section toggles / recruitment_open).
    pub fn show_when_empty(self, section: HomeSectionId) -> bool {
        match (self, section) {
            // ops_hub: preserve current Hub behavior
            (Self::OpsHub, HomeSectionId::Live) => false,
            (Self::OpsHub, HomeSectionId::News) => false,
            (Self::OpsHub, HomeSectionId::Teams) => true,
            // recruit_landing: preserve Landing
            (Self::RecruitLanding, HomeSectionId::Live) => true,
            (Self::RecruitLanding, HomeSectionId::News) => true,
            (Self::RecruitLanding, HomeSectionId::Teams) => true,
            // minimal: lean
            (Self::Minimal, HomeSectionId::Live) => false,
            (Self::Minimal, HomeSectionId::News) => false,
            (Self::Minimal, HomeSectionId::Teams) => false,
            // manifesto: hybrid
            (Self::Manifesto, HomeSectionId::Live) => false,
            (Self::Manifesto, HomeSectionId::News) => true,
            (Self::Manifesto, HomeSectionId::Teams) => true,
            // Ethos / Recruit: not gated by empty data
            (_, HomeSectionId::Ethos | HomeSectionId::Recruit) => true,
        }
    }

    pub fn teams_presentation(self) -> TeamsPresentation {
        match self {
            Self::OpsHub => TeamsPresentation::Table,
            Self::RecruitLanding | Self::Manifesto => TeamsPresentation::Cards,
            Self::Minimal => TeamsPresentation::Compact,
        }
    }
}

impl std::fmt::Display for HomeShell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ---------------------------------------------------------------------------
// Skins
// ---------------------------------------------------------------------------

/// Homepage visual skin (CSS personality). Does not change section order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum HomeSkin {
    /// Product default for non-competitive packs.
    #[default]
    Clean,
    /// Competitive / current homepage DNA.
    Esports,
}

impl HomeSkin {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Clean => "clean",
            Self::Esports => "esports",
        }
    }

    pub fn from_str_strict(s: &str) -> Option<Self> {
        match s.trim() {
            "clean" => Some(Self::Clean),
            "esports" => Some(Self::Esports),
            _ => None,
        }
    }

    pub fn from_str_lossy(s: &str) -> Self {
        Self::from_str_strict(s).unwrap_or(Self::Clean)
    }
}

impl std::fmt::Display for HomeSkin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ---------------------------------------------------------------------------
// Sections / teams presentation
// ---------------------------------------------------------------------------

/// Ordered homepage sections (Hero is not a section id).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HomeSectionId {
    Ethos,
    /// Schedule + tournaments panels in one Live block.
    Live,
    Teams,
    News,
    Recruit,
}

/// How the teams block is rendered (shell-driven).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TeamsPresentation {
    Table,
    Cards,
    Compact,
}

// ---------------------------------------------------------------------------
// Org initials (nav + hero watermark)
// ---------------------------------------------------------------------------

/// Up to 2 uppercase chars for decorative nav/hero marks.
///
/// - Skips the word "the"
/// - Keeps alphanumeric only (emoji-only names → `"CL"`)
/// - One word → first two letters; multi-word → first letter of first two words
pub fn org_initials(org_name: &str) -> String {
    let cleaned: String = org_name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c.is_whitespace() {
                c
            } else {
                ' '
            }
        })
        .collect();
    let parts: Vec<&str> = cleaned
        .split_whitespace()
        .filter(|p| !p.eq_ignore_ascii_case("the") && !p.is_empty())
        .collect();
    let raw: String = match parts.as_slice() {
        [] => return "CL".into(),
        [one] => one.chars().take(2).collect(),
        [a, b, ..] => [a.chars().next(), b.chars().next()]
            .into_iter()
            .flatten()
            .collect(),
    };
    let mut out: String = raw.chars().take(2).collect::<String>().to_uppercase();
    if out.is_empty() {
        out = "CL".into();
    }
    out
}

// ---------------------------------------------------------------------------
// Migration / backfill helpers (pure — used by DB/API in PR2)
// ---------------------------------------------------------------------------

fn normalize_hex_for_compare(raw: &str) -> Option<String> {
    let s = raw.trim();
    if s.is_empty() {
        return None;
    }
    let hex = s.strip_prefix('#').unwrap_or(s);
    if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    match hex.len() {
        3 => {
            let mut out = String::new();
            for c in hex.chars() {
                out.push(c);
                out.push(c);
            }
            Some(out.to_ascii_lowercase())
        }
        6 => Some(hex.to_ascii_lowercase()),
        _ => None,
    }
}

fn hex_eq(a: &str, b: &str) -> bool {
    match (normalize_hex_for_compare(a), normalize_hex_for_compare(b)) {
        (Some(x), Some(y)) => x == y,
        _ => false,
    }
}

/// Competitive pack cyan (see HomepagePreset::competitive brand).
const COMPETITIVE_DARK: &str = "#38bdf8";
const COMPETITIVE_LIGHT: &str = "#0284c7";

/// Infer skin from legacy content + brand.
///
/// Order (first match wins):
/// 1. Scuffed content markers → esports
/// 2. Competitive cyan brand → esports
/// 3. Else → clean (including product purple alone)
pub fn infer_home_skin(
    brand_accent_dark: &str,
    brand_accent_light: &str,
    content: &HomepageContent,
) -> HomeSkin {
    if looks_like_scuffed_content(content) {
        return HomeSkin::Esports;
    }
    if is_competitive_brand(brand_accent_dark, brand_accent_light) {
        return HomeSkin::Esports;
    }
    HomeSkin::Clean
}

fn looks_like_scuffed_content(content: &HomepageContent) -> bool {
    let title = content.hero_title.to_ascii_lowercase();
    if title.contains("scuffed") {
        return true;
    }
    let footer = content.footer_note.to_ascii_lowercase();
    if footer.contains("scuffed crew") {
        return true;
    }
    false
}

fn is_competitive_brand(dark: &str, light: &str) -> bool {
    hex_eq(dark, COMPETITIVE_DARK)
        || hex_eq(dark, COMPETITIVE_LIGHT)
        || hex_eq(light, COMPETITIVE_DARK)
        || hex_eq(light, COMPETITIVE_LIGHT)
}

/// Infer shell from legacy `public_layout` string.
pub fn infer_home_shell_from_public_layout(public_layout: &str) -> HomeShell {
    HomeShell::from_public_layout(PublicLayout::from_str_lossy(public_layout))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::org::homepage::HomepagePreset;

    #[test]
    fn ops_hub_order_matches_current_home() {
        assert_eq!(
            HomeShell::OpsHub.section_order(),
            &[
                HomeSectionId::Ethos,
                HomeSectionId::Live,
                HomeSectionId::Teams,
                HomeSectionId::News,
                HomeSectionId::Recruit,
            ]
        );
    }

    #[test]
    fn show_when_empty_ops_hub_teams_always() {
        assert!(HomeShell::OpsHub.show_when_empty(HomeSectionId::Teams));
        assert!(!HomeShell::OpsHub.show_when_empty(HomeSectionId::Live));
        assert!(!HomeShell::OpsHub.show_when_empty(HomeSectionId::News));
    }

    #[test]
    fn show_when_empty_minimal_hides_empty_teams() {
        assert!(!HomeShell::Minimal.show_when_empty(HomeSectionId::Teams));
    }

    #[test]
    fn show_when_empty_manifesto_hybrid() {
        assert!(!HomeShell::Manifesto.show_when_empty(HomeSectionId::Live));
        assert!(HomeShell::Manifesto.show_when_empty(HomeSectionId::News));
    }

    #[test]
    fn dual_write_layout_mapping() {
        assert_eq!(HomeShell::OpsHub.to_public_layout(), PublicLayout::Hub);
        assert_eq!(HomeShell::Minimal.to_public_layout(), PublicLayout::Hub);
        assert_eq!(
            HomeShell::RecruitLanding.to_public_layout(),
            PublicLayout::Landing
        );
        assert_eq!(
            HomeShell::Manifesto.to_public_layout(),
            PublicLayout::Landing
        );
    }

    #[test]
    fn org_initials_basic() {
        assert_eq!(org_initials("The Scuffed Crew"), "SC");
        assert_eq!(org_initials("My Clan"), "MC");
        assert_eq!(org_initials("Alpha"), "AL");
        assert_eq!(org_initials(""), "CL");
        assert_eq!(org_initials("!!!"), "CL");
        assert_eq!(org_initials("the"), "CL");
    }

    #[test]
    fn org_initials_skips_punctuation() {
        assert_eq!(org_initials("Foo-Bar Baz"), "FB");
    }

    #[test]
    fn skin_backfill_product_purple_alone_is_clean() {
        let neutral = HomepagePreset::neutral().content;
        let skin = infer_home_skin("#8f73ff", "#6d4aff", &neutral);
        assert_eq!(skin, HomeSkin::Clean);
    }

    #[test]
    fn skin_backfill_scuffed_content_is_esports() {
        let scuffed = HomepagePreset::scuffed().content;
        let skin = infer_home_skin("#8f73ff", "#6d4aff", &scuffed);
        assert_eq!(skin, HomeSkin::Esports);
    }

    #[test]
    fn skin_backfill_competitive_cyan_is_esports() {
        let competitive = HomepagePreset::competitive().content;
        let skin = infer_home_skin("#38bdf8", "#0284c7", &competitive);
        assert_eq!(skin, HomeSkin::Esports);
    }

    #[test]
    fn shell_skin_strict_parse() {
        assert_eq!(
            HomeShell::from_str_strict("ops_hub"),
            Some(HomeShell::OpsHub)
        );
        assert_eq!(HomeShell::from_str_strict("nope"), None);
        assert_eq!(
            HomeSkin::from_str_strict("esports"),
            Some(HomeSkin::Esports)
        );
        assert_eq!(HomeSkin::from_str_strict("neon"), None);
    }
}
