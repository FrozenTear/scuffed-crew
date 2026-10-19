//! Delete my tracker stats and delete my account, on the profile settings page.
//!
//! The pending-delete banner also mounts on My Stats. Wire shapes live in
//! `scuffed_types::api::account_data`.

use std::sync::Mutex;

use chrono::Datelike;

use dioxus::prelude::*;
use scuffed_api_client::{ApiClient, ClientError};
use scuffed_types::{
    CODE_CONFIRM_MISMATCH, CODE_INVALID_PASSWORD, CODE_LAST_ADMIN, CODE_NO_PENDING_DELETE,
    CODE_REAUTH_REQUIRED, DELETE_MY_ACCOUNT_PATH, DELETE_MY_STATS_PATH, DeleteMyAccountRequest,
    DeleteMyAccountResponse, DeleteMyStatsRequest, DeleteMyStatsResponse, MeResponse,
    RESTORE_MY_STATS_PATH, RestoreMyStatsRequest, RestoreMyStatsResponse, StatsDeleteSchedule,
    response_code,
};

use crate::components::{Toast, use_toast};
use crate::layouts::{focus_element, use_document_keydown};
use crate::routes::Route;
use crate::state::{AuthState, StatsViewRefresh, use_auth};

const OPEN_STATS_ID: &str = "delete-my-stats";
const OPEN_ACCOUNT_ID: &str = "delete-my-account";
const DIALOG_ID: &str = "delete-data-dialog";
const TITLE_ID: &str = "delete-data-title";
const NAME_INPUT_ID: &str = "delete-data-confirm-name";
const PASSWORD_INPUT_ID: &str = "delete-data-password";
const ERROR_ID: &str = "delete-data-error";
const CANCEL_ID: &str = "delete-data-cancel";
const CONFIRM_ID: &str = "delete-data-confirm";

pub(crate) const CHAT_DELETE_COPY: &str = "Chat messages are removed from our relay, but copies already saved by other people's apps or other relays can't be pulled back.";

pub(crate) const TRACKER_LOCAL_COPY: &str = "The tracker app on your PC keeps its local copy until you use Delete my data in the tracker's Settings.";

pub(crate) const LAST_ADMIN_COPY: &str =
    "You're the last admin. Make someone else an admin before deleting your account.";

pub(crate) const ACCOUNT_DELETED_COPY: &str = "Your account was deleted.";

pub(crate) const STATS_RESTORED_COPY: &str =
    "Your stats are back. Your tracker was signed out, so create a new sync token to keep syncing.";

const INVALID_PASSWORD_COPY: &str = "That password is wrong.";
const REAUTH_COPY: &str = "For safety, sign out and sign in again, then delete within 10 minutes.";
const CONFIRM_MISMATCH_COPY: &str = "The name you typed doesn't match your account name.";
const NOT_YET_COPY: &str = "This isn't available yet. Try again later.";
const SIGN_IN_AGAIN_COPY: &str = "Sign in again, then retry.";
const FORBIDDEN_COPY: &str = "You don't have permission to do that.";
const REJECTED_COPY: &str = "That request was rejected. Check what you typed and try again.";
const SERVER_COPY: &str = "Something went wrong on our side. Try again in a moment.";
const OFFLINE_COPY: &str = "Couldn't reach the server. Try again.";
const UNREADABLE_COPY: &str = "The server sent a response we couldn't read. Try again.";
const RESTORE_UNCONFIRMED_COPY: &str =
    "The server did not restore your stats. Refresh and try again.";
const ACCOUNT_UNCONFIRMED_COPY: &str =
    "The server did not confirm the deletion. Your account may still be here.";

struct DeleteLists {
    removes: &'static [&'static str],
    stays: &'static [&'static str],
}

struct DeleteDialogCopy {
    stats: DeleteLists,
    account: DeleteLists,
}

/// Removed and kept lines for both confirm dialogs. Adjust this one constant.
const DELETE_LISTS: DeleteDialogCopy = DeleteDialogCopy {
    stats: DeleteLists {
        removes: &[
            "Personal tracker matches",
            "Hero, role, and map rows",
            "Tracker tokens",
        ],
        stays: &[
            "Your account",
            "Your profile, game accounts, and the rest of the site",
            TRACKER_LOCAL_COPY,
        ],
    },
    account: DeleteLists {
        removes: &[
            "Direct messages you sent",
            "Tracker stats and tokens",
            "Sessions, so you are signed out everywhere",
            "RSVPs and poll votes",
            "Game accounts",
            "Personal strategies",
            "Member settings",
            "Device links",
            "Your Nostr link",
            CHAT_DELETE_COPY,
        ],
        stays: &[
            "Forum posts (threads and replies) stay up, with the author shown as \"Deleted member\", like wiki edits, articles, announcements, match results, and audit entries.",
            "On the Nostr forum, your forum thread events are also asked to be removed from our relay. The posts on the site stay as \"Deleted member\".",
        ],
    },
};

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

const ZONE_CSS: &str = r#"
    .delete-data-zone {
        margin-top: 2.5rem;
        padding-top: 1.5rem;
        border-top: 1px solid color-mix(in srgb, var(--danger) 45%, var(--border));
    }
    .delete-data-zone h2 {
        font-family: var(--font-head);
        font-size: 1.1rem;
        color: var(--danger);
        text-transform: uppercase;
        letter-spacing: 0.04em;
        margin: 0 0 0.5rem;
    }
    .delete-data-lead {
        color: var(--text-2);
        font-size: 0.85rem;
        line-height: 1.5;
        margin: 0 0 1rem;
    }
    .delete-data-actions {
        display: flex;
        flex-wrap: wrap;
        gap: 0.6rem;
    }
    .delete-data-btn {
        border: none;
        border-radius: 6px;
        font-weight: 700;
        font-size: 0.85rem;
        padding: 0.6rem 1rem;
        cursor: pointer;
    }
    .delete-data-btn:disabled {
        opacity: 0.55;
        cursor: default;
    }
    .delete-data-btn-danger {
        background: var(--danger);
        color: var(--accent-fg);
    }
    .delete-data-btn-danger:hover:not(:disabled) {
        filter: brightness(1.08);
    }
    .delete-data-btn-quiet {
        background: transparent;
        color: var(--text-2);
        border: 1px solid var(--border);
    }
    .delete-data-notice {
        color: var(--text);
        font-size: 0.85rem;
        margin: 0.75rem 0 0;
    }
    .delete-data-backdrop {
        position: fixed;
        inset: 0;
        background: var(--overlay);
        z-index: 1000;
        display: flex;
        align-items: center;
        justify-content: center;
        padding: 1rem;
    }
    .delete-data-dialog {
        background: var(--surface);
        border: 1px solid var(--border);
        border-radius: 12px;
        padding: 1.5rem;
        width: min(32rem, 100%);
        max-height: 85vh;
        overflow-y: auto;
        color: var(--text);
    }
    .delete-data-dialog h3 {
        font-family: var(--font-head);
        font-size: 1.2rem;
        margin: 0 0 0.75rem;
    }
    .delete-data-dialog h4 {
        font-size: 0.75rem;
        letter-spacing: 0.05em;
        text-transform: uppercase;
        color: var(--text-3);
        margin: 0.9rem 0 0.35rem;
    }
    .delete-data-list {
        margin: 0;
        padding-left: 1.1rem;
        color: var(--text-2);
        font-size: 0.85rem;
        line-height: 1.45;
    }
    .delete-data-note {
        color: var(--text-2);
        font-size: 0.85rem;
        line-height: 1.45;
        margin: 0.75rem 0 0;
    }
    .delete-data-field {
        margin-top: 0.9rem;
    }
    .delete-data-field label {
        display: block;
        font-size: 0.75rem;
        font-weight: 600;
        text-transform: uppercase;
        letter-spacing: 0.05em;
        color: var(--text-3);
        margin-bottom: 0.35rem;
    }
    .delete-data-field input {
        width: 100%;
        background: var(--surface-2);
        border: 1px solid var(--border);
        border-radius: 6px;
        color: var(--text);
        font-size: 0.9rem;
        padding: 0.55rem 0.7rem;
        box-sizing: border-box;
    }
    .delete-data-hint {
        color: var(--text-3);
        font-size: 0.75rem;
        margin: 0.3rem 0 0;
    }
    .delete-data-error {
        color: var(--danger);
        font-size: 0.85rem;
        min-height: 1.2em;
        margin: 0.75rem 0 0;
    }
    .delete-data-dialog-actions {
        display: flex;
        justify-content: flex-end;
        gap: 0.6rem;
        margin-top: 1rem;
    }
    .delete-data-banner {
        background: color-mix(in srgb, var(--warn) 12%, var(--surface));
        border: 1px solid color-mix(in srgb, var(--warn) 45%, var(--border));
        border-radius: 8px;
        padding: 0.9rem 1rem;
        margin: 0 0 1rem;
        color: var(--text);
        font-size: 0.9rem;
        line-height: 1.45;
    }
    .delete-data-banner p { margin: 0 0 0.6rem; }
    .delete-data-banner a { color: var(--accent); font-weight: 600; }
    .delete-data-banner .delete-data-error { margin: 0.5rem 0 0; }
    .account-deleted-notice {
        margin: 0;
        padding: 0.75rem 1.25rem;
        background: color-mix(in srgb, var(--ok) 16%, var(--surface));
        color: var(--text);
        border-bottom: 1px solid var(--border);
        text-align: center;
        font-size: 0.95rem;
    }
"#;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PasswordPrompt {
    Hidden,
    Optional,
    Required,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DeleteKind {
    Closed,
    Stats,
    Account,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BannerPhase {
    Follow,
    Restored,
    Hidden,
}

pub(crate) enum RestoreFailure {
    NothingPending,
    Message(String),
}

pub(crate) fn password_prompt(has_password: Option<bool>) -> PasswordPrompt {
    match has_password {
        Some(true) => PasswordPrompt::Required,
        Some(false) => PasswordPrompt::Hidden,
        None => PasswordPrompt::Optional,
    }
}

pub(crate) fn session_has_password(me: &MeResponse) -> Option<bool> {
    me.has_password
}

pub(crate) fn account_confirm_prompt(display_name: &str) -> String {
    format!(
        "Type your display name ({}) or your username to confirm.",
        display_name.trim()
    )
}

/// Member display name, or the login username when that display name is blank.
pub(crate) fn account_name_from_session(me: &MeResponse) -> String {
    me.member
        .as_ref()
        .map(|member| member.display_name.trim())
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| me.user.username.trim().to_string())
}

/// Trimmed, case-sensitive. An empty account name never confirms.
pub(crate) fn confirm_name_matches(typed: &str, account_name: &str) -> bool {
    let expected = account_name.trim();
    !expected.is_empty() && typed.trim() == expected
}

/// Account delete accepts the display name or the username. Each side is
/// trimmed. The compare stays case-sensitive, and a blank name never matches.
pub(crate) fn confirm_matches_display_or_username(
    typed: &str,
    display_name: &str,
    username: &str,
) -> bool {
    confirm_name_matches(typed, display_name) || confirm_name_matches(typed, username)
}

fn password_satisfied(prompt: PasswordPrompt, password: &str) -> bool {
    match prompt {
        PasswordPrompt::Required => !password.is_empty(),
        PasswordPrompt::Hidden | PasswordPrompt::Optional => true,
    }
}

/// Stats delete checks the typed name only. The request body has no password.
pub(crate) fn stats_delete_enabled(typed: &str, account_name: &str) -> bool {
    confirm_name_matches(typed, account_name)
}

pub(crate) fn delete_confirm_enabled(
    typed: &str,
    account_name: &str,
    prompt: PasswordPrompt,
    password: &str,
) -> bool {
    confirm_name_matches(typed, account_name) && password_satisfied(prompt, password)
}

pub(crate) fn account_delete_enabled(
    typed: &str,
    display_name: &str,
    username: &str,
    prompt: PasswordPrompt,
    password: &str,
) -> bool {
    confirm_matches_display_or_username(typed, display_name, username)
        && password_satisfied(prompt, password)
}

pub(crate) fn delete_account_request(
    typed: &str,
    prompt: PasswordPrompt,
    password: &str,
) -> DeleteMyAccountRequest {
    let password = match prompt {
        PasswordPrompt::Hidden => None,
        PasswordPrompt::Optional | PasswordPrompt::Required => {
            if password.is_empty() {
                None
            } else {
                Some(password.to_string())
            }
        }
    };
    DeleteMyAccountRequest {
        password,
        confirm: typed.trim().to_string(),
    }
}

pub(crate) fn delete_dialog_escape_closes(key: &str, open: bool, busy: bool) -> bool {
    open && !busy && key == "Escape"
}

pub(crate) fn deletion_failure_copy(status: Option<u16>, body: &str) -> String {
    if status == Some(405) || (status == Some(404) && !is_no_pending_delete(status, body)) {
        return NOT_YET_COPY.to_string();
    }
    if let Some(code) = response_code(body) {
        let copy = match code.as_str() {
            CODE_INVALID_PASSWORD => Some(INVALID_PASSWORD_COPY),
            CODE_REAUTH_REQUIRED => Some(REAUTH_COPY),
            CODE_CONFIRM_MISMATCH => Some(CONFIRM_MISMATCH_COPY),
            CODE_LAST_ADMIN => Some(LAST_ADMIN_COPY),
            _ => None,
        };
        if let Some(copy) = copy {
            return copy.to_string();
        }
    }
    match status {
        Some(401) => SIGN_IN_AGAIN_COPY.to_string(),
        Some(403) => FORBIDDEN_COPY.to_string(),
        Some(429) => scuffed_types::too_many_requests_message(429, body)
            .unwrap_or_else(|| scuffed_types::TRY_AGAIN_LATER.to_string()),
        Some(status) if (400..500).contains(&status) => REJECTED_COPY.to_string(),
        Some(status) if (500..600).contains(&status) => SERVER_COPY.to_string(),
        _ => OFFLINE_COPY.to_string(),
    }
}

pub(crate) fn deletion_error_copy(err: &ClientError) -> String {
    match err {
        ClientError::Http { status, body, .. } => deletion_failure_copy(Some(*status), body),
        ClientError::Network(_) => OFFLINE_COPY.to_string(),
        ClientError::Deserialize(_) => UNREADABLE_COPY.to_string(),
    }
}

pub(crate) fn is_no_pending_delete(status: Option<u16>, body: &str) -> bool {
    status == Some(404) && response_code(body).as_deref() == Some(CODE_NO_PENDING_DELETE)
}

pub(crate) fn restore_failure(err: &ClientError) -> RestoreFailure {
    let body = err.http_body().unwrap_or("");
    if is_no_pending_delete(err.http_status(), body) {
        RestoreFailure::NothingPending
    } else {
        RestoreFailure::Message(deletion_error_copy(err))
    }
}

pub(crate) fn present_schedule(value: Option<&str>) -> Option<String> {
    let trimmed = value?.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn calendar_date(year: i32, month: u32, day: u32) -> Option<String> {
    let name = MONTHS.get(month.checked_sub(1)? as usize)?;
    Some(format!("{name} {day}, {year}"))
}

fn utc_calendar_date(iso: &str) -> Option<String> {
    let parsed = chrono::DateTime::parse_from_rfc3339(iso).ok()?;
    let utc = parsed.with_timezone(&chrono::Utc);
    calendar_date(utc.year(), utc.month(), utc.day())
}

#[cfg(all(feature = "web", target_arch = "wasm32"))]
fn local_calendar_date(iso: &str) -> Option<String> {
    let date = js_sys::Date::new(&wasm_bindgen::JsValue::from_str(iso));
    if !date.get_time().is_finite() {
        return None;
    }
    let month = date.get_month();
    let day = date.get_date();
    let year = date.get_full_year();
    calendar_date(i32::try_from(year).ok()?, month.saturating_add(1), day)
}

pub(crate) fn format_deletion_date(iso: &str) -> String {
    #[cfg(all(feature = "web", target_arch = "wasm32"))]
    if let Some(local) = local_calendar_date(iso) {
        return local;
    }
    utc_calendar_date(iso).unwrap_or_else(|| iso.trim().to_string())
}

pub(crate) fn pending_delete_banner_text(iso: &str) -> String {
    format!(
        "Your tracker stats are hidden and set to be deleted on {}.",
        format_deletion_date(iso)
    )
}

fn count_label(key: &str) -> &str {
    match key {
        "personal_match" => "Matches",
        "stat_report" => "Stat reports",
        other => other,
    }
}

pub(crate) fn stats_delete_notice(resp: &DeleteMyStatsResponse) -> String {
    if resp.is_scheduled() {
        let mut message = match present_schedule(resp.stats_delete_scheduled_at.as_deref()) {
            Some(iso) => pending_delete_banner_text(&iso),
            None => "Your tracker stats are hidden and set to be deleted.".to_string(),
        };
        if let Some(tokens) = resp.tokens_revoked {
            message.push_str(&format!(" Tokens revoked: {tokens}."));
        }
        return message;
    }
    let mut message = String::from("Your tracker stats were deleted.");
    let counts = resp.deleted_counts();
    if !counts.is_empty() {
        let parts: Vec<String> = counts
            .iter()
            .map(|(key, count)| format!("{}: {count}", count_label(key)))
            .collect();
        message.push(' ');
        message.push_str(&parts.join(". "));
        message.push('.');
    }
    if let Some(tokens) = resp.tokens_revoked {
        message.push_str(&format!(" Tokens revoked: {tokens}."));
    }
    message
}

static ACCOUNT_DELETED_FLAG: Mutex<bool> = Mutex::new(false);

pub(crate) fn arm_account_deleted_notice() {
    if let Ok(mut slot) = ACCOUNT_DELETED_FLAG.lock() {
        *slot = true;
    }
}

fn account_deleted_notice_armed() -> bool {
    ACCOUNT_DELETED_FLAG
        .lock()
        .map(|slot| *slot)
        .unwrap_or(false)
}

pub(crate) fn account_deleted_notice() -> Element {
    if !account_deleted_notice_armed() {
        return rsx! {};
    }
    rsx! {
        style { {ZONE_CSS} }
        p { class: "account-deleted-notice", role: "status", "{ACCOUNT_DELETED_COPY}" }
    }
}

fn bullet_list(lines: &'static [&'static str]) -> Element {
    rsx! {
        ul { class: "delete-data-list",
            for (index, line) in lines.iter().copied().enumerate() {
                li { key: "{index}", "{line}" }
            }
        }
    }
}

fn password_label(prompt: PasswordPrompt) -> &'static str {
    match prompt {
        PasswordPrompt::Optional => "Current password (optional)",
        PasswordPrompt::Required => "Current password",
        PasswordPrompt::Hidden => "",
    }
}

fn begin_delete(
    mut typed: Signal<String>,
    mut password: Signal<String>,
    mut error: Signal<Option<String>>,
    mut return_focus: Signal<&'static str>,
    mut dialog: Signal<DeleteKind>,
    next: DeleteKind,
    focus: &'static str,
) {
    typed.set(String::new());
    password.set(String::new());
    error.set(None);
    return_focus.set(focus);
    dialog.set(next);
}

fn end_delete(
    busy: Signal<bool>,
    return_focus: Signal<&'static str>,
    mut dialog: Signal<DeleteKind>,
    mut error: Signal<Option<String>>,
) {
    if busy() {
        return;
    }
    let focus = return_focus();
    dialog.set(DeleteKind::Closed);
    error.set(None);
    focus_element(focus);
}

/// Undo banner for a tracker-token soft delete. `scheduled_at` is the
/// `stats_delete_scheduled_at` value from `GET /api/stats/me`.
#[component]
pub fn PendingStatsDeletionBanner(
    scheduled_at: Option<String>,
    on_refresh: EventHandler<()>,
) -> Element {
    let mut phase = use_signal(|| BannerPhase::Follow);
    let mut seen = use_signal(String::new);
    let mut error = use_signal(|| None::<String>);
    let mut busy = use_signal(|| false);
    let mut toast = use_toast();

    let incoming = present_schedule(scheduled_at.as_deref()).unwrap_or_default();
    if seen() != incoming {
        let show_again = !incoming.is_empty();
        seen.set(incoming);
        if show_again {
            phase.set(BannerPhase::Follow);
            error.set(None);
        }
    }

    let pending = present_schedule(scheduled_at.as_deref());
    let show_pending = phase() == BannerPhase::Follow && pending.is_some();
    let show_restored = phase() == BannerPhase::Restored;
    let undo = move |_| {
        if busy() {
            return;
        }
        busy.set(true);
        error.set(None);
        spawn(async move {
            let outcome = ApiClient::web()
                .post_json::<_, RestoreMyStatsResponse>(
                    RESTORE_MY_STATS_PATH,
                    &RestoreMyStatsRequest {},
                )
                .await;
            busy.set(false);
            match outcome {
                Ok(body) if body.restored => {
                    phase.set(BannerPhase::Restored);
                    error.set(None);
                    toast.show(Toast::success(STATS_RESTORED_COPY));
                    on_refresh.call(());
                }
                Ok(_) => error.set(Some(RESTORE_UNCONFIRMED_COPY.to_string())),
                Err(err) => match restore_failure(&err) {
                    RestoreFailure::NothingPending => {
                        phase.set(BannerPhase::Hidden);
                        error.set(None);
                        on_refresh.call(());
                    }
                    RestoreFailure::Message(message) => error.set(Some(message)),
                },
            }
        });
    };

    if !show_pending && !show_restored && error().is_none() {
        return rsx! {};
    }

    rsx! {
        style { {ZONE_CSS} }
        div { class: "delete-data-banner", role: "region", aria_label: "Tracker stats deletion",
            if show_restored {
                p { role: "status",
                    "Your stats are back. Your tracker was signed out, so create a new "
                    Link { to: Route::StatsTokens {}, "sync token" }
                    " to keep syncing."
                }
            } else if let Some(iso) = pending.filter(|_| show_pending) {
                p { role: "status", "{pending_delete_banner_text(&iso)}" }
                button {
                    r#type: "button",
                    class: "delete-data-btn delete-data-btn-quiet",
                    disabled: busy(),
                    onclick: undo,
                    "Undo"
                }
            }
            if let Some(message) = error() {
                p { class: "delete-data-error", role: "alert", aria_live: "assertive", "{message}" }
            }
        }
    }
}

#[component]
pub fn DeleteDataSection(
    account_name: String,
    username: String,
    has_password: Option<bool>,
) -> Element {
    let nav = use_navigator();
    let mut auth = use_auth();
    let mut toast = use_toast();
    let views = StatsViewRefresh::current();
    let prompt = password_prompt(has_password);

    let mut schedule_tick = use_signal(|| 0u64);
    let schedule = use_resource(move || {
        let _ = schedule_tick();
        async move {
            ApiClient::web()
                .fetch::<StatsDeleteSchedule>(DELETE_MY_STATS_PATH)
                .await
                .ok()
                .and_then(|body| present_schedule(body.stats_delete_scheduled_at.as_deref()))
        }
    });

    let mut dialog = use_signal(|| DeleteKind::Closed);
    let mut typed = use_signal(String::new);
    let mut password = use_signal(String::new);
    let mut error = use_signal(|| None::<String>);
    let mut busy = use_signal(|| false);
    let mut notice = use_signal(|| None::<String>);
    let return_focus = use_signal(|| OPEN_STATS_ID);

    use_effect(move || {
        if dialog() != DeleteKind::Closed {
            focus_element(NAME_INPUT_ID);
        }
    });
    use_document_keydown(move |evt| {
        if !delete_dialog_escape_closes(&evt.key(), dialog() != DeleteKind::Closed, busy()) {
            return;
        }
        evt.prevent_default();
        let focus = return_focus();
        dialog.set(DeleteKind::Closed);
        error.set(None);
        focus_element(focus);
    });

    let scheduled_at = schedule.read().as_ref().and_then(|value| value.clone());
    let kind = dialog();
    let enabled = match kind {
        DeleteKind::Account => {
            account_delete_enabled(&typed(), &account_name, &username, prompt, &password())
        }
        DeleteKind::Stats | DeleteKind::Closed => stats_delete_enabled(&typed(), &account_name),
    };

    let mut submit = {
        let account_name = account_name.clone();
        let username = username.clone();
        move |_| {
            if busy() {
                return;
            }
            let kind = dialog();
            let typed_now = typed();
            let password_now = password();
            let allowed = match kind {
                DeleteKind::Account => account_delete_enabled(
                    &typed_now,
                    &account_name,
                    &username,
                    prompt,
                    &password_now,
                ),
                DeleteKind::Stats | DeleteKind::Closed => {
                    stats_delete_enabled(&typed_now, &account_name)
                }
            };
            if !allowed {
                return;
            }
            busy.set(true);
            error.set(None);
            spawn(async move {
                match kind {
                    DeleteKind::Stats => {
                        let result = ApiClient::web()
                            .delete_json::<_, DeleteMyStatsResponse>(
                                DELETE_MY_STATS_PATH,
                                &DeleteMyStatsRequest::website(),
                            )
                            .await;
                        busy.set(false);
                        match result {
                            Ok(body) => {
                                let message = stats_delete_notice(&body);
                                dialog.set(DeleteKind::Closed);
                                notice.set(Some(message.clone()));
                                toast.show(Toast::success(message));
                                schedule_tick += 1;
                                views.bump();
                                focus_element(OPEN_STATS_ID);
                            }
                            Err(err) => error.set(Some(deletion_error_copy(&err))),
                        }
                    }
                    DeleteKind::Account => {
                        let request = delete_account_request(&typed_now, prompt, &password_now);
                        let result = ApiClient::web()
                            .delete_json::<_, DeleteMyAccountResponse>(
                                DELETE_MY_ACCOUNT_PATH,
                                &request,
                            )
                            .await;
                        busy.set(false);
                        match result {
                            Ok(body) if body.deleted => {
                                arm_account_deleted_notice();
                                auth.set(AuthState {
                                    user: None,
                                    loading: false,
                                });
                                nav.push(Route::Home {});
                            }
                            Ok(_) => error.set(Some(ACCOUNT_UNCONFIRMED_COPY.to_string())),
                            Err(err) => error.set(Some(deletion_error_copy(&err))),
                        }
                    }
                    DeleteKind::Closed => busy.set(false),
                }
            });
        }
    };

    let (title, removes, stays) = match kind {
        DeleteKind::Stats => (
            "Delete my tracker stats",
            DELETE_LISTS.stats.removes,
            DELETE_LISTS.stats.stays,
        ),
        DeleteKind::Account | DeleteKind::Closed => (
            "Delete my account",
            DELETE_LISTS.account.removes,
            DELETE_LISTS.account.stays,
        ),
    };
    let (invalid, described_by) = if error().is_some() {
        (Some("true"), Some(ERROR_ID.to_string()))
    } else {
        (None, None)
    };

    rsx! {
        style { {ZONE_CSS} }
        section { class: "delete-data-zone", aria_labelledby: "delete-data-heading",
            h2 { id: "delete-data-heading", "Delete my data" }
            p { class: "delete-data-lead",
                "Delete your tracker stats, or delete your whole account. A stats delete that the tracker scheduled can be undone here."
            }
            PendingStatsDeletionBanner {
                scheduled_at: scheduled_at.clone(),
                on_refresh: move |_| {
                    schedule_tick += 1;
                    views.bump();
                },
            }
            div { class: "delete-data-actions",
                button {
                    r#type: "button",
                    id: OPEN_STATS_ID,
                    class: "delete-data-btn delete-data-btn-danger",
                    onclick: move |_| begin_delete(
                        typed,
                        password,
                        error,
                        return_focus,
                        dialog,
                        DeleteKind::Stats,
                        OPEN_STATS_ID,
                    ),
                    "Delete my tracker stats"
                }
                button {
                    r#type: "button",
                    id: OPEN_ACCOUNT_ID,
                    class: "delete-data-btn delete-data-btn-danger",
                    onclick: move |_| begin_delete(
                        typed,
                        password,
                        error,
                        return_focus,
                        dialog,
                        DeleteKind::Account,
                        OPEN_ACCOUNT_ID,
                    ),
                    "Delete my account"
                }
            }
            if let Some(message) = notice() {
                p { class: "delete-data-notice", role: "status", aria_live: "polite", "{message}" }
            }
        }
        if kind != DeleteKind::Closed {
            div {
                class: "delete-data-backdrop",
                onclick: move |_| end_delete(busy, return_focus, dialog, error),
                div {
                    id: DIALOG_ID,
                    class: "delete-data-dialog",
                    role: "dialog",
                    aria_modal: "true",
                    aria_labelledby: TITLE_ID,
                    tabindex: "-1",
                    onclick: move |evt| evt.stop_propagation(),
                    h3 { id: TITLE_ID, "{title}" }
                    h4 { "This removes" }
                    {bullet_list(removes)}
                    h4 { "This stays" }
                    {bullet_list(stays)}
                    if kind == DeleteKind::Account {
                        p { class: "delete-data-note", "{LAST_ADMIN_COPY}" }
                    }
                    form {
                        onsubmit: move |evt| {
                            evt.prevent_default();
                            submit(());
                        },
                        div { class: "delete-data-field",
                            label { r#for: NAME_INPUT_ID, "Account name" }
                            input {
                                id: NAME_INPUT_ID,
                                r#type: "text",
                                value: typed(),
                                autocomplete: "off",
                                aria_invalid: invalid,
                                aria_describedby: described_by,
                                oninput: move |evt| typed.set(evt.value()),
                            }
                            p { class: "delete-data-hint",
                                if kind == DeleteKind::Account {
                                    "{account_confirm_prompt(&account_name)}"
                                } else {
                                    "Type your account name exactly: {account_name}"
                                }
                            }
                        }
                        if kind == DeleteKind::Account && prompt != PasswordPrompt::Hidden {
                            div { class: "delete-data-field",
                                label { r#for: PASSWORD_INPUT_ID, "{password_label(prompt)}" }
                                input {
                                    id: PASSWORD_INPUT_ID,
                                    r#type: "password",
                                    value: password(),
                                    autocomplete: "current-password",
                                    oninput: move |evt| password.set(evt.value()),
                                }
                                if prompt == PasswordPrompt::Optional {
                                    p { class: "delete-data-hint",
                                        "Leave this blank if you sign in without a password."
                                    }
                                }
                            }
                        }
                        p {
                            id: ERROR_ID,
                            class: "delete-data-error",
                            role: "alert",
                            aria_live: "assertive",
                            {error().unwrap_or_default()}
                        }
                        div { class: "delete-data-dialog-actions",
                            button {
                                r#type: "button",
                                id: CANCEL_ID,
                                class: "delete-data-btn delete-data-btn-quiet",
                                disabled: busy(),
                                onclick: move |_| end_delete(busy, return_focus, dialog, error),
                                "Cancel"
                            }
                            button {
                                r#type: "submit",
                                id: CONFIRM_ID,
                                class: "delete-data-btn delete-data-btn-danger",
                                disabled: !enabled || busy(),
                                if kind == DeleteKind::Stats { "Delete my tracker stats" } else { "Delete my account" }
                            }
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
    use scuffed_types::MeMember;

    fn http(status: u16, body: &str) -> ClientError {
        ClientError::http(status, body, None)
    }

    fn assert_plain(text: &str) {
        assert!(!text.contains('\u{2014}'), "{text}");
        assert!(!text.contains('\u{2013}'), "{text}");
    }

    fn session(display_name: &str, username: &str, has_password: Option<bool>) -> MeResponse {
        MeResponse {
            user: scuffed_types::MeUser {
                id: "u".into(),
                username: username.into(),
                avatar_url: None,
            },
            member: Some(MeMember {
                id: "m".into(),
                org_role: "member".into(),
                display_name: display_name.into(),
                nostr_pubkey: None,
                nostr_key_mode: None,
            }),
            nip05: None,
            has_password,
        }
    }

    #[test]
    fn confirm_requires_a_trimmed_case_sensitive_match() {
        assert!(confirm_name_matches("Ada", "Ada"));
        assert!(confirm_name_matches("  Ada  ", "Ada"));
        assert!(confirm_name_matches("Ada", "  Ada  "));
        assert!(confirm_name_matches("Ada Lovelace", "Ada Lovelace"));
        assert!(!confirm_name_matches("ada", "Ada"));
        assert!(!confirm_name_matches("ADA", "Ada"));
        assert!(!confirm_name_matches("Ada Lovelace", "Ada"));
        assert!(!confirm_name_matches("Ada  Lovelace", "Ada Lovelace"));
        assert!(!confirm_name_matches("Ada", ""));
        assert!(!confirm_name_matches("", ""));
        assert!(!confirm_name_matches("   ", "   "));
        assert!(delete_confirm_enabled(
            " Ada ",
            "Ada",
            PasswordPrompt::Hidden,
            ""
        ));
        assert!(!delete_confirm_enabled(
            "Ada",
            "Ada",
            PasswordPrompt::Required,
            ""
        ));
        assert!(delete_confirm_enabled(
            "Ada",
            "Ada",
            PasswordPrompt::Required,
            "secret"
        ));
        assert!(!delete_confirm_enabled(
            "ada",
            "Ada",
            PasswordPrompt::Hidden,
            "secret"
        ));
    }

    #[test]
    fn account_confirm_accepts_display_name_or_username() {
        let prompt = account_confirm_prompt("Ada");
        assert_eq!(
            prompt,
            "Type your display name (Ada) or your username to confirm."
        );
        assert_plain(&prompt);
        assert_eq!(
            account_confirm_prompt("  Ada  "),
            "Type your display name (Ada) or your username to confirm."
        );
        assert!(confirm_matches_display_or_username("Ada", "Ada", "ada"));
        assert!(confirm_matches_display_or_username("  ada  ", "Ada", "ada"));
        assert!(confirm_matches_display_or_username("Ada", "  Ada  ", "ada"));
        assert!(confirm_matches_display_or_username("ada", "Ada", "  ada "));
        assert!(confirm_matches_display_or_username("Ada", "Ada", "Ada"));
        assert!(confirm_matches_display_or_username("ada", "", "ada"));
        assert!(!confirm_matches_display_or_username("ADA", "Ada", "ada"));
        assert!(!confirm_matches_display_or_username(
            "Ada Lovelace",
            "Ada",
            "ada"
        ));
        assert!(!confirm_matches_display_or_username("Ada", "", "ada"));
        assert!(!confirm_matches_display_or_username("", "Ada", "ada"));
        assert!(!confirm_matches_display_or_username("   ", "   ", "   "));
        assert!(!confirm_matches_display_or_username("Ada", "", ""));
        assert!(account_delete_enabled(
            "ada",
            "Ada",
            "ada",
            PasswordPrompt::Hidden,
            ""
        ));
        assert!(!account_delete_enabled(
            "nope",
            "Ada",
            "ada",
            PasswordPrompt::Hidden,
            ""
        ));
        assert!(!account_delete_enabled(
            "Ada",
            "Ada",
            "ada",
            PasswordPrompt::Required,
            ""
        ));
        assert!(account_delete_enabled(
            "ada",
            "Ada",
            "ada",
            PasswordPrompt::Required,
            "pw"
        ));
    }

    #[test]
    fn account_request_trims_the_name_and_omits_a_blank_password() {
        let hidden = delete_account_request("  Ada  ", PasswordPrompt::Hidden, "ignored");
        assert_eq!(hidden.confirm, "Ada");
        assert_eq!(hidden.password, None);
        let kept = delete_account_request("Ada", PasswordPrompt::Required, " secret ");
        assert_eq!(kept.password.as_deref(), Some(" secret "));
        assert_eq!(
            serde_json::to_string(&kept).unwrap(),
            r#"{"password":" secret ","confirm":"Ada"}"#
        );
    }

    #[test]
    fn account_name_prefers_the_member_display_name() {
        assert_eq!(
            account_name_from_session(&session("Ada", "ada", Some(false))),
            "Ada"
        );
        assert_eq!(
            account_name_from_session(&session("  ", "ada", None)),
            "ada"
        );
        assert_eq!(
            session_has_password(&session("Ada", "ada", Some(false))),
            Some(false)
        );
        assert_eq!(
            session_has_password(&session("Ada", "ada", Some(true))),
            Some(true)
        );
        assert_eq!(session_has_password(&session("Ada", "ada", None)), None);
        assert_eq!(password_prompt(Some(false)), PasswordPrompt::Hidden);
        assert_eq!(password_prompt(Some(true)), PasswordPrompt::Required);
        assert_eq!(password_prompt(None), PasswordPrompt::Optional);
    }

    #[test]
    fn has_password_shows_requires_or_omits_the_field() {
        assert_eq!(password_prompt(Some(true)), PasswordPrompt::Required);
        assert!(!delete_confirm_enabled(
            "Ada",
            "Ada",
            PasswordPrompt::Required,
            ""
        ));
        assert_eq!(
            delete_account_request("Ada", PasswordPrompt::Required, "pw")
                .password
                .as_deref(),
            Some("pw")
        );

        assert_eq!(password_prompt(Some(false)), PasswordPrompt::Hidden);
        assert!(delete_confirm_enabled(
            "Ada",
            "Ada",
            PasswordPrompt::Hidden,
            ""
        ));
        assert_eq!(
            delete_account_request("Ada", PasswordPrompt::Hidden, "pw").password,
            None
        );

        // None is a visible optional field. A blank password is omitted, never hidden.
        assert_eq!(password_prompt(None), PasswordPrompt::Optional);
        assert!(delete_confirm_enabled(
            "Ada",
            "Ada",
            PasswordPrompt::Optional,
            ""
        ));
        assert_eq!(
            delete_account_request("Ada", PasswordPrompt::Optional, "").password,
            None
        );
        assert_eq!(
            delete_account_request("Ada", PasswordPrompt::Optional, "pw")
                .password
                .as_deref(),
            Some("pw")
        );
    }

    #[test]
    fn stats_delete_ignores_the_password() {
        assert!(stats_delete_enabled("Ada", "Ada"));
        assert!(stats_delete_enabled("  Ada  ", "Ada"));
        assert!(!stats_delete_enabled("ada", "Ada"));
        assert!(!stats_delete_enabled("", "Ada"));
        assert!(!stats_delete_enabled("Ada", ""));
        // The same name with a required empty password still blocks account delete.
        assert!(!account_delete_enabled(
            "Ada",
            "Ada",
            "ada",
            PasswordPrompt::Required,
            ""
        ));
        let stats = scuffed_types::DeleteMyStatsRequest::website();
        assert_eq!(
            serde_json::to_string(&stats).unwrap(),
            r#"{"confirm":"delete my stats"}"#
        );
    }

    #[test]
    fn deletion_errors_use_the_code_then_the_status() {
        assert_eq!(
            deletion_error_copy(&http(401, r#"{"code":"invalid_password"}"#)),
            INVALID_PASSWORD_COPY
        );
        assert_eq!(
            deletion_error_copy(&http(401, r#"{"code":"reauth_required"}"#)),
            REAUTH_COPY
        );
        assert_eq!(
            deletion_error_copy(&http(400, r#"{"code":"confirm_mismatch"}"#)),
            CONFIRM_MISMATCH_COPY
        );
        assert_eq!(
            deletion_error_copy(&http(409, r#"{"code":"last_admin"}"#)),
            LAST_ADMIN_COPY
        );
        assert_eq!(
            deletion_error_copy(&http(401, r#"{"error":"invalid_password"}"#)),
            SIGN_IN_AGAIN_COPY
        );
        assert_eq!(deletion_error_copy(&http(403, "{}")), FORBIDDEN_COPY);
        assert_eq!(
            deletion_error_copy(&http(429, r#"{"error":"rate_limited","retry_after":9}"#)),
            "Try again in 9 s"
        );
        assert_eq!(
            deletion_error_copy(&http(429, "slow down")),
            "Too many requests. Try again later."
        );
        assert_eq!(
            deletion_error_copy(&http(400, r#"{"code":"other"}"#)),
            REJECTED_COPY
        );
        assert_eq!(deletion_error_copy(&http(404, "missing")), NOT_YET_COPY);
        assert_eq!(
            deletion_error_copy(&http(404, r#"{"code":"missing"}"#)),
            NOT_YET_COPY
        );
        assert_eq!(deletion_error_copy(&http(405, "missing")), NOT_YET_COPY);
        assert_eq!(
            deletion_error_copy(&http(405, r#"{"error":"Method not allowed"}"#)),
            NOT_YET_COPY
        );
        assert_eq!(
            deletion_error_copy(&http(405, r#"{"code":"last_admin"}"#)),
            NOT_YET_COPY
        );
        assert_eq!(
            deletion_error_copy(&http(404, r#"{"code":"invalid_password"}"#)),
            NOT_YET_COPY
        );
        assert_eq!(deletion_error_copy(&http(500, "x")), SERVER_COPY);
        assert_eq!(deletion_error_copy(&http(503, "")), SERVER_COPY);
        assert_eq!(
            deletion_error_copy(&ClientError::Network("offline".into())),
            OFFLINE_COPY
        );
        assert_eq!(
            deletion_error_copy(&ClientError::Deserialize("bad".into())),
            UNREADABLE_COPY
        );
    }

    #[test]
    fn restore_404_with_no_pending_delete_is_not_a_generic_error() {
        let pending = http(404, r#"{"code":"no_pending_delete"}"#);
        assert!(matches!(
            restore_failure(&pending),
            RestoreFailure::NothingPending
        ));
        let other = http(404, r#"{"code":"missing"}"#);
        assert!(matches!(
            restore_failure(&other),
            RestoreFailure::Message(message) if message == NOT_YET_COPY
        ));
        let method = http(405, r#"{"error":"Method not allowed"}"#);
        assert!(matches!(
            restore_failure(&method),
            RestoreFailure::Message(message) if message == NOT_YET_COPY
        ));
    }

    #[test]
    fn escape_closes_only_an_open_idle_dialog() {
        assert!(delete_dialog_escape_closes("Escape", true, false));
        assert!(!delete_dialog_escape_closes("Escape", true, true));
        assert!(!delete_dialog_escape_closes("Escape", false, false));
        assert!(!delete_dialog_escape_closes("Enter", true, false));
    }

    #[test]
    fn stats_notice_shows_present_counts_and_either_shape() {
        let immediate: DeleteMyStatsResponse = serde_json::from_str(
            r#"{"deleted":{"stat_report":1,"personal_match":4,"custom_row":2},"tokens_revoked":2}"#,
        )
        .unwrap();
        assert_eq!(
            stats_delete_notice(&immediate),
            "Your tracker stats were deleted. Matches: 4. Stat reports: 1. custom_row: 2. Tokens revoked: 2."
        );
        let bare = DeleteMyStatsResponse::default();
        assert_eq!(
            stats_delete_notice(&bare),
            "Your tracker stats were deleted."
        );
        let scheduled: DeleteMyStatsResponse = serde_json::from_str(
            r#"{"scheduled":true,"stats_delete_scheduled_at":"2026-10-26T15:04:05Z","tokens_revoked":1}"#,
        )
        .unwrap();
        assert_eq!(
            stats_delete_notice(&scheduled),
            "Your tracker stats are hidden and set to be deleted on October 26, 2026. Tokens revoked: 1."
        );
        assert_eq!(
            pending_delete_banner_text("2026-10-26T15:04:05Z"),
            "Your tracker stats are hidden and set to be deleted on October 26, 2026."
        );
        assert_eq!(present_schedule(Some("  ")), None);
        assert_eq!(present_schedule(None), None);
    }

    #[test]
    fn user_facing_copy_has_no_em_dash() {
        let lines = DELETE_LISTS
            .stats
            .removes
            .iter()
            .chain(DELETE_LISTS.stats.stays)
            .chain(DELETE_LISTS.account.removes)
            .chain(DELETE_LISTS.account.stays);
        for line in lines {
            assert_plain(line);
        }
        assert!(DELETE_LISTS.account.removes.contains(&CHAT_DELETE_COPY));
        for line in [
            "Direct messages you sent",
            "Tracker stats and tokens",
            "Sessions, so you are signed out everywhere",
            "RSVPs and poll votes",
            "Game accounts",
            "Personal strategies",
            "Member settings",
            "Device links",
            "Your Nostr link",
        ] {
            assert!(
                DELETE_LISTS.account.removes.contains(&line),
                "missing removed line {line}"
            );
        }
        assert!(
            !DELETE_LISTS
                .account
                .removes
                .iter()
                .any(|line| line.to_ascii_lowercase().contains("forum")),
            "forum posts stay up"
        );
        let stays = DELETE_LISTS.account.stays.join(" ");
        for phrase in [
            "Forum posts (threads and replies)",
            "wiki edits",
            "articles",
            "announcements",
            "match results",
            "audit entries",
            "Deleted member",
            "On the Nostr forum",
            "forum thread events",
        ] {
            assert!(stays.contains(phrase), "kept copy missing {phrase}");
        }
        assert!(DELETE_LISTS.stats.stays.contains(&TRACKER_LOCAL_COPY));
        assert_plain(STATS_RESTORED_COPY);
        assert_plain(ACCOUNT_DELETED_COPY);
        assert_plain(LAST_ADMIN_COPY);
        assert_plain(REAUTH_COPY);
        assert_plain(CONFIRM_MISMATCH_COPY);
        assert_plain(&pending_delete_banner_text("not-a-date"));
        assert_eq!(format_deletion_date("not-a-date"), "not-a-date");
    }

    #[test]
    fn banner_is_on_the_account_page_and_the_stats_page() {
        let section = include_str!("delete_data.rs");
        let start = section
            .find("pub fn DeleteDataSection")
            .expect("account section");
        assert!(
            section[start..].contains("PendingStatsDeletionBanner"),
            "the account page section must include the undo banner"
        );
        assert!(
            section.contains("DeleteMyStatsRequest::website()"),
            "stats delete sends the confirm phrase"
        );
        assert!(
            section.contains("DELETE_MY_STATS_PATH"),
            "stats delete uses DELETE /api/stats/me"
        );
        let stats = include_str!("stats/mod.rs");
        assert!(
            stats.contains("PendingStatsDeletionBanner"),
            "personal stats must include the undo banner"
        );
        let profile = include_str!("profile_settings.rs");
        assert!(
            profile.contains("DeleteDataSection"),
            "delete my data lives on the profile settings page"
        );
    }
}
