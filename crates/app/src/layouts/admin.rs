use dioxus::prelude::*;

use super::nav::{NavContext, admin_entries, aria_current, is_current};
use super::shell::SiteShell;
use crate::components::member_gate::Wall;
use crate::components::ui::{PageShell, PageWidth, btn_class};
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
    /* The session check. It sits outside `.admin-main`. */
    .admin-gate {
        display: flex;
        align-items: center;
        justify-content: center;
        min-height: 50vh;
        margin: 0;
        padding: var(--space-section) var(--space-page);
        text-align: center;
        color: var(--text-3);
        font-size: var(--text-md);
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
        is_recruit: session.is_recruit(),
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
            {admin_body(session.loading, allowed, signed_in, username, return_path)}
        }
    }
}

/// Text of the session status line. It is always rendered; only this changes.
fn gate_status(loading: bool) -> &'static str {
    if loading { "Checking session" } else { "" }
}

/// What sits inside the site shell: the session check, the no-access card, or
/// the admin pages. The status line is one node in every state, so the wait
/// and its end are announced as a text change. It is the visible content while
/// the session loads and only speaks afterwards.
fn admin_body(
    loading: bool,
    allowed: bool,
    signed_in: bool,
    username: String,
    return_path: String,
) -> Element {
    let status = gate_status(loading);
    rsx! {
        if loading {
            // Wait for /api/auth/me so a refresh does not flash the no-access
            // page. The page has no title yet, so it is named "Admin".
            h1 { class: "visually-hidden", "Admin" }
        }
        p {
            class: if loading { "admin-gate" } else { "visually-hidden" },
            role: "status",
            "{status}"
        }
        if !loading {
            if !allowed {
                AdminNoAccess { signed_in, username, return_path }
            } else {
                div { class: "admin-layout",
                    div { class: "admin-main", Outlet::<Route> {} }
                }
            }
        }
    }
}

/// The no-access state (`d1-no-permission` frame): the wall card in the
/// narrow page container, as the frame draws it. A signed-in member is never sent to sign in; a
/// guest gets the sign-in link, and it returns to this page afterwards.
#[component]
fn AdminNoAccess(signed_in: bool, username: String, return_path: String) -> Element {
    let lead = if signed_in {
        format!(
            "Admin is limited to officers and admins. You are signed in as {username}, and this page is not part of your role."
        )
    } else {
        "Admin is limited to officers and admins. Sign in with an account that has that role."
            .to_string()
    };
    rsx! {
        PageShell { width: PageWidth::Narrow, class: "admin-no-access",
            Wall {
                title: "You do not have access",
                lead,
                page_title: true,
                actions: rsx! {
                    if signed_in {
                        Link { to: Route::Home {}, class: btn_class(true), "Return home" }
                        Link {
                            to: Route::ProfileSettings {},
                            class: btn_class(false),
                            "Open your profile"
                        }
                    } else {
                        Link {
                            to: Route::Login {},
                            class: btn_class(true),
                            onclick: move |_| crate::pages::remember_login_return(&return_path),
                            "Sign in"
                        }
                        Link { to: Route::Home {}, class: btn_class(false), "Return home" }
                        if cfg!(debug_assertions) {
                            a { class: btn_class(false), href: "/api/dev/login",
                                "Dev login (in-memory only)"
                            }
                        }
                    }
                },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::components::member_gate::test_route::{render_route, wall};
    use scuffed_types::OrgRole;

    #[test]
    fn member_without_access_gets_the_wall_card_and_no_sign_in_link() {
        let html = render_route("/admin", Some(OrgRole::Member));
        assert!(
            html.contains("class=\"ui-shell ui-shell--narrow admin-no-access\""),
            "{html}"
        );
        assert_eq!(html.matches("<h1").count(), 1, "{html}");
        assert_eq!(html.matches("<main").count(), 1, "{html}");
        assert!(!html.contains("class=\"admin-main\""), "{html}");
        let wall = wall(&html);
        assert!(wall.contains("<h1>You do not have access</h1>"), "{wall}");
        assert!(wall.contains("signed in as Player01"), "{wall}");
        assert!(!wall.contains("href=\"/login\""), "{wall}");
        assert!(!wall.contains("/api/dev/login"), "{wall}");
        let actions = &wall[wall.find("wall-actions").expect("actions")..];
        assert_eq!(actions.matches("<a ").count(), 2, "{actions}");
        assert!(!actions.contains("<button"), "{actions}");
        let home = actions.find(">Return home<").expect("home");
        let profile = actions.find(">Open your profile<").expect("profile");
        assert!(home < profile, "{actions}");
        assert_eq!(actions.matches("is-primary").count(), 1, "{actions}");
        // The session status line stays in the tree, with nothing to say.
        assert!(
            html.contains("<p class=\"visually-hidden\" role=\"status\"></p>"),
            "{html}"
        );
    }

    #[test]
    fn session_check_has_an_h1_and_a_status_line_outside_admin_main() {
        fn loading() -> dioxus::prelude::Element {
            super::admin_body(true, false, false, String::new(), "/admin".to_string())
        }
        let mut dom = dioxus::prelude::VirtualDom::new(loading);
        dom.rebuild_in_place();
        let html = dioxus_ssr::render(&dom);
        assert_eq!(
            html,
            "<h1 class=\"visually-hidden\">Admin</h1>\
             <p class=\"admin-gate\" role=\"status\">Checking session</p>"
        );
        assert_eq!(super::gate_status(false), "");
    }

    #[test]
    fn guest_gets_sign_in_first_then_return_home() {
        let html = render_route("/admin", None);
        assert_eq!(html.matches("<h1").count(), 1, "{html}");
        let wall = wall(&html);
        assert!(!wall.contains("signed in as"), "{wall}");
        let actions = &wall[wall.find("wall-actions").expect("actions")..];
        let sign_in = actions.find(">Sign in<").expect("sign in");
        let home = actions.find(">Return home<").expect("home");
        assert!(sign_in < home, "{actions}");
        assert!(actions.contains("href=\"/login\""), "{actions}");
        assert!(!actions.contains(">Open your profile<"), "{actions}");
        assert_eq!(actions.matches("is-primary").count(), 1, "{actions}");
    }

    #[test]
    fn officers_get_the_admin_pages_not_the_wall() {
        let html = render_route("/admin", Some(OrgRole::Officer));
        assert!(html.contains("class=\"admin-main\""), "{html}");
        assert!(!html.contains("admin-no-access"), "{html}");
        assert!(!html.contains("You do not have access"), "{html}");
    }
}
