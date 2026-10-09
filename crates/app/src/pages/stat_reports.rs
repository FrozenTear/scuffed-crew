//! Member and officer views for tracker bug reports.
//!
//! List and withdraw shapes are the shared types from PR 198. The withdraw
//! request body is not in those types, so it stays here. Officer-only JSON
//! keys are omitted for a member. `Option` fields accept that omission.
//!
//! The page never renders report contents (reason text, manifest, images, or
//! the zip). The member column on the officer list is the member id from the
//! API. It is not a display name.

use chrono::{DateTime, Utc};
use dioxus::prelude::*;
use serde::Serialize;

use scuffed_api_client::ClientError;

use crate::components::AccessDenied;
use crate::state::auth::AuthState;

use super::login::remember_login_return;

pub(crate) use scuffed_types::{StatReportList, StatReportListItem, StatReportWithdrawn};

/// `POST /api/stat-reports/{id}/withdraw` body. The server accepts this object
/// and nothing else.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub(crate) struct WithdrawTrainingBody {
    training: bool,
}

impl WithdrawTrainingBody {
    pub(crate) fn clear() -> Self {
        Self { training: false }
    }
}

pub(crate) const COPY_LOADING_MINE: &str = "Loading your reports.";
pub(crate) const COPY_ERROR_MINE: &str =
    "Something went wrong loading your reports. Try again in a moment.";
pub(crate) const COPY_EMPTY_MINE: &str = "You have no reports.";
pub(crate) const COPY_SIGN_IN: &str = "Sign in to view your reports.";
pub(crate) const COPY_MEMBERSHIP: &str = "You need to be an org member to view your reports.";
pub(crate) const COPY_INACTIVE: &str =
    "Your membership is not active, so your reports are not available.";
pub(crate) const COPY_INTRO_MINE: &str = "Reports you sent from the stat tracker. Training consent keeps a report until you delete it. Without that consent, the report is deleted 30 days after it was received. The stored file is not shown here.";
pub(crate) const COPY_CHECKING: &str = "Checking session.";
pub(crate) const COPY_FORBIDDEN: &str = "You need officer permissions to access the admin panel.";
pub(crate) const COPY_LOADING_ALL: &str = "Loading reports.";
pub(crate) const COPY_ERROR_ALL: &str =
    "Something went wrong loading reports. Try again in a moment.";
pub(crate) const COPY_EMPTY_ALL: &str = "No reports yet.";
pub(crate) const COPY_INTRO_ALL: &str = "Tracker reports from members. Download saves the zip. This page does not show what is inside a report.";
pub(crate) const COPY_DELETED: &str = "Report deleted.";
pub(crate) const COPY_WITHDRAWN: &str = "Training consent withdrawn.";
pub(crate) const COPY_DELETE_FAILED: &str = "Could not delete the report.";
pub(crate) const COPY_WITHDRAW_FAILED: &str = "Could not withdraw training consent.";
pub(crate) const COPY_TRY_AGAIN: &str = "Try again";
pub(crate) const COPY_NO_DELETION: &str = "No deletion date";
pub(crate) const COPY_DELETION_MISSING: &str = "Deletion date missing";
pub(crate) const COPY_UNUSABLE: &str = "This report cannot be changed from this page.";
pub(crate) const COPY_DISABLED: &str = "Reports are off right now.";
pub(crate) const COPY_RATE_LIMIT_WAIT: &str = scuffed_types::TRY_AGAIN_LATER;
pub(crate) const COPY_OFFICER_SIGN_IN: &str = "Sign in to view reports.";
pub(crate) const OFFICER_REPORTS_PATH: &str = "/admin/reports";

pub(crate) const MY_REPORTS_RETRY_ID: &str = "my-reports-retry";
pub(crate) const OFFICER_REPORTS_RETRY_ID: &str = "officer-reports-retry";
pub(crate) const CONFIRM_DIALOG_ID: &str = "report-confirm-dialog";
pub(crate) const CONFIRM_FORM_ID: &str = "report-confirm-form";
pub(crate) const CONFIRM_CANCEL_ID: &str = "report-confirm-cancel";
pub(crate) const CONFIRM_SUBMIT_ID: &str = "report-confirm-submit";

const REPORTS_CSS: &str = r#"
.reports-page { max-width: 960px; margin: 0 auto; padding: 2rem 1.25rem 3rem; }
.reports-page h1 {
    font-family: var(--font-head); font-size: 1.8rem; color: var(--text);
    letter-spacing: 0.04em; margin: 0 0 0.75rem;
}
.reports-intro { color: var(--text-2); font-size: 0.95rem; line-height: 1.5; margin: 0 0 1.5rem; }
.reports-status { color: var(--text-3); padding: 1.5rem 0; }
.reports-error { color: var(--danger); }
.reports-scroll { overflow-x: auto; }
.reports-table { width: 100%; border-collapse: collapse; }
.reports-table th, .reports-table td {
    text-align: left; padding: 0.65rem 0.5rem; border-bottom: 1px solid var(--border);
    vertical-align: top; font-size: 0.9rem; color: var(--text);
}
.reports-table th {
    font-size: 0.75rem; text-transform: uppercase; letter-spacing: 0.04em;
    color: var(--text-3); font-weight: 600;
}
.reports-sub { display: block; color: var(--text-2); font-size: 0.8rem; margin-top: 0.2rem; }
.reports-actions { display: flex; flex-wrap: wrap; gap: 0.4rem; align-items: center; }
.report-btn {
    font-family: var(--font-body); font-weight: 600; font-size: 0.85rem;
    border-radius: var(--radius-md); padding: 0.4rem 0.75rem; cursor: pointer;
    background: var(--surface); color: var(--text); border: 1px solid var(--border);
    text-decoration: none; display: inline-flex; align-items: center;
}
.report-btn-danger { background: var(--danger); color: var(--accent-fg); border: none; }
.report-btn:disabled { opacity: 0.5; cursor: not-allowed; }
.report-member-id { font-family: var(--font-mono); font-size: 0.8rem; word-break: break-all; }
.report-control-label {
    position: absolute; width: 1px; height: 1px; padding: 0; margin: -1px;
    overflow: hidden; clip: rect(0, 0, 0, 0); white-space: nowrap; border: 0;
}
.report-dialog-backdrop {
    position: fixed; inset: 0; background: var(--overlay); z-index: 1000;
    display: flex; align-items: center; justify-content: center; padding: 1rem;
}
.report-dialog {
    background: var(--surface); border: 1px solid var(--border); border-radius: 12px;
    padding: 1.5rem; max-width: 28rem; width: 100%;
}
.report-dialog h2 { font-family: var(--font-head); color: var(--text); font-size: 1.2rem; margin: 0 0 0.75rem; }
.report-dialog p { color: var(--text-2); line-height: 1.5; margin: 0 0 1rem; }
.report-dialog-actions { display: flex; justify-content: flex-end; gap: 0.6rem; flex-wrap: wrap; }
"#;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum MemberScreen {
    Loading,
    SignIn,
    Membership,
    Inactive,
    Error,
    Empty,
    Ready,
    Disabled,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum OfficerScreen {
    Checking,
    Forbidden,
    Loading,
    Error,
    Empty,
    Ready,
    Disabled,
    SignIn,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ReportIntent {
    Delete,
    Withdraw,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PendingReportAction {
    pub intent: ReportIntent,
    pub report_id: String,
    /// Row button that opened the dialog. Focus returns here when it closes.
    pub return_focus_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ReportMutation {
    pub intent: ReportIntent,
    pub path: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub(crate) struct ConfirmGate {
    pending: Option<PendingReportAction>,
}

impl ConfirmGate {
    pub(crate) fn pending(&self) -> Option<&PendingReportAction> {
        self.pending.as_ref()
    }

    /// Row click. Opens the dialog and does not return a request.
    pub(crate) fn arm(&mut self, intent: ReportIntent, report_id: &str) {
        if is_report_id(report_id) {
            self.pending = Some(PendingReportAction {
                intent,
                report_id: report_id.to_string(),
                return_focus_id: opener_focus_id(intent, report_id),
            });
        }
    }

    pub(crate) fn cancel(&mut self) {
        self.pending = None;
    }

    /// Dialog confirm. This is the only call that returns a request.
    #[must_use]
    pub(crate) fn confirm(&mut self) -> Option<ReportMutation> {
        let pending = self.pending.take()?;
        mutation_for(pending.intent, &pending.report_id)
    }
}

#[derive(Clone, Debug)]
pub(crate) enum PreparedRows {
    PendingMember,
    Rows(Vec<StatReportListItem>),
}

pub(crate) fn is_report_id(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

pub(crate) fn control_id(kind: &str, report_id: &str) -> String {
    format!("report-{kind}-{report_id}")
}

pub(crate) fn opener_focus_id(intent: ReportIntent, report_id: &str) -> String {
    let kind = match intent {
        ReportIntent::Delete => "delete",
        ReportIntent::Withdraw => "withdraw",
    };
    control_id(kind, report_id)
}

/// Escape closes an open confirm dialog. A busy request keeps the dialog up.
pub(crate) fn confirm_escape_closes(key: &str, open: bool, busy: bool) -> bool {
    open && !busy && key == "Escape"
}

pub(crate) fn login_return_href(path: &str) -> String {
    format!("/login?return={path}")
}

pub(crate) fn mutation_for(intent: ReportIntent, report_id: &str) -> Option<ReportMutation> {
    if !is_report_id(report_id) {
        return None;
    }
    let path = match intent {
        ReportIntent::Delete => format!("/api/stat-reports/{report_id}"),
        ReportIntent::Withdraw => format!("/api/stat-reports/{report_id}/withdraw"),
    };
    Some(ReportMutation { intent, path })
}

/// Officer download. `GET /api/stat-reports/{id}` from PR 189.
pub(crate) fn download_action(report_id: &str) -> Option<String> {
    if !is_report_id(report_id) {
        return None;
    }
    Some(format!("/api/stat-reports/{report_id}"))
}

pub(crate) fn format_report_when(at: DateTime<Utc>) -> String {
    at.format("%Y-%m-%d %H:%M UTC").to_string()
}

pub(crate) fn format_report_size(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = 1024 * 1024;
    if bytes >= MIB {
        let whole = bytes / MIB;
        let frac = (bytes % MIB) * 10 / MIB;
        format!("{whole}.{frac} MB")
    } else if bytes >= KIB {
        let whole = bytes / KIB;
        let frac = (bytes % KIB) * 10 / KIB;
        format!("{whole}.{frac} KB")
    } else if bytes == 1 {
        "1 byte".into()
    } else {
        format!("{bytes} bytes")
    }
}

pub(crate) fn status_text(training_consent: bool) -> &'static str {
    if training_consent {
        "Kept for training"
    } else {
        "Scheduled for deletion"
    }
}

pub(crate) fn training_consent_text(training_consent: bool) -> &'static str {
    if training_consent {
        "Training consent on"
    } else {
        "Training consent off"
    }
}

pub(crate) fn extra_consent_text(own_name_included: bool, glyphs_included: bool) -> String {
    let own = if own_name_included {
        "Own name included"
    } else {
        "Own name not included"
    };
    let glyphs = if glyphs_included {
        "Wrong glyphs included"
    } else {
        "Wrong glyphs not included"
    };
    format!("{own}. {glyphs}.")
}

pub(crate) fn expiry_text(training_consent: bool, expires_at: Option<DateTime<Utc>>) -> String {
    if training_consent {
        COPY_NO_DELETION.into()
    } else if let Some(at) = expires_at {
        format!("Deleted on {}", format_report_when(at))
    } else {
        COPY_DELETION_MISSING.into()
    }
}

pub(crate) fn confirm_copy(intent: ReportIntent) -> (&'static str, &'static str, &'static str) {
    match intent {
        ReportIntent::Delete => (
            "Delete report",
            "Delete this report now? The file is removed and cannot be restored.",
            "Delete report",
        ),
        ReportIntent::Withdraw => (
            "Withdraw training consent",
            "Withdraw training consent for this report? It is deleted 30 days after it was received. If that date has already passed, the report is deleted now.",
            "Withdraw consent",
        ),
    }
}

/// HTTP failure kept as a status code plus the body. Display text is not used.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ListedError {
    pub status: Option<u16>,
    pub body: String,
    pub header_seconds: Option<u64>,
}

impl ListedError {
    pub(crate) fn from_client(err: &ClientError) -> Self {
        Self {
            status: err.http_status(),
            body: err.http_body().unwrap_or("").to_string(),
            header_seconds: err.retry_after_header(),
        }
    }
}

/// `503` whose JSON body is `{"error":"reports_disabled"}`.
pub(crate) fn is_reports_disabled(status: Option<u16>, body: &str) -> bool {
    if status != Some(503) {
        return false;
    }
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|value| {
            value
                .get("error")
                .and_then(|error| error.as_str())
                .map(str::to_owned)
        })
        .is_some_and(|message| message == "reports_disabled")
}

/// Training consent does not expire. Any other report whose `expires_at` is
/// strictly before `now` is already due, so the client hides it as well.
pub(crate) fn report_is_current(row: &StatReportListItem, now: DateTime<Utc>) -> bool {
    if row.training_consent {
        return true;
    }
    !row.expires_at.is_some_and(|expires_at| expires_at < now)
}

pub(crate) fn without_expired(
    rows: Vec<StatReportListItem>,
    now: DateTime<Utc>,
) -> Vec<StatReportListItem> {
    rows.into_iter()
        .filter(|row| report_is_current(row, now))
        .collect()
}

/// Seconds to wait on a 429. JSON `retry_after` wins. The header is the fallback.
/// A plain-text body with no usable wait returns `None`.
/// [`scuffed_types::json_retry_after`] drops waits outside `1..=3600`, arrays, and null.
pub(crate) fn retry_after_seconds(body: &str, header_seconds: Option<u64>) -> Option<u64> {
    scuffed_types::json_retry_after(body).or_else(|| header_seconds.filter(|seconds| *seconds >= 1))
}

pub(crate) fn rate_limit_sentence(seconds: Option<u64>) -> String {
    match seconds {
        Some(1) => "Too many requests. Try again in 1 second.".to_string(),
        Some(seconds) => format!("Too many requests. Try again in {seconds} seconds."),
        None => COPY_RATE_LIMIT_WAIT.to_string(),
    }
}

/// Load failures stay in plain words. The raw client string is not shown.
pub(crate) fn visible_load_error(
    fallback: &str,
    status: Option<u16>,
    body: &str,
    header_seconds: Option<u64>,
) -> String {
    if status == Some(429) {
        rate_limit_sentence(retry_after_seconds(body, header_seconds))
    } else {
        fallback.to_string()
    }
}

pub(crate) fn mutation_failure_copy(
    intent: ReportIntent,
    status: Option<u16>,
    body: &str,
    header_seconds: Option<u64>,
) -> String {
    if status == Some(429) {
        rate_limit_sentence(retry_after_seconds(body, header_seconds))
    } else {
        match intent {
            ReportIntent::Delete => COPY_DELETE_FAILED.to_string(),
            ReportIntent::Withdraw => COPY_WITHDRAW_FAILED.to_string(),
        }
    }
}

pub(crate) fn member_screen(
    auth: &AuthState,
    error: Option<&ListedError>,
    row_count: Option<usize>,
) -> MemberScreen {
    if auth.loading {
        return MemberScreen::Loading;
    }
    if !auth.is_logged_in() {
        return MemberScreen::SignIn;
    }
    if !auth.is_org_member() {
        return MemberScreen::Membership;
    }
    if let Some(err) = error {
        if err.status == Some(401) {
            return MemberScreen::SignIn;
        }
        if err.status == Some(403) {
            return if auth.is_org_member() {
                MemberScreen::Inactive
            } else {
                MemberScreen::Membership
            };
        }
        if is_reports_disabled(err.status, &err.body) {
            return MemberScreen::Disabled;
        }
        return MemberScreen::Error;
    }
    match row_count {
        None => MemberScreen::Loading,
        Some(0) => MemberScreen::Empty,
        Some(_) => MemberScreen::Ready,
    }
}

/// Delete or withdraw answered `reports_disabled` after the list had loaded.
/// Sign-in, membership, and loading still win. A list, an empty list, and a
/// generic error are replaced by the switched-off sentence.
pub(crate) fn apply_reports_switch(screen: MemberScreen, switched_off: bool) -> MemberScreen {
    if switched_off
        && !matches!(
            screen,
            MemberScreen::SignIn
                | MemberScreen::Membership
                | MemberScreen::Inactive
                | MemberScreen::Loading
        )
    {
        MemberScreen::Disabled
    } else {
        screen
    }
}

pub(crate) fn officer_screen(
    auth: &AuthState,
    error: Option<&ListedError>,
    row_count: Option<usize>,
) -> OfficerScreen {
    if auth.loading {
        return OfficerScreen::Checking;
    }
    if let Some(err) = error {
        if err.status == Some(401) {
            return OfficerScreen::SignIn;
        }
        if err.status == Some(403) {
            return OfficerScreen::Forbidden;
        }
    }
    if !auth.is_officer_or_above() {
        return OfficerScreen::Forbidden;
    }
    if let Some(err) = error {
        if is_reports_disabled(err.status, &err.body) {
            return OfficerScreen::Disabled;
        }
        return OfficerScreen::Error;
    }
    match row_count {
        None => OfficerScreen::Loading,
        Some(0) => OfficerScreen::Empty,
        Some(_) => OfficerScreen::Ready,
    }
}

/// A member response has no `member_id`. An officer response lists every
/// report and includes `member_id`, so the member page keeps only that
/// viewer's rows. Without the viewer's id it returns no rows.
pub(crate) fn prepare_member_rows(
    reports: &[StatReportListItem],
    member_id: Option<&str>,
) -> PreparedRows {
    if reports.iter().all(|row| row.member_id.is_none()) {
        return PreparedRows::Rows(reports.to_vec());
    }
    let Some(member_id) = member_id else {
        return PreparedRows::PendingMember;
    };
    PreparedRows::Rows(
        reports
            .iter()
            .filter(|row| row.member_id.as_deref() == Some(member_id))
            .cloned()
            .collect(),
    )
}

pub(crate) fn apply_own_filter(
    screen: MemberScreen,
    prepared: &PreparedRows,
    me_settled: bool,
) -> MemberScreen {
    if !matches!(screen, MemberScreen::Ready | MemberScreen::Empty) {
        return screen;
    }
    match prepared {
        PreparedRows::PendingMember if !me_settled => MemberScreen::Loading,
        PreparedRows::PendingMember => MemberScreen::Error,
        PreparedRows::Rows(rows) if rows.is_empty() => MemberScreen::Empty,
        PreparedRows::Rows(_) => MemberScreen::Ready,
    }
}

fn static_copy() -> &'static [&'static str] {
    &[
        COPY_LOADING_MINE,
        COPY_ERROR_MINE,
        COPY_EMPTY_MINE,
        COPY_SIGN_IN,
        COPY_MEMBERSHIP,
        COPY_INACTIVE,
        COPY_INTRO_MINE,
        COPY_CHECKING,
        COPY_FORBIDDEN,
        COPY_LOADING_ALL,
        COPY_ERROR_ALL,
        COPY_EMPTY_ALL,
        COPY_INTRO_ALL,
        COPY_DELETED,
        COPY_WITHDRAWN,
        COPY_DELETE_FAILED,
        COPY_WITHDRAW_FAILED,
        COPY_TRY_AGAIN,
        COPY_NO_DELETION,
        COPY_DELETION_MISSING,
        COPY_UNUSABLE,
        COPY_DISABLED,
        COPY_RATE_LIMIT_WAIT,
        COPY_OFFICER_SIGN_IN,
        "Too many requests. Try again in 12 seconds.",
        "Too many requests. Try again in 1 second.",
        "My reports",
        "Reports",
        "Kept for training",
        "Scheduled for deletion",
        "Training consent on",
        "Training consent off",
        "Own name included",
        "Own name not included",
        "Wrong glyphs included",
        "Wrong glyphs not included",
        "Delete",
        "Withdraw consent",
        "Download",
        "Cancel",
        "Sign in",
        "Apply",
        "Member id",
        "Member id missing",
        "Date",
        "Size",
        "Status",
        "Training consent",
        "Expiry",
        "Actions",
    ]
}

fn labeled_control(id: &str, label: &str, button: Element) -> Element {
    rsx! {
        span { class: "reports-actions",
            label { r#for: "{id}", class: "report-control-label", "{label}" }
            {button}
        }
    }
}

#[component]
pub(crate) fn MyReportsBody(
    screen: MemberScreen,
    #[props(default)] http_status: Option<u16>,
    #[props(default)] error_body: String,
    #[props(default)] retry_after_seconds: Option<u64>,
    rows: Vec<StatReportListItem>,
    pending: Option<PendingReportAction>,
    busy: bool,
    #[props(default)] on_retry: EventHandler<()>,
    #[props(default)] on_arm_delete: EventHandler<String>,
    #[props(default)] on_arm_withdraw: EventHandler<String>,
    #[props(default)] on_confirm: EventHandler<()>,
    #[props(default)] on_cancel: EventHandler<()>,
) -> Element {
    let show_dialog = screen == MemberScreen::Ready && pending.is_some();
    rsx! {
        style { {REPORTS_CSS} }
        main { class: "reports-page",
            h1 { "My reports" }
            if screen != MemberScreen::Disabled {
                p { class: "reports-intro", "{COPY_INTRO_MINE}" }
            }
            match screen {
                MemberScreen::Loading => rsx! { p { class: "reports-status", "{COPY_LOADING_MINE}" } },
                MemberScreen::SignIn => rsx! {
                    p { class: "reports-status", "{COPY_SIGN_IN}" }
                    a { href: "/login", class: "report-btn", "Sign in" }
                },
                MemberScreen::Membership => rsx! {
                    p { class: "reports-status", "{COPY_MEMBERSHIP}" }
                    a { href: "/apply", class: "report-btn", "Apply" }
                },
                MemberScreen::Inactive => rsx! { p { class: "reports-status", "{COPY_INACTIVE}" } },
                MemberScreen::Error => rsx! {
                    p { class: "reports-status reports-error", "{visible_load_error(COPY_ERROR_MINE, http_status, &error_body, retry_after_seconds)}" }
                    {retry_button(MY_REPORTS_RETRY_ID, on_retry)}
                },
                MemberScreen::Empty => rsx! { p { class: "reports-status", "{COPY_EMPTY_MINE}" } },
                MemberScreen::Disabled => rsx! { p { class: "reports-status", "{COPY_DISABLED}" } },
                MemberScreen::Ready => rsx! {
                    div { class: "reports-scroll",
                        table { class: "reports-table",
                            thead {
                                tr {
                                    th { "Date" }
                                    th { "Size" }
                                    th { "Status" }
                                    th { "Training consent" }
                                    th { "Expiry" }
                                    th { "Actions" }
                                }
                            }
                            tbody {
                                for row in rows.iter() {
                                    {member_row(row, busy, on_arm_delete, on_arm_withdraw)}
                                }
                            }
                        }
                    }
                },
            }
            if show_dialog {
                if let Some(action) = pending {
                    {confirm_dialog(action, busy, on_confirm, on_cancel)}
                }
            }
        }
    }
}

fn member_row(
    row: &StatReportListItem,
    busy: bool,
    on_arm_delete: EventHandler<String>,
    on_arm_withdraw: EventHandler<String>,
) -> Element {
    let when = format_report_when(row.created_at);
    let size = format_report_size(row.size_bytes);
    let status = status_text(row.training_consent);
    let consent = training_consent_text(row.training_consent);
    let extra = extra_consent_text(row.own_name_included, row.glyphs_included);
    let expiry = expiry_text(row.training_consent, row.expires_at);
    let usable = is_report_id(&row.id);
    let delete_id = control_id("delete", &row.id);
    let withdraw_id = control_id("withdraw", &row.id);
    let delete_label = format!("Delete report {}", row.id);
    let withdraw_label = format!("Withdraw training consent for report {}", row.id);
    let delete_report_id = row.id.clone();
    let withdraw_report_id = row.id.clone();
    let training = row.training_consent;
    rsx! {
        tr {
            td { "{when}" }
            td { "{size}" }
            td { "{status}" }
            td {
                "{consent}"
                span { class: "reports-sub", "{extra}" }
            }
            td { "{expiry}" }
            td {
                if usable {
                    div { class: "reports-actions",
                        {labeled_control(&delete_id, &delete_label, rsx! {
                            button {
                                r#type: "button",
                                id: "{delete_id}",
                                name: "delete-report",
                                class: "report-btn report-btn-danger",
                                disabled: busy,
                                onclick: move |_| on_arm_delete.call(delete_report_id.clone()),
                                "Delete"
                            }
                        })}
                        if training {
                            {labeled_control(&withdraw_id, &withdraw_label, rsx! {
                                button {
                                    r#type: "button",
                                    id: "{withdraw_id}",
                                    name: "withdraw-training",
                                    class: "report-btn",
                                    disabled: busy,
                                    onclick: move |_| on_arm_withdraw.call(withdraw_report_id.clone()),
                                    "Withdraw consent"
                                }
                            })}
                        }
                    }
                } else {
                    span { "{COPY_UNUSABLE}" }
                }
            }
        }
    }
}

fn confirm_dialog(
    action: PendingReportAction,
    busy: bool,
    on_confirm: EventHandler<()>,
    on_cancel: EventHandler<()>,
) -> Element {
    let (title, body, submit_label) = confirm_copy(action.intent);
    let danger = action.intent == ReportIntent::Delete;
    let submit_class = if danger {
        "report-btn report-btn-danger"
    } else {
        "report-btn"
    };
    rsx! {
        div {
            class: "report-dialog-backdrop",
            role: "dialog",
            aria_modal: "true",
            aria_labelledby: "report-confirm-title",
            onclick: move |_| on_cancel.call(()),
            div {
                id: CONFIRM_DIALOG_ID,
                tabindex: "-1",
                class: "report-dialog",
                onclick: move |evt| evt.stop_propagation(),
                h2 { id: "report-confirm-title", "{title}" }
                p { id: "report-confirm-message", "{body}" }
                form {
                    id: CONFIRM_FORM_ID,
                    onsubmit: move |evt| {
                        evt.prevent_default();
                        on_confirm.call(());
                    },
                    div { class: "report-dialog-actions",
                        label { r#for: CONFIRM_CANCEL_ID, class: "report-control-label", "Cancel" }
                        button {
                            r#type: "button",
                            id: CONFIRM_CANCEL_ID,
                            name: "cancel",
                            class: "report-btn",
                            disabled: busy,
                            onclick: move |_| on_cancel.call(()),
                            "Cancel"
                        }
                        label { r#for: CONFIRM_SUBMIT_ID, class: "report-control-label", "{submit_label}" }
                        button {
                            r#type: "submit",
                            id: CONFIRM_SUBMIT_ID,
                            name: "confirm",
                            class: "{submit_class}",
                            disabled: busy,
                            "{submit_label}"
                        }
                    }
                }
            }
        }
    }
}

fn retry_button(id: &'static str, on_retry: EventHandler<()>) -> Element {
    rsx! {
        label { r#for: "{id}", class: "report-control-label", "{COPY_TRY_AGAIN}" }
        button {
            r#type: "button",
            id: "{id}",
            name: "retry",
            class: "report-btn",
            onclick: move |_| on_retry.call(()),
            "{COPY_TRY_AGAIN}"
        }
    }
}

#[component]
pub(crate) fn OfficerReportsBody(
    screen: OfficerScreen,
    #[props(default)] http_status: Option<u16>,
    #[props(default)] error_body: String,
    #[props(default)] retry_after_seconds: Option<u64>,
    rows: Vec<StatReportListItem>,
    #[props(default)] on_retry: EventHandler<()>,
) -> Element {
    rsx! {
        style { {REPORTS_CSS} }
        match screen {
            OfficerScreen::Checking => rsx! { p { class: "reports-status", "{COPY_CHECKING}" } },
            OfficerScreen::Forbidden => rsx! {
                AccessDenied { message: COPY_FORBIDDEN.to_string() }
            },
            OfficerScreen::Loading => rsx! {
                div { class: "reports-page",
                    h1 { "Reports" }
                    p { class: "reports-status", "{COPY_LOADING_ALL}" }
                }
            },
            OfficerScreen::Error => rsx! {
                div { class: "reports-page",
                    h1 { "Reports" }
                    p { class: "reports-intro", "{COPY_INTRO_ALL}" }
                    p { class: "reports-status reports-error", "{visible_load_error(COPY_ERROR_ALL, http_status, &error_body, retry_after_seconds)}" }
                    {retry_button(OFFICER_REPORTS_RETRY_ID, on_retry)}
                }
            },
            OfficerScreen::SignIn => rsx! {
                div { class: "reports-page",
                    h1 { "Reports" }
                    p { class: "reports-status", "{COPY_OFFICER_SIGN_IN}" }
                    a {
                        href: "{login_return_href(OFFICER_REPORTS_PATH)}",
                        class: "report-btn",
                        onclick: move |_| remember_login_return(OFFICER_REPORTS_PATH),
                        "Sign in"
                    }
                }
            },
            OfficerScreen::Empty => rsx! {
                div { class: "reports-page",
                    h1 { "Reports" }
                    p { class: "reports-intro", "{COPY_INTRO_ALL}" }
                    p { class: "reports-status", "{COPY_EMPTY_ALL}" }
                }
            },
            OfficerScreen::Disabled => rsx! {
                div { class: "reports-page",
                    h1 { "Reports" }
                    p { class: "reports-status", "{COPY_DISABLED}" }
                }
            },
            OfficerScreen::Ready => rsx! {
                div { class: "reports-page",
                    h1 { "Reports" }
                    p { class: "reports-intro", "{COPY_INTRO_ALL}" }
                    div { class: "reports-scroll",
                        table { class: "reports-table",
                            thead {
                                tr {
                                    th { "Member id" }
                                    th { "Date" }
                                    th { "Size" }
                                    th { "Training consent" }
                                    th { "Download" }
                                }
                            }
                            tbody {
                                for row in rows.iter() {
                                    {officer_row(row)}
                                }
                            }
                        }
                    }
                }
            },
        }
    }
}

fn officer_row(row: &StatReportListItem) -> Element {
    let when = format_report_when(row.created_at);
    let size = format_report_size(row.size_bytes);
    let consent = training_consent_text(row.training_consent);
    let extra = extra_consent_text(row.own_name_included, row.glyphs_included);
    let member = row
        .member_id
        .clone()
        .filter(|id| !id.trim().is_empty())
        .unwrap_or_else(|| "Member id missing".into());
    let download = download_action(&row.id);
    let download_id = control_id("download", &row.id);
    let download_label = format!("Download report {}", row.id);
    rsx! {
        tr {
            td { code { class: "report-member-id", "{member}" } }
            td { "{when}" }
            td { "{size}" }
            td {
                "{consent}"
                span { class: "reports-sub", "{extra}" }
            }
            td {
                if let Some(action) = download {
                    form {
                        method: "get",
                        action: "{action}",
                        label { r#for: "{download_id}", class: "report-control-label", "{download_label}" }
                        button {
                            r#type: "submit",
                            id: "{download_id}",
                            name: "download-report",
                            value: "1",
                            class: "report-btn",
                            "Download"
                        }
                    }
                } else {
                    span { "{COPY_UNUSABLE}" }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use dioxus::dioxus_core::NoOpMutations;
    use scuffed_api_client::ClientError;
    use scuffed_types::{OrgRole, UserInfo};

    const REPORT_A: &str = "0123456789abcdef0123456789abcdef";
    const REPORT_B: &str = "fedcba9876543210fedcba9876543210";

    fn auth(role: Option<OrgRole>, loading: bool) -> AuthState {
        AuthState {
            user: role.map(|role| UserInfo {
                id: "user-1".into(),
                username: "account".into(),
                avatar_url: None,
                role: Some(role),
            }),
            loading,
        }
    }

    fn at(y: i32, m: u32, d: u32, hh: u32, mm: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, hh, mm, 0).unwrap()
    }

    fn listed(status: u16, body: &str) -> ListedError {
        ListedError {
            status: Some(status),
            body: body.to_string(),
            header_seconds: None,
        }
    }

    fn row(id: &str, training: bool, expires: Option<DateTime<Utc>>) -> StatReportListItem {
        StatReportListItem {
            id: id.into(),
            created_at: at(2026, 10, 9, 12, 0),
            expires_at: expires,
            training_consent: training,
            own_name_included: false,
            glyphs_included: false,
            size_bytes: 2048,
            member_id: None,
            reason_category: None,
            reason_text: None,
            app_version: None,
            recognizer_matcher: None,
            recognizer_ocr: None,
            zip_sha256: None,
        }
    }

    fn pump(dom: &mut VirtualDom) {
        for _ in 0..8 {
            dom.render_immediate(&mut NoOpMutations);
        }
    }

    fn html_of(view: fn() -> Element) -> String {
        let mut dom = VirtualDom::new(view);
        dom.rebuild_in_place();
        pump(&mut dom);
        dioxus_ssr::render(&dom)
    }

    fn assert_plain(text: &str) {
        assert!(!text.contains('\u{2014}'), "em dash in {text}");
        assert!(!text.contains('\u{2013}'), "en dash in {text}");
    }

    fn assert_labeled(html: &str, id: &str, name: &str) {
        assert!(html.contains(&format!("id=\"{id}\"")), "{html}");
        assert!(html.contains(&format!("name=\"{name}\"")), "{html}");
        assert!(html.contains(&format!("for=\"{id}\"")), "{html}");
    }

    #[test]
    fn copy_has_no_em_or_en_dash() {
        for line in static_copy() {
            assert_plain(line);
        }
        let (title, body, button) = confirm_copy(ReportIntent::Delete);
        assert_plain(title);
        assert_plain(body);
        assert_plain(button);
        let (title, body, button) = confirm_copy(ReportIntent::Withdraw);
        assert_plain(title);
        assert_plain(body);
        assert_plain(button);
    }

    #[test]
    fn officer_gate_matches_admin_layout() {
        let roles = [
            None,
            Some(OrgRole::Recruit),
            Some(OrgRole::Member),
            Some(OrgRole::Officer),
            Some(OrgRole::Admin),
        ];
        for role in roles {
            let state = auth(role, false);
            let open = officer_screen(&state, None, Some(1)) == OfficerScreen::Ready;
            assert_eq!(open, state.is_officer_or_above());
        }
        assert_eq!(
            officer_screen(&auth(Some(OrgRole::Member), false), None, Some(1)),
            OfficerScreen::Forbidden
        );
        assert_eq!(
            officer_screen(&auth(Some(OrgRole::Recruit), false), None, Some(0)),
            OfficerScreen::Forbidden
        );
        assert_eq!(
            officer_screen(&auth(None, false), None, Some(1)),
            OfficerScreen::Forbidden
        );
        assert_eq!(
            officer_screen(&auth(Some(OrgRole::Admin), true), None, Some(1)),
            OfficerScreen::Checking
        );
        assert_eq!(
            officer_screen(&auth(Some(OrgRole::Officer), false), None, None),
            OfficerScreen::Loading
        );
        assert_eq!(
            officer_screen(&auth(Some(OrgRole::Officer), false), None, Some(0)),
            OfficerScreen::Empty
        );
        assert_eq!(
            officer_screen(
                &auth(Some(OrgRole::Officer), false),
                Some(&listed(500, "HTTP error 500: Internal error")),
                None
            ),
            OfficerScreen::Error
        );
    }

    #[test]
    fn my_reports_requires_a_signed_in_member() {
        assert_eq!(
            member_screen(&AuthState::new(), None, None),
            MemberScreen::Loading
        );
        assert_eq!(
            member_screen(&auth(None, false), None, Some(1)),
            MemberScreen::SignIn
        );
        let signed_out = AuthState {
            user: Some(UserInfo {
                id: "user-1".into(),
                username: "account".into(),
                avatar_url: None,
                role: None,
            }),
            loading: false,
        };
        assert!(signed_out.is_logged_in());
        assert!(!signed_out.is_org_member());
        assert_eq!(
            member_screen(&signed_out, None, Some(1)),
            MemberScreen::Membership
        );
        for role in [
            OrgRole::Recruit,
            OrgRole::Member,
            OrgRole::Officer,
            OrgRole::Admin,
        ] {
            assert_eq!(
                member_screen(&auth(Some(role), false), None, Some(2)),
                MemberScreen::Ready,
                "{role:?}"
            );
        }
        let member = auth(Some(OrgRole::Member), false);
        assert_eq!(
            member_screen(
                &member,
                Some(&listed(401, "HTTP error 401: Unauthorized")),
                None
            ),
            MemberScreen::SignIn
        );
        assert_eq!(
            member_screen(&member, Some(&listed(403, "Forbidden")), None),
            MemberScreen::Inactive
        );
        assert_eq!(
            member_screen(
                &member,
                Some(&listed(500, "HTTP error 500: Internal error")),
                None
            ),
            MemberScreen::Error
        );
        assert_eq!(member_screen(&member, None, None), MemberScreen::Loading);
        assert_eq!(member_screen(&member, None, Some(0)), MemberScreen::Empty);
    }

    #[test]
    fn expiry_text_names_a_deletion_date_only_without_training_consent() {
        let when = at(2026, 11, 8, 15, 30);
        let scheduled = expiry_text(false, Some(when));
        assert_eq!(scheduled, "Deleted on 2026-11-08 15:30 UTC");
        assert_plain(&scheduled);
        let kept = expiry_text(true, Some(when));
        assert_eq!(kept, "No deletion date");
        assert!(!kept.contains("Deleted on"));
        let missing = expiry_text(false, None);
        assert_eq!(missing, "Deletion date missing");
        assert!(!missing.contains("Deleted on"));
        assert_eq!(training_consent_text(true), "Training consent on");
        assert_eq!(training_consent_text(false), "Training consent off");
        assert_eq!(status_text(true), "Kept for training");
        assert_eq!(status_text(false), "Scheduled for deletion");
        assert_eq!(
            extra_consent_text(false, false),
            "Own name not included. Wrong glyphs not included."
        );
    }

    #[test]
    fn delete_is_not_sent_until_confirm() {
        let mut gate = ConfirmGate::default();
        gate.arm(ReportIntent::Delete, REPORT_A);
        assert_eq!(
            gate.pending().map(|pending| pending.intent),
            Some(ReportIntent::Delete)
        );
        assert!(gate.confirm().is_some_and(|mutation| {
            mutation
                == ReportMutation {
                    intent: ReportIntent::Delete,
                    path: format!("/api/stat-reports/{REPORT_A}"),
                }
        }));
        assert!(gate.confirm().is_none());

        gate.arm(ReportIntent::Delete, REPORT_A);
        gate.cancel();
        assert!(gate.pending().is_none());
        assert!(gate.confirm().is_none());

        gate.arm(ReportIntent::Delete, "not-a-report-id");
        assert!(gate.pending().is_none());
        assert!(mutation_for(ReportIntent::Delete, "not-a-report-id").is_none());

        gate.arm(ReportIntent::Withdraw, REPORT_B);
        let mutation = gate.confirm().unwrap();
        assert_eq!(mutation.intent, ReportIntent::Withdraw);
        assert_eq!(
            mutation.path,
            format!("/api/stat-reports/{REPORT_B}/withdraw")
        );
        assert_eq!(
            download_action(REPORT_A).as_deref(),
            Some(format!("/api/stat-reports/{REPORT_A}").as_str())
        );
        let body = serde_json::to_value(WithdrawTrainingBody::clear()).unwrap();
        assert_eq!(body, serde_json::json!({"training": false}));
    }

    #[test]
    fn member_page_hides_other_members_when_the_list_is_an_officer_payload() {
        let mut mine = row(REPORT_A, false, Some(at(2026, 11, 8, 12, 0)));
        mine.member_id = Some("m-100".into());
        mine.reason_text = Some("QXNAME".into());
        let mut other = row(REPORT_B, true, None);
        other.member_id = Some("m-200".into());
        other.reason_text = Some("QXNAME".into());
        match prepare_member_rows(&[mine.clone(), other.clone()], Some("m-100")) {
            PreparedRows::Rows(rows) => {
                assert_eq!(rows.len(), 1);
                assert_eq!(rows[0].id, REPORT_A);
            }
            PreparedRows::PendingMember => panic!("viewer id was known"),
        }
        assert!(matches!(
            prepare_member_rows(&[mine, other], None),
            PreparedRows::PendingMember
        ));
        let untagged = row(REPORT_A, false, None);
        match prepare_member_rows(std::slice::from_ref(&untagged), None) {
            PreparedRows::Rows(rows) => assert_eq!(rows.len(), 1),
            PreparedRows::PendingMember => panic!("member payload has no member id"),
        }
        assert_eq!(
            apply_own_filter(MemberScreen::Ready, &PreparedRows::PendingMember, false),
            MemberScreen::Loading
        );
        assert_eq!(
            apply_own_filter(MemberScreen::Ready, &PreparedRows::PendingMember, true),
            MemberScreen::Error
        );
        assert_eq!(
            apply_own_filter(
                MemberScreen::SignIn,
                &PreparedRows::Rows(vec![untagged]),
                true
            ),
            MemberScreen::SignIn
        );
    }

    #[test]
    fn member_payload_parses_without_officer_fields() {
        let parsed: StatReportList = serde_json::from_str(
            r#"{
                "reports": [{
                    "id": "0123456789abcdef0123456789abcdef",
                    "created_at": "2026-10-09T12:00:00Z",
                    "expires_at": "2026-11-08T12:00:00Z",
                    "training_consent": false,
                    "own_name_included": false,
                    "glyphs_included": false,
                    "size_bytes": 2048
                }]
            }"#,
        )
        .unwrap();
        assert!(parsed.reports[0].member_id.is_none());
        assert!(parsed.reports[0].reason_text.is_none());
        assert_eq!(
            expiry_text(false, parsed.reports[0].expires_at),
            "Deleted on 2026-11-08 12:00 UTC"
        );
    }

    #[test]
    fn training_row_does_not_say_deleted_on() {
        fn view() -> Element {
            rsx! {
                MyReportsBody {
                    screen: MemberScreen::Ready,
                    rows: vec![row(REPORT_A, true, Some(at(2026, 11, 8, 15, 30)))],
                    pending: None,
                    busy: false,
                }
            }
        }
        let html = html_of(view);
        assert!(html.contains("Kept for training"), "{html}");
        assert!(html.contains("Training consent on"), "{html}");
        assert!(html.contains("No deletion date"), "{html}");
        assert!(!html.contains("Deleted on"), "{html}");
        assert!(html.contains("Withdraw consent"), "{html}");
        assert_plain(&html);
        assert_labeled(
            &html,
            &control_id("withdraw", REPORT_A),
            "withdraw-training",
        );
    }

    #[test]
    fn expiry_row_says_deleted_on_and_delete_waits_for_the_dialog() {
        fn closed() -> Element {
            let mut item = row(REPORT_A, false, Some(at(2026, 11, 8, 15, 30)));
            item.reason_text = Some("QXNAME".into());
            item.reason_category = Some("wrong_stats".into());
            rsx! {
                MyReportsBody {
                    screen: MemberScreen::Ready,
                    rows: vec![item],
                    pending: None,
                    busy: false,
                }
            }
        }
        let html = html_of(closed);
        assert!(html.contains("Deleted on 2026-11-08 15:30 UTC"), "{html}");
        assert!(html.contains("Scheduled for deletion"), "{html}");
        assert!(html.contains("Training consent off"), "{html}");
        assert!(!html.contains("QXNAME"), "{html}");
        assert!(!html.contains("wrong_stats"), "{html}");
        assert!(!html.contains(CONFIRM_FORM_ID), "{html}");
        assert!(!html.contains("/withdraw"), "{html}");
        assert!(html.contains("type=\"button\""), "{html}");
        assert!(!html.contains("Withdraw consent"), "{html}");
        assert_labeled(&html, &control_id("delete", REPORT_A), "delete-report");
        assert_plain(&html);

        fn open() -> Element {
            rsx! {
                MyReportsBody {
                    screen: MemberScreen::Ready,
                    rows: vec![row(REPORT_A, false, Some(at(2026, 11, 8, 15, 30)))],
                    pending: Some(PendingReportAction {
                        intent: ReportIntent::Delete,
                        report_id: REPORT_A.into(),
                        return_focus_id: opener_focus_id(ReportIntent::Delete, REPORT_A),
                    }),
                    busy: false,
                }
            }
        }
        let html = html_of(open);
        assert!(html.contains(CONFIRM_FORM_ID), "{html}");
        assert!(
            html.contains("class=\"report-btn report-btn-danger\""),
            "{html}"
        );
        assert!(
            html.contains("Delete this report now? The file is removed and cannot be restored."),
            "{html}"
        );
        assert!(!html.contains("action=\"/api/stat-reports"), "{html}");
        assert_labeled(&html, CONFIRM_SUBMIT_ID, "confirm");
        assert_labeled(&html, CONFIRM_CANCEL_ID, "cancel");
        assert_plain(&html);
    }

    #[test]
    fn member_states_use_plain_words() {
        fn loading() -> Element {
            rsx! {
                MyReportsBody {
                    screen: MemberScreen::Loading,
                    rows: Vec::new(),
                    pending: None,
                    busy: false,
                }
            }
        }
        fn empty() -> Element {
            rsx! {
                MyReportsBody {
                    screen: MemberScreen::Empty,
                    rows: Vec::new(),
                    pending: None,
                    busy: false,
                }
            }
        }
        fn failed() -> Element {
            rsx! {
                MyReportsBody {
                    screen: MemberScreen::Error,
                    http_status: Some(500),
                    rows: Vec::new(),
                    pending: None,
                    busy: false,
                }
            }
        }
        fn signed_out() -> Element {
            rsx! {
                MyReportsBody {
                    screen: MemberScreen::SignIn,
                    rows: Vec::new(),
                    pending: None,
                    busy: false,
                }
            }
        }
        let loading_html = html_of(loading);
        assert!(loading_html.contains(COPY_LOADING_MINE), "{loading_html}");
        assert!(!loading_html.contains(COPY_EMPTY_MINE), "{loading_html}");
        let empty_html = html_of(empty);
        assert!(empty_html.contains(COPY_EMPTY_MINE), "{empty_html}");
        let failed_html = html_of(failed);
        assert!(failed_html.contains(COPY_ERROR_MINE), "{failed_html}");
        assert!(
            !failed_html.contains("HTTP error 500: Internal error"),
            "{failed_html}"
        );
        assert!(!failed_html.contains("HTTP error"), "{failed_html}");
        assert_labeled(&failed_html, MY_REPORTS_RETRY_ID, "retry");
        let signed_out_html = html_of(signed_out);
        assert!(signed_out_html.contains(COPY_SIGN_IN), "{signed_out_html}");
        assert!(
            signed_out_html.contains("href=\"/login\""),
            "{signed_out_html}"
        );
        for html in [&loading_html, &empty_html, &failed_html, &signed_out_html] {
            assert_plain(html);
        }
    }

    #[test]
    fn officer_list_shows_member_id_and_download_not_report_contents() {
        fn view() -> Element {
            let mut item = row(REPORT_A, true, None);
            item.member_id = Some("m-100".into());
            item.reason_text = Some("QXNAME".into());
            item.reason_category = Some("wrong_stats".into());
            item.zip_sha256 = Some("abc".into());
            item.app_version = Some("9.9.9".into());
            rsx! {
                OfficerReportsBody {
                    screen: OfficerScreen::Ready,
                    rows: vec![item],
                }
            }
        }
        let html = html_of(view);
        assert!(html.contains("m-100"), "{html}");
        assert!(html.contains("Member id"), "{html}");
        assert!(html.contains("Download"), "{html}");
        assert!(
            html.contains(&format!("action=\"/api/stat-reports/{REPORT_A}\"")),
            "{html}"
        );
        assert!(html.contains("method=\"get\""), "{html}");
        assert!(!html.contains("QXNAME"), "{html}");
        assert!(!html.contains("wrong_stats"), "{html}");
        assert!(!html.contains("9.9.9"), "{html}");
        assert!(!html.contains(">abc<"), "{html}");
        assert_labeled(&html, &control_id("download", REPORT_A), "download-report");
        assert_plain(&html);
        assert_eq!(
            COPY_FORBIDDEN,
            "You need officer permissions to access the admin panel."
        );
    }

    #[test]
    fn load_errors_stay_plain_and_rate_limits_name_the_wait() {
        let raw = "HTTP error 500: Internal error";
        let mine = visible_load_error(COPY_ERROR_MINE, Some(500), raw, None);
        assert_eq!(mine, COPY_ERROR_MINE);
        assert!(!mine.contains("HTTP error"));
        assert!(!mine.contains("Internal error"));
        let officer = visible_load_error(COPY_ERROR_ALL, Some(500), r#"{"error":"boom"}"#, None);
        assert_eq!(officer, COPY_ERROR_ALL);
        assert!(!officer.contains("boom"));

        let json = r#"{"error":"rate_limited","retry_after":12}"#;
        let limited = ClientError::http(429, json, Some("99"));
        assert_eq!(limited.http_status(), Some(429));
        assert_eq!(limited.to_string(), "Try again in 12 s");
        assert!(!limited.to_string().contains("HTTP error"));
        let from_json = visible_load_error(
            COPY_ERROR_MINE,
            limited.http_status(),
            limited.http_body().unwrap_or(""),
            limited.retry_after_header(),
        );
        assert_eq!(from_json, "Too many requests. Try again in 12 seconds.");
        let from_header = visible_load_error(COPY_ERROR_ALL, Some(429), "slow down", Some(8));
        assert_eq!(from_header, "Too many requests. Try again in 8 seconds.");
        assert!(!from_header.contains("slow down"));
        let missing = visible_load_error(COPY_ERROR_MINE, Some(429), "slow down", None);
        assert_eq!(missing, "Too many requests. Try again later.");
        assert_eq!(missing, COPY_RATE_LIMIT_WAIT);
        assert!(!missing.contains("slow down"));
        assert!(!missing.contains("HTTP error"));
        let one = visible_load_error(
            COPY_ERROR_MINE,
            Some(429),
            r#"{"error":"rate_limited","retry_after":"1"}"#,
            Some(99),
        );
        assert_eq!(one, "Too many requests. Try again in 1 second.");
        assert_eq!(
            scuffed_types::json_retry_after(r#"{"error":"rate_limited","retry_after":3601}"#),
            None
        );
        assert_eq!(
            scuffed_types::json_retry_after(r#"{"error":"rate_limited","retry_after":-3}"#),
            None
        );
        assert_eq!(
            scuffed_types::json_retry_after(r#"{"error":"rate_limited","retry_after":[]}"#),
            None
        );
        assert_eq!(
            scuffed_types::json_retry_after(r#"{"error":"rate_limited","retry_after":null}"#),
            None
        );
        assert_eq!(
            retry_after_seconds(r#"{"error":"rate_limited","retry_after":3601}"#, Some(9)),
            Some(9)
        );
        let over_hour = visible_load_error(
            COPY_ERROR_MINE,
            Some(429),
            r#"{"error":"rate_limited","retry_after":3601}"#,
            None,
        );
        assert_eq!(over_hour, COPY_RATE_LIMIT_WAIT);
        let header_only = retry_after_seconds("not json", Some(4));
        assert_eq!(header_only, Some(4));
        assert_eq!(
            retry_after_seconds(r#"{"error":"rate_limited"}"#, Some(4)),
            Some(4)
        );
        assert_eq!(
            mutation_failure_copy(
                ReportIntent::Delete,
                Some(500),
                r#"{"error":"Internal error"}"#,
                None
            ),
            COPY_DELETE_FAILED
        );
        assert_eq!(
            mutation_failure_copy(ReportIntent::Withdraw, Some(429), json, None),
            "Too many requests. Try again in 12 seconds."
        );
        assert_plain(&from_json);
        assert_plain(&missing);

        fn limited() -> Element {
            rsx! {
                MyReportsBody {
                    screen: MemberScreen::Error,
                    http_status: Some(429),
                    error_body: String::from(r#"{"error":"rate_limited","retry_after":12}"#),
                    retry_after_seconds: Some(99),
                    rows: vec![row(REPORT_A, false, None)],
                    pending: None,
                    busy: false,
                }
            }
        }
        let html = html_of(limited);
        assert!(
            html.contains("Too many requests. Try again in 12 seconds."),
            "{html}"
        );
        assert!(!html.contains(REPORT_A), "{html}");
        assert!(!html.contains(COPY_EMPTY_MINE), "{html}");
        assert!(!html.contains("rate_limited"), "{html}");
        assert!(!html.contains("<table"), "{html}");
        assert_plain(&html);
    }

    #[test]
    fn officer_401_goes_to_sign_in_and_403_is_no_access() {
        let officer = auth(Some(OrgRole::Officer), false);
        let expired = listed(401, "HTTP error 401: Unauthorized");
        let denied = listed(403, "HTTP error 403: Forbidden");
        assert_eq!(
            officer_screen(&officer, Some(&expired), Some(2)),
            OfficerScreen::SignIn
        );
        assert_eq!(
            officer_screen(&officer, Some(&denied), Some(0)),
            OfficerScreen::Forbidden
        );
        assert_eq!(
            officer_screen(&auth(Some(OrgRole::Member), false), Some(&denied), Some(1)),
            OfficerScreen::Forbidden
        );
        assert_eq!(
            officer_screen(&auth(None, false), None, Some(1)),
            OfficerScreen::Forbidden
        );
        assert_eq!(
            login_return_href(OFFICER_REPORTS_PATH),
            "/login?return=/admin/reports"
        );
        assert_eq!(
            super::super::login::safe_return_route(OFFICER_REPORTS_PATH),
            Some(crate::routes::Route::AdminReports {})
        );
        assert_eq!(
            super::super::login::route_after_sign_in(
                true,
                Some(crate::routes::Route::AdminReports {})
            ),
            crate::routes::Route::AdminReports {}
        );
        assert_eq!(
            super::super::login::route_after_sign_in(true, None),
            crate::routes::Route::Home {}
        );
        assert!(
            super::super::login::safe_return_route("https://evil.example/admin/reports").is_none()
        );
        assert!(super::super::login::safe_return_route("//evil.example").is_none());

        fn signed_out() -> Element {
            rsx! {
                OfficerReportsBody {
                    screen: OfficerScreen::SignIn,
                    http_status: Some(401),
                    rows: vec![row(REPORT_A, false, None)],
                }
            }
        }
        let html = html_of(signed_out);
        assert!(html.contains(COPY_OFFICER_SIGN_IN), "{html}");
        assert!(
            html.contains("href=\"/login?return=/admin/reports\""),
            "{html}"
        );
        assert!(html.contains("Sign in"), "{html}");
        assert!(!html.contains(REPORT_A), "{html}");
        assert!(!html.contains("HTTP error"), "{html}");
        assert!(!html.contains(COPY_EMPTY_ALL), "{html}");
        assert_plain(&html);
        assert_eq!(
            COPY_FORBIDDEN,
            "You need officer permissions to access the admin panel."
        );
    }

    #[test]
    fn confirm_dialog_focus_returns_to_the_row_button() {
        let mut gate = ConfirmGate::default();
        gate.arm(ReportIntent::Delete, REPORT_A);
        let pending = gate.pending().unwrap();
        assert_eq!(
            pending.return_focus_id,
            opener_focus_id(ReportIntent::Delete, REPORT_A)
        );
        assert_eq!(CONFIRM_DIALOG_ID, "report-confirm-dialog");
        assert!(confirm_escape_closes("Escape", true, false));
        assert!(!confirm_escape_closes("Escape", true, true));
        assert!(!confirm_escape_closes("Enter", true, false));
        assert!(!confirm_escape_closes("Escape", false, false));
        let return_id = gate.pending().unwrap().return_focus_id.clone();
        gate.cancel();
        assert!(gate.pending().is_none());
        assert_eq!(return_id, control_id("delete", REPORT_A));

        gate.arm(ReportIntent::Withdraw, REPORT_B);
        assert_eq!(
            gate.pending().unwrap().return_focus_id,
            control_id("withdraw", REPORT_B)
        );

        fn open() -> Element {
            rsx! {
                MyReportsBody {
                    screen: MemberScreen::Ready,
                    rows: vec![row(REPORT_A, true, None)],
                    pending: Some(PendingReportAction {
                        intent: ReportIntent::Delete,
                        report_id: REPORT_A.into(),
                        return_focus_id: opener_focus_id(ReportIntent::Delete, REPORT_A),
                    }),
                    busy: false,
                }
            }
        }
        let html = html_of(open);
        assert!(html.contains("id=\"report-confirm-dialog\""), "{html}");
        assert!(html.contains("tabindex=\"-1\""), "{html}");
        assert!(
            html.contains(&format!("id=\"{}\"", control_id("delete", REPORT_A))),
            "{html}"
        );
        assert_plain(&html);
    }

    fn reports_disabled_error() -> ListedError {
        ListedError::from_client(&ClientError::http(
            503,
            r#"{"error":"reports_disabled"}"#,
            None,
        ))
    }

    #[test]
    fn reports_disabled_replaces_the_list_and_the_empty_state() {
        assert_eq!(COPY_DISABLED, "Reports are off right now.");
        let off = reports_disabled_error();
        assert_eq!(off.status, Some(503));
        assert!(is_reports_disabled(off.status, &off.body));
        let wrong_status = ClientError::http(500, r#"{"error":"reports_disabled"}"#, None);
        assert!(!is_reports_disabled(
            wrong_status.http_status(),
            wrong_status.http_body().unwrap_or("")
        ));
        let other = ClientError::http(503, r#"{"error":"unavailable"}"#, None);
        assert!(!is_reports_disabled(
            other.http_status(),
            other.http_body().unwrap_or("")
        ));
        let plain = ClientError::http(503, "offline", None);
        assert!(!is_reports_disabled(
            plain.http_status(),
            plain.http_body().unwrap_or("")
        ));

        let member = auth(Some(OrgRole::Member), false);
        assert_eq!(
            member_screen(&member, Some(&off), Some(0)),
            MemberScreen::Disabled
        );
        assert_eq!(
            member_screen(&member, Some(&off), Some(3)),
            MemberScreen::Disabled
        );
        assert_eq!(
            member_screen(&member, Some(&off), None),
            MemberScreen::Disabled
        );
        assert_eq!(
            member_screen(&auth(None, false), Some(&off), Some(0)),
            MemberScreen::SignIn
        );
        let signed_out = AuthState {
            user: Some(UserInfo {
                id: "user-1".into(),
                username: "account".into(),
                avatar_url: None,
                role: None,
            }),
            loading: false,
        };
        assert_eq!(
            member_screen(&signed_out, Some(&off), Some(1)),
            MemberScreen::Membership
        );

        let officer = auth(Some(OrgRole::Officer), false);
        assert_eq!(
            officer_screen(&officer, Some(&off), Some(0)),
            OfficerScreen::Disabled
        );
        assert_eq!(
            officer_screen(&officer, Some(&off), Some(4)),
            OfficerScreen::Disabled
        );
        assert_eq!(
            officer_screen(&auth(Some(OrgRole::Member), false), Some(&off), Some(2)),
            OfficerScreen::Forbidden
        );
        assert_eq!(
            officer_screen(&auth(Some(OrgRole::Admin), true), Some(&off), Some(1)),
            OfficerScreen::Checking
        );

        assert_eq!(
            apply_reports_switch(MemberScreen::Ready, true),
            MemberScreen::Disabled
        );
        assert_eq!(
            apply_reports_switch(MemberScreen::Empty, true),
            MemberScreen::Disabled
        );
        assert_eq!(
            apply_reports_switch(MemberScreen::Error, true),
            MemberScreen::Disabled
        );
        assert_eq!(
            apply_reports_switch(MemberScreen::SignIn, true),
            MemberScreen::SignIn
        );
        assert_eq!(
            apply_reports_switch(MemberScreen::Ready, false),
            MemberScreen::Ready
        );

        fn member_off() -> Element {
            rsx! {
                MyReportsBody {
                    screen: MemberScreen::Disabled,
                    http_status: Some(503),
                    rows: vec![row(REPORT_A, false, Some(at(2026, 11, 8, 15, 30)))],
                    pending: None,
                    busy: false,
                }
            }
        }
        let html = html_of(member_off);
        assert!(html.contains(COPY_DISABLED), "{html}");
        assert!(!html.contains(COPY_EMPTY_MINE), "{html}");
        assert!(!html.contains(COPY_INTRO_MINE), "{html}");
        assert!(!html.contains(REPORT_A), "{html}");
        assert!(!html.contains("<table"), "{html}");
        assert!(!html.contains("reports_disabled"), "{html}");
        assert_plain(&html);

        fn officer_off() -> Element {
            let mut item = row(REPORT_A, false, Some(at(2026, 11, 8, 15, 30)));
            item.member_id = Some("m-100".into());
            rsx! {
                OfficerReportsBody {
                    screen: OfficerScreen::Disabled,
                    http_status: Some(503),
                    rows: vec![item],
                }
            }
        }
        let html = html_of(officer_off);
        assert!(html.contains(COPY_DISABLED), "{html}");
        assert!(!html.contains(COPY_EMPTY_ALL), "{html}");
        assert!(!html.contains(COPY_INTRO_ALL), "{html}");
        assert!(!html.contains(REPORT_A), "{html}");
        assert!(!html.contains("m-100"), "{html}");
        assert!(!html.contains("Download"), "{html}");
        assert!(!html.contains("<table"), "{html}");
        assert!(!html.contains("reports_disabled"), "{html}");
        assert_plain(&html);
    }

    #[test]
    fn past_expiry_is_hidden_unless_training_consent() {
        let now = at(2026, 10, 9, 12, 0);
        let past = row(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            false,
            Some(at(2026, 10, 8, 12, 0)),
        );
        let future = row(REPORT_A, false, Some(at(2026, 11, 8, 12, 0)));
        let boundary = row(REPORT_B, false, Some(now));
        let missing = row("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb", false, None);
        let training_past = row(
            "cccccccccccccccccccccccccccccccc",
            true,
            Some(at(2026, 1, 1, 0, 0)),
        );
        let training_open = row("dddddddddddddddddddddddddddddddd", true, None);
        let source = vec![
            past.clone(),
            future.clone(),
            boundary.clone(),
            missing.clone(),
            training_past.clone(),
            training_open.clone(),
        ];

        let PreparedRows::Rows(member_rows) = prepare_member_rows(&source, None) else {
            panic!("member payload has no member id");
        };
        let member_rows = without_expired(member_rows, now);
        let member_ids: Vec<_> = member_rows.iter().map(|row| row.id.as_str()).collect();
        assert!(!member_ids.contains(&past.id.as_str()));
        assert_eq!(
            member_ids,
            vec![
                future.id.as_str(),
                boundary.id.as_str(),
                missing.id.as_str(),
                training_past.id.as_str(),
                training_open.id.as_str(),
            ]
        );
        assert_eq!(
            apply_own_filter(MemberScreen::Ready, &PreparedRows::Rows(member_rows), true),
            MemberScreen::Ready
        );

        let officer_rows = without_expired(source, now);
        assert!(officer_rows.iter().all(|row| row.id != past.id));
        assert_eq!(
            officer_screen(
                &auth(Some(OrgRole::Officer), false),
                None,
                Some(officer_rows.len())
            ),
            OfficerScreen::Ready
        );

        let only_expired = without_expired(vec![past], now);
        assert!(only_expired.is_empty());
        assert_eq!(
            officer_screen(
                &auth(Some(OrgRole::Officer), false),
                None,
                Some(only_expired.len())
            ),
            OfficerScreen::Empty
        );
        assert_eq!(
            apply_own_filter(MemberScreen::Ready, &PreparedRows::Rows(only_expired), true),
            MemberScreen::Empty
        );
        assert_eq!(
            officer_screen(
                &auth(Some(OrgRole::Officer), false),
                None,
                Some(without_expired(vec![training_past], now).len())
            ),
            OfficerScreen::Ready
        );
    }

    #[test]
    fn withdraw_consent_waits_for_confirm() {
        let mut gate = ConfirmGate::default();
        gate.arm(ReportIntent::Withdraw, REPORT_A);
        assert_eq!(
            gate.pending().map(|pending| pending.intent),
            Some(ReportIntent::Withdraw)
        );
        gate.cancel();
        assert!(gate.pending().is_none());
        assert!(gate.confirm().is_none());

        gate.arm(ReportIntent::Withdraw, REPORT_A);
        let mutation = gate
            .confirm()
            .expect("confirm returns the withdraw request");
        assert_eq!(mutation.intent, ReportIntent::Withdraw);
        assert_eq!(
            mutation.path,
            format!("/api/stat-reports/{REPORT_A}/withdraw")
        );
        assert!(gate.confirm().is_none());

        gate.arm(ReportIntent::Withdraw, "not-a-report-id");
        assert!(gate.pending().is_none());
        assert!(mutation_for(ReportIntent::Withdraw, "not-a-report-id").is_none());

        fn closed() -> Element {
            rsx! {
                MyReportsBody {
                    screen: MemberScreen::Ready,
                    rows: vec![row(REPORT_A, true, None)],
                    pending: None,
                    busy: false,
                }
            }
        }
        let html = html_of(closed);
        assert!(html.contains("Withdraw consent"), "{html}");
        assert!(!html.contains(CONFIRM_FORM_ID), "{html}");
        assert!(
            !html.contains("Withdraw training consent for this report?"),
            "{html}"
        );
        assert!(!html.contains("action=\"/api/stat-reports"), "{html}");

        fn open() -> Element {
            rsx! {
                MyReportsBody {
                    screen: MemberScreen::Ready,
                    rows: vec![row(REPORT_A, true, None)],
                    pending: Some(PendingReportAction {
                        intent: ReportIntent::Withdraw,
                        report_id: REPORT_A.into(),
                        return_focus_id: opener_focus_id(ReportIntent::Withdraw, REPORT_A),
                    }),
                    busy: false,
                }
            }
        }
        let html = html_of(open);
        assert!(html.contains(CONFIRM_FORM_ID), "{html}");
        assert!(
            html.contains("Withdraw training consent for this report? It is deleted 30 days after it was received. If that date has already passed, the report is deleted now."),
            "{html}"
        );
        assert!(!html.contains("action=\"/api/stat-reports"), "{html}");
        assert_labeled(&html, CONFIRM_SUBMIT_ID, "confirm");
        assert_labeled(&html, CONFIRM_CANCEL_ID, "cancel");
        assert_labeled(
            &html,
            &control_id("withdraw", REPORT_A),
            "withdraw-training",
        );
        assert_plain(&html);
    }

    #[test]
    fn format_size_uses_plain_units() {
        assert_eq!(format_report_size(0), "0 bytes");
        assert_eq!(format_report_size(1), "1 byte");
        assert_eq!(format_report_size(2048), "2.0 KB");
        assert_plain(&format_report_size(5 * 1024 * 1024));
    }
}
