use leptos::prelude::*;

#[cfg(not(feature = "dev-noauth"))]
use crate::state::use_admin_state;

/// Wrapper component that only renders children if the user is an officer or admin.
/// In dev builds with `dev-noauth` feature, auth is bypassed entirely.
#[component]
pub fn RequireOfficer(children: Children) -> impl IntoView {
    #[cfg(feature = "dev-noauth")]
    {
        view! {
            <div style:display="contents">
                {children()}
            </div>
        }
        .into_any()
    }

    #[cfg(not(feature = "dev-noauth"))]
    {
        let state = use_admin_state();

        let authorized = move || {
            !state.loading.get() && state.is_authenticated() && state.is_officer_or_above()
        };

        view! {
            {move || {
                if state.loading.get() {
                    Some(view! {
                        <div class="admin-loading">
                            <p>"Loading..."</p>
                        </div>
                    }.into_any())
                } else if !state.is_authenticated() {
                    Some(view! {
                        <div class="admin-denied">
                            <h2>"Not Authenticated"</h2>
                            <p>"Please "<a href="/api/auth/discord/login">"log in"</a>" first."</p>
                        </div>
                    }.into_any())
                } else if !state.is_officer_or_above() {
                    Some(view! {
                        <div class="admin-denied">
                            <h2>"Access Denied"</h2>
                            <p>"You need officer or admin privileges to access this panel."</p>
                        </div>
                    }.into_any())
                } else {
                    None
                }
            }}
            <div style:display=move || if authorized() { "contents" } else { "none" }>
                {children()}
            </div>
        }
        .into_any()
    }
}
