//! Public "What's changed" page: `/changelog/site` and `/changelog/tracker`.
//!
//! The content is two static TOML files embedded at build time (see
//! [`data`]), so the page needs no server route and no database. The Site /
//! Tracker switch is two real links, so each tab is its own URL.

mod data;

use std::cell::Cell;
use std::rc::Rc;

use dioxus::prelude::*;

pub use data::Tab;
use data::{Entry, ListState};

use crate::layouts::focus_element;
use crate::pages::{NotFoundLink, NotFoundPanel};
use crate::routes::Route;

const CHANGELOG_CSS: &str = r#"
    .changelog-page {
        max-width: var(--content-max-narrow);
        margin: 0 auto;
        padding: var(--space-8) var(--space-page) var(--space-12);
        font-family: var(--font-body);
        color: var(--text);
        min-width: 0;
    }
    .changelog-page .page-header {
        margin: 0 0 var(--space-6);
    }
    .changelog-page .page-header h1 {
        font-family: var(--font-head);
        font-weight: var(--font-head-weight);
        letter-spacing: var(--font-head-tracking);
        font-size: var(--text-3xl);
        line-height: var(--leading-tight);
        color: var(--text);
        margin: 0;
    }
    .changelog-page .page-lead {
        font-size: var(--text-md);
        line-height: var(--leading-body);
        color: var(--text-3);
        margin: var(--space-2) 0 0;
    }
    .cl-toggle {
        display: inline-flex;
        max-width: 100%;
        gap: var(--space-1);
        padding: var(--space-1);
        margin: 0 0 var(--space-5);
        border: 1px solid var(--card-border);
        border-radius: var(--radius-pill);
        background: var(--surface);
    }
    .cl-toggle a {
        display: inline-flex;
        align-items: center;
        justify-content: center;
        min-height: var(--hit-target-min);
        min-width: var(--hit-target-min);
        padding: 0 var(--space-5);
        border-radius: var(--radius-pill);
        font-family: var(--font-body);
        font-size: var(--text-md);
        font-weight: var(--font-weight-600);
        color: var(--text);
        text-decoration: none;
    }
    .cl-toggle a:hover { background: var(--surface-sunken); }
    .cl-toggle a[aria-current="page"],
    .cl-toggle a[aria-current="page"]:hover {
        background: var(--accent-solid);
        color: var(--on-accent);
    }
    .changelog-page a:focus-visible,
    .changelog-page button:focus-visible,
    .changelog-page h2:focus-visible {
        outline: var(--focus-ring-width) solid var(--focus-ring);
        outline-offset: var(--focus-ring-offset);
    }
    .cl-entry + .cl-entry { margin-top: var(--space-6); }
    .changelog-page h2.section-title {
        font-family: var(--font-head);
        font-weight: var(--font-head-weight);
        font-size: var(--text-xl);
        line-height: var(--leading-snug);
        letter-spacing: var(--font-head-tracking);
        text-transform: none;
        color: var(--text);
        margin: 0;
    }
    .cl-meta {
        font-size: var(--text-xs);
        color: var(--text-3);
        margin: var(--space-1) 0 0;
    }
    .cl-link {
        display: inline-block;
        margin-top: var(--space-2);
        font-size: var(--text-sm);
        color: var(--link);
    }
    .cl-link:hover { color: var(--link-hover); }
    @media (max-width: 820px) {
        /* A standalone link, so it gets the phone target, not the inline exception. */
        .cl-link {
            display: inline-flex;
            align-items: center;
            min-height: var(--hit-target-min);
            margin-top: 0;
        }
    }
    .changelog-page h3 {
        font-family: var(--font-head);
        font-weight: var(--font-weight-600);
        font-size: var(--text-md);
        color: var(--text);
        margin: var(--space-3) 0 0;
    }
    .cl-items {
        margin: var(--space-2) 0 0;
        padding-left: var(--space-5);
        font-size: var(--text-md);
        line-height: var(--leading-body);
        color: var(--text);
        overflow-wrap: anywhere;
    }
    .cl-items li + li { margin-top: var(--space-1); }
    .cl-more {
        margin-top: var(--space-6);
    }
    .changelog-page .cl-btn {
        min-height: var(--hit-target-min);
        justify-content: center;
    }
    .cl-empty-title {
        font-family: var(--font-head);
        font-size: var(--text-lg);
        color: var(--text);
        margin: 0 0 var(--space-2);
    }
    .changelog-page .ui-empty {
        border: 1px solid var(--card-border);
        border-radius: var(--radius-card);
        background: var(--surface);
    }
    .changelog-page .ui-empty p { margin: 0; }
    .changelog-page .ui-empty .cl-empty-title { margin-bottom: var(--space-2); }
    .cl-error {
        display: flex;
        flex-wrap: wrap;
        align-items: center;
        gap: var(--space-3);
    }
    .cl-error p {
        margin: 0;
        color: var(--text);
        font-size: var(--text-md);
    }
    .cl-live {
        position: absolute;
        width: 1px;
        height: 1px;
        padding: 0;
        margin: -1px;
        overflow: hidden;
        clip: rect(0, 0, 0, 0);
        white-space: nowrap;
        border: 0;
    }
"#;

fn tab_route(tab: Tab) -> Route {
    Route::Changelog {
        tab: tab.slug().to_string(),
    }
}

/// Links offered by the changelog not-found card.
pub(crate) fn changelog_not_found_links() -> Vec<NotFoundLink> {
    vec![
        NotFoundLink {
            label: "Site changes",
            to: tab_route(Tab::Site),
        },
        NotFoundLink {
            label: "Tracker changes",
            to: tab_route(Tab::Tracker),
        },
    ]
}

#[component]
pub fn ChangelogNotFound(path: String) -> Element {
    rsx! {
        NotFoundPanel {
            title: "Page not found",
            message: "There is no changelog here. Pick the Site or the Tracker changes.",
            path,
            links: changelog_not_found_links(),
        }
    }
}

/// Route component. An unknown tab is the in-chrome not-found card. The
/// page itself is a child component so its hooks never change count when
/// the tab prop flips between known and unknown.
#[component]
pub fn Changelog(tab: String) -> Element {
    match Tab::from_slug(&tab) {
        Some(tab) => rsx! { ChangelogPage { tab } },
        None => rsx! { ChangelogNotFound { path: format!("/changelog/{tab}") } },
    }
}

fn entry_heading_id(tab: Tab, index: usize) -> String {
    format!("cl-{}-{index}", tab.slug())
}

/// Tracks toggle navigations. A cold load stays quiet; once the tab prop
/// differs from the last render, the live region may speak.
#[derive(Clone)]
struct SwitchTracker {
    last: Rc<Cell<Tab>>,
    switched: Rc<Cell<bool>>,
}

impl SwitchTracker {
    fn new(tab: Tab) -> Self {
        Self {
            last: Rc::new(Cell::new(tab)),
            switched: Rc::new(Cell::new(false)),
        }
    }

    /// Record this render's tab and say whether any switch has happened.
    fn observe(&self, tab: Tab) -> bool {
        if self.last.replace(tab) != tab {
            self.switched.set(true);
        }
        self.switched.get()
    }
}

#[component]
fn ChangelogPage(tab: Tab) -> Element {
    let tracker = use_hook(|| SwitchTracker::new(tab));
    let after_switch = tracker.observe(tab);
    // (tab, shown). A tab switch starts the list over at the first batch.
    let mut shown = use_signal(|| (tab, 0usize));
    // Retry re-reads the list only. With an embedded source it reparses.
    let mut attempt = use_signal(|| 0u32);
    let mut pending_focus = use_signal(|| None::<String>);

    use_effect(move || {
        if let Some(id) = pending_focus() {
            focus_element(&id);
            pending_focus.set(None);
        }
    });

    let _ = attempt();
    let loaded = data::load(tab);
    if let Err(err) = &loaded {
        tracing::error!("changelog: {err}");
    }

    let (state, list) = match &loaded {
        Ok(entries) => {
            let (shown_tab, count) = shown();
            let count = if shown_tab == tab {
                count
                    .max(data::first_batch(entries.len()))
                    .min(entries.len())
            } else {
                data::first_batch(entries.len())
            };
            let state = ListState::Ready { shown: count };
            let total = entries.len();
            let list = if total == 0 {
                rsx! {
                    div { class: "ui-empty",
                        p { class: "cl-empty-title", "No changes yet." }
                        p { "{data::empty_text(tab)}" }
                    }
                }
            } else {
                rsx! {
                    for (i, entry) in entries.iter().take(count).enumerate() {
                        ChangelogEntry {
                            key: "{tab.slug()}-{i}",
                            heading_id: entry_heading_id(tab, i),
                            entry: entry.clone(),
                        }
                    }
                    if data::has_older(count, total) {
                        div { class: "cl-more",
                            button {
                                r#type: "button",
                                class: "ui-btn ui-btn--ghost ui-btn--md cl-btn",
                                onclick: move |_| {
                                    let next = data::next_batch(count, total);
                                    shown.set((tab, next));
                                    pending_focus.set(Some(entry_heading_id(tab, count)));
                                },
                                "Show older"
                            }
                        }
                    }
                }
            };
            (state, list)
        }
        Err(_) => (
            ListState::Error,
            rsx! {
                div { class: "cl-error",
                    p { "{data::error_text(tab)}" }
                    button {
                        r#type: "button",
                        class: "ui-btn ui-btn--ghost ui-btn--md cl-btn",
                        onclick: move |_| attempt += 1,
                        "Retry"
                    }
                }
            },
        ),
    };
    let live = data::live_text(tab, state, after_switch);

    rsx! {
        crate::styles::StyleOnce { id: "scuffed-css-pages-changelog", css: CHANGELOG_CSS }
        div { class: "changelog-page",
            header { class: "page-header",
                h1 { "What's changed" }
                p { class: "page-lead", "Updates to the Scuffed Crew site and the Overwatch tracker." }
            }
            nav { class: "cl-toggle", aria_label: "What's changed",
                for option in Tab::ALL {
                    Link {
                        key: "{option.slug()}",
                        to: tab_route(option),
                        // Link sets aria-current="page" on the active route itself.
                        "{option.label()}"
                    }
                }
            }
            p { class: "cl-live", role: "status", aria_live: "polite", "{live}" }
            section {
                class: "cl-list",
                aria_label: "{tab.label()} changes",
                {list}
            }
        }
    }
}

#[component]
fn ChangelogEntry(heading_id: String, entry: Entry) -> Element {
    let iso = entry.date_iso();
    let date = entry.date_label();
    let link_context = entry.version.clone().unwrap_or_else(|| date.clone());
    rsx! {
        article { class: "cl-entry",
            if let Some(version) = &entry.version {
                h2 { class: "section-title", id: "{heading_id}", tabindex: "-1", "{version}" }
                p { class: "cl-meta",
                    time { datetime: "{iso}", "{date}" }
                }
            } else {
                h2 { class: "section-title", id: "{heading_id}", tabindex: "-1",
                    time { datetime: "{iso}", "{date}" }
                }
            }
            if let Some(link) = &entry.link {
                a {
                    class: "cl-link",
                    href: "{link}",
                    rel: "noopener noreferrer",
                    "Public release notes"
                    span { class: "cl-live", " for {link_context}" }
                }
            }
            if !entry.changes.is_empty() {
                ul { class: "cl-items",
                    for (i, item) in entry.changes.iter().enumerate() {
                        li { key: "{i}", "{item}" }
                    }
                }
            }
            for group in entry.groups.iter() {
                Fragment { key: "{group.kind.heading()}",
                    h3 { "{group.kind.heading()}" }
                    ul { class: "cl-items",
                        for (i, item) in group.items.iter().enumerate() {
                            li { key: "{i}", "{item}" }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;
    use crate::state::AuthState;

    thread_local! {
        static ROUTE_PATH: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
        static HISTORY: std::cell::RefCell<Option<std::rc::Rc<dioxus::history::MemoryHistory>>> =
            const { std::cell::RefCell::new(None) };
    }

    fn shell() -> Element {
        let path = ROUTE_PATH.with(|slot| slot.borrow().clone());
        let history = use_hook(|| {
            let h = std::rc::Rc::new(dioxus::history::MemoryHistory::with_initial_path(path));
            HISTORY.with(|slot| *slot.borrow_mut() = Some(h.clone()));
            h
        });
        let auth = use_signal(|| AuthState {
            user: None,
            loading: false,
        });
        use_context_provider(|| auth);
        crate::state::provide_site_settings();
        rsx! {
            crate::theme::ThemeProvider {
                dioxus::router::components::HistoryProvider {
                    history: move |_| history.clone() as std::rc::Rc<dyn dioxus::history::History>,
                    Router::<Route> {}
                }
            }
        }
    }

    /// Render `path` with `source` as both tabs' file (None = shipped files).
    /// Returns the HTML and the URL the router settled on.
    fn render_with(path: &str, source: Option<&str>) -> (String, String) {
        data::TEST_SOURCE.with(|slot| *slot.borrow_mut() = source.map(String::from));
        ROUTE_PATH.with(|slot| *slot.borrow_mut() = path.to_string());
        let mut dom = VirtualDom::new(shell);
        dom.rebuild_in_place();
        let html = dioxus_ssr::render(&dom);
        let url = HISTORY.with(|slot| {
            use dioxus::history::History;
            slot.borrow().as_ref().unwrap().current_route()
        });
        data::TEST_SOURCE.with(|slot| *slot.borrow_mut() = None);
        (html, url)
    }

    fn render(path: &str, source: Option<&str>) -> String {
        render_with(path, source).0
    }

    fn page_of(html: &str) -> &str {
        let start = html
            .find("class=\"changelog-page\"")
            .expect("changelog page");
        &html[start..]
    }

    fn fake_site(n: usize) -> String {
        let mut out = String::new();
        for i in 0..n {
            let day =
                chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap() + chrono::Days::new(i as u64);
            out.push_str(&format!(
                "[[entry]]\ndate = \"{}\"\nchanges = [\"Fake change {i}.\"]\n\n",
                day.format("%Y-%m-%d")
            ));
        }
        out
    }

    #[test]
    fn changelog_redirects_to_the_site_tab_and_the_url_follows() {
        assert_eq!(
            Route::from_str("/changelog").unwrap(),
            Route::Changelog { tab: "site".into() }
        );
        assert_eq!(
            Route::Changelog { tab: "site".into() }.to_string(),
            "/changelog/site"
        );
        let (html, url) = render_with("/changelog", None);
        assert_eq!(url, "/changelog/site");
        assert!(html.contains("What&#39;s changed") || html.contains("What's changed"));
        assert!(page_of(&html).contains("aria-current=\"page\""), "{html}");
    }

    #[test]
    fn tracker_tab_renders_and_unknown_values_are_the_in_chrome_not_found() {
        assert_eq!(
            Route::from_str("/changelog/tracker").unwrap(),
            Route::Changelog {
                tab: "tracker".into()
            }
        );
        for path in [
            "/changelog/unknown",
            "/changelog/site/extra",
            "/changelog/Site",
        ] {
            let (html, url) = render_with(path, None);
            assert_eq!(url, path, "an unknown tab is not redirected");
            assert!(html.contains("nf-page"), "{path}: {html}");
            assert!(html.contains("d1-top"), "{path}: keeps the site chrome");
            assert!(html.contains("href=\"/changelog/site\""), "{path}: {html}");
            assert!(
                html.contains("href=\"/changelog/tracker\""),
                "{path}: {html}"
            );
            assert!(!html.contains("changelog-page"), "{path}");
        }
    }

    #[test]
    fn aria_current_is_on_the_active_toggle_link_only() {
        for (path, active, other) in [
            ("/changelog/site", "/changelog/site", "/changelog/tracker"),
            (
                "/changelog/tracker",
                "/changelog/tracker",
                "/changelog/site",
            ),
        ] {
            let html = render(path, None);
            let page = page_of(&html);
            let nav_start = page.find("<nav").unwrap();
            let nav = &page[nav_start..nav_start + page[nav_start..].find("</nav>").unwrap()];
            assert!(
                nav.contains("aria-label=\"What&#39;s changed\"")
                    || nav.contains("aria-label=\"What's changed\""),
                "{nav}"
            );
            assert_eq!(nav.matches("aria-current").count(), 1, "{nav}");
            let active_at = nav.find(&format!("href=\"{active}\"")).unwrap();
            let current_at = nav.find("aria-current=\"page\"").unwrap();
            let other_at = nav.find(&format!("href=\"{other}\"")).unwrap();
            // The attribute sits on the active link's tag.
            let tag_start = nav[..current_at].rfind("<a").unwrap();
            assert!(
                tag_start <= active_at
                    && active_at < nav[tag_start..].find('>').unwrap() + tag_start,
                "{nav}"
            );
            assert!(!(tag_start..current_at).contains(&other_at), "{nav}");
            assert!(!nav.contains("role=\"tab"), "links, not a tablist: {nav}");
        }
    }

    #[test]
    fn one_h1_an_h2_per_entry_and_dated_with_time() {
        let src = "[[entry]]\ndate = \"2026-11-15\"\nadded = [\"Fake add.\"]\nfixed = [\"Fake fix.\"]\nlink = \"https://example.com/notes\"\n\n[[entry]]\ndate = \"2026-10-01\"\nchanges = [\"Fake change.\"]\n";
        let html = render("/changelog/site", Some(src));
        assert_eq!(html.matches("<h1").count(), 1, "{html}");
        let page = page_of(&html);
        assert_eq!(
            page.matches("<h2 class=\"section-title\"").count(),
            2,
            "{page}"
        );
        assert!(
            page.contains("<time datetime=\"2026-11-15\">15 Nov 2026</time>"),
            "{page}"
        );
        // Newest first.
        assert!(page.find("2026-11-15").unwrap() < page.find("2026-10-01").unwrap());
        // Only the groups the source has.
        assert!(page.contains("<h3>Added</h3>"), "{page}");
        assert!(page.contains("<h3>Fixed</h3>"), "{page}");
        assert!(!page.contains("<h3>Changed</h3>"), "{page}");
        assert_eq!(page.matches("<h3>").count(), 2, "{page}");
        // One public link, on the entry that has one.
        assert_eq!(page.matches("Public release notes").count(), 1, "{page}");
        assert!(
            page.contains("href=\"https://example.com/notes\""),
            "{page}"
        );
        // Cold load: the live region is present and quiet.
        assert!(
            page.contains("role=\"status\" aria-live=\"polite\"></p>"),
            "{page}"
        );
    }

    #[test]
    fn tracker_entries_show_a_version_and_a_date() {
        let src = "[[entry]]\nversion = \"v0.6.0-alpha.3.1\"\ndate = \"2026-11-02\"\nchanges = [\"Fake tracker change.\"]\n";
        let html = render("/changelog/tracker", Some(src));
        let page = page_of(&html);
        assert!(page.contains(">v0.6.0-alpha.3.1</h2>"), "{page}");
        assert!(
            page.contains("class=\"cl-meta\"><time datetime=\"2026-11-02\">2 Nov 2026</time>"),
            "{page}"
        );
        assert!(
            !page.contains("Public release notes"),
            "private release, no link"
        );
    }

    #[test]
    fn first_paint_is_ten_entries_with_show_older() {
        let html = render("/changelog/site", Some(&fake_site(23)));
        let page = page_of(&html);
        assert_eq!(page.matches("<article").count(), 10, "{page}");
        assert!(page.contains(">Show older</button>"), "{page}");
        // Newest first: day 23 (index 22) leads.
        assert!(page.contains("Fake change 22."), "{page}");
        assert!(!page.contains("Fake change 12."), "{page}");

        let html = render("/changelog/site", Some(&fake_site(10)));
        assert_eq!(page_of(&html).matches("<article").count(), 10);
        assert!(
            !html.contains("Show older"),
            "no button when nothing is left"
        );
    }

    #[test]
    fn empty_and_error_states_match_the_copy() {
        let html = render("/changelog/tracker", Some(""));
        let page = page_of(&html);
        assert!(page.contains("class=\"ui-empty\""), "{page}");
        assert!(page.contains("No changes yet."), "{page}");
        assert!(
            page.contains("New Tracker changes will show up here."),
            "{page}"
        );
        assert!(page.contains("class=\"cl-toggle\""), "toggle stays");

        let html = render("/changelog/site", Some("[[entry]\n"));
        let page = page_of(&html);
        assert!(page.contains("Site changes could not be loaded."), "{page}");
        assert!(page.contains(">Retry</button>"), "{page}");
        assert!(
            page.contains("href=\"/changelog/tracker\""),
            "the other tab stays usable"
        );
        assert_eq!(html.matches("<h1").count(), 1);
        assert!(
            page.contains("aria-live=\"polite\">Site changes could not be loaded. Retry.</p>"),
            "an error speaks even on a cold load: {page}"
        );
    }

    #[test]
    fn switch_tracker_only_speaks_after_a_tab_change() {
        let t = SwitchTracker::new(Tab::Site);
        assert!(!t.observe(Tab::Site));
        assert!(!t.observe(Tab::Site));
        assert!(t.observe(Tab::Tracker));
        assert!(t.observe(Tab::Site), "stays on after the first switch");
    }

    #[test]
    fn footer_and_drawer_link_to_the_site_changelog() {
        for path in ["/", "/changelog/tracker"] {
            let html = render(path, None);
            let footer_start = html.find("<footer").unwrap();
            let footer = &html[footer_start..];
            let account = &footer[footer.find("<h2>Account</h2>").unwrap()..];
            assert!(
                account.contains("href=\"/changelog/site\""),
                "{path}: {account}"
            );
            assert!(account.contains("What&#39;s changed") || account.contains("What's changed"));
        }
    }
}
