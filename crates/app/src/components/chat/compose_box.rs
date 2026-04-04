//! Reply input + send button for the chat widget.

use dioxus::prelude::*;

const COMPOSE_BOX_CSS: &str = r#"
.chat-compose {
    display: flex;
    gap: 8px;
    padding: 8px 12px;
    border-top: 1px solid var(--border);
    background: var(--bg-card);
    align-items: flex-end;
}

.chat-compose__input {
    flex: 1;
    min-height: 36px;
    max-height: 100px;
    padding: 8px 12px;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--bg-surface);
    color: var(--text-primary);
    font-family: var(--font-body);
    font-size: 13px;
    line-height: 1.4;
    resize: none;
    outline: none;
    transition: border-color 0.15s ease;
}

.chat-compose__input::placeholder {
    color: var(--text-muted);
}

.chat-compose__input:focus {
    border-color: var(--accent);
}

.chat-compose__input:disabled {
    opacity: 0.5;
    cursor: not-allowed;
}

.chat-compose__send {
    padding: 8px 14px;
    border: none;
    border-radius: 8px;
    background: var(--accent);
    color: #fff;
    font-size: 13px;
    font-weight: 600;
    cursor: pointer;
    transition: opacity 0.15s ease, transform 0.1s ease;
    white-space: nowrap;
}

.chat-compose__send:hover:not(:disabled) {
    opacity: 0.9;
}

.chat-compose__send:active:not(:disabled) {
    transform: scale(0.96);
}

.chat-compose__send:disabled {
    opacity: 0.4;
    cursor: not-allowed;
}

.chat-compose__hint {
    font-size: 10px;
    color: var(--text-muted);
    padding: 2px 12px 4px;
    background: var(--bg-card);
}
"#;

/// Chat message compose box with text input and send button.
#[component]
pub fn ComposeBox(
    on_send: EventHandler<String>,
    #[props(default = false)] disabled: bool,
    #[props(default = "Type a message...".to_string())] placeholder: String,
    #[props(default = false)] encrypted: bool,
) -> Element {
    let mut draft = use_signal(String::new);

    let can_send = !disabled && !draft.read().trim().is_empty();

    let mut submit = move || {
        let text = draft.read().trim().to_string();
        if !text.is_empty() && !disabled {
            on_send.call(text);
            draft.set(String::new());
        }
    };

    rsx! {
        style { {COMPOSE_BOX_CSS} }
        if encrypted {
            div { class: "chat-compose__hint",
                "Messages in this channel are end-to-end encrypted"
            }
        }
        div { class: "chat-compose",
            input {
                class: "chat-compose__input",
                r#type: "text",
                placeholder: "{placeholder}",
                disabled: disabled,
                value: "{draft}",
                oninput: move |evt| draft.set(evt.value()),
                onkeypress: move |evt| {
                    if evt.key() == Key::Enter {
                        submit();
                    }
                },
            }
            button {
                class: "chat-compose__send",
                disabled: !can_send,
                onclick: move |_| submit(),
                "Send"
            }
        }
    }
}
