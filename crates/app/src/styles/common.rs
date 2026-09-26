/// Styles shared across both admin and public pages:
/// status/role pills, empty state, loading state, animations.
pub const CSS: &str = r#"
    /* Status / Role pills */
    .status-pill {
        display: inline-block; padding: 0.15rem 0.5rem; border-radius: 999px;
        font-size: 0.65rem; font-weight: 700; text-transform: uppercase; letter-spacing: 0.04em;
    }
    .status-pill.pending { background: color-mix(in srgb, var(--warn) 20%, transparent); color: var(--warn); }
    .status-pill.active, .status-pill.accepted { background: color-mix(in srgb, var(--ok) 20%, transparent); color: var(--ok); }
    .status-pill.inactive, .status-pill.rejected { background: color-mix(in srgb, var(--danger) 20%, transparent); color: var(--danger); }
    .status-pill.trial { background: color-mix(in srgb, var(--accent) 20%, transparent); color: var(--accent); }
    .status-pill.draft { background: color-mix(in srgb, var(--text-3) 20%, transparent); color: var(--text-3); }
    .status-pill.registration { background: color-mix(in srgb, var(--accent) 20%, transparent); color: var(--accent); }
    .status-pill.completed { background: color-mix(in srgb, var(--ok) 20%, transparent); color: var(--ok); }
    .status-pill.in_progress { background: color-mix(in srgb, var(--warn) 20%, transparent); color: var(--warn); }
    .status-pill.withdrawn { background: color-mix(in srgb, var(--text-3) 20%, transparent); color: var(--text-3); }

    .role-pill {
        display: inline-block; padding: 0.15rem 0.5rem; border-radius: 999px;
        font-size: 0.65rem; font-weight: 700; text-transform: uppercase; letter-spacing: 0.04em;
    }
    /* Roles are rank, not status: brand/neutral tints, never ok/warn/danger.
       Text stays --text/--text-2 so it clears AA whatever the org accent is. */
    .role-pill.admin { background: var(--accent); color: var(--accent-fg); }
    .role-pill.officer { background: var(--accent-soft); color: var(--text); box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--accent) 45%, transparent); }
    .role-pill.member { background: var(--surface-2); color: var(--text-2); box-shadow: inset 0 0 0 1px var(--border); }
    .role-pill.recruit { background: transparent; color: var(--text-2); box-shadow: inset 0 0 0 1px var(--border); }

    /* Empty state */
    .empty-state { color: var(--text-3); text-align: center; padding: 3rem 1rem; font-size: 0.9rem; }

    /* Loading */
    .loading-state, .admin-loading { color: var(--text-3); padding: 2rem; font-size: 0.9rem; }

    @keyframes fade-in { from { opacity: 0; } to { opacity: 1; } }
    @keyframes slide-up { from { transform: translateY(10px); opacity: 0; } to { transform: translateY(0); opacity: 1; } }

    .fetch-error-wrap { text-align: center; }
    .fetch-error {
        color: var(--danger);
        text-align: center;
        padding: 2rem 1rem 0.75rem;
        margin: 0;
    }
    .fetch-error__retry {
        display: inline-flex;
        margin: 0.25rem auto 2rem;
        background: transparent;
        border: 1px solid var(--border);
        color: var(--text);
        border-radius: 6px;
        padding: 0.35rem 0.8rem;
        font: inherit;
        cursor: pointer;
    }

    .list-cap-notice {
        color: var(--text-2);
        font-size: 0.85rem;
        padding: 0.75rem 0;
        display: flex;
        align-items: center;
        gap: 0.6rem;
        flex-wrap: wrap;
    }
    .list-cap-notice__more {
        background: transparent;
        border: 1px solid var(--border);
        color: var(--text);
        border-radius: 6px;
        padding: 0.3rem 0.7rem;
        font: inherit;
        cursor: pointer;
    }

    /* One keyboard focus ring for every control; mouse clicks stay clean.
       --text, not --accent: org accents can sit under 3:1 on the page bg. */
    :where(a, button, input, select, textarea, summary, [tabindex]):focus-visible {
        outline: 2px solid var(--text);
        outline-offset: 2px;
    }

    @media (prefers-reduced-motion: reduce) {
        *, *::before, *::after {
            animation-duration: 0.01ms !important;
            animation-iteration-count: 1 !important;
            transition-duration: 0.01ms !important;
            scroll-behavior: auto !important;
        }
    }
"#;
