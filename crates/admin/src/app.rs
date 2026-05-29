use leptos::prelude::*;
use leptos_meta::*;
use leptos_router::{
    components::{Route, Router, Routes},
    path,
};

use scuffed_ui::components::button::BUTTON_STYLES;
use scuffed_ui::components::card::CARD_STYLES;
use scuffed_ui::components::modal::MODAL_STYLES;
use scuffed_ui::components::toast::{ToastContainer, ToastState, TOAST_STYLES};
use scuffed_ui::{scuffed_crew_theme, ThemeProvider};

use crate::components::confirm_dialog::CONFIRM_DIALOG_STYLES;
use crate::components::form_modal::FORM_MODAL_STYLES;

use crate::components::sidebar::Sidebar;
use crate::guards::RequireOfficer;
use crate::pages::*;
use crate::state::AdminState;

const ADMIN_STYLES: &str = r#"
    /* ─── Layout ─── */
    .admin-layout {
        display: flex;
        min-height: 100vh;
        background: var(--bg-void);
        color: var(--text-primary);
        font-family: var(--font-body);
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

    /* ─── Summary Cards ─── */
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

    /* ─── Data Table ─── */
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
    .data-table tr:hover td {
        background: var(--bg-card-alt);
    }

    /* ─── Pills ─── */
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

    /* ─── Form System ─── */
    .admin-form {
        display: flex;
        flex-direction: column;
        gap: 1.25rem;
        max-width: 500px;
    }
    .form-group {
        display: flex;
        flex-direction: column;
        gap: 0.4rem;
    }
    .form-label {
        color: var(--text-secondary);
        font-size: 0.8rem;
        font-weight: 600;
        text-transform: uppercase;
        letter-spacing: 0.04em;
        font-family: var(--font-display);
    }
    .form-input,
    .admin-form input,
    .admin-form select,
    .admin-form textarea {
        background: var(--bg-surface);
        border: 1px solid var(--border);
        border-radius: 6px;
        padding: 0.6rem 0.85rem;
        color: var(--text-primary);
        font-family: var(--font-body);
        font-size: 0.9rem;
        width: 100%;
        transition: border-color 0.2s, box-shadow 0.2s;
    }
    .form-input:focus,
    .admin-form input:focus,
    .admin-form select:focus,
    .admin-form textarea:focus {
        outline: none;
        border-color: var(--accent);
        box-shadow: 0 0 0 3px var(--accent-soft);
    }
    .form-input::placeholder {
        color: var(--text-muted);
    }
    .admin-form select {
        cursor: pointer;
        appearance: none;
        background-image: url("data:image/svg+xml,%3Csvg width='10' height='6' viewBox='0 0 10 6' fill='none' xmlns='http://www.w3.org/2000/svg'%3E%3Cpath d='M1 1L5 5L9 1' stroke='%23807a70' stroke-width='1.5' stroke-linecap='round' stroke-linejoin='round'/%3E%3C/svg%3E");
        background-repeat: no-repeat;
        background-position: right 0.75rem center;
        padding-right: 2rem;
    }
    .admin-form textarea {
        resize: vertical;
        min-height: 4rem;
    }
    .checkbox-label {
        display: flex;
        align-items: center;
        gap: 0.6rem;
        cursor: pointer;
        color: var(--text-primary);
        font-size: 0.9rem;
        padding: 0.25rem 0;
    }
    .checkbox-label input[type="checkbox"] {
        width: 1.15rem;
        height: 1.15rem;
        accent-color: var(--accent);
        cursor: pointer;
        flex-shrink: 0;
    }

    /* ─── Settings Page ─── */
    .settings-form {
        max-width: 600px;
    }
    .settings-form .form-section {
        background: var(--bg-card);
        border: 1px solid var(--border);
        border-radius: 8px;
        padding: 1.5rem;
        margin-bottom: 1.25rem;
    }
    .settings-form .form-section-title {
        font-family: var(--font-display);
        font-size: 0.9rem;
        color: var(--text-muted);
        text-transform: uppercase;
        letter-spacing: 0.06em;
        margin-bottom: 1rem;
        padding-bottom: 0.5rem;
        border-bottom: 1px solid var(--border);
    }
    .settings-form .form-section .admin-form {
        gap: 1rem;
    }

    /* ─── Actions ─── */
    .page-actions {
        display: flex;
        justify-content: flex-end;
        gap: 0.75rem;
        margin-bottom: 1rem;
    }
    .table-actions {
        display: flex;
        gap: 0.5rem;
    }
    .table-actions .sc-btn {
        padding: 0.3rem 0.8rem;
        font-size: 0.75rem;
    }

    /* ─── Empty State ─── */
    .empty-state {
        text-align: center;
        padding: 3rem 1rem;
        color: var(--text-muted);
        font-size: 0.95rem;
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
        "{}\n{}\n{}\n{}\n{}\n{}\n{}",
        BUTTON_STYLES,
        CARD_STYLES,
        TOAST_STYLES,
        MODAL_STYLES,
        CONFIRM_DIALOG_STYLES,
        FORM_MODAL_STYLES,
        ADMIN_STYLES,
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
                                <Route path=path!("/games") view=GamesPage/>
                                <Route path=path!("/teams") view=TeamsPage/>
                                <Route path=path!("/schedule") view=SchedulePage/>
                                <Route path=path!("/applications") view=ApplicationsPage/>
                                <Route path=path!("/matches") view=MatchesPage/>
                                <Route path=path!("/announcements") view=AnnouncementsPage/>
                                <Route path=path!("/audit-log") view=AuditLogPage/>
                                <Route path=path!("/settings") view=SettingsPage/>
                                <Route path=path!("/tournaments") view=TournamentsPage/>
                            </Routes>
                        </main>
                    </div>
                    <ToastContainer/>
                </Router>
            </RequireOfficer>
        </ThemeProvider>
    }
}
