//! Signed-in member list of tracker bug reports (`/reports`).

use chrono::Utc;
use dioxus::prelude::*;

use scuffed_api_client::{ApiClient, ClientError};

use crate::components::{Toast, use_toast};
use crate::layouts::{focus_element, use_document_keydown};
use crate::state::use_auth;

use super::stat_reports::{
    CONFIRM_DIALOG_ID, COPY_DELETED, COPY_WITHDRAWN, ConfirmGate, ListedError, MyReportsBody,
    PreparedRows, ReportIntent, StatReportList, StatReportWithdrawn, WithdrawTrainingBody,
    apply_own_filter, apply_reports_switch, confirm_escape_closes, is_reports_disabled,
    member_screen, mutation_failure_copy, prepare_member_rows, without_expired,
};

#[component]
pub fn MyReports() -> Element {
    let auth = use_auth();
    let mut toast = use_toast();
    let mut refresh = use_signal(|| 0u64);
    let mut list_error = use_signal(|| None::<ClientError>);
    let reports = use_resource(move || async move {
        let _generation = refresh();
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
    let me = use_resource(|| async move { ApiClient::web().get_me().await });
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
            focus_element(CONFIRM_DIALOG_ID);
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
        let mutation = gate.write().confirm();
        if let Some(id) = return_id {
            focus_element(&id);
        }
        let Some(mutation) = mutation else {
            return;
        };
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
            match outcome {
                Ok(deleted_now) => {
                    let message = match mutation.intent {
                        ReportIntent::Delete => COPY_DELETED,
                        ReportIntent::Withdraw if deleted_now => COPY_DELETED,
                        ReportIntent::Withdraw => COPY_WITHDRAWN,
                    };
                    toast.show(Toast::success(message));
                    refresh += 1;
                }
                Err(err) => {
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
