use dioxus::prelude::*;
use scuffed_types::HeroRole;

const HERO_PICKER_CSS: &str = r##"
    .hero-picker {
        display: flex;
        flex-direction: column;
        background: var(--surface);
        overflow-y: auto;
    }
    .hero-picker .panel-title {
        font-family: var(--font-head);
        font-size: 0.75rem;
        color: var(--text-3);
        text-transform: uppercase;
        letter-spacing: 0.08em;
        padding: 0.75rem 0.75rem 0.5rem;
        margin: 0;
        border-bottom: 1px solid var(--border);
    }

    /* ---- Role sections ---- */
    .hero-role-section {
        border-bottom: 1px solid var(--border);
    }
    .role-header {
        display: flex;
        align-items: center;
        width: 100%;
        padding: 0.5rem 0.75rem;
        background: none;
        border: none;
        border-left: 3px solid transparent;
        color: var(--text-2);
        font-size: 0.8rem;
        cursor: pointer;
        transition: background 0.12s;
    }
    .role-header:hover {
        background: var(--surface-2);
    }
    .role-header.expanded {
        background: var(--surface-2);
    }
    .role-name {
        flex: 1;
        text-align: left;
        font-weight: 600;
    }
    .role-count {
        font-size: 0.7rem;
        color: var(--text-3);
        margin-right: 0.5rem;
    }
    .expand-icon {
        font-size: 0.65rem;
        color: var(--text-3);
    }

    /* ---- Hero grid ---- */
    .hero-grid {
        display: grid;
        grid-template-columns: repeat(auto-fill, minmax(64px, 1fr));
        gap: 0.25rem;
        padding: 0.5rem;
    }
    .hero-btn {
        display: flex;
        flex-direction: column;
        align-items: center;
        gap: 0.2rem;
        padding: 0.35rem;
        background: none;
        border: 1px solid transparent;
        border-radius: 6px;
        color: var(--text-2);
        cursor: pointer;
        transition: background 0.12s, border-color 0.12s;
    }
    .hero-btn:hover {
        background: var(--surface-2);
    }
    .hero-btn.selected {
        background: var(--accent-soft);
        border-color: var(--accent);
    }
    .hero-icon {
        width: 36px;
        height: 36px;
        border-radius: 50%;
        object-fit: cover;
        background: var(--surface-2);
    }
    .hero-name-short {
        font-size: 0.6rem;
        text-align: center;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
        max-width: 60px;
    }

    /* ---- Selected hero info ---- */
    .selected-hero-info {
        border-top: 1px solid var(--border);
        padding: 0.75rem;
    }
    .hero-details {
        display: flex;
        align-items: center;
        gap: 0.5rem;
        margin-bottom: 0.5rem;
    }
    .hero-detail-icon {
        width: 40px;
        height: 40px;
        border-radius: 50%;
        object-fit: cover;
        background: var(--surface-2);
    }
    .hero-detail-info h4 {
        margin: 0;
        font-size: 0.9rem;
        color: var(--text);
    }
    .hero-role-badge {
        font-size: 0.7rem;
        font-weight: 600;
    }
    .hero-abilities {
        display: flex;
        flex-direction: column;
        gap: 0.25rem;
    }
    .ability-item {
        display: flex;
        align-items: center;
        gap: 0.35rem;
        padding: 0.2rem 0.3rem;
        border-radius: 4px;
        font-size: 0.7rem;
        color: var(--text-2);
    }
    .ability-item:hover {
        background: var(--surface-2);
    }
    .ability-key {
        display: inline-flex;
        align-items: center;
        justify-content: center;
        width: 18px;
        height: 18px;
        background: var(--surface-2);
        border: 1px solid var(--border);
        border-radius: 3px;
        font-size: 0.6rem;
        font-weight: 700;
        color: var(--text-3);
    }
    .ability-name {
        flex: 1;
    }
    .ability-cooldown {
        color: var(--text-3);
        font-size: 0.65rem;
    }
    .hero-wr-badge {
        font-size: 0.55rem;
        font-weight: 700;
        padding: 0.05rem 0.2rem;
        border-radius: 3px;
        line-height: 1;
    }
    .hero-wr-badge.high { color: var(--ok); background: color-mix(in srgb, var(--ok) 12%, transparent); }
    .hero-wr-badge.mid { color: var(--warn); background: color-mix(in srgb, var(--warn) 12%, transparent); }
    .hero-wr-badge.low { color: var(--danger); background: color-mix(in srgb, var(--danger) 12%, transparent); }
"##;

/// Hero definition for the picker UI.
#[derive(Clone, PartialEq)]
struct HeroDef {
    id: &'static str,
    name: &'static str,
    role: HeroRole,
    icon_path: String,
    abilities: Vec<AbilityDef>,
}

#[derive(Clone, PartialEq)]
struct AbilityDef {
    key: &'static str,
    name: &'static str,
    description: &'static str,
    cooldown: Option<f32>,
}

/// Returns the canonical hero roster grouped by role.
fn heroes_by_role(role: HeroRole) -> Vec<HeroDef> {
    let roster: &[(&str, &str)] = match role {
        HeroRole::Tank => &[
            ("dva", "D.Va"),
            ("domina", "Domina"),
            ("doomfist", "Doomfist"),
            ("hazard", "Hazard"),
            ("junker-queen", "Junker Queen"),
            ("mauga", "Mauga"),
            ("orisa", "Orisa"),
            ("ramattra", "Ramattra"),
            ("reinhardt", "Reinhardt"),
            ("roadhog", "Roadhog"),
            ("sigma", "Sigma"),
            ("winston", "Winston"),
            ("wrecking-ball", "Wrecking Ball"),
            ("zarya", "Zarya"),
        ],
        HeroRole::Damage => &[
            ("anran", "Anran"),
            ("ashe", "Ashe"),
            ("bastion", "Bastion"),
            ("cassidy", "Cassidy"),
            ("echo", "Echo"),
            ("emre", "Emre"),
            ("freja", "Freja"),
            ("genji", "Genji"),
            ("hanzo", "Hanzo"),
            ("junkrat", "Junkrat"),
            ("mei", "Mei"),
            ("pharah", "Pharah"),
            ("reaper", "Reaper"),
            ("sierra", "Sierra"),
            ("sojourn", "Sojourn"),
            ("soldier-76", "Soldier: 76"),
            ("sombra", "Sombra"),
            ("symmetra", "Symmetra"),
            ("torbjorn", "Torbjorn"),
            ("tracer", "Tracer"),
            ("vendetta", "Vendetta"),
            ("venture", "Venture"),
            ("widowmaker", "Widowmaker"),
        ],
        HeroRole::Support => &[
            ("ana", "Ana"),
            ("baptiste", "Baptiste"),
            ("brigitte", "Brigitte"),
            ("illari", "Illari"),
            ("juno", "Juno"),
            ("kiriko", "Kiriko"),
            ("lifeweaver", "Lifeweaver"),
            ("lucio", "Lucio"),
            ("mercy", "Mercy"),
            ("mizuki", "Mizuki"),
            ("moira", "Moira"),
            ("wuyang", "Wuyang"),
            ("zenyatta", "Zenyatta"),
        ],
    };

    roster
        .iter()
        .map(|(id, name)| HeroDef {
            id,
            name,
            role,
            icon_path: format!("/assets/heroes/{id}/icon.webp"),
            abilities: Vec::new(), // abilities are populated when selected
        })
        .collect()
}

/// Returns the full hero definition with abilities for the info panel.
fn hero_by_id(id: &str) -> Option<HeroDef> {
    // Walk all roles to find the hero
    for role in [HeroRole::Tank, HeroRole::Damage, HeroRole::Support] {
        if let Some(hero) = heroes_by_role(role).into_iter().find(|h| h.id == id) {
            return Some(hero);
        }
    }
    None
}

/// Hero winrate entry for display in the picker.
#[derive(Clone, PartialEq)]
pub struct HeroWinRate {
    pub hero_name: String,
    pub winrate: f64,
}

fn normalize_hero_id(name: &str) -> String {
    name.to_lowercase()
        .replace(".", "")
        .replace(": ", "-")
        .replace(" ", "-")
        .replace("ö", "o")
        .replace("ú", "u")
}

fn wr_badge_class(pct: f64) -> &'static str {
    if pct >= 55.0 {
        "hero-wr-badge high"
    } else if pct >= 45.0 {
        "hero-wr-badge mid"
    } else {
        "hero-wr-badge low"
    }
}

#[component]
pub fn HeroPicker(
    /// Currently selected hero ID.
    #[props(default)]
    selected_hero: Option<String>,

    /// Fired when a hero is clicked.
    on_select: EventHandler<String>,

    /// Optional personal winrate data per hero.
    #[props(default)]
    hero_winrates: Option<Vec<HeroWinRate>>,
) -> Element {
    let mut expanded_role = use_signal(|| Option::<HeroRole>::None);

    rsx! {
        style { {HERO_PICKER_CSS} }
        div { class: "hero-picker",
            h3 { class: "panel-title", "Heroes" }

            // Role sections
            {[HeroRole::Tank, HeroRole::Damage, HeroRole::Support].iter().map(|role| {
                let role = *role;
                let heroes = heroes_by_role(role);
                let hero_count = heroes.len();
                let role_name = role.to_string();
                let role_color = role.color_hex();
                let is_expanded = expanded_role() == Some(role);
                let header_cls = if is_expanded { "role-header expanded" } else { "role-header" };

                rsx! {
                    div { class: "hero-role-section",
                        button {
                            class: "{header_cls}",
                            style: "border-left-color: {role_color};",
                            onclick: move |_| {
                                expanded_role.set(
                                    if expanded_role() == Some(role) { None } else { Some(role) }
                                );
                            },
                            span { class: "role-name", "{role_name}" }
                            span { class: "role-count", "{hero_count}" }
                            span { class: "expand-icon",
                                if is_expanded { "\u{25bc}" } else { "\u{25b6}" }
                            }
                        }

                        if is_expanded {
                            div { class: "hero-grid",
                                {heroes.iter().map(|hero| {
                                    let hero_id = hero.id.to_string();
                                    let hero_id_click = hero_id.clone();
                                    let hero_name = hero.name;
                                    let icon = hero.icon_path.clone();
                                    let is_selected = selected_hero.as_deref() == Some(hero.id);
                                    let btn_cls = if is_selected { "hero-btn selected" } else { "hero-btn" };

                                    let wr = hero_winrates.as_ref().and_then(|rates| {
                                        let norm_id = hero.id;
                                        rates.iter().find(|r| normalize_hero_id(&r.hero_name) == norm_id)
                                    });

                                    rsx! {
                                        button {
                                            class: "{btn_cls}",
                                            title: "{hero_name}",
                                            onclick: move |_| on_select.call(hero_id_click.clone()),
                                            img {
                                                class: "hero-icon",
                                                src: "{icon}",
                                                alt: "{hero_name}",
                                            }
                                            span { class: "hero-name-short", "{hero_name}" }
                                            if let Some(wr) = wr {
                                                {
                                                    let cls = wr_badge_class(wr.winrate);
                                                    let val = wr.winrate;
                                                    rsx! {
                                                        span { class: "{cls}", "{val:.0}%" }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                })}
                            }
                        }
                    }
                }
            })}

            // Selected hero info panel
            if let Some(ref hero_id) = selected_hero {
                if let Some(hero) = hero_by_id(hero_id) {
                    div { class: "selected-hero-info",
                        div { class: "hero-details",
                            img {
                                class: "hero-detail-icon",
                                src: "{hero.icon_path}",
                                alt: "{hero.name}",
                            }
                            div { class: "hero-detail-info",
                                h4 { "{hero.name}" }
                                span {
                                    class: "hero-role-badge",
                                    style: "color: {hero.role.color_hex()};",
                                    "{hero.role}"
                                }
                            }
                        }
                        if !hero.abilities.is_empty() {
                            div { class: "hero-abilities",
                                {hero.abilities.iter().map(|ability| {
                                    rsx! {
                                        div {
                                            class: "ability-item",
                                            title: "{ability.description}",
                                            span { class: "ability-key", "{ability.key}" }
                                            span { class: "ability-name", "{ability.name}" }
                                            if let Some(cd) = ability.cooldown {
                                                span { class: "ability-cooldown", "{cd:.0}s" }
                                            }
                                        }
                                    }
                                })}
                            }
                        }
                    }
                }
            }
        }
    }
}
