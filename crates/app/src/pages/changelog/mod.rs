//! Public "What's changed" page: `/changelog/site` and `/changelog/tracker`.
//!
//! The content is two static TOML files that `build.rs` checks and compiles
//! in (see [`data`]), so the page needs no server route and no database. The Site /
//! Tracker switch is two real links, so each tab is its own URL.

mod data;
mod model;
#[cfg(test)]
mod parse;

use std::cell::Cell;
use std::rc::Rc;

use dioxus::prelude::*;

use model::Entry;
pub use model::Tab;

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
    /* The selected tab is marked by more than its fill: a thick underline
       and a border, so it still reads in forced colours and without colour. */
    .cl-toggle a { border: var(--focus-ring-width) solid transparent; }
    .cl-toggle a[aria-current="page"],
    .cl-toggle a[aria-current="page"]:hover {
        background: var(--accent-solid);
        color: var(--on-accent);
        border-color: currentColor;
        text-decoration: underline;
        text-decoration-thickness: var(--focus-ring-width);
        text-underline-offset: 0.25em;
    }
    @media (forced-colors: active) {
        .cl-toggle a { border-color: Canvas; color: LinkText; }
        .cl-toggle a[aria-current="page"],
        .cl-toggle a[aria-current="page"]:hover {
            border-color: Highlight;
            color: CanvasText;
        }
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
    /* The polite status region. */
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
    /* Screen reader only text, such as the link suffix. Its own class, so a
       later change to the live region cannot un-hide it. */
    .cl-sr {
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

/// Page lead. Names the configured org, the same name the title uses. While
/// settings are pending or failed it stays generic rather than invent one.
/// A name that already starts with "The " gets no second article.
fn page_lead(org_name: Option<&str>) -> String {
    match org_name.map(str::trim).filter(|name| !name.is_empty()) {
        Some(name) if starts_with_the(name) => {
            format!("Updates to {name} site and the Overwatch tracker.")
        }
        Some(name) => format!("Updates to the {name} site and the Overwatch tracker."),
        None => "Updates to the site and the Overwatch tracker.".to_string(),
    }
}

/// True when the name opens with the word "The" (any case), e.g. "The
/// Scuffed Crew", "THE OWLS" or a bare "The", but not "Theta". Any
/// whitespace may follow the word.
fn starts_with_the(name: &str) -> bool {
    name.get(..3)
        .is_some_and(|head| head.eq_ignore_ascii_case("the"))
        && name[3..].chars().next().is_none_or(char::is_whitespace)
}

fn entry_heading_id(tab: Tab, index: usize) -> String {
    format!("cl-{}-{index}", tab.slug())
}

/// What the list remembers across renders of one page instance. The route
/// component re-renders this page with a new `tab` prop on a toggle
/// navigation, so the memory notices the switch itself.
struct ListMemory {
    last: Cell<Tab>,
    /// Entries shown on the current tab. 0 means the first batch.
    shown: Cell<usize>,
    /// The live region names the tab after a switch, until "Show older".
    announce: Cell<bool>,
}

impl ListMemory {
    fn new(tab: Tab) -> Self {
        Self {
            last: Cell::new(tab),
            shown: Cell::new(0),
            announce: Cell::new(false),
        }
    }

    /// Record this render's tab. A switch starts the list over at the first
    /// batch and turns the announcement on.
    fn observe(&self, tab: Tab) {
        if self.last.replace(tab) != tab {
            self.shown.set(0);
            self.announce.set(true);
        }
    }

    /// Entries to paint out of `total`.
    fn shown(&self, total: usize) -> usize {
        self.shown.get().max(data::first_batch(total)).min(total)
    }

    /// "Show older": remember the new count and go quiet, as on a fresh load.
    fn show_older(&self, next: usize) {
        self.shown.set(next);
        self.announce.set(false);
    }

    fn announce(&self) -> bool {
        self.announce.get()
    }
}

#[component]
fn ChangelogPage(tab: Tab) -> Element {
    let site_settings = crate::state::use_site_settings();
    let lead = page_lead(
        crate::state::loaded_site_settings(site_settings.resolved.read().as_ref())
            .map(|settings| settings.org_name.as_str()),
    );
    let memory = use_hook(|| Rc::new(ListMemory::new(tab)));
    memory.observe(tab);
    // Bumped by "Show older" so the page re-renders; the count is in `memory`.
    let mut redraw = use_signal(|| 0u32);
    let _ = redraw();
    let mut pending_focus = use_signal(|| None::<String>);

    use_effect(move || {
        if let Some(id) = pending_focus() {
            focus_element(&id);
            pending_focus.set(None);
        }
    });

    let entries = data::load(tab);
    let total = entries.len();
    let count = memory.shown(total);
    let live = data::live_text(tab, memory.announce(), data::first_batch(total));
    let list = if total == 0 {
        rsx! {
            div { class: "ui-empty",
                p { class: "cl-empty-title", "No changes yet." }
                p { "{data::empty_text(tab)}" }
            }
        }
    } else {
        let memory = memory.clone();
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
                            memory.show_older(data::next_batch(count, total));
                            pending_focus.set(Some(entry_heading_id(tab, count)));
                            redraw += 1;
                        },
                        "Show older"
                    }
                }
            }
        }
    };

    rsx! {
        crate::styles::StyleOnce { id: "scuffed-css-pages-changelog", css: CHANGELOG_CSS }
        div { class: "changelog-page",
            header { class: "page-header",
                h1 { "What's changed" }
                p { class: "page-lead", "{lead}" }
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
                    span { class: "cl-sr", " for {link_context}" }
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

    /// Render `path` with the shipped files and `org_name` as the embedded
    /// site settings (None = no settings embedded).
    fn render_with_org(path: &str, org_name: Option<&str>) -> String {
        crate::state::set_test_embedded_json(org_name.map(|name| {
            serde_json::json!({
                "id": "site",
                "org_name": name,
                "site_description": "desc",
                "recruitment_open": true,
                "recruitment_message": "msg",
                "min_age": 16,
                "forum_backend": "local",
                "extra_relay_urls": "",
                "updated_at": "2026-10-06T00:00:00Z",
            })
            .to_string()
        }));
        let html = render(path, None);
        crate::state::set_test_embedded_json(None);
        html
    }

    fn page_of(html: &str) -> &str {
        let start = html
            .find("class=\"changelog-page\"")
            .expect("changelog page");
        &html[start..]
    }

    /// The page without the footer that follows it.
    fn page_only(html: &str) -> &str {
        let page = page_of(html);
        &page[..page.find("<footer").unwrap_or(page.len())]
    }

    fn toggle_nav(page: &str) -> &str {
        let start = page.find("<nav class=\"cl-toggle\"").expect("toggle nav");
        let end = start + page[start..].find("</nav>").unwrap();
        &page[start..end]
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
        // Scoped to the page, so the footer's "What's changed" link cannot
        // satisfy either check.
        let page = page_only(&html);
        assert!(
            page.contains("<h1>What&#39;s changed</h1>")
                || page.contains("<h1>What's changed</h1>"),
            "{page}"
        );
        let nav = toggle_nav(page);
        assert_eq!(nav.matches("aria-current=\"page\"").count(), 1, "{nav}");
        assert!(page.contains("aria-label=\"Site changes\""), "{page}");
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
            assert!(html.contains("d1-top"), "{path}: keeps the site chrome");
            // Scoped to the not-found card: the footer also links Site.
            let card = &html[html.find("nf-page").expect("not-found card")..];
            let card = &card[..card.find("<footer").unwrap_or(card.len())];
            assert!(card.contains("href=\"/changelog/site\""), "{path}: {card}");
            assert!(
                card.contains("href=\"/changelog/tracker\""),
                "{path}: {card}"
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
    fn empty_state_matches_the_copy_and_there_is_no_error_state() {
        let html = render("/changelog/tracker", Some(""));
        let page = page_of(&html);
        assert!(page.contains("class=\"ui-empty\""), "{page}");
        assert!(page.contains("No changes yet."), "{page}");
        assert!(
            page.contains("New Tracker changes will show up here."),
            "{page}"
        );
        assert!(page.contains("class=\"cl-toggle\""), "toggle stays");
        // build.rs rejects a broken file, so the page has no error or Retry.
        for path in ["/changelog/site", "/changelog/tracker"] {
            let html = render(path, None);
            assert!(!html.contains("Retry"), "{path}");
            assert!(!html.contains("could not be loaded"), "{path}");
        }
    }

    #[test]
    fn a_tab_switch_starts_the_list_over_and_show_older_goes_quiet() {
        let m = ListMemory::new(Tab::Site);
        m.observe(Tab::Site);
        assert!(!m.announce(), "a cold load is quiet");
        assert_eq!(m.shown(23), 10);
        m.show_older(20);
        m.observe(Tab::Site);
        assert_eq!(m.shown(23), 20, "a re-render keeps Show older");

        m.observe(Tab::Tracker);
        assert!(m.announce(), "a switch speaks");
        assert_eq!(m.shown(23), 10, "a switch starts over");
        m.show_older(20);
        assert!(
            !m.announce(),
            "Show older after a switch is quiet, as on a fresh load"
        );

        // Site, Show older, Tracker, back to Site: Site starts over too.
        m.observe(Tab::Site);
        assert_eq!(m.shown(23), 10);
        assert!(m.announce());
        assert_eq!(m.shown(4), 4);
        assert_eq!(m.shown(0), 0);
    }

    #[test]
    fn the_link_suffix_uses_its_own_screen_reader_class() {
        let src = "[[entry]]\nversion = \"v1.0.0\"\ndate = \"2026-11-02\"\nchanges = [\"x\"]\nlink = \"https://example.com/notes\"\n";
        let html = render("/changelog/tracker", Some(src));
        let page = page_only(&html);
        assert!(
            page.contains("<span class=\"cl-sr\"> for v1.0.0</span>"),
            "{page}"
        );
        assert_eq!(page.matches("class=\"cl-live\"").count(), 1, "{page}");
        let sr = CHANGELOG_CSS
            .split_once(".cl-sr {")
            .map(|(_, rest)| rest.split_once('}').unwrap().0)
            .expect("cl-sr rule");
        assert!(sr.contains("clip: rect(0, 0, 0, 0);"), "{sr}");
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

            // The phone drawer has its own link in the Account group.
            let drawer_start = html.find("id=\"site-drawer\"").expect("drawer");
            let drawer = &html[drawer_start..];
            let drawer = &drawer[..drawer.find("<main").expect("main after drawer")];
            let account = &drawer[drawer.find("<h2>Account</h2>").expect("drawer Account")..];
            assert!(
                account.contains("href=\"/changelog/site\""),
                "{path}: {account}"
            );
            assert!(account.contains("What&#39;s changed") || account.contains("What's changed"));
        }
    }

    #[test]
    fn page_lead_names_the_configured_org_or_stays_generic() {
        assert_eq!(
            page_lead(Some("Night Owls")),
            "Updates to the Night Owls site and the Overwatch tracker."
        );
        assert_eq!(
            page_lead(Some("  Night Owls ")),
            "Updates to the Night Owls site and the Overwatch tracker."
        );
        // A name that already opens with "The" gets no second article.
        for name in [
            "The Scuffed Crew",
            "the Scuffed Crew",
            "THE OWLS",
            " The Owls ",
            "The\tOwls",
            "The",
            "the",
            "  THE  ",
        ] {
            let lead = page_lead(Some(name));
            assert_eq!(
                lead,
                format!("Updates to {} site and the Overwatch tracker.", name.trim()),
                "{name}"
            );
            assert!(!lead.to_lowercase().contains("the the"), "{lead}");
        }
        // Only the whole word counts: "Theta" keeps the article.
        assert_eq!(
            page_lead(Some("Theta Squad")),
            "Updates to the Theta Squad site and the Overwatch tracker."
        );
        assert_eq!(
            page_lead(Some("Theta")),
            "Updates to the Theta site and the Overwatch tracker."
        );
        for none in [None, Some(""), Some("   ")] {
            assert_eq!(
                page_lead(none),
                "Updates to the site and the Overwatch tracker."
            );
        }
    }

    #[test]
    fn page_lead_uses_the_org_name_from_site_settings() {
        for path in ["/changelog/site", "/changelog/tracker"] {
            let html = render_with_org(path, Some("Night Owls"));
            let page = page_of(&html);
            assert!(
                page.contains(
                    "<p class=\"page-lead\">Updates to the Night Owls site and the Overwatch tracker.</p>"
                ),
                "{path}: {page}"
            );
            assert!(!page.contains("Scuffed Crew"), "{path}: {page}");

            let html = render_with_org(path, Some("The Scuffed Crew"));
            let page = page_of(&html);
            assert!(
                page.contains(
                    "<p class=\"page-lead\">Updates to The Scuffed Crew site and the Overwatch tracker.</p>"
                ),
                "{path}: {page}"
            );
            assert!(!page.contains("the The"), "{path}: {page}");

            let html = render_with_org(path, None);
            let page = page_of(&html);
            assert!(
                page.contains(
                    "<p class=\"page-lead\">Updates to the site and the Overwatch tracker.</p>"
                ),
                "{path}: {page}"
            );
            assert!(!page.contains("Scuffed Crew"), "{path}: {page}");
        }
    }

    /// The selected tab must not rely on its fill alone: it carries an
    /// underline and a border, and forced colours give it a Highlight border.
    #[test]
    fn selected_tab_has_a_non_colour_cue_and_a_forced_colours_rule() {
        let css = CHANGELOG_CSS;
        let selected = css
            .split_once(".cl-toggle a[aria-current=\"page\"]:hover {")
            .map(|(_, rest)| rest.split_once('}').unwrap().0)
            .expect("selected tab rule");
        assert!(
            selected.contains("text-decoration: underline;"),
            "{selected}"
        );
        assert!(
            selected.contains("text-decoration-thickness:"),
            "{selected}"
        );
        assert!(
            selected.contains("border-color: currentColor;"),
            "{selected}"
        );
        // Every tab carries the same border width, so nothing shifts.
        assert!(
            css.contains(".cl-toggle a { border: var(--focus-ring-width) solid transparent; }"),
            "{css}"
        );

        let (_, forced) = css
            .split_once("@media (forced-colors: active) {")
            .expect("forced colours block");
        let forced_selected = forced
            .split_once(".cl-toggle a[aria-current=\"page\"]:hover {")
            .map(|(_, rest)| rest.split_once('}').unwrap().0)
            .expect("forced selected rule");
        assert!(
            forced_selected.contains("border-color: Highlight;"),
            "{forced_selected}"
        );
        assert!(
            forced_selected.contains("color: CanvasText;"),
            "{forced_selected}"
        );
        // The underline is outside the media block, so it applies there too.
        assert!(
            !forced_selected.contains("text-decoration: none"),
            "{forced_selected}"
        );
    }
}
