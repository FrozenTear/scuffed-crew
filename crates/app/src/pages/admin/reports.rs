//! Officer list of tracker bug reports (`/admin/reports`).
//!
//! Non-officers hit the same officer check as the admin layout. Download uses
//! `GET /api/stat-reports/{id}` from PR 189. The zip is not rendered here.

use dioxus::prelude::*;

use crate::hooks::use_api;
use crate::state::use_auth;

use super::super::stat_reports::{OfficerReportsBody, StatReportList, officer_screen};

#[component]
pub fn AdminReports() -> Element {
    let auth = use_auth();
    let reports = use_api::<StatReportList>("/api/stat-reports");
    let auth_now = auth();
    let error = reports.error.read().as_ref().cloned();
    let list = {
        let data = reports.data.read();
        data.as_ref().and_then(|inner| inner.clone())
    };
    let row_count = list.as_ref().map(|list| list.reports.len());
    let screen = officer_screen(&auth_now, error.as_deref(), row_count);
    let rows = list.map(|list| list.reports).unwrap_or_default();
    let mut refresh = reports.refresh;

    rsx! {
        OfficerReportsBody {
            screen,
            error_detail: error.unwrap_or_default(),
            rows,
            on_retry: move |_| {
                refresh += 1;
            },
        }
    }
}
