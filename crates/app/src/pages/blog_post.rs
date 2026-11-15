use dioxus::prelude::*;
use serde::Deserialize;

use scuffed_api_client::ApiClient;

use super::article_author::{author_byline, fetch_author_names};
use crate::components::is_http_status;
use crate::markdown::render_markdown;
use crate::routes::Route;
use crate::util::{FetchClass, classify_fetch};

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
        color: var(--text-3);
        font-size: 0.8rem;
        margin-bottom: 1.5rem;
        text-decoration: none;
        transition: color 0.15s;
    }
    .blog-post-back:hover {
        color: var(--text);
    }
    .blog-post-cover {
        width: 100%;
        max-height: 400px;
        object-fit: cover;
        border-radius: 10px;
        margin-bottom: 1.5rem;
    }
    .blog-post-title {
        font-family: var(--font-head);
        font-size: 2.2rem;
        color: var(--text);
        letter-spacing: 2px;
        margin: 0 0 0.75rem;
    }
    .blog-post-meta {
        display: flex;
        align-items: center;
        gap: 0.75rem;
        font-size: 0.75rem;
        color: var(--text-3);
        margin-bottom: 2rem;
        padding-bottom: 1.5rem;
        border-bottom: 1px solid var(--border);
    }
    .blog-post-content {
        color: var(--text-2);
        font-size: 0.9rem;
        line-height: 1.85;
        overflow-wrap: break-word;
    }
    .blog-post-content p,
    .blog-post-content ul,
    .blog-post-content ol,
    .blog-post-content blockquote,
    .blog-post-content pre,
    .blog-post-content table {
        margin: 0 0 1rem;
    }
    .blog-post-content a {
        color: var(--accent);
    }
    .blog-post-content img {
        max-width: 100%;
        height: auto;
    }
    .blog-post-content blockquote {
        border-left: 3px solid var(--border);
        padding-left: 1rem;
        color: var(--text-3);
    }
    .blog-post-content pre {
        background: var(--surface);
        border: 1px solid var(--border);
        border-radius: 6px;
        padding: 0.75rem 1rem;
        overflow-x: auto;
    }
    .blog-post-content table {
        border-collapse: collapse;
    }
    .blog-post-content th,
    .blog-post-content td {
        border: 1px solid var(--border);
        padding: 0.35rem 0.6rem;
    }
    .blog-post-loading, .blog-post-error {
        color: var(--text-3);
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
    let mut refresh = use_signal(|| 0u32);
    let slug_owned = slug.clone();
    let article = use_resource(move || {
        let s = slug_owned.clone();
        let _ = refresh();
        async move {
            if s.is_empty() {
                return Err("HTTP error: 404".to_string());
            }
            ApiClient::web()
                .fetch::<FullArticle>(&format!("/api/articles/{s}"))
                .await
                .map_err(|e| e.to_string())
        }
    });
    // Articles carry only the author id; resolve it once the article loads.
    let author_names = use_resource(move || {
        let ids: Vec<String> = article
            .read()
            .as_ref()
            .and_then(|r| r.as_ref().ok())
            .map(|a| vec![a.author_member_id.clone()])
            .unwrap_or_default();
        fetch_author_names(ids)
    });

    rsx! {
        crate::styles::StyleOnce { id: "scuffed-css-pages-blog_post-page-css", css: PAGE_CSS }
        crate::styles::StyleOnce { id: crate::markdown::MARKDOWN_CSS_ID, css: crate::markdown::MARKDOWN_CSS }

        main { class: "blog-post-page",
            Link { to: Route::Blog {}, class: "blog-post-back",
                "← Back to Blog"
            }

            {
                let data = article.read();
                match classify_fetch(data.as_ref()) {
                    FetchClass::Loading => rsx! { p { class: "blog-post-loading", "Loading..." } },
                    FetchClass::Error => {
                        let err = data
                            .as_ref()
                            .and_then(|r| r.as_ref().err())
                            .cloned()
                            .unwrap_or_default();
                        if is_http_status(&err, 404) {
                            rsx! { p { class: "blog-post-error", "Post not found" } }
                        } else {
                            rsx! {
                                p { class: "blog-post-error", role: "alert", "Couldn't load this post. {err}" }
                                button {
                                    r#type: "button",
                                    class: "fetch-error__retry",
                                    onclick: move |_| refresh += 1,
                                    "Retry"
                                }
                            }
                        }
                    }
                    FetchClass::Ready => {
                        match data.as_ref().and_then(|r| r.as_ref().ok()) {
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
                                    {author_byline(author_names.read().as_ref(), &a.author_member_id)}
                                }
                                div {
                                    class: "blog-post-content md-body",
                                    dangerous_inner_html: render_markdown(&a.content_markdown),
                                }
                            },
                            None => rsx! { p { class: "blog-post-loading", "Loading..." } },
                        }
                    }
                }
            }
        }
    }
}
