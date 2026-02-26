//! Client-side auth state management for Leptos apps.

use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::UserInfo;
use super::api::{self, ApiError};

/// Client-side authentication state.
///
/// Wraps a reactive signal holding the current user. Provides helpers
/// to fetch the session from the server and to log out.
#[derive(Clone, Copy)]
pub struct AuthState {
    pub user: RwSignal<Option<UserInfo>>,
    pub loading: RwSignal<bool>,
}

impl AuthState {
    pub fn new() -> Self {
        Self {
            user: RwSignal::new(None),
            loading: RwSignal::new(false),
        }
    }

    /// Whether the user is currently authenticated.
    pub fn is_authenticated(&self) -> bool {
        self.user.get().is_some()
    }

    /// Fetch the current session from the server (`/api/auth/me`).
    ///
    /// Call this once on mount (e.g. from an Effect in your root component).
    /// Pass a custom endpoint if your app mounts auth routes elsewhere.
    pub fn fetch_session(&self, endpoint: &str) {
        let user_signal = self.user;
        let loading_signal = self.loading;
        let url = endpoint.to_string();

        spawn_local(async move {
            loading_signal.set(true);

            match api::fetch_json::<UserInfo>(&url).await {
                Ok(user) => user_signal.set(Some(user)),
                Err(ApiError::Unauthorized) => user_signal.set(None),
                Err(e) => {
                    leptos::logging::warn!("Failed to fetch session: {}", e);
                    user_signal.set(None);
                }
            }

            loading_signal.set(false);
        });
    }

    /// Log the user out by POSTing to the logout endpoint, then clear state.
    pub fn logout(&self, endpoint: &str) {
        let user_signal = self.user;
        let url = endpoint.to_string();

        spawn_local(async move {
            let _ = api::post_empty(&url, &()).await;
            user_signal.set(None);

            // Redirect to home
            if let Some(window) = web_sys::window() {
                let _ = window.location().set_href("/");
            }
        });
    }
}

impl Default for AuthState {
    fn default() -> Self {
        Self::new()
    }
}

/// Hook to access the AuthState from context.
pub fn use_auth() -> AuthState {
    expect_context::<AuthState>()
}
