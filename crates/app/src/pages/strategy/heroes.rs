use dioxus::prelude::*;
use serde::Deserialize;

use scuffed_api_client::ApiClient;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Deserialize)]
struct Hero {
    id: String,
    name: String,
    role: String, // "tank", "damage", "support"
    portrait_url: String,
    abilities: Vec<Ability>,
    health: u32,
    armor: u32,
    shields: u32,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
struct Ability {
    name: String,
    key: String, // "LMB", "RMB", "Shift", "E", "Q", "Passive"
    description: String,
    cooldown: Option<f32>,
    icon_url: Option<String>,
}

impl Hero {
    fn total_hp(&self) -> u32 {
        self.health + self.armor + self.shields
    }

    fn role_label(&self) -> &str {
        match self.role.as_str() {
            "tank" => "Tank",
            "damage" => "Damage",
            "support" => "Support",
            _ => &self.role,
        }
    }

    fn role_color(&self) -> &str {
        match self.role.as_str() {
            "tank" => "#3b82f6",
            "damage" => "#ef4444",
            "support" => "#22c55e",
            _ => "#94a3b8",
        }
    }

    fn initial(&self) -> String {
        self.name
            .chars()
            .next()
            .map(|c| c.to_uppercase().to_string())
            .unwrap_or_default()
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn role_bg(role: &str) -> &str {
    match role {
        "tank" => "rgba(59, 130, 246, 0.12)",
        "damage" => "rgba(239, 68, 68, 0.12)",
        "support" => "rgba(34, 197, 94, 0.12)",
        _ => "rgba(148, 163, 184, 0.12)",
    }
}

fn role_heading_color(role: &str) -> &str {
    match role {
        "tank" => "#60a5fa",
        "damage" => "#f87171",
        "support" => "#4ade80",
        _ => "#94a3b8",
    }
}

// ---------------------------------------------------------------------------
// CSS
// ---------------------------------------------------------------------------

const PAGE_CSS: &str = r#"
.heroes-page { display:flex; height:100%; min-height:calc(100vh - 60px); }

/* Left panel */
.heroes-left { width:340px; min-width:280px; border-right:1px solid var(--border); overflow-y:auto; padding:1.25rem; background:var(--bg-surface); flex-shrink:0; }
.heroes-search { width:100%; padding:0.5rem 0.75rem; border:1px solid var(--border); border-radius:6px; background:var(--bg-card); color:var(--text-bright); font-family:var(--font-body); font-size:0.85rem; outline:none; margin-bottom:1.25rem; transition:border-color 0.2s; }
.heroes-search:focus { border-color:var(--accent); }
.heroes-search::placeholder { color:var(--text-muted); }
.heroes-role-heading { font-family:var(--font-display); font-size:0.75rem; font-weight:700; text-transform:uppercase; letter-spacing:0.08em; margin:1rem 0 0.5rem; padding-bottom:0.35rem; border-bottom:1px solid var(--border); }
.heroes-role-heading:first-of-type { margin-top:0; }
.heroes-grid { display:grid; grid-template-columns:repeat(auto-fill,minmax(90px,1fr)); gap:0.5rem; margin-bottom:0.75rem; }
.hero-btn { display:flex; flex-direction:column; align-items:center; gap:0.25rem; padding:0.5rem 0.25rem; border:1px solid var(--border); border-radius:8px; background:var(--bg-card); cursor:pointer; transition:border-color 0.15s,background 0.15s,transform 0.1s; text-align:center; }
.hero-btn:hover { border-color:var(--border-light); background:var(--bg-card-alt); transform:translateY(-1px); }
.hero-btn.selected { border-color:var(--accent); background:var(--accent-soft); box-shadow:0 0 0 1px var(--accent); }
.hero-btn-portrait { width:40px; height:40px; border-radius:50%; overflow:hidden; background:var(--bg-elevated); display:flex; align-items:center; justify-content:center; flex-shrink:0; }
.hero-btn-portrait img { width:100%; height:100%; object-fit:cover; }
.hero-btn-initial { font-family:var(--font-display-hero); font-size:1.1rem; color:var(--text-muted); line-height:1; }
.hero-btn-name { font-family:var(--font-display); font-size:0.7rem; font-weight:600; color:var(--text-primary); line-height:1.2; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; max-width:100%; }

/* Right panel */
.heroes-right { flex:1; overflow-y:auto; padding:2rem; min-width:0; }
.heroes-placeholder { display:flex; align-items:center; justify-content:center; height:100%; min-height:400px; color:var(--text-muted); font-size:1rem; font-family:var(--font-display); text-transform:uppercase; letter-spacing:0.06em; }

/* Detail header */
.hero-detail-header { display:flex; align-items:center; gap:1.25rem; margin-bottom:1.75rem; }
.hero-detail-portrait { width:80px; height:80px; border-radius:12px; overflow:hidden; background:var(--bg-elevated); display:flex; align-items:center; justify-content:center; flex-shrink:0; }
.hero-detail-portrait img { width:100%; height:100%; object-fit:cover; }
.hero-detail-portrait-initial { font-family:var(--font-display-hero); font-size:2rem; color:var(--text-muted); }
.hero-detail-info { display:flex; flex-direction:column; gap:0.35rem; }
.hero-detail-name { font-family:var(--font-display-hero); font-size:2rem; color:var(--text-bright); letter-spacing:2px; line-height:1; }
.hero-detail-role-badge { display:inline-block; font-size:0.65rem; padding:0.15rem 0.6rem; border-radius:999px; font-weight:700; text-transform:uppercase; letter-spacing:0.05em; width:fit-content; }

/* HP breakdown */
.hero-hp-section { margin-bottom:2rem; }
.hero-hp-label { font-family:var(--font-display); font-size:0.75rem; font-weight:600; text-transform:uppercase; letter-spacing:0.06em; color:var(--text-muted); margin-bottom:0.5rem; }
.hero-hp-bar { display:flex; height:18px; border-radius:4px; overflow:hidden; background:var(--bg-elevated); margin-bottom:0.4rem; }
.hero-hp-segment { display:flex; align-items:center; justify-content:center; font-size:0.6rem; font-weight:700; font-family:var(--font-mono); color:#000; min-width:24px; }
.hero-hp-legend { display:flex; gap:1rem; font-size:0.7rem; color:var(--text-secondary); font-family:var(--font-mono); }
.hero-hp-legend span { display:flex; align-items:center; gap:0.3rem; }
.hero-hp-dot { width:8px; height:8px; border-radius:2px; display:inline-block; }

/* Abilities */
.hero-abilities-title { font-family:var(--font-display); font-size:0.9rem; font-weight:700; text-transform:uppercase; letter-spacing:0.06em; color:var(--text-bright); margin-bottom:0.75rem; padding-bottom:0.4rem; border-bottom:1px solid var(--border); }
.hero-abilities-list { display:flex; flex-direction:column; gap:0.75rem; }
.ability-card { display:flex; gap:0.75rem; padding:0.85rem 1rem; background:var(--bg-card); border:1px solid var(--border); border-radius:8px; transition:border-color 0.15s; }
.ability-card:hover { border-color:var(--border-light); }
.ability-key-badge { flex-shrink:0; width:42px; height:34px; display:flex; align-items:center; justify-content:center; border-radius:5px; background:var(--bg-elevated); border:1px solid var(--border-light); font-family:var(--font-mono); font-size:0.65rem; font-weight:700; color:var(--text-bright); text-transform:uppercase; letter-spacing:0.03em; }
.ability-body { flex:1; min-width:0; }
.ability-header { display:flex; align-items:baseline; gap:0.5rem; margin-bottom:0.25rem; }
.ability-name { font-family:var(--font-display); font-size:0.85rem; font-weight:700; color:var(--text-bright); }
.ability-cooldown { font-family:var(--font-mono); font-size:0.65rem; color:var(--text-muted); }
.ability-desc { font-size:0.8rem; color:var(--text-secondary); line-height:1.5; }

/* States */
.heroes-loading, .heroes-empty { color:var(--text-muted); text-align:center; padding:3rem 0; }
.heroes-error { color:var(--danger); text-align:center; padding:3rem 0; font-size:0.85rem; }

/* Responsive */
@media (max-width:720px) {
    .heroes-page { flex-direction:column; }
    .heroes-left { width:100%; min-width:unset; border-right:none; border-bottom:1px solid var(--border); max-height:45vh; }
    .heroes-right { padding:1.25rem; }
}
"#;

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

#[component]
pub fn StrategyHeroes() -> Element {
    let mut selected_hero: Signal<Option<Hero>> = use_signal(|| None);
    let mut search_filter: Signal<String> = use_signal(String::new);

    let heroes = use_resource(|| async {
        ApiClient::web()
            .fetch::<Vec<Hero>>("/api/strategy/heroes")
            .await
    });

    rsx! {
        style { {PAGE_CSS} }

        div { class: "heroes-page",
            // ---- LEFT PANEL: hero grid ----
            div { class: "heroes-left",
                input {
                    class: "heroes-search",
                    r#type: "text",
                    placeholder: "Search heroes...",
                    value: "{search_filter}",
                    oninput: move |e| search_filter.set(e.value()),
                }

                {render_hero_grid(&heroes, &search_filter, &selected_hero)}
            }

            // ---- RIGHT PANEL: hero detail ----
            div { class: "heroes-right",
                if let Some(hero) = selected_hero() {
                    {render_hero_detail(&hero)}
                } else {
                    div { class: "heroes-placeholder",
                        "Select a hero to view details"
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Left panel: hero grid grouped by role
// ---------------------------------------------------------------------------

fn render_hero_grid(
    heroes: &Resource<Result<Vec<Hero>, scuffed_api_client::ClientError>>,
    search_filter: &Signal<String>,
    selected_hero: &Signal<Option<Hero>>,
) -> Element {
    let data = heroes.read();
    match data.as_ref() {
        None => rsx! { p { class: "heroes-loading", "Loading heroes..." } },
        Some(Err(_)) => rsx! {
            p { class: "heroes-error", "Failed to load hero data." }
        },
        Some(Ok(list)) if list.is_empty() => rsx! {
            p { class: "heroes-empty", "No heroes found." }
        },
        Some(Ok(list)) => {
            let query = search_filter.read().to_lowercase();
            let filtered: Vec<&Hero> = list
                .iter()
                .filter(|h| query.is_empty() || h.name.to_lowercase().contains(&query))
                .collect();

            // Group by role
            let mut tank: Vec<&Hero> = Vec::new();
            let mut damage: Vec<&Hero> = Vec::new();
            let mut support: Vec<&Hero> = Vec::new();

            for h in &filtered {
                match h.role.as_str() {
                    "tank" => tank.push(h),
                    "damage" => damage.push(h),
                    "support" => support.push(h),
                    _ => damage.push(h), // fallback
                }
            }

            // Sort each group alphabetically
            tank.sort_by(|a, b| a.name.cmp(&b.name));
            damage.sort_by(|a, b| a.name.cmp(&b.name));
            support.sort_by(|a, b| a.name.cmp(&b.name));

            let sections: Vec<(&str, &str, Vec<&Hero>)> = vec![
                ("Tank", "tank", tank),
                ("Damage", "damage", damage),
                ("Support", "support", support),
            ];

            if filtered.is_empty() {
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
                for (label, role, heroes_in_role) in sections.iter() {
                    if !heroes_in_role.is_empty() {
                        {render_role_section(label, role, heroes_in_role, &selected_id, selected_hero)}
                    }
                }
            }
        }
    }
}

fn render_role_section(
    label: &str,
    role: &str,
    heroes: &[&Hero],
    selected_id: &str,
    selected_hero: &Signal<Option<Hero>>,
) -> Element {
    let heading_color = role_heading_color(role);
    let mut selected_hero = *selected_hero;

    rsx! {
        div {
            class: "heroes-role-heading",
            style: "color: {heading_color};",
            "{label}"
        }
        div { class: "heroes-grid",
            for hero in heroes.iter() {
                {render_hero_button(hero, selected_id, &mut selected_hero)}
            }
        }
    }
}

fn render_hero_button(
    hero: &Hero,
    selected_id: &str,
    selected_hero: &mut Signal<Option<Hero>>,
) -> Element {
    let is_selected = hero.id == selected_id;
    let btn_class = if is_selected { "hero-btn selected" } else { "hero-btn" };
    let hero_clone = hero.clone();
    let portrait_url = hero.portrait_url.clone();
    let initial = hero.initial();
    let name = hero.name.clone();
    let hero_id = hero.id.clone();
    let mut sig = *selected_hero;

    rsx! {
        button {
            key: "{hero_id}",
            class: "{btn_class}",
            onclick: move |_| {
                sig.set(Some(hero_clone.clone()));
            },
            div { class: "hero-btn-portrait",
                if !portrait_url.is_empty() {
                    img { src: "{portrait_url}", alt: "{name}" }
                } else {
                    span { class: "hero-btn-initial", "{initial}" }
                }
            }
            span { class: "hero-btn-name", "{name}" }
        }
    }
}

// ---------------------------------------------------------------------------
// Right panel: hero detail view
// ---------------------------------------------------------------------------

fn render_hero_detail(hero: &Hero) -> Element {
    let total = hero.total_hp();
    let role_color = hero.role_color();
    let role_label = hero.role_label();
    let role_bg = role_bg(&hero.role);

    // HP bar segment widths (percentage)
    let health_pct = if total > 0 {
        (hero.health as f64 / total as f64) * 100.0
    } else {
        0.0
    };
    let armor_pct = if total > 0 {
        (hero.armor as f64 / total as f64) * 100.0
    } else {
        0.0
    };
    let shields_pct = if total > 0 {
        (hero.shields as f64 / total as f64) * 100.0
    } else {
        0.0
    };

    rsx! {
        // Header: portrait + name + role badge
        div { class: "hero-detail-header",
            div { class: "hero-detail-portrait",
                if !hero.portrait_url.is_empty() {
                    img { src: "{hero.portrait_url}", alt: "{hero.name}" }
                } else {
                    span { class: "hero-detail-portrait-initial", "{hero.initial()}" }
                }
            }
            div { class: "hero-detail-info",
                div { class: "hero-detail-name", "{hero.name}" }
                span {
                    class: "hero-detail-role-badge",
                    style: "background: {role_bg}; color: {role_color};",
                    "{role_label}"
                }
            }
        }

        // HP breakdown
        div { class: "hero-hp-section",
            div { class: "hero-hp-label", "Hit Points — {total}" }
            div { class: "hero-hp-bar",
                if hero.health > 0 {
                    div {
                        class: "hero-hp-segment",
                        style: "width:{health_pct:.1}%; background:#e2e8f0;",
                        "{hero.health}"
                    }
                }
                if hero.armor > 0 {
                    div {
                        class: "hero-hp-segment",
                        style: "width:{armor_pct:.1}%; background:#f59e0b;",
                        "{hero.armor}"
                    }
                }
                if hero.shields > 0 {
                    div {
                        class: "hero-hp-segment",
                        style: "width:{shields_pct:.1}%; background:#38bdf8;",
                        "{hero.shields}"
                    }
                }
            }
            div { class: "hero-hp-legend",
                if hero.health > 0 {
                    span {
                        span { class: "hero-hp-dot", style: "background:#e2e8f0;" }
                        "Health {hero.health}"
                    }
                }
                if hero.armor > 0 {
                    span {
                        span { class: "hero-hp-dot", style: "background:#f59e0b;" }
                        "Armor {hero.armor}"
                    }
                }
                if hero.shields > 0 {
                    span {
                        span { class: "hero-hp-dot", style: "background:#38bdf8;" }
                        "Shields {hero.shields}"
                    }
                }
            }
        }

        // Abilities
        div { class: "hero-abilities-title", "Abilities" }
        div { class: "hero-abilities-list",
            for ability in hero.abilities.iter() {
                {render_ability_card(ability)}
            }
        }
    }
}

fn render_ability_card(ability: &Ability) -> Element {
    let cooldown_text = ability
        .cooldown
        .map(|cd| {
            if cd == cd.floor() {
                format!("{:.0}s", cd)
            } else {
                format!("{:.1}s", cd)
            }
        });

    rsx! {
        div { class: "ability-card", key: "{ability.key}-{ability.name}",
            div { class: "ability-key-badge", "{ability.key}" }
            div { class: "ability-body",
                div { class: "ability-header",
                    span { class: "ability-name", "{ability.name}" }
                    if let Some(cd) = &cooldown_text {
                        span { class: "ability-cooldown", "{cd}" }
                    }
                }
                p { class: "ability-desc", "{ability.description}" }
            }
        }
    }
}
