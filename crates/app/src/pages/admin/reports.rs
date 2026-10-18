//! Officer list of tracker bug reports (`/admin/reports`).
//!
//! Non-officers hit the same officer check as the admin layout. Download
//! fetches the zip and saves it as a file. The zip is not rendered here.

use chrono::Utc;
use dioxus::prelude::*;

use scuffed_api_client::{ApiClient, ClientError};

use crate::state::use_auth;

use super::super::stat_reports::{
    ListedError, OfficerReportsBody, officer_screen, parse_officer_reports, report_is_current,
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

    rsx! {
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
}
