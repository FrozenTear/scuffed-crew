use dioxus::prelude::*;

use crate::routes::Route;
use crate::state::use_auth;
use crate::theme::ThemeToggle;

const ADMIN_CSS: &str = r#"
    .admin-layout {
        display: flex;
        min-height: 100vh;
        background: var(--bg);
        color: var(--text);
        font-family: var(--font-body);
    }
    .admin-sidebar {
        width: 220px;
        flex-shrink: 0;
        background: var(--surface);
        border-right: 1px solid var(--border);
        padding: 1.5rem 0;
        display: flex;
        flex-direction: column;
        position: sticky;
        top: 0;
        height: 100vh;
        z-index: 40;
        overflow-y: auto;
    }
    .admin-sidebar .brand {
        padding: 0 1.25rem 1.25rem;
        border-bottom: 1px solid var(--border);
        margin-bottom: 1rem;
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 0.5rem;
    }
    .admin-sidebar .brand-text h2 {
        font-family: var(--font-head);
        font-size: 1.2rem;
        color: var(--accent);
        text-transform: uppercase;
        margin: 0;
    }
    .admin-sidebar .brand-text span {
        font-size: 0.7rem;
        color: var(--text-3);
        text-transform: uppercase;
        letter-spacing: 0.05em;
    }
    .admin-sidebar nav {
        display: flex;
        flex-direction: column;
        gap: 0.15rem;
        flex: 1;
    }
    .admin-sidebar nav a {
        display: block;
        padding: 0.6rem 1.25rem;
        color: var(--text-2);
        text-decoration: none;
        font-size: 0.9rem;
        transition: background 0.15s, color 0.15s;
    }
    .admin-sidebar nav a:hover {
        background: var(--surface-2);
        color: var(--text);
    }
    .admin-sidebar .user-info {
        padding: 1rem 1.25rem 0;
        border-top: 1px solid var(--border);
        margin-top: auto;
    }
    .admin-sidebar .user-info .name {
        color: var(--text);
        font-size: 0.85rem;
        font-weight: 600;
    }
    .admin-sidebar .user-info .role {
        font-size: 0.7rem;
        color: var(--text-3);
        text-transform: uppercase;
    }
    .admin-sidebar-close {
        display: none;
        margin-left: auto;
        background: transparent;
        border: 1px solid var(--border);
        color: var(--text-2);
        border-radius: 6px;
        width: 2.25rem;
        height: 2.25rem;
        cursor: pointer;
        font-size: 1.1rem;
        line-height: 1;
        flex-shrink: 0;
    }
    .admin-sidebar-close:hover { color: var(--text); border-color: var(--accent-soft); }
    .admin-mobile-bar {
        display: none;
    }
    .admin-nav-overlay {
        display: none;
    }
    .admin-main {
        flex: 1;
        min-width: 0;
        padding: 2rem 2.5rem;
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
    @media (max-width: 768px) {
        .admin-layout { flex-direction: column; }
        .admin-mobile-bar {
            display: flex;
            align-items: center;
            gap: 0.75rem;
            position: sticky;
            top: 0;
            z-index: 30;
            padding: 0.65rem 0.85rem;
            background: color-mix(in srgb, var(--surface) 94%, transparent);
            backdrop-filter: blur(10px);
            border-bottom: 1px solid var(--border);
        }
        .admin-mobile-bar .menu-btn {
            background: var(--surface-2);
            border: 1px solid var(--border);
            color: var(--text);
            border-radius: 6px;
            min-width: 2.75rem;
            min-height: 2.75rem;
            font-size: 1.15rem;
            cursor: pointer;
            line-height: 1;
        }
        .admin-mobile-bar .menu-btn:hover { border-color: var(--accent-soft); }
        .admin-mobile-bar .title {
            font-family: var(--font-head);
            font-size: 0.95rem;
            font-weight: 700;
            color: var(--accent);
            text-transform: uppercase;
            letter-spacing: 0.04em;
        }
        .admin-sidebar {
            position: fixed;
            left: 0;
            top: 0;
            bottom: 0;
            height: 100vh;
            width: min(18rem, 86vw);
            transform: translateX(-105%);
            transition: transform 0.2s ease-out;
            box-shadow: none;
            border-right: 1px solid var(--border);
        }
        .admin-sidebar.is-open {
            transform: translateX(0);
            box-shadow: 8px 0 24px var(--overlay, color-mix(in srgb, var(--bg) 40%, transparent));
        }
        .admin-sidebar-close { display: inline-flex; align-items: center; justify-content: center; }
        .admin-nav-overlay {
            display: block;
            position: fixed;
            inset: 0;
            z-index: 35;
            background: var(--overlay);
            border: none;
            padding: 0;
            cursor: pointer;
        }
        .admin-main {
            padding: 1rem;
            width: 100%;
        }
        .admin-main h1 { font-size: 1.35rem; margin-bottom: 1rem; }
    }
"#;

#[component]
pub fn AdminLayout() -> Element {
    let auth = use_auth();
    let mut nav_open = use_signal(|| false);

    let is_admin = auth().is_admin();
    let username = auth()
        .user
        .as_ref()
        .map(|u| u.username.clone())
        .unwrap_or_default();
    let role_display = auth()
        .user
        .as_ref()
        .and_then(|u| u.role.as_ref())
        .map(|r| format!("{r:?}"))
        .unwrap_or_else(|| "—".into());

    // Auth loading: avoid flashing Access Denied before /api/auth/me resolves
    if auth().loading {
        return rsx! {
            div {
                style: "display:flex;flex-direction:column;align-items:center;justify-content:center;min-height:100vh;color:var(--text-3);",
                p { "Checking session…" }
            }
        };
    }

    // Auth guard: officer+ only
    if !auth().is_officer_or_above() {
        return rsx! {
            div {
                style: "display:flex;flex-direction:column;align-items:center;justify-content:center;min-height:100vh;color:var(--text-2);gap:0.5rem;",
                h2 { style: "color:var(--text);margin-bottom:0.5rem;", "Access Denied" }
                p { "You need officer permissions to access the admin panel." }
                Link {
                    to: Route::Login {},
                    style: "color:var(--accent);margin-top:0.5rem;",
                    "Sign in"
                }
                if cfg!(debug_assertions) {
                    a {
                        href: "/api/dev/login",
                        style: "color:var(--text-2);",
                        "Dev login (in-memory only)"
                    }
                }
                Link { to: Route::Home {}, style: "color:var(--accent);margin-top:1rem;", "Return home" }
            }
        };
    }

    let sidebar_class = if nav_open() {
        "admin-sidebar is-open"
    } else {
        "admin-sidebar"
    };

    // Close drawer after navigation (mobile); desktop ignores open state via CSS.
    let close_nav = move |_| nav_open.set(false);

    rsx! {
        style { {ADMIN_CSS} }
        style { {crate::styles::admin::CSS} }
        div { class: "admin-layout",
            div { class: "admin-mobile-bar",
                button {
                    class: "menu-btn",
                    r#type: "button",
                    "aria-label": "Open admin menu",
                    "aria-expanded": if nav_open() { "true" } else { "false" },
                    onclick: move |_| nav_open.set(true),
                    "☰"
                }
                span { class: "title", "Admin" }
            }
            if nav_open() {
                button {
                    class: "admin-nav-overlay",
                    r#type: "button",
                    "aria-label": "Close admin menu",
                    onclick: move |_| nav_open.set(false),
                }
            }
            aside { class: "{sidebar_class}",
                div { class: "brand",
                    div { class: "brand-text",
                        h2 { "Scuffed Crew" }
                        span { "Admin Panel" }
                    }
                    ThemeToggle {}
                    button {
                        class: "admin-sidebar-close",
                        r#type: "button",
                        "aria-label": "Close admin menu",
                        onclick: move |_| nav_open.set(false),
                        "×"
                    }
                }
                nav {
                    Link { to: Route::AdminDashboard {}, onclick: close_nav, "Dashboard" }
                    Link { to: Route::AdminMembers {}, onclick: close_nav, "Members" }
                    Link { to: Route::AdminGames {}, onclick: close_nav, "Games" }
                    Link { to: Route::AdminTeams {}, onclick: close_nav, "Teams" }
                    Link { to: Route::AdminSchedule {}, onclick: close_nav, "Schedule" }
                    Link { to: Route::AdminApplications {}, onclick: close_nav, "Applications" }
                    Link { to: Route::AdminMatches {}, onclick: close_nav, "Matches" }
                    Link { to: Route::AdminTournaments {}, onclick: close_nav, "Tournaments" }
                    Link { to: Route::AdminAnnouncements {}, onclick: close_nav, "Announcements" }
                    Link { to: Route::AdminArticles {}, onclick: close_nav, "Articles" }
                    Link { to: Route::AdminForum {}, onclick: close_nav, "Forum" }
                    // Moderation is OfficerUser-gated server-side (list/create), so it is
                    // visible to every officer+ — matching the AdminLayout access tier.
                    Link { to: Route::AdminModeration {}, onclick: close_nav, "Moderation" }
                    if is_admin {
                        Link { to: Route::AdminRelay {}, onclick: close_nav, "Relay" }
                        Link { to: Route::AdminAuditLog {}, onclick: close_nav, "Audit Log" }
                        Link { to: Route::AdminSettings {}, onclick: close_nav, "Settings" }
                    }
                }
                div { class: "user-info",
                    div { class: "name", "{username}" }
                    div { class: "role", "{role_display}" }
                }
            }
            main { class: "admin-main",
                Outlet::<Route> {}
            }
        }
    }
}
