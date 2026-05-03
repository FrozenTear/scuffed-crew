use dioxus::prelude::*;
use pulldown_cmark::{Options, Parser, html};
use serde::Deserialize;

use scuffed_api_client::ApiClient;

use crate::routes::Route;

#[derive(Debug, Clone, Deserialize)]
struct Article {
    title: String,
    content_markdown: String,
    summary: Option<String>,
    cover_image_url: Option<String>,
    author_member_id: String,
    published_at: Option<String>,
}

fn markdown_to_html(md: &str) -> String {
    let options =
        Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_HEADING_ATTRIBUTES;
    let parser = Parser::new_ext(md, options);
    let mut html_output = String::new();
    html::push_html(&mut html_output, parser);
    html_output
}

const PAGE_CSS: &str = r#"
    .article-page {
        padding: 3rem 2rem;
        max-width: 760px;
        margin: 0 auto;
    }
    .article-back {
        display: inline-flex;
        align-items: center;
        gap: 0.3rem;
        color: var(--text-muted);
        font-size: 0.8rem;
        margin-bottom: 1.5rem;
        text-decoration: none;
    }
    .article-back:hover {
        color: var(--text-secondary);
    }
    .article-cover {
        width: 100%;
        max-height: 400px;
        object-fit: cover;
        border-radius: 12px;
        margin-bottom: 1.5rem;
    }
    .article-header {
        margin-bottom: 2rem;
    }
    .article-title {
        font-family: 'Bebas Neue', sans-serif;
        font-size: 2.2rem;
        color: var(--text-bright);
        letter-spacing: 2px;
        margin: 0 0 0.5rem;
        line-height: 1.2;
    }
    .article-meta {
        color: var(--text-muted);
        font-size: 0.75rem;
    }
    .article-content {
        color: var(--text-secondary);
        font-size: 0.95rem;
        line-height: 1.8;
    }
    .article-content h1,
    .article-content h2,
    .article-content h3 {
        color: var(--text-bright);
        font-family: 'Rajdhani', sans-serif;
        margin: 1.5em 0 0.5em;
    }
    .article-content h1 { font-size: 1.6rem; }
    .article-content h2 { font-size: 1.3rem; }
    .article-content h3 { font-size: 1.1rem; }
    .article-content p {
        margin: 0 0 1em;
    }
    .article-content a {
        color: var(--accent);
    }
    .article-content blockquote {
        border-left: 3px solid var(--accent-soft);
        margin: 1em 0;
        padding: 0.5em 1em;
        color: var(--text-muted);
        background: rgba(255,255,255,0.02);
        border-radius: 0 6px 6px 0;
    }
    .article-content pre {
        background: var(--bg-card);
        border: 1px solid var(--border);
        border-radius: 8px;
        padding: 1em;
        overflow-x: auto;
        font-size: 0.85rem;
    }
    .article-content code {
        background: var(--bg-card);
        padding: 0.15em 0.35em;
        border-radius: 4px;
        font-size: 0.85em;
    }
    .article-content pre code {
        background: none;
        padding: 0;
    }
    .article-content img {
        max-width: 100%;
        border-radius: 8px;
        margin: 1em 0;
    }
    .article-content table {
        width: 100%;
        border-collapse: collapse;
        margin: 1em 0;
    }
    .article-content th,
    .article-content td {
        border: 1px solid var(--border);
        padding: 0.5em 0.75em;
        text-align: left;
    }
    .article-content th {
        background: var(--bg-card);
        color: var(--text-bright);
    }
    .article-content ul, .article-content ol {
        margin: 0 0 1em;
        padding-left: 1.5em;
    }
    .article-loading, .article-error {
        color: var(--text-muted);
        text-align: center;
        padding: 3rem 0;
    }
"#;

#[component]
pub fn BlogArticle(slug: String) -> Element {
    let slug_clone = slug.clone();
    let article = use_resource(move || {
        let slug = slug_clone.clone();
        async move {
            ApiClient::web()
                .fetch::<Article>(&format!("/api/articles/{slug}"))
                .await
                .ok()
        }
    });

    rsx! {
        style { {PAGE_CSS} }

        main { class: "article-page",
            Link { to: Route::Blog {}, class: "article-back", "< Back to Blog" }

            {
                let data = article.read();
                let data = data.as_ref().and_then(|d| d.as_ref());
                match data {
                    None => rsx! { p { class: "article-loading", "Loading..." } },
                    Some(a) => {
                        let date: String = a
                            .published_at
                            .as_deref()
                            .map(|d| d.chars().take(10).collect())
                            .unwrap_or_default();
                        let content_html = markdown_to_html(&a.content_markdown);
                        rsx! {
                            if let Some(ref cover) = a.cover_image_url {
                                img {
                                    class: "article-cover",
                                    src: "{cover}",
                                    alt: "{a.title}",
                                }
                            }
                            div { class: "article-header",
                                h1 { class: "article-title", "{a.title}" }
                                div { class: "article-meta", "Published {date}" }
                            }
                            div {
                                class: "article-content",
                                dangerous_inner_html: "{content_html}",
                            }
                        }
                    },
                }
            }
        }
    }
}
