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
    /* Every admin page brings its own `PageShell` (width, gutters, rhythm)
       and its one H1 in a `PageHeader`, so this wrapper adds no padding. */
    .admin-main {
        min-width: 0;
        /* Wide admin content never widens the page. `clip` (not `auto`)
           makes no scroll container, so a `position: sticky` bar inside
           (the settings save bar) sticks to the viewport. Wide tables scroll
           in their own `.data-table-scroll`. */
        overflow-x: clip;
    }
    /* A page body that is not in a `PageShell` still gets the page gutters
       and the wide column, so it never runs edge to edge. */
    .admin-main:not(:has(> .ui-shell)) {
        box-sizing: border-box;
        max-width: var(--content-max-wide);
        margin: 0 auto;
        padding: var(--space-section) var(--space-page);
    }
    /* Only a bare page's own title: `> h1`, so a page with its own header
       (Brand settings' `.customise-header h1`) is never matched and its gap
       does not depend on which stylesheet was injected first. */
    .admin-main:not(:has(> .ui-shell)) > h1 {
        margin: 0 0 var(--space-section);
        font-family: var(--font-head);
        font-weight: var(--font-head-weight);
        letter-spacing: var(--font-head-tracking);
        text-transform: var(--font-head-transform);
        line-height: var(--leading-display);
        font-size: var(--text-3xl);
        color: var(--text);
    }
    /* The sub-nav (base rule in layouts/shell.rs). On desktop it wraps, so
       no tab is ever cut off; on phone it scrolls, the current tab is
       scrolled into view, and an edge fades while there is more that way. */
    nav.admin-subnav { position: relative; }
    @media (min-width: 821px) {
        nav.admin-subnav { flex-wrap: wrap; overflow-x: visible; }
    }
    nav.admin-subnav.has-more-end {
        mask-image: linear-gradient(to right, black calc(100% - var(--space-10)), transparent);
    }
    nav.admin-subnav.has-more-start {
        mask-image: linear-gradient(to left, black calc(100% - var(--space-10)), transparent);
    }
    nav.admin-subnav.has-more-start.has-more-end {
        mask-image: linear-gradient(to right, transparent, black var(--space-10),
            black calc(100% - var(--space-10)), transparent);
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

    // Which ends of the sub-nav have more tabs past the edge (phone).
    let mut edges = use_signal(|| (false, false));
    let route_key = route.to_string();
    use_effect(use_reactive((&route_key, &allowed), move |(_, _)| {
        reveal_active_subnav_tab();
        edges.set(subnav_edges());
    }));
    let (more_start, more_end) = edges();
    let edge_class = match (more_start, more_end) {
        (true, true) => " has-more-start has-more-end",
        (true, false) => " has-more-start",
        (false, true) => " has-more-end",
        (false, false) => "",
    };

    // The sub-nav is only for people who can open the pages it lists.
    let subnav = (!session.loading && allowed).then(|| {
        rsx! {
            // tabindex -1: Firefox makes a scrolling box a Tab stop of its
            // own. Its links still scroll it when they take focus.
            nav {
                id: SUBNAV_ID,
                class: "admin-subnav{edge_class}",
                aria_label: "Admin",
                tabindex: "-1",
                onscroll: move |_| edges.set(subnav_edges()),
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

const SUBNAV_ID: &str = "admin-subnav";

/// Scroll the sub-nav sideways (only sideways) so the current tab is in view.
fn reveal_active_subnav_tab() {
    #[cfg(target_arch = "wasm32")]
    {
        let Some(nav) = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.get_element_by_id(SUBNAV_ID))
        else {
            return;
        };
        let Some(link) = nav.query_selector("a.is-active").ok().flatten() else {
            return;
        };
        let (nav_box, link_box) = (
            nav.get_bounding_client_rect(),
            link.get_bounding_client_rect(),
        );
        let current = f64::from(nav.scroll_left());
        // Where the tab starts and ends inside the scrolled strip.
        let start = link_box.left() - nav_box.left() + current;
        let end = start + link_box.width();
        let view = f64::from(nav.client_width());
        let margin = view / 4.0;
        let next = if start < current {
            (start - margin).max(0.0)
        } else if end > current + view {
            end - view + margin
        } else {
            return;
        };
        nav.set_scroll_left(next.round() as i32);
    }
}

/// (more tabs before the left edge, more after the right edge).
fn subnav_edges() -> (bool, bool) {
    #[cfg(target_arch = "wasm32")]
    {
        if let Some(nav) = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.get_element_by_id(SUBNAV_ID))
        {
            let left = nav.scroll_left();
            let room = nav.scroll_width() - nav.client_width();
            return (left > 1, room - left > 1);
        }
    }
    (false, false)
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
        "Admin is limited to officers and admins. Log in with an account that has that role."
            .to_string()
    };
    // The tab says what the page shows, not the admin page that was asked for.
    crate::head::use_page_title(Some("No access".to_string()));
    rsx! {
        PageShell { width: PageWidth::Narrow, class: "admin-no-access",
            Wall {
                // A guest is not denied, just not signed in yet.
                title: if signed_in { "You do not have access".to_string() } else { "Log in to open Admin".to_string() },
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
                            "Log in"
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
        assert!(wall.contains("<h1>Log in to open Admin</h1>"), "{wall}");
        assert!(!wall.contains("You do not have access"), "{wall}");
        assert!(!wall.contains("signed in as"), "{wall}");
        let actions = &wall[wall.find("wall-actions").expect("actions")..];
        let sign_in = actions.find(">Log in<").expect("sign in");
        let home = actions.find(">Return home<").expect("home");
        assert!(sign_in < home, "{actions}");
        assert!(actions.contains("href=\"/login\""), "{actions}");
        assert!(!actions.contains(">Open your profile<"), "{actions}");
        assert_eq!(actions.matches("is-primary").count(), 1, "{actions}");
    }

    #[test]
    fn admin_chrome_is_on_tokens_and_leaves_the_h1_to_the_page_header() {
        use crate::components::ui::test_support::{assert_sizes_from_tokens, css_rule, has_decl};
        assert_sizes_from_tokens(super::ADMIN_CSS);
        // No padding of its own: the page's `PageShell` owns the gutters.
        let main = css_rule(super::ADMIN_CSS, ".admin-main {");
        assert!(!main.contains("padding"), "{main}");
        // The old uppercase H1 rule is gone; the fallback follows the head
        // font's own transform token.
        assert!(!super::ADMIN_CSS.contains("text-transform: uppercase"));
        let bare = css_rule(super::ADMIN_CSS, ".admin-main:not(:has(> .ui-shell)) {");
        for decl in [
            "max-width: var(--content-max-wide)",
            "padding: var(--space-section) var(--space-page)",
        ] {
            assert!(has_decl(&bare, decl), "{decl}: {bare}");
        }
        // The fallback title rule reaches only a direct child h1. A
        // descendant selector ties with `.customise .customise-header h1`
        // (Brand settings) and the winner was load order (Vision nit 2 on
        // PR 706).
        assert!(
            super::ADMIN_CSS.contains(".admin-main:not(:has(> .ui-shell)) > h1 {"),
            "{}",
            super::ADMIN_CSS
        );
        assert!(!super::ADMIN_CSS.contains(".admin-main:not(:has(> .ui-shell)) h1"));
    }

    #[test]
    fn officers_get_the_admin_pages_not_the_wall() {
        let html = render_route("/admin", Some(OrgRole::Officer));
        assert!(html.contains("class=\"admin-main\""), "{html}");
        assert!(!html.contains("admin-no-access"), "{html}");
        assert!(!html.contains("You do not have access"), "{html}");
    }
}
