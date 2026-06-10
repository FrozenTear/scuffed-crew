use dioxus::prelude::*;
use serde::Serialize;

use crate::components::ui::{BtnVariant, Button};
use crate::components::{Toast, use_toast};
use scuffed_api_client::ApiClient;

#[derive(Serialize)]
struct CreatePollBody {
    title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    options: Vec<String>,
    #[serde(default)]
    allow_multiple: bool,
}

const CREATE_CSS: &str = r#"
.poll-create {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 10px;
    padding: 1.25rem 1.5rem;
    margin-bottom: 1.5rem;
}
.poll-create-title {
    font-family: var(--font-head);
    font-weight: 700;
    font-size: 1rem;
    color: var(--text);
    margin: 0 0 0.75rem;
}
.poll-create input[type="text"],
.poll-create textarea {
    width: 100%;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 0.5rem 0.75rem;
    color: var(--text);
    font-size: 0.85rem;
    font-family: var(--font-body);
}
.poll-create input[type="text"]:focus,
.poll-create textarea:focus {
    outline: none;
    border-color: var(--accent);
}
.poll-create input::placeholder,
.poll-create textarea::placeholder {
    color: var(--text-3);
}
.poll-create-field {
    margin-bottom: 0.5rem;
}
.poll-create-field label {
    display: block;
    color: var(--text-2);
    font-size: 0.75rem;
    margin-bottom: 0.25rem;
    text-transform: uppercase;
    letter-spacing: 0.05em;
}
.poll-create-options {
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
    margin-bottom: 0.5rem;
}
.poll-create-option-row {
    display: flex;
    gap: 0.35rem;
}
.poll-create-option-row input {
    flex: 1;
}
.poll-create-remove-btn {
    background: none;
    border: 1px solid var(--border);
    color: var(--text-3);
    border-radius: 4px;
    padding: 0.25rem 0.5rem;
    font-size: 0.75rem;
    cursor: pointer;
}
.poll-create-remove-btn:hover {
    border-color: var(--danger);
    color: var(--danger);
}
.poll-create-add-btn {
    background: none;
    border: 1px dashed var(--border);
    color: var(--text-3);
    border-radius: 6px;
    padding: 0.35rem 0.75rem;
    font-size: 0.8rem;
    cursor: pointer;
    width: 100%;
    transition: border-color 0.15s, color 0.15s;
}
.poll-create-add-btn:hover {
    border-color: var(--text-2);
    color: var(--text-2);
}
.poll-create-check {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    margin: 0.5rem 0;
    color: var(--text-2);
    font-size: 0.85rem;
}
.poll-create-actions {
    display: flex;
    justify-content: flex-end;
    margin-top: 0.75rem;
}
"#;

#[component]
pub fn PollCreate(on_created: EventHandler<()>) -> Element {
    let mut title = use_signal(String::new);
    let mut description = use_signal(String::new);
    let mut options = use_signal(|| vec!["".to_string(), "".to_string()]);
    let mut allow_multiple = use_signal(|| false);
    let mut submitting = use_signal(|| false);
    let mut toasts = use_toast();

    let has_title = !title().trim().is_empty();
    let valid_options = options().iter().filter(|o| !o.trim().is_empty()).count() >= 2;
    let can_submit = has_title && valid_options && !submitting();

    let handle_submit = move |_| {
        let t = title().trim().to_string();
        if t.is_empty() {
            return;
        }

        let opts: Vec<String> = options()
            .iter()
            .map(|o| o.trim().to_string())
            .filter(|o| !o.is_empty())
            .collect();

        if opts.len() < 2 {
            return;
        }

        let desc = {
            let d = description().trim().to_string();
            if d.is_empty() { None } else { Some(d) }
        };

        let multi = allow_multiple();

        spawn(async move {
            submitting.set(true);
            let body = CreatePollBody {
                title: t,
                description: desc,
                options: opts,
                allow_multiple: multi,
            };
            match ApiClient::web()
                .post_json::<_, serde_json::Value>("/api/polls", &body)
                .await
            {
                Ok(_) => {
                    toasts.show(Toast::success("Poll created"));
                    title.set(String::new());
                    description.set(String::new());
                    options.set(vec!["".to_string(), "".to_string()]);
                    allow_multiple.set(false);
                    on_created.call(());
                }
                Err(e) => {
                    toasts.show(Toast::error(format!("Failed to create poll: {e}")));
                }
            }
            submitting.set(false);
        });
    };

    let option_count = options().len();

    rsx! {
        style { {CREATE_CSS} }
        div { class: "poll-create",
            p { class: "poll-create-title", "Create a Poll" }

            div { class: "poll-create-field",
                label { "Title" }
                input {
                    r#type: "text",
                    placeholder: "What do you want to ask?",
                    value: "{title}",
                    oninput: move |e| title.set(e.value()),
                }
            }

            div { class: "poll-create-field",
                label { "Description (optional)" }
                input {
                    r#type: "text",
                    placeholder: "Add some context...",
                    value: "{description}",
                    oninput: move |e| description.set(e.value()),
                }
            }

            div { class: "poll-create-field",
                label { "Options" }
                div { class: "poll-create-options",
                    for (i, _opt) in options().iter().enumerate() {
                        div { class: "poll-create-option-row",
                            input {
                                r#type: "text",
                                placeholder: "Option {i + 1}",
                                value: "{options()[i]}",
                                oninput: move |e: Event<FormData>| {
                                    options.write()[i] = e.value();
                                },
                            }
                            if option_count > 2 {
                                button {
                                    class: "poll-create-remove-btn",
                                    onclick: move |_| {
                                        options.write().remove(i);
                                    },
                                    "×"
                                }
                            }
                        }
                    }
                    button {
                        class: "poll-create-add-btn",
                        onclick: move |_| {
                            options.write().push(String::new());
                        },
                        "+ Add option"
                    }
                }
            }

            label { class: "poll-create-check",
                input {
                    r#type: "checkbox",
                    checked: allow_multiple(),
                    onchange: move |e: Event<FormData>| {
                        allow_multiple.set(e.checked());
                    },
                }
                "Allow multiple selections"
            }

            div { class: "poll-create-actions",
                Button {
                    variant: BtnVariant::Primary,
                    disabled: !can_submit,
                    onclick: handle_submit,
                    if submitting() { "Creating..." } else { "Create Poll" }
                }
            }
        }
    }
}
