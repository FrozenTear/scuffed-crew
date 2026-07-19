use dioxus::prelude::*;
use serde::{Deserialize, Serialize};
use wasm_bindgen::JsCast;

use crate::components::{
    ConfirmDialog, DataTable, FormModal, RolePill, StatusPill, SummaryCard, Toast, admin_pending,
    use_toast,
};
use crate::hooks::{ModalController, use_api, use_api_list};
use scuffed_api_client::ApiClient;
use scuffed_types::api::{ChangeRoleRequest, CreateGameAccountRequest, ToggleActiveRequest};

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

#[derive(Debug, Clone, Deserialize)]
struct UploadResponse {
    url: String,
}

#[derive(Serialize)]
struct UpdateAvatarBody {
    avatar_url: Option<String>,
}

const ROLES: [&str; 4] = ["recruit", "member", "officer", "admin"];

#[component]
pub fn AdminMembers() -> Element {
    let mut members = use_api_list::<Member>("/api/members");
    let mut games = use_api::<Vec<Game>>("/api/games");
    let mut toast = use_toast();

    // Role change modal
    let mut role_modal = ModalController::<Member>::new();
    let mut role_value = use_signal(String::new);

    // Toggle active confirm
    let mut toggle_modal = ModalController::<Member>::new();

    // Mod history modal
    let mut mod_modal = ModalController::<Member>::new();
    let mut mod_data: Signal<Vec<ModerationAction>> = use_signal(Vec::new);
    let mut mod_loading = use_signal(|| false);
    // Distinguish a failed fetch from a genuinely-empty record (FRONT-003):
    // an unsurfaced error would render a clean moderation history for a member
    // who actually has one.
    let mut mod_error: Signal<Option<String>> = use_signal(|| None);

    // Attendance stats modal
    let mut stats_modal = ModalController::<Member>::new();
    let mut stats_data: Signal<Option<AttendanceStats>> = use_signal(|| None);
    let mut stats_loading = use_signal(|| false);
    let mut stats_error: Signal<Option<String>> = use_signal(|| None);

    // Game accounts modal
    let mut accts_modal = ModalController::<Member>::new();
    let mut accts_data: Signal<Vec<GameAccount>> = use_signal(Vec::new);
    let mut accts_refresh = use_signal(|| 0u64);
    let mut accts_loading = use_signal(|| false);
    let mut accts_error: Signal<Option<String>> = use_signal(|| None);

    // Add game account form
    let mut add_acct_game_id = use_signal(String::new);
    let mut add_acct_name = use_signal(String::new);
    let mut add_acct_id = use_signal(String::new);
    let mut add_acct_submitting = use_signal(|| false);

    // Delete game account confirm
    let mut del_acct_modal = ModalController::<GameAccount>::new();

    // Local password reset modal (admin recovery path — local accounts have no email)
    let mut pw_modal = ModalController::<Member>::new();
    let mut pw_new = use_signal(String::new);

    // Avatar upload modal
    let mut avatar_modal = ModalController::<Member>::new();
    let mut avatar_file: Signal<Option<web_sys::File>> = use_signal(|| None);
    let mut avatar_uploading = use_signal(|| false);

    // --- Fetch helpers for sub-modals ---

    let _accts_loader = use_resource(move || async move {
        let _ = accts_refresh();
        if let Some(member) = accts_modal.get_target() {
            accts_loading.set(true);
            accts_error.set(None);
            match ApiClient::web()
                .fetch::<Vec<GameAccount>>(&format!("/api/members/{}/game-accounts", member.id))
                .await
            {
                Ok(list) => accts_data.set(list),
                Err(e) => accts_error.set(Some(e.to_string())),
            }
            accts_loading.set(false);
        }
    });

    // --- Role change handlers ---

    let mut open_role = move |member: Member| {
        role_value.set(member.org_role.clone());
        role_modal.show(member);
    };

    let on_role_close = move |_| {
        role_modal.close();
    };

    let on_pw_close = move |_| {
        pw_new.set(String::new());
        pw_modal.close();
    };

    let on_pw_submit = move |_| {
        if let Some(member) = pw_modal.get_target() {
            let id = member.id.clone();
            let new_password = pw_new();
            if new_password.len() < 12 {
                toast.show(Toast::error("Password must be at least 12 characters."));
                return;
            }
            pw_modal.start_submit();
            spawn(async move {
                let result = ApiClient::web()
                    .post_json_empty(
                        &format!("/api/members/{id}/reset-password"),
                        &serde_json::json!({ "new_password": new_password }),
                    )
                    .await;
                pw_modal.end_submit();
                match result {
                    Ok(_) => {
                        toast.show(Toast::success(
                            "Password reset. Share it with the member securely.",
                        ));
                        pw_new.set(String::new());
                        pw_modal.close();
                    }
                    Err(e) => toast.show(Toast::error(format!("Reset failed: {e}"))),
                }
            });
        }
    };

    let on_role_submit = move |_| {
        if let Some(member) = role_modal.get_target() {
            // Selection unchanged — nothing to do (the server rejects same-role changes).
            if role_value() == member.org_role {
                role_modal.close();
                return;
            }
            let id = member.id.clone();
            let body = ChangeRoleRequest { role: role_value() };
            role_modal.start_submit();
            spawn(async move {
                let result = ApiClient::web()
                    .patch_json_empty(&format!("/api/members/{id}/role"), &body)
                    .await;
                role_modal.end_submit();
                match result {
                    Ok(_) => {
                        toast.show(Toast::success("Role updated."));
                        role_modal.close();
                        members.refresh += 1;
                        games.refresh += 1;
                    }
                    Err(e) => toast.show(Toast::error(format!("Failed to change role: {e}"))),
                }
            });
        }
    };

    // --- Toggle active handlers ---

    let mut open_toggle = move |member: Member| {
        toggle_modal.show(member);
    };

    let on_toggle_confirm = move |_| {
        if let Some(member) = toggle_modal.get_target() {
            let id = member.id.clone();
            let new_active = !member.is_active;
            let body = ToggleActiveRequest {
                is_active: Some(new_active),
            };
            toggle_modal.close();
            spawn(async move {
                let result = ApiClient::web()
                    .put_json_empty(&format!("/api/members/{id}"), &body)
                    .await;
                match result {
                    Ok(_) => {
                        let action = if new_active {
                            "activated"
                        } else {
                            "deactivated"
                        };
                        toast.show(Toast::success(format!("Member {action}.")));
                        members.refresh += 1;
                        games.refresh += 1;
                    }
                    Err(e) => toast.show(Toast::error(format!("Failed: {e}"))),
                }
            });
        }
    };

    let on_toggle_cancel = move |_| {
        toggle_modal.close();
    };

    // --- Mod history handlers ---

    let mut open_mod_history = move |member: Member| {
        mod_data.set(Vec::new());
        mod_error.set(None);
        mod_loading.set(true);
        let mid = member.id.clone();
        mod_modal.show(member);
        spawn(async move {
            match ApiClient::web()
                .fetch::<Vec<ModerationAction>>(&format!("/api/members/{mid}/moderation"))
                .await
            {
                Ok(list) => mod_data.set(list),
                Err(e) => mod_error.set(Some(e.to_string())),
            }
            mod_loading.set(false);
        });
    };

    let mut on_mod_close = move |_| {
        mod_modal.close();
    };

    // --- Stats handlers ---

    let mut open_stats = move |member: Member| {
        stats_data.set(None);
        stats_error.set(None);
        stats_loading.set(true);
        let mid = member.id.clone();
        stats_modal.show(member);
        spawn(async move {
            match ApiClient::web()
                .fetch::<AttendanceStats>(&format!("/api/members/{mid}/attendance/stats"))
                .await
            {
                Ok(data) => stats_data.set(Some(data)),
                Err(e) => stats_error.set(Some(e.to_string())),
            }
            stats_loading.set(false);
        });
    };

    let mut on_stats_close = move |_| {
        stats_modal.close();
    };

    // --- Game accounts handlers ---

    let mut open_accounts = move |member: Member| {
        accts_data.set(Vec::new());
        add_acct_game_id.set(String::new());
        add_acct_name.set(String::new());
        add_acct_id.set(String::new());
        accts_refresh += 1;
        accts_modal.show(member);
    };

    let mut on_accts_close = move |_| {
        accts_modal.close();
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
            account_id: if acct_id_raw.is_empty() {
                None
            } else {
                Some(acct_id_raw)
            },
        };
        if let Some(member) = accts_modal.get_target() {
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
        del_acct_modal.show(acct);
    };

    let on_del_acct_confirm = move |_| {
        if let Some(acct) = del_acct_modal.get_target()
            && let Some(member) = accts_modal.get_target()
        {
            let mid = member.id.clone();
            let aid = acct.id.clone();
            del_acct_modal.close();
            spawn(async move {
                match ApiClient::web()
                    .delete(&format!("/api/members/{mid}/game-accounts/{aid}"))
                    .await
                {
                    Ok(_) => {
                        toast.show(Toast::success("Game account removed."));
                        accts_refresh += 1;
                    }
                    Err(e) => toast.show(Toast::error(format!("Delete failed: {e}"))),
                }
            });
        }
    };

    let on_del_acct_cancel = move |_| {
        del_acct_modal.close();
    };

    // --- Avatar handlers ---

    let mut open_avatar = move |member: Member| {
        avatar_file.set(None);
        avatar_modal.show(member);
    };

    let mut on_avatar_close = move |_| {
        avatar_modal.close();
    };

    let on_avatar_file_change = move |_e: Event<FormData>| {
        // Access the file input via DOM query to get the web_sys::File
        let Some(document) = web_sys::window().and_then(|w| w.document()) else {
            return;
        };
        if let Some(el) = document.get_element_by_id("avatar-file-input")
            && let Ok(input) = el.dyn_into::<web_sys::HtmlInputElement>()
            && let Some(file_list) = input.files()
            && let Some(file) = file_list.get(0)
        {
            avatar_file.set(Some(file));
        }
    };

    let on_avatar_submit = move |_| {
        let Some(file) = avatar_file() else {
            toast.show(Toast::error("Select a file first."));
            return;
        };
        if file.size() > 2_000_000.0 {
            toast.show(Toast::error("File must be under 2MB."));
            return;
        }
        let Some(member) = avatar_modal.get_target() else {
            return;
        };
        let mid = member.id.clone();
        avatar_uploading.set(true);
        spawn(async move {
            // Upload via FormData
            let Ok(form_data) = web_sys::FormData::new() else {
                toast.show(Toast::error("Could not prepare upload."));
                avatar_uploading.set(false);
                return;
            };
            let _ = form_data.append_with_blob("file", &file);

            let opts = web_sys::RequestInit::new();
            opts.set_method("POST");
            opts.set_body(&form_data.into());
            opts.set_credentials(web_sys::RequestCredentials::SameOrigin);

            let Ok(request) = web_sys::Request::new_with_str_and_init("/api/upload/avatar", &opts)
            else {
                toast.show(Toast::error("Could not build upload request."));
                avatar_uploading.set(false);
                return;
            };

            let Some(window) = web_sys::window() else {
                toast.show(Toast::error("Upload failed: no browser window."));
                avatar_uploading.set(false);
                return;
            };
            let resp_val =
                wasm_bindgen_futures::JsFuture::from(window.fetch_with_request(&request)).await;

            match resp_val {
                Ok(resp_val) => {
                    let resp: web_sys::Response = resp_val.unchecked_into();
                    if resp.ok() {
                        let text_promise = match resp.text() {
                            Ok(p) => p,
                            Err(_) => {
                                toast.show(Toast::error("Failed to read upload response."));
                                avatar_uploading.set(false);
                                return;
                            }
                        };
                        let text = wasm_bindgen_futures::JsFuture::from(text_promise).await;
                        if let Ok(text) = text {
                            let text_str = text.as_string().unwrap_or_default();
                            if let Ok(upload) = serde_json::from_str::<UploadResponse>(&text_str) {
                                let body = UpdateAvatarBody {
                                    avatar_url: Some(upload.url),
                                };
                                match ApiClient::web()
                                    .put_json_empty(&format!("/api/members/{mid}"), &body)
                                    .await
                                {
                                    Ok(_) => {
                                        toast.show(Toast::success("Avatar updated."));
                                        avatar_modal.close();
                                        members.refresh += 1;
                                    }
                                    Err(e) => toast.show(Toast::error(format!(
                                        "Failed to update profile: {e}"
                                    ))),
                                }
                            } else {
                                toast.show(Toast::error("Failed to parse upload response."));
                            }
                        } else {
                            toast.show(Toast::error("Failed to read upload response."));
                        }
                    } else {
                        toast.show(Toast::error(format!(
                            "Upload failed: HTTP {}",
                            resp.status()
                        )));
                    }
                }
                Err(_) => toast.show(Toast::error("Upload request failed.")),
            }
            avatar_uploading.set(false);
        });
    };

    // --- Render ---

    rsx! {

        div { class: "admin-toolbar",
            h1 { "Members" }
        }

        // Members table
        {
            let data = members.data.read();
            let data = data.as_ref().and_then(|d| d.as_ref());
            match data {
                None => admin_pending(&members, "members"),
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
                                let m_avatar = member.clone();
                                let m_pw = member.clone();
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
                                                    onclick: move |_| { pw_new.set(String::new()); pw_modal.show(m_pw.clone()); },
                                                    "PW"
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
                                                button {
                                                    class: "row-btn",
                                                    onclick: move |_| open_avatar(m_avatar.clone()),
                                                    "Avatar"
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
                role_modal.get_target().map(|m| m.display_name).unwrap_or_default()
            ),
            open: role_modal.is_open(),
            submitting: role_modal.is_submitting(),
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

        // Local password reset modal
        FormModal {
            title: format!(
                "Reset Password: {}",
                pw_modal.get_target().map(|m| m.display_name).unwrap_or_default()
            ),
            open: pw_modal.is_open(),
            submitting: pw_modal.is_submitting(),
            on_close: on_pw_close,
            on_submit: on_pw_submit,

            div { class: "form-field",
                label { class: "form-label", "New temporary password" }
                input {
                    class: "form-input",
                    r#type: "text",
                    value: "{pw_new}",
                    placeholder: "min 12 characters — member should change it after login",
                    oninput: move |e| pw_new.set(e.value()),
                }
                p { class: "form-hint", "Only works for local (username/password) accounts." }
            }
        }

        // Toggle active confirm
        ConfirmDialog {
            title: if toggle_modal.get_target().map(|m| m.is_active).unwrap_or(false) {
                "Deactivate Member".to_string()
            } else {
                "Activate Member".to_string()
            },
            message: format!(
                "{} \"{}\"?",
                if toggle_modal.get_target().map(|m| m.is_active).unwrap_or(false) { "Deactivate" } else { "Activate" },
                toggle_modal.get_target().map(|m| m.display_name).unwrap_or_default()
            ),
            open: toggle_modal.is_open(),
            danger: toggle_modal.get_target().map(|m| m.is_active).unwrap_or(false),
            on_confirm: on_toggle_confirm,
            on_cancel: on_toggle_cancel,
        }

        // Mod history modal
        if mod_modal.is_open() {
            div {
                class: "form-modal-overlay",
                onclick: move |_| on_mod_close(()),
                div {
                    class: "form-modal",
                    style: "max-width:700px;",
                    onclick: move |e| e.stop_propagation(),
                    div { class: "form-modal-header",
                        "Moderation: {mod_modal.get_target().map(|m| m.display_name).unwrap_or_default()}"
                    }
                    div { class: "form-modal-body",
                        if mod_loading() {
                            p { class: "admin-loading", "Loading..." }
                        } else if let Some(err) = mod_error() {
                            p { class: "empty-state", style: "color: var(--danger);",
                                "Failed to load moderation history: {err}"
                            }
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
        if stats_modal.is_open() {
            div {
                class: "form-modal-overlay",
                onclick: move |_| on_stats_close(()),
                div {
                    class: "form-modal",
                    style: "max-width:550px;",
                    onclick: move |e| e.stop_propagation(),
                    div { class: "form-modal-header",
                        "Attendance: {stats_modal.get_target().map(|m| m.display_name).unwrap_or_default()}"
                    }
                    div { class: "form-modal-body",
                        if stats_loading() {
                            p { class: "admin-loading", "Loading..." }
                        } else if let Some(err) = stats_error() {
                            p { class: "empty-state", style: "color: var(--danger);",
                                "Failed to load attendance stats: {err}"
                            }
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
                                    style: "font-family:var(--font-head);font-size:2.5rem;color:var(--accent);",
                                    "{stats.attendance_rate:.1}%"
                                }
                                div {
                                    style: "font-size:0.75rem;color:var(--text-3);text-transform:uppercase;letter-spacing:0.05em;",
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
        if accts_modal.is_open() {
            div {
                class: "form-modal-overlay",
                onclick: move |_| on_accts_close(()),
                div {
                    class: "form-modal",
                    style: "max-width:700px;",
                    onclick: move |e| e.stop_propagation(),
                    div { class: "form-modal-header",
                        "Game Accounts: {accts_modal.get_target().map(|m| m.display_name).unwrap_or_default()}"
                    }
                    div { class: "form-modal-body",
                        if accts_loading() {
                            p { class: "admin-loading", "Loading..." }
                        } else if let Some(err) = accts_error() {
                            p { class: "empty-state", style: "color: var(--danger);",
                                "Failed to load game accounts: {err}"
                            }
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
                                style: "font-family:var(--font-head);font-size:0.9rem;font-weight:700;color:var(--text);text-transform:uppercase;margin-bottom:0.75rem;",
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
                                            let g = games.data.read();
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
                                        placeholder: "e.g. Player#TAG",
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
                del_acct_modal.get_target().map(|a| a.account_name).unwrap_or_default()
            ),
            open: del_acct_modal.is_open(),
            danger: true,
            on_confirm: on_del_acct_confirm,
            on_cancel: on_del_acct_cancel,
        }

        // Avatar upload modal
        if avatar_modal.is_open() {
            div {
                class: "form-modal-overlay",
                onclick: move |_| on_avatar_close(()),
                div {
                    class: "form-modal",
                    style: "max-width:450px;",
                    onclick: move |e| e.stop_propagation(),
                    div { class: "form-modal-header",
                        "Avatar: {avatar_modal.get_target().map(|m| m.display_name).unwrap_or_default()}"
                    }
                    div { class: "form-modal-body",
                        div { class: "form-field",
                            label { class: "form-label", "Profile photo" }
                            p { style: "color:var(--text-3);font-size:0.8rem;margin:0 0 0.65rem;",
                                "PNG or JPEG, max 2MB. Click the area below to choose a file."
                            }
                            {
                                let file_label = match avatar_file() {
                                    Some(f) => format!(
                                        "{} ({:.1} KB)",
                                        f.name(),
                                        f.size() / 1024.0
                                    ),
                                    None => "No file selected yet — click here".to_string(),
                                };
                                rsx! {
                                    label {
                                        r#for: "avatar-file-input",
                                        style: "display:flex;flex-direction:column;align-items:center;justify-content:center;gap:0.5rem;min-height:7.5rem;padding:1.25rem;border:2px dashed var(--border);border-radius:10px;background:var(--surface-2);cursor:pointer;text-align:center;position:relative;",
                                        span { style: "font-weight:600;color:var(--text);", "Choose image" }
                                        span { style: "font-size:0.8rem;color:var(--text-3);", "{file_label}" }
                                        input {
                                            id: "avatar-file-input",
                                            r#type: "file",
                                            accept: "image/png,image/jpeg,image/webp,image/gif",
                                            style: "position:absolute;inset:0;width:100%;height:100%;opacity:0;cursor:pointer;",
                                            onchange: on_avatar_file_change,
                                        }
                                    }
                                }
                            }
                        }
                    }
                    div { class: "form-modal-footer",
                        button {
                            class: "btn-cancel",
                            onclick: move |_| on_avatar_close(()),
                            "Cancel"
                        }
                        button {
                            class: "btn-save",
                            disabled: avatar_uploading() || avatar_file().is_none(),
                            onclick: on_avatar_submit,
                            if avatar_uploading() { "Uploading..." } else { "Upload" }
                        }
                    }
                }
            }
        }
    }
}
