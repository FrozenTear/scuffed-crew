use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

use crate::components::{Toast, use_toast};
use crate::routes::Route;
use crate::state::auth::use_auth;
use scuffed_api_client::ApiClient;

#[derive(Debug, Clone, Deserialize)]
struct ForumThread {
    id: String,
    title: String,
    category: String,
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
}

#[derive(Serialize)]
struct CreateThreadBody {
    title: String,
    content: String,
    category: String,
}

const CATEGORIES: &[(&str, &str)] = &[
    ("all", "All"),
    ("general", "General"),
    ("game", "Game"),
    ("strategy", "Strategy"),
    ("offtopic", "Off-Topic"),
];

const PAGE_CSS: &str = r#"
    .forum-page {
        padding: 3rem 2rem;
        max-width: 900px;
        margin: 0 auto;
    }
    .forum-page-title {
        font-family: var(--font-head);
        font-size: 2.5rem;
        color: var(--text);
        letter-spacing: 3px;
        margin: 0 0 1.5rem;
    }
    .forum-tabs {
        display: flex;
        gap: 0.25rem;
        margin-bottom: 1.5rem;
        border-bottom: 1px solid var(--border);
        padding-bottom: 0;
    }
    .forum-tab {
        padding: 0.5rem 1rem;
        background: none;
        border: none;
        border-bottom: 2px solid transparent;
        color: var(--text-2);
        font-size: 0.85rem;
        cursor: pointer;
        transition: color 0.15s, border-color 0.15s;
        font-family: var(--font-head);
        font-weight: 600;
        text-transform: uppercase;
        letter-spacing: 0.04em;
    }
    .forum-tab:hover {
        color: var(--text);
    }
    .forum-tab.active {
        color: var(--text);
        border-bottom-color: var(--accent);
    }
    .forum-toolbar {
        display: flex;
        justify-content: flex-end;
        margin-bottom: 1rem;
    }
    .forum-new-btn {
        display: inline-flex;
        align-items: center;
        gap: 0.4rem;
        padding: 0.5rem 1rem;
        background: var(--accent);
        color: var(--accent-fg);
        border: none;
        border-radius: 6px;
        font-size: 0.85rem;
        font-weight: 600;
        cursor: pointer;
        transition: filter 0.15s;
    }
    .forum-new-btn:hover {
        filter: brightness(1.15);
    }
    .forum-list {
        display: flex;
        flex-direction: column;
        gap: 0.5rem;
    }
    .forum-thread-card {
        display: flex;
        align-items: center;
        gap: 1rem;
        padding: 1rem 1.25rem;
        background: var(--surface);
        border: 1px solid var(--border);
        border-radius: 8px;
        text-decoration: none;
        transition: border-color 0.2s;
    }
    .forum-thread-card:hover {
        border-color: var(--accent-soft);
    }
    .forum-thread-card.pinned {
        border-color: var(--accent-soft);
        background: color-mix(in srgb, var(--accent) 4%, var(--surface));
    }
    .forum-thread-info {
        flex: 1;
        min-width: 0;
    }
    .forum-thread-title-row {
        display: flex;
        align-items: center;
        gap: 0.5rem;
        margin-bottom: 0.25rem;
    }
    .forum-thread-title {
        font-family: var(--font-head);
        font-weight: 700;
        font-size: 1rem;
        color: var(--text);
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }
    .forum-badge {
        font-size: 0.6rem;
        padding: 0.1rem 0.4rem;
        border-radius: 4px;
        font-weight: 600;
        text-transform: uppercase;
        white-space: nowrap;
    }
    .forum-badge-pin {
        background: color-mix(in srgb, var(--accent) 20%, transparent);
        color: var(--accent);
    }
    .forum-badge-locked {
        background: color-mix(in srgb, var(--danger) 20%, transparent);
        color: var(--danger);
    }
    .forum-badge-cat {
        background: color-mix(in srgb, var(--text-2) 15%, transparent);
        color: var(--text-2);
    }
    .forum-thread-meta {
        font-size: 0.7rem;
        color: var(--text-3);
    }
    .forum-thread-replies {
        text-align: center;
        min-width: 60px;
    }
    .forum-thread-replies-count {
        font-family: var(--font-head);
        font-weight: 700;
        font-size: 1.1rem;
        color: var(--text-2);
    }
    .forum-thread-replies-label {
        font-size: 0.65rem;
        color: var(--text-3);
        text-transform: uppercase;
    }
    .forum-empty, .forum-loading {
        color: var(--text-3);
        text-align: center;
        padding: 3rem 0;
    }
    .forum-compose {
        background: var(--surface);
        border: 1px solid var(--border);
        border-radius: 10px;
        padding: 1.5rem;
        margin-bottom: 1.5rem;
    }
    .forum-compose-title {
        font-family: var(--font-head);
        font-weight: 700;
        font-size: 1.1rem;
        color: var(--text);
        margin: 0 0 1rem;
    }
    .forum-field {
        margin-bottom: 1rem;
    }
    .forum-label {
        display: block;
        font-size: 0.75rem;
        color: var(--text-3);
        text-transform: uppercase;
        letter-spacing: 0.04em;
        margin-bottom: 0.35rem;
        font-weight: 600;
    }
    .forum-input, .forum-textarea, .forum-select {
        width: 100%;
        background: var(--bg);
        border: 1px solid var(--border);
        border-radius: 6px;
        color: var(--text);
        padding: 0.6rem 0.75rem;
        font-size: 0.9rem;
        font-family: inherit;
    }
    .forum-input:focus, .forum-textarea:focus, .forum-select:focus {
        outline: none;
        border-color: var(--accent);
    }
    .forum-textarea {
        resize: vertical;
        min-height: 80px;
    }
    .forum-compose-actions {
        display: flex;
        gap: 0.75rem;
        justify-content: flex-end;
    }
    .forum-cancel-btn {
        padding: 0.5rem 1rem;
        background: var(--bg);
        color: var(--text-2);
        border: 1px solid var(--border);
        border-radius: 6px;
        font-size: 0.85rem;
        cursor: pointer;
    }
    .forum-submit-btn {
        padding: 0.5rem 1rem;
        background: var(--accent);
        color: var(--accent-fg);
        border: none;
        border-radius: 6px;
        font-size: 0.85rem;
        font-weight: 600;
        cursor: pointer;
    }
    .forum-submit-btn:hover { filter: brightness(1.15); }
    .forum-submit-btn:disabled { opacity: 0.5; cursor: not-allowed; }
"#;

#[component]
pub fn Forum() -> Element {
    let auth = use_auth();
    let mut active_tab = use_signal(|| "all".to_string());
    let mut show_compose = use_signal(|| false);
    let mut refresh = use_signal(|| 0u64);

    let mut compose_title = use_signal(String::new);
    let mut compose_content = use_signal(String::new);
    let mut compose_category = use_signal(|| "general".to_string());
    let mut submitting = use_signal(|| false);

    let tab = active_tab();
    let threads = use_resource(move || {
        let tab = tab.clone();
        let _ = refresh();
        async move {
            let cat_param = if tab == "all" {
                String::new()
            } else {
                format!("&category={tab}")
            };
            let url = format!("/api/forum/threads?limit=50{cat_param}");
            ApiClient::web()
                .fetch::<ThreadListResponse>(&url)
                .await
                .ok()
        }
    });

    rsx! {
        style { {PAGE_CSS} }

        main { class: "forum-page",
            h1 { class: "forum-page-title", "Forum" }

            // Category tabs
            div { class: "forum-tabs",
                for &(key, label) in CATEGORIES.iter() {
                    {
                        let k = key.to_string();
                        let is_active = active_tab() == k;
                        let cls = if is_active { "forum-tab active" } else { "forum-tab" };
                        rsx! {
                            button {
                                class: "{cls}",
                                onclick: move |_| {
                                    active_tab.set(k.clone());
                                },
                                "{label}"
                            }
                        }
                    }
                }
            }

            // New thread button
            if auth().is_logged_in() {
                div { class: "forum-toolbar",
                    button {
                        class: "forum-new-btn",
                        onclick: move |_| show_compose.toggle(),
                        if show_compose() { "Cancel" } else { "+ New Thread" }
                    }
                }
            }

            // Compose form
            if show_compose() {
                div { class: "forum-compose",
                    h3 { class: "forum-compose-title", "Create New Thread" }
                    div { class: "forum-field",
                        label { class: "forum-label", "Title" }
                        input {
                            class: "forum-input",
                            r#type: "text",
                            placeholder: "Thread title...",
                            value: "{compose_title}",
                            oninput: move |e| compose_title.set(e.value()),
                        }
                    }
                    div { class: "forum-field",
                        label { class: "forum-label", "Category" }
                        select {
                            class: "forum-select",
                            value: "{compose_category}",
                            onchange: move |e| compose_category.set(e.value()),
                            option { value: "general", "General" }
                            option { value: "game", "Game" }
                            option { value: "strategy", "Strategy" }
                            option { value: "offtopic", "Off-Topic" }
                        }
                    }
                    div { class: "forum-field",
                        label { class: "forum-label", "Content" }
                        textarea {
                            class: "forum-textarea",
                            rows: 5,
                            placeholder: "What's on your mind?",
                            value: "{compose_content}",
                            oninput: move |e| compose_content.set(e.value()),
                        }
                    }
                    div { class: "forum-compose-actions",
                        button {
                            class: "forum-cancel-btn",
                            onclick: move |_| show_compose.set(false),
                            "Cancel"
                        }
                        button {
                            class: "forum-submit-btn",
                            disabled: submitting(),
                            onclick: move |_| {
                                let title = compose_title();
                                let content = compose_content();
                                let category = compose_category();
                                if title.trim().is_empty() || content.trim().is_empty() {
                                    let mut toast = use_toast();
                                    toast.show(Toast::error("Title and content are required"));
                                    return;
                                }
                                submitting.set(true);
                                spawn(async move {
                                    let body = CreateThreadBody { title, content, category };
                                    match ApiClient::web().post_json::<_, serde_json::Value>("/api/forum/threads", &body).await {
                                        Ok(_) => {
                                            let mut toast = use_toast();
                                            toast.show(Toast::success("Thread created!"));
                                            show_compose.set(false);
                                            compose_title.set(String::new());
                                            compose_content.set(String::new());
                                            compose_category.set("general".to_string());
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
                            if submitting() { "Posting..." } else { "Post Thread" }
                        }
                    }
                }
            }

            // Thread list
            {
                let data = threads.read();
                let data = data.as_ref().and_then(|d| d.as_ref());
                match data {
                    None => rsx! { p { class: "forum-loading", "Loading..." } },
                    Some(resp) if resp.threads.is_empty() => rsx! {
                        p { class: "forum-empty", "No threads yet. Be the first to start a discussion!" }
                    },
                    Some(resp) => rsx! {
                        div { class: "forum-list",
                            for t in resp.threads.iter() {
                                {render_thread_card(t)}
                            }
                        }
                    },
                }
            }
        }
    }
}

fn render_thread_card(t: &ForumThread) -> Element {
    let date: String = t.created_at.chars().take(10).collect();
    let card_class = if t.pinned {
        "forum-thread-card pinned"
    } else {
        "forum-thread-card"
    };

    let cat_label = match t.category.as_str() {
        "general" => "General",
        "game" => "Game",
        "strategy" => "Strategy",
        "offtopic" => "Off-Topic",
        other => other,
    };

    rsx! {
        Link {
            to: Route::ForumThread { id: t.id.clone() },
            class: "{card_class}",
            div { class: "forum-thread-info",
                div { class: "forum-thread-title-row",
                    if t.pinned {
                        span { class: "forum-badge forum-badge-pin", "Pinned" }
                    }
                    if t.locked {
                        span { class: "forum-badge forum-badge-locked", "Locked" }
                    }
                    span { class: "forum-badge forum-badge-cat", "{cat_label}" }
                    span { class: "forum-thread-title", "{t.title}" }
                }
                div { class: "forum-thread-meta",
                    "{date}"
                }
            }
            div { class: "forum-thread-replies",
                div { class: "forum-thread-replies-count", "{t.reply_count}" }
                div { class: "forum-thread-replies-label", "replies" }
            }
        }
    }
}
