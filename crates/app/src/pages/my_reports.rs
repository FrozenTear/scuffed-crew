//! Signed-in member list of tracker bug reports (`/reports`).

use chrono::Utc;
use dioxus::prelude::*;

use scuffed_api_client::{ApiClient, ClientError};

use crate::components::{Toast, use_toast};
use crate::layouts::{focus_element, use_document_keydown};
use crate::state::use_auth;

use super::stat_reports::{
    CONFIRM_CANCEL_ID, COPY_DELETED, COPY_WITHDRAWN, ConfirmGate, ListedError,
    MY_REPORTS_HEADING_ID, MyReportsBody, PreparedRows, ReportIntent, StatReportList,
    StatReportWithdrawn, WithdrawTrainingBody, apply_own_filter, apply_reports_switch,
    confirm_escape_closes, is_reports_disabled, member_screen, mutation_failure_copy,
    prepare_member_rows, without_expired,
};

/// Id of the list region the pager scrolls back to.
pub(crate) const MY_REPORTS_LIST_ID: &str = "my-reports-list";
/// Reports per pager page.
pub(crate) const MY_REPORTS_PAGE_SIZE: usize = 20;

/// The wide list of the reports page: a table on the table tokens on
/// desktop, one stacked card per report at phone width (the
/// `d1-admin-applications` phone pattern), so no column is clipped and no
/// cell wraps one word per line. Scoped to `.my-reports-page`; the officer
/// list keeps its own sheet in stat_reports.rs.
pub(crate) const MY_REPORTS_CSS: &str = r#"
.my-reports-wait {
    margin: 0;
    color: var(--text-3);
    font-family: var(--font-body);
    font-size: var(--text-md);
    line-height: var(--leading-body);
}
.my-reports-list {
    display: flex;
    flex-direction: column;
    gap: var(--space-stack);
    min-width: 0;
}
.my-reports-list:focus { outline: none; }
.my-reports-scroll {
    box-sizing: border-box;
    min-width: 0;
    max-width: 100%;
    overflow-x: auto;
    background: var(--table-row-bg);
    border: 1px solid var(--card-border);
    border-radius: var(--table-radius);
}
.my-reports-table {
    width: 100%;
    border-collapse: collapse;
    font-family: var(--table-font);
    font-size: var(--text-sm);
    color: var(--table-row-fg);
}
.my-reports-table th {
    padding: var(--space-3) var(--table-pad-x);
    text-align: left;
    white-space: nowrap;
    font-family: var(--font-head);
    font-size: var(--text-xs);
    font-weight: var(--table-header-weight);
    text-transform: uppercase;
    letter-spacing: var(--font-head-tracking);
    color: var(--table-header-fg);
    border-bottom: 1px solid var(--table-row-border);
}
.my-reports-table td {
    height: var(--table-row-height);
    padding: var(--space-3) var(--table-pad-x);
    vertical-align: top;
    line-height: var(--leading-body);
    border-bottom: 1px solid var(--table-row-border);
}
.my-reports-table tbody tr:last-child td { border-bottom: 0; }
.my-reports-table .report-when { white-space: nowrap; font-variant-numeric: tabular-nums; }
.my-reports-table .reports-sub {
    display: block;
    margin-top: var(--space-1);
    color: var(--text-3);
    font-size: var(--text-xs);
    line-height: var(--leading-body);
}
.my-reports-table .report-card-label { display: none; }
.my-reports-actions {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
    position: relative;
}
.my-reports-actions .reports-actions { position: relative; }
.my-reports-note {
    margin: 0;
    color: var(--text-3);
    font-size: var(--text-sm);
    line-height: var(--leading-body);
}
.my-reports-page .page-header h1:focus { outline: none; }
.my-reports-page .page-header h1:focus-visible {
    outline: var(--focus-ring-width) solid var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
}
@media (max-width: 820px) {
    .my-reports-scroll {
        overflow-x: visible;
        background: transparent;
        border: 0;
        border-radius: 0;
    }
    .my-reports-table,
    .my-reports-table tbody,
    .my-reports-table tr,
    .my-reports-table td { display: block; width: auto; min-width: 0; }
    .my-reports-table thead {
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
    .my-reports-table tr {
        padding: var(--space-card-pad);
        background: var(--card-bg);
        border: 1px solid var(--card-border);
        border-radius: var(--card-radius);
    }
    .my-reports-table tr + tr { margin-top: var(--space-3); }
    .my-reports-table td {
        height: auto;
        padding: var(--space-1) 0;
        border-bottom: 0;
    }
    .my-reports-table .report-when { white-space: normal; }
    .my-reports-table .report-card-label {
        display: block;
        font-size: var(--text-xs);
        font-weight: var(--font-label-weight);
        text-transform: uppercase;
        letter-spacing: var(--font-head-tracking);
        color: var(--table-header-fg);
    }
    .my-reports-table .my-reports-actions-cell { padding-top: var(--space-3); }
    .my-reports-table .my-reports-actions-cell > .report-card-label {
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
    .my-reports-actions > .reports-actions { flex: 1 1 auto; display: flex; }
    .my-reports-actions .btn { flex: 1 1 auto; }
}
"#;

#[component]
pub fn MyReports() -> Element {
    let auth = use_auth();
    let mut toast = use_toast();
    let mut refresh = use_signal(|| 0u64);
    let mut list_error = use_signal(|| None::<ClientError>);
    let reports = use_resource(move || async move {
        let _generation = refresh();
        // A guest gets the sign-in wall; asking the API would only 401.
        // Reading auth here reruns the fetch once the session is known.
        if !auth.read().is_logged_in() {
            return None;
        }
        match ApiClient::web()
            .fetch::<StatReportList>("/api/stat-reports")
            .await
        {
            Ok(list) => {
                list_error.set(None);
                Some(list)
            }
            Err(err) => {
                list_error.set(Some(err));
                None
            }
        }
    });
    let mut me = use_resource(|| async move { ApiClient::web().get_me().await });
    let mut gate = use_signal(ConfirmGate::default);
    let mut busy = use_signal(|| false);
    let mut switched_off = use_signal(|| false);

    let auth_now = auth();
    let failure = {
        let current = list_error.read();
        current.as_ref().map(ListedError::from_client)
    };
    let list = reports.read().clone().flatten();
    let (me_settled, member_id) = {
        let me_data = me.read();
        let settled = me_data.is_some();
        let member_id = me_data
            .as_ref()
            .and_then(|result| result.as_ref().ok())
            .and_then(|body| body.member.as_ref().map(|member| member.id.clone()));
        (settled, member_id)
    };
    let prepared = match list
        .as_ref()
        .map(|list| prepare_member_rows(&list.reports, member_id.as_deref()))
    {
        Some(PreparedRows::Rows(rows)) => PreparedRows::Rows(without_expired(rows, Utc::now())),
        Some(other) => other,
        None => PreparedRows::Rows(Vec::new()),
    };
    let row_count = list.as_ref().map(|list| list.reports.len());
    let screen = apply_reports_switch(
        apply_own_filter(
            member_screen(&auth_now, failure.as_ref(), row_count),
            &prepared,
            me_settled,
        ),
        switched_off(),
    );
    let rows = match prepared {
        PreparedRows::Rows(rows) => rows,
        PreparedRows::PendingMember => Vec::new(),
    };
    let pending = gate.read().pending().cloned();

    use_effect(move || {
        if gate.read().pending().is_some() {
            focus_element(CONFIRM_CANCEL_ID);
        }
    });
    use_document_keydown(move |evt| {
        let open = gate.read().pending().is_some();
        if !confirm_escape_closes(&evt.key(), open, busy()) {
            return;
        }
        evt.prevent_default();
        let return_id = gate
            .read()
            .pending()
            .map(|action| action.return_focus_id.clone());
        gate.write().cancel();
        if let Some(id) = return_id {
            focus_element(&id);
        }
    });

    let arm_delete = move |id: String| {
        if busy() {
            return;
        }
        gate.write().arm(ReportIntent::Delete, &id);
    };
    let arm_withdraw = move |id: String| {
        if busy() {
            return;
        }
        gate.write().arm(ReportIntent::Withdraw, &id);
    };
    let on_cancel = move |_| {
        if busy() {
            return;
        }
        let return_id = gate
            .read()
            .pending()
            .map(|action| action.return_focus_id.clone());
        gate.write().cancel();
        if let Some(id) = return_id {
            focus_element(&id);
        }
    };
    let on_confirm = move |_| {
        if busy() {
            return;
        }
        let return_id = gate
            .read()
            .pending()
            .map(|action| action.return_focus_id.clone());
        let Some(mutation) = gate.write().confirm() else {
            return;
        };
        // The dialog stays up with its buttons disabled until this finishes.
        busy.set(true);
        spawn(async move {
            let client = ApiClient::web();
            let outcome = match mutation.intent {
                ReportIntent::Delete => client.delete(&mutation.path).await.map(|()| false),
                ReportIntent::Withdraw => client
                    .post_json::<WithdrawTrainingBody, StatReportWithdrawn>(
                        &mutation.path,
                        &WithdrawTrainingBody::clear(),
                    )
                    .await
                    .map(|body| body.deleted),
            };
            busy.set(false);
            gate.write().finish();
            match outcome {
                Ok(deleted_now) => {
                    let removed = mutation.intent == ReportIntent::Delete || deleted_now;
                    let message = if removed {
                        COPY_DELETED
                    } else {
                        COPY_WITHDRAWN
                    };
                    toast.show(Toast::success(message));
                    // A deleted row and its button are gone after the refresh.
                    if removed {
                        focus_element(MY_REPORTS_HEADING_ID);
                    } else if let Some(id) = &return_id {
                        focus_element(id);
                    }
                    refresh += 1;
                }
                Err(err) => {
                    if let Some(id) = &return_id {
                        focus_element(id);
                    }
                    let body = err.http_body().unwrap_or("");
                    if is_reports_disabled(err.http_status(), body) {
                        switched_off.set(true);
                    } else {
                        toast.show(Toast::error(mutation_failure_copy(
                            mutation.intent,
                            err.http_status(),
                            body,
                            err.retry_after_header(),
                        )));
                    }
                }
            }
        });
    };
    let on_retry = move |_| {
        // A failed `/api/auth/me` also lands on the error screen. Retry both.
        me.restart();
        refresh += 1;
    };

    rsx! {
        MyReportsBody {
            screen,
            http_status: failure.as_ref().and_then(|err| err.status),
            error_body: failure
                .as_ref()
                .map(|err| err.body.clone())
                .unwrap_or_default(),
            retry_after_seconds: failure.as_ref().and_then(|err| err.header_seconds),
            rows,
            truncated: list.as_ref().is_some_and(|list| list.truncated),
            pending,
            busy: busy(),
            on_retry,
            on_arm_delete: arm_delete,
            on_arm_withdraw: arm_withdraw,
            on_confirm,
            on_cancel,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::admin_shared::test_page::render_admin_route as render_route;
    use crate::components::member_gate::test_route::wall;
    use crate::components::ui::test_support::{assert_sizes_from_tokens, css_rule, has_decl};
    use scuffed_types::OrgRole;

    #[test]
    fn a_guest_gets_the_sign_in_wall_in_the_wide_page() {
        let html = render_route("/reports", None);
        assert_eq!(html.matches("<h1").count(), 1, "{html}");
        assert_eq!(html.matches("<main").count(), 1, "{html}");
        assert!(
            html.contains("ui-shell ui-shell--wide my-reports-page"),
            "{html}"
        );
        let card = wall(&html);
        assert!(card.contains("Log in to view your reports."), "{card}");
        assert!(card.contains("href=\"/login?return=/reports\""), "{card}");
    }

    #[test]
    fn a_member_sees_a_labelled_wait() {
        let html = render_route("/reports", Some(OrgRole::Member));
        assert_eq!(html.matches("<h1").count(), 1, "{html}");
        assert!(
            html.contains("<p class=\"my-reports-wait\" role=\"status\">Loading your reports.</p>"),
            "{html}"
        );
        assert!(html.contains("ui-skel-list"), "{html}");
    }

    #[test]
    fn list_css_uses_tokens_and_stacks_on_phone() {
        assert_sizes_from_tokens(MY_REPORTS_CSS);
        let phone = MY_REPORTS_CSS
            .split("@media (max-width: 820px)")
            .nth(1)
            .expect("phone block");
        let row = css_rule(phone, ".my-reports-table tr {");
        assert!(has_decl(&row, "border-radius: var(--card-radius)"), "{row}");
        assert!(
            phone.contains(".my-reports-table td { display: block;")
                || phone.contains(
                    ".my-reports-table td { display: block; width: auto; min-width: 0; }"
                )
        );
        let date = css_rule(MY_REPORTS_CSS, ".my-reports-table .report-when {");
        assert!(has_decl(&date, "white-space: nowrap"), "{date}");
    }
}
