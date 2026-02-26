use leptos::prelude::*;
use leptos_meta::*;
use leptos_router::{
    components::{Route, Router, Routes},
    path,
};

use scuffed_ui::{scuffed_crew_theme, ThemeProvider};
use scuffed_ui::components::button::BUTTON_STYLES;
use scuffed_ui::components::card::CARD_STYLES;
use scuffed_ui::components::toast::{ToastContainer, ToastState, TOAST_STYLES};

use crate::components::sidebar::Sidebar;
use crate::guards::RequireOfficer;
use crate::pages::*;
use crate::state::AdminState;

const ADMIN_STYLES: &str = r#"
    .admin-layout {
        display: flex;
        min-height: 100vh;
        background: var(--bg-void);
        color: var(--text-primary);
        font-family: var(--font-body);
    }
    .admin-main {
        flex: 1;
        padding: 2rem;
        overflow-y: auto;
    }
    .admin-main h1 {
        font-family: var(--font-display);
        font-size: 1.8rem;
        color: var(--text-bright);
        margin-bottom: 1.5rem;
        text-transform: uppercase;
    }
    .admin-loading, .admin-denied {
        display: flex;
        flex-direction: column;
        align-items: center;
        justify-content: center;
        min-height: 100vh;
        color: var(--text-secondary);
        font-family: var(--font-body);
    }
    .admin-denied h2 {
        color: var(--text-bright);
        margin-bottom: 0.5rem;
    }
    .admin-denied a {
        color: var(--accent);
    }
    .summary-cards {
        display: grid;
        grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
        gap: 1rem;
        margin-bottom: 2rem;
    }
    .summary-card {
        background: var(--bg-card);
        border: 1px solid var(--border);
        border-radius: 8px;
        padding: 1.25rem;
    }
    .summary-card .label {
        color: var(--text-muted);
        font-size: 0.8rem;
        text-transform: uppercase;
        letter-spacing: 0.05em;
    }
    .summary-card .value {
        font-size: 2rem;
        font-weight: 700;
        color: var(--accent);
        font-family: var(--font-display);
    }
    .data-table {
        width: 100%;
        border-collapse: collapse;
        background: var(--bg-card);
        border: 1px solid var(--border);
        border-radius: 8px;
        overflow: hidden;
    }
    .data-table th {
        text-align: left;
        padding: 0.75rem 1rem;
        background: var(--bg-elevated);
        color: var(--text-muted);
        font-size: 0.75rem;
        text-transform: uppercase;
        letter-spacing: 0.05em;
        border-bottom: 1px solid var(--border);
    }
    .data-table td {
        padding: 0.75rem 1rem;
        border-bottom: 1px solid var(--border-light);
        color: var(--text-primary);
    }
    .data-table tr:last-child td {
        border-bottom: none;
    }
    .status-pill {
        display: inline-block;
        padding: 0.2rem 0.6rem;
        border-radius: 999px;
        font-size: 0.7rem;
        font-weight: 600;
        text-transform: uppercase;
        letter-spacing: 0.03em;
    }
    .status-pill.pending { background: #7c3aed33; color: #a78bfa; }
    .status-pill.trial { background: #f59e0b33; color: #fbbf24; }
    .status-pill.accepted { background: #10b98133; color: #34d399; }
    .status-pill.rejected { background: #ef444433; color: #f87171; }
    .status-pill.withdrawn { background: #6b728033; color: #9ca3af; }
    .role-pill {
        display: inline-block;
        padding: 0.2rem 0.6rem;
        border-radius: 999px;
        font-size: 0.7rem;
        font-weight: 600;
        text-transform: uppercase;
    }
    .role-pill.admin { background: #ef444433; color: #f87171; }
    .role-pill.officer { background: #f59e0b33; color: #fbbf24; }
    .role-pill.member { background: #7c3aed33; color: #a78bfa; }
    .role-pill.recruit { background: #6b728033; color: #9ca3af; }
    .admin-form {
        display: flex;
        flex-direction: column;
        gap: 1rem;
        max-width: 500px;
        margin-bottom: 2rem;
    }
    .admin-form label {
        color: var(--text-secondary);
        font-size: 0.85rem;
    }
    .admin-form input, .admin-form select, .admin-form textarea {
        background: var(--bg-surface);
        border: 1px solid var(--border);
        border-radius: 6px;
        padding: 0.5rem 0.75rem;
        color: var(--text-primary);
        font-family: var(--font-body);
        font-size: 0.9rem;
    }
    .admin-form input:focus, .admin-form select:focus, .admin-form textarea:focus {
        outline: none;
        border-color: var(--accent);
    }
"#;

#[component]
pub fn AdminApp() -> impl IntoView {
    provide_meta_context();
    provide_context(ToastState::new());

    let admin_state = AdminState::new();
    admin_state.fetch_me();
    provide_context(admin_state);

    let theme = scuffed_crew_theme();

    let styles = format!(
        "{}\n{}\n{}\n{}",
        BUTTON_STYLES, CARD_STYLES, TOAST_STYLES, ADMIN_STYLES,
    );

    view! {
        <Title text="Admin — The Scuffed Crew"/>
        <Style>{styles}</Style>

        <ThemeProvider theme=theme>
            <RequireOfficer>
                <Router base="/admin">
                    <div class="admin-layout">
                        <Sidebar/>
                        <main class="admin-main">
                            <Routes fallback=|| view! { <p>"Page not found"</p> }>
                                <Route path=path!("/") view=DashboardPage/>
                                <Route path=path!("/members") view=MembersPage/>
                                <Route path=path!("/teams") view=TeamsPage/>
                                <Route path=path!("/schedule") view=SchedulePage/>
                                <Route path=path!("/applications") view=ApplicationsPage/>
                                <Route path=path!("/matches") view=MatchesPage/>
                            </Routes>
                        </main>
                    </div>
                    <ToastContainer/>
                </Router>
            </RequireOfficer>
        </ThemeProvider>
    }
}
