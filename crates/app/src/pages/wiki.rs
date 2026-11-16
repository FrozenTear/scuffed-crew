use dioxus::prelude::*;
use serde::Deserialize;

use scuffed_api_client::ApiClient;

use super::article_author::{
    ARTICLE_CSS, ARTICLE_CSS_ID, AuthorNames, Batched, ENTRY_PAGE_SIZE, LIST_BATCH, Load,
    author_byline, entry_date, fetch_offset_list, is_retrying, list_status, use_author_names,
};
use crate::components::ui::{
    EmptyCard, LoadError, PageHeader, PageShell, PageWidth, Pager, page_slice, show_list_top,
};
use crate::markdown::plain_excerpt;
use crate::routes::Route;

#[derive(Debug, Clone, Deserialize)]
struct WikiPageSummary {
    id: String,
    topic: String,
    title: String,
    content_markdown: String,
    author_member_id: String,
    updated_at: String,
}

#[derive(Debug, Clone, Deserialize)]
struct WikiListResponse {
    data: Vec<WikiPageSummary>,
}

/// One row of the index. The excerpt is cut from the page body once, when the
/// list arrives, so the full markdown is not kept or cloned per render.
#[derive(Debug, Clone, PartialEq)]
struct WikiEntry {
    id: String,
    topic: String,
    title: String,
    excerpt: String,
    author_member_id: String,
    updated_at: String,
}

const EXCERPT_CHARS: usize = 140;

impl From<WikiPageSummary> for WikiEntry {
    fn from(page: WikiPageSummary) -> Self {
        WikiEntry {
            excerpt: plain_excerpt(&page.content_markdown, EXCERPT_CHARS),
            id: page.id,
            topic: page.topic,
            title: page.title,
            author_member_id: page.author_member_id,
            updated_at: page.updated_at,
        }
    }
}

/// The list endpoint takes `q`, `limit` (at most 100) and `offset` and
/// returns no total. The page asks for the next offset while a batch comes
/// back full (see `fetch_offset_list`) and pages through the rows here.
/// Search runs on the server.
fn list_url(query: &str, offset: usize) -> String {
    let query = query.trim();
    let base = format!("/api/wiki?limit={LIST_BATCH}&offset={offset}");
    if query.is_empty() {
        base
    } else {
        format!("{base}&q={}", crate::util::encode_query(query))
    }
}

/// One finished list fetch: the rows and the search text they answer. The
/// text travels with the rows, so the empty state and the status line speak
/// about the list on screen and not about what is in the field right now.
#[derive(Debug, Clone, PartialEq)]
struct WikiFound {
    query: String,
    entries: Batched<WikiEntry>,
}

/// The list section. It takes focus after a page change.
const LIST_ID: &str = "wiki-list";

/// Pause after the last keystroke before the search runs.
const SEARCH_DEBOUNCE_MS: u32 = 300;

const NO_MATCH_STATUS: &str = "No pages match your search.";

const PAGE_CSS: &str = r#"
/* The field spans the column, so its edges line up with the cards under
   it (the frames' toolbar search fills its row the same way). */
.wiki-search {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
  min-width: 0;
}
"#;

async fn pause(ms: u32) {
    #[cfg(feature = "web")]
    {
        gloo_timers::future::TimeoutFuture::new(ms).await;
    }
    #[cfg(not(feature = "web"))]
    {
        tokio::time::sleep(std::time::Duration::from_millis(u64::from(ms))).await;
    }
}

#[component]
pub fn Wiki() -> Element {
    // What the field shows, and what the list was last asked for.
    let mut input_text = use_signal(String::new);
    let mut search_text = use_signal(String::new);
    let mut page = use_signal(|| 1usize);
    let mut refresh = use_signal(|| 0u32);

    // The fetch follows the field after a pause in typing, not per keystroke.
    let _debounce = use_resource(move || {
        let current = input_text();
        async move {
            pause(SEARCH_DEBOUNCE_MS).await;
            if *search_text.peek() != current {
                search_text.set(current);
                page.set(1);
            }
        }
    });

    let pages = use_resource(move || {
        let query = search_text();
        let _ = refresh();
        async move {
            let entries = fetch_offset_list(|offset| {
                let url = list_url(&query, offset);
                async move {
                    ApiClient::web()
                        .fetch::<WikiListResponse>(&url)
                        .await
                        .map(|resp| {
                            resp.data
                                .into_iter()
                                .map(WikiEntry::from)
                                .collect::<Vec<_>>()
                        })
                        .map_err(|e| e.to_string())
                }
            })
            .await?;
            Ok::<_, String>(WikiFound { query, entries })
        }
    });
    // Names for the authors of the page on screen only.
    let names = use_author_names(move || {
        let page = page();
        pages
            .read()
            .as_ref()
            .and_then(|r| r.as_ref().ok())
            .map(|found| {
                page_slice(&found.entries.items, page, ENTRY_PAGE_SIZE)
                    .iter()
                    .map(|p| p.author_member_id.clone())
                    .collect()
            })
            .unwrap_or_default()
    });
    let (load, capped, list_query) = match Load::of(pages.read().as_ref()) {
        Load::Ready(found) => (
            Load::Ready(found.entries.items),
            found.entries.truncated,
            found.query,
        ),
        Load::Loading => (Load::Loading, false, String::new()),
        Load::Failed => (Load::Failed, false, String::new()),
        Load::Missing => (Load::Missing, false, String::new()),
    };
    let state = pages.state()();
    let retrying = is_retrying(state, &load);
    // `use_resource` keeps the last list while the next search is in flight.
    let refetching = state == UseResourceState::Pending && matches!(load, Load::Ready(_));

    rsx! {
        WikiIndex {
            load,
            names: Some(names()),
            query: input_text(),
            list_query,
            page: page(),
            capped,
            retrying,
            refetching,
            on_retry: move |_| refresh += 1,
            on_query: move |text| input_text.set(text),
            on_page: move |next| {
                page.set(next);
                show_list_top(LIST_ID);
            },
        }
    }
}

/// Text of the status line: loading (first load, retry or a new search), a
/// search with no hits, or a cut list.
fn wiki_status(
    load: &Load<Vec<WikiEntry>>,
    list_query: &str,
    capped: bool,
    retrying: bool,
    refetching: bool,
) -> String {
    if refetching {
        return "Loading wiki pages".to_string();
    }
    match load {
        Load::Ready(list) if list.is_empty() && !list_query.trim().is_empty() => {
            NO_MATCH_STATUS.to_string()
        }
        _ => list_status(load, retrying, capped, "Loading wiki pages", "pages"),
    }
}

/// The page for one fetch state. The status line and the pager are always in
/// the tree; only their text changes.
#[component]
fn WikiIndex(
    load: Load<Vec<WikiEntry>>,
    names: Option<AuthorNames>,
    /// Text in the search field.
    query: String,
    /// Search text the list on screen answers.
    list_query: String,
    page: usize,
    /// The load stopped at its batch budget: more pages exist.
    #[props(default = false)]
    capped: bool,
    /// A Retry is running.
    #[props(default = false)]
    retrying: bool,
    /// A new search is in flight while the last list is still shown.
    #[props(default = false)]
    refetching: bool,
    on_retry: EventHandler<()>,
    on_query: EventHandler<String>,
    on_page: EventHandler<usize>,
) -> Element {
    let status = wiki_status(&load, &list_query, capped, retrying, refetching);
    let total = match &load {
        Load::Ready(list) => list.len(),
        _ => 0,
    };
    let searched = !list_query.trim().is_empty();

    rsx! {
        crate::styles::StyleOnce { id: ARTICLE_CSS_ID, css: ARTICLE_CSS }
        crate::styles::StyleOnce { id: "scuffed-css-pages-wiki-page-css", css: PAGE_CSS }

        PageShell { width: PageWidth::Default, class: "wiki-page",
            PageHeader {
                title: "Wiki",
                lead: "Living notes for the crew.",
            }
            div { class: "wiki-search", role: "search",
                label { class: "field-label", r#for: "wiki-search", "Search the wiki" }
                input {
                    id: "wiki-search",
                    class: "field-input",
                    name: "wiki-search",
                    r#type: "search",
                    placeholder: "Title or topic",
                    value: "{query}",
                    oninput: move |e| on_query.call(e.value()),
                }
            }
            section {
                id: LIST_ID,
                class: "ui-stack pager-target",
                tabindex: "-1",
                aria_label: "Wiki pages",
                // With no hits the card below says the same thing, so the
                // line is for assistive tech only then.
                p {
                    class: if status == NO_MATCH_STATUS { "entry-status visually-hidden" } else { "entry-status" },
                    role: "status",
                    "{status}"
                }
                {match &load {
                    Load::Loading => rsx! {},
                    Load::Failed | Load::Missing => rsx! {
                        LoadError {
                            message: "Couldn't load the wiki. Check your connection and try again.",
                            on_retry: move |_| on_retry.call(()),
                            retrying,
                        }
                    },
                    Load::Ready(list) if list.is_empty() && searched => rsx! {
                        EmptyCard {
                            title: "No pages match",
                            message: "Try a shorter search, or clear it to see every page.",
                        }
                    },
                    Load::Ready(list) if list.is_empty() => rsx! {
                        EmptyCard {
                            title: "No pages yet",
                            message: "Wiki pages written by the crew will show up here.",
                        }
                    },
                    Load::Ready(list) => rsx! {
                        div { class: "entry-list",
                            for entry in page_slice(list, page, ENTRY_PAGE_SIZE) {
                                {render_wiki_card(entry, names.as_ref())}
                            }
                        }
                    },
                }}
                Pager {
                    id: "wiki-pager",
                    page,
                    total,
                    page_size: ENTRY_PAGE_SIZE,
                    noun: "pages",
                    noun_singular: "page",
                    on_change: move |next| on_page.call(next),
                }
            }
        }
    }
}

/// The page row only names who started the page, so the credit is "By", and
/// the date is the last update.
fn render_wiki_card(entry: &WikiEntry, names: Option<&AuthorNames>) -> Element {
    rsx! {
        article { key: "{entry.id}", class: "entry-row",
            h2 {
                Link { to: Route::WikiPage { topic: entry.topic.clone() }, "{entry.title}" }
            }
            if !entry.excerpt.is_empty() {
                p { "{entry.excerpt}" }
            }
            p { class: "entry-meta",
                {author_byline(names, &entry.author_member_id, "By")}
                span { "Updated" }
                {entry_date(&entry.updated_at)}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::article_author::test_support::render_routed;
    use super::*;

    const AUTHOR: &str = "member:3b7e01c9aa";

    fn entry(n: usize) -> WikiEntry {
        WikiEntry {
            id: format!("wiki_page:{n}"),
            topic: format!("topic-{n}"),
            title: format!("Page {n}"),
            excerpt: format!("Excerpt {n}"),
            author_member_id: AUTHOR.to_string(),
            updated_at: "2026-11-10T08:00:00Z".to_string(),
        }
    }

    fn names() -> AuthorNames {
        AuthorNames::from([(AUTHOR.to_string(), Some("Player03".to_string()))])
    }

    #[derive(Clone, PartialEq)]
    struct Scene {
        load: Load<Vec<WikiEntry>>,
        query: String,
        list_query: String,
        page: usize,
        capped: bool,
        retrying: bool,
        refetching: bool,
    }

    fn scene(load: Load<Vec<WikiEntry>>, query: &str, page: usize) -> Scene {
        Scene {
            load,
            query: query.to_string(),
            list_query: query.to_string(),
            page,
            capped: false,
            retrying: false,
            refetching: false,
        }
    }

    fn render_scene(scene: Scene) -> String {
        render_routed(move || {
            let scene = scene.clone();
            rsx! {
                WikiIndex {
                    load: scene.load,
                    names: Some(names()),
                    query: scene.query,
                    list_query: scene.list_query,
                    page: scene.page,
                    capped: scene.capped,
                    retrying: scene.retrying,
                    refetching: scene.refetching,
                    on_retry: move |_| {},
                    on_query: move |_| {},
                    on_page: move |_| {},
                }
            }
        })
    }

    fn render(load: Load<Vec<WikiEntry>>, query: &str, page: usize) -> String {
        render_scene(scene(load, query, page))
    }

    #[test]
    fn row_shows_title_link_excerpt_author_name_and_update_date() {
        let html = render(Load::Ready(vec![entry(1)]), "", 1);
        assert!(!html.contains("<main"), "{html}");
        assert_eq!(html.matches("<h1").count(), 1, "{html}");
        assert!(html.contains("<h1>Wiki</h1>"), "{html}");
        assert!(html.contains("class=\"ui-shell wiki-page\""), "{html}");
        assert!(html.contains("href=\"/wiki/topic-1\""), "{html}");
        assert!(html.contains(">Page 1</a>"), "{html}");
        assert!(html.contains("<p>Excerpt 1</p>"), "{html}");
        assert!(html.contains(">By Player03</span>"), "{html}");
        assert!(
            html.contains(
                "<span>Updated</span><time class=\"entry-date\" datetime=\"2026-11-10\">10 Nov 2026</time>"
            ),
            "{html}"
        );
        assert!(!html.contains(AUTHOR), "raw author id leaked: {html}");
    }

    #[test]
    fn excerpt_comes_from_the_page_body_as_plain_text() {
        let page = WikiPageSummary {
            id: "wiki_page:1".to_string(),
            topic: "tank-basics".to_string(),
            title: "Tank basics".to_string(),
            content_markdown:
                "# Tank basics\n\nSpace, **peels**, and when to hold a shield.\n\nMore.\n"
                    .to_string(),
            author_member_id: AUTHOR.to_string(),
            updated_at: "2026-11-10T08:00:00Z".to_string(),
        };
        let entry = WikiEntry::from(page);
        assert_eq!(entry.excerpt, "Space, peels, and when to hold a shield.");
        assert_eq!(entry.topic, "tank-basics");
    }

    #[test]
    fn search_field_has_a_visible_label_and_keeps_its_text() {
        let html = render(Load::Ready(vec![entry(1)]), "tank", 1);
        assert!(
            html.contains(
                "<label class=\"field-label\" for=\"wiki-search\">Search the wiki</label>"
            ),
            "{html}"
        );
        assert!(html.contains("id=\"wiki-search\""), "{html}");
        assert!(html.contains("value=\"tank\""), "{html}");
    }

    #[test]
    fn search_text_is_encoded_into_the_list_url() {
        assert_eq!(list_url("", 0), "/api/wiki?limit=100&offset=0");
        assert_eq!(list_url("   ", 0), "/api/wiki?limit=100&offset=0");
        assert_eq!(
            list_url("tank basics", 0),
            "/api/wiki?limit=100&offset=0&q=tank%20basics"
        );
        assert_eq!(
            list_url("a&b=c", 100),
            "/api/wiki?limit=100&offset=100&q=a%26b%3Dc"
        );
    }

    #[test]
    fn empty_states_tell_a_search_miss_from_an_empty_wiki() {
        let none = render(Load::Ready(Vec::new()), "", 1);
        assert!(none.contains("<h2>No pages yet</h2>"), "{none}");
        let miss = render(Load::Ready(Vec::new()), "zzz", 1);
        assert!(miss.contains("<h2>No pages match</h2>"), "{miss}");
        assert!(!miss.contains("No pages yet"), "{miss}");
        // A search with no hits is announced, not only drawn.
        assert!(
            miss.contains(
                "<p class=\"entry-status visually-hidden\" role=\"status\">No pages match your search.</p>"
            ),
            "{miss}"
        );
        assert!(
            none.contains("<p class=\"entry-status\" role=\"status\"></p>"),
            "{none}"
        );
    }

    #[test]
    fn empty_state_follows_the_list_on_screen_not_the_field() {
        // "zzz" found nothing; the field was cleared and the next fetch is
        // still in flight. The list on screen is still the answer to "zzz".
        let mut cleared = scene(Load::Ready(Vec::new()), "", 1);
        cleared.list_query = "zzz".to_string();
        cleared.refetching = true;
        let html = render_scene(cleared);
        assert!(html.contains("<h2>No pages match</h2>"), "{html}");
        assert!(!html.contains("No pages yet"), "{html}");
        assert!(html.contains("value=\"\""), "{html}");
        assert!(
            html.contains("<p class=\"entry-status\" role=\"status\">Loading wiki pages</p>"),
            "{html}"
        );
    }

    #[test]
    fn status_text_covers_search_retry_and_a_cut_list() {
        let some = Load::Ready(vec![entry(1), entry(2)]);
        let none: Load<Vec<WikiEntry>> = Load::Ready(Vec::new());
        assert_eq!(wiki_status(&some, "", false, false, false), "");
        assert_eq!(wiki_status(&some, "tank", false, false, false), "");
        assert_eq!(wiki_status(&none, "", false, false, false), "");
        assert_eq!(
            wiki_status(&none, "zzz", false, false, false),
            NO_MATCH_STATUS
        );
        assert_eq!(
            wiki_status(&some, "tank", false, false, true),
            "Loading wiki pages"
        );
        assert_eq!(
            wiki_status(&Load::Failed, "", false, true, false),
            "Loading wiki pages"
        );
        assert_eq!(
            wiki_status(&some, "", true, false, false),
            "Showing the newest 2 pages."
        );
    }

    #[test]
    fn loading_and_failed_states_keep_the_live_regions() {
        let loading = render(Load::Loading, "", 1);
        assert!(
            loading.contains("<p class=\"entry-status\" role=\"status\">Loading wiki pages</p>"),
            "{loading}"
        );
        let failed = render(Load::Failed, "", 1);
        assert!(failed.contains("role=\"alert\""), "{failed}");
        assert!(
            failed.contains("load the wiki. Check your connection"),
            "{failed}"
        );
        assert!(failed.contains(">Retry</button>"), "{failed}");
        for html in [&loading, &failed] {
            assert!(
                html.contains("class=\"entry-status\" role=\"status\""),
                "{html}"
            );
            assert!(html.contains("id=\"wiki-pager-meta\""), "{html}");
            assert!(html.contains("id=\"wiki-search\""), "{html}");
            // The list section is the focus target after a page change.
            assert!(
                html.contains(
                    "<section id=\"wiki-list\" class=\"ui-stack pager-target\" tabindex=\"-1\" aria-label=\"Wiki pages\">"
                ),
                "{html}"
            );
        }
        let mut retry = scene(Load::Failed, "", 1);
        retry.retrying = true;
        let retrying = render_scene(retry);
        assert!(retrying.contains(">Retrying...</button>"), "{retrying}");
        assert!(retrying.contains("aria-disabled=\"true\""), "{retrying}");
        assert!(!retrying.contains(" disabled"), "{retrying}");
        assert!(
            retrying.contains("<p class=\"entry-status\" role=\"status\">Loading wiki pages</p>"),
            "{retrying}"
        );
    }

    #[test]
    fn long_list_is_paged_ten_at_a_time() {
        let entries: Vec<WikiEntry> = (1..=11).map(entry).collect();
        let first = render(Load::Ready(entries.clone()), "", 1);
        assert_eq!(first.matches("class=\"entry-row\"").count(), 10, "{first}");
        assert!(first.contains(">1 to 10 of 11 pages</p>"), "{first}");
        let second = render(Load::Ready(entries), "", 2);
        assert_eq!(second.matches("class=\"entry-row\"").count(), 1, "{second}");
        assert!(second.contains(">11 of 11 pages</p>"), "{second}");
    }
}
