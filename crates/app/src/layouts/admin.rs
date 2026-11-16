use dioxus::prelude::*;

use super::nav::{NavContext, admin_entries, aria_current, is_current};
use super::shell::SiteShell;
use crate::routes::Route;
use crate::state::use_auth;

const ADMIN_CSS: &str = r#"
    .admin-layout {
        background: var(--bg);
        color: var(--text);
        font-family: var(--font-body);
    }
    .admin-main {
        min-width: 0;
        padding: 2rem 2.5rem;
        /* Wide admin content scrolls inside the admin area and never widens
           the page. `overflow-y: auto` makes `overflow-x` compute to auto too. */
        overflow-y: auto;
    }
    .admin-main h1 {
        font-family: var(--font-head);
        font-size: 1.8rem;
        color: var(--text);
        margin-bottom: 1.5rem;
        text-transform: uppercase;
        letter-spacing: 0.04em;
    }
    .admin-gate {
        display: flex;
        flex-direction: column;
        align-items: center;
        justify-content: center;
        gap: var(--space-2);
        min-height: 50vh;
        text-align: center;
        color: var(--text-3);
    }
    .admin-gate p {
        margin: 0;
        max-width: var(--content-max-narrow);
    }
    .admin-gate-actions {
        display: flex;
        flex-wrap: wrap;
        justify-content: center;
        gap: var(--space-2) var(--space-4);
        margin-top: var(--space-3);
    }
    .admin-gate-actions a {
        display: inline-flex;
        align-items: center;
        min-height: var(--hit-target-min);
        color: var(--accent-text);
    }
    @media (max-width: 768px) {
        .admin-main {
            padding: 1rem;
        }
        .admin-main h1 { font-size: 1.35rem; margin-bottom: 1rem; }
    }
"#;

/// Admin pages keep the site shell and add a horizontal sub-nav under the
/// top bar. Officers and admins get in; everyone else sees why not, with the
/// site nav still around them.
#[component]
pub fn AdminLayout() -> Element {
    let auth = use_auth();
    let route = use_route::<Route>();
    // A signed-out visit comes back to this page after sign-in.
    let return_path = route.to_string();

    let session = auth();
    let signed_in = session.is_logged_in();
    let allowed = session.is_officer_or_above();
    let username = session
        .user
        .as_ref()
        .map(|u| u.username.clone())
        .unwrap_or_default();
    let entries = admin_entries(NavContext {
        signed_in,
        admin_access: allowed,
        is_admin: session.is_admin(),
        is_member: session.is_org_member(),
        strategies_enabled: true,
        relay_on: true,
    });

    // The sub-nav is only for people who can open the pages it lists.
    let subnav = (!session.loading && allowed).then(|| {
        rsx! {
            // tabindex -1: Firefox makes a scrolling box a Tab stop of its
            // own. Its links still scroll it when they take focus.
            nav { class: "admin-subnav", aria_label: "Admin", tabindex: "-1",
                for entry in entries.iter() {
                    Link {
                        key: "{entry.id}",
                        to: entry.route.clone(),
                        class: if is_current(&entry.route, &route) { "is-active" } else { "" },
                        "aria-current": aria_current(&entry.route, &route),
                        "{entry.label}"
                    }
                }
            }
        }
    });

    rsx! {
        crate::styles::StyleOnce { id: "scuffed-css-layouts-admin-admin-css", css: ADMIN_CSS }
        crate::styles::StyleOnce { id: "scuffed-css-admin-shared", css: crate::styles::admin::CSS }
        SiteShell { subnav,
            div { class: "admin-layout",
                div { class: "admin-main",
                    if session.loading {
                        // Wait for /api/auth/me so a refresh does not flash the no-access page.
                        div { class: "admin-gate",
                            p { "Checking session…" }
                        }
                    } else if !allowed {
                        div { class: "admin-gate",
                            h1 { "You do not have access" }
                            if signed_in {
                                p {
                                    "Admin is limited to officers and admins. You are signed in as {username}, and this page is not part of your role."
                                }
                            } else {
                                p { "Admin is limited to officers and admins. Sign in with an account that has that role." }
                            }
                            div { class: "admin-gate-actions",
                                Link { to: Route::Home {}, "Return home" }
                                if signed_in {
                                    Link { to: Route::ProfileSettings {}, "Open your profile" }
                                } else {
                                    Link {
                                        to: Route::Login {},
                                        onclick: move |_| crate::pages::remember_login_return(&return_path),
                                        "Sign in"
                                    }
                                    if cfg!(debug_assertions) {
                                        a { href: "/api/dev/login", "Dev login (in-memory only)" }
                                    }
                                }
                            }
                        }
                    } else {
                        Outlet::<Route> {}
                    }
                }
            }
        }
    }
}
