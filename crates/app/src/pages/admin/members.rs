use dioxus::prelude::*;
use serde::{Deserialize, Serialize};
use wasm_bindgen::JsCast;

use crate::components::{
    ConfirmDialog, DataTable, FormModal, RolePill, StatusPill, SummaryCard, Toast, admin_pending,
    list_cap_notice, use_toast,
};
use crate::hooks::{ModalController, use_api_list, use_api_list_prefer};
use crate::state::use_auth;
use scuffed_api_client::{ApiClient, ClientError};
use scuffed_types::AttendanceStats;
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
struct UploadResponse {
    url: String,
}

#[derive(Serialize)]
struct UpdateAvatarBody {
    avatar_url: Option<String>,
}

const ROLES: [&str; 4] = ["recruit", "member", "officer", "admin"];

/// Active-only list — any org member. Do not put `include_inactive` here.
const ADMIN_MEMBERS_ACTIVE_ONLY: &str = "/api/members";
/// Officer+ list. Recruit/member requests get 403 and fall back to active-only.
const ADMIN_MEMBERS_INCLUDE_INACTIVE: &str = "/api/members?include_inactive=true";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MembersListMode {
    /// Flag not sent (non-officer).
    ActiveOnly,
    /// Officer requested include_inactive and the server accepted (200).
    IncludeInactive,
    /// Officer requested include_inactive, got 403, fell back to active-only.
    FallbackActiveOnly,
}

fn preferred_members_path(is_officer: bool) -> Option<&'static str> {
    is_officer.then_some(ADMIN_MEMBERS_INCLUDE_INACTIVE)
}

fn members_list_mode(is_officer: bool, used_forbidden_fallback: bool) -> MembersListMode {
    if !is_officer {
        MembersListMode::ActiveOnly
    } else if used_forbidden_fallback {
        MembersListMode::FallbackActiveOnly
    } else {
        MembersListMode::IncludeInactive
    }
}

fn list_intro_copy(mode: MembersListMode) -> &'static str {
    match mode {
        MembersListMode::IncludeInactive => {
            "Inactive members stay on this list (dimmed). Activate restores one."
        }
        MembersListMode::FallbackActiveOnly => {
            "Could not load inactive members (officer access required). Showing active members only. Members you deactivate this session stay under Recently deactivated."
        }
        MembersListMode::ActiveOnly => "Active members only.",
    }
}

fn session_inactive_note() -> &'static str {
    "These members were deactivated during this session and are not in the list above. Activate still works here."
}

fn member_row_class(is_active: bool) -> &'static str {
    if is_active { "" } else { "is-inactive" }
}

fn session_inactive_not_in_list(recent: &[Member], listed_ids: &[String]) -> Vec<Member> {
    recent
        .iter()
        .filter(|m| !listed_ids.iter().any(|id| id == &m.id))
        .cloned()
        .collect()
}

/// Officer avatar upload for one member on the admin list.
///
/// `member_id` is the same record key `PUT /api/members/{mid}` already uses.
/// It is percent-encoded and otherwise left as-is (a `member:` prefix stays).
fn admin_avatar_upload_url(member_id: &str) -> String {
    format!(
        "/api/upload/avatar?member_id={}",
        crate::util::encode_query(member_id)
    )
}

/// Toast copy when the avatar upload is rejected. 403 and 404 come from the
/// same profile-edit check as `PUT /api/members/{mid}`.
fn avatar_upload_failure_message(status: u16) -> String {
    match status {
        403 => "You don't have permission to change this member's avatar.".to_string(),
        404 => "That member no longer exists.".to_string(),
        other => format!("Upload failed: HTTP {other}"),
    }
}

/// User-facing copy for a failed attendance stats fetch. Never includes the
/// raw serde error (field names, column numbers).
fn attendance_stats_load_message(err: &ClientError) -> String {
    if err.http_status() == Some(429) {
        let body = match err {
            ClientError::Http { body, .. } => body.as_str(),
            _ => "",
        };
        return match scuffed_types::json_retry_after(body) {
            Some(seconds) => attendance_retry_after_message(seconds),
            None => "Too many requests. Try again later.".to_string(),
        };
    }
    match err {
        ClientError::Deserialize(_) => "Attendance stats could not be read. Try again.",
        ClientError::Http { status: 403, .. } => {
            "You don't have permission to view this member's attendance."
        }
        ClientError::Http { status: 404, .. } => "That member no longer exists.",
        ClientError::Network(_) | ClientError::Http { .. } => {
            "Could not load attendance stats. Try again."
        }
    }
    .to_string()
}

fn attendance_retry_after_message(seconds: u64) -> String {
    if seconds == 1 {
        "Too many requests. Try again in 1 second.".to_string()
    } else {
        format!("Too many requests. Try again in {seconds} seconds.")
    }
}

#[component]
fn AttendanceStatsError(message: String) -> Element {
    rsx! {
        p {
            class: "empty-state",
            role: "alert",
            style: "color: var(--danger);",
            "{message}"
        }
    }
}

#[component]
pub fn AdminMembers() -> Element {
    let auth = use_auth();
    let is_admin = auth().is_admin();
    let (mut members, include_inactive_fell_back) = use_api_list_prefer::<Member>(
        move || preferred_members_path(auth().is_officer_or_above()).map(str::to_string),
        ADMIN_MEMBERS_ACTIVE_ONLY,
    );
    let mut games = use_api_list::<Game>("/api/games");
    let mut toast = use_toast();
    // Session fallback: a 403 on the inactive list drops deactivated rows from
    // the active-only response. Keep those here so Activate still works. When
    // the inactive list loads, they reappear in `members` and this list hides.
    let mut recently_inactive: Signal<Vec<Member>> = use_signal(Vec::new);
    let list_mode = members_list_mode(auth().is_officer_or_above(), include_inactive_fell_back());

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
    // In-flight stats fetch. Reopening or closing cancels it, so a slow
    // response for one member never lands in another member's modal.
    let mut stats_task: Signal<Option<dioxus::dioxus_core::Task>> = use_signal(|| None);

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
                        if new_active {
                            recently_inactive.write().retain(|m| m.id != id);
                            toast.show(Toast::success("Member activated."));
                        } else {
                            let mut row = member.clone();
                            row.is_active = false;
                            recently_inactive.write().retain(|m| m.id != id);
                            recently_inactive.write().push(row);
                            toast.show(Toast::success("Member deactivated."));
                        }
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
        if let Some(task) = stats_task.take() {
            task.cancel();
        }
        stats_data.set(None);
        stats_error.set(None);
        stats_loading.set(true);
        let mid = member.id.clone();
        stats_modal.show(member);
        let task = spawn(async move {
            match ApiClient::web()
                .fetch::<AttendanceStats>(&format!("/api/members/{mid}/attendance/stats"))
                .await
            {
                Ok(data) => stats_data.set(Some(data)),
                Err(e) => stats_error.set(Some(attendance_stats_load_message(&e))),
            }
            stats_loading.set(false);
        });
        stats_task.set(Some(task));
    };

    let mut on_stats_close = move |_| {
        if let Some(task) = stats_task.take() {
            task.cancel();
        }
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

            let upload_url = admin_avatar_upload_url(&mid);
            let Ok(request) = web_sys::Request::new_with_str_and_init(&upload_url, &opts) else {
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
                        toast.show(Toast::error(avatar_upload_failure_message(resp.status())));
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
        p { class: "empty-state", style: "text-align:left;padding:0 0 1rem;margin:0;",
            "{list_intro_copy(list_mode)}"
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
                                let joined = crate::util::format_datetime(&member.joined_at);
                                let row_class = member_row_class(member.is_active);
                                rsx! {
                                    tr { key: "{member.id}", class: "{row_class}",
                                        td { "{member.display_name}" }
                                        td { RolePill { role: member.org_role.clone() } }
                                        td { StatusPill { status: status_str.to_string() } }
                                        td { "{joined}" }
                                        td {
                                            div { class: "row-actions",
                                                if is_admin {
                                                    button {
                                                        class: "row-btn",
                                                        onclick: move |_| open_role(m_role.clone()),
                                                        "Role"
                                                    }
                                                }
                                                if member.is_active {
                                                    button {
                                                        class: "row-btn danger",
                                                        onclick: move |_| open_toggle(m_toggle.clone()),
                                                        "Deactivate"
                                                    }
                                                } else {
                                                    button {
                                                        class: "row-btn",
                                                        onclick: move |_| open_toggle(m_toggle.clone()),
                                                        "Activate"
                                                    }
                                                }
                                                if is_admin {
                                                    button {
                                                        class: "row-btn",
                                                        onclick: move |_| { pw_new.set(String::new()); pw_modal.show(m_pw.clone()); },
                                                        "PW"
                                                    }
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

        {
            let listed_ids: Vec<String> = members
                .data
                .read()
                .as_ref()
                .and_then(|d| d.as_ref())
                .map(|list| list.iter().map(|m| m.id.clone()).collect())
                .unwrap_or_default();
            let recent = recently_inactive();
            let inactive = session_inactive_not_in_list(&recent, &listed_ids);
            if inactive.is_empty() {
                rsx! {}
            } else {
                rsx! {
                    h2 {
                        style: "font-family:var(--font-head);font-size:0.95rem;color:var(--text);margin:1.5rem 0 0.75rem;",
                        "Recently deactivated (this session)"
                    }
                    p { class: "empty-state", style: "text-align:left;padding:0 0 0.75rem;margin:0;",
                        "{session_inactive_note()}"
                    }
                    DataTable { headers: vec!["Name", "Role", "Status", "Actions"],
                        for member in inactive.iter() {
                            {
                                let m_toggle = member.clone();
                                rsx! {
                                    tr { key: "inactive-{member.id}", class: "is-inactive",
                                        td { "{member.display_name}" }
                                        td { RolePill { role: member.org_role.clone() } }
                                        td { StatusPill { status: "inactive".to_string() } }
                                        td {
                                            button {
                                                class: "row-btn",
                                                onclick: move |_| open_toggle(m_toggle.clone()),
                                                "Activate"
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

        {list_cap_notice(&members, "members")}
        {list_cap_notice(&games, "games")}

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
                                            let created = crate::util::format_datetime(&action.created_at);
                                            rsx! {
                                                tr { key: "{action.id}",
                                                    td { "{action.action_type}" }
                                                    td { "{action.reason}" }
                                                    td { StatusPill { status: active_str.to_string() } }
                                                    td { "{created}" }
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
                            AttendanceStatsError { message: err }
                        } else if let Some(stats) = stats_data() {
                            {
                                let total = stats.total.to_string();
                                let attended = stats.attended.to_string();
                                let absent = stats.no_show.to_string();
                                let excused = stats.excused.to_string();
                                let rate = format!("{:.1}%", stats.attendance_rate());
                                let no_events = stats.total == 0;
                                rsx! {
                                    div { class: "summary-cards",
                                        SummaryCard { value: total, label: "Total Events" }
                                        SummaryCard { value: attended, label: "Attended" }
                                        SummaryCard { value: absent, label: "Absent" }
                                        SummaryCard { value: excused, label: "Excused" }
                                    }
                                    if no_events {
                                        p { class: "empty-state", "No events recorded for this member." }
                                    }
                                    div {
                                        style: "text-align:center;margin-top:1rem;",
                                        span {
                                            style: "font-family:var(--font-head);font-size:2.5rem;color:var(--accent);",
                                            "{rate}"
                                        }
                                        div {
                                            style: "font-size:0.75rem;color:var(--text-3);text-transform:uppercase;letter-spacing:0.05em;",
                                            "Attendance Rate"
                                        }
                                    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn member(id: &str, active: bool) -> Member {
        Member {
            id: id.into(),
            display_name: id.into(),
            org_role: "member".into(),
            is_active: active,
            joined_at: "2026-09-02T00:00:00Z".into(),
        }
    }

    #[test]
    fn officer_requests_include_inactive_non_officer_omits_flag() {
        assert_eq!(
            preferred_members_path(true),
            Some("/api/members?include_inactive=true")
        );
        assert_eq!(preferred_members_path(false), None);
        assert!(
            !ADMIN_MEMBERS_ACTIVE_ONLY.contains("include_inactive"),
            "fallback/active-only path must not send the flag"
        );
    }

    #[test]
    fn list_mode_maps_officer_and_403_fallback() {
        assert_eq!(members_list_mode(false, false), MembersListMode::ActiveOnly);
        assert_eq!(
            members_list_mode(false, true),
            MembersListMode::ActiveOnly,
            "non-officer never requested the flag"
        );
        assert_eq!(
            members_list_mode(true, false),
            MembersListMode::IncludeInactive
        );
        assert_eq!(
            members_list_mode(true, true),
            MembersListMode::FallbackActiveOnly
        );
    }

    #[test]
    fn session_workaround_hides_when_api_returns_the_row() {
        let recent = vec![member("gone", false), member("listed", false)];
        let listed = vec!["listed".to_string()];
        let visible = session_inactive_not_in_list(&recent, &listed);
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].id, "gone");
    }

    #[test]
    fn session_workaround_empty_when_include_inactive_lists_them() {
        let recent = vec![member("a", false)];
        let listed = vec!["a".to_string()];
        assert!(session_inactive_not_in_list(&recent, &listed).is_empty());
    }

    #[test]
    fn inactive_rows_use_distinct_class() {
        assert_eq!(member_row_class(true), "");
        assert_eq!(member_row_class(false), "is-inactive");
    }

    #[test]
    fn intro_copy_is_user_facing_and_not_a_pr_note() {
        for mode in [
            MembersListMode::IncludeInactive,
            MembersListMode::FallbackActiveOnly,
            MembersListMode::ActiveOnly,
        ] {
            let copy = list_intro_copy(mode);
            assert!(!copy.contains("#52"), "{copy}");
            assert!(!copy.contains("include_inactive"), "{copy}");
        }
        assert!(list_intro_copy(MembersListMode::IncludeInactive).contains("Inactive members"));
        assert!(
            list_intro_copy(MembersListMode::FallbackActiveOnly)
                .contains("officer access required")
        );
        assert_eq!(
            list_intro_copy(MembersListMode::ActiveOnly),
            "Active members only."
        );
        assert!(!session_inactive_note().contains("#52"));
        assert!(!session_inactive_note().contains("include_inactive"));
        assert!(session_inactive_note().contains("Activate"));
    }

    #[test]
    fn admin_avatar_upload_url_keeps_a_plain_key() {
        assert_eq!(
            admin_avatar_upload_url("abc123"),
            "/api/upload/avatar?member_id=abc123"
        );
    }

    #[test]
    fn admin_avatar_upload_url_keeps_a_member_prefix() {
        assert_eq!(
            admin_avatar_upload_url("member:abc123"),
            "/api/upload/avatar?member_id=member%3Aabc123"
        );
    }

    #[test]
    fn admin_avatar_upload_url_encodes_reserved_characters() {
        assert_eq!(
            admin_avatar_upload_url("a b/c+d"),
            "/api/upload/avatar?member_id=a%20b%2Fc%2Bd"
        );
    }

    #[test]
    fn stats_modal_cancels_the_previous_fetch() {
        // The fetch is a spawned task, so this guards the wiring in source.
        // Split so this test does not contain the code it is looking for.
        let src = include_str!("members.rs");
        let cancel = format!("stats_task.{}", "take()");
        for handler in ["open_stats", "on_stats_close"] {
            let start = format!("let mut {handler} = {}", "move |");
            let body = src
                .split(start.as_str())
                .nth(1)
                .and_then(|rest| rest.split("\n    };").next())
                .unwrap_or_else(|| panic!("{handler} not found"));
            assert!(
                body.contains(&cancel),
                "{handler} must cancel the in-flight stats fetch"
            );
        }
        assert!(src.contains(&format!("stats_task.{}", "set(Some(task))")));
    }

    #[test]
    fn attendance_stats_load_message_hides_serde_text() {
        let raw =
            ClientError::Deserialize("missing field `total_events` at line 1 column 83".into());
        let msg = attendance_stats_load_message(&raw);
        assert_eq!(msg, "Attendance stats could not be read. Try again.");
        assert!(!msg.contains("total_events"));
        assert!(!msg.contains("missing field"));
        assert!(!msg.contains("Deserialization"));

        assert_eq!(
            attendance_stats_load_message(&ClientError::Http {
                status: 403,
                body: r#"{"error":"Can only view your own attendance stats"}"#.into(),
            }),
            "You don't have permission to view this member's attendance."
        );
        assert_eq!(
            attendance_stats_load_message(&ClientError::Network("connection reset".into())),
            "Could not load attendance stats. Try again."
        );
    }

    fn render(view: fn() -> Element) -> String {
        let mut dom = VirtualDom::new(view);
        dom.rebuild_in_place();
        dioxus_ssr::render(&dom)
    }

    #[test]
    fn attendance_stats_429_names_the_wait_and_alerts() {
        let limited = ClientError::Http {
            status: 429,
            body: r#"{"error":"rate_limited","retry_after":12}"#.into(),
        };
        assert_eq!(limited.http_status(), Some(429));
        let wait = attendance_stats_load_message(&limited);
        assert_eq!(wait, "Too many requests. Try again in 12 seconds.");
        assert!(!wait.contains('\u{2014}'), "{wait}");

        assert_eq!(
            attendance_stats_load_message(&ClientError::Http {
                status: 429,
                body: "Too Many Requests".into(),
            }),
            "Too many requests. Try again later."
        );
        assert_eq!(
            attendance_stats_load_message(&ClientError::Http {
                status: 429,
                body: r#"{"error":"rate_limited","retry_after":3601}"#.into(),
            }),
            "Too many requests. Try again later."
        );
        assert_eq!(
            attendance_stats_load_message(&ClientError::Http {
                status: 500,
                body: r#"{"error":"rate_limited","retry_after":9}"#.into(),
            }),
            "Could not load attendance stats. Try again."
        );

        fn view() -> Element {
            rsx! {
                AttendanceStatsError {
                    message: "Too many requests. Try again in 12 seconds.".to_string(),
                }
            }
        }
        let html = render(view);
        assert!(html.contains("role=\"alert\""), "{html}");
        assert!(html.contains(&wait), "{html}");
        assert!(!html.contains('\u{2014}'), "{html}");
    }

    #[test]
    fn attendance_stats_429_one_second_is_singular_and_two_are_plural() {
        assert_eq!(
            attendance_stats_load_message(&ClientError::Http {
                status: 429,
                body: r#"{"error":"rate_limited","retry_after":1}"#.into(),
            }),
            "Too many requests. Try again in 1 second."
        );
        assert_eq!(
            attendance_stats_load_message(&ClientError::Http {
                status: 429,
                body: r#"{"error":"rate_limited","retry_after":2}"#.into(),
            }),
            "Too many requests. Try again in 2 seconds."
        );
        assert!(!attendance_retry_after_message(1).contains('\u{2014}'));
        assert!(!attendance_retry_after_message(2).contains('\u{2014}'));
    }

    #[test]
    fn avatar_upload_failure_message_names_403_and_404() {
        assert_eq!(
            avatar_upload_failure_message(403),
            "You don't have permission to change this member's avatar."
        );
        assert_eq!(
            avatar_upload_failure_message(404),
            "That member no longer exists."
        );
        assert_eq!(
            avatar_upload_failure_message(500),
            "Upload failed: HTTP 500"
        );
    }
}
