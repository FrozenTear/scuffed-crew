use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

use crate::components::{Toast, use_toast};
use crate::routes::Route;
use crate::state::auth::use_auth;
use scuffed_api_client::ApiClient;

#[derive(Debug, Clone, Deserialize, PartialEq)]
struct ForumCategory {
    id: String,
    name: String,
    slug: String,
    description: Option<String>,
    sort_order: i32,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
struct ForumBoard {
    id: String,
    category_id: String,
    parent_board_id: Option<String>,
    name: String,
    slug: String,
    description: Option<String>,
    sort_order: i32,
    is_locked: bool,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
struct ForumBoardNode {
    #[serde(flatten)]
    board: ForumBoard,
    sub_boards: Vec<ForumBoard>,
    thread_count: u64,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
struct ForumCategoryNode {
    #[serde(flatten)]
    category: ForumCategory,
    boards: Vec<ForumBoardNode>,
}

#[derive(Debug, Clone, Deserialize)]
struct ForumThread {
    id: String,
    title: String,
    #[allow(dead_code)]
    category: String,
    board_id: Option<String>,
    #[allow(dead_code)]
    author_member_id: String,
    pinned: bool,
    locked: bool,
    created_at: String,
    reply_count: u64,
}

#[derive(Debug, Clone, Deserialize)]
struct ThreadListResponse {
    threads: Vec<ForumThread>,
    #[allow(dead_code)]
    total: usize,
    board: Option<ForumBoard>,
}

#[derive(Serialize)]
struct CreateThreadBody {
    title: String,
    content: String,
    board: String,
}

const PAGE_CSS: &str = r#"
    .forum-page { padding: 2rem; max-width: 960px; margin: 0 auto; }
    .forum-page-title {
        font-family: var(--font-head); font-size: 2rem; color: var(--text);
        letter-spacing: 2px; margin: 0 0 0.35rem;
    }
    .forum-lead { color: var(--text-2); font-size: 0.9rem; margin: 0 0 1.5rem; }
    .forum-section { margin-bottom: 1.75rem; }
    .forum-section-title {
        font-family: var(--font-head); font-size: 0.85rem; letter-spacing: 0.12em;
        text-transform: uppercase; color: var(--accent); margin: 0 0 0.35rem;
    }
    .forum-section-desc { color: var(--text-3); font-size: 0.8rem; margin: 0 0 0.65rem; }
    .forum-board-list {
        border: 1px solid var(--border); border-radius: 8px; overflow: hidden;
        background: var(--surface);
    }
    .forum-board-row {
        display: grid; grid-template-columns: 1fr auto; gap: 1rem; align-items: center;
        padding: 0.85rem 1rem; border-bottom: 1px solid var(--border);
        text-decoration: none; color: inherit;
    }
    .forum-board-row:last-child { border-bottom: none; }
    .forum-board-row:hover { background: var(--surface-2); }
    .forum-board-row.sub { padding-left: 2rem; }
    .forum-board-name {
        font-family: var(--font-head); font-weight: 600; color: var(--text); font-size: 1rem;
    }
    .forum-board-desc { color: var(--text-3); font-size: 0.8rem; margin-top: 0.15rem; }
    .forum-board-meta { color: var(--text-3); font-size: 0.75rem; text-align: right; }
    .forum-locked { color: var(--text-3); font-size: 0.7rem; margin-left: 0.4rem; }
    .forum-toolbar { display: flex; justify-content: space-between; align-items: center; margin-bottom: 1rem; gap: 1rem; flex-wrap: wrap; }
    .forum-crumb { font-size: 0.8rem; color: var(--text-3); }
    .forum-crumb a { color: var(--text-2); text-decoration: none; }
    .forum-crumb a:hover { color: var(--accent); }
    .forum-new-btn {
        padding: 0.5rem 1rem; background: var(--accent); color: var(--accent-fg);
        border: none; border-radius: 6px; font-weight: 600; cursor: pointer; font-size: 0.85rem;
    }
    .forum-new-btn:disabled { opacity: 0.5; cursor: not-allowed; }
    .forum-thread-list { border: 1px solid var(--border); border-radius: 8px; overflow: hidden; }
    .forum-thread-row {
        display: block; padding: 0.85rem 1rem; border-bottom: 1px solid var(--border);
        text-decoration: none; color: inherit;
    }
    .forum-thread-row:last-child { border-bottom: none; }
    .forum-thread-row:hover { background: var(--surface-2); }
    .forum-thread-title { font-weight: 600; color: var(--text); }
    .forum-thread-meta { font-size: 0.75rem; color: var(--text-3); margin-top: 0.25rem; }
    .forum-badge {
        display: inline-block; font-size: 0.65rem; padding: 0.1rem 0.4rem;
        border-radius: 4px; margin-right: 0.35rem; text-transform: uppercase;
        letter-spacing: 0.04em; font-weight: 700;
    }
    .forum-badge-pin { background: color-mix(in srgb, var(--accent) 25%, transparent); color: var(--accent); }
    .forum-badge-lock { background: var(--surface-2); color: var(--text-3); }
    .forum-empty { color: var(--text-3); padding: 2rem; text-align: center; }
    .forum-compose {
        border: 1px solid var(--border); border-radius: 8px; padding: 1rem;
        margin-bottom: 1rem; background: var(--surface);
    }
    .forum-compose input, .forum-compose textarea, .forum-compose select {
        width: 100%; background: var(--bg); border: 1px solid var(--border);
        color: var(--text); border-radius: 6px; padding: 0.5rem 0.65rem; margin-bottom: 0.65rem;
        font: inherit;
    }
    .forum-compose textarea { min-height: 120px; resize: vertical; }
    .forum-compose-actions { display: flex; gap: 0.5rem; justify-content: flex-end; }
    .forum-btn-ghost {
        background: transparent; border: 1px solid var(--border); color: var(--text-2);
        padding: 0.45rem 0.85rem; border-radius: 6px; cursor: pointer;
    }
"#;

#[component]
pub fn Forum() -> Element {
    let tree = use_resource(|| async move {
        ApiClient::web()
            .fetch::<Vec<ForumCategoryNode>>("/api/forum/tree")
            .await
            .ok()
    });

    rsx! {
        style { {PAGE_CSS} }
        div { class: "forum-page",
            h1 { class: "forum-page-title", "Forum" }
            p { class: "forum-lead", "Old-school boards — pick a section, then a board (or sub-board)." }

            match tree.value()() {
                Some(Some(cats)) if !cats.is_empty() => rsx! {
                    for cat in cats.iter() {
                        div { class: "forum-section",
                            h2 { class: "forum-section-title", "{cat.category.name}" }
                            if let Some(desc) = &cat.category.description {
                                p { class: "forum-section-desc", "{desc}" }
                            }
                            div { class: "forum-board-list",
                                for node in cat.boards.iter() {
                                    Link {
                                        to: Route::ForumBoardPage { slug: node.board.slug.clone() },
                                        class: "forum-board-row",
                                        div {
                                            div { class: "forum-board-name",
                                                "{node.board.name}"
                                                if node.board.is_locked {
                                                    span { class: "forum-locked", "· locked" }
                                                }
                                            }
                                            if let Some(d) = &node.board.description {
                                                div { class: "forum-board-desc", "{d}" }
                                            }
                                        }
                                        div { class: "forum-board-meta", "{node.thread_count} threads" }
                                    }
                                    for sub in node.sub_boards.iter() {
                                        Link {
                                            to: Route::ForumBoardPage { slug: sub.slug.clone() },
                                            class: "forum-board-row sub",
                                            div {
                                                div { class: "forum-board-name", "↳ {sub.name}" }
                                                if let Some(d) = &sub.description {
                                                    div { class: "forum-board-desc", "{d}" }
                                                }
                                            }
                                            div { class: "forum-board-meta", "" }
                                        }
                                    }
                                }
                            }
                        }
                    }
                },
                Some(Some(_)) => rsx! {
                    p { class: "forum-empty", "No boards yet. An officer can create them in Admin → Forum." }
                },
                Some(None) => rsx! {
                    p { class: "forum-empty",
                        "Couldn't load the forum. Check your connection and try refreshing."
                    }
                },
                None => rsx! { p { class: "forum-empty", "Loading…" } },
            }
        }
    }
}

#[component]
pub fn ForumBoardPage(slug: String) -> Element {
    let auth = use_auth();
    let mut toast = use_toast();
    let mut show_compose = use_signal(|| false);
    let mut title = use_signal(String::new);
    let mut content = use_signal(String::new);
    let mut refresh = use_signal(|| 0u32);
    let slug_for_fetch = slug.clone();

    let list = use_resource(move || {
        let slug = slug_for_fetch.clone();
        let _ = refresh();
        async move {
            ApiClient::web()
                .fetch::<ThreadListResponse>(&format!("/api/forum/threads?board={slug}&limit=50"))
                .await
                .ok()
        }
    });

    let is_member = auth().is_logged_in();
    let board_locked = list.value()()
        .flatten()
        .and_then(|r| r.board.clone())
        .map(|b| b.is_locked)
        .unwrap_or(false);

    rsx! {
        style { {PAGE_CSS} }
        div { class: "forum-page",
            div { class: "forum-toolbar",
                div { class: "forum-crumb",
                    Link { to: Route::Forum {}, "Forum" }
                    " / "
                    {
                        let name = list.value()().flatten()
                            .and_then(|r| r.board.map(|b| b.name))
                            .unwrap_or_else(|| slug.clone());
                        rsx! { span { "{name}" } }
                    }
                }
                if is_member && !board_locked {
                    button {
                        class: "forum-new-btn",
                        onclick: move |_| show_compose.set(true),
                        "New thread"
                    }
                }
            }

            if show_compose() {
                div { class: "forum-compose",
                    input {
                        placeholder: "Thread title",
                        value: "{title}",
                        oninput: move |e| title.set(e.value()),
                    }
                    textarea {
                        placeholder: "Write your post…",
                        value: "{content}",
                        oninput: move |e| content.set(e.value()),
                    }
                    div { class: "forum-compose-actions",
                        button {
                            class: "forum-btn-ghost",
                            onclick: move |_| show_compose.set(false),
                            "Cancel"
                        }
                        button {
                            class: "forum-new-btn",
                            onclick: move |_| {
                                let slug = slug.clone();
                                let t = title().trim().to_string();
                                let c = content().trim().to_string();
                                if t.is_empty() || c.is_empty() {
                                    toast.show(Toast::error("Title and content required"));
                                    return;
                                }
                                spawn(async move {
                                    let body = CreateThreadBody {
                                        title: t,
                                        content: c,
                                        board: slug,
                                    };
                                    match ApiClient::web()
                                        .post_json::<_, serde_json::Value>("/api/forum/threads", &body)
                                        .await
                                    {
                                        Ok(_) => {
                                            title.set(String::new());
                                            content.set(String::new());
                                            show_compose.set(false);
                                            refresh.set(refresh() + 1);
                                            toast.show(Toast::success("Thread created"));
                                        }
                                        Err(e) => toast.show(Toast::error(e.to_string())),
                                    }
                                });
                            },
                            "Post"
                        }
                    }
                }
            }

            match list.value()() {
                Some(Some(resp)) => {
                    if resp.threads.is_empty() {
                        rsx! { p { class: "forum-empty", "No threads yet. Start one." } }
                    } else {
                        rsx! {
                            div { class: "forum-thread-list",
                                for t in resp.threads.iter() {
                                    {
                                        let tid = t.id.clone();
                                        rsx! {
                                            Link {
                                                to: Route::ForumThread { id: tid },
                                                class: "forum-thread-row",
                                                div { class: "forum-thread-title",
                                                    if t.pinned {
                                                        span { class: "forum-badge forum-badge-pin", "Pinned" }
                                                    }
                                                    if t.locked {
                                                        span { class: "forum-badge forum-badge-lock", "Locked" }
                                                    }
                                                    "{t.title}"
                                                }
                                                div { class: "forum-thread-meta",
                                                    "{t.reply_count} replies · {t.created_at}"
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                Some(None) => rsx! { p { class: "forum-empty", "Board not found or failed to load." } },
                None => rsx! { p { class: "forum-empty", "Loading…" } },
            }
        }
    }
}
