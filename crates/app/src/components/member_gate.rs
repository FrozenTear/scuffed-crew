//! Shared pending/auth UX for member-only public pages.
//!
//! `use_api*` stores failures as `None` data + `error`. Pages that only match
//! `None => "Loading…"` then hang forever on 401/403. Gate on auth first, and
//! treat 401 as sign-in / 403 as apply-or-inactive — never infinite Loading.

use dioxus::prelude::*;

use crate::hooks::ApiResource;
use crate::routes::Route;
use crate::state::auth::AuthState;

const GATE_CSS: &str = r#"
    .member-gate {
        text-align: center;
        padding: 3rem 1.5rem;
        max-width: 28rem;
        margin: 0 auto;
    }
    .member-gate h2 {
        font-family: var(--font-head);
        font-size: 1.35rem;
        color: var(--text);
        margin: 0 0 0.6rem;
    }
    .member-gate p {
        color: var(--text-2);
        font-size: 0.9rem;
        line-height: 1.5;
        margin: 0 0 1.25rem;
    }
    .member-gate-actions {
        display: flex;
        gap: 0.6rem;
        justify-content: center;
        flex-wrap: wrap;
    }
    .member-gate-status {
        color: var(--text-3);
        text-align: center;
        padding: 3rem 0;
    }
    .member-gate-error {
        color: var(--danger);
        text-align: center;
        padding: 2rem 1rem 0.75rem;
    }
"#;

/// True when a `use_api*` error string is that HTTP status (`ClientError` Display).
pub fn is_http_status(err: &str, status: u16) -> bool {
    err.contains(&format!("HTTP error {status}:")) || err.contains(&format!("HTTP error: {status}"))
}

fn sign_in_panel(what: &str) -> Element {
    rsx! {
        style { {GATE_CSS} }
        div { class: "member-gate",
            h2 { "Sign in required" }
            p { "Sign in to view {what}." }
            div { class: "member-gate-actions",
                Link { to: Route::Login {}, class: "ui-btn ui-btn--primary ui-btn--md", "Sign in" }
            }
        }
    }
}

fn apply_panel(what: &str) -> Element {
    rsx! {
        style { {GATE_CSS} }
        div { class: "member-gate",
            h2 { "Membership required" }
            p { "You need to be an org member to view {what}." }
            div { class: "member-gate-actions",
                Link { to: Route::Apply {}, class: "ui-btn ui-btn--primary ui-btn--md", "Apply" }
            }
        }
    }
}

fn inactive_panel(what: &str) -> Element {
    rsx! {
        style { {GATE_CSS} }
        div { class: "member-gate",
            h2 { "Membership inactive" }
            p { "Your membership isn't active, so {what} isn't available." }
        }
    }
}

/// Pending UI for a member-only `use_api*` whose `data` is still `None`.
/// Call from the `None` arm only — do not re-read `resource.data` here
/// (callers already hold that guard).
pub fn member_pending<T: 'static>(
    auth: &AuthState,
    resource: &ApiResource<T>,
    what: &str,
) -> Element {
    if !auth.loading && !auth.is_logged_in() {
        return sign_in_panel(what);
    }
    if !auth.loading && !auth.is_org_member() {
        return apply_panel(what);
    }

    if let Some(err) = resource.error.read().as_ref().cloned() {
        if is_http_status(&err, 401) {
            return sign_in_panel(what);
        }
        if is_http_status(&err, 403) {
            return if auth.is_org_member() {
                inactive_panel(what)
            } else {
                apply_panel(what)
            };
        }
        let mut refresh = resource.refresh;
        let what = what.to_string();
        return rsx! {
            style { {GATE_CSS} }
            p { class: "member-gate-error", "Failed to load {what}: {err}" }
            div { class: "member-gate-actions",
                button {
                    class: "ui-btn ui-btn--md",
                    onclick: move |_| refresh += 1,
                    "Retry"
                }
            }
        };
    }

    rsx! {
        style { {GATE_CSS} }
        p { class: "member-gate-status", "Loading {what}..." }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scuffed_types::{OrgRole, UserInfo};

    #[test]
    fn http_status_matches_client_error_display() {
        assert!(is_http_status("HTTP error 401: Unauthorized", 401));
        assert!(is_http_status("HTTP error: 401", 401));
        assert!(is_http_status("HTTP error 403: Not an org member", 403));
        assert!(!is_http_status("HTTP error 500: Internal error", 401));
        assert!(!is_http_status("Network error: failed", 401));
        // Must not treat 4010 / substring noise as 401
        assert!(!is_http_status("HTTP error 4010: weird", 401));
    }

    #[test]
    fn logged_out_is_not_org_member() {
        let a = AuthState {
            user: None,
            loading: false,
        };
        assert!(!a.is_logged_in());
        assert!(!a.is_org_member());
    }

    #[test]
    fn recruit_is_org_member() {
        let a = AuthState {
            user: Some(UserInfo {
                id: "u1".into(),
                username: "dev".into(),
                avatar_url: None,
                role: Some(OrgRole::Recruit),
            }),
            loading: false,
        };
        assert!(a.is_org_member());
    }
}
