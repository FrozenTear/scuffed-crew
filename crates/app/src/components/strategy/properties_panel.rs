use dioxus::prelude::*;
use scuffed_types::{Color, ElementType, HeroId, HeroRole, StrategyElement, TimelinePhase};
use uuid::Uuid;

const PROPERTIES_CSS: &str = r#"
    .props-panel {
        display: flex;
        flex-direction: column;
        background: var(--surface);
        border-left: 1px solid var(--border);
        width: 240px;
        overflow-y: auto;
    }
    .props-title {
        font-family: var(--font-head);
        font-size: 0.75rem;
        color: var(--text-3);
        text-transform: uppercase;
        letter-spacing: 0.08em;
        padding: 0.75rem 0.75rem 0.5rem;
        border-bottom: 1px solid var(--border);
        margin: 0;
    }
    .props-empty {
        padding: 1.5rem 0.75rem;
        text-align: center;
    }
    .props-empty p {
        color: var(--text-3);
        font-size: 0.8rem;
        margin: 0.25rem 0;
    }
    .props-empty .hint {
        font-size: 0.7rem;
        color: var(--text-3);
        opacity: 0.7;
    }
    .props-content {
        display: flex;
        flex-direction: column;
        gap: 0.1rem;
    }
    .prop-group {
        padding: 0.5rem 0.75rem;
        border-bottom: 1px solid var(--border);
    }
    .prop-label {
        display: block;
        font-size: 0.65rem;
        color: var(--text-3);
        text-transform: uppercase;
        letter-spacing: 0.06em;
        margin-bottom: 0.3rem;
    }
    .prop-value {
        font-size: 0.8rem;
        color: var(--text-2);
    }
    .prop-input {
        width: 100%;
        background: var(--surface-2);
        border: 1px solid var(--border);
        border-radius: 4px;
        color: var(--text);
        font-size: 0.8rem;
        padding: 0.3rem 0.4rem;
        outline: none;
        transition: border-color 0.15s;
    }
    .prop-input:focus {
        border-color: var(--accent);
    }
    select.prop-input {
        cursor: pointer;
    }

    /* ---- Color swatches ---- */
    .prop-colors {
        display: flex;
        gap: 0.3rem;
    }
    .prop-color-btn {
        width: 20px;
        height: 20px;
        border: 2px solid transparent;
        border-radius: 50%;
        cursor: pointer;
        transition: border-color 0.12s, transform 0.12s;
    }
    .prop-color-btn:hover {
        transform: scale(1.15);
    }
    .prop-color-btn.active {
        border-color: var(--accent);
        box-shadow: 0 0 0 2px var(--accent-soft);
    }

    /* ---- Layer controls ---- */
    .prop-layer-controls {
        display: flex;
        gap: 0.35rem;
    }
    .prop-layer-controls button {
        flex: 1;
        padding: 0.25rem 0.4rem;
        font-size: 0.7rem;
        background: var(--surface-2);
        border: 1px solid var(--border);
        border-radius: 4px;
        color: var(--text-2);
        cursor: pointer;
        transition: background 0.12s, color 0.12s;
    }
    .prop-layer-controls button:hover {
        background: var(--surface-2);
        color: var(--text);
    }

    /* ---- Actions ---- */
    .prop-actions {
        padding-top: 0.75rem;
    }
    .prop-delete-btn {
        width: 100%;
        padding: 0.35rem;
        border: 1px solid var(--danger);
        border-radius: 4px;
        background: color-mix(in srgb, var(--danger) 10%, transparent);
        color: var(--danger);
        font-size: 0.8rem;
        cursor: pointer;
        transition: background 0.12s;
    }
    .prop-delete-btn:hover {
        background: color-mix(in srgb, var(--danger) 25%, transparent);
    }
"#;

/// Available preset colors for elements.
const PRESET_COLORS: [(Color, &str); 5] = [
    (Color::BLUE_TEAM, "Blue Team"),
    (Color::RED_TEAM, "Red Team"),
    (Color::TANK, "Tank"),
    (Color::DAMAGE, "Damage"),
    (Color::SUPPORT, "Support"),
];

/// Known heroes for the dropdown.
const HEROES: &[(HeroRole, &[(&str, &str)])] = &[
    (
        HeroRole::Tank,
        &[
            ("dva", "D.Va"),
            ("doomfist", "Doomfist"),
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
            ("hazard", "Hazard"),
        ],
    ),
    (
        HeroRole::Damage,
        &[
            ("ashe", "Ashe"),
            ("bastion", "Bastion"),
            ("cassidy", "Cassidy"),
            ("echo", "Echo"),
            ("genji", "Genji"),
            ("hanzo", "Hanzo"),
            ("junkrat", "Junkrat"),
            ("mei", "Mei"),
            ("pharah", "Pharah"),
            ("reaper", "Reaper"),
            ("sojourn", "Sojourn"),
            ("soldier-76", "Soldier: 76"),
            ("sombra", "Sombra"),
            ("symmetra", "Symmetra"),
            ("torbjorn", "Torbjorn"),
            ("tracer", "Tracer"),
            ("venture", "Venture"),
            ("widowmaker", "Widowmaker"),
        ],
    ),
    (
        HeroRole::Support,
        &[
            ("ana", "Ana"),
            ("baptiste", "Baptiste"),
            ("brigitte", "Brigitte"),
            ("illari", "Illari"),
            ("juno", "Juno"),
            ("kiriko", "Kiriko"),
            ("lifeweaver", "Lifeweaver"),
            ("lucio", "Lucio"),
            ("mercy", "Mercy"),
            ("moira", "Moira"),
            ("zenyatta", "Zenyatta"),
        ],
    ),
];

fn all_heroes() -> Vec<(&'static str, &'static str)> {
    HEROES
        .iter()
        .flat_map(|(_, heroes)| heroes.iter().copied())
        .collect()
}

#[component]
pub fn PropertiesPanel(
    /// The currently selected element, if any.
    #[props(default)]
    element: Option<StrategyElement>,

    /// Available phases.
    phases: Vec<TimelinePhase>,

    // ---- Mutation callbacks ----
    on_label_change: EventHandler<(Uuid, Option<String>)>,
    on_color_change: EventHandler<(Uuid, Color)>,
    on_hero_change: EventHandler<(Uuid, Option<HeroId>)>,
    on_phase_change: EventHandler<(Uuid, Option<Uuid>)>,
    on_move_up: EventHandler<Uuid>,
    on_move_down: EventHandler<Uuid>,
    on_delete: EventHandler<Uuid>,
) -> Element {
    rsx! {
        style { {PROPERTIES_CSS} }
        div { class: "props-panel",
            h3 { class: "props-title", "Properties" }

            match element {
                Some(ref el) => {
                    {
                        let element_id = el.id;
                        let element_type_name = match &el.element_type {
                            ElementType::PlayerMarker => "Player Marker",
                            ElementType::Route { .. } => "Route",
                            ElementType::Area { .. } => "Area",
                            ElementType::Arrow { .. } => "Arrow",
                            ElementType::Text { .. } => "Text",
                            ElementType::Icon { .. } => "Icon",
                            ElementType::Drawing { .. } => "Drawing",
                            ElementType::Ability { .. } => "Ability",
                        };
                        let is_player_marker = matches!(el.element_type, ElementType::PlayerMarker);
                        let current_label = el.label.clone().unwrap_or_default();
                        let current_color = el.color;
                        let current_hero_id = el.hero_id.clone();
                        let current_phase_id = el.phase_id;

                        rsx! {
                            div { class: "props-content",
                                // ---- Type (read-only) ----
                                div { class: "prop-group",
                                    label { class: "prop-label", "Type" }
                                    div { class: "prop-value", "{element_type_name}" }
                                }

                                // ---- Label ----
                                div { class: "prop-group",
                                    label { class: "prop-label", "Label" }
                                    input {
                                        r#type: "text",
                                        class: "prop-input",
                                        placeholder: "Add label...",
                                        value: "{current_label}",
                                        oninput: move |e: Event<FormData>| {
                                            let v = e.value();
                                            let lbl = if v.is_empty() { None } else { Some(v) };
                                            on_label_change.call((element_id, lbl));
                                        },
                                    }
                                }

                                // ---- Color ----
                                div { class: "prop-group",
                                    label { class: "prop-label", "Color" }
                                    div { class: "prop-colors",
                                        {PRESET_COLORS.iter().map(|(color, name)| {
                                            let c = *color;
                                            let css = c.to_css();
                                            let active_cls = if c == current_color { "prop-color-btn active" } else { "prop-color-btn" };
                                            rsx! {
                                                button {
                                                    class: "{active_cls}",
                                                    title: "{name}",
                                                    style: "background-color: {css};",
                                                    onclick: move |_| on_color_change.call((element_id, c)),
                                                }
                                            }
                                        })}
                                    }
                                }

                                // ---- Hero dropdown (PlayerMarker only) ----
                                if is_player_marker {
                                    div { class: "prop-group",
                                        label { class: "prop-label", "Hero" }
                                        select {
                                            class: "prop-input",
                                            value: current_hero_id.clone().unwrap_or_default(),
                                            onchange: move |e: Event<FormData>| {
                                                let v = e.value();
                                                let hero = if v.is_empty() { None } else { Some(v) };
                                                on_hero_change.call((element_id, hero));
                                            },
                                            option { value: "", "None" }
                                            {all_heroes().into_iter().map(|(id, name)| {
                                                rsx! {
                                                    option { value: "{id}", "{name}" }
                                                }
                                            })}
                                        }
                                    }
                                }

                                // ---- Phase assignment ----
                                div { class: "prop-group",
                                    label { class: "prop-label", "Phase" }
                                    select {
                                        class: "prop-input",
                                        value: current_phase_id.map(|p| p.to_string()).unwrap_or_default(),
                                        onchange: move |e: Event<FormData>| {
                                            let v = e.value();
                                            let phase = if v.is_empty() {
                                                None
                                            } else {
                                                Uuid::parse_str(&v).ok()
                                            };
                                            on_phase_change.call((element_id, phase));
                                        },
                                        option { value: "", "All Phases" }
                                        {phases.iter().map(|phase| {
                                            let pid = phase.id.to_string();
                                            let pname = phase.name.clone();
                                            rsx! {
                                                option { value: "{pid}", "{pname}" }
                                            }
                                        })}
                                    }
                                }

                                // ---- Layer (z-index) ----
                                div { class: "prop-group",
                                    label { class: "prop-label", "Layer" }
                                    div { class: "prop-layer-controls",
                                        button {
                                            title: "Move Down",
                                            onclick: move |_| on_move_down.call(element_id),
                                            "Down"
                                        }
                                        button {
                                            title: "Move Up",
                                            onclick: move |_| on_move_up.call(element_id),
                                            "Up"
                                        }
                                    }
                                }

                                // ---- Delete ----
                                div { class: "prop-group prop-actions",
                                    button {
                                        class: "prop-delete-btn",
                                        onclick: move |_| on_delete.call(element_id),
                                        "Delete Element"
                                    }
                                }
                            }
                        }
                    }
                }
                None => rsx! {
                    div { class: "props-empty",
                        p { "No element selected" }
                        p { class: "hint", "Click an element on the canvas to edit its properties" }
                    }
                },
            }
        }
    }
}
