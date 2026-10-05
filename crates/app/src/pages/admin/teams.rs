use dioxus::prelude::*;
use serde::Deserialize;

use crate::components::{
    ConfirmDialog, DataTable, FormModal, Toast, admin_pending, list_cap_notice, use_toast,
};
use crate::hooks::{ModalController, use_api_list};
use crate::state::use_auth;
use scuffed_api_client::ApiClient;
use scuffed_types::api::{AddRosterMemberRequest, CreateTeamRequest, UpdateRosterRoleRequest};
use scuffed_types::{OrgRole, SiteSettings};

/// Settings slot for the officer team-edit gate.
/// `Loading` is in flight — officers must not see edit controls yet.
/// `Loaded(false)` is the default and a failed load.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OfficersEditFlag {
    Loading,
    Loaded(bool),
}

/// Show controls that `PUT /api/teams/{id}`.
/// Admins always. Officers only after settings load with the flag on.
/// Members, recruits, and signed-out callers never.
fn team_edit_allowed(role: Option<OrgRole>, flag: OfficersEditFlag) -> bool {
    match role {
        Some(OrgRole::Admin) => true,
        Some(OrgRole::Officer) => matches!(flag, OfficersEditFlag::Loaded(true)),
        Some(OrgRole::Member | OrgRole::Recruit) | None => false,
    }
}

/// Outer `None`: `GET /api/settings` still in flight.
/// Inner `None`: settled with no payload — treat the flag as off.
fn officers_edit_flag(slot: Option<Option<bool>>) -> OfficersEditFlag {
    match slot {
        None => OfficersEditFlag::Loading,
        Some(flag) => OfficersEditFlag::Loaded(flag.unwrap_or(false)),
    }
}

fn read_officers_edit_flag(settings: Resource<Option<SiteSettings>>) -> OfficersEditFlag {
    let loaded = settings.read();
    let slot = loaded
        .as_ref()
        .map(|payload| payload.as_ref().map(|s| s.officers_can_edit_teams));
    officers_edit_flag(slot)
}

const OFFICER_TEAM_EDIT_DENIED: &str = "Only admins can edit teams right now.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RosterPhase {
    Loading,
    Ready,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RosterPanel {
    Loading,
    Error,
    Empty,
    Rows,
}

/// A failed roster fetch is not an empty roster.
fn roster_panel(phase: RosterPhase, len: usize) -> RosterPanel {
    match phase {
        RosterPhase::Loading => RosterPanel::Loading,
        RosterPhase::Failed => RosterPanel::Error,
        RosterPhase::Ready if len == 0 => RosterPanel::Empty,
        RosterPhase::Ready => RosterPanel::Rows,
    }
}

/// Officers who cannot edit team fields still manage rosters. The intro must
/// not tell them they can edit those fields.
fn teams_admin_intro(can_edit: bool) -> &'static str {
    if can_edit {
        "You can edit team name, game, color, and division, and change the roster. Teams cannot be deleted or archived."
    } else {
        "You can change the roster. Team name, game, color, and division are admin-only right now. Teams cannot be deleted or archived."
    }
}

/// Officer edit that the server rejected because the flag is off (or turned off
/// between page load and save). Other 403s keep the generic toast.
fn team_save_denied_message(
    editing: bool,
    is_admin: bool,
    forbidden: bool,
) -> Option<&'static str> {
    if editing && !is_admin && forbidden {
        Some(OFFICER_TEAM_EDIT_DENIED)
    } else {
        None
    }
}

// --- Types ---
// Local response types with API-enriched fields (joined names).

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

const TEAM_ROLES: [&str; 4] = ["player", "captain", "coach", "sub"];

#[component]
pub fn AdminTeams() -> Element {
    let auth = use_auth();
    let is_admin = auth().is_admin();
    let mut teams = use_api_list::<Team>("/api/teams");
    let mut games = use_api_list::<Game>("/api/games");
    let mut members = use_api_list::<Member>("/api/members");
    let mut toast = use_toast();

    // Team form state
    let mut modal = ModalController::<String>::new();
    let mut form_name = use_signal(String::new);
    let mut form_game_id = use_signal(String::new);
    let mut form_color = use_signal(String::new);
    let mut form_division = use_signal(String::new);

    // Roster modal state
    let mut roster_modal = ModalController::<Team>::new();
    let mut roster_data: Signal<Vec<RosterEntry>> = use_signal(Vec::new);
    let mut roster_phase = use_signal(|| RosterPhase::Loading);
    let mut roster_refresh = use_signal(|| 0u64);

    // Add member to roster form
    let mut add_member_id = use_signal(String::new);
    let mut add_member_role = use_signal(|| "player".to_string());
    let mut add_submitting = use_signal(|| false);

    // Remove member confirm
    let mut remove_modal = ModalController::<RosterEntry>::new();

    // Team-edit gate. Always fetched (hooks stay unconditional). Admins ignore
    // the flag; officers wait until it settles so Edit does not flash on.
    let mut settings_refresh = use_signal(|| 0u64);
    let team_settings = use_resource(move || {
        let _tick = settings_refresh();
        async move {
            ApiClient::web()
                .fetch::<SiteSettings>("/api/settings")
                .await
                .ok()
        }
    });
    // Read the resource inside the effect so it re-runs when settings settle
    // or refresh after a 403. Closes an open edit if the flag dropped.
    use_effect(move || {
        let role = auth().user.as_ref().and_then(|u| u.role);
        let flag = read_officers_edit_flag(team_settings);
        let editing = modal.is_open() && modal.get_target().is_some();
        if editing && !team_edit_allowed(role, flag) {
            modal.close();
        }
    });

    // Fetch roster when team selected. Failure stays Failed so the modal
    // does not look like a team with nobody on it.
    let _roster_loader = use_resource(move || async move {
        let _ = roster_refresh();
        let Some(team) = roster_modal.get_target() else {
            return;
        };
        let requested = team.id.clone();
        roster_phase.set(RosterPhase::Loading);
        match ApiClient::web()
            .fetch::<Vec<RosterEntry>>(&format!("/api/teams/{requested}/roster"))
            .await
        {
            Ok(entries) => {
                let current = roster_modal.get_target().map(|open| open.id);
                if current.as_deref() == Some(requested.as_str()) {
                    roster_data.set(entries);
                    roster_phase.set(RosterPhase::Ready);
                }
            }
            Err(_) => {
                let current = roster_modal.get_target().map(|open| open.id);
                if current.as_deref() == Some(requested.as_str()) {
                    roster_phase.set(RosterPhase::Failed);
                }
            }
        }
    });

    // --- Team CRUD handlers ---

    let open_create = move |_| {
        form_name.set(String::new());
        form_game_id.set(String::new());
        form_color.set(String::new());
        form_division.set(String::new());
        modal.show_empty();
    };

    let mut open_edit = move |team: Team| {
        form_name.set(team.name);
        form_game_id.set(team.game_id);
        form_color.set(team.color.unwrap_or_default());
        form_division.set(team.division.unwrap_or_default());
        modal.show(team.id);
    };

    let on_close = move |_| modal.close();

    let on_submit = move |_| {
        let name = form_name().trim().to_string();
        let game_id = form_game_id().trim().to_string();
        if name.is_empty() || game_id.is_empty() {
            toast.show(Toast::error("Name and game are required."));
            return;
        }
        let color_raw = form_color().trim().to_string();
        let div_raw = form_division().trim().to_string();
        let body = CreateTeamRequest {
            name,
            game_id,
            color: if color_raw.is_empty() {
                None
            } else {
                Some(color_raw)
            },
            division: if div_raw.is_empty() {
                None
            } else {
                Some(div_raw)
            },
        };
        let edit_id = modal.get_target();
        let editing = edit_id.is_some();
        let caller_is_admin = auth().is_admin();
        modal.start_submit();
        spawn(async move {
            let client = ApiClient::web();
            let result = if let Some(id) = edit_id {
                client
                    .put_json::<_, Team>(&format!("/api/teams/{id}"), &body)
                    .await
            } else {
                client.post_json::<_, Team>("/api/teams", &body).await
            };
            modal.end_submit();
            match result {
                Ok(_) => {
                    toast.show(Toast::success("Team saved."));
                    modal.close();
                    teams.refresh += 1;
                    games.refresh += 1;
                    members.refresh += 1;
                }
                Err(e) => {
                    if let Some(msg) =
                        team_save_denied_message(editing, caller_is_admin, e.is_forbidden())
                    {
                        toast.show(Toast::error(msg.to_string()));
                        modal.close();
                        settings_refresh += 1;
                    } else {
                        toast.show(Toast::error(format!("Failed to save team: {e}")));
                    }
                }
            }
        });
    };

    // --- Roster handlers ---

    let mut open_roster = move |team: Team| {
        roster_data.set(Vec::new());
        roster_phase.set(RosterPhase::Loading);
        add_member_id.set(String::new());
        add_member_role.set("player".to_string());
        roster_modal.show(team);
        roster_refresh += 1;
    };

    let mut on_roster_close = move |_| {
        roster_modal.close();
    };

    let on_add_member = move |_| {
        let member_id = add_member_id().trim().to_string();
        if member_id.is_empty() {
            return;
        }
        if let Some(team) = roster_modal.get_target() {
            let team_id = team.id.clone();
            let body = AddRosterMemberRequest {
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
        if let Some(team) = roster_modal.get_target() {
            let team_id = team.id.clone();
            let body = UpdateRosterRoleRequest {
                team_role: new_role,
            };
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
        remove_modal.show(entry);
    };

    let on_remove_confirm = move |_| {
        if let Some(entry) = remove_modal.get_target()
            && let Some(team) = roster_modal.get_target()
        {
            let team_id = team.id.clone();
            let member_id = entry.member_id.clone();
            remove_modal.close();
            spawn(async move {
                match ApiClient::web()
                    .delete(&format!("/api/teams/{team_id}/roster/{member_id}"))
                    .await
                {
                    Ok(_) => {
                        toast.show(Toast::success("Member removed from roster."));
                        roster_refresh += 1;
                    }
                    Err(e) => toast.show(Toast::error(format!("Remove failed: {e}"))),
                }
            });
        }
    };

    let on_remove_cancel = move |_| {
        remove_modal.close();
    };

    let can_edit_teams = team_edit_allowed(
        auth().user.as_ref().and_then(|u| u.role),
        read_officers_edit_flag(team_settings),
    );
    let roster_view = roster_panel(roster_phase(), roster_data.read().len());

    // --- Render ---

    rsx! {

        div { class: "admin-toolbar",
            h1 { "Teams" }
            if is_admin {
                button { class: "btn-add", onclick: open_create, "+ Add Team" }
            }
        }
        p { class: "empty-state", style: "text-align:left;padding:0 0 1rem;margin:0;",
            "{teams_admin_intro(can_edit_teams)}"
        }

        // Teams table
        {
            let data = teams.data.read();
            let data = data.as_ref().and_then(|d| d.as_ref());
            match data {
                None => admin_pending(&teams, "teams"),
                Some(list) if list.is_empty() => rsx! {
                    p { class: "empty-state", "No teams yet." }
                },
                Some(list) => rsx! {
                    DataTable { headers: vec!["Name", "Game", "Division", "Color", "Actions"],
                        for team in list.iter() {
                            {
                                let t_edit = team.clone();
                                let t_roster = team.clone();
                                // API Team has no game_name; resolve from games list already loaded
                                // for the create/edit form (F-AUI-002). Prefer API field if present.
                                let game_display = team
                                    .game_name
                                    .clone()
                                    .or_else(|| {
                                        games
                                            .data
                                            .read()
                                            .as_ref()
                                            .and_then(|d| d.as_ref())
                                            .and_then(|games_list| {
                                                games_list
                                                    .iter()
                                                    .find(|g| g.id == team.game_id)
                                                    .map(|g| g.name.clone())
                                            })
                                    })
                                    .unwrap_or_else(|| "\u{2014}".into());
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
                                                if can_edit_teams {
                                                    button {
                                                        class: "row-btn",
                                                        onclick: move |_| open_edit(t_edit.clone()),
                                                        "Edit"
                                                    }
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

        {list_cap_notice(&teams, "teams")}
        {list_cap_notice(&games, "games")}
        {list_cap_notice(&members, "members")}

        // Create/Edit Team modal
        FormModal {
            title: if modal.get_target().is_some() { "Edit Team".to_string() } else { "Add Team".to_string() },
            open: modal.is_open(),
            submitting: modal.is_submitting(),
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
                        let games_data = games.data.read();
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
                    placeholder: "e.g. #rrggbb",
                    value: "{form_color}",
                    oninput: move |e| form_color.set(e.value()),
                }
            }
        }

        // Roster modal (wide)
        if roster_modal.is_open() {
            div {
                class: "form-modal-overlay",
                onclick: move |_| on_roster_close(()),
                div {
                    class: "form-modal",
                    style: "max-width:800px;",
                    onclick: move |e| e.stop_propagation(),

                    div { class: "form-modal-header",
                        "Roster: {roster_modal.get_target().map(|t| t.name).unwrap_or_default()}"
                    }

                    div { class: "form-modal-body",
                        // Roster table
                        match roster_view {
                            RosterPanel::Loading => rsx! {
                                p { class: "empty-state", "Loading roster…" }
                            },
                            RosterPanel::Error => rsx! {
                                div { class: "fetch-error-wrap", role: "alert",
                                    p { class: "fetch-error", "Couldn't load this roster." }
                                    button {
                                        r#type: "button",
                                        class: "fetch-error__retry",
                                        onclick: move |_| {
                                            roster_phase.set(RosterPhase::Loading);
                                            roster_refresh += 1;
                                        },
                                        "Retry"
                                    }
                                }
                            },
                            RosterPanel::Empty => rsx! {
                                p { class: "empty-state", "No members on this roster yet." }
                            },
                            RosterPanel::Rows => rsx! {
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
                                        let mems = members.data.read();
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
                remove_modal.get_target().map(|e| e.member_name).unwrap_or_default()
            ),
            open: remove_modal.is_open(),
            danger: true,
            on_confirm: on_remove_confirm,
            on_cancel: on_remove_cancel,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admins_always_see_team_edit() {
        for flag in [
            OfficersEditFlag::Loading,
            OfficersEditFlag::Loaded(false),
            OfficersEditFlag::Loaded(true),
        ] {
            assert!(team_edit_allowed(Some(OrgRole::Admin), flag));
        }
    }

    #[test]
    fn officers_see_team_edit_only_after_flag_loads_on() {
        assert!(!team_edit_allowed(
            Some(OrgRole::Officer),
            OfficersEditFlag::Loading
        ));
        assert!(!team_edit_allowed(
            Some(OrgRole::Officer),
            officers_edit_flag(None)
        ));
        assert!(!team_edit_allowed(
            Some(OrgRole::Officer),
            OfficersEditFlag::Loaded(false)
        ));
        assert!(!team_edit_allowed(
            Some(OrgRole::Officer),
            officers_edit_flag(Some(None))
        ));
        assert!(!team_edit_allowed(
            Some(OrgRole::Officer),
            officers_edit_flag(Some(Some(false)))
        ));
        assert!(team_edit_allowed(
            Some(OrgRole::Officer),
            OfficersEditFlag::Loaded(true)
        ));
        assert!(team_edit_allowed(
            Some(OrgRole::Officer),
            officers_edit_flag(Some(Some(true)))
        ));
    }

    #[test]
    fn other_roles_never_see_team_edit() {
        for role in [None, Some(OrgRole::Member), Some(OrgRole::Recruit)] {
            assert!(!team_edit_allowed(role, OfficersEditFlag::Loaded(true)));
            assert!(!team_edit_allowed(role, OfficersEditFlag::Loading));
        }
    }

    #[test]
    fn roster_failure_is_not_an_empty_roster() {
        assert_eq!(roster_panel(RosterPhase::Failed, 0), RosterPanel::Error);
        assert_eq!(roster_panel(RosterPhase::Failed, 3), RosterPanel::Error);
        assert_eq!(roster_panel(RosterPhase::Ready, 0), RosterPanel::Empty);
        assert_eq!(roster_panel(RosterPhase::Ready, 2), RosterPanel::Rows);
        assert_eq!(roster_panel(RosterPhase::Loading, 0), RosterPanel::Loading);
    }

    #[test]
    fn read_only_intro_does_not_promise_team_edits() {
        let read_only = teams_admin_intro(false);
        assert!(!read_only.to_lowercase().contains("you can edit"));
        assert!(read_only.contains("roster"));
        assert!(read_only.contains("admin-only"));
        let editable = teams_admin_intro(true);
        assert!(editable.contains("edit team name"));
        assert!(editable.contains("roster"));
    }

    #[test]
    fn officer_team_edit_403_uses_specific_copy() {
        assert_eq!(
            team_save_denied_message(true, false, true),
            Some("Only admins can edit teams right now.")
        );
        assert_eq!(team_save_denied_message(true, true, true), None);
        assert_eq!(team_save_denied_message(true, false, false), None);
        assert_eq!(team_save_denied_message(false, false, true), None);
    }

    #[test]
    fn edit_gate_effect_reads_settings_inside_the_effect() {
        let src = include_str!("teams.rs");
        let start = src
            .find("use_effect(move || {")
            .expect("team edit gate effect");
        let body = &src[start..];
        let end = body.find("});").expect("effect end");
        let effect = &body[..end];
        assert!(
            effect.contains("read_officers_edit_flag"),
            "effect must read settings inside so it re-runs when GET /api/settings settles"
        );
        assert!(
            effect.contains("auth()"),
            "effect must read auth inside so a role change re-runs the gate"
        );
    }
}
