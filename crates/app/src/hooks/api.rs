use dioxus::prelude::*;
use scuffed_api_client::ApiClient;
use serde::de::DeserializeOwned;

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
