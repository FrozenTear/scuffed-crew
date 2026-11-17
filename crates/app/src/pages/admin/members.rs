use dioxus::prelude::*;

use crate::components::admin_shared::{after_delete_focus_ids, focus_after_delete_soon};
use serde::{Deserialize, Serialize};

use crate::components::ui::inline_confirm::{
    focus_element_by_id, focus_element_by_id_soon, key_cancels,
};
use crate::components::ui::{
    ChipButton, ChipRow, EmptyCard, PageHeader, PageShell, PageWidth, Pager, page_slice,
    show_list_top,
};
use crate::components::{
    ADMIN_PAGE_SIZE, AdminDate, ConfirmDialog, DataTable, FormModal, FormStatus, RolePill,
    RowActions, SaveState, StatusPill, SummaryCard, Toast, admin_list_pending, list_cap_notice,
    row_menu_toggle_id, status_label, use_toast,
};
use crate::hooks::{ModalController, use_api_list, use_api_list_prefer};
use crate::routes::Route;
use crate::state::use_auth;
use crate::upload::UploadFailure;
use scuffed_api_client::{ApiClient, ClientError};
use scuffed_types::api::{ChangeRoleRequest, CreateGameAccountRequest, ToggleActiveRequest};
use scuffed_types::{AttendanceStats, OrgRole, UserInfo};

// --- Types ---
// These local types have API-enriched fields (joined names, computed stats)
// that differ from the base org types in scuffed_types.

#[derive(Debug, Clone, PartialEq, Deserialize)]
struct Member {
    id: String,
    /// Login user of this member. Matches `UserInfo::id`, so an officer's own row is known.
    #[serde(default)]
    user_id: String,
    display_name: String,
    org_role: String,
    is_active: bool,
    joined_at: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
struct GameAccount {
    id: String,
    game_id: String,
    account_name: String,
    account_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
struct Game {
    id: String,
    name: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
struct ModerationAction {
    id: String,
    action_type: String,
    reason: String,
    is_active: bool,
    created_at: String,
}

#[derive(Serialize)]
struct UpdateAvatarBody {
    avatar_url: Option<String>,
}

const ROLES: [&str; 4] = ["recruit", "member", "officer", "admin"];

/// Role chips: the stored role (empty for all) and the chip's word.
const ROLE_FILTERS: [(&str, &str); 5] = [
    ("", "All roles"),
    ("member", "Member"),
    ("officer", "Officer"),
    ("recruit", "Recruit"),
    ("admin", "Admin"),
];

/// The list region: the pager's focus target after a page change.
const LIST_ID: &str = "admin-members-list";
const SEARCH_ID: &str = "members-search";
const SELECT_PAGE_ID: &str = "members-select-page";
const BULK_COUNT_ID: &str = "members-bulk-count";
const BULK_STATUS_ID: &str = "members-bulk-status";
const BULK_ROLE_OPEN_ID: &str = "members-bulk-role-open";
const BULK_DEACTIVATE_OPEN_ID: &str = "members-bulk-deactivate-open";
const BULK_ROLE_FIELD_ID: &str = "members-bulk-role";
const AVATAR_INPUT_ID: &str = "member-avatar-file";

/// Active-only list: any org member. Do not put `include_inactive` here.
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

/// Id of a role chip, from its role.
fn role_filter_id(role: &str) -> String {
    let name = if role.is_empty() { "all" } else { role };
    format!("members-filter-{name}")
}

/// The members a role chip and the search keep. The empty role keeps all
/// roles; the search matches the display name, ignoring case.
fn filter_members(list: &[Member], role: &str, query: &str) -> Vec<Member> {
    let query = query.trim().to_lowercase();
    list.iter()
        .filter(|m| role.is_empty() || m.org_role == role)
        .filter(|m| query.is_empty() || m.display_name.to_lowercase().contains(&query))
        .cloned()
        .collect()
}

/// Id stem of a member's row actions.
fn member_actions_id(member_id: &str) -> String {
    format!("member-actions-{member_id}")
}

/// The "Actions" button of a member's row: where focus goes back when a
/// dialog opened from that row closes.
fn member_actions_toggle_id(member_id: &str) -> String {
    row_menu_toggle_id(&member_actions_id(member_id))
}

/// Id of the hidden note that says why a row action is locked.
fn member_lock_note_id(member_id: &str) -> String {
    format!("member-lock-{member_id}")
}

/// "Deactivated 2 members." or, when some failed, the count that went
/// through plus each failure with the server's reason. `skipped` are the
/// sentences for rows the action left out on purpose, by name.
fn bulk_result(
    verb: &str,
    done: usize,
    attempted: usize,
    failures: &[String],
    skipped: &[String],
) -> SaveState {
    let noun = if attempted == 1 { "member" } else { "members" };
    let mut text = if failures.is_empty() {
        format!("{verb} {done} {noun}.")
    } else {
        format!(
            "{verb} {done} of {attempted} {noun}. {}",
            failures.join(" ")
        )
    };
    for line in skipped {
        text.push(' ');
        text.push_str(line);
    }
    if failures.is_empty() {
        SaveState::Saved(text)
    } else {
        SaveState::Failed(text)
    }
}

/// The rows a bulk action sends, and a sentence for each row it leaves out
/// on purpose (by name), so nothing is skipped silently.
#[derive(Debug, Default, PartialEq)]
struct BulkTargets {
    targets: Vec<Member>,
    skipped: Vec<String>,
}

/// True when `member` is the signed-in account's own row.
fn is_own_row(user: Option<&UserInfo>, member: &Member) -> bool {
    user.is_some_and(|user| user.id == member.user_id)
}

/// Targets of the bulk role change. Rows that already have `role` are left
/// out quietly. The caller's own row is always left out and named: the server
/// lets an admin demote themselves while another admin exists, and the rest
/// of the batch would then fail with 403 while the page still shows admin
/// controls.
fn bulk_role_targets(selected: Vec<Member>, role: &str, user: Option<&UserInfo>) -> BulkTargets {
    let mut out = BulkTargets::default();
    for member in selected {
        if member.org_role == role {
            continue;
        }
        if is_own_row(user, &member) {
            out.skipped.push(format!(
                "Left out {} (your own account): change your own role from its row.",
                member.display_name
            ));
            continue;
        }
        out.targets.push(member);
    }
    out
}

/// Targets of the bulk deactivate. Inactive rows are left out quietly. The
/// caller's own row (the server never lets anyone change their own active
/// status) and, for an officer, admin and officer rows are left out and named.
fn bulk_deactivate_targets(selected: Vec<Member>, user: Option<&UserInfo>) -> BulkTargets {
    let mut out = BulkTargets::default();
    for member in selected {
        if !member.is_active {
            continue;
        }
        if is_own_row(user, &member) {
            out.skipped.push(format!(
                "Left out {} (your own account): you cannot deactivate yourself.",
                member.display_name
            ));
            continue;
        }
        if officer_locked_out(user, &member) {
            out.skipped.push(format!(
                "Left out {}: officers cannot deactivate admins or other officers.",
                member.display_name
            ));
            continue;
        }
        out.targets.push(member);
    }
    out
}

/// Question of the bulk deactivate dialog: the count that will be sent, then
/// each row left out by name, so the admin confirms what will happen.
fn bulk_deactivate_message(plan: &BulkTargets) -> String {
    let n = plan.targets.len();
    let mut text = match n {
        0 => "None of the selected members can be deactivated.".to_string(),
        1 => {
            "Deactivate 1 member? They lose access until someone activates them again.".to_string()
        }
        n => {
            format!("Deactivate {n} members? They lose access until someone activates them again.")
        }
    };
    for line in &plan.skipped {
        text.push(' ');
        text.push_str(line);
    }
    text
}

/// Note in the bulk role dialog when the caller's own row is selected.
fn bulk_role_own_note(selected: &[Member], user: Option<&UserInfo>) -> Option<String> {
    selected.iter().find(|m| is_own_row(user, m)).map(|m| {
        format!(
            "Your own account, {}, is left out. Change your own role from its row.",
            m.display_name
        )
    })
}

/// One failure sentence of a bulk action: always starts with the member's
/// name, then the plain reason (the officer rank refusal) or the server text.
fn bulk_failure(member: &Member, err: &ClientError) -> String {
    let reason = profile_edit_failure(&member.display_name, err);
    let reason = reason.trim_end_matches('.');
    if reason.starts_with(&member.display_name) {
        format!("{reason}.")
    } else {
        format!("{}: {reason}.", member.display_name)
    }
}

/// "2 selected", or nothing when no row is selected.
fn selection_text(count: usize) -> String {
    if count == 0 {
        String::new()
    } else {
        format!("{count} selected")
    }
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

/// What the server answers when an officer edits an admin or another officer.
const OFFICER_RANK_ERROR: &str = "Officers cannot edit the profile of admins or other officers";

/// Shown beside a locked control and in a toast for that refusal.
const OFFICER_RANK_COPY: &str =
    "Officers cannot edit the profile of admins or other officers. Ask an admin.";

/// Same rule as the server's `can_edit_other_profile`: an officer may edit
/// themselves, a member, or a recruit, and nobody above. Admins may edit anyone.
fn officer_locked_out(user: Option<&UserInfo>, target: &Member) -> bool {
    let Some(user) = user else {
        return false;
    };
    user.role == Some(OrgRole::Officer)
        && user.id != target.user_id
        && !matches!(target.org_role.as_str(), "member" | "recruit")
}

/// Toast for a failed profile, avatar, or game account write. The officer rank
/// refusal gets plain copy. Anything else keeps the server text.
fn profile_edit_failure(prefix: &str, err: &ClientError) -> String {
    if err.http_status() == Some(403) && err.to_string().contains(OFFICER_RANK_ERROR) {
        return OFFICER_RANK_COPY.to_string();
    }
    format!("{prefix}: {err}")
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
        p { class: "fetch-error", role: "alert", "{message}" }
    }
}

const MEMBERS_CSS: &str = r#"
    /* Selection bar: the select-all box, the live count, then the bulk
       actions once a row is selected. */
    .members-bulk-bar {
        display: flex;
        flex-wrap: wrap;
        align-items: center;
        gap: var(--space-2) var(--space-4);
        padding: var(--space-3) var(--space-card-pad);
        background: var(--card-bg);
        color: var(--card-fg);
        border: 1px solid var(--card-border);
        border-radius: var(--card-radius);
    }
    .members-bulk-bar.has-selection {
        background: var(--accent-soft);
        border-color: var(--accent-soft);
    }
    .members-bulk-bar > .admin-check-row { width: auto; }
    .members-bulk-count {
        margin: 0;
        color: var(--text);
        font-family: var(--font-head);
        font-size: var(--text-md);
        font-weight: var(--font-head-weight);
    }
    /* No text: out of the layout, still in the tree (a live line
       that is display: none drops out of what screen readers track). */
    .members-bulk-count:empty {
        position: absolute;
        width: 1px;
        height: 1px;
        padding: 0;
        margin: -1px;
        overflow: hidden;
        clip: rect(0, 0, 0, 0);
        white-space: nowrap;
        border: 0;
    }
    .members-bulk-actions {
        display: flex;
        flex-wrap: wrap;
        align-items: center;
        gap: var(--space-2);
    }
    .members-bulk-bar > .admin-form-status { flex: 1 1 100%; }
    /* The row checkbox: a 44px box around the 18px control. */
    .member-select.admin-check-row {
        width: var(--hit-target-min);
        justify-content: center;
    }
    .admin-members-page .data-table td.cell-select { width: var(--hit-target-min); }
    /* In a phone card the box sits on the card's left text edge, in line
       with the pill and meta lines under it; the 44px target runs right. */
    .admin-members-page .is-cards .data-table td.cell-select .member-select.admin-check-row {
        justify-content: flex-start;
    }
    .members-inactive {
        display: flex;
        flex-direction: column;
        gap: var(--space-stack);
        min-width: 0;
    }
    .members-inactive > p {
        margin: 0;
        color: var(--text-3);
        font-size: var(--text-sm);
        line-height: var(--leading-body);
    }
    /* Read-only member dialogs (moderation history, attendance). */
    .member-dialog-loading {
        margin: 0;
        color: var(--text-3);
        font-size: var(--text-md);
    }
    .member-dialog-empty {
        margin: 0;
        color: var(--text-3);
        font-size: var(--text-md);
    }
    .member-attendance-rate {
        display: flex;
        flex-direction: column;
        align-items: center;
        gap: var(--space-1);
        margin: 0;
    }
    .member-attendance-rate .value {
        color: var(--accent-text);
        font-family: var(--font-head);
        font-size: var(--text-xl);
        font-weight: var(--font-head-weight);
        font-variant-numeric: tabular-nums;
    }
    .member-attendance-rate .label {
        color: var(--text-3);
        font-size: var(--text-xs);
        font-weight: var(--font-label-weight);
        letter-spacing: var(--tracking-caps);
        text-transform: uppercase;
    }
    .member-accounts-add {
        padding-top: var(--space-5);
        border-top: 1px solid var(--card-border);
    }
    .member-accounts-add > h3 {
        margin: 0 0 var(--space-4);
        color: var(--text);
        font-family: var(--font-head);
        font-size: var(--text-md);
        font-weight: var(--font-head-weight);
    }
"#;

/// Fixed ids of the read-only member dialog. One is open at a time.
const MEMBER_DIALOG_ID: &str = "member-dialog";
const MEMBER_DIALOG_TITLE_ID: &str = "member-dialog-title";
const MEMBER_DIALOG_CLOSE_ID: &str = "member-dialog-close";

/// A read-only dialog (moderation history, attendance): the title takes
/// focus on open, Close is the one tab stop (Tab and Shift+Tab stay on it), Escape and the overlay close it, and
/// focus goes back to `restore_focus_id`.
#[component]
fn MemberDialog(
    title: String,
    restore_focus_id: String,
    on_close: EventHandler<()>,
    #[props(default = false)] wide: bool,
    children: Element,
) -> Element {
    use_effect(|| focus_element_by_id(MEMBER_DIALOG_TITLE_ID));
    let close = use_callback(move |_: ()| {
        on_close.call(());
        focus_element_by_id_soon(restore_focus_id.clone());
    });
    rsx! {
        div {
            class: "form-modal-overlay",
            onclick: move |_| close.call(()),
            onkeydown: move |evt| {
                if key_cancels(&evt.key()) {
                    evt.stop_propagation();
                    close.call(());
                } else if evt.key() == Key::Tab {
                    // Close is the only tab stop, so Tab and Shift+Tab stay on it.
                    evt.prevent_default();
                    focus_element_by_id(MEMBER_DIALOG_CLOSE_ID);
                }
            },
            div {
                class: if wide { "form-modal wide" } else { "form-modal" },
                id: MEMBER_DIALOG_ID,
                role: "dialog",
                aria_modal: "true",
                aria_labelledby: MEMBER_DIALOG_TITLE_ID,
                onclick: move |e| e.stop_propagation(),
                h2 {
                    id: MEMBER_DIALOG_TITLE_ID,
                    class: "form-modal-header",
                    tabindex: "-1",
                    "{title}"
                }
                div { class: "form-modal-body", {children} }
                div { class: "form-modal-footer",
                    button {
                        id: MEMBER_DIALOG_CLOSE_ID,
                        class: "btn is-ghost",
                        r#type: "button",
                        onclick: move |_| close.call(()),
                        "Close"
                    }
                }
            }
        }
    }
}

/// A row action, chosen from a member's "Actions" group.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MemberAction {
    Role,
    Toggle,
    Password,
    Moderation,
    Attendance,
    Accounts,
    Avatar,
}

/// The table of one page of members. On phone every row is a card: the
/// select box, the name and "Actions" on the top line, then role, status
/// and join date on one line. Every action sits in the "Actions" group, on
/// desktop too (eight actions do not fit a row).
#[component]
fn MembersTable(
    rows: Vec<Member>,
    selected: Vec<String>,
    is_admin: bool,
    me: Option<UserInfo>,
    on_select: EventHandler<(String, bool)>,
    on_action: EventHandler<(MemberAction, Member)>,
) -> Element {
    rsx! {
        DataTable {
            headers: vec!["Select", "Name", "Role", "Status", "Joined", "Actions"],
            num_cols: vec![4],
            cards: true,
            label: "Members",
            for member in rows.iter() {
                {
                    let id = member.id.clone();
                    let name = member.display_name.clone();
                    let checked = selected.contains(&member.id);
                    let select_id = member.id.clone();
                    let locked = officer_locked_out(me.as_ref(), member);
                    let lock_note = member_lock_note_id(&member.id);
                    let locked_by = locked.then(|| lock_note.clone());
                    let status = if member.is_active { "active" } else { "inactive" };
                    let status_word = status_label(status);
                    let toggle_word = if member.is_active { "Deactivate" } else { "Activate" };
                    let toggle_class = if member.is_active { "btn is-danger" } else { "btn is-ghost" };
                    let m_role = member.clone();
                    let m_toggle = member.clone();
                    let m_pw = member.clone();
                    let m_mod = member.clone();
                    let m_stats = member.clone();
                    let m_accts = member.clone();
                    let m_avatar = member.clone();
                    let more = rsx! {
                        if locked {
                            span { id: "{lock_note}", class: "visually-hidden", "{OFFICER_RANK_COPY}" }
                        }
                        if is_admin {
                            button {
                                r#type: "button",
                                class: "btn is-ghost",
                                aria_label: "Change role for {name}",
                                onclick: move |_| on_action.call((MemberAction::Role, m_role.clone())),
                                "Change role"
                            }
                        }
                        button {
                            r#type: "button",
                            class: toggle_class,
                            aria_label: "{toggle_word} {name}",
                            aria_disabled: if locked { "true" },
                            aria_describedby: locked_by.clone(),
                            title: if locked { OFFICER_RANK_COPY },
                            onclick: move |_| {
                                if !locked {
                                    on_action.call((MemberAction::Toggle, m_toggle.clone()));
                                }
                            },
                            "{toggle_word}"
                        }
                        if is_admin {
                            button {
                                r#type: "button",
                                class: "btn is-ghost",
                                aria_label: "Reset password for {name}",
                                onclick: move |_| on_action.call((MemberAction::Password, m_pw.clone())),
                                "Reset password"
                            }
                        }
                        button {
                            r#type: "button",
                            class: "btn is-ghost",
                            aria_label: "Moderation history for {name}",
                            onclick: move |_| on_action.call((MemberAction::Moderation, m_mod.clone())),
                            "Moderation"
                        }
                        button {
                            r#type: "button",
                            class: "btn is-ghost",
                            aria_label: "Attendance for {name}",
                            onclick: move |_| on_action.call((MemberAction::Attendance, m_stats.clone())),
                            "Attendance"
                        }
                        Link {
                            class: "btn is-ghost",
                            to: Route::AdminMemberMatches { id: member.id.clone() },
                            aria_label: "Matches for {name}",
                            "Matches"
                        }
                        button {
                            r#type: "button",
                            class: "btn is-ghost",
                            aria_label: "Game accounts for {name}",
                            aria_disabled: if locked { "true" },
                            aria_describedby: locked_by.clone(),
                            title: if locked { OFFICER_RANK_COPY },
                            onclick: move |_| {
                                if !locked {
                                    on_action.call((MemberAction::Accounts, m_accts.clone()));
                                }
                            },
                            "Game accounts"
                        }
                        button {
                            r#type: "button",
                            class: "btn is-ghost",
                            aria_label: "Avatar for {name}",
                            aria_disabled: if locked { "true" },
                            aria_describedby: locked_by.clone(),
                            title: if locked { OFFICER_RANK_COPY },
                            onclick: move |_| {
                                if !locked {
                                    on_action.call((MemberAction::Avatar, m_avatar.clone()));
                                }
                            },
                            "Avatar"
                        }
                    };
                    rsx! {
                        tr { key: "{id}", class: member_row_class(member.is_active),
                            td { class: "cell-select",
                                label { class: "admin-check-row member-select",
                                    input {
                                        r#type: "checkbox",
                                        id: "member-select-{id}",
                                        aria_label: "Select {name}",
                                        checked,
                                        onchange: move |e| on_select.call((select_id.clone(), e.checked())),
                                    }
                                }
                            }
                            td { class: "cell-who", "{name}" }
                            td { class: "cell-meta", RolePill { role: member.org_role.clone() } }
                            td { class: "cell-meta", "{status_word}" }
                            td { class: "cell-meta num", AdminDate { iso: member.joined_at.clone() } }
                            td { class: "cell-actions",
                                RowActions {
                                    id: member_actions_id(&member.id),
                                    label: "Actions for {name}",
                                    more: Some(more),
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// The selection bar above the table. The count line is always rendered
/// (`role="status"`), so a new selection is announced; the bulk result line
/// is a [`FormStatus`] that keeps the server's reason.
#[component]
fn BulkBar(
    count: usize,
    page_selected: bool,
    page_has_rows: bool,
    is_admin: bool,
    busy: bool,
    state: SaveState,
    on_toggle_page: EventHandler<bool>,
    on_role: EventHandler<()>,
    on_deactivate: EventHandler<()>,
    on_clear: EventHandler<()>,
) -> Element {
    let text = selection_text(count);
    rsx! {
        div {
            class: if count > 0 { "members-bulk-bar has-selection" } else { "members-bulk-bar" },
            role: "group",
            aria_label: "Bulk actions",
            if page_has_rows {
                label { class: "admin-check-row", r#for: SELECT_PAGE_ID,
                    input {
                        r#type: "checkbox",
                        id: SELECT_PAGE_ID,
                        checked: page_selected,
                        onchange: move |e| on_toggle_page.call(e.checked()),
                    }
                    "Select all on this page"
                }
            }
            p { id: BULK_COUNT_ID, class: "members-bulk-count", role: "status", "{text}" }
            if count > 0 {
                div { class: "members-bulk-actions",
                    if is_admin {
                        button {
                            r#type: "button",
                            id: BULK_ROLE_OPEN_ID,
                            class: "btn is-ghost",
                            aria_disabled: if busy { "true" },
                            onclick: move |_| {
                                if !busy {
                                    on_role.call(());
                                }
                            },
                            "Change role"
                        }
                    }
                    button {
                        r#type: "button",
                        id: BULK_DEACTIVATE_OPEN_ID,
                        class: "btn is-danger",
                        aria_disabled: if busy { "true" },
                        onclick: move |_| {
                            if !busy {
                                on_deactivate.call(());
                            }
                        },
                        "Deactivate"
                    }
                    button {
                        r#type: "button",
                        class: "btn is-ghost",
                        onclick: move |_| on_clear.call(()),
                        "Clear selection"
                    }
                }
            }
            FormStatus { state, id: Some(BULK_STATUS_ID.to_string()), focusable: true }
        }
    }
}

/// Columns of the linked game accounts table, in cell order. The first cell
/// is the account name (`cell-who`, the card title on a phone), then the game.
const GAME_ACCOUNT_HEADERS: [&str; 4] = ["Account name", "Game", "Account ID", "Actions"];

/// The member's linked game accounts. One keyed row per account; the game
/// cell shows the game's name, or its id when the game is gone.
#[component]
fn GameAccountsTable(
    accounts: Vec<GameAccount>,
    games: Vec<Game>,
    on_remove: EventHandler<GameAccount>,
) -> Element {
    rsx! {
        DataTable {
            headers: GAME_ACCOUNT_HEADERS.to_vec(),
            cards: true,
            label: "Linked game accounts",
            for acct in accounts {
                tr { key: "{acct.id}",
                    td { class: "cell-who", "{acct.account_name}" }
                    td { class: "cell-meta",
                        {
                            games
                                .iter()
                                .find(|g| g.id == acct.game_id)
                                .map(|g| g.name.clone())
                                .unwrap_or_else(|| acct.game_id.clone())
                        }
                    }
                    td { "data-label": "Account ID",
                        {acct.account_id.clone().unwrap_or_else(|| "None".into())}
                    }
                    td { class: "cell-actions",
                        RowActions {
                            button {
                                r#type: "button",
                                id: account_remove_id(&acct.id),
                                class: "btn is-danger",
                                aria_label: "Remove {acct.account_name}",
                                onclick: {
                                    let acct = acct.clone();
                                    move |_| on_remove.call(acct.clone())
                                },
                                "Remove"
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Role select of the role dialogs. `id` is the record id plus the field
/// name, or a fixed page-unique name for the bulk dialog.
#[component]
fn RoleField(id: String, value: String, on_change: EventHandler<String>) -> Element {
    rsx! {
        div { class: "form-field",
            label { class: "field-label", r#for: "{id}", "Role" }
            select {
                class: "field-input",
                id: "{id}",
                value: "{value}",
                onchange: move |e| on_change.call(e.value()),
                for role in ROLES.iter() {
                    option { value: "{role}", selected: value == *role, "{status_label(role)}" }
                }
            }
        }
    }
}

#[component]
fn PasswordField(member_id: String, value: String, on_input: EventHandler<String>) -> Element {
    let field = format!("member-{member_id}-new-password");
    let hint = format!("{field}-hint");
    rsx! {
        div { class: "form-field",
            label { class: "field-label", r#for: "{field}", "New temporary password" }
            input {
                class: "field-input",
                id: "{field}",
                r#type: "text",
                autocomplete: "off",
                value: "{value}",
                aria_describedby: "{hint}",
                oninput: move |e| on_input.call(e.value()),
            }
            p { class: "field-hint", id: "{hint}",
                "At least 12 characters. Only works for local (username and password) accounts. The member should change it after signing in."
            }
        }
    }
}

/// Id of a linked account's Remove button.
fn account_remove_id(account_id: &str) -> String {
    format!("member-account-remove-{account_id}")
}

/// The game accounts dialog's draft: only its add form. Removals in the
/// table save right away, and an add that went through clears the form.
fn account_draft_unsaved(game_id: &str, name: &str, account_id: &str) -> bool {
    [game_id, name, account_id]
        .iter()
        .any(|field| !field.trim().is_empty())
}

/// The add form of the game accounts dialog.
#[component]
fn AddAccountFields(
    member_id: String,
    games: Vec<Game>,
    game_id: String,
    name: String,
    account_id: String,
    on_game: EventHandler<String>,
    on_name: EventHandler<String>,
    on_account_id: EventHandler<String>,
) -> Element {
    let stem = format!("member-{member_id}-account");
    rsx! {
        div { class: "form-grid",
            div { class: "form-field span-full",
                label { class: "field-label", r#for: "{stem}-game", "Game" }
                select {
                    class: "field-input",
                    id: "{stem}-game",
                    value: "{game_id}",
                    onchange: move |e| on_game.call(e.value()),
                    option { value: "", selected: game_id.is_empty(), "Choose a game" }
                    for game in games.iter() {
                        option { value: "{game.id}", selected: game.id == game_id, "{game.name}" }
                    }
                }
            }
            div { class: "form-field",
                label { class: "field-label", r#for: "{stem}-name", "Account name" }
                input {
                    class: "field-input",
                    id: "{stem}-name",
                    r#type: "text",
                    placeholder: "Player01 BattleTag",
                    value: "{name}",
                    oninput: move |e| on_name.call(e.value()),
                }
            }
            div { class: "form-field",
                label { class: "field-label", r#for: "{stem}-id", "Account ID (optional)" }
                input {
                    class: "field-input",
                    id: "{stem}-id",
                    r#type: "text",
                    value: "{account_id}",
                    oninput: move |e| on_account_id.call(e.value()),
                }
            }
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

    // Role chip, search, page and selection. All live in memory.
    let mut role_filter = use_signal(|| "");
    let mut search = use_signal(String::new);
    let mut page = use_signal(|| 1usize);
    let mut selected: Signal<Vec<String>> = use_signal(Vec::new);
    let mut bulk_state = use_signal(SaveState::default);
    let mut bulk_busy = use_signal(|| false);
    let mut bulk_role_open = use_signal(|| false);
    let mut bulk_role_value = use_signal(|| "member".to_string());
    let mut bulk_deactivate_open = use_signal(|| false);

    // Role change modal
    let mut role_modal = ModalController::<Member>::new();
    let mut role_value = use_signal(String::new);
    let mut role_error: Signal<Option<String>> = use_signal(|| None);

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
    let mut add_acct_error: Signal<Option<String>> = use_signal(|| None);

    // Delete game account confirm
    let mut del_acct_modal = ModalController::<GameAccount>::new();

    // Local password reset modal (admin recovery path; local accounts have no email)
    let mut pw_modal = ModalController::<Member>::new();
    let mut pw_new = use_signal(String::new);
    let mut pw_error: Signal<Option<String>> = use_signal(|| None);

    // Avatar upload modal
    let mut avatar_modal = ModalController::<Member>::new();
    let mut avatar_file: Signal<Option<web_sys::File>> = use_signal(|| None);
    let mut avatar_uploading = use_signal(|| false);
    let mut avatar_error: Signal<Option<String>> = use_signal(|| None);

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
        role_error.set(None);
        role_modal.show(member);
    };

    let on_role_close = move |_| {
        role_modal.close();
    };

    let on_pw_close = move |_| {
        pw_new.set(String::new());
        pw_error.set(None);
        pw_modal.close();
    };

    let on_pw_submit = move |_| {
        if let Some(member) = pw_modal.get_target() {
            let id = member.id.clone();
            let new_password = pw_new();
            if new_password.len() < 12 {
                let message = "Password must be at least 12 characters.";
                pw_error.set(Some(message.to_string()));
                toast.show(Toast::error(message));
                return;
            }
            pw_error.set(None);
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
                    Err(e) => {
                        let message = format!("Reset failed: {e}");
                        pw_error.set(Some(message.clone()));
                        toast.show(Toast::error(message));
                    }
                }
            });
        }
    };

    let on_role_submit = move |_| {
        if let Some(member) = role_modal.get_target() {
            // Selection unchanged: nothing to do (the server rejects same-role changes).
            if role_value() == member.org_role {
                role_modal.close();
                return;
            }
            let id = member.id.clone();
            let body = ChangeRoleRequest { role: role_value() };
            role_error.set(None);
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
                    Err(e) => {
                        let message = format!("Failed to change role: {e}");
                        role_error.set(Some(message.clone()));
                        toast.show(Toast::error(message));
                    }
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
            // The row's "Actions" group closed when the action was chosen.
            focus_element_by_id_soon(member_actions_toggle_id(&id));
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
                    Err(e) => toast.show(Toast::error(profile_edit_failure("Failed", &e))),
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
        add_acct_error.set(None);
        accts_refresh += 1;
        accts_modal.show(member);
    };

    let on_accts_close = move |_| {
        accts_modal.close();
    };

    let on_add_acct = move |_| {
        if add_acct_submitting() {
            return;
        }
        let game_id = add_acct_game_id().trim().to_string();
        let acct_name = add_acct_name().trim().to_string();
        if game_id.is_empty() || acct_name.is_empty() {
            let message = "Game and account name are required.";
            add_acct_error.set(Some(message.to_string()));
            toast.show(Toast::error(message));
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
            add_acct_error.set(None);
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
                    Err(e) => {
                        let message = profile_edit_failure("Failed to add account", &e);
                        add_acct_error.set(Some(message.clone()));
                        toast.show(Toast::error(message));
                    }
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
            // B9: the removed row's button leaves the dialog. Once it is
            // gone, focus the next account's Remove, else the previous one's,
            // else the dialog title.
            let keys: Vec<String> = accts_data.peek().iter().map(|a| a.id.clone()).collect();
            let focus_next = after_delete_focus_ids(
                &keys,
                &aid,
                |k| vec![account_remove_id(k)],
                &["form-modal-title"],
            );
            del_acct_modal.close();
            spawn(async move {
                match ApiClient::web()
                    .delete(&format!("/api/members/{mid}/game-accounts/{aid}"))
                    .await
                {
                    Ok(_) => {
                        toast.show(Toast::success("Game account removed."));
                        accts_refresh += 1;
                        focus_after_delete_soon(account_remove_id(&aid), focus_next);
                    }
                    Err(e) => toast.show(Toast::error(profile_edit_failure("Delete failed", &e))),
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
        avatar_error.set(None);
        avatar_modal.show(member);
    };

    let on_avatar_close = move |_| {
        avatar_modal.close();
    };

    let on_avatar_file_change = move |_e: Event<FormData>| {
        if let Some(file) = crate::upload::selected_file(AVATAR_INPUT_ID) {
            avatar_error.set(None);
            avatar_file.set(Some(file));
        }
    };

    let on_avatar_submit = move |_| {
        let Some(file) = avatar_file() else {
            let message = "Select a file first.";
            avatar_error.set(Some(message.to_string()));
            toast.show(Toast::error(message));
            return;
        };
        if file.size() > 2_000_000.0 {
            let message = "File must be under 2MB.";
            avatar_error.set(Some(message.to_string()));
            toast.show(Toast::error(message));
            return;
        }
        let Some(member) = avatar_modal.get_target() else {
            return;
        };
        let mid = member.id.clone();
        avatar_error.set(None);
        avatar_uploading.set(true);
        spawn(async move {
            let failure = match crate::upload::post_file(&admin_avatar_upload_url(&mid), &file)
                .await
            {
                Ok(url) => {
                    let body = UpdateAvatarBody {
                        avatar_url: Some(url),
                    };
                    match ApiClient::web()
                        .put_json_empty(&format!("/api/members/{mid}"), &body)
                        .await
                    {
                        Ok(_) => {
                            toast.show(Toast::success("Avatar updated."));
                            avatar_modal.close();
                            members.refresh += 1;
                            None
                        }
                        Err(e) => Some(format!("Failed to update profile: {e}")),
                    }
                }
                Err(UploadFailure::Status(status)) => Some(avatar_upload_failure_message(status)),
                Err(UploadFailure::Message(msg)) => Some(msg.to_string()),
            };
            if let Some(message) = failure {
                avatar_error.set(Some(message.clone()));
                toast.show(Toast::error(message));
            }
            avatar_uploading.set(false);
        });
    };

    // --- Bulk handlers ---

    // The selected members, from the loaded list.
    let selected_members = move || -> Vec<Member> {
        let ids = selected();
        members
            .data
            .read()
            .as_ref()
            .and_then(|d| d.as_ref())
            .map(|list| {
                list.iter()
                    .filter(|m| ids.contains(&m.id))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    };

    let on_bulk_deactivate = move |_| {
        bulk_deactivate_open.set(false);
        let BulkTargets { targets, skipped } =
            bulk_deactivate_targets(selected_members(), auth().user.as_ref());
        if targets.is_empty() {
            let text = if skipped.is_empty() {
                "The selected members are already inactive.".to_string()
            } else {
                format!("No member was deactivated. {}", skipped.join(" "))
            };
            bulk_state.set(SaveState::Saved(text));
            focus_element_by_id_soon(BULK_STATUS_ID.to_string());
            return;
        }
        bulk_busy.set(true);
        bulk_state.set(SaveState::Saving);
        spawn(async move {
            let attempted = targets.len();
            let mut done = 0;
            let mut failures = Vec::new();
            let mut failed_ids = Vec::new();
            for member in targets {
                let body = ToggleActiveRequest {
                    is_active: Some(false),
                };
                match ApiClient::web()
                    .put_json_empty(&format!("/api/members/{}", member.id), &body)
                    .await
                {
                    Ok(_) => {
                        done += 1;
                        let mut row = member.clone();
                        row.is_active = false;
                        recently_inactive.write().retain(|m| m.id != member.id);
                        recently_inactive.write().push(row);
                    }
                    Err(e) => {
                        failures.push(bulk_failure(&member, &e));
                        failed_ids.push(member.id.clone());
                    }
                }
            }
            let result = bulk_result("Deactivated", done, attempted, &failures, &skipped);
            if result.is_error() {
                toast.show(Toast::error(result.text()));
            } else {
                toast.show(Toast::success(result.text()));
            }
            bulk_state.set(result);
            bulk_busy.set(false);
            // Failed rows stay selected, so the admin can retry them.
            selected.set(failed_ids);
            members.refresh += 1;
            games.refresh += 1;
            focus_element_by_id_soon(BULK_STATUS_ID.to_string());
        });
    };

    let on_bulk_role_submit = move |_| {
        let role = bulk_role_value();
        let BulkTargets { targets, skipped } =
            bulk_role_targets(selected_members(), &role, auth().user.as_ref());
        bulk_role_open.set(false);
        if targets.is_empty() {
            let text = if skipped.is_empty() {
                format!(
                    "The selected members already have the {} role.",
                    status_label(&role)
                )
            } else {
                format!("No role was changed. {}", skipped.join(" "))
            };
            bulk_state.set(SaveState::Saved(text));
            focus_element_by_id_soon(BULK_STATUS_ID.to_string());
            return;
        }
        bulk_busy.set(true);
        bulk_state.set(SaveState::Saving);
        spawn(async move {
            let attempted = targets.len();
            let mut done = 0;
            let mut failures = Vec::new();
            let mut failed_ids = Vec::new();
            for member in targets {
                let body = ChangeRoleRequest { role: role.clone() };
                match ApiClient::web()
                    .patch_json_empty(&format!("/api/members/{}/role", member.id), &body)
                    .await
                {
                    Ok(_) => done += 1,
                    Err(e) => {
                        failures.push(format!("{}: {e}.", member.display_name));
                        failed_ids.push(member.id.clone());
                    }
                }
            }
            let result = bulk_result("Changed the role of", done, attempted, &failures, &skipped);
            if result.is_error() {
                toast.show(Toast::error(result.text()));
            } else {
                toast.show(Toast::success(result.text()));
            }
            bulk_state.set(result);
            bulk_busy.set(false);
            selected.set(failed_ids);
            members.refresh += 1;
            games.refresh += 1;
            focus_element_by_id_soon(BULK_STATUS_ID.to_string());
        });
    };

    let on_action = move |(action, member): (MemberAction, Member)| match action {
        MemberAction::Role => open_role(member),
        MemberAction::Toggle => open_toggle(member),
        MemberAction::Password => {
            pw_new.set(String::new());
            pw_error.set(None);
            pw_modal.show(member);
        }
        MemberAction::Moderation => open_mod_history(member),
        MemberAction::Attendance => open_stats(member),
        MemberAction::Accounts => open_accounts(member),
        MemberAction::Avatar => open_avatar(member),
    };

    // --- Render ---

    let me = auth().user;
    let all: Option<Vec<Member>> = members.data.read().as_ref().and_then(|d| d.clone());
    let shown = all
        .as_deref()
        .map(|list| filter_members(list, role_filter(), &search()))
        .unwrap_or_default();
    let total = shown.len();
    let rows: Vec<Member> = page_slice(&shown, page(), ADMIN_PAGE_SIZE).to_vec();
    let page_ids: Vec<String> = rows.iter().map(|m| m.id.clone()).collect();
    let selected_now = selected();
    let page_has_rows = !page_ids.is_empty();
    let page_selected = page_has_rows && page_ids.iter().all(|id| selected_now.contains(id));
    let games_list: Vec<Game> = games
        .data
        .read()
        .as_ref()
        .and_then(|d| d.clone())
        .unwrap_or_default();

    let list = match all.as_ref() {
        None => admin_list_pending(&members, "members"),
        Some(all) => rsx! {
            if all.is_empty() {
                EmptyCard {
                    title: "No members yet",
                    message: "Members show up here once an application is accepted. Review open applications to add someone.",
                    actions: rsx! {
                        Link { to: Route::AdminApplications {}, class: "btn is-ghost", "Open applications" }
                    },
                }
            } else if rows.is_empty() {
                EmptyCard {
                    title: "No members match these filters",
                    message: "Clear the role filter and the search to see the full roster.",
                    actions: rsx! {
                        button {
                            r#type: "button",
                            class: "btn is-ghost",
                            onclick: move |_| {
                                role_filter.set("");
                                search.set(String::new());
                                selected.set(Vec::new());
                                page.set(1);
                            },
                            "Clear filters"
                        }
                    },
                }
            } else {
                MembersTable {
                    rows: rows.clone(),
                    selected: selected_now.clone(),
                    is_admin,
                    me: me.clone(),
                    on_select: move |(id, on): (String, bool)| {
                        let mut list = selected.write();
                        list.retain(|s| s != &id);
                        if on {
                            list.push(id);
                        }
                    },
                    on_action,
                }
            }
            // Always rendered: its count line is the list's live line.
            Pager {
                id: "members-pager",
                page: page(),
                total,
                page_size: ADMIN_PAGE_SIZE,
                noun: "members",
                noun_singular: "member",
                show_page_count: true,
                on_change: move |next| {
                    page.set(next);
                    show_list_top(LIST_ID);
                },
            }
        },
    };

    let inactive = {
        let listed_ids: Vec<String> = all
            .as_ref()
            .map(|list| list.iter().map(|m| m.id.clone()).collect())
            .unwrap_or_default();
        session_inactive_not_in_list(&recently_inactive(), &listed_ids)
    };

    rsx! {
        crate::styles::StyleOnce { id: "scuffed-css-pages-admin-members-css", css: MEMBERS_CSS }
        PageShell { width: PageWidth::Wide, class: "admin-members-page",
            PageHeader { title: "Members", lead: list_intro_copy(list_mode).to_string() }

            div { class: "admin-toolbar",
                ChipRow { label: "Filter by role", scroll: true,
                    for (role, word) in ROLE_FILTERS {
                        ChipButton {
                            key: "{word}",
                            class: "chip",
                            id: role_filter_id(role),
                            label: word,
                            pressed: role_filter() == role,
                            onclick: move |_| {
                                role_filter.set(role);
                                selected.set(Vec::new());
                                page.set(1);
                            },
                        }
                    }
                }
                div { class: "admin-toolbar-search",
                    input {
                        class: "field-input",
                        id: SEARCH_ID,
                        r#type: "search",
                        aria_label: "Search members",
                        placeholder: "Search members",
                        value: "{search}",
                        oninput: move |e| {
                            search.set(e.value());
                            selected.set(Vec::new());
                            page.set(1);
                        },
                    }
                }
            }

            BulkBar {
                count: selected_now.len(),
                page_selected,
                page_has_rows,
                is_admin,
                busy: bulk_busy(),
                state: bulk_state(),
                on_toggle_page: move |on: bool| {
                    let mut list = selected.write();
                    list.retain(|id| !page_ids.contains(id));
                    if on {
                        list.extend(page_ids.iter().cloned());
                    }
                },
                on_role: move |_| {
                    bulk_role_value.set("member".to_string());
                    bulk_role_open.set(true);
                },
                on_deactivate: move |_| bulk_deactivate_open.set(true),
                on_clear: move |_| {
                    selected.set(Vec::new());
                    focus_element_by_id_soon(SEARCH_ID.to_string());
                },
            }

            section {
                id: LIST_ID,
                class: "admin-list pager-target",
                tabindex: "-1",
                aria_label: "Members",
                {list}
            }

            if !inactive.is_empty() {
                section { class: "members-inactive", aria_labelledby: "members-inactive-title",
                    h2 { id: "members-inactive-title", class: "ui-section-title",
                        "Recently deactivated (this session)"
                    }
                    p { "{session_inactive_note()}" }
                    DataTable {
                        headers: vec!["Name", "Role", "Status", "Actions"],
                        cards: true,
                        label: "Recently deactivated members",
                        for member in inactive.iter() {
                            {
                                let m_toggle = member.clone();
                                let locked = officer_locked_out(me.as_ref(), member);
                                let lock_note = format!("member-inactive-lock-{}", member.id);
                                rsx! {
                                    tr { key: "inactive-{member.id}", class: "is-inactive",
                                        td { class: "cell-who", "{member.display_name}" }
                                        td { class: "cell-meta", RolePill { role: member.org_role.clone() } }
                                        td { class: "cell-meta", "Inactive" }
                                        td { class: "cell-actions",
                                            RowActions {
                                                if locked {
                                                    span { id: "{lock_note}", class: "visually-hidden", "{OFFICER_RANK_COPY}" }
                                                }
                                                button {
                                                    r#type: "button",
                                                    id: "member-activate-{member.id}",
                                                    class: "btn is-ghost",
                                                    aria_label: "Activate {member.display_name}",
                                                    aria_disabled: if locked { "true" },
                                                    aria_describedby: locked.then(|| lock_note.clone()),
                                                    title: if locked { OFFICER_RANK_COPY },
                                                    onclick: move |_| {
                                                        if !locked {
                                                            open_toggle(m_toggle.clone());
                                                        }
                                                    },
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
        }

        // Role change modal
        {
            let target = role_modal.get_target();
            let member_id = target.as_ref().map(|m| m.id.clone()).unwrap_or_default();
            rsx! {
                FormModal {
                    title: format!(
                        "Change role: {}",
                        target.as_ref().map(|m| m.display_name.clone()).unwrap_or_default()
                    ),
                    open: role_modal.is_open(),
                    submitting: role_modal.is_submitting(),
                    error: role_error(),
                    restore_focus_id: Some(member_actions_toggle_id(&member_id)),
                    on_close: on_role_close,
                    on_submit: on_role_submit,
                    RoleField {
                        id: format!("member-{member_id}-role"),
                        value: role_value(),
                        on_change: move |v| role_value.set(v),
                    }
                }
            }
        }

        // Bulk role change
        FormModal {
            title: format!("Change role: {}", selection_text(selected_now.len())),
            open: bulk_role_open(),
            submitting: false,
            submit_label: Some("Change role".to_string()),
            restore_focus_id: Some(BULK_ROLE_OPEN_ID.to_string()),
            on_close: move |_| bulk_role_open.set(false),
            on_submit: on_bulk_role_submit,
            RoleField {
                id: BULK_ROLE_FIELD_ID.to_string(),
                value: bulk_role_value(),
                on_change: move |v| bulk_role_value.set(v),
            }
            if let Some(note) = bulk_role_own_note(&selected_members(), me.as_ref()) {
                p { class: "field-hint", "{note}" }
            }
        }

        // Bulk deactivate
        ConfirmDialog {
            title: "Deactivate members".to_string(),
            message: bulk_deactivate_message(
                &bulk_deactivate_targets(selected_members(), me.as_ref()),
            ),
            open: bulk_deactivate_open(),
            danger: true,
            confirm_label: "Deactivate".to_string(),
            restore_focus_id: Some(BULK_DEACTIVATE_OPEN_ID.to_string()),
            on_confirm: on_bulk_deactivate,
            on_cancel: move |_| bulk_deactivate_open.set(false),
        }

        // Local password reset modal
        {
            let target = pw_modal.get_target();
            let member_id = target.as_ref().map(|m| m.id.clone()).unwrap_or_default();
            rsx! {
                FormModal {
                    title: format!(
                        "Reset password: {}",
                        target.as_ref().map(|m| m.display_name.clone()).unwrap_or_default()
                    ),
                    open: pw_modal.is_open(),
                    submitting: pw_modal.is_submitting(),
                    submit_label: Some("Reset password".to_string()),
                    error: pw_error(),
                    restore_focus_id: Some(member_actions_toggle_id(&member_id)),
                    on_close: on_pw_close,
                    on_submit: on_pw_submit,
                    PasswordField {
                        member_id,
                        value: pw_new(),
                        on_input: move |v| pw_new.set(v),
                    }
                }
            }
        }

        // Toggle active confirm
        {
            let target = toggle_modal.get_target();
            let active = target.as_ref().map(|m| m.is_active).unwrap_or(false);
            let verb = if active { "Deactivate" } else { "Activate" };
            let name = target.as_ref().map(|m| m.display_name.clone()).unwrap_or_default();
            // From the list the dialog was opened through "Actions"; from the
            // recently deactivated list, through the row's Activate button.
            let opener = target.as_ref().map(|m| {
                if recently_inactive().iter().any(|r| r.id == m.id)
                    && !all.as_ref().is_some_and(|l| l.iter().any(|r| r.id == m.id))
                {
                    format!("member-activate-{}", m.id)
                } else {
                    member_actions_toggle_id(&m.id)
                }
            });
            rsx! {
                ConfirmDialog {
                    title: format!("{verb} member"),
                    message: format!("{verb} \"{name}\"?"),
                    open: toggle_modal.is_open(),
                    danger: active,
                    confirm_label: verb.to_string(),
                    restore_focus_id: opener,
                    on_confirm: on_toggle_confirm,
                    on_cancel: on_toggle_cancel,
                }
            }
        }

        // Mod history modal
        if let Some(target) = mod_modal.get_target().filter(|_| mod_modal.is_open()) {
            MemberDialog {
                title: format!("Moderation: {}", target.display_name),
                restore_focus_id: member_actions_toggle_id(&target.id),
                wide: true,
                on_close: move |_| on_mod_close(()),
                if mod_loading() {
                    p { class: "member-dialog-loading", role: "status", "Loading moderation history..." }
                } else if let Some(err) = mod_error() {
                    p { class: "fetch-error", role: "alert",
                        "Could not load the moderation history. {err}"
                    }
                } else if mod_data.read().is_empty() {
                    p { class: "member-dialog-empty", "No moderation history for this member." }
                } else {
                    DataTable {
                        headers: vec!["Action", "Reason", "Status", "Date"],
                        num_cols: vec![3],
                        cards: true,
                        label: "Moderation history",
                        for action in mod_data.read().iter() {
                            {
                                let active_str = if action.is_active { "active" } else { "inactive" };
                                rsx! {
                                    tr { key: "{action.id}",
                                        td { class: "cell-who", "{status_label(&action.action_type)}" }
                                        td { "data-label": "Reason", "{action.reason}" }
                                        td { class: "cell-badge", StatusPill { status: active_str.to_string() } }
                                        td { class: "cell-meta num", AdminDate { iso: action.created_at.clone() } }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Attendance stats modal
        if let Some(target) = stats_modal.get_target().filter(|_| stats_modal.is_open()) {
            MemberDialog {
                title: format!("Attendance: {}", target.display_name),
                restore_focus_id: member_actions_toggle_id(&target.id),
                on_close: move |_| on_stats_close(()),
                if stats_loading() {
                    p { class: "member-dialog-loading", role: "status", "Loading attendance..." }
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
                                SummaryCard { value: total, label: "Total events" }
                                SummaryCard { value: attended, label: "Attended" }
                                SummaryCard { value: absent, label: "Absent" }
                                SummaryCard { value: excused, label: "Excused" }
                            }
                            if no_events {
                                p { class: "member-dialog-empty", "No events recorded for this member." }
                            }
                            p { class: "member-attendance-rate",
                                span { class: "value", "{rate}" }
                                span { class: "label", "Attendance rate" }
                            }
                        }
                    }
                } else {
                    p { class: "member-dialog-empty", "No attendance data." }
                }
            }
        }

        // Game accounts modal: the linked accounts, then the add form. Save
        // adds the account in the form.
        {
            let target = accts_modal.get_target();
            let member_id = target.as_ref().map(|m| m.id.clone()).unwrap_or_default();
            rsx! {
                FormModal {
                    title: format!(
                        "Game accounts: {}",
                        target.as_ref().map(|m| m.display_name.clone()).unwrap_or_default()
                    ),
                    open: accts_modal.is_open(),
                    submitting: add_acct_submitting(),
                    wide: true,
                    submit_label: Some("Add account".to_string()),
                    error: add_acct_error(),
                    restore_focus_id: Some(member_actions_toggle_id(&member_id)),
                    dirty: Some(account_draft_unsaved(&add_acct_game_id(), &add_acct_name(), &add_acct_id())),
                    on_close: on_accts_close,
                    on_submit: on_add_acct,
                    if accts_loading() {
                        p { class: "member-dialog-loading", role: "status", "Loading game accounts..." }
                    } else if let Some(err) = accts_error() {
                        p { class: "fetch-error", role: "alert", "Could not load game accounts. {err}" }
                    } else if accts_data.read().is_empty() {
                        p { class: "member-dialog-empty", "No game accounts linked yet. Add one below." }
                    } else {
                        GameAccountsTable {
                            accounts: accts_data.read().clone(),
                            games: games_list.clone(),
                            on_remove: move |acct: GameAccount| open_del_acct(acct),
                        }
                    }
                    div { class: "member-accounts-add",
                        h3 { "Add an account" }
                        AddAccountFields {
                            member_id,
                            games: games_list.clone(),
                            game_id: add_acct_game_id(),
                            name: add_acct_name(),
                            account_id: add_acct_id(),
                            on_game: move |v| add_acct_game_id.set(v),
                            on_name: move |v| add_acct_name.set(v),
                            on_account_id: move |v| add_acct_id.set(v),
                        }
                    }
                }
            }
        }

        // Delete game account confirm
        {
            let target = del_acct_modal.get_target();
            rsx! {
                ConfirmDialog {
                    title: "Remove game account".to_string(),
                    message: format!(
                        "Remove account \"{}\"?",
                        target.as_ref().map(|a| a.account_name.clone()).unwrap_or_default()
                    ),
                    open: del_acct_modal.is_open(),
                    danger: true,
                    confirm_label: "Remove".to_string(),
                    restore_focus_id: target.as_ref().map(|a| account_remove_id(&a.id)),
                    on_confirm: on_del_acct_confirm,
                    on_cancel: on_del_acct_cancel,
                }
            }
        }

        // Avatar upload modal
        {
            let target = avatar_modal.get_target();
            let member_id = target.as_ref().map(|m| m.id.clone()).unwrap_or_default();
            let file_label = match avatar_file() {
                Some(f) => format!("Selected: {} ({:.1} KB)", f.name(), f.size() / 1024.0),
                None => "No file selected yet.".to_string(),
            };
            rsx! {
                FormModal {
                    title: format!(
                        "Avatar: {}",
                        target.as_ref().map(|m| m.display_name.clone()).unwrap_or_default()
                    ),
                    open: avatar_modal.is_open(),
                    submitting: avatar_uploading(),
                    submit_label: Some("Upload".to_string()),
                    error: avatar_error(),
                    restore_focus_id: Some(member_actions_toggle_id(&member_id)),
                    on_close: on_avatar_close,
                    on_submit: on_avatar_submit,
                    AvatarField { file_label, on_change: on_avatar_file_change }
                }
            }
        }
    }
}

/// The avatar file field: a visible label, the accepted types as a hint, and
/// the chosen file named in a live line.
#[component]
fn AvatarField(file_label: String, on_change: EventHandler<Event<FormData>>) -> Element {
    rsx! {
        div { class: "form-field",
            label { class: "field-label", r#for: AVATAR_INPUT_ID, "Profile photo" }
            input {
                class: "field-input",
                id: AVATAR_INPUT_ID,
                r#type: "file",
                accept: crate::upload::IMAGE_UPLOAD_ACCEPT,
                aria_describedby: "{AVATAR_INPUT_ID}-hint {AVATAR_INPUT_ID}-chosen",
                onchange: move |e| on_change.call(e),
            }
            p { class: "field-hint", id: "{AVATAR_INPUT_ID}-hint",
                "PNG, JPEG, WebP, GIF or AVIF, up to 2 MB."
            }
            p { class: "field-hint", id: "{AVATAR_INPUT_ID}-chosen", role: "status", "{file_label}" }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_game_accounts_dialog_asks_only_about_its_add_form() {
        // Vision S4 on PR 706: an add or a removal that saved still left the
        // dialog asking about unsaved changes on close.
        assert!(!account_draft_unsaved("", "", ""));
        assert!(!account_draft_unsaved(" ", "", "  "));
        assert!(account_draft_unsaved("ow", "", ""));
        assert!(account_draft_unsaved("", "Player01", ""));
        assert!(account_draft_unsaved("", "", "1234"));
        let src = include_str!("members.rs");
        let prod = src.split("#[cfg(test)]").next().unwrap();
        assert!(prod.contains(
            "dirty: Some(account_draft_unsaved(&add_acct_game_id(), &add_acct_name(), &add_acct_id())),"
        ));
        // A successful add clears all three fields: back to the baseline.
        let ok = prod
            .split("toast.show(Toast::success(\"Game account added.\"));")
            .nth(1)
            .and_then(|rest| rest.split("Err(e)").next())
            .unwrap();
        for field in ["add_acct_game_id", "add_acct_name", "add_acct_id"] {
            assert!(
                ok.contains(&format!("{field}.set(String::new());")),
                "{field}"
            );
        }
    }

    #[test]
    fn removing_a_game_account_focuses_the_next_account() {
        // B9: focus used to jump to the dialog title before the request ran.
        let src = include_str!("members.rs");
        let prod = src.split("#[cfg(test)]").next().unwrap();
        let confirm = prod
            .split("let on_del_acct_confirm = move |_| {")
            .nth(1)
            .and_then(|rest| rest.split("let on_del_acct_cancel").next())
            .unwrap();
        assert!(!confirm.contains("focus_element_by_id_soon"), "{confirm}");
        assert!(
            confirm.contains("|k| vec![account_remove_id(k)],"),
            "{confirm}"
        );
        assert!(confirm.contains("&[\"form-modal-title\"],"), "{confirm}");
        assert!(
            confirm.contains("focus_after_delete_soon(account_remove_id(&aid), focus_next);"),
            "{confirm}"
        );
        assert_eq!(account_remove_id("a1"), "member-account-remove-a1");
    }

    fn member(id: &str, active: bool) -> Member {
        Member {
            id: id.into(),
            user_id: format!("user-{id}"),
            display_name: id.into(),
            org_role: "member".into(),
            is_active: active,
            joined_at: "2026-09-02T00:00:00Z".into(),
        }
    }

    fn user(id: &str, role: OrgRole) -> UserInfo {
        UserInfo {
            id: id.into(),
            username: id.into(),
            avatar_url: None,
            role: Some(role),
        }
    }

    fn member_with_role(id: &str, role: &str) -> Member {
        Member {
            org_role: role.into(),
            ..member(id, true)
        }
    }

    #[test]
    fn empty_status_line_stays_in_the_tree() {
        // Vision nit 5 on PR 706: `display: none` dropped the live line.
        crate::styles::admin::assert_live_line_kept(
            super::MEMBERS_CSS,
            ".members-bulk-count:empty {",
        );
    }

    #[test]
    fn officer_is_locked_out_of_admins_and_officers_only() {
        let officer = user("user-me", OrgRole::Officer);
        for (role, locked) in [
            ("admin", true),
            ("officer", true),
            ("member", false),
            ("recruit", false),
        ] {
            let target = member_with_role("t", role);
            assert_eq!(
                officer_locked_out(Some(&officer), &target),
                locked,
                "{role}"
            );
        }
        // An officer's own row stays editable, as on the server.
        let mut own = member_with_role("me", "officer");
        own.user_id = "user-me".into();
        assert!(!officer_locked_out(Some(&officer), &own));
    }

    #[test]
    fn admins_and_signed_out_are_never_locked_out() {
        let target = member_with_role("t", "admin");
        assert!(!officer_locked_out(
            Some(&user("a", OrgRole::Admin)),
            &target
        ));
        assert!(!officer_locked_out(None, &target));
    }

    #[test]
    fn officer_rank_403_gets_plain_copy() {
        let rank = ClientError::http(403, format!(r#"{{"error":"{OFFICER_RANK_ERROR}"}}"#), None);
        assert_eq!(profile_edit_failure("Failed", &rank), OFFICER_RANK_COPY);
        let other = ClientError::http(403, r#"{"error":"Can only edit your own profile"}"#, None);
        assert_eq!(
            profile_edit_failure("Failed", &other),
            "Failed: HTTP error 403: Can only edit your own profile"
        );
        assert!(!OFFICER_RANK_COPY.contains('\u{2014}'));
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
                retry_after: None,
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
            retry_after: None,
        };
        assert_eq!(limited.http_status(), Some(429));
        let wait = attendance_stats_load_message(&limited);
        assert_eq!(wait, "Too many requests. Try again in 12 seconds.");
        assert!(!wait.contains('\u{2014}'), "{wait}");

        assert_eq!(
            attendance_stats_load_message(&ClientError::Http {
                status: 429,
                body: "Too Many Requests".into(),
                retry_after: None,
            }),
            "Too many requests. Try again later."
        );
        assert_eq!(
            attendance_stats_load_message(&ClientError::Http {
                status: 429,
                body: r#"{"error":"rate_limited","retry_after":3601}"#.into(),
                retry_after: None,
            }),
            "Too many requests. Try again later."
        );
        assert_eq!(
            attendance_stats_load_message(&ClientError::Http {
                status: 500,
                body: r#"{"error":"rate_limited","retry_after":9}"#.into(),
                retry_after: None,
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
                retry_after: None,
            }),
            "Too many requests. Try again in 1 second."
        );
        assert_eq!(
            attendance_stats_load_message(&ClientError::Http {
                status: 429,
                body: r#"{"error":"rate_limited","retry_after":2}"#.into(),
                retry_after: None,
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

    #[test]
    fn page_has_one_h1_role_chips_search_and_a_labelled_wait() {
        use crate::components::admin_shared::test_page::render_admin_route;
        let html = render_admin_route("/admin/members", Some(OrgRole::Admin));
        assert!(
            html.contains("class=\"ui-shell ui-shell--wide admin-members-page\""),
            "{html}"
        );
        assert_eq!(html.matches("<h1").count(), 1, "{html}");
        assert!(html.contains("<h1>Members</h1>"), "{html}");
        assert_eq!(html.matches("<main").count(), 1, "{html}");
        assert!(
            html.contains(
                "<div class=\"chip-row is-scroll\" role=\"group\" aria-label=\"Filter by role\">"
            ),
            "{html}"
        );
        assert_eq!(html.matches("id=\"members-filter-").count(), 5, "{html}");
        assert!(
            html.contains("id=\"members-filter-all\" aria-pressed=\"true\""),
            "{html}"
        );
        assert!(html.contains(">All roles</button>"), "{html}");
        let search = &html[html.find("id=\"members-search\"").expect("search")..];
        let search = &search[..search.find('>').unwrap()];
        assert!(search.contains("type=\"search\""), "{search}");
        assert!(search.contains("aria-label=\"Search members\""), "{search}");
        assert!(
            html.contains(
                "<section id=\"admin-members-list\" class=\"admin-list pager-target\" tabindex=\"-1\" aria-label=\"Members\">"
            ),
            "{html}"
        );
        assert!(
            html.contains("role=\"status\">Loading members...</p>"),
            "{html}"
        );
        // The selection count and the bulk result are live lines that exist
        // before anything is selected.
        assert!(
            html.contains(
                "<p id=\"members-bulk-count\" class=\"members-bulk-count\" role=\"status\"></p>"
            ),
            "{html}"
        );
        assert!(
            html.contains(
                "<p id=\"members-bulk-status\" class=\"admin-form-status\" role=\"status\" tabindex=\"-1\"></p>"
            ),
            "{html}"
        );
        assert!(!html.contains('\u{2014}'), "{html}");
    }

    fn named(id: &str, name: &str, role: &str, active: bool) -> Member {
        Member {
            display_name: name.into(),
            org_role: role.into(),
            ..member(id, active)
        }
    }

    #[test]
    fn rows_are_cards_with_a_select_box_and_every_action_behind_actions() {
        let html = crate::components::ui::test_support::render_routed(|| {
            rsx! {
                MembersTable {
                    rows: vec![
                        named("m1", "Player01", "admin", true),
                        named("m2", "Player02", "member", false),
                    ],
                    selected: vec!["m1".to_string()],
                    is_admin: false,
                    me: Some(user("user-me", OrgRole::Officer)),
                    on_select: |_| {},
                    on_action: |_| {},
                }
            }
        });
        assert!(html.contains("data-table-scroll is-cards"), "{html}");
        assert!(
            html.contains("<th scope=\"col\" class=\"num\">Joined</th>"),
            "{html}"
        );
        let (first, second) = html
            .split_once("</tr><tr")
            .map(|(a, b)| (a.rsplit("<tr").next().unwrap().to_string(), b.to_string()))
            .expect("two body rows");
        // Select box first, then the name, the meta line and the actions.
        let select = first.find("class=\"cell-select\"").expect("select");
        let who = first.find("class=\"cell-who\"").expect("who");
        let meta = first.find("class=\"cell-meta\"").expect("meta");
        let actions = first.find("class=\"cell-actions\"").expect("actions");
        assert!(select < who && who < meta && meta < actions, "{first}");
        assert!(first.contains("id=\"member-select-m1\""), "{first}");
        assert!(first.contains("aria-label=\"Select Player01\""), "{first}");
        assert!(first.contains("checked"), "{first}");
        assert!(first.contains(">Player01</td>"), "{first}");
        assert!(first.contains(">Admin</span>"), "{first}");
        assert!(first.contains(">Active</td>"), "{first}");
        assert!(
            first.contains("<time class=\"date\" datetime=\"2026-09-02\">2 Sep 2026</time>"),
            "{first}"
        );
        // Eight actions do not fit a row: "Actions" on every width.
        assert!(first.contains("class=\"row-actions\""), "{first}");
        assert!(!first.contains("is-inline-desktop"), "{first}");
        assert!(
            first.contains(
                "aria-controls=\"member-actions-m1-more\" aria-label=\"Actions for Player01\""
            ),
            "{first}"
        );
        let group = &first[first.find("class=\"row-menu-group\"").expect("group")..];
        for word in [
            "Deactivate",
            "Moderation",
            "Attendance",
            "Game accounts",
            "Avatar",
        ] {
            assert!(
                group.contains(&format!(">{word}</button>")),
                "{word}: {group}"
            );
        }
        assert!(group.contains(">Matches</a>"), "{group}");
        assert!(
            group.contains("aria-label=\"Matches for Player01\""),
            "{group}"
        );
        // Officers do not see the admin-only actions.
        assert!(!group.contains("Change role"), "{group}");
        assert!(!group.contains("Reset password"), "{group}");
        // An officer cannot edit an admin: the edits stay focusable, say why
        // and do nothing.
        assert!(
            group.contains(&format!(
                "<span id=\"member-lock-m1\" class=\"visually-hidden\">{OFFICER_RANK_COPY}</span>"
            )),
            "{group}"
        );
        assert_eq!(
            group.matches("aria-describedby=\"member-lock-m1\"").count(),
            3,
            "{group}"
        );
        assert!(!group.contains(" disabled"), "{group}");
        // A member row: inactive fill, the word, and nothing locked.
        assert!(second.starts_with(" class=\"is-inactive\""), "{second}");
        assert!(second.contains(">Inactive</td>"), "{second}");
        assert!(second.contains(">Activate</button>"), "{second}");
        assert!(!second.contains("aria-disabled"), "{second}");
        assert!(!second.contains("member-lock-"), "{second}");
        assert!(!html.contains("style="), "{html}");
    }

    #[test]
    fn admins_get_role_and_password_actions() {
        let html = crate::components::ui::test_support::render_routed(|| {
            rsx! {
                MembersTable {
                    rows: vec![named("m1", "Player01", "member", true)],
                    selected: vec![],
                    is_admin: true,
                    me: Some(user("user-admin", OrgRole::Admin)),
                    on_select: |_| {},
                    on_action: |_| {},
                }
            }
        });
        assert!(html.contains(">Change role</button>"), "{html}");
        assert!(html.contains(">Reset password</button>"), "{html}");
        assert!(
            html.contains("aria-label=\"Change role for Player01\""),
            "{html}"
        );
        assert!(!html.contains("aria-disabled"), "{html}");
    }

    /// Every `for` names an element id in the same markup, and every field
    /// control has a label.
    fn assert_labels_tied(html: &str, controls: usize) {
        let fors: Vec<&str> = html
            .split("for=\"")
            .skip(1)
            .map(|rest| rest.split('"').next().unwrap())
            .collect();
        assert_eq!(fors.len(), controls, "{html}");
        for id in fors {
            assert!(!id.is_empty(), "{html}");
            assert_eq!(
                html.matches(&format!(" id=\"{id}\"")).count(),
                1,
                "label for {id} must name one control: {html}"
            );
        }
    }

    #[test]
    fn dialog_fields_have_labels_tied_by_ids_from_data() {
        let role = render(|| {
            rsx! { RoleField { id: "member-m1-role".to_string(), value: "officer".to_string(), on_change: |_| {} } }
        });
        assert_labels_tied(&role, 1);
        assert!(role.contains("id=\"member-m1-role\""), "{role}");
        assert!(role.contains(">Officer</option>"), "{role}");

        let password = render(|| {
            rsx! { PasswordField { member_id: "m1".to_string(), value: String::new(), on_input: |_| {} } }
        });
        assert_labels_tied(&password, 1);
        assert!(
            password.contains("aria-describedby=\"member-m1-new-password-hint\""),
            "{password}"
        );

        let accounts = render(|| {
            rsx! {
                AddAccountFields {
                    member_id: "m1".to_string(),
                    games: vec![Game { id: "g1".into(), name: "Overwatch".into() }],
                    game_id: String::new(),
                    name: String::new(),
                    account_id: String::new(),
                    on_game: |_| {},
                    on_name: |_| {},
                    on_account_id: |_| {},
                }
            }
        });
        assert_labels_tied(&accounts, 3);
        for id in [
            "member-m1-account-game",
            "member-m1-account-name",
            "member-m1-account-id",
        ] {
            assert!(
                accounts.contains(&format!("id=\"{id}\"")),
                "{id}: {accounts}"
            );
        }
        assert!(accounts.contains(">Overwatch</option>"), "{accounts}");

        let avatar = render(|| {
            rsx! { AvatarField { file_label: "No file selected yet.".to_string(), on_change: |_| {} } }
        });
        assert_labels_tied(&avatar, 1);
        assert!(
            avatar.contains("role=\"status\">No file selected yet.</p>"),
            "{avatar}"
        );
        for html in [&role, &password, &accounts, &avatar] {
            assert!(!html.contains("style="), "{html}");
        }
    }

    #[test]
    fn bulk_bar_keeps_its_live_lines_and_shows_actions_once_rows_are_selected() {
        let idle = render(|| {
            rsx! {
                BulkBar {
                    count: 0,
                    page_selected: false,
                    page_has_rows: true,
                    is_admin: true,
                    busy: false,
                    state: SaveState::Idle,
                    on_toggle_page: |_| {},
                    on_role: |_| {},
                    on_deactivate: |_| {},
                    on_clear: |_| {},
                }
            }
        });
        assert!(
            idle.contains(
                "<p id=\"members-bulk-count\" class=\"members-bulk-count\" role=\"status\"></p>"
            ),
            "{idle}"
        );
        assert!(idle.contains("id=\"members-bulk-status\""), "{idle}");
        // Focus moves to the result line after a bulk action, so it must be
        // focusable or focus falls to the body.
        assert!(
            idle.contains(
                "<p id=\"members-bulk-status\" class=\"admin-form-status\" role=\"status\" tabindex=\"-1\">"
            ),
            "{idle}"
        );
        assert!(!idle.contains(">Deactivate</button>"), "{idle}");
        assert_labels_tied(&idle, 1);
        assert!(idle.contains(">Select all on this page</label>"), "{idle}");

        let busy = render(|| {
            rsx! {
                BulkBar {
                    count: 2,
                    page_selected: true,
                    page_has_rows: true,
                    is_admin: true,
                    busy: false,
                    state: SaveState::Failed("Deactivated 1 of 2 members. Player03: HTTP error 409: last admin.".to_string()),
                    on_toggle_page: |_| {},
                    on_role: |_| {},
                    on_deactivate: |_| {},
                    on_clear: |_| {},
                }
            }
        });
        assert!(busy.contains("members-bulk-bar has-selection"), "{busy}");
        assert!(busy.contains("role=\"status\">2 selected</p>"), "{busy}");
        assert!(busy.contains(">Change role</button>"), "{busy}");
        assert!(busy.contains(">Deactivate</button>"), "{busy}");
        assert!(busy.contains("HTTP error 409: last admin."), "{busy}");
    }

    fn bulk_row(id: &str, role: &str, active: bool) -> Member {
        Member {
            org_role: role.into(),
            display_name: format!("Player{id}"),
            ..member(id, active)
        }
    }

    #[test]
    fn bulk_role_change_leaves_out_and_names_the_callers_own_row() {
        let me = user("user-01", OrgRole::Admin);
        let selected = vec![
            bulk_row("01", "admin", true),
            bulk_row("02", "member", true),
            bulk_row("03", "officer", true),
        ];
        let plan = bulk_role_targets(selected.clone(), "officer", Some(&me));
        let ids: Vec<&str> = plan.targets.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["02"]);
        assert_eq!(
            plan.skipped,
            ["Left out Player01 (your own account): change your own role from its row."]
        );
        assert_eq!(
            bulk_role_own_note(&selected, Some(&me)).as_deref(),
            Some("Your own account, Player01, is left out. Change your own role from its row.")
        );
        assert_eq!(bulk_role_own_note(&selected[1..], Some(&me)), None);
        // The skipped line reaches the result.
        assert_eq!(
            bulk_result("Changed the role of", 1, 1, &[], &plan.skipped).text(),
            "Changed the role of 1 member. Left out Player01 (your own account): change your own role from its row."
        );
    }

    #[test]
    fn bulk_deactivate_skips_own_and_locked_rows_by_name() {
        let officer = user("user-01", OrgRole::Officer);
        let selected = vec![
            bulk_row("01", "officer", true),
            bulk_row("02", "member", true),
            bulk_row("03", "admin", true),
            bulk_row("04", "recruit", false),
            bulk_row("05", "recruit", true),
        ];
        let plan = bulk_deactivate_targets(selected.clone(), Some(&officer));
        let ids: Vec<&str> = plan.targets.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["02", "05"]);
        assert_eq!(
            plan.skipped,
            [
                "Left out Player01 (your own account): you cannot deactivate yourself.",
                "Left out Player03: officers cannot deactivate admins or other officers.",
            ]
        );
        assert_eq!(
            bulk_deactivate_message(&plan),
            "Deactivate 2 members? They lose access until someone activates them again. Left out Player01 (your own account): you cannot deactivate yourself. Left out Player03: officers cannot deactivate admins or other officers."
        );
        // An admin may deactivate the admin row; only their own is left out.
        let admin = user("user-01", OrgRole::Admin);
        let plan = bulk_deactivate_targets(selected, Some(&admin));
        let ids: Vec<&str> = plan.targets.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["02", "03", "05"]);
        assert_eq!(plan.skipped.len(), 1);
        assert_eq!(
            bulk_deactivate_message(&BulkTargets::default()),
            "None of the selected members can be deactivated."
        );
    }

    #[test]
    fn bulk_failures_always_name_the_member() {
        let target = bulk_row("03", "admin", true);
        let rank = ClientError::http(403, format!(r#"{{"error":"{OFFICER_RANK_ERROR}"}}"#), None);
        assert_eq!(
            bulk_failure(&target, &rank),
            format!("Player03: {OFFICER_RANK_COPY}")
        );
        let other = ClientError::http(409, r#"{"error":"last admin"}"#, None);
        let text = bulk_failure(&target, &other);
        assert!(text.starts_with("Player03: "), "{text}");
        assert!(text.ends_with('.'), "{text}");
        assert!(!text.contains(".."), "{text}");
    }

    #[test]
    fn game_accounts_headers_match_the_cells_and_rows_are_keyed() {
        fn view() -> Element {
            rsx! {
                GameAccountsTable {
                    accounts: vec![
                        GameAccount {
                            id: "acct-1".into(),
                            game_id: "game-ow".into(),
                            account_name: "Player01 EU".into(),
                            account_id: None,
                        },
                        GameAccount {
                            id: "acct-2".into(),
                            game_id: "game-gone".into(),
                            account_name: "Player02 NA".into(),
                            account_id: Some("42".into()),
                        },
                    ],
                    games: vec![Game { id: "game-ow".into(), name: "Overwatch".into() }],
                    on_remove: |_| {},
                }
            }
        }
        let html = render(view);
        // Header order is the cell order: account name, then game.
        let head = html.split("<tbody>").next().unwrap();
        let name_at = head.find(">Account name</th>").expect(&html);
        let game_at = head.find(">Game</th>").expect(&html);
        assert!(name_at < game_at, "{html}");
        assert!(
            html.contains(
                "<td class=\"cell-who\">Player01 EU</td><td class=\"cell-meta\">Overwatch</td>"
            ),
            "{html}"
        );
        // A game that is gone shows its id.
        assert!(
            html.contains("<td class=\"cell-meta\">game-gone</td>"),
            "{html}"
        );
        assert!(
            html.contains("<td data-label=\"Account ID\">42</td>"),
            "{html}"
        );
        assert!(
            html.contains("id=\"member-account-remove-acct-2\""),
            "{html}"
        );
    }

    #[test]
    fn bulk_result_counts_and_keeps_the_server_reason() {
        assert_eq!(
            bulk_result("Deactivated", 2, 2, &[], &[]),
            SaveState::Saved("Deactivated 2 members.".to_string())
        );
        assert_eq!(
            bulk_result("Deactivated", 1, 1, &[], &[]),
            SaveState::Saved("Deactivated 1 member.".to_string())
        );
        let failed = bulk_result(
            "Deactivated",
            1,
            2,
            &["Player03: HTTP error 409: last admin.".to_string()],
            &[],
        );
        assert!(failed.is_error());
        assert_eq!(
            failed.text(),
            "Deactivated 1 of 2 members. Player03: HTTP error 409: last admin."
        );
        assert_eq!(selection_text(0), "");
        assert_eq!(selection_text(3), "3 selected");
    }

    #[test]
    fn role_chips_and_search_filter_the_roster() {
        let list = vec![
            named("a", "Player01", "member", true),
            named("b", "Player02", "officer", true),
            named("c", "Sample12", "member", false),
        ];
        assert_eq!(filter_members(&list, "", "").len(), 3);
        assert_eq!(filter_members(&list, "member", "").len(), 2);
        let found = filter_members(&list, "", "  player0 ");
        assert_eq!(found.len(), 2);
        assert_eq!(filter_members(&list, "officer", "sample").len(), 0);
        assert_eq!(role_filter_id(""), "members-filter-all");
        assert_eq!(role_filter_id("recruit"), "members-filter-recruit");
        let words: Vec<&str> = ROLE_FILTERS.iter().map(|(_, w)| *w).collect();
        assert_eq!(
            words,
            ["All roles", "Member", "Officer", "Recruit", "Admin"]
        );
        assert_eq!(member_actions_toggle_id("m1"), "member-actions-m1-toggle");
    }

    #[test]
    fn read_only_dialog_is_labelled_and_focuses_its_title() {
        let html = render(|| {
            rsx! {
                MemberDialog {
                    title: "Moderation: Player01".to_string(),
                    restore_focus_id: "member-actions-m1-toggle".to_string(),
                    on_close: |_| {},
                    p { "body" }
                }
            }
        });
        assert!(html.contains("role=\"dialog\""), "{html}");
        assert!(html.contains("aria-modal=\"true\""), "{html}");
        assert!(
            html.contains("aria-labelledby=\"member-dialog-title\""),
            "{html}"
        );
        assert!(
            html.contains("<h2 id=\"member-dialog-title\" class=\"form-modal-header\" tabindex=\"-1\">Moderation: Player01</h2>"),
            "{html}"
        );
        assert!(html.contains("id=\"member-dialog-close\""), "{html}");
        assert!(!html.contains("style="), "{html}");
    }

    #[test]
    fn page_css_is_on_tokens_and_markup_has_no_inline_style() {
        crate::components::ui::test_support::assert_sizes_from_tokens(MEMBERS_CSS);
        let src = include_str!("members.rs");
        let prod = src.split("#[cfg(test)]").next().unwrap();
        assert!(!prod.contains("style: \""), "no inline styles");
        assert!(!prod.contains('\u{2014}'), "no em dashes");
    }
}
