use dioxus::prelude::*;
use serde::Deserialize;

use scuffed_api_client::ApiClient;

use crate::routes::Route;

#[derive(Debug, Clone, Deserialize)]
struct Article {
    slug: String,
    title: String,
    summary: Option<String>,
    cover_image_url: Option<String>,
    author_member_id: String,
    published_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct ArticleListResponse {
    articles: Vec<Article>,
    total: u64,
}

const PAGE_CSS: &str = r#"
    .blog-page {
        padding: 3rem 2rem;
        max-width: 900px;
        margin: 0 auto;
    }
    .blog-page-title {
        font-family: 'Bebas Neue', sans-serif;
        font-size: 2.5rem;
        color: var(--text-bright);
        letter-spacing: 3px;
        margin: 0 0 2rem;
    }
    .blog-grid {
        display: grid;
        grid-template-columns: 1fr;
        gap: 1.5rem;
    }
    .blog-card {
        background: var(--bg-card);
        border: 1px solid var(--border);
        border-radius: 12px;
        overflow: hidden;
        transition: border-color 0.2s, transform 0.2s;
        cursor: pointer;
        text-decoration: none;
        color: inherit;
        display: block;
    }
    .blog-card:hover {
        border-color: var(--accent-soft);
        transform: translateY(-2px);
    }
    .blog-card-cover {
        width: 100%;
        height: 200px;
        object-fit: cover;
        display: block;
    }
    .blog-card-body {
        padding: 1.25rem;
    }
    .blog-card-meta {
        font-size: 0.7rem;
        color: var(--text-muted);
        margin-bottom: 0.5rem;
    }
    .blog-card-title {
        font-family: 'Rajdhani', sans-serif;
        font-size: 1.3rem;
        font-weight: 700;
        color: var(--text-bright);
        margin: 0 0 0.5rem;
    }
    .blog-card-summary {
        color: var(--text-secondary);
        font-size: 0.85rem;
        line-height: 1.6;
        margin: 0;
    }
    .blog-loading, .blog-empty {
        color: var(--text-muted);
        text-align: center;
        padding: 3rem 0;
    }
    .blog-pagination {
        display: flex;
        justify-content: center;
        align-items: center;
        gap: 1rem;
        margin-top: 2rem;
    }
    .blog-pagination button {
        background: var(--bg-card);
        border: 1px solid var(--border);
        color: var(--text-secondary);
        padding: 0.4rem 1rem;
        border-radius: 6px;
        cursor: pointer;
        font-size: 0.8rem;
    }
    .blog-pagination button:disabled {
        opacity: 0.4;
        cursor: default;
    }
    .blog-pagination span {
        color: var(--text-muted);
        font-size: 0.8rem;
    }
    @media (min-width: 600px) {
        .blog-grid {
            grid-template-columns: repeat(2, 1fr);
        }
    }
"#;

const PAGE_SIZE: u64 = 10;

#[component]
pub fn Blog() -> Element {
    let mut page = use_signal(|| 0u64);

    let articles = use_resource(move || async move {
        let offset = page() * PAGE_SIZE;
        ApiClient::web()
            .fetch::<ArticleListResponse>(&format!(
                "/api/articles?limit={PAGE_SIZE}&offset={offset}"
            ))
            .await
            .ok()
    });

    rsx! {
        style { {PAGE_CSS} }

        main { class: "blog-page",
            h1 { class: "blog-page-title", "Blog" }

            {
                let data = articles.read();
                let data = data.as_ref().and_then(|d| d.as_ref());
                match data {
                    None => rsx! { p { class: "blog-loading", "Loading..." } },
                    Some(resp) if resp.articles.is_empty() => rsx! {
                        p { class: "blog-empty", "No articles published yet." }
                    },
                    Some(resp) => {
                        let total_pages = (resp.total.saturating_sub(1) / PAGE_SIZE) + 1;
                        rsx! {
                            div { class: "blog-grid",
                                for a in resp.articles.iter() {
                                    {render_blog_card(a)}
                                }
                            }
                            if total_pages > 1 {
                                div { class: "blog-pagination",
                                    button {
                                        disabled: page() == 0,
                                        onclick: move |_| page -= 1,
                                        "Previous"
                                    }
                                    span { "Page {page() + 1} of {total_pages}" }
                                    button {
                                        disabled: page() + 1 >= total_pages,
                                        onclick: move |_| page += 1,
                                        "Next"
                                    }
                                }
                            }
                        }
                    },
                }
            }
        }
    }
}

fn render_blog_card(a: &Article) -> Element {
    let date: String = a
        .published_at
        .as_deref()
        .map(|d| d.chars().take(10).collect())
        .unwrap_or_default();
    let slug = a.slug.clone();

    rsx! {
        Link {
            to: Route::BlogArticle { slug },
            class: "blog-card",
            if let Some(ref cover) = a.cover_image_url {
                img {
                    class: "blog-card-cover",
                    src: "{cover}",
                    alt: "{a.title}",
                }
            }
            div { class: "blog-card-body",
                div { class: "blog-card-meta",
                    time { "{date}" }
                }
                h2 { class: "blog-card-title", "{a.title}" }
                if let Some(ref summary) = a.summary {
                    p { class: "blog-card-summary", "{summary}" }
                }
            }
        }
    }
}
