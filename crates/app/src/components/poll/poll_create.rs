use dioxus::prelude::*;
use serde::Serialize;

use scuffed_api_client::ApiClient;

use crate::components::{Toast, use_toast};

#[derive(Debug, Clone, Serialize)]
struct CreatePollRequest {
    title: String,
    description: Option<String>,
    options: Vec<String>,
    close_at: Option<String>,
    allow_multiple: bool,
}

#[component]
pub fn PollCreate(on_created: EventHandler<()>) -> Element {
    let mut toast = use_toast();

    let mut title = use_signal(String::new);
    let mut description = use_signal(String::new);
    let mut options_text = use_signal(String::new);
    let mut close_at = use_signal(String::new);
    let mut allow_multiple = use_signal(|| false);
    let mut submitting = use_signal(|| false);

    let on_submit = move |_| {
        if submitting() {
            return;
        }

        let clean_title = title().trim().to_string();
        if clean_title.is_empty() {
            toast.show(Toast::error("Title is required."));
            return;
        }

        let options: Vec<String> = options_text()
            .lines()
            .map(|line| line.trim())
            .filter(|line| !line.is_empty())
            .map(|line| line.to_string())
            .collect();
        if options.len() < 2 {
            toast.show(Toast::error("Add at least two options (one per line)."));
            return;
        }

        let description_value = {
            let trimmed = description().trim().to_string();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            }
        };

        let close_at_value = {
            let trimmed = close_at().trim().to_string();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            }
        };

        let payload = CreatePollRequest {
            title: clean_title,
            description: description_value,
            options,
            close_at: close_at_value,
            allow_multiple: allow_multiple(),
        };

        submitting.set(true);
        spawn(async move {
            let result = ApiClient::web()
                .post_json::<_, serde_json::Value>("/api/polls", &payload)
                .await;

            submitting.set(false);
            match result {
                Ok(_) => {
                    title.set(String::new());
                    description.set(String::new());
                    options_text.set(String::new());
                    close_at.set(String::new());
                    allow_multiple.set(false);
                    toast.show(Toast::success("Poll created."));
                    on_created.call(());
                }
                Err(err) => {
                    toast.show(Toast::error(format!("Failed to create poll: {err}")));
                }
            }
        });
    };

    rsx! {
        section { class: "poll-create",
            h2 { "Create Poll" }
            div { class: "poll-create-grid",
                div { class: "poll-create-field full",
                    label { class: "poll-create-label", "Title" }
                    input {
                        class: "poll-create-input",
                        r#type: "text",
                        value: "{title}",
                        placeholder: "Choose next scrim day?",
                        oninput: move |e| title.set(e.value()),
                    }
                }

                div { class: "poll-create-field full",
                    label { class: "poll-create-label", "Description (optional)" }
                    textarea {
                        class: "poll-create-textarea",
                        value: "{description}",
                        placeholder: "Add context for voters...",
                        oninput: move |e| description.set(e.value()),
                    }
                }

                div { class: "poll-create-field full",
                    label { class: "poll-create-label", "Options (one per line)" }
                    textarea {
                        class: "poll-create-textarea",
                        value: "{options_text}",
                        placeholder: "Monday 20:00 UTC\nTuesday 19:00 UTC",
                        oninput: move |e| options_text.set(e.value()),
                    }
                }

                div { class: "poll-create-field",
                    label { class: "poll-create-label", "Close At (optional, RFC3339 UTC)" }
                    input {
                        class: "poll-create-input",
                        r#type: "text",
                        value: "{close_at}",
                        placeholder: "2026-05-10T20:00:00Z",
                        oninput: move |e| close_at.set(e.value()),
                    }
                }

                div { class: "poll-create-field",
                    label { class: "poll-create-label", "Voting mode" }
                    div { class: "poll-create-check",
                        input {
                            r#type: "checkbox",
                            checked: allow_multiple(),
                            onchange: move |e| allow_multiple.set(e.checked()),
                        }
                        span { "Allow multiple choices per member" }
                    }
                }
            }

            div { class: "poll-create-actions",
                span { class: "poll-meta", "Officer-only action" }
                button {
                    class: "poll-btn primary",
                    disabled: submitting(),
                    onclick: on_submit,
                    if submitting() { "Creating..." } else { "Create Poll" }
                }
            }
        }
    }
}
