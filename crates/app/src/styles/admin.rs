/// Styles specific to admin pages: data tables, row actions, toolbar,
/// form modals, form fields, pagination, summary cards, etc.
pub const CSS: &str = r#"
    /* Data Table */
    .data-table { width: 100%; border-collapse: collapse; font-size: 0.85rem; }
    .data-table th {
        text-align: left; padding: 0.6rem 0.75rem; font-family: var(--font-head);
        font-weight: 700; font-size: 0.75rem; text-transform: uppercase;
        letter-spacing: 0.05em; color: var(--text-3); border-bottom: 1px solid var(--border);
    }
    .data-table td { padding: 0.6rem 0.75rem; border-bottom: 1px solid var(--border); color: var(--text-2); }
    .data-table tr:hover td { background: var(--surface); }

    /* Action buttons in table rows */
    .row-actions { display: flex; gap: 0.35rem; flex-wrap: wrap; }
    .row-btn {
        padding: 0.2rem 0.55rem; border-radius: 4px; border: 1px solid var(--border);
        background: var(--surface); color: var(--text-2); font-size: 0.7rem;
        cursor: pointer; transition: all 0.15s; white-space: nowrap;
    }
    .row-btn:hover { border-color: var(--accent-soft); color: var(--text); }
    .row-btn.danger:hover { border-color: var(--danger); color: var(--danger); }
    .row-btn.primary { background: var(--accent); color: white; border-color: var(--accent); }
    .row-btn.primary:hover { filter: brightness(1.15); }

    /* Page-level action bar */
    .admin-toolbar { display: flex; justify-content: space-between; align-items: center; margin-bottom: 1.5rem; flex-wrap: wrap; gap: 0.75rem; }
    .admin-toolbar select {
        background: var(--surface); border: 1px solid var(--border); color: var(--text);
        padding: 0.4rem 0.75rem; border-radius: 6px; font-size: 0.85rem;
    }

    /* Add button */
    .btn-add {
        display: inline-flex; align-items: center; gap: 0.4rem; padding: 0.5rem 1.2rem;
        border-radius: 6px; background: var(--accent); color: white; border: none;
        font-size: 0.85rem; font-weight: 600; cursor: pointer; transition: all 0.2s;
        text-transform: uppercase; letter-spacing: 0.03em;
    }
    .btn-add:hover { filter: brightness(1.15); box-shadow: 0 0 15px var(--accent-soft); }

    /* Form Modal */
    .form-modal-overlay {
        position: fixed; top: 0; left: 0; right: 0; bottom: 0; z-index: 1000;
        background: var(--overlay); display: flex; align-items: center; justify-content: center;
        animation: fade-in 0.15s ease-out;
    }
    .form-modal {
        background: var(--surface-2); border: 1px solid var(--border); border-radius: 12px;
        padding: 0; width: 90vw; max-width: 500px; max-height: 85vh; overflow-y: auto;
        animation: slide-up 0.2s ease-out;
    }
    .form-modal.wide { max-width: 640px; }
    .form-modal-header {
        padding: 1.25rem 1.5rem 0.75rem; border-bottom: 1px solid var(--border);
        font-family: var(--font-head); font-weight: 700; font-size: 1.2rem;
        color: var(--text); text-transform: uppercase; letter-spacing: 0.03em;
    }
    .form-modal-body { padding: 1.25rem 1.5rem; display: flex; flex-direction: column; gap: 1rem; }
    .form-modal-footer {
        padding: 0.75rem 1.5rem 1.25rem; display: flex; justify-content: flex-end; gap: 0.75rem;
    }
    .btn-cancel {
        padding: 0.5rem 1rem; border-radius: 6px; background: var(--surface);
        border: 1px solid var(--border); color: var(--text-2); font-size: 0.85rem;
        cursor: pointer; transition: all 0.15s;
    }
    .btn-cancel:hover { color: var(--text); }
    .btn-save {
        padding: 0.5rem 1rem; border-radius: 6px; background: var(--accent);
        border: none; color: white; font-size: 0.85rem; font-weight: 600;
        cursor: pointer; transition: all 0.15s;
    }
    .btn-save:hover { filter: brightness(1.15); }
    .btn-save:disabled { opacity: 0.5; cursor: not-allowed; }
    .btn-save.danger { background: var(--danger); }
    .btn-save.danger:hover { filter: brightness(1.1); }

    /* Form Fields */
    .form-field { display: flex; flex-direction: column; gap: 0.3rem; }
    .form-label {
        font-family: var(--font-head); font-weight: 600; font-size: 0.8rem;
        color: var(--text); text-transform: uppercase; letter-spacing: 0.04em;
    }
    .form-input, .form-select, .form-textarea {
        background: var(--surface); border: 1px solid var(--border); border-radius: 6px;
        color: var(--text); padding: 0.5rem 0.75rem; font-size: 0.85rem; font-family: inherit;
        color-scheme: dark;
    }
    .form-input:focus, .form-select:focus, .form-textarea:focus { outline: none; border-color: var(--accent); }
    .form-input[type="date"], .form-input[type="time"] { position: relative; }
    .form-input[type="date"]::-webkit-calendar-picker-indicator,
    .form-input[type="time"]::-webkit-calendar-picker-indicator {
        position: absolute; inset: 0; width: auto; height: auto;
        color: transparent; background: transparent; cursor: pointer;
    }
    .form-textarea { resize: vertical; min-height: 80px; }
    .form-checkbox-row { display: flex; align-items: center; gap: 0.5rem; }
    .form-checkbox-row input[type="checkbox"] { accent-color: var(--accent); }

    /* Two-column form grid (modals + settings) */
    .form-grid { display: grid; grid-template-columns: 1fr; gap: 0.75rem 1rem; }
    .form-grid .span-full { grid-column: 1 / -1; }
    @media (min-width: 480px) {
        .form-grid { grid-template-columns: 1fr 1fr; }
    }

    /* Confirm Dialog */
    .confirm-body { padding: 1.25rem 1.5rem; color: var(--text-2); font-size: 0.9rem; line-height: 1.6; }

    /* Summary Cards (dashboard) */
    .summary-cards { display: grid; grid-template-columns: repeat(auto-fill, minmax(180px, 1fr)); gap: 1rem; margin-bottom: 2rem; }
    .summary-card {
        background: var(--surface); border: 1px solid var(--border); border-radius: 10px;
        padding: 1.25rem; text-align: center;
    }
    .summary-card .value {
        font-family: var(--font-head); font-size: 2.2rem; color: var(--accent);
        letter-spacing: 2px; line-height: 1;
    }
    .summary-card .label {
        font-size: 0.75rem; color: var(--text-3); text-transform: uppercase;
        letter-spacing: 0.05em; margin-top: 0.3rem;
    }

    /* Pagination */
    .pagination { display: flex; align-items: center; gap: 0.75rem; margin-top: 1.5rem; }
    .pagination button {
        padding: 0.35rem 0.75rem; border-radius: 4px; border: 1px solid var(--border);
        background: var(--surface); color: var(--text-2); font-size: 0.8rem; cursor: pointer;
    }
    .pagination button:hover:not(:disabled) { color: var(--text); border-color: var(--accent-soft); }
    .pagination button:disabled { opacity: 0.4; cursor: not-allowed; }
    .pagination .page-info { font-size: 0.8rem; color: var(--text-3); }

    /* Form sections */
    .form-section { margin-bottom: 2rem; }
    .form-section h2 {
        font-family: var(--font-head); font-size: 1.1rem; font-weight: 700;
        color: var(--text); text-transform: uppercase; letter-spacing: 0.04em;
        margin: 0 0 0.4rem; padding-bottom: 0;
        border-bottom: none;
    }
    .form-section-lead {
        color: var(--text-3); font-size: 0.85rem; line-height: 1.45;
        margin: 0 0 1rem; max-width: 42rem;
    }
    .form-section-card {
        background: var(--surface); border: 1px solid var(--border);
        border-radius: 12px; padding: 1.15rem 1.25rem;
    }
    .form-inline { display: flex; flex-direction: column; gap: 1rem; max-width: 500px; }

    /* Ghost / secondary toolbar button */
    .btn-ghost {
        display: inline-flex; align-items: center; gap: 0.4rem;
        padding: 0.5rem 1rem; border-radius: 6px;
        background: transparent; color: var(--text-2);
        border: 1px solid var(--border); font-size: 0.85rem; font-weight: 600;
        cursor: pointer; text-decoration: none; transition: all 0.15s;
        text-transform: uppercase; letter-spacing: 0.03em;
    }
    .btn-ghost:hover { color: var(--text); border-color: var(--accent-soft); }

    /* Identity pack cards */
    .pack-grid {
        display: grid;
        grid-template-columns: repeat(auto-fill, minmax(11.5rem, 1fr));
        gap: 0.65rem;
        margin-bottom: 1rem;
    }
    .pack-card {
        text-align: left; cursor: pointer;
        padding: 0.85rem 0.9rem; border-radius: 10px;
        border: 1px solid var(--border); background: var(--surface-2);
        color: var(--text); transition: border-color 0.15s, box-shadow 0.15s;
        display: flex; flex-direction: column; gap: 0.35rem;
        font: inherit;
    }
    .pack-card:hover { border-color: color-mix(in srgb, var(--accent) 45%, var(--border)); }
    .pack-card.is-selected {
        border-color: var(--accent);
        box-shadow: 0 0 0 1px color-mix(in srgb, var(--accent) 35%, transparent);
        background: color-mix(in srgb, var(--accent) 8%, var(--surface-2));
    }
    .pack-card-name {
        font-family: var(--font-head); font-weight: 700; font-size: 0.9rem;
        letter-spacing: 0.02em;
    }
    .pack-card-desc {
        font-size: 0.75rem; color: var(--text-3); line-height: 1.4;
        display: -webkit-box; -webkit-line-clamp: 3; -webkit-box-orient: vertical;
        overflow: hidden;
    }
    .pack-card-meta {
        font-family: var(--font-mono); font-size: 0.65rem; color: var(--text-3);
        letter-spacing: 0.04em; margin-top: 0.15rem;
    }
    .pack-actions {
        display: flex; flex-wrap: wrap; align-items: center; gap: 0.75rem;
        padding-top: 0.25rem;
    }

    /* Option tiles (shell / skin) */
    .option-tiles {
        display: grid;
        grid-template-columns: repeat(auto-fill, minmax(10.5rem, 1fr));
        gap: 0.55rem;
    }
    .option-tile {
        text-align: left; cursor: pointer; font: inherit; color: var(--text);
        padding: 0.7rem 0.8rem; border-radius: 9px;
        border: 1px solid var(--border); background: var(--surface-2);
        transition: border-color 0.15s, background 0.15s;
    }
    .option-tile:hover { border-color: color-mix(in srgb, var(--accent) 40%, var(--border)); }
    .option-tile.is-selected {
        border-color: var(--accent);
        background: color-mix(in srgb, var(--accent) 10%, var(--surface-2));
    }
    .option-tile-title {
        font-family: var(--font-head); font-weight: 700; font-size: 0.82rem;
        margin-bottom: 0.25rem;
    }
    .option-tile-blurb { font-size: 0.72rem; color: var(--text-3); line-height: 1.35; }

    /* Section visibility chips */
    .section-chips {
        display: flex; flex-wrap: wrap; gap: 0.45rem;
    }
    .section-chip {
        display: inline-flex; align-items: center; gap: 0.4rem;
        padding: 0.4rem 0.7rem; border-radius: 999px;
        border: 1px solid var(--border); background: var(--surface-2);
        font-size: 0.78rem; color: var(--text-2); cursor: pointer;
        user-select: none; transition: all 0.15s;
    }
    .section-chip input { accent-color: var(--accent); margin: 0; }
    .section-chip.is-on {
        border-color: color-mix(in srgb, var(--accent) 50%, var(--border));
        color: var(--text);
        background: color-mix(in srgb, var(--accent) 12%, var(--surface-2));
    }

    /* Collapsible copy panels */
    .copy-stack { display: flex; flex-direction: column; gap: 0.5rem; }
    .copy-panel {
        border: 1px solid var(--border); border-radius: 10px;
        background: var(--surface); overflow: hidden;
    }
    .copy-panel-head {
        width: 100%; display: flex; align-items: center; justify-content: space-between;
        gap: 0.75rem; padding: 0.75rem 1rem; border: none; background: transparent;
        color: var(--text); cursor: pointer; font: inherit; text-align: left;
    }
    .copy-panel-head:hover { background: color-mix(in srgb, var(--surface-2) 80%, transparent); }
    .copy-panel-title {
        font-family: var(--font-head); font-weight: 700; font-size: 0.85rem;
        text-transform: uppercase; letter-spacing: 0.06em;
    }
    .copy-panel-chevron {
        font-size: 0.65rem; color: var(--text-3); line-height: 1;
        width: 1.25rem; text-align: center; flex-shrink: 0;
    }
    .copy-panel-body {
        padding: 0 1rem 1rem;
        border-top: 1px solid var(--border);
        display: flex; flex-direction: column; gap: 0.15rem;
    }
    .copy-panel-body .form-grid {
        display: grid; grid-template-columns: 1fr 1fr; gap: 0.65rem 0.85rem;
        margin-top: 0.75rem;
    }
    .copy-panel-body .form-grid .span-full { grid-column: 1 / -1; }
    @media (max-width: 640px) {
        .copy-panel-body .form-grid { grid-template-columns: 1fr; }
    }

    .settings-hint {
        font-size: 0.78rem; color: var(--text-3); margin: 0.3rem 0 0; line-height: 1.4;
    }

    /* Settings page: readable width + sticky actions */
    .settings-page {
        max-width: 52rem;
        padding-bottom: 5rem;
    }
    .admin-toolbar.sticky-actions {
        position: sticky; top: 0; z-index: 20;
        margin: -0.5rem -0.5rem 1.25rem;
        padding: 0.75rem 0.5rem;
        background: color-mix(in srgb, var(--bg) 92%, transparent);
        backdrop-filter: blur(10px);
        border-bottom: 1px solid color-mix(in srgb, var(--border) 80%, transparent);
    }
    .settings-subhead {
        font-family: var(--font-head); font-size: 0.72rem; font-weight: 700;
        text-transform: uppercase; letter-spacing: 0.08em; color: var(--text-3);
        margin: 0 0 0.55rem;
    }
    .settings-divider {
        height: 1px; background: var(--border); margin: 1.1rem 0;
    }
    .color-row {
        display: flex; flex-wrap: wrap; gap: 1rem; align-items: flex-end;
    }
    .color-field {
        display: flex; flex-direction: column; gap: 0.3rem;
        min-width: 11rem; max-width: 16rem; flex: 0 1 16rem;
    }
    .color-field .swatch-row {
        display: flex; align-items: center; gap: 0.5rem;
    }
    .color-field input[type="color"] {
        width: 2.5rem; height: 2.5rem; padding: 0;
        border: 1px solid var(--border); border-radius: 8px;
        background: transparent; cursor: pointer; flex-shrink: 0;
    }
    .color-field .form-input { flex: 1; min-width: 0; font-family: var(--font-mono); font-size: 0.8rem; }

    /* Compact nav editor */
    .nav-column {
        margin-bottom: 0.75rem; padding: 0.75rem 0.85rem;
        border: 1px solid var(--border); border-radius: 10px; background: var(--surface);
    }
    .nav-column h3 {
        font-family: var(--font-mono); font-size: 0.68rem; letter-spacing: 0.1em;
        text-transform: uppercase; color: var(--text-3); margin: 0 0 0.2rem;
    }
    .nav-column .nav-hint {
        color: var(--text-3); font-size: 0.75rem; margin: 0 0 0.55rem; line-height: 1.35;
    }
    .nav-row {
        display: flex; align-items: center; gap: 0.5rem; flex-wrap: wrap;
        padding: 0.45rem 0.55rem; border-radius: 7px;
        border: 1px solid transparent;
    }
    .nav-row:hover { background: var(--surface-2); border-color: var(--border); }
    .nav-row-label { flex: 1; min-width: 5rem; font-weight: 500; font-size: 0.88rem; }
    .nav-row-id {
        font-family: var(--font-mono); font-size: 0.62rem; color: var(--text-3);
        opacity: 0.85;
    }
    .nav-row .form-input {
        width: auto; min-width: 6.5rem; padding: 0.28rem 0.45rem; font-size: 0.78rem;
    }
    .nav-row .row-btn { padding: 0.22rem 0.45rem; min-width: 1.75rem; }
"#;
