use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

use scuffed_api_client::ApiClient;
use crate::components::{DataTable, FormModal, ConfirmDialog, Toast, use_toast, ADMIN_SHARED_CSS};

// --- Types ---

#[derive(Debug, Clone, Deserialize)]
struct Team {
    id: String,
    name: String,
    game_id: String,
    game_name: Option<String>,
    division: Option<String>,
    color: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct Game {
    id: String,
    name: String,
}

#[derive(Debug, Clone, Deserialize)]
struct RosterEntry {
    member_id: String,
    member_name: String,
    team_role: String,
}

#[derive(Debug, Clone, Deserialize)]
struct Member {
    id: String,
    display_name: String,
}

#[derive(Serialize)]
struct CreateTeam {
    name: String,
    game_id: String,
    color: Option<String>,
    division: Option<String>,
}

#[derive(Serialize)]
struct AddRosterMember {
    member_id: String,
    team_role: String,
}

#[derive(Serialize)]
struct UpdateRosterRole {
    team_role: String,
}

const TEAM_ROLES: [&str; 4] = ["player", "captain", "coach", "sub"];

#[component]
pub fn AdminTeams() -> Element {
    let mut refresh = use_signal(|| 0u64);
    let mut toast = use_toast();

    // Team form state
    let mut modal_open = use_signal(|| false);
    let mut submitting = use_signal(|| false);
    let mut editing_id: Signal<Option<String>> = use_signal(|| None);
    let mut form_name = use_signal(String::new);
    let mut form_game_id = use_signal(String::new);
    let mut form_color = use_signal(String::new);
    let mut form_division = use_signal(String::new);

    // Delete confirm state
    let mut delete_open = use_signal(|| false);
    let mut delete_target: Signal<Option<Team>> = use_signal(|| None);

    // Roster modal state
    let mut roster_open = use_signal(|| false);
    let mut roster_team: Signal<Option<Team>> = use_signal(|| None);
    let mut roster_data: Signal<Vec<RosterEntry>> = use_signal(Vec::new);
    let mut roster_refresh = use_signal(|| 0u64);

    // Add member to roster form
    let mut add_member_id = use_signal(String::new);
    let mut add_member_role = use_signal(|| "player".to_string());
    let mut add_submitting = use_signal(|| false);

    // Remove member confirm
    let mut remove_open = use_signal(|| false);
    let mut remove_target: Signal<Option<RosterEntry>> = use_signal(|| None);

    // Data
    let teams = use_resource(move || async move {
        let _ = refresh();
        ApiClient::web().fetch::<Vec<Team>>("/api/teams").await.ok()
    });

    let games = use_resource(move || async move {
        let _ = refresh();
        ApiClient::web().fetch::<Vec<Game>>("/api/games").await.ok()
    });

    let members = use_resource(move || async move {
        let _ = refresh();
        ApiClient::web().fetch::<Vec<Member>>("/api/members").await.ok()
    });

    // Fetch roster when team selected
    let _roster_loader = use_resource(move || async move {
        let _ = roster_refresh();
        if let Some(team) = roster_team() {
            if let Ok(entries) = ApiClient::web()
                .fetch::<Vec<RosterEntry>>(&format!("/api/teams/{}/roster", team.id))
                .await
            {
                roster_data.set(entries);
            }
        }
    });

    // --- Team CRUD handlers ---

    let open_create = move |_| {
        editing_id.set(None);
        form_name.set(String::new());
        form_game_id.set(String::new());
        form_color.set(String::new());
        form_division.set(String::new());
        modal_open.set(true);
    };

    let mut open_edit = move |team: Team| {
        editing_id.set(Some(team.id));
        form_name.set(team.name);
        form_game_id.set(team.game_id);
        form_color.set(team.color.unwrap_or_default());
        form_division.set(team.division.unwrap_or_default());
        modal_open.set(true);
    };

    let on_close = move |_| modal_open.set(false);

    let on_submit = move |_| {
        let name = form_name().trim().to_string();
        let game_id = form_game_id().trim().to_string();
        if name.is_empty() || game_id.is_empty() {
            toast.show(Toast::error("Name and game are required."));
            return;
        }
        let color_raw = form_color().trim().to_string();
        let div_raw = form_division().trim().to_string();
        let body = CreateTeam {
            name,
            game_id,
            color: if color_raw.is_empty() { None } else { Some(color_raw) },
            division: if div_raw.is_empty() { None } else { Some(div_raw) },
        };
        let edit_id = editing_id();
        submitting.set(true);
        spawn(async move {
            let client = ApiClient::web();
            let result = if let Some(id) = edit_id {
                client.put_json::<_, Team>(&format!("/api/teams/{id}"), &body).await
            } else {
                client.post_json::<_, Team>("/api/teams", &body).await
            };
            submitting.set(false);
            match result {
                Ok(_) => {
                    toast.show(Toast::success("Team saved."));
                    modal_open.set(false);
                    refresh += 1;
                }
                Err(e) => toast.show(Toast::error(format!("Failed to save team: {e}"))),
            }
        });
    };

    // --- Delete handlers ---

    let mut open_delete = move |team: Team| {
        delete_target.set(Some(team));
        delete_open.set(true);
    };

    let on_delete_confirm = move |_| {
        if let Some(team) = delete_target() {
            let id = team.id.clone();
            spawn(async move {
                match ApiClient::web().delete(&format!("/api/teams/{id}")).await {
                    Ok(_) => {
                        toast.show(Toast::success("Team deleted."));
                        delete_open.set(false);
                        delete_target.set(None);
                        refresh += 1;
                    }
                    Err(e) => toast.show(Toast::error(format!("Delete failed: {e}"))),
                }
            });
        }
    };

    let on_delete_cancel = move |_| {
        delete_open.set(false);
        delete_target.set(None);
    };

    // --- Roster handlers ---

    let mut open_roster = move |team: Team| {
        roster_team.set(Some(team));
        roster_data.set(Vec::new());
        add_member_id.set(String::new());
        add_member_role.set("player".to_string());
        roster_refresh += 1;
        roster_open.set(true);
    };

    let mut on_roster_close = move |_| {
        roster_open.set(false);
        roster_team.set(None);
    };

    let on_add_member = move |_| {
        let member_id = add_member_id().trim().to_string();
        if member_id.is_empty() {
            return;
        }
        if let Some(team) = roster_team() {
            let team_id = team.id.clone();
            let body = AddRosterMember {
                member_id,
                team_role: add_member_role(),
            };
            add_submitting.set(true);
            spawn(async move {
                let result = ApiClient::web()
                    .post_json::<_, RosterEntry>(&format!("/api/teams/{team_id}/roster"), &body)
                    .await;
                add_submitting.set(false);
                match result {
                    Ok(_) => {
                        toast.show(Toast::success("Member added to roster."));
                        add_member_id.set(String::new());
                        add_member_role.set("player".to_string());
                        roster_refresh += 1;
                    }
                    Err(e) => toast.show(Toast::error(format!("Failed to add member: {e}"))),
                }
            });
        }
    };

    let on_role_change = move |(member_id, new_role): (String, String)| {
        if let Some(team) = roster_team() {
            let team_id = team.id.clone();
            let body = UpdateRosterRole { team_role: new_role };
            spawn(async move {
                let result = ApiClient::web()
                    .put_json::<_, RosterEntry>(
                        &format!("/api/teams/{team_id}/roster/{member_id}"),
                        &body,
                    )
                    .await;
                match result {
                    Ok(_) => {
                        toast.show(Toast::success("Role updated."));
                        roster_refresh += 1;
                    }
                    Err(e) => toast.show(Toast::error(format!("Failed to update role: {e}"))),
                }
            });
        }
    };

    let mut open_remove = move |entry: RosterEntry| {
        remove_target.set(Some(entry));
        remove_open.set(true);
    };

    let on_remove_confirm = move |_| {
        if let Some(entry) = remove_target() {
            if let Some(team) = roster_team() {
                let team_id = team.id.clone();
                let member_id = entry.member_id.clone();
                spawn(async move {
                    match ApiClient::web()
                        .delete(&format!("/api/teams/{team_id}/roster/{member_id}"))
                        .await
                    {
                        Ok(_) => {
                            toast.show(Toast::success("Member removed from roster."));
                            remove_open.set(false);
                            remove_target.set(None);
                            roster_refresh += 1;
                        }
                        Err(e) => toast.show(Toast::error(format!("Remove failed: {e}"))),
                    }
                });
            }
        }
    };

    let on_remove_cancel = move |_| {
        remove_open.set(false);
        remove_target.set(None);
    };

    // --- Render ---

    rsx! {
        style { {ADMIN_SHARED_CSS} }

        div { class: "admin-toolbar",
            h1 { "Teams" }
            button { class: "btn-add", onclick: open_create, "+ Add Team" }
        }

        // Teams table
        {
            let data = teams.read();
            let data = data.as_ref().and_then(|d| d.as_ref());
            match data {
                None => rsx! { p { class: "admin-loading", "Loading..." } },
                Some(list) if list.is_empty() => rsx! {
                    p { class: "empty-state", "No teams yet." }
                },
                Some(list) => rsx! {
                    DataTable { headers: vec!["Name", "Game", "Division", "Color", "Actions"],
                        for team in list.iter() {
                            {
                                let t_edit = team.clone();
                                let t_del = team.clone();
                                let t_roster = team.clone();
                                let game_display = team.game_name.clone().unwrap_or_else(|| "\u{2014}".into());
                                let div_display = team.division.clone().unwrap_or_else(|| "\u{2014}".into());
                                let color_display = team.color.clone().unwrap_or_else(|| "\u{2014}".into());
                                rsx! {
                                    tr { key: "{team.id}",
                                        td { "{team.name}" }
                                        td { "{game_display}" }
                                        td { "{div_display}" }
                                        td {
                                            if let Some(ref c) = team.color {
                                                span {
                                                    style: "display:inline-block;width:12px;height:12px;border-radius:50%;background:{c};margin-right:0.4rem;vertical-align:middle;",
                                                }
                                            }
                                            "{color_display}"
                                        }
                                        td {
                                            div { class: "row-actions",
                                                button {
                                                    class: "row-btn",
                                                    onclick: move |_| open_edit(t_edit.clone()),
                                                    "Edit"
                                                }
                                                button {
                                                    class: "row-btn danger",
                                                    onclick: move |_| open_delete(t_del.clone()),
                                                    "Delete"
                                                }
                                                button {
                                                    class: "row-btn primary",
                                                    onclick: move |_| open_roster(t_roster.clone()),
                                                    "Roster"
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                },
            }
        }

        // Create/Edit Team modal
        FormModal {
            title: if editing_id().is_some() { "Edit Team".to_string() } else { "Add Team".to_string() },
            open: modal_open(),
            submitting: submitting(),
            on_close: on_close,
            on_submit: on_submit,

            div { class: "form-field",
                label { class: "form-label", "Name" }
                input {
                    class: "form-input",
                    r#type: "text",
                    value: "{form_name}",
                    oninput: move |e| form_name.set(e.value()),
                }
            }
            div { class: "form-field",
                label { class: "form-label", "Game" }
                select {
                    class: "form-select",
                    value: "{form_game_id}",
                    onchange: move |e| form_game_id.set(e.value()),
                    option { value: "", "-- Select Game --" }
                    {
                        let games_data = games.read();
                        let games_data = games_data.as_ref().and_then(|d| d.as_ref());
                        match games_data {
                            Some(list) => rsx! {
                                for g in list.iter() {
                                    option { value: "{g.id}", "{g.name}" }
                                }
                            },
                            None => rsx! {},
                        }
                    }
                }
            }
            div { class: "form-field",
                label { class: "form-label", "Division (optional)" }
                input {
                    class: "form-input",
                    r#type: "text",
                    placeholder: "e.g. Division 1",
                    value: "{form_division}",
                    oninput: move |e| form_division.set(e.value()),
                }
            }
            div { class: "form-field",
                label { class: "form-label", "Color (optional)" }
                input {
                    class: "form-input",
                    r#type: "text",
                    placeholder: "#7c3aed",
                    value: "{form_color}",
                    oninput: move |e| form_color.set(e.value()),
                }
            }
        }

        // Delete confirm
        ConfirmDialog {
            title: "Delete Team".to_string(),
            message: format!(
                "Are you sure you want to delete \"{}\"? All roster data will be lost.",
                delete_target().map(|t| t.name).unwrap_or_default()
            ),
            open: delete_open(),
            danger: true,
            on_confirm: on_delete_confirm,
            on_cancel: on_delete_cancel,
        }

        // Roster modal (wide)
        if roster_open() {
            div {
                class: "form-modal-overlay",
                onclick: move |_| on_roster_close(()),
                div {
                    class: "form-modal",
                    style: "max-width:800px;",
                    onclick: move |e| e.stop_propagation(),

                    div { class: "form-modal-header",
                        "Roster: {roster_team().map(|t| t.name).unwrap_or_default()}"
                    }

                    div { class: "form-modal-body",
                        // Roster table
                        if roster_data.read().is_empty() {
                            p { class: "empty-state", "No members on this roster yet." }
                        } else {
                            table { class: "data-table",
                                thead {
                                    tr {
                                        th { "Member" }
                                        th { "Role" }
                                        th { "Actions" }
                                    }
                                }
                                tbody {
                                    for entry in roster_data.read().iter() {
                                        {
                                            let e_remove = entry.clone();
                                            let mid = entry.member_id.clone();
                                            let current_role = entry.team_role.clone();
                                            rsx! {
                                                tr { key: "{entry.member_id}",
                                                    td { "{entry.member_name}" }
                                                    td {
                                                        select {
                                                            class: "form-select",
                                                            value: "{current_role}",
                                                            onchange: move |e| {
                                                                on_role_change((mid.clone(), e.value()));
                                                            },
                                                            for role in TEAM_ROLES.iter() {
                                                                option { value: "{role}", "{role}" }
                                                            }
                                                        }
                                                    }
                                                    td {
                                                        button {
                                                            class: "row-btn danger",
                                                            onclick: move |_| open_remove(e_remove.clone()),
                                                            "Remove"
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        // Add member form
                        div {
                            style: "border-top:1px solid var(--border);padding-top:1rem;margin-top:1rem;display:flex;gap:0.5rem;align-items:flex-end;flex-wrap:wrap;",
                            div { class: "form-field", style: "flex:1;min-width:150px;",
                                label { class: "form-label", "Add Member" }
                                select {
                                    class: "form-select",
                                    value: "{add_member_id}",
                                    onchange: move |e| add_member_id.set(e.value()),
                                    option { value: "", "-- Select --" }
                                    {
                                        let mems = members.read();
                                        let mems = mems.as_ref().and_then(|d| d.as_ref());
                                        match mems {
                                            Some(list) => rsx! {
                                                for m in list.iter() {
                                                    option { value: "{m.id}", "{m.display_name}" }
                                                }
                                            },
                                            None => rsx! {},
                                        }
                                    }
                                }
                            }
                            div { class: "form-field", style: "min-width:100px;",
                                label { class: "form-label", "Role" }
                                select {
                                    class: "form-select",
                                    value: "{add_member_role}",
                                    onchange: move |e| add_member_role.set(e.value()),
                                    for role in TEAM_ROLES.iter() {
                                        option { value: "{role}", "{role}" }
                                    }
                                }
                            }
                            button {
                                class: "btn-save",
                                disabled: add_submitting(),
                                onclick: on_add_member,
                                if add_submitting() { "Adding..." } else { "Add" }
                            }
                        }
                    }

                    div { class: "form-modal-footer",
                        button {
                            class: "btn-cancel",
                            onclick: move |_| on_roster_close(()),
                            "Close"
                        }
                    }
                }
            }
        }

        // Remove roster member confirm
        ConfirmDialog {
            title: "Remove from Roster".to_string(),
            message: format!(
                "Remove \"{}\" from this team's roster?",
                remove_target().map(|e| e.member_name).unwrap_or_default()
            ),
            open: remove_open(),
            danger: true,
            on_confirm: on_remove_confirm,
            on_cancel: on_remove_cancel,
        }
    }
}
