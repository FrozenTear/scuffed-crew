use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

use crate::components::{Toast, use_toast};
use crate::routes::Route;
use crate::state::auth::use_auth;
use scuffed_api_client::ApiClient;

#[derive(Debug, Clone, Deserialize)]
struct ForumThreadData {
    #[allow(dead_code)]
    id: String,
    title: String,
    category: String,
    board_id: Option<String>,
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
struct BoardMeta {
    name: String,
    slug: String,
}

#[derive(Debug, Clone, Deserialize)]
struct CategoryMeta {
    name: String,
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
    board: Option<BoardMeta>,
    category: Option<CategoryMeta>,
    parent_board: Option<BoardMeta>,
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
        color: var(--text-3);
        font-size: 0.8rem;
        margin-bottom: 1.5rem;
        text-decoration: none;
    }
    .thread-back:hover { color: var(--text); }
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
        font-family: var(--font-head);
        font-size: 2rem;
        color: var(--text);
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
    .thread-badge-pin {
        background: color-mix(in srgb, var(--accent) 20%, transparent);
        color: var(--accent);
    }
    .thread-badge-locked {
        background: color-mix(in srgb, var(--danger) 20%, transparent);
        color: var(--danger);
    }
    .thread-badge-cat {
        background: color-mix(in srgb, var(--text-2) 15%, transparent);
        color: var(--text-2);
    }
    .thread-meta {
        font-size: 0.75rem;
        color: var(--text-3);
    }
    .thread-op {
        background: var(--surface);
        border: 1px solid var(--border);
        border-radius: 10px;
        padding: 1.5rem;
        margin-bottom: 2rem;
    }
    .thread-op-content {
        color: var(--text-2);
        font-size: 0.9rem;
        line-height: 1.7;
        white-space: pre-wrap;
        margin: 0;
    }
    .thread-replies-heading {
        font-family: var(--font-head);
        font-weight: 700;
        font-size: 1.1rem;
        color: var(--text);
        margin: 0 0 1rem;
    }
    .thread-replies-list {
        display: flex;
        flex-direction: column;
        gap: 0.75rem;
        margin-bottom: 2rem;
    }
    .thread-reply-card {
        background: var(--surface);
        border: 1px solid var(--border);
        border-radius: 8px;
        padding: 1rem 1.25rem;
    }
    .thread-reply-meta {
        font-size: 0.7rem;
        color: var(--text-3);
        margin-bottom: 0.5rem;
    }
    .thread-reply-content {
        color: var(--text-2);
        font-size: 0.85rem;
        line-height: 1.6;
        white-space: pre-wrap;
        margin: 0;
    }
    .thread-compose {
        background: var(--surface);
        border: 1px solid var(--border);
        border-radius: 10px;
        padding: 1.5rem;
    }
    .thread-compose-title {
        font-family: var(--font-head);
        font-weight: 700;
        font-size: 1rem;
        color: var(--text);
        margin: 0 0 0.75rem;
    }
    .thread-compose-textarea {
        width: 100%;
        background: var(--bg);
        border: 1px solid var(--border);
        border-radius: 6px;
        color: var(--text);
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
        color: var(--accent-fg);
        border: none;
        border-radius: 6px;
        font-size: 0.85rem;
        font-weight: 600;
        cursor: pointer;
    }
    .thread-reply-btn:hover { filter: brightness(1.15); }
    .thread-reply-btn:disabled { opacity: 0.5; cursor: not-allowed; }
    .thread-locked-notice {
        color: var(--text-3);
        font-size: 0.85rem;
        text-align: center;
        padding: 1rem;
        background: var(--surface);
        border: 1px solid var(--border);
        border-radius: 8px;
    }
    .thread-loading, .thread-error {
        color: var(--text-3);
        text-align: center;
        padding: 3rem 0;
    }
    .thread-no-replies {
        color: var(--text-3);
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
            {
                let data = detail.read();
                let data = data.as_ref().and_then(|d| d.as_ref());
                match data {
                    None => rsx! {
                        Link { to: Route::Forum {}, class: "thread-back", "< Back to Forum" }
                        p { class: "thread-loading", "Loading..." }
                    },
                    Some(resp) => {
                        let t = &resp.thread;
                        let date: String = t.created_at.chars().take(10).collect();
                        let reply_id = id.clone();
                        let board_slug = resp.board.as_ref().map(|b| b.slug.clone());
                        let board_name = resp
                            .board
                            .as_ref()
                            .map(|b| b.name.clone())
                            .unwrap_or_else(|| t.category.clone());
                        let cat_name = resp.category.as_ref().map(|c| c.name.clone());
                        let parent_name = resp.parent_board.as_ref().map(|b| b.name.clone());
                        let parent_slug = resp.parent_board.as_ref().map(|b| b.slug.clone());

                        rsx! {
                            div { class: "thread-back", style: "display:flex;flex-wrap:wrap;gap:0.35rem;align-items:center;",
                                Link { to: Route::Forum {}, "Forum" }
                                if let Some(cn) = cat_name {
                                    span { " / {cn}" }
                                }
                                if let (Some(ps), Some(pn)) = (parent_slug.clone(), parent_name) {
                                    span { " / " }
                                    Link { to: Route::ForumBoardPage { slug: ps }, "{pn}" }
                                }
                                if let Some(bs) = board_slug.clone() {
                                    span { " / " }
                                    Link { to: Route::ForumBoardPage { slug: bs }, "{board_name}" }
                                }
                            }

                            // Thread header
                            div { class: "thread-header",
                                div { class: "thread-title-row",
                                    if t.pinned {
                                        span { class: "thread-badge thread-badge-pin", "Pinned" }
                                    }
                                    if t.locked {
                                        span { class: "thread-badge thread-badge-locked", "Locked" }
                                    }
                                    span { class: "thread-badge thread-badge-cat", "{board_name}" }
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
