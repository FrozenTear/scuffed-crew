use dioxus::prelude::*;
use serde::Deserialize;

use scuffed_api_client::ApiClient;

use super::article_author::{
    ARTICLE_CSS, ARTICLE_CSS_ID, ArticleByline, ArticleCrumb, AuthorNames, Load, author_label,
    entry_date, is_retrying, load_status, use_author_names,
};
use super::not_found::{NotFoundLink, NotFoundPanel};
use crate::components::ui::{LoadError, PageHeader, PageShell, PageWidth};
use crate::markdown::render_markdown;
use crate::routes::Route;

#[derive(Debug, Clone, PartialEq, Deserialize)]
struct FullArticle {
    title: String,
    content_markdown: String,
    cover_image_url: Option<String>,
    author_member_id: String,
    published_at: Option<String>,
}

/// API path of one article. The slug is one encoded path segment, so a `/`
/// or `%2F` in the address cannot point the request at another `/api` path.
fn article_api_path(slug: &str) -> String {
    format!("/api/articles/{}", crate::util::encode_query(slug))
}

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
                .fetch::<FullArticle>(&article_api_path(&s))
                .await
                .map_err(|e| e.to_string())
        }
    });
    // Articles carry only the author id; resolve it once the article loads.
    let names = use_author_names(move || {
        article
            .read()
            .as_ref()
            .and_then(|r| r.as_ref().ok())
            .map(|a| vec![a.author_member_id.clone()])
            .unwrap_or_default()
    });
    let load = Load::of(article.read().as_ref());
    let retrying = is_retrying(article.state()(), &load);
    crate::head::use_page_title(tab_label(&load));

    rsx! {
        BlogPostView {
            load,
            names: Some(names()),
            retrying,
            on_retry: move |_| refresh += 1,
        }
    }
}

/// The browser tab names the post by its title once it has loaded, not by the
/// slug in the address. `None` keeps the route's default label.
fn tab_label(load: &Load<FullArticle>) -> Option<String> {
    match load {
        Load::Ready(a) => Some(format!("{} | Blog", a.title.trim())),
        Load::Missing => Some("Post not found | Blog".to_string()),
        Load::Loading | Load::Failed => None,
    }
}

/// The page for one fetch state. Every state has one H1, and the status line
/// is always in the tree; only its text changes.
#[component]
fn BlogPostView(
    load: Load<FullArticle>,
    names: Option<AuthorNames>,
    /// A Retry is running.
    #[props(default = false)]
    retrying: bool,
    on_retry: EventHandler<()>,
) -> Element {
    let status = load_status(&load, retrying, "Loading post");
    let current = match &load {
        Load::Ready(a) => Some(a.title.clone()),
        _ => None,
    };

    rsx! {
        crate::styles::StyleOnce { id: ARTICLE_CSS_ID, css: ARTICLE_CSS }
        crate::styles::StyleOnce { id: crate::markdown::MARKDOWN_CSS_ID, css: crate::markdown::MARKDOWN_CSS }

        PageShell { width: PageWidth::Narrow, class: "blog-post-page",
            // The not-found card has its own "Back to the blog" link, so a
            // one item trail above it would only repeat it.
            if load != Load::Missing {
                ArticleCrumb { to: Route::Blog {}, section: "Blog", current }
            }
            {match &load {
                // The title is not known yet, so the page is named by its kind.
                Load::Loading => rsx! { PageHeader { title: "Blog post" } },
                Load::Failed => rsx! { PageHeader { title: "Couldn't load this post" } },
                Load::Missing | Load::Ready(_) => rsx! {},
            }}
            p { class: "entry-status", role: "status", "{status}" }
            {match &load {
                Load::Loading => rsx! {},
                Load::Missing => rsx! {
                    NotFoundPanel {
                        title: "Post not found",
                        message: "This post does not exist or is no longer published.",
                        links: vec![
                            NotFoundLink { label: "Back to the blog", to: Route::Blog {} },
                            NotFoundLink { label: "Return home", to: Route::Home {} },
                        ],
                    }
                },
                Load::Failed => rsx! {
                    LoadError {
                        message: "Check your connection and try again.",
                        on_retry: move |_| on_retry.call(()),
                        retrying,
                    }
                },
                Load::Ready(a) => rsx! {
                    PageHeader {
                        title: a.title.clone(),
                        meta: rsx! {
                            ArticleByline { author: author_label(names.as_ref(), &a.author_member_id),
                                {match a.published_at.as_deref() {
                                    Some(date) => entry_date(date),
                                    None => rsx! { span { class: "entry-date", "Draft" } },
                                }}
                            }
                        },
                    }
                    if let Some(ref cover) = a.cover_image_url {
                        // The title is right above; the cover adds no text.
                        img { class: "article-cover", src: "{cover}", alt: "" }
                    }
                    div {
                        class: "md-body",
                        dangerous_inner_html: render_markdown(&a.content_markdown),
                    }
                },
            }}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::article_author::AUTHOR_FALLBACK;
    use super::super::article_author::test_support::render_routed;
    use super::*;

    const AUTHOR: &str = "member:7f3a9c2e1b";

    fn post() -> FullArticle {
        FullArticle {
            title: "How we run play nights".to_string(),
            content_markdown: concat!(
                "Play nights are open.\n\n",
                "# What you need to bring\n\n",
                "- A working mic\n\n",
                "```py\nstart = \"19:00\"\n```\n\n",
                "> We start on time.\n\n",
                "![Schedule diagram](/img/night.png \"A typical Friday\")\n\n",
                "Post on the [Looking for a group](/forum) board.\n",
            )
            .to_string(),
            cover_image_url: None,
            author_member_id: AUTHOR.to_string(),
            published_at: Some("2026-11-12T18:45:00Z".to_string()),
        }
    }

    fn render_state(load: Load<FullArticle>, names: Option<AuthorNames>, retrying: bool) -> String {
        render_routed(move || {
            rsx! {
                BlogPostView {
                    load: load.clone(),
                    names: names.clone(),
                    retrying,
                    on_retry: move |_| {},
                }
            }
        })
    }

    fn render(load: Load<FullArticle>, names: Option<AuthorNames>) -> String {
        render_state(load, names, false)
    }

    fn names() -> AuthorNames {
        AuthorNames::from([(AUTHOR.to_string(), Some("Player02".to_string()))])
    }

    #[test]
    fn the_slug_is_one_encoded_path_segment() {
        assert_eq!(article_api_path("post-1"), "/api/articles/post-1");
        assert_eq!(
            article_api_path("../auth/me"),
            "/api/articles/..%2Fauth%2Fme"
        );
        assert_eq!(article_api_path("a%2Fb"), "/api/articles/a%252Fb");
        assert_eq!(article_api_path("a/b?c").matches('/').count(), 3);
    }

    #[test]
    fn article_has_crumb_one_h1_byline_and_rendered_prose() {
        let html = render(Load::Ready(post()), Some(names()));
        assert!(!html.contains("<main"), "{html}");
        assert!(
            html.contains("class=\"ui-shell ui-shell--narrow blog-post-page\""),
            "{html}"
        );
        assert_eq!(html.matches("<h1").count(), 1, "{html}");
        assert!(html.contains("<h1>How we run play nights</h1>"), "{html}");
        assert!(
            html.contains("<nav class=\"article-crumb\" aria-label=\"Breadcrumb\">"),
            "{html}"
        );
        assert!(html.contains("href=\"/blog\""), "{html}");
        assert!(
            html.contains("<span aria-current=\"page\">How we run play nights</span>"),
            "{html}"
        );
        assert!(
            html.contains("<span class=\"article-avatar\" aria-hidden=\"true\">PL</span>"),
            "{html}"
        );
        assert!(
            html.contains("<span class=\"entry-author\">Player02</span>"),
            "{html}"
        );
        assert!(
            html.contains("datetime=\"2026-11-12\">12 Nov 2026</time>"),
            "{html}"
        );
        assert!(!html.contains(AUTHOR), "raw author id leaked: {html}");

        // Rendered markdown, not source.
        assert!(html.contains("<div class=\"md-body\">"), "{html}");
        assert!(html.contains("<h2>What you need to bring</h2>"), "{html}");
        assert!(html.contains("<li>A working mic</li>"), "{html}");
        assert!(html.contains("<blockquote>"), "{html}");
        assert!(
            html.contains("<figcaption>A typical Friday</figcaption>"),
            "{html}"
        );
        assert!(
            html.contains("<a href=\"/forum\">Looking for a group</a>"),
            "{html}"
        );
        assert!(
            html.contains(
                "<pre class=\"md-code\" tabindex=\"0\" role=\"group\" aria-label=\"Code sample\">"
            ),
            "{html}"
        );
        assert!(!html.contains("```"), "{html}");
    }

    #[test]
    fn author_is_never_a_raw_id() {
        let failed = AuthorNames::from([(AUTHOR.to_string(), None)]);
        let html = render(Load::Ready(post()), Some(failed));
        assert!(
            html.contains(&format!(
                "<span class=\"entry-author\">{AUTHOR_FALLBACK}</span>"
            )),
            "{html}"
        );
        assert!(!html.contains(AUTHOR), "{html}");

        let pending = render(Load::Ready(post()), None);
        assert!(!pending.contains("article-avatar"), "{pending}");
        assert!(!pending.contains(AUTHOR), "{pending}");
        assert!(pending.contains("12 Nov 2026"), "{pending}");
    }

    #[test]
    fn missing_slug_is_a_not_found_page_with_a_way_back() {
        let html = render(Load::Missing, None);
        assert_eq!(html.matches("<h1").count(), 1, "{html}");
        assert!(html.contains("<h1>Post not found</h1>"), "{html}");
        // The shared not-found card, with a way back as its first action.
        assert!(html.contains("<section class=\"wall\">"), "{html}");
        assert!(
            html.contains("<p class=\"wall-kicker\">Error 404</p>"),
            "{html}"
        );
        assert!(
            html.contains("href=\"/blog\" class=\"btn is-primary\">Back to the blog</a>"),
            "{html}"
        );
        assert!(html.contains(">Return home</a>"), "{html}");
        assert!(!html.contains("role=\"alert\""), "{html}");
        assert!(!html.contains("aria-current=\"page\""), "{html}");
        // No one item trail above the card: its first action is the way back.
        assert!(!html.contains("article-crumb"), "{html}");
    }

    #[test]
    fn tab_is_named_by_the_post_title_not_the_slug() {
        assert_eq!(
            tab_label(&Load::Ready(post())).as_deref(),
            Some("How we run play nights | Blog")
        );
        assert_eq!(
            tab_label(&Load::Missing).as_deref(),
            Some("Post not found | Blog")
        );
        assert_eq!(tab_label(&Load::Loading), None);
        assert_eq!(tab_label(&Load::Failed), None);
    }

    #[test]
    fn loading_and_failed_states() {
        let loading = render(Load::Loading, None);
        assert!(
            loading.contains("<p class=\"entry-status\" role=\"status\">Loading post</p>"),
            "{loading}"
        );
        assert!(!loading.contains("not found"), "{loading}");
        assert!(loading.contains("<h1>Blog post</h1>"), "{loading}");

        let failed = render(Load::Failed, None);
        assert!(failed.contains("role=\"alert\""), "{failed}");
        assert!(
            failed.contains("<h1>Couldn&#39;t load this post</h1>"),
            "{failed}"
        );
        assert!(
            failed.contains("Check your connection and try again."),
            "{failed}"
        );
        assert!(failed.contains(">Retry</button>"), "{failed}");
        assert!(!failed.contains("Post not found"), "{failed}");

        // A retry keeps the error and its button, and says it is loading.
        let retrying = render_state(Load::Failed, None, true);
        assert!(
            retrying.contains("<p class=\"entry-status\" role=\"status\">Loading post</p>"),
            "{retrying}"
        );
        assert!(retrying.contains(">Retrying...</button>"), "{retrying}");
        assert!(retrying.contains("aria-disabled=\"true\""), "{retrying}");
        assert!(!retrying.contains(" disabled"), "{retrying}");

        // One H1 in every state.
        for html in [&loading, &failed, &retrying] {
            assert_eq!(html.matches("<h1").count(), 1, "{html}");
        }

        // The status line exists in every state, empty once loading is over.
        let ready = render(Load::Ready(post()), Some(names()));
        for html in [&failed, &ready] {
            assert!(
                html.contains("<p class=\"entry-status\" role=\"status\"></p>"),
                "{html}"
            );
        }
    }

    #[test]
    fn cover_is_decorative_and_drafts_say_so() {
        let mut draft = post();
        draft.cover_image_url = Some("/uploads/cover.png".to_string());
        draft.published_at = None;
        let html = render(Load::Ready(draft), Some(names()));
        assert!(
            html.contains("<img class=\"article-cover\" src=\"/uploads/cover.png\" alt=\"\""),
            "{html}"
        );
        assert!(
            html.contains("<span class=\"entry-date\">Draft</span>"),
            "{html}"
        );
    }
}
