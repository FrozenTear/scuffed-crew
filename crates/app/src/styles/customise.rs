//! Shared form styles for the two customisation pages: admin brand settings
//! and the member appearance page. Semantic and component tokens only, so
//! theme, density and the brand accent restyle both pages.

pub const CUSTOMISE_CSS_ID: &str = "scuffed-css-customise";

pub const CUSTOMISE_CSS: &str = r#"
    .customise {
        width: 100%;
        max-width: var(--content-max-narrow);
        margin: 0 auto;
        display: flex;
        flex-direction: column;
        gap: var(--space-section);
        color: var(--text);
    }
    .customise.is-page { padding: var(--space-section) var(--space-page); }
    .customise p { margin: 0; }
    .customise .customise-header h1 {
        margin: 0;
        font-family: var(--font-head);
        font-weight: var(--font-head-weight);
        letter-spacing: var(--font-head-tracking);
        line-height: var(--leading-display);
        font-size: var(--text-3xl);
        color: var(--text);
    }
    .customise .customise-lead {
        margin-top: var(--space-2);
        color: var(--text-3);
        font-size: var(--text-md);
    }
    .customise-card {
        background: var(--card-bg);
        color: var(--card-fg);
        border: 1px solid var(--card-border);
        border-radius: var(--card-radius);
        box-shadow: var(--card-shadow);
        padding: var(--card-padding);
        display: flex;
        flex-direction: column;
        gap: var(--space-4);
    }
    .customise .customise-kicker {
        font-family: var(--font-head);
        font-weight: var(--font-label-weight);
        font-size: var(--text-xs);
        letter-spacing: var(--font-label-tracking);
        text-transform: uppercase;
        color: var(--accent-text);
    }
    .customise .customise-title {
        margin: 0;
        font-family: var(--font-head);
        font-weight: var(--font-head-weight);
        letter-spacing: var(--font-head-tracking);
        line-height: var(--leading-display);
        font-size: var(--text-xl);
        color: var(--text);
    }
    .customise .customise-hint {
        color: var(--text-3);
        font-size: var(--text-sm);
    }

    .customise-choice-row {
        display: flex;
        flex-wrap: wrap;
        gap: var(--space-2);
    }
    .customise-choice {
        position: relative;
        min-height: var(--hit-target-min);
        min-width: var(--hit-target-min);
        padding: 0 var(--space-4);
        border-radius: var(--radius-control);
        border: 1px solid var(--border-strong);
        background: var(--surface);
        color: var(--text);
        font-family: var(--font-head);
        font-size: var(--text-sm);
        font-weight: var(--font-label-weight);
        line-height: var(--leading-ui);
        display: inline-flex;
        align-items: center;
        justify-content: center;
        cursor: pointer;
    }
    /* The native radio covers the whole choice, so the label is the hit
       target and arrow keys move the selection. */
    .customise-choice input {
        position: absolute;
        inset: 0;
        width: 100%;
        height: 100%;
        margin: 0;
        opacity: 0;
        cursor: pointer;
    }
    .customise-choice.is-selected {
        background: var(--chip-selected-bg);
        color: var(--chip-selected-fg);
        border-color: var(--chip-selected-border);
        border-width: var(--chip-selected-border-width);
        font-weight: var(--chip-selected-weight);
    }
    .customise-choice:has(input:focus-visible) {
        outline: var(--focus-ring-width) solid var(--focus-ring);
        outline-offset: var(--focus-ring-offset);
    }
    .customise-choice.is-locked {
        color: var(--text-disabled);
        border-color: var(--border);
        cursor: not-allowed;
    }
    .customise-choice.is-locked input { cursor: not-allowed; }

    .customise-accent-row {
        display: flex;
        flex-wrap: wrap;
        align-items: flex-end;
        gap: var(--space-3);
    }
    .customise-swatch {
        width: var(--hit-target-min);
        height: var(--hit-target-min);
        border-radius: var(--radius-control);
        border: 1px solid var(--border-strong);
        background: var(--surface-sunken);
        flex: 0 0 auto;
        position: relative;
        overflow: hidden;
        display: inline-block;
    }
    .customise-swatch input[type="color"] {
        position: absolute;
        inset: 0;
        width: 100%;
        height: 100%;
        padding: 0;
        border: 0;
        opacity: 0;
        cursor: pointer;
    }
    .customise-swatch:has(input:focus-visible) {
        outline: var(--focus-ring-width) solid var(--focus-ring);
        outline-offset: var(--focus-ring-offset);
    }
    .customise-field {
        flex: 1 1 12rem;
        min-width: 0;
        display: flex;
        flex-direction: column;
        gap: var(--space-2);
    }
    .customise-field label {
        font-family: var(--font-head);
        font-size: var(--text-xs);
        font-weight: var(--font-label-weight);
        letter-spacing: var(--font-label-tracking);
        text-transform: uppercase;
        color: var(--text-subtle);
    }
    .customise-field input {
        width: 100%;
        height: var(--hit-target-min);
        padding: 0 var(--space-3);
        border-radius: var(--radius-control);
        border: 1px solid var(--border-strong);
        background: var(--surface);
        color: var(--text);
        font-family: var(--font-body);
        font-size: var(--text-md);
    }
    .customise-field input[aria-invalid="true"] { border-color: var(--loss); }

    .customise-contrast-grid {
        display: grid;
        grid-template-columns: 1fr 1fr;
        gap: var(--space-3);
    }
    .customise-contrast-card {
        background: var(--bg);
        color: var(--text);
        border: 1px solid var(--border-strong);
        border-radius: var(--card-radius);
        padding: var(--card-padding);
    }
    .customise .customise-contrast-card h3 {
        margin: 0 0 var(--space-2);
        font-family: var(--font-head);
        font-weight: var(--font-head-weight);
        font-size: var(--text-md);
        color: var(--text);
    }
    .customise-preview-btn {
        height: var(--button-height);
        padding: 0 var(--button-pad-x);
        margin-bottom: var(--space-3);
        border-radius: var(--button-radius);
        border: 1px solid var(--accent-solid);
        background: var(--accent-solid);
        color: var(--on-accent);
        font-family: var(--button-font);
        font-size: var(--button-font-size);
        font-weight: var(--button-font-weight);
        display: inline-flex;
        align-items: center;
    }
    .customise .customise-contrast-sample {
        margin: 0 0 var(--space-3);
        color: var(--accent-text);
        background: var(--bg);
    }
    .customise-contrast-line {
        display: flex;
        justify-content: space-between;
        gap: var(--space-3);
        font-size: var(--text-sm);
        min-height: var(--space-6);
        align-items: center;
    }
    .customise-contrast-line .is-pass,
    .customise-contrast-line .is-fail {
        display: inline-flex;
        align-items: center;
        gap: var(--space-2);
        font-weight: var(--font-label-weight);
        white-space: nowrap;
    }
    .customise-contrast-line .is-pass { color: var(--text); }
    .customise-contrast-line .is-fail { color: var(--loss); }
    /* A warning triangle, so a failing pair is a shape and a word as well
       as a colour. */
    .customise-fail-icon {
        width: 0;
        height: 0;
        border-left: var(--space-2) solid transparent;
        border-right: var(--space-2) solid transparent;
        border-bottom: var(--space-4) solid var(--loss);
        flex: 0 0 auto;
    }
    .customise .customise-reason {
        color: var(--text);
        font-size: var(--text-sm);
    }
    /* The live region stays in the tree when it has nothing to say; this
       only takes back the card gap it would otherwise add. */
    .customise .customise-reason:empty { margin-top: calc(-1 * var(--space-4)); }

    .customise-actions {
        display: flex;
        flex-wrap: wrap;
        gap: var(--space-2);
        align-items: center;
    }
    .customise-btn {
        height: var(--button-height);
        min-height: var(--hit-target-min);
        min-width: var(--hit-target-min);
        padding: 0 var(--button-pad-x);
        border-radius: var(--button-radius);
        border: 1px solid var(--button-secondary-border);
        background: transparent;
        color: var(--text);
        font-family: var(--button-font);
        font-size: var(--button-font-size);
        font-weight: var(--button-font-weight);
        display: inline-flex;
        align-items: center;
        justify-content: center;
        cursor: pointer;
    }
    .customise-btn.is-primary {
        background: var(--button-primary-bg);
        color: var(--button-primary-fg);
        border-color: var(--button-primary-border);
    }
    /* Blocked, not disabled: the button keeps focus so the reason it points
       at is read out. The click handler does the blocking. */
    .customise-btn[aria-disabled="true"] {
        background: var(--surface);
        color: var(--text-disabled);
        border-color: var(--border);
        cursor: not-allowed;
    }
    .customise-btn:focus-visible {
        outline: var(--focus-ring-width) solid var(--focus-ring);
        outline-offset: var(--focus-ring-offset);
    }
    .customise .customise-status {
        color: var(--text-3);
        font-size: var(--text-sm);
        min-height: var(--space-5);
    }

    .customise-links {
        list-style: none;
        margin: 0;
        padding: 0;
        display: flex;
        flex-wrap: wrap;
        gap: 0 var(--space-5);
    }
    .customise-links a {
        min-height: var(--hit-target-min);
        display: inline-flex;
        align-items: center;
        color: var(--link);
        font-size: var(--text-md);
    }

    @media (max-width: 640px) {
        .customise-contrast-grid { grid-template-columns: 1fr; }
    }
"#;
