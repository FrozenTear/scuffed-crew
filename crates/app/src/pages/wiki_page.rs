use dioxus::prelude::*;
use serde::Deserialize;

use crate::routes::Route;
use crate::state::auth::use_auth;
use scuffed_api_client::ApiClient;

#[derive(Debug, Clone, Deserialize)]
struct WikiPageData {
    #[allow(dead_code)]
    id: String,
    topic: String,
    title: String,
    content_markdown: String,
    #[allow(dead_code)]
    author_member_id: String,
    created_at: String,
    updated_at: String,
    #[allow(dead_code)]
    is_active: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct WikiRevisionData {
    #[allow(dead_code)]
    id: String,
    #[allow(dead_code)]
    page_id: String,
    #[allow(dead_code)]
    content_markdown: String,
    #[allow(dead_code)]
    edited_by: String,
    edited_at: String,
    revision_note: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct WikiRevisionsResponse {
    data: Vec<WikiRevisionData>,
}

const PAGE_CSS: &str = r#"
    .wiki-detail {
        padding: 3rem 2rem;
        max-width: 800px;
        margin: 0 auto;
    }
    .wiki-detail-header {
        margin-bottom: 2rem;
    }
    .wiki-detail-title {
        font-family: 'Bebas Neue', sans-serif;
        font-size: 2.2rem;
        color: var(--text-bright);
        letter-spacing: 2px;
        margin: 0 0 0.5rem;
    }
    .wiki-detail-meta {
        display: flex;
        gap: 1rem;
        font-size: 0.75rem;
        color: var(--text-muted);
        align-items: center;
        flex-wrap: wrap;
    }
    .wiki-detail-topic {
        font-family: monospace;
        color: #a78bfa;
        font-size: 0.75rem;
    }
    .wiki-detail-actions {
        display: flex;
        gap: 0.5rem;
        margin-top: 0.75rem;
    }
    .wiki-detail-actions a,
    .wiki-detail-actions button {
        padding: 0.35rem 0.75rem;
        font-size: 0.75rem;
        border-radius: 6px;
        cursor: pointer;
        text-decoration: none;
        transition: background 0.15s;
    }
    .wiki-btn-edit {
        background: var(--accent);
        color: white;
        border: none;
        font-weight: 600;
    }
    .wiki-btn-edit:hover {
        filter: brightness(1.15);
    }
    .wiki-btn-secondary {
        background: var(--bg-card);
        color: var(--text-secondary);
        border: 1px solid var(--border);
    }
    .wiki-btn-secondary:hover {
        border-color: var(--accent-soft);
        color: var(--text-bright);
    }
    .wiki-content {
        background: var(--bg-card);
        border: 1px solid var(--border);
        border-radius: 10px;
        padding: 1.5rem;
    }
    .wiki-content pre {
        white-space: pre-wrap;
        word-wrap: break-word;
        color: var(--text-secondary);
        font-size: 0.85rem;
        line-height: 1.7;
        margin: 0;
        font-family: inherit;
    }
    .wiki-revisions {
        margin-top: 2rem;
    }
    .wiki-revisions-title {
        font-family: 'Rajdhani', sans-serif;
        font-size: 1.1rem;
        font-weight: 700;
        color: var(--text-bright);
        margin: 0 0 0.75rem;
    }
    .wiki-revision-list {
        display: flex;
        flex-direction: column;
        gap: 0.5rem;
    }
    .wiki-revision-item {
        background: var(--bg-card);
        border: 1px solid var(--border);
        border-radius: 8px;
        padding: 0.75rem 1rem;
        font-size: 0.8rem;
        display: flex;
        justify-content: space-between;
        align-items: center;
        flex-wrap: wrap;
        gap: 0.5rem;
    }
    .wiki-revision-note {
        color: var(--text-secondary);
    }
    .wiki-revision-date {
        color: var(--text-muted);
        font-size: 0.7rem;
    }
    .wiki-edit-form {
        margin-top: 1.5rem;
    }
    .wiki-edit-form textarea {
        width: 100%;
        min-height: 300px;
        padding: 1rem;
        background: var(--bg-card);
        border: 1px solid var(--border);
        border-radius: 8px;
        color: var(--text-bright);
        font-size: 0.85rem;
        line-height: 1.6;
        font-family: monospace;
        resize: vertical;
    }
    .wiki-edit-form textarea:focus {
        outline: none;
        border-color: var(--accent-soft);
    }
    .wiki-edit-form input {
        width: 100%;
        padding: 0.5rem 0.75rem;
        margin-top: 0.5rem;
        background: var(--bg-card);
        border: 1px solid var(--border);
        border-radius: 6px;
        color: var(--text-bright);
        font-size: 0.8rem;
    }
    .wiki-edit-form input:focus {
        outline: none;
        border-color: var(--accent-soft);
    }
    .wiki-edit-form input::placeholder {
        color: var(--text-muted);
    }
    .wiki-edit-buttons {
        display: flex;
        gap: 0.5rem;
        margin-top: 0.75rem;
    }
    .wiki-loading, .wiki-error {
        color: var(--text-muted);
        text-align: center;
        padding: 3rem 0;
    }
    .wiki-back {
        display: inline-block;
        color: var(--text-muted);
        font-size: 0.8rem;
        margin-bottom: 1rem;
        text-decoration: none;
    }
    .wiki-back:hover {
        color: var(--text-bright);
    }
"#;

#[component]
pub fn WikiPage(topic: String) -> Element {
    let auth = use_auth();
    let topic_clone = topic.clone();
    let page_resource = use_resource(move || {
        let t = topic_clone.clone();
        async move {
            ApiClient::web()
                .fetch::<WikiPageData>(&format!("/api/wiki/{t}"))
                .await
                .ok()
        }
    });

    let topic_for_revisions = topic.clone();
    let mut show_revisions = use_signal(|| false);
    let revisions = use_resource(move || {
        let t = topic_for_revisions.clone();
        let show = show_revisions();
        async move {
            if !show {
                return None;
            }
            ApiClient::web()
                .fetch::<WikiRevisionsResponse>(&format!("/api/wiki/{t}/revisions"))
                .await
                .ok()
        }
    });

    let mut editing = use_signal(|| false);
    let mut edit_content = use_signal(String::new);
    let mut revision_note = use_signal(String::new);
    let mut save_error = use_signal(|| Option::<String>::None);

    rsx! {
        style { {PAGE_CSS} }

        main { class: "wiki-detail",
            Link { to: Route::Wiki {}, class: "wiki-back", "< Back to Wiki" }

            {
                let data = page_resource.read();
                let data = data.as_ref().and_then(|d| d.as_ref());
                match data {
                    None => rsx! { p { class: "wiki-loading", "Loading..." } },
                    Some(page) => {
                        let created: String = page.created_at.chars().take(10).collect();
                        let updated: String = page.updated_at.chars().take(10).collect();
                        let content = page.content_markdown.clone();
                        let page_topic = page.topic.clone();

                        rsx! {
                            div { class: "wiki-detail-header",
                                h1 { class: "wiki-detail-title", "{page.title}" }
                                div { class: "wiki-detail-meta",
                                    span { class: "wiki-detail-topic", "{page.topic}" }
                                    span { "Created {created}" }
                                    span { "Updated {updated}" }
                                }
                                div { class: "wiki-detail-actions",
                                    if auth().is_logged_in() && !editing() {
                                        button {
                                            class: "wiki-btn-edit",
                                            onclick: {
                                                let c = content.clone();
                                                move |_| {
                                                    edit_content.set(c.clone());
                                                    revision_note.set(String::new());
                                                    editing.set(true);
                                                }
                                            },
                                            "Edit"
                                        }
                                    }
                                    button {
                                        class: "wiki-btn-secondary",
                                        onclick: move |_| {
                                            show_revisions.set(!show_revisions());
                                        },
                                        if show_revisions() { "Hide History" } else { "History" }
                                    }
                                }
                            }

                            if editing() {
                                div { class: "wiki-edit-form",
                                    textarea {
                                        value: "{edit_content}",
                                        oninput: move |e| edit_content.set(e.value()),
                                    }
                                    input {
                                        r#type: "text",
                                        placeholder: "Revision note (optional)",
                                        value: "{revision_note}",
                                        oninput: move |e| revision_note.set(e.value()),
                                    }
                                    if let Some(err) = save_error() {
                                        p { style: "color: #ef4444; font-size: 0.8rem; margin-top: 0.5rem;",
                                            "{err}"
                                        }
                                    }
                                    div { class: "wiki-edit-buttons",
                                        button {
                                            class: "wiki-btn-edit",
                                            onclick: {
                                                let pt = page_topic.clone();
                                                move |_| {
                                                    let pt = pt.clone();
                                                    let content = edit_content();
                                                    let note = revision_note();
                                                    let note_opt = if note.is_empty() { None } else { Some(note) };
                                                    spawn(async move {
                                                        #[derive(serde::Serialize)]
                                                        struct UpdateBody {
                                                            content_markdown: String,
                                                            revision_note: Option<String>,
                                                        }
                                                        let body = UpdateBody {
                                                            content_markdown: content,
                                                            revision_note: note_opt,
                                                        };
                                                        match ApiClient::web()
                                                            .put_json::<_, WikiPageData>(
                                                                &format!("/api/wiki/{pt}"),
                                                                &body,
                                                            )
                                                            .await
                                                        {
                                                            Ok(_) => {
                                                                editing.set(false);
                                                                save_error.set(None);
                                                                // Refresh by navigating
                                                                navigator().push(Route::WikiPage { topic: pt });
                                                            }
                                                            Err(e) => {
                                                                save_error.set(Some(format!("{e}")));
                                                            }
                                                        }
                                                    });
                                                }
                                            },
                                            "Save"
                                        }
                                        button {
                                            class: "wiki-btn-secondary",
                                            onclick: move |_| {
                                                editing.set(false);
                                                save_error.set(None);
                                            },
                                            "Cancel"
                                        }
                                    }
                                }
                            } else {
                                div { class: "wiki-content",
                                    pre { "{content}" }
                                }
                            }

                            if show_revisions() {
                                div { class: "wiki-revisions",
                                    h3 { class: "wiki-revisions-title", "Revision History" }
                                    {
                                        let rev_data = revisions.read();
                                        let rev_data = rev_data.as_ref().and_then(|d| d.as_ref());
                                        match rev_data {
                                            None => rsx! { p { class: "wiki-loading", "Loading revisions..." } },
                                            Some(resp) if resp.data.is_empty() => rsx! {
                                                p { class: "wiki-loading", "No revisions yet." }
                                            },
                                            Some(resp) => rsx! {
                                                div { class: "wiki-revision-list",
                                                    for rev in resp.data.iter() {
                                                        {render_revision(rev)}
                                                    }
                                                }
                                            },
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

fn render_revision(rev: &WikiRevisionData) -> Element {
    let date: String = rev.edited_at.chars().take(16).collect();
    let note = rev.revision_note.as_deref().unwrap_or("No note");

    rsx! {
        div { class: "wiki-revision-item",
            span { class: "wiki-revision-note", "{note}" }
            span { class: "wiki-revision-date", "{date}" }
        }
    }
}
