//! Hero list types for the Heroes page.
//!
//! `GET /api/strategy/heroes` (API PR 502) sends `{ "data": [...] }` with
//! `id`, `name`, `role` and `portrait_url` for every hero. Abilities and the
//! health, armor and shields numbers are optional: the shared list does not
//! send them yet, so each one decodes to "not there" when missing or null and
//! the page leaves its section out.

use serde::{Deserialize, Deserializer};

/// The role columns the page groups by, in display order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RoleGroup {
    Tank,
    Damage,
    Support,
    /// A role the page does not know. Shown last instead of being filed
    /// under Damage, so a new role is visible but not mislabelled.
    Other,
}

impl RoleGroup {
    pub(super) const ORDER: [RoleGroup; 4] = [
        RoleGroup::Tank,
        RoleGroup::Damage,
        RoleGroup::Support,
        RoleGroup::Other,
    ];

    /// `role` is already trimmed and lowercased by [`Hero`]'s decoder.
    pub(super) fn from_role(role: &str) -> Self {
        match role {
            "tank" => RoleGroup::Tank,
            "damage" => RoleGroup::Damage,
            "support" => RoleGroup::Support,
            _ => RoleGroup::Other,
        }
    }

    pub(super) fn label(self) -> &'static str {
        match self {
            RoleGroup::Tank => "Tank",
            RoleGroup::Damage => "Damage",
            RoleGroup::Support => "Support",
            RoleGroup::Other => "Other",
        }
    }

    pub(super) fn color(self) -> &'static str {
        match self {
            RoleGroup::Tank => "var(--chart-5)",
            RoleGroup::Damage => "var(--danger)",
            RoleGroup::Support => "var(--ok)",
            RoleGroup::Other => "var(--text-3)",
        }
    }

    /// Soft role tint for badges and portrait fallbacks.
    pub(super) fn tint(self) -> &'static str {
        match self {
            RoleGroup::Tank => "color-mix(in srgb, var(--chart-5) 12%, transparent)",
            RoleGroup::Damage => "color-mix(in srgb, var(--danger) 12%, transparent)",
            RoleGroup::Support => "color-mix(in srgb, var(--ok) 12%, transparent)",
            RoleGroup::Other => "color-mix(in srgb, var(--text-3) 12%, transparent)",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub(super) struct Hero {
    pub id: String,
    pub name: String,
    /// "tank", "damage" or "support". Lowercased on decode so "Support"
    /// from an older payload groups the same as "support".
    #[serde(deserialize_with = "lowercase_role")]
    pub role: String,
    /// Empty today: game art is not in git, so the page draws a fallback.
    #[serde(default)]
    pub portrait_url: Option<String>,
    #[serde(default, deserialize_with = "null_as_default")]
    pub abilities: Vec<Ability>,
    #[serde(default)]
    pub health: Option<u32>,
    #[serde(default)]
    pub armor: Option<u32>,
    #[serde(default)]
    pub shields: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub(super) struct Ability {
    pub name: String,
    /// "LMB", "RMB", "Shift", "E", "Q" or "Passive". May be missing.
    #[serde(default, deserialize_with = "null_as_default")]
    pub key: String,
    #[serde(default, deserialize_with = "null_as_default")]
    pub description: String,
    #[serde(default)]
    pub cooldown: Option<f32>,
    #[serde(default)]
    pub icon_url: Option<String>,
}

/// `{ "data": [...] }` envelope, same unwrap as Browse and My Strategies.
/// `data` is required so a bare `[]` or `{}` cannot look like an empty list.
#[derive(Debug, Clone, Deserialize)]
pub(super) struct ListResponse {
    #[serde(alias = "heroes")]
    pub data: Vec<Hero>,
}

fn lowercase_role<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    String::deserialize(deserializer).map(|role| role.trim().to_ascii_lowercase())
}

/// A `null` reads the same as a missing field.
fn null_as_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Option::unwrap_or_default)
}

impl Hero {
    pub(super) fn group(&self) -> RoleGroup {
        RoleGroup::from_role(&self.role)
    }

    /// The portrait URL, or `None` when it is missing or blank.
    pub(super) fn portrait(&self) -> Option<&str> {
        self.portrait_url
            .as_deref()
            .map(str::trim)
            .filter(|url| !url.is_empty())
    }

    pub(super) fn initial(&self) -> String {
        self.name
            .chars()
            .find(|c| c.is_alphanumeric())
            .map(|c| c.to_uppercase().to_string())
            .unwrap_or_else(|| "?".to_string())
    }

    pub(super) fn total_hp(&self) -> u32 {
        [self.health, self.armor, self.shields]
            .into_iter()
            .flatten()
            .sum()
    }

    /// The HP bar is only drawn when at least one number is above zero.
    pub(super) fn has_hp_detail(&self) -> bool {
        self.total_hp() > 0
    }

    pub(super) fn has_abilities(&self) -> bool {
        !self.abilities.is_empty()
    }
}

/// Heroes whose name contains `query` (case-insensitive), grouped by role in
/// [`RoleGroup::ORDER`] and sorted by name. Empty groups are left out.
pub(super) fn group_heroes<'a>(heroes: &'a [Hero], query: &str) -> Vec<(RoleGroup, Vec<&'a Hero>)> {
    let query = query.trim().to_lowercase();
    RoleGroup::ORDER
        .into_iter()
        .filter_map(|group| {
            let mut members: Vec<&Hero> = heroes
                .iter()
                .filter(|h| h.group() == group)
                .filter(|h| query.is_empty() || h.name.to_lowercase().contains(&query))
                .collect();
            members.sort_by_key(|h| h.name.to_lowercase());
            (!members.is_empty()).then_some((group, members))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hero(id: &str, name: &str, role: &str) -> Hero {
        let body = format!(r#"{{"id":"{id}","name":"{name}","role":"{role}","portrait_url":""}}"#);
        serde_json::from_str(&body).unwrap()
    }

    #[test]
    fn list_envelope_unwraps_data_not_bare_vec() {
        let empty: ListResponse = serde_json::from_str(r#"{"data":[]}"#).unwrap();
        assert!(empty.data.is_empty());

        let aliased: ListResponse = serde_json::from_str(r#"{"heroes":[]}"#).unwrap();
        assert!(aliased.data.is_empty());

        let empty_obj: Result<ListResponse, _> = serde_json::from_str("{}");
        assert!(empty_obj.is_err(), "missing data key is not an empty list");

        let bare_empty: Result<ListResponse, _> = serde_json::from_str("[]");
        assert!(
            bare_empty.is_err(),
            "the list is an envelope, not a bare Vec"
        );
    }

    #[test]
    fn four_field_hero_from_shared_list_decodes() {
        let body =
            r#"{"data":[{"id":"doctrine","name":"Doctrine","role":"support","portrait_url":""}]}"#;
        let list: ListResponse = serde_json::from_str(body).unwrap();
        let hero = &list.data[0];
        assert_eq!(hero.id, "doctrine");
        assert_eq!(hero.group(), RoleGroup::Support);
        assert_eq!(hero.portrait(), None, "blank URL means no portrait");
        assert_eq!(hero.health, None);
        assert!(!hero.has_hp_detail(), "no HP bar for a list-only hero");
        assert!(!hero.has_abilities(), "no Abilities heading either");
    }

    #[test]
    fn missing_or_null_optional_fields_decode() {
        let missing = r#"{"id":"ana","name":"Ana","role":"support"}"#;
        let hero: Hero = serde_json::from_str(missing).unwrap();
        assert_eq!(hero.portrait(), None);
        assert!(!hero.has_abilities());

        let nulls = r#"{"id":"ana","name":"Ana","role":"support","portrait_url":null,
            "abilities":null,"health":null,"armor":null,"shields":null}"#;
        let hero: Hero = serde_json::from_str(nulls).unwrap();
        assert_eq!(hero.portrait(), None);
        assert!(!hero.has_abilities());
        assert!(!hero.has_hp_detail());
    }

    #[test]
    fn any_hp_field_shows_the_hp_bar() {
        for field in ["health", "armor", "shields"] {
            let body =
                format!(r#"{{"id":"x","name":"X","role":"tank","portrait_url":"","{field}":100}}"#);
            let hero: Hero = serde_json::from_str(&body).unwrap();
            assert!(hero.has_hp_detail(), "{field}");
            assert_eq!(hero.total_hp(), 100);
        }
    }

    #[test]
    fn full_hero_shape_still_decodes() {
        let body = r#"{
            "id": "ana",
            "name": "Ana",
            "role": "support",
            "portrait_url": "/img/ana.png",
            "abilities": [{
                "name": "Sleep Dart",
                "key": "Shift",
                "description": "Puts an enemy to sleep.",
                "cooldown": 14.0,
                "icon_url": null
            }],
            "health": 250,
            "armor": 0,
            "shields": 0
        }"#;
        let hero: Hero = serde_json::from_str(body).unwrap();
        assert_eq!(hero.portrait(), Some("/img/ana.png"));
        assert_eq!(hero.abilities[0].key, "Shift");
        assert_eq!(hero.abilities[0].cooldown, Some(14.0));
        assert_eq!(hero.total_hp(), 250);
        assert!(hero.has_hp_detail());
        assert!(hero.has_abilities());
    }

    #[test]
    fn an_ability_with_only_a_name_decodes() {
        let body = r#"{"id":"x","name":"X","role":"tank","abilities":[{"name":"Charge"}]}"#;
        let hero: Hero = serde_json::from_str(body).unwrap();
        assert_eq!(hero.abilities[0].name, "Charge");
        assert_eq!(hero.abilities[0].key, "");
        assert_eq!(hero.abilities[0].description, "");
        assert_eq!(hero.abilities[0].cooldown, None);
    }

    #[test]
    fn capitalised_role_is_lowercased() {
        let body = r#"{"id":"reinhardt","name":"Reinhardt","role":" Tank ","portrait_url":""}"#;
        let hero: Hero = serde_json::from_str(body).unwrap();
        assert_eq!(hero.role, "tank");
        assert_eq!(hero.group().label(), "Tank");
    }

    #[test]
    fn groups_follow_role_order_and_sort_by_name() {
        let heroes = vec![
            hero("zenyatta", "Zenyatta", "support"),
            hero("ana", "Ana", "support"),
            hero("tracer", "Tracer", "damage"),
            hero("dva", "D.Va", "tank"),
            hero("new", "Newcomer", "flex"),
        ];
        let groups = group_heroes(&heroes, "");
        let labels: Vec<&str> = groups.iter().map(|(g, _)| g.label()).collect();
        assert_eq!(labels, ["Tank", "Damage", "Support", "Other"]);
        let support: Vec<&str> = groups[2].1.iter().map(|h| h.name.as_str()).collect();
        assert_eq!(support, ["Ana", "Zenyatta"]);
        assert_eq!(
            groups[3].1[0].id, "new",
            "unknown role is not filed as Damage"
        );
    }

    #[test]
    fn search_filters_and_drops_empty_groups() {
        let heroes = vec![hero("ana", "Ana", "support"), hero("dva", "D.Va", "tank")];
        let groups = group_heroes(&heroes, "  AN ");
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].0, RoleGroup::Support);
        assert!(group_heroes(&heroes, "zzz").is_empty());
    }

    #[test]
    fn initial_skips_punctuation() {
        assert_eq!(hero("dva", "D.Va", "tank").initial(), "D");
        assert_eq!(hero("x", "", "tank").initial(), "?");
    }
}
