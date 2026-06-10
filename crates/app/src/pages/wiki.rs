use dioxus::prelude::*;
use serde::Deserialize;

use crate::routes::Route;
use scuffed_api_client::ApiClient;

fn encode_uri(s: &str) -> String {
    js_sys::encode_uri_component(s)
        .as_string()
        .unwrap_or_default()
}

#[derive(Debug, Clone, Deserialize)]
struct WikiPageSummary {
    #[allow(dead_code)]
    id: String,
    topic: String,
    title: String,
    #[allow(dead_code)]
    content_markdown: String,
    #[allow(dead_code)]
    author_member_id: String,
    #[allow(dead_code)]
    created_at: String,
    updated_at: String,
    #[allow(dead_code)]
    is_active: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct WikiListResponse {
    data: Vec<WikiPageSummary>,
}

const PAGE_CSS: &str = r#"
    .wiki-page {
        padding: 3rem 2rem;
        max-width: 900px;
        margin: 0 auto;
    }
    .wiki-header {
        display: flex;
        align-items: center;
        justify-content: space-between;
        margin-bottom: 2rem;
        flex-wrap: wrap;
        gap: 1rem;
    }
    .wiki-page-title {
        font-family: var(--font-head);
        font-size: 2.5rem;
        color: var(--text);
        letter-spacing: 3px;
        margin: 0;
    }
    .wiki-search {
        display: flex;
        gap: 0.5rem;
        width: 100%;
        max-width: 400px;
    }
    .wiki-search input {
        flex: 1;
        padding: 0.5rem 0.75rem;
        background: var(--surface);
        border: 1px solid var(--border);
        border-radius: 6px;
        color: var(--text);
        font-size: 0.85rem;
    }
    .wiki-search input::placeholder {
        color: var(--text-3);
    }
    .wiki-search input:focus {
        outline: none;
        border-color: var(--accent-soft);
    }
    .wiki-list {
        display: flex;
        flex-direction: column;
        gap: 0.75rem;
    }
    .wiki-card {
        background: var(--surface);
        border: 1px solid var(--border);
        border-radius: 10px;
        padding: 1.25rem;
        text-decoration: none;
        display: flex;
        flex-direction: column;
        gap: 0.35rem;
        transition: border-color 0.2s, transform 0.2s;
    }
    .wiki-card:hover {
        border-color: var(--accent-soft);
        transform: translateY(-1px);
    }
    .wiki-card-title {
        font-family: var(--font-head);
        font-weight: 700;
        font-size: 1.1rem;
        color: var(--text);
    }
    .wiki-card-meta {
        display: flex;
        gap: 0.75rem;
        font-size: 0.7rem;
        color: var(--text-3);
    }
    .wiki-card-topic {
        font-size: 0.75rem;
        color: var(--accent);
        font-family: var(--font-mono);
    }
    .wiki-loading, .wiki-empty {
        color: var(--text-3);
        text-align: center;
        padding: 3rem 0;
    }
"#;

#[component]
pub fn Wiki() -> Element {
    let mut search_text = use_signal(String::new);

    let pages = use_resource(move || {
        let q = search_text();
        async move {
            let url = if q.is_empty() {
                "/api/wiki".to_string()
            } else {
                format!("/api/wiki?q={}", encode_uri(&q))
            };
            ApiClient::web().fetch::<WikiListResponse>(&url).await.ok()
        }
    });

    rsx! {
        style { {PAGE_CSS} }

        main { class: "wiki-page",
            div { class: "wiki-header",
                h1 { class: "wiki-page-title", "Knowledge Base" }
            }

            div { class: "wiki-search",
                input {
                    r#type: "text",
                    placeholder: "Search wiki...",
                    value: "{search_text}",
                    oninput: move |e| search_text.set(e.value()),
                }
            }

            div { style: "margin-top: 1.5rem;",
                {
                    let data = pages.read();
                    let data = data.as_ref().and_then(|d| d.as_ref());
                    match data {
                        None => rsx! { p { class: "wiki-loading", "Loading..." } },
                        Some(resp) if resp.data.is_empty() => rsx! {
                            p { class: "wiki-empty", "No wiki pages found." }
                        },
                        Some(resp) => rsx! {
                            div { class: "wiki-list",
                                for page in resp.data.iter() {
                                    {render_wiki_card(page)}
                                }
                            }
                        },
                    }
                }
            }
        }
    }
}

fn render_wiki_card(page: &WikiPageSummary) -> Element {
    let date: String = page.updated_at.chars().take(10).collect();

    rsx! {
        Link { to: Route::WikiPage { topic: page.topic.clone() }, class: "wiki-card",
            div { class: "wiki-card-title", "{page.title}" }
            div { class: "wiki-card-meta",
                span { class: "wiki-card-topic", "{page.topic}" }
                span { "Updated {date}" }
            }
        }
    }
}
