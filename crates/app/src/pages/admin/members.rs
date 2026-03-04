use dioxus::prelude::*;
use serde::Deserialize;

use scuffed_api_client::ApiClient;
use scuffed_types::api::{ChangeRoleRequest, ToggleActiveRequest, CreateGameAccountRequest};
use crate::components::{
    DataTable, FormModal, ConfirmDialog, StatusPill, RolePill, SummaryCard, Toast, use_toast,
    ADMIN_SHARED_CSS,
};

// --- Types ---
// These local types have API-enriched fields (joined names, computed stats)
// that differ from the base org types in scuffed_types.

#[derive(Debug, Clone, Deserialize)]
struct Member {
    id: String,
    display_name: String,
    org_role: String,
    is_active: bool,
    joined_at: String,
}

#[derive(Debug, Clone, Deserialize)]
struct GameAccount {
    id: String,
    game_id: String,
    account_name: String,
    account_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct Game {
    id: String,
    name: String,
}

#[derive(Debug, Clone, Deserialize)]
struct ModerationAction {
    id: String,
    action_type: String,
    reason: String,
    is_active: bool,
    created_at: String,
}

#[derive(Debug, Clone, Deserialize)]
struct AttendanceStats {
    total_events: u32,
    attended: u32,
    absent: u32,
    excused: u32,
    attendance_rate: f64,
}

const ROLES: [&str; 4] = ["recruit", "member", "officer", "admin"];

#[component]
pub fn AdminMembers() -> Element {
    let mut refresh = use_signal(|| 0u64);
    let mut toast = use_toast();

    // Data
    let members = use_resource(move || async move {
        let _ = refresh();
        ApiClient::web().fetch::<Vec<Member>>("/api/members").await.ok()
    });

    let games = use_resource(move || async move {
        let _ = refresh();
        ApiClient::web().fetch::<Vec<Game>>("/api/games").await.ok()
    });

    // Role change modal
    let mut role_open = use_signal(|| false);
    let mut role_target: Signal<Option<Member>> = use_signal(|| None);
    let mut role_value = use_signal(String::new);
    let mut role_submitting = use_signal(|| false);

    // Toggle active confirm
    let mut toggle_open = use_signal(|| false);
    let mut toggle_target: Signal<Option<Member>> = use_signal(|| None);

    // Mod history modal
    let mut mod_open = use_signal(|| false);
    let mut mod_target: Signal<Option<Member>> = use_signal(|| None);
    let mut mod_data: Signal<Vec<ModerationAction>> = use_signal(Vec::new);
    let mut mod_loading = use_signal(|| false);

    // Attendance stats modal
    let mut stats_open = use_signal(|| false);
    let mut stats_target: Signal<Option<Member>> = use_signal(|| None);
    let mut stats_data: Signal<Option<AttendanceStats>> = use_signal(|| None);
    let mut stats_loading = use_signal(|| false);

    // Game accounts modal
    let mut accts_open = use_signal(|| false);
    let mut accts_target: Signal<Option<Member>> = use_signal(|| None);
    let mut accts_data: Signal<Vec<GameAccount>> = use_signal(Vec::new);
    let mut accts_refresh = use_signal(|| 0u64);
    let mut accts_loading = use_signal(|| false);

    // Add game account form
    let mut add_acct_game_id = use_signal(String::new);
    let mut add_acct_name = use_signal(String::new);
    let mut add_acct_id = use_signal(String::new);
    let mut add_acct_submitting = use_signal(|| false);

    // Delete game account confirm
    let mut del_acct_open = use_signal(|| false);
    let mut del_acct_target: Signal<Option<GameAccount>> = use_signal(|| None);

    // --- Fetch helpers for sub-modals ---

    let _accts_loader = use_resource(move || async move {
        let _ = accts_refresh();
        if let Some(member) = accts_target() {
            accts_loading.set(true);
            if let Ok(list) = ApiClient::web()
                .fetch::<Vec<GameAccount>>(&format!("/api/members/{}/game-accounts", member.id))
                .await
            {
                accts_data.set(list);
            }
            accts_loading.set(false);
        }
    });

    // --- Role change handlers ---

    let mut open_role = move |member: Member| {
        role_value.set(member.org_role.clone());
        role_target.set(Some(member));
        role_open.set(true);
    };

    let on_role_close = move |_| {
        role_open.set(false);
        role_target.set(None);
    };

    let on_role_submit = move |_| {
        if let Some(member) = role_target() {
            let id = member.id.clone();
            let body = ChangeRoleRequest { role: role_value() };
            role_submitting.set(true);
            spawn(async move {
                let result = ApiClient::web()
                    .patch_json_empty(&format!("/api/members/{id}/role"), &body)
                    .await;
                role_submitting.set(false);
                match result {
                    Ok(_) => {
                        toast.show(Toast::success("Role updated."));
                        role_open.set(false);
                        role_target.set(None);
                        refresh += 1;
                    }
                    Err(e) => toast.show(Toast::error(format!("Failed to change role: {e}"))),
                }
            });
        }
    };

    // --- Toggle active handlers ---

    let mut open_toggle = move |member: Member| {
        toggle_target.set(Some(member));
        toggle_open.set(true);
    };

    let on_toggle_confirm = move |_| {
        if let Some(member) = toggle_target() {
            let id = member.id.clone();
            let new_active = !member.is_active;
            let body = ToggleActiveRequest { is_active: Some(new_active) };
            spawn(async move {
                let result = ApiClient::web()
                    .put_json_empty(&format!("/api/members/{id}"), &body)
                    .await;
                match result {
                    Ok(_) => {
                        let action = if new_active { "activated" } else { "deactivated" };
                        toast.show(Toast::success(format!("Member {action}.")));
                        toggle_open.set(false);
                        toggle_target.set(None);
                        refresh += 1;
                    }
                    Err(e) => toast.show(Toast::error(format!("Failed: {e}"))),
                }
            });
        }
    };

    let on_toggle_cancel = move |_| {
        toggle_open.set(false);
        toggle_target.set(None);
    };

    // --- Mod history handlers ---

    let mut open_mod_history = move |member: Member| {
        mod_target.set(Some(member.clone()));
        mod_data.set(Vec::new());
        mod_loading.set(true);
        mod_open.set(true);
        let mid = member.id.clone();
        spawn(async move {
            if let Ok(list) = ApiClient::web()
                .fetch::<Vec<ModerationAction>>(&format!("/api/members/{mid}/moderation"))
                .await
            {
                mod_data.set(list);
            }
            mod_loading.set(false);
        });
    };

    let mut on_mod_close = move |_| {
        mod_open.set(false);
        mod_target.set(None);
    };

    // --- Stats handlers ---

    let mut open_stats = move |member: Member| {
        stats_target.set(Some(member.clone()));
        stats_data.set(None);
        stats_loading.set(true);
        stats_open.set(true);
        let mid = member.id.clone();
        spawn(async move {
            if let Ok(data) = ApiClient::web()
                .fetch::<AttendanceStats>(&format!("/api/members/{mid}/attendance/stats"))
                .await
            {
                stats_data.set(Some(data));
            }
            stats_loading.set(false);
        });
    };

    let mut on_stats_close = move |_| {
        stats_open.set(false);
        stats_target.set(None);
    };

    // --- Game accounts handlers ---

    let mut open_accounts = move |member: Member| {
        accts_target.set(Some(member));
        accts_data.set(Vec::new());
        add_acct_game_id.set(String::new());
        add_acct_name.set(String::new());
        add_acct_id.set(String::new());
        accts_refresh += 1;
        accts_open.set(true);
    };

    let mut on_accts_close = move |_| {
        accts_open.set(false);
        accts_target.set(None);
    };

    let on_add_acct = move |_| {
        let game_id = add_acct_game_id().trim().to_string();
        let acct_name = add_acct_name().trim().to_string();
        if game_id.is_empty() || acct_name.is_empty() {
            toast.show(Toast::error("Game and account name are required."));
            return;
        }
        let acct_id_raw = add_acct_id().trim().to_string();
        let body = CreateGameAccountRequest {
            game_id,
            account_name: acct_name,
            account_id: if acct_id_raw.is_empty() { None } else { Some(acct_id_raw) },
        };
        if let Some(member) = accts_target() {
            let mid = member.id.clone();
            add_acct_submitting.set(true);
            spawn(async move {
                let result = ApiClient::web()
                    .put_json_empty(&format!("/api/members/{mid}/game-accounts"), &body)
                    .await;
                add_acct_submitting.set(false);
                match result {
                    Ok(_) => {
                        toast.show(Toast::success("Game account added."));
                        add_acct_game_id.set(String::new());
                        add_acct_name.set(String::new());
                        add_acct_id.set(String::new());
                        accts_refresh += 1;
                    }
                    Err(e) => toast.show(Toast::error(format!("Failed to add account: {e}"))),
                }
            });
        }
    };

    let mut open_del_acct = move |acct: GameAccount| {
        del_acct_target.set(Some(acct));
        del_acct_open.set(true);
    };

    let on_del_acct_confirm = move |_| {
        if let Some(acct) = del_acct_target() {
            if let Some(member) = accts_target() {
                let mid = member.id.clone();
                let aid = acct.id.clone();
                spawn(async move {
                    match ApiClient::web()
                        .delete(&format!("/api/members/{mid}/game-accounts/{aid}"))
                        .await
                    {
                        Ok(_) => {
                            toast.show(Toast::success("Game account removed."));
                            del_acct_open.set(false);
                            del_acct_target.set(None);
                            accts_refresh += 1;
                        }
                        Err(e) => toast.show(Toast::error(format!("Delete failed: {e}"))),
                    }
                });
            }
        }
    };

    let on_del_acct_cancel = move |_| {
        del_acct_open.set(false);
        del_acct_target.set(None);
    };

    // --- Render ---

    rsx! {
        style { {ADMIN_SHARED_CSS} }

        div { class: "admin-toolbar",
            h1 { "Members" }
        }

        // Members table
        {
            let data = members.read();
            let data = data.as_ref().and_then(|d| d.as_ref());
            match data {
                None => rsx! { p { class: "admin-loading", "Loading..." } },
                Some(list) if list.is_empty() => rsx! {
                    p { class: "empty-state", "No members yet." }
                },
                Some(list) => rsx! {
                    DataTable { headers: vec!["Name", "Role", "Status", "Joined", "Actions"],
                        for member in list.iter() {
                            {
                                let m_role = member.clone();
                                let m_toggle = member.clone();
                                let m_mod = member.clone();
                                let m_stats = member.clone();
                                let m_accts = member.clone();
                                let status_str = if member.is_active { "active" } else { "inactive" };
                                let toggle_label = if member.is_active { "Deactivate" } else { "Activate" };
                                rsx! {
                                    tr { key: "{member.id}",
                                        td { "{member.display_name}" }
                                        td { RolePill { role: member.org_role.clone() } }
                                        td { StatusPill { status: status_str.to_string() } }
                                        td { "{member.joined_at}" }
                                        td {
                                            div { class: "row-actions",
                                                button {
                                                    class: "row-btn",
                                                    onclick: move |_| open_role(m_role.clone()),
                                                    "Role"
                                                }
                                                button {
                                                    class: "row-btn danger",
                                                    onclick: move |_| open_toggle(m_toggle.clone()),
                                                    "{toggle_label}"
                                                }
                                                button {
                                                    class: "row-btn",
                                                    onclick: move |_| open_mod_history(m_mod.clone()),
                                                    "Mod"
                                                }
                                                button {
                                                    class: "row-btn",
                                                    onclick: move |_| open_stats(m_stats.clone()),
                                                    "Stats"
                                                }
                                                button {
                                                    class: "row-btn",
                                                    onclick: move |_| open_accounts(m_accts.clone()),
                                                    "Accounts"
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

        // Role change modal
        FormModal {
            title: format!(
                "Change Role: {}",
                role_target().map(|m| m.display_name).unwrap_or_default()
            ),
            open: role_open(),
            submitting: role_submitting(),
            on_close: on_role_close,
            on_submit: on_role_submit,

            div { class: "form-field",
                label { class: "form-label", "Role" }
                select {
                    class: "form-select",
                    value: "{role_value}",
                    onchange: move |e| role_value.set(e.value()),
                    for role in ROLES.iter() {
                        option { value: "{role}", "{role}" }
                    }
                }
            }
        }

        // Toggle active confirm
        ConfirmDialog {
            title: if toggle_target().map(|m| m.is_active).unwrap_or(false) {
                "Deactivate Member".to_string()
            } else {
                "Activate Member".to_string()
            },
            message: format!(
                "{} \"{}\"?",
                if toggle_target().map(|m| m.is_active).unwrap_or(false) { "Deactivate" } else { "Activate" },
                toggle_target().map(|m| m.display_name).unwrap_or_default()
            ),
            open: toggle_open(),
            danger: toggle_target().map(|m| m.is_active).unwrap_or(false),
            on_confirm: on_toggle_confirm,
            on_cancel: on_toggle_cancel,
        }

        // Mod history modal
        if mod_open() {
            div {
                class: "form-modal-overlay",
                onclick: move |_| on_mod_close(()),
                div {
                    class: "form-modal",
                    style: "max-width:700px;",
                    onclick: move |e| e.stop_propagation(),
                    div { class: "form-modal-header",
                        "Moderation: {mod_target().map(|m| m.display_name).unwrap_or_default()}"
                    }
                    div { class: "form-modal-body",
                        if mod_loading() {
                            p { class: "admin-loading", "Loading..." }
                        } else if mod_data.read().is_empty() {
                            p { class: "empty-state", "No moderation history." }
                        } else {
                            table { class: "data-table",
                                thead {
                                    tr {
                                        th { "Action" }
                                        th { "Reason" }
                                        th { "Active" }
                                        th { "Date" }
                                    }
                                }
                                tbody {
                                    for action in mod_data.read().iter() {
                                        {
                                            let active_str = if action.is_active { "active" } else { "inactive" };
                                            rsx! {
                                                tr { key: "{action.id}",
                                                    td { "{action.action_type}" }
                                                    td { "{action.reason}" }
                                                    td { StatusPill { status: active_str.to_string() } }
                                                    td { "{action.created_at}" }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    div { class: "form-modal-footer",
                        button {
                            class: "btn-cancel",
                            onclick: move |_| on_mod_close(()),
                            "Close"
                        }
                    }
                }
            }
        }

        // Attendance stats modal
        if stats_open() {
            div {
                class: "form-modal-overlay",
                onclick: move |_| on_stats_close(()),
                div {
                    class: "form-modal",
                    style: "max-width:550px;",
                    onclick: move |e| e.stop_propagation(),
                    div { class: "form-modal-header",
                        "Attendance: {stats_target().map(|m| m.display_name).unwrap_or_default()}"
                    }
                    div { class: "form-modal-body",
                        if stats_loading() {
                            p { class: "admin-loading", "Loading..." }
                        } else if let Some(stats) = stats_data() {
                            div { class: "summary-cards",
                                SummaryCard { value: stats.total_events.to_string(), label: "Total Events" }
                                SummaryCard { value: stats.attended.to_string(), label: "Attended" }
                                SummaryCard { value: stats.absent.to_string(), label: "Absent" }
                                SummaryCard { value: stats.excused.to_string(), label: "Excused" }
                            }
                            div {
                                style: "text-align:center;margin-top:1rem;",
                                span {
                                    style: "font-family:'Bebas Neue',sans-serif;font-size:2.5rem;color:var(--accent);",
                                    "{stats.attendance_rate:.1}%"
                                }
                                div {
                                    style: "font-size:0.75rem;color:var(--text-muted);text-transform:uppercase;letter-spacing:0.05em;",
                                    "Attendance Rate"
                                }
                            }
                        } else {
                            p { class: "empty-state", "No attendance data." }
                        }
                    }
                    div { class: "form-modal-footer",
                        button {
                            class: "btn-cancel",
                            onclick: move |_| on_stats_close(()),
                            "Close"
                        }
                    }
                }
            }
        }

        // Game accounts modal
        if accts_open() {
            div {
                class: "form-modal-overlay",
                onclick: move |_| on_accts_close(()),
                div {
                    class: "form-modal",
                    style: "max-width:700px;",
                    onclick: move |e| e.stop_propagation(),
                    div { class: "form-modal-header",
                        "Game Accounts: {accts_target().map(|m| m.display_name).unwrap_or_default()}"
                    }
                    div { class: "form-modal-body",
                        if accts_loading() {
                            p { class: "admin-loading", "Loading..." }
                        } else if accts_data.read().is_empty() {
                            p { class: "empty-state", "No game accounts linked." }
                        } else {
                            table { class: "data-table",
                                thead {
                                    tr {
                                        th { "Game" }
                                        th { "Account Name" }
                                        th { "Account ID" }
                                        th { "Actions" }
                                    }
                                }
                                tbody {
                                    for acct in accts_data.read().iter() {
                                        {
                                            let a_del = acct.clone();
                                            let acct_id_display = acct.account_id.clone().unwrap_or_else(|| "\u{2014}".into());
                                            rsx! {
                                                tr { key: "{acct.id}",
                                                    td { "{acct.game_id}" }
                                                    td { "{acct.account_name}" }
                                                    td { "{acct_id_display}" }
                                                    td {
                                                        button {
                                                            class: "row-btn danger",
                                                            onclick: move |_| open_del_acct(a_del.clone()),
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

                        // Add account form
                        div {
                            style: "border-top:1px solid var(--border);padding-top:1rem;margin-top:1rem;",
                            h3 {
                                style: "font-family:'Rajdhani',sans-serif;font-size:0.9rem;font-weight:700;color:var(--text-bright);text-transform:uppercase;margin-bottom:0.75rem;",
                                "Add Account"
                            }
                            div { style: "display:flex;gap:0.5rem;flex-wrap:wrap;align-items:flex-end;",
                                div { class: "form-field", style: "min-width:120px;",
                                    label { class: "form-label", "Game" }
                                    select {
                                        class: "form-select",
                                        value: "{add_acct_game_id}",
                                        onchange: move |e| add_acct_game_id.set(e.value()),
                                        option { value: "", "-- Select --" }
                                        {
                                            let g = games.read();
                                            let g = g.as_ref().and_then(|d| d.as_ref());
                                            match g {
                                                Some(list) => rsx! {
                                                    for game in list.iter() {
                                                        option { value: "{game.id}", "{game.name}" }
                                                    }
                                                },
                                                None => rsx! {},
                                            }
                                        }
                                    }
                                }
                                div { class: "form-field", style: "flex:1;min-width:120px;",
                                    label { class: "form-label", "Account Name" }
                                    input {
                                        class: "form-input",
                                        r#type: "text",
                                        placeholder: "e.g. Player#1234",
                                        value: "{add_acct_name}",
                                        oninput: move |e| add_acct_name.set(e.value()),
                                    }
                                }
                                div { class: "form-field", style: "flex:1;min-width:100px;",
                                    label { class: "form-label", "Account ID (optional)" }
                                    input {
                                        class: "form-input",
                                        r#type: "text",
                                        value: "{add_acct_id}",
                                        oninput: move |e| add_acct_id.set(e.value()),
                                    }
                                }
                                button {
                                    class: "btn-save",
                                    disabled: add_acct_submitting(),
                                    onclick: on_add_acct,
                                    if add_acct_submitting() { "Adding..." } else { "Add" }
                                }
                            }
                        }
                    }
                    div { class: "form-modal-footer",
                        button {
                            class: "btn-cancel",
                            onclick: move |_| on_accts_close(()),
                            "Close"
                        }
                    }
                }
            }
        }

        // Delete game account confirm
        ConfirmDialog {
            title: "Remove Game Account".to_string(),
            message: format!(
                "Remove account \"{}\"?",
                del_acct_target().map(|a| a.account_name).unwrap_or_default()
            ),
            open: del_acct_open(),
            danger: true,
            on_confirm: on_del_acct_confirm,
            on_cancel: on_del_acct_cancel,
        }
    }
}
