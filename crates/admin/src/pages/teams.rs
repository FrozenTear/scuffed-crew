use std::collections::HashMap;

use leptos::prelude::*;
use leptos::task::spawn_local;
use serde::{Deserialize, Serialize};

use scuffed_ui::components::button::{Button, ButtonVariant};
use scuffed_ui::components::modal::Modal;
use scuffed_ui::components::toast::{use_toast, Toast};

use crate::api;
use crate::components::confirm_dialog::ConfirmDialog;
use crate::components::data_table::DataTable;
use crate::components::form_modal::FormModal;
use crate::components::forms::FormField;

#[derive(Debug, Clone, Deserialize)]
struct Team {
    id: String,
    name: String,
    game_id: String,
    color: Option<String>,
    division: Option<String>,
    lore_quote: Option<String>,
    is_active: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct Game {
    id: String,
    name: String,
}

#[derive(Serialize)]
struct CreateTeamBody {
    name: String,
    game_id: String,
    color: Option<String>,
    division: Option<String>,
    lore_quote: Option<String>,
}

#[derive(Serialize)]
struct UpdateTeamBody {
    name: Option<String>,
    game_id: Option<String>,
    color: Option<Option<String>>,
    division: Option<Option<String>>,
    lore_quote: Option<Option<String>>,
}

#[derive(Debug, Clone, Deserialize)]
struct RosterEntry {
    #[allow(dead_code)]
    id: String,
    member_id: String,
    #[allow(dead_code)]
    team_id: String,
    team_role: String,
    #[allow(dead_code)]
    is_active: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct Member {
    id: String,
    display_name: String,
}

#[derive(Serialize)]
struct AddToRosterBody {
    member_id: String,
    team_role: String,
}

#[derive(Serialize)]
struct UpdateRosterRoleBody {
    team_role: String,
}

#[component]
pub fn TeamsPage() -> impl IntoView {
    let refresh = RwSignal::new(0u32);
    let toast = use_toast();

    let roster_counts: RwSignal<HashMap<String, usize>> = RwSignal::new(HashMap::new());

    let teams = LocalResource::new(move || {
        refresh.get();
        async { api::get_list::<Team>("/api/teams").await.ok() }
    });

    // Fetch roster counts whenever teams refresh
    {
        let refresh_val = refresh;
        Effect::new(move || {
            refresh_val.get();
            spawn_local(async move {
                if let Ok(team_list) = api::get_list::<Team>("/api/teams").await {
                    let mut counts = HashMap::new();
                    for t in &team_list {
                        if let Ok(roster) =
                            api::get::<Vec<RosterEntry>>(&format!("/api/teams/{}/roster", t.id))
                                .await
                        {
                            counts.insert(t.id.clone(), roster.len());
                        }
                    }
                    roster_counts.set(counts);
                }
            });
        });
    }

    // Games list for dropdown
    let games_list = LocalResource::new(move || {
        refresh.get();
        async { api::get::<Vec<Game>>("/api/games").await.ok() }
    });

    // Create/Edit modal state
    let form_open = RwSignal::new(false);
    let form_editing_id = RwSignal::new(Option::<String>::None);
    let form_name = RwSignal::new(String::new());
    let form_game_id = RwSignal::new(String::new());
    let form_division = RwSignal::new(String::new());
    let form_color = RwSignal::new(String::new());
    let form_lore = RwSignal::new(String::new());
    let form_submitting = RwSignal::new(false);

    // Roster modal state
    let roster_open = RwSignal::new(false);
    let roster_team_id = RwSignal::new(String::new());
    let roster_team_name = RwSignal::new(String::new());
    let roster_refresh = RwSignal::new(0u32);

    let roster_entries = LocalResource::new(move || {
        roster_refresh.get();
        let tid = roster_team_id.get();
        async move {
            if tid.is_empty() {
                return None;
            }
            api::get::<Vec<RosterEntry>>(&format!("/api/teams/{tid}/roster"))
                .await
                .ok()
        }
    });

    let all_members = LocalResource::new(move || {
        roster_refresh.get();
        async { api::get_list::<Member>("/api/members").await.ok() }
    });

    // Add member to roster state
    let add_member_id = RwSignal::new(String::new());
    let add_member_role = RwSignal::new("player".to_string());

    // Remove from roster confirm
    let remove_open = RwSignal::new(false);
    let remove_member_id = RwSignal::new(String::new());

    let open_create = move || {
        form_editing_id.set(None);
        form_name.set(String::new());
        form_game_id.set(String::new());
        form_division.set(String::new());
        form_color.set(String::new());
        form_lore.set(String::new());
        form_open.set(true);
    };

    let open_edit = move |t: &Team| {
        form_editing_id.set(Some(t.id.clone()));
        form_name.set(t.name.clone());
        form_game_id.set(t.game_id.clone());
        form_division.set(t.division.clone().unwrap_or_default());
        form_color.set(t.color.clone().unwrap_or_default());
        form_lore.set(t.lore_quote.clone().unwrap_or_default());
        form_open.set(true);
    };

    let do_submit_team = move || {
        let editing_id = form_editing_id.get();
        let name = form_name.get();
        let game_id = form_game_id.get();
        let division = form_division.get();
        let color = form_color.get();
        let lore = form_lore.get();

        if game_id.is_empty() {
            toast.show(Toast::warning("Select a game first"));
            return;
        }

        form_submitting.set(true);

        spawn_local(async move {
            let result = if let Some(id) = editing_id {
                let body = UpdateTeamBody {
                    name: Some(name),
                    game_id: Some(game_id),
                    color: Some(if color.is_empty() { None } else { Some(color) }),
                    division: Some(if division.is_empty() {
                        None
                    } else {
                        Some(division)
                    }),
                    lore_quote: Some(if lore.is_empty() { None } else { Some(lore) }),
                };
                api::put::<_, Team>(&format!("/api/teams/{id}"), &body)
                    .await
                    .map(|_| "Team updated")
            } else {
                let body = CreateTeamBody {
                    name,
                    game_id,
                    color: if color.is_empty() { None } else { Some(color) },
                    division: if division.is_empty() {
                        None
                    } else {
                        Some(division)
                    },
                    lore_quote: if lore.is_empty() { None } else { Some(lore) },
                };
                api::post::<_, Team>("/api/teams", &body)
                    .await
                    .map(|_| "Team created")
            };

            match result {
                Ok(msg) => {
                    toast.show(Toast::success(msg));
                    form_open.set(false);
                    refresh.update(|n| *n += 1);
                }
                Err(e) => toast.show(Toast::error(format!("Failed: {e}"))),
            }
            form_submitting.set(false);
        });
    };

    let do_add_to_roster = move || {
        let tid = roster_team_id.get();
        let mid = add_member_id.get();
        let role = add_member_role.get();
        if mid.is_empty() {
            toast.show(Toast::warning("Select a member first"));
            return;
        }
        spawn_local(async move {
            let body = AddToRosterBody {
                member_id: mid,
                team_role: role,
            };
            match api::post::<_, RosterEntry>(&format!("/api/teams/{tid}/roster"), &body).await {
                Ok(_) => {
                    toast.show(Toast::success("Member added to roster"));
                    add_member_id.set(String::new());
                    add_member_role.set("player".to_string());
                    roster_refresh.update(|n| *n += 1);
                    // Update roster count
                    roster_counts.update(|counts| {
                        let e = counts.entry(tid).or_insert(0);
                        *e += 1;
                    });
                }
                Err(e) => toast.show(Toast::error(format!("Failed: {e}"))),
            }
        });
    };

    let do_update_roster_role = move |member_id: String, new_role: String| {
        let tid = roster_team_id.get();
        spawn_local(async move {
            let body = UpdateRosterRoleBody {
                team_role: new_role.clone(),
            };
            match api::put::<_, ()>(&format!("/api/teams/{tid}/roster/{member_id}"), &body).await {
                Ok(_) => {
                    toast.show(Toast::success(format!("Role updated to {new_role}")));
                    roster_refresh.update(|n| *n += 1);
                }
                Err(e) => toast.show(Toast::error(format!("Failed: {e}"))),
            }
        });
    };

    let do_remove_from_roster = move || {
        let tid = roster_team_id.get();
        let mid = remove_member_id.get();
        spawn_local(async move {
            match api::delete(&format!("/api/teams/{tid}/roster/{mid}")).await {
                Ok(_) => {
                    toast.show(Toast::success("Removed from roster"));
                    remove_open.set(false);
                    roster_refresh.update(|n| *n += 1);
                    // Update roster count
                    roster_counts.update(|counts| {
                        if let Some(c) = counts.get_mut(&tid) {
                            *c = c.saturating_sub(1);
                        }
                    });
                }
                Err(e) => toast.show(Toast::error(format!("Failed: {e}"))),
            }
        });
    };

    let form_title = Signal::derive(move || {
        if form_editing_id.get().is_some() {
            "Edit Team".to_string()
        } else {
            "Create Team".to_string()
        }
    });

    let roster_title =
        Signal::derive(move || format!("Roster \u{2014} {}", roster_team_name.get()));

    view! {
        <h1>"Teams"</h1>
        <div class="page-actions">
            <Button
                variant=ButtonVariant::Primary
                on_click=Callback::new(move |_| open_create())
            >
                "Create Team"
            </Button>
        </div>
        {move || match teams.get().flatten() {
            None => view! { <p>"Loading..."</p> }.into_any(),
            Some(list) if list.is_empty() => view! { <p>"No teams yet."</p> }.into_any(),
            Some(list) => {
                let games_map: std::collections::HashMap<String, String> = games_list
                    .get()
                    .flatten()
                    .unwrap_or_default()
                    .into_iter()
                    .map(|g| (g.id, g.name))
                    .collect();
                view! {
                <DataTable headers=vec!["Name", "Game", "Division", "Status", "Actions"]>
                    {list.into_iter().map(|t| {
                        let active = if t.is_active { "Active" } else { "Inactive" };
                        let game_name = games_map.get(&t.game_id).cloned().unwrap_or_else(|| t.game_id.clone());
                        let t_edit = t.clone();
                        let t_roster_id = t.id.clone();
                        let t_roster_name = t.name.clone();
                        let t_id_for_count = t.id.clone();
                        view! {
                            <tr>
                                <td>{t.name.clone()}</td>
                                <td>{game_name}</td>
                                <td>{t.division.clone().unwrap_or_else(|| "\u{2014}".into())}</td>
                                <td>{active}</td>
                                <td class="table-actions">
                                    <Button
                                        variant=ButtonVariant::Secondary
                                        on_click=Callback::new(move |_| open_edit(&t_edit))
                                    >
                                        "Edit"
                                    </Button>
                                    <Button
                                        variant=ButtonVariant::Ghost
                                        on_click=Callback::new(move |_| {
                                            roster_team_id.set(t_roster_id.clone());
                                            roster_team_name.set(t_roster_name.clone());
                                            roster_refresh.update(|n| *n += 1);
                                            roster_open.set(true);
                                        })
                                    >
                                        {move || {
                                            let count = roster_counts.get().get(&t_id_for_count).copied().unwrap_or(0);
                                            if count > 0 {
                                                format!("Roster ({count})")
                                            } else {
                                                "Roster".to_string()
                                            }
                                        }}
                                    </Button>
                                </td>
                            </tr>
                        }
                    }).collect_view()}
                </DataTable>
            }.into_any()
            },
        }}

        // Create/Edit Team Modal
        <FormModal
            open=form_open
            on_close=Callback::new(move |_| form_open.set(false))
            title=form_title
            on_submit=Callback::new(move |_| do_submit_team())
            submitting=form_submitting
        >
            <FormField label="Name" value=form_name/>
            <div class="form-group">
                <label class="form-label">"Game"</label>
                <select
                    class="form-input"
                    prop:value=move || form_game_id.get()
                    on:change=move |ev| form_game_id.set(event_target_value(&ev))
                >
                    <option value="">"Select a game..."</option>
                    {move || {
                        games_list.get().flatten().unwrap_or_default().into_iter().map(|g| {
                            view! { <option value={g.id.clone()}>{g.name}</option> }
                        }).collect_view()
                    }}
                </select>
            </div>
            <FormField label="Division" value=form_division/>
            <FormField label="Color" value=form_color/>
            <FormField label="Lore Quote" value=form_lore/>
        </FormModal>

        // Roster Modal
        <Modal open=roster_open on_close=Callback::new(move |_| roster_open.set(false))>
            <div style="min-width: 500px;">
                <h3 class="form-modal-title">{move || roster_title.get()}</h3>

                // Roster table
                {move || match roster_entries.get().flatten() {
                    None => view! { <p>"Loading roster..."</p> }.into_any(),
                    Some(entries) if entries.is_empty() => view! { <p style="color: var(--text-muted);">"No members on this roster."</p> }.into_any(),
                    Some(entries) => view! {
                        <DataTable headers=vec!["Member", "Role", "Actions"]>
                            {entries.into_iter().map(|e| {
                                let mid = e.member_id.clone();
                                let mid2 = e.member_id.clone();
                                let current_role = e.team_role.clone();
                                view! {
                                    <tr>
                                        <td>{e.member_id.clone()}</td>
                                        <td>
                                            <select
                                                style="background: var(--bg-surface); border: 1px solid var(--border); border-radius: 4px; padding: 0.2rem 0.4rem; color: var(--text-primary); font-size: 0.8rem;"
                                                prop:value=current_role
                                                on:change=move |ev| {
                                                    let new_role = event_target_value(&ev);
                                                    do_update_roster_role(mid.clone(), new_role);
                                                }
                                            >
                                                <option value="captain">"Captain"</option>
                                                <option value="player">"Player"</option>
                                                <option value="sub">"Sub"</option>
                                                <option value="coach">"Coach"</option>
                                            </select>
                                        </td>
                                        <td class="table-actions">
                                            <Button
                                                variant=ButtonVariant::Danger
                                                on_click=Callback::new(move |_| {
                                                    remove_member_id.set(mid2.clone());
                                                    remove_open.set(true);
                                                })
                                            >
                                                "Remove"
                                            </Button>
                                        </td>
                                    </tr>
                                }
                            }).collect_view()}
                        </DataTable>
                    }.into_any(),
                }}

                // Add member form
                <div style="margin-top: 1rem; display: flex; gap: 0.5rem; align-items: flex-end;">
                    <div style="flex: 1;">
                        <label style="color: var(--text-secondary); font-size: 0.8rem;">"Member"</label>
                        <select
                            style="width: 100%; background: var(--bg-surface); border: 1px solid var(--border); border-radius: 6px; padding: 0.4rem; color: var(--text-primary);"
                            prop:value=move || add_member_id.get()
                            on:change=move |ev| add_member_id.set(event_target_value(&ev))
                        >
                            <option value="">"Select member..."</option>
                            {move || {
                                let members = all_members.get().flatten().unwrap_or_default();
                                let rostered: std::collections::HashSet<String> = roster_entries
                                    .get()
                                    .flatten()
                                    .unwrap_or_default()
                                    .iter()
                                    .map(|e| e.member_id.clone())
                                    .collect();
                                members.into_iter()
                                    .filter(|m| !rostered.contains(&m.id))
                                    .map(|m| {
                                        view! { <option value={m.id.clone()}>{m.display_name}</option> }
                                    })
                                    .collect_view()
                            }}
                        </select>
                    </div>
                    <div>
                        <label style="color: var(--text-secondary); font-size: 0.8rem;">"Role"</label>
                        <select
                            style="background: var(--bg-surface); border: 1px solid var(--border); border-radius: 6px; padding: 0.4rem; color: var(--text-primary);"
                            prop:value=move || add_member_role.get()
                            on:change=move |ev| add_member_role.set(event_target_value(&ev))
                        >
                            <option value="captain">"Captain"</option>
                            <option value="player">"Player"</option>
                            <option value="sub">"Sub"</option>
                            <option value="coach">"Coach"</option>
                        </select>
                    </div>
                    <Button
                        variant=ButtonVariant::Primary
                        on_click=Callback::new(move |_| do_add_to_roster())
                    >
                        "Add"
                    </Button>
                </div>
            </div>
        </Modal>

        <ConfirmDialog
            open=remove_open
            on_confirm=Callback::new(move |_| do_remove_from_roster())
            on_cancel=Callback::new(move |_| remove_open.set(false))
            title="Remove from Roster".to_string()
            message="Are you sure you want to remove this member from the roster?".to_string()
            danger=true
        />
    }
}
