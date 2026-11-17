use dioxus::prelude::*;

use crate::components::member_gate::Wall;
use crate::components::ui::{PageShell, PageWidth, btn_class};
use crate::routes::Route;

/// One link under the not-found message. The first link in a panel is the
/// primary button, the rest are ghost buttons.
#[derive(Clone, PartialEq)]
pub struct NotFoundLink {
    pub label: &'static str,
    pub to: Route,
}

/// The one not-found view: the wall card of the no-permission frame with
/// not-found copy. Used by the catch-all route and by pages whose record does
/// not exist (match, member, game), so a missing thing always reads the same
/// and always offers a next step. The title is the page's H1; the page that
/// embeds it supplies the container.
#[component]
pub fn NotFoundPanel(
    title: &'static str,
    message: &'static str,
    #[props(default)] path: String,
    links: Vec<NotFoundLink>,
) -> Element {
    rsx! {
        Wall {
            title,
            lead: message,
            page_title: true,
            kicker: "Error 404",
            path,
            actions: rsx! {
                for (i, link) in links.iter().enumerate() {
                    Link {
                        key: "{link.label}",
                        to: link.to.clone(),
                        class: btn_class(i == 0),
                        "{link.label}"
                    }
                }
            },
        }
    }
}

#[component]
pub fn NotFound(segments: Vec<String>) -> Element {
    let path = if segments.is_empty() {
        String::new()
    } else {
        format!("/{}", segments.join("/"))
    };

    // Anything deeper under /changelog offers both changelogs.
    if segments.first().map(String::as_str) == Some("changelog") {
        return rsx! { crate::pages::changelog::ChangelogNotFound { path } };
    }

    rsx! {
        // The no-permission frame this view copies is a narrow page.
        PageShell { width: PageWidth::Narrow, class: "nf-page",
            NotFoundPanel {
                title: "Page not found",
                message: "That page does not exist. It may have moved, or the link is wrong.",
                path,
                links: vec![
                    NotFoundLink { label: "Return home", to: Route::Home {} },
                    NotFoundLink { label: "Browse members", to: Route::Members {} },
                ],
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::components::member_gate::test_route::{render_route, wall};
    use scuffed_types::OrgRole;

    #[test]
    fn unknown_path_renders_not_found_inside_the_public_layout() {
        let html = render_route("/definitely/not/a/page", None);
        assert!(
            html.contains("class=\"ui-shell ui-shell--narrow nf-page\""),
            "{html}"
        );
        assert!(html.contains("Page not found"), "{html}");
        assert!(html.contains("/definitely/not/a/page"), "{html}");
        assert!(
            html.contains("d1-top"),
            "the 404 must keep the site nav: {html}"
        );
    }

    #[test]
    fn unknown_admin_and_strategy_paths_also_keep_the_nav() {
        for path in ["/admin/nope", "/strategy/nope"] {
            let html = render_route(path, None);
            assert!(html.contains("nf-page"), "{path}: {html}");
            assert!(html.contains("d1-top"), "{path}: {html}");
        }
    }

    #[test]
    fn not_found_is_one_h1_a_lead_and_two_links_in_the_wall_card() {
        let html = render_route("/definitely/not/a/page", None);
        assert_eq!(html.matches("<h1").count(), 1, "{html}");
        assert_eq!(html.matches("<main").count(), 1, "{html}");
        let wall = wall(&html);
        assert!(
            wall.contains("<p class=\"wall-kicker\">Error 404</p>"),
            "{wall}"
        );
        assert!(wall.contains("<h1>Page not found</h1>"), "{wall}");
        assert!(wall.contains("class=\"page-lead\""), "{wall}");
        assert!(
            wall.contains("<p class=\"wall-path\">/definitely/not/a/page</p>"),
            "{wall}"
        );
        let actions = &wall[wall.find("wall-actions").expect("actions")..];
        assert_eq!(actions.matches("<a ").count(), 2, "{actions}");
        assert!(!actions.contains("<button"), "{actions}");
        let home = actions.find(">Return home<").expect("home");
        let members = actions.find(">Browse members<").expect("members");
        assert!(home < members, "{actions}");
        assert!(actions.contains("href=\"/members\""), "{actions}");
        assert_eq!(actions.matches("is-primary").count(), 1, "{actions}");
        assert_eq!(actions.matches("btn is-ghost").count(), 1, "{actions}");
    }

    #[test]
    fn not_found_offers_no_sign_in_link_in_the_card() {
        for role in [None, Some(OrgRole::Member)] {
            let html = render_route("/definitely/not/a/page", role);
            assert!(!wall(&html).contains("href=\"/login\""), "{html}");
        }
    }
}
