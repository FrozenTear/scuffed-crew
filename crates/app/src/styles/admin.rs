/// Shared admin styles: the list pattern (toolbar, table, phone card rows,
/// row actions and the overflow disclosure), the form pattern (fields,
/// checkbox rows, save line), dialogs, summary tiles and the settings blocks.
///
/// Everything is on the design tokens: spacing from `--space-*` and the
/// density tokens, type from `--text-*`, colour from the semantic and
/// component tokens. The phone breakpoint is the shell's (820px).
///
/// New code uses the shared `.btn` recipe (`btn is-primary`, `btn is-ghost`,
/// `btn is-danger`) and the `.field-*` classes. The older names (`row-btn`,
/// `btn-add`, `btn-save`, `form-input`, ...) are kept as aliases on the same
/// tokens, so a page that still uses them looks the same.
pub const CSS: &str = r#"
    /* ------------------------------------------------------------------
       Buttons. `.btn` and its danger variant live in
       components/ui/button.rs. The older admin names share its recipe.
       ------------------------------------------------------------------ */
    .row-btn,
    .btn-add,
    .btn-ghost,
    .btn-cancel,
    .btn-save {
        box-sizing: border-box;
        display: inline-flex;
        align-items: center;
        justify-content: center;
        gap: var(--space-2);
        min-height: var(--button-height);
        min-width: var(--hit-target-min);
        margin: 0;
        padding: var(--space-2) var(--button-pad-x);
        border: 1px solid var(--border-strong);
        border-radius: var(--button-radius);
        background: transparent;
        color: var(--text);
        font-family: var(--button-font);
        font-size: var(--button-font-size);
        font-weight: var(--button-font-weight);
        letter-spacing: var(--font-head-tracking);
        line-height: var(--leading-ui);
        text-align: center;
        text-decoration: none;
        white-space: nowrap;
        cursor: pointer;
    }
    .row-btn:hover,
    .btn-ghost:hover,
    .btn-cancel:hover { border-color: var(--text); }
    .row-btn.primary,
    .btn-add,
    .btn-save {
        background: var(--button-primary-bg);
        color: var(--button-primary-fg);
        border-color: var(--button-primary-border);
    }
    .row-btn.primary:hover,
    .btn-add:hover,
    .btn-save:hover {
        background: var(--button-primary-hover-bg);
        text-decoration: underline;
    }
    .row-btn.danger,
    .btn-save.danger {
        background: var(--button-danger-bg);
        color: var(--button-danger-fg);
        border-color: var(--button-danger-border);
    }
    .row-btn.danger:hover,
    .btn-save.danger:hover {
        background: var(--button-danger-bg);
        border-color: var(--button-danger-fg);
        text-decoration: underline;
    }
    /* A row-btn that is a link (a Link renders an anchor). Centered like the
       buttons beside it. */
    .row-btn.row-btn-link {
        display: inline-flex;
        align-items: center;
        justify-content: center;
        box-sizing: border-box;
        text-decoration: none;
    }
    .row-btn:focus-visible,
    .btn-add:focus-visible,
    .btn-ghost:focus-visible,
    .btn-cancel:focus-visible,
    .btn-save:focus-visible {
        outline: var(--focus-ring-width) solid var(--focus-ring);
        outline-offset: var(--focus-ring-offset);
    }
    .row-btn:disabled,
    .btn-add:disabled,
    .btn-save:disabled,
    .btn:disabled {
        opacity: 0.6;
        cursor: not-allowed;
    }

    /* ------------------------------------------------------------------
       List pattern: toolbar, table, phone card rows, row actions.
       ------------------------------------------------------------------ */

    /* Filter chips on the left, search or the page action on the right. */
    .admin-toolbar {
        display: flex;
        flex-wrap: wrap;
        align-items: center;
        justify-content: space-between;
        gap: var(--space-3);
        min-width: 0;
    }
    .admin-toolbar-search { flex: 1 1 auto; min-width: 0; }
    .admin-toolbar-actions {
        display: flex;
        flex-wrap: wrap;
        align-items: center;
        gap: var(--space-2);
    }
    .admin-toolbar select {
        box-sizing: border-box;
        min-height: var(--hit-target-min);
        padding: var(--space-2) var(--space-3);
        border: 1px solid var(--border-strong);
        border-radius: var(--radius-control);
        background: var(--surface);
        color: var(--text);
        font-family: var(--font-body);
        font-size: var(--text-md);
    }

    /* The list region of a page: status line, table, pager. */
    .admin-list {
        display: flex;
        flex-direction: column;
        gap: var(--space-stack);
        min-width: 0;
    }

    /* Loading and failed loads (`admin_pending`). */
    .admin-pending { min-width: 0; }
    .admin-pending-text {
        margin: 0;
        color: var(--text-3);
        font-family: var(--font-body);
        font-size: var(--text-md);
        line-height: var(--leading-body);
    }

    .data-table-scroll {
        box-sizing: border-box;
        width: 100%;
        max-width: 100%;
        overflow-x: auto;
        background: var(--table-row-bg);
        border: 1px solid var(--card-border);
        border-radius: var(--table-radius);
        box-shadow: var(--card-shadow);
    }
    .data-table {
        width: 100%;
        border-collapse: collapse;
        font-family: var(--table-font);
        font-size: var(--text-sm);
        line-height: var(--leading-ui);
        color: var(--table-row-fg);
    }
    /* A table that does not turn into cards keeps its columns readable and
       scrolls sideways inside its own box. */
    .data-table-scroll:not(.is-cards) .data-table { min-width: var(--container-sm); }
    .data-table th {
        box-sizing: border-box;
        height: var(--table-row-height);
        padding: 0 var(--table-pad-x);
        border-bottom: 1px solid var(--table-row-border);
        background: var(--table-row-bg);
        color: var(--table-header-fg);
        font-family: var(--font-head);
        font-size: var(--text-xs);
        font-weight: var(--table-header-weight);
        letter-spacing: var(--font-head-tracking);
        text-align: start;
        text-transform: uppercase;
        white-space: nowrap;
    }
    .data-table td {
        box-sizing: border-box;
        height: var(--table-row-height);
        padding: var(--space-1) var(--table-pad-x);
        border-bottom: 1px solid var(--table-row-border);
        color: var(--table-row-fg);
        vertical-align: middle;
        overflow-wrap: anywhere;
    }
    .data-table tbody tr:last-child td { border-bottom: 0; }
    .data-table tbody tr:hover td { background: var(--table-row-hover); }
    /* Numbers and dates: right aligned, like their header. */
    .data-table th.num,
    .data-table td.num {
        text-align: end;
        font-variant-numeric: tabular-nums;
    }
    .data-table td.num { color: var(--table-row-muted); }
    /* The cell that names the row. */
    .data-table td.cell-who {
        color: var(--text);
        font-weight: var(--font-label-weight);
    }
    .data-table td.cell-actions { overflow-wrap: normal; }
    /* A date never drops its year to the next line. */
    .date { white-space: nowrap; }
    /* An inactive record: a soft fill. The row also says so in a pill. */
    .data-table tr.is-inactive td { background: var(--loss-soft); }

    /* Row actions: one row on desktop. `RowActions` renders the always
       visible actions in `.row-actions-main` and the rest in
       `.row-menu-group`, behind the "Actions" button. */
    .row-actions {
        display: flex;
        flex-direction: column;
        align-items: stretch;
        gap: var(--space-2);
        min-width: 0;
    }
    .row-actions-main {
        display: flex;
        flex-wrap: nowrap;
        align-items: center;
        gap: var(--space-2);
    }
    /* Older markup: buttons straight inside `.row-actions`. */
    .row-actions:not(:has(> .row-actions-main)) {
        flex-direction: row;
        flex-wrap: wrap;
        align-items: center;
    }
    /* The overflow group. Closed, it is out of the layout and stays in the
       tree, so the button's aria-controls always points at it. */
    .row-menu-group {
        display: none;
        flex-direction: column;
        align-items: stretch;
        gap: var(--space-2);
        min-width: 0;
    }
    .row-actions.is-open > .row-menu-group { display: flex; }
    .row-menu-toggle[aria-expanded="true"] {
        background: var(--accent-soft);
        color: var(--accent-text);
        border-color: var(--accent-text);
    }
    /* The marker turns with the state, so open is not colour alone. */
    .row-menu-toggle::after {
        content: "";
        box-sizing: border-box;
        width: var(--space-2);
        height: var(--space-2);
        margin-inline-start: var(--space-1);
        border-inline-end: var(--focus-ring-width) solid currentColor;
        border-bottom: var(--focus-ring-width) solid currentColor;
        transform: translateY(calc(var(--space-1) / -2)) rotate(45deg);
    }
    .row-menu-toggle[aria-expanded="true"]::after {
        transform: translateY(calc(var(--space-1) / 2)) rotate(-135deg);
    }
    /* A short action set (`inline_on_desktop`): on desktop every action is in
       the row and there is no "Actions" button. Phone folds the extra ones. */
    @media (min-width: 821px) {
        .row-actions.is-inline-desktop,
        .row-actions.is-inline-desktop > .row-menu-group {
            display: flex;
            flex-direction: row;
            flex-wrap: nowrap;
            align-items: center;
        }
        .row-actions.is-inline-desktop .row-menu-toggle { display: none; }
        /* A long overflow set opened on desktop: compact buttons that wrap
           in the cell, and the row's other cells stay level with the
           Actions button instead of floating to the middle of a tall row. */
        .row-actions:not(.is-inline-desktop) > .row-menu-group {
            flex-direction: row;
            flex-wrap: wrap;
            align-items: center;
            /* Capped, so the open group wraps instead of widening the
               Actions column and squeezing the name and status columns. */
            max-width: calc(var(--hit-target-min) * 8);
        }
        .data-table tr:has(.row-actions.is-open:not(.is-inline-desktop)) td {
            vertical-align: top;
        }
        /* Text cells drop to the middle of the 44px Actions button. */
        .data-table tr:has(.row-actions.is-open:not(.is-inline-desktop))
            td:not(.cell-actions):not(.cell-select) {
            padding-top: calc(var(--space-1) + var(--space-3));
        }
    }

    /* Older page lists. New pages use the shared Pager. */
    .pagination {
        display: flex;
        flex-wrap: wrap;
        align-items: center;
        gap: var(--space-2);
    }
    .pagination button {
        box-sizing: border-box;
        min-width: var(--hit-target-min);
        min-height: var(--hit-target-min);
        padding: 0 var(--space-3);
        border: 1px solid var(--border-strong);
        border-radius: var(--radius-control);
        background: var(--surface);
        color: var(--text);
        font-family: var(--font-head);
        font-size: var(--text-sm);
        font-weight: var(--font-label-weight);
        cursor: pointer;
    }
    .pagination button:hover:not(:disabled) { border-color: var(--text); }
    .pagination button:disabled {
        color: var(--text-disabled);
        border-color: var(--border);
        cursor: not-allowed;
    }
    .pagination .page-info { color: var(--text-3); font-size: var(--text-sm); }

    @media (max-width: 820px) {
        .admin-toolbar {
            flex-direction: column;
            align-items: stretch;
        }
        .admin-toolbar-search,
        .admin-toolbar-search .field-input { width: 100%; }

        /* Phone: every row of a `cards` table is a stacked card. The name
           and the first actions share the top line, the status badge has a
           line of its own, the meta cells follow on one line, and any other
           cell is a full line that carries its column name. */
        .data-table-scroll.is-cards {
            overflow: visible;
            background: transparent;
            border: 0;
            border-radius: 0;
            box-shadow: none;
        }
        .is-cards .data-table,
        .is-cards .data-table tbody { display: block; }
        /* The header row is out of the layout, still in the tree. */
        .is-cards .data-table thead {
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
        .is-cards .data-table tr {
            display: flex;
            flex-wrap: wrap;
            align-items: center;
            column-gap: var(--space-2);
            padding: var(--space-3);
            background: var(--card-bg);
            color: var(--card-fg);
            border: 1px solid var(--card-border);
            border-radius: var(--card-radius);
        }
        /* Ends the top line (name and first actions), whatever follows. */
        .is-cards .data-table tr::after {
            content: "";
            order: 2;
            flex: 0 0 100%;
            height: 0;
        }
        .is-cards .data-table tr + tr { margin-top: var(--space-3); }
        .is-cards .data-table tr.is-inactive { background: var(--loss-soft); }
        .is-cards .data-table tbody tr:hover td,
        .is-cards .data-table tr.is-inactive td { background: transparent; }
        .is-cards .data-table td {
            display: block;
            order: 5;
            flex: 0 0 100%;
            height: auto;
            min-width: 0;
            margin-top: var(--space-2);
            padding: 0;
            border: 0;
            color: var(--text-3);
            font-size: var(--text-sm);
            text-align: start;
        }
        .is-cards .data-table td:empty { display: none; }
        .is-cards .data-table td[data-label]::before {
            content: attr(data-label) ": ";
            color: var(--text-3);
            font-weight: var(--font-label-weight);
        }
        .is-cards .data-table td.cell-who {
            order: 1;
            flex: 1 1 30%;
            margin-top: 0;
            color: var(--text);
            font-family: var(--font-head);
            font-size: var(--text-md);
            font-weight: var(--font-head-weight);
        }
        /* A row checkbox (bulk select) leads the top line. */
        .is-cards .data-table td.cell-select {
            order: 0;
            flex: 0 0 auto;
            margin-top: 0;
        }
        .is-cards .data-table td.cell-badge { order: 3; }
        .is-cards .data-table td.cell-meta {
            order: 4;
            flex: 0 1 auto;
        }
        /* The dot ends a fact that has another after it, so when the line
           wraps the dot stays at the end of the line above and the next
           line never starts with a lone dot. */
        .is-cards .data-table td.cell-meta:not(:empty):has(~ td.cell-meta:not(:empty))::after {
            content: "\00B7";
            margin-inline-start: var(--space-2);
            color: var(--text-3);
            font-weight: var(--font-body-weight);
        }
        /* The action cell gives its buttons to the card: the always visible
           ones sit beside the name, and the open overflow group is the last
           line, where a long action set wraps instead of clipping. */
        .is-cards .data-table td.cell-actions,
        .is-cards .data-table td.cell-actions > .row-actions,
        .is-cards .data-table td.cell-actions .row-actions-main { display: contents; }
        .is-cards .data-table td.cell-actions .row-actions > *,
        .is-cards .data-table td.cell-actions .row-actions-main > * {
            order: 2;
            flex: 0 0 auto;
        }
        .is-cards .data-table td.cell-actions .row-menu-group {
            order: 8;
            flex: 0 0 100%;
            flex-direction: row;
            flex-wrap: wrap;
            align-items: center;
            margin-top: var(--space-3);
            padding-top: var(--space-3);
            border-top: 1px solid var(--card-border);
        }
    }

    /* ------------------------------------------------------------------
       Dialogs: FormModal, ConfirmDialog and the application detail.
       ------------------------------------------------------------------ */
    .form-modal-overlay {
        position: fixed;
        inset: 0;
        z-index: 1000;
        display: flex;
        align-items: center;
        justify-content: center;
        padding: var(--space-4);
        background: var(--overlay);
        animation: fade-in 0.15s ease-out;
    }
    .form-modal {
        box-sizing: border-box;
        width: 100%;
        max-width: var(--container-sm);
        max-height: 100%;
        overflow-y: auto;
        background: var(--surface-raised);
        color: var(--text);
        border: 1px solid var(--card-border);
        border-radius: var(--card-radius);
        box-shadow: var(--shadow-overlay);
        animation: slide-up 0.2s ease-out;
    }
    .form-modal.wide { max-width: var(--container-md); }
    /* The application detail stays inside a 390px viewport: the overlay pads
       it by --space-4 on each side. */
    .form-modal.application-detail-modal {
        width: 100%;
        max-width: var(--container-sm);
    }
    .form-modal-header {
        margin: 0;
        padding: var(--space-5) var(--space-5) var(--space-4);
        border-bottom: 1px solid var(--border);
        color: var(--text);
        font-family: var(--font-head);
        font-size: var(--text-xl);
        font-weight: var(--font-head-weight);
        letter-spacing: var(--font-head-tracking);
        text-transform: var(--font-head-transform);
        line-height: var(--leading-display);
        overflow-wrap: anywhere;
    }
    .form-modal-header:focus-visible {
        outline: var(--focus-ring-width) solid var(--focus-ring);
        outline-offset: calc(-1 * var(--focus-ring-width));
    }
    .form-modal-body {
        display: flex;
        flex-direction: column;
        padding: var(--space-5);
    }
    .form-modal-body > * + * { margin-top: var(--space-5); }
    .form-modal-body > .form-checkbox-row + .form-checkbox-row,
    .form-modal-body > .admin-check-row + .admin-check-row { margin-top: var(--space-3); }
    .confirm-body {
        padding: var(--space-5);
        color: var(--text-3);
        font-size: var(--text-md);
        line-height: var(--leading-body);
    }
    .confirm-body > p { margin: 0; }
    .confirm-body > * + * { margin-top: var(--space-5); }
    .form-modal-footer {
        display: flex;
        flex-wrap: wrap;
        align-items: center;
        justify-content: flex-end;
        gap: var(--space-2);
        padding: 0 var(--space-5) var(--space-5);
    }
    /* The dialog's live line: the save state and the server's reason for a
       failed save. Always in the tree; empty, it takes no room. */
    .form-modal-status {
        flex: 1 1 100%;
        margin: 0;
        color: var(--text-3);
        font-size: var(--text-sm);
        line-height: var(--leading-body);
        text-align: start;
    }
    .form-modal-status.is-error { color: var(--loss); }
    .form-modal-status:empty { display: none; }

    .application-detail dl {
        display: grid;
        grid-template-columns: minmax(0, 1fr) minmax(0, 3fr);
        column-gap: var(--space-4);
        row-gap: var(--space-3);
        margin: 0;
    }
    .application-detail dt {
        margin: 0;
        color: var(--text-3);
        font-family: var(--font-head);
        font-size: var(--text-xs);
        font-weight: var(--font-label-weight);
        letter-spacing: var(--font-head-tracking);
        text-transform: uppercase;
    }
    .application-detail dd {
        margin: 0;
        min-width: 0;
        color: var(--text);
        font-size: var(--text-md);
        overflow-wrap: anywhere;
    }
    .application-detail dd.application-detail-message {
        white-space: pre-wrap;
        overflow-wrap: anywhere;
    }
    @media (max-width: 820px) {
        .form-modal-footer > .btn,
        .form-modal-footer > .btn-cancel,
        .form-modal-footer > .btn-save { flex: 1 1 auto; }
        .application-detail dl { grid-template-columns: minmax(0, 1fr); row-gap: var(--space-1); }
        .application-detail dd + dt { margin-top: var(--space-3); }
    }

    /* ------------------------------------------------------------------
       Form pattern: label --space-2 above its control, --space-5 between
       fields, full-width checkbox rows --space-3 apart, Save --space-5 below
       the last field, and an always-rendered save line.
       ------------------------------------------------------------------ */
    .form-stack {
        display: flex;
        flex-direction: column;
        min-width: 0;
    }
    .form-stack > * + * { margin-top: var(--space-5); }
    .form-stack > .admin-check-row + .admin-check-row,
    .form-stack > .form-checkbox-row + .form-checkbox-row { margin-top: var(--space-3); }
    .form-field {
        display: flex;
        flex-direction: column;
        gap: var(--space-2);
        min-width: 0;
        margin: 0;
        padding: 0;
        border: 0;
    }
    /* `.form-label`, `.form-input`, `.form-select`, `.form-textarea` and
       `.form-hint` are the older admin names of the shared field classes in
       components/ui/field.rs (`.field-label`, `.field-input`, `.field-hint`)
       and carry the same values. New markup uses the shared names. */
    .form-label {
        display: block;
        min-height: 0;
        margin: 0;
        padding: 0;
        color: var(--text);
        font-family: var(--font-head);
        font-size: var(--text-sm);
        font-weight: var(--font-label-weight);
        line-height: var(--leading-ui);
    }
    .form-input,
    .form-select,
    .form-textarea {
        box-sizing: border-box;
        width: 100%;
        min-height: var(--hit-target-min);
        margin: 0;
        padding: var(--space-2) var(--space-3);
        border: 1px solid var(--border-strong);
        border-radius: var(--radius-control);
        background: var(--surface);
        color: var(--text);
        font-family: var(--font-body);
        font-size: var(--text-md);
        font-weight: var(--font-body-weight);
        line-height: var(--leading-body);
    }
    .form-textarea {
        min-height: calc(var(--hit-target-min) * 3);
        padding: var(--space-3);
        resize: vertical;
    }
    .form-input::placeholder,
    .form-textarea::placeholder { color: var(--text-subtle); opacity: 1; }
    .form-input:focus,
    .form-select:focus,
    .form-textarea:focus { border-color: var(--accent-solid); }
    .form-input[aria-invalid="true"],
    .form-select[aria-invalid="true"],
    .form-textarea[aria-invalid="true"] { border-color: var(--loss); }
    .form-input[type="date"], .form-input[type="time"] { position: relative; }
    .form-input[type="date"]::-webkit-calendar-picker-indicator,
    .form-input[type="time"]::-webkit-calendar-picker-indicator {
        position: absolute; inset: 0; width: auto; height: auto;
        color: transparent; background: transparent; cursor: pointer;
    }
    .form-hint {
        margin: 0;
        color: var(--text-3);
        font-size: var(--text-sm);
        line-height: var(--leading-body);
    }
    /* A checkbox and its label: one full-width row, 44px tall. */
    .admin-check-row,
    .form-checkbox-row {
        display: flex;
        align-items: center;
        gap: var(--space-3);
        width: 100%;
        max-width: 100%;
        min-height: var(--hit-target-min);
        color: var(--text);
        font-family: var(--font-body);
        font-size: var(--text-md);
        line-height: var(--leading-body);
        cursor: pointer;
    }
    .admin-check-row input[type="checkbox"],
    .form-checkbox-row input[type="checkbox"],
    .section-chip input[type="checkbox"] {
        flex: 0 0 auto;
        width: calc(var(--space-4) + var(--space-1) / 2);
        height: calc(var(--space-4) + var(--space-1) / 2);
        margin: 0;
        accent-color: var(--accent-solid);
        cursor: pointer;
    }
    .admin-check-row input[type="checkbox"]:focus-visible,
    .form-checkbox-row input[type="checkbox"]:focus-visible,
    .section-chip input[type="checkbox"]:focus-visible {
        outline: var(--focus-ring-width) solid var(--focus-ring);
        outline-offset: var(--focus-ring-offset);
    }
    /* A group of checkbox rows with one name (a fieldset). */
    .admin-check-group {
        display: flex;
        flex-direction: column;
        gap: var(--space-3);
        min-width: 0;
        margin: 0;
        padding: 0;
        border: 0;
    }
    .admin-check-group > legend {
        margin: 0 0 var(--space-2);
        padding: 0;
    }

    /* Two-column field grid (dialogs and settings). One column on phone. */
    .form-grid {
        display: grid;
        grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
        gap: var(--space-5) var(--space-4);
    }
    .form-grid .span-full { grid-column: 1 / -1; }

    /* Save row and its live line. */
    .admin-form-actions {
        display: flex;
        flex-wrap: wrap;
        align-items: center;
        gap: var(--space-3);
    }
    .admin-form-status {
        margin: 0;
        color: var(--text-3);
        font-size: var(--text-sm);
        line-height: var(--leading-body);
    }
    .admin-form-status.is-error { color: var(--loss); }
    .admin-form-status:empty { display: none; }

    /* A titled block of a long form: H2, one line of lead, then the card. */
    .form-section {
        display: flex;
        flex-direction: column;
        gap: var(--space-3);
        min-width: 0;
    }
    .form-section h2 {
        margin: 0;
        color: var(--text);
        font-family: var(--font-head);
        font-size: var(--text-xl);
        font-weight: var(--font-head-weight);
        letter-spacing: var(--font-head-tracking);
        text-transform: var(--font-head-transform);
        line-height: var(--leading-display);
    }
    .form-section-lead {
        margin: 0;
        max-width: var(--content-max-narrow);
        color: var(--text-3);
        font-size: var(--text-md);
        line-height: var(--leading-body);
    }
    .form-section-card {
        box-sizing: border-box;
        min-width: 0;
        padding: var(--space-section);
        background: var(--card-bg);
        color: var(--card-fg);
        border: 1px solid var(--card-border);
        border-radius: var(--card-radius);
        box-shadow: var(--card-shadow);
    }
    .form-inline {
        display: flex;
        flex-direction: column;
        gap: var(--space-5);
        max-width: var(--container-sm);
    }

    @media (max-width: 820px) {
        .form-grid { grid-template-columns: minmax(0, 1fr); }
        /* 16px control text, so the browser does not zoom on focus. */
        .form-input,
        .form-select,
        .form-textarea { font-size: var(--text-lg); }
        .admin-form-actions > .btn,
        .admin-form-actions > .btn-add { flex: 1 1 auto; }
    }

    /* ------------------------------------------------------------------
       Summary tiles (the admin dashboard): the Home stat
       treatment. Value in the display face with tabular figures, label
       under it.
       ------------------------------------------------------------------ */
    .summary-cards {
        display: grid;
        grid-template-columns: repeat(auto-fit, minmax(calc(var(--hit-target-min) * 4), 1fr));
        gap: var(--space-3);
        min-width: 0;
    }
    .summary-card {
        box-sizing: border-box;
        display: flex;
        flex-direction: column;
        align-items: center;
        justify-content: center;
        min-width: 0;
        min-height: calc(var(--density-row) + var(--space-card-pad));
        padding: var(--card-padding);
        background: var(--card-bg);
        color: var(--card-fg);
        border: 1px solid var(--card-border);
        border-radius: var(--card-radius);
        box-shadow: var(--card-shadow);
        text-align: center;
    }
    a.summary-card-link {
        color: inherit;
        text-decoration: none;
        cursor: pointer;
    }
    a.summary-card-link:hover { border-color: var(--border-strong); }
    a.summary-card-link:hover .label { text-decoration: underline; }
    a.summary-card-link:focus-visible {
        outline: var(--focus-ring-width) solid var(--focus-ring);
        outline-offset: var(--focus-ring-offset);
    }
    .summary-card .value {
        color: var(--text);
        font-family: var(--font-head);
        font-size: var(--text-2xl);
        font-weight: var(--font-head-weight);
        letter-spacing: var(--font-head-tracking);
        font-variant-numeric: tabular-nums;
        line-height: var(--leading-display);
    }
    .summary-card .label {
        margin-top: var(--space-1);
        color: var(--text-3);
        font-size: var(--text-sm);
        line-height: var(--leading-ui);
    }
    /* A tile whose number is loading or failed: never a fake zero. */
    .summary-card-pending { gap: var(--space-2); }
    .summary-card-pending .label { margin-top: 0; }
    .summary-card-skel {
        width: var(--space-8);
        height: var(--space-6);
        border-radius: var(--radius-control);
        background: color-mix(in srgb, var(--text) 12%, var(--surface-sunken));
    }
    .summary-card-error {
        margin: 0;
        color: var(--loss);
        font-size: var(--text-sm);
        line-height: var(--leading-body);
        overflow-wrap: anywhere;
    }
    @media (max-width: 820px) {
        .summary-cards { grid-template-columns: minmax(0, 1fr); }
        .summary-card {
            min-height: var(--hit-target-min);
            padding: var(--space-2);
        }
        .summary-card .value { font-size: var(--text-lg); }
        .summary-card .label { font-size: var(--text-xs); }
    }

    /* ------------------------------------------------------------------
       Settings page blocks.
       ------------------------------------------------------------------ */
    /* The sections of the settings page, one section step apart. */
    .settings-page {
        display: flex;
        flex-direction: column;
        gap: var(--space-section);
        min-width: 0;
    }
    .settings-hint {
        margin: 0;
        color: var(--text-3);
        font-size: var(--text-sm);
        line-height: var(--leading-body);
    }
    .settings-subhead {
        margin: 0;
        color: var(--text);
        font-family: var(--font-head);
        font-size: var(--text-lg);
        font-weight: var(--font-label-weight);
        line-height: var(--leading-ui);
    }
    .settings-save-bar {
        position: sticky;
        bottom: var(--tab-bar-height, 0);
        z-index: 5;
        padding: var(--space-3) 0;
        background: var(--bg);
        border-top: 1px solid var(--border);
    }
    .settings-divider {
        height: 1px;
        margin: var(--space-5) 0;
        background: var(--border);
    }
    /* The card of a settings section stacks its blocks like a form. */
    .form-section-card > * + * { margin-top: var(--space-5); }
    .form-section-card > .form-label + *,
    .form-section-card > .field-label + *,
    .form-section-card > .settings-subhead + * { margin-top: var(--space-2); }
    .form-section-card > .settings-hint { margin-top: var(--space-2); }
    .form-section-card > .admin-check-row + .admin-check-row { margin-top: var(--space-3); }
    .form-field > .settings-hint,
    .form-field > .field-hint { margin-top: 0; }
    /* The live line under Apply pack stays in the tree when it has nothing
       to say; empty, it takes no space. It sits a small step under the
       pack actions, like any hint in the card. */
    .pack-brand-status { margin-top: var(--space-2); }
    .pack-brand-status:empty { margin: 0; }

    /* Identity pack cards */
    .pack-grid {
        display: grid;
        grid-template-columns: repeat(auto-fill, minmax(calc(var(--hit-target-min) * 4), 1fr));
        gap: var(--space-3);
    }
    .pack-card,
    .option-tile {
        box-sizing: border-box;
        display: flex;
        flex-direction: column;
        gap: var(--space-1);
        min-height: var(--hit-target-min);
        padding: var(--space-3);
        border: 1px solid var(--border-strong);
        border-radius: var(--radius-control);
        background: var(--surface);
        color: var(--text);
        font: inherit;
        text-align: start;
        cursor: pointer;
    }
    .pack-card:hover,
    .option-tile:hover { border-color: var(--text); }
    .pack-card:focus-visible,
    .option-tile:focus-visible,
    .copy-panel-head:focus-visible {
        outline: var(--focus-ring-width) solid var(--focus-ring);
        outline-offset: var(--focus-ring-offset);
    }
    /* Selected: soft fill, accent edge drawn twice and a heavier title, so
       the choice is not colour alone. */
    .pack-card.is-selected,
    .option-tile.is-selected {
        background: var(--accent-soft);
        border-color: var(--accent-text);
        box-shadow: inset 0 0 0 1px var(--accent-text);
    }
    .pack-card-name,
    .option-tile-title {
        color: var(--text);
        font-family: var(--font-head);
        font-size: var(--text-md);
        font-weight: var(--font-label-weight);
        line-height: var(--leading-ui);
    }
    .pack-card.is-selected .pack-card-name,
    .option-tile.is-selected .option-tile-title { font-weight: var(--font-head-weight); }
    .pack-card-desc,
    .option-tile-blurb {
        color: var(--text-3);
        font-size: var(--text-sm);
        line-height: var(--leading-body);
    }
    .pack-card-meta {
        color: var(--text-3);
        font-family: var(--font-mono);
        font-size: var(--text-xs);
    }
    .pack-actions {
        display: flex;
        flex-wrap: wrap;
        align-items: center;
        gap: var(--space-3);
    }

    /* Option tiles (shell / skin) */
    .option-tiles {
        display: grid;
        grid-template-columns: repeat(auto-fill, minmax(calc(var(--hit-target-min) * 4), 1fr));
        gap: var(--space-3);
    }

    /* A checkbox with its label in a row of other controls. */
    .section-chips {
        display: flex;
        flex-direction: column;
        gap: var(--space-3);
    }
    .section-chip {
        display: inline-flex;
        align-items: center;
        gap: var(--space-3);
        min-height: var(--hit-target-min);
        color: var(--text);
        font-family: var(--font-body);
        font-size: var(--text-md);
        line-height: var(--leading-body);
        cursor: pointer;
    }

    /* Collapsible copy panels */
    .copy-stack {
        display: flex;
        flex-direction: column;
        gap: var(--space-3);
    }
    .copy-panel {
        overflow: hidden;
        background: var(--card-bg);
        border: 1px solid var(--card-border);
        border-radius: var(--card-radius);
        box-shadow: var(--card-shadow);
    }
    .copy-panel-heading { margin: 0; }
    .copy-panel-head {
        box-sizing: border-box;
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: var(--space-3);
        width: 100%;
        min-height: var(--density-row);
        padding: var(--space-2) var(--space-4);
        border: 0;
        background: transparent;
        color: var(--text);
        font: inherit;
        text-align: start;
        cursor: pointer;
    }
    .copy-panel-head:focus-visible { outline-offset: calc(-1 * var(--focus-ring-width)); }
    .copy-panel-head:hover { background: var(--surface-raised); }
    .copy-panel-title {
        font-family: var(--font-head);
        font-size: var(--text-lg);
        font-weight: var(--font-label-weight);
        line-height: var(--leading-ui);
    }
    /* A chevron drawn from two borders. It turns when the panel is open. */
    .copy-panel-chevron {
        flex: 0 0 auto;
        box-sizing: border-box;
        width: var(--space-2);
        height: var(--space-2);
        margin-inline-end: var(--space-1);
        border-inline-end: var(--focus-ring-width) solid var(--text-3);
        border-bottom: var(--focus-ring-width) solid var(--text-3);
        transform: rotate(45deg);
    }
    .copy-panel.is-open .copy-panel-chevron { transform: rotate(-135deg); }
    .copy-panel-body {
        padding: var(--space-4);
        border-top: 1px solid var(--card-border);
    }

    /* Colour fields: swatch plus hex text. */
    .color-row {
        display: flex;
        flex-wrap: wrap;
        align-items: flex-end;
        gap: var(--space-4);
    }
    .color-field {
        display: flex;
        flex: 1 1 calc(var(--hit-target-min) * 5);
        flex-direction: column;
        gap: var(--space-2);
        min-width: 0;
    }
    .color-field .swatch-row,
    .settings-inline-row {
        display: flex;
        flex-wrap: wrap;
        align-items: center;
        gap: var(--space-2);
    }
    .color-field input[type="color"] {
        flex: 0 0 auto;
        box-sizing: border-box;
        width: var(--hit-target-min);
        height: var(--hit-target-min);
        padding: 0;
        border: 1px solid var(--border-strong);
        border-radius: var(--radius-control);
        background: transparent;
        cursor: pointer;
    }
    .color-field .form-input,
    .color-field .field-input,
    .settings-inline-row .form-input,
    .settings-inline-row .field-input {
        flex: 1 1 calc(var(--hit-target-min) * 3);
        width: auto;
        min-width: 0;
    }
    .color-field .form-input,
    .color-field .field-input { font-family: var(--font-mono); }
"#;

#[cfg(test)]
mod tests {
    use super::CSS;
    use crate::components::ui::test_support::{
        assert_ink_clears_aa, assert_sizes_from_tokens, css_rule, has_decl,
    };
    use crate::theme::brand::{BrandConfig, TEST_SWEEP_ACCENTS, contrast_ratio};
    use crate::theme::tokens::{resolved_tokens, theme_css};

    /// QA 694 nit 3: text on the selected pack card must keep 4.5:1 on its
    /// accent tint for any accent, in both themes (`--text-subtle` fell to
    /// 4.05 with the pale yellow accent in dark). The S4d redesign paints
    /// the selected card and option tile with `--accent-soft` instead of an
    /// 8% mix on `--surface-sunken`, so the sweep now runs on that fill, for
    /// the title ink and the muted description and meta ink.
    #[test]
    fn selected_pack_card_text_keeps_aa_for_any_accent() {
        let selected = css_rule(
            CSS,
            ".pack-card.is-selected,\n    .option-tile.is-selected {",
        );
        assert!(
            has_decl(&selected, "background: var(--accent-soft)"),
            "{selected}"
        );
        let title = css_rule(CSS, ".pack-card-name,\n    .option-tile-title {");
        assert!(has_decl(&title, "color: var(--text)"), "{title}");
        let desc = css_rule(CSS, ".pack-card-desc,\n    .option-tile-blurb {");
        assert!(has_decl(&desc, "color: var(--text-3)"), "{desc}");
        assert!(has_decl(&desc, "font-size: var(--text-sm)"), "{desc}");
        let meta = css_rule(CSS, ".pack-card-meta {");
        assert!(has_decl(&meta, "color: var(--text-3)"), "{meta}");
        assert!(has_decl(&meta, "font-size: var(--text-xs)"), "{meta}");

        for accent in TEST_SWEEP_ACCENTS {
            let css = theme_css(&BrandConfig::from_settings(accent, accent));
            for theme in ["dark", "light"] {
                let tokens = resolved_tokens(&css, theme);
                let fill = &tokens["--accent-soft"];
                for ink in ["--text", "--text-3"] {
                    let text = &tokens[ink];
                    let ratio = contrast_ratio(text, fill);
                    assert!(
                        ratio >= 4.5,
                        "{theme} {accent}: {ink} {text} on {fill} = {ratio:.2}"
                    );
                }
                // Normal-size text, so 4.5:1 is the bar, at the 12px floor.
                assert_eq!(tokens["--text-xs"], "12px");
                assert_eq!(tokens["--text-sm"], "13px");
            }
        }
    }

    /// Type on the `--text-*` tokens (12px floor), no rem, px only for
    /// hairlines.
    #[test]
    fn admin_css_sizes_come_from_tokens() {
        assert_sizes_from_tokens(CSS);
        assert!(!CSS.contains('\u{2014}'));
    }

    #[test]
    fn form_pattern_spacing_matches_the_settings_frame() {
        // Label --space-2 above its control.
        let field = css_rule(CSS, ".form-field {");
        assert!(has_decl(&field, "gap: var(--space-2)"), "{field}");
        // --space-5 from one field to the next, and above Save.
        let step = css_rule(CSS, ".form-stack > * + * {");
        assert!(has_decl(&step, "margin-top: var(--space-5)"), "{step}");
        // Checkbox rows: full width, 44px, --space-3 apart.
        let row = css_rule(CSS, ".admin-check-row,\n    .form-checkbox-row {");
        for decl in ["width: 100%", "min-height: var(--hit-target-min)"] {
            assert!(has_decl(&row, decl), "{decl}: {row}");
        }
        let rows = css_rule(
            CSS,
            ".form-stack > .admin-check-row + .admin-check-row,\n    .form-stack > .form-checkbox-row + .form-checkbox-row {",
        );
        assert!(has_decl(&rows, "margin-top: var(--space-3)"), "{rows}");
        // The save line is out of the layout while it has nothing to say.
        assert!(CSS.contains(".admin-form-status:empty { display: none; }"));
    }

    #[test]
    fn controls_are_44px_and_buttons_use_the_button_tokens() {
        let legacy = css_rule(
            CSS,
            ".row-btn,\n    .btn-add,\n    .btn-ghost,\n    .btn-cancel,\n    .btn-save {",
        );
        for decl in [
            "min-height: var(--button-height)",
            "min-width: var(--hit-target-min)",
            "font-size: var(--button-font-size)",
        ] {
            assert!(has_decl(&legacy, decl), "{decl}: {legacy}");
        }
        let input = css_rule(CSS, ".form-input,\n    .form-select,\n    .form-textarea {");
        assert!(
            has_decl(&input, "min-height: var(--hit-target-min)"),
            "{input}"
        );
        // Danger ink on the dialog and page surfaces.
        assert_ink_clears_aa("--loss", "--surface", 4.5);
        assert_ink_clears_aa("--loss", "--surface-raised", 4.5);
        assert_ink_clears_aa("--loss", "--bg", 4.5);
        // Muted text on cards, and the open "Actions" button.
        assert_ink_clears_aa("--text-3", "--surface", 4.5);
        assert_ink_clears_aa("--accent-text", "--accent-soft", 4.5);
    }

    #[test]
    fn phone_tables_become_cards_and_actions_never_clip() {
        let (_, phone) = CSS
            .split_once("@media (max-width: 820px) {")
            .expect("phone block");
        // The row is the card; the badge and the meta line follow the name.
        let row = css_rule(phone, ".is-cards .data-table tr {");
        for decl in [
            "display: flex",
            "flex-wrap: wrap",
            "border: 1px solid var(--card-border)",
            "border-radius: var(--card-radius)",
        ] {
            assert!(has_decl(&row, decl), "{decl}: {row}");
        }
        let badge = css_rule(phone, ".is-cards .data-table td.cell-badge {");
        assert!(has_decl(&badge, "order: 3"), "{badge}");
        // The open overflow group is a full line of the card, so a long
        // action set wraps instead of clipping.
        let group = css_rule(
            phone,
            ".is-cards .data-table td.cell-actions .row-menu-group {",
        );
        for decl in ["flex: 0 0 100%", "flex-wrap: wrap"] {
            assert!(has_decl(&group, decl), "{decl}: {group}");
        }
        // Closed, the group is out of the layout at every width.
        let closed = css_rule(CSS, ".row-menu-group {");
        assert!(has_decl(&closed, "display: none"), "{closed}");
        assert!(CSS.contains(".row-actions.is-open > .row-menu-group { display: flex; }"));
        // Dates keep their year on the same line, at every width.
        assert!(CSS.contains(".date { white-space: nowrap; }"));
        let num = css_rule(CSS, ".data-table th.num,\n    .data-table td.num {");
        assert!(has_decl(&num, "text-align: end"), "{num}");
    }
}
