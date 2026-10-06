//! Hero names and roles for the strategy editor.
//!
//! Names come from [`scuffed_types::HEROES`]. Roles come from
//! [`scuffed_types::role_for_hero_name`]. Icon ids stay the kebab form the
//! assets use (`/assets/heroes/{id}/icon.webp`).

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

pub(super) fn roster() -> Vec<CatalogHero> {
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
}

pub(super) fn heroes_for_role(role: HeroRole) -> Vec<CatalogHero> {
    roster()
        .into_iter()
        .filter(|hero| hero.role == role)
        .collect()
}

pub(super) fn hero_by_id(id: &str) -> Option<CatalogHero> {
    roster().into_iter().find(|hero| hero.id == id)
}

pub(super) fn icon_path(id: &str) -> String {
    format!("/assets/heroes/{id}/icon.webp")
}
