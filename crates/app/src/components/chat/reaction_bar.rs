use dioxus::prelude::*;
use serde::Serialize;

use crate::components::{Toast, ToastState, use_toast};
use scuffed_api_client::ApiClient;

const QUICK_EMOJIS: &[(&str, &str)] = &[
    ("\u{1f44d}", "thumbs up"),
    ("\u{2764}\u{fe0f}", "heart"),
    ("\u{1f602}", "laugh"),
    ("\u{1f62e}", "wow"),
    ("\u{1f525}", "fire"),
    ("\u{1f389}", "party"),
    ("\u{1f44e}", "thumbs down"),
];

#[derive(Clone, Debug, PartialEq)]
pub struct ReactionCount {
    pub emoji: String,
    pub count: u32,
    pub reacted_by_me: bool,
}

#[derive(Serialize)]
struct ReactBody {
    event_id: String,
    event_author_pubkey: String,
    content: String,
}

const REACTION_CSS: &str = r#"
.reaction-bar {
    display: flex;
    align-items: center;
    gap: 4px;
    flex-wrap: wrap;
    margin-top: 4px;
}

.reaction-pill {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    padding: 1px 6px;
    border-radius: 10px;
    border: 1px solid var(--border);
    background: var(--bg);
    font-size: 12px;
    cursor: pointer;
    transition: border-color 0.15s, background 0.15s;
    color: var(--text-2);
    line-height: 1.6;
}

.reaction-pill:hover {
    border-color: var(--accent-soft);
    background: var(--surface-2);
}

.reaction-pill--mine {
    border-color: var(--accent);
    background: color-mix(in srgb, var(--accent) 9%, transparent);
}

.reaction-pill__count {
    font-size: 11px;
    font-weight: 600;
    min-width: 8px;
    text-align: center;
}

.reaction-add {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 24px;
    height: 24px;
    border-radius: 50%;
    border: 1px dashed var(--border);
    background: none;
    cursor: pointer;
    font-size: 12px;
    color: var(--text-3);
    transition: border-color 0.15s, color 0.15s;
    position: relative;
}

.reaction-add:hover {
    border-color: var(--accent-soft);
    color: var(--text-2);
}

.reaction-picker {
    position: absolute;
    bottom: calc(100% + 6px);
    left: 50%;
    transform: translateX(-50%);
    display: flex;
    gap: 2px;
    padding: 4px 6px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 8px;
    box-shadow: 0 4px 12px var(--overlay);
    z-index: 20;
}

.reaction-picker button {
    background: none;
    border: none;
    cursor: pointer;
    font-size: 16px;
    padding: 4px;
    border-radius: 4px;
    transition: background 0.1s;
    line-height: 1;
}

.reaction-picker button:hover {
    background: var(--bg);
}
"#;

#[component]
pub fn ReactionBar(
    event_id: String,
    event_author_pubkey: String,
    reactions: Vec<ReactionCount>,
) -> Element {
    let mut picker_open = use_signal(|| false);
    let toasts = use_toast();

    rsx! {
        style { {REACTION_CSS} }
        div { class: "reaction-bar",
            for r in reactions.iter() {
                {render_pill(
                    r.clone(),
                    event_id.clone(),
                    event_author_pubkey.clone(),
                    toasts,
                )}
            }
            div { style: "position: relative; display: inline-flex;",
                button {
                    class: "reaction-add",
                    aria_label: "Add reaction",
                    onclick: move |_| picker_open.toggle(),
                    "+"
                }
                if picker_open() {
                    div { class: "reaction-picker",
                        for &(emoji, label) in QUICK_EMOJIS.iter() {
                            {render_picker_btn(
                                emoji,
                                label,
                                event_id.clone(),
                                event_author_pubkey.clone(),
                                toasts,
                                picker_open,
                            )}
                        }
                    }
                }
            }
        }
    }
}

fn render_pill(r: ReactionCount, event_id: String, author: String, toasts: ToastState) -> Element {
    let pill_class = if r.reacted_by_me {
        "reaction-pill reaction-pill--mine"
    } else {
        "reaction-pill"
    };
    let emoji = r.emoji.clone();

    rsx! {
        button {
            class: pill_class,
            aria_label: "React with {emoji}",
            onclick: move |_| {
                let event_id = event_id.clone();
                let author = author.clone();
                let emoji = emoji.clone();
                let toasts = toasts;
                spawn(async move {
                    send_reaction(&event_id, &author, &emoji, toasts).await;
                });
            },
            span { "{r.emoji}" }
            span { class: "reaction-pill__count", "{r.count}" }
        }
    }
}

fn render_picker_btn(
    emoji: &str,
    label: &str,
    event_id: String,
    author: String,
    toasts: ToastState,
    mut picker_open: Signal<bool>,
) -> Element {
    let emoji_owned = emoji.to_string();
    let label_owned = label.to_string();

    rsx! {
        button {
            aria_label: "{label_owned}",
            onclick: move |_| {
                let event_id = event_id.clone();
                let author = author.clone();
                let emoji = emoji_owned.clone();
                let toasts = toasts;
                picker_open.set(false);
                spawn(async move {
                    send_reaction(&event_id, &author, &emoji, toasts).await;
                });
            },
            "{emoji}"
        }
    }
}

async fn send_reaction(event_id: &str, author_pubkey: &str, content: &str, mut toasts: ToastState) {
    let body = ReactBody {
        event_id: event_id.to_string(),
        event_author_pubkey: author_pubkey.to_string(),
        content: content.to_string(),
    };

    match ApiClient::web()
        .post_json::<_, serde_json::Value>("/api/nostr/react", &body)
        .await
    {
        Ok(_) => {}
        Err(e) => {
            toasts.show(Toast::error(format!("Reaction failed: {e}")));
        }
    }
}
