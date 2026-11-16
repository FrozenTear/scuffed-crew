//! Member appearance settings (`/settings`): theme and density only.
//!
//! Accent and display font belong to admins (`/admin/brand`), so this page
//! has no control for either. Both choices persist per browser, the way the
//! theme toggle always has, and reach the root element through
//! [`ThemeCtx`].

use dioxus::prelude::*;

use crate::routes::Route;
use crate::state::use_auth;
use crate::styles::customise::{CUSTOMISE_CSS, CUSTOMISE_CSS_ID};
use crate::theme::{Density, ThemeCtx, ThemeMode};

const THEMES: [ThemeMode; 2] = [ThemeMode::Dark, ThemeMode::Light];

/// Where density reaches today. Only this page and admin brand settings read
/// the density tokens, so the page says so until the rest of the site is
/// ported. "Settings" alone would point members at Admin Settings, which does
/// not read them.
const DENSITY_SCOPE_NOTE: &str = "Density changes this page and brand settings for now. The rest of the site follows as each page is redesigned.";

/// The line under Save. `saved` is what the root element has now.
fn status_text(
    draft: (ThemeMode, Density),
    saved: (ThemeMode, Density),
    just_saved: bool,
) -> &'static str {
    if draft != saved {
        "You have unsaved changes."
    } else if just_saved {
        "Appearance saved on this device."
    } else {
        ""
    }
}

#[component]
pub fn AppearanceSettings() -> Element {
    let auth = use_auth();
    let mut ctx = use_context::<ThemeCtx>();
    let mut theme = use_signal(|| *ctx.mode.peek());
    let mut density = use_signal(|| *ctx.density.peek());
    let mut just_saved = use_signal(|| false);
    // The top bar toggle also writes the shared theme. Follow it, so this
    // page never reports its own stale draft as an unsaved change.
    use_effect(move || theme.set((ctx.mode)()));
    use_effect(move || density.set((ctx.density)()));

    let auth_now = auth();
    // While the session is still loading the page shows neither the account
    // links nor the sign-in hint, so a signed-out visitor never gets a flash
    // of links they cannot use.
    let signed_in = auth_now.is_logged_in();
    let signed_out = !auth_now.loading && !signed_in;
    // The account uses its own id so two accounts on one page never share
    // radio groups or label targets.
    let owner = auth_now
        .user
        .as_ref()
        .map(|user| user.id.clone())
        .unwrap_or_else(|| "guest".to_string());
    let owner: String = owner
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let theme_title_id = format!("{owner}-appearance-theme-title");
    let density_title_id = format!("{owner}-appearance-density-title");
    let density_note_id = format!("{owner}-appearance-density-note");
    let theme_name = format!("{owner}-appearance-theme");
    let density_name = format!("{owner}-appearance-density");
    let status = status_text(
        (theme(), density()),
        ((ctx.mode)(), (ctx.density)()),
        just_saved(),
    );

    rsx! {
        crate::styles::StyleOnce { id: CUSTOMISE_CSS_ID, css: CUSTOMISE_CSS }
        // No `main` here: the public shell already wraps the page in one.
        div { class: "customise is-page",
            header { class: "customise-header",
                h1 { "Appearance" }
                p { class: "customise-lead",
                    "You can change theme and density. Accent and display font are set by an admin for the whole crew."
                }
            }
            section { class: "customise-card", "aria-labelledby": "{theme_title_id}",
                p { class: "customise-kicker", "Theme" }
                h2 { class: "customise-title", id: "{theme_title_id}", "Color theme" }
                div {
                    class: "customise-choice-row",
                    role: "radiogroup",
                    "aria-labelledby": "{theme_title_id}",
                    for mode in THEMES {
                        label {
                            key: "{mode.as_attr()}",
                            class: if theme() == mode { "customise-choice is-selected" } else { "customise-choice" },
                            input {
                                r#type: "radio",
                                name: "{theme_name}",
                                value: "{mode.as_attr()}",
                                checked: theme() == mode,
                                onchange: move |_| {
                                    theme.set(mode);
                                    just_saved.set(false);
                                },
                            }
                            span { class: "customise-choice-mark", "aria-hidden": "true" }
                            "{mode.label()}"
                        }
                    }
                }
            }
            section { class: "customise-card", "aria-labelledby": "{density_title_id}",
                p { class: "customise-kicker", "Density" }
                h2 { class: "customise-title", id: "{density_title_id}", "Spacing" }
                p { class: "customise-hint", id: "{density_note_id}", "{DENSITY_SCOPE_NOTE}" }
                div {
                    class: "customise-choice-row",
                    role: "radiogroup",
                    "aria-labelledby": "{density_title_id}",
                    "aria-describedby": "{density_note_id}",
                    for option in Density::ALL {
                        label {
                            key: "{option.as_attr()}",
                            class: if density() == option { "customise-choice is-selected" } else { "customise-choice" },
                            input {
                                r#type: "radio",
                                name: "{density_name}",
                                value: "{option.as_attr()}",
                                checked: density() == option,
                                onchange: move |_| {
                                    density.set(option);
                                    just_saved.set(false);
                                },
                            }
                            span { class: "customise-choice-mark", "aria-hidden": "true" }
                            "{option.label()}"
                        }
                    }
                }
            }
            div { class: "customise-actions",
                button {
                    class: "customise-btn is-primary",
                    r#type: "button",
                    onclick: move |_| {
                        // `ThemeProvider` writes both root attributes and
                        // persists them when these signals change.
                        ctx.mode.set(theme());
                        ctx.density.set(density());
                        just_saved.set(true);
                    },
                    "Save appearance"
                }
            }
            // Always rendered; only the text changes.
            p { class: "customise-status", role: "status", "{status}" }
            if signed_out {
                p { class: "customise-hint",
                    "Sign in to reach your profile, account keys and reports."
                }
            } else if signed_in {
                nav { "aria-label": "Account pages",
                    ul { class: "customise-links",
                        li { Link { to: Route::ProfileSettings {}, "Edit your profile" } }
                        li { Link { to: Route::IdentitySettings {}, "Account keys" } }
                        li { Link { to: Route::MyReports {}, "My reports" } }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::AuthState;
    use scuffed_types::{OrgRole, UserInfo};

    /// `/settings` through the real router, so the account links resolve.
    /// Returns the page without the public chrome around it.
    fn render(role: Option<OrgRole>, signed_in: bool) -> String {
        render_with(role, signed_in, false)
    }

    fn render_with(role: Option<OrgRole>, signed_in: bool, loading: bool) -> String {
        let html = render_document(role, signed_in, loading);
        let start = html
            .find("<div class=\"customise is-page\"")
            .unwrap_or_else(|| panic!("appearance page: {html}"));
        // The page is the last thing in the shell's `main`.
        let end = start + html[start..].find("</main>").expect("end of page");
        html[start..end].to_string()
    }

    /// The whole document: the public shell with the page in it.
    fn render_document(role: Option<OrgRole>, signed_in: bool, loading: bool) -> String {
        #[component]
        fn Harness(role: Option<OrgRole>, signed_in: bool, loading: bool) -> Element {
            let auth = use_signal(|| AuthState {
                user: signed_in.then(|| UserInfo {
                    id: "u1".into(),
                    username: "Player01".into(),
                    avatar_url: None,
                    role,
                }),
                loading,
            });
            use_context_provider(|| auth);
            crate::state::provide_site_settings();
            let history = use_hook(|| {
                std::rc::Rc::new(dioxus::history::MemoryHistory::with_initial_path(
                    "/settings",
                ))
            });
            rsx! {
                crate::theme::ThemeProvider {
                    dioxus::router::components::HistoryProvider {
                        history: move |_| history.clone() as std::rc::Rc<dyn dioxus::history::History>,
                        Router::<Route> {}
                    }
                }
            }
        }
        let mut dom = VirtualDom::new_with_props(
            Harness,
            HarnessProps {
                role,
                signed_in,
                loading,
            },
        );
        dom.rebuild_in_place();
        dioxus_ssr::render(&dom)
    }

    /// The public shell provides the one `main` landmark. The page must not
    /// nest a second one inside it.
    #[test]
    fn the_page_adds_no_second_main_landmark() {
        for (role, signed_in) in [(None, false), (Some(OrgRole::Member), true)] {
            let html = render_document(role, signed_in, false);
            assert_eq!(html.matches("<main").count(), 1, "{html}");
            assert_eq!(html.matches("role=\"main\"").count(), 0, "{html}");
            let main = &html[html.find("<main").expect("shell main")..];
            let main = &main[..main.find("</main>").expect("end of main")];
            assert!(main.contains("<div class=\"customise is-page\""), "{main}");
            assert_eq!(main.matches("<h1").count(), 1, "{main}");
        }
    }

    #[test]
    fn appearance_offers_theme_and_density_only() {
        for role in [
            Some(OrgRole::Member),
            Some(OrgRole::Recruit),
            Some(OrgRole::Admin),
        ] {
            let html = render(role, true);
            assert_eq!(html.matches("<h1").count(), 1, "{html}");
            assert_eq!(html.matches("role=\"radiogroup\"").count(), 2, "{html}");
            assert_eq!(html.matches("type=\"radio\"").count(), 5, "{html}");
            for value in ["dark", "light", "compact", "default", "comfortable"] {
                assert!(html.contains(&format!("value=\"{value}\"")), "{value}");
            }
            // No admin brand controls here, for any role.
            for control in [
                "type=\"color\"",
                "type=\"text\"",
                "Accent hex",
                "Display font",
                "Save brand",
                "Urbanist",
            ] {
                assert!(!html.contains(control), "{role:?} sees {control}");
            }
            assert!(html.contains("Save appearance"), "{html}");
            assert!(html.contains("href=\"/profile/edit\""), "{html}");
            assert!(html.contains("href=\"/identity\""), "{html}");
            assert!(html.contains("href=\"/reports\""), "{html}");
        }
    }

    #[test]
    fn the_defaults_are_selected_and_the_status_region_is_empty() {
        let html = render(Some(OrgRole::Member), true);
        assert_eq!(html.matches("customise-choice is-selected").count(), 2);
        // Off wasm there is no stored choice: dark theme, default density.
        for selected in ["Dark", "Default"] {
            let at = html.find(&format!("{selected}</label>")).expect(selected);
            let label = &html[html[..at].rfind("<label").unwrap()..at];
            assert!(label.contains("is-selected"), "{label}");
        }
        assert!(
            html.contains("class=\"customise-status\" role=\"status\"></p>"),
            "{html}"
        );
        assert!(
            html.contains("id=\"u1-appearance-density-title\""),
            "{html}"
        );
        assert!(
            html.contains("aria-labelledby=\"u1-appearance-density-title\""),
            "{html}"
        );
    }

    const ACCOUNT_LINKS: [&str; 3] = [
        "href=\"/profile/edit\"",
        "href=\"/identity\"",
        "href=\"/reports\"",
    ];
    const SIGN_IN_HINT: &str = "Sign in to reach your profile";

    #[test]
    fn signed_out_visitors_get_no_account_links() {
        let html = render(None, false);
        for link in ACCOUNT_LINKS {
            assert!(!html.contains(link), "{link}: {html}");
        }
        assert!(html.contains(SIGN_IN_HINT), "{html}");
        assert!(html.contains("Save appearance"), "{html}");
    }

    /// The session check has not answered yet: no links (a signed-out
    /// visitor would see them flash) and no sign-in hint (a member would).
    #[test]
    fn nothing_account_related_shows_while_the_session_loads() {
        let html = render_with(None, false, true);
        for link in ACCOUNT_LINKS {
            assert!(!html.contains(link), "{link}: {html}");
        }
        assert!(!html.contains("Account pages"), "{html}");
        assert!(!html.contains(SIGN_IN_HINT), "{html}");
        assert!(html.contains("Save appearance"), "{html}");

        // A session that is already known keeps its links during a refresh.
        let html = render_with(Some(OrgRole::Member), true, true);
        for link in ACCOUNT_LINKS {
            assert!(html.contains(link), "{link}: {html}");
        }
    }

    /// Every choice carries the ring the selected dot is drawn in, so the
    /// selection is a shape and not only a colour (the CSS half is pinned in
    /// `styles::customise`).
    #[test]
    fn every_choice_draws_the_selection_mark() {
        let html = render(Some(OrgRole::Member), true);
        assert_eq!(
            html.matches("<span class=\"customise-choice-mark\" aria-hidden=\"true\"></span>")
                .count(),
            5,
            "{html}"
        );
        for label in html.split("<label").skip(1) {
            let label = &label[..label.find("</label>").expect("end of label")];
            assert!(label.contains("type=\"radio\""), "{label}");
            assert!(label.contains("customise-choice-mark"), "{label}");
        }
    }

    /// Density only restyles the redesigned pages today, and the page says so
    /// next to the control.
    #[test]
    fn density_card_says_where_density_applies() {
        let html = render(Some(OrgRole::Member), true);
        assert!(
            html.contains(
                "id=\"u1-appearance-density-note\">Density changes this page and brand settings for now. The rest of the site follows as each page is redesigned.</p>"
            ),
            "{html}"
        );
        let group = html
            .split("role=\"radiogroup\"")
            .nth(2)
            .expect("density group");
        let group = &group[..group.find('>').expect("end of tag")];
        assert!(
            group.contains("aria-describedby=\"u1-appearance-density-note\""),
            "{group}"
        );
    }

    #[test]
    fn status_line_follows_the_draft() {
        let saved = (ThemeMode::Dark, Density::Default);
        assert_eq!(status_text(saved, saved, false), "");
        assert_eq!(
            status_text((ThemeMode::Dark, Density::Compact), saved, false),
            "You have unsaved changes."
        );
        assert_eq!(
            status_text((ThemeMode::Light, Density::Default), saved, true),
            "You have unsaved changes."
        );
        assert_eq!(
            status_text(saved, saved, true),
            "Appearance saved on this device."
        );
    }

    /// Save is the only write to the shared theme and density, and density
    /// reaches the root through the provider, not through this page.
    #[test]
    fn save_writes_theme_and_density_through_the_provider() {
        let src = include_str!("appearance.rs");
        let prod = src.split("#[cfg(test)]\nmod tests").next().unwrap();
        assert_eq!(prod.matches("ctx.mode.set(").count(), 1);
        assert_eq!(prod.matches("ctx.density.set(").count(), 1);
        assert!(!prod.contains("set_attribute"));
        assert!(!prod.contains("local_storage"));
    }
}
