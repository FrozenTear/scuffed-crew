use dioxus::prelude::*;
use serde::Deserialize;

use scuffed_api_client::ApiClient;

use crate::routes::Route;

#[derive(Debug, Clone, Deserialize)]
struct BlogArticle {
    #[allow(dead_code)]
    id: String,
    slug: String,
    title: String,
    summary: Option<String>,
    cover_image_url: Option<String>,
    author_member_id: String,
    published_at: Option<String>,
}

const PAGE_CSS: &str = r#"
    .blog-page {
        padding: 3rem 2rem;
        max-width: 900px;
        margin: 0 auto;
    }
    .blog-page-title {
        font-family: var(--font-head);
        font-size: 2.5rem;
        color: var(--text);
        letter-spacing: 3px;
        margin: 0 0 2rem;
    }
    .blog-list {
        display: flex;
        flex-direction: column;
        gap: 1.5rem;
    }
    .blog-card {
        background: var(--surface);
        border: 1px solid var(--border);
        border-radius: 10px;
        overflow: hidden;
        transition: border-color 0.2s, transform 0.15s;
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
        padding: 1.25rem 1.5rem;
    }
    .blog-card-meta {
        display: flex;
        align-items: center;
        gap: 0.75rem;
        font-size: 0.7rem;
        color: var(--text-3);
        margin-bottom: 0.5rem;
    }
    .blog-card-title {
        font-family: var(--font-head);
        font-size: 1.3rem;
        font-weight: 700;
        color: var(--text);
        margin: 0 0 0.5rem;
    }
    .blog-card-summary {
        color: var(--text-2);
        font-size: 0.85rem;
        line-height: 1.7;
        margin: 0;
    }
    .blog-loading, .blog-empty {
        color: var(--text-3);
        text-align: center;
        padding: 3rem 0;
    }
    @media (max-width: 600px) {
        .blog-page { padding: 2rem 1rem; }
        .blog-card-cover { height: 150px; }
    }
"#;

#[component]
pub fn Blog() -> Element {
    let articles = use_resource(|| async {
        ApiClient::web()
            .fetch::<Vec<BlogArticle>>("/api/articles?limit=50")
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
                    Some(list) if list.is_empty() => rsx! {
                        p { class: "blog-empty", "No articles yet." }
                    },
                    Some(list) => rsx! {
                        div { class: "blog-list",
                            for a in list.iter() {
                                {render_blog_card(a)}
                            }
                        }
                    },
                }
            }
        }
    }
}

fn render_blog_card(a: &BlogArticle) -> Element {
    let date: String = a
        .published_at
        .as_deref()
        .map(|d| d.chars().take(10).collect())
        .unwrap_or_else(|| "Draft".to_string());

    let slug = a.slug.clone();

    rsx! {
        Link {
            to: Route::BlogPost { slug },
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
                    span { "by {a.author_member_id}" }
                }
                h2 { class: "blog-card-title", "{a.title}" }
                if let Some(ref summary) = a.summary {
                    p { class: "blog-card-summary", "{summary}" }
                }
            }
        }
    }
}
