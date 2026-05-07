use dioxus::prelude::*;
use serde::Deserialize;

use scuffed_api_client::ApiClient;

use crate::routes::Route;

#[derive(Debug, Clone, Deserialize)]
struct FullArticle {
    #[allow(dead_code)]
    id: String,
    #[allow(dead_code)]
    slug: String,
    title: String,
    content_markdown: String,
    #[allow(dead_code)]
    summary: Option<String>,
    cover_image_url: Option<String>,
    author_member_id: String,
    published_at: Option<String>,
}

const PAGE_CSS: &str = r#"
    .blog-post-page {
        padding: 3rem 2rem;
        max-width: 800px;
        margin: 0 auto;
    }
    .blog-post-back {
        display: inline-flex;
        align-items: center;
        gap: 0.4rem;
        color: var(--text-muted);
        font-size: 0.8rem;
        margin-bottom: 1.5rem;
        text-decoration: none;
        transition: color 0.15s;
    }
    .blog-post-back:hover {
        color: var(--text-bright);
    }
    .blog-post-cover {
        width: 100%;
        max-height: 400px;
        object-fit: cover;
        border-radius: 10px;
        margin-bottom: 1.5rem;
    }
    .blog-post-title {
        font-family: 'Bebas Neue', sans-serif;
        font-size: 2.2rem;
        color: var(--text-bright);
        letter-spacing: 2px;
        margin: 0 0 0.75rem;
    }
    .blog-post-meta {
        display: flex;
        align-items: center;
        gap: 0.75rem;
        font-size: 0.75rem;
        color: var(--text-muted);
        margin-bottom: 2rem;
        padding-bottom: 1.5rem;
        border-bottom: 1px solid var(--border);
    }
    .blog-post-content {
        color: var(--text-secondary);
        font-size: 0.9rem;
        line-height: 1.85;
        white-space: pre-wrap;
        word-wrap: break-word;
    }
    .blog-post-loading, .blog-post-error {
        color: var(--text-muted);
        text-align: center;
        padding: 3rem 0;
    }
    @media (max-width: 600px) {
        .blog-post-page { padding: 2rem 1rem; }
        .blog-post-title { font-size: 1.6rem; }
        .blog-post-cover { max-height: 250px; }
    }
"#;

#[component]
pub fn BlogPost(slug: String) -> Element {
    let slug_owned = slug.clone();
    let article = use_resource(move || {
        let s = slug_owned.clone();
        async move {
            ApiClient::web()
                .fetch::<FullArticle>(&format!("/api/articles/{s}"))
                .await
                .ok()
        }
    });

    rsx! {
        style { {PAGE_CSS} }

        main { class: "blog-post-page",
            Link { to: Route::Blog {}, class: "blog-post-back",
                "← Back to Blog"
            }

            {
                let data = article.read();
                let data = data.as_ref().and_then(|d| d.as_ref());
                match data {
                    None => rsx! { p { class: "blog-post-loading", "Loading..." } },
                    Some(a) => rsx! {
                        if let Some(ref cover) = a.cover_image_url {
                            img {
                                class: "blog-post-cover",
                                src: "{cover}",
                                alt: "{a.title}",
                            }
                        }
                        h1 { class: "blog-post-title", "{a.title}" }
                        div { class: "blog-post-meta",
                            time {
                                {a.published_at.as_deref()
                                    .map(|d| d.chars().take(10).collect::<String>())
                                    .unwrap_or_else(|| "Draft".to_string())}
                            }
                            span { "by {a.author_member_id}" }
                        }
                        div { class: "blog-post-content",
                            "{a.content_markdown}"
                        }
                    },
                }
            }
        }
    }
}
