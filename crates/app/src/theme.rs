pub const THEME_CSS: &str = r#"
:root {
    /* Backgrounds */
    --bg-void: #08080c;
    --bg-surface: #0e0e14;
    --bg-card: #14141e;
    --bg-card-alt: #1a1a28;
    --bg-elevated: #20202e;

    /* Accent (purple) */
    --accent: #7c3aed;
    --accent-soft: rgba(124, 58, 237, 0.15);
    --accent-glow: rgba(124, 58, 237, 0.25);
    --accent-bright: #a78bfa;

    /* Semantic */
    --danger: #d63031;
    --success: #00c853;
    --warning: #f0b232;
    --info: #4a9eff;

    /* Text */
    --text-bright: #f0eee8;
    --text-primary: #ccc8c0;
    --text-secondary: #807a70;
    --text-muted: #504c44;

    /* Borders */
    --border: #2a2832;
    --border-light: #363440;

    /* Fonts */
    --font-display-hero: 'Bebas Neue', sans-serif;
    --font-display: 'Rajdhani', sans-serif;
    --font-body: 'Source Sans 3', sans-serif;
    --font-mono: 'DM Mono', monospace;
}

/* Strategy section overrides */
[data-theme="strategy"] {
    --bg-void: #05070f;
    --bg-surface: #0a0e1a;
    --bg-card: #111827;
    --bg-card-alt: #1a2332;
    --bg-elevated: #243044;

    --accent: #ff6a00;
    --accent-soft: rgba(255, 106, 0, 0.15);
    --accent-glow: rgba(255, 106, 0, 0.4);
    --accent-bright: #ff9500;

    --danger: #ef4444;
    --success: #22c55e;
    --warning: #fbbf24;
    --info: #00f0ff;

    --text-bright: #ffffff;
    --text-primary: #e2e8f0;
    --text-secondary: #94a3b8;
    --text-muted: #64748b;

    --border: #1e293b;
    --border-light: #334155;

    --font-display-hero: 'Orbitron', sans-serif;
    --font-display: 'Exo 2', sans-serif;
    --font-body: 'Exo 2', sans-serif;
    --font-mono: 'JetBrains Mono', monospace;
}

/* Base resets */
*, *::before, *::after {
    box-sizing: border-box;
    margin: 0;
    padding: 0;
}

body {
    background: var(--bg-void);
    color: var(--text-primary);
    font-family: var(--font-body);
    line-height: 1.6;
    -webkit-font-smoothing: antialiased;
}

a {
    color: inherit;
    text-decoration: none;
}

/* Scrollbar */
::-webkit-scrollbar { width: 8px; }
::-webkit-scrollbar-track { background: var(--bg-surface); }
::-webkit-scrollbar-thumb { background: var(--border-light); border-radius: 4px; }
::-webkit-scrollbar-thumb:hover { background: var(--text-muted); }
"#;
