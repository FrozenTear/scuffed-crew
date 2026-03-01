use leptos::prelude::*;
use serde::Deserialize;

use scuffed_ui::components::button::{Button, ButtonVariant};

use crate::api;
use crate::components::data_table::DataTable;

#[derive(Debug, Clone, Deserialize)]
struct AuditLogEntry {
    #[allow(dead_code)]
    id: String,
    actor_id: String,
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

#[component]
pub fn AuditLogPage() -> impl IntoView {
    let page = RwSignal::new(0u32);
    let page_size = 50u32;

    let data = LocalResource::new(move || {
        let offset = page.get() * page_size;
        async move {
            api::get::<AuditLogResponse>(&format!(
                "/api/audit-log?limit={page_size}&offset={offset}"
            ))
            .await
            .ok()
        }
    });

    let total_pages = move || {
        data.get()
            .flatten()
            .map(|d| ((d.total as f64) / (page_size as f64)).ceil() as u32)
            .unwrap_or(1)
    };

    view! {
        <h1>"Audit Log"</h1>
        {move || match data.get().flatten() {
            None => view! { <p>"Loading..."</p> }.into_any(),
            Some(resp) if resp.entries.is_empty() => {
                view! { <p>"No audit log entries yet."</p> }.into_any()
            }
            Some(resp) => view! {
                <DataTable headers=vec!["Timestamp", "Actor", "Action", "Target", "Details"]>
                    {resp.entries.into_iter().map(|e| {
                        let ts = e.created_at.chars().take(19).collect::<String>().replace('T', " ");
                        let details = e.details.unwrap_or_else(|| "\u{2014}".into());
                        let target = format!("{}:{}", e.target_type, e.target_id);
                        view! {
                            <tr>
                                <td style="white-space: nowrap; font-size: 0.8rem; font-family: var(--font-mono);">{ts}</td>
                                <td>{e.actor_id}</td>
                                <td><span class="status-pill" style="background: var(--accent-soft); color: var(--accent-bright);">{e.action}</span></td>
                                <td style="font-size: 0.8rem; font-family: var(--font-mono);">{target}</td>
                                <td style="font-size: 0.85rem;">{details}</td>
                            </tr>
                        }
                    }).collect_view()}
                </DataTable>

                <div style="display: flex; justify-content: center; gap: 1rem; margin-top: 1rem; align-items: center;">
                    <Button
                        variant=ButtonVariant::Ghost
                        disabled=page.get() == 0
                        on_click=Callback::new(move |_| page.update(|p| *p = p.saturating_sub(1)))
                    >
                        "Previous"
                    </Button>
                    <span style="color: var(--text-muted); font-size: 0.85rem;">
                        {move || format!("Page {} of {}", page.get() + 1, total_pages())}
                    </span>
                    <Button
                        variant=ButtonVariant::Ghost
                        disabled={page.get() + 1 >= total_pages()}
                        on_click=Callback::new(move |_| page.update(|p| *p += 1))
                    >
                        "Next"
                    </Button>
                </div>
            }.into_any(),
        }}
    }
}
