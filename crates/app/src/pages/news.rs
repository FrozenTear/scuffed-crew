use dioxus::prelude::*;
use serde::Deserialize;

use super::article_author::{
    ARTICLE_CSS, ARTICLE_CSS_ID, AuthorNames, ENTRY_PAGE_SIZE, Load, author_byline, entry_date,
    is_retrying, list_status, use_author_names,
};
use crate::components::list_cap_notice;
use crate::components::ui::{
    EmptyCard, LoadError, PageHeader, PageShell, PageWidth, Pager, Pill, page_slice, show_list_top,
    use_url_page,
};
use crate::hooks::use_api_list;

#[derive(Debug, Clone, PartialEq, Deserialize)]
struct Announcement {
    id: String,
    title: String,
    content: String,
    /// Member id of the poster. Shown only as a display name.
    #[serde(default)]
    author_id: String,
    pinned: bool,
    created_at: String,
}

/// The list endpoint is cursor paged (`limit` at most 100, `cursor`) and has
/// no total. `use_api_list` follows the cursor (pinned first, then newest) and
/// the cap notice under the pager says so if rows remain past its budget.
const LIST_URL: &str = "/api/announcements";

/// The list section. It takes focus after a page change.
const LIST_ID: &str = "news-list";

const PAGE_CSS: &str = r#"
.news-page .news-body { white-space: pre-line; }
"#;

#[component]
pub fn News() -> Element {
    // The page lives in the address (`?page=2`), as on the blog list.
    let page = use_url_page();
    let announcements = use_api_list::<Announcement>(LIST_URL);
    let mut refresh = announcements.refresh;
    // Names for the authors of the page on screen only.
    let names = use_author_names(move || {
        let page = page.get();
        announcements
            .data
            .read()
            .as_ref()
            .and_then(|slot| slot.as_ref())
            .map(|list| {
                page_slice(list, page, ENTRY_PAGE_SIZE)
                    .iter()
                    .map(|a| a.author_id.clone())
                    .collect()
            })
            .unwrap_or_default()
    });
    let load = Load::of_api(announcements.data.read().as_ref());
    let retrying = is_retrying(announcements.data.state()(), &load);

    rsx! {
        NewsIndex {
            load,
            names: Some(names()),
            page: page.get(),
            retrying,
            on_retry: move |_| refresh += 1,
            on_page: move |next| {
                page.go(next);
                show_list_top(LIST_ID);
            },
            notice: list_cap_notice(&announcements, "announcements"),
        }
    }
}

/// The page for one fetch state. The status line and the pager are always in
/// the tree; only their text changes.
#[component]
fn NewsIndex(
    load: Load<Vec<Announcement>>,
    names: Option<AuthorNames>,
    page: usize,
    /// A Retry is running.
    #[props(default = false)]
    retrying: bool,
    on_retry: EventHandler<()>,
    on_page: EventHandler<usize>,
    /// Shown under the pager (the "Load more" notice of a cut list).
    #[props(default)]
    notice: Option<Element>,
) -> Element {
    let status = list_status(
        &load,
        retrying,
        false,
        "Loading announcements",
        "announcements",
    );
    let total = match &load {
        Load::Ready(list) => list.len(),
        _ => 0,
    };

    rsx! {
        crate::styles::StyleOnce { id: ARTICLE_CSS_ID, css: ARTICLE_CSS }
        crate::styles::StyleOnce { id: "scuffed-css-pages-news-page-css", css: PAGE_CSS }

        PageShell { width: PageWidth::Default, class: "news-page",
            PageHeader {
                title: "News",
                lead: "Announcements for the crew and guests.",
            }
            section {
                id: LIST_ID,
                class: "ui-stack pager-target",
                tabindex: "-1",
                aria_label: "Announcements",
                p { class: "entry-status", role: "status", "{status}" }
                {match &load {
                    Load::Loading => rsx! {},
                    Load::Failed | Load::Missing => rsx! {
                        LoadError {
                            message: "Couldn't load the news. Check your connection and try again.",
                            on_retry: move |_| on_retry.call(()),
                            retrying,
                        }
                    },
                    Load::Ready(list) if list.is_empty() => rsx! {
                        EmptyCard {
                            title: "No announcements yet",
                            message: "Check back soon.",
                        }
                    },
                    Load::Ready(list) => rsx! {
                        div { class: "entry-list",
                            for a in page_slice(list, page, ENTRY_PAGE_SIZE) {
                                {render_news_card(a, names.as_ref())}
                            }
                        }
                    },
                }}
                Pager {
                    id: "news-pager",
                    page,
                    total,
                    page_size: ENTRY_PAGE_SIZE,
                    noun: "announcements",
                    noun_singular: "announcement",
                    on_change: move |next| on_page.call(next),
                }
                {notice}
            }
        }
    }
}

/// "Pinned" is a word in a badge next to the title, so it does not rest on
/// colour or position.
fn render_news_card(a: &Announcement, names: Option<&AuthorNames>) -> Element {
    rsx! {
        article { key: "{a.id}", class: "entry-row",
            h2 {
                span { "{a.title}" }
                if a.pinned {
                    Pill { "Pinned" }
                }
            }
            p { class: "news-body", "{a.content}" }
            p { class: "entry-meta",
                {author_byline(names, &a.author_id, "Posted by")}
                {entry_date(&a.created_at)}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::article_author::AUTHOR_FALLBACK;
    use super::*;

    const AUTHOR: &str = "member:9d1c44a0e2";

    fn item(n: usize, pinned: bool) -> Announcement {
        Announcement {
            id: format!("announcement:{n}"),
            title: format!("Notice {n}"),
            content: format!("Body {n}"),
            author_id: AUTHOR.to_string(),
            pinned,
            created_at: "2026-11-08T10:00:00Z".to_string(),
        }
    }

    fn names() -> AuthorNames {
        AuthorNames::from([(AUTHOR.to_string(), Some("Player06".to_string()))])
    }

    #[derive(Props, Clone, PartialEq)]
    struct ProbeProps {
        load: Load<Vec<Announcement>>,
        names: Option<AuthorNames>,
        page: usize,
        retrying: bool,
    }

    #[allow(non_snake_case)]
    fn Probe(props: ProbeProps) -> Element {
        rsx! {
            NewsIndex {
                load: props.load,
                names: props.names,
                page: props.page,
                retrying: props.retrying,
                on_retry: move |_| {},
                on_page: move |_| {},
            }
        }
    }

    fn render_state(
        load: Load<Vec<Announcement>>,
        names: Option<AuthorNames>,
        page: usize,
        retrying: bool,
    ) -> String {
        let props = ProbeProps {
            load,
            names,
            page,
            retrying,
        };
        let mut dom = VirtualDom::new_with_props(Probe, props);
        dom.rebuild_in_place();
        dioxus_ssr::render(&dom)
    }

    fn render(load: Load<Vec<Announcement>>, names: Option<AuthorNames>, page: usize) -> String {
        render_state(load, names, page, false)
    }

    #[test]
    fn pinned_is_a_word_in_a_badge_next_to_the_title() {
        let html = render(
            Load::Ready(vec![item(1, true), item(2, false)]),
            Some(names()),
            1,
        );
        assert_eq!(html.matches("<h1").count(), 1, "{html}");
        assert!(html.contains("<h1>News</h1>"), "{html}");
        assert!(!html.contains("<main"), "{html}");
        assert!(
            html.contains(
                "<h2><span>Notice 1</span><span class=\"ui-pill ui-pill--neutral\">Pinned</span></h2>"
            ),
            "{html}"
        );
        assert!(html.contains("<h2><span>Notice 2</span></h2>"), "{html}");
        assert_eq!(html.matches(">Pinned<").count(), 1, "{html}");
    }

    #[test]
    fn card_shows_author_display_name_and_date() {
        let html = render(Load::Ready(vec![item(1, false)]), Some(names()), 1);
        assert!(html.contains(">Posted by Player06</span>"), "{html}");
        assert!(
            html.contains("<time class=\"entry-date\" datetime=\"2026-11-08\">8 Nov 2026</time>"),
            "{html}"
        );
        assert!(html.contains("<p class=\"news-body\">Body 1</p>"), "{html}");
        assert!(!html.contains(AUTHOR), "raw author id leaked: {html}");

        let failed = AuthorNames::from([(AUTHOR.to_string(), None)]);
        let html = render(Load::Ready(vec![item(1, false)]), Some(failed), 1);
        assert!(
            html.contains(&format!(">Posted by {AUTHOR_FALLBACK}</span>")),
            "{html}"
        );
        assert!(!html.contains(AUTHOR), "{html}");
    }

    #[test]
    fn announcement_without_an_author_field_still_parses() {
        let a: Announcement = serde_json::from_str(
            r#"{"id":"announcement:1","title":"T","content":"C","pinned":false,
                "created_at":"2026-11-08T10:00:00Z"}"#,
        )
        .unwrap();
        assert_eq!(a.author_id, "");
        // No lookup runs for a blank id, so no credit and no fallback shows.
        let html = render(Load::Ready(vec![a]), Some(AuthorNames::new()), 1);
        assert!(!html.contains("Posted by"), "{html}");
        assert!(html.contains("8 Nov 2026"), "{html}");
    }

    #[test]
    fn empty_loading_and_failed_states() {
        let empty = render(Load::Ready(Vec::new()), None, 1);
        assert!(empty.contains("<h2>No announcements yet</h2>"), "{empty}");

        let loading = render(Load::Loading, None, 1);
        assert!(
            loading.contains("<p class=\"entry-status\" role=\"status\">Loading announcements</p>"),
            "{loading}"
        );
        assert!(!loading.contains("No announcements yet"), "{loading}");

        // A failed fetch is an error with a retry, not an endless "Loading".
        let failed = render(Load::Failed, None, 1);
        assert!(failed.contains("role=\"alert\""), "{failed}");
        assert!(
            failed.contains("load the news. Check your connection"),
            "{failed}"
        );
        assert!(failed.contains(">Retry</button>"), "{failed}");
        assert!(!failed.contains("Loading"), "{failed}");

        // A retry keeps the error and its button, and says it is loading.
        let retrying = render_state(Load::Failed, None, 1, true);
        assert!(
            retrying
                .contains("<p class=\"entry-status\" role=\"status\">Loading announcements</p>"),
            "{retrying}"
        );
        assert!(retrying.contains(">Retrying...</button>"), "{retrying}");
        assert!(retrying.contains("aria-disabled=\"true\""), "{retrying}");
        assert!(!retrying.contains(" disabled"), "{retrying}");

        for html in [&empty, &loading, &failed] {
            assert!(
                html.contains("class=\"entry-status\" role=\"status\""),
                "{html}"
            );
            assert!(html.contains("id=\"news-pager-meta\""), "{html}");
            assert_eq!(html.matches("<h1").count(), 1, "{html}");
            // The list section is the focus target after a page change.
            assert!(
                html.contains(
                    "<section id=\"news-list\" class=\"ui-stack pager-target\" tabindex=\"-1\" aria-label=\"Announcements\">"
                ),
                "{html}"
            );
        }
    }

    #[test]
    fn long_list_is_paged_ten_at_a_time() {
        let items: Vec<Announcement> = (1..=12).map(|n| item(n, n == 1)).collect();
        let first = render(Load::Ready(items.clone()), Some(names()), 1);
        assert_eq!(first.matches("class=\"entry-row\"").count(), 10, "{first}");
        assert!(
            first.contains(">1 to 10 of 12 announcements</p>"),
            "{first}"
        );

        let second = render(Load::Ready(items), Some(names()), 2);
        assert_eq!(second.matches("class=\"entry-row\"").count(), 2, "{second}");
        assert!(second.contains("<span>Notice 12</span>"), "{second}");
        assert!(!second.contains("Pinned"), "{second}");
    }
}
