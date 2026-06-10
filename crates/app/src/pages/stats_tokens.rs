use dioxus::prelude::*;

use chrono::{DateTime, Utc};
use serde::Deserialize;

use scuffed_api_client::ApiClient;
use scuffed_types::api::{CreateDaemonTokenRequest, CreateDaemonTokenResponse};

use crate::components::{DataTable, FormModal, Toast, use_toast};
use crate::hooks::{ModalController, use_api};

#[derive(Debug, Clone, Deserialize)]
struct DaemonToken {
    id: String,
    #[allow(dead_code)]
    member_id: String,
    label: String,
    is_active: bool,
    created_at: DateTime<Utc>,
    last_used_at: Option<DateTime<Utc>>,
}

const TOKENS_CSS: &str = r#"
    .tokens-page {
        max-width: 900px;
        margin: 0 auto;
        padding: 2rem 1.5rem;
    }
    .tokens-page h1 {
        font-family: var(--font-head);
        font-size: 1.8rem;
        color: var(--text);
        text-transform: uppercase;
        letter-spacing: 0.04em;
    }
    .token-reveal {
        background: var(--surface);
        border: 1px solid var(--accent);
        border-radius: 8px;
        padding: 1rem;
        margin-bottom: 1.5rem;
    }
    .token-reveal p {
        color: var(--text-2);
        font-size: 0.85rem;
        margin-bottom: 0.5rem;
    }
    .token-reveal .warning {
        color: var(--warn);
        font-size: 0.8rem;
        font-weight: 600;
    }
    .token-reveal code {
        display: block;
        background: var(--bg);
        border: 1px solid var(--border);
        border-radius: 4px;
        padding: 0.6rem 0.75rem;
        font-family: var(--font-mono);
        font-size: 0.8rem;
        color: var(--accent);
        word-break: break-all;
        margin: 0.5rem 0;
        user-select: all;
    }
"#;

fn format_date(dt: &DateTime<Utc>) -> String {
    dt.format("%b %d, %Y %H:%M").to_string()
}

#[component]
pub fn StatsTokens() -> Element {
    let mut tokens = use_api::<Vec<DaemonToken>>("/api/stats/tokens");
    let mut toast = use_toast();

    let mut modal = ModalController::<String>::new();
    let mut form_label = use_signal(|| "default".to_string());

    let mut revealed_token: Signal<Option<String>> = use_signal(|| None);

    let open_create = move |_| {
        form_label.set("default".to_string());
        modal.show_empty();
    };

    let on_close = move |_| {
        modal.close();
    };

    let on_submit = move |_| {
        let label = form_label().trim().to_string();
        if label.is_empty() {
            return;
        }

        modal.start_submit();
        spawn(async move {
            let body = CreateDaemonTokenRequest { label };
            let result = ApiClient::web()
                .post_json::<_, CreateDaemonTokenResponse>("/api/stats/tokens", &body)
                .await;

            modal.end_submit();
            match result {
                Ok(resp) => {
                    toast.show(Toast::success("Token created."));
                    revealed_token.set(Some(resp.token));
                    modal.close();
                    tokens.refresh += 1;
                }
                Err(e) => {
                    toast.show(Toast::error(format!("Failed: {e}")));
                }
            }
        });
    };

    let on_revoke = move |token_id: String| {
        spawn(async move {
            let result = ApiClient::web()
                .delete(&format!("/api/stats/tokens/{token_id}"))
                .await;
            match result {
                Ok(()) => {
                    toast.show(Toast::success("Token revoked."));
                    tokens.refresh += 1;
                }
                Err(e) => {
                    toast.show(Toast::error(format!("Failed: {e}")));
                }
            }
        });
    };

    rsx! {
        style { {TOKENS_CSS} }
        style { {crate::styles::admin::CSS} }

        div { class: "tokens-page",
            div { class: "admin-toolbar",
                h1 { "Daemon Tokens" }
                button { class: "btn-add", onclick: open_create, "+ New Token" }
            }

            if let Some(raw) = revealed_token() {
                div { class: "token-reveal",
                    p { "Your new token:" }
                    code { "{raw}" }
                    p { class: "warning", "Copy this now — it will not be shown again." }
                    button {
                        class: "btn-cancel",
                        onclick: move |_| revealed_token.set(None),
                        "Dismiss"
                    }
                }
            }

            {
                let data = tokens.data.read();
                let data = data.as_ref().and_then(|d| d.as_ref());
                match data {
                    None => rsx! { p { class: "loading-state", "Loading tokens..." } },
                    Some(list) if list.is_empty() => rsx! {
                        p { class: "empty-state", "No daemon tokens yet. Create one to start uploading stats." }
                    },
                    Some(list) => rsx! {
                        DataTable { headers: vec!["Label", "Status", "Created", "Last Used", "Actions"],
                            for token in list.iter() {
                                {
                                    let tid = token.id.clone();
                                    let status = if token.is_active { "Active" } else { "Revoked" };
                                    let status_class = if token.is_active { "status-pill active" } else { "status-pill inactive" };
                                    let created = format_date(&token.created_at);
                                    let used = token.last_used_at.as_ref().map(format_date).unwrap_or_else(|| "Never".into());
                                    rsx! {
                                        tr { key: "{token.id}",
                                            td { "{token.label}" }
                                            td { span { class: "{status_class}", "{status}" } }
                                            td { "{created}" }
                                            td { "{used}" }
                                            td {
                                                if token.is_active {
                                                    div { class: "row-actions",
                                                        button {
                                                            class: "row-btn danger",
                                                            onclick: move |_| on_revoke(tid.clone()),
                                                            "Revoke"
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    },
                }
            }

            FormModal {
                title: "Create Daemon Token".to_string(),
                open: modal.is_open(),
                submitting: modal.is_submitting(),
                on_close: on_close,
                on_submit: on_submit,

                div { class: "form-field",
                    label { class: "form-label", "Label" }
                    input {
                        class: "form-input",
                        r#type: "text",
                        placeholder: "e.g. my-desktop",
                        value: "{form_label}",
                        oninput: move |e| form_label.set(e.value()),
                    }
                }
            }
        }
    }
}
