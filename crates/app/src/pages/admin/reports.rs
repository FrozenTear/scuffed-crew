//! Officer list of tracker bug reports (`/admin/reports`).
//!
//! Non-officers hit the same officer check as the admin layout. Download
//! fetches the zip and saves it as a file. The zip is not rendered here.
//!
//! The list body (its H1, table and states) is `OfficerReportsBody` in
//! `pages/stat_reports.rs`, shared with that page's tests. This route puts it
//! in the wide admin column and answers the no-access and session-check
//! states itself, so the no-access page is not nested inside the column.

use chrono::Utc;
use dioxus::prelude::*;

use scuffed_api_client::{ApiClient, ClientError};

use crate::components::AccessDenied;
use crate::components::ui::{PageHeader, PageShell, PageWidth, SkeletonBars};
use crate::state::use_auth;

use super::super::stat_reports::{
    COPY_CHECKING, COPY_FORBIDDEN, ListedError, OfficerReportsBody, OfficerScreen, officer_screen,
    parse_officer_reports, report_is_current,
};

#[component]
pub fn AdminReports() -> Element {
    let auth = use_auth();
    let mut refresh = use_signal(|| 0u64);
    let mut list_error = use_signal(|| None::<ClientError>);
    let reports = use_resource(move || async move {
        let _generation = refresh();
        match ApiClient::web()
            .fetch::<serde_json::Value>("/api/stat-reports")
            .await
        {
            Ok(value) => match parse_officer_reports(&value) {
                Ok(list) => {
                    list_error.set(None);
                    Some(list)
                }
                Err(message) => {
                    list_error.set(Some(ClientError::Deserialize(message)));
                    None
                }
            },
            Err(err) => {
                list_error.set(Some(err));
                None
            }
        }
    });
    let auth_now = auth();
    let failure = {
        let current = list_error.read();
        current.as_ref().map(ListedError::from_client)
    };
    let list = reports.read().clone().flatten();
    let now = Utc::now();
    let (rows, display_names) = list
        .as_ref()
        .map(|list| {
            let mut rows = Vec::new();
            let mut names = Vec::new();
            for (row, name) in list.rows.iter().zip(list.display_names.iter()) {
                if report_is_current(row, now) {
                    rows.push(row.clone());
                    names.push(name.clone());
                }
            }
            (rows, names)
        })
        .unwrap_or_default();
    let row_count = list.as_ref().map(|_| rows.len());
    let screen = officer_screen(&auth_now, failure.as_ref(), row_count);

    match screen {
        OfficerScreen::Forbidden => rsx! {
            AccessDenied { message: COPY_FORBIDDEN.to_string() }
        },
        OfficerScreen::Checking => rsx! {
            PageShell { width: PageWidth::Wide, class: "admin-reports-page",
                PageHeader { title: "Reports" }
                div { class: "admin-pending",
                    p { class: "admin-pending-text", role: "status", "{COPY_CHECKING}" }
                    SkeletonBars { rows: 3 }
                }
            }
        },
        _ => rsx! {
            PageShell { width: PageWidth::Wide, class: "admin-reports-page",
                OfficerReportsBody {
                    screen,
                    http_status: failure.as_ref().and_then(|err| err.status),
                    error_body: failure
                        .as_ref()
                        .map(|err| err.body.clone())
                        .unwrap_or_default(),
                    retry_after_seconds: failure.as_ref().and_then(|err| err.header_seconds),
                    rows,
                    display_names,
                    truncated: list.as_ref().is_some_and(|list| list.truncated),
                    on_retry: move |_| {
                        refresh += 1;
                    },
                }
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use crate::components::admin_shared::test_page::render_admin_route;
    use scuffed_types::OrgRole;

    #[test]
    fn reports_sit_in_the_wide_column_with_one_h1() {
        let html = render_admin_route("/admin/reports", Some(OrgRole::Officer));
        assert!(
            html.contains("class=\"ui-shell ui-shell--wide admin-reports-page\""),
            "{html}"
        );
        assert_eq!(html.matches("<h1").count(), 1, "{html}");
        assert!(html.contains("<h1>Reports</h1>"), "{html}");
        assert_eq!(html.matches("<main").count(), 1, "{html}");
        assert!(!html.contains("ui-shell--narrow"), "{html}");
    }
}
