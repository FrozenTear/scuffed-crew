/// Styles shared across both admin and public pages:
/// empty state, loading state, animations, focus ring.
pub const CSS: &str = r#"
    /* Status and role pills (.status-pill, .role-pill) are styled with the
       badge recipe in components/ui/pill.rs (PILL_CSS). */

    /* Empty state */
    .empty-state { color: var(--text-subtle); text-align: center; padding: 3rem 1rem; font-size: 0.9rem; }

    /* Loading */
    .loading-state, .admin-loading { color: var(--text-subtle); padding: 2rem; font-size: 0.9rem; }

    @keyframes fade-in { from { opacity: 0; } to { opacity: 1; } }
    @keyframes slide-up { from { transform: translateY(10px); opacity: 0; } to { transform: translateY(0); opacity: 1; } }

    .brand-pending {
        display: inline-block;
        width: 5.5rem;
        height: 0.7rem;
        border-radius: 4px;
        background: var(--surface-sunken);
        vertical-align: middle;
    }
    .name-field-error {
        color: var(--loss);
        font-size: 0.85rem;
        margin: 0.35rem 0 0;
    }
    /* Text for assistive tech only. Still in the tree, out of the layout. */
    .visually-hidden {
        position: absolute;
        width: 1px;
        height: 1px;
        padding: 0;
        margin: -1px;
        overflow: hidden;
        clip: rect(0, 0, 0, 0);
        white-space: nowrap;
        border: 0;
    }

    .fetch-error-wrap { text-align: center; }
    .fetch-error {
        color: var(--loss);
        text-align: center;
        padding: var(--space-section) var(--space-4) var(--space-3);
        margin: 0;
        font-size: var(--text-md);
        line-height: var(--leading-body);
    }
    /* Retry and Load more: outlined buttons on the button tokens, 44px tall
       at every width. */
    .fetch-error__retry,
    .list-cap-notice__more {
        box-sizing: border-box;
        display: inline-flex;
        align-items: center;
        justify-content: center;
        min-height: var(--button-height);
        min-width: var(--hit-target-min);
        padding: 0 var(--button-pad-x);
        background: transparent;
        border: 1px solid var(--button-secondary-border);
        border-radius: var(--button-radius);
        color: var(--text);
        font-family: var(--button-font);
        font-size: var(--button-font-size);
        font-weight: var(--button-font-weight);
        line-height: var(--leading-ui);
        cursor: pointer;
    }
    .fetch-error__retry:hover,
    .list-cap-notice__more:hover { border-color: var(--text); }
    .fetch-error__retry { margin: var(--space-1) auto var(--space-section); }
    /* In a redesigned page column (`PageShell`) every block starts at the
       left edge, under the left aligned title. The failed state does too,
       and it takes its spacing from the column, not from a section pad. */
    .ui-shell .fetch-error-wrap { text-align: start; }
    .ui-shell .fetch-error {
        text-align: start;
        padding: 0 0 var(--space-3);
    }
    .ui-shell .fetch-error__retry:not(.is-compact) { margin: 0; }
    /* In-panel retries. The page-level button keeps the section margin. */
    .fetch-error__retry.is-compact { margin: var(--space-1) 0 0; }
    /* Retry stays focusable while a retry runs ("Retrying..."). */
    .fetch-error__retry:disabled,
    .fetch-error__retry[aria-disabled="true"] {
        opacity: 0.55;
        color: var(--text-subtle);
        cursor: not-allowed;
    }

    .list-cap-notice {
        display: flex;
        flex-wrap: wrap;
        align-items: center;
        gap: var(--space-2);
        padding: var(--space-3) 0;
        color: var(--text-3);
        font-size: var(--text-sm);
        line-height: var(--leading-body);
    }
    .list-cap-notice__text { margin: 0; }
    /* Nothing was cut off: out of the layout, still in the tree so the next
       text is announced. */
    .list-cap-notice.is-idle {
        position: absolute;
        width: 1px;
        height: 1px;
        padding: 0;
        margin: -1px;
        overflow: hidden;
        clip: rect(0, 0, 0, 0);
        white-space: nowrap;
        border: 0;
    }

    @media (prefers-reduced-motion: reduce) {
        *, *::before, *::after {
            animation-duration: 0.01ms !important;
            animation-iteration-count: 1 !important;
            transition-duration: 0.01ms !important;
            scroll-behavior: auto !important;
        }
    }

    /* Keyboard focus. :focus-visible stays off for mouse clicks. */
    a:focus-visible,
    button:focus-visible,
    input:focus-visible,
    select:focus-visible,
    textarea:focus-visible,
    [tabindex]:focus-visible {
        outline: 2px solid var(--focus-ring);
        outline-offset: 2px;
    }
"#;

#[cfg(test)]
mod tests {
    use super::CSS;
    use crate::components::ui::test_support::{css_rule, has_decl};

    /// Retry and Load more are shared by every page, so their 44px target and
    /// token sizes live here and not in per-page overrides.
    #[test]
    fn retry_and_load_more_are_44px_buttons_on_the_button_tokens() {
        let rule = css_rule(CSS, ".fetch-error__retry,\n    .list-cap-notice__more {");
        for decl in [
            "min-height: var(--button-height)",
            "min-width: var(--hit-target-min)",
            "padding: 0 var(--button-pad-x)",
            "border: 1px solid var(--button-secondary-border)",
            "border-radius: var(--button-radius)",
            "font-size: var(--button-font-size)",
        ] {
            assert!(has_decl(&rule, decl), "missing {decl}: {rule}");
        }
        for marker in [".fetch-error {", ".list-cap-notice {"] {
            let rule = css_rule(CSS, marker);
            assert!(!rule.contains("rem"), "{marker}: {rule}");
            assert!(rule.contains("font-size: var(--text-"), "{marker}: {rule}");
        }
        // --button-height is the 44px hit target at every width.
        let tokens = include_str!("../theme/css/components.css");
        assert!(tokens.contains("--button-height: var(--hit-target-min);"));
    }

    #[test]
    fn idle_cap_notice_and_hidden_text_stay_in_the_tree() {
        for marker in [".visually-hidden {", ".list-cap-notice.is-idle {"] {
            let rule = css_rule(CSS, marker);
            assert!(has_decl(&rule, "position: absolute"), "{marker}: {rule}");
            assert!(!rule.contains("display"), "{marker}: {rule}");
        }
    }
}
