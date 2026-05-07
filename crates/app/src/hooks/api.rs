use dioxus::prelude::*;
use scuffed_api_client::ApiClient;
use serde::de::DeserializeOwned;
use serde::Deserialize;

/// A resource that fetches data from an API endpoint with built-in refresh support.
#[derive(Clone, Copy)]
pub struct ApiResource<T: 'static> {
    pub data: Resource<Option<T>>,
    pub refresh: Signal<u64>,
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
    let data = use_resource(move || async move {
        let _ = refresh();
        ApiClient::web().fetch::<T>(url).await.ok()
    });
    ApiResource { data, refresh }
}

/// Wrapper for cursor-paginated list responses from the server.
#[derive(Deserialize)]
pub struct CursorPage<T> {
    pub data: Vec<T>,
}

/// Fetch a cursor-paginated list from a static API endpoint, auto-unwrapping the wrapper.
pub fn use_api_list<T: DeserializeOwned + 'static>(url: &'static str) -> ApiResource<Vec<T>> {
    let refresh = use_signal(|| 0u64);
    let data = use_resource(move || async move {
        let _ = refresh();
        ApiClient::web()
            .fetch::<CursorPage<T>>(url)
            .await
            .ok()
            .map(|r| r.data)
    });
    ApiResource { data, refresh }
}

/// Fetch a cursor-paginated list from a dynamic API endpoint, auto-unwrapping the wrapper.
pub fn use_api_list_with<T: DeserializeOwned + 'static>(
    url: impl Fn() -> String + 'static,
) -> ApiResource<Vec<T>> {
    let refresh = use_signal(|| 0u64);
    let data = use_resource(move || {
        let url = url();
        async move {
            let _ = refresh();
            if url.is_empty() {
                return None;
            }
            ApiClient::web()
                .fetch::<CursorPage<T>>(&url)
                .await
                .ok()
                .map(|r| r.data)
        }
    });
    ApiResource { data, refresh }
}

/// Fetch data from a dynamic API endpoint with automatic refresh support.
pub fn use_api_with<T: DeserializeOwned + 'static>(
    url: impl Fn() -> String + 'static,
) -> ApiResource<T> {
    let refresh = use_signal(|| 0u64);
    let data = use_resource(move || {
        let url = url();
        async move {
            let _ = refresh();
            if url.is_empty() {
                return None;
            }
            ApiClient::web().fetch::<T>(&url).await.ok()
        }
    });
    ApiResource { data, refresh }
}
