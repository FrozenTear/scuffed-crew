//! Shared pieces for the reading pages: Blog, News and Wiki.
//!
//! Author names: rows only carry a member id (`author_member_id`, or
//! `author_id` on announcements). Names come from the public member profile
//! (`GET /api/public/members/{id}`), the same endpoint the member profile
//! page uses. The raw id is never shown: a missing, inactive or failed lookup
//! reads as [`AUTHOR_FALLBACK`].
//!
//! Lookups are keyed per author. An id only gets an entry in [`AuthorNames`]
//! once its lookup has finished, so an author that is still loading has no
//! label yet. [`use_author_names`] looks up only the authors on screen and
//! keeps what it has learned, so a page change costs no repeat requests.
//!
//! Layout: [`ARTICLE_CSS`] holds the entry list (index cards), the breadcrumb,
//! the article byline and the status line. The article body itself is styled
//! by `crate::markdown::MARKDOWN_CSS`. Buttons, the empty card and the pager
//! come from `components/ui`.

use std::collections::{HashMap, HashSet};
use std::future::Future;

use dioxus::prelude::*;
use serde::Deserialize;

use super::public_fetch::{PublicFetch, fetch_public};
use crate::routes::Route;

pub(crate) const AUTHOR_FALLBACK: &str = "Crew member";

/// Finished lookups: `Some(name)` on success, `None` when the lookup failed
/// or the member is not public. Ids still loading have no entry.
pub(crate) type AuthorNames = HashMap<String, Option<String>>;

#[derive(Deserialize)]
struct PublicMemberName {
    display_name: String,
}

/// Label for `member_id`, or `None` while its lookup has not finished.
pub(crate) fn author_label(names: Option<&AuthorNames>, member_id: &str) -> Option<String> {
    let resolved = names?.get(member_id)?;
    let name = resolved
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or(AUTHOR_FALLBACK);
    Some(name.to_string())
}

/// Credit for `member_id` ("By Player02", "Posted by Player06") followed by
/// its separator dot, or nothing while the lookup is pending. The dot is
/// hidden from assistive tech.
pub(crate) fn author_byline(
    names: Option<&AuthorNames>,
    member_id: &str,
    prefix: &'static str,
) -> Element {
    match author_label(names, member_id) {
        Some(label) => rsx! {
            span { class: "entry-author", "{prefix} {label}" }
            span { class: "entry-sep", aria_hidden: "true", "·" }
        },
        None => rsx! {},
    }
}

/// Up to two initials of a display name, upper case. Two or more words give
/// the first letter of the first two ("Player Two" is "PT"); a single word
/// gives its first two letters or digits ("Player02" is "PL"), the same two
/// the account avatar in the top bar shows for that name.
pub(crate) fn initials(name: &str) -> String {
    let mut words = name.split_whitespace();
    let (first, second) = (words.next().unwrap_or(""), words.next());
    let picked: Vec<char> = match second {
        Some(second) => first
            .chars()
            .take(1)
            .chain(second.chars().take(1))
            .collect(),
        None => first
            .chars()
            .filter(|c| c.is_alphanumeric())
            .take(2)
            .collect(),
    };
    picked.into_iter().flat_map(char::to_uppercase).collect()
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// `2026-11-12T19:00:00Z` as (`2026-11-12`, `12 Nov 2026`): the machine date
/// and the visible one. `None` when the value does not start with a date.
/// The date is the UTC one the server stored.
pub(crate) fn short_date(iso: &str) -> Option<(String, String)> {
    let date = iso.get(..10)?;
    let mut parts = date.split('-');
    let (year, month, day) = (parts.next()?, parts.next()?, parts.next()?);
    if year.len() != 4 || month.len() != 2 || day.len() != 2 {
        return None;
    }
    year.parse::<u16>().ok()?;
    let month_name = MONTHS.get(month.parse::<usize>().ok()?.checked_sub(1)?)?;
    let day_number = day.parse::<u8>().ok().filter(|d| (1..=31).contains(d))?;
    Some((
        date.to_string(),
        format!("{day_number} {month_name} {year}"),
    ))
}

/// `<time>` for a stored timestamp, or the raw text when it is not a date.
pub(crate) fn entry_date(iso: &str) -> Element {
    match short_date(iso) {
        Some((machine, text)) => rsx! {
            time { class: "entry-date", datetime: "{machine}", "{text}" }
        },
        None => rsx! { span { class: "entry-date", "{iso}" } },
    }
}

/// State of a list or article fetch, as the views take it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Load<T> {
    Loading,
    /// The fetch failed for a reason a retry may fix.
    Failed,
    /// The server said 404.
    Missing,
    Ready(T),
}

impl<T: Clone> Load<T> {
    /// From a `use_resource` slot holding `Result<T, String>` where the error
    /// is the client error text.
    pub(crate) fn of(slot: Option<&Result<T, String>>) -> Self {
        match slot {
            None => Load::Loading,
            Some(Ok(value)) => Load::Ready(value.clone()),
            Some(Err(err)) if crate::components::is_http_status(err, 404) => Load::Missing,
            Some(Err(_)) => Load::Failed,
        }
    }

    /// From a `use_api*` slot: `None` is in flight, `Some(None)` failed.
    pub(crate) fn of_api(slot: Option<&Option<T>>) -> Self {
        match slot {
            None => Load::Loading,
            Some(None) => Load::Failed,
            Some(Some(value)) => Load::Ready(value.clone()),
        }
    }
}

impl<T> Load<Batched<T>> {
    /// The rows, and whether the load stopped at its batch budget.
    pub(crate) fn split(self) -> (Load<Vec<T>>, bool) {
        match self {
            Load::Loading => (Load::Loading, false),
            Load::Failed => (Load::Failed, false),
            Load::Missing => (Load::Missing, false),
            Load::Ready(batched) => (Load::Ready(batched.items), batched.truncated),
        }
    }
}

/// A Retry is running. `use_resource` keeps the failed value while it
/// refetches, so the error stays on screen and its button says "Retrying...".
pub(crate) fn is_retrying<T>(state: UseResourceState, load: &Load<T>) -> bool {
    state == UseResourceState::Pending && matches!(load, Load::Failed)
}

/// Text of an article page's status line: the loading words during the first
/// load and during a retry, empty otherwise.
pub(crate) fn load_status<T>(
    load: &Load<T>,
    retrying: bool,
    loading: &'static str,
) -> &'static str {
    if retrying || matches!(load, Load::Loading) {
        loading
    } else {
        ""
    }
}

/// Text of a list page's status line. Loading words during a load or a retry;
/// once loaded, a line saying the list was cut off when it was.
pub(crate) fn list_status<T>(
    load: &Load<Vec<T>>,
    retrying: bool,
    capped: bool,
    loading: &'static str,
    noun: &str,
) -> String {
    match load {
        _ if retrying => loading.to_string(),
        Load::Loading => loading.to_string(),
        Load::Ready(list) if capped => format!("Showing the newest {} {noun}.", list.len()),
        _ => String::new(),
    }
}

/// Rows per page on the Blog, News and Wiki lists.
pub(crate) const ENTRY_PAGE_SIZE: usize = 10;

/// Rows per request on the `limit`/`offset` list endpoints (the server cap).
pub(crate) const LIST_BATCH: usize = 100;

/// Requests one load follows before it stops and says so.
pub(crate) const LIST_MAX_BATCHES: usize = 10;

/// A list loaded in batches. `truncated` is true when rows exist past the
/// last batch allowed, so older rows were not loaded.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Batched<T> {
    pub items: Vec<T>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BatchStep {
    Continue,
    /// The budget is spent and its last batch was full. One more request
    /// tells a list of exactly that many rows from a longer one; its rows
    /// are not kept.
    Probe,
    Done,
    Truncated,
}

/// What to do after batch number `batches_done` (counted from 1) came back
/// with `batch_len` rows. A short batch is the end of the list. Batch
/// `LIST_MAX_BATCHES + 1` is the probe: empty means the list ended exactly
/// on the budget, anything else means it was cut.
fn batch_step(batch_len: usize, batches_done: usize) -> BatchStep {
    if batches_done > LIST_MAX_BATCHES {
        if batch_len == 0 {
            BatchStep::Done
        } else {
            BatchStep::Truncated
        }
    } else if batch_len < LIST_BATCH {
        BatchStep::Done
    } else if batches_done == LIST_MAX_BATCHES {
        BatchStep::Probe
    } else {
        BatchStep::Continue
    }
}

/// Load a `limit`/`offset` list. The endpoints return no total, so this asks
/// for the next offset while a batch comes back full, up to the batch budget.
/// `fetch_batch` gets the offset to request.
pub(crate) async fn fetch_offset_list<T, F, Fut>(fetch_batch: F) -> Result<Batched<T>, String>
where
    F: Fn(usize) -> Fut,
    Fut: Future<Output = Result<Vec<T>, String>>,
{
    let mut items = Vec::new();
    let mut batches_done = 0;
    loop {
        let batch = fetch_batch(batches_done * LIST_BATCH).await?;
        let batch_len = batch.len();
        batches_done += 1;
        // The probe's rows are past the budget: they only answer "is there
        // more", so the list stays at the newest rows the budget allows.
        if batches_done <= LIST_MAX_BATCHES {
            items.extend(batch);
        }
        match batch_step(batch_len, batches_done) {
            BatchStep::Continue | BatchStep::Probe => {}
            BatchStep::Done => {
                return Ok(Batched {
                    items,
                    truncated: false,
                });
            }
            BatchStep::Truncated => {
                return Ok(Batched {
                    items,
                    truncated: true,
                });
            }
        }
    }
}

/// `StyleOnce` id for [`ARTICLE_CSS`].
pub(crate) const ARTICLE_CSS_ID: &str = "scuffed-css-pages-article-shared";

/// Entry list, breadcrumb, byline and status line for Blog, News and Wiki.
pub(crate) const ARTICLE_CSS: &str = r#"
.entry-list {
  display: grid;
  gap: var(--space-3);
  margin: 0;
  padding: 0;
  list-style: none;
  min-width: 0;
}
.entry-row {
  position: relative;
  box-sizing: border-box;
  min-width: 0;
  padding: var(--space-card-pad);
  background: var(--card-bg);
  color: var(--card-fg);
  border: 1px solid var(--card-border);
  border-radius: var(--card-radius);
  box-shadow: var(--card-shadow);
}
/* Plain text flow, not a flex row: a pill after the title follows its last
   word and wraps with it, on a phone too. */
.entry-row h2 {
  margin: 0 0 var(--space-2);
  font-family: var(--font-head);
  font-size: var(--text-lg);
  font-weight: var(--font-label-weight);
  letter-spacing: var(--font-head-tracking);
  line-height: var(--leading-ui);
  color: var(--text);
  overflow-wrap: anywhere;
}
.entry-row h2 .ui-pill {
  margin-inline-start: var(--space-2);
  vertical-align: middle;
}
/* A linked title is in the link ink, as the frames draw it: on a touch
   screen there is no hover to tell a card that opens from one that does not. */
.entry-row h2 a {
  color: var(--accent-text);
  text-decoration: none;
}
/* The title link covers the whole card, so the target is the card, not one
   line of text. */
.entry-row h2 a::after {
  content: "";
  position: absolute;
  inset: 0;
  border-radius: inherit;
}
.entry-row h2 a:hover { text-decoration: underline; }
.entry-row:has(h2 a:hover) { border-color: var(--border-strong); }
.entry-row h2 a:focus-visible { outline: none; }
.entry-row h2 a:focus-visible::after {
  outline: var(--focus-ring-width) solid var(--focus-ring);
  outline-offset: var(--focus-ring-offset);
}
.entry-row p {
  margin: 0;
  color: var(--text-3);
  font-family: var(--font-body);
  font-size: var(--text-sm);
  line-height: var(--leading-body);
  overflow-wrap: anywhere;
}
.entry-row .entry-meta {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-2);
  margin-top: var(--space-1);
}
.entry-date { white-space: nowrap; }

.entry-status {
  margin: 0;
  color: var(--text-3);
  font-family: var(--font-body);
  font-size: var(--text-sm);
  line-height: var(--leading-body);
}
/* Nothing to say: out of the layout, still in the tree so the next text is
   announced. */
.entry-status:empty {
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

/* One line: the current page is cut with an ellipsis when it does not fit.
   The H1 right under it carries the full title. */
.article-crumb {
  display: flex;
  flex-wrap: nowrap;
  align-items: center;
  gap: var(--space-2);
  margin: 0;
  min-width: 0;
  font-family: var(--font-body);
  font-size: var(--text-sm);
  line-height: var(--leading-ui);
  color: var(--text-3);
}
/* 44px each way. A short label ("Blog") gets its width from inline padding,
   pulled back by the same margin so the text stays on the column edge. */
.article-crumb a {
  flex: 0 0 auto;
  display: inline-flex;
  align-items: center;
  min-height: var(--hit-target-min);
  min-width: var(--hit-target-min);
  padding-inline: var(--space-3);
  margin-inline: calc(-1 * var(--space-3));
  color: var(--accent-text);
  text-decoration: underline;
}
.article-crumb a:hover { text-decoration: none; }
.article-crumb [aria-current="page"] {
  flex: 0 1 auto;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text);
}

.article-byline {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-2);
  margin: var(--space-3) 0 0;
  color: var(--text-3);
  font-family: var(--font-body);
  font-size: var(--text-sm);
  line-height: var(--leading-ui);
}
.article-avatar {
  flex: 0 0 auto;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: var(--control-size-sm);
  height: var(--control-size-sm);
  border-radius: var(--radius-pill);
  border: 1px solid var(--border-strong);
  background: var(--surface);
  color: var(--text);
  font-family: var(--font-head);
  font-size: var(--text-xs);
  font-weight: var(--font-head-weight);
}
.article-cover {
  display: block;
  width: 100%;
  height: auto;
  border-radius: var(--radius-card);
  border: 1px solid var(--border);
}
"#;

/// Breadcrumb over an article: the section link, then the article title as
/// the current page once it is known.
#[component]
pub(crate) fn ArticleCrumb(
    to: Route,
    section: String,
    #[props(default)] current: Option<String>,
) -> Element {
    rsx! {
        nav { class: "article-crumb", aria_label: "Breadcrumb",
            Link { to, "{section}" }
            if let Some(current) = current {
                span { aria_hidden: "true", "/" }
                span { aria_current: "page", "{current}" }
            }
        }
    }
}

/// Author line under an article title: initials disc, display name, then the
/// date part the page passes as children. The name stays out while its lookup
/// is pending.
#[component]
pub(crate) fn ArticleByline(author: Option<String>, children: Element) -> Element {
    rsx! {
        p { class: "article-byline",
            if let Some(name) = author {
                span { class: "article-avatar", aria_hidden: "true", "{initials(&name)}" }
                span { class: "entry-author", "{name}" }
                span { class: "entry-sep", aria_hidden: "true", "·" }
            }
            {children}
        }
    }
}

/// Non-empty ids in first-seen order, each once.
fn distinct_author_ids(ids: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    ids.into_iter()
        .filter(|id| !id.is_empty() && seen.insert(id.clone()))
        .collect()
}

/// The ids in `wanted` that have no finished lookup in `known` yet. A lookup
/// that failed has no entry, so it is asked for again.
fn missing_author_ids(wanted: Vec<String>, known: &AuthorNames) -> Vec<String> {
    distinct_author_ids(wanted)
        .into_iter()
        .filter(|id| !known.contains_key(id))
        .collect()
}

/// What a finished lookup stores. `Some(Some(name))` for a public member and
/// `Some(None)` for a member the API does not show (the fallback label).
/// `None` for a request that failed: nothing is stored, so the name is asked
/// for again the next time its row is shown, not only after a reload.
fn author_lookup_entry(fetched: PublicFetch<PublicMemberName>) -> Option<Option<String>> {
    match fetched {
        PublicFetch::Found(m) => Some(Some(m.display_name)),
        PublicFetch::NotFound => Some(None),
        PublicFetch::Failed => None,
    }
}

/// Look one member id up. See [`author_lookup_entry`].
async fn fetch_author_name(id: &str) -> Option<Option<String>> {
    let path = format!("/api/public/members/{}", crate::util::encode_query(id));
    author_lookup_entry(fetch_public::<PublicMemberName>(&path).await)
}

/// Display names for the authors on screen.
///
/// `visible_ids` reads the page's signals and returns the author ids of the
/// rows now shown (one page of a list, or one article). Only ids with no
/// finished lookup are requested, and each name lands in the map as soon as it
/// resolves, so the first byline does not wait for the last one and a row on
/// another page costs nothing until it is shown.
pub(crate) fn use_author_names(
    visible_ids: impl Fn() -> Vec<String> + 'static,
) -> Signal<AuthorNames> {
    let mut names = use_signal(AuthorNames::new);
    let _lookups = use_resource(move || {
        let wanted = missing_author_ids(visible_ids(), &names.peek());
        async move {
            for id in wanted {
                if let Some(entry) = fetch_author_name(&id).await {
                    names.write().insert(id, entry);
                }
            }
        }
    });
    names
}

/// Test-only: the shared routed renderer, under the name the article pages
/// already use.
#[cfg(test)]
pub(crate) mod test_support {
    pub(crate) use crate::components::ui::test_support::render_routed;
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "member:7f3a9c2e1b";

    #[test]
    fn known_author_shows_display_name() {
        let names = AuthorNames::from([(ID.to_string(), Some("Player01".to_string()))]);
        assert_eq!(author_label(Some(&names), ID).as_deref(), Some("Player01"));
    }

    #[test]
    fn pending_author_has_no_label() {
        // No map yet, or a map from an earlier run that lacks this id.
        let stale = AuthorNames::new();
        let other = AuthorNames::from([("member:other".to_string(), Some("Player02".to_string()))]);
        for names in [None, Some(&stale), Some(&other)] {
            assert_eq!(author_label(names, ID), None);
        }
    }

    #[test]
    fn raw_id_is_never_shown() {
        let failed = AuthorNames::from([(ID.to_string(), None)]);
        let blank = AuthorNames::from([(ID.to_string(), Some("   ".to_string()))]);
        for names in [&failed, &blank] {
            let label = author_label(Some(names), ID).expect("lookup finished");
            assert_eq!(label, AUTHOR_FALLBACK);
            assert!(!label.contains(ID));
        }
    }

    #[test]
    fn distinct_ids_dedup_and_skip_empty() {
        let ids = ["a", "b", "", "a", "c", "b"].map(String::from).to_vec();
        assert_eq!(distinct_author_ids(ids), ["a", "b", "c"]);
    }

    #[test]
    fn only_authors_without_a_finished_lookup_are_requested() {
        let known = AuthorNames::from([
            ("a".to_string(), Some("Player01".to_string())),
            // A member the API does not show is finished too: not asked again.
            ("b".to_string(), None),
        ]);
        let wanted = ["a", "c", "b", "", "c", "d"].map(String::from).to_vec();
        assert_eq!(missing_author_ids(wanted, &known), ["c", "d"]);
        assert!(missing_author_ids(vec!["a".to_string()], &known).is_empty());
    }

    #[test]
    fn a_full_batch_asks_for_the_next_until_the_budget_is_spent() {
        assert_eq!(batch_step(0, 1), BatchStep::Done);
        assert_eq!(batch_step(LIST_BATCH - 1, 1), BatchStep::Done);
        assert_eq!(batch_step(LIST_BATCH, 1), BatchStep::Continue);
        assert_eq!(
            batch_step(LIST_BATCH, LIST_MAX_BATCHES - 1),
            BatchStep::Continue
        );
        // A full last batch does not prove a cut: one probe request does.
        assert_eq!(batch_step(LIST_BATCH, LIST_MAX_BATCHES), BatchStep::Probe);
        assert_eq!(batch_step(0, LIST_MAX_BATCHES + 1), BatchStep::Done);
        assert_eq!(batch_step(1, LIST_MAX_BATCHES + 1), BatchStep::Truncated);
        assert_eq!(
            batch_step(LIST_BATCH, LIST_MAX_BATCHES + 1),
            BatchStep::Truncated
        );
        // A short last batch is the end of the list, not a cut.
        assert_eq!(batch_step(3, LIST_MAX_BATCHES), BatchStep::Done);
    }

    /// Run the batched loader over a list of `total` rows. Returns the result
    /// and the offsets it asked for. The fake fetch is ready at once, so one
    /// poll finishes it.
    fn load_batched(total: usize) -> (Batched<usize>, Vec<usize>) {
        use std::cell::RefCell;
        use std::pin::pin;
        use std::task::{Context, Poll, Waker};
        let offsets = RefCell::new(Vec::new());
        let result = {
            let future = fetch_offset_list(|offset| {
                offsets.borrow_mut().push(offset);
                let end = total.min(offset + LIST_BATCH);
                let rows: Vec<usize> = (offset.min(end)..end).collect();
                async move { Ok::<_, String>(rows) }
            });
            let mut future = pin!(future);
            match future
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
            {
                Poll::Ready(result) => result.expect("the fake fetch never fails"),
                Poll::Pending => panic!("the fake fetch is always ready"),
            }
        };
        (result, offsets.into_inner())
    }

    #[test]
    fn a_list_of_exactly_the_budget_is_not_called_cut() {
        let budget = LIST_BATCH * LIST_MAX_BATCHES;
        assert_eq!(budget, 1000);
        // Exactly 1,000 rows: every batch is full, the probe comes back
        // empty, and nothing claims older rows exist.
        let (exact, offsets) = load_batched(budget);
        assert_eq!(exact.items.len(), budget);
        assert!(!exact.truncated);
        assert_eq!(offsets.len(), LIST_MAX_BATCHES + 1);
        assert_eq!(offsets.last(), Some(&budget));
        let status = list_status(
            &Load::Ready(exact.items),
            false,
            exact.truncated,
            "Loading",
            "posts",
        );
        assert_eq!(status, "");
        // One row more: cut, and the probe's row is not shown.
        let (over, offsets) = load_batched(budget + 1);
        assert_eq!(over.items.len(), budget);
        assert_eq!(over.items.last(), Some(&(budget - 1)));
        assert!(over.truncated);
        assert_eq!(offsets.len(), LIST_MAX_BATCHES + 1);
        // Short lists stop at the first short batch, with no probe.
        let (short, offsets) = load_batched(250);
        assert_eq!(short.items.len(), 250);
        assert!(!short.truncated);
        assert_eq!(offsets, [0, 100, 200]);
        let (empty, offsets) = load_batched(0);
        assert!(empty.items.is_empty() && !empty.truncated);
        assert_eq!(offsets, [0]);
    }

    #[test]
    fn keyed_cards_are_mapped_into_their_lists() {
        // `for x in xs { {render_card(x)} }` puts each card inside an unkeyed
        // wrapper, so the `key` on the card's root never reaches the list
        // and rows are patched in place when the page changes. Mapping the
        // iterator makes each keyed card a direct child.
        for (file, src, card) in [
            ("blog.rs", include_str!("blog.rs"), "render_blog_card"),
            ("news.rs", include_str!("news.rs"), "render_news_card"),
            ("wiki.rs", include_str!("wiki.rs"), "render_wiki_card"),
            (
                "wiki_page.rs",
                include_str!("wiki_page.rs"),
                "render_revision",
            ),
            (
                "home/blocks.rs",
                include_str!("home/blocks.rs"),
                "render_news_card",
            ),
            (
                "strategy/patch_notes.rs",
                include_str!("strategy/patch_notes.rs"),
                "render_patch_row",
            ),
        ] {
            let page = src.split("#[cfg(test)]").next().unwrap_or(src);
            let wrapped = ["{", card, "("].concat();
            assert!(!page.contains(&wrapped), "{file}: {card} is wrapped");
            let by_closure = ["| ", card, "("].concat();
            let by_name = [".map(", card, ")"].concat();
            assert!(
                page.contains(&by_closure) || page.contains(&by_name),
                "{file}: {card} is not mapped"
            );
            // The card's root carries the key.
            let body = page
                .split(&["fn ", card, "("].concat())
                .nth(1)
                .unwrap_or_else(|| panic!("{file}: no {card}"));
            let root = body.split("rsx! {").nth(1).unwrap_or_default();
            let head: String = root.chars().take(400).collect();
            assert!(head.contains("key:"), "{file}: {card} root has no key");
        }
    }

    #[test]
    fn a_failed_author_lookup_is_not_cached() {
        let found = PublicFetch::Found(PublicMemberName {
            display_name: "Player02".to_string(),
        });
        assert_eq!(
            author_lookup_entry(found),
            Some(Some("Player02".to_string()))
        );
        // Not public: finished, shown with the fallback label.
        assert_eq!(author_lookup_entry(PublicFetch::NotFound), Some(None));
        // A failed request stores nothing, so the id is still missing and is
        // asked for again the next time its row is shown.
        assert_eq!(author_lookup_entry(PublicFetch::Failed), None);
        let known = AuthorNames::new();
        assert_eq!(missing_author_ids(vec!["a".to_string()], &known), ["a"]);
    }

    #[test]
    fn batched_load_splits_into_rows_and_the_cut_flag() {
        let ready = Load::Ready(Batched {
            items: vec![1u8, 2],
            truncated: true,
        });
        assert_eq!(ready.split(), (Load::Ready(vec![1u8, 2]), true));
        assert_eq!(
            Load::<Batched<u8>>::Failed.split(),
            (Load::<Vec<u8>>::Failed, false)
        );
        assert_eq!(
            Load::<Batched<u8>>::Loading.split(),
            (Load::<Vec<u8>>::Loading, false)
        );
    }

    #[test]
    fn status_lines_cover_loading_retry_and_a_cut_list() {
        let ready = Load::Ready(vec![1u8, 2, 3]);
        assert_eq!(
            list_status(
                &Load::<Vec<u8>>::Loading,
                false,
                false,
                "Loading posts",
                "posts"
            ),
            "Loading posts"
        );
        // A retry keeps the failed value on screen and says it is loading.
        assert_eq!(
            list_status(
                &Load::<Vec<u8>>::Failed,
                true,
                false,
                "Loading posts",
                "posts"
            ),
            "Loading posts"
        );
        assert_eq!(
            list_status(
                &Load::<Vec<u8>>::Failed,
                false,
                false,
                "Loading posts",
                "posts"
            ),
            ""
        );
        assert_eq!(
            list_status(&ready, false, false, "Loading posts", "posts"),
            ""
        );
        assert_eq!(
            list_status(&ready, false, true, "Loading posts", "posts"),
            "Showing the newest 3 posts."
        );
        assert_eq!(
            load_status(&Load::<u8>::Loading, false, "Loading post"),
            "Loading post"
        );
        assert_eq!(
            load_status(&Load::<u8>::Failed, true, "Loading post"),
            "Loading post"
        );
        assert_eq!(load_status(&Load::Ready(1u8), false, "Loading post"), "");

        assert!(is_retrying(UseResourceState::Pending, &Load::<u8>::Failed));
        assert!(!is_retrying(UseResourceState::Ready, &Load::<u8>::Failed));
        assert!(!is_retrying(UseResourceState::Pending, &Load::Ready(1u8)));
        assert!(!is_retrying(
            UseResourceState::Pending,
            &Load::<u8>::Loading
        ));
    }

    #[test]
    fn api_slots_map_to_load_states() {
        assert_eq!(Load::<u8>::of_api(None), Load::Loading);
        assert_eq!(Load::<u8>::of_api(Some(&None)), Load::Failed);
        assert_eq!(Load::of_api(Some(&Some(7u8))), Load::Ready(7));
    }

    #[derive(Props, Clone, PartialEq)]
    struct BylineProbe {
        names: Option<AuthorNames>,
    }

    #[allow(non_snake_case)]
    fn Probe(props: BylineProbe) -> Element {
        rsx! { div { {author_byline(props.names.as_ref(), ID, "By")} } }
    }

    fn render_byline(names: Option<AuthorNames>) -> String {
        let mut dom = VirtualDom::new_with_props(Probe, BylineProbe { names });
        dom.rebuild_in_place();
        dioxus_ssr::render(&dom)
    }

    #[test]
    fn pending_state_renders_no_byline() {
        for names in [None, Some(AuthorNames::new())] {
            let html = render_byline(names);
            assert!(!html.contains("By "), "{html}");
            assert!(!html.contains("entry-sep"), "{html}");
            assert!(!html.contains(AUTHOR_FALLBACK), "{html}");
            assert!(!html.contains(ID), "{html}");
        }
    }

    #[test]
    fn hostile_display_name_renders_as_text() {
        let hostile = "<img src=x onerror=alert(1)>&";
        let names = AuthorNames::from([(ID.to_string(), Some(hostile.to_string()))]);
        let html = render_byline(Some(names));
        assert!(!html.contains("<img"), "{html}");
        assert!(
            html.contains("By &#60;img src=x onerror=alert(1)&#62;&#38;</span>"),
            "{html}"
        );
        assert!(html.contains("<span class=\"entry-author\">By "), "{html}");
    }
    #[test]
    fn short_date_is_day_month_year() {
        assert_eq!(
            short_date("2026-11-12T18:45:00Z"),
            Some(("2026-11-12".to_string(), "12 Nov 2026".to_string()))
        );
        assert_eq!(
            short_date("2026-01-05").map(|(_, text)| text).as_deref(),
            Some("5 Jan 2026")
        );
        for bad in [
            "",
            "Draft",
            "2026-13-01T00:00:00Z",
            "2026-00-10",
            "2026-02-00",
            "12/11/2026",
            "ééééééé",
        ] {
            assert_eq!(short_date(bad), None, "{bad}");
        }
    }

    #[test]
    fn entry_date_is_a_time_element_or_plain_text() {
        fn app() -> Element {
            rsx! {
                {entry_date("2026-11-12T18:45:00Z")}
                {entry_date("someday")}
            }
        }
        let mut dom = VirtualDom::new(app);
        dom.rebuild_in_place();
        let html = dioxus_ssr::render(&dom);
        assert_eq!(
            html,
            "<time class=\"entry-date\" datetime=\"2026-11-12\">12 Nov 2026</time>\
             <span class=\"entry-date\">someday</span>"
        );
    }

    #[test]
    fn initials_take_up_to_two_words() {
        assert_eq!(initials("Player02"), "PL");
        assert_eq!(initials("x"), "X");
        assert_eq!(initials("player two extra"), "PT");
        assert_eq!(initials("  "), "");
    }

    #[test]
    fn load_tells_missing_from_failed() {
        let ok: Result<u8, String> = Ok(7);
        let missing: Result<u8, String> = Err("HTTP error: 404".to_string());
        let down: Result<u8, String> = Err("HTTP error: 500".to_string());
        assert_eq!(Load::<u8>::of(None), Load::Loading);
        assert_eq!(Load::of(Some(&ok)), Load::Ready(7));
        assert_eq!(Load::of(Some(&missing)), Load::Missing);
        assert_eq!(Load::of(Some(&down)), Load::Failed);
    }

    #[test]
    fn crumb_marks_the_current_page_once_the_title_is_known() {
        let html = test_support::render_routed(|| {
            rsx! {
                ArticleCrumb { to: Route::Wiki {}, section: "Wiki", current: "Tank basics".to_string() }
            }
        });
        assert_eq!(
            html,
            "<nav class=\"article-crumb\" aria-label=\"Breadcrumb\"><a href=\"/wiki\">Wiki</a>\
             <span aria-hidden=\"true\">/</span>\
             <span aria-current=\"page\">Tank basics</span></nav>"
        );

        let loading = test_support::render_routed(|| {
            rsx! { ArticleCrumb { to: Route::Blog {}, section: "Blog" } }
        });
        assert!(
            loading.contains("<a href=\"/blog\">Blog</a></nav>"),
            "{loading}"
        );
        assert!(!loading.contains("aria-current"), "{loading}");
    }

    #[test]
    fn shared_styles_use_tokens_and_keep_targets_at_44() {
        use crate::components::ui::test_support::{assert_sizes_from_tokens, css_rule, has_decl};

        assert_sizes_from_tokens(ARTICLE_CSS);
        let crumb = css_rule(ARTICLE_CSS, ".article-crumb a {");
        for decl in [
            "min-height: var(--hit-target-min)",
            "min-width: var(--hit-target-min)",
        ] {
            assert!(has_decl(&crumb, decl), "{decl}: {crumb}");
        }
        // The trail is one line; a long current page is cut, not wrapped.
        let current = css_rule(ARTICLE_CSS, ".article-crumb [aria-current=\"page\"] {");
        assert!(has_decl(&current, "text-overflow: ellipsis"), "{current}");
        // A pill in a card title follows the title text.
        let title = css_rule(ARTICLE_CSS, ".entry-row h2 {");
        assert!(!title.contains("display"), "{title}");
        // Buttons, the empty card and the scroll offset are the shared ones.
        for gone in [".article-btn", ".entry-empty", "scroll-margin-top"] {
            assert!(!ARTICLE_CSS.contains(gone), "{gone} is shared now");
        }
        // No primitive weight tokens in page CSS.
        assert!(
            !ARTICLE_CSS.contains("--font-weight-"),
            "use semantic weights"
        );
        // The title link is stretched over its card, so the card is the target.
        let stretch = css_rule(ARTICLE_CSS, ".entry-row h2 a::after {");
        assert!(has_decl(&stretch, "inset: 0"), "{stretch}");
        assert!(has_decl(
            &css_rule(ARTICLE_CSS, ".entry-row {"),
            "padding: var(--space-card-pad)"
        ));
        // The empty status line leaves the layout but stays in the tree.
        let empty = css_rule(ARTICLE_CSS, ".entry-status:empty {");
        assert!(has_decl(&empty, "position: absolute"), "{empty}");
        assert!(!empty.contains("display"), "{empty}");
    }
}
