use dioxus::prelude::*;

use crate::routes::Route;
use crate::state::use_auth;

const ADMIN_CSS: &str = r#"
    .admin-layout {
        display: flex;
        min-height: 100vh;
        background: var(--bg-void);
        color: var(--text-primary);
        font-family: var(--font-body);
    }
    .admin-sidebar {
        width: 220px;
        background: var(--bg-surface);
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
    }
    .admin-sidebar .brand h2 {
        font-family: var(--font-display-hero);
        font-size: 1.2rem;
        color: var(--accent);
        text-transform: uppercase;
        margin: 0;
    }
    .admin-sidebar .brand span {
        font-size: 0.7rem;
        color: var(--text-muted);
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
        color: var(--text-secondary);
        text-decoration: none;
        font-size: 0.9rem;
        transition: background 0.15s, color 0.15s;
    }
    .admin-sidebar nav a:hover {
        background: var(--bg-card);
        color: var(--text-bright);
    }
    .admin-sidebar .user-info {
        padding: 1rem 1.25rem 0;
        border-top: 1px solid var(--border);
        margin-top: auto;
    }
    .admin-sidebar .user-info .name {
        color: var(--text-bright);
        font-size: 0.85rem;
        font-weight: 600;
    }
    .admin-sidebar .user-info .role {
        font-size: 0.7rem;
        color: var(--text-muted);
        text-transform: uppercase;
    }
    .admin-main {
        flex: 1;
        padding: 2rem 2.5rem;
        overflow-y: auto;
    }
    .admin-main h1 {
        font-family: var(--font-display);
        font-size: 1.8rem;
        color: var(--text-bright);
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

    // Auth guard: redirect if not officer+
    if !auth().loading && !auth().is_officer_or_above() {
        return rsx! {
            div {
                style: "display:flex;flex-direction:column;align-items:center;justify-content:center;min-height:100vh;color:var(--text-secondary);",
                h2 { style: "color:var(--text-bright);margin-bottom:0.5rem;", "Access Denied" }
                p { "You need officer permissions to access the admin panel." }
                Link { to: Route::Home {}, style: "color:var(--accent);margin-top:1rem;", "Return home" }
            }
        };
    }

    rsx! {
        style { {ADMIN_CSS} }
        div { class: "admin-layout",
            aside { class: "admin-sidebar",
                div { class: "brand",
                    h2 { "Scuffed Crew" }
                    span { "Admin Panel" }
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
                    if is_admin {
                        Link { to: Route::AdminModeration {}, "Moderation" }
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
