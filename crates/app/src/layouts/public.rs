use dioxus::prelude::*;
use scuffed_api_client::ApiClient;
use scuffed_types::{NavConfig, NavPlacement, SiteSettings};

use super::{focus_element, use_document_keydown};
use crate::routes::Route;
use crate::state::auth::{AuthState, use_auth};
use crate::state::{loaded_site_settings, use_site_settings};
use crate::theme::ThemeToggle;

const NAV_TOGGLE_ID: &str = "site-nav-toggle";
const NAV_MENU_ID: &str = "site-nav-menu";
const MORE_TOGGLE_ID: &str = "nav-more-toggle";
const MORE_MENU_ID: &str = "nav-more-menu";
const ACCOUNT_TOGGLE_ID: &str = "nav-account-toggle";
const ACCOUNT_MENU_ID: &str = "nav-account-menu";

/// Which trigger should regain focus when a disclosure closes.
/// Mobile wins when several are open (resize while a desktop menu is open).
fn disclosure_focus_id(mobile: bool, more: bool, account: bool) -> Option<&'static str> {
    if mobile {
        Some(NAV_TOGGLE_ID)
    } else if more {
        Some(MORE_TOGGLE_ID)
    } else if account {
        Some(ACCOUNT_TOGGLE_ID)
    } else {
        None
    }
}

/// Closed `#site-nav-menu` is `inert` and never `aria-hidden`.
/// `aria-hidden` while a descendant still has focus is the console warning
/// "Blocked aria-hidden on an element because its descendant retained focus".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct NavMenuA11y {
    inert: bool,
    aria_hidden: Option<bool>,
}

fn nav_menu_a11y(open: bool) -> NavMenuA11y {
    NavMenuA11y {
        inert: !open,
        aria_hidden: None,
    }
}

/// Hamburger regains focus when the mobile menu closes.
/// Already-closed stays `None` so a desktop click does not steal focus.
fn mobile_menu_close_focus(was_open: bool) -> Option<&'static str> {
    if was_open { Some(NAV_TOGGLE_ID) } else { None }
}

/// Move focus to the hamburger first, then mark the menu closed.
/// The next render sets `inert`. Doing it in this order means the closed
/// overlay is not `aria-hidden` over a link that still holds focus.
fn close_mobile_nav(mut mobile_open: Signal<bool>) {
    let was_open = mobile_open();
    if let Some(id) = mobile_menu_close_focus(was_open) {
        focus_element(id);
    }
    if was_open {
        mobile_open.set(false);
    }
}

fn close_disclosures(
    mobile_open: Signal<bool>,
    mut more_open: Signal<bool>,
    mut account_open: Signal<bool>,
) {
    if mobile_open() {
        close_mobile_nav(mobile_open);
    } else if let Some(id) = disclosure_focus_id(false, more_open(), account_open()) {
        focus_element(id);
    }
    more_open.set(false);
    account_open.set(false);
}

/// Map catalog id → public route. Unknown ids are skipped.
pub(crate) fn nav_route(id: &str) -> Option<Route> {
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
        "leaderboards" => Route::Leaderboards {},
        "strategy" => Route::StrategyBrowse {},
        "patch_notes" => Route::PatchNotes {},
        "scrims" => Route::Scrims {},
        "chat" => Route::TeamChat {},
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

/// `strategy` follows `GET /api/settings`.strategies_enabled (default ON).
/// Patch Notes, Stats, and Leaderboards are not gated here.
fn nav_id_visible(id: &str, strategies_enabled: bool) -> bool {
    strategies_enabled || id != "strategy"
}

fn strategies_enabled_or_default(settings: Option<&SiteSettings>) -> bool {
    settings.map(|s| s.strategies_enabled).unwrap_or(true)
}

/// Pending settings show no catalog rows. A settled failure with nothing
/// painted uses the product default. A loaded org uses its own nav.
fn nav_for_resolved(resolved: Option<&Result<SiteSettings, String>>) -> NavConfig {
    if resolved.is_none() {
        return NavConfig { items: Vec::new() };
    }
    match loaded_site_settings(resolved) {
        Some(settings) => {
            let mut nav = settings.nav.clone();
            nav.normalize();
            nav
        }
        None => NavConfig::default(),
    }
}

fn resolve_nav(cfg: &NavConfig, placement: NavPlacement, strategies_enabled: bool) -> Vec<NavLink> {
    cfg.items_in(placement)
        .into_iter()
        .filter(|item| nav_id_visible(&item.id, strategies_enabled))
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
    .nav-icon.is-pending {
        color: transparent;
        background: var(--surface-2);
        box-shadow: none;
    }
    .nav-mark-text {
        font-family: var(--font-mono, var(--font-head));
        font-size: 0.72rem;
        letter-spacing: 0.12em;
        text-transform: uppercase;
        white-space: nowrap;
        /* Holds short and medium org names so `.nav-center` does not jump
           when the pending bar is replaced. The icon is already 26px. */
        min-width: 9rem;
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
    .nav-backdrop {
        position: absolute;
        inset: 0;
        z-index: 0;
        border: none;
        margin: 0;
        padding: 0;
        background: transparent;
        cursor: pointer;
    }
    .nav-overlay-sheet {
        position: relative;
        z-index: 1;
        display: flex;
        flex-direction: column;
        align-items: stretch;
        gap: 0.15rem;
    }
    .nav-dismiss-layer {
        position: fixed;
        inset: 0;
        z-index: 90;
    }
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

    // Off wasm, `Closure::wrap` aborts before this listener is installed.
    // Desktop still has a window; the non-wasm stub is what skips the handler.
    use_document_keydown(move |evt| {
        if evt.key() != "Escape" {
            return;
        }
        if !(mobile_open() || more_open() || account_open()) {
            return;
        }
        evt.prevent_default();
        close_disclosures(mobile_open, more_open, account_open);
    });

    let site_settings = use_site_settings();
    let resolved = site_settings.resolved.read();
    let loaded_settings = loaded_site_settings(resolved.as_ref());
    let bg_css = loaded_settings
        .as_ref()
        .map(|s| page_bg_css(&s.page_bg_color, &s.page_bg_image_url))
        .unwrap_or_default();

    let nav_cfg = nav_for_resolved(resolved.as_ref());
    let strategies_enabled = strategies_enabled_or_default(loaded_settings);
    let primary_links = resolve_nav(&nav_cfg, NavPlacement::Primary, strategies_enabled);
    let more_links = resolve_nav(&nav_cfg, NavPlacement::More, strategies_enabled);

    let is_logged_in = auth().is_logged_in();
    let is_officer = auth().is_officer_or_above();
    let username = auth()
        .user
        .as_ref()
        .map(|u| u.username.clone())
        .unwrap_or_default();
    let loading = auth().loading;

    let org_name = loaded_settings.as_ref().map(|s| s.org_name.clone());
    let site_description = loaded_settings
        .as_ref()
        .map(|s| s.site_description.trim().to_string())
        .filter(|d| !d.is_empty());
    let footer_text = org_name.as_ref().map(|name| match &site_description {
        Some(desc) => format!("© {name} · {desc}"),
        None => format!("© {name}"),
    });
    let mark_label = org_name.clone().unwrap_or_default();
    let nav_initials = org_name
        .as_deref()
        .map(scuffed_types::org_initials)
        .unwrap_or_default();
    let nav_icon_class = if nav_initials.is_empty() {
        "nav-icon is-pending"
    } else {
        "nav-icon"
    };

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
    let menu_a11y = nav_menu_a11y(mobile_open());
    debug_assert!(
        menu_a11y.aria_hidden.is_none(),
        "site nav menu must not set aria-hidden"
    );

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
                    close_mobile_nav(mobile_open);
                    more_open.set(false);
                    account_open.set(false);
                },
                div {
                    class: "{nav_icon_class}",
                    aria_hidden: "true",
                    "{nav_initials}"
                }
                span { class: "nav-mark-text",
                    if mark_label.is_empty() {
                        span { class: "brand-pending", aria_hidden: "true" }
                    } else {
                        "{mark_label}"
                    }
                }
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
                            id: MORE_TOGGLE_ID,
                            class: "nav-linkish",
                            r#type: "button",
                            aria_expanded: if more_open() { "true" } else { "false" },
                            aria_controls: MORE_MENU_ID,
                            aria_haspopup: "menu",
                            onclick: move |_| {
                                let closing = more_open();
                                more_open.toggle();
                                account_open.set(false);
                                if closing {
                                    focus_element(MORE_TOGGLE_ID);
                                }
                            },
                            "More ▾"
                        }
                        div { id: MORE_MENU_ID, class: "nav-drop-menu left",
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
                            id: ACCOUNT_TOGGLE_ID,
                            class: "nav-linkish",
                            r#type: "button",
                            aria_expanded: if account_open() { "true" } else { "false" },
                            aria_controls: ACCOUNT_MENU_ID,
                            aria_haspopup: "menu",
                            onclick: move |_| {
                                let closing = account_open();
                                account_open.toggle();
                                more_open.set(false);
                                if closing {
                                    focus_element(ACCOUNT_TOGGLE_ID);
                                }
                            },
                            span { class: "nav-user-chip", title: "{username}", "{username}" }
                            " ▾"
                        }
                        div { id: ACCOUNT_MENU_ID, class: "nav-drop-menu",
                            if is_officer {
                                Link {
                                    to: Route::AdminDashboard {},
                                    onclick: move |_| account_open.set(false),
                                    "Admin"
                                }
                            }
                            Link {
                                to: Route::ProfileSettings {},
                                onclick: move |_| account_open.set(false),
                                "Edit Profile"
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
                            Link {
                                to: Route::TeamChat {},
                                onclick: move |_| account_open.set(false),
                                "Chat"
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
                                    close_mobile_nav(mobile_open);
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
                    id: NAV_TOGGLE_ID,
                    class: hamburger_class,
                    r#type: "button",
                    aria_label: if mobile_open() { "Close menu" } else { "Open menu" },
                    aria_expanded: if mobile_open() { "true" } else { "false" },
                    aria_controls: NAV_MENU_ID,
                    onclick: move |_| {
                        more_open.set(false);
                        account_open.set(false);
                        if mobile_open() {
                            close_mobile_nav(mobile_open);
                        } else {
                            mobile_open.set(true);
                        }
                    },
                    span {}
                    span {}
                    span {}
                }
            }
        }

        if more_open() || account_open() {
            div {
                class: "nav-dismiss-layer",
                aria_hidden: "true",
                onclick: move |_| {
                    let focus = disclosure_focus_id(false, more_open(), account_open());
                    more_open.set(false);
                    account_open.set(false);
                    if let Some(id) = focus {
                        focus_element(id);
                    }
                },
            }
        }

        div {
            class: overlay_class,
            id: NAV_MENU_ID,
            inert: menu_a11y.inert,
            button {
                class: "nav-backdrop",
                r#type: "button",
                tabindex: "-1",
                aria_label: "Close menu",
                onclick: move |_| close_mobile_nav(mobile_open),
            }
            div { class: "nav-overlay-sheet",
            for link in primary_links.iter() {
                Link {
                    key: "m-{link.id}",
                    to: link.route.clone(),
                    onclick: move |_| close_mobile_nav(mobile_open),
                    "{link.label}"
                }
            }
            Link {
                to: Route::Apply {},
                class: "nav-cta",
                onclick: move |_| close_mobile_nav(mobile_open),
                "Apply"
            }
            if !more_links.is_empty() {
                div { class: "nav-overlay-label", "More" }
                for link in more_links.iter() {
                    Link {
                        key: "mm-{link.id}",
                        to: link.route.clone(),
                        onclick: move |_| close_mobile_nav(mobile_open),
                        "{link.label}"
                    }
                }
            }
            div { class: "nav-overlay-label", "Account" }
            if is_logged_in {
                if is_officer {
                    Link {
                        to: Route::AdminDashboard {},
                        onclick: move |_| close_mobile_nav(mobile_open),
                        "Admin"
                    }
                }
                Link {
                    to: Route::ProfileSettings {},
                    onclick: move |_| close_mobile_nav(mobile_open),
                    "Edit Profile"
                }
                Link {
                    to: Route::IdentitySettings {},
                    onclick: move |_| close_mobile_nav(mobile_open),
                    "Settings"
                }
                Link {
                    to: Route::DmInbox {},
                    onclick: move |_| close_mobile_nav(mobile_open),
                    "DMs"
                }
                Link {
                    to: Route::TeamChat {},
                    onclick: move |_| close_mobile_nav(mobile_open),
                    "Chat"
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
                        close_mobile_nav(mobile_open);
                    },
                    "Log out"
                }
            } else if !loading {
                Link {
                    to: Route::Login {},
                    onclick: move |_| close_mobile_nav(mobile_open),
                    "Login"
                }
            }
            div { class: "nav-overlay-theme",
                span { "Theme" }
                ThemeToggle {}
            }
            }
        }

        main { style: "padding-top: 48px; min-height: 100vh;",
            Outlet::<Route> {}
        }

        footer { class: "site-footer",
            if let Some(text) = footer_text {
                "{text}"
            } else {
                "© "
                span { class: "brand-pending", aria_hidden: "true" }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scuffed_types::NAV_CATALOG;

    #[test]
    fn pending_nav_is_empty_and_the_default_contains_members() {
        assert!(nav_for_resolved(None).items.is_empty());
        assert!(
            NavConfig::default()
                .items
                .iter()
                .any(|item| item.id == "members")
        );
        let failed: Option<Result<SiteSettings, String>> = Some(Err("offline".into()));
        assert!(
            nav_for_resolved(failed.as_ref())
                .items
                .iter()
                .any(|item| item.id == "members")
        );
    }

    #[test]
    fn catalog_ids_resolve_to_routes() {
        for entry in NAV_CATALOG {
            assert!(
                nav_route(entry.id).is_some(),
                "catalog id `{}` must map to a public route so Admin placement works",
                entry.id
            );
        }
    }

    #[test]
    fn patch_notes_catalog_id_opens_public_route() {
        assert_eq!(nav_route("patch_notes"), Some(Route::PatchNotes {}));
        assert_eq!(nav_route("leaderboards"), Some(Route::Leaderboards {}));
        assert_ne!(nav_route("patch_notes"), Some(Route::StrategyPatchNotes {}));
        assert_eq!(nav_route("strategy"), Some(Route::StrategyBrowse {}));
        assert_eq!(nav_route("not_in_catalog"), None);
    }

    #[test]
    fn strategy_nav_follows_feature_flag_patch_notes_does_not() {
        let cfg = NavConfig::default();
        let more_off = resolve_nav(&cfg, NavPlacement::More, false);
        assert!(
            more_off.iter().all(|l| l.id != "strategy"),
            "Strategies must leave Primary/More when the flag is off"
        );
        assert!(
            more_off.iter().any(|l| l.id == "patch_notes"),
            "Patch Notes must stay in nav when Strategies is off"
        );
        let more_on = resolve_nav(&cfg, NavPlacement::More, true);
        assert!(more_on.iter().any(|l| l.id == "strategy"));
        assert!(more_on.iter().any(|l| l.id == "patch_notes"));
        assert!(nav_id_visible("strategy", true));
        assert!(!nav_id_visible("strategy", false));
        assert!(nav_id_visible("patch_notes", false));
        assert!(nav_id_visible("stats", false));
        assert!(nav_id_visible("leaderboards", false));
        assert!(
            more_off.iter().any(|l| l.id == "leaderboards"),
            "Leaderboards follows Stats: the strategies flag does not hide it"
        );
        let primary_off = resolve_nav(&cfg, NavPlacement::Primary, false);
        assert!(primary_off.iter().any(|l| l.id == "stats"));
    }

    #[test]
    fn escape_returns_focus_to_the_open_trigger() {
        assert_eq!(disclosure_focus_id(false, false, false), None);
        assert_eq!(disclosure_focus_id(true, false, false), Some(NAV_TOGGLE_ID));
        assert_eq!(
            disclosure_focus_id(false, true, false),
            Some(MORE_TOGGLE_ID)
        );
        assert_eq!(
            disclosure_focus_id(false, false, true),
            Some(ACCOUNT_TOGGLE_ID)
        );
        assert_eq!(disclosure_focus_id(true, true, true), Some(NAV_TOGGLE_ID));
    }

    fn render(root: fn() -> Element) -> String {
        let mut dom = VirtualDom::new(root);
        dom.rebuild_in_place();
        dioxus_ssr::render(&dom)
    }

    fn opening_tag_with_id(html: &str, id: &str) -> String {
        let needle = format!("id=\"{id}\"");
        let id_at = html
            .find(&needle)
            .unwrap_or_else(|| panic!("missing {needle} in {html}"));
        let start = html[..id_at].rfind('<').expect("tag start");
        let end = html[id_at..]
            .find('>')
            .map(|rel| id_at + rel)
            .expect("tag end");
        html[start..=end].to_string()
    }

    #[component]
    fn NavMenuA11yProbe(open: bool) -> Element {
        let a11y = nav_menu_a11y(open);
        rsx! {
            div {
                id: NAV_MENU_ID,
                class: if open { "nav-overlay open" } else { "nav-overlay" },
                inert: a11y.inert,
                a { href: "/members", "Members" }
            }
        }
    }

    fn closed_menu_probe() -> Element {
        rsx! { NavMenuA11yProbe { open: false } }
    }

    fn open_menu_probe() -> Element {
        rsx! { NavMenuA11yProbe { open: true } }
    }

    fn public_shell() -> Element {
        let history =
            use_hook(|| std::rc::Rc::new(dioxus::history::MemoryHistory::with_initial_path("/")));
        let auth = use_signal(|| AuthState {
            user: None,
            loading: false,
        });
        use_context_provider(|| auth);
        crate::state::provide_site_settings();
        rsx! {
            crate::theme::ThemeProvider {
                dioxus::router::components::HistoryProvider {
                    history: move |_| history.clone() as std::rc::Rc<dyn dioxus::history::History>,
                    Router::<crate::routes::Route> {}
                }
            }
        }
    }

    #[test]
    fn closed_overlay_is_inert_and_not_aria_hidden_with_focus() {
        let closed = nav_menu_a11y(false);
        assert!(closed.inert);
        assert_eq!(
            closed.aria_hidden, None,
            "aria-hidden must stay unset so a focused link cannot trip the warning"
        );
        let open = nav_menu_a11y(true);
        assert!(!open.inert);
        assert_eq!(open.aria_hidden, None);
        assert_eq!(mobile_menu_close_focus(true), Some(NAV_TOGGLE_ID));
        assert_eq!(mobile_menu_close_focus(false), None);

        let probe = render(closed_menu_probe);
        let tag = opening_tag_with_id(&probe, NAV_MENU_ID);
        assert!(tag.contains("inert"), "{tag}");
        assert!(!tag.contains("aria-hidden"), "{tag}");
        assert!(
            probe.contains("<a "),
            "closed menu still contains a link that could hold focus: {probe}"
        );

        let opened = render(open_menu_probe);
        let open_tag = opening_tag_with_id(&opened, NAV_MENU_ID);
        assert!(
            !open_tag.contains("inert"),
            "open menu must stay focusable: {open_tag}"
        );
        assert!(!open_tag.contains("aria-hidden"), "{open_tag}");

        let shell = render(public_shell);
        let shell_tag = opening_tag_with_id(&shell, NAV_MENU_ID);
        assert!(
            shell_tag.contains("inert"),
            "closed #site-nav-menu must be inert: {shell_tag}"
        );
        assert!(
            !shell_tag.contains("aria-hidden"),
            "closed overlay must not be aria-hidden while a descendant can keep focus: {shell_tag}"
        );
        assert!(
            shell.contains("site-nav-toggle"),
            "closing returns focus to the hamburger, html={shell}"
        );

        let src = include_str!("public.rs");
        let prod = src.split("mod tests").next().expect("tests module");
        assert_eq!(
            prod.matches("mobile_open.set(false)").count(),
            1,
            "only close_mobile_nav may close the menu"
        );
        let close_at = prod.find("fn close_mobile_nav").expect("close_mobile_nav");
        let body = &prod[close_at..];
        let end = body.find("\nfn ").expect("next fn");
        let body = &body[..end];
        let focus_at = body.find("focus_element").expect("focus");
        let set_at = body.find("mobile_open.set(false)").expect("set");
        assert!(
            focus_at < set_at,
            "focus the hamburger before the closed render sets inert: {body}"
        );
    }
}
