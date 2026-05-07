use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

use scuffed_api_client::ApiClient;
use crate::components::{Toast, use_toast};
use crate::routes::Route;
use crate::state::auth::use_auth;

#[derive(Debug, Clone, Deserialize)]
struct ForumThreadData {
    #[allow(dead_code)]
    id: String,
    title: String,
    category: String,
    #[allow(dead_code)]
    author_member_id: String,
    content: String,
    pinned: bool,
    locked: bool,
    created_at: String,
    #[allow(dead_code)]
    is_active: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct ForumReplyData {
    #[allow(dead_code)]
    id: String,
    #[allow(dead_code)]
    author_member_id: String,
    content: String,
    created_at: String,
}

#[derive(Debug, Clone, Deserialize)]
struct ThreadDetailResponse {
    thread: ForumThreadData,
    replies: Vec<ForumReplyData>,
    reply_count: u64,
}

#[derive(Serialize)]
struct CreateReplyBody {
    content: String,
}

const PAGE_CSS: &str = r#"
    .thread-page {
        padding: 3rem 2rem;
        max-width: 800px;
        margin: 0 auto;
    }
    .thread-back {
        display: inline-flex;
        align-items: center;
        gap: 0.35rem;
        color: var(--text-muted);
        font-size: 0.8rem;
        margin-bottom: 1.5rem;
        text-decoration: none;
    }
    .thread-back:hover { color: var(--text-bright); }
    .thread-header {
        margin-bottom: 2rem;
    }
    .thread-title-row {
        display: flex;
        align-items: center;
        gap: 0.5rem;
        flex-wrap: wrap;
        margin-bottom: 0.5rem;
    }
    .thread-title {
        font-family: 'Bebas Neue', sans-serif;
        font-size: 2rem;
        color: var(--text-bright);
        letter-spacing: 2px;
        margin: 0;
    }
    .thread-badge {
        font-size: 0.6rem;
        padding: 0.15rem 0.5rem;
        border-radius: 4px;
        font-weight: 600;
        text-transform: uppercase;
    }
    .thread-badge-pin { background: #7c3aed33; color: #a78bfa; }
    .thread-badge-locked { background: #ef444433; color: #f87171; }
    .thread-badge-cat { background: #3b82f622; color: #60a5fa; }
    .thread-meta {
        font-size: 0.75rem;
        color: var(--text-muted);
    }
    .thread-op {
        background: var(--bg-card);
        border: 1px solid var(--border);
        border-radius: 10px;
        padding: 1.5rem;
        margin-bottom: 2rem;
    }
    .thread-op-content {
        color: var(--text-secondary);
        font-size: 0.9rem;
        line-height: 1.7;
        white-space: pre-wrap;
        margin: 0;
    }
    .thread-replies-heading {
        font-family: 'Rajdhani', sans-serif;
        font-weight: 700;
        font-size: 1.1rem;
        color: var(--text-bright);
        margin: 0 0 1rem;
    }
    .thread-replies-list {
        display: flex;
        flex-direction: column;
        gap: 0.75rem;
        margin-bottom: 2rem;
    }
    .thread-reply-card {
        background: var(--bg-card);
        border: 1px solid var(--border);
        border-radius: 8px;
        padding: 1rem 1.25rem;
    }
    .thread-reply-meta {
        font-size: 0.7rem;
        color: var(--text-muted);
        margin-bottom: 0.5rem;
    }
    .thread-reply-content {
        color: var(--text-secondary);
        font-size: 0.85rem;
        line-height: 1.6;
        white-space: pre-wrap;
        margin: 0;
    }
    .thread-compose {
        background: var(--bg-card);
        border: 1px solid var(--border);
        border-radius: 10px;
        padding: 1.5rem;
    }
    .thread-compose-title {
        font-family: 'Rajdhani', sans-serif;
        font-weight: 700;
        font-size: 1rem;
        color: var(--text-bright);
        margin: 0 0 0.75rem;
    }
    .thread-compose-textarea {
        width: 100%;
        background: var(--bg-surface);
        border: 1px solid var(--border);
        border-radius: 6px;
        color: var(--text-bright);
        padding: 0.6rem 0.75rem;
        font-size: 0.9rem;
        font-family: inherit;
        resize: vertical;
        min-height: 80px;
        margin-bottom: 0.75rem;
    }
    .thread-compose-textarea:focus {
        outline: none;
        border-color: var(--accent);
    }
    .thread-compose-actions {
        display: flex;
        justify-content: flex-end;
    }
    .thread-reply-btn {
        padding: 0.5rem 1rem;
        background: var(--accent);
        color: white;
        border: none;
        border-radius: 6px;
        font-size: 0.85rem;
        font-weight: 600;
        cursor: pointer;
    }
    .thread-reply-btn:hover { filter: brightness(1.15); }
    .thread-reply-btn:disabled { opacity: 0.5; cursor: not-allowed; }
    .thread-locked-notice {
        color: var(--text-muted);
        font-size: 0.85rem;
        text-align: center;
        padding: 1rem;
        background: var(--bg-card);
        border: 1px solid var(--border);
        border-radius: 8px;
    }
    .thread-loading, .thread-error {
        color: var(--text-muted);
        text-align: center;
        padding: 3rem 0;
    }
    .thread-no-replies {
        color: var(--text-muted);
        font-size: 0.85rem;
        text-align: center;
        padding: 1.5rem;
    }
"#;

#[component]
pub fn ForumThread(id: String) -> Element {
    let auth = use_auth();
    let mut refresh = use_signal(|| 0u64);
    let mut reply_content = use_signal(String::new);
    let mut submitting = use_signal(|| false);

    let thread_id = id.clone();
    let detail = use_resource(move || {
        let tid = thread_id.clone();
        let _ = refresh();
        async move {
            ApiClient::web()
                .fetch::<ThreadDetailResponse>(&format!("/api/forum/threads/{tid}"))
                .await
                .ok()
        }
    });

    rsx! {
        style { {PAGE_CSS} }

        main { class: "thread-page",
            Link { to: Route::Forum {}, class: "thread-back", "< Back to Forum" }

            {
                let data = detail.read();
                let data = data.as_ref().and_then(|d| d.as_ref());
                match data {
                    None => rsx! { p { class: "thread-loading", "Loading..." } },
                    Some(resp) => {
                        let t = &resp.thread;
                        let date: String = t.created_at.chars().take(10).collect();
                        let cat_label = match t.category.as_str() {
                            "general" => "General",
                            "game" => "Game",
                            "strategy" => "Strategy",
                            "offtopic" => "Off-Topic",
                            other => other,
                        };
                        let reply_id = id.clone();

                        rsx! {
                            // Thread header
                            div { class: "thread-header",
                                div { class: "thread-title-row",
                                    if t.pinned {
                                        span { class: "thread-badge thread-badge-pin", "Pinned" }
                                    }
                                    if t.locked {
                                        span { class: "thread-badge thread-badge-locked", "Locked" }
                                    }
                                    span { class: "thread-badge thread-badge-cat", "{cat_label}" }
                                }
                                h1 { class: "thread-title", "{t.title}" }
                                div { class: "thread-meta",
                                    "{date} \u{00b7} {resp.reply_count} replies"
                                }
                            }

                            // Original post
                            div { class: "thread-op",
                                p { class: "thread-op-content", "{t.content}" }
                            }

                            // Replies section
                            h3 { class: "thread-replies-heading", "Replies" }

                            if resp.replies.is_empty() {
                                p { class: "thread-no-replies", "No replies yet. Be the first to respond!" }
                            } else {
                                div { class: "thread-replies-list",
                                    for r in resp.replies.iter() {
                                        {render_reply(r)}
                                    }
                                }
                            }

                            // Reply compose
                            if auth().is_logged_in() {
                                if t.locked {
                                    p { class: "thread-locked-notice", "This thread is locked. No new replies can be posted." }
                                } else {
                                    div { class: "thread-compose",
                                        h4 { class: "thread-compose-title", "Post a Reply" }
                                        textarea {
                                            class: "thread-compose-textarea",
                                            rows: 4,
                                            placeholder: "Write your reply...",
                                            value: "{reply_content}",
                                            oninput: move |e| reply_content.set(e.value()),
                                        }
                                        div { class: "thread-compose-actions",
                                            button {
                                                class: "thread-reply-btn",
                                                disabled: submitting(),
                                                onclick: move |_| {
                                                    let content = reply_content();
                                                    if content.trim().is_empty() {
                                                        let mut toast = use_toast();
                                                        toast.show(Toast::error("Reply cannot be empty"));
                                                        return;
                                                    }
                                                    let tid = reply_id.clone();
                                                    submitting.set(true);
                                                    spawn(async move {
                                                        let body = CreateReplyBody { content };
                                                        let url = format!("/api/forum/threads/{tid}/replies");
                                                        match ApiClient::web().post_json::<_, serde_json::Value>(&url, &body).await {
                                                            Ok(_) => {
                                                                let mut toast = use_toast();
                                                                toast.show(Toast::success("Reply posted!"));
                                                                reply_content.set(String::new());
                                                                refresh += 1;
                                                            }
                                                            Err(e) => {
                                                                let mut toast = use_toast();
                                                                toast.show(Toast::error(format!("Failed: {e}")));
                                                            }
                                                        }
                                                        submitting.set(false);
                                                    });
                                                },
                                                if submitting() { "Posting..." } else { "Reply" }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn render_reply(r: &ForumReplyData) -> Element {
    let date: String = r.created_at.chars().take(16).collect();

    rsx! {
        div { class: "thread-reply-card",
            div { class: "thread-reply-meta", "{date}" }
            p { class: "thread-reply-content", "{r.content}" }
        }
    }
}
