//! Signed-in member list of tracker bug reports (`/reports`).

use dioxus::prelude::*;

use scuffed_api_client::ApiClient;

use crate::components::{Toast, use_toast};
use crate::hooks::use_api;
use crate::state::use_auth;

use super::stat_reports::{
    COPY_DELETE_FAILED, COPY_DELETED, COPY_WITHDRAW_FAILED, COPY_WITHDRAWN, ConfirmGate,
    MyReportsBody, PreparedRows, ReportIntent, StatReportList, StatReportWithdrawn,
    WithdrawTrainingBody, apply_own_filter, member_screen, prepare_member_rows,
};

#[component]
pub fn MyReports() -> Element {
    let auth = use_auth();
    let mut toast = use_toast();
    let reports = use_api::<StatReportList>("/api/stat-reports");
    let me = use_resource(|| async move { ApiClient::web().get_me().await });
    let mut gate = use_signal(ConfirmGate::default);
    let mut busy = use_signal(|| false);

    let auth_now = auth();
    let error = reports.error.read().as_ref().cloned();
    let list = {
        let data = reports.data.read();
        data.as_ref().and_then(|inner| inner.clone())
    };
    let (me_settled, member_id) = {
        let me_data = me.read();
        let settled = me_data.is_some();
        let member_id = me_data
            .as_ref()
            .and_then(|result| result.as_ref().ok())
            .and_then(|body| body.member.as_ref().map(|member| member.id.clone()));
        (settled, member_id)
    };
    let prepared = list
        .as_ref()
        .map(|list| prepare_member_rows(&list.reports, member_id.as_deref()))
        .unwrap_or(PreparedRows::Rows(Vec::new()));
    let row_count = list.as_ref().map(|list| list.reports.len());
    let screen = apply_own_filter(
        member_screen(&auth_now, error.as_deref(), row_count),
        &prepared,
        me_settled,
    );
    let rows = match prepared {
        PreparedRows::Rows(rows) => rows,
        PreparedRows::PendingMember => Vec::new(),
    };
    let pending = gate.read().pending().cloned();

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
        if !busy() {
            gate.write().cancel();
        }
    };
    let on_confirm = move |_| {
        if busy() {
            return;
        }
        let mutation = gate.write().confirm();
        let Some(mutation) = mutation else {
            return;
        };
        busy.set(true);
        let mut refresh = reports.refresh;
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
                    let lead = match mutation.intent {
                        ReportIntent::Delete => COPY_DELETE_FAILED,
                        ReportIntent::Withdraw => COPY_WITHDRAW_FAILED,
                    };
                    toast.show(Toast::error(format!("{lead} {err}")));
                }
            }
        });
    };
    let mut refresh = reports.refresh;
    let on_retry = move |_| {
        refresh += 1;
    };

    rsx! {
        MyReportsBody {
            screen,
            error_detail: error.unwrap_or_default(),
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
