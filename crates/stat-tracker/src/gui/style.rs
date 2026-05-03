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
"#;
