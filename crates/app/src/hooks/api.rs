use dioxus::prelude::*;
use scuffed_api_client::ApiClient;
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

async fn fetch_all_pages<T: DeserializeOwned>(base_url: &str) -> Result<Vec<T>, String> {
    let mut all = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..LIST_MAX_PAGES {
        let path = with_limit_and_cursor(base_url, cursor.as_deref());
        let page = ApiClient::web()
            .fetch::<CursorPage<T>>(&path)
            .await
            .map_err(|e| e.to_string())?;
        all.extend(page.data);
        match page.next_cursor {
            Some(c) if !c.is_empty() => cursor = Some(c),
            _ => break,
        }
    }
    Ok(all)
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
