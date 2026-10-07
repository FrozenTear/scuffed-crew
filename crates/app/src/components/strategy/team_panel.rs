use dioxus::prelude::*;
use scuffed_types::{HeroId, HeroRole, HeroSelection, TeamFormat, TeamSlot};

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
    .slot-assign-btn {
        flex: 1;
        text-align: left;
        background: none;
        border: 1px dashed var(--border);
        border-radius: 4px;
        color: var(--accent);
        font-size: 0.75rem;
        font-weight: 600;
        padding: 0.2rem 0.45rem;
        cursor: pointer;
        transition: background 0.12s, border-color 0.12s;
    }
    .slot-assign-btn:hover {
        background: var(--accent-soft);
        border-color: var(--accent);
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

/// Name and role for a filled slot. The role is the catalog's `role`, which
/// the roster sets from `role_for_hero_name`. Unknown ids stay "Unknown" and
/// count as Damage.
fn assigned_hero(id: &str) -> (&'static str, HeroRole) {
    match super::hero_catalog::hero_by_id(id) {
        Some(hero) => (hero.name, hero.role),
        None => ("Unknown", HeroRole::Damage),
    }
}

/// 6v6 slots accept any role. 5v5 slots accept only the slot's required role.
/// The role is the catalog's `role`, same as a filled slot.
fn slot_accepts(hero: &super::hero_catalog::CatalogHero, is_6v6: bool, slot: TeamSlot) -> bool {
    is_6v6 || hero.role == slot.required_role()
}

#[component]
pub fn TeamPanel(
    /// Current team format (5v5 or 6v6).
    team_format: TeamFormat,

    /// Current hero selections in team slots.
    composition: Vec<HeroSelection>,

    /// The hero currently picked in the hero picker, if any. When set, empty
    /// slots that accept this hero's role offer one-click assignment.
    #[props(default)]
    selected_hero: Option<HeroId>,

    // ---- Mutation callbacks ----
    on_format_change: EventHandler<TeamFormat>,
    on_clear_slot: EventHandler<TeamSlot>,
    on_clear_all: EventHandler<()>,
    /// Assign the currently-picked hero to the given slot.
    on_assign: EventHandler<TeamSlot>,
) -> Element {
    let slots = team_format.slots();
    let is_6v6 = team_format == TeamFormat::SixVSix;
    let picked = selected_hero
        .as_deref()
        .and_then(super::hero_catalog::hero_by_id);

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
                                            let (hname, hrole) = assigned_hero(hid);
                                            let role_color = hrole.color_hex();
                                            let icon_path = super::hero_catalog::icon_path(hid);

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
                                    None => {
                                        // Offer one-click assignment when a hero is picked and
                                        // its role fits this slot (6v6 slots accept any role).
                                        let assignable_hero = picked
                                            .filter(|hero| slot_accepts(hero, is_6v6, slot));
                                        match assignable_hero {
                                            Some(hero) => {
                                                let hname = hero.name;
                                                rsx! {
                                                    button {
                                                        class: "slot-assign-btn",
                                                        title: "Assign {hname} to this slot",
                                                        onclick: move |_| on_assign.call(slot),
                                                        "+ {hname}"
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

#[cfg(test)]
mod tests {
    use super::*;

    fn accepts(id: &str, is_6v6: bool, slot: TeamSlot) -> bool {
        super::super::hero_catalog::hero_by_id(id)
            .is_some_and(|hero| slot_accepts(hero, is_6v6, slot))
    }

    #[test]
    fn five_v_five_checks_the_shared_role() {
        assert!(accepts("domina", false, TeamSlot::Tank1));
        assert!(accepts("dmon", false, TeamSlot::Tank1));
        assert!(!accepts("sombra", false, TeamSlot::Tank1));
        assert!(accepts("sombra", false, TeamSlot::Support1));
        assert!(accepts("doctrine", false, TeamSlot::Support1));
        assert!(accepts("jetpack-cat", false, TeamSlot::Support2));
        assert!(accepts("mizuki", false, TeamSlot::Support1));
        assert!(accepts("wuyang", false, TeamSlot::Support2));
        assert!(!accepts("mizuki", false, TeamSlot::Dps1));
        assert!(!accepts("domina", false, TeamSlot::Dps1));
        assert!(!accepts("jetpack-cat", false, TeamSlot::Dps1));
    }

    #[test]
    fn six_v_six_accepts_any_role() {
        assert!(accepts("sombra", true, TeamSlot::Tank1));
        assert!(accepts("domina", true, TeamSlot::Dps1));
        assert!(accepts("wuyang", true, TeamSlot::Tank2));
        assert!(accepts("dmon", true, TeamSlot::Support1));
    }

    #[test]
    fn filled_slots_use_role_for_hero_name() {
        for name in scuffed_types::HEROES {
            let id = super::super::hero_catalog::hero_id(name);
            let (shown, role) = assigned_hero(&id);
            assert_eq!(shown, *name, "{name}");
            assert_eq!(
                role,
                scuffed_types::role_for_hero_name(name).unwrap(),
                "{name}"
            );
        }
        assert_eq!(assigned_hero("dmon"), ("D.Mon", HeroRole::Tank));
        assert_eq!(assigned_hero("sombra"), ("Sombra", HeroRole::Support));
        assert_eq!(assigned_hero("doctrine"), ("Doctrine", HeroRole::Support));
        assert_eq!(
            assigned_hero("jetpack-cat"),
            ("Jetpack Cat", HeroRole::Support)
        );
        assert_eq!(assigned_hero("not-a-hero"), ("Unknown", HeroRole::Damage));
    }
}
