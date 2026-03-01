use leptos::prelude::*;
use leptos_meta::Style;

use crate::state::use_admin_state;

const SIDEBAR_STYLES: &str = r#"
    .admin-sidebar {
        width: 220px;
        background: var(--bg-surface);
        border-right: 1px solid var(--border);
        padding: 1.5rem 0;
        display: flex;
        flex-direction: column;
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
"#;

#[component]
pub fn Sidebar() -> impl IntoView {
    let state = use_admin_state();

    let user_display = move || {
        state.me.get().map(|me| {
            let role = me
                .member
                .as_ref()
                .map(|m| m.org_role.as_str())
                .unwrap_or("unknown");
            let name = me
                .member
                .as_ref()
                .map(|m| m.display_name.as_str())
                .unwrap_or(me.user.username.as_str());
            (name.to_string(), role.to_string())
        })
    };

    view! {
        <Style>{SIDEBAR_STYLES}</Style>
        <aside class="admin-sidebar">
            <div class="brand">
                <h2>"Scuffed Crew"</h2>
                <span>"Admin Panel"</span>
            </div>
            <nav>
                <a href="/admin/">"Dashboard"</a>
                <a href="/admin/members">"Members"</a>
                <a href="/admin/games">"Games"</a>
                <a href="/admin/teams">"Teams"</a>
                <a href="/admin/schedule">"Schedule"</a>
                <a href="/admin/applications">"Applications"</a>
                <a href="/admin/matches">"Matches"</a>
                <a href="/admin/tournaments">"Tournaments"</a>
                <a href="/admin/announcements">"Announcements"</a>
                {move || state.is_admin().then(|| view! {
                    <a href="/admin/audit-log">"Audit Log"</a>
                    <a href="/admin/settings">"Settings"</a>
                })}
            </nav>
            <div class="user-info">
                {move || user_display().map(|(name, role)| view! {
                    <div class="name">{name}</div>
                    <div class="role">{role}</div>
                })}
            </div>
        </aside>
    }
}
