use dioxus::prelude::*;
use serde::Deserialize;

use scuffed_api_client::ApiClient;

use super::article_author::{
    ARTICLE_CSS, ARTICLE_CSS_ID, AuthorNames, ENTRY_PAGE_SIZE, LIST_BATCH, Load, author_byline,
    entry_date, fetch_offset_list, is_retrying, list_status, use_author_names,
};
use crate::components::ui::{
    EmptyCard, LoadError, PageHeader, PageShell, PageWidth, Pager, page_slice, show_list_top,
    use_url_page,
};
use crate::routes::Route;

#[derive(Debug, Clone, PartialEq, Deserialize)]
struct BlogArticle {
    id: String,
    slug: String,
    title: String,
    summary: Option<String>,
    author_member_id: String,
    published_at: Option<String>,
}

/// The list endpoint takes `limit` (at most 100) and `offset` and returns a
/// bare array with no total. The page asks for the next offset while a batch
/// comes back full (see `fetch_offset_list`) and pages through the rows here.
fn list_url(offset: usize) -> String {
    format!("/api/articles?limit={LIST_BATCH}&offset={offset}")
}

/// The list section. It takes focus after a page change.
const LIST_ID: &str = "blog-list";

#[component]
pub fn Blog() -> Element {
    let mut refresh = use_signal(|| 0u32);
    // The page lives in the address (`?page=2`), so Back from a post returns
    // to the page it was opened from.
    let page = use_url_page();
    let articles = use_resource(move || {
        let _ = refresh();
        fetch_offset_list(|offset| async move {
            ApiClient::web()
                .fetch::<Vec<BlogArticle>>(&list_url(offset))
                .await
                .map_err(|e| crate::client_error::logged_message("blog list", &e))
        })
    });
    // Names for the authors of the page on screen only.
    let names = use_author_names(move || {
        let page = page.get();
        articles
            .read()
            .as_ref()
            .and_then(|r| r.as_ref().ok())
            .map(|list| {
                page_slice(&list.items, page, ENTRY_PAGE_SIZE)
                    .iter()
                    .map(|a| a.author_member_id.clone())
                    .collect()
            })
            .unwrap_or_default()
    });
    let (load, capped) = Load::of(articles.read().as_ref()).split();
    let retrying = is_retrying(articles.state()(), &load);

    rsx! {
        BlogIndex {
            load,
            names: Some(names()),
            page: page.get(),
            capped,
            retrying,
            on_retry: move |_| refresh += 1,
            on_page: move |next| {
                page.go(next);
                show_list_top(LIST_ID);
            },
        }
    }
}

/// The page for one fetch state. The status line and the pager are always in
/// the tree; only their text changes.
#[component]
fn BlogIndex(
    load: Load<Vec<BlogArticle>>,
    names: Option<AuthorNames>,
    page: usize,
    /// The load stopped at its batch budget: older posts exist.
    #[props(default = false)]
    capped: bool,
    /// A Retry is running.
    #[props(default = false)]
    retrying: bool,
    on_retry: EventHandler<()>,
    on_page: EventHandler<usize>,
) -> Element {
    let status = list_status(&load, retrying, capped, "Loading posts", "posts");
    let total = match &load {
        Load::Ready(list) => list.len(),
        _ => 0,
    };

    rsx! {
        crate::styles::StyleOnce { id: ARTICLE_CSS_ID, css: ARTICLE_CSS }

        PageShell { width: PageWidth::Default, class: "blog-page",
            PageHeader {
                title: "Blog",
                lead: "Guides and notes from the crew.",
            }
            section {
                id: LIST_ID,
                class: "ui-stack pager-target",
                tabindex: "-1",
                aria_label: "Posts",
                p { class: "entry-status", role: "status", "{status}" }
                {match &load {
                    Load::Loading => rsx! {},
                    Load::Failed | Load::Missing => rsx! {
                        LoadError {
                            message: "Couldn't load the blog. Check your connection and try again.",
                            on_retry: move |_| on_retry.call(()),
                            retrying,
                        }
                    },
                    Load::Ready(list) if list.is_empty() => rsx! {
                        EmptyCard {
                            title: "No posts yet",
                            message: "New posts from the crew will show up here.",
                        }
                    },
                    Load::Ready(list) => rsx! {
                        div { class: "entry-list",
                            // Mapped, not wrapped in a `for` body, so the
                            // key on each card's root reaches this list.
                            {page_slice(list, page, ENTRY_PAGE_SIZE)
                                .iter()
                                .map(|a| render_blog_card(a, names.as_ref()))}
                        }
                    },
                }}
                Pager {
                    id: "blog-pager",
                    page,
                    total,
                    page_size: ENTRY_PAGE_SIZE,
                    noun: "posts",
                    noun_singular: "post",
                    on_change: move |next| on_page.call(next),
                }
            }
        }
    }
}

/// The credit stays out until the lookup for this card's author has finished,
/// so it never flashes the fallback label.
fn render_blog_card(a: &BlogArticle, names: Option<&AuthorNames>) -> Element {
    let slug = a.slug.clone();
    let summary = a.summary.as_deref().map(str::trim).unwrap_or_default();

    rsx! {
        article { key: "{a.id}", class: "entry-row",
            h2 {
                Link { to: Route::BlogPost { slug }, "{a.title}" }
            }
            if !summary.is_empty() {
                p { "{summary}" }
            }
            p { class: "entry-meta",
                {author_byline(names, &a.author_member_id, "By")}
                {match a.published_at.as_deref() {
                    Some(date) => entry_date(date),
                    None => rsx! { span { class: "entry-date", "Draft" } },
                }}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::article_author::AUTHOR_FALLBACK;
    use super::super::article_author::test_support::render_routed;
    use super::*;

    const AUTHOR: &str = "member:7f3a9c2e1b";

    fn post(n: usize) -> BlogArticle {
        BlogArticle {
            id: format!("article:{n}"),
            slug: format!("post-{n}"),
            title: format!("Post {n}"),
            summary: Some(format!("Summary {n}")),
            author_member_id: AUTHOR.to_string(),
            published_at: Some("2026-11-12T18:45:00Z".to_string()),
        }
    }

    fn names() -> AuthorNames {
        AuthorNames::from([(AUTHOR.to_string(), Some("Player02".to_string()))])
    }

    fn render_state(
        load: Load<Vec<BlogArticle>>,
        names: Option<AuthorNames>,
        page: usize,
        capped: bool,
        retrying: bool,
    ) -> String {
        render_routed(move || {
            rsx! {
                BlogIndex {
                    load: load.clone(),
                    names: names.clone(),
                    page,
                    capped,
                    retrying,
                    on_retry: move |_| {},
                    on_page: move |_| {},
                }
            }
        })
    }

    fn render(load: Load<Vec<BlogArticle>>, names: Option<AuthorNames>, page: usize) -> String {
        render_state(load, names, page, false, false)
    }

    #[test]
    fn list_shows_title_link_summary_author_name_and_date() {
        let html = render(Load::Ready(vec![post(1)]), Some(names()), 1);
        assert!(!html.contains("<main"), "{html}");
        assert_eq!(html.matches("<h1").count(), 1, "{html}");
        assert!(html.contains("<h1>Blog</h1>"), "{html}");
        assert!(html.contains("class=\"ui-shell blog-page\""), "{html}");
        assert!(html.contains("href=\"/blog/post-1\""), "{html}");
        assert!(html.contains(">Post 1</a>"), "{html}");
        assert!(html.contains("<p>Summary 1</p>"), "{html}");
        assert!(html.contains(">By Player02</span>"), "{html}");
        assert!(
            html.contains("<time class=\"entry-date\" datetime=\"2026-11-12\">12 Nov 2026</time>"),
            "{html}"
        );
        assert!(!html.contains(AUTHOR), "raw author id leaked: {html}");
        // No button or second link inside the title link.
        assert_eq!(html.matches("<a ").count(), 1, "{html}");
    }

    #[test]
    fn author_is_never_a_raw_id() {
        let failed = AuthorNames::from([(AUTHOR.to_string(), None)]);
        let html = render(Load::Ready(vec![post(1)]), Some(failed), 1);
        assert!(
            html.contains(&format!(">By {AUTHOR_FALLBACK}</span>")),
            "{html}"
        );
        assert!(!html.contains(AUTHOR), "{html}");

        let pending = render(Load::Ready(vec![post(1)]), None, 1);
        assert!(!pending.contains("By "), "{pending}");
        assert!(!pending.contains(AUTHOR), "{pending}");
        assert!(pending.contains("12 Nov 2026"), "{pending}");
    }

    #[test]
    fn empty_loading_and_failed_states() {
        let empty = render(Load::Ready(Vec::new()), None, 1);
        assert!(empty.contains("<h2>No posts yet</h2>"), "{empty}");
        assert!(!empty.contains("entry-list"), "{empty}");

        let loading = render(Load::Loading, None, 1);
        assert!(
            loading.contains("<p class=\"entry-status\" role=\"status\">Loading posts</p>"),
            "{loading}"
        );
        assert!(!loading.contains("No posts yet"), "{loading}");

        let failed = render(Load::Failed, None, 1);
        assert!(failed.contains("role=\"alert\""), "{failed}");
        assert!(
            failed.contains("load the blog. Check your connection"),
            "{failed}"
        );
        assert!(failed.contains(">Retry</button>"), "{failed}");

        // The status line and the pager's range line exist in every state.
        for html in [&empty, &loading, &failed] {
            assert!(
                html.contains("class=\"entry-status\" role=\"status\""),
                "{html}"
            );
            assert!(html.contains("id=\"blog-pager-meta\""), "{html}");
            assert_eq!(html.matches("<h1").count(), 1, "{html}");
        }
        assert!(
            empty.contains("<p class=\"entry-status\" role=\"status\"></p>"),
            "{empty}"
        );
    }

    #[test]
    fn retry_says_so_and_keeps_the_button_focusable() {
        let html = render_state(Load::Failed, None, 1, false, true);
        assert!(
            html.contains("<p class=\"entry-status\" role=\"status\">Loading posts</p>"),
            "{html}"
        );
        assert!(html.contains(">Retrying...</button>"), "{html}");
        assert!(html.contains("aria-disabled=\"true\""), "{html}");
        assert!(!html.contains(" disabled"), "{html}");
        assert!(html.contains("aria-busy=\"true\""), "{html}");
    }

    #[test]
    fn list_section_takes_focus_after_a_page_change() {
        let html = render(Load::Ready(vec![post(1)]), Some(names()), 1);
        assert!(
            html.contains(
                "<section id=\"blog-list\" class=\"ui-stack pager-target\" tabindex=\"-1\" aria-label=\"Posts\">"
            ),
            "{html}"
        );
    }

    #[test]
    fn a_cut_list_says_how_many_posts_it_shows() {
        let posts: Vec<BlogArticle> = (1..=12).map(post).collect();
        let html = render_state(Load::Ready(posts), Some(names()), 1, true, false);
        assert!(
            html.contains(
                "<p class=\"entry-status\" role=\"status\">Showing the newest 12 posts.</p>"
            ),
            "{html}"
        );
        assert_eq!(list_url(0), "/api/articles?limit=100&offset=0");
        assert_eq!(list_url(200), "/api/articles?limit=100&offset=200");
    }

    #[test]
    fn long_list_is_paged_ten_at_a_time() {
        let posts: Vec<BlogArticle> = (1..=23).map(post).collect();
        let first = render(Load::Ready(posts.clone()), Some(names()), 1);
        assert_eq!(first.matches("class=\"entry-row\"").count(), 10, "{first}");
        assert!(first.contains(">1 to 10 of 23 posts</p>"), "{first}");
        assert!(first.contains("aria-label=\"Pagination\""), "{first}");
        assert!(first.contains(">Post 10</a>"), "{first}");
        assert!(!first.contains(">Post 11</a>"), "{first}");

        let last = render(Load::Ready(posts), Some(names()), 3);
        assert_eq!(last.matches("class=\"entry-row\"").count(), 3, "{last}");
        assert!(last.contains(">21 to 23 of 23 posts</p>"), "{last}");
        assert!(last.contains(">Post 23</a>"), "{last}");

        let one = render(Load::Ready(vec![post(1)]), Some(names()), 1);
        assert!(one.contains(">1 post</p>"), "{one}");
        assert!(!one.contains("aria-label=\"Pagination\""), "{one}");
    }

    #[test]
    fn draft_and_blank_summary_rows() {
        let mut draft = post(1);
        draft.published_at = None;
        draft.summary = Some("   ".to_string());
        let html = render(Load::Ready(vec![draft]), Some(names()), 1);
        assert!(
            html.contains("<span class=\"entry-date\">Draft</span>"),
            "{html}"
        );
        assert!(!html.contains("<p></p>"), "{html}");
        assert!(!html.contains("<p>   </p>"), "{html}");
    }
}
