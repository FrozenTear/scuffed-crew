use dioxus::prelude::*;
use scuffed_types::{HeroRole, HeroSelection, TeamFormat, TeamSlot};

const TEAM_PANEL_CSS: &str = r#"
    .team-panel {
        display: flex;
        flex-direction: column;
        background: var(--surface);
        overflow-y: auto;
    }
    .team-panel .panel-title {
        font-family: var(--font-head);
        font-size: 0.75rem;
        color: var(--text-3);
        text-transform: uppercase;
        letter-spacing: 0.08em;
        padding: 0.75rem 0.75rem 0.5rem;
        margin: 0;
        border-bottom: 1px solid var(--border);
    }

    /* ---- Format toggle ---- */
    .format-toggle {
        display: flex;
        gap: 0.25rem;
        padding: 0.5rem 0.75rem;
        border-bottom: 1px solid var(--border);
    }
    .format-btn {
        flex: 1;
        padding: 0.35rem 0;
        border: 1px solid var(--border);
        border-radius: 4px;
        background: none;
        color: var(--text-2);
        font-size: 0.8rem;
        font-weight: 600;
        cursor: pointer;
        transition: background 0.12s, color 0.12s, border-color 0.12s;
    }
    .format-btn:hover {
        background: var(--surface-2);
    }
    .format-btn.active {
        background: var(--accent-soft);
        color: var(--accent);
        border-color: var(--accent);
    }

    /* ---- Team slots ---- */
    .team-slots {
        display: flex;
        flex-direction: column;
    }
    .team-slot {
        display: flex;
        align-items: center;
        gap: 0.5rem;
        padding: 0.5rem 0.75rem;
        border-bottom: 1px solid var(--border);
        border-left: 3px solid transparent;
    }
    .slot-label {
        font-size: 0.7rem;
        color: var(--text-3);
        text-transform: uppercase;
        letter-spacing: 0.04em;
        min-width: 55px;
    }
    .slot-hero {
        flex: 1;
        display: flex;
        align-items: center;
    }
    .slot-empty {
        font-size: 0.75rem;
        color: var(--text-3);
        font-style: italic;
    }

    /* ---- Assigned hero display ---- */
    .assigned-hero {
        display: flex;
        align-items: center;
        gap: 0.35rem;
        flex: 1;
    }
    .slot-hero-icon {
        width: 24px;
        height: 24px;
        border-radius: 50%;
        object-fit: cover;
        background: var(--surface-2);
    }
    .slot-hero-name {
        flex: 1;
        font-size: 0.8rem;
    }
    .slot-remove-btn {
        background: none;
        border: none;
        color: var(--text-3);
        font-size: 0.85rem;
        cursor: pointer;
        padding: 0 0.2rem;
        transition: color 0.12s;
    }
    .slot-remove-btn:hover {
        color: var(--danger);
    }

    /* ---- Actions ---- */
    .team-actions {
        padding: 0.5rem 0.75rem;
    }
    .team-clear-btn {
        width: 100%;
        padding: 0.3rem;
        border: 1px solid var(--border);
        border-radius: 4px;
        background: none;
        color: var(--text-2);
        font-size: 0.75rem;
        cursor: pointer;
        transition: background 0.12s, color 0.12s;
    }
    .team-clear-btn:hover {
        background: var(--surface-2);
        color: var(--text);
    }
"#;

/// Hero name lookup by ID (minimal inline table).
fn hero_name(id: &str) -> &'static str {
    match id {
        "dva" => "D.Va",
        "doomfist" => "Doomfist",
        "junker-queen" => "Junker Queen",
        "mauga" => "Mauga",
        "orisa" => "Orisa",
        "ramattra" => "Ramattra",
        "reinhardt" => "Reinhardt",
        "roadhog" => "Roadhog",
        "sigma" => "Sigma",
        "winston" => "Winston",
        "wrecking-ball" => "Wrecking Ball",
        "zarya" => "Zarya",
        "hazard" => "Hazard",
        "ashe" => "Ashe",
        "bastion" => "Bastion",
        "cassidy" => "Cassidy",
        "echo" => "Echo",
        "genji" => "Genji",
        "hanzo" => "Hanzo",
        "junkrat" => "Junkrat",
        "mei" => "Mei",
        "pharah" => "Pharah",
        "reaper" => "Reaper",
        "sojourn" => "Sojourn",
        "soldier-76" => "Soldier: 76",
        "sombra" => "Sombra",
        "symmetra" => "Symmetra",
        "torbjorn" => "Torbjorn",
        "tracer" => "Tracer",
        "venture" => "Venture",
        "widowmaker" => "Widowmaker",
        "ana" => "Ana",
        "baptiste" => "Baptiste",
        "brigitte" => "Brigitte",
        "illari" => "Illari",
        "juno" => "Juno",
        "kiriko" => "Kiriko",
        "lifeweaver" => "Lifeweaver",
        "lucio" => "Lucio",
        "mercy" => "Mercy",
        "moira" => "Moira",
        "zenyatta" => "Zenyatta",
        _ => "Unknown",
    }
}

/// Hero role lookup by ID.
fn hero_role(id: &str) -> HeroRole {
    match id {
        "dva" | "doomfist" | "junker-queen" | "mauga" | "orisa" | "ramattra" | "reinhardt"
        | "roadhog" | "sigma" | "winston" | "wrecking-ball" | "zarya" | "hazard" => HeroRole::Tank,
        "ana" | "baptiste" | "brigitte" | "illari" | "juno" | "kiriko" | "lifeweaver" | "lucio"
        | "mercy" | "moira" | "zenyatta" => HeroRole::Support,
        _ => HeroRole::Damage,
    }
}

#[component]
pub fn TeamPanel(
    /// Current team format (5v5 or 6v6).
    team_format: TeamFormat,

    /// Current hero selections in team slots.
    composition: Vec<HeroSelection>,

    // ---- Mutation callbacks ----
    on_format_change: EventHandler<TeamFormat>,
    on_clear_slot: EventHandler<TeamSlot>,
    on_clear_all: EventHandler<()>,
) -> Element {
    let slots = team_format.slots();
    let is_6v6 = team_format == TeamFormat::SixVSix;

    rsx! {
        style { {TEAM_PANEL_CSS} }
        div { class: "team-panel",
            h3 { class: "panel-title", "Team Composition" }

            // ---- Format toggle ----
            div { class: "format-toggle",
                button {
                    class: if team_format == TeamFormat::FiveVFive { "format-btn active" } else { "format-btn" },
                    onclick: move |_| on_format_change.call(TeamFormat::FiveVFive),
                    "5v5"
                }
                button {
                    class: if team_format == TeamFormat::SixVSix { "format-btn active" } else { "format-btn" },
                    onclick: move |_| on_format_change.call(TeamFormat::SixVSix),
                    "6v6"
                }
            }

            // ---- Slots ----
            div { class: "team-slots",
                {slots.iter().map(|slot| {
                    let slot = *slot;
                    let name = if is_6v6 {
                        match slot {
                            TeamSlot::Tank1 => "Slot 1",
                            TeamSlot::Tank2 => "Slot 2",
                            TeamSlot::Dps1 => "Slot 3",
                            TeamSlot::Dps2 => "Slot 4",
                            TeamSlot::Support1 => "Slot 5",
                            TeamSlot::Support2 => "Slot 6",
                        }
                    } else {
                        slot.display_name()
                    };

                    let border_color = if is_6v6 {
                        "var(--accent)".to_string()
                    } else {
                        slot.required_role().color_hex().to_string()
                    };

                    // Find hero in this slot
                    let hero_in_slot = composition.iter().find(|h| h.slot == slot);

                    rsx! {
                        div {
                            class: "team-slot",
                            style: "border-left-color: {border_color};",
                            span { class: "slot-label", "{name}" }
                            div { class: "slot-hero",
                                match hero_in_slot {
                                    Some(sel) => {
                                        {
                                            let hid = &sel.hero_id;
                                            let hname = hero_name(hid);
                                            let hrole = hero_role(hid);
                                            let role_color = hrole.color_hex();
                                            let icon_path = format!("/assets/heroes/{hid}/icon.webp");

                                            rsx! {
                                                div { class: "assigned-hero",
                                                    img {
                                                        class: "slot-hero-icon",
                                                        src: "{icon_path}",
                                                        alt: "{hname}",
                                                    }
                                                    span {
                                                        class: "slot-hero-name",
                                                        style: "color: {role_color};",
                                                        "{hname}"
                                                    }
                                                    button {
                                                        class: "slot-remove-btn",
                                                        title: "Remove",
                                                        onclick: move |e: Event<MouseData>| {
                                                            e.stop_propagation();
                                                            on_clear_slot.call(slot);
                                                        },
                                                        "\u{00d7}"
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    None => rsx! {
                                        span { class: "slot-empty", "Empty" }
                                    },
                                }
                            }
                        }
                    }
                })}
            }

            // ---- Clear all ----
            div { class: "team-actions",
                button {
                    class: "team-clear-btn",
                    onclick: move |_| on_clear_all.call(()),
                    "Clear All"
                }
            }
        }
    }
}
