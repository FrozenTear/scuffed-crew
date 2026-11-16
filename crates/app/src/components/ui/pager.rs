//! Pager for long lists: range text, Previous / Next and page numbers.
//!
//! [`Pager`] is controlled: the page holds the current page number and gets
//! the new one through `on_change`. The helpers are pure, so a page can slice
//! its list ([`page_slice`]) with the same clamping the pager shows.
//!
//! Page 20 of a long list is the same control: the window always keeps the
//! first page, the last page and the neighbours of the current one, and puts
//! an ellipsis where it skips two or more pages.

use dioxus::prelude::*;

/// One position in the row of page numbers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageSlot {
    Page(usize),
    /// Two or more pages are skipped here.
    Gap,
}

/// Most slots the number row holds. With more pages than this, gaps appear.
const WINDOW_SLOTS: usize = 7;

/// Number of pages for `total` items. Never zero: an empty list is one
/// empty page. A page size of zero counts as one.
pub fn page_count(total: usize, page_size: usize) -> usize {
    total.div_ceil(page_size.max(1)).max(1)
}

/// `page` moved into `1..=pages`.
pub fn clamp_page(page: usize, pages: usize) -> usize {
    page.clamp(1, pages.max(1))
}

/// First and last item on `page`, counted from 1. `None` for an empty list.
/// A page past the end shows the last page.
pub fn page_range(page: usize, total: usize, page_size: usize) -> Option<(usize, usize)> {
    if total == 0 {
        return None;
    }
    let size = page_size.max(1);
    let page = clamp_page(page, page_count(total, size));
    let first = (page - 1) * size + 1;
    Some((first, (first + size - 1).min(total)))
}

/// The items on `page` (counted from 1), with the pager's clamping.
pub fn page_slice<T>(items: &[T], page: usize, page_size: usize) -> &[T] {
    match page_range(page, items.len(), page_size) {
        Some((first, last)) => &items[first - 1..last],
        None => &[],
    }
}

/// Which page numbers to show. Up to seven pages are all listed. Past that
/// the row is always seven slots: the first page, the last page, the current
/// page with its neighbours, and a gap wherever two or more pages are skipped.
pub fn page_window(current: usize, pages: usize) -> Vec<PageSlot> {
    let pages = pages.max(1);
    let current = clamp_page(current, pages);
    if pages <= WINDOW_SLOTS {
        return (1..=pages).map(PageSlot::Page).collect();
    }
    // Five numbers in a row next to an end, three in the middle.
    let run = WINDOW_SLOTS - 2;
    let mut slots = Vec::with_capacity(WINDOW_SLOTS);
    if current < run {
        slots.extend((1..=run).map(PageSlot::Page));
        slots.push(PageSlot::Gap);
        slots.push(PageSlot::Page(pages));
    } else if current > pages - run + 1 {
        slots.push(PageSlot::Page(1));
        slots.push(PageSlot::Gap);
        slots.extend((pages - run + 1..=pages).map(PageSlot::Page));
    } else {
        slots.push(PageSlot::Page(1));
        slots.push(PageSlot::Gap);
        slots.extend((current - 1..=current + 1).map(PageSlot::Page));
        slots.push(PageSlot::Gap);
        slots.push(PageSlot::Page(pages));
    }
    slots
}

/// "1 to 8 of 24 members". Empty for an empty list. One item on the page
/// reads "25 of 25 members".
pub fn range_text(page: usize, total: usize, page_size: usize, noun: &str) -> String {
    match page_range(page, total, page_size) {
        None => String::new(),
        Some((first, last)) if first == last => format!("{first} of {total} {noun}"),
        Some((first, last)) => format!("{first} to {last} of {total} {noun}"),
    }
}

/// After a page change: bring the list (the element with `dom_id`) to the top
/// of the view and move keyboard focus to it, so the next Tab reaches the new
/// rows and focus is not left on a pager button that scrolled away. The target
/// needs `tabindex="-1"` and a `scroll-margin-top` that clears the sticky top
/// bar ([`PAGER_CSS`] gives it to `.pager-target`). Does nothing outside
/// the browser.
pub fn show_list_top(dom_id: &str) {
    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::JsCast;
        let Some(el) = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.get_element_by_id(dom_id))
        else {
            return;
        };
        el.scroll_into_view();
        // `focus({ preventScroll: true })`: the scroll above already honours
        // the scroll margin; a second scroll from focus would not.
        let options = js_sys::Object::new();
        let _ = js_sys::Reflect::set(&options, &"preventScroll".into(), &true.into());
        let focus = js_sys::Reflect::get(&el, &"focus".into()).ok();
        if let Some(focus) = focus.as_ref().and_then(|f| f.dyn_ref::<js_sys::Function>()) {
            let _ = focus.call1(&el, &options);
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = dom_id;
    }
}

/// The page number a route string carries as `?page=N` ("/blog?page=3" is
/// 3). 1 when there is none or it is not a positive number.
pub fn page_in_route(route: &str) -> usize {
    let without_hash = route.split('#').next().unwrap_or(route);
    without_hash
        .split_once('?')
        .map(|(_, query)| query)
        .unwrap_or("")
        .split('&')
        .filter_map(|pair| pair.strip_prefix("page="))
        .filter_map(|value| value.parse::<usize>().ok())
        .find(|page| *page >= 1)
        .unwrap_or(1)
}

/// `route` pointing at `page`: other query pairs are kept, page 1 has no
/// `page` pair (so the first page keeps its plain address) and a fragment is
/// dropped.
pub fn route_for_page(route: &str, page: usize) -> String {
    let without_hash = route.split('#').next().unwrap_or(route);
    let (path, query) = without_hash.split_once('?').unwrap_or((without_hash, ""));
    let mut pairs: Vec<String> = query
        .split('&')
        .filter(|pair| !pair.is_empty() && *pair != "page" && !pair.starts_with("page="))
        .map(str::to_string)
        .collect();
    if page > 1 {
        pairs.push(format!("page={page}"));
    }
    if pairs.is_empty() {
        path.to_string()
    } else {
        format!("{path}?{}", pairs.join("&"))
    }
}

thread_local! {
    /// Path and query the browser was opened on. The router rewrites the
    /// address without undeclared query pairs when it mounts, so `main`
    /// snapshots it first and the first paged list to mount takes it.
    static LANDING_URL: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

/// Snapshot the address the app was opened on. Call once from `main`, before
/// the router mounts. Does nothing outside the browser.
pub fn remember_landing_url() {
    #[cfg(target_arch = "wasm32")]
    {
        let Some(location) = web_sys::window().map(|w| w.location()) else {
            return;
        };
        let url = location.pathname().unwrap_or_default() + &location.search().unwrap_or_default();
        LANDING_URL.with(|slot| *slot.borrow_mut() = Some(url));
    }
}

/// The page to restore on the first mount of a paged list: the landing
/// address's page when it was opened on this same path past page 1.
fn landing_page(landing: Option<&str>, current: &str) -> Option<usize> {
    let landing = landing?;
    let path = |route: &str| {
        route
            .split(['?', '#'])
            .next()
            .unwrap_or("")
            .trim_end_matches('/')
            .to_string()
    };
    let page = page_in_route(landing);
    (path(landing) == path(current) && page > 1 && page_in_route(current) == 1).then_some(page)
}

/// The page of a list, kept in the address as `?page=N`, so a page can be
/// linked and reloaded and Back from an entry returns to the page it was
/// opened from. Needs a router above it.
#[derive(Clone, Copy, PartialEq)]
pub struct UrlPage {
    page: Memo<usize>,
}

impl UrlPage {
    /// Current page, counted from 1. Reading it subscribes like a signal.
    pub fn get(&self) -> usize {
        (self.page)()
    }

    /// Go to `page` as a new history entry.
    pub fn go(&self, page: usize) {
        let router = router();
        let target = route_for_page(&router.full_route_string(), page);
        router.push(target);
    }
}

/// See [`UrlPage`]. The page follows the address, so Back, Forward and a nav
/// link to the list all move it.
pub fn use_url_page() -> UrlPage {
    let page = use_memo(|| page_in_route(&router().full_route_string()));
    use_effect(|| {
        let Some(landing) = LANDING_URL.with(|slot| slot.borrow_mut().take()) else {
            return;
        };
        let router = router();
        let current = router.full_route_string();
        if let Some(page) = landing_page(Some(&landing), &current) {
            router.replace(route_for_page(&current, page));
        }
    });
    UrlPage { page }
}

pub const PAGER_CSS: &str = r#"
/* The list a pager pages through. It takes focus after a page change and
   lands below the sticky top bar. */
.pager-target { scroll-margin-top: calc(var(--nav-height-used) + var(--space-section)); }
.pager-target:focus-visible {
  outline: var(--focus-ring-width) solid var(--focus-ring);
  outline-offset: var(--focus-ring-offset);
}
.pager {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-3);
  padding-top: var(--space-2);
  min-width: 0;
}
/* Nothing to show for an empty list, but the status line stays in the
   tree so its next text is announced. */
.pager.is-empty {
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
.pager-meta {
  margin: 0;
  color: var(--text-3);
  font-family: var(--font-body);
  font-size: var(--text-sm);
  line-height: var(--leading-body);
}
.pager-pages {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-2);
  min-width: 0;
}
.pager-pages button {
  box-sizing: border-box;
  min-width: var(--hit-target-min);
  min-height: var(--hit-target-min);
  height: var(--hit-target-min);
  margin: 0;
  padding: 0 var(--space-3);
  border: 1px solid var(--border-strong);
  border-radius: var(--radius-control);
  background: var(--surface);
  color: var(--text);
  font-family: var(--font-head);
  font-size: var(--text-sm);
  font-weight: var(--font-label-weight);
  line-height: var(--leading-ui);
  display: inline-flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  text-box-trim: trim-both;
  text-box-edge: cap alphabetic;
}
.pager-pages button:hover { border-color: var(--text); }
/* Current page: soft fill and accent ink, plus a doubled edge and a heavier
   weight so it is not colour alone. */
.pager-pages .page-btn.is-current {
  background: var(--accent-soft);
  color: var(--accent-text);
  border-color: var(--accent-text);
  box-shadow: inset 0 0 0 1px var(--accent-text);
  font-weight: var(--chip-selected-weight);
  cursor: default;
}
.pager-pages .is-disabled,
.pager-pages .is-disabled:hover {
  background: var(--surface);
  color: var(--text-disabled);
  border-color: var(--border);
  cursor: not-allowed;
}
.pager-gap {
  min-width: var(--space-6);
  color: var(--text-subtle);
  font-family: var(--font-head);
  font-size: var(--text-sm);
  text-align: center;
  user-select: none;
}
@supports not (text-box-trim: trim-both) {
  .pager-pages button { padding-top: 0.08em; }
}
"#;

/// A pager under a list.
///
/// Always render it, also while the list is empty or fits one page: the range
/// line is a live region (`role="status"`), so it has to exist before its text
/// changes. The number row only shows when there is more than one page.
///
/// The ends are `aria-disabled` buttons, not `disabled` ones, so keyboard
/// focus stays on Next when the last page is reached.
#[component]
pub fn Pager(
    /// Id stem, unique on the page ("members"). Element ids derive from it.
    id: String,
    /// Current page, counted from 1. Out of range values are clamped.
    page: usize,
    /// Number of items in the whole list.
    total: usize,
    page_size: usize,
    /// Plural noun after the count ("members").
    noun: String,
    /// Noun for a list of exactly one ("member").
    #[props(default)]
    noun_singular: Option<String>,
    /// Also say "Page 1 of 5" after the range.
    #[props(default = false)]
    show_page_count: bool,
    /// Accessible name of the nav landmark.
    #[props(default = "Pagination".to_string())]
    label: String,
    /// Called with the new page (counted from 1) when it changes.
    on_change: EventHandler<usize>,
) -> Element {
    let pages = page_count(total, page_size);
    let current = clamp_page(page, pages);
    let mut meta = match (total, noun_singular) {
        (1, Some(one)) => format!("1 {one}"),
        // One page and no number row: a plain count. "1 to 4 of 4" under a
        // list with no buttons reads like a pager that lost its controls.
        _ if pages == 1 && total > 1 => format!("{total} {noun}"),
        _ => range_text(current, total, page_size, &noun),
    };
    if show_page_count && pages > 1 {
        meta.push_str(&format!(" · Page {current} of {pages}"));
    }
    let at_start = current == 1;
    let at_end = current == pages;
    let step_class = |off: bool| {
        if off {
            "pager-step is-disabled"
        } else {
            "pager-step"
        }
    };
    let window = page_window(current, pages);
    rsx! {
        div { id: "{id}", class: if total == 0 { "pager is-empty" } else { "pager" },
            p { id: "{id}-meta", class: "pager-meta", role: "status", "{meta}" }
            if pages > 1 {
                nav { class: "pager-pages", aria_label: "{label}",
                    button {
                        r#type: "button",
                        id: "{id}-prev",
                        class: step_class(at_start),
                        aria_label: "Previous page",
                        aria_disabled: if at_start { "true" },
                        onclick: move |_| {
                            if !at_start {
                                on_change.call(current - 1);
                            }
                        },
                        "Previous"
                    }
                    {window.iter().enumerate().map(|(index, slot)| match *slot {
                        PageSlot::Page(n) => rsx! {
                            button {
                                key: "page-{n}",
                                r#type: "button",
                                id: "{id}-page-{n}",
                                class: if n == current { "page-btn is-current" } else { "page-btn" },
                                aria_label: "Page {n}",
                                aria_current: if n == current { "page" },
                                onclick: move |_| {
                                    if n != current {
                                        on_change.call(n);
                                    }
                                },
                                "{n}"
                            }
                        },
                        PageSlot::Gap => rsx! {
                            span {
                                key: "gap-{index}",
                                class: "pager-gap",
                                aria_hidden: "true",
                                "…"
                            }
                        },
                    })}
                    button {
                        r#type: "button",
                        id: "{id}-next",
                        class: step_class(at_end),
                        aria_label: "Next page",
                        aria_disabled: if at_end { "true" },
                        onclick: move |_| {
                            if !at_end {
                                on_change.call(current + 1);
                            }
                        },
                        "Next"
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::PageSlot::{Gap, Page};
    use super::*;
    use crate::components::ui::test_support::{
        assert_ink_clears_aa, assert_sizes_from_tokens, css_rule, has_decl, render,
    };

    #[test]
    fn page_number_is_read_from_and_written_to_the_route_query() {
        assert_eq!(page_in_route("/blog"), 1);
        assert_eq!(page_in_route("/blog?page=3"), 3);
        assert_eq!(page_in_route("/blog?q=a&page=2#top"), 2);
        for bad in [
            "/blog?page=0",
            "/blog?page=x",
            "/blog?page=",
            "/blog?pages=4",
            "/blog#page=4",
        ] {
            assert_eq!(page_in_route(bad), 1, "{bad}");
        }
        assert_eq!(route_for_page("/blog", 2), "/blog?page=2");
        assert_eq!(route_for_page("/blog?page=2", 3), "/blog?page=3");
        // Page 1 keeps the plain address.
        assert_eq!(route_for_page("/blog?page=2", 1), "/blog");
        assert_eq!(route_for_page("/news?q=a&page=5#x", 1), "/news?q=a");
        assert_eq!(route_for_page("/news?q=a", 4), "/news?q=a&page=4");
    }

    #[test]
    fn landing_page_is_restored_only_on_the_path_it_was_opened_on() {
        assert_eq!(landing_page(Some("/blog?page=2"), "/blog"), Some(2));
        assert_eq!(landing_page(Some("/blog/?page=2"), "/blog"), Some(2));
        assert_eq!(landing_page(Some("/blog?page=2"), "/news"), None);
        assert_eq!(landing_page(Some("/blog"), "/blog"), None);
        assert_eq!(landing_page(Some("/blog?page=2"), "/blog?page=3"), None);
        assert_eq!(landing_page(None, "/blog"), None);
    }

    #[test]
    fn page_count_rounds_up_and_is_never_zero() {
        assert_eq!(page_count(0, 8), 1);
        assert_eq!(page_count(1, 8), 1);
        assert_eq!(page_count(8, 8), 1);
        assert_eq!(page_count(9, 8), 2);
        assert_eq!(page_count(24, 8), 3);
        assert_eq!(page_count(40, 8), 5);
        assert_eq!(page_count(5, 0), 5, "a zero page size counts as one");
    }

    #[test]
    fn clamp_keeps_the_page_in_range() {
        assert_eq!(clamp_page(0, 3), 1);
        assert_eq!(clamp_page(2, 3), 2);
        assert_eq!(clamp_page(9, 3), 3);
        assert_eq!(clamp_page(4, 0), 1);
    }

    #[test]
    fn range_and_slice_agree() {
        let items: Vec<usize> = (1..=24).collect();
        assert_eq!(page_range(1, 24, 8), Some((1, 8)));
        assert_eq!(page_slice(&items, 1, 8), &items[0..8]);
        assert_eq!(page_range(3, 24, 8), Some((17, 24)));
        assert_eq!(page_slice(&items, 3, 8), &items[16..24]);

        let odd: Vec<usize> = (1..=25).collect();
        assert_eq!(page_range(4, 25, 8), Some((25, 25)));
        assert_eq!(page_slice(&odd, 4, 8), &[25]);
        // Past the end shows the last page, page 0 shows the first.
        assert_eq!(page_slice(&odd, 99, 8), &[25]);
        assert_eq!(page_slice(&odd, 0, 8), &odd[0..8]);

        let none: Vec<usize> = Vec::new();
        assert_eq!(page_range(1, 0, 8), None);
        assert!(page_slice(&none, 1, 8).is_empty());
    }

    #[test]
    fn range_text_matches_the_frames() {
        assert_eq!(range_text(1, 24, 8, "members"), "1 to 8 of 24 members");
        assert_eq!(
            range_text(1, 40, 8, "ranked players"),
            "1 to 8 of 40 ranked players"
        );
        assert_eq!(range_text(3, 24, 8, "members"), "17 to 24 of 24 members");
        assert_eq!(range_text(4, 25, 8, "members"), "25 of 25 members");
        assert_eq!(range_text(1, 0, 8, "members"), "");
    }

    #[test]
    fn short_lists_show_every_page() {
        assert_eq!(page_window(1, 1), vec![Page(1)]);
        assert_eq!(page_window(1, 3), vec![Page(1), Page(2), Page(3)]);
        assert_eq!(
            page_window(4, 7),
            (1..=7).map(Page).collect::<Vec<_>>(),
            "seven pages fit without a gap"
        );
        assert_eq!(page_window(1, 0), vec![Page(1)]);
    }

    #[test]
    fn long_lists_keep_the_ends_and_the_neighbours() {
        assert_eq!(
            page_window(1, 20),
            vec![Page(1), Page(2), Page(3), Page(4), Page(5), Gap, Page(20)]
        );
        assert_eq!(
            page_window(4, 20),
            vec![Page(1), Page(2), Page(3), Page(4), Page(5), Gap, Page(20)]
        );
        assert_eq!(
            page_window(5, 20),
            vec![Page(1), Gap, Page(4), Page(5), Page(6), Gap, Page(20)]
        );
        assert_eq!(
            page_window(10, 20),
            vec![Page(1), Gap, Page(9), Page(10), Page(11), Gap, Page(20)]
        );
        assert_eq!(
            page_window(16, 20),
            vec![Page(1), Gap, Page(15), Page(16), Page(17), Gap, Page(20)]
        );
        assert_eq!(
            page_window(17, 20),
            vec![
                Page(1),
                Gap,
                Page(16),
                Page(17),
                Page(18),
                Page(19),
                Page(20)
            ]
        );
        assert_eq!(
            page_window(20, 20),
            vec![
                Page(1),
                Gap,
                Page(16),
                Page(17),
                Page(18),
                Page(19),
                Page(20)
            ]
        );
        assert_eq!(
            page_window(4, 8),
            vec![Page(1), Page(2), Page(3), Page(4), Page(5), Gap, Page(8)]
        );
        assert_eq!(
            page_window(5, 8),
            vec![Page(1), Gap, Page(4), Page(5), Page(6), Page(7), Page(8)]
        );
        // Out of range is clamped, not a panic.
        assert_eq!(page_window(99, 20), page_window(20, 20));
        assert_eq!(page_window(0, 20), page_window(1, 20));
    }

    /// For every page of every list length: first, last, current and its
    /// neighbours are shown, numbers rise, and a gap hides at least two pages.
    #[test]
    fn window_invariants_hold_for_every_page() {
        for pages in 1..=60 {
            for current in 1..=pages {
                let window = page_window(current, pages);
                let ctx = format!("page {current} of {pages}: {window:?}");
                assert!(window.len() <= WINDOW_SLOTS, "{ctx}");
                if pages > WINDOW_SLOTS {
                    assert_eq!(window.len(), WINDOW_SLOTS, "{ctx}");
                }
                assert_eq!(window.first(), Some(&Page(1)), "{ctx}");
                assert_eq!(window.last(), Some(&Page(pages)), "{ctx}");
                for want in [
                    current.saturating_sub(1).max(1),
                    current,
                    (current + 1).min(pages),
                ] {
                    assert!(window.contains(&Page(want)), "missing {want}, {ctx}");
                }
                let mut prev = 0;
                let mut after_gap = false;
                for slot in &window {
                    match *slot {
                        Page(n) => {
                            if after_gap {
                                assert!(n >= prev + 3, "a gap hides two or more, {ctx}");
                            } else {
                                assert_eq!(n, prev + 1, "{ctx}");
                            }
                            prev = n;
                            after_gap = false;
                        }
                        Gap => {
                            assert!(!after_gap, "{ctx}");
                            after_gap = true;
                        }
                    }
                }
                assert!(!after_gap, "{ctx}");
            }
        }
    }

    #[test]
    fn first_page_matches_the_members_frame() {
        fn app() -> Element {
            rsx! {
                Pager {
                    id: "members-pager",
                    page: 1,
                    total: 24,
                    page_size: 8,
                    noun: "members",
                    on_change: |_| {},
                }
            }
        }
        let html = render(app);
        assert!(
            html.contains(
                "<p id=\"members-pager-meta\" class=\"pager-meta\" role=\"status\">1 to 8 of 24 members</p>"
            ),
            "{html}"
        );
        assert!(
            html.contains("<nav class=\"pager-pages\" aria-label=\"Pagination\">"),
            "{html}"
        );
        assert_eq!(html.matches("<nav").count(), 1, "{html}");
        assert_eq!(html.matches("<button").count(), 5, "{html}");
        assert_eq!(
            html.matches("type=\"button\"").count(),
            5,
            "no submit buttons: {html}"
        );
        assert!(!html.contains("<a"), "no links, so no dead links: {html}");
        assert!(!html.contains("href"), "{html}");
        assert_eq!(html.matches("aria-current=\"page\"").count(), 1, "{html}");
        assert!(
            html.contains(
                "<button type=\"button\" id=\"members-pager-page-1\" class=\"page-btn is-current\" aria-label=\"Page 1\" aria-current=\"page\">1</button>"
            ),
            "{html}"
        );
        assert!(
            html.contains(
                "<button type=\"button\" id=\"members-pager-page-2\" class=\"page-btn\" aria-label=\"Page 2\">2</button>"
            ),
            "{html}"
        );
        // Previous is off on page 1, Next is live.
        assert!(
            html.contains(
                "<button type=\"button\" id=\"members-pager-prev\" class=\"pager-step is-disabled\" aria-label=\"Previous page\" aria-disabled=\"true\">Previous</button>"
            ),
            "{html}"
        );
        assert!(
            html.contains(
                "<button type=\"button\" id=\"members-pager-next\" class=\"pager-step\" aria-label=\"Next page\">Next</button>"
            ),
            "{html}"
        );
        assert_eq!(html.matches("aria-disabled").count(), 1, "{html}");
        assert!(!html.contains("pager-gap"), "{html}");
    }

    #[test]
    fn last_page_turns_next_off() {
        fn app() -> Element {
            rsx! {
                Pager {
                    id: "lb",
                    page: 5,
                    total: 40,
                    page_size: 8,
                    noun: "ranked players",
                    show_page_count: true,
                    on_change: |_| {},
                }
            }
        }
        let html = render(app);
        assert!(
            html.contains(">33 to 40 of 40 ranked players · Page 5 of 5</p>"),
            "{html}"
        );
        assert!(
            html.contains("id=\"lb-next\" class=\"pager-step is-disabled\" aria-label=\"Next page\" aria-disabled=\"true\""),
            "{html}"
        );
        assert!(
            html.contains("id=\"lb-prev\" class=\"pager-step\" aria-label=\"Previous page\">"),
            "{html}"
        );
        assert!(
            html.contains("id=\"lb-page-5\" class=\"page-btn is-current\""),
            "{html}"
        );
    }

    #[test]
    fn page_twenty_uses_the_same_control_with_gaps() {
        fn app() -> Element {
            rsx! {
                Pager {
                    id: "roster",
                    page: 20,
                    total: 400,
                    page_size: 8,
                    noun: "members",
                    on_change: |_| {},
                }
            }
        }
        let html = render(app);
        assert!(html.contains(">153 to 160 of 400 members</p>"), "{html}");
        assert_eq!(
            html.matches("<span class=\"pager-gap\" aria-hidden=\"true\">…</span>")
                .count(),
            2,
            "{html}"
        );
        for n in [1, 19, 20, 21, 50] {
            assert!(
                html.contains(&format!("id=\"roster-page-{n}\"")),
                "{n}: {html}"
            );
        }
        assert!(!html.contains("id=\"roster-page-2\""), "{html}");
        assert_eq!(html.matches("class=\"page-btn").count(), 5, "{html}");
        assert_eq!(html.matches("aria-current=\"page\"").count(), 1, "{html}");
        assert!(!html.contains("aria-disabled"), "{html}");
    }

    #[test]
    fn out_of_range_page_shows_the_last_page() {
        fn app() -> Element {
            rsx! {
                Pager {
                    id: "m",
                    page: 9,
                    total: 24,
                    page_size: 8,
                    noun: "members",
                    on_change: |_| {},
                }
            }
        }
        let html = render(app);
        assert!(html.contains(">17 to 24 of 24 members</p>"), "{html}");
        assert!(
            html.contains("id=\"m-page-3\" class=\"page-btn is-current\""),
            "{html}"
        );
    }

    /// The status line is rendered for one page and for no items, so a
    /// later change is announced. The nav only exists with two pages or more.
    #[test]
    fn status_line_is_always_rendered() {
        fn one_page() -> Element {
            rsx! {
                Pager {
                    id: "m",
                    page: 1,
                    total: 5,
                    page_size: 8,
                    noun: "members",
                    on_change: |_| {},
                }
            }
        }
        let html = render(one_page);
        assert_eq!(
            html,
            "<div id=\"m\" class=\"pager\"><p id=\"m-meta\" class=\"pager-meta\" role=\"status\">5 members</p></div>"
        );

        fn empty() -> Element {
            rsx! {
                Pager {
                    id: "m",
                    page: 1,
                    total: 0,
                    page_size: 8,
                    noun: "members",
                    on_change: |_| {},
                }
            }
        }
        let html = render(empty);
        assert_eq!(
            html,
            "<div id=\"m\" class=\"pager is-empty\"><p id=\"m-meta\" class=\"pager-meta\" role=\"status\"></p></div>"
        );

        fn single() -> Element {
            rsx! {
                Pager {
                    id: "m",
                    page: 1,
                    total: 1,
                    page_size: 8,
                    noun: "members",
                    noun_singular: "member",
                    on_change: |_| {},
                }
            }
        }
        let html = render(single);
        assert!(html.contains("role=\"status\">1 member</p>"), "{html}");
    }

    #[test]
    fn paged_list_clears_the_sticky_top_bar_and_shows_focus() {
        let target = css_rule(PAGER_CSS, ".pager-target {");
        assert!(
            has_decl(
                &target,
                "scroll-margin-top: calc(var(--nav-height-used) + var(--space-section))"
            ),
            "{target}"
        );
        let focus = css_rule(PAGER_CSS, ".pager-target:focus-visible {");
        assert!(
            has_decl(
                &focus,
                "outline: var(--focus-ring-width) solid var(--focus-ring)"
            ),
            "{focus}"
        );
        // Native build: no DOM, so the call is a no-op and must not panic.
        show_list_top("missing");
    }

    #[test]
    fn targets_are_44px_and_sizes_come_from_tokens() {
        assert_sizes_from_tokens(PAGER_CSS);
        let button = css_rule(PAGER_CSS, ".pager-pages button {");
        for decl in [
            "min-width: var(--hit-target-min)",
            "min-height: var(--hit-target-min)",
            "height: var(--hit-target-min)",
            "font-family: var(--font-head)",
            "font-size: var(--text-sm)",
            "background: var(--surface)",
            "color: var(--text)",
        ] {
            assert!(has_decl(&button, decl), "missing {decl}: {button}");
        }
        let pager = css_rule(PAGER_CSS, ".pager {");
        assert!(has_decl(&pager, "flex-wrap: wrap"), "{pager}");
        let pages = css_rule(PAGER_CSS, ".pager-pages {");
        assert!(has_decl(&pages, "flex-wrap: wrap"), "{pages}");
    }

    #[test]
    fn current_page_is_not_colour_alone_and_text_clears_contrast() {
        let current = css_rule(PAGER_CSS, ".pager-pages .page-btn.is-current {");
        for decl in [
            "background: var(--accent-soft)",
            "color: var(--accent-text)",
            "border-color: var(--accent-text)",
            "box-shadow: inset 0 0 0 1px var(--accent-text)",
            "font-weight: var(--chip-selected-weight)",
        ] {
            assert!(has_decl(&current, decl), "missing {decl}: {current}");
        }
        assert_ink_clears_aa("--accent-text", "--accent-soft", 4.5);
        assert_ink_clears_aa("--text", "--surface", 4.5);
        // Hover uses the same edge as the other outlined buttons.
        assert!(has_decl(
            &css_rule(PAGER_CSS, ".pager-pages button:hover {"),
            "border-color: var(--text)"
        ));
        // The range line sits on the page background or on a card.
        let meta = css_rule(PAGER_CSS, ".pager-meta {");
        assert!(has_decl(&meta, "color: var(--text-3)"), "{meta}");
        assert_ink_clears_aa("--text-3", "--bg", 4.5);
        assert_ink_clears_aa("--text-3", "--surface", 4.5);
        // Disabled text is exempt from 4.5:1; the house floor is 3:1.
        let off = css_rule(PAGER_CSS, ".pager-pages .is-disabled:hover {");
        assert!(has_decl(&off, "color: var(--text-disabled)"), "{off}");
        assert!(has_decl(&off, "background: var(--surface)"), "{off}");
        assert_ink_clears_aa("--text-disabled", "--surface", 3.0);
    }
}
