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

    /* Two-column form grid for wider modals on desktop */
    @media (min-width: 480px) {
        .form-grid { display: grid; grid-template-columns: 1fr 1fr; gap: 1rem; }
        .form-grid .span-full { grid-column: 1 / -1; }
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
        margin: 0 0 1rem; padding-bottom: 0.5rem; border-bottom: 1px solid var(--border);
    }
    .form-inline { display: flex; flex-direction: column; gap: 1rem; max-width: 500px; }
"#;
