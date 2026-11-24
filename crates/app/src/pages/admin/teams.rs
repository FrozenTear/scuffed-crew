use dioxus::prelude::*;

use crate::components::admin_shared::{after_delete_focus_ids, focus_after_delete_soon};
use serde::Deserialize;

use crate::components::ui::{
    EmptyCard, FieldError, PageHeader, PageShell, PageWidth, Pager, SkeletonBars, page_slice,
    show_list_top,
};
use crate::components::{
    ADMIN_PAGE_SIZE, ConfirmDialog, DataTable, FormModal, RowActions, SaveState, Toast,
    admin_list_pending, list_cap_notice, load_error_text, status_label, use_toast,
};
use crate::hooks::{ModalController, use_api_list};
use crate::routes::Route;
use crate::state::use_auth;
use scuffed_api_client::ApiClient;
use scuffed_types::api::{AddRosterMemberRequest, CreateTeamRequest, UpdateRosterRoleRequest};
use scuffed_types::{OrgRole, SiteSettings};

/// Settings slot for the officer team-edit gate.
/// `Loading` is in flight; officers must not see edit controls yet.
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
/// Inner `None`: settled with no payload; treat the flag as off.
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
    /// Loaded, and loading again after a role change, removal or add. The
    /// rows stay on screen, so the control that has focus is not removed
    /// and Tab stays in the dialog.
    Refreshing,
    Failed,
}

/// Phase while the roster loads: a first load shows the wait, a reload of a
/// loaded roster keeps its rows.
fn roster_load_phase(current: RosterPhase) -> RosterPhase {
    match current {
        RosterPhase::Ready | RosterPhase::Refreshing => RosterPhase::Refreshing,
        RosterPhase::Loading | RosterPhase::Failed => RosterPhase::Loading,
    }
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
        RosterPhase::Ready | RosterPhase::Refreshing if len == 0 => RosterPanel::Empty,
        RosterPhase::Ready | RosterPhase::Refreshing => RosterPanel::Rows,
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

/// The list region: the pager's focus target after a page change.
const LIST_ID: &str = "admin-teams-list";
const NEW_TEAM_ID: &str = "teams-new";
const TEAM_NAME_ID: &str = "team-name";
const TEAM_NAME_ERROR_ID: &str = "team-name-error";
const TEAM_GAME_ID: &str = "team-game";
const TEAM_GAME_ERROR_ID: &str = "team-game-error";
const TEAM_DIVISION_ID: &str = "team-division";
const TEAM_COLOR_ID: &str = "team-color";
const TEAM_COLOR_HINT_ID: &str = "team-color-hint";
const ROSTER_HINT_ID: &str = "roster-hint";
const ROSTER_ADD_MEMBER_ID: &str = "roster-add-member";
const ROSTER_ADD_MEMBER_ERROR_ID: &str = "roster-add-member-error";
const ROSTER_ADD_ROLE_ID: &str = "roster-add-role";

/// Page CSS: the team colour swatch. Everything else is the shared admin
/// pattern.
const PAGE_CSS: &str = r#"
.admin-teams-page .team-swatch,
.form-modal .team-swatch {
    display: inline-block;
    flex: 0 0 auto;
    width: var(--space-3);
    height: var(--space-3);
    margin-right: var(--space-2);
    vertical-align: middle;
}
.admin-roster-hint { margin: 0; }
"#;

/// Id of a row's Edit button, from the team id.
fn team_edit_button_id(team_id: &str) -> String {
    format!("team-edit-{team_id}")
}

/// Id of a row's Roster button, from the team id.
fn team_roster_button_id(team_id: &str) -> String {
    format!("team-roster-{team_id}")
}

/// Id of a roster row's role select, from the member id.
fn roster_role_id(member_id: &str) -> String {
    format!("roster-role-{member_id}")
}

/// The role the roster add form starts with, and goes back to after an add.
const ROSTER_ADD_DEFAULT_ROLE: &str = "player";

/// The roster dialog's draft: only its add form. Role changes and removals
/// in the table save right away, and an add that went through resets the
/// form, so neither leaves anything to ask about.
fn roster_draft_unsaved(member_id: &str, role: &str) -> bool {
    !member_id.trim().is_empty() || role != ROSTER_ADD_DEFAULT_ROLE
}

/// Id of a roster row's Remove button, from the member id.
fn roster_remove_id(member_id: &str) -> String {
    format!("roster-remove-{member_id}")
}

/// Blank optional values read as the word None. A bare dash is not a value.
fn or_none(value: Option<&str>) -> String {
    match value.map(str::trim) {
        Some(v) if !v.is_empty() => v.to_string(),
        _ => "None".to_string(),
    }
}

fn teams_empty_message(is_admin: bool) -> &'static str {
    if is_admin {
        "Create a team, then add members to its roster."
    } else {
        "Ask an admin to create a team. You can manage its roster once it exists."
    }
}

/// A team's colour as a small filled circle beside its value. The colour is
/// data, so it is drawn with the SVG `fill` attribute, never inline CSS, and
/// the value is always written next to it.
#[component]
fn TeamSwatch(color: String) -> Element {
    rsx! {
        svg {
            class: "team-swatch",
            view_box: "0 0 10 10",
            "aria-hidden": "true",
            "focusable": "false",
            circle { cx: "5", cy: "5", r: "5", fill: "{color}" }
        }
    }
}

#[component]
pub fn AdminTeams() -> Element {
    let auth = use_auth();
    let is_admin = auth().is_admin();
    let mut teams = use_api_list::<Team>("/api/teams");
    let mut games = use_api_list::<Game>("/api/games");
    let mut members = use_api_list::<Member>("/api/members");
    let mut toast = use_toast();
    let mut page = use_signal(|| 1usize);

    // Team form state
    let mut modal = ModalController::<String>::new();
    let mut form_name = use_signal(String::new);
    let mut form_game_id = use_signal(String::new);
    let mut form_color = use_signal(String::new);
    let mut form_division = use_signal(String::new);
    let mut name_error = use_signal(String::new);
    let mut game_error = use_signal(String::new);
    let mut save_error = use_signal(|| None::<String>);
    let mut opener = use_signal(|| NEW_TEAM_ID.to_string());

    // Roster modal state
    let mut roster_modal = ModalController::<Team>::new();
    let mut roster_data: Signal<Vec<RosterEntry>> = use_signal(Vec::new);
    let mut roster_phase = use_signal(|| RosterPhase::Loading);
    let mut roster_error = use_signal(String::new);
    let mut roster_refresh = use_signal(|| 0u64);

    // Add member to roster form
    let mut add_member_id = use_signal(String::new);
    let mut add_member_role = use_signal(|| ROSTER_ADD_DEFAULT_ROLE.to_string());
    let mut add_submitting = use_signal(|| false);
    let mut add_member_error = use_signal(String::new);
    let mut add_error = use_signal(|| None::<String>);

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
        // `peek`: reading the phase must not make this loader re-run.
        let phase = roster_load_phase(*roster_phase.peek());
        roster_phase.set(phase);
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
            Err(e) => {
                let current = roster_modal.get_target().map(|open| open.id);
                if current.as_deref() == Some(requested.as_str()) {
                    roster_error.set(crate::client_error::logged_message("team roster", &e));
                    roster_phase.set(RosterPhase::Failed);
                }
            }
        }
    });

    // --- Team CRUD handlers ---

    let mut clear_team_errors = move || {
        name_error.set(String::new());
        game_error.set(String::new());
        save_error.set(None);
    };

    let mut open_create = move || {
        form_name.set(String::new());
        form_game_id.set(String::new());
        form_color.set(String::new());
        form_division.set(String::new());
        clear_team_errors();
        opener.set(NEW_TEAM_ID.to_string());
        modal.show_empty();
    };

    let mut open_edit = move |team: Team| {
        form_name.set(team.name);
        form_game_id.set(team.game_id);
        form_color.set(team.color.unwrap_or_default());
        form_division.set(team.division.unwrap_or_default());
        clear_team_errors();
        opener.set(team_edit_button_id(&team.id));
        modal.show(team.id);
    };

    let on_close = move |_| modal.close();

    let on_submit = move |_| {
        clear_team_errors();
        let name = form_name().trim().to_string();
        let game_id = form_game_id().trim().to_string();
        if name.is_empty() || game_id.is_empty() {
            if name.is_empty() {
                name_error.set("Enter a name for the team.".to_string());
            }
            if game_id.is_empty() {
                game_error.set("Pick the game this team plays.".to_string());
            }
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
                        save_error.set(Some(
                            SaveState::failed("Could not save the team", &e).text(),
                        ));
                        toast.show(Toast::error_logged_elsewhere("Failed to save team", &e));
                    }
                }
            }
        });
    };

    // --- Roster handlers ---

    let mut open_roster = move |team: Team| {
        roster_data.set(Vec::new());
        roster_phase.set(RosterPhase::Loading);
        roster_error.set(String::new());
        add_member_id.set(String::new());
        add_member_role.set(ROSTER_ADD_DEFAULT_ROLE.to_string());
        add_member_error.set(String::new());
        add_error.set(None);
        roster_modal.show(team);
        roster_refresh += 1;
    };

    let on_roster_close = move |_| {
        roster_modal.close();
    };

    let on_add_member = move |_| {
        let member_id = add_member_id().trim().to_string();
        add_error.set(None);
        if member_id.is_empty() {
            add_member_error.set("Pick a member to add.".to_string());
            return;
        }
        add_member_error.set(String::new());
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
                        add_member_role.set(ROSTER_ADD_DEFAULT_ROLE.to_string());
                        roster_refresh += 1;
                    }
                    Err(e) => {
                        add_error.set(Some(
                            SaveState::failed("Could not add the member", &e).text(),
                        ));
                        toast.show(Toast::error_logged_elsewhere("Failed to add member", &e));
                    }
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
                    Err(e) => toast.show(Toast::error_from("Failed to update role", &e)),
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
            // B9: once the row is gone, focus the next member's role, else
            // the previous one's, else the add form's member picker.
            let keys: Vec<String> = roster_data
                .peek()
                .iter()
                .map(|e| e.member_id.clone())
                .collect();
            let focus_next = after_delete_focus_ids(
                &keys,
                &member_id,
                |k| vec![roster_role_id(k)],
                &[ROSTER_ADD_MEMBER_ID],
            );
            remove_modal.close();
            spawn(async move {
                match ApiClient::web()
                    .delete(&format!("/api/teams/{team_id}/roster/{member_id}"))
                    .await
                {
                    Ok(_) => {
                        toast.show(Toast::success("Member removed from roster."));
                        roster_refresh += 1;
                        focus_after_delete_soon(roster_remove_id(&member_id), focus_next);
                    }
                    Err(e) => toast.show(Toast::error_from("Remove failed", &e)),
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

    // Game names for the table, from the games list loaded for the form
    // (F-AUI-002). The API field wins when present.
    let game_name_of = move |team: &Team| -> String {
        team.game_name
            .clone()
            .or_else(|| {
                games
                    .data
                    .read()
                    .as_ref()
                    .and_then(|d| d.as_ref())
                    .and_then(|list| {
                        list.iter()
                            .find(|g| g.id == team.game_id)
                            .map(|g| g.name.clone())
                    })
            })
            .unwrap_or_else(|| "None".into())
    };

    // --- Render ---

    let list = {
        let data = teams.data.read();
        match data.as_ref().and_then(|d| d.as_ref()) {
            None => admin_list_pending(&teams, "teams"),
            Some(all) => {
                let total = all.len();
                let rows = page_slice(all, page(), ADMIN_PAGE_SIZE).to_vec();
                rsx! {
                    if all.is_empty() {
                        EmptyCard {
                            title: "No teams yet",
                            message: teams_empty_message(is_admin).to_string(),
                            actions: is_admin.then(|| rsx! {
                                button {
                                    r#type: "button",
                                    class: "btn is-primary",
                                    onclick: move |_| open_create(),
                                    "New team"
                                }
                            }),
                        }
                    } else {
                        DataTable {
                            headers: vec!["Team", "Game", "Division", "Color", "Actions"],
                            cards: true,
                            label: "Teams",
                            for team in rows.iter() {
                                Fragment { key: "{team.id}", {
                                    let t_edit = team.clone();
                                    let t_roster = team.clone();
                                    let name = team.name.clone();
                                    let game_display = game_name_of(team);
                                    let div_display = or_none(team.division.as_deref());
                                    let color_display = or_none(team.color.as_deref());
                                    let swatch = team.color.clone().filter(|c| !c.trim().is_empty());
                                    rsx! {
                                        tr { key: "{team.id}",
                                            td { class: "cell-who", "{team.name}" }
                                            td { class: "cell-meta", "{game_display}" }
                                            td { "data-label": "Division", "{div_display}" }
                                            td { "data-label": "Color",
                                                if let Some(c) = swatch {
                                                    TeamSwatch { color: c }
                                                }
                                                "{color_display}"
                                            }
                                            td { class: "cell-actions",
                                                RowActions {
                                                    if can_edit_teams {
                                                        button {
                                                            r#type: "button",
                                                            class: "btn is-ghost",
                                                            id: team_edit_button_id(&team.id),
                                                            aria_label: "Edit {name}",
                                                            onclick: move |_| open_edit(t_edit.clone()),
                                                            "Edit"
                                                        }
                                                    }
                                                    button {
                                                        r#type: "button",
                                                        class: "btn is-ghost",
                                                        id: team_roster_button_id(&team.id),
                                                        aria_label: "Roster of {name}",
                                                        onclick: move |_| open_roster(t_roster.clone()),
                                                        "Roster"
                                                    }
                                                }
                                            }
                                        }
                                    }
                                } }
                            }
                        }
                    }
                    // Always rendered: its count line is the list's live line.
                    Pager {
                        id: "teams-pager",
                        page: page(),
                        total,
                        page_size: ADMIN_PAGE_SIZE,
                        noun: "teams",
                        noun_singular: "team",
                        show_page_count: true,
                        on_change: move |next| {
                            page.set(next);
                            show_list_top(LIST_ID);
                        },
                    }
                }
            }
        }
    };

    let roster_team = roster_modal.get_target();
    let roster_title = format!(
        "Roster: {}",
        roster_team
            .as_ref()
            .map(|t| t.name.clone())
            .unwrap_or_default()
    );
    let roster_focus = roster_team.as_ref().map(|t| team_roster_button_id(&t.id));
    let remove_focus = remove_modal
        .get_target()
        .map(|e| roster_remove_id(&e.member_id));
    let roster_load_error = load_error_text("this roster", &roster_error());
    let games_loaded_empty = games
        .data
        .read()
        .as_ref()
        .and_then(|d| d.as_ref())
        .is_some_and(|list| list.is_empty());
    let add_member_invalid = !add_member_error().is_empty();

    rsx! {
        crate::styles::StyleOnce { id: "scuffed-css-pages-admin-teams-page-css", css: PAGE_CSS }
        PageShell { width: PageWidth::Wide, class: "admin-teams-page",
            PageHeader {
                title: "Teams",
                lead: teams_admin_intro(can_edit_teams).to_string(),
                actions: is_admin.then(|| rsx! {
                    button {
                        r#type: "button",
                        class: "btn is-primary",
                        id: NEW_TEAM_ID,
                        onclick: move |_| open_create(),
                        "New team"
                    }
                }),
            }

            section {
                id: LIST_ID,
                class: "admin-list pager-target",
                tabindex: "-1",
                aria_label: "Teams",
                {list}
            }

            {list_cap_notice(&teams, "teams")}
            {list_cap_notice(&games, "games")}
            {list_cap_notice(&members, "members")}
        }

        // Create/Edit Team modal
        FormModal {
            title: if modal.get_target().is_some() { "Edit team".to_string() } else { "New team".to_string() },
            open: modal.is_open(),
            submitting: modal.is_submitting(),
            on_close: on_close,
            on_submit: on_submit,
            submit_label: modal.get_target().is_none().then(|| "Create".to_string()),
            error: save_error(),
            restore_focus_id: Some(opener()),

            div { class: "form-field",
                label { class: "field-label", r#for: TEAM_NAME_ID, "Name" }
                input {
                    class: "field-input",
                    id: TEAM_NAME_ID,
                    r#type: "text",
                    value: "{form_name}",
                    aria_invalid: if !name_error().is_empty() { "true" },
                    aria_describedby: TEAM_NAME_ERROR_ID,
                    oninput: move |e| form_name.set(e.value()),
                }
                FieldError { id: TEAM_NAME_ERROR_ID.to_string(), text: name_error() }
            }
            div { class: "form-field",
                label { class: "field-label", r#for: TEAM_GAME_ID, "Game" }
                select {
                    class: "field-input",
                    id: TEAM_GAME_ID,
                    value: "{form_game_id}",
                    aria_invalid: if !game_error().is_empty() { "true" },
                    aria_describedby: TEAM_GAME_ERROR_ID,
                    onchange: move |e| form_game_id.set(e.value()),
                    option { value: "", "Pick a game" }
                    {
                        let games_data = games.data.read();
                        let games_data = games_data.as_ref().and_then(|d| d.as_ref());
                        match games_data {
                            Some(list) => rsx! {
                                for g in list.iter() {
                                    option { key: "{g.id}", value: "{g.id}", "{g.name}" }
                                }
                            },
                            None => rsx! {},
                        }
                    }
                }
                if games_loaded_empty {
                    p { class: "field-hint",
                        "No games yet. "
                        Link { to: Route::AdminGames {}, "Add a game first" }
                        "."
                    }
                }
                FieldError { id: TEAM_GAME_ERROR_ID.to_string(), text: game_error() }
            }
            div { class: "form-grid",
                div { class: "form-field",
                    label { class: "field-label", r#for: TEAM_DIVISION_ID, "Division (optional)" }
                    input {
                        class: "field-input",
                        id: TEAM_DIVISION_ID,
                        r#type: "text",
                        placeholder: "Division 1",
                        value: "{form_division}",
                        oninput: move |e| form_division.set(e.value()),
                    }
                }
                div { class: "form-field",
                    label { class: "field-label", r#for: TEAM_COLOR_ID, "Color (optional)" }
                    input {
                        class: "field-input",
                        id: TEAM_COLOR_ID,
                        r#type: "text",
                        placeholder: "#rrggbb",
                        aria_describedby: TEAM_COLOR_HINT_ID,
                        value: "{form_color}",
                        oninput: move |e| form_color.set(e.value()),
                    }
                    p { class: "field-hint", id: TEAM_COLOR_HINT_ID, "A hex colour, shown beside the team name." }
                }
            }
        }

        // Roster dialog: the roster table, then the add form. Its main
        // button adds the picked member; role changes and removals in the
        // table save right away.
        FormModal {
            title: roster_title,
            open: roster_modal.is_open(),
            submitting: add_submitting(),
            on_close: on_roster_close,
            on_submit: on_add_member,
            wide: true,
            submit_label: Some("Add to roster".to_string()),
            error: add_error(),
            restore_focus_id: roster_focus,
            dirty: Some(roster_draft_unsaved(&add_member_id(), &add_member_role())),

            p { class: "field-hint admin-roster-hint", id: ROSTER_HINT_ID,
                "Role changes and removals save right away. To add someone, pick a member and a role, then Add to roster."
            }
            match roster_view {
                RosterPanel::Loading => rsx! {
                    div { class: "admin-pending",
                        p { class: "admin-pending-text", role: "status", "Loading roster..." }
                        SkeletonBars { rows: 3 }
                    }
                },
                RosterPanel::Error => rsx! {
                    div { class: "fetch-error-wrap", role: "alert",
                        p { class: "fetch-error", "{roster_load_error}" }
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
                    p { class: "field-hint", "Nobody is on this roster yet. Add the first member below." }
                },
                RosterPanel::Rows => rsx! {
                    DataTable {
                        headers: vec!["Member", "Role", "Actions"],
                        cards: true,
                        label: "Roster",
                        for entry in roster_data.read().iter() {
                            Fragment { key: "{entry.member_id}", {
                                let e_remove = entry.clone();
                                let mid = entry.member_id.clone();
                                let current_role = entry.team_role.clone();
                                let member_name = entry.member_name.clone();
                                rsx! {
                                    tr { key: "{entry.member_id}",
                                        td { class: "cell-who", "{entry.member_name}" }
                                        td { "data-label": "Role",
                                            select {
                                                class: "field-input",
                                                id: roster_role_id(&entry.member_id),
                                                aria_label: "Role of {member_name}",
                                                value: "{current_role}",
                                                onchange: move |e| {
                                                    on_role_change((mid.clone(), e.value()));
                                                },
                                                for role in TEAM_ROLES.iter() {
                                                    option { key: "{role}", value: "{role}", "{status_label(role)}" }
                                                }
                                            }
                                        }
                                        td { class: "cell-actions",
                                            RowActions {
                                                button {
                                                    r#type: "button",
                                                    class: "btn is-danger",
                                                    id: roster_remove_id(&entry.member_id),
                                                    aria_label: "Remove {member_name}",
                                                    onclick: move |_| open_remove(e_remove.clone()),
                                                    "Remove"
                                                }
                                            }
                                        }
                                    }
                                }
                            } }
                        }
                    }
                },
            }

            div { class: "form-grid",
                div { class: "form-field",
                    label { class: "field-label", r#for: ROSTER_ADD_MEMBER_ID, "Member to add" }
                    select {
                        class: "field-input",
                        id: ROSTER_ADD_MEMBER_ID,
                        value: "{add_member_id}",
                        aria_invalid: if add_member_invalid { "true" },
                        aria_describedby: ROSTER_ADD_MEMBER_ERROR_ID,
                        onchange: move |e| add_member_id.set(e.value()),
                        option { value: "", "Pick a member" }
                        {
                            let mems = members.data.read();
                            let mems = mems.as_ref().and_then(|d| d.as_ref());
                            match mems {
                                Some(list) => rsx! {
                                    for m in list.iter() {
                                        option { key: "{m.id}", value: "{m.id}", "{m.display_name}" }
                                    }
                                },
                                None => rsx! {},
                            }
                        }
                    }
                    FieldError { id: ROSTER_ADD_MEMBER_ERROR_ID.to_string(), text: add_member_error() }
                }
                div { class: "form-field",
                    label { class: "field-label", r#for: ROSTER_ADD_ROLE_ID, "Role" }
                    select {
                        class: "field-input",
                        id: ROSTER_ADD_ROLE_ID,
                        value: "{add_member_role}",
                        onchange: move |e| add_member_role.set(e.value()),
                        for role in TEAM_ROLES.iter() {
                            option { key: "{role}", value: "{role}", "{status_label(role)}" }
                        }
                    }
                }
            }
        }

        // Remove roster member confirm
        ConfirmDialog {
            title: "Remove from roster".to_string(),
            message: format!(
                "Remove \"{}\" from this team's roster?",
                remove_modal.get_target().map(|e| e.member_name).unwrap_or_default()
            ),
            open: remove_modal.is_open(),
            danger: true,
            confirm_label: "Remove".to_string(),
            restore_focus_id: remove_focus,
            on_confirm: on_remove_confirm,
            on_cancel: on_remove_cancel,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_roster_dialog_asks_only_about_its_add_form() {
        // Vision S4 on PR 706: a role change or a removal saved right away,
        // then closing asked about unsaved changes. The draft is the add
        // form against its empty baseline, which a successful add resets.
        assert!(!roster_draft_unsaved("", ROSTER_ADD_DEFAULT_ROLE));
        assert!(!roster_draft_unsaved("  ", "player"));
        assert!(roster_draft_unsaved("member:1", ROSTER_ADD_DEFAULT_ROLE));
        assert!(roster_draft_unsaved("", "captain"));
        let src = include_str!("teams.rs");
        let prod = src.split("#[cfg(test)]").next().unwrap();
        assert!(
            prod.contains(
                "dirty: Some(roster_draft_unsaved(&add_member_id(), &add_member_role())),"
            )
        );
        // The add resets the form to the same baseline it opens with.
        assert_eq!(
            prod.matches("add_member_role.set(ROSTER_ADD_DEFAULT_ROLE.to_string());")
                .count(),
            2
        );
        assert!(prod.contains("use_signal(|| ROSTER_ADD_DEFAULT_ROLE.to_string())"));
    }

    #[test]
    fn a_roster_removal_moves_focus_to_the_next_member() {
        // B9: the removed row's button goes with it.
        let src = include_str!("teams.rs");
        let prod = src.split("#[cfg(test)]").next().unwrap();
        let confirm = prod
            .split("let on_remove_confirm = move |_| {")
            .nth(1)
            .and_then(|rest| rest.split("let on_remove_cancel").next())
            .unwrap();
        assert!(
            confirm.contains("|k| vec![roster_role_id(k)],"),
            "{confirm}"
        );
        assert!(confirm.contains("&[ROSTER_ADD_MEMBER_ID],"), "{confirm}");
        assert!(
            confirm.contains("focus_after_delete_soon(roster_remove_id(&member_id), focus_next);"),
            "{confirm}"
        );
    }

    /// This sheet sets no type, so the shared check (which needs a
    /// font-size) does not apply: no rem, no px past a hairline, no hex.
    fn assert_page_css_on_tokens(css: &str) {
        for word in css.split(|c: char| c.is_whitespace() || ";:(),".contains(c)) {
            assert!(!word.ends_with("rem"), "{word}");
            assert!(!word.ends_with("px") || word == "1px", "{word}");
            assert!(!word.starts_with('#'), "{word}");
        }
        assert!(
            !css.contains("font-size"),
            "type goes through assert_sizes_from_tokens"
        );
    }

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
    fn a_roster_reload_keeps_the_rows_and_their_focus() {
        // Vision blocking 9 on PR 706: a role change swapped the table for a
        // skeleton, removing the focused select, and Tab left the dialog.
        assert_eq!(
            roster_load_phase(RosterPhase::Ready),
            RosterPhase::Refreshing
        );
        assert_eq!(
            roster_load_phase(RosterPhase::Refreshing),
            RosterPhase::Refreshing
        );
        assert_eq!(roster_panel(RosterPhase::Refreshing, 2), RosterPanel::Rows);
        assert_eq!(roster_panel(RosterPhase::Refreshing, 0), RosterPanel::Empty);
        // A first load and a retry after a failure still show the wait.
        assert_eq!(
            roster_load_phase(RosterPhase::Loading),
            RosterPhase::Loading
        );
        assert_eq!(roster_load_phase(RosterPhase::Failed), RosterPhase::Loading);
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

    #[test]
    fn page_has_one_h1_and_a_labelled_wait_in_the_wide_shell() {
        use crate::components::admin_shared::test_page::render_admin_route;
        let html = render_admin_route("/admin/teams", Some(OrgRole::Admin));
        assert!(
            html.contains("class=\"ui-shell ui-shell--wide admin-teams-page\""),
            "{html}"
        );
        assert_eq!(html.matches("<h1").count(), 1, "{html}");
        assert!(html.contains("<h1>Teams</h1>"), "{html}");
        assert_eq!(html.matches("<main").count(), 1, "{html}");
        assert!(html.contains("id=\"teams-new\""), "{html}");
        assert!(
            html.contains(
                "<section id=\"admin-teams-list\" class=\"admin-list pager-target\" tabindex=\"-1\" aria-label=\"Teams\">"
            ),
            "{html}"
        );
        assert!(
            html.contains("role=\"status\">Loading teams...</p>"),
            "{html}"
        );
        // Officers manage rosters but cannot create teams.
        let officer = render_admin_route("/admin/teams", Some(OrgRole::Officer));
        assert!(!officer.contains("id=\"teams-new\""), "{officer}");
    }

    #[test]
    fn team_swatch_draws_the_colour_without_inline_css() {
        fn view() -> Element {
            rsx! { TeamSwatch { color: "red".to_string() } }
        }
        let html = crate::components::ui::test_support::render(view);
        assert!(html.contains("fill=\"red\""), "{html}");
        assert!(html.contains("aria-hidden=\"true\""), "{html}");
        assert!(!html.contains("style="), "{html}");
        assert_page_css_on_tokens(PAGE_CSS);
    }

    #[test]
    fn every_field_has_a_tied_label_and_ids_come_from_data() {
        let src = include_str!("teams.rs");
        let prod = src.split("mod tests").next().unwrap();
        for id in [
            "TEAM_NAME_ID",
            "TEAM_GAME_ID",
            "TEAM_DIVISION_ID",
            "TEAM_COLOR_ID",
            "ROSTER_ADD_MEMBER_ID",
            "ROSTER_ADD_ROLE_ID",
        ] {
            assert!(prod.contains(&format!("r#for: {id}")), "{id}");
            assert!(prod.contains(&format!("id: {id},")), "{id}");
        }
        assert!(!prod.contains("style:"), "no inline styles");
        assert!(!prod.contains('\u{2014}'));
        assert_eq!(roster_role_id("m1"), "roster-role-m1");
        assert_eq!(team_roster_button_id("t1"), "team-roster-t1");
        assert_eq!(or_none(Some(" ")), "None");
    }
}
