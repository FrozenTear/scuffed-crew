use dioxus::prelude::*;
use scuffed_api_client::ApiClient;
use scuffed_types::{NavConfig, NavPlacement, SiteSettings};

use crate::routes::Route;
use crate::state::auth::{AuthState, use_auth};
use crate::theme::ThemeToggle;

/// Map catalog id → public route. Unknown ids are skipped.
fn nav_route(id: &str) -> Option<Route> {
    Some(match id {
        "members" => Route::Members {},
        "tournaments" => Route::Tournaments {},
        "news" => Route::News {},
        "forum" => Route::Forum {},
        "events" => Route::Events {},
        "community" => Route::Community {},
        "feed" => Route::Feed {},
        "polls" => Route::Polls {},
        "blog" => Route::Blog {},
        "wiki" => Route::Wiki {},
        "stats" => Route::Stats {},
        "strategy" => Route::StrategyBrowse {},
        "scrims" => Route::Scrims {},
        _ => return None,
    })
}

fn nav_label(id: &str) -> String {
    NavConfig::catalog_label(id).unwrap_or(id).to_string()
}

/// Resolved nav link for rendering (cloneable into rsx closures).
#[derive(Clone, PartialEq)]
struct NavLink {
    id: String,
    label: String,
    route: Route,
}

fn resolve_nav(cfg: &NavConfig, placement: NavPlacement) -> Vec<NavLink> {
    cfg.items_in(placement)
        .into_iter()
        .filter_map(|item| {
            let route = nav_route(&item.id)?;
            Some(NavLink {
                id: item.id.clone(),
                label: nav_label(&item.id),
                route,
            })
        })
        .collect()
}

/// Build a safe CSS snippet that applies admin-configured page background.
fn page_bg_css(color: &str, image_url: &str) -> String {
    let mut decls = String::new();
    let color = color.trim();
    if !color.is_empty()
        && color.starts_with('#')
        && color.len() <= 9
        && color.chars().skip(1).all(|c| c.is_ascii_hexdigit())
    {
        decls.push_str(&format!("--page-bg-color:{color};"));
    }
    let url = image_url.trim();
    if !url.is_empty()
        && !url.contains(['"', '\'', '(', ')', ';', '<', '>', '\\', '\n', '\r'])
        && (url.starts_with('/') || url.starts_with("https://") || url.starts_with("http://"))
    {
        decls.push_str(&format!("--page-bg-image:url(\"{url}\");"));
    }
    if decls.is_empty() {
        String::new()
    } else {
        format!(":root{{{decls}}}")
    }
}

const NAV_CSS: &str = r#"
    .site-nav {
        position: fixed;
        top: 0;
        left: 0;
        right: 0;
        z-index: 100;
        display: flex;
        align-items: center;
        gap: 1.25rem;
        padding: 0 1.25rem;
        height: 48px;
        background: color-mix(in srgb, var(--surface) 92%, transparent);
        backdrop-filter: blur(16px);
        border-bottom: 1px solid var(--border);
    }
    .nav-mark {
        display: flex;
        align-items: center;
        gap: 0.5rem;
        color: var(--text);
        text-decoration: none;
        flex-shrink: 0;
    }
    .nav-icon {
        width: 26px;
        height: 26px;
        background: var(--accent);
        border-radius: 4px;
        display: grid;
        place-items: center;
        font-family: var(--font-head);
        font-size: 0.85rem;
        font-weight: 700;
        color: var(--accent-fg);
        box-shadow: 0 0 12px color-mix(in srgb, var(--accent) 35%, transparent);
    }
    .nav-mark-text {
        font-family: var(--font-mono, var(--font-head));
        font-size: 0.72rem;
        letter-spacing: 0.12em;
        text-transform: uppercase;
        white-space: nowrap;
    }
    .nav-center {
        display: flex;
        align-items: center;
        gap: 0.15rem;
        flex: 1;
        min-width: 0;
        list-style: none;
        margin: 0;
        padding: 0;
    }
    .nav-center a,
    .nav-center button.nav-linkish,
    .nav-actions a,
    .nav-actions button.nav-linkish {
        padding: 0.3rem 0.55rem;
        color: var(--text-2);
        font-family: var(--font-mono, var(--font-head));
        font-size: 0.68rem;
        letter-spacing: 0.08em;
        text-transform: uppercase;
        border-radius: 4px;
        text-decoration: none;
        background: none;
        border: none;
        cursor: pointer;
        white-space: nowrap;
        transition: color 0.15s, background 0.15s;
    }
    .nav-center a:hover,
    .nav-center button.nav-linkish:hover,
    .nav-actions a:hover,
    .nav-actions button.nav-linkish:hover {
        color: var(--text);
        background: var(--surface-2);
    }
    .nav-actions {
        display: flex;
        align-items: center;
        gap: 0.35rem;
        margin-left: auto;
        flex-shrink: 0;
        list-style: none;
        margin: 0 0 0 auto;
        padding: 0;
    }
    .nav-cta {
        background: var(--accent) !important;
        color: var(--accent-fg) !important;
        padding: 0.35rem 0.75rem !important;
        font-weight: 500;
        box-shadow: 0 0 16px color-mix(in srgb, var(--accent) 30%, transparent);
    }
    .nav-cta:hover {
        filter: brightness(1.1);
    }
    .nav-ghost {
        color: var(--text-3) !important;
    }
    .nav-drop {
        position: relative;
    }
    .nav-drop-menu {
        display: none;
        position: absolute;
        top: calc(100% + 6px);
        right: 0;
        min-width: 10.5rem;
        padding: 0.35rem;
        background: var(--surface);
        border: 1px solid var(--border);
        border-radius: 8px;
        box-shadow: 0 12px 40px color-mix(in srgb, var(--bg) 80%, transparent);
        z-index: 120;
    }
    .nav-drop-menu.left {
        right: auto;
        left: 0;
    }
    .nav-drop.open .nav-drop-menu {
        display: flex;
        flex-direction: column;
        gap: 0.1rem;
    }
    .nav-drop-menu a,
    .nav-drop-menu button {
        display: block;
        width: 100%;
        text-align: left;
        padding: 0.45rem 0.65rem;
        color: var(--text-2);
        font-family: var(--font-mono, var(--font-head));
        font-size: 0.68rem;
        letter-spacing: 0.06em;
        text-transform: uppercase;
        border-radius: 5px;
        text-decoration: none;
        background: none;
        border: none;
        cursor: pointer;
    }
    .nav-drop-menu a:hover,
    .nav-drop-menu button:hover {
        color: var(--text);
        background: var(--surface-2);
    }
    .nav-drop-sep {
        height: 1px;
        background: var(--border);
        margin: 0.25rem 0.35rem;
    }
    .nav-user-chip {
        max-width: 7rem;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
        display: inline-block;
        vertical-align: bottom;
    }
    .nav-mobile-tools {
        display: none;
        align-items: center;
        gap: 0.35rem;
        margin-left: auto;
        flex-shrink: 0;
    }
    .nav-hamburger {
        display: none;
        flex-direction: column;
        gap: 4px;
        background: none;
        border: none;
        cursor: pointer;
        padding: 4px;
        flex-shrink: 0;
    }
    .nav-hamburger span {
        width: 18px;
        height: 2px;
        background: var(--text-2);
        transition: transform 0.2s, opacity 0.2s;
    }
    .nav-hamburger.open span:nth-child(1) { transform: rotate(45deg) translate(3px, 3px); }
    .nav-hamburger.open span:nth-child(2) { opacity: 0; }
    .nav-hamburger.open span:nth-child(3) { transform: rotate(-45deg) translate(3px, -3px); }
    .nav-overlay {
        display: none;
        position: fixed;
        inset: 0;
        z-index: 99;
        background: color-mix(in srgb, var(--bg) 97%, transparent);
        flex-direction: column;
        align-items: stretch;
        justify-content: flex-start;
        gap: 0.15rem;
        padding: 4.5rem 1.5rem 2rem;
        overflow-y: auto;
    }
    .nav-overlay.open { display: flex; }
    .nav-overlay a,
    .nav-overlay button {
        color: var(--text);
        font-size: 0.95rem;
        font-family: var(--font-head);
        text-align: left;
        text-decoration: none;
        background: none;
        border: none;
        cursor: pointer;
        padding: 0.55rem 0.25rem;
    }
    .nav-overlay-label {
        font-family: var(--font-mono, var(--font-head));
        font-size: 0.65rem;
        letter-spacing: 0.14em;
        text-transform: uppercase;
        color: var(--text-3);
        margin: 0.85rem 0 0.25rem;
    }
    .nav-overlay a.nav-cta {
        display: inline-flex;
        align-self: flex-start;
        margin: 0.35rem 0 0.5rem;
        background: var(--accent);
        color: var(--accent-fg);
        padding: 0.55rem 1rem;
        border-radius: 6px;
        font-size: 0.85rem;
        font-weight: 500;
    }
    .nav-overlay-theme {
        display: flex;
        align-items: center;
        gap: 0.65rem;
        margin-top: 1rem;
        padding-top: 0.75rem;
        border-top: 1px solid var(--border);
    }
    .nav-overlay-theme span {
        font-family: var(--font-mono, var(--font-head));
        font-size: 0.65rem;
        letter-spacing: 0.12em;
        text-transform: uppercase;
        color: var(--text-3);
    }
    .site-footer {
        border-top: 1px solid var(--border);
        padding: 2rem;
        text-align: center;
        color: var(--text-3);
        font-size: 0.8rem;
    }
    .theme-toggle {
        background: transparent;
        border: 1px solid var(--border);
        color: var(--text);
        width: 30px;
        height: 30px;
        border-radius: var(--radius-md, 6px);
        cursor: pointer;
        font-size: 0.9rem;
        display: inline-flex;
        align-items: center;
        justify-content: center;
        flex-shrink: 0;
    }
    .theme-toggle:hover { background: var(--surface-2); }
    @media (max-width: 820px) {
        .nav-center, .nav-actions { display: none; }
        .nav-mobile-tools { display: flex; }
        .nav-hamburger { display: flex; }
        .nav-mark-text { display: none; }
    }
"#;

#[component]
pub fn PublicLayout() -> Element {
    let mut mobile_open = use_signal(|| false);
    let mut more_open = use_signal(|| false);
    let mut account_open = use_signal(|| false);
    let auth = use_auth();

    let site_settings = use_resource(|| async {
        ApiClient::web()
            .fetch::<SiteSettings>("/api/settings")
            .await
            .ok()
    });
    let bg_css = site_settings
        .read()
        .as_ref()
        .and_then(|o| o.as_ref())
        .map(|s| page_bg_css(&s.page_bg_color, &s.page_bg_image_url))
        .unwrap_or_default();

    let nav_cfg = site_settings
        .read()
        .as_ref()
        .and_then(|o| o.as_ref())
        .map(|s| {
            let mut n = s.nav.clone();
            n.normalize();
            n
        })
        .unwrap_or_default();
    let primary_links = resolve_nav(&nav_cfg, NavPlacement::Primary);
    let more_links = resolve_nav(&nav_cfg, NavPlacement::More);

    let is_logged_in = auth().is_logged_in();
    let is_officer = auth().is_officer_or_above();
    let username = auth()
        .user
        .as_ref()
        .map(|u| u.username.clone())
        .unwrap_or_default();
    let loading = auth().loading;

    let org_name = site_settings
        .read()
        .as_ref()
        .and_then(|o| o.as_ref())
        .map(|s| s.org_name.clone())
        .unwrap_or_else(|| "My Clan".into());
    let site_description = site_settings
        .read()
        .as_ref()
        .and_then(|o| o.as_ref())
        .map(|s| s.site_description.trim().to_string())
        .filter(|d| !d.is_empty());
    let footer_text = match &site_description {
        Some(desc) => format!("© {org_name} · {desc}"),
        None => format!("© {org_name}"),
    };
    let mark_label = org_name.clone();
    let nav_initials = scuffed_types::org_initials(&org_name);

    let more_class = if more_open() {
        "nav-drop open"
    } else {
        "nav-drop"
    };
    let account_class = if account_open() {
        "nav-drop open"
    } else {
        "nav-drop"
    };
    let hamburger_class = if mobile_open() {
        "nav-hamburger open"
    } else {
        "nav-hamburger"
    };
    let overlay_class = if mobile_open() {
        "nav-overlay open"
    } else {
        "nav-overlay"
    };

    rsx! {
        style { {NAV_CSS} }
        if !bg_css.is_empty() {
            style { {bg_css} }
        }
        nav { class: "site-nav",
            Link {
                to: Route::Home {},
                class: "nav-mark",
                onclick: move |_| {
                    mobile_open.set(false);
                    more_open.set(false);
                    account_open.set(false);
                },
                div {
                    class: "nav-icon",
                    aria_hidden: "true",
                    "{nav_initials}"
                }
                span { class: "nav-mark-text", "{mark_label}" }
            }

            ul { class: "nav-center",
                for link in primary_links.iter() {
                    li {
                        key: "{link.id}",
                        Link { to: link.route.clone(), "{link.label}" }
                    }
                }
                if !more_links.is_empty() {
                    li { class: "{more_class}",
                        button {
                            class: "nav-linkish",
                            onclick: move |_| {
                                more_open.toggle();
                                account_open.set(false);
                            },
                            "More ▾"
                        }
                        div { class: "nav-drop-menu left",
                            for link in more_links.iter() {
                                Link {
                                    key: "{link.id}",
                                    to: link.route.clone(),
                                    onclick: move |_| more_open.set(false),
                                    "{link.label}"
                                }
                            }
                        }
                    }
                }
            }

            ul { class: "nav-actions",
                li {
                    Link { to: Route::Apply {}, class: "nav-cta", "Apply" }
                }
                if loading {
                    li { span { class: "nav-user-chip", "…" } }
                } else if is_logged_in {
                    li { class: "{account_class}",
                        button {
                            class: "nav-linkish",
                            onclick: move |_| {
                                account_open.toggle();
                                more_open.set(false);
                            },
                            span { class: "nav-user-chip", title: "{username}", "{username}" }
                            " ▾"
                        }
                        div { class: "nav-drop-menu",
                            if is_officer {
                                Link {
                                    to: Route::AdminDashboard {},
                                    onclick: move |_| account_open.set(false),
                                    "Admin"
                                }
                            }
                            Link {
                                to: Route::IdentitySettings {},
                                onclick: move |_| account_open.set(false),
                                "Settings"
                            }
                            Link {
                                to: Route::DmInbox {},
                                onclick: move |_| account_open.set(false),
                                "DMs"
                            }
                            div { class: "nav-drop-sep" }
                            button {
                                onclick: move |_| {
                                    let mut auth = auth;
                                    spawn(async move {
                                        let _ = ApiClient::web().logout().await;
                                        auth.set(AuthState {
                                            user: None,
                                            loading: false,
                                        });
                                    });
                                    account_open.set(false);
                                    mobile_open.set(false);
                                },
                                "Log out"
                            }
                        }
                    }
                } else {
                    li {
                        Link {
                            to: Route::Login {},
                            class: "nav-linkish",
                            "Login"
                        }
                    }
                }
                li { ThemeToggle {} }
            }

            div { class: "nav-mobile-tools",
                ThemeToggle {}
                button {
                    class: hamburger_class,
                    aria_label: "Toggle menu",
                    onclick: move |_| {
                        mobile_open.toggle();
                        more_open.set(false);
                        account_open.set(false);
                    },
                    span {}
                    span {}
                    span {}
                }
            }
        }

        if more_open() || account_open() {
            div {
                style: "position:fixed;inset:0;z-index:90;",
                onclick: move |_| {
                    more_open.set(false);
                    account_open.set(false);
                }
            }
        }

        div { class: overlay_class,
            for link in primary_links.iter() {
                Link {
                    key: "m-{link.id}",
                    to: link.route.clone(),
                    onclick: move |_| mobile_open.set(false),
                    "{link.label}"
                }
            }
            Link {
                to: Route::Apply {},
                class: "nav-cta",
                onclick: move |_| mobile_open.set(false),
                "Apply"
            }
            if !more_links.is_empty() {
                div { class: "nav-overlay-label", "More" }
                for link in more_links.iter() {
                    Link {
                        key: "mm-{link.id}",
                        to: link.route.clone(),
                        onclick: move |_| mobile_open.set(false),
                        "{link.label}"
                    }
                }
            }
            div { class: "nav-overlay-label", "Account" }
            if is_logged_in {
                if is_officer {
                    Link {
                        to: Route::AdminDashboard {},
                        onclick: move |_| mobile_open.set(false),
                        "Admin"
                    }
                }
                Link {
                    to: Route::IdentitySettings {},
                    onclick: move |_| mobile_open.set(false),
                    "Settings"
                }
                Link {
                    to: Route::DmInbox {},
                    onclick: move |_| mobile_open.set(false),
                    "DMs"
                }
                button {
                    onclick: move |_| {
                        let mut auth = auth;
                        spawn(async move {
                            let _ = ApiClient::web().logout().await;
                            auth.set(AuthState {
                                user: None,
                                loading: false,
                            });
                        });
                        mobile_open.set(false);
                    },
                    "Log out"
                }
            } else if !loading {
                Link {
                    to: Route::Login {},
                    onclick: move |_| mobile_open.set(false),
                    "Login"
                }
            }
            div { class: "nav-overlay-theme",
                span { "Theme" }
                ThemeToggle {}
            }
        }

        main { style: "padding-top: 48px; min-height: 100vh;",
            Outlet::<Route> {}
        }

        footer { class: "site-footer",
            "{footer_text}"
        }
    }
}
