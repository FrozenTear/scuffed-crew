use leptos::prelude::*;
use serde::Deserialize;

use scuffed_auth::UserInfo;

use crate::api;

/// Mirrors the MeResponse from the server.
#[derive(Debug, Clone, Deserialize)]
pub struct MeResponse {
    pub user: UserInfo,
    pub member: Option<MemberInfo>,
}

/// Minimal member info for the admin panel.
#[derive(Debug, Clone, Deserialize)]
pub struct MemberInfo {
    pub id: String,
    pub org_role: String,
    pub display_name: String,
}

/// Admin auth state — provides user/member info and role checks.
#[derive(Clone, Copy)]
pub struct AdminState {
    pub me: RwSignal<Option<MeResponse>>,
    pub loading: RwSignal<bool>,
    pub error: RwSignal<Option<String>>,
}

impl AdminState {
    pub fn new() -> Self {
        Self {
            me: RwSignal::new(None),
            loading: RwSignal::new(true),
            error: RwSignal::new(None),
        }
    }

    pub fn fetch_me(&self) {
        let me = self.me;
        let loading = self.loading;
        let error = self.error;

        leptos::task::spawn_local(async move {
            loading.set(true);
            match api::get::<MeResponse>("/api/auth/me").await {
                Ok(resp) => {
                    me.set(Some(resp));
                    error.set(None);
                }
                Err(e) => {
                    me.set(None);
                    error.set(Some(e.to_string()));
                }
            }
            loading.set(false);
        });
    }

    pub fn is_authenticated(&self) -> bool {
        self.me.get().is_some()
    }

    pub fn is_officer_or_above(&self) -> bool {
        self.me
            .get()
            .and_then(|m| m.member)
            .map(|m| matches!(m.org_role.as_str(), "admin" | "officer"))
            .unwrap_or(false)
    }

    pub fn is_admin(&self) -> bool {
        self.me
            .get()
            .and_then(|m| m.member)
            .map(|m| m.org_role == "admin")
            .unwrap_or(false)
    }
}

pub fn use_admin_state() -> AdminState {
    expect_context::<AdminState>()
}
