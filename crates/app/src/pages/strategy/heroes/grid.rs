//! Left panel: the hero grid, grouped by role.

use dioxus::prelude::*;

use super::model::{Hero, RoleGroup, group_heroes};
use super::portrait::HeroPortrait;

pub(super) fn render_hero_grid(
    heroes: &[Hero],
    query: &str,
    selected_hero: Signal<Option<Hero>>,
) -> Element {
    let groups = group_heroes(heroes, query);
    if groups.is_empty() {
        return rsx! {
            p { class: "heroes-empty", "No heroes match your search." }
        };
    }

    let selected_id = selected_hero
        .read()
        .as_ref()
        .map(|h| h.id.clone())
        .unwrap_or_default();

    rsx! {
        for (group, members) in groups.iter() {
            {render_role_section(*group, members, &selected_id, selected_hero)}
        }
    }
}

fn render_role_section(
    group: RoleGroup,
    heroes: &[&Hero],
    selected_id: &str,
    selected_hero: Signal<Option<Hero>>,
) -> Element {
    let label = group.label();
    let color = group.color();

    rsx! {
        section { key: "{label}", class: "heroes-role", aria_label: "{label} heroes",
            h2 { class: "heroes-role-heading", style: "color: {color};", "{label}" }
            div { class: "heroes-grid",
                for hero in heroes.iter() {
                    {render_hero_button(hero, selected_id, selected_hero)}
                }
            }
        }
    }
}

fn render_hero_button(
    hero: &Hero,
    selected_id: &str,
    mut selected_hero: Signal<Option<Hero>>,
) -> Element {
    let is_selected = hero.id == selected_id;
    let btn_class = if is_selected {
        "hero-btn selected"
    } else {
        "hero-btn"
    };
    let pressed = if is_selected { "true" } else { "false" };
    let hero_clone = hero.clone();

    rsx! {
        button {
            key: "{hero.id}",
            r#type: "button",
            class: "{btn_class}",
            aria_pressed: "{pressed}",
            onclick: move |_| selected_hero.set(Some(hero_clone.clone())),
            HeroPortrait {
                url: hero.portrait().map(str::to_string),
                name: hero.name.clone(),
                initial: hero.initial(),
                group: hero.group(),
                class: "hero-btn-portrait",
                initial_class: "hero-btn-initial",
            }
            span { class: "hero-btn-name", "{hero.name}" }
        }
    }
}
