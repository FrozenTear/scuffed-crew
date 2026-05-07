pub const CSS: &str = r#"
:root {
    --bg: #0f0f14;
    --bg-card: #1a1a24;
    --bg-input: #12121a;
    --border: #2a2a3a;
    --accent: #7c3aed;
    --accent-hover: #6d28d9;
    --text: #e2e2ea;
    --text-dim: #8888a0;
    --success: #22c55e;
    --warning: #f59e0b;
    --error: #ef4444;
    --font: "Inter", -apple-system, sans-serif;
    --font-mono: "JetBrains Mono", "Fira Code", monospace;
}

* { box-sizing: border-box; margin: 0; padding: 0; }

body {
    font-family: var(--font);
    background: var(--bg);
    color: var(--text);
    font-size: 14px;
    line-height: 1.5;
}

.app {
    min-height: 100vh;
    display: flex;
    flex-direction: column;
}

.nav {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 1rem 1.5rem;
    border-bottom: 1px solid var(--border);
    background: var(--bg-card);
}

.logo {
    font-size: 1rem;
    font-weight: 700;
    color: var(--accent);
    text-transform: uppercase;
    letter-spacing: 0.05em;
}

.nav-links {
    display: flex;
    gap: 1rem;
}

.nav-links a {
    color: var(--text-dim);
    text-decoration: none;
    font-size: 0.85rem;
    font-weight: 500;
    padding: 0.4rem 0.8rem;
    border-radius: 6px;
    transition: all 0.15s;
}

.nav-links a:hover, .nav-links a.active {
    color: var(--text);
    background: rgba(124, 58, 237, 0.1);
}

.panel {
    padding: 1.5rem;
    max-width: 700px;
    margin: 0 auto;
    width: 100%;
}

.panel h2 {
    font-size: 1.1rem;
    font-weight: 600;
    margin-bottom: 1.2rem;
    color: var(--text);
}

.card {
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 10px;
    padding: 1.2rem;
    margin-bottom: 1rem;
}

.card h3 {
    font-size: 0.85rem;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-dim);
    margin-bottom: 0.8rem;
}

.stat-row {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 0.4rem 0;
}

.stat-row .label { color: var(--text-dim); font-size: 0.85rem; }
.stat-row .value { font-weight: 600; font-family: var(--font-mono); font-size: 0.85rem; }

.status-dot {
    display: inline-block;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    margin-right: 0.4rem;
}
.status-dot.ok { background: var(--success); }
.status-dot.warn { background: var(--warning); }
.status-dot.err { background: var(--error); }

.field {
    margin-bottom: 1rem;
}

.field label {
    display: block;
    font-size: 0.8rem;
    font-weight: 500;
    color: var(--text-dim);
    margin-bottom: 0.3rem;
    text-transform: uppercase;
    letter-spacing: 0.03em;
}

.field input, .field select {
    width: 100%;
    padding: 0.6rem 0.8rem;
    background: var(--bg-input);
    border: 1px solid var(--border);
    border-radius: 6px;
    color: var(--text);
    font-size: 0.85rem;
    font-family: var(--font);
    outline: none;
    transition: border-color 0.15s;
}

.field input:focus, .field select:focus {
    border-color: var(--accent);
}

.field input[type="checkbox"] {
    width: auto;
    margin-right: 0.5rem;
}

.checkbox-row {
    display: flex;
    align-items: center;
    padding: 0.4rem 0;
}

.checkbox-row label {
    margin-bottom: 0;
    text-transform: none;
    font-size: 0.85rem;
    color: var(--text);
}

.btn {
    padding: 0.6rem 1.2rem;
    border: none;
    border-radius: 6px;
    font-size: 0.85rem;
    font-weight: 600;
    cursor: pointer;
    transition: all 0.15s;
}

.btn-primary {
    background: var(--accent);
    color: white;
}

.btn-primary:hover { background: var(--accent-hover); }

.btn-secondary {
    background: transparent;
    border: 1px solid var(--border);
    color: var(--text-dim);
}

.btn-secondary:hover {
    border-color: var(--accent);
    color: var(--text);
}

.btn-danger {
    background: var(--error);
    color: white;
}

.btn-danger:hover { background: #dc2626; }

.actions {
    display: flex;
    gap: 0.8rem;
    margin-top: 1.2rem;
}

.toast {
    position: fixed;
    bottom: 1.5rem;
    right: 1.5rem;
    padding: 0.8rem 1.2rem;
    border-radius: 8px;
    font-size: 0.85rem;
    font-weight: 500;
    animation: slide-in 0.2s ease;
}

.toast.success { background: var(--success); color: white; }
.toast.error { background: var(--error); color: white; }

@keyframes slide-in {
    from { transform: translateY(10px); opacity: 0; }
    to { transform: translateY(0); opacity: 1; }
}

.preview-img {
    border-radius: 6px;
    border: 1px solid var(--border);
    max-width: 100%;
    margin-top: 0.8rem;
}

.card-error {
    border-color: var(--error);
    color: var(--error);
}

.card-error p { font-size: 0.85rem; }

.card-warning {
    border-color: var(--warning);
}

.card-warning h3 { color: var(--warning); }
.card-warning p { font-size: 0.85rem; color: var(--text-dim); }

.text-dim { color: var(--text-dim); }
.text-sm { font-size: 0.8rem; margin-top: 0.6rem; }

.btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
}

.panel-wide { max-width: 960px; }

.match-count {
    font-size: 0.8rem;
    margin-bottom: 0.8rem;
}

/* Match history table */
.match-table {
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 10px;
    overflow: hidden;
}

.match-header, .match-row {
    display: grid;
    grid-template-columns: 60px 100px 70px 1fr 40px 40px 40px 50px 50px 50px 90px;
    align-items: center;
    padding: 0.5rem 0.8rem;
    font-size: 0.8rem;
    gap: 0.3rem;
}

.match-header {
    background: rgba(124, 58, 237, 0.08);
    font-weight: 600;
    color: var(--text-dim);
    text-transform: uppercase;
    font-size: 0.7rem;
    letter-spacing: 0.04em;
}

.match-row {
    border-top: 1px solid var(--border);
    font-family: var(--font-mono);
}

.match-row:hover { background: rgba(124, 58, 237, 0.04); }

.col-outcome { font-weight: 700; text-transform: uppercase; font-size: 0.75rem; }
.col-hero { font-weight: 500; }
.col-role { font-size: 0.75rem; }
.col-map { font-size: 0.75rem; color: var(--text-dim); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.col-stat { text-align: right; font-size: 0.8rem; }
.col-time { text-align: right; font-size: 0.75rem; }

.outcome-win { color: var(--success); }
.outcome-loss { color: var(--error); }
.outcome-draw { color: var(--warning); }

/* Stats page */
.stats-grid {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: 1rem;
}

.stat-block {
    text-align: center;
    padding: 0.8rem 0;
}

.stat-big {
    font-size: 1.6rem;
    font-weight: 700;
    font-family: var(--font-mono);
}

.stat-win { color: var(--success); }
.stat-loss { color: var(--error); }
.stat-label { font-size: 0.75rem; color: var(--text-dim); text-transform: uppercase; margin-top: 0.2rem; }

/* Role breakdown */
.role-grid {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 0.8rem;
}

.role-card {
    background: var(--bg-input);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 0.8rem;
}

.role-name {
    font-weight: 700;
    font-size: 0.9rem;
    margin-bottom: 0.3rem;
}

.role-tank { color: #f5b43c; }
.role-damage { color: #e65050; }
.role-support { color: #64c878; }

.role-stats { font-size: 0.8rem; margin-bottom: 0.5rem; }

.wr-bar {
    height: 4px;
    background: var(--border);
    border-radius: 2px;
    overflow: hidden;
}

.wr-fill {
    height: 100%;
    background: var(--accent);
    border-radius: 2px;
    transition: width 0.3s;
}

/* Hero table */
.hero-table {
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 10px;
    overflow: hidden;
}

.hero-header, .hero-row {
    display: grid;
    grid-template-columns: 110px 70px 55px 55px 40px 40px 40px 50px 50px 50px;
    align-items: center;
    padding: 0.5rem 0.8rem;
    font-size: 0.8rem;
    gap: 0.3rem;
}

.hero-header {
    background: rgba(124, 58, 237, 0.08);
    font-weight: 600;
    color: var(--text-dim);
    text-transform: uppercase;
    font-size: 0.7rem;
    letter-spacing: 0.04em;
}

.hero-row {
    border-top: 1px solid var(--border);
    font-family: var(--font-mono);
}

.hero-row:hover { background: rgba(124, 58, 237, 0.04); }

.col-hero-name { font-weight: 500; }
.col-hero-role { font-size: 0.75rem; }
.col-hero-games { text-align: right; }
.col-hero-wr { text-align: right; font-weight: 600; }

/* Session list */
.session-list {
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
}

.session-item {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 0.6rem 0.8rem;
    background: var(--bg-input);
    border: 1px solid var(--border);
    border-radius: 6px;
    cursor: pointer;
    transition: all 0.15s;
}

.session-item:hover {
    border-color: var(--accent);
}

.session-item-active {
    border-color: var(--accent);
    background: rgba(124, 58, 237, 0.08);
}

.session-hero { font-weight: 600; font-size: 0.85rem; }
.session-meta { font-size: 0.75rem; }

/* Progression charts */
.chart-grid {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 0.8rem;
}

.stat-chart {
    background: var(--bg-input);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 0.6rem;
}

.stat-chart-header {
    display: flex;
    align-items: baseline;
    gap: 0.4rem;
    margin-bottom: 0.3rem;
}

.stat-chart-label {
    font-size: 0.7rem;
    font-weight: 600;
    color: var(--text-dim);
    text-transform: uppercase;
    letter-spacing: 0.03em;
}

.stat-chart-value {
    font-size: 0.85rem;
    font-weight: 700;
    font-family: var(--font-mono);
    margin-left: auto;
}

.stat-chart-delta {
    font-size: 0.7rem;
    font-family: var(--font-mono);
    color: var(--text-dim);
}

.progression-svg {
    width: 100%;
    height: 50px;
}

/* Timeline table */
.timeline-table {
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 10px;
    overflow: hidden;
}

.timeline-header, .timeline-row {
    display: grid;
    grid-template-columns: 40px 40px 40px 40px 55px 55px 55px 70px;
    align-items: center;
    padding: 0.5rem 0.8rem;
    font-size: 0.8rem;
    gap: 0.3rem;
}

.timeline-header {
    background: rgba(124, 58, 237, 0.08);
    font-weight: 600;
    color: var(--text-dim);
    text-transform: uppercase;
    font-size: 0.7rem;
    letter-spacing: 0.04em;
}

.timeline-row {
    border-top: 1px solid var(--border);
    font-family: var(--font-mono);
}

.timeline-row:hover { background: rgba(124, 58, 237, 0.04); }

.col-capture { font-weight: 600; color: var(--accent); }

/* Map winrate table */
.map-table {
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 10px;
    overflow: hidden;
}

.map-header, .map-row {
    display: grid;
    grid-template-columns: 1fr 55px 60px 55px 100px;
    align-items: center;
    padding: 0.5rem 0.8rem;
    font-size: 0.8rem;
    gap: 0.3rem;
}

.map-header {
    background: rgba(124, 58, 237, 0.08);
    font-weight: 600;
    color: var(--text-dim);
    text-transform: uppercase;
    font-size: 0.7rem;
    letter-spacing: 0.04em;
}

.map-row {
    border-top: 1px solid var(--border);
    font-family: var(--font-mono);
}

.map-row:hover { background: rgba(124, 58, 237, 0.04); }

.col-map-name { font-weight: 500; }
.col-map-games { text-align: right; }
.col-map-wl { text-align: right; font-size: 0.75rem; color: var(--text-dim); }
.col-map-wr { text-align: right; font-weight: 600; }
.col-map-bar { padding-left: 0.4rem; }

/* Win trend chart */
.trend-svg {
    width: 100%;
    height: 70px;
}

.trend-header {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    margin-bottom: 0.4rem;
}

.trend-current {
    font-family: var(--font-mono);
    font-size: 1rem;
    font-weight: 700;
}

/* Map bar chart */
.map-bar-svg {
    width: 100%;
    height: auto;
}

/* Hero-map expandable rows */
.hero-row-expandable { cursor: pointer; }
.hero-row-expandable:hover { background: rgba(124, 58, 237, 0.06); }

.hero-row-sub {
    background: rgba(124, 58, 237, 0.03);
}

.col-sub-indent {
    padding-left: 1.2rem;
    font-size: 0.75rem;
    color: var(--text-dim);
}
"#;
