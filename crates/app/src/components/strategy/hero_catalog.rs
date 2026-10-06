//! Hero names and roles for the strategy editor.
//!
//! Names come from [`scuffed_types::HEROES`]. Roles come from
//! [`scuffed_types::role_for_hero_name`]. `hero_id` is the kebab form the
//! picker and team panel request (`/assets/heroes/{id}/icon.webp`). The
//! canvas requests a different file, `/assets/heroes/{id}.png`.

use std::sync::LazyLock;

use scuffed_types::HeroRole;

pub(super) struct CatalogHero {
    pub id: String,
    pub name: &'static str,
    pub role: HeroRole,
}

/// Asset id for a display name. `D.Va` → `dva`, `Soldier: 76` → `soldier-76`.
pub(super) fn hero_id(name: &str) -> String {
    name.to_lowercase()
        .replace(".", "")
        .replace(": ", "-")
        .replace(" ", "-")
        .replace("ö", "o")
        .replace("ú", "u")
}

static ROSTER: LazyLock<Vec<CatalogHero>> = LazyLock::new(|| {
    scuffed_types::HEROES
        .iter()
        .filter_map(|name| {
            let role = scuffed_types::role_for_hero_name(name)?;
            Some(CatalogHero {
                id: hero_id(name),
                name,
                role,
            })
        })
        .collect()
});

pub(super) fn heroes_for_role(role: HeroRole) -> impl Iterator<Item = &'static CatalogHero> {
    ROSTER.iter().filter(move |hero| hero.role == role)
}

pub(super) fn hero_by_id(id: &str) -> Option<&'static CatalogHero> {
    ROSTER.iter().find(|hero| hero.id == id)
}

pub(super) fn icon_path(id: &str) -> String {
    format!("/assets/heroes/{id}/icon.webp")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_strategy_ids_stay_stable() {
        let cases = [
            ("D.Va", "dva"),
            ("Junker Queen", "junker-queen"),
            ("Wrecking Ball", "wrecking-ball"),
            ("Soldier: 76", "soldier-76"),
            ("Torbjorn", "torbjorn"),
            ("Lucio", "lucio"),
            ("D.Mon", "dmon"),
            ("Jetpack Cat", "jetpack-cat"),
        ];
        for (name, id) in cases {
            assert_eq!(hero_id(name), id, "{name}");
            let found = hero_by_id(id).unwrap_or_else(|| panic!("{id} is not on the roster"));
            assert_eq!(found.name, name);
        }
    }
}
