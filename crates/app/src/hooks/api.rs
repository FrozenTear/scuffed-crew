use dioxus::prelude::*;
use scuffed_api_client::{ApiClient, ClientError};
use serde::Deserialize;
use serde::de::DeserializeOwned;

/// A resource that fetches data from an API endpoint with built-in refresh support.
#[derive(Clone, Copy)]
pub struct ApiResource<T: 'static> {
    pub data: Resource<Option<T>>,
    pub refresh: Signal<u64>,
    /// Last fetch error (None when ok or still loading).
    pub error: Signal<Option<String>>,
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
    let data = use_resource(move || async move {
        let _ = refresh();
        error.set(None);
        match ApiClient::web().fetch::<T>(url).await {
            Ok(v) => Some(v),
            Err(e) => {
                error.set(Some(e.to_string()));
                None
            }
        }
    });
    ApiResource {
        data,
        refresh,
        error,
    }
}

/// Wrapper for cursor-paginated list responses from the server.
#[derive(Deserialize)]
pub struct CursorPage<T> {
    pub data: Vec<T>,
    #[serde(default)]
    pub next_cursor: Option<String>,
}

/// Max pages to auto-follow for admin lists (limit=100 → up to 1000 rows).
const LIST_MAX_PAGES: usize = 10;
const LIST_PAGE_LIMIT: u32 = 100;

fn with_limit_and_cursor(url: &str, cursor: Option<&str>) -> String {
    let sep = if url.contains('?') { '&' } else { '?' };
    let mut path = format!("{url}{sep}limit={LIST_PAGE_LIMIT}");
    if let Some(c) = cursor {
        path.push_str(&format!("&cursor={c}"));
    }
    path
}

async fn fetch_all_pages_inner<T: DeserializeOwned>(base_url: &str) -> Result<Vec<T>, ClientError> {
    let mut all = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..LIST_MAX_PAGES {
        let path = with_limit_and_cursor(base_url, cursor.as_deref());
        let page = ApiClient::web().fetch::<CursorPage<T>>(&path).await?;
        all.extend(page.data);
        match page.next_cursor {
            Some(c) if !c.is_empty() => cursor = Some(c),
            _ => break,
        }
    }
    Ok(all)
}

async fn fetch_all_pages<T: DeserializeOwned>(base_url: &str) -> Result<Vec<T>, String> {
    fetch_all_pages_inner(base_url)
        .await
        .map_err(|e| e.to_string())
}

/// Try `preferred` first. On HTTP 403, fetch `fallback` and report `used_fallback`.
///
/// Used so officer Admin Members can request `include_inactive=true` and still
/// render if that contract (#52) is not merged yet or the caller is not officer.
async fn fetch_all_pages_or_forbidden_fallback<T: DeserializeOwned>(
    preferred: Option<&str>,
    fallback: &str,
) -> Result<(Vec<T>, bool), ClientError> {
    if let Some(url) = preferred {
        match fetch_all_pages_inner::<T>(url).await {
            Ok(items) => Ok((items, false)),
            Err(e) if e.is_forbidden() => {
                let items = fetch_all_pages_inner::<T>(fallback).await?;
                Ok((items, true))
            }
            Err(e) => Err(e),
        }
    } else {
        Ok((fetch_all_pages_inner::<T>(fallback).await?, false))
    }
}

/// Fetch a cursor-paginated list, auto-following pages (limit 100, max 10 pages).
pub fn use_api_list<T: DeserializeOwned + 'static>(url: &'static str) -> ApiResource<Vec<T>> {
    let refresh = use_signal(|| 0u64);
    let mut error = use_signal(|| Option::<String>::None);
    let data = use_resource(move || async move {
        let _ = refresh();
        error.set(None);
        match fetch_all_pages::<T>(url).await {
            Ok(v) => Some(v),
            Err(e) => {
                error.set(Some(e));
                None
            }
        }
    });
    ApiResource {
        data,
        refresh,
        error,
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
    let mut used_fallback = use_signal(|| false);
    let data = use_resource(move || {
        let preferred = preferred();
        async move {
            let _ = refresh();
            error.set(None);
            match fetch_all_pages_or_forbidden_fallback::<T>(preferred.as_deref(), fallback).await {
                Ok((v, fell_back)) => {
                    used_fallback.set(fell_back);
                    Some(v)
                }
                Err(e) => {
                    error.set(Some(e.to_string()));
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
    let data = use_resource(move || {
        let url = url();
        async move {
            let _ = refresh();
            error.set(None);
            if url.is_empty() {
                return None;
            }
            match fetch_all_pages::<T>(&url).await {
                Ok(v) => Some(v),
                Err(e) => {
                    error.set(Some(e));
                    None
                }
            }
        }
    });
    ApiResource {
        data,
        refresh,
        error,
    }
}

/// Fetch data from a dynamic API endpoint with automatic refresh support.
pub fn use_api_with<T: DeserializeOwned + 'static>(
    url: impl Fn() -> String + 'static,
) -> ApiResource<T> {
    let refresh = use_signal(|| 0u64);
    let mut error = use_signal(|| Option::<String>::None);
    let data = use_resource(move || {
        let url = url();
        async move {
            let _ = refresh();
            error.set(None);
            if url.is_empty() {
                return None;
            }
            match ApiClient::web().fetch::<T>(&url).await {
                Ok(v) => Some(v),
                Err(e) => {
                    error.set(Some(e.to_string()));
                    None
                }
            }
        }
    });
    ApiResource {
        data,
        refresh,
        error,
    }
}

#[cfg(test)]
mod tests {
    use super::{LIST_PAGE_LIMIT, with_limit_and_cursor};

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
}
