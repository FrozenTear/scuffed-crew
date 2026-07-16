//! Shared homepage CSS. Shell/skin layers add overrides via data attributes.
//! Current base styles match ops_hub + esports DNA (clipped badges, dense metrics).

/// Structural + default visual styles (shared by all shells).
pub const HOME_SHARED_CSS: &str = r#"
    .home-wrap {
        --home-pad-x: clamp(1.25rem, 3.5vw, 2.75rem);
        --home-max: min(72rem, 100%);
        position: relative;
        min-height: 100%;
    }
    /* Constrained body column under full-bleed hero */
    .home {
        width: 100%;
        max-width: var(--home-max);
        margin: 0 auto;
        padding: 0.5rem var(--home-pad-x) 4.5rem;
        position: relative;
        z-index: 1;
        box-sizing: border-box;
    }

    /* ——— HERO: full viewport bleed, content rail constrained ——— */
    .home-hero {
        position: relative;
        width: 100%;
        margin: 0;
        padding: 0;
        overflow: hidden;
        border-bottom: 1px solid color-mix(in srgb, var(--border) 80%, transparent);
        background:
            radial-gradient(ellipse 70% 90% at 12% 20%, color-mix(in srgb, var(--accent) 16%, transparent) 0%, transparent 55%),
            radial-gradient(ellipse 50% 70% at 88% 80%, color-mix(in srgb, var(--accent) 8%, transparent) 0%, transparent 50%),
            linear-gradient(180deg, color-mix(in srgb, var(--surface) 92%, var(--bg)) 0%, var(--bg) 100%);
    }
    .home-hero::before {
        content: '';
        position: absolute;
        right: 0;
        top: -15%;
        width: min(48vw, 560px);
        height: 130%;
        background:
            repeating-linear-gradient(
                -18deg,
                transparent,
                transparent 14px,
                color-mix(in srgb, var(--accent) 7%, transparent) 14px,
                color-mix(in srgb, var(--accent) 7%, transparent) 15px
            );
        transform: skewX(-10deg);
        pointer-events: none;
        opacity: 0.9;
        mask-image: linear-gradient(90deg, transparent 0%, black 35%);
        -webkit-mask-image: linear-gradient(90deg, transparent 0%, black 35%);
    }
    .home-hero::after {
        content: '';
        position: absolute;
        inset: auto 0 0 0;
        height: 4rem;
        background: linear-gradient(180deg, transparent, var(--bg));
        pointer-events: none;
        z-index: 3;
    }
    /* Decorative org initials — fills the empty right half of wide viewports */
    .home-hero-mark {
        position: absolute;
        right: max(2%, calc((100% - var(--home-max)) / 2 + 4%));
        bottom: -0.08em;
        font-family: var(--font-head);
        font-size: clamp(7rem, 28vw, 16rem);
        line-height: 0.78;
        letter-spacing: 0.02em;
        color: transparent;
        -webkit-text-stroke: 1.5px color-mix(in srgb, var(--accent) 18%, transparent);
        opacity: 0.55;
        pointer-events: none;
        user-select: none;
        z-index: 1;
    }
    /* Aligns hero copy with body column */
    .home-hero-rail {
        position: relative;
        z-index: 2;
        width: 100%;
        max-width: var(--home-max);
        margin: 0 auto;
        padding: 3.5rem var(--home-pad-x) 3.25rem;
        box-sizing: border-box;
    }
    .home-hero-inner {
        position: relative;
        z-index: 2;
        max-width: min(40rem, 100%);
    }
    .home-badge {
        display: inline-flex;
        align-items: center;
        gap: 0.5rem;
        font-family: var(--font-mono);
        font-size: 0.68rem;
        letter-spacing: 0.16em;
        text-transform: uppercase;
        color: var(--accent);
        margin-bottom: 1.1rem;
        padding: 0.35rem 0.7rem 0.35rem 0.5rem;
        border: 1px solid color-mix(in srgb, var(--accent) 45%, transparent);
        background: color-mix(in srgb, var(--accent) 12%, transparent);
        clip-path: polygon(0 0, 100% 0, 100% calc(100% - 6px), calc(100% - 6px) 100%, 0 100%);
        box-shadow: 0 0 24px color-mix(in srgb, var(--accent) 12%, transparent);
    }
    .home-badge::before {
        content: '';
        width: 6px;
        height: 6px;
        background: var(--accent);
        box-shadow: 0 0 8px var(--accent);
        animation: pulse-dot 1.6s ease-in-out infinite;
    }
    @keyframes pulse-dot {
        0%, 100% { opacity: 1; }
        50% { opacity: 0.35; }
    }
    .home-title {
        font-family: var(--font-head);
        font-size: clamp(2.85rem, 9.5vw, 5.5rem);
        line-height: 0.92;
        letter-spacing: 0.01em;
        color: var(--text);
        margin: 0;
        text-transform: uppercase;
        text-shadow: 3px 0 0 color-mix(in srgb, var(--accent) 28%, transparent);
    }
    .home-title em {
        font-style: normal;
        display: block;
        color: var(--accent);
        text-shadow:
            0 0 48px color-mix(in srgb, var(--accent) 40%, transparent),
            -2px 0 0 color-mix(in srgb, var(--accent) 22%, transparent);
    }
    .home-sub {
        margin: 1.35rem 0 0;
        max-width: 32rem;
        color: var(--text-2);
        font-size: 1.05rem;
        line-height: 1.6;
        font-weight: 400;
        border-left: 3px solid var(--accent);
        padding-left: 0.95rem;
    }
    .home-actions {
        display: flex;
        flex-wrap: wrap;
        gap: 0.65rem;
        margin-top: 1.85rem;
    }
    .home-metrics {
        display: inline-flex;
        flex-wrap: wrap;
        margin-top: 2rem;
        border: 1px solid var(--border);
        background: color-mix(in srgb, var(--bg) 65%, var(--surface));
        box-shadow: 0 8px 32px color-mix(in srgb, var(--bg) 40%, transparent);
    }
    .home-metric {
        padding: 0.9rem 1.35rem;
        border-right: 1px solid var(--border);
        min-width: 5.75rem;
        flex: 0 1 auto;
    }
    .home-metric:last-child { border-right: none; }
    .home-metric strong {
        display: block;
        font-family: var(--font-head);
        font-size: 1.85rem;
        letter-spacing: 0.04em;
        color: var(--text);
        line-height: 1;
        font-variant-numeric: tabular-nums;
    }
    .home-metric span {
        display: block;
        margin-top: 0.3rem;
        font-family: var(--font-mono);
        font-size: 0.58rem;
        letter-spacing: 0.14em;
        text-transform: uppercase;
        color: var(--text-3);
    }
    @media (max-width: 520px) {
        .home-metrics { display: flex; width: 100%; }
        .home-metric { border-right: none; border-bottom: 1px solid var(--border); flex: 1 1 100%; }
        .home-metric:last-child { border-bottom: none; }
        .home-hero-mark { font-size: clamp(5rem, 40vw, 8rem); opacity: 0.35; }
        .home-hero-rail { padding-top: 2.5rem; padding-bottom: 2.5rem; }
    }

    /* ——— Sections (quieter than hero; denser rhythm) ——— */
    .home-block {
        padding: 2rem 0;
        border-bottom: 1px solid color-mix(in srgb, var(--border) 70%, transparent);
    }
    .home-block:last-of-type { border-bottom: none; }
    .home-kicker {
        font-family: var(--font-mono);
        font-size: 0.62rem;
        letter-spacing: 0.14em;
        text-transform: uppercase;
        color: var(--text-3);
        margin-bottom: 0.4rem;
    }
    .home-kicker.compete {
        color: var(--warn);
    }
    .home-heading {
        font-family: var(--font-head);
        font-weight: 700;
        font-size: clamp(1.2rem, 2.4vw, 1.5rem);
        color: var(--text);
        letter-spacing: 0.01em;
        margin: 0 0 0.4rem;
        text-transform: none;
        line-height: 1.25;
    }
    /* Underlines only on hero — sections stay quiet */
    .home-heading::after {
        content: none;
    }
    .home-body {
        color: var(--text-2);
        font-size: 0.95rem;
        line-height: 1.65;
        max-width: 40rem;
        margin: 0.4rem 0 0;
    }

    /* Numbered rules — punchy 2×2 board */
    .rules {
        list-style: none;
        margin: 1.25rem 0 0;
        padding: 0;
        display: grid;
        grid-template-columns: 1fr 1fr;
        gap: 0.65rem;
    }
    @media (max-width: 700px) {
        .rules { grid-template-columns: 1fr; }
    }
    .rules li {
        display: grid;
        grid-template-columns: auto 1fr;
        gap: 0.75rem;
        align-items: start;
        padding: 0.9rem 1rem 0.9rem 0.85rem;
        background: color-mix(in srgb, var(--surface-2) 88%, var(--bg));
        border: 1px solid var(--border);
        border-left: 3px solid var(--accent);
        color: var(--text);
        font-size: 0.9rem;
        line-height: 1.45;
        transition: border-color 0.15s, background 0.15s, transform 0.15s;
    }
    .rules li:hover {
        border-color: color-mix(in srgb, var(--accent) 40%, var(--border));
        background: color-mix(in srgb, var(--accent) 6%, var(--surface-2));
        transform: translateX(2px);
    }
    .rules li .rn {
        font-family: var(--font-head);
        font-size: 1.35rem;
        letter-spacing: 0.04em;
        color: var(--accent);
        line-height: 1;
        min-width: 1.4rem;
        opacity: 0.95;
    }

    /* Live board — one column when only one side has data */
    .live-grid {
        display: grid;
        grid-template-columns: 1fr 1fr;
        gap: 1rem;
        margin-top: 0.85rem;
    }
    .live-grid.single {
        grid-template-columns: 1fr;
        max-width: 36rem;
    }
    @media (max-width: 720px) {
        .live-grid { grid-template-columns: 1fr; }
    }
    .live-panel {
        background: var(--surface-2);
        border: 1px solid var(--border);
        padding: 1.1rem 1.15rem 1.25rem;
        position: relative;
        overflow: hidden;
    }
    .live-panel::before {
        content: '';
        position: absolute;
        top: 0; left: 0; right: 0;
        height: 2px;
        background: linear-gradient(90deg, var(--border), transparent 80%);
    }
    /* Compete / tournaments: gold accent (not brand purple) */
    .live-panel.compete::before {
        background: linear-gradient(90deg, var(--warn), transparent 75%);
    }
    .live-panel .home-heading {
        font-size: 1.15rem;
    }
    .live-list {
        list-style: none;
        margin: 0.85rem 0 0;
        padding: 0;
    }
    .live-list li {
        display: flex;
        justify-content: space-between;
        gap: 1rem;
        align-items: baseline;
        padding: 0.65rem 0;
        border-bottom: 1px solid var(--border);
        font-size: 0.9rem;
    }
    .live-list li:last-child { border-bottom: none; }
    .live-list a {
        color: var(--text);
        font-family: var(--font-head);
        font-weight: 700;
        font-size: 1rem;
    }
    .live-list a:hover { color: var(--accent); }
    .live-meta {
        font-family: var(--font-mono);
        font-size: 0.68rem;
        color: var(--text-3);
        white-space: nowrap;
    }
    .tag {
        font-family: var(--font-mono);
        font-size: 0.58rem;
        letter-spacing: 0.1em;
        text-transform: uppercase;
        color: var(--warn);
        border: 1px solid color-mix(in srgb, var(--warn) 40%, transparent);
        padding: 0.15rem 0.4rem;
        background: color-mix(in srgb, var(--warn) 10%, transparent);
    }
    .tag.live {
        color: var(--danger);
        border-color: color-mix(in srgb, var(--danger) 50%, transparent);
        background: color-mix(in srgb, var(--danger) 12%, transparent);
    }
    .tag.open {
        color: var(--warn);
        border-color: color-mix(in srgb, var(--warn) 45%, transparent);
        background: color-mix(in srgb, var(--warn) 10%, transparent);
    }
    .home-link.compete {
        color: var(--warn);
    }
    .home-link.compete:hover {
        color: var(--warn);
        border-bottom-color: var(--warn);
    }

    /* Team rows — shared fixed tracks so every column lines up across rows */
    .team-rows {
        margin-top: 1.1rem;
        border-top: 1px solid var(--border);
        /* name | game | roster | division | record */
        --tm-cols: minmax(0, 1.4fr) minmax(7.5rem, 0.9fr) 5.5rem 6.5rem 3.25rem;
    }
    .team-head,
    .team-row {
        display: grid;
        grid-template-columns: var(--tm-cols);
        column-gap: 1rem;
        align-items: baseline;
        padding: 0.7rem 0.5rem 0.7rem 0.65rem;
    }
    .team-head {
        padding-top: 0.55rem;
        padding-bottom: 0.45rem;
        border-bottom: 1px solid var(--border);
        font-family: var(--font-mono);
        font-size: 0.58rem;
        letter-spacing: 0.12em;
        text-transform: uppercase;
        color: var(--text-3);
    }
    .team-head span:nth-child(3),
    .team-head span:nth-child(4),
    .team-head span:nth-child(5) {
        text-align: right;
    }
    .team-row {
        border-bottom: 1px solid var(--border);
        font-size: 0.88rem;
        position: relative;
        transition: background 0.15s;
    }
    .team-row::before {
        content: '';
        position: absolute;
        left: 0; top: 0; bottom: 0;
        width: 0;
        background: var(--accent);
        transition: width 0.15s;
    }
    .team-row:hover {
        background: color-mix(in srgb, var(--accent) 6%, transparent);
    }
    .team-row:hover::before { width: 3px; }
    .team-row .tm-name {
        font-family: var(--font-head);
        font-weight: 700;
        color: var(--text);
        font-size: 1.05rem;
        letter-spacing: 0.02em;
        min-width: 0;
    }
    .team-row .tm-name-link {
        color: inherit;
        text-decoration: none;
    }
    .team-row .tm-name-link:hover {
        color: var(--accent);
        text-decoration: underline;
    }
    .team-row .tm-game {
        font-family: var(--font-mono);
        font-size: 0.65rem;
        letter-spacing: 0.08em;
        text-transform: uppercase;
        color: var(--text-3);
        min-width: 0;
    }
    .team-row .tm-roster {
        font-family: var(--font-mono);
        font-size: 0.65rem;
        letter-spacing: 0.06em;
        text-transform: uppercase;
        color: var(--text-3);
        text-align: right;
        font-variant-numeric: tabular-nums;
        white-space: nowrap;
    }
    .team-row .tm-div {
        color: var(--text-2);
        font-size: 0.8rem;
        text-align: right;
        white-space: nowrap;
        overflow: hidden;
        text-overflow: ellipsis;
    }
    .team-row .tm-wl {
        font-family: var(--font-mono);
        font-size: 0.8rem;
        letter-spacing: 0.04em;
        color: var(--text-2);
        text-align: right;
        font-variant-numeric: tabular-nums;
        white-space: nowrap;
    }
    .team-row.forming {
        opacity: 0.92;
    }
    .team-row .tm-forming {
        display: inline-block;
        margin-left: 0.45rem;
        font-family: var(--font-mono);
        font-size: 0.58rem;
        letter-spacing: 0.1em;
        text-transform: uppercase;
        color: var(--warn);
        border: 1px solid color-mix(in srgb, var(--warn) 40%, transparent);
        background: color-mix(in srgb, var(--warn) 10%, transparent);
        padding: 0.1rem 0.35rem;
        vertical-align: middle;
    }
    .team-row .tm-roster.forming {
        color: var(--warn);
    }
    .team-lore {
        grid-column: 1 / -1;
        color: var(--text-3);
        font-size: 0.8rem;
        font-style: italic;
        margin-top: 0.15rem;
    }
    @media (max-width: 720px) {
        .team-rows {
            --tm-cols: minmax(0, 1fr) auto;
        }
        .team-head span:nth-child(n+3),
        .team-row .tm-roster,
        .team-row .tm-div { display: none; }
        .team-head span:nth-child(2) { display: none; }
        .team-row .tm-game { display: none; }
        .team-row .tm-wl { justify-self: end; }
    }

    /* News */
    .news-rows { margin-top: 1rem; }
    .news-row {
        padding: 1rem 0 1.1rem;
        border-bottom: 1px solid var(--border);
        transition: padding-left 0.15s;
    }
    .news-row:hover { padding-left: 0.4rem; }
    .news-row:last-child { border-bottom: none; }
    .news-row time {
        font-family: var(--font-mono);
        font-size: 0.65rem;
        color: var(--text-3);
        letter-spacing: 0.1em;
    }
    .news-row h3 {
        font-family: var(--font-head);
        font-weight: 700;
        font-size: 1.2rem;
        color: var(--text);
        margin: 0.25rem 0 0.4rem;
    }
    .news-row p {
        color: var(--text-2);
        font-size: 0.9rem;
        line-height: 1.55;
        margin: 0;
        max-width: 40rem;
    }
    .pin {
        font-family: var(--font-mono);
        font-size: 0.58rem;
        letter-spacing: 0.1em;
        text-transform: uppercase;
        color: var(--accent);
        margin-left: 0.5rem;
        border: 1px solid color-mix(in srgb, var(--accent) 40%, transparent);
        padding: 0.05rem 0.3rem;
    }

    /* Recruit banner */
    .recruit-banner {
        margin-top: 0.75rem;
        display: grid;
        grid-template-columns: 1.15fr 1fr;
        gap: 0;
        border: 1px solid var(--border);
        background: var(--surface-2);
        overflow: hidden;
        box-shadow: 0 12px 40px color-mix(in srgb, var(--bg) 35%, transparent);
    }
    @media (max-width: 720px) {
        .recruit-banner { grid-template-columns: 1fr; }
    }
    .recruit-left {
        padding: 1.6rem 1.45rem;
        background:
            linear-gradient(135deg, color-mix(in srgb, var(--accent) 16%, transparent) 0%, transparent 55%),
            var(--surface-2);
        border-right: 1px solid var(--border);
    }
    @media (max-width: 720px) {
        .recruit-left { border-right: none; border-bottom: 1px solid var(--border); }
    }
    .recruit-right {
        padding: 1.4rem 1.3rem;
        background: color-mix(in srgb, var(--surface) 90%, var(--bg));
    }
    .expect-list {
        list-style: none;
        margin: 0.5rem 0 0;
        padding: 0;
    }
    .expect-list li {
        display: flex;
        gap: 0.55rem;
        padding: 0.45rem 0;
        color: var(--text-2);
        font-size: 0.86rem;
        border-bottom: 1px solid var(--border);
    }
    .expect-list li::before {
        content: "▸";
        color: var(--accent);
        font-weight: 700;
        flex-shrink: 0;
    }
    .never-box {
        margin-top: 1rem;
        padding: 0.85rem 0.9rem;
        border: 1px dashed color-mix(in srgb, var(--accent) 35%, transparent);
        background: color-mix(in srgb, var(--bg) 40%, transparent);
    }
    .never-box h4 {
        font-family: var(--font-mono);
        font-size: 0.62rem;
        letter-spacing: 0.12em;
        text-transform: uppercase;
        color: var(--accent);
        margin: 0 0 0.4rem;
    }
    .never-box p {
        margin: 0;
        font-size: 0.8rem;
        color: var(--text-2);
        line-height: 1.5;
    }
    .seek-tags {
        display: flex;
        flex-wrap: wrap;
        gap: 0.4rem;
        margin-top: 1rem;
    }
    .seek-tag {
        font-family: var(--font-mono);
        font-size: 0.65rem;
        letter-spacing: 0.08em;
        text-transform: uppercase;
        padding: 0.3rem 0.55rem;
        border: 1px solid color-mix(in srgb, var(--accent) 40%, transparent);
        color: var(--text);
        background: color-mix(in srgb, var(--accent) 12%, transparent);
    }

    .home-link {
        display: inline-block;
        margin-top: 0.9rem;
        font-family: var(--font-mono);
        font-size: 0.7rem;
        letter-spacing: 0.12em;
        text-transform: uppercase;
        color: var(--accent);
        border-bottom: 1px solid transparent;
    }
    .home-link:hover {
        color: var(--text);
        border-bottom-color: var(--accent);
    }

    .btn {
        display: inline-flex;
        align-items: center;
        padding: 0.7rem 1.25rem;
        font-family: var(--font-mono);
        font-size: 0.72rem;
        letter-spacing: 0.14em;
        text-transform: uppercase;
        border-radius: 0;
        border: 1px solid transparent;
        cursor: pointer;
        text-decoration: none;
        transition: transform 0.15s, filter 0.15s, border-color 0.15s, color 0.15s, box-shadow 0.15s;
        clip-path: polygon(0 0, 100% 0, 100% calc(100% - 8px), calc(100% - 8px) 100%, 0 100%);
    }
    .btn-primary {
        background: var(--accent);
        color: var(--accent-fg);
        box-shadow: 0 0 0 0 color-mix(in srgb, var(--accent) 40%, transparent);
    }
    .btn-primary:hover {
        filter: brightness(1.12);
        transform: translateY(-1px);
        box-shadow: 0 0 28px color-mix(in srgb, var(--accent) 40%, transparent);
    }
    .btn-outline {
        background: transparent;
        border-color: var(--border);
        color: var(--text);
    }
    .btn-outline:hover {
        border-color: var(--text-2);
        color: var(--text);
        transform: translateY(-1px);
    }

    .muted {
        color: var(--text-3);
        font-size: 0.88rem;
    }
    .home-foot {
        margin-top: 2.5rem;
        padding-top: 1.25rem;
        border-top: 1px solid var(--border);
        font-family: var(--font-mono);
        font-size: 0.65rem;
        letter-spacing: 0.1em;
        text-transform: uppercase;
        color: var(--text-3);
    }

    /* ——— Alignment: Center = hero only; body always left ——— */
    .home-wrap:has(.home.align-center) .home-hero-inner {
        margin-left: auto;
        margin-right: auto;
        text-align: center;
        max-width: 42rem;
    }
    .home-wrap:has(.home.align-center) .home-sub {
        margin-left: auto;
        margin-right: auto;
        border-left: none;
        border-top: 3px solid var(--accent);
        padding-left: 0;
        padding-top: 0.85rem;
        max-width: 32rem;
    }
    .home-wrap:has(.home.align-center) .home-actions {
        justify-content: center;
    }
    .home-wrap:has(.home.align-center) .home-metrics {
        margin-left: auto;
        margin-right: auto;
    }
    .home-wrap:has(.home.align-center) .home-badge {
        margin-left: auto;
        margin-right: auto;
    }
    .home-wrap:has(.home.align-center) .home-hero-mark {
        right: 50%;
        transform: translateX(50%);
        opacity: 0.35;
    }
    /* Body blocks intentionally stay left — no mixed centered headers over left lists */
    .home.align-center .home-block {
        text-align: left;
    }
    .home.align-center .recruit-left {
        text-align: left;
    }
"#;

/// Clean skin overrides (calmer radii, less clip, softer watermark).
pub const HOME_SKIN_CLEAN_CSS: &str = r#"
.home-wrap[data-home-skin="clean"] .home-badge {
    clip-path: none;
    border-radius: var(--radius-sm, 7px);
    letter-spacing: 0.1em;
}
.home-wrap[data-home-skin="clean"] .home-title {
    letter-spacing: -0.02em;
    font-weight: 600;
}
.home-wrap[data-home-skin="clean"] .home-hero-mark {
    -webkit-text-stroke: 1px color-mix(in srgb, var(--accent) 12%, transparent);
    opacity: 0.45;
}
.home-wrap[data-home-skin="clean"] .home-metrics {
    border-radius: var(--radius-md, 9px);
    overflow: hidden;
}
.home-wrap[data-home-skin="clean"] .rules li {
    border-radius: 0 var(--radius-sm, 7px) var(--radius-sm, 7px) 0;
}
.home-wrap[data-home-skin="clean"] .recruit-banner {
    border-radius: var(--radius-md, 9px);
    overflow: hidden;
}
.home-wrap[data-home-skin="clean"] .btn {
    clip-path: none;
    border-radius: var(--radius-sm, 7px);
}
.home-wrap[data-home-skin="clean"] .seek-tag {
    border-radius: var(--radius-pill, 999px);
}
"#;

/// Esports skin: current base already is esports; keep hook for future motif tweaks.
pub const HOME_SKIN_ESPORTS_CSS: &str = r#"
.home-wrap[data-home-skin="esports"] .home-badge {
    /* clipped badge geometry lives in shared CSS */
}
"#;

/// Shell-specific composition tweaks.
pub const HOME_SHELL_CSS: &str = r#"
/* Ops hub: slightly wider body for dense boards */
.home-wrap[data-home-shell="ops_hub"] {
    --home-max: min(80rem, 100%);
}

/* Recruit landing: more air, recruit emphasis */
.home-wrap[data-home-shell="recruit_landing"] {
    --home-max: min(72rem, 100%);
}
.home-wrap[data-home-shell="recruit_landing"] .home-hero-rail {
    padding-top: 4.25rem;
    padding-bottom: 3.5rem;
}
.home-wrap[data-home-shell="recruit_landing"] .home-hero-inner {
    max-width: min(44rem, 100%);
}
.home-wrap[data-home-shell="recruit_landing"] .home-sub {
    font-size: 1.08rem;
    max-width: min(36rem, 100%);
}
.home-wrap[data-home-shell="recruit_landing"] .home-metrics {
    opacity: 0.92;
}

/* Minimal: compact column */
.home-wrap[data-home-shell="minimal"] {
    --home-max: min(48rem, 100%);
}
.home-wrap[data-home-shell="minimal"] .home-hero-rail {
    padding-top: 2.5rem;
    padding-bottom: 2rem;
}
.home-wrap[data-home-shell="minimal"] .home-hero-inner {
    max-width: 100%;
}
.home-wrap[data-home-shell="minimal"] .home-hero-mark {
    font-size: clamp(4rem, 16vw, 8rem);
    opacity: 0.4;
}
.home-wrap[data-home-shell="minimal"] .home-title {
    font-size: clamp(2.4rem, 8vw, 3.6rem);
}
.home-wrap[data-home-shell="minimal"] .home-metrics {
    display: none;
}

/* Manifesto: editorial reading width */
.home-wrap[data-home-shell="manifesto"] {
    --home-max: min(64rem, 100%);
}
.home-wrap[data-home-shell="manifesto"] .home-hero-inner {
    max-width: min(38rem, 100%);
}
.home-wrap[data-home-shell="manifesto"] .home-hero-rail {
    padding-top: 3.75rem;
    padding-bottom: 3.25rem;
}
.home-wrap[data-home-shell="manifesto"] .home-body {
    font-size: 1.02rem;
    line-height: 1.65;
    max-width: min(42rem, 100%);
}
.home-wrap[data-home-shell="manifesto"] .never-box {
    border-style: solid;
}
.home-wrap[data-home-shell="manifesto"] .rules {
    max-width: 48rem;
}

/* Teams presentations */
.home-wrap .team-cards {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(14rem, 1fr));
    gap: 0.75rem;
    margin-top: 0.85rem;
}
.home-wrap a.team-card {
    display: block;
    color: inherit;
    text-decoration: none;
}
.home-wrap a.team-chip {
    display: inline-block;
    text-decoration: none;
}
.home-wrap .team-card {
    border: 1px solid var(--border);
    background: color-mix(in srgb, var(--surface) 92%, var(--bg));
    padding: 1rem 1.1rem;
    border-radius: var(--radius-md, 9px);
    transition: border-color 0.15s, background 0.15s;
}
.home-wrap .team-card:hover {
    border-color: color-mix(in srgb, var(--accent) 35%, var(--border));
    background: color-mix(in srgb, var(--accent) 5%, var(--surface));
}
.home-wrap .team-card .tc-name {
    font-family: var(--font-head);
    font-weight: 600;
    font-size: 1.05rem;
}
.home-wrap .team-card .tc-meta {
    margin-top: 0.35rem;
    font-size: 0.8rem;
    color: var(--text-2);
}
.home-wrap .team-card.forming {
    border-style: dashed;
    opacity: 0.9;
}
.home-wrap .team-compact {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
    margin-top: 0.85rem;
}
.home-wrap .team-chip {
    font-family: var(--font-mono);
    font-size: 0.72rem;
    letter-spacing: 0.04em;
    padding: 0.4rem 0.75rem;
    border: 1px solid var(--border);
    background: var(--surface-2);
    border-radius: var(--radius-pill, 999px);
    color: var(--text-2);
    transition: border-color 0.15s, color 0.15s;
}
.home-wrap .team-chip:hover {
    border-color: color-mix(in srgb, var(--accent) 40%, var(--border));
    color: var(--text);
}
.home-wrap .team-chip.forming {
    border-style: dashed;
    color: var(--text-3);
}
"#;

pub fn home_css_layers() -> String {
    let mut s = String::with_capacity(
        HOME_SHARED_CSS.len()
            + HOME_SHELL_CSS.len()
            + HOME_SKIN_CLEAN_CSS.len()
            + HOME_SKIN_ESPORTS_CSS.len()
            + 8,
    );
    s.push_str(HOME_SHARED_CSS);
    s.push_str(HOME_SHELL_CSS);
    s.push_str(HOME_SKIN_CLEAN_CSS);
    s.push_str(HOME_SKIN_ESPORTS_CSS);
    s
}
