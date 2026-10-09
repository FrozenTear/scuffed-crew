//! Officer list of tracker bug reports (`/admin/reports`).
//!
//! Non-officers hit the same officer check as the admin layout. Download uses
//! `GET /api/stat-reports/{id}` from PR 189. The zip is not rendered here.

use chrono::Utc;
use dioxus::prelude::*;

use scuffed_api_client::{ApiClient, ClientError};

use crate::state::use_auth;

use super::super::stat_reports::{
    OfficerReportsBody, StatReportList, officer_screen, without_expired,
};

#[component]
pub fn AdminReports() -> Element {
    let auth = use_auth();
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
    let auth_now = auth();
    let (error_detail, error_body, retry_after_seconds) = {
        let current = list_error.read();
        match current.as_ref() {
            Some(err) => (
                err.to_string(),
                err.http_body().unwrap_or("").to_string(),
                err.retry_after_header(),
            ),
            None => (String::new(), String::new(), None),
        }
    };
    let error = (!error_detail.is_empty()).then(|| error_detail.clone());
    let list = reports.read().clone().flatten();
    let rows = list
        .as_ref()
        .map(|list| without_expired(list.reports.clone(), Utc::now()))
        .unwrap_or_default();
    let row_count = list.as_ref().map(|_| rows.len());
    let screen = officer_screen(&auth_now, error.as_deref(), row_count);

    rsx! {
        OfficerReportsBody {
            screen,
            error_detail,
            error_body,
            retry_after_seconds,
            rows,
            on_retry: move |_| {
                refresh += 1;
            },
        }
    }
}
