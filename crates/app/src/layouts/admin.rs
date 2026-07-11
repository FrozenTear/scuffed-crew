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
        background: var(--surface);
        border-right: 1px solid var(--border);
        padding: 1.5rem 0;
        display: flex;
        flex-direction: column;
        position: sticky;
        top: 0;
        height: 100vh;
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
    .admin-main {
        flex: 1;
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
"#;

#[component]
pub fn AdminLayout() -> Element {
    let auth = use_auth();

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

    rsx! {
        style { {ADMIN_CSS} }
        style { {crate::styles::admin::CSS} }
        div { class: "admin-layout",
            aside { class: "admin-sidebar",
                div { class: "brand",
                    div { class: "brand-text",
                        h2 { "Scuffed Crew" }
                        span { "Admin Panel" }
                    }
                    ThemeToggle {}
                }
                nav {
                    Link { to: Route::AdminDashboard {}, "Dashboard" }
                    Link { to: Route::AdminMembers {}, "Members" }
                    Link { to: Route::AdminGames {}, "Games" }
                    Link { to: Route::AdminTeams {}, "Teams" }
                    Link { to: Route::AdminSchedule {}, "Schedule" }
                    Link { to: Route::AdminApplications {}, "Applications" }
                    Link { to: Route::AdminMatches {}, "Matches" }
                    Link { to: Route::AdminTournaments {}, "Tournaments" }
                    Link { to: Route::AdminAnnouncements {}, "Announcements" }
                    Link { to: Route::AdminArticles {}, "Articles" }
                    Link { to: Route::AdminForum {}, "Forum" }
                    if is_admin {
                        Link { to: Route::AdminModeration {}, "Moderation" }
                        Link { to: Route::AdminRelay {}, "Relay" }
                        Link { to: Route::AdminAuditLog {}, "Audit Log" }
                        Link { to: Route::AdminSettings {}, "Settings" }
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
