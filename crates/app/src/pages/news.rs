use dioxus::prelude::*;
use serde::Deserialize;

use scuffed_api_client::ApiClient;

#[derive(Debug, Clone, Deserialize)]
struct Announcement {
    #[allow(dead_code)]
    id: String,
    title: String,
    content: String,
    pinned: bool,
    created_at: String,
}

const PAGE_CSS: &str = r#"
    .news-page {
        padding: 3rem 2rem;
        max-width: 800px;
        margin: 0 auto;
    }
    .news-page-title {
        font-family: 'Bebas Neue', sans-serif;
        font-size: 2.5rem;
        color: var(--text-bright);
        letter-spacing: 3px;
        margin: 0 0 2rem;
    }
    .news-list {
        display: flex;
        flex-direction: column;
        gap: 1.25rem;
    }
    .news-card {
        background: var(--bg-card);
        border: 1px solid var(--border);
        border-radius: 10px;
        padding: 1.5rem;
    }
    .news-card.pinned {
        border-color: var(--accent-soft);
    }
    .news-meta {
        display: flex;
        align-items: center;
        gap: 0.5rem;
        font-size: 0.7rem;
        color: var(--text-muted);
        margin-bottom: 0.5rem;
    }
    .news-pin {
        background: #7c3aed33;
        color: #a78bfa;
        padding: 0.1rem 0.4rem;
        border-radius: 4px;
        font-size: 0.6rem;
        font-weight: 600;
        text-transform: uppercase;
    }
    .news-title {
        font-family: 'Rajdhani', sans-serif;
        font-size: 1.2rem;
        font-weight: 700;
        color: var(--text-bright);
        margin: 0 0 0.5rem;
    }
    .news-body {
        color: var(--text-secondary);
        font-size: 0.85rem;
        line-height: 1.7;
        margin: 0;
    }
    .news-loading, .news-empty {
        color: var(--text-muted);
        text-align: center;
        padding: 3rem 0;
    }
"#;

#[component]
pub fn News() -> Element {
    let announcements = use_resource(|| async {
        ApiClient::web().fetch::<Vec<Announcement>>("/api/announcements").await.ok()
    });

    rsx! {
        style { {PAGE_CSS} }

        main { class: "news-page",
            h1 { class: "news-page-title", "News & Announcements" }

            {
                let data = announcements.read();
                let data = data.as_ref().and_then(|d| d.as_ref());
                match data {
                    None => rsx! { p { class: "news-loading", "Loading..." } },
                    Some(list) if list.is_empty() => rsx! {
                        p { class: "news-empty", "No announcements yet." }
                    },
                    Some(list) => rsx! {
                        div { class: "news-list",
                            for a in list.iter() {
                                {render_news_card(a)}
                            }
                        }
                    },
                }
            }
        }
    }
}

fn render_news_card(a: &Announcement) -> Element {
    let date: String = a.created_at.chars().take(10).collect();
    let card_class = if a.pinned { "news-card pinned" } else { "news-card" };

    rsx! {
        article { class: "{card_class}",
            div { class: "news-meta",
                time { "{date}" }
                if a.pinned {
                    span { class: "news-pin", "Pinned" }
                }
            }
            h2 { class: "news-title", "{a.title}" }
            p { class: "news-body", "{a.content}" }
        }
    }
}
