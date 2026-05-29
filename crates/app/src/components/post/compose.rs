use dioxus::prelude::*;
use serde::Serialize;

use super::card::FeedPost;
use crate::components::{Toast, use_toast};
use scuffed_api_client::ApiClient;

#[derive(Serialize)]
struct CreatePostBody {
    content: String,
    hashtags: Vec<String>,
}

const COMPOSE_CSS: &str = r#"
.post-compose {
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 10px;
    padding: 1.25rem 1.5rem;
    margin-bottom: 1.5rem;
}
.post-compose-label {
    font-family: 'Rajdhani', sans-serif;
    font-weight: 700;
    font-size: 1rem;
    color: var(--text-bright);
    margin: 0 0 0.75rem;
}
.post-compose textarea {
    width: 100%;
    min-height: 80px;
    background: var(--bg-surface);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 0.75rem;
    color: var(--text-bright);
    font-size: 0.85rem;
    font-family: 'Source Sans 3', sans-serif;
    resize: vertical;
    line-height: 1.6;
}
.post-compose textarea:focus {
    outline: none;
    border-color: #7c3aed;
}
.post-compose textarea::placeholder {
    color: var(--text-muted);
}
.post-compose-tags {
    margin-top: 0.5rem;
}
.post-compose-tags input {
    width: 100%;
    background: var(--bg-surface);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 0.5rem 0.75rem;
    color: var(--text-bright);
    font-size: 0.8rem;
    font-family: 'Source Sans 3', sans-serif;
}
.post-compose-tags input:focus {
    outline: none;
    border-color: #7c3aed;
}
.post-compose-tags input::placeholder {
    color: var(--text-muted);
}
.post-compose-actions {
    display: flex;
    justify-content: flex-end;
    margin-top: 0.75rem;
}
.post-compose-submit {
    background: #7c3aed;
    color: white;
    border: none;
    border-radius: 6px;
    padding: 0.45rem 1.25rem;
    font-size: 0.85rem;
    font-weight: 600;
    cursor: pointer;
    transition: filter 0.15s;
}
.post-compose-submit:hover:not(:disabled) {
    filter: brightness(1.15);
}
.post-compose-submit:disabled {
    opacity: 0.5;
    cursor: not-allowed;
}
"#;

#[component]
pub fn PostCompose(on_post: EventHandler<FeedPost>) -> Element {
    let mut content = use_signal(String::new);
    let mut tags_input = use_signal(String::new);
    let mut submitting = use_signal(|| false);
    let mut toasts = use_toast();

    let can_submit = !content().trim().is_empty() && !submitting();

    let handle_submit = move |_| {
        let text = content().trim().to_string();
        if text.is_empty() {
            return;
        }

        let hashtags: Vec<String> = tags_input()
            .split(',')
            .map(|t| t.trim().trim_start_matches('#').to_string())
            .filter(|t| !t.is_empty())
            .collect();

        let text_clone = text.clone();
        let tags_clone = hashtags.clone();

        let optimistic = FeedPost {
            id: format!("pending-{}", js_sys::Date::now() as u64),
            pubkey: String::new(),
            author_name: Some("You".to_string()),
            content: text_clone.clone(),
            hashtags: tags_clone.clone(),
            created_at: (js_sys::Date::now() / 1000.0) as i64,
            reactions: vec![],
            reply_count: 0,
        };
        on_post.call(optimistic);

        content.set(String::new());
        tags_input.set(String::new());

        spawn(async move {
            submitting.set(true);
            let body = CreatePostBody {
                content: text_clone,
                hashtags: tags_clone,
            };
            match ApiClient::web()
                .post_json::<_, serde_json::Value>("/api/nostr/post", &body)
                .await
            {
                Ok(_) => {
                    toasts.show(Toast::success("Post published"));
                }
                Err(e) => {
                    toasts.show(Toast::error(format!("Failed to post: {e}")));
                }
            }
            submitting.set(false);
        });
    };

    rsx! {
        style { {COMPOSE_CSS} }
        div { class: "post-compose",
            p { class: "post-compose-label", "What's on your mind?" }
            textarea {
                placeholder: "Share something with the crew...",
                value: "{content}",
                oninput: move |e| content.set(e.value()),
            }
            div { class: "post-compose-tags",
                input {
                    r#type: "text",
                    placeholder: "Tags (comma-separated, e.g. valorant, lfg, highlight)",
                    value: "{tags_input}",
                    oninput: move |e| tags_input.set(e.value()),
                }
            }
            div { class: "post-compose-actions",
                button {
                    class: "post-compose-submit",
                    disabled: !can_submit,
                    onclick: handle_submit,
                    if submitting() { "Posting..." } else { "Post" }
                }
            }
        }
    }
}
