use dioxus::prelude::*;
use serde::Deserialize;

use crate::components::DataTable;
use crate::hooks::use_api_with;

#[derive(Debug, Clone, Deserialize)]
struct AuditLogEntry {
    #[allow(dead_code)]
    id: String,
    #[allow(dead_code)]
    actor_id: String,
    actor_name: String,
    action: String,
    target_type: String,
    target_id: String,
    details: Option<String>,
    created_at: String,
}

#[derive(Debug, Clone, Deserialize)]
struct AuditLogResponse {
    entries: Vec<AuditLogEntry>,
    total: u64,
}

const PAGE_SIZE: u64 = 50;

#[component]
pub fn AdminAuditLog() -> Element {
    let mut page = use_signal(|| 0u64);

    let log_data = use_api_with::<AuditLogResponse>(move || {
        let offset = page() * PAGE_SIZE;
        format!("/api/audit-log?limit={PAGE_SIZE}&offset={offset}")
    });

    let on_prev = move |_| {
        if page() > 0 {
            page -= 1;
        }
    };

    let on_next = {
        let log_data = log_data.data;
        move |_| {
            let data = log_data.read();
            if let Some(Some(resp)) = data.as_ref() {
                let max_page = resp.total.saturating_sub(1) / PAGE_SIZE;
                if page() < max_page {
                    page += 1;
                }
            }
        }
    };

    rsx! {

        div { class: "admin-toolbar",
            h1 { "Audit Log" }
        }

        {
            let data = log_data.data.read();
            let data = data.as_ref().and_then(|d| d.as_ref());
            match data {
                None => rsx! { p { class: "admin-loading", "Loading..." } },
                Some(resp) if resp.entries.is_empty() => rsx! {
                    p { class: "empty-state", "No audit log entries." }
                },
                Some(resp) => {
                    let total_pages = resp.total.div_ceil(PAGE_SIZE);
                    let current = page() + 1;
                    rsx! {
                        DataTable { headers: vec!["Time", "Actor", "Action", "Target", "Details"],
                            for entry in resp.entries.iter() {
                                {
                                    let timestamp: String = entry.created_at.chars().take(19).collect();
                                    let target = format!("{}:{}", entry.target_type, entry.target_id);
                                    let details = entry.details.clone().unwrap_or_else(|| "—".into());
                                    rsx! {
                                        tr { key: "{entry.id}",
                                            td { "{timestamp}" }
                                            td { "{entry.actor_name}" }
                                            td { "{entry.action}" }
                                            td { "{target}" }
                                            td { "{details}" }
                                        }
                                    }
                                }
                            }
                        }
                        div { class: "pagination",
                            button {
                                disabled: page() == 0,
                                onclick: on_prev,
                                "Previous"
                            }
                            span { class: "page-info", "Page {current} of {total_pages}" }
                            button {
                                disabled: current >= total_pages,
                                onclick: on_next,
                                "Next"
                            }
                        }
                    }
                }
            }
        }
    }
}
