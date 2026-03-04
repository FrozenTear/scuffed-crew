/// Styles shared across both admin and public pages:
/// status/role pills, empty state, loading state, animations.
pub const CSS: &str = r#"
    /* Status / Role pills */
    .status-pill {
        display: inline-block; padding: 0.15rem 0.5rem; border-radius: 999px;
        font-size: 0.65rem; font-weight: 700; text-transform: uppercase; letter-spacing: 0.04em;
    }
    .status-pill.pending { background: #f59e0b33; color: #fbbf24; }
    .status-pill.active, .status-pill.accepted { background: #10b98133; color: #34d399; }
    .status-pill.inactive, .status-pill.rejected { background: #ef444433; color: #f87171; }
    .status-pill.trial { background: #3b82f633; color: #60a5fa; }
    .status-pill.draft { background: #6b728033; color: #9ca3af; }
    .status-pill.registration { background: #8b5cf633; color: #a78bfa; }
    .status-pill.completed { background: #10b98133; color: #34d399; }
    .status-pill.in_progress { background: #f59e0b33; color: #fbbf24; }
    .status-pill.withdrawn { background: #6b728033; color: #9ca3af; }

    .role-pill {
        display: inline-block; padding: 0.15rem 0.5rem; border-radius: 999px;
        font-size: 0.65rem; font-weight: 700; text-transform: uppercase; letter-spacing: 0.04em;
    }
    .role-pill.admin { background: #ef444433; color: #f87171; }
    .role-pill.officer { background: #f9731633; color: #f97316; }
    .role-pill.member { background: #7c3aed33; color: #a78bfa; }
    .role-pill.recruit { background: #6b728033; color: #9ca3af; }

    /* Empty state */
    .empty-state { color: var(--text-muted); text-align: center; padding: 3rem 1rem; font-size: 0.9rem; }

    /* Loading */
    .loading-state, .admin-loading { color: var(--text-muted); padding: 2rem; font-size: 0.9rem; }

    @keyframes fade-in { from { opacity: 0; } to { opacity: 1; } }
    @keyframes slide-up { from { transform: translateY(10px); opacity: 0; } to { transform: translateY(0); opacity: 1; } }
"#;
