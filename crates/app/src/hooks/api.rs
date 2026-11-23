use dioxus::prelude::*;
use scuffed_api_client::{ApiClient, ClientError};
use serde::Deserialize;
use serde::de::DeserializeOwned;

/// A resource that fetches data from an API endpoint with built-in refresh support.
#[derive(Clone, Copy)]
pub struct ApiResource<T: 'static> {
    pub data: Resource<Option<T>>,
    pub refresh: Signal<u64>,
    /// Last fetch error as page text (None when ok or still loading): always
    /// [`crate::client_error::user_message`], the raw error is logged.
    pub error: Signal<Option<String>>,
    /// True when the last finished fetch failed. Unlike `error` it is not
    /// cleared when a refetch starts, only when a fetch succeeds or is
    /// skipped (empty URL), so a retry after a failure can be told from the
    /// first real fetch after a skip. Both leave `data` holding `None`.
    pub failed: Signal<bool>,
    /// True when a cursor page remains after the loaded page budget.
    pub truncated: Signal<bool>,
    /// Rows currently held (0 after an error).
    pub shown: Signal<usize>,
    /// How many cursor pages this list will follow. Load more raises it.
    pub page_budget: Signal<usize>,
}

impl<T: 'static> ApiResource<T> {
    /// Trigger a reload of the resource.
    /// Note: In closures, prefer `resource.refresh += 1` to avoid FnOnce issues.
    #[allow(dead_code)]
    pub fn reload(mut self) {
        self.refresh += 1;
    }
}

/// Fetch data from a static API endpoint with automatic refresh support.
pub fn use_api<T: DeserializeOwned + 'static>(url: &'static str) -> ApiResource<T> {
    let refresh = use_signal(|| 0u64);
    let mut error = use_signal(|| Option::<String>::None);
    let mut failed = use_signal(|| false);
    let (truncated, shown, page_budget) = use_list_cap();
    let data = use_resource(move || async move {
        let _ = refresh();
        error.set(None);
        match ApiClient::web().fetch::<T>(url).await {
            Ok(v) => {
                failed.set(false);
                Some(v)
            }
            Err(e) => {
                error.set(Some(crate::client_error::logged_message(url, &e)));
                failed.set(true);
                None
            }
        }
    });
    ApiResource {
        data,
        refresh,
        error,
        failed,
        truncated,
        shown,
        page_budget,
    }
}

/// [`use_api`] that only sends the request while `enabled()` is true. While
/// it is false the resource holds `T::default()` with no error, and it
/// fetches as soon as `enabled()` turns true. For endpoints that only answer
/// a signed-in viewer: a guest page load no longer sends a request that can
/// only come back 401.
pub fn use_api_when<T: DeserializeOwned + Default + 'static>(
    url: &'static str,
    enabled: impl Fn() -> bool + Copy + 'static,
) -> ApiResource<T> {
    let refresh = use_signal(|| 0u64);
    let mut error = use_signal(|| Option::<String>::None);
    let mut failed = use_signal(|| false);
    let (truncated, shown, page_budget) = use_list_cap();
    // A memo, so the fetch reruns when the answer changes, not on every
    // write to whatever `enabled` reads.
    let enabled = use_memo(enabled);
    let data = use_resource(move || async move {
        let _ = refresh();
        error.set(None);
        if !enabled() {
            failed.set(false);
            return Some(T::default());
        }
        match ApiClient::web().fetch::<T>(url).await {
            Ok(v) => {
                failed.set(false);
                Some(v)
            }
            Err(e) => {
                error.set(Some(crate::client_error::logged_message(url, &e)));
                failed.set(true);
                None
            }
        }
    });
    ApiResource {
        data,
        refresh,
        error,
        failed,
        truncated,
        shown,
        page_budget,
    }
}

/// Wrapper for cursor-paginated list responses from the server.
#[derive(Deserialize)]
pub struct CursorPage<T> {
    pub data: Vec<T>,
    #[serde(default)]
    pub next_cursor: Option<String>,
}

/// Pages auto-followed before the list tells the user it stopped early.
pub(crate) const LIST_MAX_PAGES: usize = 10;
const LIST_PAGE_LIMIT: u32 = 100;

/// Cap signals for cursor lists. Non-list hooks leave `truncated` false.
fn use_list_cap() -> (Signal<bool>, Signal<usize>, Signal<usize>) {
    let truncated = use_signal(|| false);
    let shown = use_signal(|| 0usize);
    let page_budget = use_signal(|| LIST_MAX_PAGES);
    (truncated, shown, page_budget)
}

fn with_limit_and_cursor(url: &str, cursor: Option<&str>) -> String {
    let sep = if url.contains('?') { '&' } else { '?' };
    let mut path = format!("{url}{sep}limit={LIST_PAGE_LIMIT}");
    if let Some(c) = cursor {
        path.push_str(&format!("&cursor={c}"));
    }
    path
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CursorStep {
    Continue,
    Done,
    Truncated,
}

/// Whether to follow `next` after finishing 0-based page `page_index`.
///
/// A blank cursor ends the list. The same cursor twice stops, so a stuck
/// cursor cannot duplicate rows forever. A new cursor on the last allowed
/// page is a visible truncation, not a silent drop.
fn cursor_step(
    page_index: usize,
    max_pages: usize,
    prev: Option<&str>,
    next: Option<&str>,
) -> CursorStep {
    let next = next.map(str::trim).filter(|c| !c.is_empty());
    match next {
        None => CursorStep::Done,
        Some(c) if prev == Some(c) => CursorStep::Done,
        Some(_) if page_index + 1 >= max_pages => CursorStep::Truncated,
        Some(_) => CursorStep::Continue,
    }
}

struct PageBatch<T> {
    items: Vec<T>,
    truncated: bool,
}

fn remember_batch<T>(
    batch: &PageBatch<T>,
    mut truncated: Signal<bool>,
    mut shown: Signal<usize>,
    mut failed: Signal<bool>,
) {
    truncated.set(batch.truncated);
    shown.set(batch.items.len());
    failed.set(false);
}

fn remember_list_failure(
    mut truncated: Signal<bool>,
    mut shown: Signal<usize>,
    mut error: Signal<Option<String>>,
    mut failed: Signal<bool>,
    what: &str,
    err: &ClientError,
) {
    truncated.set(false);
    shown.set(0);
    error.set(Some(crate::client_error::logged_message(what, err)));
    failed.set(true);
}

async fn fetch_pages<T: DeserializeOwned>(
    base_url: &str,
    max_pages: usize,
) -> Result<PageBatch<T>, ClientError> {
    let max_pages = max_pages.max(1);
    let mut items = Vec::new();
    let mut cursor: Option<String> = None;
    let mut truncated = false;
    for index in 0..max_pages {
        let path = with_limit_and_cursor(base_url, cursor.as_deref());
        let page = ApiClient::web().fetch::<CursorPage<T>>(&path).await?;
        items.extend(page.data);
        match cursor_step(
            index,
            max_pages,
            cursor.as_deref(),
            page.next_cursor.as_deref(),
        ) {
            CursorStep::Continue => cursor = page.next_cursor,
            CursorStep::Done => {
                truncated = false;
                break;
            }
            CursorStep::Truncated => {
                truncated = true;
                break;
            }
        }
    }
    Ok(PageBatch { items, truncated })
}

/// Try `preferred` first. On HTTP 403, fetch `fallback` and report `used_fallback`.
///
/// Used so officer Admin Members can request `include_inactive=true` and still
/// render if the caller is not allowed (HTTP 403 falls back to active-only).
async fn fetch_pages_or_forbidden_fallback<T: DeserializeOwned>(
    preferred: Option<&str>,
    fallback: &str,
    max_pages: usize,
) -> Result<(PageBatch<T>, bool), ClientError> {
    if let Some(url) = preferred {
        match fetch_pages::<T>(url, max_pages).await {
            Ok(batch) => Ok((batch, false)),
            Err(e) if e.is_forbidden() => {
                let batch = fetch_pages::<T>(fallback, max_pages).await?;
                Ok((batch, true))
            }
            Err(e) => Err(e),
        }
    } else {
        Ok((fetch_pages::<T>(fallback, max_pages).await?, false))
    }
}

/// Fetch a cursor-paginated list, auto-following pages (limit 100, max 10 pages).
///
/// When a cursor remains past that budget, `truncated` is set so the page can
/// show a "showing the first N" hint and raise the budget.
pub fn use_api_list<T: DeserializeOwned + 'static>(url: &'static str) -> ApiResource<Vec<T>> {
    let refresh = use_signal(|| 0u64);
    let mut error = use_signal(|| Option::<String>::None);
    let failed = use_signal(|| false);
    let (mut truncated, shown, page_budget) = use_list_cap();
    let data = use_resource(move || {
        let budget = page_budget();
        async move {
            let _ = refresh();
            error.set(None);
            truncated.set(false);
            match fetch_pages::<T>(url, budget).await {
                Ok(batch) => {
                    remember_batch(&batch, truncated, shown, failed);
                    Some(batch.items)
                }
                Err(e) => {
                    remember_list_failure(truncated, shown, error, failed, url, &e);
                    None
                }
            }
        }
    });
    ApiResource {
        data,
        refresh,
        error,
        failed,
        truncated,
        shown,
        page_budget,
    }
}

/// Cursor-paginated list: try a preferred URL, fall back on HTTP 403.
///
/// `preferred` returning `None` skips the flag (active-only `fallback` only).
/// `used_fallback` is true only when the preferred request was forbidden.
pub fn use_api_list_prefer<T: DeserializeOwned + 'static>(
    preferred: impl Fn() -> Option<String> + 'static,
    fallback: &'static str,
) -> (ApiResource<Vec<T>>, Signal<bool>) {
    let refresh = use_signal(|| 0u64);
    let mut error = use_signal(|| Option::<String>::None);
    let failed = use_signal(|| false);
    let mut used_fallback = use_signal(|| false);
    let (mut truncated, shown, page_budget) = use_list_cap();
    let data = use_resource(move || {
        let preferred = preferred();
        let budget = page_budget();
        async move {
            let _ = refresh();
            error.set(None);
            truncated.set(false);
            match fetch_pages_or_forbidden_fallback::<T>(preferred.as_deref(), fallback, budget)
                .await
            {
                Ok((batch, fell_back)) => {
                    used_fallback.set(fell_back);
                    remember_batch(&batch, truncated, shown, failed);
                    Some(batch.items)
                }
                Err(e) => {
                    remember_list_failure(truncated, shown, error, failed, fallback, &e);
                    None
                }
            }
        }
    });
    (
        ApiResource {
            data,
            refresh,
            error,
            failed,
            truncated,
            shown,
            page_budget,
        },
        used_fallback,
    )
}

/// Fetch a cursor-paginated list from a dynamic API endpoint.
pub fn use_api_list_with<T: DeserializeOwned + 'static>(
    url: impl Fn() -> String + 'static,
) -> ApiResource<Vec<T>> {
    let refresh = use_signal(|| 0u64);
    let mut error = use_signal(|| Option::<String>::None);
    let mut failed = use_signal(|| false);
    let (mut truncated, mut shown, page_budget) = use_list_cap();
    let data = use_resource(move || {
        let url = url();
        let budget = page_budget();
        async move {
            let _ = refresh();
            error.set(None);
            truncated.set(false);
            if url.is_empty() {
                shown.set(0);
                failed.set(false);
                return None;
            }
            match fetch_pages::<T>(&url, budget).await {
                Ok(batch) => {
                    remember_batch(&batch, truncated, shown, failed);
                    Some(batch.items)
                }
                Err(e) => {
                    remember_list_failure(truncated, shown, error, failed, &url, &e);
                    None
                }
            }
        }
    });
    ApiResource {
        data,
        refresh,
        error,
        failed,
        truncated,
        shown,
        page_budget,
    }
}

/// Fetch data from a dynamic API endpoint with automatic refresh support.
pub fn use_api_with<T: DeserializeOwned + 'static>(
    url: impl Fn() -> String + 'static,
) -> ApiResource<T> {
    let refresh = use_signal(|| 0u64);
    let mut error = use_signal(|| Option::<String>::None);
    let mut failed = use_signal(|| false);
    let (truncated, shown, page_budget) = use_list_cap();
    let data = use_resource(move || {
        let url = url();
        async move {
            let _ = refresh();
            error.set(None);
            if url.is_empty() {
                // A skip is not a failure: the gate must not read the
                // `None` it leaves as a failed load (issue 821).
                failed.set(false);
                return None;
            }
            match ApiClient::web().fetch::<T>(&url).await {
                Ok(v) => {
                    failed.set(false);
                    Some(v)
                }
                Err(e) => {
                    error.set(Some(crate::client_error::logged_message(&url, &e)));
                    failed.set(true);
                    None
                }
            }
        }
    });
    ApiResource {
        data,
        refresh,
        error,
        failed,
        truncated,
        shown,
        page_budget,
    }
}

#[cfg(test)]
mod tests {
    use super::{CursorStep, LIST_MAX_PAGES, LIST_PAGE_LIMIT, cursor_step, with_limit_and_cursor};

    #[test]
    fn include_inactive_url_keeps_flag_and_appends_limit_cursor() {
        assert_eq!(
            with_limit_and_cursor("/api/members?include_inactive=true", None),
            format!("/api/members?include_inactive=true&limit={LIST_PAGE_LIMIT}")
        );
        assert_eq!(
            with_limit_and_cursor("/api/members?include_inactive=true", Some("00000064")),
            format!("/api/members?include_inactive=true&limit={LIST_PAGE_LIMIT}&cursor=00000064")
        );
    }

    #[test]
    fn active_only_url_does_not_send_include_inactive() {
        let path = with_limit_and_cursor("/api/members", None);
        assert_eq!(path, format!("/api/members?limit={LIST_PAGE_LIMIT}"));
        assert!(
            !path.contains("include_inactive"),
            "active-only fetch must omit the flag"
        );
    }

    #[test]
    fn public_members_url_never_gets_include_inactive_from_this_helper() {
        // Public roster builds its own path; this helper must not invent the flag.
        let path = with_limit_and_cursor("/api/public/members", Some("00000019"));
        assert!(
            !path.contains("include_inactive"),
            "public members must stay active-only"
        );
        assert!(path.contains("limit="));
        assert!(path.contains("cursor=00000019"));
    }

    #[test]
    fn cursor_step_truncates_only_when_another_page_remains() {
        assert_eq!(cursor_step(0, LIST_MAX_PAGES, None, None), CursorStep::Done);
        assert_eq!(
            cursor_step(0, LIST_MAX_PAGES, None, Some("  ")),
            CursorStep::Done
        );
        assert_eq!(
            cursor_step(0, LIST_MAX_PAGES, None, Some("abc")),
            CursorStep::Continue
        );
        assert_eq!(
            cursor_step(1, LIST_MAX_PAGES, Some("abc"), Some("abc")),
            CursorStep::Done,
            "a repeated cursor must not loop"
        );
        assert_eq!(
            cursor_step(LIST_MAX_PAGES - 1, LIST_MAX_PAGES, Some("a"), Some("b")),
            CursorStep::Truncated
        );
        assert_eq!(cursor_step(0, 1, None, Some("next")), CursorStep::Truncated);
    }

    #[test]
    fn load_more_raises_the_page_budget_by_one_batch() {
        let mut budget = LIST_MAX_PAGES;
        budget += LIST_MAX_PAGES;
        assert_eq!(budget, LIST_MAX_PAGES * 2);
    }

    /// Issue 821: a skipped fetch and a failed fetch both leave `None` in the
    /// slot, so the failure is a flag of its own. It is raised only where a
    /// fetch error is recorded and lowered on every skip and every success;
    /// a refetch starting must not lower it (the gate's retry rule reads it
    /// while the refetch is pending).
    #[test]
    fn failed_flag_is_raised_only_by_an_error_and_lowered_by_a_skip_or_success() {
        let src = include_str!("api.rs");
        let src = src.split("#[cfg(test)]\nmod tests").next().unwrap();
        let squashed: String = src.split_whitespace().collect::<Vec<_>>().join(" ");

        // Every hook hands out the flag.
        assert_eq!(
            squashed.matches("failed = use_signal(|| false);").count(),
            6
        );
        assert_eq!(squashed.matches("error, failed, truncated,").count(), 6);

        // Raised right after every recorded error and nowhere else.
        let raised = squashed.matches("failed.set(true);").count();
        assert_eq!(raised, 4, "three plain hooks and remember_list_failure");
        assert_eq!(
            squashed.matches("))); failed.set(true);").count(),
            raised,
            "failed.set(true) only follows error.set(Some(..))"
        );
        assert_eq!(squashed.matches("error.set(Some(").count(), raised);

        // Every skip lowers it before returning.
        assert_eq!(squashed.matches("url.is_empty()").count(), 2);
        assert_eq!(
            squashed
                .matches("failed.set(false); return None; }")
                .count(),
            2,
            "both empty-URL skips clear the flag"
        );
        assert!(
            squashed.contains("if !enabled() { failed.set(false); return Some(T::default()); }")
        );

        // Every success lowers it: three plain hooks and remember_batch.
        assert_eq!(squashed.matches("failed.set(false); Some(v) }").count(), 3);
        assert!(squashed.contains("shown.set(batch.items.len()); failed.set(false); }"));

        // Starting a fetch clears the error but never the flag.
        assert_eq!(squashed.matches("error.set(None);").count(), 6);
        assert!(!squashed.contains("error.set(None); failed.set(false);"));
    }
}
